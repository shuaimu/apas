"use client";

/**
 * The Overview's "Resource use" section: each provider's usage limits for the
 * current project. Reuses the existing UsageLimitsDisplay component so the
 * rendering matches what each tab's header shows. Lists one card per provider
 * that actually has limits cached.
 */
import { useMemo } from "react";
import { useStore, SupportedProvider, UsageLimits } from "@/lib/store";
import { providerDisplayName } from "@/lib/launchProfileLabels";
import { UsageLimitsDisplay } from "../UsageLimits";
import { OverviewEmpty, overviewCard, overviewMuted } from "./OverviewElements";

export function ResourceUseRollup() {
  const cliClientId = useStore((s) => s.cliClientId);
  const usageLimits = useStore((s) => s.usageLimits);
  const paneConfigs = useStore((s) => s.paneConfigs);

  // Providers that this project's panes actually use (so we don't list
  // limits for providers the user hasn't opted into).
  const providers = useMemo(() => {
    const set = new Set<SupportedProvider>();
    for (const p of paneConfigs) {
      // Retired providers cannot run, so they have no limits to show. A
      // saved Pi tab stays listed among the tabs and adds no card here.
      if (p.provider !== "minimax" && p.provider !== "glm" && p.provider !== "pi") {
        set.add(p.provider);
      }
    }
    return Array.from(set);
  }, [paneConfigs]);

  const limitsForClient = cliClientId ? usageLimits.get(cliClientId) : undefined;

  const rows: Array<{ provider: SupportedProvider; limits: UsageLimits }> = useMemo(() => {
    if (!limitsForClient) return [];
    const out: Array<{ provider: SupportedProvider; limits: UsageLimits }> = [];
    for (const p of providers) {
      const l = limitsForClient[p];
      if (l && (l.fiveHour || l.sevenDay)) {
        out.push({ provider: p, limits: l });
      }
    }
    return out;
  }, [providers, limitsForClient]);

  if (rows.length === 0) {
    return (
      <OverviewEmpty>
        No usage telemetry yet. Limits populate after each provider runs its first turn.
      </OverviewEmpty>
    );
  }

  return (
    <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-6">
      {rows.map(({ provider, limits }) => (
        <div key={provider} className={`${overviewCard} px-3 py-2`}>
          <div className={`mb-1 ${overviewMuted}`}>{providerDisplayName(provider)}</div>
          <UsageLimitsDisplay limits={limits} compact />
        </div>
      ))}
    </div>
  );
}
