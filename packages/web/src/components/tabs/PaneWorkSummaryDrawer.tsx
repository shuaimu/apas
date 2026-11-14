"use client";

import React, { useCallback, useEffect, useRef, useState } from "react";
import { ResizeHandle } from "@/components/ResizeHandle";
import {
  paneWorkSummaryKey,
  useStore,
} from "@/lib/store";
import { PaneWorkSummaryList } from "./PaneWorkSummaryContent";

export { formatSummaryWindow } from "./PaneWorkSummaryContent";

const WIDTH_KEY = "apas_layout_global_summary_width";
const DEFAULT_WIDTH = 416;
const MIN_WIDTH = 280;
const MAX_WIDTH = 800;

export function PaneWorkSummaryDrawer({
  sessionId,
  paneId,
  paneLabel,
  onClose,
}: {
  sessionId: string;
  paneId: number;
  paneLabel: string;
  onClose: () => void;
}) {
  const key = paneWorkSummaryKey(sessionId, paneId);
  const cache = useStore((state) => state.paneWorkSummaries[key]);
  const list = useStore((state) => state.listPaneWorkSummaries);
  const refresh = useStore((state) => state.refreshPaneWorkSummary);
  const drawerRef = useRef<HTMLElement>(null);
  const [preferredWidth, setPreferredWidth] = useState(DEFAULT_WIDTH);
  const [widthLoaded, setWidthLoaded] = useState(false);
  const [availableWidth, setAvailableWidth] = useState<number | null>(null);

  useEffect(() => {
    try {
      const saved = Number(localStorage.getItem(WIDTH_KEY));
      if (Number.isFinite(saved) && saved >= MIN_WIDTH && saved <= MAX_WIDTH) {
        setPreferredWidth(saved);
      }
    } catch {
      // Resizing still works when browser storage is unavailable.
    }
    setWidthLoaded(true);
  }, []);

  useEffect(() => {
    if (!widthLoaded) return;
    try {
      localStorage.setItem(WIDTH_KEY, String(preferredWidth));
    } catch {
      // Keep the preference for this mounted drawer only.
    }
  }, [preferredWidth, widthLoaded]);

  useEffect(() => {
    const container = drawerRef.current?.parentElement;
    if (!container) return;
    const measure = () => {
      const width = container.getBoundingClientRect().width;
      if (width > 0) setAvailableWidth(width);
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(container);
    window.addEventListener("resize", measure);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, []);

  // Leave at least 320px for the conversation where possible. In a narrower
  // workspace, each side gets half rather than overflowing the window.
  const maxWidth = availableWidth === null ? MAX_WIDTH
    : Math.min(MAX_WIDTH, availableWidth - Math.min(320, availableWidth / 2));
  const minWidth = Math.min(MIN_WIDTH, maxWidth);
  const width = Math.min(maxWidth, Math.max(minWidth, preferredWidth));
  const resize = useCallback((delta: number) => {
    setPreferredWidth((previous) => {
      const visible = Math.min(maxWidth, Math.max(minWidth, previous));
      return Math.max(MIN_WIDTH, Math.min(maxWidth, Math.max(minWidth, visible - delta)));
    });
  }, [maxWidth, minWidth]);

  const resizeWithKeyboard = (event: React.KeyboardEvent) => {
    const step = event.shiftKey ? 48 : 16;
    switch (event.key) {
      case "ArrowLeft": resize(-step); break;
      case "ArrowRight": resize(step); break;
      case "Home": setPreferredWidth(MIN_WIDTH); break;
      case "End": setPreferredWidth(Math.max(MIN_WIDTH, maxWidth)); break;
      default: return;
    }
    event.preventDefault();
  };

  useEffect(() => {
    list(sessionId, paneId, true);
  }, [list, paneId, sessionId]);

  return (
    <aside
      id="pane-work-summary-drawer"
      ref={drawerRef}
      className="relative hidden h-full min-w-0 shrink-0 flex-col border-l border-gray-200 bg-gray-50 dark:border-gray-700 dark:bg-gray-950 md:flex"
      style={{ width }}
      aria-label={`Work summaries for ${paneLabel}`}
    >
      <div
        role="separator"
        tabIndex={0}
        aria-label="Resize work summary"
        aria-orientation="vertical"
        aria-controls="pane-work-summary-drawer"
        aria-valuemin={Math.round(minWidth)}
        aria-valuemax={Math.round(maxWidth)}
        aria-valuenow={Math.round(width)}
        aria-valuetext={`${Math.round(width)} pixels wide`}
        onKeyDown={resizeWithKeyboard}
        className="absolute inset-y-0 left-0 z-10 w-2 focus-visible:outline-2 focus-visible:outline-blue-500"
        title="Drag or use arrow keys to resize the summary"
      >
        <ResizeHandle direction="horizontal" onResize={resize} className="h-full touch-none" />
      </div>
      <header className="flex items-center gap-2 border-b border-gray-200 px-4 py-3 dark:border-gray-700">
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-sm font-semibold text-gray-900 dark:text-gray-100">Work summary</h2>
          <p className="truncate text-xs text-gray-500 dark:text-gray-400">{paneLabel} · 3-hour windows</p>
        </div>
        <button
          type="button"
          onClick={() => refresh(sessionId, paneId)}
          disabled={cache?.loading}
          className="rounded px-2 py-1 text-xs font-medium text-blue-600 hover:bg-blue-50 disabled:opacity-50 dark:text-blue-300 dark:hover:bg-blue-950"
          title="Refresh the current three-hour window"
        >
          Refresh
        </button>
        <button
          type="button"
          onClick={onClose}
          className="rounded p-1 text-gray-500 hover:bg-gray-200 hover:text-gray-900 dark:hover:bg-gray-800 dark:hover:text-gray-100"
          aria-label="Close work summary"
        >
          <svg className="h-4 w-4" fill="none" stroke="currentColor" strokeWidth="2" viewBox="0 0 24 24"><path d="M6 6l12 12M18 6L6 18" /></svg>
        </button>
      </header>

      <div className="flex-1 space-y-3 overflow-y-auto p-3">
        <PaneWorkSummaryList
          cache={cache}
          onRetry={(windowStart) => refresh(sessionId, paneId, windowStart)}
        />
      </div>
    </aside>
  );
}
