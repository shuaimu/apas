"use client";

import { PaneGrid } from "./PaneGrid";
import { ResourceUseRollup } from "./ResourceUseRollup";
import { UsageStatsRollup } from "./UsageStatsRollup";
import { AllowedTabTypesCard } from "./AllowedTabTypesCard";
import { OverviewSection } from "./OverviewElements";

/**
 * The interactive half of a project's Overview, in the order the project is
 * used: its tabs, what they have used, how close each provider is to its
 * limits, and what a new tab may launch. The workspace puts the project's
 * name above these sections and its autonomous workers below them.
 */
interface OverviewViewProps {
  onOpenPane: (paneId: number) => void;
  onOpenDiff: (paneId: number) => void;
  onOpenRole: (paneId: number) => void;
  onRemovePane: (paneId: number) => void;
}

export function OverviewView({
  onOpenPane,
  onOpenDiff,
  onOpenRole,
  onRemovePane,
}: OverviewViewProps) {
  return (
    <section aria-label="Interactive project" className="flex min-w-0 flex-col gap-8">
      {/* Names the group for a screen reader's outline. On screen its four
          sections stand beside "Autonomous workers" as equals, so the page
          has one level of section heading. */}
      <h2 className="sr-only">Interactive project</h2>

      <OverviewSection title="Tabs" level={3}>
        <PaneGrid
          onOpenPane={onOpenPane}
          onOpenDiff={onOpenDiff}
          onOpenRole={onOpenRole}
          onRemovePane={onRemovePane}
        />
      </OverviewSection>

      <UsageStatsRollup />

      <OverviewSection
        title="Resource use"
        level={3}
        description="How much of each provider's usage limit is used, and when it resets."
      >
        <ResourceUseRollup />
      </OverviewSection>

      <AllowedTabTypesCard />
    </section>
  );
}
