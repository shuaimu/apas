"use client";

/**
 * The Overview's "Usage" section: prompt and token counts, cache usage,
 * completed responses and reported cost, for the project and for each tab.
 * Fed by the server's ServerToWeb::ProjectUsageStats push (keyed by
 * session_id), which is also replayed on attach. A small switch chooses
 * between lifetime totals and the rolling 7-day and today windows.
 *
 * The table lists the project's open tabs first. Tabs that were closed, which
 * after a few months are most of the rows, are folded into one line with
 * their sum; the totals above always count every tab.
 */
import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { useStore, type PaneConfig, type PaneUsageStats, type UsageCounters } from "@/lib/store";
import { formatCompactCount, formatUsd } from "@/lib/formatCount";
import { formatRelativeTime } from "@/lib/formatRelativeTime";
import { providerDisplayName } from "@/lib/launchProfileLabels";
import { OverviewEmpty, OverviewSection, overviewCard, overviewMuted } from "./OverviewElements";
import { StatCard, UsagePeriodControl, type UsagePeriod as Window } from "./UsagePeriodControl";

function formatInt(n: number): string {
  return n.toLocaleString();
}

// The "Last active" column. last_active is RFC3339 UTC from the server; an
// absent or unparseable value is a dash. Same wording as the tab's card.
function formatLastActive(iso?: string): string {
  return iso ? formatRelativeTime(Date.parse(iso)) : "—";
}

function totalTokens(c: UsageCounters): number {
  return c.input_tokens + c.output_tokens + c.cache_read_tokens + c.cache_creation_tokens;
}

function sumCounters(rows: UsageCounters[]): UsageCounters {
  const sum: UsageCounters = {
    prompts: 0,
    responses: 0,
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    cache_creation_tokens: 0,
    cost_usd: 0,
  };
  for (const row of rows) {
    sum.prompts += row.prompts;
    sum.responses += row.responses;
    sum.input_tokens += row.input_tokens;
    sum.output_tokens += row.output_tokens;
    sum.cache_read_tokens += row.cache_read_tokens;
    sum.cache_creation_tokens += row.cache_creation_tokens;
    sum.cost_usd += row.cost_usd;
  }
  return sum;
}

function lastActiveMs(pane: PaneUsageStats): number {
  const t = pane.last_active ? Date.parse(pane.last_active) : Number.NaN;
  return Number.isNaN(t) ? Number.NEGATIVE_INFINITY : t;
}

const NUMBER_CELL = "px-2 py-2 text-right tabular-nums";

/** The eight figures of one row, in the table's column order. */
function CounterCells({ counters, lastActive }: { counters: UsageCounters; lastActive?: string }) {
  const cache = counters.cache_read_tokens + counters.cache_creation_tokens;
  return (
    <>
      <td className={NUMBER_CELL}>{formatInt(counters.prompts)}</td>
      <td className={NUMBER_CELL}>{formatInt(counters.responses)}</td>
      <td className={NUMBER_CELL}>{formatCompactCount(counters.input_tokens)}</td>
      <td className={NUMBER_CELL}>{formatCompactCount(counters.output_tokens)}</td>
      <td className={NUMBER_CELL}>{formatCompactCount(cache)}</td>
      <td className={`${NUMBER_CELL} font-medium`}>{formatCompactCount(totalTokens(counters))}</td>
      <td className={NUMBER_CELL}>{formatUsd(counters.cost_usd)}</td>
      <td className={`whitespace-nowrap px-3 py-2 text-right ${overviewMuted}`} title={lastActive}>
        {formatLastActive(lastActive)}
      </td>
    </>
  );
}

export function UsageStatsRollup() {
  const sessionId = useStore((s) => s.sessionId);
  const usageStats = useStore((s) => s.usageStats);
  const paneConfigs = useStore((s) => s.paneConfigs);
  const [window, setWindow] = useState<Window>("lifetime");
  const [showClosed, setShowClosed] = useState(false);

  const stats = sessionId ? usageStats[sessionId] : undefined;

  const paneConfigById = useMemo(() => {
    const m = new Map<number, PaneConfig>();
    for (const p of paneConfigs) m.set(p.pane_id, p);
    return m;
  }, [paneConfigs]);

  // Open tabs, then the usage no tab accounts for, then closed tabs. Within
  // each, the most recently active first; a tie goes to the heavier user of
  // the selected window, then to the lower id so the order is stable.
  const { listed, closed } = useMemo(() => {
    const byActivity = (a: PaneUsageStats, b: PaneUsageStats) =>
      lastActiveMs(b) - lastActiveMs(a)
      || totalTokens(b[window]) - totalTokens(a[window])
      || a.pane_id - b.pane_id;
    const panes = stats?.panes ?? [];
    return {
      listed: [
        ...panes.filter((pane) => paneConfigById.has(pane.pane_id)).sort(byActivity),
        ...panes.filter((pane) => pane.pane_id === 0),
      ],
      closed: panes.filter((pane) => pane.pane_id !== 0 && !paneConfigById.has(pane.pane_id)).sort(byActivity),
    };
  }, [stats, window, paneConfigById]);

  if (!stats || stats.panes.length === 0) {
    return (
      <OverviewSection title="Usage" level={3}>
        <OverviewEmpty>
          No usage recorded yet. Stats accumulate as tabs send prompts and complete turns.
        </OverviewEmpty>
      </OverviewSection>
    );
  }

  const totals = stats[window];
  // With nothing else to list, the closed tabs are the table.
  const closedShown = showClosed || listed.length === 0;
  const closedLabel = `${closed.length} closed ${closed.length === 1 ? "tab" : "tabs"}`;
  const closedLastActive = closed[0]?.last_active;

  return (
    <OverviewSection
      title="Usage"
      level={3}
      aside={<UsagePeriodControl value={window} onChange={setWindow} />}
    >
      {/* Project totals: every tab, open or closed. */}
      <dl className="mb-3 grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-6">
        <StatCard label="Prompts" value={formatInt(totals.prompts)} />
        <StatCard label="Responses" value={formatInt(totals.responses)} />
        <StatCard label="Input tokens" value={formatCompactCount(totals.input_tokens)} />
        <StatCard label="Output tokens" value={formatCompactCount(totals.output_tokens)} />
        <StatCard label="Total tokens" value={formatCompactCount(totalTokens(totals))} />
        <StatCard label="Cost" value={formatUsd(totals.cost_usd)} />
      </dl>

      {/* Per-tab breakdown */}
      <div className={`${overviewCard} overflow-x-auto`}>
        <table className="w-full whitespace-nowrap text-sm text-gray-900 dark:text-gray-100">
          <thead>
            <tr className="border-b border-gray-200 text-left text-xs text-gray-500 dark:border-gray-700 dark:text-gray-400">
              <th scope="col" className="px-3 py-2 font-medium">Tab</th>
              <th scope="col" className="px-2 py-2 text-right font-medium">Prompts</th>
              <th scope="col" className="px-2 py-2 text-right font-medium">Responses</th>
              <th scope="col" className="px-2 py-2 text-right font-medium">Input</th>
              <th scope="col" className="px-2 py-2 text-right font-medium">Output</th>
              <th scope="col" className="px-2 py-2 text-right font-medium">Cache</th>
              <th scope="col" className="px-2 py-2 text-right font-medium">Total</th>
              <th scope="col" className="px-2 py-2 text-right font-medium">Cost</th>
              <th scope="col" className="px-3 py-2 text-right font-medium">Last active</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-gray-100 dark:divide-gray-800">
            {listed.map((pane) => {
              const config = paneConfigById.get(pane.pane_id);
              return (
                <tr key={pane.pane_id}>
                  <th scope="row" className="max-w-56 px-3 py-2 text-left font-medium">
                    <span className="flex items-baseline gap-1.5">
                      <span className="truncate">
                        {config ? config.label || config.role || `Tab ${pane.pane_id}` : "Unattributed"}
                      </span>
                      {config && (
                        <span className={`shrink-0 font-normal ${overviewMuted}`}>
                          {providerDisplayName(config.provider)}
                        </span>
                      )}
                    </span>
                  </th>
                  <CounterCells counters={pane[window]} lastActive={pane.last_active} />
                </tr>
              );
            })}
            {closed.length > 0 && (
              <tr className="bg-gray-50 text-gray-600 dark:bg-gray-800/60 dark:text-gray-300">
                <th scope="row" className="px-3 py-1 text-left font-normal">
                  {listed.length === 0 ? closedLabel : (
                    <button
                      type="button"
                      aria-expanded={closedShown}
                      onClick={() => setShowClosed((shown) => !shown)}
                      className="-ml-1.5 inline-flex h-7 items-center gap-1 rounded px-1.5 text-sm font-medium text-gray-700 hover:bg-gray-200/70 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-500 dark:text-gray-200 dark:hover:bg-gray-700"
                    >
                      {closedShown
                        ? <ChevronDown className="h-3.5 w-3.5" aria-hidden="true" />
                        : <ChevronRight className="h-3.5 w-3.5" aria-hidden="true" />}
                      {closedShown ? "Hide" : "Show"} {closedLabel}
                    </button>
                  )}
                </th>
                <CounterCells
                  counters={sumCounters(closed.map((pane) => pane[window]))}
                  lastActive={closedLastActive}
                />
              </tr>
            )}
            {closedShown && closed.map((pane) => (
              <tr key={pane.pane_id} className="text-gray-600 dark:text-gray-300">
                <th scope="row" className="px-3 py-2 pl-8 text-left font-normal">Tab {pane.pane_id}</th>
                <CounterCells counters={pane[window]} lastActive={pane.last_active} />
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </OverviewSection>
  );
}
