"use client";

import { useRef, useCallback, useEffect, useState, useMemo } from "react";
import { useStore, Message, PaneConfig, PANE_ID_DEADLOOP, PANE_ID_INTERACTIVE, paneKey } from "@/lib/store";
import { UserMessage } from "../chat/UserMessage";
import { AssistantMessage } from "../chat/AssistantMessage";
import { TabBar } from "./TabBar";

// Sentinel pane_id for the single-pane fallback (no pane system)
const PANE_ID_MAIN = 0;

// Store scroll positions per session+pane combination
interface ScrollState {
  scrollTop: number;
  wasAtBottom: boolean;
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

// Synthesize PaneConfig entries from observed pane_id keys when no PaneList was received
function synthesizeConfigs(
  paneMessages: Record<string, Message[]>,
  pausedPanes: number[],
  sessionId: string | null,
): PaneConfig[] {
  const configs: PaneConfig[] = [];
  const keys = Object.keys(paneMessages).sort();
  for (const key of keys) {
    const numericId = parseInt(key, 10);
    if (isNaN(numericId)) continue;
    const isDeadloop = numericId === PANE_ID_DEADLOOP;
    configs.push({
      pane_id: numericId,
      provider: "claude",
      mode: isDeadloop ? "deadloop" : "interactive",
      session_id: sessionId || "",
      is_paused: pausedPanes.includes(numericId),
      label: isDeadloop ? "Deadloop" : "Interactive",
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
  const pausedPanes = useStore((s) => s.pausedPanes);
  const loadingMorePane = useStore((s) => s.loadingMorePane);
  const isAttached = useStore((s) => s.isAttached);
  const isDualPane = useStore((s) => s.isDualPane);
  const hasMoreMessages = useStore((s) => s.hasMoreMessages);
  const isLoadingMore = useStore((s) => s.isLoadingMore);
  const cliClientId = useStore((s) => s.cliClientId);

  const sendMessageToPane = useStore((s) => s.sendMessageToPane);
  const loadMoreMessages = useStore((s) => s.loadMoreMessages);
  const pausePane = useStore((s) => s.pausePane);
  const resumePane = useStore((s) => s.resumePane);
  const addPane = useStore((s) => s.addPane);
  const removePane = useStore((s) => s.removePane);
  const startBot = useStore((s) => s.startBot);
  const stopBot = useStore((s) => s.stopBot);
  const rebootCli = useStore((s) => s.rebootCli);
  const downloadSession = useStore((s) => s.downloadSession);
  // Legacy pause/resume for backward compat with CLI
  const pauseDeadloop = useStore((s) => s.pauseDeadloop);
  const resumeDeadloop = useStore((s) => s.resumeDeadloop);

  // Determine effective tabs: use paneConfigs from server, or synthesize from observed messages
  const effectiveTabs = useMemo(() => {
    if (paneConfigs.length > 0) return paneConfigs;
    if (isDualPane && Object.keys(paneMessages).length > 0) {
      return synthesizeConfigs(paneMessages, pausedPanes, sessionId);
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
  }, [paneConfigs, isDualPane, paneMessages, pausedPanes, sessionId, messages.length]);

  // Active tab state, persisted per project
  const [activeTabId, setActiveTabId] = useState<number | null>(null);
  const prevTabIdsRef = useRef<number[]>([]);

  // Stable list of tab IDs (avoids re-running effects on every message)
  const tabIds = useMemo(
    () => effectiveTabs.map((t) => t.pane_id).join(","),
    [effectiveTabs],
  );

  // Load saved active tab when project or available tabs change
  useEffect(() => {
    const ids = tabIds.split(",").filter(Boolean).map(Number);
    const saved = getProjectLayout(cliClientId, "active_tab", "");
    const savedNum = saved ? parseInt(saved, 10) : NaN;
    if (!isNaN(savedNum) && ids.includes(savedNum)) {
      setActiveTabId(savedNum);
    } else if (ids.length > 0) {
      setActiveTabId(ids[0]);
    }
  }, [cliClientId, tabIds]);

  // Auto-switch to newly created tabs
  useEffect(() => {
    const ids = tabIds.split(",").filter(Boolean).map(Number);
    const prevIds = prevTabIdsRef.current;
    if (prevIds.length === 0) {
      prevTabIdsRef.current = ids;
      return;
    }
    const added = ids.filter((id) => !prevIds.includes(id));
    if (added.length > 0) {
      const nextActive = added[added.length - 1];
      setActiveTabId(nextActive);
      setProjectLayout(cliClientId, "active_tab", String(nextActive));
    }
    prevTabIdsRef.current = ids;
  }, [cliClientId, tabIds]);

  // If active tab no longer exists, reset to first
  useEffect(() => {
    const ids = tabIds.split(",").filter(Boolean).map(Number);
    if (activeTabId != null && ids.length > 0 && !ids.includes(activeTabId)) {
      setActiveTabId(ids[0]);
    }
  }, [activeTabId, tabIds]);

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

  const handleAddTab = useCallback((provider: string = "claude") => {
    const prefix = provider === "codex" ? "Codex" : "Claude";
    const label = `${prefix} ${effectiveTabs.length + 1}`;
    addPane(provider, "interactive", label);
  }, [addPane, effectiveTabs.length]);

  // Get messages for active tab
  const activeMessages = useMemo(() => {
    if (activeTabId == null) return [];
    if (activeTabId === PANE_ID_MAIN) return messages;
    return paneMessages[paneKey(activeTabId)] || [];
  }, [activeTabId, messages, paneMessages]);

  const activeConfig = effectiveTabs.find((t) => t.pane_id === activeTabId);
  const activeHasMore = activeTabId === PANE_ID_MAIN ? hasMoreMessages : (activeTabId != null ? paneHasMore[paneKey(activeTabId)] || false : false);
  const activeIsLoading = activeTabId === PANE_ID_MAIN ? isLoadingMore : loadingMorePane === activeTabId;
  const activeStatus = activeTabId != null ? paneStatuses[paneKey(activeTabId)] || null : null;
  const activeIsPaused = activeTabId != null ? pausedPanes.includes(activeTabId) : false;
  const activeIsBot = activeConfig?.mode === "deadloop";

  const handleLoadMore = useCallback(() => {
    if (activeTabId == null) return;
    if (activeTabId === PANE_ID_MAIN) {
      loadMoreMessages();
    } else {
      // Map pane_id to legacy pane_type for loadMoreMessages
      const legacyType = activeTabId === PANE_ID_DEADLOOP ? "deadloop" : activeTabId === PANE_ID_INTERACTIVE ? "interactive" : undefined;
      loadMoreMessages(legacyType as "deadloop" | "interactive" | undefined);
    }
  }, [activeTabId, loadMoreMessages]);

  const handleSend = useCallback(
    (text: string) => {
      if (activeTabId == null) return { success: false, error: "No active tab" };
      if (activeTabId === PANE_ID_MAIN) {
        const { ws } = useStore.getState();
        if (!ws || ws.readyState !== WebSocket.OPEN) return { success: false, error: "Not connected" };
        ws.send(JSON.stringify({ type: "input", text }));
        return { success: true };
      }
      return sendMessageToPane(text, activeTabId);
    },
    [activeTabId, sendMessageToPane],
  );

  const handlePauseResume = useCallback(() => {
    if (activeTabId == null) return;
    if (activeIsPaused) {
      resumePane(activeTabId);
      if (activeTabId === PANE_ID_DEADLOOP) resumeDeadloop();
    } else {
      pausePane(activeTabId);
      if (activeTabId === PANE_ID_DEADLOOP) pauseDeadloop();
    }
  }, [activeTabId, activeIsPaused, pausePane, resumePane, pauseDeadloop, resumeDeadloop]);

  const handleStartBot = useCallback(() => {
    if (activeTabId == null) return;
    startBot(activeTabId);
  }, [activeTabId, startBot]);

  const handleStopBot = useCallback(() => {
    if (activeTabId == null) return;
    stopBot(activeTabId);
  }, [activeTabId, stopBot]);

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
        paneStatuses={paneStatuses}
        pausedPanes={pausedPanes}
      />

      {/* Toolbar */}
      <div className="flex items-center gap-2 px-3 py-1.5 border-b border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/30 flex-shrink-0">
        {/* Start/Stop Bot + Pause/Resume */}
        {isAttached && activeTabId != null && activeTabId !== PANE_ID_MAIN && (
          activeIsBot ? (
            <>
              <button
                onClick={handlePauseResume}
                className={`flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded transition-colors ${
                  activeIsPaused
                    ? "bg-green-500 hover:bg-green-600 text-white"
                    : "bg-amber-500 hover:bg-amber-600 text-white"
                }`}
              >
                {activeIsPaused ? (
                  <>
                    <svg className="w-3 h-3" fill="currentColor" viewBox="0 0 24 24"><path d="M8 5v14l11-7z" /></svg>
                    Resume Bot
                  </>
                ) : (
                  <>
                    <svg className="w-3 h-3" fill="currentColor" viewBox="0 0 24 24"><path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z" /></svg>
                    Pause Bot
                  </>
                )}
              </button>
              <button
                onClick={handleStopBot}
                className="flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded transition-colors bg-red-500 hover:bg-red-600 text-white"
              >
                <svg className="w-3 h-3" fill="currentColor" viewBox="0 0 24 24"><path d="M6 6h12v12H6z" /></svg>
                Stop Bot
              </button>
            </>
          ) : (
            <button
              onClick={handleStartBot}
              className="flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded transition-colors bg-green-500 hover:bg-green-600 text-white"
            >
              <svg className="w-3 h-3" fill="currentColor" viewBox="0 0 24 24"><path d="M8 5v14l11-7z" /></svg>
              Start Bot
            </button>
          )
        )}

        <div className="flex-1" />

        {/* Actions */}
        {isAttached && (
          <button
            onClick={() => {
              if (confirm("Are you sure you want to reboot the CLI?")) rebootCli();
            }}
            className="px-2.5 py-1 text-xs font-medium rounded transition-colors bg-red-500 hover:bg-red-600 text-white"
          >
            Reboot
          </button>
        )}
        <button
          onClick={downloadSession}
          className="px-2.5 py-1 text-xs font-medium rounded transition-colors bg-blue-500 hover:bg-blue-600 text-white"
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
        {activeIsBot && !activeIsPaused ? (
          <div className="text-center text-sm text-gray-400 dark:text-gray-500 py-2">
            Bot is running autonomously. Pause to send messages.
          </div>
        ) : (
          <InteractiveInput onSend={handleSend} />
        )}
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
    shouldAutoScroll.current = checkIfAtBottom();

    if (containerRef.current) {
      scrollPositions.set(scrollKey, {
        scrollTop: containerRef.current.scrollTop,
        wasAtBottom: shouldAutoScroll.current,
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
      if (containerRef.current) {
        scrollPositions.set(scrollKey, {
          scrollTop: containerRef.current.scrollTop,
          wasAtBottom: shouldAutoScroll.current,
        });
      }
    };
  }, [scrollKey]);

  // Restore scroll position on mount
  useEffect(() => {
    if (!containerRef.current) return;

    const savedState = scrollPositions.get(scrollKey);
    if (savedState) {
      isRestoringScroll.current = true;
      shouldAutoScroll.current = savedState.wasAtBottom;
      requestAnimationFrame(() => {
        if (containerRef.current) {
          if (savedState.wasAtBottom) {
            containerRef.current.scrollTop = containerRef.current.scrollHeight;
          } else {
            containerRef.current.scrollTop = savedState.scrollTop;
          }
        }
        isRestoringScroll.current = false;
      });
    } else {
      shouldAutoScroll.current = true;
      requestAnimationFrame(() => {
        messagesEndRef.current?.scrollIntoView();
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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

  // Auto-scroll for new messages
  useEffect(() => {
    if (shouldAutoScroll.current && !isRestoringScroll.current) {
      messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [messages]);

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

function MessageComponent({ message }: { message: Message }) {
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
}

// --- InteractiveInput ---

interface InteractiveInputProps {
  onSend: (text: string) => { success: boolean; error?: string };
}

function InteractiveInput({ onSend }: InteractiveInputProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = () => {
    const text = textareaRef.current?.value.trim();
    if (text) {
      const result = onSend(text);
      if (result.success) {
        if (textareaRef.current) {
          textareaRef.current.value = "";
          textareaRef.current.style.height = "auto";
        }
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

  const handleInput = () => {
    const textarea = textareaRef.current;
    if (textarea) {
      textarea.style.height = "auto";
      textarea.style.height = Math.min(textarea.scrollHeight, 150) + "px";
    }
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
          rows={1}
          placeholder="Type a message..."
          className="flex-1 resize-none rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
          onKeyDown={handleKeyDown}
          onInput={handleInput}
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
