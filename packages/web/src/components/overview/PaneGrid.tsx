"use client";

/**
 * Phase 5.1b — pane grid for the Overview tab. One card per pane,
 * showing the at-a-glance roll-up the user described:
 *   label + role chip · status pill · mode · worktree · last-activity
 * Quick-action row: Diff / Role (modal opens) and a click-anywhere
 * "Open" affordance that switches the active tab.
 */
import { useMemo } from "react";
import {
  PaneConfig,
  useStore,
  paneKey,
  PANE_ID_DEADLOOP,
  PANE_ID_INTERACTIVE,
} from "@/lib/store";

interface PaneGridProps {
  onOpenPane: (paneId: number) => void;
  onOpenDiff: (paneId: number) => void;
  onOpenRole: (paneId: number) => void;
}

export function PaneGrid({ onOpenPane, onOpenDiff, onOpenRole }: PaneGridProps) {
  const paneConfigs = useStore((s) => s.paneConfigs);
  const paneStatuses = useStore((s) => s.paneStatuses);
  const pausedPanes = useStore((s) => s.pausedPanes);
  const paneMessages = useStore((s) => s.paneMessages);
  const paneDiffs = useStore((s) => s.paneDiffs);

  const sorted = useMemo(() => {
    const arr = [...paneConfigs];
    arr.sort((a, b) => a.pane_id - b.pane_id);
    return arr;
  }, [paneConfigs]);

  if (sorted.length === 0) {
    return (
      <div className="rounded border border-dashed border-gray-300 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/30 p-4 text-sm italic text-gray-500 dark:text-gray-400">
        No panes in this project yet. Add one via the + button in the tab bar.
      </div>
    );
  }

  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
      {sorted.map((pane) => {
        const status = paneStatuses[paneKey(pane.pane_id)];
        const isPaused = pausedPanes.includes(pane.pane_id);
        const messages = paneMessages[paneKey(pane.pane_id)] ?? [];
        const lastMsg = messages[messages.length - 1];
        const diff = paneDiffs[pane.pane_id];
        return (
          <PaneCard
            key={pane.pane_id}
            pane={pane}
            status={status ?? null}
            isPaused={isPaused}
            lastActivity={lastMsg?.timestamp}
            diffStats={diff?.diff ? summarizeDiffStats(diff.diff) : undefined}
            onOpen={() => onOpenPane(pane.pane_id)}
            onOpenDiff={() => onOpenDiff(pane.pane_id)}
            onOpenRole={() => onOpenRole(pane.pane_id)}
          />
        );
      })}
    </div>
  );
}

interface PaneCardProps {
  pane: PaneConfig;
  status: string | null;
  isPaused: boolean;
  lastActivity?: Date;
  diffStats?: { added: number; removed: number };
  onOpen: () => void;
  onOpenDiff: () => void;
  onOpenRole: () => void;
}

function PaneCard({
  pane,
  status,
  isPaused,
  lastActivity,
  diffStats,
  onOpen,
  onOpenDiff,
  onOpenRole,
}: PaneCardProps) {
  const isBot = pane.mode === "deadloop";
  const isThinking = !!status && !isPaused;
  const modeIndicator = isBot
    ? isPaused
      ? { icon: "⏸", label: "paused" }
      : { icon: "⏵", label: "running" }
    : isThinking
      ? { icon: "●", label: "thinking" }
      : { icon: "•", label: "idle" };
  const modeColor = isBot
    ? isPaused
      ? "text-amber-500"
      : "text-emerald-500"
    : isThinking
      ? "text-blue-500 animate-pulse"
      : "text-gray-400";
  const providerBadge = pane.model
    ? `${pane.provider} · ${pane.model}`
    : pane.provider;
  const label = pane.label || `Tab ${pane.pane_id}`;
  return (
    <button
      type="button"
      onClick={onOpen}
      className="flex flex-col items-stretch gap-2 rounded border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800/60 p-3 text-left transition-colors hover:border-indigo-400 dark:hover:border-indigo-500"
      title={`Open ${label}`}
    >
      <div className="flex flex-wrap items-baseline gap-2">
        <span className={`flex-shrink-0 text-base ${modeColor}`} aria-label={modeIndicator.label}>
          {modeIndicator.icon}
        </span>
        <span className="font-semibold text-gray-900 dark:text-gray-100 truncate">{label}</span>
        {pane.role && (
          <span className="rounded bg-purple-100 dark:bg-purple-900/40 px-1.5 py-0.5 text-[10px] font-medium text-purple-700 dark:text-purple-300">
            {pane.role}
          </span>
        )}
        <span className="ml-auto rounded bg-gray-100 dark:bg-gray-700 px-1.5 py-0.5 text-[10px] font-mono text-gray-600 dark:text-gray-400">
          {providerBadge}
        </span>
      </div>

      {status && (
        <div className="rounded bg-blue-50 dark:bg-blue-900/30 px-2 py-1 text-xs text-blue-700 dark:text-blue-300">
          {status}
        </div>
      )}

      {pane.worktree_path && (
        <div className="flex items-center gap-2 text-[11px] text-gray-500 dark:text-gray-400">
          <span className="font-mono">apas-pane-{pane.pane_id}</span>
          {diffStats && (
            <>
              <span className="text-emerald-500">+{diffStats.added}</span>
              <span className="text-red-500">-{diffStats.removed}</span>
            </>
          )}
        </div>
      )}

      <div className="flex items-center gap-2 text-[11px] text-gray-500 dark:text-gray-400">
        <span>{lastActivity ? `last: ${formatRelative(lastActivity)}` : "no activity yet"}</span>
        <span className="ml-auto flex gap-1.5">
          {pane.worktree_path && (
            <span
              className="rounded border border-emerald-400 dark:border-emerald-700 bg-emerald-50 dark:bg-emerald-900/30 px-1.5 py-0.5 text-emerald-700 dark:text-emerald-300 hover:bg-emerald-100 dark:hover:bg-emerald-900/50"
              role="button"
              onClick={(e) => {
                e.stopPropagation();
                onOpenDiff();
              }}
            >
              Diff
            </span>
          )}
          <span
            className="rounded border border-purple-400 dark:border-purple-700 bg-purple-50 dark:bg-purple-900/30 px-1.5 py-0.5 text-purple-700 dark:text-purple-300 hover:bg-purple-100 dark:hover:bg-purple-900/50"
            role="button"
            onClick={(e) => {
              e.stopPropagation();
              onOpenRole();
            }}
          >
            Role
          </span>
          {(pane.pane_id === PANE_ID_INTERACTIVE || pane.pane_id === PANE_ID_DEADLOOP) && (
            <span
              className="rounded bg-gray-100 dark:bg-gray-700 px-1.5 py-0.5 text-[10px] text-gray-500 dark:text-gray-400"
              title="Built-in default pane (can't be removed)"
            >
              default
            </span>
          )}
        </span>
      </div>
    </button>
  );
}

function summarizeDiffStats(diff: string): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const line of diff.split("\n")) {
    if (line.startsWith("+") && !line.startsWith("+++")) added++;
    else if (line.startsWith("-") && !line.startsWith("---")) removed++;
  }
  return { added, removed };
}

function formatRelative(when: Date): string {
  const seconds = Math.floor((Date.now() - when.getTime()) / 1000);
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}
