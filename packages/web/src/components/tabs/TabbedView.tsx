"use client";

import { useRef, useCallback, useEffect, useState, useMemo, memo } from "react";
import { useStore, Message, PaneConfig, PANE_ID_DEADLOOP, PANE_ID_INTERACTIVE, paneKey } from "@/lib/store";
import { UserMessage } from "../chat/UserMessage";
import { AssistantMessage } from "../chat/AssistantMessage";
import { TabBar } from "./TabBar";
import { UsageLimitsDisplay } from "../UsageLimits";

// Sentinel pane_id for the single-pane fallback (no pane system)
const PANE_ID_MAIN = 0;
const DEFAULT_BOT_MIN_INTERVAL_MINUTES = 15;
const DEFAULT_BOT_LOOP_PROMPT = `Work on tasks defined in TODO.md. Do the following steps. Don't ask me for advice, just pick the best option you think that is honest, complete, and not corner-cutting:

1. Do a git pull to check if there are any remote updates. Pick the top high-priority undone task, choose its first leaf task. If there are no undone TODO items left, sleep a minute and exit.
2. Analyze the task, check if this can be done with not too many LOC (i.e., smaller than 500 lines code give or take). If not, try to analyze this task and break it down into several smaller tasks, expanding it in the TODO.md. The breakdown can be nested and hierarchical. Try to make each leaf task small enough (<500 lines LOC). You can document your analysis in the doc folder for future reference.
3. Try to execute the first leaf task. Make a plan for the task before execute. You can document key findings in either the TODO.md (a few sentences in the TODO item, or doc it in the docs folder for longer details and discussions.
4. Make sure to add comprehensive test for the task executed. Run the whole test suites to make sure no regression happens. If tests fail, fix them using the best, honest, complete approach, run test suites again to verify fixes work. Repeat this step until no tests fail.
5. Prepare for git commit, remove all temporary files, especially not to commit any binary files. For plan files, remove the implementation plan and keep the design rational and user manual and put it in the docs folder.
6. Git commit the changes. First do git pull --rebase, and fix conflicts if any. Then do git push.`;

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

// Listed lowest → highest. xhigh sits between high and max (Opus-only
// extra-deep tier); max is the highest level.
const CLAUDE_EFFORT_OPTIONS = [
  { value: "default", label: "Default" },
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "xhigh", label: "XHigh" },
  { value: "max", label: "Max" },
] as const;
type ClaudeEffortOption = (typeof CLAUDE_EFFORT_OPTIONS)[number]["value"];

function normalizeClaudeEffortOption(raw?: string | null): ClaudeEffortOption {
  if (typeof raw !== "string") return "default";
  const normalized = raw.trim().toLowerCase();
  if (
    normalized === "default" ||
    normalized === "low" ||
    normalized === "medium" ||
    normalized === "high" ||
    normalized === "max" ||
    normalized === "xhigh"
  ) {
    return normalized;
  }
  if (normalized === "x-high") return "xhigh";
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

export function TabbedView() {
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
  const addPane = useStore((s) => s.addPane);
  const removePane = useStore((s) => s.removePane);
  const updatePaneLabel = useStore((s) => s.updatePaneLabel);
  const updatePaneEffort = useStore((s) => s.updatePaneEffort);
  const interruptPane = useStore((s) => s.interruptPane);
  const reorderPanes = useStore((s) => s.reorderPanes);
  const startBot = useStore((s) => s.startBot);
  const stopBot = useStore((s) => s.stopBot);
  const startMachineProjectCli = useStore((s) => s.startMachineProjectCli);
  const rebootCli = useStore((s) => s.rebootCli);
  const downloadSession = useStore((s) => s.downloadSession);

  // Active tab state, persisted per project
  const [activeTabId, setActiveTabId] = useState<number | null>(null);
  const [startBotModalOpen, setStartBotModalOpen] = useState(false);
  const [viewBotPromptModalOpen, setViewBotPromptModalOpen] = useState(false);
  const [startBotPaneId, setStartBotPaneId] = useState<number | null>(null);
  const [botPromptDraft, setBotPromptDraft] = useState("");
  const [botMinIntervalDraft, setBotMinIntervalDraft] = useState(String(DEFAULT_BOT_MIN_INTERVAL_MINUTES));
  const [botEffortDraft, setBotEffortDraft] = useState<ClaudeEffortOption>("default");
  const [inputDrafts, setInputDrafts] = useState<Record<string, string>>({});
  const [addTabError, setAddTabError] = useState<string | null>(null);

  // Determine effective tabs: use paneConfigs from server, or synthesize from observed messages
  const effectiveTabs = useMemo(() => {
    const applyModeHints = (tabs: PaneConfig[]) =>
      tabs.map((tab) => {
        const hintedMode = paneModes[paneKey(tab.pane_id)];
        if (!hintedMode || hintedMode === tab.mode) return tab;
        return { ...tab, mode: hintedMode };
      });

    if (paneConfigs.length > 0) return applyModeHints(paneConfigs);
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

    // Same project, current pick is still valid → no change.
    if (!clientChanged && activeTabId != null && ids.includes(activeTabId)) {
      return;
    }
    // Same project, pane_list is synthesized (no authoritative panes yet)
    // → keep current selection to avoid jumps from transient data.
    if (!clientChanged && activeTabId != null && paneConfigs.length === 0) {
      return;
    }

    const saved = getProjectLayout(cliClientId, "active_tab", "");
    const savedNum = saved ? parseInt(saved, 10) : NaN;
    if (!isNaN(savedNum) && ids.includes(savedNum)) {
      if (activeTabId !== savedNum) setActiveTabId(savedNum);
      return;
    }
    // Fall through: persisted pref is gone or no longer matches a real
    // pane. Pick the first visible tab visually but DON'T persist —
    // we'd otherwise overwrite the user's intent on the next save.
    if (activeTabId !== ids[0]) setActiveTabId(ids[0]);
  }, [activeTabId, cliClientId, paneConfigs.length, tabIds]);

  const handleSelectTab = useCallback(
    (paneId: number) => {
      setActiveTabId(paneId);
      setProjectLayout(cliClientId, "active_tab", String(paneId));
    },
    [cliClientId],
  );

  const handleCloseTab = useCallback(
    (paneId: number) => {
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

  const handleAddTab = useCallback((provider: string = "claude", model?: string) => {
    const isMiniMax = provider === "minimax" || (provider === "claude" && isMiniMaxModel(model));
    const isGlm = provider === "glm" || (provider === "claude" && isGlmModel(model));
    const prefix = provider === "codex"
      ? "Codex"
      : provider === "cursor-agent"
        ? "Cursor"
        : provider === "opencode"
          ? "OpenCode"
          : isMiniMax ? "MiniMax" : isGlm ? "GLM" : "Claude";
    const label = `${prefix} ${effectiveTabs.length + 1}`;
    const result = addPane(provider, "interactive", label, undefined, model);
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
  const activeUsageProvider = activeIsMiniMax ? "minimax" : activeIsGlm ? "glm" : activeProvider;
  const activeSupportsClaudeEffort = activeUsageProvider === "claude";
  const activeBotEffortOption = normalizeClaudeEffortOption(activeConfig?.effort);
  const activeBotPrompt = activeConfig?.prompt && activeConfig.prompt.trim().length > 0
    ? activeConfig.prompt
    : DEFAULT_BOT_LOOP_PROMPT;
  const activeBotMinIntervalMinutes = typeof activeConfig?.min_iteration_interval_minutes === "number"
    ? activeConfig.min_iteration_interval_minutes
    : DEFAULT_BOT_MIN_INTERVAL_MINUTES;
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
        const { ws } = useStore.getState();
        if (!ws || ws.readyState !== WebSocket.OPEN) return { success: false, error: "Not connected" };
        ws.send(JSON.stringify({ type: "input", text }));
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
    const savedPrompt = activeConfig?.prompt;
    const savedMinInterval = typeof activeConfig?.min_iteration_interval_minutes === "number"
      ? activeConfig.min_iteration_interval_minutes
      : DEFAULT_BOT_MIN_INTERVAL_MINUTES;
    setBotPromptDraft(
      savedPrompt && savedPrompt.trim().length > 0 ? savedPrompt : DEFAULT_BOT_LOOP_PROMPT,
    );
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
      trimmed.length > 0 ? botPromptDraft : DEFAULT_BOT_LOOP_PROMPT,
      minIntervalMinutes,
      activeSupportsClaudeEffort ? botEffortDraft : undefined,
    );
    setStartBotModalOpen(false);
    setStartBotPaneId(null);
  }, [activeSupportsClaudeEffort, botEffortDraft, botMinIntervalDraft, botPromptDraft, startBot, startBotPaneId]);

  useEffect(() => {
    if (!activeIsBot && viewBotPromptModalOpen) {
      setViewBotPromptModalOpen(false);
    }
  }, [activeIsBot, viewBotPromptModalOpen]);

  // No session or no tabs - empty state
  if (!sessionId || effectiveTabs.length === 0) {
    return (
      <div className="flex-1 flex items-center justify-center text-gray-400">
        <div className="text-center">
          <p className="text-lg">No messages yet</p>
          <p className="text-sm mt-1">Waiting for activity...</p>
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

      {/* Toolbar */}
      <div className="flex items-center gap-2 px-3 py-1.5 border-b border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/30 flex-shrink-0">
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
                    const next = normalizeClaudeEffortOption(e.target.value);
                    setBotEffortDraft(next);
                    // Persist on the server (and CLI's .apas) so the choice
                    // survives tab-switch and CLI restart.
                    if (activeTabId != null) {
                      updatePaneEffort(activeTabId, next === "default" ? null : next);
                    }
                  }}
                  className="rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 px-1.5 py-1 text-xs focus:outline-none focus:ring-2 focus:ring-blue-500"
                  title="Claude thinking effort — persisted per tab"
                >
                  {CLAUDE_EFFORT_OPTIONS.map((option) => (
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
          onClick={downloadSession}
          className="hidden md:inline-flex items-center px-2.5 py-1 text-xs font-medium rounded transition-colors bg-blue-500 hover:bg-blue-600 text-white"
          title="Download session data"
        >
          Download
        </button>
      </div>

      {/* Message pane for active tab — keyed so each tab gets its own DOM/scroll state */}
      <MessagePane
        key={`${sessionId}-${activeTabId ?? PANE_ID_MAIN}`}
        paneId={activeTabId ?? PANE_ID_MAIN}
        messages={activeMessages}
        onLoadMore={handleLoadMore}
        isLoading={activeIsLoading}
        hasMore={activeHasMore}
      />

      {/* Status bar */}
      {activeStatus && (
        <div className="px-3 py-2 border-t border-gray-200 dark:border-gray-700 bg-blue-50 dark:bg-blue-900/20 flex-shrink-0">
          <div className="flex items-center gap-2 text-sm text-blue-700 dark:text-blue-300">
            <div className="animate-pulse">●</div>
            <span>{activeStatus}</span>
          </div>
        </div>
      )}

      {/* Input box - disabled for running deadloop panes */}
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
            Edit the loop prompt for this tab. This prompt is saved per tab in `.apas`.
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

// --- MessagePane ---

interface MessagePaneProps {
  paneId: number;
  messages: Message[];
  onLoadMore?: () => void;
  isLoading?: boolean;
  hasMore?: boolean;
}

function MessagePane({ paneId, messages, onLoadMore, isLoading, hasMore }: MessagePaneProps) {
  const sessionId = useStore((s) => s.sessionId);
  const containerRef = useRef<HTMLDivElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const shouldAutoScroll = useRef(true);
  const prevScrollHeight = useRef<number>(0);
  const isRestoringScroll = useRef(false);
  const scrollThrottleRef = useRef<number>(0);
  const hasRestoredRef = useRef(false);

  const scrollKey = getScrollKey(sessionId, paneId);

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

    if (checkIfNearTop() && onLoadMore && !isLoading && hasMore) {
      prevScrollHeight.current = containerRef.current?.scrollHeight || 0;
      onLoadMore();
    }
  }, [checkIfAtBottom, checkIfNearTop, onLoadMore, isLoading, hasMore, scrollKey]);

  // Save scroll position on unmount (component is keyed by paneId, so unmount = tab switch)
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

  // Restore scroll position once messages are available.
  // Waiting avoids restoring against an empty/partial DOM during session re-attach.
  useEffect(() => {
    if (!containerRef.current) return;
    if (hasRestoredRef.current) return;
    if (messages.length === 0) return;

    const savedState = scrollPositions.get(scrollKey);
    if (savedState) {
      isRestoringScroll.current = true;
      shouldAutoScroll.current = savedState.wasAtBottom;
      requestAnimationFrame(() => {
        const container = containerRef.current;
        if (container) {
          const maxScrollTop = Math.max(0, container.scrollHeight - container.clientHeight);
          if (savedState.wasAtBottom) {
            container.scrollTop = maxScrollTop;
          } else {
            const savedMaxScrollTop = Math.max(
              0,
              savedState.scrollHeight - savedState.clientHeight,
            );
            const nextScrollTop = savedMaxScrollTop > 0
              ? (savedState.scrollTop / savedMaxScrollTop) * maxScrollTop
              : savedState.scrollTop;
            container.scrollTop = Math.max(
              0,
              Math.min(maxScrollTop, nextScrollTop),
            );
          }
        }
        hasRestoredRef.current = true;
        isRestoringScroll.current = false;
      });
    } else {
      shouldAutoScroll.current = true;
      requestAnimationFrame(() => {
        messagesEndRef.current?.scrollIntoView();
      });
      hasRestoredRef.current = true;
    }
  }, [messages.length, scrollKey]);

  // Maintain scroll position when prepending messages
  useEffect(() => {
    if (prevScrollHeight.current > 0 && containerRef.current) {
      const newScrollHeight = containerRef.current.scrollHeight;
      const scrollDiff = newScrollHeight - prevScrollHeight.current;
      if (scrollDiff > 0) {
        containerRef.current.scrollTop = scrollDiff;
      }
      prevScrollHeight.current = 0;
    }
  }, [messages.length]);

  // Auto-scroll for new messages (only when count changes, not on every array ref change)
  useEffect(() => {
    if (shouldAutoScroll.current && !isRestoringScroll.current) {
      messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [messages.length]);

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
      onScroll={handleScroll}
      className="flex-1 overflow-y-auto overflow-x-hidden px-2 sm:px-4 py-4 space-y-3 min-h-0"
    >
      {isLoading && (
        <div className="text-center text-gray-400 text-sm py-2">Loading...</div>
      )}
      {messages.map((message) => (
        <MessageComponent key={message.id} message={message} />
      ))}
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
