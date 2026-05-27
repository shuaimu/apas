"use client";

import { useMemo, useState } from "react";
import { UserPlus, LayoutDashboard, MessageSquare } from "lucide-react";
import { useStore } from "@/lib/store";
import { PaneGrid } from "./PaneGrid";
import { ScratchpadTicker } from "./ScratchpadTicker";
import { DelegationBoard } from "./DelegationBoard";
import { ResourceUseRollup } from "./ResourceUseRollup";
import { AddWorkerModal } from "./AddWorkerModal";
import { ProjectGoalBar } from "./ProjectGoalBar";
import { ManagerChatPanel } from "./ManagerChatPanel";
import { TechLeadStream } from "./TechLeadStream";

/**
 * Phase 5.1 / v3 split — team overview pseudo-tab.
 *
 * Layout:
 *  - Top: ProjectGoalBar (project goal + Start/Pause for Manager + Tech Lead).
 *  - Bottom: 2-column split on lg+ screens, stacked on mobile.
 *      Left:  tab toggle [Status | Chat with Manager].
 *      Right: TechLeadStream — embedded view of the Tech Lead pane's
 *             iterations (placeholder when no Tech Lead is running).
 */
interface OverviewViewProps {
  onOpenPane: (paneId: number) => void;
  onOpenDiff: (paneId: number) => void;
  onOpenRole: (paneId: number) => void;
  onPausePane: (paneId: number) => void;
  onResumePane: (paneId: number) => void;
  onRemovePane: (paneId: number) => void;
}

type LeftTab = "status" | "chat";

export function OverviewView({
  onOpenPane,
  onOpenDiff,
  onOpenRole,
  onPausePane,
  onResumePane,
  onRemovePane,
}: OverviewViewProps) {
  const [addWorkerOpen, setAddWorkerOpen] = useState(false);
  const [leftTab, setLeftTab] = useState<LeftTab>("chat");
  const paneConfigs = useStore((s) => s.paneConfigs);
  const techLeadPane = useMemo(
    () =>
      paneConfigs.find((p) => {
        const lower = (p.role ?? "").toLowerCase();
        return lower.includes("tech lead") && p.mode === "deadloop";
      }),
    [paneConfigs],
  );

  return (
    <div className="flex-1 overflow-auto p-4">
      <div className="mx-auto max-w-7xl">
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
          Project goal sits at the top. Below: switch the left side between
          team status and the chat with your Manager; the right side mirrors
          the Tech Lead pane&apos;s iteration stream when one is running.
        </p>

        <ProjectGoalBar />

        <div className="flex flex-col gap-4 lg:flex-row">
          {/* Left column: tab toggle */}
          <div className="flex min-w-0 flex-1 flex-col">
            <div className="mb-3 inline-flex self-start rounded border border-gray-200 bg-gray-50 p-0.5 dark:border-gray-700 dark:bg-gray-800">
              <button
                type="button"
                onClick={() => setLeftTab("status")}
                className={`flex items-center gap-1.5 rounded px-2.5 py-1 text-xs font-medium transition-colors ${
                  leftTab === "status"
                    ? "bg-white text-gray-900 shadow-sm dark:bg-gray-700 dark:text-gray-100"
                    : "text-gray-600 hover:text-gray-900 dark:text-gray-400 dark:hover:text-gray-100"
                }`}
              >
                <LayoutDashboard className="h-3.5 w-3.5" /> Status
              </button>
              <button
                type="button"
                onClick={() => setLeftTab("chat")}
                className={`flex items-center gap-1.5 rounded px-2.5 py-1 text-xs font-medium transition-colors ${
                  leftTab === "chat"
                    ? "bg-white text-gray-900 shadow-sm dark:bg-gray-700 dark:text-gray-100"
                    : "text-gray-600 hover:text-gray-900 dark:text-gray-400 dark:hover:text-gray-100"
                }`}
                title="Chat with the Manager (the interactive user-facing pane)"
              >
                <MessageSquare className="h-3.5 w-3.5" /> Manager
              </button>
            </div>

            {leftTab === "status" ? (
              <>
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
              </>
            ) : (
              <ManagerChatPanel />
            )}
          </div>

          {/* Right column: Tech Lead's iteration stream */}
          <div className="flex min-w-0 flex-1 flex-col lg:max-w-xl lg:sticky lg:top-4 lg:self-start" style={{ minHeight: "60vh" }}>
            <TechLeadStream techLeadPane={techLeadPane} onOpenPane={onOpenPane} />
          </div>
        </div>
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
