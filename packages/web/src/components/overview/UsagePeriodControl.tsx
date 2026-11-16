"use client";

/**
 * The period switch and the stat card of the Overview's Usage section, shared
 * with the cluster page so both read the same: "All time", "Last 7 days" and
 * "Today", the two windows counted in UTC days.
 */
import { overviewCard, overviewMuted } from "./OverviewElements";

export type UsagePeriod = "lifetime" | "last_7d" | "today";

export const USAGE_PERIOD_LABELS: Record<UsagePeriod, string> = {
  lifetime: "All time",
  last_7d: "Last 7 days",
  today: "Today",
};

export function UsagePeriodControl({ value, onChange }: { value: UsagePeriod; onChange: (value: UsagePeriod) => void }) {
  return (
    <>
      {value !== "lifetime" && <span className={overviewMuted}>UTC days</span>}
      <div
        role="group"
        aria-label="Usage period"
        className="inline-flex h-8 items-center rounded border border-gray-300 bg-white p-0.5 dark:border-gray-600 dark:bg-gray-800"
      >
        {(Object.keys(USAGE_PERIOD_LABELS) as UsagePeriod[]).map((w) => (
          <button
            key={w}
            type="button"
            aria-pressed={value === w}
            onClick={() => onChange(w)}
            className={`h-full rounded-sm px-2.5 text-xs font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-500 ${
              value === w
                ? "bg-blue-100 text-blue-700 dark:bg-blue-900/50 dark:text-blue-200"
                : "text-gray-600 hover:text-gray-900 dark:text-gray-300 dark:hover:text-gray-100"
            }`}
          >
            {USAGE_PERIOD_LABELS[w]}
          </button>
        ))}
      </div>
    </>
  );
}

export function StatCard({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <div className={`${overviewCard} px-3 py-2`}>
      <dt className={overviewMuted}>{label}</dt>
      <dd className="mt-0.5 text-lg font-semibold tabular-nums text-gray-900 dark:text-gray-100" title={title}>
        {value}
      </dd>
    </div>
  );
}
