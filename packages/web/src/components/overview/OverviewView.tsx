"use client";

import { useMemo, useState } from "react";
import { UserPlus, Sparkles } from "lucide-react";
import { useStore } from "@/lib/store";
import type { PaneConfig } from "@/lib/store";
import { PaneGrid } from "./PaneGrid";
import { ScratchpadTicker } from "./ScratchpadTicker";
import { DelegationBoard } from "./DelegationBoard";
import { ResourceUseRollup } from "./ResourceUseRollup";
import { AddWorkerModal } from "./AddWorkerModal";
import { ProjectGoalBar } from "./ProjectGoalBar";
import { TeamTodoPanel } from "./TeamTodoPanel";

/**
 * Phase 5.1 / v3 split — team overview pseudo-tab.
 *
 * Layout: single column, stacked sections.
 *  - ProjectGoalBar (project goal + Start/Pause for Manager + Tech Lead)
 *  - TeamTodoPanel (the TODO queue + agent status + add form)
 *  - Pane grid + scratchpad ticker + delegation board + resource roll-up
 *
 * The Manager + Tech Lead each have their own regular tabs in the TabBar,
 * so there are no embedded chat / iteration-stream boxes here — would be
 * redundant. Click into the pane to interact directly.
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
  const paneConfigs = useStore((s) => s.paneConfigs);
  const sendMessageToPane = useStore((s) => s.sendMessageToPane);
  const showToast = useStore((s) => s.showToast);
  const managerPane = useMemo(
    () =>
      paneConfigs.find((p) => {
        const lower = (p.role ?? "").toLowerCase();
        return lower.includes("manager") && !lower.includes("tech lead") && p.mode === "interactive";
      }),
    [paneConfigs],
  );

  const handleSuggestWorkers = () => {
    if (!managerPane) {
      showToast("Start the Manager first — they need to be running to suggest workers.", "error");
      return;
    }
    const roster = paneConfigs
      .map((p: PaneConfig) => {
        const lower = (p.role ?? "").toLowerCase();
        const tag = lower.includes("tech lead")
          ? "tech-lead"
          : lower.includes("manager")
            ? "manager"
            : p.role || "no-role";
        return `  - pane_id=${p.pane_id} (${p.label ?? "untitled"}, ${tag})`;
      })
      .join("\n");
    const prompt = `Given the current project goal (in project_goal.md) and the team you already have:\n\n${roster}\n\nSuggest 2-3 additional worker panes that would help advance the goal. For each, give:\n  - a short role label (developer / qa / reviewer / researcher / devops, or your own)\n  - a one-sentence goal/scope describing what they'd own\n  - whether they need an isolated git worktree (yes for developers; usually no for reviewers/researchers)\n\nKeep the suggestions tight — quality over quantity. If the current team is sufficient, say so and explain why.`;
    const result = sendMessageToPane(prompt, managerPane.pane_id);
    if (result.success) {
      showToast("Asked the Manager for worker suggestions — check the Manager tab.", "info");
    } else {
      showToast(result.error ?? "Failed to reach Manager", "error");
    }
  };

  return (
    <div className="flex-1 overflow-auto p-4">
      <div className="mx-auto max-w-7xl">
        <div className="mb-1 flex flex-wrap items-center justify-between gap-2">
          <h2 className="text-xl font-semibold text-gray-900 dark:text-gray-100">
            Team Overview
          </h2>
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              onClick={handleSuggestWorkers}
              disabled={!managerPane}
              className="flex items-center gap-1.5 rounded border border-indigo-400 bg-indigo-50 px-3 py-1.5 text-sm font-medium text-indigo-700 transition-colors hover:bg-indigo-100 disabled:cursor-not-allowed disabled:opacity-50 dark:border-indigo-700 dark:bg-indigo-900/30 dark:text-indigo-300 dark:hover:bg-indigo-900/50"
              title={managerPane ? "Ask the Manager to suggest workers for the current project goal" : "Start the Manager first — needed to generate suggestions"}
            >
              <Sparkles className="h-4 w-4" />
              Suggest workers
            </button>
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
        </div>
        <p className="mb-6 text-sm text-gray-500 dark:text-gray-400">
          Project goal at the top, then the TODO queue, then per-pane
          status. Talk to the Manager or watch the Tech Lead from their
          own tabs.
        </p>

        <ProjectGoalBar />

        <TeamTodoPanel />

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
