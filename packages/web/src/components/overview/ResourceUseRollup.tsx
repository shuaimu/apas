"use client";

/**
 * Phase 5.1e — collapsed view of per-provider usage limits for the
 * current project. Reuses the existing UsageLimitsDisplay component
 * so the rendering matches what each per-pane header shows. Lists
 * one row per provider that actually has limits cached.
 */
import { useMemo } from "react";
import { useStore, Provider, UsageLimits } from "@/lib/store";
import { UsageLimitsDisplay } from "../UsageLimits";

const PROVIDER_LABELS: Record<Provider, string> = {
  claude: "Claude",
  codex: "Codex",
  minimax: "MiniMax",
  glm: "GLM",
  opencode: "OpenCode",
  "cursor-agent": "Cursor",
};

export function ResourceUseRollup() {
  const cliClientId = useStore((s) => s.cliClientId);
  const usageLimits = useStore((s) => s.usageLimits);
  const paneConfigs = useStore((s) => s.paneConfigs);

  // Providers that this project's panes actually use (so we don't list
  // limits for providers the user hasn't opted into).
  const providers = useMemo(() => {
    const set = new Set<Provider>();
    for (const p of paneConfigs) {
      set.add(p.provider);
    }
    return Array.from(set);
  }, [paneConfigs]);

  const limitsForClient = cliClientId ? usageLimits.get(cliClientId) : undefined;

  const rows: Array<{ provider: Provider; limits: UsageLimits }> = useMemo(() => {
    if (!limitsForClient) return [];
    const out: Array<{ provider: Provider; limits: UsageLimits }> = [];
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
      <div className="rounded border border-dashed border-gray-300 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/30 p-4 text-sm italic text-gray-500 dark:text-gray-400">
        No usage telemetry yet. Limits populate after each provider runs its first turn.
      </div>
    );
  }

  return (
    <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3">
      {rows.map(({ provider, limits }) => (
        <div
          key={provider}
          className="rounded border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800/40 px-3 py-2"
        >
          <div className="mb-1 text-[10px] uppercase tracking-wide text-gray-500 dark:text-gray-400">
            {PROVIDER_LABELS[provider] ?? provider}
          </div>
          <UsageLimitsDisplay limits={limits} compact />
        </div>
      ))}
    </div>
  );
}
