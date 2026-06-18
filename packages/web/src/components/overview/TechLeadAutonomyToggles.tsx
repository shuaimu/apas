"use client";

import { useStore } from "@/lib/store";

/**
 * Two project-level toggles that change what the Tech Lead is allowed
 * to do during its autonomous deadloop:
 *
 *  - auto-approve TODOs — Tech Lead may flip its own `proposed` Globals
 *    to `approved` when they align with `project_goal.md`, skipping the
 *    user's approve-on-Overview gate.
 *  - auto-merge PRs — Tech Lead may `gh pr merge` (or close with a
 *    rejection comment / post a "needs more work" review) on PRs in
 *    `pr_open` Globals.
 *
 * Both default OFF. They live on `ProjectMetadata` in `.apas` so they
 * survive a CLI reboot; the Tech Lead re-reads `.apas` every iteration
 * so a toggle flip takes effect at the next loop boundary.
 */
export function TechLeadAutonomyToggles() {
  const sessionId = useStore((s) => s.sessionId);
  const projectFlags = useStore((s) => s.projectFlags);
  const updateProjectFlags = useStore((s) => s.updateProjectFlags);

  const flags = sessionId
    ? projectFlags[sessionId] ?? { autoApproveTodos: false, autoMergePrs: false }
    : { autoApproveTodos: false, autoMergePrs: false };

  const flip = (key: "autoApproveTodos" | "autoMergePrs") => {
    updateProjectFlags({
      ...flags,
      [key]: !flags[key],
    });
  };

  return (
    <div className="mb-4 rounded-lg border border-amber-200 bg-amber-50/60 px-4 py-3 dark:border-amber-700/40 dark:bg-amber-900/10">
      <div className="mb-1.5 flex items-center gap-2">
        <span className="text-xs font-semibold uppercase tracking-wide text-amber-700 dark:text-amber-400">
          Tech Lead autonomy
        </span>
        <span className="text-[11px] text-amber-700/70 dark:text-amber-400/70">
          (takes effect next iteration)
        </span>
      </div>
      <div className="flex flex-wrap gap-x-6 gap-y-2">
        <label className="flex cursor-pointer items-start gap-2 text-sm">
          <input
            type="checkbox"
            className="mt-0.5 h-4 w-4"
            checked={flags.autoApproveTodos}
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
        <label className="flex cursor-pointer items-start gap-2 text-sm">
          <input
            type="checkbox"
            className="mt-0.5 h-4 w-4"
            checked={flags.autoMergePrs}
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
