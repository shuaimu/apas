import type { MobileBootstrapResponse, PaneWorkSummary } from "@apas/protocol";

import {
  paneWorkSummariesSupported,
  paneWorkSummaryKey,
  useMobileStore,
} from "./store";

const sessionId = "58dbd62d-c40f-4b5b-a1d4-96aca52ea595";

function summary(paneId: number, windowStart: string, status: PaneWorkSummary["status"] = "complete"): PaneWorkSummary {
  return {
    session_id: sessionId,
    pane_id: paneId,
    window_start: windowStart,
    window_end: "2026-08-08T15:00:00Z",
    status,
    summary: `${paneId}:${windowStart}`,
  };
}

describe("native pane work summary state", () => {
  beforeEach(() => useMobileStore.getState().reset());

  it("retains exact negotiated support", () => {
    useMobileStore.getState().setNegotiatedCapabilities(["terminal"]);
    expect(paneWorkSummariesSupported()).toBe(false);
    useMobileStore.getState().setNegotiatedCapabilities(["terminal", "pane_work_summary_v1"]);
    expect(paneWorkSummariesSupported()).toBe(true);
  });

  it("replaces authoritative snapshots and upserts windows newest first without sibling leakage", () => {
    const state = useMobileStore.getState();
    state.replacePaneWorkSummaries(sessionId, 3, [
      summary(3, "2026-08-08T09:00:00Z"),
      summary(4, "2026-08-08T12:00:00Z"),
      summary(3, "2026-08-08T12:00:00Z", "failed"),
    ], "available", "2026-08-08T15:01:00Z");
    state.upsertPaneWorkSummary(sessionId, 3, summary(3, "2026-08-08T12:00:00Z", "complete"));

    const cache = useMobileStore.getState().paneWorkSummaries[paneWorkSummaryKey(sessionId, 3)];
    expect(cache.summaries.map((item) => [item.pane_id, item.window_start, item.status])).toEqual([
      [3, "2026-08-08T12:00:00Z", "complete"],
      [3, "2026-08-08T09:00:00Z", "complete"],
    ]);
    expect(useMobileStore.getState().paneWorkSummaries[paneWorkSummaryKey(sessionId, 4)]).toBeUndefined();
  });

  it("does not let a late disk hydration replace a newer socket snapshot", () => {
    const state = useMobileStore.getState();
    state.replacePaneWorkSummaries(sessionId, 3, [summary(3, "2026-08-08T12:00:00Z")], "available", "2026-08-08T15:02:00Z");
    state.hydratePaneWorkSummaries(sessionId, 3, [summary(3, "2026-08-08T09:00:00Z")], "unknown", "2026-08-08T15:01:00Z");
    expect(useMobileStore.getState().paneWorkSummaries[paneWorkSummaryKey(sessionId, 3)].summaries[0].window_start)
      .toBe("2026-08-08T12:00:00Z");
  });

  it("removes inaccessible summary state and an open stale pane during bootstrap", () => {
    useMobileStore.getState().replacePaneWorkSummaries(sessionId, 3, [summary(3, "2026-08-08T12:00:00Z")]);
    useMobileStore.getState().setVisibleSummaryPane({ sessionId, paneId: 3 });
    useMobileStore.getState().applyBootstrap({
      user_id: "1027ac43-3c54-467a-90e6-e50a170d4882",
      user_email: "mobile@example.test",
      cluster_role: "user",
      account_status: "active",
      protocol_min_version: 1,
      protocol_max_version: 1,
      features: {},
      sessions: [],
      machines: [],
      launch_targets: [],
    } satisfies MobileBootstrapResponse);

    expect(useMobileStore.getState().paneWorkSummaries).toEqual({});
    expect(useMobileStore.getState().visibleSummaryPane).toBeNull();
  });

  it("moves sessions between working, idle, and offline from live status messages", () => {
    useMobileStore.setState({
      sessions: [{
        id: sessionId,
        status: "connected",
        is_active: true,
        is_working: false,
      }],
    });
    useMobileStore.getState().setPaneStatus(sessionId, 3, "Working…");
    expect(useMobileStore.getState().sessions[0]).toMatchObject({ is_active: true, is_working: true });

    useMobileStore.getState().setPaneStatus(sessionId, 3, null);
    expect(useMobileStore.getState().sessions[0]).toMatchObject({ is_active: true, is_working: false });

    useMobileStore.getState().setSessionActive(sessionId, false);
    expect(useMobileStore.getState().sessions[0]).toMatchObject({ is_active: false, is_working: false });
    expect(useMobileStore.getState().paneStatusesBySession[sessionId]).toBeUndefined();
  });
});
