"use client";

import { useMemo, useState } from "react";
import { UserPlus, Sparkles } from "lucide-react";
import { useStore } from "@/lib/store";
import type { PaneConfig } from "@/lib/store";
import { PaneGrid } from "./PaneGrid";
import { ScratchpadTicker } from "./ScratchpadTicker";
import { DelegationBoard } from "./DelegationBoard";
import { ResourceUseRollup } from "./ResourceUseRollup";
import { UsageStatsRollup } from "./UsageStatsRollup";
import { AddWorkerModal } from "./AddWorkerModal";
import { ProjectGoalBar } from "./ProjectGoalBar";
import { TeamSetupCard } from "./TeamSetupCard";
import { TeamTodoPanel } from "./TeamTodoPanel";
import { TechLeadAutonomyToggles } from "./TechLeadAutonomyToggles";
import { useTeamEnabled } from "@/lib/projectRole";
import { SuggestedWorkersPanel } from "./SuggestedWorkersPanel";

/**
 * Phase 5.1 / v3 split — team overview pseudo-tab.
 *
 * Layout: single column, stacked sections.
 *  - ProjectGoalBar (project goal + canonical team role slots)
 *  - TeamTodoPanel (the TODO queue + agent status + add form)
 *  - Pane grid + scratchpad ticker + delegation board + resource roll-up
 *
 * Team panes each have their own regular tabs in the TabBar,
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

export function buildSuggestWorkersPrompt(paneConfigs: PaneConfig[]): string {
  // Only show managed panes. Side chats are user scratch space, not part of
  // the team the Manager should reason about when proposing additions.
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

  return `Use the current team-mode queue before suggesting more workers.

First read these project files / records:
- project_goal.md for the current goal.
- team-todo.md for proposed, approved, in_progress, under_review, pr_open, and done Global TODOs plus assigned pane subtasks.
- suggested-workers.md for already-proposed worker suggestions and the next SUG-NNN id.
- .apas for the managed pane roster and each pane's role/mode/worktree state.
- Recent .apas-team.jsonl records, especially delegations, reviews, decisions, PR openings, and reviewer/Tech Lead feedback.

Managed team currently visible in the Overview:

${roster}

Suggest 2-3 additional worker panes only if they would help advance the current goal and queue. Avoid duplicates:
- Do not duplicate existing managed panes from .apas or the roster above.
- Do not duplicate existing suggestions already present in suggested-workers.md.
- Do not suggest a worker for proposed, approved, or in_progress Global TODOs in team-todo.md that already have an obvious owner or active worker.

Append each useful suggestion as a section in **suggested-workers.md** (use the Edit/Write tool) — they'll appear in the Overview's "Suggested workers" box with one-click Accept buttons.

Keep this exact schema per suggestion:

## SUG-NNN — short label
- role: developer | qa | reviewer | researcher | devops | ...
- goal: one-sentence scope describing what they'd own
- backstory: 1-2 sentences of relevant context / expertise
- needs_worktree: yes | no   (yes for developers; usually no for reviewers/researchers)

Pick NNN past the existing max (SUG-001 if the file is empty). Quality over quantity. If the current team is sufficient or every obvious gap is already represented by a managed pane, existing suggestion, or Global TODO owner, say so here in chat and skip the file.`;
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
  // Must stay with the other hooks, above any early return — a hook after one
  // is React error #310, which takes down the whole app.
  const teamEnabled = useTeamEnabled();
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
    const prompt = buildSuggestWorkersPrompt(paneConfigs);
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

        {/* Team surfaces only exist when the project's owner/admin has turned
            team mode on. The settings card always renders — it is where the
            toggle lives, and how someone discovers why the team is missing. */}
        {teamEnabled && <TeamSetupCard />}

        <ProjectGoalBar />

        <TechLeadAutonomyToggles />

        {!teamEnabled && (
          <div className="mb-4 rounded-lg border border-gray-200 bg-gray-50 px-4 py-3 text-sm text-gray-600 dark:border-gray-700 dark:bg-gray-800/40 dark:text-gray-400">
            Team mode is off for this project, so the Manager / Tech Lead /
            Developer / Reviewer panes are unavailable. The project owner or an
            admin can turn it on under Project settings above.
          </div>
        )}

        {teamEnabled && <TeamTodoPanel />}

        {teamEnabled && (
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
        )}

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

        <OverviewSection title="Usage stats">
          <UsageStatsRollup />
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
