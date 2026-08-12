import type { ServerToWeb, WebToServer } from "@apas/protocol";

import { writePaneWorkSummarySnapshot } from "@/storage/cache";
import { paneWorkSummariesSupported, paneWorkSummaryKey, useMobileStore } from "@/state/store";

export function handlePaneWorkSummaryMessage(message: ServerToWeb): boolean {
  if (message.type === "pane_work_summaries") {
    const updatedAt = new Date().toISOString();
    const availability = message.availability ?? "unknown";
    useMobileStore.getState().replacePaneWorkSummaries(
      message.session_id,
      message.pane_id,
      message.summaries ?? [],
      availability,
      updatedAt,
    );
    persistPaneWorkSummaryCache(message.session_id, message.pane_id, updatedAt);
    return true;
  }
  if (message.type === "pane_work_summary_updated") {
    const updatedAt = new Date().toISOString();
    useMobileStore.getState().upsertPaneWorkSummary(
      message.session_id,
      message.pane_id,
      message.summary,
      message.availability,
      updatedAt,
    );
    persistPaneWorkSummaryCache(message.session_id, message.pane_id, updatedAt);
    return true;
  }
  return false;
}

export function reconcileVisiblePaneWorkSummaries(send: (message: WebToServer) => boolean): boolean {
  const state = useMobileStore.getState();
  const pane = state.visibleSummaryPane;
  if (!pane || !paneWorkSummariesSupported()) return false;
  state.beginPaneWorkSummaryRequest(pane.sessionId, pane.paneId);
  return send({
    type: "list_pane_work_summaries",
    session_id: pane.sessionId,
    pane_id: pane.paneId,
    include_current: true,
  });
}

function persistPaneWorkSummaryCache(sessionId: string, paneId: number, updatedAt: string): void {
  const cache = useMobileStore.getState().paneWorkSummaries[paneWorkSummaryKey(sessionId, paneId)];
  if (!cache) return;
  void writePaneWorkSummarySnapshot(
    sessionId,
    paneId,
    cache.summaries,
    cache.availability,
    updatedAt,
  ).catch(() => undefined);
}
