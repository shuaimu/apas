"use client";

import { useCallback, useMemo, useSyncExternalStore } from "react";
import { useStore } from "@/lib/store";
import type { RepoGroup } from "@/lib/projectList";

type ProjectOrder = { groups: string[]; projects: Record<string, string[]> };
type Placement = "before" | "after";
const STORAGE_PREFIX = "apas_project_order:";
const EMPTY_ORDER: ProjectOrder = { groups: [], projects: {} };
const snapshots = new Map<string, ProjectOrder>();
const listeners = new Set<() => void>();
let listening = false;

function readOrder(key: string): ProjectOrder {
  try {
    const value = JSON.parse(window.localStorage.getItem(key) || "null");
    const ids = (raw: unknown): string[] => Array.isArray(raw)
      ? [...new Set(raw.filter((id): id is string => typeof id === "string"))]
      : [];
    if (!value || typeof value !== "object") return EMPTY_ORDER;
    return {
      groups: ids(value.groups),
      projects: Object.fromEntries(Object.entries(value.projects || {}).map(
        ([group, order]) => [group, ids(order)],
      )),
    };
  } catch {
    return EMPTY_ORDER;
  }
}

function getSnapshot(key: string | null): ProjectOrder {
  if (!key || typeof window === "undefined") return EMPTY_ORDER;
  if (!snapshots.has(key)) snapshots.set(key, readOrder(key));
  return snapshots.get(key)!;
}

function emit() {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  // Like useTheme, one page-wide store keeps the expanded and collapsed
  // sidebars consistent, including when localStorage writes are unavailable.
  if (!listening) {
    listening = true;
    window.addEventListener("storage", (event) => {
      if (event.key === null) snapshots.clear();
      else if (event.key.startsWith(STORAGE_PREFIX)) snapshots.delete(event.key);
      else return;
      emit();
    });
  }
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

function saveOrder(key: string | null, order: ProjectOrder) {
  if (!key) return;
  snapshots.set(key, order);
  try {
    window.localStorage.setItem(key, JSON.stringify(order));
  } catch {
    // Keep the current page usable when storage is blocked or full.
  }
  emit();
}

function ordered<T>(items: T[], saved: string[], id: (item: T) => string): T[] {
  if (!saved.length) return items;
  const ranks = new Map(saved.map((key, index) => [key, index]));
  return [...items].sort((a, b) =>
    (ranks.get(id(a)) ?? saved.length) - (ranks.get(id(b)) ?? saved.length),
  );
}

function moveOrder(
  visible: string[], saved: string[], from: string, to: string, placement: Placement,
): string[] | null {
  if (from === to || !visible.includes(from) || !visible.includes(to)) return null;
  // Retain temporarily absent projects: a partial reconnect must not erase
  // their positions. Newly discovered entries follow the saved ones.
  const order = [...new Set([...saved, ...visible])].filter((id) => id !== from);
  order.splice(order.indexOf(to) + (placement === "after" ? 1 : 0), 0, from);
  return order;
}

/** Browser-local, account-scoped layout; never a project policy or mutation. */
export function useProjectOrder(groups: RepoGroup[]) {
  const userId = useStore((state) => state.userId);
  const key = userId ? `${STORAGE_PREFIX}${userId}` : null;
  const order = useSyncExternalStore(
    subscribe,
    useCallback(() => getSnapshot(key), [key]),
    () => EMPTY_ORDER,
  );
  const repoGroups = useMemo(() => ordered(groups, order.groups, (group) => group.key)
    .map((group) => ({
      ...group,
      projects: ordered(group.projects, order.projects[group.key] ?? [], (project) => project.projectId),
    })), [groups, order]);

  const moveGroup = (from: string, to: string, placement: Placement) => {
    const next = moveOrder(repoGroups.map((group) => group.key), order.groups, from, to, placement);
    if (next) saveOrder(key, { ...order, groups: next });
  };
  const moveProject = (groupKey: string, from: string, to: string, placement: Placement) => {
    const group = repoGroups.find((group) => group.key === groupKey);
    if (!group) return;
    const next = moveOrder(
      group.projects.map((project) => project.projectId),
      order.projects[groupKey] ?? [], from, to, placement,
    );
    if (next) saveOrder(key, {
      groups: order.groups.length ? order.groups : repoGroups.map((group) => group.key),
      projects: { ...order.projects, [groupKey]: next },
    });
  };
  return { repoGroups, moveGroup, moveProject };
}
