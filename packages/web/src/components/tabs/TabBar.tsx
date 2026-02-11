"use client";

import { useRef, useEffect, useState } from "react";
import { PaneConfig, paneKey } from "@/lib/store";

interface TabBarProps {
  tabs: PaneConfig[];
  activeTabId: number | null;
  onSelectTab: (paneId: number) => void;
  onCloseTab: (paneId: number) => void;
  onAddTab: (provider?: string) => void;
  paneStatuses: Record<string, string | null>;
  pausedPanes: number[];
}

export function TabBar({
  tabs,
  activeTabId,
  onSelectTab,
  onCloseTab,
  onAddTab,
  paneStatuses,
  pausedPanes,
}: TabBarProps) {
  const scrollRef = useRef<HTMLDivElement>(null);

  // Scroll active tab into view when it changes
  useEffect(() => {
    if (activeTabId == null || !scrollRef.current) return;
    const activeEl = scrollRef.current.querySelector(`[data-tab-id="${activeTabId}"]`);
    if (activeEl) {
      activeEl.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "nearest" });
    }
  }, [activeTabId]);

  return (
    <div className="flex items-end border-b border-gray-200 dark:border-gray-700 bg-gray-100 dark:bg-gray-800/50 flex-shrink-0 min-h-[40px]">
      <div
        ref={scrollRef}
        className="flex-1 flex flex-wrap items-end overflow-x-hidden overflow-y-hidden"
      >
        {tabs.map((tab, index) => {
          const isActive = tab.pane_id === activeTabId;
          const isBot = tab.mode === "deadloop";
          const isPaused = pausedPanes.includes(tab.pane_id);
          const status = paneStatuses[paneKey(tab.pane_id)];
          const hasActivity = !!status;
          const label = tab.label || `Tab ${index + 1}`;

          return (
            <button
              key={tab.pane_id}
              data-tab-id={tab.pane_id}
              onClick={() => onSelectTab(tab.pane_id)}
              className={`group relative flex items-center gap-1.5 px-3 py-2 text-sm font-medium transition-colors flex-shrink-0 border-r border-gray-200 dark:border-gray-700 max-w-[200px] ${
                isActive
                  ? "bg-white dark:bg-gray-900 text-gray-900 dark:text-gray-100 border-b-2 border-b-blue-500 -mb-px"
                  : "text-gray-500 dark:text-gray-400 hover:bg-gray-200 dark:hover:bg-gray-700/50 hover:text-gray-700 dark:hover:text-gray-300"
              }`}
              style={{ scrollSnapAlign: "start" }}
            >
              {/* Status dot */}
              {hasActivity && !isPaused && (
                <span className="w-2 h-2 rounded-full bg-blue-500 animate-pulse flex-shrink-0" />
              )}
              {isPaused && (
                <span className="w-2 h-2 rounded-full bg-amber-500 flex-shrink-0" />
              )}
              {!hasActivity && !isPaused && (
                <span
                  className={`w-2 h-2 rounded-full flex-shrink-0 ${
                    tab.provider === "codex" ? "bg-green-500" : "bg-blue-500"
                  }`}
                  title={tab.provider === "codex" ? "Codex" : "Claude"}
                />
              )}

              {/* Label */}
              <span className="truncate">
                {label}
                {isBot && " (Bot)"}
              </span>

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

      {/* Add tab button with provider dropdown */}
      <AddTabButton onAddTab={onAddTab} />
    </div>
  );
}

function AddTabButton({ onAddTab }: { onAddTab: (provider?: string) => void }) {
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
            <span className="w-2 h-2 rounded-full bg-blue-500 flex-shrink-0" />
            Claude Tab
          </button>
          <div className="border-t border-gray-100 dark:border-gray-700 my-0.5" />
          <button
            onClick={() => { onAddTab("codex"); setShowMenu(false); }}
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2"
          >
            <span className="w-2 h-2 rounded-full bg-green-500 flex-shrink-0" />
            Codex Tab
          </button>
        </div>
      )}
    </div>
  );
}
