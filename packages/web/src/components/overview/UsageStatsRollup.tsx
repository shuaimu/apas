"use client";

/**
 * Per-project / per-pane usage stats for the Overview: prompt + token counts,
 * cache usage, completed responses, and real cost (Claude only). Fed by the
 * server's ServerToWeb::ProjectUsageStats push (keyed by session_id), which is
 * also replayed on attach. A small window toggle switches between cumulative
 * lifetime totals and the rolling 7-day / today windows.
 */
import { useMemo, useState } from "react";
import { useStore, type UsageCounters, type PaneConfig } from "@/lib/store";

type Window = "lifetime" | "last_7d" | "today";

const WINDOW_LABELS: Record<Window, string> = {
  lifetime: "All time",
  last_7d: "Last 7 days",
  today: "Today",
};

function formatInt(n: number): string {
  return n.toLocaleString();
}

// Compact token formatting: 1.2k / 3.4M so wide token columns stay readable.
// The M threshold is 999_950 (not 1_000_000) so values that would round up to
// "1000.0k" roll into "1.0M" instead.
function formatTokens(n: number): string {
  if (n >= 999_950) return `${(n / 1_000_000).toFixed(n % 1_000_000 === 0 ? 0 : 1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(n % 1_000 === 0 ? 0 : 1)}k`;
  return String(n);
}

// Short relative time for the per-pane "Last active" column. last_active is
// RFC3339 UTC from the server; unparseable/absent values render as a dash.
function formatRelative(iso?: string): string {
  if (!iso) return "—";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "—";
  const diffMs = Date.now() - t;
  if (diffMs < 60_000) return "just now";
  const mins = Math.floor(diffMs / 60_000);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

// Cost is real only for Claude transport; render a dash (not "$0.0000") when
// a pane/provider reports no cost so empty cells aren't mistaken for $0 spend.
function formatCost(n: number): string {
  if (!(n > 0)) return "—";
  return `$${n < 1 ? n.toFixed(4) : n.toFixed(2)}`;
}

function totalTokens(c: UsageCounters): number {
  return c.input_tokens + c.output_tokens + c.cache_read_tokens + c.cache_creation_tokens;
}

function paneLabelFor(
  paneId: number,
  configs: Map<number, PaneConfig>,
): { name: string; provider?: string } {
  if (paneId === 0) return { name: "Unattributed" };
  const cfg = configs.get(paneId);
  if (!cfg) return { name: `Pane ${paneId}` };
  return { name: cfg.label || cfg.role || `Pane ${paneId}`, provider: cfg.provider };
}

function StatCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800/40 px-3 py-2">
      <div className="text-[10px] uppercase tracking-wide text-gray-500 dark:text-gray-400">
        {label}
      </div>
      <div className="mt-0.5 text-base font-semibold tabular-nums text-gray-900 dark:text-gray-100">
        {value}
      </div>
    </div>
  );
}

export function UsageStatsRollup() {
  const sessionId = useStore((s) => s.sessionId);
  const usageStats = useStore((s) => s.usageStats);
  const paneConfigs = useStore((s) => s.paneConfigs);
  const [window, setWindow] = useState<Window>("lifetime");

  const stats = sessionId ? usageStats[sessionId] : undefined;

  const paneConfigById = useMemo(() => {
    const m = new Map<number, PaneConfig>();
    for (const p of paneConfigs) m.set(p.pane_id, p);
    return m;
  }, [paneConfigs]);

  const rows = useMemo(() => {
    if (!stats) return [];
    // Sort by total tokens in the selected window (busiest pane first), then
    // by pane id for stable ordering.
    return [...stats.panes].sort(
      (a, b) => totalTokens(b[window]) - totalTokens(a[window]) || a.pane_id - b.pane_id,
    );
  }, [stats, window]);

  if (!stats || stats.panes.length === 0) {
    return (
      <div className="rounded border border-dashed border-gray-300 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/30 p-4 text-sm italic text-gray-500 dark:text-gray-400">
        No usage recorded yet. Stats accumulate as panes send prompts and complete turns.
      </div>
    );
  }

  const totals = stats[window];

  return (
    <div>
      {/* Window toggle */}
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <div className="inline-flex rounded-md border border-gray-200 dark:border-gray-700 p-0.5">
          {(Object.keys(WINDOW_LABELS) as Window[]).map((w) => (
            <button
              key={w}
              type="button"
              onClick={() => setWindow(w)}
              className={`rounded px-2.5 py-1 text-xs font-medium transition-colors ${
                window === w
                  ? "bg-blue-100 text-blue-700 dark:bg-blue-900/40 dark:text-blue-300"
                  : "text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200"
              }`}
            >
              {WINDOW_LABELS[w]}
            </button>
          ))}
        </div>
        {window !== "lifetime" && (
          <span className="text-[10px] uppercase tracking-wide text-gray-400">UTC days</span>
        )}
      </div>

      {/* Project totals */}
      <div className="mb-3 grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-6">
        <StatCard label="Prompts" value={formatInt(totals.prompts)} />
        <StatCard label="Responses" value={formatInt(totals.responses)} />
        <StatCard label="Input tokens" value={formatTokens(totals.input_tokens)} />
        <StatCard label="Output tokens" value={formatTokens(totals.output_tokens)} />
        <StatCard label="Total tokens" value={formatTokens(totalTokens(totals))} />
        <StatCard label="Cost" value={formatCost(totals.cost_usd)} />
      </div>

      {/* Per-pane breakdown */}
      <div className="overflow-x-auto rounded border border-gray-200 dark:border-gray-700">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-gray-200 bg-gray-50 text-left text-[11px] uppercase tracking-wide text-gray-500 dark:border-gray-700 dark:bg-gray-800/50 dark:text-gray-400">
              <th className="px-3 py-2 font-medium">Pane</th>
              <th className="px-2 py-2 text-right font-medium">Prompts</th>
              <th className="px-2 py-2 text-right font-medium">Responses</th>
              <th className="px-2 py-2 text-right font-medium">Input</th>
              <th className="px-2 py-2 text-right font-medium">Output</th>
              <th className="px-2 py-2 text-right font-medium">Cache</th>
              <th className="px-2 py-2 text-right font-medium">Total</th>
              <th className="px-2 py-2 text-right font-medium">Cost</th>
              <th className="px-3 py-2 text-right font-medium">Active</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((pane) => {
              const c = pane[window];
              const { name, provider } = paneLabelFor(pane.pane_id, paneConfigById);
              const cache = c.cache_read_tokens + c.cache_creation_tokens;
              return (
                <tr
                  key={pane.pane_id}
                  className="border-b border-gray-100 last:border-0 dark:border-gray-800"
                >
                  <td className="px-3 py-2">
                    <span className="font-medium text-gray-900 dark:text-gray-100">{name}</span>
                    {provider && (
                      <span className="ml-1.5 text-[10px] uppercase tracking-wide text-gray-400">
                        {provider}
                      </span>
                    )}
                  </td>
                  <td className="px-2 py-2 text-right tabular-nums">{formatInt(c.prompts)}</td>
                  <td className="px-2 py-2 text-right tabular-nums">{formatInt(c.responses)}</td>
                  <td className="px-2 py-2 text-right tabular-nums">{formatTokens(c.input_tokens)}</td>
                  <td className="px-2 py-2 text-right tabular-nums">{formatTokens(c.output_tokens)}</td>
                  <td className="px-2 py-2 text-right tabular-nums">{formatTokens(cache)}</td>
                  <td className="px-2 py-2 text-right font-medium tabular-nums">
                    {formatTokens(totalTokens(c))}
                  </td>
                  <td className="px-2 py-2 text-right tabular-nums">{formatCost(c.cost_usd)}</td>
                  <td
                    className="px-3 py-2 text-right text-xs text-gray-500 dark:text-gray-400"
                    title={pane.last_active ?? undefined}
                  >
                    {formatRelative(pane.last_active)}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
