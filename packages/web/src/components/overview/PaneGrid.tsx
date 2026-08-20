"use client";

/**
 * Phase 5.1b — pane grid for the Overview tab. One card per pane,
 * showing the at-a-glance roll-up the user described:
 *   label + role chip · status pill · mode · worktree · last-activity
 * Quick-action row: Diff / Role (modal opens) and a click-anywhere
 * "Open" affordance that switches the active tab.
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
  DEEPSEEK_DEFAULT_MODEL,
  isDeepseekModel,
  isRetiredProviderModel,
} from "@/lib/providerOptions";
import { ActivitySparkline } from "./ActivitySparkline";
import { useIsLaunchProfileAllowed } from "@/lib/tabTypes";

interface PaneGridProps {
  onOpenPane: (paneId: number) => void;
  onOpenDiff: (paneId: number) => void;
  onOpenRole: (paneId: number) => void;
  onPausePane: (paneId: number) => void;
  onResumePane: (paneId: number) => void;
  onRemovePane: (paneId: number) => void;
}

export function PaneGrid({
  onOpenPane,
  onOpenDiff,
  onOpenRole,
  onPausePane,
  onResumePane,
  onRemovePane,
}: PaneGridProps) {
  const paneConfigs = useStore((s) => s.paneConfigs);
  const paneStatuses = useStore((s) => s.paneStatuses);
  const pausedPanes = useStore((s) => s.pausedPanes);
  const paneMessages = useStore((s) => s.paneMessages);
  const paneDiffs = useStore((s) => s.paneDiffs);
  // One grid: the managed/unmanaged split existed for team mode, and a pane
  // still marked managed is now just a pane.
  const sorted = useMemo(() => {
    const arr = [...paneConfigs];
    arr.sort((a, b) => a.pane_id - b.pane_id);
    return arr;
  }, [paneConfigs]);

  if (sorted.length === 0) {
    return (
      <div className="rounded border border-dashed border-gray-300 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/30 p-3 text-xs italic text-gray-500 dark:text-gray-400">
        No panes. Click <strong>+</strong> in the tab bar to start one.
      </div>
    );
  }

  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
      {sorted.map((pane) => {
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
            isPaused={isPaused}
            messages={messages}
            lastActivity={lastMsg?.timestamp}
            diffStats={diff?.diff ? summarizeDiffStats(diff.diff) : undefined}
            onOpen={() => onOpenPane(pane.pane_id)}
            onOpenDiff={() => onOpenDiff(pane.pane_id)}
            onOpenRole={() => onOpenRole(pane.pane_id)}
            onPause={() => onPausePane(pane.pane_id)}
            onResume={() => onResumePane(pane.pane_id)}
            onRemove={() => onRemovePane(pane.pane_id)}
          />
        );
      })}
    </div>
  );
}

interface PaneCardProps {
  pane: PaneConfig;
  status: string | null;
  isPaused: boolean;
  messages: Message[];
  lastActivity?: Date;
  diffStats?: { added: number; removed: number };
  onOpen: () => void;
  onOpenDiff: () => void;
  onOpenRole: () => void;
  onPause: () => void;
  onResume: () => void;
  onRemove: () => void;
  /** Present only on unmanaged cards — one-way promote to the team. */
  onPromote?: () => void;
}

function PaneCard({
  pane,
  status,
  isPaused,
  messages,
  lastActivity,
  diffStats,
  onOpen,
  onOpenDiff,
  onOpenRole,
  onPause,
  onResume,
  onRemove,
  onPromote,
}: PaneCardProps) {
  const isBot = pane.mode === "deadloop";
  const isTerminal = pane.kind === "terminal";
  const isUnsupported = isRetiredProviderModel(pane.provider, pane.model);
  const isThinking = !!status && !isPaused;
  const modeIndicator = isUnsupported
    ? { icon: "!", label: "unsupported provider" }
    : isBot
    ? isPaused
      ? { icon: "⏸", label: "paused" }
      : { icon: "⏵", label: "running" }
    : isThinking
      ? { icon: "●", label: "thinking" }
      : { icon: "•", label: "idle" };
  const modeColor = isUnsupported
    ? "text-red-500"
    : isBot
    ? isPaused
      ? "text-amber-500"
      : "text-emerald-500"
    : isThinking
      ? "text-blue-500 animate-pulse"
      : "text-gray-400";
  const updatePaneModel = useStore((s) => s.updatePaneModel);
  const isLaunchProfileAllowed = useIsLaunchProfileAllowed();
  // Inline agent switcher. Mirrors the "+" new-tab menu's
  // frontend/backend grouping: the select picks an
  // agent-frontend × API-backend combo (e.g. "Claude / DeepSeek" =
  // claude CLI talking to deepseek's anthropic-compatible endpoint
  // via the env-override path).
  const AGENT_OPTS: ReadonlyArray<{
    value: string;
    label: string;
    /// Provider sent on the wire.
    provider: string;
    /// Model override for backend-swap agents; null clears the override.
    model: string | null;
  }> = [
    { value: "claude/official", label: "Claude / Official", provider: "claude", model: null },
    { value: "claude/deepseek", label: "Claude / DeepSeek", provider: "claude", model: DEEPSEEK_DEFAULT_MODEL },
    { value: "codex/official", label: "Codex / Official", provider: "codex", model: null },
    { value: "opencode/official", label: "OpenCode / Official", provider: "opencode", model: null },
    { value: "cursor-agent/official", label: "Cursor / Official", provider: "cursor-agent", model: null },
  ];
  // Reverse mapping from (provider, model) → agent option value, so the
  // dropdown reflects whichever combo the pane is actually running.
  // Claude with a backend-specific model classifies into the matching
  // sub-option even if the user typed a variant string.
  const currentAgentValue = ((): string => {
    const provider = pane.provider;
    if (isUnsupported) return "unsupported";
    if (provider === "claude") {
      if (isDeepseekModel(pane.model)) return "claude/deepseek";
      return "claude/official";
    }
    if (provider === "codex") return "codex/official";
    if (provider === "opencode") return "opencode/official";
    if (provider === "cursor-agent") return "cursor-agent/official";
    if (provider === "deepseek") return "claude/deepseek";
    return "unsupported";
  })();
  const visibleAgentOptions = AGENT_OPTS.filter(
    (option) =>
      option.value === currentAgentValue
      || isLaunchProfileAllowed("agent", option.provider, option.model ?? undefined),
  );
  const label = pane.label || `Tab ${pane.pane_id}`;
  return (
    <button
      type="button"
      onClick={onOpen}
      className="flex flex-col items-stretch gap-2 rounded border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800/60 p-3 text-left transition-colors hover:border-indigo-400 dark:hover:border-indigo-500"
      title={`Open ${label}`}
    >
      <div className="flex flex-wrap items-baseline gap-2">
        <span className={`flex-shrink-0 text-base ${modeColor}`} aria-label={modeIndicator.label}>
          {modeIndicator.icon}
        </span>
        <span className="font-semibold text-gray-900 dark:text-gray-100 truncate">{label}</span>
        {pane.role && (
          <span className="rounded bg-purple-100 dark:bg-purple-900/40 px-1.5 py-0.5 text-[10px] font-medium text-purple-700 dark:text-purple-300">
            {pane.role}
          </span>
        )}
        {/* v3.2: autonomous/manual toggle. Hidden for the legacy default
            panes and for Manager/Tech-Lead panes (those have their own
            semantics — Manager is always user-facing, Tech Lead is the
            orchestrator itself, not a delegation target). */}
        {!isUnsupported && !isTerminal && pane.pane_id !== PANE_ID_INTERACTIVE &&
          pane.pane_id !== PANE_ID_DEADLOOP &&
          !isManagerOrTechLead(pane.role) && (
            <WorkerModeToggle
              paneId={pane.pane_id}
              manualMode={pane.manual_mode === true}
            />
          )}
        {isUnsupported ? (
          <span className="ml-auto rounded bg-red-100 px-2 py-0.5 text-[10px] font-medium text-red-700 dark:bg-red-900/40 dark:text-red-300">
            Unsupported provider
          </span>
        ) : isTerminal ? (
          <span className="ml-auto rounded bg-gray-100 px-2 py-0.5 text-[10px] font-medium text-gray-600 dark:bg-gray-700 dark:text-gray-300">
            Terminal
          </span>
        ) : <span className="ml-auto flex items-center gap-1" onClick={(e) => e.stopPropagation()}>
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
              if (
                !canLiveSwap &&
                !confirm(
                  `Switch agent to ${picked.label}? The current turn will be interrupted and the agent respawns with a fresh context — chat history stays visible but is NOT in the new agent's prompt. Make sure the host running this pane has the required CLI installed + authenticated.`,
                )
              ) {
                return;
              }
              updatePaneModel(pane.pane_id, picked.model, picked.provider);
            }}
            className="rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 px-1 py-0 text-[10px] font-mono text-gray-700 dark:text-gray-300 focus:outline-none"
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

      {pane.goal && (
        <div
          className="line-clamp-2 text-xs text-gray-600 dark:text-gray-300"
          title={pane.goal}
        >
          {pane.goal}
        </div>
      )}

      {status && (
        <div className="rounded bg-blue-50 dark:bg-blue-900/30 px-2 py-1 text-xs text-blue-700 dark:text-blue-300">
          {status}
        </div>
      )}

      {pane.worktree_path && (
        <div className="flex items-center gap-2 text-[11px] text-gray-500 dark:text-gray-400">
          <span className="font-mono">apas-pane-{pane.pane_id}</span>
          {diffStats && (
            <>
              <span className="text-emerald-500">+{diffStats.added}</span>
              <span className="text-red-500">-{diffStats.removed}</span>
            </>
          )}
        </div>
      )}

      <div className="flex items-center gap-2 text-[11px] text-gray-500 dark:text-gray-400">
        <span>{lastActivity ? `last: ${formatRelative(lastActivity)}` : "no activity yet"}</span>
        <ActivitySparkline messages={messages} />
        <span className="ml-auto flex gap-1.5">
          {pane.worktree_path && (
            <span
              className="rounded border border-emerald-400 dark:border-emerald-700 bg-emerald-50 dark:bg-emerald-900/30 px-1.5 py-0.5 text-emerald-700 dark:text-emerald-300 hover:bg-emerald-100 dark:hover:bg-emerald-900/50"
              role="button"
              onClick={(e) => {
                e.stopPropagation();
                onOpenDiff();
              }}
            >
              Diff
            </span>
          )}
          {!isUnsupported && !isTerminal && (
            <span
              className="rounded border border-purple-400 dark:border-purple-700 bg-purple-50 dark:bg-purple-900/30 px-1.5 py-0.5 text-purple-700 dark:text-purple-300 hover:bg-purple-100 dark:hover:bg-purple-900/50"
              role="button"
              onClick={(e) => {
                e.stopPropagation();
                onOpenRole();
              }}
              title="Edit role / goal / backstory"
            >
              Role
            </span>
          )}
          {!isUnsupported && isBot && (
            isPaused ? (
              <span
                className="rounded border border-emerald-400 dark:border-emerald-700 bg-emerald-50 dark:bg-emerald-900/30 px-1.5 py-0.5 text-emerald-700 dark:text-emerald-300 hover:bg-emerald-100 dark:hover:bg-emerald-900/50"
                role="button"
                onClick={(e) => {
                  e.stopPropagation();
                  onResume();
                }}
                title="Resume this deadloop"
              >
                Resume
              </span>
            ) : (
              <span
                className="rounded border border-amber-400 dark:border-amber-700 bg-amber-50 dark:bg-amber-900/30 px-1.5 py-0.5 text-amber-700 dark:text-amber-300 hover:bg-amber-100 dark:hover:bg-amber-900/50"
                role="button"
                onClick={(e) => {
                  e.stopPropagation();
                  onPause();
                }}
                title="Pause this deadloop"
              >
                Pause
              </span>
            )
          )}
          {!isUnsupported && onPromote && (
            <span
              className="rounded border border-blue-500 bg-blue-600 px-1.5 py-0.5 text-white hover:bg-blue-500"
              role="button"
              onClick={(e) => {
                e.stopPropagation();
                onPromote();
              }}
              title="Promote to a team member — Tech Lead may delegate work to it. One-way; you can't demote back."
            >
              + Add to team
            </span>
          )}
          {(isUnsupported || (
            pane.pane_id !== PANE_ID_INTERACTIVE &&
            pane.pane_id !== PANE_ID_DEADLOOP &&
            !(pane.managed && isManagerOrTechLead(pane.role))
          )) && pane.pane_id !== PANE_ID_INTERACTIVE &&
            pane.pane_id !== PANE_ID_DEADLOOP && (
              <span
                className="rounded border border-red-400 dark:border-red-700 bg-red-50 dark:bg-red-900/30 px-1.5 py-0.5 text-red-700 dark:text-red-300 hover:bg-red-100 dark:hover:bg-red-900/50"
                role="button"
                onClick={(e) => {
                  e.stopPropagation();
                  onRemove();
                }}
                title="Close this pane (worktree-cleanup prompt opens if applicable)"
              >
                Remove
              </span>
            )}
          {(pane.pane_id === PANE_ID_INTERACTIVE || pane.pane_id === PANE_ID_DEADLOOP) && (
            <span
              className="rounded bg-gray-100 dark:bg-gray-700 px-1.5 py-0.5 text-[10px] text-gray-500 dark:text-gray-400"
              title="Built-in default pane (can't be removed)"
            >
              default
            </span>
          )}
          {isManagerRole(pane.role) &&
            pane.pane_id !== PANE_ID_INTERACTIVE &&
            pane.pane_id !== PANE_ID_DEADLOOP && (
              <span
                className="rounded bg-violet-100 dark:bg-violet-900/40 px-1.5 py-0.5 text-[10px] text-violet-700 dark:text-violet-300"
                title="Manager pane — user-facing role; controlled from the Overview panel"
              >
                manager
              </span>
            )}
          {isTechLeadRole(pane.role) &&
            pane.pane_id !== PANE_ID_INTERACTIVE &&
            pane.pane_id !== PANE_ID_DEADLOOP && (
              <span
                className="rounded bg-indigo-100 dark:bg-indigo-900/40 px-1.5 py-0.5 text-[10px] text-indigo-700 dark:text-indigo-300"
                title="Tech Lead pane — autonomous orchestrator; controlled from the Overview panel"
              >
                tech lead
              </span>
            )}
        </span>
      </div>
    </button>
  );
}

// v3 role detection — Manager and Tech Lead panes are managed from the
// Overview's Team panel (Pause/Resume/Edit goal), so PaneGrid hides the
// Remove button on them to avoid accidental destruction.
function isManagerRole(role: string | undefined): boolean {
  if (!role) return false;
  const lower = role.toLowerCase();
  return lower.includes("manager") && !lower.includes("tech lead");
}

function isTechLeadRole(role: string | undefined): boolean {
  if (!role) return false;
  return role.toLowerCase().includes("tech lead");
}

function isManagerOrTechLead(role: string | undefined): boolean {
  return isManagerRole(role) || isTechLeadRole(role);
}

// v3.2: per-worker autonomous/manual toggle. Autonomous = Tech Lead may
// delegate to this pane via .apas-team.jsonl. Manual = pane reserved for
// direct user conversation; Tech Lead skips it.
function WorkerModeToggle({
  paneId,
  manualMode,
}: {
  paneId: number;
  manualMode: boolean;
}) {
  const updatePaneManualMode = useStore((s) => s.updatePaneManualMode);
  const onClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    updatePaneManualMode(paneId, !manualMode);
  };
  return manualMode ? (
    <span
      role="button"
      onClick={onClick}
      className="cursor-pointer rounded border border-rose-300 bg-rose-50 px-1.5 py-0.5 text-[10px] font-medium text-rose-700 hover:bg-rose-100 dark:border-rose-800 dark:bg-rose-900/30 dark:text-rose-300 dark:hover:bg-rose-900/50"
      title="Manual mode: only the user chats with this worker; Tech Lead won't delegate. Click to flip to autonomous."
    >
      👤 manual
    </span>
  ) : (
    <span
      role="button"
      onClick={onClick}
      className="cursor-pointer rounded border border-sky-300 bg-sky-50 px-1.5 py-0.5 text-[10px] font-medium text-sky-700 hover:bg-sky-100 dark:border-sky-800 dark:bg-sky-900/30 dark:text-sky-300 dark:hover:bg-sky-900/50"
      title="Autonomous mode: Tech Lead may delegate work to this pane. Click to flip to manual."
    >
      🤖 auto
    </span>
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

function formatRelative(when: Date): string {
  const seconds = Math.floor((Date.now() - when.getTime()) / 1000);
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}
