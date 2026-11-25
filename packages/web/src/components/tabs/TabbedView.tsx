"use client";

import React, { useRef, useCallback, useEffect, useState, useMemo, memo } from "react";
import dynamic from "next/dynamic";
import { useStore, Message, PaneConfig, PaneCleanupAction, PaneKind, PlanReviewMode, PANE_ID_DEADLOOP, PANE_ID_INTERACTIVE, paneKey, selectCliLifecycleInventory, type SupportedProvider } from "@/lib/store";
import { useTerminalViewModes, type TerminalViewMode } from "@/lib/terminalViewMode";
import { OverviewView } from "../overview/OverviewView";
import { UserMessage } from "../chat/UserMessage";
import { AssistantMessage } from "../chat/AssistantMessage";
import { ToolGroupCard, groupMessagesForRender, isToolLikeMessage } from "../chat/ToolGroupCard";
import { TabBar } from "./TabBar";
import { ProjectWorkersWorkspace } from "../workers/ProjectWorkersWorkspace";
import { UsageLimitsDisplay } from "../UsageLimits";
import { CodeBlock } from "../code/CodeBlock";
import { useIsLaunchProfileAllowed } from "@/lib/tabTypes";
import {
  canonicalDeepseekModel,
  DEEPSEEK_FLASH_MODEL,
  isRetiredPiProvider,
  isRetiredProviderModel,
  PI_RETIRED_MESSAGE,
} from "@/lib/providerOptions";
import { resolveMachineProjectTarget } from "@/lib/machineProjectTarget";
import { paneIsAwaitingAnswerStatus } from "@/lib/paneStatus";
import { instancesNegotiated, primaryInstance, selectSessionInstances } from "@/lib/projectInstances";
import { buildMachineChoices, sessionCanCloneJoin, type MachineChoice } from "@/lib/machineChoice";
import {
  cloneHereCopy,
  cloneInstanceName,
  confirmationText,
  describeTabHosts,
  forgetMachineTabsCopy,
  instanceRebootTargets,
  sessionIsOwned,
} from "@/lib/tabHosts";
import { AddPaneLaunches } from "./AddPaneLaunches";
import {
  AUTOPILOT_PROJECT_NOT_RUNNING_TEXT,
  autopilotRequestsOfSession,
  closeProjectAutopilotNote,
  closeTabAutopilotNote,
  paneCanHaveAutopilot,
  rebootAgentAutopilotNote,
} from "@/lib/autopilot";
import { useAutopilotNegotiated, useSessionAutopilotStatuses } from "@/lib/useAutopilot";
import { AutopilotControl } from "./AutopilotControl";
import { LoopPromptControl } from "./LoopPromptControl";
import { LOOP_PROMPT_OFFLINE_TEXT, paneCanHaveLoopPrompt } from "@/lib/loopPrompt";
import { AutopilotRequestNotices, AutopilotStatusLine } from "./AutopilotStatus";
import {
  buildProjectOpenModel,
  NOTHING_SENT_TEXT,
  openStateNegotiated,
  selectProjectOpenReport,
} from "@/lib/projectOpenState";
import { projectNameFor } from "@/lib/projectList";
import { DeleteProjectDialog } from "../project/DeleteProjectDialog";
import { ProjectOpenNotice } from "./ProjectOpenNotice";

// xterm.js touches `document` at import time, so it can't be part of the
// server bundle. Loading it lazily also keeps the ~300 KB emulator out of
// the initial payload for the (common) case of no terminal panes.
import { TerminalViewToggle } from "./TerminalViewToggle";
import { TerminalChatInput } from "./TerminalChatInput";
import { PaneWorkSummaryDrawer } from "./PaneWorkSummaryDrawer";

function useDesktopViewport(): boolean {
  const [desktop, setDesktop] = useState(() =>
    typeof window !== "undefined" ? window.innerWidth >= 768 : false,
  );
  useEffect(() => {
    if (typeof window.matchMedia !== "function") {
      const updateFromWidth = () => setDesktop(window.innerWidth >= 768);
      window.addEventListener("resize", updateFromWidth);
      return () => window.removeEventListener("resize", updateFromWidth);
    }
    const query = window.matchMedia("(min-width: 768px)");
    const update = () => setDesktop(query.matches);
    update();
    query.addEventListener?.("change", update);
    return () => query.removeEventListener?.("change", update);
  }, []);
  return desktop;
}

/**
 * A terminal pane with its two controlled views. The active mode lives in
 * `TabbedView`, where the toolbar switch is rendered.
 */
function TerminalPaneWithViews({
  sessionId,
  paneId,
  messages,
  mode,
  isActive,
  onLoadMore,
  isLoading,
  hasMore,
  disabledReason,
}: {
  sessionId: string | null;
  paneId: number;
  messages: Message[];
  mode: TerminalViewMode;
  /** Whether this pane's tab is the one on screen. */
  isActive: boolean;
  onLoadMore: () => void;
  isLoading: boolean;
  hasMore: boolean;
  /** Why the pane cannot take input now, e.g. its machine is not running
   *  the project. Disables both views' input, and the conversation's
   *  questions and approvals. */
  disabledReason?: string;
}) {
  return (
    <>
      {/* Hidden rather than unmounted — see the call site. */}
      <div className={mode === "terminal" ? "flex-1 flex flex-col min-h-0" : "hidden"}>
        {/* The terminal is on screen only when its tab is active *and* the
            toggle is showing the terminal rather than the transcript. Both
            hide it the same way, so both must clear this flag or focus is
            left on the document body. */}
        <TerminalPane
          key={`terminal-${sessionId}-${paneId}`}
          paneId={paneId}
          visible={isActive && mode === "terminal"}
          // Still shows the retained screen, but sends no keys or sizes the
          // server could only refuse; the banner above says why.
          inputDisabledReason={disabledReason}
        />
      </div>
      {mode === "conversation" && (
        <>
          <MessagePane
            key={`terminal-chat-${sessionId}-${paneId}`}
            paneId={paneId}
            messages={messages}
            onLoadMore={onLoadMore}
            isLoading={isLoading}
            hasMore={hasMore}
            isActive={isActive}
            disabledReason={disabledReason}
          />
          {/* Writes go into the pty, not through MCP — a tool server can only
              answer calls, never push a turn into a live conversation. */}
          <TerminalChatInput paneId={paneId} disabledReason={disabledReason} />
        </>
      )}
    </>
  );
}

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
const PANE_REBOOT_CONFIRM_MESSAGE =
  "Reboot this pane's agent? The running process is killed and respawned on the SAME session (so the agent resumes with its prior conversation context).";
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
  /** Whether an authoritative pane roster has arrived for this project. */
  paneListReceived: boolean;
  savedActiveTab: string;
  tabIds: number[];
}): number | null {
  // Overview belongs to the project workspace, never to the pane roster.
  if (args.tabIds.length === 0) return null;

  // Same project, current pick is still valid -> no change.
  if (
    !args.clientChanged &&
    args.activeTabId != null &&
    args.tabIds.includes(args.activeTabId)
  ) {
    return args.activeTabId;
  }
  // Same project, pane_list is synthesized (no authoritative panes yet)
  // -> keep current selection to avoid jumps from transient data.
  //
  // Only while the roster is genuinely unknown. Once one has arrived and is
  // empty, holding the previous selection pins the workspace to a pane that
  // no longer exists — the one the user just closed.
  if (
    !args.clientChanged &&
    args.activeTabId != null &&
    !args.paneListReceived &&
    args.paneConfigsLength === 0
  ) {
    return args.activeTabId;
  }

  const savedNum = args.savedActiveTab ? parseInt(args.savedActiveTab, 10) : NaN;
  if (!isNaN(savedNum) && args.tabIds.includes(savedNum)) {
    return savedNum;
  }

  // Keep the existing Manager preference, otherwise select a real pane.
  return args.managerTabId ?? args.tabIds[0] ?? null;
}

export function lazyPaneMessageLoadTargets(args: {
  activeTabId: number | null;
  tabIds: number[];
}): number[] {
  if (args.activeTabId == null) return [];
  if (!args.tabIds.includes(args.activeTabId)) return [];
  return [args.activeTabId];
}

export function shouldShowPaneRebootButton(activeTabId: number | null): boolean {
  return activeTabId != null;
}

export function confirmedPaneRebootTarget(
  activeTabId: number | null,
  confirmFn: (message: string) => boolean,
  /** What else the reboot does to this tab, such as pausing its autopilot. */
  note?: string,
): number | null {
  if (!shouldShowPaneRebootButton(activeTabId)) return null;
  const message = note ? `${PANE_REBOOT_CONFIRM_MESSAGE}\n\n${note}` : PANE_REBOOT_CONFIRM_MESSAGE;
  return confirmFn(message) ? activeTabId : null;
}

export function requestConfirmedPaneReboot(
  activeTabId: number | null,
  confirmFn: (message: string) => boolean,
  rebootPane: (paneId: number) => void,
  note?: string,
): void {
  const target = confirmedPaneRebootTarget(activeTabId, confirmFn, note);
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

function isDeepseekModel(model?: string): boolean {
  if (typeof model !== "string") return false;
  const normalized = model.trim().toLowerCase();
  return normalized.includes("deepseek");
}

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
  { value: "antigravity", label: "Antigravity" },
  { value: "omp", label: "OMP" },
  { value: "deepseek", label: "DeepSeek" },
] as const;
type ProviderOption = (typeof PROVIDER_OPTIONS)[number]["value"];
const PROVIDER_LABEL: Record<ProviderOption, string> = Object.fromEntries(
  PROVIDER_OPTIONS.map((o) => [o.value, o.label]),
) as Record<ProviderOption, string>;


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
  const accountId = useStore((state) => state.userId);
  const pendingPaneSelection = useStore((s) => s.pendingPaneSelection);
  const clearPendingPaneSelection = useStore((s) => s.clearPendingPaneSelection);
  const negotiatedCapabilities = useStore((s) => s.negotiatedCapabilities);
  const messages = useStore((s) => s.messages);
  const paneConfigs = useStore((s) => s.paneConfigs);
  // Distinguishes "the roster is empty" from "no roster has arrived";
  // see the field's doc comment in the store.
  const paneListReceived = useStore((s) => s.paneListReceived);
  const paneMessages = useStore((s) => s.paneMessages);
  const paneHasMore = useStore((s) => s.paneHasMore);
  const paneStatuses = useStore((s) => s.paneStatuses);
  const paneModes = useStore((s) => s.paneModes);
  const pausedPanes = useStore((s) => s.pausedPanes);
  const loadingMorePane = useStore((s) => s.loadingMorePane);
  const isAttached = useStore((s) => s.isAttached);
  const departedProject = useStore((s) => s.departedProject);
  const isDualPane = useStore((s) => s.isDualPane);
  const hasMoreMessages = useStore((s) => s.hasMoreMessages);
  const isLoadingMore = useStore((s) => s.isLoadingMore);
  const cliClientId = useStore((s) => s.cliClientId);
  const sessions = useStore((s) => s.sessions);
  const machines = useStore((s) => s.machines);
  const isLaunchProfileAllowed = useIsLaunchProfileAllowed();
  const sendMessageToPane = useStore((s) => s.sendMessageToPane);
  const loadMoreMessages = useStore((s) => s.loadMoreMessages);
  const loadPaneMessagesIfNeeded = useStore((s) => s.loadPaneMessagesIfNeeded);
  const refreshPaneWindow = useStore((s) => s.refreshPaneWindow);
  const connected = useStore((s) => s.connected);
  const addPane = useStore((s) => s.addPane);
  const removePane = useStore((s) => s.removePane);
  const updatePaneLabel = useStore((s) => s.updatePaneLabel);
  const updatePaneModel = useStore((s) => s.updatePaneModel);
  const interruptPane = useStore((s) => s.interruptPane);
  const reorderPanes = useStore((s) => s.reorderPanes);
  const openProject = useStore((s) => s.openProject);
  const closeProject = useStore((s) => s.closeProject);
  const rebootCli = useStore((s) => s.rebootCli);
  const cloneAndAddPane = useStore((s) => s.cloneAndAddPane);
  const dismissAddPaneRequest = useStore((s) => s.dismissAddPaneRequest);
  const forgetMachinePanes = useStore((s) => s.forgetMachinePanes);
  const addPaneRequests = useStore((s) => s.addPaneRequests);
  const autopilotRequests = useStore((s) => s.autopilotRequests);
  const dismissAutopilotRequest = useStore((s) => s.dismissAutopilotRequest);
  // No autopilot control is shown at all on a server without autopilot.
  const autopilotNegotiated = useAutopilotNegotiated();
  // The project's attached instances, one per machine. Selector-safe: an
  // unchanged set keeps its identity.
  const instances = useStore((s) => selectSessionInstances(s, s.sessionId));
  const instancesSupported = useStore(instancesNegotiated);
  // Whether the server records a project's open state. Without it the
  // project is shown from what runs, as before.
  const openStateSupported = useStore(openStateNegotiated);
  const lifecycleInventoriesByMachine = useStore((s) =>
    s.sessionId ? s.cliLifecycleInventories[s.sessionId] : undefined,
  );
  const lifecycleInventory = useStore((s) => selectCliLifecycleInventory(s, s.sessionId));
  const lifecycleStatus = useStore((s) => {
    if (!s.sessionId) return undefined;
    const requestId = s.cliLifecycleLatestBySession[s.sessionId];
    return requestId ? s.cliLifecycleOperations[requestId] : undefined;
  });
  const rebootPane = useStore((s) => s.rebootPane);
  const requestPaneDiff = useStore((s) => s.requestPaneDiff);
  const createPanePr = useStore((s) => s.createPanePr);
  const paneDiffs = useStore((s) => s.paneDiffs);
  const updatePaneRole = useStore((s) => s.updatePaneRole);
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
  const [inputDrafts, setInputDrafts] = useState<Record<string, string>>({});
  const [addTabError, setAddTabError] = useState<string | null>(null);
  const addTabErrorTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Shown in the tab bar's notice slot: a new tab that could not be sent, or
  // a tab control refused before it was sent.
  const showTabError = useCallback((message: string) => {
    setAddTabError(message);
    if (addTabErrorTimer.current) clearTimeout(addTabErrorTimer.current);
    addTabErrorTimer.current = setTimeout(() => {
      addTabErrorTimer.current = null;
      setAddTabError(null);
    }, 4000);
  }, []);
  useEffect(() => () => {
    if (addTabErrorTimer.current) clearTimeout(addTabErrorTimer.current);
  }, []);
  // 3-option cleanup dialog shown when closing a pane that owns a worktree.
  // null = closed. paneId/worktreePath populated when open.
  const [cleanupDialog, setCleanupDialog] = useState<
    { paneId: number; worktreePath: string; note?: string } | null
  >(null);
  // Diff viewer modal (Phase 1.2a). When set, shows paneDiffs[paneId].
  const [diffModalPaneId, setDiffModalPaneId] = useState<number | null>(null);
  // Role drawer modal (Phase 2.1c). When set, edits role/goal/backstory.
  const [roleModalPaneId, setRoleModalPaneId] = useState<number | null>(null);
  const [summaryDrawerOpen, setSummaryDrawerOpen] = useState(false);
  const isDesktopViewport = useDesktopViewport();
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
    // An authoritative roster is the answer even when it is empty: a project
    // with no panes is a real state, and synthesizing here would rebuild the
    // pane the user just closed out of its leftover messages — as the only
    // tab, one whose close button used to be hidden, so it could not be
    // closed again.
    if (paneListReceived) return paneConfigs;
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
    paneListReceived,
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

  const currentSession = useMemo(
    () => sessions.find((item) => item.id === sessionId),
    [sessionId, sessions],
  );

  // The machine the project view's Open and Close address. Both act on every
  // machine with the project's tabs; this one is the fallback.
  const bootTarget = useMemo(() => {
    if (!sessionId) return null;
    return resolveMachineProjectTarget(currentSession, machines);
  }, [currentSession, machines, sessionId]);

  // The project's open state as the server reports it: the state to show,
  // the Open and Close to offer with their confirmations, and the machines
  // that are not following. From a server that records no state it is
  // derived from what runs.
  const projectOpenReport = useStore((s) => selectProjectOpenReport(s, [currentSession?.projectId, sessionId]));
  const projectName = currentSession ? projectNameFor(currentSession) : undefined;
  const projectOpenModel = useMemo(() => buildProjectOpenModel({
    supported: openStateSupported,
    projectIds: [currentSession?.projectId, sessionId],
    projectName,
    report: projectOpenReport,
    session: currentSession ?? null,
    // Only a roster the server sent says which machines own tabs, and only
    // a server that reports instances says which of them run the project.
    panes: paneListReceived && instancesSupported ? paneConfigs : undefined,
    instances: instancesSupported ? instances : undefined,
    machines,
    target: bootTarget,
    isOwner: sessionIsOwned(currentSession),
  }), [
    bootTarget,
    currentSession,
    instances,
    instancesSupported,
    machines,
    openStateSupported,
    paneConfigs,
    paneListReceived,
    projectName,
    projectOpenReport,
    sessionId,
  ]);

  // Where each tab runs, and why a tab cannot be used: its machine is not
  // running the project, or this viewer may not use that machine. Empty for an
  // older server and for a single-machine project with nothing attached, so
  // the tab bar is then what it always was. The project's open state words
  // the reason: a closed project is said to be closed, and a machine with a
  // known reason for not running it is given that reason.
  const tabHosts = useMemo(() => describeTabHosts({
    panes: effectiveTabs,
    instances,
    machines,
    instancesSupported,
    projectIds: [currentSession?.projectId, sessionId].filter((id): id is string => Boolean(id)),
    isOwner: sessionIsOwned(currentSession),
    project: projectOpenModel,
  }), [currentSession, effectiveTabs, instances, instancesSupported, machines, projectOpenModel, sessionId]);

  const selectedTabConfig = activeTabId != null
    ? effectiveTabs.find((tab) => tab.pane_id === activeTabId)
    : undefined;
  // The new-tab menu's machine step; a new tab defaults to the selected tab's
  // machine. A tab on a machine that is not running the project opens the
  // project, and the step names the other machines that start with it.
  const machineChoices = useMemo(() => buildMachineChoices({
    projectId: currentSession?.projectId ?? sessionId ?? "",
    instances,
    machines,
    instancesSupported,
    canClone: sessionCanCloneJoin(currentSession),
    selectedPane: selectedTabConfig ?? null,
    fallbackMachineId: bootTarget?.machineId,
    openStateSupported,
    projectMachines: projectOpenModel.machines,
    projectOpen: projectOpenModel.supported && projectOpenModel.state === "open",
  }), [
    bootTarget?.machineId,
    currentSession,
    instances,
    instancesSupported,
    machines,
    openStateSupported,
    projectOpenModel.machines,
    projectOpenModel.supported,
    projectOpenModel.state,
    selectedTabConfig,
    sessionId,
  ]);

  const rebootTargets = useMemo(
    () => instanceRebootTargets(instances, lifecycleInventoriesByMachine),
    [instances, lifecycleInventoriesByMachine],
  );

  const launches = useMemo(
    () => Object.values(addPaneRequests)
      .filter((request) => request.sessionId === sessionId)
      .sort((a, b) => a.startedAt - b.startedAt),
    [addPaneRequests, sessionId],
  );

  // The tabs with an autopilot to show: on, paused, or ended with a reason.
  const autopilotStatuses = useSessionAutopilotStatuses(sessionId, effectiveTabs);
  const autopilotRequestList = useMemo(
    () => autopilotRequestsOfSession(autopilotRequests, sessionId),
    [autopilotRequests, sessionId],
  );
  // A tab's name as the tab bar shows it.
  const tabDisplayLabel = useCallback((paneId: number): string | undefined => {
    const index = effectiveTabs.findIndex((tab) => tab.pane_id === paneId);
    return index < 0 ? undefined : effectiveTabs[index].label || `Tab ${index + 1}`;
  }, [effectiveTabs]);
  // Closing the project stops every agent in it; the confirmation names the
  // tabs that are on autopilot and says what becomes of it.
  const closeProjectNote = useMemo(
    () => closeProjectAutopilotNote(effectiveTabs
      .filter((tab) => autopilotStatuses.get(tab.pane_id)?.kind === "on")
      .map((tab) => tabDisplayLabel(tab.pane_id) ?? `Tab ${tab.pane_id}`)),
    [autopilotStatuses, effectiveTabs, tabDisplayLabel],
  );

  const activeTabHost = activeTabId != null ? tabHosts.tabs.get(activeTabId) : undefined;
  const activeRefusal = activeTabHost?.refusal;

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

  // An entry point that named an agent rather than a project wins over the
  // remembered tab: the person asked for that pane, and the preference is keyed
  // by CLI client, which the caller could not know before attaching.
  useEffect(() => {
    if (!pendingPaneSelection || pendingPaneSelection.sessionId !== sessionId) return;
    const wanted = pendingPaneSelection.paneId;
    if (!paneConfigs.some((pane) => pane.pane_id === wanted)) return;
    setActiveTabId(wanted);
    clearPendingPaneSelection();
  }, [clearPendingPaneSelection, paneConfigs, pendingPaneSelection, sessionId]);

  // Derive activeTabId from cliClientId + available panes + persisted
  // preference. Writes to localStorage only happen through
  // handleSelectTab (i.e. explicit user click) — this effect never
  // persists the auto-derived fallback, so projects retain whichever
  // tab the user last actually chose.
  useEffect(() => {
    const ids = tabIds.split(",").filter(Boolean).map(Number);
    const clientChanged = lastDerivedForRef.current !== cliClientId;
    lastDerivedForRef.current = cliClientId;

    const saved = getProjectLayout(cliClientId, "active_tab", "");
    const nextActiveTabId = deriveInitialActiveTabId({
      activeTabId,
      clientChanged,
      managerTabId,
      paneConfigsLength: paneConfigs.length,
      paneListReceived,
      savedActiveTab: saved,
      tabIds: ids,
    });
    if (activeTabId !== nextActiveTabId) {
      setActiveTabId(nextActiveTabId);
    }
  }, [activeTabId, cliClientId, managerTabId, paneConfigs.length, paneListReceived, tabIds]);

  // Lazy-load: when activeTabId changes (initial pick or user click),
  // fetch that pane's messages if we haven't already. Server's attach
  // reply no longer ships every pane's tail, so each pane opens
  // on-demand. Project navigation does not change the selected pane.
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
    if (pid == null) return;
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
  // via CSS) so re-activating them is instant.
  useEffect(() => {
    if (activeTabId == null) return;
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

  /**
   * Move the selection off a pane being closed.
   *
   * Closing the last pane leaves nothing to select, and leaving the selection
   * on the dead pane is not harmless: it is what fed the pane id back into tab
   * synthesis and pinned the active-tab derivation to a pane that no longer
   * exists. Clear the persisted choice too, or a reload restores it.
   */
  const selectAfterClosing = useCallback(
    (paneId: number) => {
      if (paneId !== activeTabId) return;
      const remaining = effectiveTabs.filter((t) => t.pane_id !== paneId);
      if (remaining.length > 0) {
        handleSelectTab(remaining[0].pane_id);
        return;
      }
      setActiveTabId(null);
      setProjectLayout(cliClientId, "active_tab", "");
    },
    [activeTabId, cliClientId, effectiveTabs, handleSelectTab],
  );

  const handleCloseTab = useCallback(
    (paneId: number) => {
      // Refused before asking anything: the machine that holds this tab is
      // not running the project, so a confirmed close could only fail.
      const refusal = tabHosts.tabs.get(paneId)?.refusal;
      if (refusal) {
        showTabError(refusal);
        return;
      }
      const pane = effectiveTabs.find((t) => t.pane_id === paneId);
      const worktreePath = pane?.worktree_path;
      // Closing a tab ends its agent, and its autopilot with it.
      const note = closeTabAutopilotNote(autopilotStatuses.get(paneId) ?? null);
      if (worktreePath) {
        // Pane owns an isolated worktree — open the 3-option dialog.
        setCleanupDialog({ paneId, worktreePath, note });
        return;
      }
      if (!confirm(note ? `Close this tab?\n\n${note}` : "Close this tab?")) return;
      removePane(paneId);
      selectAfterClosing(paneId);
    },
    // `effectiveTabs` is read above to find the pane's worktree; leaving it out
    // would let a stale closure miss one and skip the cleanup dialog.
    [autopilotStatuses, effectiveTabs, removePane, selectAfterClosing, showTabError, tabHosts],
  );

  const handleConfirmCleanup = useCallback(
    (action: PaneCleanupAction) => {
      if (!cleanupDialog) return;
      const { paneId } = cleanupDialog;
      removePane(paneId, action);
      setCleanupDialog(null);
      selectAfterClosing(paneId);
    },
    [cleanupDialog, removePane, selectAfterClosing],
  );

  const handleAddTab = useCallback((
    provider: string = "claude",
    model?: string,
    isolatedWorktree?: boolean,
    kind: PaneKind = "terminal",
    machine?: MachineChoice,
  ) => {
    const isDeepseek = provider === "deepseek" || (provider === "claude" && isDeepseekModel(model));
    const deepseekModel = canonicalDeepseekModel(model);
    const basePrefix = provider === "codex"
      ? "Codex"
      : provider === "cursor-agent"
        ? "Cursor"
        : provider === "opencode"
          ? "OpenCode"
          : provider === "antigravity" ? "Antigravity"
          : provider === "omp"
            ? "OMP"
            : isDeepseek || deepseekModel === DEEPSEEK_FLASH_MODEL
              ? "DeepSeek"
              : "Claude";
    const label = `${basePrefix} ${effectiveTabs.length + 1}`;
    // The machine step's pick, else the default (the selected tab's machine).
    // A hidden step's default is a running instance, and is still sent:
    // naming it keeps the tab off any other machine. A default that would
    // start the project is never hidden (`buildMachineChoices`).
    const target = machine ?? machineChoices.defaultChoice;
    let result: { success: boolean; error?: string };
    if (target?.kind === "clone") {
      const gitRemote = currentSession?.gitRemote;
      if (!target.machineId || !gitRemote) {
        result = { success: false, error: "This project has no repository to clone." };
      } else if (!confirm(confirmationText(cloneHereCopy(target.hostname, gitRemote, target.alsoStarts, target.projectOpen)))) {
        return;
      } else {
        result = cloneAndAddPane(
          {
            machineId: target.machineId,
            gitRemote,
            instanceName: cloneInstanceName(gitRemote),
            cloneUrl: currentSession?.gitRemoteUrl,
          },
          { provider, mode: "interactive", label, model, isolatedWorktree, managed: false, kind },
        );
      }
    } else if (!target && machineChoices.showChoice) {
      result = { success: false, error: "Choose where the new tab runs." };
    } else {
      result = addPane(
        provider, "interactive", label, undefined, model, isolatedWorktree, undefined, false, kind,
        target?.machineId,
      );
    }
    if (result.success) {
      setAddTabError(null);
    } else {
      showTabError(result.error || "Failed to create tab");
    }
  }, [addPane, cloneAndAddPane, currentSession, effectiveTabs.length, machineChoices, showTabError]);

  const handleForgetMachine = useCallback((machineId: string, hostname: string) => {
    if (!sessionId) return;
    if (!confirm(confirmationText(forgetMachineTabsCopy(hostname)))) return;
    forgetMachinePanes(sessionId, machineId);
  }, [forgetMachinePanes, sessionId]);

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
  const canShowSummary = Boolean(
    isDesktopViewport
      && connected
      && negotiatedCapabilities.has("pane_work_summary_v1")
      && activeConfig
      && activeTabId != null
      && activeTabId !== PANE_ID_MAIN,
  );

  useEffect(() => {
    if (!canShowSummary) setSummaryDrawerOpen(false);
  }, [canShowSummary]);
  const visibleProviderOptions = PROVIDER_OPTIONS.filter(
    (option) =>
      option.value === activeConfig?.provider
      || isLaunchProfileAllowed(
        activeConfig?.kind ?? "agent",
        option.value,
        undefined,
      ),
  );
  const activeHasMore = activeTabId === PANE_ID_MAIN ? hasMoreMessages : (activeTabId != null ? paneHasMore[paneKey(activeTabId)] || false : false);
  const activeIsLoading = activeTabId === PANE_ID_MAIN ? isLoadingMore : loadingMorePane === activeTabId;
  const activeStatus = activeTabId != null ? paneStatuses[paneKey(activeTabId)] || null : null;
  const activeIsAwaitingAnswer = paneIsAwaitingAnswerStatus(activeStatus);
  const activeIsBot = activeConfig?.mode === "deadloop";
  // Terminal panes take keystrokes directly in xterm.js. Showing the chat
  // composer under one would be worse than redundant: its text goes down
  // the agent input path, which a terminal pane has no channel for.
  const activeIsTerminal = activeConfig?.kind === "terminal";
  const { modeForPane: terminalViewModeForPane, setModeForPane: setTerminalViewModeForPane } =
    useTerminalViewModes(sessionId);
  const activeTerminalViewMode = terminalViewModeForPane(
    activeTabId ?? PANE_ID_MAIN,
  );
  const setActiveTerminalViewMode = useCallback(
    (mode: TerminalViewMode) => {
      if (activeTabId != null) setTerminalViewModeForPane(activeTabId, mode);
    },
    [activeTabId, setTerminalViewModeForPane],
  );
  const activeProvider = activeConfig?.provider;
  const activeIsUnsupported = isRetiredProviderModel(
    activeProvider,
    activeConfig?.model,
  );
  const activeIsDeepseek = activeProvider === "deepseek" || (
    activeProvider === "claude" && (
      isDeepseekModel(activeConfig?.model) ||
      (typeof activeConfig?.label === "string" && activeConfig.label.toLowerCase().includes("deepseek"))
    )
  );
  const activeUsageProvider: SupportedProvider | null = activeIsUnsupported
    || !activeProvider
    || activeProvider === "minimax"
    || activeProvider === "glm"
    || activeProvider === "pi"
    ? null
    : activeIsDeepseek
      ? "deepseek"
      : activeProvider;

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
    if (activeUsageProvider === "deepseek") return "DeepSeek Usage";
    return "Claude Usage";
  }, [activeUsageProvider]);

  // Open and Close act on the project as a whole. Each request is sent
  // once; what happened is what the server reports afterwards, through the
  // model above. A request that could not be sent says so and is not kept
  // for later.
  const [projectRequestNotSent, setProjectRequestNotSent] = useState<string | null>(null);
  const openCloseRequest = projectOpenModel.request;
  const handleOpenProject = useCallback(() => {
    if (!openCloseRequest) return;
    const sent = openProject(openCloseRequest.machineId, openCloseRequest.projectId);
    setProjectRequestNotSent(sent ? null : sessionId);
  }, [openCloseRequest, openProject, sessionId]);

  const handleCloseProject = useCallback(() => {
    if (!openCloseRequest) return;
    const sent = closeProject(openCloseRequest.machineId, openCloseRequest.projectId);
    setProjectRequestNotSent(sent ? null : sessionId);
  }, [closeProject, openCloseRequest, sessionId]);

  // "Delete project…" is the owner's, and a control of its own: nothing
  // else opens this dialog, closing the project included.
  const deletableProjectId = projectOpenModel.canDelete
    ? currentSession?.projectId ?? sessionId ?? null
    : null;
  const [deleteDialogProjectId, setDeleteDialogProjectId] = useState<string | null>(null);
  const deleteDialogOpen = deletableProjectId != null && deleteDialogProjectId === deletableProjectId;

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
      if (activeRefusal) return { success: false, error: activeRefusal };
      if (activeIsUnsupported) {
        return {
          success: false,
          error: "This pane uses a retired provider and cannot be relaunched or receive input.",
        };
      }
      if (activeIsBot) {
        return {
          success: false,
          error: "Legacy bot history is read-only. Create an autonomous worker with fresh approval.",
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
    [activeIsBot, activeIsUnsupported, activeRefusal, activeTabId, effectiveTabs, sendMessageToPane],
  );

  // No selected project/session — keep this distinct from a selected project
  // whose authoritative pane list is empty. The latter continues into the
  // normal workspace so Overview and the first-pane action remain available.
  if (!sessionId) {
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
          {departedProject ? (
            // The project on screen was deleted and its workspace was left.
            // Say so: the lines below read like an empty project that is
            // still there.
            <div role="status" className="text-center px-6">
              <p className="text-lg text-gray-600 dark:text-gray-300">
                {departedProject.name ? `${departedProject.name} was deleted.` : "This project was deleted."}
              </p>
              <p className="text-sm mt-1">Pick another project from the list.</p>
              <p className="text-xs mt-3 md:hidden">Tap ☰ at the top-left to pick a project.</p>
            </div>
          ) : (
            <div className="text-center px-6">
              <p className="text-lg">No messages yet</p>
              <p className="text-sm mt-1">Waiting for activity...</p>
              <p className="text-xs mt-3 md:hidden">Tap ☰ at the top-left to pick a project.</p>
            </div>
          )}
        </div>
      </div>
    );
  }

  const selectedSession = currentSession;
  const projectId = (selectedSession?.projectId !== selectedSession?.id ? selectedSession?.projectId : undefined)
    ?? bootTarget?.projectId ?? selectedSession?.projectId;
  const machineId = bootTarget?.projectId === projectId ? bootTarget?.machineId : undefined;
  const registration = machines.find((item) => item.machine.machineId === machineId)
    ?.projects.find((project) => project.projectId === projectId);
  const projectContext = projectId ? {
    id: projectId,
    name: registration?.name || selectedSession?.workingDir?.split("/").filter(Boolean).at(-1) || "Selected project",
    machineId,
  } : undefined;

  // The overview's pause and resume reach the pane's machine as well; say
  // why instead of sending what the server can only refuse.
  const paneControlAllowed = (paneId: number) => {
    const refusal = tabHosts.tabs.get(paneId)?.refusal;
    if (refusal) showTabError(refusal);
    return !refusal;
  };
  const stoppedActions = activeTabHost?.stopped;
  // The selected tab's machine is not running the project, or this viewer
  // may not use it: its controls stay visible, disabled, with the reason.
  const paneControlsDisabled = Boolean(activeRefusal);
  // The selected tab, when autopilot applies to it: an interactive OMP
  // terminal tab, on a server that has autopilot.
  const activeAutopilotTab = autopilotNegotiated
    && activeConfig
    && activeTabId != null
    && !activeIsUnsupported
    && paneCanHaveAutopilot(activeConfig)
    ? activeConfig
    : undefined;
  // The selected tab, when it offers the loop prompt instead: a Codex, Claude
  // (or DeepSeek) or OpenCode terminal tab. It needs no server capability:
  // it is an ordinary message.
  const activeLoopPromptTab = activeConfig
    && activeTabId != null
    && paneCanHaveLoopPrompt(activeConfig)
    ? activeConfig
    : undefined;
  // Merging or discarding a tab's branch closes the tab too.
  const withAutopilotNote = (question: string, paneId: number) => {
    const note = closeTabAutopilotNote(autopilotStatuses.get(paneId) ?? null);
    return note ? `${question}\n\n${note}` : question;
  };

  return (
    <ProjectWorkersWorkspace
      key={`${accountId}:${projectId ?? sessionId}`}
      project={projectContext}
      paneSelectionRequested={pendingPaneSelection?.sessionId === sessionId}
      initialOverview={getProjectLayout(cliClientId, "active_tab", "") === "-1"}
      renderOverviewPanels={(selectInteractive) => (
        <OverviewView
          onOpenPane={(paneId) => { selectInteractive(); handleSelectTab(paneId); }}
          onOpenDiff={(paneId) => {
            setDiffModalPaneId(paneId);
            requestPaneDiff(paneId);
          }}
          onOpenRole={setRoleModalPaneId}
          onRemovePane={handleCloseTab}
          tabHosts={tabHosts.tabs}
          // Only the session list saying so makes a project "not running";
          // before it arrives nothing is claimed.
          projectRunning={currentSession?.isActive !== false}
        />
      )}
    >
    {(workspace) => (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Tab bar */}
      <TabBar
        tabs={effectiveTabs}
        activeTabId={workspace.active ? null : activeTabId}
        onSelectTab={(paneId) => { workspace.selectInteractive(); handleSelectTab(paneId); }}
        onCloseTab={handleCloseTab}
        onAddTab={(...args) => { workspace.selectInteractive(); handleAddTab(...args); }}
        jobTabs={workspace.tabs}
        overviewTab={workspace.overviewTab}
        onRenameTab={handleRenameTab}
        onReorderTabs={handleReorderTabs}
        projectControls={{
          openState: projectOpenModel.supported ? projectOpenModel.stateLabel : undefined,
          recordedBy: projectOpenModel.setBy,
          openAction: projectOpenModel.openAction,
          closeAction: projectOpenModel.closeAction,
          onOpen: handleOpenProject,
          onClose: handleCloseProject,
          closeNote: closeProjectNote,
          onDelete: deletableProjectId
            ? () => setDeleteDialogProjectId(deletableProjectId)
            : undefined,
        }}
        onRebootCli={rebootCli}
        lifecycleInventory={lifecycleInventory}
        lifecycleStatus={lifecycleStatus}
        rebootTargets={rebootTargets}
        showRebootButton={isAttached}
        tabHosts={tabHosts.tabs}
        onForgetMachine={handleForgetMachine}
        machineChoices={machineChoices}
        autopilotTabs={autopilotStatuses}
        paneStatuses={paneStatuses}
        pausedPanes={pausedPanes}
        leading={mobileLeading}
        trailing={mobileTrailing}
      />

      {/* An Open or Close that did not leave this browser. It is not sent
          later; the person asks again. */}
      {projectRequestNotSent === sessionId && (
        <div
          role="alert"
          className="flex flex-shrink-0 items-center gap-2 border-b border-red-200 bg-red-50 px-3 py-1.5 text-xs text-red-700 dark:border-red-800 dark:bg-red-900/30 dark:text-red-300"
        >
          <span className="min-w-0 flex-1">{NOTHING_SENT_TEXT}</span>
          <button
            type="button"
            onClick={() => setProjectRequestNotSent(null)}
            className="rounded px-1.5 py-0.5 font-medium hover:bg-red-100 dark:hover:bg-red-900/50"
          >
            Dismiss
          </button>
        </div>
      )}
      <ProjectOpenNotice lines={projectOpenModel.notice} />
      {deleteDialogOpen && deletableProjectId && (
        <DeleteProjectDialog
          project={{ id: deletableProjectId, name: projectName ?? "" }}
          stoppedFirstHosts={projectOpenModel.stoppedFirstHosts}
          offlineTabOwnerHosts={projectOpenModel.offlineTabOwnerHosts}
          onClose={() => setDeleteDialogProjectId(null)}
        />
      )}

      <AddPaneLaunches launches={launches} onDismiss={dismissAddPaneRequest} />
      <AutopilotRequestNotices
        requests={autopilotRequestList}
        tabLabel={tabDisplayLabel}
        onDismiss={dismissAutopilotRequest}
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

      {workspace.content}
      <div hidden={workspace.active} className={workspace.active ? "hidden" : "flex flex-1 min-h-0 flex-col"}>
      {/* Toolbar controls wrap on narrow phones instead of overflowing past
          the right edge of the pane frame. */}
      <div className="flex flex-wrap items-center gap-2 px-3 py-1.5 border-b border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/30 flex-shrink-0">
        {/* Interactive pane controls; legacy bot history cannot start work. */}
        {activeIsBot && activeTabId != null && <details className="w-full text-sm">
          <summary className="min-h-11 cursor-pointer">Earlier bot instructions</summary>
          <p>Original provider: {activeConfig?.provider}. This retained history is read-only. New autonomous workers are created and approved independently.</p>
          <p className="whitespace-pre-wrap break-words">{activeConfig?.prompt || "No saved bot prompt is available."}</p>
        </details>}
        {activeIsUnsupported ? (
          <div
            className="rounded border border-red-300 bg-red-50 px-2.5 py-1 text-xs font-medium text-red-700 dark:border-red-800 dark:bg-red-950/40 dark:text-red-300"
            role="status"
          >
            {isRetiredPiProvider(activeProvider)
              ? "Pi is no longer supported. History is read-only."
              : "Unsupported provider — history is read-only"}
          </div>
        ) : isAttached && activeTabId != null && activeTabId !== PANE_ID_MAIN && !activeIsBot && (
            <>
              {!activeIsTerminal && activeTabId != null && activeProvider && (
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
                  disabled={paneControlsDisabled}
                  className="rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 px-1.5 py-1 text-xs focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:cursor-not-allowed disabled:opacity-50"
                  title={activeRefusal ?? "Agent backend — switching kills the current agent child and respawns with a fresh session id"}
                >
                  {visibleProviderOptions.map((option) => (
                    <option
                      key={option.value}
                      value={option.value}
                      disabled={!isLaunchProfileAllowed(
                        activeConfig?.kind ?? "agent",
                        option.value,
                        undefined,
                      )}
                    >
                      {option.label}
                    </option>
                  ))}
                </select>
              )}
              {/* Not on a terminal tab: the CLI ignores it there, so the
                  button a person reaches for first on a working tab would do
                  nothing. A tab on autopilot has Stop now. */}
              {!activeIsTerminal && activeStatus && !activeIsAwaitingAnswer && activeTabId != null && (
                <button
                  onClick={() => {
                    if (activeTabId == null) return;
                    if (confirm(
                      "Interrupt this pane's current turn? The agent process will be sent SIGINT (and SIGKILL after 2s if it doesn't exit). Queued input will run next.",
                    )) {
                      interruptPane(activeTabId);
                    }
                  }}
                  disabled={paneControlsDisabled}
                  className="flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded transition-colors bg-red-500 hover:bg-red-600 text-white disabled:cursor-not-allowed disabled:opacity-50"
                  title={activeRefusal ?? "Interrupt the current agent run (SIGINT). Use when a turn is wedged on a hung tool call."}
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
                  disabled={paneControlsDisabled}
                  className="flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded transition-colors bg-emerald-600 hover:bg-emerald-700 text-white disabled:cursor-not-allowed disabled:opacity-50"
                  title={activeRefusal ?? `View git diff for branch (worktree at ${activeConfig.worktree_path})`}
                >
                  Diff
                </button>
              )}
              {!activeIsTerminal && activeTabId != null && activeConfig && (
                <button
                  onClick={() => setRoleModalPaneId(activeTabId)}
                  disabled={paneControlsDisabled}
                  className="flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded transition-colors bg-purple-600 hover:bg-purple-700 text-white disabled:cursor-not-allowed disabled:opacity-50"
                  title={activeRefusal ?? "Edit role / goal / backstory for this pane (composed into --append-system-prompt at next spawn)"}
                >
                  Role
                </button>
              )}
            </>
        )}

        {/* Before the view switch, so nothing stands between that switch and
            the provider usage after it. Only for an OMP terminal tab, and
            only on a server that has autopilot. */}
        {activeAutopilotTab && (
          <AutopilotControl
            // One control per tab: an open menu, a half-typed start or a send
            // error never carries over to the tab selected next.
            key={`${sessionId}:${activeAutopilotTab.pane_id}`}
            sessionId={sessionId}
            paneId={activeAutopilotTab.pane_id}
            tabLabel={tabDisplayLabel(activeAutopilotTab.pane_id) ?? `Tab ${activeAutopilotTab.pane_id}`}
            hostname={activeTabHost?.hostname
              ?? activeAutopilotTab.hostname
              ?? primaryInstance(instances)?.hostname
              ?? currentSession?.hostname}
            // A tab's own reason comes first. With nothing known per tab, a
            // project that is not running has no instance to ask either.
            tabRefusal={activeRefusal ?? (isAttached ? undefined : AUTOPILOT_PROJECT_NOT_RUNNING_TEXT)}
            sharedCheckout={!activeAutopilotTab.worktree_path}
          />
        )}
        {/* In the autopilot control's place on the tabs that cannot have
            autopilot. It only sends a message, so it shows no state. */}
        {activeLoopPromptTab && sessionId && (
          <LoopPromptControl
            key={`${sessionId}:${activeLoopPromptTab.pane_id}`}
            sessionId={sessionId}
            paneId={activeLoopPromptTab.pane_id}
            tabLabel={tabDisplayLabel(activeLoopPromptTab.pane_id) ?? `Tab ${activeLoopPromptTab.pane_id}`}
            hostname={activeTabHost?.hostname
              ?? activeLoopPromptTab.hostname
              ?? primaryInstance(instances)?.hostname
              ?? currentSession?.hostname}
            // The composer's conditions: a tab's own reason first, then a
            // project that is not running, then no connection.
            disabledReason={activeRefusal
              ?? (isAttached ? undefined : AUTOPILOT_PROJECT_NOT_RUNNING_TEXT)
              ?? (connected ? undefined : LOOP_PROMPT_OFFLINE_TEXT)}
          />
        )}

        {activeIsTerminal && activeProvider !== "antigravity" && !activeIsUnsupported && (
          <TerminalViewToggle
            mode={activeTerminalViewMode}
            onChange={setActiveTerminalViewMode}
            turnCount={activeMessages.length}
          />
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
        {canShowSummary && (
          <button
            type="button"
            onClick={() => setSummaryDrawerOpen((open) => !open)}
            className={`inline-flex items-center rounded px-2.5 py-1 text-xs font-medium transition-colors ${
              summaryDrawerOpen
                ? "bg-blue-700 text-white hover:bg-blue-800"
                : "border border-gray-300 bg-white text-gray-700 hover:bg-gray-100 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-200 dark:hover:bg-gray-700"
            }`}
            aria-expanded={summaryDrawerOpen}
            aria-controls="pane-work-summary-drawer"
          >
            Summary
          </button>
        )}
        {!activeIsBot && !activeIsUnsupported && shouldShowPaneRebootButton(activeTabId) && (
          <button
            onClick={() => {
              requestConfirmedPaneReboot(
                activeTabId,
                (message) => typeof window === "undefined" || window.confirm(message),
                rebootPane,
                rebootAgentAutopilotNote(
                  (activeTabId != null ? autopilotStatuses.get(activeTabId) : undefined) ?? null,
                ),
              );
            }}
            disabled={paneControlsDisabled}
            className="inline-flex items-center px-2.5 py-1 text-xs font-medium rounded transition-colors bg-rose-600 hover:bg-rose-700 text-white disabled:cursor-not-allowed disabled:opacity-50"
            title={activeRefusal ?? "Reboot just this pane's agent — kills the running process and respawns with --resume on the same session so prior context is preserved. Use when a pane is wedged and you don't want to recycle the entire CLI."}
          >
            Reboot
          </button>
        )}
      </div>

      {/* The selected tab belongs to a machine that is not running the
          project (or that this viewer may not use). Its history still reads;
          what it needs is that machine, so say which. That machine alone is
          not started from here: opening the project again starts it. */}
      {activeRefusal && (
        <div
          role="status"
          className="flex flex-shrink-0 flex-wrap items-center gap-2 border-b border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-900 dark:border-amber-900 dark:bg-amber-950/30 dark:text-amber-200"
        >
          <span className="min-w-0 flex-1 basis-56">{activeRefusal}</span>
          {stoppedActions?.canForget && (
            <button
              type="button"
              onClick={() => handleForgetMachine(stoppedActions.machineId, stoppedActions.hostname)}
              className="rounded border border-amber-300 px-2.5 py-1 text-xs font-medium hover:bg-amber-100 dark:border-amber-800 dark:hover:bg-amber-900/40"
            >
              Forget {stoppedActions.hostname}&apos;s tabs
            </button>
          )}
        </div>
      )}

      <div className="flex flex-1 min-h-0 min-w-0">
        <div className="flex flex-1 flex-col min-w-0 min-h-0">
      {/* Visited pane bodies stay mounted and visibility-toggled, including
          while the project overview or a job is selected. */}
      {paneListReceived && effectiveTabs.length === 0 ? (
        // The project really has no panes: either it is new, or the user
        // just closed the last one. Distinct from the case below, where no
        // roster has arrived and the legacy single-pane view is still the
        // right guess.
        //
        // Deliberately no create button here. The tab bar's "+" is already
        // rendered above with the project's launch policy applied, and a
        // second entry point would have to duplicate that filtering — the
        // same reason the zero-pane design rejected one for new projects.
        <div className="flex-1 flex items-center justify-center text-gray-400">
          <div className="text-center px-6">
            <p className="text-lg">No pane is open</p>
            <p className="text-sm mt-1">
              Create a pane with <span className="font-semibold">+</span> in the
              tab bar above to start work.
            </p>
          </div>
        </div>
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
          isActive={!workspace.active}
        />
      ) : (
        effectiveTabs.map((tab) => {
          const isActive = tab.pane_id === activeTabId && !workspace.active;
          const tabIsUnsupported = isRetiredProviderModel(tab.provider, tab.model);
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
              {tabIsUnsupported ? (
                <>
                  <div className="border-b border-red-200 bg-red-50 px-4 py-2 text-sm text-red-700 dark:border-red-900 dark:bg-red-950/30 dark:text-red-300">
                    {isRetiredPiProvider(tab.provider)
                      ? `${PI_RETIRED_MESSAGE} This tab's history is available, but it cannot receive input or be restarted.`
                      : "This historical pane uses a retired provider. Its history is available, but it cannot receive input or be restarted."}
                  </div>
                  <MessagePane
                    key={`${sessionId}-${tab.pane_id}-unsupported`}
                    paneId={tab.pane_id}
                    messages={msgs}
                    onLoadMore={() => loadMoreMessages(tab.pane_id)}
                    isLoading={loadingMorePane === tab.pane_id}
                    hasMore={paneHasMore[paneKey(tab.pane_id)] || false}
                    isActive={isActive}
                  />
                </>
              ) : tab.kind === "terminal" ? (
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
                  mode={tab.provider === "antigravity" ? "terminal" : terminalViewModeForPane(tab.pane_id)}
                  isActive={isActive}
                  onLoadMore={() => loadMoreMessages(tab.pane_id)}
                  isLoading={loadingMorePane === tab.pane_id}
                  hasMore={paneHasMore[paneKey(tab.pane_id)] || false}
                  disabledReason={tab.mode === "deadloop" ? "Legacy bot history is read-only." : tabHosts.tabs.get(tab.pane_id)?.refusal}
                />
              ) : (
                <MessagePane
                  key={`${sessionId}-${tab.pane_id}`}
                  paneId={tab.pane_id}
                  messages={msgs}
                  onLoadMore={() => loadMoreMessages(tab.pane_id)}
                  isLoading={loadingMorePane === tab.pane_id}
                  hasMore={paneHasMore[paneKey(tab.pane_id)] || false}
                  isActive={isActive}
                  disabledReason={tab.mode === "deadloop" ? "Legacy bot history is read-only." : tabHosts.tabs.get(tab.pane_id)?.refusal}
                />
              )}
            </div>
          );
        })
      )}

      {/* Status bar for the selected interactive pane. */}
      {activeStatus && (
        <div className={`flex-shrink-0 border-t px-3 py-2 ${activeIsAwaitingAnswer ? "border-amber-200 bg-amber-50 dark:border-amber-900 dark:bg-amber-950/30" : "border-gray-200 bg-blue-50 dark:border-gray-700 dark:bg-blue-900/20"}`}>
          <div className={`flex items-center gap-2 text-sm ${activeIsAwaitingAnswer ? "text-amber-800 dark:text-amber-200" : "text-blue-700 dark:text-blue-300"}`}>
            <div className={activeIsAwaitingAnswer ? "" : "animate-pulse"}>●</div>
            <span>{activeStatus}</span>
          </div>
        </div>
      )}

      {/* Autopilot is a second attribute: its line stands beside the status
          above, never in its place. OMP's own modes are shown here too. */}
      {activeAutopilotTab && (
        <AutopilotStatusLine sessionId={sessionId} paneId={activeAutopilotTab.pane_id} />
      )}

      {/* Input box — disabled for running deadloop panes and hidden on
          terminal panes (which take input in xterm). */}
      {!activeIsTerminal && !activeIsUnsupported && (
        <div className="p-3 border-t border-gray-200 dark:border-gray-700 flex-shrink-0">
          {activeIsBot ? (
            <div className="text-center text-sm text-gray-400 dark:text-gray-500 py-2">
              Legacy bot history is read-only. To set up recurring work, use New autonomous worker on the Overview tab.
            </div>
          ) : (
            <InteractiveInput
              draft={activeInputDraft}
              onDraftChange={handleInputDraftChange}
              onSend={handleSend}
              disabledReason={activeRefusal}
            />
          )}
        </div>
      )}
        </div>
        {summaryDrawerOpen && canShowSummary && sessionId && activeTabId != null && activeConfig && (
          <PaneWorkSummaryDrawer
            sessionId={sessionId}
            paneId={activeTabId}
            paneLabel={activeConfig.label || `Pane ${activeTabId}`}
            onClose={() => setSummaryDrawerOpen(false)}
          />
        )}
      </div>
      </div>

      <WorktreeCleanupModal
        open={cleanupDialog !== null}
        worktreePath={cleanupDialog?.worktreePath || ""}
        note={cleanupDialog?.note}
        onCancel={() => setCleanupDialog(null)}
        onConfirm={handleConfirmCleanup}
      />

      <PaneDiffModal
        open={diffModalPaneId !== null}
        diff={diffModalPaneId !== null ? paneDiffs[diffModalPaneId] : undefined}
        onClose={() => setDiffModalPaneId(null)}
        onRefresh={() => {
          if (diffModalPaneId !== null) requestPaneDiff(diffModalPaneId);
        }}
        onMerge={() => {
          if (diffModalPaneId === null) return;
          if (!confirm(withAutopilotNote("Merge this pane's branch into the project's HEAD, then close the pane? Aborts on conflict — resolve manually with git if it fails.", diffModalPaneId))) return;
          removePane(diffModalPaneId, "merge_and_remove");
          setDiffModalPaneId(null);
        }}
        onDiscard={() => {
          if (diffModalPaneId === null) return;
          if (!confirm(withAutopilotNote("Discard this branch AND its worktree, then close the pane? This is destructive — uncommitted changes are lost.", diffModalPaneId))) return;
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

      {!workspace.active && planReviewPending.length > 0 && (
        <div className="fixed inset-x-0 bottom-0 z-40 flex justify-center p-3 pointer-events-none">
          <div className="pointer-events-auto flex w-full max-w-3xl flex-col gap-2">
            {planReviewPending.map((item) => {
              // A held tool on a machine that is not running the project (or
              // that this viewer may not use) can only be refused; the
              // optimistic answer would also drop the review from view.
              const reviewRefusal = effectiveTabs.find((pane) => pane.pane_id === item.paneId)?.mode === "deadloop" ? "Legacy bot history is read-only." : tabHosts.tabs.get(item.paneId)?.refusal;
              return (
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
                {reviewRefusal && (
                  <p role="status" className="mb-2 text-xs text-orange-200">{reviewRefusal}</p>
                )}
                <div className="flex justify-end gap-2">
                  <button
                    type="button"
                    disabled={Boolean(reviewRefusal)}
                    title={reviewRefusal}
                    onClick={() => answerPlanReview(item.toolUseId, false)}
                    className="rounded border border-red-700 bg-red-900/40 px-3 py-1 text-xs text-red-200 hover:bg-red-900/60 disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    Deny
                  </button>
                  <button
                    type="button"
                    disabled={Boolean(reviewRefusal)}
                    title={reviewRefusal}
                    onClick={() => answerPlanReview(item.toolUseId, true)}
                    className="rounded border border-emerald-700 bg-emerald-700 px-3 py-1 text-xs text-emerald-50 hover:bg-emerald-600 disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    Approve
                  </button>
                </div>
              </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
    )}
    </ProjectWorkersWorkspace>
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
            {/* Every pane with a worktree gets this, also one an older
                `.apas` marked managed: the team review that used to open
                its PR is gone. */}
            <button
              type="button"
              onClick={onCreatePr}
              className="rounded border border-sky-600 bg-sky-900/40 px-3 py-1 text-xs text-sky-200 hover:bg-sky-900/60"
              title="Push this branch to origin and open a GitHub PR via `gh pr create --fill`. Requires `gh` on the CLI host."
            >
              Create PR
            </button>
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
  /** What else closing does to this tab, such as ending its autopilot. */
  note?: string;
  onCancel: () => void;
  onConfirm: (action: PaneCleanupAction) => void;
}

function WorktreeCleanupModal({ open, worktreePath, note, onCancel, onConfirm }: WorktreeCleanupModalProps) {
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
        {note && (
          <p role="note" className="mb-4 rounded border border-amber-700/70 bg-amber-950/30 px-3 py-2 text-sm text-amber-100">
            {note}
          </p>
        )}
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
  /** Why the pane cannot take answers now: its machine is not running the
   *  project, or this viewer may not use it. Locks its questions and
   *  approvals with the reason; the history still reads. */
  disabledReason?: string;
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

export function MessagePane({ paneId, messages, onLoadMore, isLoading, hasMore, isActive, disabledReason }: MessagePaneProps) {
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
          <MessageComponent key={item.message.id} message={item.message} disabledReason={disabledReason} />
        ),
      )}
      <div ref={messagesEndRef} />
    </div>
  );
}

// --- MessageComponent ---

const MessageComponent = memo(function MessageComponent({
  message,
  disabledReason,
}: {
  message: Message;
  disabledReason?: string;
}) {
  switch (message.role) {
    case "user":
      return <UserMessage message={message} />;
    case "assistant":
      return <AssistantMessage message={message} disabledReason={disabledReason} />;
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
  /** Why the pane cannot take input now; the draft stays editable. */
  disabledReason?: string;
}

function InteractiveInput({ draft, onDraftChange, onSend, disabledReason }: InteractiveInputProps) {
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
          placeholder={disabledReason ?? "Type a message..."}
          title={disabledReason}
          className="flex-1 resize-none rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
          onKeyDown={handleKeyDown}
          onChange={(e) => handleInput(e.target.value)}
        />
        <button
          onClick={handleSubmit}
          disabled={Boolean(disabledReason)}
          title={disabledReason}
          className="px-4 py-2 bg-blue-500 hover:bg-blue-600 text-white rounded-lg text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50 disabled:hover:bg-blue-500"
        >
          Send
        </button>
      </div>
    </div>
  );
}
