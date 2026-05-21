"use client";

import { PaneGrid } from "./PaneGrid";

/**
 * Phase 5.1 — team overview pseudo-tab.
 *
 * Sub-leaves complete: 5.1a (scaffold), 5.1b (pane grid).
 * Remaining: 5.1c (scratchpad ticker), 5.1d (delegation board),
 * 5.1e (resource use).
 */
interface OverviewViewProps {
  onOpenPane: (paneId: number) => void;
  onOpenDiff: (paneId: number) => void;
  onOpenRole: (paneId: number) => void;
}

export function OverviewView({ onOpenPane, onOpenDiff, onOpenRole }: OverviewViewProps) {
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
          <PaneGrid
            onOpenPane={onOpenPane}
            onOpenDiff={onOpenDiff}
            onOpenRole={onOpenRole}
          />
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
