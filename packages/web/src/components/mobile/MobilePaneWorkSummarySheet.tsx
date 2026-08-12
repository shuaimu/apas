"use client";

import { History, X } from "lucide-react";
import { useEffect } from "react";
import { PaneWorkSummaryList } from "@/components/tabs/PaneWorkSummaryContent";
import {
  paneWorkSummaryKey,
  useStore,
  type PaneConfig,
} from "@/lib/store";

function paneLabel(pane: PaneConfig): string {
  return pane.label?.trim() || `${pane.kind === "terminal" ? "Terminal" : "Pane"} ${pane.pane_id}`;
}

export function MobilePaneWorkSummarySheet({
  connected,
  sessionId,
  paneId,
  panes,
  onSelectPane,
  onClose,
}: {
  connected: boolean;
  sessionId: string;
  paneId: number;
  panes: PaneConfig[];
  onSelectPane: (paneId: number) => void;
  onClose: () => void;
}) {
  const key = paneWorkSummaryKey(sessionId, paneId);
  const cache = useStore((state) => state.paneWorkSummaries[key]);
  const supported = useStore((state) => state.negotiatedCapabilities.has("pane_work_summary_v1"));
  const list = useStore((state) => state.listPaneWorkSummaries);
  const refresh = useStore((state) => state.refreshPaneWorkSummary);
  const selected = panes.find((pane) => pane.pane_id === paneId);
  const controlsDisabled = !connected || !supported || Boolean(cache?.loading);

  useEffect(() => {
    if (supported) list(sessionId, paneId, true);
  }, [list, paneId, sessionId, supported]);

  return (
    <div className="fixed inset-0 z-[96] flex items-end bg-black/45" onClick={onClose}>
      <section
        role="dialog"
        aria-modal="true"
        aria-label={`Work summaries for ${selected ? paneLabel(selected) : `Pane ${paneId}`}`}
        onClick={(event) => event.stopPropagation()}
        className="flex max-h-[90dvh] min-h-[62dvh] w-full flex-col rounded-t-[1.4rem] border-t border-[#dedee7] bg-[#f7f7fa] shadow-2xl dark:border-[#383842] dark:bg-[#111115]"
      >
        <header className="shrink-0 border-b border-[#dedee7] px-4 pt-4 pb-3 dark:border-[#383842]">
          <div className="flex items-start gap-3">
            <div className="mt-0.5 rounded-xl bg-[#eeecff] p-2 text-[#5b4de0] dark:bg-[#292452] dark:text-[#c8c1ff]">
              <History className="h-5 w-5" />
            </div>
            <div className="min-w-0 flex-1">
              <h2 className="text-lg font-extrabold">Work summary</h2>
              <p className="truncate text-xs text-[#686873] dark:text-[#aaaab6]">
                {selected ? paneLabel(selected) : `Pane ${paneId}`} · 3-hour windows
              </p>
            </div>
            <button
              type="button"
              onClick={() => refresh(sessionId, paneId)}
              disabled={controlsDisabled}
              className="rounded-lg px-2 py-1.5 text-xs font-bold text-[#5b4de0] disabled:opacity-40 dark:text-[#c8c1ff]"
            >
              Refresh
            </button>
            <button type="button" aria-label="Close work summary" onClick={onClose} className="rounded-lg p-1.5 hover:bg-[#efeff5] dark:hover:bg-[#25252d]">
              <X className="h-5 w-5" />
            </button>
          </div>

          {panes.length > 1 && (
            <div className="no-scrollbar mt-3 flex gap-2 overflow-x-auto" aria-label="Summary pane">
              {panes.map((pane) => (
                <button
                  key={pane.pane_id}
                  type="button"
                  aria-pressed={pane.pane_id === paneId}
                  onClick={() => onSelectPane(pane.pane_id)}
                  className={`shrink-0 rounded-full border px-3 py-1.5 text-xs font-bold ${pane.pane_id === paneId ? "border-[#6d5efc] text-[#6d5efc]" : "border-[#dedee7] text-[#686873] dark:border-[#383842] dark:text-[#aaaab6]"}`}
                >
                  {paneLabel(pane)}
                </button>
              ))}
            </div>
          )}
        </header>

        <div className="min-h-0 flex-1 space-y-3 overflow-y-auto p-3 pb-[max(1rem,env(safe-area-inset-bottom))]">
          {!connected && (
            <div className="rounded-md border border-amber-200 bg-amber-50 p-3 text-xs text-amber-800 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
              Offline. Showing the latest summaries already loaded in this browser; reconnect to refresh or retry.
            </div>
          )}
          <PaneWorkSummaryList
            cache={cache}
            retryDisabled={controlsDisabled}
            onRetry={(windowStart) => refresh(sessionId, paneId, windowStart)}
          />
        </div>
      </section>
    </div>
  );
}
