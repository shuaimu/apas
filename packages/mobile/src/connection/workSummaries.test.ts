import type { PaneWorkSummary } from "@apas/protocol";

import { paneWorkSummaryKey, useMobileStore } from "@/state/store";
import { handlePaneWorkSummaryMessage, reconcileVisiblePaneWorkSummaries } from "./workSummaries";
import { writePaneWorkSummarySnapshot } from "@/storage/cache";

jest.mock("@/storage/cache", () => ({
  writePaneWorkSummarySnapshot: jest.fn(async () => undefined),
}));

const sessionId = "58dbd62d-c40f-4b5b-a1d4-96aca52ea595";

function summary(windowStart: string, status: PaneWorkSummary["status"] = "complete"): PaneWorkSummary {
  return {
    session_id: sessionId,
    pane_id: 3,
    window_start: windowStart,
    window_end: "2026-08-08T15:00:00Z",
    status,
    summary: windowStart,
  };
}

describe("native summary connection routing", () => {
  beforeEach(() => {
    useMobileStore.getState().reset();
    jest.mocked(writePaneWorkSummarySnapshot).mockClear();
  });

  it("accepts snapshots and incremental updates without creating activity events", () => {
    expect(handlePaneWorkSummaryMessage({
      type: "pane_work_summaries",
      session_id: sessionId,
      pane_id: 3,
      availability: "available",
      summaries: [summary("2026-08-08T09:00:00Z")],
    })).toBe(true);
    expect(handlePaneWorkSummaryMessage({
      type: "pane_work_summary_updated",
      session_id: sessionId,
      pane_id: 3,
      availability: "available",
      summary: summary("2026-08-08T12:00:00Z", "partial"),
    })).toBe(true);

    const state = useMobileStore.getState();
    expect(state.eventsBySession).toEqual({});
    expect(state.paneWorkSummaries[paneWorkSummaryKey(sessionId, 3)].summaries.map((item) => item.window_start)).toEqual([
      "2026-08-08T12:00:00Z",
      "2026-08-08T09:00:00Z",
    ]);
    expect(writePaneWorkSummarySnapshot).toHaveBeenCalledTimes(2);
  });

  it("re-requests exactly the visible pane after synchronization", () => {
    const send = jest.fn(() => true);
    useMobileStore.getState().setNegotiatedCapabilities(["pane_work_summary_v1"]);
    useMobileStore.getState().setVisibleSummaryPane({ sessionId, paneId: 7 });

    expect(reconcileVisiblePaneWorkSummaries(send)).toBe(true);
    expect(send).toHaveBeenCalledWith({
      type: "list_pane_work_summaries",
      session_id: sessionId,
      pane_id: 7,
      include_current: true,
    });
    expect(useMobileStore.getState().paneWorkSummaries[paneWorkSummaryKey(sessionId, 7)].loading).toBe(true);
  });

  it("does not reconcile an unsupported mixed-version connection", () => {
    const send = jest.fn(() => true);
    useMobileStore.getState().setVisibleSummaryPane({ sessionId, paneId: 7 });
    expect(reconcileVisiblePaneWorkSummaries(send)).toBe(false);
    expect(send).not.toHaveBeenCalled();
  });
});
