"use client";

/**
 * Phase 5.1d — pairs delegate-to / reply-to records from the team
 * scratchpad so the human can see which dispatched tasks are still
 * awaiting a worker reply. Current records correlate by `task:<TODO-NNN>`
 * tags, with legacy `task-id:<uuid>` tags retained as a fallback.
 *
 * Source data: store.teamRecords (already populated by the CLI
 * watcher; no new wire types).
 */
import { useMemo } from "react";
import { useStore, TeamRecord } from "@/lib/store";

interface DelegationRow {
  delegate: TeamRecord;
  /** Pane the work was delegated TO (parsed from delegate-to:N tag). */
  toPane?: number;
  /** Current task tag value or legacy task-id fallback used for correlation. */
  taskKey?: string;
  /** Matching reply record if one has arrived. */
  reply?: TeamRecord;
}

const MAX_ROWS = 20;

export function DelegationBoard() {
  const teamRecords = useStore((s) => s.teamRecords);

  const rows = useMemo(() => buildRows(teamRecords), [teamRecords]);

  if (rows.length === 0) {
    return (
      <div className="rounded border border-dashed border-gray-300 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/30 p-4 text-sm italic text-gray-500 dark:text-gray-400">
        No delegations seen yet. A coordinator pane appends records with{" "}
        <code>delegate-to:&lt;pane_id&gt;</code> and{" "}
        <code>task:&lt;TODO-NNN&gt;</code> tags to dispatch work.
      </div>
    );
  }

  return (
    <div className="overflow-x-auto">
      <table className="w-full text-xs">
        <thead>
          <tr className="border-b border-gray-200 dark:border-gray-700 text-left text-[10px] uppercase tracking-wide text-gray-500 dark:text-gray-400">
            <th className="px-2 py-1.5 font-semibold">From → to</th>
            <th className="px-2 py-1.5 font-semibold">Task</th>
            <th className="px-2 py-1.5 font-semibold">Body</th>
            <th className="px-2 py-1.5 font-semibold">Status</th>
          </tr>
        </thead>
        <tbody>
          {rows.slice(0, MAX_ROWS).map((row, i) => (
            <DelegationRowView key={`${row.delegate.ts}-${i}`} row={row} />
          ))}
        </tbody>
      </table>
      {rows.length > MAX_ROWS && (
        <p className="mt-2 text-[11px] text-gray-500 dark:text-gray-400 italic">
          Showing newest {MAX_ROWS} of {rows.length}.
        </p>
      )}
    </div>
  );
}

function DelegationRowView({ row }: { row: DelegationRow }) {
  const { delegate, toPane, taskKey, reply } = row;
  const status = reply
    ? "replied"
    : taskKey
      ? "awaiting reply"
      : "untracked";
  const statusColor = reply
    ? "bg-emerald-100 dark:bg-emerald-900/40 text-emerald-700 dark:text-emerald-300"
    : taskKey
      ? "bg-amber-100 dark:bg-amber-900/40 text-amber-700 dark:text-amber-300"
      : "bg-gray-100 dark:bg-gray-800 text-gray-500 dark:text-gray-400";
  const replyDelta = reply ? deltaTs(delegate.ts, reply.ts) : null;
  return (
    <tr className="border-b border-gray-100 dark:border-gray-800 align-top">
      <td className="px-2 py-1.5 font-mono whitespace-nowrap text-gray-700 dark:text-gray-300">
        {delegate.pane_id !== undefined ? `pane ${delegate.pane_id}` : "?"}
        {" → "}
        {toPane !== undefined ? `pane ${toPane}` : "?"}
      </td>
      <td
        className="px-2 py-1.5 font-mono whitespace-nowrap text-gray-600 dark:text-gray-400"
        title={taskKey}
      >
        {taskKey ? taskLabel(taskKey) : "—"}
      </td>
      <td className="px-2 py-1.5 text-gray-700 dark:text-gray-300 max-w-[40ch] truncate" title={delegate.body}>
        {delegate.body}
      </td>
      <td className="px-2 py-1.5 whitespace-nowrap">
        <span className={`rounded px-1.5 py-0.5 font-medium ${statusColor}`}>{status}</span>
        {replyDelta && (
          <span className="ml-1.5 text-[10px] text-gray-500 dark:text-gray-400">
            {replyDelta}
          </span>
        )}
      </td>
    </tr>
  );
}

function buildRows(records: TeamRecord[]): DelegationRow[] {
  // First pass: collect delegates. Second pass: attach replies.
  const delegates: DelegationRow[] = [];
  // task key or alias → row index for fast reply attachment.
  const byTask = new Map<string, number>();
  for (const r of records) {
    const toTag = r.tags.find((t) => t.startsWith("delegate-to:"));
    if (toTag) {
      const toPaneStr = toTag.slice("delegate-to:".length);
      const toPane = /^\d+$/.test(toPaneStr) ? parseInt(toPaneStr, 10) : undefined;
      const taskKeys = taskTags(r);
      const taskKey = taskKeys[0] ?? legacyTaskId(r);
      const idx = delegates.length;
      delegates.push({ delegate: r, toPane, taskKey });
      for (const key of taskKeys.length > 0 ? taskKeys : taskKey ? [taskKey] : []) {
        for (const alias of taskAliases(key)) {
          byTask.set(alias, idx);
        }
      }
    }
  }
  for (const r of records) {
    if (r.tags.some((t) => t.startsWith("delegate-to:"))) continue;
    const replyKeys = replyTaskKeys(r);
    for (const key of replyKeys) {
      const idx = byTask.get(key);
      if (idx !== undefined && !delegates[idx].reply) {
        delegates[idx].reply = r;
        break;
      }
    }
  }
  // Newest first.
  return delegates.reverse();
}

function taskTags(record: TeamRecord): string[] {
  return record.tags
    .filter((t) => t.startsWith("task:"))
    .map((t) => t.slice("task:".length))
    .filter(Boolean);
}

function legacyTaskId(record: TeamRecord): string | undefined {
  const taskTag = record.tags.find((t) => t.startsWith("task-id:"));
  return taskTag ? taskTag.slice("task-id:".length) : undefined;
}

function replyTaskKeys(record: TeamRecord): string[] {
  const keys = taskTags(record).flatMap(taskAliases);
  if (keys.length > 0) return keys;
  return record.tags
    .filter((t) => t.startsWith("reply-to:"))
    .map((t) => t.slice("reply-to:".length))
    .flatMap(taskAliases);
}

function taskAliases(taskKey: string): string[] {
  const aliases = new Set<string>([taskKey]);
  const todo = taskKey.match(/^TODO-\d+/i)?.[0];
  if (todo) aliases.add(todo);
  return [...aliases];
}

function taskLabel(taskKey: string): string {
  return taskKey.match(/^TODO-\d+/i)?.[0] ?? shortId(taskKey);
}

function shortId(s: string): string {
  return s.length > 8 ? s.slice(0, 8) : s;
}

function deltaTs(start: string, end: string): string | null {
  const s = Date.parse(start);
  const e = Date.parse(end);
  if (isNaN(s) || isNaN(e)) return null;
  const secs = Math.max(0, Math.floor((e - s) / 1000));
  if (secs < 60) return `+${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `+${mins}m`;
  const hours = Math.floor(mins / 60);
  return `+${hours}h`;
}
