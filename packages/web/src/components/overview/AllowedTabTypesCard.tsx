"use client";

import { useStore } from "@/lib/store";
import { launchProfileLabel } from "@/lib/launchProfileLabels";
import { isRetiredLaunchProfileKey } from "@/lib/providerOptions";
import { OverviewSection, overviewCard, overviewChip, overviewMuted } from "./OverviewElements";

/**
 * The Overview's "Allowed launch profiles" section: what a new tab of this
 * project may run. Read-only, for normal project users and owners alike.
 *
 * A profile is shown by its name from the provider catalog, with the policy's
 * own key as the tooltip. A key the catalog cannot name is shown as it is.
 * A profile of a retired provider is left out, as the Machines and admin
 * policy lists leave it out: a policy may still carry its key, but no menu
 * offers it and nothing can launch it.
 */
export function AllowedTabTypesCard() {
  const sessionId = useStore((state) => state.sessionId);
  const policy = useStore((state) => sessionId ? state.projectPolicies?.[sessionId] : undefined);
  const profiles = (policy?.allowedLaunchProfiles ?? []).filter((key) => !isRetiredLaunchProfileKey(key));

  return (
    <OverviewSection
      title="Allowed launch profiles"
      level={3}
      aside={<span className={overviewMuted}>Read-only cluster policy</span>}
      description="What a new tab of this project may run."
    >
      <div className={`${overviewCard} px-3 py-3`}>
        {!policy ? (
          <p className={overviewMuted}>Waiting for an authoritative policy snapshot…</p>
        ) : profiles.length === 0 ? (
          <p className="text-sm text-amber-700 dark:text-amber-400">No new tabs may be launched.</p>
        ) : (
          <ul className="flex flex-wrap gap-2">
            {profiles.map((key) => {
              const profile = launchProfileLabel(key);
              return (
                <li
                  key={key}
                  title={key}
                  className={`${overviewChip} ${profile.known ? "" : "font-mono"}`}
                >
                  {profile.label}
                </li>
              );
            })}
          </ul>
        )}
        {policy && policy.noncompliantPaneIds.length > 0 && (
          <p className={`mt-2 ${overviewMuted}`}>
            Tabs {policy.noncompliantPaneIds.join(", ")} use a combination this policy no longer
            allows. They keep running and can be relaunched; the combination just cannot be chosen
            for a new tab.
          </p>
        )}
      </div>
    </OverviewSection>
  );
}
