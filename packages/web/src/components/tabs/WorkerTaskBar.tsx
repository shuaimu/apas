"use client";

/**
 * Small one-line bar that sits at the top of a worker pane (or the
 * Reviewer pane) showing the task that pane is currently working on —
 * derived from the most recent `delegate-to:<this_pane_id>` record on
 * `.apas-team.jsonl`. Helps the user click into a pane and immediately
 * see what it's doing without reading the message log.
 *
 * Skipped for Manager / Tech Lead panes — those don't receive task
 * delegations in the per-TODO sense.
 */
import { useEffect, useMemo, useState } from "react";
import { ClipboardList } from "lucide-react";
import { useStore } from "@/lib/store";

interface WorkerTaskBarProps {
  paneId: number;
  role?: string;
}

export function WorkerTaskBar({ paneId, role }: WorkerTaskBarProps) {
  const teamRecords = useStore((s) => s.teamRecords);
  const [now, setNow] = useState(() => Date.now());

  // Re-render every minute so the "Xm ago" stays accurate without
  // burning a render per second on the active tab.
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(id);
  }, []);

  // Skip orchestrator roles — they receive different shapes of
  // delegations and "current task" isn't a meaningful framing for them.
  const lower = (role ?? "").toLowerCase();
  const skip =
    lower.includes("manager") && !lower.includes("tech lead")
      ? true
      : lower.includes("tech lead");
  const current = useMemo(() => {
    if (skip) return null;
    const tag = `delegate-to:${paneId}`;
    // Walk backward so the latest delegation wins.
    for (let i = teamRecords.length - 1; i >= 0; i--) {
      const r = teamRecords[i];
      if (!r.tags.includes(tag)) continue;
      const taskTag = r.tags.find((t) => t.startsWith("task:"));
      if (!taskTag) continue;
      return {
        taskId: taskTag.slice("task:".length),
        receivedAt: r.ts,
        body: r.body,
      };
    }
    return null;
  }, [skip, paneId, teamRecords]);

  if (skip) return null;
  if (!current) {
    return (
      <div className="border-b border-gray-200 px-3 py-1.5 text-[11px] text-gray-400 dark:border-gray-700 dark:text-gray-500">
        <ClipboardList className="mr-1 inline h-3 w-3" />
        No active task — waiting for the Tech Lead to delegate.
      </div>
    );
  }
  return (
    <div
      className="border-b border-gray-200 px-3 py-1.5 text-xs dark:border-gray-700"
      title={current.body}
    >
      <ClipboardList className="mr-1 inline h-3 w-3 text-blue-600 dark:text-blue-400" />
      <span className="text-gray-500 dark:text-gray-400">Current task:</span>{" "}
      <code className="font-mono text-gray-900 dark:text-gray-100">
        {current.taskId}
      </code>
      <span className="ml-2 text-gray-400 dark:text-gray-500">
        · received {relative(parseTs(current.receivedAt), now)}
      </span>
    </div>
  );
}

function parseTs(s: string): number | null {
  const n = Date.parse(s);
  return Number.isFinite(n) ? n : null;
}

function relative(ts: number | null, now: number): string {
  if (ts == null) return "—";
  const ageMs = Math.max(0, now - ts);
  const s = Math.round(ageMs / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.round(h / 24)}d ago`;
}
