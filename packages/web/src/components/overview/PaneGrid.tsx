"use client";

/**
 * The Overview's "Tabs" section: one card per tab of the project.
 *
 * Every card has the same parts in the same order: a status dot, the tab's
 * name and what it runs; one status line; the autopilot line, the goal and the
 * worktree when the tab has them; then an activity strip and the card's
 * actions. The name opens the tab, and so does a click anywhere on the card.
 */
import { useMemo } from "react";
import {
  Message,
  PaneConfig,
  useStore,
  paneKey,
  PANE_ID_DEADLOOP,
  PANE_ID_INTERACTIVE,
} from "@/lib/store";
import {
  isDeepseekModel,
  isRetiredProviderModel,
  providerModelValue,
  PROVIDER_MODEL_OPTIONS,
} from "@/lib/providerOptions";
import { ActivitySparkline } from "./ActivitySparkline";
import { paneIsAwaitingAnswerStatus } from "@/lib/paneStatus";
import { useIsLaunchProfileAllowed } from "@/lib/tabTypes";
import { useSessionAutopilotStatuses } from "@/lib/useAutopilot";
import type { AutopilotListStatus } from "@/lib/autopilot";
import { providerModelDisplayName } from "@/lib/launchProfileLabels";
import { formatRelativeTime } from "@/lib/formatRelativeTime";
import { tabNotRunningLabel, type TabHost } from "@/lib/tabHosts";
import { OverviewEmpty, overviewCard, overviewChip, overviewQuietButton, overviewQuietDangerButton } from "./OverviewElements";

interface PaneGridProps {
  onOpenPane: (paneId: number) => void;
  onOpenDiff: (paneId: number) => void;
  onOpenRole: (paneId: number) => void;
  onRemovePane: (paneId: number) => void;
  /** Where each tab runs (`describeTabHosts`). A tab whose machine is not
   *  running the project reads "Not running on <host>", not "Idle". */
  tabHosts?: Map<number, TabHost>;
  /** Whether anything runs the project. When nothing does and nothing is
   *  known per tab, every card reads "Not running". Assumed when absent. */
  projectRunning?: boolean;
}

const OVERVIEW_AGENT_OPTION_VALUES = new Set([
  "claude/official",
  "claude/deepseek",
  "claude/deepseek-flash",
  "codex/official",
  "opencode/official",
  "cursor-agent/official",
]);

const AGENT_OPTS = PROVIDER_MODEL_OPTIONS
  .filter((option) => OVERVIEW_AGENT_OPTION_VALUES.has(option.value))
  .map((option) => ({ ...option, model: option.model ?? null }));

export function PaneGrid({
  onOpenPane,
  onOpenDiff,
  onOpenRole,
  onRemovePane,
  tabHosts,
  projectRunning = true,
}: PaneGridProps) {
  const paneConfigs = useStore((s) => s.paneConfigs);
  const paneStatuses = useStore((s) => s.paneStatuses);
  const pausedPanes = useStore((s) => s.pausedPanes);
  const paneMessages = useStore((s) => s.paneMessages);
  const paneDiffs = useStore((s) => s.paneDiffs);
  const sessionId = useStore((s) => s.sessionId);
  const usageStats = useStore((s) => s.usageStats);
  const autopilotStatuses = useSessionAutopilotStatuses(sessionId, paneConfigs);
  // The server's record of when each tab last did something. The messages in
  // this browser cover only the tabs that were opened here, so a card that
  // read them alone said "no activity yet" for a tab that had just worked.
  const reportedActivity = useMemo(() => {
    const byPane = new Map<number, Date>();
    for (const pane of (sessionId ? usageStats[sessionId]?.panes : undefined) ?? []) {
      const at = pane.last_active ? Date.parse(pane.last_active) : Number.NaN;
      if (!Number.isNaN(at)) byPane.set(pane.pane_id, new Date(at));
    }
    return byPane;
  }, [sessionId, usageStats]);

  if (paneConfigs.length === 0) {
    return <OverviewEmpty>No tabs yet. Use + in the tab bar to open one.</OverviewEmpty>;
  }

  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
      {/* One grid, in the tab bar's order: these are the same tabs, and a tab
          must not sit in a different place here than in the bar above. Pane
          ids are not creation order (they are reused, and assigned per
          machine), so sorting by id shuffled the cards. A card also keeps its
          place when its tab starts or stops working, so it never moves from
          under a pointer. */}
      {paneConfigs.map((pane) => {
        const status = paneStatuses[paneKey(pane.pane_id)];
        const isPaused = pausedPanes.includes(pane.pane_id);
        const messages = paneMessages[paneKey(pane.pane_id)] ?? [];
        const lastMsg = messages[messages.length - 1];
        const diff = paneDiffs[pane.pane_id];
        return (
          <PaneCard
            key={pane.pane_id}
            pane={pane}
            status={status ?? null}
            notRunning={tabNotRunningLabel(tabHosts?.get(pane.pane_id), projectRunning)}
            isPaused={isPaused}
            autopilot={autopilotStatuses.get(pane.pane_id)}
            messages={messages}
            lastActivity={latest(lastMsg?.timestamp, reportedActivity.get(pane.pane_id))}
            diffStats={diff?.diff ? summarizeDiffStats(diff.diff) : undefined}
            onOpen={() => onOpenPane(pane.pane_id)}
            onOpenDiff={() => onOpenDiff(pane.pane_id)}
            onOpenRole={() => onOpenRole(pane.pane_id)}
            onRemove={() => onRemovePane(pane.pane_id)}
          />
        );
      })}
    </div>
  );
}

function latest(a?: Date, b?: Date): Date | undefined {
  if (!a) return b;
  if (!b) return a;
  return a.getTime() >= b.getTime() ? a : b;
}

interface PaneCardProps {
  pane: PaneConfig;
  status: string | null;
  /** "Not running on zoo-005": the tab cannot be doing anything, whatever
   *  status it last reported. Absent for a tab that runs. */
  notRunning?: string;
  isPaused: boolean;
  /** The tab's autopilot, when it has one to show: its condition, or why it
   *  paused or ended. Shown beside the status, never in its place. */
  autopilot?: AutopilotListStatus;
  messages: Message[];
  lastActivity?: Date;
  diffStats?: { added: number; removed: number };
  onOpen: () => void;
  onOpenDiff: () => void;
  onOpenRole: () => void;
  onRemove: () => void;
}

function PaneCard({
  pane,
  status,
  notRunning,
  isPaused,
  autopilot,
  messages,
  lastActivity,
  diffStats,
  onOpen,
  onOpenDiff,
  onOpenRole,
  onRemove,
}: PaneCardProps) {
  const isTerminal = pane.kind === "terminal";
  const readOnlyHistory = pane.mode === "deadloop";
  const mappedAgentValue = providerModelValue(pane.provider, pane.model);
  // Fable remains an official-Claude model on this combined backend control;
  // this change only adds the two explicit DeepSeek variants.
  const currentAgentValue = mappedAgentValue === "claude/fable"
    ? "claude/official"
    : mappedAgentValue;
  const isUnsupported = isRetiredProviderModel(pane.provider, pane.model)
    || currentAgentValue === "unsupported";
  // A tab that is not running awaits nothing and thinks nothing: its last
  // reported status is stale.
  const isAwaitingAnswer = !notRunning && paneIsAwaitingAnswerStatus(status) && !isPaused;
  const isThinking = !notRunning && !!status && !isPaused && !isAwaitingAnswer;
  // The dot's label is the tab's state in a word; the status line beside it
  // carries what the tab itself reports.
  const modeIndicator = readOnlyHistory
    ? { label: "earlier bot history", dot: "bg-gray-300 dark:bg-gray-600" }
    : isUnsupported
    ? { label: "unsupported provider", dot: "bg-red-500" }
    : notRunning
    ? { label: "not running", dot: "bg-gray-300 dark:bg-gray-600" }
    : isAwaitingAnswer
    ? { label: "pending answer", dot: "bg-amber-500" }
    : isThinking
      ? { label: "thinking", dot: "bg-blue-500 animate-pulse" }
      : { label: "idle", dot: "bg-gray-300 dark:bg-gray-600" };
  const statusText = notRunning ?? (status || (isPaused ? "Paused" : "Idle"));
  const statusColor = isAwaitingAnswer
    ? "text-amber-700 dark:text-amber-300"
    : isThinking
      ? "text-blue-700 dark:text-blue-300"
      : "text-gray-700 dark:text-gray-300";
  const updatePaneModel = useStore((s) => s.updatePaneModel);
  const isLaunchProfileAllowed = useIsLaunchProfileAllowed();
  // Inline agent switcher. Mirrors the "+" new-tab menu's
  // frontend/backend grouping: the select picks an
  // agent-frontend × API-backend combo (e.g. "Claude / DeepSeek" =
  // claude CLI talking to deepseek's anthropic-compatible endpoint
  // via the env-override path).
  // Reverse mapping from (provider, model) → agent option value, so the
  // dropdown reflects whichever combo the pane is actually running.
  const visibleAgentOptions = AGENT_OPTS.filter(
    (option) =>
      option.value === currentAgentValue
      || isLaunchProfileAllowed("agent", option.provider, option.model ?? undefined),
  );
  const label = pane.label || `Tab ${pane.pane_id}`;
  const runs = providerModelDisplayName(pane.provider, pane.model);
  const builtIn = pane.pane_id === PANE_ID_INTERACTIVE || pane.pane_id === PANE_ID_DEADLOOP;
  // The card opens its tab wherever it is clicked. The name is the real
  // button, so the same thing is reachable from the keyboard; the controls
  // inside stop their own clicks from also opening the tab.
  return (
    <div
      onClick={onOpen}
      title={`Open ${label}`}
      className={`${overviewCard} flex min-w-0 cursor-pointer flex-col gap-2 p-3 transition-colors hover:border-blue-400 dark:hover:border-blue-500`}
    >
      <div className="flex min-w-0 items-center gap-2">
        <span
          role="img"
          aria-label={modeIndicator.label}
          className={`h-2 w-2 shrink-0 rounded-full ${modeIndicator.dot}`}
        />
        <h4 className="min-w-0 flex-1 text-sm font-semibold text-gray-900 dark:text-gray-100">
          <button
            type="button"
            aria-label={`Open ${label}`}
            onClick={(e) => {
              e.stopPropagation();
              onOpen();
            }}
            className="block max-w-full truncate rounded text-left hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-500"
          >
            {label}
          </button>
        </h4>
        {pane.role && (
          <span className={`${overviewChip} max-w-28 truncate`} title={`Role: ${pane.role}`}>
            {pane.role}
          </span>
        )}
        {readOnlyHistory ? (
          <span className={overviewChip}>Earlier bot · Read-only</span>
        ) : isUnsupported ? (
          <span className="shrink-0 rounded bg-red-100 px-1.5 py-0.5 text-[11px] font-medium text-red-700 dark:bg-red-900/40 dark:text-red-300">
            Unsupported provider
          </span>
        ) : isTerminal ? (
          <span className={`${overviewChip} max-w-40 truncate`} title={`${runs ?? pane.provider} terminal tab`}>
            {runs ?? pane.provider}
          </span>
        ) : <span className="flex shrink-0 items-center" onClick={(e) => e.stopPropagation()}>
          <select
            value={currentAgentValue}
            onChange={(e) => {
              const next = e.target.value;
              if (next === currentAgentValue) return;
              const picked = AGENT_OPTS.find((o) => o.value === next);
              if (!picked) return;
              // Live-swap eligibility mirrors the CLI's fast path in
              // UpdatePaneModel: pane stays on Claude AND neither the
              // old nor the new model uses DeepSeek's backend bridge.
              const isBackendSwap = (model?: string | null) =>
                isDeepseekModel(model);
              const stayingOnClaude =
                pane.provider === "claude" && picked.provider === "claude";
              const canLiveSwap =
                stayingOnClaude &&
                !isBackendSwap(pane.model) &&
                !isBackendSwap(picked.model) &&
                picked.model != null;
              const isDeepseekVariantSwitch =
                isDeepseekModel(pane.model) && isDeepseekModel(picked.model);
              const confirmation = isDeepseekVariantSwitch
                ? `Switch agent to ${picked.label}? The current turn will be interrupted and Claude restarts with fresh prompt context. Chat history stays visible in APAS but is NOT part of the new agent's prompt.`
                : `Switch agent to ${picked.label}? The current turn will be interrupted and the agent respawns with a fresh context — chat history stays visible but is NOT in the new agent's prompt. Make sure the host running this pane has the required CLI installed + authenticated.`;
              if (
                !canLiveSwap &&
                !confirm(confirmation)
              ) {
                return;
              }
              updatePaneModel(pane.pane_id, picked.model, picked.provider);
            }}
            className="h-6 max-w-40 rounded border border-gray-300 bg-white px-1 text-[11px] font-medium text-gray-700 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-500 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-300"
            title="Agent frontend / API backend — Claude→Claude with no backend swap is a live swap (next turn takes effect); provider change or backend swap respawns with a fresh session id"
          >
            {visibleAgentOptions.map((o) => (
              <option
                key={o.value}
                value={o.value}
                disabled={!isLaunchProfileAllowed("agent", o.provider, o.model ?? undefined)}
              >
                {o.label}
              </option>
            ))}
          </select>
        </span>}
      </div>

      <p className="flex flex-wrap items-baseline gap-x-1.5 text-xs">
        <span className={`font-medium ${statusColor}`}>{statusText}</span>
        <span className="text-gray-500 dark:text-gray-400" title={lastActivity?.toLocaleString()}>
          {lastActivity ? `· Last active ${formatRelativeTime(lastActivity)}` : "· No activity yet"}
        </span>
      </p>

      {autopilot && (
        <div
          data-testid="pane-card-autopilot"
          className={`rounded px-2 py-1 text-xs ${
            autopilot.kind === "on"
              ? "bg-indigo-50 text-indigo-700 dark:bg-indigo-900/30 dark:text-indigo-300"
              : autopilot.kind === "paused"
                ? "bg-amber-50 text-amber-800 dark:bg-amber-900/30 dark:text-amber-200"
                : "bg-gray-100 text-gray-600 dark:bg-gray-700/50 dark:text-gray-300"
          }`}
        >
          Autopilot · {autopilot.text}
        </div>
      )}

      {pane.goal && (
        <p
          className="line-clamp-2 text-xs text-gray-600 dark:text-gray-300"
          title={pane.goal}
        >
          {pane.goal}
        </p>
      )}

      {pane.worktree_path && (
        <p className="flex items-center gap-2 text-xs text-gray-500 dark:text-gray-400">
          <span className="truncate font-mono text-[11px]">apas-pane-{pane.pane_id}</span>
          {diffStats && (
            <>
              <span className="text-emerald-600 dark:text-emerald-400">+{diffStats.added}</span>
              <span className="text-red-600 dark:text-red-400">-{diffStats.removed}</span>
            </>
          )}
        </p>
      )}

      <div className="mt-auto flex items-center gap-1 pt-1">
        <span className="mr-auto flex min-w-0 items-center">
          <ActivitySparkline messages={messages} />
        </span>
        {pane.worktree_path && (
          <button
            type="button"
            className={overviewQuietButton}
            onClick={(e) => {
              e.stopPropagation();
              onOpenDiff();
            }}
            title="Show this tab's changes in its worktree"
          >
            Diff
          </button>
        )}
        {!readOnlyHistory && !isUnsupported && !isTerminal && (
          <button
            type="button"
            className={overviewQuietButton}
            onClick={(e) => {
              e.stopPropagation();
              onOpenRole();
            }}
            title="Edit role / goal / backstory"
          >
            Role
          </button>
        )}
        {!builtIn && (
          <button
            type="button"
            aria-label={`Close tab ${label}`}
            className={overviewQuietDangerButton}
            onClick={(e) => {
              e.stopPropagation();
              onRemove();
            }}
            title="Close this pane (worktree-cleanup prompt opens if applicable)"
          >
            Close tab
          </button>
        )}
        {builtIn && (
          <span className={overviewChip} title="Built-in default pane (can't be removed)">
            default
          </span>
        )}
      </div>
    </div>
  );
}

function summarizeDiffStats(diff: string): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const line of diff.split("\n")) {
    if (line.startsWith("+") && !line.startsWith("+++")) added++;
    else if (line.startsWith("-") && !line.startsWith("---")) removed++;
  }
  return { added, removed };
}
