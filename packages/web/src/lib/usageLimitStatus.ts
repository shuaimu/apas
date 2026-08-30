import { isDeepseekModel } from "@/lib/providerOptions";
import type {
  SessionInfo,
  SessionPaneSummary,
  SupportedProvider,
  UsageLimitedStatus,
  UsageLimits,
  UsageLimitsByProvider,
} from "@/lib/store";

function paneUsageProvider(pane: SessionPaneSummary): SupportedProvider | null {
  if (pane.provider === "deepseek") return "deepseek";
  if (pane.provider === "codex") return "codex";
  if (pane.provider !== "claude") return null;
  if (
    isDeepseekModel(pane.model)
    || pane.label?.trim().toLowerCase().includes("deepseek")
  ) {
    return "deepseek";
  }
  return "claude";
}

/** Ignore a stale blocking snapshot as soon as its provider reset has passed. */
export function activeUsageLimit(
  limits: UsageLimits | undefined,
  nowMs: number = Date.now(),
): UsageLimitedStatus | null {
  const limited = limits?.usageLimited;
  if (!limited) return null;
  if (!limited.resetsAt) return enrichScopedUsageLimit(limited, limits);

  const resetMs = Date.parse(limited.resetsAt);
  if (Number.isFinite(resetMs) && resetMs <= nowMs) return null;
  return enrichScopedUsageLimit(limited, limits);
}

function utilizationForLimitWindow(
  status: UsageLimitedStatus,
  limits: UsageLimits,
): number | undefined {
  const window = status.window.trim().toLowerCase();
  if (window.includes("week") || window.includes("7")) {
    return limits.sevenDay?.utilization;
  }
  if (window.includes("hour") || window.includes("5")) {
    return limits.fiveHour?.utilization;
  }
  return undefined;
}

function enrichScopedUsageLimit(
  status: UsageLimitedStatus,
  limits: UsageLimits,
): UsageLimitedStatus {
  if (!status.model?.trim()) return status;
  const allModelsUtilization = utilizationForLimitWindow(status, limits);
  if (!Number.isFinite(allModelsUtilization)) return status;
  return { ...status, allModelsUtilization };
}

function usageLimitAppliesToPane(
  limited: UsageLimitedStatus,
  pane: SessionPaneSummary,
): boolean {
  const scope = limited.model?.trim().toLowerCase();
  if (!scope) return true;
  const paneModel = pane.model?.trim().toLowerCase();
  if (!paneModel) return false;
  return paneModel === scope
    || paneModel.includes(scope)
    || scope.includes(paneModel);
}

function activePaneUsageLimit(
  limits: UsageLimits | undefined,
  pane: SessionPaneSummary,
  nowMs: number,
): UsageLimitedStatus | null {
  const limited = activeUsageLimit(limits, nowMs);
  return limited && usageLimitAppliesToPane(limited, pane) ? limited : null;
}

/**
 * Resolve one pane's provider availability without conflating it with work.
 * A live per-client usage payload wins over the session-list snapshot; shared
 * viewers may have only the privacy-preserving snapshot.
 */
export function paneUsageLimit(
  session: Pick<SessionInfo, "cliClientId">,
  pane: SessionPaneSummary,
  usageLimits: Map<string, UsageLimitsByProvider>,
  nowMs: number = Date.now(),
): UsageLimitedStatus | null {
  const provider = paneUsageProvider(pane);
  const liveLimits = session.cliClientId && provider
    ? usageLimits.get(session.cliClientId)?.[provider]
    : undefined;

  if (liveLimits) return activePaneUsageLimit(liveLimits, pane, nowMs);

  return activePaneUsageLimit(
    pane.usage_limited
      ? { usageLimited: {
          window: pane.usage_limited.window,
          resetsAt: pane.usage_limited.resetsAt
            ?? pane.usage_limited.resets_at,
          model: pane.usage_limited.model,
        } }
      : undefined,
    pane,
    nowMs,
  );
}

export function usageLimitedLabel(status: UsageLimitedStatus): string {
  const window = status.window.trim();
  const model = status.model?.trim();
  if (model) {
    const scopedLabel = window
      ? `${model} ${window} usage limited`
      : `${model} usage limited`;
    if (Number.isFinite(status.allModelsUtilization)) {
      return `${scopedLabel}; all models at ${formatUsagePercent(status.allModelsUtilization!)}%`;
    }
    return scopedLabel;
  }
  if (!window) return "Usage limited";
  return `${window[0].toUpperCase()}${window.slice(1)} usage limited`;
}

/** Keep high-but-incomplete usage from rounding up to a misleading 100%. */
export function formatUsagePercent(utilization: number): string {
  if (!Number.isFinite(utilization)) return "0";

  const rawPercent = Math.max(0, utilization * 100);
  const displayPercent = utilization < 1.0
    ? Math.min(rawPercent, 99.9)
    : rawPercent;

  if (displayPercent >= 100) return displayPercent.toFixed(0);
  if (displayPercent >= 99) {
    return displayPercent.toFixed(1).replace(/\.0$/, "");
  }
  return displayPercent.toFixed(0);
}

export function usageLimitResetLabel(
  status: UsageLimitedStatus,
  nowMs: number = Date.now(),
): string | null {
  if (!status.resetsAt) return null;
  const resetMs = Date.parse(status.resetsAt);
  if (!Number.isFinite(resetMs) || resetMs <= nowMs) return null;

  const minutes = Math.max(1, Math.ceil((resetMs - nowMs) / 60_000));
  const days = Math.floor(minutes / (24 * 60));
  const hours = Math.floor((minutes % (24 * 60)) / 60);
  const remainingMinutes = minutes % 60;
  if (days > 0) return `Resets in ${days}d ${hours}h`;
  if (hours > 0) return `Resets in ${hours}h ${remainingMinutes}m`;
  return `Resets in ${remainingMinutes}m`;
}
