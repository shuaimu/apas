"use client";

import { useStore } from "@/lib/store";
import { useCanManageCurrentProject } from "@/lib/projectRole";

const DEFAULT_FLAGS = {
  autoApproveTodos: false,
  autoMergePrs: false,
  teamEnabled: false,
};

/**
 * Project-level settings. All of these are policy for *everyone* on the
 * project, not per-seat preference, so they are restricted to the project's
 * owner and admins — a plain shared `user` sees them read-only. The server
 * enforces the same boundary in `ws_web::can_manage_project_settings`; this
 * only decides what to render.
 *
 *  - team mode — whether the managed Manager / Tech Lead / Developer /
 *    Reviewer panes may run at all. OFF for every project until someone turns
 *    it on, including projects created before the flag existed. Turning it off
 *    stops any team that is currently running.
 *  - auto-approve TODOs — Tech Lead may flip its own `proposed` Globals
 *    to `approved` when they align with `project_goal.md`, skipping the
 *    user's approve-on-Overview gate.
 *  - auto-merge PRs — Tech Lead may `gh pr merge` (or close with a
 *    rejection comment / post a "needs more work" review) on PRs in
 *    `pr_open` Globals.
 *
 * All default OFF. They live on `ProjectMetadata` in `.apas` so they survive a
 * CLI reboot; the Tech Lead re-reads `.apas` every iteration so a flip takes
 * effect at the next loop boundary.
 */
export function TechLeadAutonomyToggles() {
  const sessionId = useStore((s) => s.sessionId);
  const projectFlags = useStore((s) => s.projectFlags);
  const updateProjectFlags = useStore((s) => s.updateProjectFlags);
  const showToast = useStore((s) => s.showToast);
  const canManage = useCanManageCurrentProject();

  const flags = sessionId ? projectFlags[sessionId] ?? DEFAULT_FLAGS : DEFAULT_FLAGS;

  const flip = (key: "autoApproveTodos" | "autoMergePrs" | "teamEnabled") => {
    if (!canManage) return;
    if (key === "teamEnabled" && flags.teamEnabled) {
      // Turning team mode off stops every managed pane mid-turn, so make the
      // consequence explicit rather than letting a checkbox do it silently.
      if (
        typeof window !== "undefined" &&
        !window.confirm(
          "Turn off team mode? Any running Manager / Tech Lead / Developer / Reviewer panes will be interrupted and paused, and the team cannot be started again until you turn this back on.",
        )
      ) {
        return;
      }
    }
    updateProjectFlags({ ...flags, [key]: !flags[key] });
    if (key === "teamEnabled") {
      showToast(
        flags.teamEnabled ? "Team mode disabled" : "Team mode enabled",
        "success",
      );
    }
  };

  return (
    <div className="mb-4 rounded-lg border border-amber-200 bg-amber-50/60 px-4 py-3 dark:border-amber-700/40 dark:bg-amber-900/10">
      <div className="mb-1.5 flex items-center gap-2">
        <span className="text-xs font-semibold uppercase tracking-wide text-amber-700 dark:text-amber-400">
          Project settings
        </span>
        <span className="text-[11px] text-amber-700/70 dark:text-amber-400/70">
          {canManage
            ? "(takes effect next iteration)"
            : "(read-only — only the project owner or an admin can change these)"}
        </span>
      </div>
      <div className="flex flex-wrap gap-x-6 gap-y-2">
        <label
          className={`flex items-start gap-2 text-sm ${
            canManage ? "cursor-pointer" : "cursor-not-allowed opacity-60"
          }`}
        >
          <input
            type="checkbox"
            className="mt-0.5 h-4 w-4"
            checked={flags.teamEnabled}
            disabled={!canManage}
            onChange={() => flip("teamEnabled")}
          />
          <span>
            <span className="font-medium text-gray-800 dark:text-gray-200">
              Team mode
            </span>
            <span className="block text-xs text-gray-500 dark:text-gray-400">
              Allow the managed Manager / Tech Lead / Developer / Reviewer panes
              to run on this project. Off by default. Turning it off stops any
              running team.
            </span>
          </span>
        </label>
        <label
          className={`flex items-start gap-2 text-sm ${
            canManage ? "cursor-pointer" : "cursor-not-allowed opacity-60"
          }`}
        >
          <input
            type="checkbox"
            className="mt-0.5 h-4 w-4"
            checked={flags.autoApproveTodos}
            disabled={!canManage}
            onChange={() => flip("autoApproveTodos")}
          />
          <span>
            <span className="font-medium text-gray-800 dark:text-gray-200">
              Auto-approve TODOs
            </span>
            <span className="block text-xs text-gray-500 dark:text-gray-400">
              Tech Lead may flip its own <code className="text-[11px]">proposed</code> Globals
              to <code className="text-[11px]">approved</code> without asking.
            </span>
          </span>
        </label>
        <label
          className={`flex items-start gap-2 text-sm ${
            canManage ? "cursor-pointer" : "cursor-not-allowed opacity-60"
          }`}
        >
          <input
            type="checkbox"
            className="mt-0.5 h-4 w-4"
            checked={flags.autoMergePrs}
            disabled={!canManage}
            onChange={() => flip("autoMergePrs")}
          />
          <span>
            <span className="font-medium text-gray-800 dark:text-gray-200">
              Auto-merge PRs
            </span>
            <span className="block text-xs text-gray-500 dark:text-gray-400">
              Tech Lead may <code className="text-[11px]">gh pr merge</code>, close with a
              rejection comment, or comment "needs more work" on PRs in{" "}
              <code className="text-[11px]">pr_open</code> Globals.
            </span>
            <span className="block text-xs text-gray-500 dark:text-gray-400">
              Requires GitHub auto-merge enabled on the target repository,
              Reviewer approval, mergeability, and green/no-pending CI.
            </span>
          </span>
        </label>
      </div>
    </div>
  );
}
