"use client";

import { useMemo } from "react";
import { Play } from "lucide-react";
import { useStore, type PaneConfig } from "@/lib/store";

/**
 * Prompt to spawn the default team (Manager + Tech Lead + Reviewer +
 * Developer) when none of those roles are present in the project.
 *
 * Pre-v3.4 behaviour: the CLI auto-spawned all four on boot. The user
 * asked to make this opt-in so a fresh `apas` invocation doesn't burn
 * tokens on a team they don't want — this banner is what they click to
 * get it. The CLI side is idempotent (only spawns the roles that are
 * missing), so the button stays useful even after a partial setup.
 */
function isTeamRole(p: PaneConfig): boolean {
  if (!p.managed) return false;
  const lower = (p.role ?? "").toLowerCase();
  return (
    lower.includes("manager") ||
    lower.includes("tech lead") ||
    lower.includes("reviewer") ||
    lower.includes("developer")
  );
}

export function StartTeamBanner() {
  const paneConfigs = useStore((s) => s.paneConfigs);
  const startTeam = useStore((s) => s.startTeam);

  const teamPresent = useMemo(() => paneConfigs.some(isTeamRole), [paneConfigs]);
  if (teamPresent) return null;

  return (
    <div className="mb-4 flex flex-col gap-3 rounded-lg border border-indigo-200 bg-indigo-50 px-4 py-3 dark:border-indigo-700/50 dark:bg-indigo-900/20 sm:flex-row sm:items-center sm:justify-between">
      <div>
        <div className="text-sm font-semibold text-indigo-900 dark:text-indigo-200">
          Team not started
        </div>
        <p className="mt-0.5 text-xs text-indigo-800/80 dark:text-indigo-300/80">
          New projects no longer auto-spawn agents. Click below to create
          Manager, Tech Lead, Reviewer and a default Developer pane.
        </p>
      </div>
      <button
        onClick={startTeam}
        className="inline-flex items-center justify-center gap-1.5 rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-medium text-white shadow-sm transition hover:bg-indigo-700 dark:bg-indigo-500 dark:hover:bg-indigo-400"
      >
        <Play className="h-3.5 w-3.5" />
        Start team
      </button>
    </div>
  );
}
