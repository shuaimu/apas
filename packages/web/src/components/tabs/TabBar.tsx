"use client";

import { useCallback, useRef, useEffect, useState } from "react";
import { Bot } from "lucide-react";
import { PaneConfig, paneKey } from "@/lib/store";

interface TabBarProps {
  tabs: PaneConfig[];
  activeTabId: number | null;
  onSelectTab: (paneId: number) => void;
  onCloseTab: (paneId: number) => void;
  onAddTab: (provider?: string, model?: string) => void;
  onRenameTab?: (paneId: number, newLabel: string) => void;
  customLabels?: Record<number, string>;
  onReorderTabs?: (orderedIds: number[]) => void;
  onBootCli?: () => void;
  onRebootCli?: () => void;
  showBootButton?: boolean;
  showRebootButton?: boolean;
  paneStatuses: Record<string, string | null>;
  pausedPanes: number[];
}

const MINIMAX_DEFAULT_MODEL = "MiniMax-M2.7";
const GLM_DEFAULT_MODEL = "glm-5.1";

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

function isMiniMaxTab(provider: string, model?: string, label?: string): boolean {
  if (provider === "minimax") return true;
  if (provider !== "claude") return false;
  if (isMiniMaxModel(model)) return true;
  return typeof label === "string" && label.toLowerCase().includes("minimax");
}

function isGlmTab(provider: string, model?: string, label?: string): boolean {
  if (provider === "glm") return true;
  if (provider !== "claude") return false;
  if (isMiniMaxModel(model)) return false;
  if (isGlmModel(model)) return true;
  return typeof label === "string" && label.toLowerCase().includes("glm");
}

function ProviderIcon({
  provider,
  model,
  label,
  className = "w-3.5 h-3.5",
}: {
  provider: string;
  model?: string;
  label?: string;
  className?: string;
}) {
  if (provider === "codex") {
    // OpenAI logo — stylized hexagonal node ring
    return (
      <svg className={className} viewBox="0 0 24 24" fill="currentColor" aria-label="Codex">
        <path d="M22.282 9.821a5.985 5.985 0 0 0-.516-4.91 6.046 6.046 0 0 0-6.51-2.9A6.065 6.065 0 0 0 4.981 4.18a5.985 5.985 0 0 0-3.998 2.9 6.046 6.046 0 0 0 .743 7.097 5.98 5.98 0 0 0 .51 4.911 6.051 6.051 0 0 0 6.515 2.9A5.985 5.985 0 0 0 13.26 24a6.056 6.056 0 0 0 5.772-4.206 5.99 5.99 0 0 0 3.997-2.9 6.056 6.056 0 0 0-.747-7.073zM13.26 22.43a4.476 4.476 0 0 1-2.876-1.04l.141-.081 4.779-2.758a.795.795 0 0 0 .392-.681v-6.737l2.02 1.168a.071.071 0 0 1 .038.052v5.583a4.504 4.504 0 0 1-4.494 4.494zM3.6 18.304a4.47 4.47 0 0 1-.535-3.014l.142.085 4.783 2.759a.771.771 0 0 0 .78 0l5.843-3.369v2.332a.08.08 0 0 1-.033.062L9.74 19.95a4.5 4.5 0 0 1-6.14-1.646zM2.34 7.896a4.485 4.485 0 0 1 2.366-1.973V11.6a.766.766 0 0 0 .388.676l5.815 3.355-2.02 1.168a.076.076 0 0 1-.071 0l-4.83-2.786A4.504 4.504 0 0 1 2.34 7.872zm16.597 3.855l-5.833-3.387L15.119 7.2a.076.076 0 0 1 .071 0l4.83 2.791a4.494 4.494 0 0 1-.676 8.105v-6.678a.79.79 0 0 0-.407-.667zm2.01-3.023l-.141-.085-4.774-2.782a.776.776 0 0 0-.785 0L9.409 9.23V6.897a.066.066 0 0 1 .028-.061l4.83-2.787a4.5 4.5 0 0 1 6.68 4.66zm-12.64 4.135l-2.02-1.164a.08.08 0 0 1-.038-.057V6.075a4.5 4.5 0 0 1 7.375-3.453l-.142.08L8.704 5.46a.795.795 0 0 0-.393.681zm1.097-2.365l2.602-1.5 2.607 1.5v2.999l-2.597 1.5-2.607-1.5z" />
      </svg>
    );
  }
  if (isMiniMaxTab(provider, model, label)) {
    // MiniMax logo — abstract "M" shape
    return (
      <svg className={className} viewBox="0 0 24 24" fill="currentColor" aria-label="MiniMax">
        <path d="M3 3h4.5L12 10l4.5-7H21v18h-4v-9.5L13 17h-2L7 11.5V21H3V3z" />
      </svg>
    );
  }
  if (isGlmTab(provider, model, label)) {
    // GLM / Zhipu AI logo — abstract "Z" shape
    return (
      <svg className={className} viewBox="0 0 24 24" fill="currentColor" aria-label="GLM">
        <path d="M4 4h16v3H9l11 13H4v-3h11L4 4zm0 0" />
      </svg>
    );
  }
  // Claude / Anthropic logo — stylized starburst
  return (
    <svg className={className} viewBox="0 0 24 24" fill="currentColor" aria-label="Claude">
      <path d="M17.304 3.541l-5.357 16.918H9.598L14.955 3.54h2.349zM6.696 3L1.338 19.918h2.349L9.045 3H6.696zM17.304 3l-2.149 6.79h2.378L19.683 3h-2.379zM6.696 3l2.15 6.79H6.467L4.317 3h2.38zM12 8.742L9.851 15.53h4.298L12 8.742z" />
    </svg>
  );
}

export function TabBar({
  tabs,
  activeTabId,
  onSelectTab,
  onCloseTab,
  onAddTab,
  onRenameTab,
  customLabels,
  onReorderTabs,
  onBootCli,
  onRebootCli,
  showBootButton = false,
  showRebootButton = false,
  paneStatuses,
  pausedPanes,
}: TabBarProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [contextMenu, setContextMenu] = useState<{ paneId: number; x: number; y: number } | null>(null);
  const [renamingPaneId, setRenamingPaneId] = useState<number | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const renameInputRef = useRef<HTMLInputElement>(null);
  const contextMenuRef = useRef<HTMLDivElement>(null);

  // Drag-and-drop state
  const [draggedPaneId, setDraggedPaneId] = useState<number | null>(null);
  const [dropTargetId, setDropTargetId] = useState<number | null>(null);
  const [dropSide, setDropSide] = useState<"left" | "right" | null>(null);

  // Scroll active tab into view when it changes
  useEffect(() => {
    if (activeTabId == null || !scrollRef.current) return;
    const activeEl = scrollRef.current.querySelector(`[data-tab-id="${activeTabId}"]`);
    if (activeEl) {
      activeEl.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "nearest" });
    }
  }, [activeTabId]);

  // Dismiss context menu on outside click
  useEffect(() => {
    if (!contextMenu) return;
    const handleClick = (e: MouseEvent) => {
      if (contextMenuRef.current && !contextMenuRef.current.contains(e.target as Node)) {
        setContextMenu(null);
      }
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [contextMenu]);

  // Auto-focus rename input
  useEffect(() => {
    if (renamingPaneId != null && renameInputRef.current) {
      renameInputRef.current.focus();
      renameInputRef.current.select();
    }
  }, [renamingPaneId]);

  const handleContextMenu = useCallback((e: React.MouseEvent, paneId: number) => {
    e.preventDefault();
    e.stopPropagation();
    setContextMenu({ paneId, x: e.clientX, y: e.clientY });
  }, []);

  const handleStartRename = useCallback(() => {
    if (contextMenu == null) return;
    const paneId = contextMenu.paneId;
    const tab = tabs.find((t) => t.pane_id === paneId);
    const label = customLabels?.[paneId] ?? tab?.label ?? "";
    setRenameDraft(label);
    setRenamingPaneId(paneId);
    setContextMenu(null);
  }, [contextMenu, tabs, customLabels]);

  const handleFinishRename = useCallback(() => {
    if (renamingPaneId == null) return;
    const trimmed = renameDraft.trim();
    if (trimmed && onRenameTab) {
      onRenameTab(renamingPaneId, trimmed);
    }
    setRenamingPaneId(null);
    setRenameDraft("");
  }, [renamingPaneId, renameDraft, onRenameTab]);

  const handleRenameKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      handleFinishRename();
    } else if (e.key === "Escape") {
      e.preventDefault();
      setRenamingPaneId(null);
      setRenameDraft("");
    }
  }, [handleFinishRename]);

  // Drag handlers
  const handleDragStart = useCallback((e: React.DragEvent, paneId: number) => {
    setDraggedPaneId(paneId);
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", String(paneId));
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent, paneId: number) => {
    if (draggedPaneId == null || draggedPaneId === paneId) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const midX = rect.left + rect.width / 2;
    setDropTargetId(paneId);
    setDropSide(e.clientX < midX ? "left" : "right");
  }, [draggedPaneId]);

  const handleDrop = useCallback((e: React.DragEvent, targetPaneId: number) => {
    e.preventDefault();
    if (draggedPaneId == null || draggedPaneId === targetPaneId || !onReorderTabs) {
      setDraggedPaneId(null);
      setDropTargetId(null);
      setDropSide(null);
      return;
    }
    const currentOrder = tabs.map((t) => t.pane_id);
    const fromIndex = currentOrder.indexOf(draggedPaneId);
    let toIndex = currentOrder.indexOf(targetPaneId);
    if (fromIndex === -1 || toIndex === -1) return;

    const newOrder = currentOrder.filter((id) => id !== draggedPaneId);
    toIndex = newOrder.indexOf(targetPaneId);
    const insertAt = dropSide === "right" ? toIndex + 1 : toIndex;
    newOrder.splice(insertAt, 0, draggedPaneId);

    onReorderTabs(newOrder);
    setDraggedPaneId(null);
    setDropTargetId(null);
    setDropSide(null);
  }, [draggedPaneId, dropSide, tabs, onReorderTabs]);

  const handleDragEnd = useCallback(() => {
    setDraggedPaneId(null);
    setDropTargetId(null);
    setDropSide(null);
  }, []);

  return (
    <div className="flex items-end border-b border-gray-200 dark:border-gray-700 bg-gray-100 dark:bg-gray-800/50 flex-shrink-0 min-h-[40px]">
      <div
        ref={scrollRef}
        className="flex-1 flex flex-wrap items-end overflow-x-hidden overflow-y-hidden"
      >
        {tabs.map((tab, index) => {
          const isActive = tab.pane_id === activeTabId;
          const isBot = tab.mode === "deadloop";
          const isMiniMax = isMiniMaxTab(tab.provider, tab.model, tab.label);
          const isGlm = isGlmTab(tab.provider, tab.model, tab.label);
          const isPaused = pausedPanes.includes(tab.pane_id);
          const status = paneStatuses[paneKey(tab.pane_id)];
          const hasActivity = !!status;
          const label = customLabels?.[tab.pane_id] ?? (tab.label || `Tab ${index + 1}`);
          const isRenaming = renamingPaneId === tab.pane_id;

          return (
            <button
              key={tab.pane_id}
              data-tab-id={tab.pane_id}
              draggable={!isRenaming}
              onDragStart={(e) => handleDragStart(e, tab.pane_id)}
              onDragOver={(e) => handleDragOver(e, tab.pane_id)}
              onDrop={(e) => handleDrop(e, tab.pane_id)}
              onDragEnd={handleDragEnd}
              onClick={() => { if (!isRenaming) onSelectTab(tab.pane_id); }}
              onContextMenu={(e) => handleContextMenu(e, tab.pane_id)}
              className={`group relative flex items-center gap-1.5 px-3 py-2 text-sm font-medium transition-colors flex-shrink-0 border-r border-gray-200 dark:border-gray-700 max-w-[200px] ${
                isActive
                  ? "bg-white dark:bg-gray-900 text-gray-900 dark:text-gray-100 border-b-2 border-b-blue-500 -mb-px"
                  : "text-gray-500 dark:text-gray-400 hover:bg-gray-200 dark:hover:bg-gray-700/50 hover:text-gray-700 dark:hover:text-gray-300"
              } ${draggedPaneId === tab.pane_id ? "opacity-40" : ""}`}
              style={{ scrollSnapAlign: "start" }}
            >
              {/* Drop indicator */}
              {dropTargetId === tab.pane_id && dropSide === "left" && (
                <div className="absolute left-0 top-0 bottom-0 w-0.5 bg-blue-500 -translate-x-1/2 z-10" />
              )}
              {dropTargetId === tab.pane_id && dropSide === "right" && (
                <div className="absolute right-0 top-0 bottom-0 w-0.5 bg-blue-500 translate-x-1/2 z-10" />
              )}

              {/* Provider icon + status badge */}
              <span
                className={`relative inline-flex items-center justify-center flex-shrink-0 ${
                  tab.provider === "codex"
                    ? "text-green-500"
                    : isMiniMax
                      ? "text-cyan-500"
                      : isGlm
                        ? "text-emerald-500"
                        : "text-blue-500"
                }`}
                title={tab.provider === "codex" ? "Codex" : isMiniMax ? "MiniMax" : isGlm ? "GLM" : "Claude"}
              >
                <ProviderIcon provider={tab.provider} model={tab.model} label={tab.label} />
                {hasActivity && !isPaused && (
                  <span className="absolute -top-0.5 -right-0.5 w-1.5 h-1.5 rounded-full bg-blue-500 animate-pulse" />
                )}
                {isPaused && (
                  <span className="absolute -top-0.5 -right-0.5 w-1.5 h-1.5 rounded-full bg-amber-500" />
                )}
              </span>

              {/* Label */}
              {isRenaming ? (
                <input
                  ref={renameInputRef}
                  value={renameDraft}
                  onChange={(e) => setRenameDraft(e.target.value)}
                  onBlur={handleFinishRename}
                  onKeyDown={handleRenameKeyDown}
                  onClick={(e) => e.stopPropagation()}
                  className="w-full min-w-[60px] max-w-[140px] px-1 py-0 text-sm bg-white dark:bg-gray-800 border border-blue-400 rounded outline-none"
                />
              ) : (
                <span className="truncate">
                  {label}
                  {isBot && " (Bot)"}
                </span>
              )}

              {/* Close button - visible on hover or when active */}
              {tabs.length > 1 && (
                <span
                  onClick={(e) => {
                    e.stopPropagation();
                    onCloseTab(tab.pane_id);
                  }}
                  className={`ml-1 p-0.5 rounded hover:bg-gray-300 dark:hover:bg-gray-600 flex-shrink-0 ${
                    isActive
                      ? "opacity-60 hover:opacity-100"
                      : "opacity-0 group-hover:opacity-60 hover:!opacity-100"
                  }`}
                  title="Close tab"
                >
                  <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                  </svg>
                </span>
              )}
            </button>
          );
        })}
      </div>

      {/* Right-click context menu */}
      {contextMenu && (
        <div
          ref={contextMenuRef}
          className="fixed bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg shadow-lg py-1 z-50 min-w-[120px]"
          style={{ left: contextMenu.x, top: contextMenu.y }}
        >
          {onRenameTab && (
            <button
              onClick={handleStartRename}
              className="w-full text-left px-3 py-1.5 text-sm hover:bg-gray-100 dark:hover:bg-gray-700"
            >
              Rename
            </button>
          )}
          <button
            onClick={() => {
              if (contextMenu) onCloseTab(contextMenu.paneId);
              setContextMenu(null);
            }}
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-gray-100 dark:hover:bg-gray-700 text-red-600 dark:text-red-400"
          >
            Close
          </button>
        </div>
      )}

      {/* Global tab-bar actions */}
      <div className="flex items-center gap-1 pr-1">
        <AddTabButton onAddTab={onAddTab} />
        {showBootButton && onBootCli && (
          <button
            onClick={() => {
              if (confirm("Boot APAS CLI for this project?")) onBootCli();
            }}
            className="px-2.5 h-8 my-1 text-xs font-medium rounded transition-colors bg-blue-500 hover:bg-blue-600 text-white"
            title="Boot CLI"
          >
            Boot
          </button>
        )}
        {showRebootButton && onRebootCli && (
          <button
            onClick={() => {
              if (confirm("Are you sure you want to reboot the CLI?")) onRebootCli();
            }}
            className="px-2.5 h-8 my-1 text-xs font-medium rounded transition-colors bg-red-500 hover:bg-red-600 text-white"
            title="Reboot CLI"
          >
            Reboot
          </button>
        )}
      </div>
    </div>
  );
}

function AddTabButton({ onAddTab }: { onAddTab: (provider?: string, model?: string) => void }) {
  const [showMenu, setShowMenu] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!showMenu) return;
    const handleClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setShowMenu(false);
      }
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [showMenu]);

  return (
    <div className="relative flex-shrink-0" ref={menuRef}>
      <button
        onClick={() => setShowMenu((v) => !v)}
        className="flex items-center justify-center w-8 h-8 m-1 rounded text-gray-400 dark:text-gray-500 hover:text-gray-600 dark:hover:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-700 transition-colors"
        title="New tab"
      >
        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
        </svg>
      </button>
      {showMenu && (
        <div className="absolute right-0 top-full mt-1 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg shadow-lg py-1 z-50 min-w-[160px]">
          <button
            onClick={() => { onAddTab("claude"); setShowMenu(false); }}
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2"
          >
            <span className="text-blue-500 flex-shrink-0">
              <ProviderIcon provider="claude" className="w-4 h-4" />
            </span>
            Claude Tab
          </button>
          <button
            onClick={() => {
              onAddTab("claude", MINIMAX_DEFAULT_MODEL);
              setShowMenu(false);
            }}
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2"
          >
            <span className="text-cyan-500 flex-shrink-0">
              <ProviderIcon provider="claude" model={MINIMAX_DEFAULT_MODEL} className="w-4 h-4" />
            </span>
            MiniMax 2.7 Tab
          </button>
          <button
            onClick={() => {
              onAddTab("claude", GLM_DEFAULT_MODEL);
              setShowMenu(false);
            }}
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2"
          >
            <span className="text-emerald-500 flex-shrink-0">
              <ProviderIcon provider="claude" model={GLM_DEFAULT_MODEL} className="w-4 h-4" />
            </span>
            GLM 5.1 Tab
          </button>
          <div className="border-t border-gray-100 dark:border-gray-700 my-0.5" />
          <button
            onClick={() => { onAddTab("codex"); setShowMenu(false); }}
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2"
          >
            <span className="text-green-500 flex-shrink-0">
              <ProviderIcon provider="codex" className="w-4 h-4" />
            </span>
            Codex Tab
          </button>
        </div>
      )}
    </div>
  );
}
