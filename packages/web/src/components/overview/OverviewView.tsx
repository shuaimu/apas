"use client";

import { PaneGrid } from "./PaneGrid";
import { ResourceUseRollup } from "./ResourceUseRollup";
import { UsageStatsRollup } from "./UsageStatsRollup";
import { AllowedTabTypesCard } from "./AllowedTabTypesCard";

/**
 * The project overview: what this project may launch, the panes it is running,
 * and what they are costing.
 *
 * It used to be the team overview — a goal bar, a TODO queue, a delegation
 * board, a scratchpad ticker and a managed pane grid, all gated on team mode.
 * That feature is gone, and with it the distinction between managed and
 * unmanaged panes, so there is one grid rather than two.
 */
interface OverviewViewProps {
  onOpenPane: (paneId: number) => void;
  onOpenDiff: (paneId: number) => void;
  onOpenRole: (paneId: number) => void;
  onPausePane: (paneId: number) => void;
  onResumePane: (paneId: number) => void;
  onRemovePane: (paneId: number) => void;
}

export function OverviewView({
  onOpenPane,
  onOpenDiff,
  onOpenRole,
  onPausePane,
  onResumePane,
  onRemovePane,
}: OverviewViewProps) {
  return (
    <div className="h-full overflow-y-auto bg-gray-50 p-6 dark:bg-gray-950">
      <div className="mx-auto max-w-7xl">
        <div className="mb-1 flex flex-wrap items-center justify-between gap-2">
          <h2 className="text-xl font-semibold text-gray-900 dark:text-gray-100">
            Overview
          </h2>
        </div>
        <p className="mb-6 text-sm text-gray-500 dark:text-gray-400">
          What this project may launch, the panes it is running, and what they
          are using.
        </p>

        <AllowedTabTypesCard />

        <OverviewSection title="Panes">
          <PaneGrid
            onOpenPane={onOpenPane}
            onOpenDiff={onOpenDiff}
            onOpenRole={onOpenRole}
            onPausePane={onPausePane}
            onResumePane={onResumePane}
            onRemovePane={onRemovePane}
          />
        </OverviewSection>

        <OverviewSection title="Usage stats">
          <UsageStatsRollup />
        </OverviewSection>

        <OverviewSection title="Resource use">
          <ResourceUseRollup />
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
