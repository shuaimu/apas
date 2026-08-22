"use client";

import { useStore, UsageLimits as UsageLimitsType } from "@/lib/store";
import {
  activeUsageLimit,
  usageLimitedLabel,
  usageLimitResetLabel,
} from "@/lib/usageLimitStatus";
import { useEffect, useMemo, useState } from "react";

function formatTimeUntilReset(resetsAt: string | undefined): string {
  if (!resetsAt) return "";

  const now = new Date();
  const resetDate = new Date(resetsAt);
  if (Number.isNaN(resetDate.getTime())) return "";
  const diffMs = resetDate.getTime() - now.getTime();

  if (diffMs <= 0) return "resetting...";

  const hours = Math.floor(diffMs / (1000 * 60 * 60));
  const minutes = Math.floor((diffMs % (1000 * 60 * 60)) / (1000 * 60));

  if (hours > 24) {
    const days = Math.floor(hours / 24);
    return `${days}d ${hours % 24}h`;
  }
  if (hours > 0) {
    return `${hours}h ${minutes}m`;
  }
  return `${minutes}m`;
}

function formatResetDateTime(resetsAt: string | undefined): string {
  if (!resetsAt) return "";

  const now = new Date();
  const resetDate = new Date(resetsAt);
  if (Number.isNaN(resetDate.getTime())) return "";

  const sameDay = resetDate.toDateString() === now.toDateString();
  const sameYear = resetDate.getFullYear() === now.getFullYear();

  if (sameDay) {
    return new Intl.DateTimeFormat(undefined, {
      hour: "numeric",
      minute: "2-digit",
    }).format(resetDate);
  }

  if (sameYear) {
    return new Intl.DateTimeFormat(undefined, {
      weekday: "short",
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
    }).format(resetDate);
  }

  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(resetDate);
}

function formatResetMeta(resetsAt: string | undefined): string {
  if (!resetsAt) return "";
  const relative = formatTimeUntilReset(resetsAt);
  const absolute = formatResetDateTime(resetsAt);

  if (relative === "resetting...") return relative;
  if (relative && absolute) return `${relative} · resets ${absolute}`;
  if (absolute) return `resets ${absolute}`;
  return relative;
}

function getUtilizationColor(utilization: number): string {
  if (utilization >= 1.0) return "bg-red-500";
  if (utilization >= 0.9) return "bg-orange-500";
  if (utilization >= 0.75) return "bg-yellow-500";
  return "bg-green-500";
}

function getTextColor(utilization: number): string {
  if (utilization >= 1.0) return "text-red-600 dark:text-red-400";
  if (utilization >= 0.9) return "text-orange-600 dark:text-orange-400";
  if (utilization >= 0.75) return "text-yellow-600 dark:text-yellow-400";
  return "text-green-600 dark:text-green-400";
}

function formatUtilizationPercent(utilization: number): string {
  if (!Number.isFinite(utilization)) return "0";

  const rawPercent = Math.max(0, utilization * 100);
  // Avoid showing "100%" for sub-100 utilization due to rounding.
  const displayPercent = utilization < 1.0 ? Math.min(rawPercent, 99.9) : rawPercent;

  if (displayPercent >= 100) {
    return displayPercent.toFixed(0);
  }
  if (displayPercent >= 99) {
    return displayPercent.toFixed(1).replace(/\.0$/, "");
  }
  return displayPercent.toFixed(0);
}

interface UsageBarProps {
  label: string;
  utilization: number;
  resetsAt?: string;
}

function UsageBar({ label, utilization, resetsAt }: UsageBarProps) {
  const percentage = Math.min(utilization * 100, 100);
  const displayPercentage = formatUtilizationPercent(utilization);
  const resetMeta = formatResetMeta(resetsAt);

  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between text-xs">
        <span className="text-gray-600 dark:text-gray-400">{label}</span>
        <span className={`font-medium ${getTextColor(utilization)}`}>
          {displayPercentage}%
          {resetMeta && (
            <span className="text-gray-500 ml-1">
              ({resetMeta})
            </span>
          )}
        </span>
      </div>
      <div className="h-2 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
        <div
          className={`h-full transition-all duration-300 ${getUtilizationColor(utilization)}`}
          style={{ width: `${percentage}%` }}
        />
      </div>
    </div>
  );
}

interface UsageLimitsDisplayProps {
  limits: UsageLimitsType;
  compact?: boolean;
}

interface CompactUsageWindow {
  label: string;
  utilization: number;
  resetsAt: string | undefined;
}

export function UsageLimitsDisplay({ limits, compact = false }: UsageLimitsDisplayProps) {
  const [availabilityNow, setAvailabilityNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setAvailabilityNow(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, []);
  const limited = activeUsageLimit(limits, availabilityNow);

  if (!limits.fiveHour && !limits.sevenDay && !limited) {
    return null;
  }

  if (compact) {
    if (limited) {
      const resetLabel = usageLimitResetLabel(limited, availabilityNow);
      return (
        <div className="flex items-center gap-1 text-xs font-semibold text-red-600 dark:text-red-400">
          <div className="h-2 w-2 rounded-full bg-red-500" />
          <span>{usageLimitedLabel(limited)}</span>
          {resetLabel && (
            <span className="font-normal text-gray-500 dark:text-gray-400">· {resetLabel}</span>
          )}
        </div>
      );
    }

    // In compact mode, prefer weekly usage whenever available.
    const primary: CompactUsageWindow | null = limits.sevenDay
      ? {
          label: "Weekly",
          utilization: limits.sevenDay.utilization,
          resetsAt: limits.sevenDay.resetsAt,
        }
      : limits.fiveHour
      ? {
          label: "5h",
          utilization: limits.fiveHour.utilization,
          resetsAt: limits.fiveHour.resetsAt,
        }
      : null;

    if (!primary) return null;

    const percentage = formatUtilizationPercent(primary.utilization);
    // Mobile-friendly compact rendering: only the dot + percentage live
    // in the header strip. Reset info is hidden until utilization is
    // actually meaningful (≥50% — below that, the window resetting in
    // "5d" doesn't matter), or when the window is actively resetting
    // (a transition state worth surfacing). The full relative + absolute
    // text moves to a title= tooltip so the info isn't lost.
    const relative = formatTimeUntilReset(primary.resetsAt);
    const fullMeta = formatResetMeta(primary.resetsAt);
    const isResetting = relative === "resetting...";
    const showReset = !!relative && (primary.utilization >= 0.5 || isResetting);

    return (
      <div
        className={`flex items-center gap-1 text-xs ${getTextColor(primary.utilization)}`}
        title={fullMeta || undefined}
      >
        <div className={`w-2 h-2 rounded-full ${getUtilizationColor(primary.utilization)}`} />
        <span>{percentage}%</span>
        {showReset && (
          <span className="text-gray-500 dark:text-gray-400">· {relative}</span>
        )}
      </div>
    );
  }

  const resetLabel = limited
    ? usageLimitResetLabel(limited, availabilityNow)
    : null;

  return (
    <div className="space-y-2 p-2 bg-gray-50 dark:bg-gray-800/50 rounded-lg">
      {limited && (
        <div className="rounded-md bg-red-50 px-2 py-1.5 text-xs font-semibold text-red-700 dark:bg-red-950/50 dark:text-red-300">
          {usageLimitedLabel(limited)}
          {resetLabel && (
            <span className="ml-1 font-normal">
              · {resetLabel}
            </span>
          )}
        </div>
      )}
      {limits.sevenDay && (
        <UsageBar
          label="Weekly"
          utilization={limits.sevenDay.utilization}
          resetsAt={limits.sevenDay.resetsAt}
        />
      )}
      {limits.fiveHour && (
        <UsageBar
          label="5-Hour"
          utilization={limits.fiveHour.utilization}
          resetsAt={limits.fiveHour.resetsAt}
        />
      )}
    </div>
  );
}

const PROVIDER_LABELS: Record<string, string> = {
  claude: "Claude",
  codex: "Codex",
  deepseek: "DeepSeek",
};

const PROVIDER_COLORS: Record<string, string> = {
  claude: "border-l-amber-500",
  codex: "border-l-blue-500",
  deepseek: "border-l-indigo-500",
};

export function AllProvidersUsage() {
  const usageLimits = useStore((s) => s.usageLimits);

  // Aggregate usage across all CLI clients — take the most recently fetched entry per provider.
  const aggregated = useMemo(() => {
    const latest: Partial<Record<string, { limits: UsageLimitsType; fetchedAt?: string }>> = {};

    for (const [, byProvider] of usageLimits) {
      for (const [provider, limits] of Object.entries(byProvider)) {
        if (!(provider in PROVIDER_LABELS)) continue;
        const existing = latest[provider];
        if (!limits.fiveHour && !limits.sevenDay) continue;
        if (
          !existing ||
          (limits.fetchedAt && (!existing.fetchedAt || limits.fetchedAt > existing.fetchedAt))
        ) {
          latest[provider] = { limits, fetchedAt: limits.fetchedAt };
        }
      }
    }
    return latest;
  }, [usageLimits]);

  const providers = Object.entries(aggregated);
  if (providers.length === 0) {
    return (
      <div className="text-xs text-gray-500">
        No usage data available. Start a session to see usage limits.
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {providers.map(([provider, entry]) => {
        if (!entry) return null;
        return (
          <div
            key={provider}
            className={`border-l-4 ${PROVIDER_COLORS[provider] || "border-l-gray-400"} pl-3`}
          >
            <div className="mb-1 text-sm font-medium">
              {PROVIDER_LABELS[provider] || provider}
            </div>
            <UsageLimitsDisplay limits={entry.limits} />
          </div>
        );
      })}
    </div>
  );
}

export function UsageLimitsPanel() {
  const { usageLimits, cliClients, sessionId, sessions } = useStore();

  // Get the CLI client ID for the current session
  const currentCliClientId = useMemo(() => {
    if (!sessionId) return null;

    // First check if any CLI client has this as their active session
    const activeClient = cliClients.find(c => c.activeSession === sessionId);
    if (activeClient) return activeClient.id;

    // Fall back to the session's CLI client ID from the sessions list
    const session = sessions.find(s => s.id === sessionId);
    return session?.cliClientId ?? null;
  }, [sessionId, cliClients, sessions]);

  // Get usage limits for the current session's CLI client
  const currentUsage = useMemo(() => {
    if (!currentCliClientId) return null;
    const limitsByProvider = usageLimits.get(currentCliClientId);
    if (!limitsByProvider) return null;

    if (limitsByProvider.deepseek) {
      return { label: "DeepSeek Usage", limits: limitsByProvider.deepseek };
    }
    if (limitsByProvider.codex) {
      return { label: "Codex Usage", limits: limitsByProvider.codex };
    }
    if (limitsByProvider.claude) {
      return { label: "Claude Usage", limits: limitsByProvider.claude };
    }
    return null;
  }, [currentCliClientId, usageLimits]);

  if (!currentUsage) {
    return null;
  }

  return (
    <div className="border-t border-gray-200 dark:border-gray-800 p-3">
      <div className="text-xs font-medium text-gray-500 mb-2">{currentUsage.label}</div>
      <UsageLimitsDisplay limits={currentUsage.limits} />
    </div>
  );
}
