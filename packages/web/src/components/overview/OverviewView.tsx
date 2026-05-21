"use client";

/**
 * Phase 5.1 — team overview pseudo-tab.
 *
 * Sub-leaf 5.1a: scaffold only. Empty shell with section placeholders;
 * 5.1b–5.1e fill them in.
 */
export function OverviewView() {
  return (
    <div className="flex-1 overflow-auto p-4">
      <div className="mx-auto max-w-5xl">
        <h2 className="mb-1 text-xl font-semibold text-gray-900 dark:text-gray-100">
          Team Overview
        </h2>
        <p className="mb-6 text-sm text-gray-500 dark:text-gray-400">
          Roll-up of every pane in this project. Use the regular tabs above for the per-pane chat / timeline / role views.
        </p>

        <OverviewSection title="Pane grid">
          <Placeholder note="5.1b — cards per pane (status pill, mode, worktree, last activity, quick actions)" />
        </OverviewSection>

        <OverviewSection title="Team scratchpad">
          <Placeholder note="5.1c — recent .apas-team.jsonl records + filter chips" />
        </OverviewSection>

        <OverviewSection title="Delegation board">
          <Placeholder note="5.1d — delegate-to / reply-to pairing" />
        </OverviewSection>

        <OverviewSection title="Resource use">
          <Placeholder note="5.1e — per-provider usage limits rollup" />
        </OverviewSection>
      </div>
    </div>
  );
}

function OverviewSection({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="mb-6">
      <h3 className="mb-2 text-sm font-semibold uppercase tracking-wide text-gray-500 dark:text-gray-400">
        {title}
      </h3>
      {children}
    </section>
  );
}

function Placeholder({ note }: { note: string }) {
  return (
    <div className="rounded border border-dashed border-gray-300 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/30 p-4 text-xs italic text-gray-500 dark:text-gray-400">
      {note}
    </div>
  );
}
