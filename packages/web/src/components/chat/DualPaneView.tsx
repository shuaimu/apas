"use client";

import { useRef, useCallback, useEffect, useState } from "react";
import { useStore, Message, PaneType } from "@/lib/store";
import { UserMessage } from "./UserMessage";
import { AssistantMessage } from "./AssistantMessage";
import { InputBox } from "./InputBox";
import { ResizeHandle } from "../ResizeHandle";

const MIN_PANE_PERCENT = 20;
const MAX_PANE_PERCENT = 80;
const DEFAULT_DEADLOOP_PERCENT = 50;

// Store scroll positions per session+pane combination
interface ScrollState {
  scrollTop: number;
  wasAtBottom: boolean;
}
const scrollPositions = new Map<string, ScrollState>();

function getScrollKey(sessionId: string | null, paneType: PaneType): string {
  return `${sessionId || 'none'}-${paneType}`;
}

export function DualPaneView() {
  const deadloopMessages = useStore((state) => state.deadloopMessages);
  const interactiveMessages = useStore((state) => state.interactiveMessages);
  const sendMessageToPane = useStore((state) => state.sendMessageToPane);
  const loadMoreMessages = useStore((state) => state.loadMoreMessages);
  const isLoadingMore = useStore((state) => state.isLoadingMore);
  const hasMoreMessages = useStore((state) => state.hasMoreMessages);
  const isDeadloopPaused = useStore((state) => state.isDeadloopPaused);
  const pauseDeadloop = useStore((state) => state.pauseDeadloop);
  const resumeDeadloop = useStore((state) => state.resumeDeadloop);
  const rebootCli = useStore((state) => state.rebootCli);
  const downloadSession = useStore((state) => state.downloadSession);
  const isAttached = useStore((state) => state.isAttached);
  const interactiveStatus = useStore((state) => state.interactiveStatus);
  const deadloopStatus = useStore((state) => state.deadloopStatus);

  // Initialize activePane from localStorage to persist across page refreshes
  const [activePane, setActivePane] = useState<PaneType>(() => {
    if (typeof window !== 'undefined') {
      const saved = localStorage.getItem("apas_active_pane");
      if (saved === "interactive" || saved === "deadloop") {
        return saved;
      }
    }
    return "deadloop";
  });

  // Save activePane to localStorage when it changes
  const handleSetActivePane = (pane: PaneType) => {
    setActivePane(pane);
    localStorage.setItem("apas_active_pane", pane);
  };

  // Deadloop pane width percentage (desktop only)
  const [deadloopPercent, setDeadloopPercent] = useState(() => {
    if (typeof window !== 'undefined') {
      const saved = localStorage.getItem("apas_deadloop_percent");
      if (saved) {
        const percent = parseFloat(saved);
        if (!isNaN(percent) && percent >= MIN_PANE_PERCENT && percent <= MAX_PANE_PERCENT) {
          return percent;
        }
      }
    }
    return DEFAULT_DEADLOOP_PERCENT;
  });

  const containerRef = useRef<HTMLDivElement>(null);

  const handlePaneResize = useCallback((delta: number) => {
    if (!containerRef.current) return;
    const containerWidth = containerRef.current.offsetWidth;
    const deltaPercent = (delta / containerWidth) * 100;
    setDeadloopPercent(prev => {
      const newPercent = Math.min(MAX_PANE_PERCENT, Math.max(MIN_PANE_PERCENT, prev + deltaPercent));
      return newPercent;
    });
  }, []);

  const handlePaneResizeEnd = useCallback(() => {
    localStorage.setItem("apas_deadloop_percent", deadloopPercent.toString());
  }, [deadloopPercent]);

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Mobile tab switcher - only visible on small screens */}
      <div className="mobile-tabs flex md:hidden border-b border-gray-200 dark:border-gray-700 flex-shrink-0">
        <button
          onClick={() => handleSetActivePane("deadloop")}
          className={`flex-1 px-4 py-2 text-sm font-medium transition-colors ${
            activePane === "deadloop"
              ? "bg-amber-50 dark:bg-amber-900/20 text-amber-700 dark:text-amber-300 border-b-2 border-amber-500"
              : "text-gray-500 hover:bg-gray-100 dark:hover:bg-gray-800"
          }`}
        >
          Deadloop {isDeadloopPaused && "(Paused)"}
        </button>
        {activePane === "deadloop" && isAttached && (
          <>
            <button
              onClick={isDeadloopPaused ? resumeDeadloop : pauseDeadloop}
              className={`px-3 py-1 m-1 text-xs font-medium rounded transition-colors ${
                isDeadloopPaused
                  ? "bg-green-500 hover:bg-green-600 text-white"
                  : "bg-amber-500 hover:bg-amber-600 text-white"
              }`}
            >
              {isDeadloopPaused ? "Resume" : "Pause"}
            </button>
            <button
              onClick={() => {
                if (confirm("Are you sure you want to reboot the CLI?")) {
                  rebootCli();
                }
              }}
              className="px-3 py-1 m-1 text-xs font-medium rounded transition-colors bg-red-500 hover:bg-red-600 text-white"
            >
              Reboot
            </button>
          </>
        )}
        <button
          onClick={() => handleSetActivePane("interactive")}
          className={`flex-1 px-4 py-2 text-sm font-medium transition-colors ${
            activePane === "interactive"
              ? "bg-cyan-50 dark:bg-cyan-900/20 text-cyan-700 dark:text-cyan-300 border-b-2 border-cyan-500"
              : "text-gray-500 hover:bg-gray-100 dark:hover:bg-gray-800"
          }`}
        >
          Interactive
        </button>
        <button
          onClick={downloadSession}
          className="px-3 py-1 m-1 text-xs font-medium rounded transition-colors bg-blue-500 hover:bg-blue-600 text-white"
          title="Download session data"
        >
          ↓
        </button>
      </div>

      {/* Desktop: side-by-side view, Mobile: single pane view */}
      <div ref={containerRef} className="flex flex-1 min-h-0 overflow-hidden">
        {/* Left Pane - Deadloop */}
        <div
          className={`flex-col overflow-hidden ${
            activePane === "deadloop" ? "flex" : "hidden"
          } md:flex w-full`}
          style={{ width: typeof window !== 'undefined' && window.innerWidth >= 768 ? `${deadloopPercent}%` : undefined }}
        >
          <PaneHeader title="Deadloop (Autonomous)" type="deadloop" className="hidden md:block" />
          <MessagePane
            messages={deadloopMessages}
            paneType="deadloop"
            onLoadMore={loadMoreMessages}
            isLoading={isLoadingMore}
            hasMore={hasMoreMessages}
          />
          {deadloopStatus && (
            <div className="px-3 sm:px-4 py-2 border-t border-gray-200 dark:border-gray-700 bg-amber-50 dark:bg-amber-900/20 flex-shrink-0">
              <div className="flex items-center gap-2 text-sm text-amber-700 dark:text-amber-300">
                <div className="animate-pulse">●</div>
                <span>{deadloopStatus}</span>
              </div>
            </div>
          )}
        </div>

        {/* Resize handle between panes - desktop only */}
        <div className="hidden md:flex">
          <ResizeHandle
            direction="horizontal"
            onResize={handlePaneResize}
            onResizeEnd={handlePaneResizeEnd}
            className="h-full"
          />
        </div>

        {/* Right Pane - Interactive */}
        <div className={`flex-col overflow-hidden ${
          activePane === "interactive" ? "flex" : "hidden"
        } md:flex md:flex-1 w-full`}>
          <PaneHeader title="Interactive" type="interactive" className="hidden md:block" />
          <MessagePane
            messages={interactiveMessages}
            paneType="interactive"
            onLoadMore={loadMoreMessages}
            isLoading={isLoadingMore}
            hasMore={hasMoreMessages}
          />
          {interactiveStatus && (
            <div className="px-3 sm:px-4 py-2 border-t border-gray-200 dark:border-gray-700 bg-cyan-50 dark:bg-cyan-900/20 flex-shrink-0">
              <div className="flex items-center gap-2 text-sm text-cyan-700 dark:text-cyan-300">
                <div className="animate-pulse">●</div>
                <span>{interactiveStatus}</span>
              </div>
            </div>
          )}
          <div className="p-3 sm:p-4 border-t border-gray-200 dark:border-gray-700 flex-shrink-0">
            <InteractiveInput
              onSend={(text) => sendMessageToPane(text, "interactive")}
            />
          </div>
        </div>
      </div>
    </div>
  );
}

interface PaneHeaderProps {
  title: string;
  type: PaneType;
  className?: string;
}

function PaneHeader({ title, type, className }: PaneHeaderProps) {
  const isDeadloopPaused = useStore((state) => state.isDeadloopPaused);
  const pauseDeadloop = useStore((state) => state.pauseDeadloop);
  const resumeDeadloop = useStore((state) => state.resumeDeadloop);
  const rebootCli = useStore((state) => state.rebootCli);
  const downloadSession = useStore((state) => state.downloadSession);
  const isAttached = useStore((state) => state.isAttached);

  return (
    <div className={`px-4 py-2 border-b flex-shrink-0 flex items-center justify-between ${
      type === "deadloop"
        ? "bg-amber-50 dark:bg-amber-900/20 border-amber-200 dark:border-amber-800"
        : "bg-cyan-50 dark:bg-cyan-900/20 border-cyan-200 dark:border-cyan-800"
    } ${className || ""}`}>
      <h2 className={`font-semibold ${
        type === "deadloop"
          ? "text-amber-700 dark:text-amber-300"
          : "text-cyan-700 dark:text-cyan-300"
      }`}>
        {title}
        {type === "deadloop" && isDeadloopPaused && (
          <span className="ml-2 text-xs font-normal text-amber-600 dark:text-amber-400">(Paused)</span>
        )}
      </h2>
      {type === "deadloop" && (
        <div className="flex gap-2">
          {isAttached && (
            <>
              <button
                onClick={isDeadloopPaused ? resumeDeadloop : pauseDeadloop}
                className={`px-3 py-1 text-xs font-medium rounded transition-colors ${
                  isDeadloopPaused
                    ? "bg-green-500 hover:bg-green-600 text-white"
                    : "bg-amber-500 hover:bg-amber-600 text-white"
                }`}
              >
                {isDeadloopPaused ? "Resume" : "Pause"}
              </button>
              <button
                onClick={() => {
                  if (confirm("Are you sure you want to reboot the CLI?")) {
                    rebootCli();
                  }
                }}
                className="px-3 py-1 text-xs font-medium rounded transition-colors bg-red-500 hover:bg-red-600 text-white"
              >
                Reboot
              </button>
            </>
          )}
          <button
            onClick={downloadSession}
            className="px-3 py-1 text-xs font-medium rounded transition-colors bg-blue-500 hover:bg-blue-600 text-white"
            title="Download session data"
          >
            Download
          </button>
        </div>
      )}
    </div>
  );
}

interface MessagePaneProps {
  messages: Message[];
  paneType: PaneType;
  onLoadMore?: () => void;
  isLoading?: boolean;
  hasMore?: boolean;
}

function MessagePane({ messages, paneType, onLoadMore, isLoading, hasMore }: MessagePaneProps) {
  const sessionId = useStore((state) => state.sessionId);
  const containerRef = useRef<HTMLDivElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const shouldAutoScroll = useRef(true);
  const prevScrollHeight = useRef<number>(0);
  const previousSessionId = useRef<string | null>(null);
  const isRestoringScroll = useRef(false);

  const scrollKey = getScrollKey(sessionId, paneType);

  const checkIfAtBottom = useCallback(() => {
    const container = containerRef.current;
    if (!container) return true;
    const threshold = 100;
    const distanceFromBottom = container.scrollHeight - container.scrollTop - container.clientHeight;
    return distanceFromBottom <= threshold;
  }, []);

  const checkIfNearTop = useCallback(() => {
    const container = containerRef.current;
    if (!container) return false;
    return container.scrollTop < 100;
  }, []);

  const handleScroll = useCallback(() => {
    // Don't update state while we're restoring scroll position
    if (isRestoringScroll.current) return;

    shouldAutoScroll.current = checkIfAtBottom();

    // Save scroll position for current session+pane
    if (containerRef.current) {
      scrollPositions.set(scrollKey, {
        scrollTop: containerRef.current.scrollTop,
        wasAtBottom: shouldAutoScroll.current,
      });
    }

    // Check if near top and should load more
    if (checkIfNearTop() && onLoadMore && !isLoading && hasMore) {
      prevScrollHeight.current = containerRef.current?.scrollHeight || 0;
      onLoadMore();
    }
  }, [checkIfAtBottom, checkIfNearTop, onLoadMore, isLoading, hasMore, scrollKey]);

  // Save scroll position when switching away from a session
  useEffect(() => {
    const prevKey = getScrollKey(previousSessionId.current, paneType);
    if (previousSessionId.current && previousSessionId.current !== sessionId && containerRef.current) {
      scrollPositions.set(prevKey, {
        scrollTop: containerRef.current.scrollTop,
        wasAtBottom: shouldAutoScroll.current,
      });
    }
    previousSessionId.current = sessionId;
  }, [sessionId, paneType]);

  // Restore scroll position when switching to a session
  useEffect(() => {
    if (!sessionId || !containerRef.current) return;

    const savedState = scrollPositions.get(scrollKey);
    if (savedState) {
      isRestoringScroll.current = true;
      shouldAutoScroll.current = savedState.wasAtBottom;

      // Use requestAnimationFrame to ensure DOM is updated
      requestAnimationFrame(() => {
        if (containerRef.current) {
          if (savedState.wasAtBottom) {
            // Scroll to bottom
            containerRef.current.scrollTop = containerRef.current.scrollHeight;
          } else {
            // Restore exact position
            containerRef.current.scrollTop = savedState.scrollTop;
          }
        }
        isRestoringScroll.current = false;
      });
    } else {
      // New session - scroll to bottom
      shouldAutoScroll.current = true;
      requestAnimationFrame(() => {
        messagesEndRef.current?.scrollIntoView();
      });
    }
  }, [sessionId, scrollKey, messages.length]); // Re-run when messages load

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

  useEffect(() => {
    if (shouldAutoScroll.current && !isRestoringScroll.current) {
      messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [messages]);

  if (messages.length === 0) {
    return (
      <div className="flex-1 flex items-center justify-center text-gray-400 px-4">
        {paneType === "interactive" ? (
          <div className="text-center">
            <p className="text-sm">No messages yet</p>
            <p className="text-xs mt-1 opacity-75">Ask Claude to create TODO items here</p>
          </div>
        ) : (
          <p className="text-sm">Waiting for autonomous activity...</p>
        )}
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
        // Only clear on success
        if (textareaRef.current) {
          textareaRef.current.value = "";
          textareaRef.current.style.height = "auto";
        }
        setError(null);
      } else {
        // Show error and keep the message
        setError(result.error || "Failed to send message");
        // Auto-clear error after 5 seconds
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
    // Clear error when user starts typing again
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
          className="flex-1 resize-none rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-cyan-500"
          onKeyDown={handleKeyDown}
          onInput={handleInput}
        />
        <button
          onClick={handleSubmit}
          className="px-4 py-2 bg-cyan-500 hover:bg-cyan-600 text-white rounded-lg text-sm font-medium transition-colors"
        >
          Send
        </button>
      </div>
    </div>
  );
}
