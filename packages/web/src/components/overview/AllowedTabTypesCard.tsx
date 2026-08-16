"use client";

import { useStore } from "@/lib/store";

/** Read-only launch-profile policy for normal project users and owners. */
export function AllowedTabTypesCard() {
  const sessionId = useStore((state) => state.sessionId);
  const policy = useStore((state) => sessionId ? state.projectPolicies?.[sessionId] : undefined);

  return (
    <div className="mb-4 rounded-lg border border-sky-200 bg-sky-50/60 px-4 py-3 dark:border-sky-700/40 dark:bg-sky-900/10">
      <div className="mb-1 flex items-center justify-between gap-2">
        <span className="text-xs font-semibold uppercase tracking-wide text-sky-700 dark:text-sky-400">
          Allowed launch profiles
        </span>
        <span className="text-[11px] text-gray-500">Read-only cluster policy</span>
      </div>
      {!policy ? (
        <p className="text-xs text-gray-500">Waiting for an authoritative policy snapshot…</p>
      ) : policy.allowedLaunchProfiles.length === 0 ? (
        <p className="text-xs text-amber-700 dark:text-amber-400">No new panes may be launched.</p>
      ) : (
        <div className="flex flex-wrap gap-2">
          {policy.allowedLaunchProfiles.map((key) => (
            <span key={key} className="rounded bg-white px-2 py-1 font-mono text-[11px] text-gray-700 shadow-sm dark:bg-gray-800 dark:text-gray-300">
              {key}
            </span>
          ))}
        </div>
      )}
      {policy && policy.noncompliantPaneIds.length > 0 && (
        <p className="mt-2 text-xs text-gray-500 dark:text-gray-400">
          Panes {policy.noncompliantPaneIds.join(", ")} use a combination this policy no longer
          allows. They keep running and can be relaunched; the combination just cannot be chosen
          for a new tab.
        </p>
      )}
    </div>
  );
}
