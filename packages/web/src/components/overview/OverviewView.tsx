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
import { TeamSetupCard } from "./TeamSetupCard";
import { TeamTodoPanel } from "./TeamTodoPanel";
import { TechLeadAutonomyToggles } from "./TechLeadAutonomyToggles";
import { SuggestedWorkersPanel } from "./SuggestedWorkersPanel";

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
    // Only show managed panes — side chats are user scratch space, not
    // part of the team the Manager should reason about when proposing
    // additions. Empty roster (everything's unmanaged) is fine; the
    // Manager will just propose from scratch.
    const roster = paneConfigs
      .filter((p: PaneConfig) => p.managed === true)
      .map((p: PaneConfig) => {
        const lower = (p.role ?? "").toLowerCase();
        const tag = lower.includes("tech lead")
          ? "tech-lead"
          : lower.includes("manager")
            ? "manager"
            : p.role || "no-role";
        return `  - pane_id=${p.pane_id} (${p.label ?? "untitled"}, ${tag})`;
      })
      .join("\n") || "  (no managed team members yet)";
    const prompt = `Given the current project goal (in project_goal.md) and the team you already have:\n\n${roster}\n\nSuggest 2-3 additional worker panes that would help advance the goal. Append each suggestion as a section in **suggested-workers.md** (use the Edit/Write tool) — they'll appear in the Overview's "Suggested workers" box with one-click Accept buttons.\n\nFormat per suggestion:\n\n## SUG-NNN — short label\n- role: developer | qa | reviewer | researcher | devops | ...\n- goal: one-sentence scope describing what they'd own\n- backstory: 1-2 sentences of relevant context / expertise\n- needs_worktree: yes | no   (yes for developers; usually no for reviewers/researchers)\n\nPick NNN past the existing max (SUG-001 if the file is empty). Quality over quantity. If the current team is sufficient, say so here in chat and skip the file.`;
    const result = sendMessageToPane(prompt, managerPane.pane_id);
    if (result.success) {
      showToast("Asked the Manager for suggestions — they'll appear in the Suggested workers box below.", "info");
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
        </div>
        <p className="mb-6 text-sm text-gray-500 dark:text-gray-400">
          Project goal at the top, then the TODO queue, then per-pane
          status. Talk to the Manager or watch the Tech Lead from their
          own tabs.
        </p>

        <TeamSetupCard />

        <ProjectGoalBar />

        <TechLeadAutonomyToggles />

        <TeamTodoPanel />

        <OverviewSection title="Team (managed)">
          <PaneGrid
            kind="managed"
            onOpenPane={onOpenPane}
            onOpenDiff={onOpenDiff}
            onOpenRole={onOpenRole}
            onPausePane={onPausePane}
            onResumePane={onResumePane}
            onRemovePane={onRemovePane}
          />
        </OverviewSection>

        <div className="mb-4 flex flex-wrap justify-end gap-2">
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

        <OverviewSection title="Suggested workers">
          <SuggestedWorkersPanel />
        </OverviewSection>

        <OverviewSection title="Side chats (unmanaged)">
          <PaneGrid
            kind="unmanaged"
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
