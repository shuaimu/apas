"use client";

import React, { useRef, useCallback, useEffect, useState, useMemo, memo } from "react";
import dynamic from "next/dynamic";
import { useStore, Message, PaneConfig, PaneCleanupAction, PaneKind, PlanReviewMode, TeamRecord, PANE_ID_DEADLOOP, PANE_ID_INTERACTIVE, paneKey, selectActiveTeamRecords } from "@/lib/store";
import { ROLE_TEMPLATES, TEMPLATE_COLOR_CLASSES } from "@/lib/roleTemplates";
import { extractTimeline, TimelineEntry } from "@/lib/timeline";
import { OverviewView } from "../overview/OverviewView";
import { UserMessage } from "../chat/UserMessage";
import { AssistantMessage } from "../chat/AssistantMessage";
import { ToolGroupCard, groupMessagesForRender, isToolLikeMessage } from "../chat/ToolGroupCard";
import { TabBar } from "./TabBar";
import { WorkerTaskBar } from "./WorkerTaskBar";
import { UsageLimitsDisplay } from "../UsageLimits";
import { CodeBlock } from "../code/CodeBlock";

// xterm.js touches `document` at import time, so it can't be part of the
// server bundle. Loading it lazily also keeps the ~300 KB emulator out of
// the initial payload for the (common) case of no terminal panes.
import { TerminalViewToggle } from "./TerminalViewToggle";
import { TerminalChatInput } from "./TerminalChatInput";

/**
 * A terminal pane with its two views.
 *
 * Its own component because `useTerminalViewMode` is a hook and the tab list is
 * rendered inside a `.map` — calling a hook there would break the rules-of-hooks
 * ordering the moment the tab count changed (React error #310, which takes the
 * whole app down).
 */
function TerminalPaneWithViews({
  sessionId,
  paneId,
  messages,
}: {
  sessionId: string | null;
  paneId: number;
  messages: Message[];
}) {
  const [mode, setMode] = useTerminalViewMode(sessionId, paneId);
  return (
    <>
      <TerminalViewToggle mode={mode} onChange={setMode} turnCount={messages.length} />
      {/* Hidden rather than unmounted — see the call site. */}
      <div className={mode === "terminal" ? "flex-1 flex flex-col min-h-0" : "hidden"}>
        <TerminalPane key={`terminal-${sessionId}-${paneId}`} paneId={paneId} />
      </div>
      {mode === "conversation" && (
        <>
          <MessagePane
            key={`terminal-chat-${sessionId}-${paneId}`}
            paneId={paneId}
            messages={messages}
            isActive
          />
          {/* Writes go into the pty, not through MCP — a tool server can only
              answer calls, never push a turn into a live conversation. */}
          <TerminalChatInput paneId={paneId} />
        </>
      )}
    </>
  );
}

import { useTerminalViewMode } from "@/lib/terminalViewMode";

const TerminalPane = dynamic(
  () => import("./TerminalPane").then((m) => m.TerminalPane),
  {
    ssr: false,
    loading: () => (
      <div className="flex-1 bg-[#0a0a0a] p-3 font-mono text-xs text-neutral-500">
        Loading terminal…
      </div>
    ),
  },
);

// Sentinel pane_id for the single-pane fallback (no pane system)
const PANE_ID_MAIN = 0;
// Sentinel for the Overview pseudo-tab (Phase 5.1). Negative so it
// can't collide with real pane ids (Rust-side u32, dynamic ids start
// from 3) and survives the `parseInt` round-trip in localStorage.
export const OVERVIEW_PANE_ID = -1;
const DEFAULT_BOT_MIN_INTERVAL_MINUTES = 15;
const PANE_REBOOT_CONFIRM_MESSAGE =
  "Reboot this pane's agent? The running process is killed and respawned on the SAME session (so the agent resumes with its prior conversation context).";
export const CLASSIC_TODO_BOT_LOOP_PROMPT = `Work on tasks defined in TODO.md. Do the following steps. Don't ask me for advice, just pick the best option you think that is honest, complete, and not corner-cutting:

1. Do a git pull to check if there are any remote updates. Pick the top high-priority undone task, choose its first leaf task. If there are no undone TODO items left, sleep a minute and exit.
2. Analyze the task, check if this can be done with not too many LOC (i.e., smaller than 500 lines code give or take). If not, try to analyze this task and break it down into several smaller tasks, expanding it in the TODO.md. The breakdown can be nested and hierarchical. Try to make each leaf task small enough (<500 lines LOC). You can document your analysis in the doc folder for future reference.
3. Try to execute the first leaf task. Make a plan for the task before execute. You can document key findings in either the TODO.md (a few sentences in the TODO item, or doc it in the docs folder for longer details and discussions.
4. Make sure to add comprehensive test for the task executed. Run the whole test suites to make sure no regression happens. If tests fail, fix them using the best, honest, complete approach, run test suites again to verify fixes work. Repeat this step until no tests fail.
5. Prepare for git commit, remove all temporary files, especially not to commit any binary files. For plan files, remove the implementation plan and keep the design rational and user manual and put it in the docs folder.
6. Git commit the changes. First do git pull --rebase, and fix conflicts if any. Then do git push.`;

type BotPromptPaneConfig = Pick<
  PaneConfig,
  "prompt" | "managed" | "role" | "goal" | "backstory"
>;

export function managedTeamBotPromptForPane(
  config: BotPromptPaneConfig | undefined,
): string | null {
  if (config?.managed !== true) return null;
  const role = config.role?.trim();
  const goal = config.goal?.trim();
  const backstory = config.backstory?.trim();
  const roleLine = role ? `Role: ${role}` : "Role: managed team worker";
  const goalLine = goal ? `Goal: ${goal}` : "Goal: Work only from delegated team tasks.";
  const backstoryBlock = backstory ? `\n\nBackstory and constraints:\n${backstory}` : "";

  return `You are this project's managed team worker.\n\n${roleLine}\n${goalLine}${backstoryBlock}\n\nUse the team-mode workflow: read project_goal.md, team-todo.md, and .apas-team.jsonl; act only on delegated work for this pane; publish diff/review/decision records on the team scratchpad as appropriate.`;
}

export function defaultBotPromptForPane(
  config: BotPromptPaneConfig | undefined,
): string {
  return managedTeamBotPromptForPane(config) ?? CLASSIC_TODO_BOT_LOOP_PROMPT;
}

export function botPromptForPane(
  config: BotPromptPaneConfig | undefined,
): string {
  const savedPrompt = config?.prompt;
  if (savedPrompt && savedPrompt.trim().length > 0) return savedPrompt;
  return defaultBotPromptForPane(config);
}

// Store scroll positions per session+pane combination
interface ScrollState {
  scrollTop: number;
  wasAtBottom: boolean;
  scrollHeight: number;
  clientHeight: number;
}
const scrollPositions = new Map<string, ScrollState>();

function getScrollKey(sessionId: string | null, paneId: number): string {
  return `${sessionId || "none"}-${paneId}`;
}

/// Decide how MessagePane should react to a `messages.length` change.
///
/// - "none": don't touch scroll. Pane is hidden, the user has scrolled
///   away from the bottom, or another effect is mid-restore.
/// - "instant": snap directly to `scrollHeight`. Used for the bulk
///   initial cache restore on hard refresh and for catchup batches —
///   a smooth animation across hundreds of messages takes seconds, and
///   if anything appends mid-animation the target bottom shifts further
///   down while the animation is still headed for the OLD bottom,
///   stranding the user in the middle of the chat.
/// - "smooth": eased scrollIntoView for a normal stream-message arrival
///   so the chat slides down gracefully.
export type AutoScrollMode = "none" | "instant" | "smooth";

export function decideAutoScrollMode(args: {
  isActive: boolean;
  shouldAutoScroll: boolean;
  isRestoringScroll: boolean;
  prevCount: number;
  newCount: number;
}): AutoScrollMode {
  if (!args.isActive) return "none";
  if (!args.shouldAutoScroll) return "none";
  if (args.isRestoringScroll) return "none";
  const grew = args.newCount - args.prevCount;
  if (args.prevCount === 0 || grew > 3) return "instant";
  return "smooth";
}

// Per-project layout persistence helpers
function getProjectLayoutKey(cliClientId: string | null, key: string): string {
  return cliClientId ? `apas_layout_${cliClientId}_${key}` : `apas_layout_global_${key}`;
}

function getProjectLayout(cliClientId: string | null, key: string, defaultValue: string): string {
  if (typeof window === "undefined") return defaultValue;
  if (cliClientId) {
    const v = localStorage.getItem(getProjectLayoutKey(cliClientId, key));
    if (v !== null) return v;
  }
  const g = localStorage.getItem(`apas_layout_global_${key}`);
  return g !== null ? g : defaultValue;
}

function setProjectLayout(cliClientId: string | null, key: string, value: string): void {
  if (typeof window === "undefined") return;
  localStorage.setItem(getProjectLayoutKey(cliClientId, key), value);
}

export function deriveInitialActiveTabId(args: {
  activeTabId: number | null;
  clientChanged: boolean;
  managerTabId: number | null;
  paneConfigsLength: number;
  savedActiveTab: string;
  tabIds: number[];
}): number | null {
  if (args.tabIds.length === 0) return null;

  const isValidTab = (paneId: number) =>
    paneId === OVERVIEW_PANE_ID || args.tabIds.includes(paneId);

  // Same project, current pick is still valid -> no change.
  if (
    !args.clientChanged &&
    args.activeTabId != null &&
    isValidTab(args.activeTabId)
  ) {
    return args.activeTabId;
  }
  // Same project, pane_list is synthesized (no authoritative panes yet)
  // -> keep current selection to avoid jumps from transient data.
  if (
    !args.clientChanged &&
    args.activeTabId != null &&
    args.paneConfigsLength === 0
  ) {
    return args.activeTabId;
  }

  const savedNum = args.savedActiveTab ? parseInt(args.savedActiveTab, 10) : NaN;
  if (!isNaN(savedNum) && isValidTab(savedNum)) {
    return savedNum;
  }

  // Persisted pref is gone or no longer matches a real pane. Prefer the
  // Manager pane (chat-first landing); otherwise land on the Overview
  // pseudo-tab where Start Manager and the project-goal input live.
  return args.managerTabId ?? OVERVIEW_PANE_ID;
}

export function lazyPaneMessageLoadTargets(args: {
  activeTabId: number | null;
  tabIds: number[];
}): number[] {
  if (args.activeTabId == null) return [];
  if (args.activeTabId === OVERVIEW_PANE_ID) return [];
  if (!args.tabIds.includes(args.activeTabId)) return [];
  return [args.activeTabId];
}

export function shouldShowPaneRebootButton(activeTabId: number | null): boolean {
  return activeTabId != null && activeTabId !== OVERVIEW_PANE_ID;
}

export function confirmedPaneRebootTarget(
  activeTabId: number | null,
  confirmFn: (message: string) => boolean,
): number | null {
  if (!shouldShowPaneRebootButton(activeTabId)) return null;
  return confirmFn(PANE_REBOOT_CONFIRM_MESSAGE) ? activeTabId : null;
}

export function requestConfirmedPaneReboot(
  activeTabId: number | null,
  confirmFn: (message: string) => boolean,
  rebootPane: (paneId: number) => void,
): void {
  const target = confirmedPaneRebootTarget(activeTabId, confirmFn);
  if (target == null) return;
  rebootPane(target);
}

function getInputDraftStorageKey(sessionId: string | null, paneId: number): string {
  return `apas_input_draft_${sessionId || "none"}_${paneId}`;
}

function loadInputDraft(sessionId: string | null, paneId: number): string {
  if (typeof window === "undefined") return "";
  return localStorage.getItem(getInputDraftStorageKey(sessionId, paneId)) ?? "";
}

function persistInputDraft(sessionId: string | null, paneId: number, value: string): void {
  if (typeof window === "undefined") return;
  const key = getInputDraftStorageKey(sessionId, paneId);
  if (!value.trim()) {
    localStorage.removeItem(key);
  } else {
    localStorage.setItem(key, value);
  }
}

function normalizeComparablePath(path: string | undefined): string | null {
  if (!path) return null;
  const trimmed = path.trim();
  if (!trimmed) return null;
  let normalized = trimmed;
  while (normalized.length > 1 && normalized.endsWith("/")) {
    normalized = normalized.slice(0, -1);
  }
  return normalized;
}

function isMiniMaxModel(model?: string): boolean {
  if (typeof model !== "string") return false;
  const normalized = model.trim().toLowerCase();
  return normalized.includes("minimax") || normalized.startsWith("m2");
}

function isGlmModel(model?: string): boolean {
  if (typeof model !== "string") return false;
  const normalized = model.trim().toLowerCase();
  return normalized.startsWith("glm") || normalized.includes("glm-");
}

function isDeepseekModel(model?: string): boolean {
  if (typeof model !== "string") return false;
  const normalized = model.trim().toLowerCase();
  return normalized.includes("deepseek");
}

// Listed lowest → highest. xhigh sits between high and max (Opus-only
// extra-deep tier); max is the highest level. `ultracode` is a special
// top-of-list entry — it is NOT a strict effort tier but an apas-only
// workflow (xhigh wire flag + auto multi-agent prompt prefix), so it
// renders at the top of the dropdown above max despite breaking the
// ordering invariant.
const CLAUDE_EFFORT_OPTIONS = [
  { value: "ultracode", label: "UltraCode" },
  { value: "default", label: "Default" },
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "xhigh", label: "XHigh" },
  { value: "max", label: "Max" },
] as const;
type ClaudeEffortOption = (typeof CLAUDE_EFFORT_OPTIONS)[number]["value"];

/// User-visible Claude model choices for the per-pane switcher.
/// The `value` is what gets passed to `claude --model` (claude CLI
/// understands short names like `sonnet` / `opus` / `haiku` plus the
/// fully-qualified `claude-<family>-<version>` IDs). Default = let
/// claude pick (currently sonnet).
const CLAUDE_MODEL_OPTIONS = [
  { value: "default", label: "Default" },
  // Fable has no short alias in the claude CLI yet, so pass the
  // fully-qualified ID (the CLI accepts both forms).
  { value: "claude-fable-5", label: "Fable" },
  { value: "sonnet", label: "Sonnet" },
  { value: "opus", label: "Opus" },
  { value: "haiku", label: "Haiku" },
] as const;
type ClaudeModelOption = (typeof CLAUDE_MODEL_OPTIONS)[number]["value"];

/// Provider switcher options. Maps to `shared::Provider` on the wire
/// (serde rename_all=snake_case, plus `cursor-agent` serializing as
/// that explicit string). Order: Claude first (default), then the
/// alternatives, since most users start on Claude and reach for the
/// switcher to spread load across other agents.
const PROVIDER_OPTIONS = [
  { value: "claude", label: "Claude" },
  { value: "codex", label: "Codex" },
  { value: "cursor-agent", label: "Cursor" },
  { value: "opencode", label: "OpenCode" },
  { value: "minimax", label: "MiniMax" },
  { value: "glm", label: "GLM" },
  { value: "deepseek", label: "DeepSeek" },
] as const;
type ProviderOption = (typeof PROVIDER_OPTIONS)[number]["value"];
const PROVIDER_LABEL: Record<ProviderOption, string> = Object.fromEntries(
  PROVIDER_OPTIONS.map((o) => [o.value, o.label]),
) as Record<ProviderOption, string>;

function normalizeClaudeModelOption(raw?: string | null): ClaudeModelOption {
  if (typeof raw !== "string") return "default";
  const normalized = raw.trim().toLowerCase();
  for (const opt of CLAUDE_MODEL_OPTIONS) {
    if (opt.value === normalized) return opt.value;
  }
  // Tolerate fully-qualified IDs like `claude-sonnet-4-6` by matching
  // the family substring; falls back to "default" for anything truly
  // foreign so the dropdown doesn't render a blank.
  if (normalized.includes("fable")) return "claude-fable-5";
  if (normalized.includes("sonnet")) return "sonnet";
  if (normalized.includes("opus")) return "opus";
  if (normalized.includes("haiku")) return "haiku";
  return "default";
}

function normalizeClaudeEffortOption(raw?: string | null): ClaudeEffortOption {
  if (typeof raw !== "string") return "default";
  const normalized = raw.trim().toLowerCase();
  if (
    normalized === "default" ||
    normalized === "low" ||
    normalized === "medium" ||
    normalized === "high" ||
    normalized === "max" ||
    normalized === "xhigh" ||
    normalized === "ultracode"
  ) {
    return normalized;
  }
  if (normalized === "x-high") return "xhigh";
  return "default";
}

/// Codex per-pane model choices. `value` is passed to `codex --model`. The
/// gpt-5.6 lineup (sol/terra/luna) from ~/.codex/models_cache.json; Default =
/// let codex use ~/.codex/config.toml.
const CODEX_MODEL_OPTIONS = [
  { value: "default", label: "Default" },
  { value: "gpt-5.6-sol", label: "Sol" },
  { value: "gpt-5.6-terra", label: "Terra" },
  { value: "gpt-5.6-luna", label: "Luna" },
] as const;
type CodexModelOption = (typeof CODEX_MODEL_OPTIONS)[number]["value"];

/// Codex reasoning-effort choices, passed via codex's
/// `-c model_reasoning_effort=<level>`. sol/terra support up to `ultra`; luna
/// tops out at `max`. Default = codex config.toml default.
const CODEX_EFFORT_OPTIONS = [
  { value: "default", label: "Default" },
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "xhigh", label: "XHigh" },
  { value: "max", label: "Max" },
  { value: "ultra", label: "Ultra" },
] as const;
type CodexEffortOption = (typeof CODEX_EFFORT_OPTIONS)[number]["value"];

function normalizeCodexModelOption(raw?: string | null): CodexModelOption {
  if (typeof raw !== "string") return "default";
  const normalized = raw.trim().toLowerCase();
  for (const opt of CODEX_MODEL_OPTIONS) {
    if (opt.value === normalized) return opt.value;
  }
  if (normalized.includes("sol")) return "gpt-5.6-sol";
  if (normalized.includes("terra")) return "gpt-5.6-terra";
  if (normalized.includes("luna")) return "gpt-5.6-luna";
  return "default";
}

function normalizeCodexEffortOption(raw?: string | null): CodexEffortOption {
  if (typeof raw !== "string") return "default";
  const normalized = raw.trim().toLowerCase();
  for (const opt of CODEX_EFFORT_OPTIONS) {
    if (opt.value === normalized) return opt.value;
  }
  if (normalized === "x-high") return "xhigh";
  if (normalized === "ultracode") return "ultra"; // claude→codex bridge
  if (normalized === "minimal") return "low";
  return "default";
}

// Synthesize PaneConfig entries from observed pane_id keys when no PaneList was received
function synthesizeConfigs(
  paneMessages: Record<string, Message[]>,
  paneStatuses: Record<string, string | null>,
  paneModes: Record<string, "deadloop" | "interactive">,
  pausedPanes: number[],
  activePaneId: number | null,
  sessionId: string | null,
): PaneConfig[] {
  const configs: PaneConfig[] = [];
  const paneIds = new Set<number>();

  for (const key of Object.keys(paneMessages)) {
    const numericId = parseInt(key, 10);
    if (!isNaN(numericId)) paneIds.add(numericId);
  }
  for (const key of Object.keys(paneStatuses)) {
    const numericId = parseInt(key, 10);
    if (!isNaN(numericId)) paneIds.add(numericId);
  }
  for (const paneId of pausedPanes) {
    paneIds.add(paneId);
  }
  if (activePaneId != null && activePaneId > PANE_ID_MAIN) {
    paneIds.add(activePaneId);
  }

  const sortedPaneIds = Array.from(paneIds).sort((a, b) => a - b);
  for (const numericId of sortedPaneIds) {
    const hintedMode = paneModes[paneKey(numericId)];
    const isDeadloop = hintedMode ? hintedMode === "deadloop" : numericId === PANE_ID_DEADLOOP;
    const isLegacyInteractive = numericId === PANE_ID_INTERACTIVE;
    configs.push({
      pane_id: numericId,
      provider: "claude",
      mode: hintedMode || (isDeadloop ? "deadloop" : "interactive"),
      session_id: sessionId || "",
      is_paused: pausedPanes.includes(numericId),
      label: numericId === PANE_ID_DEADLOOP
        ? "Deadloop"
        : isLegacyInteractive
          ? "Interactive"
          : `Tab ${numericId}`,
    });
  }
  return configs;
}

export function TabbedView({
  mobileLeading,
  mobileTrailing,
}: {
  mobileLeading?: React.ReactNode;
  mobileTrailing?: React.ReactNode;
} = {}) {
  const sessionId = useStore((s) => s.sessionId);
  const messages = useStore((s) => s.messages);
  const paneConfigs = useStore((s) => s.paneConfigs);
  const paneMessages = useStore((s) => s.paneMessages);
  const paneHasMore = useStore((s) => s.paneHasMore);
  const paneStatuses = useStore((s) => s.paneStatuses);
  const paneModes = useStore((s) => s.paneModes);
  const pausedPanes = useStore((s) => s.pausedPanes);
  const loadingMorePane = useStore((s) => s.loadingMorePane);
  const isAttached = useStore((s) => s.isAttached);
  const isDualPane = useStore((s) => s.isDualPane);
  const hasMoreMessages = useStore((s) => s.hasMoreMessages);
  const isLoadingMore = useStore((s) => s.isLoadingMore);
  const cliClientId = useStore((s) => s.cliClientId);
  const sessions = useStore((s) => s.sessions);
  const machines = useStore((s) => s.machines);

  const sendMessageToPane = useStore((s) => s.sendMessageToPane);
  const loadMoreMessages = useStore((s) => s.loadMoreMessages);
  const loadPaneMessagesIfNeeded = useStore((s) => s.loadPaneMessagesIfNeeded);
  const refreshPaneWindow = useStore((s) => s.refreshPaneWindow);
  const connected = useStore((s) => s.connected);
  const addPane = useStore((s) => s.addPane);
  const removePane = useStore((s) => s.removePane);
  const pausePane = useStore((s) => s.pausePane);
  const resumePane = useStore((s) => s.resumePane);
  const updatePaneLabel = useStore((s) => s.updatePaneLabel);
  const updatePaneEffort = useStore((s) => s.updatePaneEffort);
  const updatePaneModel = useStore((s) => s.updatePaneModel);
  const interruptPane = useStore((s) => s.interruptPane);
  const reorderPanes = useStore((s) => s.reorderPanes);
  const startBot = useStore((s) => s.startBot);
  const stopBot = useStore((s) => s.stopBot);
  const startMachineProjectCli = useStore((s) => s.startMachineProjectCli);
  const rebootCli = useStore((s) => s.rebootCli);
  const rebootPane = useStore((s) => s.rebootPane);
  const downloadSession = useStore((s) => s.downloadSession);
  const requestPaneDiff = useStore((s) => s.requestPaneDiff);
  const createPanePr = useStore((s) => s.createPanePr);
  const paneDiffs = useStore((s) => s.paneDiffs);
  const updatePaneRole = useStore((s) => s.updatePaneRole);
  const teamRecords = useStore(selectActiveTeamRecords);
  const planReviewPending = useStore((s) => s.planReviewPending);
  const answerPlanReview = useStore((s) => s.answerPlanReview);
  const updatePaneReviewMode = useStore((s) => s.updatePaneReviewMode);

  // Active tab state, persisted per project
  const [activeTabId, setActiveTabId] = useState<number | null>(null);
  // Lazy-mount panes. On project switch we used to eagerly mount every
  // pane's MessagePane (visibility-toggled via `hidden`) so tab clicks
  // were instant — but with 7+ panes × hundreds of messages each, that
  // reconciliation pegged the main thread for 1–2 s during the switch.
  // Now we mount only the active pane; first activation of a non-active
  // tab pays a one-time mount cost, and the pane stays mounted after so
  // subsequent switches are still instant. Reset on session change.
  const [mountedPanes, setMountedPanes] = useState<Set<number>>(() => new Set());
  const [startBotModalOpen, setStartBotModalOpen] = useState(false);
  const [viewBotPromptModalOpen, setViewBotPromptModalOpen] = useState(false);
  const [startBotPaneId, setStartBotPaneId] = useState<number | null>(null);
  const [botPromptDraft, setBotPromptDraft] = useState("");
  const [botMinIntervalDraft, setBotMinIntervalDraft] = useState(String(DEFAULT_BOT_MIN_INTERVAL_MINUTES));
  // Holds a Claude or Codex effort value depending on the active pane's
  // provider, so it's typed as the widened string rather than either union.
  const [botEffortDraft, setBotEffortDraft] = useState<string>("default");
  const [inputDrafts, setInputDrafts] = useState<Record<string, string>>({});
  const [addTabError, setAddTabError] = useState<string | null>(null);
  // 3-option cleanup dialog shown when closing a pane that owns a worktree.
  // null = closed. paneId/worktreePath populated when open.
  const [cleanupDialog, setCleanupDialog] = useState<
    { paneId: number; worktreePath: string } | null
  >(null);
  // Diff viewer modal (Phase 1.2a). When set, shows paneDiffs[paneId].
  const [diffModalPaneId, setDiffModalPaneId] = useState<number | null>(null);
  // Role drawer modal (Phase 2.1c). When set, edits role/goal/backstory.
  const [roleModalPaneId, setRoleModalPaneId] = useState<number | null>(null);
  // Team scratchpad modal (Phase 2.2b). Just a boolean — content lives in store.
  const [teamModalOpen, setTeamModalOpen] = useState(false);
  // Per-pane timeline-vs-raw-chat toggle (Phase 4.2b).
  const [timelinePanes, setTimelinePanes] = useState<Set<number>>(new Set());

  // Determine effective tabs: use paneConfigs from server, or synthesize from observed messages
  const effectiveTabs = useMemo(() => {
    // PaneList (paneConfigs) is authoritative for pane mode. Mode hints
    // harvested from messages are HISTORY — a replayed SessionMessages
    // batch re-asserts whatever mode a pane had when those messages
    // streamed (e.g. "deadloop" for a bot that was since stopped or
    // demoted on reboot). Letting hints override configs made such a
    // pane render bot UI (input disabled, no Start Bot) forever, while
    // the CLI actually had it interactive. The CLI sends a fresh
    // PaneList on every Start/Stop/Finalize transition, so configs are
    // never meaningfully behind; hints are only used below when no
    // PaneList has arrived at all.
    if (paneConfigs.length > 0) return paneConfigs;
    if (isDualPane && Object.keys(paneMessages).length > 0) {
      return synthesizeConfigs(
        paneMessages,
        paneStatuses,
        paneModes,
        pausedPanes,
        activeTabId,
        sessionId,
      );
    }
    // Single-pane: synthesize a single tab
    if (messages.length > 0) {
      return [{
        pane_id: PANE_ID_MAIN,
        provider: "claude" as const,
        mode: "interactive" as const,
        session_id: sessionId || "",
        is_paused: false,
        label: "Chat",
      }];
    }
    return [];
  }, [
    paneConfigs,
    paneModes,
    isDualPane,
    paneMessages,
    paneStatuses,
    pausedPanes,
    activeTabId,
    sessionId,
    messages.length,
  ]);

  // Stable list of tab IDs (avoids re-running effects on every message)
  const tabIds = useMemo(
    () => effectiveTabs.map((t) => t.pane_id).join(","),
    [effectiveTabs],
  );

  // The interactive Manager pane — its tab is the preferred default
  // landing surface for a fresh project, so the user can chat the goal
  // before opening any worker tab. Matches the role detection used by
  // ProjectGoalBar and OverviewView.
  const managerTabId = useMemo(() => {
    for (const tab of effectiveTabs) {
      const lower = (tab.role ?? "").toLowerCase();
      if (
        lower.includes("manager") &&
        !lower.includes("tech lead") &&
        tab.mode === "interactive"
      ) {
        return tab.pane_id;
      }
    }
    return null;
  }, [effectiveTabs]);

  // Track the cliClientId we last derived activeTabId for, so a project
  // switch always re-reads the persisted preference even when the
  // outgoing project's activeTabId happens to be a valid id in the
  // new project. Without this, switching A→B→A used to corrupt A's
  // saved active_tab — the effect would see stale activeTabId from B,
  // fail the `ids.includes(activeTabId)` check, fall through, and
  // overwrite A's localStorage with A's first pane id.
  const lastDerivedForRef = useRef<string | null | undefined>(undefined);

  // Derive activeTabId from cliClientId + available panes + persisted
  // preference. Writes to localStorage only happen through
  // handleSelectTab (i.e. explicit user click) — this effect never
  // persists the auto-derived fallback, so projects retain whichever
  // tab the user last actually chose.
  useEffect(() => {
    const ids = tabIds.split(",").filter(Boolean).map(Number);
    if (ids.length === 0) return;

    const clientChanged = lastDerivedForRef.current !== cliClientId;
    lastDerivedForRef.current = cliClientId;

    const saved = getProjectLayout(cliClientId, "active_tab", "");
    const nextActiveTabId = deriveInitialActiveTabId({
      activeTabId,
      clientChanged,
      managerTabId,
      paneConfigsLength: paneConfigs.length,
      savedActiveTab: saved,
      tabIds: ids,
    });
    if (nextActiveTabId != null && activeTabId !== nextActiveTabId) {
      setActiveTabId(nextActiveTabId);
    }
  }, [activeTabId, cliClientId, managerTabId, paneConfigs.length, tabIds]);

  // Lazy-load: when activeTabId changes (initial pick or user click),
  // fetch that pane's messages if we haven't already. Server's attach
  // reply no longer ships every pane's tail, so each pane opens
  // on-demand. Skip for the Overview pseudo-tab.
  useEffect(() => {
    const targets = lazyPaneMessageLoadTargets({
      activeTabId,
      tabIds: tabIds.split(",").filter(Boolean).map(Number),
    });
    for (const paneId of targets) {
      loadPaneMessagesIfNeeded(paneId);
    }
  }, [activeTabId, sessionId, loadPaneMessagesIfNeeded, tabIds]);

  // Sliding-window heal: on the initial connect after a page load and on
  // every reconnect, re-fetch the active pane's recent window and reconcile
  // it as a sliding window — overwriting any hole a flaky disconnect left
  // below the catchup watermark (catchup only extends the frontier forward
  // and can't backfill). activeTabId is read via a ref so this fires ONLY on
  // connect transitions, not on every tab switch.
  const activeTabIdRef = useRef(activeTabId);
  activeTabIdRef.current = activeTabId;
  const prevConnectedRef = useRef(false);
  useEffect(() => {
    const justConnected = connected && !prevConnectedRef.current;
    prevConnectedRef.current = connected;
    if (!justConnected || !sessionId) return;
    const pid = activeTabIdRef.current;
    if (pid == null || pid <= OVERVIEW_PANE_ID) return;
    // Let WS auth/attach and the reconnect catchup settle first.
    const t = setTimeout(() => refreshPaneWindow(pid), 600);
    return () => clearTimeout(t);
  }, [connected, sessionId, refreshPaneWindow]);

  // Reset the lazy-mount set on session change so a fresh project starts
  // with zero mounted panes — the active-pane effect below mounts the
  // landing tab immediately after.
  useEffect(() => {
    setMountedPanes(new Set());
  }, [sessionId]);

  // Track which panes have ever been activated for this session. We only
  // render the bodies of mounted panes; visited ones stay mounted (hidden
  // via CSS) so re-activating them is instant. Overview has its own path.
  useEffect(() => {
    if (activeTabId == null || activeTabId === OVERVIEW_PANE_ID) return;
    setMountedPanes((prev) => {
      if (prev.has(activeTabId)) return prev;
      const next = new Set(prev);
      next.add(activeTabId);
      return next;
    });
  }, [activeTabId]);

  const handleSelectTab = useCallback(
    (paneId: number) => {
      setActiveTabId(paneId);
      setProjectLayout(cliClientId, "active_tab", String(paneId));
    },
    [cliClientId],
  );

  const handleCloseTab = useCallback(
    (paneId: number) => {
      const pane = effectiveTabs.find((t) => t.pane_id === paneId);
      const worktreePath = pane?.worktree_path;
      if (worktreePath) {
        // Pane owns an isolated worktree — open the 3-option dialog.
        setCleanupDialog({ paneId, worktreePath });
        return;
      }
      if (!confirm("Close this tab?")) return;
      removePane(paneId);
      // If closing active tab, switch to another
      if (paneId === activeTabId && effectiveTabs.length > 1) {
        const remaining = effectiveTabs.filter((t) => t.pane_id !== paneId);
        if (remaining.length > 0) {
          handleSelectTab(remaining[0].pane_id);
        }
      }
    },
    [removePane, activeTabId, effectiveTabs, handleSelectTab],
  );

  const handleConfirmCleanup = useCallback(
    (action: PaneCleanupAction) => {
      if (!cleanupDialog) return;
      const { paneId } = cleanupDialog;
      removePane(paneId, action);
      setCleanupDialog(null);
      if (paneId === activeTabId && effectiveTabs.length > 1) {
        const remaining = effectiveTabs.filter((t) => t.pane_id !== paneId);
        if (remaining.length > 0) {
          handleSelectTab(remaining[0].pane_id);
        }
      }
    },
    [cleanupDialog, removePane, activeTabId, effectiveTabs, handleSelectTab],
  );

  const handleAddTab = useCallback((provider: string = "claude", model?: string, isolatedWorktree?: boolean, kind: PaneKind = "agent") => {
    const isMiniMax = provider === "minimax" || (provider === "claude" && isMiniMaxModel(model));
    const isGlm = provider === "glm" || (provider === "claude" && isGlmModel(model));
    const isDeepseek = provider === "deepseek" || (provider === "claude" && isDeepseekModel(model));
    const basePrefix = provider === "codex"
      ? "Codex"
      : provider === "cursor-agent"
        ? "Cursor"
        : provider === "opencode"
          ? "OpenCode"
          : isMiniMax ? "MiniMax" : isGlm ? "GLM" : isDeepseek ? "DeepSeek" : "Claude";
    // Terminal tabs sit next to agent tabs in the same bar, so the label
    // has to say which is which — they behave very differently.
    const prefix = kind === "terminal" ? `${basePrefix} TTY` : basePrefix;
    const label = `${prefix} ${effectiveTabs.length + 1}`;
    const result = addPane(provider, "interactive", label, undefined, model, isolatedWorktree, undefined, false, kind);
    if (result.success) {
      setAddTabError(null);
    } else {
      setAddTabError(result.error || "Failed to create tab");
      setTimeout(() => setAddTabError(null), 4000);
    }
  }, [addPane, effectiveTabs.length]);

  const handleRenameTab = useCallback(
    (paneId: number, newLabel: string) => {
      updatePaneLabel(paneId, newLabel);
    },
    [updatePaneLabel],
  );

  // Handle tab drag reorder
  const handleReorderTabs = useCallback(
    (orderedIds: number[]) => {
      reorderPanes(orderedIds);
    },
    [reorderPanes],
  );

  // Get messages for active tab
  const activeMessages = useMemo(() => {
    if (activeTabId == null) return [];
    if (activeTabId === PANE_ID_MAIN) return messages;
    return paneMessages[paneKey(activeTabId)] || [];
  }, [activeTabId, messages, paneMessages]);

  const activeInputPaneId = activeTabId ?? PANE_ID_MAIN;
  const activeInputDraftKey = useMemo(
    () => getScrollKey(sessionId, activeInputPaneId),
    [activeInputPaneId, sessionId],
  );

  useEffect(() => {
    setInputDrafts((prev) => {
      if (Object.prototype.hasOwnProperty.call(prev, activeInputDraftKey)) {
        return prev;
      }
      const restored = loadInputDraft(sessionId, activeInputPaneId);
      if (!restored) return prev;
      return { ...prev, [activeInputDraftKey]: restored };
    });
  }, [activeInputDraftKey, activeInputPaneId, sessionId]);

  const activeInputDraft = inputDrafts[activeInputDraftKey] ?? "";

  const handleInputDraftChange = useCallback(
    (value: string) => {
      setInputDrafts((prev) => ({ ...prev, [activeInputDraftKey]: value }));
      persistInputDraft(sessionId, activeInputPaneId, value);
    },
    [activeInputDraftKey, activeInputPaneId, sessionId],
  );

  const activeConfig = effectiveTabs.find((t) => t.pane_id === activeTabId);
  const activeHasMore = activeTabId === PANE_ID_MAIN ? hasMoreMessages : (activeTabId != null ? paneHasMore[paneKey(activeTabId)] || false : false);
  const activeIsLoading = activeTabId === PANE_ID_MAIN ? isLoadingMore : loadingMorePane === activeTabId;
  const activeStatus = activeTabId != null ? paneStatuses[paneKey(activeTabId)] || null : null;
  const activeIsPaused = activeTabId != null ? pausedPanes.includes(activeTabId) : false;
  const activeIsBot = activeConfig?.mode === "deadloop";
  // Terminal panes take keystrokes directly in xterm.js. Showing the chat
  // composer under one would be worse than redundant: its text goes down
  // the agent input path, which a terminal pane has no channel for.
  const activeIsTerminal = activeConfig?.kind === "terminal";
  const activeStopRequested = activeConfig?.stop_requested === true;
  const activeProvider = activeConfig?.provider;
  const activeIsMiniMax = activeProvider === "minimax" || (
    activeProvider === "claude" && (
      isMiniMaxModel(activeConfig?.model) ||
      (typeof activeConfig?.label === "string" && activeConfig.label.toLowerCase().includes("minimax"))
    )
  );
  const activeIsGlm = activeProvider === "glm" || (
    activeProvider === "claude" && (
      isGlmModel(activeConfig?.model) ||
      (typeof activeConfig?.label === "string" && activeConfig.label.toLowerCase().includes("glm"))
    )
  );
  const activeIsDeepseek = activeProvider === "deepseek" || (
    activeProvider === "claude" && (
      isDeepseekModel(activeConfig?.model) ||
      (typeof activeConfig?.label === "string" && activeConfig.label.toLowerCase().includes("deepseek"))
    )
  );
  const activeUsageProvider = activeIsMiniMax
    ? "minimax"
    : activeIsGlm
      ? "glm"
      : activeIsDeepseek
        ? "deepseek"
        : activeProvider;
  // Model + effort switchers support Claude and Codex, each with its own
  // option list + normalizer; other backends have their own model namespaces
  // and no reasoning-effort knob. `activeSupportsClaudeEffort` keeps its name
  // for history but now means "this pane has a model/effort switcher".
  const isCodexModelEffort = activeUsageProvider === "codex";
  const activeSupportsClaudeEffort =
    activeUsageProvider === "claude" || isCodexModelEffort;
  const modelSwitcherOptions: readonly { value: string; label: string }[] =
    isCodexModelEffort ? CODEX_MODEL_OPTIONS : CLAUDE_MODEL_OPTIONS;
  const effortSwitcherOptions: readonly { value: string; label: string }[] =
    isCodexModelEffort ? CODEX_EFFORT_OPTIONS : CLAUDE_EFFORT_OPTIONS;
  const activeBotEffortOption: string = isCodexModelEffort
    ? normalizeCodexEffortOption(activeConfig?.effort)
    : normalizeClaudeEffortOption(activeConfig?.effort);
  const activeModelOption: string = isCodexModelEffort
    ? normalizeCodexModelOption(activeConfig?.model)
    : normalizeClaudeModelOption(activeConfig?.model);
  const activeBotPrompt = botPromptForPane(activeConfig);
  const activeBotMinIntervalMinutes = typeof activeConfig?.min_iteration_interval_minutes === "number"
    ? activeConfig.min_iteration_interval_minutes
    : DEFAULT_BOT_MIN_INTERVAL_MINUTES;
  const diffModalPaneConfig = diffModalPaneId !== null
    ? effectiveTabs.find((t) => t.pane_id === diffModalPaneId)
    : undefined;
  const startBotTargetConfig = useMemo(
    () => effectiveTabs.find((t) => t.pane_id === startBotPaneId),
    [effectiveTabs, startBotPaneId],
  );

  // Use a targeted selector to avoid re-renders when unrelated usage limits change
  const currentUsageLimits = useStore(
    useCallback(
      (s) => {
        if (!cliClientId || !activeUsageProvider) return null;
        return s.usageLimits.get(cliClientId)?.[activeUsageProvider] ?? null;
      },
      [cliClientId, activeUsageProvider],
    ),
  );

  const usageLabel = useMemo(() => {
    if (!activeUsageProvider) return "Usage";
    if (activeUsageProvider === "codex") return "Codex Usage";
    if (activeUsageProvider === "minimax") return "MiniMax Usage";
    if (activeUsageProvider === "glm") return "GLM Usage";
    if (activeUsageProvider === "deepseek") return "DeepSeek Usage";
    return "Claude Usage";
  }, [activeUsageProvider]);

  useEffect(() => {
    setBotEffortDraft(activeBotEffortOption);
  }, [activeBotEffortOption, activeTabId]);

  const bootTarget = useMemo(() => {
    if (!sessionId) return null;
    const session = sessions.find((item) => item.id === sessionId);
    const sessionPath = normalizeComparablePath(session?.workingDir);
    if (!sessionPath) return null;
    const sessionHostname = session?.hostname?.trim().toLowerCase() || null;

    type MachineProjectTarget = { machineId: string; projectId: string; isRunning: boolean };
    const hostMatches: MachineProjectTarget[] = [];
    const allMatches: MachineProjectTarget[] = [];

    for (const machineWithProjects of machines) {
      const machineHostname = machineWithProjects.machine.hostname.trim().toLowerCase();
      for (const project of machineWithProjects.projects) {
        const projectPath = normalizeComparablePath(project.path);
        if (projectPath !== sessionPath) continue;
        const target = {
          machineId: machineWithProjects.machine.machineId,
          projectId: project.projectId,
          isRunning: project.isRunning,
        };
        allMatches.push(target);
        if (sessionHostname && machineHostname === sessionHostname) {
          hostMatches.push(target);
        }
      }
    }

    const chooseTarget = (matches: MachineProjectTarget[]): MachineProjectTarget | null => {
      if (matches.length === 0) return null;

      // Prefer exact project-id match first when available.
      const exactMatches = matches.filter((target) => target.projectId === sessionId);
      if (exactMatches.length === 1) return exactMatches[0];
      if (exactMatches.length > 1) return null;

      // Daemon may transiently report duplicate project IDs for the same machine/path.
      // Collapse those duplicates so Boot does not disappear due to ambiguity.
      const dedupedByMachine = new Map<string, MachineProjectTarget>();
      for (const target of matches) {
        const existing = dedupedByMachine.get(target.machineId);
        if (!existing) {
          dedupedByMachine.set(target.machineId, target);
          continue;
        }
        if (target.projectId === sessionId) {
          dedupedByMachine.set(target.machineId, target);
        }
      }

      if (dedupedByMachine.size === 1) {
        return Array.from(dedupedByMachine.values())[0];
      }
      return null;
    };

    if (sessionHostname) {
      const hostTarget = chooseTarget(hostMatches);
      if (hostTarget) return hostTarget;
      if (hostMatches.length > 1) return null;
    }

    return chooseTarget(allMatches);
  }, [machines, sessionId, sessions]);

  const canBootCurrentProject = !isAttached && bootTarget != null && !bootTarget.isRunning;

  const handleBootCli = useCallback(() => {
    if (!bootTarget) return;
    startMachineProjectCli(bootTarget.machineId, bootTarget.projectId);
  }, [bootTarget, startMachineProjectCli]);

  const handleLoadMore = useCallback(() => {
    if (activeTabId == null) return;
    if (activeTabId === PANE_ID_MAIN) {
      loadMoreMessages();
    } else {
      loadMoreMessages(activeTabId);
    }
  }, [activeTabId, loadMoreMessages]);

  const handleSend = useCallback(
    (text: string) => {
      if (activeTabId == null) return { success: false, error: "No active tab" };
      if (activeIsBot) {
        return {
          success: false,
          error: "This pane is in bot mode. Click Stop Bot to switch to interactive mode.",
        };
      }
      if (activeTabId === PANE_ID_MAIN) {
        const { ws, sessionId } = useStore.getState();
        if (!ws || ws.readyState !== WebSocket.OPEN) return { success: false, error: "Not connected" };
        ws.send(JSON.stringify({ type: "input", session_id: sessionId, text }));
        return { success: true };
      }
      // Guard against a stale activeTabId left over from a previous session —
      // refuse to send if the active tab is not actually in the current
      // session's pane list. Otherwise the server routes input with a
      // pane_id the current CLI has never heard of and returns
      // "Pane worker unavailable".
      if (!effectiveTabs.some((t) => t.pane_id === activeTabId)) {
        return {
          success: false,
          error: "Active tab is stale for this project. Click a tab to retry.",
        };
      }
      return sendMessageToPane(text, activeTabId);
    },
    [activeIsBot, activeTabId, effectiveTabs, sendMessageToPane],
  );

  const handleStartBot = useCallback(() => {
    if (activeTabId == null) return;
    setStartBotPaneId(activeTabId);
    const savedMinInterval = typeof activeConfig?.min_iteration_interval_minutes === "number"
      ? activeConfig.min_iteration_interval_minutes
      : DEFAULT_BOT_MIN_INTERVAL_MINUTES;
    setBotPromptDraft(botPromptForPane(activeConfig));
    setBotMinIntervalDraft(String(savedMinInterval));
    setBotEffortDraft(activeBotEffortOption);
    setStartBotModalOpen(true);
  }, [activeBotEffortOption, activeConfig?.min_iteration_interval_minutes, activeConfig?.prompt, activeTabId]);

  const handleStopBot = useCallback(() => {
    if (activeTabId == null) return;
    stopBot(activeTabId);
  }, [activeTabId, stopBot]);

  const handleCancelStartBot = useCallback(() => {
    setStartBotModalOpen(false);
    setStartBotPaneId(null);
  }, []);

  const handleOpenBotPrompt = useCallback(() => {
    if (!activeIsBot) return;
    setViewBotPromptModalOpen(true);
  }, [activeIsBot]);

  const handleCloseBotPrompt = useCallback(() => {
    setViewBotPromptModalOpen(false);
  }, []);

  const handleConfirmStartBot = useCallback(() => {
    if (startBotPaneId == null) return;
    const trimmed = botPromptDraft.trim();
    const minutesInput = botMinIntervalDraft.trim();
    const parsedMinutes = minutesInput === "" ? NaN : Number(minutesInput);
    const minIntervalMinutes = Number.isFinite(parsedMinutes)
      ? Math.max(0, Math.floor(parsedMinutes))
      : DEFAULT_BOT_MIN_INTERVAL_MINUTES;
    startBot(
      startBotPaneId,
      trimmed.length > 0 ? botPromptDraft : defaultBotPromptForPane(startBotTargetConfig),
      minIntervalMinutes,
      activeSupportsClaudeEffort ? botEffortDraft : undefined,
    );
    setStartBotModalOpen(false);
    setStartBotPaneId(null);
  }, [activeSupportsClaudeEffort, botEffortDraft, botMinIntervalDraft, botPromptDraft, startBot, startBotPaneId, startBotTargetConfig]);

  useEffect(() => {
    if (!activeIsBot && viewBotPromptModalOpen) {
      setViewBotPromptModalOpen(false);
    }
  }, [activeIsBot, viewBotPromptModalOpen]);

  // No session or no tabs - empty state
  if (!sessionId || effectiveTabs.length === 0) {
    return (
      <div className="flex-1 flex flex-col min-h-0">
        {/* On mobile the tab bar (which holds the ☰ menu) isn't rendered in
            this empty state, so surface the menu button here — otherwise a
            user with no selected session (e.g. after clearing site data) has
            no way to open the sidebar and pick a project. Hidden on desktop,
            where the sidebar is always visible. */}
        {mobileLeading && (
          <div className="md:hidden flex items-center h-11 border-b border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/30">
            {mobileLeading}
          </div>
        )}
        <div className="flex-1 flex items-center justify-center text-gray-400">
          <div className="text-center px-6">
            <p className="text-lg">No messages yet</p>
            <p className="text-sm mt-1">Waiting for activity...</p>
            <p className="text-xs mt-3 md:hidden">Tap ☰ at the top-left to pick a project.</p>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Tab bar */}
      <TabBar
        tabs={effectiveTabs}
        activeTabId={activeTabId}
        onSelectTab={handleSelectTab}
        onCloseTab={handleCloseTab}
        onAddTab={handleAddTab}
        onRenameTab={handleRenameTab}
        onReorderTabs={handleReorderTabs}
        onBootCli={handleBootCli}
        onRebootCli={rebootCli}
        showBootButton={canBootCurrentProject}
        showRebootButton={isAttached}
        paneStatuses={paneStatuses}
        pausedPanes={pausedPanes}
        leading={mobileLeading}
        trailing={mobileTrailing}
      />

      {/* Add tab error notification */}
      {addTabError && (
        <div className="flex items-center gap-2 px-3 py-1.5 bg-red-50 dark:bg-red-900/30 border-b border-red-200 dark:border-red-800 text-red-700 dark:text-red-300 text-xs flex-shrink-0">
          <svg className="w-3.5 h-3.5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L4.082 16.5c-.77.833.192 2.5 1.732 2.5z" />
          </svg>
          <span>{addTabError}</span>
          <button
            onClick={() => setAddTabError(null)}
            className="ml-auto text-red-500 dark:text-red-400 hover:text-red-700 dark:hover:text-red-200"
          >
            <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      )}

      {/* Toolbar — flex-wrap so the controls (bot/effort/provider/model) wrap
          onto a second row on narrow phones instead of overflowing past the
          right edge, where the overflow-hidden frame would clip the rightmost
          ones (the provider + model selectors) off-screen and unreachable. */}
      <div className="flex flex-wrap items-center gap-2 px-3 py-1.5 border-b border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/30 flex-shrink-0">
        {/* Start/Stop Bot */}
        {isAttached && activeTabId != null && activeTabId !== PANE_ID_MAIN && (
          activeIsBot ? (
            <>
              {activeStopRequested ? (
                <button
                  onClick={handleStopBot}
                  className="flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded transition-colors bg-amber-600 hover:bg-amber-700 text-white"
                  title="Force stop immediately — kill the current process"
                >
                  <svg className="w-3 h-3" fill="none" stroke="currentColor" strokeWidth="2.5" viewBox="0 0 24 24"><path d="M6 18L18 6M6 6l12 12" /></svg>
                  Force Stop
                </button>
              ) : (
                <button
                  onClick={handleStopBot}
                  className="flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded transition-colors bg-red-500 hover:bg-red-600 text-white"
                  title="Stop after current work finishes (click again to force stop)"
                >
                  <svg className="w-3 h-3" fill="currentColor" viewBox="0 0 24 24"><path d="M6 6h12v12H6z" /></svg>
                  Stop Bot
                </button>
              )}
              <button
                onClick={handleOpenBotPrompt}
                className="flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded transition-colors bg-indigo-500 hover:bg-indigo-600 text-white"
                title="View this tab's current bot prompt"
              >
                <svg className="w-3 h-3" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24"><path d="M15 12h.01M12 12h.01M9 12h.01M4 6h16v12H4z" /></svg>
                View Prompt
              </button>
            </>
          ) : (
            <>
              <button
                onClick={handleStartBot}
                className="flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded transition-colors bg-green-500 hover:bg-green-600 text-white"
                title="Start autonomous bot execution in this tab"
              >
                <svg className="w-3 h-3" fill="currentColor" viewBox="0 0 24 24"><path d="M8 5v14l11-7z" /></svg>
                Bot
              </button>
              {activeSupportsClaudeEffort && (
                <select
                  value={botEffortDraft}
                  onChange={(e) => {
                    const next = isCodexModelEffort
                      ? normalizeCodexEffortOption(e.target.value)
                      : normalizeClaudeEffortOption(e.target.value);
                    setBotEffortDraft(next);
                    // Persist on the server (and CLI's .apas) so the choice
                    // survives tab-switch and CLI restart.
                    if (activeTabId != null) {
                      updatePaneEffort(activeTabId, next === "default" ? null : next);
                    }
                  }}
                  className="rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 px-1.5 py-1 text-xs focus:outline-none focus:ring-2 focus:ring-blue-500"
                  title="Reasoning effort — persisted per tab"
                >
                  {effortSwitcherOptions.map((option) => (
                    <option
                      key={option.value}
                      value={option.value}
                      {...(option.value === "ultracode"
                        ? { title: "xhigh + auto multi-agent workflows" }
                        : {})}
                    >
                      {option.label}
                    </option>
                  ))}
                </select>
              )}
              {activeTabId != null && activeProvider && (
                <select
                  value={activeProvider as ProviderOption}
                  onChange={(e) => {
                    const next = e.target.value as ProviderOption;
                    if (next === activeProvider) return;
                    if (
                      !confirm(
                        `Switch agent to ${PROVIDER_LABEL[next] ?? next}? The current turn will be interrupted and the agent respawns with a fresh context — chat history stays visible but is NOT in the new agent's prompt. Make sure ${PROVIDER_LABEL[next] ?? next} is installed + authenticated on the machine running this pane.`,
                      )
                    ) {
                      return;
                    }
                    // Provider change resets model to default — each
                    // backend has its own model namespace (sonnet only
                    // makes sense for Claude).
                    updatePaneModel(activeTabId, null, next);
                  }}
                  className="rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 px-1.5 py-1 text-xs focus:outline-none focus:ring-2 focus:ring-blue-500"
                  title="Agent backend — switching kills the current agent child and respawns with a fresh session id"
                >
                  {PROVIDER_OPTIONS.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </select>
              )}
              {activeSupportsClaudeEffort && activeTabId != null && (
                <select
                  value={activeModelOption}
                  onChange={(e) => {
                    const next = e.target.value;
                    if (next === activeModelOption) return;
                    // Claude: every specific model live-swaps via
                    // apply_flag_settings (no interrupted turn, no context
                    // reset); only clearing to "default" respawns. Codex has
                    // no live process to swap — it applies the model on its
                    // next per-turn re-exec — so it skips the confirm.
                    if (!isCodexModelEffort && next === "default") {
                      if (
                        !confirm(
                          "Clear model back to Claude's default? The current turn will be interrupted and the agent will respawn with a fresh context — chat history above stays visible but is NOT in the new agent's prompt.",
                        )
                      ) {
                        return;
                      }
                    }
                    updatePaneModel(activeTabId, next === "default" ? null : next);
                  }}
                  className="rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 px-1.5 py-1 text-xs focus:outline-none focus:ring-2 focus:ring-blue-500"
                  title={
                    isCodexModelEffort
                      ? "Codex model — applies on the next prompt (codex re-execs each turn; no context reset)."
                      : "Claude model — switching between fable/sonnet/opus/haiku live-swaps via apply_flag_settings (no context reset). Clearing to default respawns."
                  }
                >
                  {modelSwitcherOptions.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </select>
              )}
              {activeStatus && activeTabId != null && (
                <button
                  onClick={() => {
                    if (activeTabId == null) return;
                    if (confirm(
                      "Interrupt this pane's current turn? The agent process will be sent SIGINT (and SIGKILL after 2s if it doesn't exit). Queued input will run next.",
                    )) {
                      interruptPane(activeTabId);
                    }
                  }}
                  className="flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded transition-colors bg-red-500 hover:bg-red-600 text-white"
                  title="Interrupt the current agent run (SIGINT). Use when a turn is wedged on a hung tool call."
                >
                  <svg className="w-3 h-3" fill="currentColor" viewBox="0 0 24 24"><path d="M6 6h12v12H6z" /></svg>
                  Interrupt
                </button>
              )}
              {activeConfig?.worktree_path && activeTabId != null && (
                <button
                  onClick={() => {
                    if (activeTabId == null) return;
                    setDiffModalPaneId(activeTabId);
                    requestPaneDiff(activeTabId);
                  }}
                  className="flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded transition-colors bg-emerald-600 hover:bg-emerald-700 text-white"
                  title={`View git diff for branch (worktree at ${activeConfig.worktree_path})`}
                >
                  Diff
                </button>
              )}
              {activeTabId != null && activeConfig && (
                <button
                  onClick={() => setRoleModalPaneId(activeTabId)}
                  className="flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded transition-colors bg-purple-600 hover:bg-purple-700 text-white"
                  title="Edit role / goal / backstory for this pane (composed into --append-system-prompt at next spawn)"
                >
                  Role
                </button>
              )}
              {activeTabId != null && activeTabId !== OVERVIEW_PANE_ID && activeConfig && (
                <button
                  onClick={() => {
                    setTimelinePanes((prev) => {
                      const next = new Set(prev);
                      if (next.has(activeTabId)) next.delete(activeTabId);
                      else next.add(activeTabId);
                      return next;
                    });
                  }}
                  className={`flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded transition-colors ${
                    timelinePanes.has(activeTabId)
                      ? "bg-indigo-700 hover:bg-indigo-800 text-white"
                      : "bg-gray-200 dark:bg-gray-700 hover:bg-gray-300 dark:hover:bg-gray-600 text-gray-700 dark:text-gray-200"
                  }`}
                  title="Toggle between raw chat and a per-action timeline (tool name + args summary + result)"
                >
                  {timelinePanes.has(activeTabId) ? "Chat" : "Timeline"}
                </button>
              )}
            </>
          )
        )}

        {activeUsageProvider && currentUsageLimits && (
          <div className="ml-1">
            <div className="text-[10px] uppercase tracking-wide text-gray-400 dark:text-gray-500 mb-0.5">
              {usageLabel}
            </div>
            <UsageLimitsDisplay limits={currentUsageLimits} compact />
          </div>
        )}

        <div className="flex-1" />

        {/* Actions */}
        <button
          onClick={() => setTeamModalOpen(true)}
          className="inline-flex items-center px-2.5 py-1 text-xs font-medium rounded transition-colors bg-amber-600 hover:bg-amber-700 text-white"
          title="Team scratchpad — append-only timeline of artifacts (diffs, reviews, decisions) shared across panes via .apas-team.jsonl"
        >
          Team{teamRecords.length > 0 ? ` (${teamRecords.length})` : ""}
        </button>
        <button
          onClick={downloadSession}
          className="hidden md:inline-flex items-center px-2.5 py-1 text-xs font-medium rounded transition-colors bg-blue-500 hover:bg-blue-600 text-white"
          title="Download session data"
        >
          Download
        </button>
        {shouldShowPaneRebootButton(activeTabId) && (
          <button
            onClick={() => {
              requestConfirmedPaneReboot(
                activeTabId,
                (message) => typeof window === "undefined" || window.confirm(message),
                rebootPane,
              );
            }}
            className="inline-flex items-center px-2.5 py-1 text-xs font-medium rounded transition-colors bg-rose-600 hover:bg-rose-700 text-white"
            title="Reboot just this pane's agent — kills the running process and respawns with --resume on the same session so prior context is preserved. Use when a pane is wedged and you don't want to recycle the entire CLI."
          >
            Reboot
          </button>
        )}
      </div>

      {/* Pane bodies: Overview is rendered alone; otherwise every pane's
          body is mounted once and visibility-toggled via `hidden`, so
          switching tabs is a CSS class flip instead of an unmount+
          remount+paint cycle. Avoids the 100-500ms delay that used to
          look like a server fetch. Keyed by sessionId+paneId so a
          project switch still resets state cleanly. */}
      {activeTabId === OVERVIEW_PANE_ID ? (
        <OverviewView
          key="overview"
          onOpenPane={handleSelectTab}
          onOpenDiff={(pid) => {
            setDiffModalPaneId(pid);
            requestPaneDiff(pid);
          }}
          onOpenRole={(pid) => setRoleModalPaneId(pid)}
          onPausePane={pausePane}
          onResumePane={resumePane}
          onRemovePane={handleCloseTab}
        />
      ) : effectiveTabs.length === 0 ? (
        // Single-pane fallback (no pane system / synthesizing from
        // legacy messages list). Just one MessagePane, no toggling.
        <MessagePane
          key={`${sessionId}-${PANE_ID_MAIN}`}
          paneId={PANE_ID_MAIN}
          messages={messages}
          onLoadMore={handleLoadMore}
          isLoading={isLoadingMore}
          hasMore={hasMoreMessages}
          isActive
        />
      ) : (
        effectiveTabs.map((tab) => {
          const isActive = tab.pane_id === activeTabId;
          const isTimeline = timelinePanes.has(tab.pane_id);
          // Skip mounting panes the user hasn't activated yet for this
          // session — that's the freeze fix. The container <div> still
          // exists to keep the tab list stable; the heavy MessagePane
          // body only mounts on first activation and stays mounted.
          if (!mountedPanes.has(tab.pane_id)) {
            return <div key={tab.pane_id} className="hidden" />;
          }
          const msgs = paneMessages[paneKey(tab.pane_id)] || [];
          return (
            <div
              key={tab.pane_id}
              className={isActive ? "flex-1 flex flex-col min-h-0" : "hidden"}
            >
              {tab.kind === "terminal" ? (
                // Two views over one pane. The pty stream still goes straight
                // to xterm.js via terminalBus; the conversation view renders
                // the turns the CLI reads out of the provider's transcript,
                // which arrive as ordinary pane messages and so need no
                // special casing in MessagePane.
                //
                // The terminal is kept mounted and merely hidden when the
                // conversation is showing: unmounting would tear down the
                // xterm instance and force a re-attach, losing scroll position
                // and focus every time someone glanced at the transcript.
                <TerminalPaneWithViews
                  sessionId={sessionId}
                  paneId={tab.pane_id}
                  messages={msgs}
                />
              ) : isTimeline ? (
                <TimelinePane
                  key={`timeline-${sessionId}-${tab.pane_id}`}
                  messages={msgs}
                />
              ) : (
                <>
                  <WorkerTaskBar paneId={tab.pane_id} role={tab.role} managed={tab.managed} />
                  <MessagePane
                    key={`${sessionId}-${tab.pane_id}`}
                    paneId={tab.pane_id}
                    messages={msgs}
                    onLoadMore={() => loadMoreMessages(tab.pane_id)}
                    isLoading={loadingMorePane === tab.pane_id}
                    hasMore={paneHasMore[paneKey(tab.pane_id)] || false}
                    isActive={isActive}
                    role={tab.role}
                  />
                </>
              )}
            </div>
          );
        })
      )}

      {/* Status bar — never shown on the Overview pseudo-tab */}
      {activeStatus && activeTabId !== OVERVIEW_PANE_ID && (
        <div className="px-3 py-2 border-t border-gray-200 dark:border-gray-700 bg-blue-50 dark:bg-blue-900/20 flex-shrink-0">
          <div className="flex items-center gap-2 text-sm text-blue-700 dark:text-blue-300">
            <div className="animate-pulse">●</div>
            <span>{activeStatus}</span>
          </div>
        </div>
      )}

      {/* Input box — disabled for running deadloop panes, hidden on
          Overview and on terminal panes (which take input in xterm). */}
      {activeTabId !== OVERVIEW_PANE_ID && !activeIsTerminal && (
        <div className="p-3 border-t border-gray-200 dark:border-gray-700 flex-shrink-0">
          {activeIsBot ? (
            <div className="text-center text-sm text-gray-400 dark:text-gray-500 py-2">
              {activeStopRequested
                ? "Stop requested — waiting for current work to finish. Click Force Stop to kill immediately."
                : activeIsPaused
                  ? "Bot is paused from previous state. Click Stop Bot to switch to interactive mode."
                  : "Bot is running autonomously. Click Stop Bot to switch to interactive mode after current work finishes."}
            </div>
          ) : (
            <InteractiveInput
              draft={activeInputDraft}
              onDraftChange={handleInputDraftChange}
              onSend={handleSend}
            />
          )}
        </div>
      )}

      <StartBotPromptModal
        open={startBotModalOpen}
        prompt={botPromptDraft}
        minIntervalMinutes={botMinIntervalDraft}
        tabLabel={startBotTargetConfig?.label || (startBotPaneId != null ? `Tab ${startBotPaneId}` : "Tab")}
        onPromptChange={setBotPromptDraft}
        onMinIntervalChange={setBotMinIntervalDraft}
        onCancel={handleCancelStartBot}
        onConfirm={handleConfirmStartBot}
      />

      <ViewBotPromptModal
        open={viewBotPromptModalOpen}
        prompt={activeBotPrompt}
        minIntervalMinutes={activeBotMinIntervalMinutes}
        tabLabel={activeConfig?.label || (activeTabId != null ? `Tab ${activeTabId}` : "Tab")}
        onClose={handleCloseBotPrompt}
      />

      <WorktreeCleanupModal
        open={cleanupDialog !== null}
        worktreePath={cleanupDialog?.worktreePath || ""}
        onCancel={() => setCleanupDialog(null)}
        onConfirm={handleConfirmCleanup}
      />

      <PaneDiffModal
        open={diffModalPaneId !== null}
        diff={diffModalPaneId !== null ? paneDiffs[diffModalPaneId] : undefined}
        manualPrCreationDisabled={diffModalPaneConfig?.managed === true}
        onClose={() => setDiffModalPaneId(null)}
        onRefresh={() => {
          if (diffModalPaneId !== null) requestPaneDiff(diffModalPaneId);
        }}
        onMerge={() => {
          if (diffModalPaneId === null) return;
          if (!confirm("Merge this pane's branch into the project's HEAD, then close the pane? Aborts on conflict — resolve manually with git if it fails.")) return;
          removePane(diffModalPaneId, "merge_and_remove");
          setDiffModalPaneId(null);
        }}
        onDiscard={() => {
          if (diffModalPaneId === null) return;
          if (!confirm("Discard this branch AND its worktree, then close the pane? This is destructive — uncommitted changes are lost.")) return;
          removePane(diffModalPaneId, "discard");
          setDiffModalPaneId(null);
        }}
        onCreatePr={() => {
          if (diffModalPaneId === null) return;
          if (!confirm("Push this branch to origin and open a GitHub PR? Requires `gh` to be installed and authenticated on the CLI host.")) return;
          createPanePr(diffModalPaneId);
          setDiffModalPaneId(null);
        }}
      />

      <RoleModal
        open={roleModalPaneId !== null}
        pane={roleModalPaneId !== null ? effectiveTabs.find((t) => t.pane_id === roleModalPaneId) : undefined}
        onClose={() => setRoleModalPaneId(null)}
        onSave={(label, role, goal, backstory, mode) => {
          if (roleModalPaneId === null) return;
          const pane = effectiveTabs.find((t) => t.pane_id === roleModalPaneId);
          if (label.trim() && label.trim() !== (pane?.label ?? "")) {
            updatePaneLabel(roleModalPaneId, label.trim());
          }
          updatePaneRole(roleModalPaneId, role, goal, backstory);
          updatePaneReviewMode(roleModalPaneId, mode);
          setRoleModalPaneId(null);
        }}
      />

      <TeamModal
        open={teamModalOpen}
        records={teamRecords}
        onClose={() => setTeamModalOpen(false)}
      />

      {planReviewPending.length > 0 && (
        <div className="fixed inset-x-0 bottom-0 z-40 flex justify-center p-3 pointer-events-none">
          <div className="pointer-events-auto flex w-full max-w-3xl flex-col gap-2">
            {planReviewPending.map((item) => (
              <div
                key={item.toolUseId}
                className="rounded-lg border border-orange-700 bg-orange-950/90 p-3 text-zinc-100 shadow-xl"
              >
                <div className="mb-2 flex flex-wrap items-baseline justify-between gap-2">
                  <h4 className="text-sm font-semibold">
                    Plan review: pane {item.paneId} wants to call <span className="font-mono">{item.toolName}</span>
                  </h4>
                  <span className="text-[10px] uppercase tracking-wide text-orange-300">held</span>
                </div>
                <pre className="mb-2 max-h-48 overflow-auto rounded bg-black/30 p-2 text-xs font-mono whitespace-pre-wrap break-words text-zinc-200">
                  {(() => {
                    try {
                      return JSON.stringify(item.input, null, 2);
                    } catch {
                      return String(item.input);
                    }
                  })()}
                </pre>
                <div className="flex justify-end gap-2">
                  <button
                    type="button"
                    onClick={() => answerPlanReview(item.toolUseId, false)}
                    className="rounded border border-red-700 bg-red-900/40 px-3 py-1 text-xs text-red-200 hover:bg-red-900/60"
                  >
                    Deny
                  </button>
                  <button
                    type="button"
                    onClick={() => answerPlanReview(item.toolUseId, true)}
                    className="rounded border border-emerald-700 bg-emerald-700 px-3 py-1 text-xs text-emerald-50 hover:bg-emerald-600"
                  >
                    Approve
                  </button>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

interface TeamModalProps {
  open: boolean;
  records: TeamRecord[];
  onClose: () => void;
}

function TeamModal({ open, records, onClose }: TeamModalProps) {
  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="flex w-full max-w-3xl flex-col rounded-lg border border-zinc-700 bg-zinc-900 p-5 text-zinc-100 shadow-xl"
        style={{ maxHeight: "85vh" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-3 flex items-center justify-between gap-3">
          <h3 className="text-lg font-semibold">
            Team scratchpad ({records.length} record{records.length === 1 ? "" : "s"})
          </h3>
          <button
            type="button"
            onClick={onClose}
            className="rounded border border-zinc-600 bg-zinc-800 px-3 py-1 text-xs hover:bg-zinc-700"
          >
            Close
          </button>
        </div>
        <p className="mb-3 text-xs text-zinc-400">
          Append-only timeline of artifacts published by panes, stored at <code>.apas-team.jsonl</code> in the project root. Agents append via Bash/Write tools; this view tails the file.
        </p>
        <div className="flex-1 overflow-auto rounded border border-zinc-800 bg-black/30 p-3">
          {records.length === 0 ? (
            <p className="text-sm italic text-zinc-400">
              No records yet. Agents can append by writing JSON lines to <code>.apas-team.jsonl</code>.
            </p>
          ) : (
            <ul className="flex flex-col gap-3">
              {records.map((r, i) => (
                <li key={i} className="rounded border border-zinc-800 bg-zinc-900/60 p-3">
                  <div className="mb-1 flex flex-wrap items-center gap-2 text-xs text-zinc-400">
                    <span className="font-mono text-zinc-300">{r.kind}</span>
                    <span>·</span>
                    <span>{r.ts}</span>
                    {r.pane_id !== undefined && (
                      <>
                        <span>·</span>
                        <span>pane {r.pane_id}</span>
                      </>
                    )}
                    {r.tags.length > 0 && (
                      <>
                        <span>·</span>
                        {r.tags.map((t) => (
                          <span key={t} className="rounded bg-zinc-800 px-1.5 py-0.5 font-mono text-[10px] text-zinc-300">
                            {t}
                          </span>
                        ))}
                      </>
                    )}
                  </div>
                  <pre className="whitespace-pre-wrap break-words text-xs text-zinc-100 font-mono">
                    {r.body}
                  </pre>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}

interface RoleModalProps {
  open: boolean;
  pane?: PaneConfig;
  onClose: () => void;
  onSave: (
    label: string,
    role: string,
    goal: string,
    backstory: string,
    mode: PlanReviewMode,
  ) => void;
}

function RoleModal({ open, pane, onClose, onSave }: RoleModalProps) {
  const [label, setLabel] = useState(pane?.label ?? "");
  const [role, setRole] = useState(pane?.role ?? "");
  const [goal, setGoal] = useState(pane?.goal ?? "");
  const [backstory, setBackstory] = useState(pane?.backstory ?? "");
  const [mode, setMode] = useState<PlanReviewMode>(pane?.plan_review_mode ?? "never");
  // Reset fields when modal opens for a different pane (or re-opens).
  React.useEffect(() => {
    if (open) {
      setLabel(pane?.label ?? "");
      setRole(pane?.role ?? "");
      setGoal(pane?.goal ?? "");
      setBackstory(pane?.backstory ?? "");
      setMode(pane?.plan_review_mode ?? "never");
    }
  }, [open, pane?.pane_id, pane?.label, pane?.role, pane?.goal, pane?.backstory, pane?.plan_review_mode]);
  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="flex w-full max-w-2xl flex-col rounded-lg border border-zinc-700 bg-zinc-900 p-5 text-zinc-100 shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className="mb-1 text-lg font-semibold">
          Role · Goal · Backstory{pane?.label ? ` — ${pane.label}` : ""}
        </h3>
        <p className="mb-3 text-xs text-zinc-400">
          Appended to the agent&apos;s system prompt at next spawn (close + re-add the tab, or reboot the apas CLI to apply).
        </p>
        <div className="mb-4">
          <p className="mb-2 text-[11px] uppercase tracking-wide text-zinc-500">Quick pick — apply a team-role template</p>
          <div className="flex flex-wrap gap-2">
            {ROLE_TEMPLATES.map((t) => (
              <button
                key={t.id}
                type="button"
                onClick={() => {
                  setRole(t.role);
                  setGoal(t.goal);
                  setBackstory(t.backstory);
                  setMode(t.planReviewMode);
                }}
                className={`rounded border px-2 py-1 text-xs font-medium transition-colors ${TEMPLATE_COLOR_CLASSES[t.color]}`}
                title={`Apply ${t.label} template — populates role, goal, backstory, and plan review mode`}
              >
                <span className="mr-1">{t.glyph}</span>
                {t.label}
              </button>
            ))}
            <button
              type="button"
              onClick={() => {
                setRole("");
                setGoal("");
                setBackstory("");
                setMode("never");
              }}
              className="rounded border border-zinc-600 bg-zinc-800 px-2 py-1 text-xs font-medium text-zinc-300 transition-colors hover:bg-zinc-700"
              title="Clear all fields"
            >
              ✕ Clear
            </button>
          </div>
        </div>
        <div className="flex flex-col gap-3">
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-zinc-300">Name <span className="text-zinc-500">(label shown on the tab and pane card)</span></span>
            <input
              type="text"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 font-mono"
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-zinc-300">Role <span className="text-zinc-500">(e.g. &quot;backend implementer&quot;, &quot;reviewer&quot;)</span></span>
            <input
              type="text"
              value={role}
              onChange={(e) => setRole(e.target.value)}
              className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 font-mono"
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-zinc-300">Goal <span className="text-zinc-500">(this worker&apos;s responsibility / scope — what files, subsystems, branches)</span></span>
            <textarea
              value={goal}
              onChange={(e) => setGoal(e.target.value)}
              rows={3}
              className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 font-mono resize-y"
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-zinc-300">Backstory <span className="text-zinc-500">(free-form context, conventions, constraints)</span></span>
            <textarea
              value={backstory}
              onChange={(e) => setBackstory(e.target.value)}
              rows={6}
              className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 font-mono resize-y"
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-zinc-300">
              Plan review <span className="text-zinc-500">(gate tool execution behind user approval)</span>
            </span>
            <select
              value={mode}
              onChange={(e) => setMode(e.target.value as PlanReviewMode)}
              className="rounded border border-zinc-700 bg-zinc-800 px-2 py-1 text-zinc-100"
            >
              <option value="never">Never (default — agent runs tools freely)</option>
              <option value="risky_only">Risky only (gate Write / Edit / Bash / Task)</option>
              <option value="always">Always (gate every tool except AskUserQuestion)</option>
            </select>
          </label>
        </div>
        <div className="mt-5 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded border border-zinc-600 bg-zinc-800 px-3 py-1.5 text-sm hover:bg-zinc-700"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={() => onSave(label, role, goal, backstory, mode)}
            className="rounded border border-purple-700 bg-purple-700 px-3 py-1.5 text-sm hover:bg-purple-600"
          >
            Save
          </button>
        </div>
      </div>
    </div>
  );
}

interface PaneDiffModalProps {
  open: boolean;
  diff?: {
    branch?: string;
    base?: string;
    diff?: string;
    error?: string;
    fetchedAt: number;
  };
  onClose: () => void;
  onRefresh: () => void;
  onMerge: () => void;
  onDiscard: () => void;
  onCreatePr: () => void;
  manualPrCreationDisabled?: boolean;
}

// Split a unified `git diff` output into per-file sections. Each `diff
// --git a/<path> b/<path>` header opens a new section. Phase 1.2c.
function splitDiffByFile(text: string): Array<{ path: string; body: string }> {
  if (!text) return [];
  const lines = text.split("\n");
  const sections: Array<{ path: string; body: string }> = [];
  let current: { path: string; lines: string[] } | null = null;
  const flush = () => {
    if (current) {
      sections.push({ path: current.path, body: current.lines.join("\n") });
    }
  };
  for (const line of lines) {
    const m = line.match(/^diff --git a\/(.+) b\/(.+)$/);
    if (m) {
      flush();
      current = { path: m[2] || m[1], lines: [line] };
    } else if (current) {
      current.lines.push(line);
    }
  }
  flush();
  return sections;
}

interface DiffFileSectionProps {
  path: string;
  body: string;
}

function DiffFileSection({ path, body }: DiffFileSectionProps) {
  const [expanded, setExpanded] = useState(true);
  // Quick line-count summary so collapsed view still gives signal.
  let added = 0;
  let removed = 0;
  for (const line of body.split("\n")) {
    if (line.startsWith("+") && !line.startsWith("+++")) added++;
    else if (line.startsWith("-") && !line.startsWith("---")) removed++;
  }
  return (
    <div className="mb-3 rounded border border-zinc-800 bg-black/30">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="flex w-full items-center justify-between gap-2 px-3 py-2 text-left text-xs hover:bg-zinc-800/50"
      >
        <span className="font-mono text-zinc-200 break-all">{path}</span>
        <span className="flex flex-shrink-0 items-center gap-2 text-zinc-400">
          <span className="text-emerald-400">+{added}</span>
          <span className="text-red-400">-{removed}</span>
          <span>{expanded ? "▾" : "▸"}</span>
        </span>
      </button>
      {expanded && <CodeBlock code={body} language="diff" />}
    </div>
  );
}

export function PaneDiffModal({
  open,
  diff,
  onClose,
  onRefresh,
  onMerge,
  onDiscard,
  onCreatePr,
  manualPrCreationDisabled = false,
}: PaneDiffModalProps) {
  if (!open) return null;
  let body: React.ReactNode;
  if (diff?.error) {
    body = <pre className="whitespace-pre-wrap break-words text-red-300 text-xs">{diff.error}</pre>;
  } else if (diff?.diff === undefined) {
    body = <p className="text-zinc-400 text-sm italic">Loading…</p>;
  } else if (diff.diff.length === 0) {
    body = <p className="text-zinc-400 text-sm italic">No changes vs base.</p>;
  } else {
    const sections = splitDiffByFile(diff.diff);
    body = sections.length > 0
      ? <>{sections.map((s) => <DiffFileSection key={s.path} path={s.path} body={s.body} />)}</>
      : <pre className="whitespace-pre-wrap break-words text-zinc-100 text-xs font-mono">{diff.diff}</pre>;
  }
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="flex w-full max-w-4xl flex-col rounded-lg border border-zinc-700 bg-zinc-900 p-5 text-zinc-100 shadow-xl"
        style={{ maxHeight: "80vh" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-3 flex items-center justify-between gap-3">
          <h3 className="text-lg font-semibold">Pane diff</h3>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={onRefresh}
              className="rounded border border-zinc-600 bg-zinc-800 px-3 py-1 text-xs hover:bg-zinc-700"
            >
              Refresh
            </button>
            {manualPrCreationDisabled ? (
              <p className="max-w-[13rem] text-xs leading-5 text-zinc-400">
                Managed team panes open PRs after Reviewer approval.
              </p>
            ) : (
              <button
                type="button"
                onClick={onCreatePr}
                className="rounded border border-sky-600 bg-sky-900/40 px-3 py-1 text-xs text-sky-200 hover:bg-sky-900/60"
                title="Push this branch to origin and open a GitHub PR via `gh pr create --fill`. Requires `gh` on the CLI host."
              >
                Create PR
              </button>
            )}
            <button
              type="button"
              onClick={onMerge}
              className="rounded border border-emerald-700 bg-emerald-900/40 px-3 py-1 text-xs text-emerald-200 hover:bg-emerald-900/60"
              title="git merge --no-ff this pane's branch into HEAD, then close the pane. Aborts on conflict."
            >
              Merge &amp; close
            </button>
            <button
              type="button"
              onClick={onDiscard}
              className="rounded border border-red-700 bg-red-900/30 px-3 py-1 text-xs text-red-200 hover:bg-red-900/50"
              title="Force-remove the worktree and delete the branch, then close the pane. Destructive."
            >
              Discard
            </button>
            <button
              type="button"
              onClick={onClose}
              className="rounded border border-zinc-600 bg-zinc-800 px-3 py-1 text-xs hover:bg-zinc-700"
            >
              Close
            </button>
          </div>
        </div>
        {(diff?.branch || diff?.base) && (
          <div className="mb-2 text-xs text-zinc-400">
            {diff.base ?? "?"} <span className="text-zinc-500">→</span> {diff.branch ?? "?"}
          </div>
        )}
        <div className="flex-1 overflow-auto rounded border border-zinc-800 bg-black/30 p-3">
          {body}
        </div>
      </div>
    </div>
  );
}

interface WorktreeCleanupModalProps {
  open: boolean;
  worktreePath: string;
  onCancel: () => void;
  onConfirm: (action: PaneCleanupAction) => void;
}

function WorktreeCleanupModal({ open, worktreePath, onCancel, onConfirm }: WorktreeCleanupModalProps) {
  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick={onCancel}
    >
      <div
        className="w-full max-w-lg rounded-lg border border-zinc-700 bg-zinc-900 p-5 text-zinc-100 shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className="mb-1 text-lg font-semibold">Close pane with isolated worktree</h3>
        <p className="mb-4 break-all text-sm text-zinc-400">
          Worktree: <span className="font-mono text-zinc-200">{worktreePath}</span>
        </p>
        <div className="flex flex-col gap-2">
          <button
            type="button"
            onClick={() => onConfirm("leave_as_branch")}
            className="rounded border border-zinc-600 bg-zinc-800 px-3 py-2 text-left text-sm hover:bg-zinc-700"
          >
            <div className="font-medium">Leave as branch (safe)</div>
            <div className="text-xs text-zinc-400">
              Remove the worktree directory; keep the branch so you can review it later. If the worktree has uncommitted changes, removal is skipped and you&apos;ll be told to clean it up by hand.
            </div>
          </button>
          <button
            type="button"
            onClick={() => onConfirm("merge_and_remove")}
            className="rounded border border-zinc-600 bg-zinc-800 px-3 py-2 text-left text-sm hover:bg-zinc-700"
          >
            <div className="font-medium">Merge into current branch, then remove</div>
            <div className="text-xs text-zinc-400">
              git merge --no-ff the worktree&apos;s branch into the main checkout&apos;s HEAD, then remove the worktree and delete the branch. Aborts on conflicts (resolve manually with git).
            </div>
          </button>
          <button
            type="button"
            onClick={() => {
              if (!confirm("Permanently discard the worktree AND its branch? This cannot be undone.")) return;
              onConfirm("discard");
            }}
            className="rounded border border-red-700 bg-red-900/30 px-3 py-2 text-left text-sm hover:bg-red-900/50"
          >
            <div className="font-medium text-red-300">Discard everything</div>
            <div className="text-xs text-red-300/80">
              Force-remove the worktree and delete the branch. Loses uncommitted changes. Only pick this if the work is throwaway.
            </div>
          </button>
        </div>
        <div className="mt-4 flex justify-end">
          <button
            type="button"
            onClick={onCancel}
            className="rounded border border-zinc-600 bg-zinc-800 px-3 py-1.5 text-sm hover:bg-zinc-700"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}

interface ViewBotPromptModalProps {
  open: boolean;
  prompt: string;
  minIntervalMinutes: number;
  tabLabel: string;
  onClose: () => void;
}

function ViewBotPromptModal({
  open,
  prompt,
  minIntervalMinutes,
  tabLabel,
  onClose,
}: ViewBotPromptModalProps) {
  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-[100] bg-black/50 flex items-center justify-center p-4"
      onClick={onClose}
    >
      <div
        className="w-full max-w-2xl bg-white dark:bg-gray-900 rounded-xl shadow-xl border border-gray-200 dark:border-gray-700"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="px-4 py-3 border-b border-gray-200 dark:border-gray-700">
          <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">
            Bot Prompt for {tabLabel}
          </h3>
          <p className="text-xs mt-1 text-gray-500 dark:text-gray-400">
            This is the active prompt currently used by this bot tab.
          </p>
        </div>
        <div className="p-4 space-y-4">
          <div>
            <label className="block text-xs font-medium text-gray-700 dark:text-gray-300 mb-1">
              Minimum Interval Between Iterations (minutes)
            </label>
            <input
              type="text"
              value={String(minIntervalMinutes)}
              readOnly
              className="w-full rounded-lg border border-gray-300 dark:border-gray-600 bg-gray-100 dark:bg-gray-800 px-3 py-2 text-sm"
            />
          </div>
          <textarea
            value={prompt}
            readOnly
            rows={10}
            className="w-full rounded-lg border border-gray-300 dark:border-gray-600 bg-gray-100 dark:bg-gray-800 px-3 py-2 text-sm"
          />
        </div>
        <div className="px-4 py-3 border-t border-gray-200 dark:border-gray-700 flex items-center justify-end">
          <button
            onClick={onClose}
            className="px-3 py-1.5 text-xs font-medium rounded bg-indigo-500 hover:bg-indigo-600 text-white"
          >
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

interface StartBotPromptModalProps {
  open: boolean;
  prompt: string;
  minIntervalMinutes: string;
  tabLabel: string;
  onPromptChange: (value: string) => void;
  onMinIntervalChange: (value: string) => void;
  onCancel: () => void;
  onConfirm: () => void;
}

function StartBotPromptModal({
  open,
  prompt,
  minIntervalMinutes,
  tabLabel,
  onPromptChange,
  onMinIntervalChange,
  onCancel,
  onConfirm,
}: StartBotPromptModalProps) {
  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-[100] bg-black/50 flex items-center justify-center p-4"
      onClick={onCancel}
    >
      <div
        className="w-full max-w-2xl bg-white dark:bg-gray-900 rounded-xl shadow-xl border border-gray-200 dark:border-gray-700"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="px-4 py-3 border-b border-gray-200 dark:border-gray-700">
          <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">
            Start Bot on {tabLabel}
          </h3>
          <p className="text-xs mt-1 text-gray-500 dark:text-gray-400">
            Edit the loop prompt for this tab. Classic/manual side-chat tabs default to the TODO.md loop; managed team panes default to their role metadata. This prompt is saved per tab in `.apas`.
          </p>
        </div>
        <div className="p-4 space-y-4">
          <div>
            <label className="block text-xs font-medium text-gray-700 dark:text-gray-300 mb-1">
              Minimum Interval Between Iterations (minutes)
            </label>
            <input
              type="number"
              min={0}
              step={1}
              value={minIntervalMinutes}
              onChange={(e) => onMinIntervalChange(e.target.value)}
              className="w-full rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
              placeholder={String(DEFAULT_BOT_MIN_INTERVAL_MINUTES)}
            />
            <p className="text-[11px] mt-1 text-gray-500 dark:text-gray-400">
              Default is 15. Next iteration starts no sooner than this interval since the previous iteration started.
            </p>
          </div>
          <textarea
            value={prompt}
            onChange={(e) => onPromptChange(e.target.value)}
            rows={10}
            className="w-full rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
            placeholder="Enter bot loop prompt..."
          />
        </div>
        <div className="px-4 py-3 border-t border-gray-200 dark:border-gray-700 flex items-center justify-end gap-2">
          <button
            onClick={onCancel}
            className="px-3 py-1.5 text-xs font-medium rounded bg-gray-200 hover:bg-gray-300 dark:bg-gray-700 dark:hover:bg-gray-600 text-gray-800 dark:text-gray-100"
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            className="px-3 py-1.5 text-xs font-medium rounded bg-green-500 hover:bg-green-600 text-white"
          >
            Start Bot
          </button>
        </div>
      </div>
    </div>
  );
}

// --- TimelinePane (Phase 4.2b) ---

interface TimelinePaneProps {
  messages: Message[];
}

function TimelinePane({ messages }: TimelinePaneProps) {
  const entries = useMemo(() => extractTimeline(messages), [messages]);
  const [expandedIds, setExpandedIds] = useState<Set<number>>(new Set());
  if (entries.length === 0) {
    return (
      <div className="flex-1 overflow-auto p-4 text-sm italic text-gray-500 dark:text-gray-400">
        No tool calls in this pane yet. The Timeline view fills in as the agent uses tools.
      </div>
    );
  }
  return (
    <div className="flex-1 overflow-auto p-3">
      <ul className="flex flex-col gap-1.5">
        {entries.map((e: TimelineEntry, i: number) => {
          const expanded = expandedIds.has(i);
          const status = e.ok === undefined ? "•" : e.ok ? "✓" : "✗";
          const statusColor = e.ok === undefined
            ? "text-gray-400"
            : e.ok ? "text-emerald-500" : "text-red-500";
          return (
            <li
              key={e.toolUseId ?? `${e.tool}-${i}`}
              className="rounded border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800/40"
            >
              <button
                type="button"
                onClick={() => {
                  setExpandedIds((prev) => {
                    const next = new Set(prev);
                    if (next.has(i)) next.delete(i);
                    else next.add(i);
                    return next;
                  });
                }}
                className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-gray-50 dark:hover:bg-gray-700/30"
              >
                <span className={`flex-shrink-0 font-bold ${statusColor}`}>{status}</span>
                <span className="font-mono text-gray-800 dark:text-gray-200 flex-shrink-0">{e.tool}</span>
                {e.argSummary && (
                  <span className="truncate font-mono text-gray-600 dark:text-gray-400">
                    {e.argSummary}
                  </span>
                )}
                {e.resultSummary && (
                  <span className="ml-auto flex-shrink-0 truncate font-mono text-gray-500 dark:text-gray-500 max-w-[40%]">
                    → {e.resultSummary}
                  </span>
                )}
                <span className="ml-2 flex-shrink-0 text-gray-400">{expanded ? "▾" : "▸"}</span>
              </button>
              {expanded && (
                <div className="border-t border-gray-200 dark:border-gray-700 px-3 py-2 text-xs">
                  <div className="mb-1 text-gray-500 dark:text-gray-400">input:</div>
                  <pre className="mb-2 whitespace-pre-wrap break-words font-mono text-gray-800 dark:text-gray-200">
                    {(() => {
                      try {
                        return JSON.stringify(e.input, null, 2);
                      } catch {
                        return String(e.input);
                      }
                    })()}
                  </pre>
                  {e.resultBody !== undefined && (
                    <>
                      <div className="mb-1 text-gray-500 dark:text-gray-400">result:</div>
                      <pre className="whitespace-pre-wrap break-words font-mono text-gray-800 dark:text-gray-200">
                        {e.resultBody}
                      </pre>
                    </>
                  )}
                </div>
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}

// --- MessagePane ---

export interface MessagePaneProps {
  paneId: number;
  messages: Message[];
  onLoadMore?: () => void;
  isLoading?: boolean;
  hasMore?: boolean;
  /** True when this pane is the one the user currently sees. Drives
   * scroll save (going hidden) and restore (becoming visible). When
   * false, the pane is mounted but `display: none`. Phase: hide-not-
   * unmount switching. */
  isActive: boolean;
  /** Pane role string used to tailor the first-run empty state. The
   * Manager pane gets a chat-first prompt; everyone else falls back to
   * the generic placeholder. */
  role?: string;
}

// Initial number of messages mounted per pane. The full backlog stays in
// state; we only render the newest slice so opening a session with hundreds
// of cached messages doesn't block the main thread parsing markdown and
// syntax-highlighting every one at once. "Show earlier messages" (or
// scrolling to the top) reveals more from the local cache before paging the
// server.
export const INITIAL_RENDER_CAP = 30;
export const RENDER_CAP_STEP = 50;

/// Upper bound on how many raw messages the auto-fill (in MessagePane) will
/// page in while chasing `INITIAL_RENDER_CAP` non-tool messages. Tool-heavy
/// panes can run ~13 folded tool messages per real message — the newest 50
/// of one cluster pane held a single non-tool message — so without a bound a
/// pane that is almost all tool calls would page its entire history on open.
/// 400 keeps the initial catch-up fetch bounded (mobile payload) while still
/// reaching ~30 real messages for even very tool-heavy panes; anything older
/// loads on scroll-up.
export const AUTO_FILL_MESSAGE_CAP = 400;

/// How many of the oldest messages to keep unmounted. Only NON-tool
/// messages count toward the reveal budget (`revealCap`): a turn's folded
/// tool_use / tool_result rows shouldn't crowd the actual conversation out
/// of the window — 15 tool rows used to mean zero visible text. Interspersed
/// tool messages are still rendered, they just don't consume the budget.
/// Scans back from the newest until `revealCap` non-tool messages are in
/// view and hides everything older (returns the index of the oldest shown
/// message, i.e. the count hidden at the front).
export function computeHiddenCount(messages: Message[], revealCap: number): number {
  let normalSeen = 0;
  for (let i = messages.length - 1; i >= 0; i--) {
    if (!isToolLikeMessage(messages[i])) {
      normalSeen++;
      if (normalSeen >= revealCap) return i;
    }
  }
  return 0;
}

// Placeholder pinned to the top of a pane while older history is paged in
// from the server (i.e. the user scrolled to the top and the local render
// cache is already exhausted). A labelled spinner plus a couple of muted
// skeleton rows reads as "earlier history is loading up here" rather than a
// bare "Loading..." string, and reserves vertical space so the prepend
// doesn't visibly jump.
function HistoryLoadingPlaceholder({ paneId }: { paneId: number }) {
  return (
    <div
      data-testid={`history-loading-${paneId}`}
      aria-live="polite"
      className="pb-1"
    >
      <div className="flex items-center justify-center gap-2 py-1.5 text-xs font-medium text-gray-400 dark:text-gray-500">
        <span className="w-3.5 h-3.5 rounded-full border-2 border-gray-300 border-t-transparent dark:border-gray-600 dark:border-t-transparent animate-spin" />
        Fetching earlier history…
      </div>
      <div aria-hidden="true" className="space-y-3 px-1 opacity-70">
        {[0, 1].map((row) => (
          <div key={row} className="animate-pulse space-y-2">
            <div className="h-2.5 w-20 rounded bg-gray-200 dark:bg-gray-700/70" />
            <div className="h-2.5 w-full rounded bg-gray-200 dark:bg-gray-700/70" />
            <div className="h-2.5 w-3/4 rounded bg-gray-200 dark:bg-gray-700/70" />
          </div>
        ))}
      </div>
    </div>
  );
}

export function MessagePane({ paneId, messages, onLoadMore, isLoading, hasMore, isActive, role }: MessagePaneProps) {
  const sessionId = useStore((s) => s.sessionId);
  // How many newest messages to mount. Grows when the user reveals older
  // ones. `expanded` latches once they page all the way back so server-
  // loaded older messages (which prepend) stay rendered instead of being
  // re-clipped by the tail window.
  const [revealCount, setRevealCount] = useState(INITIAL_RENDER_CAP);
  const [expanded, setExpanded] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const shouldAutoScroll = useRef(true);
  const prevScrollHeight = useRef<number>(0);
  const isRestoringScroll = useRef(false);
  const scrollThrottleRef = useRef<number>(0);
  // Tracks `messages.length` between renders so the auto-scroll effect
  // can tell "single new message arrived" (smooth) apart from "100
  // messages just appeared at once" (snap to bottom).
  const prevMessageCountRef = useRef<number>(0);
  // Guards the auto-fill loop below: the raw message count at the last
  // auto-triggered load. If a load doesn't grow the window (e.g. a page of
  // older messages that all belong to other panes, so hasMore stays true but
  // nothing new lands for this pane), stop rather than paging forever.
  const autoFillLastLenRef = useRef(-1);
  // `null` on first render so we can distinguish "first time we see
  // isActive" from a later transition. After the effect runs once,
  // it holds the previous value.
  const wasActiveRef = useRef<boolean | null>(null);
  // Mirror the current render-cap state into refs so the (stable) scroll
  // handler can read them without being torn down/rebuilt on every reveal.
  const hiddenCountRef = useRef(0);
  const expandedRef = useRef(false);
  expandedRef.current = expanded;

  const scrollKey = getScrollKey(sessionId, paneId);

  // Reveal the next chunk of locally-cached older messages, preserving the
  // viewport (the prepend-scroll effect below restores it via
  // prevScrollHeight once the taller list commits).
  const revealEarlier = useCallback(() => {
    prevScrollHeight.current = containerRef.current?.scrollHeight || 0;
    setRevealCount((n) => n + RENDER_CAP_STEP);
  }, []);

  const checkIfAtBottom = useCallback(() => {
    const container = containerRef.current;
    if (!container) return true;
    return container.scrollHeight - container.scrollTop - container.clientHeight <= 100;
  }, []);

  const checkIfNearTop = useCallback(() => {
    const container = containerRef.current;
    if (!container) return false;
    return container.scrollTop < 100;
  }, []);

  const handleScroll = useCallback(() => {
    if (isRestoringScroll.current) return;

    // Always update auto-scroll flag (cheap check)
    shouldAutoScroll.current = checkIfAtBottom();

    // Throttle the expensive parts (scroll position saving, load-more checks)
    const now = Date.now();
    if (now - scrollThrottleRef.current < 250) return;
    scrollThrottleRef.current = now;

    if (containerRef.current) {
      scrollPositions.set(scrollKey, {
        scrollTop: containerRef.current.scrollTop,
        wasAtBottom: shouldAutoScroll.current,
        scrollHeight: containerRef.current.scrollHeight,
        clientHeight: containerRef.current.clientHeight,
      });
    }

    if (checkIfNearTop()) {
      if (hiddenCountRef.current > 0) {
        // Still more in the local cache — reveal those before hitting the
        // server so the common case stays instant and network-free.
        revealEarlier();
      } else if (onLoadMore && !isLoading && hasMore) {
        // Local cache exhausted: latch to render-all so server-prepended
        // older messages aren't re-clipped, then page the server.
        if (!expandedRef.current) setExpanded(true);
        prevScrollHeight.current = containerRef.current?.scrollHeight || 0;
        onLoadMore();
      }
    }
  }, [checkIfAtBottom, checkIfNearTop, onLoadMore, isLoading, hasMore, scrollKey, revealEarlier]);

  // Save scroll position on unmount. With hide-not-unmount, "unmount"
  // now only fires on session change (key includes sessionId). Tab
  // switches go through the isActive transition effect below.
  useEffect(() => {
    return () => {
      const container = containerRef.current;
      if (container) {
        const wasAtBottom =
          container.scrollHeight - container.scrollTop - container.clientHeight <= 100;
        scrollPositions.set(scrollKey, {
          scrollTop: container.scrollTop,
          wasAtBottom,
          scrollHeight: container.scrollHeight,
          clientHeight: container.clientHeight,
        });
      }
    };
  }, [scrollKey]);

  // Per-pane scroll lifecycle, driven by isActive transitions:
  //   - first activation: scroll to bottom if no saved state, else restore.
  //   - active → hidden: save current scroll position.
  //   - hidden → active: restore saved position.
  // Initial mount as inactive is a no-op (we'll handle it on first
  // activation). Replaces the old "restore once on mount" effect,
  // which would fire for every pane on initial render and miss the
  // tab-switch transitions entirely.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const prev = wasActiveRef.current;
    wasActiveRef.current = isActive;

    // Hidden → hidden or first render while hidden: nothing to do.
    if (!isActive && prev !== true) return;

    if (prev === true && !isActive) {
      // Going hidden: snapshot scroll for later restore.
      const wasAtBottom =
        container.scrollHeight - container.scrollTop - container.clientHeight <= 100;
      scrollPositions.set(scrollKey, {
        scrollTop: container.scrollTop,
        wasAtBottom,
        scrollHeight: container.scrollHeight,
        clientHeight: container.clientHeight,
      });
      return;
    }

    if (isActive) {
      // First activation OR coming back into view: restore saved state
      // if present, otherwise scroll to the bottom (latest messages).
      const saved = scrollPositions.get(scrollKey);
      isRestoringScroll.current = true;
      if (saved) {
        shouldAutoScroll.current = saved.wasAtBottom;
        requestAnimationFrame(() => {
          const c = containerRef.current;
          if (c) {
            const maxScrollTop = Math.max(0, c.scrollHeight - c.clientHeight);
            if (saved.wasAtBottom) {
              c.scrollTop = maxScrollTop;
            } else {
              const savedMaxScrollTop = Math.max(
                0,
                saved.scrollHeight - saved.clientHeight,
              );
              const nextScrollTop = savedMaxScrollTop > 0
                ? (saved.scrollTop / savedMaxScrollTop) * maxScrollTop
                : saved.scrollTop;
              c.scrollTop = Math.max(0, Math.min(maxScrollTop, nextScrollTop));
            }
          }
          isRestoringScroll.current = false;
        });
      } else {
        shouldAutoScroll.current = true;
        requestAnimationFrame(() => {
          const c = containerRef.current;
          if (c) c.scrollTop = c.scrollHeight;
          isRestoringScroll.current = false;
        });
      }
    }
  }, [isActive, scrollKey]);

  // Maintain scroll position when prepending older messages (loadMore).
  // Only meaningful for the active pane — hidden panes can't scroll-to-
  // top to trigger loadMore in the first place.
  useEffect(() => {
    if (!isActive) return;
    if (prevScrollHeight.current > 0 && containerRef.current) {
      const newScrollHeight = containerRef.current.scrollHeight;
      const scrollDiff = newScrollHeight - prevScrollHeight.current;
      if (scrollDiff > 0) {
        containerRef.current.scrollTop = scrollDiff;
      }
      prevScrollHeight.current = 0;
    }
  }, [messages.length, revealCount, isActive]);

  // Auto-scroll for new messages — only for the active pane. Hidden
  // panes accumulate messages silently; when the user comes back, the
  // becameVisible branch above restores them to wherever they were
  // (or to the bottom if they had been at the bottom on save).
  // See decideAutoScrollMode for why "smooth" is only for small jumps.
  useEffect(() => {
    const prev = prevMessageCountRef.current;
    prevMessageCountRef.current = messages.length;
    const mode = decideAutoScrollMode({
      isActive,
      shouldAutoScroll: shouldAutoScroll.current,
      isRestoringScroll: isRestoringScroll.current,
      prevCount: prev,
      newCount: messages.length,
    });
    if (mode === "instant") {
      requestAnimationFrame(() => {
        const c = containerRef.current;
        if (c) c.scrollTop = c.scrollHeight;
      });
    } else if (mode === "smooth") {
      messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [messages.length, isActive]);

  // Auto-fill the conversation window. Tool-heavy panes can have their newest
  // messages be almost entirely folded tool calls — one cluster pane's newest
  // 50 messages held a single real message — so the initial fetch lands the
  // user on a wall of tool cards with nothing to read. Page older messages in
  // until the loaded set holds INITIAL_RENDER_CAP non-tool messages, history
  // runs out (hasMore), or we hit the AUTO_FILL_MESSAGE_CAP payload bound.
  // Active pane only; sequential — each load flips isLoading, so the next
  // fires only once the prior settles. Targets the fixed INITIAL_RENDER_CAP
  // (not the growable revealCount) so paging further back via "Show earlier"
  // never retriggers a fresh fill.
  useEffect(() => {
    if (!isActive || !onLoadMore || isLoading || !hasMore) return;
    if (messages.length >= AUTO_FILL_MESSAGE_CAP) return;
    let nonTool = 0;
    for (const m of messages) {
      if (!isToolLikeMessage(m)) {
        nonTool += 1;
        if (nonTool >= INITIAL_RENDER_CAP) return; // enough real messages loaded
      }
    }
    // No progress since the last auto-load means paging further won't help
    // this pane (a page landed nothing new for it) — stop instead of looping.
    if (messages.length === autoFillLastLenRef.current) return;
    autoFillLastLenRef.current = messages.length;
    onLoadMore();
  }, [isActive, messages, hasMore, isLoading, onLoadMore]);

  // Only mount the newest `revealCount` messages until the user expands.
  // `expanded` latches once they've paged all the way back, so older
  // server-loaded (prepended) messages keep rendering. Computed BEFORE any
  // early return so the useMemo below always runs — otherwise a pane that
  // first renders empty then receives messages changes its hook count
  // between renders (React error #310: "rendered more hooks than before").
  const hiddenCount = expanded ? 0 : computeHiddenCount(messages, revealCount);
  hiddenCountRef.current = hiddenCount;
  const visibleMessages = hiddenCount > 0 ? messages.slice(hiddenCount) : messages;
  // Second-level fold: collapse consecutive tool_use / tool_result runs of at
  // least TOOL_GROUP_MIN_ITEMS into one expandable ToolGroupCard so long
  // tool-chain turns don't drown the readable text. Individual ToolCards
  // inside remain foldable; AskUserQuestion stays inline.
  const renderItems = useMemo(
    () => groupMessagesForRender(visibleMessages),
    [visibleMessages],
  );

  if (messages.length === 0) {
    const lowerRole = (role ?? "").toLowerCase();
    const isManager =
      lowerRole.includes("manager") && !lowerRole.includes("tech lead");
    if (isManager) {
      return (
        <div className="flex-1 flex items-center justify-center text-gray-500 dark:text-gray-400 px-4">
          <div className="max-w-md text-center">
            <p className="text-base font-medium text-gray-700 dark:text-gray-200">
              Talk to your Manager
            </p>
            <p className="mt-2 text-xs leading-relaxed">
              State the project goal in one or two sentences below — the
              Manager keeps <span className="font-mono">project_goal.md</span> in
              sync and hands work to the Tech Lead, who delegates to workers.
            </p>
            <p className="mt-2 text-[11px] opacity-75">
              Tip: try <em>&ldquo;Scan the repo and draft a starter goal&rdquo;</em> or
              describe the next milestone yourself.
            </p>
          </div>
        </div>
      );
    }
    return (
      <div className="flex-1 flex items-center justify-center text-gray-400 px-4">
        <div className="text-center">
          <p className="text-sm">No messages yet</p>
          <p className="text-xs mt-1 opacity-75">Type a message below to start</p>
        </div>
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      data-testid={`message-pane-${paneId}`}
      onScroll={handleScroll}
      className="flex-1 overflow-y-auto overflow-x-hidden px-2 sm:px-4 py-4 space-y-3 min-h-0"
    >
      {isLoading && <HistoryLoadingPlaceholder paneId={paneId} />}
      {hiddenCount > 0 && (
        <div className="text-center py-1">
          <button
            onClick={revealEarlier}
            className="text-xs text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200 px-3 py-1 rounded-full border border-gray-200 dark:border-gray-700 hover:bg-gray-100 dark:hover:bg-gray-800 transition-colors"
          >
            Show earlier messages ({hiddenCount})
          </button>
        </div>
      )}
      {renderItems.map((item) =>
        item.kind === "tool-group" ? (
          <ToolGroupCard key={item.id} items={item.items} />
        ) : (
          <MessageComponent key={item.message.id} message={item.message} />
        ),
      )}
      <div ref={messagesEndRef} />
    </div>
  );
}

// --- MessageComponent ---

const MessageComponent = memo(function MessageComponent({ message }: { message: Message }) {
  switch (message.role) {
    case "user":
      return <UserMessage message={message} />;
    case "assistant":
      return <AssistantMessage message={message} />;
    case "system":
      return (
        <div className="text-center text-xs text-gray-500 py-1">
          <span>{message.content}</span>
        </div>
      );
    default:
      return null;
  }
});

// --- InteractiveInput ---

interface InteractiveInputProps {
  draft: string;
  onDraftChange: (value: string) => void;
  onSend: (text: string) => { success: boolean; error?: string };
}

function InteractiveInput({ draft, onDraftChange, onSend }: InteractiveInputProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [error, setError] = useState<string | null>(null);

  const resizeTextarea = useCallback(() => {
    const textarea = textareaRef.current;
    if (textarea) {
      textarea.style.height = "auto";
      textarea.style.height = Math.min(textarea.scrollHeight, 150) + "px";
    }
  }, []);

  useEffect(() => {
    resizeTextarea();
  }, [draft, resizeTextarea]);

  const handleSubmit = () => {
    const text = draft.trim();
    if (text) {
      const result = onSend(text);
      if (result.success) {
        onDraftChange("");
        if (textareaRef.current) textareaRef.current.style.height = "auto";
        setError(null);
      } else {
        setError(result.error || "Failed to send message");
        setTimeout(() => setError(null), 5000);
      }
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
  };

  const handleInput = (value: string) => {
    onDraftChange(value);
    resizeTextarea();
    if (error) setError(null);
  };

  return (
    <div className="space-y-2">
      {error && (
        <div className="text-sm text-red-500 bg-red-50 dark:bg-red-900/20 px-3 py-2 rounded-lg">
          {error}
        </div>
      )}
      <div className="flex gap-2">
        <textarea
          ref={textareaRef}
          value={draft}
          rows={1}
          placeholder="Type a message..."
          className="flex-1 resize-none rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
          onKeyDown={handleKeyDown}
          onChange={(e) => handleInput(e.target.value)}
        />
        <button
          onClick={handleSubmit}
          className="px-4 py-2 bg-blue-500 hover:bg-blue-600 text-white rounded-lg text-sm font-medium transition-colors"
        >
          Send
        </button>
      </div>
    </div>
  );
}
