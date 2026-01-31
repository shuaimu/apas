"use client";

import { useStore, UsageLimits as UsageLimitsType } from "@/lib/store";
import { useMemo } from "react";

function formatTimeUntilReset(resetsAt: string | undefined): string {
  if (!resetsAt) return "";

  const now = new Date();
  const resetDate = new Date(resetsAt);
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

interface UsageBarProps {
  label: string;
  utilization: number;
  resetsAt?: string;
}

function UsageBar({ label, utilization, resetsAt }: UsageBarProps) {
  const percentage = Math.min(utilization * 100, 100);
  const displayPercentage = (utilization * 100).toFixed(0);

  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between text-xs">
        <span className="text-gray-600 dark:text-gray-400">{label}</span>
        <span className={`font-medium ${getTextColor(utilization)}`}>
          {displayPercentage}%
          {resetsAt && (
            <span className="text-gray-500 ml-1">
              ({formatTimeUntilReset(resetsAt)})
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

export function UsageLimitsDisplay({ limits, compact = false }: UsageLimitsDisplayProps) {
  if (!limits.fiveHour && !limits.sevenDay) {
    return null;
  }

  if (compact) {
    // Compact mode: just show the most concerning limit
    const sevenDayUtil = limits.sevenDay?.utilization ?? 0;
    const fiveHourUtil = limits.fiveHour?.utilization ?? 0;
    const maxUtil = Math.max(sevenDayUtil, fiveHourUtil);
    const percentage = (maxUtil * 100).toFixed(0);

    return (
      <div className={`flex items-center gap-1 text-xs ${getTextColor(maxUtil)}`}>
        <div className={`w-2 h-2 rounded-full ${getUtilizationColor(maxUtil)}`} />
        <span>{percentage}% used</span>
      </div>
    );
  }

  return (
    <div className="space-y-2 p-2 bg-gray-50 dark:bg-gray-800/50 rounded-lg">
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
  const currentLimits = useMemo(() => {
    if (!currentCliClientId) return null;
    return usageLimits.get(currentCliClientId) ?? null;
  }, [currentCliClientId, usageLimits]);

  if (!currentLimits) {
    return null;
  }

  return (
    <div className="border-t border-gray-200 dark:border-gray-800 p-3">
      <div className="text-xs font-medium text-gray-500 mb-2">Claude Usage</div>
      <UsageLimitsDisplay limits={currentLimits} />
    </div>
  );
}
