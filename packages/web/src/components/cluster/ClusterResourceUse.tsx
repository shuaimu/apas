"use client";

import { useMemo } from "react";
import { useStore } from "@/lib/store";
import { UsageLimitsDisplay, latestLimitsByProvider } from "@/components/UsageLimits";
import { providerDisplayName } from "@/lib/launchProfileLabels";
import { OverviewEmpty, OverviewSection } from "@/components/overview/OverviewElements";

/** Each provider's limits, from the newest report on any machine. They belong
 *  to the provider account, so they cover more than this cluster. */
export function ClusterResourceUse() {
  const usageLimits = useStore((state) => state.usageLimits);
  const rows = useMemo(() => latestLimitsByProvider(usageLimits), [usageLimits]);
  return (
    <OverviewSection
      title="Resource use"
      description="Provider limits from the latest report on any of your machines. They cover your whole account, not only this cluster."
    >
      {rows.length === 0 ? (
        <OverviewEmpty>No usage data available. Start a session to see usage limits.</OverviewEmpty>
      ) : (
        // The full display: both windows with their reset times. It is its
        // own shaded box, so it gets no card around it.
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {rows.map(({ provider, limits }) => (
            <div key={provider} className="min-w-0">
              <div className="mb-1 text-xs font-medium text-gray-700 dark:text-gray-300">{providerDisplayName(provider)}</div>
              <UsageLimitsDisplay limits={limits} />
            </div>
          ))}
        </div>
      )}
    </OverviewSection>
  );
}
