"use client";

import { useState } from "react";
import { UserPlus } from "lucide-react";
import { PaneGrid } from "./PaneGrid";
import { ScratchpadTicker } from "./ScratchpadTicker";
import { DelegationBoard } from "./DelegationBoard";
import { ResourceUseRollup } from "./ResourceUseRollup";
import { AddWorkerModal } from "./AddWorkerModal";
import { ProjectGoalBar } from "./ProjectGoalBar";

/**
 * Phase 5.1 — team overview pseudo-tab.
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
  const [addWorkerOpen, setAddWorkerOpen] = useState(false);

  return (
    <div className="flex-1 overflow-auto p-4">
      <div className="mx-auto max-w-5xl">
        <div className="mb-1 flex flex-wrap items-center justify-between gap-2">
          <h2 className="text-xl font-semibold text-gray-900 dark:text-gray-100">
            Team Overview
          </h2>
          <button
            type="button"
            onClick={() => setAddWorkerOpen(true)}
            className="flex items-center gap-1.5 rounded border border-emerald-400 bg-emerald-50 px-3 py-1.5 text-sm font-medium text-emerald-700 transition-colors hover:bg-emerald-100 dark:border-emerald-700 dark:bg-emerald-900/30 dark:text-emerald-300 dark:hover:bg-emerald-900/50"
            title="Add a new worker pane — pick a template, provider, and worktree in one modal"
          >
            <UserPlus className="h-4 w-4" />
            Add worker
          </button>
        </div>
        <p className="mb-6 text-sm text-gray-500 dark:text-gray-400">
          Roll-up of every pane in this project. Use the regular tabs above for the per-pane chat / timeline / role views.
        </p>

        <ProjectGoalBar />

        <OverviewSection title="Pane grid">
          <PaneGrid
            onOpenPane={onOpenPane}
            onOpenDiff={onOpenDiff}
            onOpenRole={onOpenRole}
            onPausePane={onPausePane}
            onResumePane={onResumePane}
            onRemovePane={onRemovePane}
          />
        </OverviewSection>

        <OverviewSection title="Team scratchpad">
          <ScratchpadTicker />
        </OverviewSection>

        <OverviewSection title="Delegation board">
          <DelegationBoard />
        </OverviewSection>

        <OverviewSection title="Resource use">
          <ResourceUseRollup />
        </OverviewSection>
      </div>
      <AddWorkerModal open={addWorkerOpen} onClose={() => setAddWorkerOpen(false)} />
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

