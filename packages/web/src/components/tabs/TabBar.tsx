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
  if (provider === "opencode") {
    // OpenCode logo — code brackets < />
    return (
      <svg className={className} viewBox="0 0 24 24" fill="currentColor" aria-label="OpenCode">
        <path d="M8 3L2 12l6 9 2-1.5L5.5 12 10 4.5 8 3zm8 0l-2 1.5L18.5 12 14 19.5 16 21l6-9-6-9z" />
      </svg>
    );
  }
  if (provider === "cursor-agent") {
    // Cursor logo — upward arrow/cursor silhouette
    return (
      <svg className={className} viewBox="0 0 24 24" fill="currentColor" aria-label="Cursor">
        <path d="M3 2l9 20 3-8 8-3L3 2z" />
      </svg>
    );
  }
  // Claude / Anthropic logo — starburst (from docs.anthropic.com)
  return (
    <svg className={className} viewBox="1 0 50 50" fill="currentColor" aria-label="Claude">
      <path d="M23.1516 45.525L23.8096 42.611L24.5616 38.851L25.1726 35.843L25.7366 32.13L26.0656 30.908L26.0186 30.814L25.7836 30.861L22.9636 34.715L18.6866 40.496L15.3026 44.068L14.5036 44.397L13.0936 43.692L13.2346 42.376L14.0336 41.248L18.6866 35.279L21.5066 31.566L23.3396 29.451L23.2926 29.169H23.1986L10.7906 37.253L8.58158 37.535L7.59458 36.642L7.73558 35.185L8.20558 34.715L11.9186 32.13L21.1776 26.96L21.3186 26.49L21.1776 26.255H20.7076L19.1566 26.161L13.8926 26.02L9.33358 25.832L4.86858 25.597L3.74058 25.362L2.70658 23.952L2.80058 23.247L3.74058 22.636L5.10358 22.73L8.06458 22.965L12.5296 23.247L15.7726 23.435L20.5666 23.952H21.3186L21.4126 23.623L21.1776 23.435L20.9896 23.247L16.3366 20.145L11.3546 16.855L8.72258 14.928L7.31258 13.941L6.60758 13.048L6.32558 11.074L7.59458 9.664L9.33358 9.805L9.75658 9.899L11.4956 11.262L15.2086 14.129L20.0966 17.748L20.8016 18.312L21.1306 18.124V17.983L20.8016 17.466L18.1696 12.672L15.3496 7.784L14.0806 5.763L13.7516 4.541C13.6263 4.118 13.5636 3.648 13.5636 3.131L15.0206 1.157L15.8196 0.874997L17.7936 1.157L18.5926 1.862L19.8146 4.635L21.7416 9.006L24.7966 14.928L25.6896 16.714L26.1596 18.312L26.3476 18.829H26.6766V18.547L26.9116 15.163L27.3816 11.074L27.8516 5.81L27.9926 4.306L28.7446 2.52L30.2016 1.58L31.3296 2.097L32.2696 3.46L32.1286 4.306L31.6116 7.925L30.4836 13.612L29.7786 17.466H30.2016L30.6716 16.949L32.5986 14.411L35.8416 10.369L37.2516 8.771L38.9436 6.985L40.0246 6.139H42.0456L43.5026 8.348L42.8446 10.651L40.7766 13.283L39.0376 15.492L36.5466 18.829L35.0426 21.508L35.1836 21.696H35.5126L41.1056 20.474L44.1606 19.957L47.7326 19.346L49.3776 20.098L49.5656 20.85L48.9076 22.448L45.0536 23.388L40.5416 24.281L33.8206 25.879L33.7266 25.926L33.8206 26.067L36.8286 26.349L38.1446 26.443H41.3406L47.2626 26.866L48.8136 27.9L49.7066 29.122L49.5656 30.109L47.1686 31.284L43.9726 30.532L36.4526 28.746L33.9146 28.135H33.5386V28.323L35.7006 30.438L39.6016 33.963L44.5366 38.522L44.7716 39.65L44.1606 40.59L43.5026 40.496L39.1786 37.206L37.4866 35.749L33.7266 32.6H33.4916V32.929L34.3376 34.198L38.9436 41.107L39.1786 43.222L38.8496 43.88L37.6276 44.303L36.3586 44.068L33.6326 40.308L30.8596 36.031L28.6036 32.224L28.3686 32.412L27.0056 46.606L26.3946 47.311L24.9846 47.875L23.8096 46.982L23.1516 45.525Z" />
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
    const label = tab?.label ?? "";
    setRenameDraft(label);
    setRenamingPaneId(paneId);
    setContextMenu(null);
  }, [contextMenu, tabs]);

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
          const label = tab.label || `Tab ${index + 1}`;
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
                    : tab.provider === "cursor-agent"
                      ? "text-sky-500"
                      : tab.provider === "opencode"
                        ? "text-orange-500"
                        : isMiniMax
                          ? "text-cyan-500"
                          : isGlm
                            ? "text-emerald-500"
                            : "text-blue-500"
                }`}
                title={
                  tab.provider === "codex"
                    ? "Codex"
                    : tab.provider === "cursor-agent"
                      ? "Cursor"
                      : tab.provider === "opencode"
                        ? "OpenCode"
                        : isMiniMax
                          ? "MiniMax"
                          : isGlm
                            ? "GLM"
                            : "Claude"
                }
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
            onClick={() => { onAddTab("claude-old"); setShowMenu(false); }}
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2"
          >
            <span className="text-blue-400 flex-shrink-0">
              <ProviderIcon provider="claude" className="w-4 h-4" />
            </span>
            Claude-old Tab
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
          <div className="border-t border-gray-100 dark:border-gray-700 my-0.5" />
          <button
            onClick={() => { onAddTab("opencode"); setShowMenu(false); }}
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2"
          >
            <span className="text-orange-500 flex-shrink-0">
              <ProviderIcon provider="opencode" className="w-4 h-4" />
            </span>
            OpenCode Tab
          </button>
          <div className="border-t border-gray-100 dark:border-gray-700 my-0.5" />
          <button
            onClick={() => { onAddTab("cursor-agent"); setShowMenu(false); }}
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2"
          >
            <span className="text-sky-500 flex-shrink-0">
              <ProviderIcon provider="cursor-agent" className="w-4 h-4" />
            </span>
            Cursor Tab
          </button>
        </div>
      )}
    </div>
  );
}
