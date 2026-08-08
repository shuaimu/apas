"use client";

import { useStore } from "@/lib/store";

/** Read-only projection of cluster-governed team availability. */
export function TeamModeSwitch() {
  const sessionId = useStore((state) => state.sessionId);
  const policy = useStore((state) => sessionId ? state.projectPolicies?.[sessionId] : undefined);
  const on = policy?.teamAvailable === true && policy.projectSuspended !== true;

  return (
    <div className={`mb-4 rounded-lg border px-4 py-3 ${
      on
        ? "border-emerald-300 bg-emerald-50/70 dark:border-emerald-700/50 dark:bg-emerald-900/15"
        : "border-gray-300 bg-gray-50 dark:border-gray-700 dark:bg-gray-800/40"
    }`}>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <div className="flex items-center gap-2">
            <span className="font-semibold">Team mode</span>
            <span className={`rounded px-1.5 py-0.5 text-[11px] font-semibold uppercase text-white ${on ? "bg-emerald-600" : "bg-gray-500"}`}>
              {policy ? (on ? "Available" : "Unavailable") : "Checking"}
            </span>
          </div>
          <p className="mt-1 text-xs text-gray-600 dark:text-gray-400">
            {policy?.projectSuspended
              ? "This project is suspended. Existing panes are preserved, but no launch or relaunch is allowed."
              : on
                ? "Team launches are allowed by the current cluster policy."
                : "Team launches are disabled by the current cluster policy."}
          </p>
        </div>
        <span className="text-xs text-gray-500">
          {policy ? `Cluster policy v${policy.version} · managed by a cluster administrator` : "Waiting for the project host"}
        </span>
      </div>
    </div>
  );
}
