"use client";

import { useStore } from "@/lib/store";
import { useCanManageCurrentProject } from "@/lib/projectRole";

const DEFAULT_FLAGS = {
  autoApproveTodos: false,
  autoMergePrs: false,
  teamEnabled: false,
};

/**
 * The project-level team on/off switch.
 *
 * Deliberately the first thing on the Overview and visually its own band:
 * this decides whether the rest of the page exists at all, so burying it in a
 * settings card below the goal bar made the page look broken rather than
 * switched off. When team mode is off this is also the only element that
 * explains *why* everything else is missing.
 *
 * Owner/admin only, matching `ws_web::can_manage_project_settings` on the
 * server — for anyone else it renders as a read-only state indicator.
 */
export function TeamModeSwitch() {
  const sessionId = useStore((s) => s.sessionId);
  const projectFlags = useStore((s) => s.projectFlags);
  const updateProjectFlags = useStore((s) => s.updateProjectFlags);
  const showToast = useStore((s) => s.showToast);
  const canManage = useCanManageCurrentProject();

  const flags = sessionId ? projectFlags[sessionId] ?? DEFAULT_FLAGS : DEFAULT_FLAGS;
  const on = flags.teamEnabled;

  const toggle = () => {
    if (!canManage) return;
    if (on) {
      // Turning off interrupts and pauses every managed pane. A switch this
      // prominent is easy to hit by accident, so the destructive direction
      // asks first. Turning on creates nothing, so it does not.
      if (
        typeof window !== "undefined" &&
        !window.confirm(
          "Turn off team mode? Any running Manager / Tech Lead / Developer / Reviewer panes will be interrupted and paused, and the team cannot be started again until you turn this back on.",
        )
      ) {
        return;
      }
    }
    updateProjectFlags({ ...flags, teamEnabled: !on });
    showToast(on ? "Team mode disabled" : "Team mode enabled", "success");
  };

  return (
    <div
      className={`mb-4 rounded-lg border px-4 py-3 ${
        on
          ? "border-emerald-300 bg-emerald-50/70 dark:border-emerald-700/50 dark:bg-emerald-900/15"
          : "border-gray-300 bg-gray-50 dark:border-gray-700 dark:bg-gray-800/40"
      }`}
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-3">
          <button
            type="button"
            role="switch"
            aria-checked={on}
            aria-label="Team mode"
            disabled={!canManage}
            onClick={toggle}
            title={
              canManage
                ? on
                  ? "Turn team mode off — stops any running team"
                  : "Turn team mode on for this project"
                : "Only the project owner or an admin can change team mode"
            }
            className={`relative inline-flex h-7 w-12 flex-shrink-0 items-center rounded-full transition-colors ${
              on ? "bg-emerald-500" : "bg-gray-400 dark:bg-gray-600"
            } ${canManage ? "cursor-pointer" : "cursor-not-allowed opacity-60"}`}
          >
            <span
              className={`inline-block h-5 w-5 transform rounded-full bg-white shadow transition-transform ${
                on ? "translate-x-6" : "translate-x-1"
              }`}
            />
          </button>
          <div>
            <div className="flex items-center gap-2">
              <span className="text-base font-semibold text-gray-900 dark:text-gray-100">
                Team mode
              </span>
              <span
                className={`rounded px-1.5 py-0.5 text-[11px] font-semibold uppercase tracking-wide ${
                  on
                    ? "bg-emerald-600 text-white"
                    : "bg-gray-500 text-white dark:bg-gray-600"
                }`}
              >
                {on ? "On" : "Off"}
              </span>
            </div>
            <p className="mt-0.5 text-xs text-gray-600 dark:text-gray-400">
              {on
                ? "Manager, Tech Lead, Developer and Reviewer panes may run on this project."
                : "The Manager / Tech Lead / Developer / Reviewer panes are unavailable for this project."}
            </p>
          </div>
        </div>
        {!canManage && (
          <span className="text-xs text-gray-500 dark:text-gray-400">
            Only the project owner or an admin can change this.
          </span>
        )}
      </div>
    </div>
  );
}
