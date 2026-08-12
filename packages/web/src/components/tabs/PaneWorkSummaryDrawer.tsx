"use client";

import React, { useEffect } from "react";
import {
  paneWorkSummaryKey,
  useStore,
} from "@/lib/store";
import { PaneWorkSummaryList } from "./PaneWorkSummaryContent";

export { formatSummaryWindow } from "./PaneWorkSummaryContent";

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

  useEffect(() => {
    list(sessionId, paneId, true);
  }, [list, paneId, sessionId]);

  return (
    <aside
      className="hidden h-full w-[min(26rem,38vw)] shrink-0 flex-col border-l border-gray-200 bg-gray-50 dark:border-gray-700 dark:bg-gray-950 md:flex"
      aria-label={`Work summaries for ${paneLabel}`}
    >
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
