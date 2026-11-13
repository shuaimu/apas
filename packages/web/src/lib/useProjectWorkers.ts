"use client";

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { WorkerListResponse, WorkerState, WorkerSummary } from "@apas/protocol";
import { useStore } from "./store";
import { onWorkerCachePurge, readWorkerRecord, workerCacheRevision, writeWorkerRecord } from "./workerCache";
import type { WorkerScope } from "./workerCache";
import { onWorkerMessage, workerFetch, workerPath } from "./workers";
import { useWorkerOnline } from "./useWorkers";

type Mode = "refresh" | "more";
interface Inventory {
  scope: string;
  revision: number;
  socket: WebSocket | null;
  workers: WorkerSummary[];
  nextCursor: string | null;
  pages: number;
  ready: boolean;
  loading: boolean;
  error: string;
}
const emptyWorkers: WorkerSummary[] = [];
function mergeWorkers(incoming: WorkerSummary[], previous: WorkerSummary[]) {
  const known = new Map(previous.map((worker) => [worker.id, worker]));
  const merged = new Map<string, WorkerSummary>();
  for (const worker of incoming) {
    const older = merged.get(worker.id) ?? known.get(worker.id);
    merged.set(worker.id, older && (older.state_version > worker.state_version || older.high_watermark > worker.high_watermark) ? older : worker);
  }
  return [...merged.values()];
}

/** undefined selects global inventory; null disables all reads, including global reads. */
export function useProjectWorkers(projectId?: string | null, status?: WorkerState | "attention") {
  const account = useStore((state) => state.userId);
  const token = useStore((state) => state.token);
  const socket = useStore((state) => state.ws);
  const online = useWorkerOnline();
  const revision = useSyncExternalStore(onWorkerCachePurge, workerCacheRevision, () => 0);
  const scopeKey = JSON.stringify([account, token, projectId ?? null, projectId === undefined, status]);
  const [inventory, setInventory] = useState<Inventory>();
  const current = useRef<Inventory | undefined>(undefined);
  const previousRevision = useRef(revision);
  const blocked = useRef(false);
  const control = useRef<{ scope: string; revision: number; run: (mode: Mode, explicit?: boolean) => void } | undefined>(undefined);

  useEffect(() => {
    if (previousRevision.current !== revision) blocked.current = true;
    previousRevision.current = revision;
    if (!account || projectId === null) { current.current = undefined; setInventory(undefined); return; }
    const scope: WorkerScope = [account, projectId ?? "", ""];
    const cacheId = status ?? "all";
    const retained = current.current?.scope === scopeKey && current.current.revision === revision ? current.current : undefined;
    let view: Inventory = {
      scope: scopeKey, revision, socket, workers: retained?.workers ?? [],
      nextCursor: retained?.nextCursor ?? null, pages: retained?.pages ?? 1,
      ready: false, loading: online && !blocked.current,
      error: blocked.current ? "Worker access changed. Refresh to check current access." : "",
    };
    let active = true;
    let busy = true;
    let refreshPending = false;
    const valid = () => active && revision === workerCacheRevision() && useStore.getState().userId === account && useStore.getState().token === token;
    const publish = (changes: Partial<Inventory>) => {
      if (!valid()) return;
      view = { ...view, ...changes };
      current.current = view;
      setInventory(view);
    };
    publish({});
    const run = async (mode: Mode, explicit = false) => {
      if (!valid() || !online) return;
      if (explicit) blocked.current = false;
      if (blocked.current) return;
      if (busy) { if (mode === "refresh") { refreshPending = true; publish({ ready: false }); } return; }
      if (mode === "more" && !view.nextCursor) return;
      busy = true;
      publish({ loading: true, ready: false, error: "" });
      try {
        let cursor = mode === "more" ? view.nextCursor : null;
        const count = mode === "more" ? 1 : view.pages;
        let workers: WorkerSummary[] = mode === "more" ? [...view.workers] : [];
        let pages = mode === "more" ? view.pages : 0;
        let firstPage: WorkerListResponse | undefined;
        const cursors = new Set<string>();
        // Reauthorize only explicitly opened pages; never prefetch history to exhaustion.
        for (let index = 0; index < count; index += 1) {
          const params = new URLSearchParams();
          if (cursor) params.set("cursor", cursor);
          if (status) params.set("status", status);
          const query = params.toString();
          const page = await workerFetch<WorkerListResponse>(`${projectId === undefined ? "/projects/workers" : workerPath(projectId)}${query ? `?${query}` : ""}`, "list");
          if (!valid()) return;
          if (projectId !== undefined && page.workers.some((worker) => worker.project_id !== projectId)) throw new Error("The server returned workers from another project.");
          if (status && page.workers.some((worker) => status === "attention" ? !worker.attention : worker.state !== status)) throw new Error("The server returned workers outside the requested status filter.");
          if (mode === "refresh" && index === 0) firstPage = page;
          workers.push(...page.workers);
          pages += 1;
          const next = page.next_cursor ?? null;
          if (next && (next === cursor || cursors.has(next))) throw new Error("Worker pagination did not advance. Refresh the inventory.");
          if (cursor) cursors.add(cursor);
          cursor = next;
          if (!cursor) break;
        }
        workers = mergeWorkers(workers, view.workers);
        if (firstPage) {
          await writeWorkerRecord(scope, "list", { ...firstPage, workers: mergeWorkers(firstPage.workers, workers) }, cacheId, revision);
          if (!valid()) return;
        }
        publish({ workers, nextCursor: cursor, pages, ready: !refreshPending, error: "" });
      } catch (failure) {
        publish({ ready: false, error: failure instanceof Error ? failure.message : "Workers could not be loaded." });
      } finally {
        busy = false;
        if (valid()) {
          publish({ loading: false });
          if (refreshPending) { refreshPending = false; void run("refresh"); }
        }
      }
    };
    control.current = { scope: scopeKey, revision, run: (mode, explicit) => { void run(mode, explicit); } };
    const restore = async () => {
      if (!retained && !blocked.current) {
        try {
          const cached = await readWorkerRecord<WorkerListResponse>(scope, "list", cacheId);
          if (!valid()) return;
          if (cached && cached.workers.every((worker) => (projectId === undefined || worker.project_id === projectId) && typeof worker.description === "string")) {
            publish({ workers: mergeWorkers(cached.workers, []), nextCursor: cached.next_cursor ?? null });
          }
        } catch (failure) { publish({ error: failure instanceof Error ? failure.message : "Cached workers could not be loaded." }); }
      }
      busy = false;
      refreshPending = false;
      if (valid()) await run("refresh");
    };
    void restore();
    const remove = onWorkerMessage((message) => {
      if (!valid() || blocked.current || useStore.getState().ws !== socket) return;
      if (message.type === "worker_inbox_changed" || message.type === "worker_event" && (projectId === undefined || message.event.project_id === projectId) || message.type === "worker_snapshot" && (projectId === undefined || message.detail.worker.project_id === projectId)) void run("refresh");
    });
    const timer = window.setInterval(() => { void run("refresh"); }, 20_000);
    return () => {
      active = false;
      remove();
      clearInterval(timer);
      if (control.current?.scope === scopeKey && control.current.revision === revision) control.current = undefined;
    };
  }, [account, token, projectId, status, scopeKey, online, socket, revision]);

  const refresh = useCallback(() => {
    if (control.current?.scope === scopeKey && control.current.revision === revision) control.current.run("refresh", true);
  }, [scopeKey, revision]);
  const loadMore = useCallback(() => {
    if (control.current?.scope === scopeKey && control.current.revision === revision) control.current.run("more", true);
  }, [scopeKey, revision]);
  const visible = inventory?.scope === scopeKey && inventory.revision === revision && projectId !== null ? inventory : undefined;
  return { workers: visible?.workers ?? emptyWorkers, ready: Boolean(online && visible?.ready && visible.socket === socket), loading: visible?.loading ?? false, error: visible?.error ?? "", hasMore: Boolean(visible?.nextCursor), loadMore, refresh };
}
