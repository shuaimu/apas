"use client";

/**
 * Phase 5.1c — inline view of the last ~20 records from .apas-team.jsonl,
 * filterable by `kind`. Reuses the store's `teamRecords` array (already
 * populated by the CLI watcher + ServerToWeb::TeamRecord). Stays
 * compact so it fits alongside the rest of the Overview.
 */
import { useMemo, useState } from "react";
import { selectActiveTeamRecords, useStore, TeamRecord } from "@/lib/store";

const KIND_FILTERS = ["all", "diff", "review", "decision", "status"] as const;
type KindFilter = (typeof KIND_FILTERS)[number];

const MAX_ROWS = 20;

export function ScratchpadTicker() {
  const teamRecords = useStore(selectActiveTeamRecords);
  const [filter, setFilter] = useState<KindFilter>("all");

  const filtered = useMemo(() => {
    let arr: TeamRecord[] =
      filter === "all"
        ? teamRecords
        : teamRecords.filter((r) => r.kind === filter);
    // Newest first.
    arr = [...arr].reverse();
    return arr.slice(0, MAX_ROWS);
  }, [teamRecords, filter]);

  if (teamRecords.length === 0) {
    return (
      <div className="rounded border border-dashed border-gray-300 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/30 p-4 text-sm italic text-gray-500 dark:text-gray-400">
        No scratchpad records yet. Agents append by writing JSON lines to <code>.apas-team.jsonl</code>.
      </div>
    );
  }

  return (
    <div>
      <div className="mb-2 flex flex-wrap gap-1">
        {KIND_FILTERS.map((k) => {
          const count =
            k === "all" ? teamRecords.length : teamRecords.filter((r) => r.kind === k).length;
          const isActive = filter === k;
          return (
            <button
              key={k}
              type="button"
              onClick={() => setFilter(k)}
              disabled={count === 0 && k !== "all"}
              className={`rounded px-2 py-0.5 text-xs font-medium transition-colors ${
                isActive
                  ? "bg-indigo-600 text-white"
                  : count === 0 && k !== "all"
                    ? "bg-gray-100 dark:bg-gray-800 text-gray-400 dark:text-gray-600 cursor-not-allowed"
                    : "bg-gray-200 dark:bg-gray-700 text-gray-700 dark:text-gray-300 hover:bg-gray-300 dark:hover:bg-gray-600"
              }`}
            >
              {k} {count > 0 && <span className="opacity-70">({count})</span>}
            </button>
          );
        })}
      </div>
      <ul className="flex flex-col gap-1.5">
        {filtered.map((r, i) => (
          <li
            key={`${r.ts}-${i}`}
            className="rounded border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800/40 px-3 py-1.5"
          >
            <div className="mb-1 flex flex-wrap items-center gap-2 text-[11px] text-gray-500 dark:text-gray-400">
              <span className="font-mono font-semibold text-gray-700 dark:text-gray-300">{r.kind}</span>
              <span>·</span>
              <span title={r.ts}>{relativeTs(r.ts)}</span>
              {r.pane_id !== undefined && (
                <>
                  <span>·</span>
                  <span>pane {r.pane_id}</span>
                </>
              )}
              {r.tags.length > 0 && (
                <>
                  <span>·</span>
                  {r.tags.slice(0, 3).map((t) => (
                    <span
                      key={t}
                      className="rounded bg-gray-100 dark:bg-gray-700 px-1.5 py-0.5 font-mono text-[10px] text-gray-600 dark:text-gray-300"
                    >
                      {t}
                    </span>
                  ))}
                  {r.tags.length > 3 && (
                    <span className="text-[10px]">+{r.tags.length - 3} more</span>
                  )}
                </>
              )}
            </div>
            <div className="whitespace-pre-wrap break-words font-mono text-xs text-gray-800 dark:text-gray-200 line-clamp-3">
              {r.body}
            </div>
          </li>
        ))}
      </ul>
      {teamRecords.length > MAX_ROWS && (
        <p className="mt-2 text-[11px] text-gray-500 dark:text-gray-400 italic">
          Showing newest {MAX_ROWS} of {teamRecords.length}.
        </p>
      )}
    </div>
  );
}

function relativeTs(iso: string): string {
  const parsed = Date.parse(iso);
  if (isNaN(parsed)) return iso;
  const seconds = Math.floor((Date.now() - parsed) / 1000);
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}
