import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  handleServerMessage,
  paneWorkSummaryKey,
  useStore,
} from "./store";

const SID = "11111111-1111-4111-8111-111111111111";
const PANE = 7;
const initialStore = useStore.getInitialState();

function socket() {
  const sent: Record<string, unknown>[] = [];
  return {
    sent,
    ws: {
      readyState: WebSocket.OPEN,
      send: (raw: string) => sent.push(JSON.parse(raw)),
    } as unknown as WebSocket,
  };
}

beforeEach(() => {
  useStore.setState(initialStore, true);
});

describe("pane work summary store", () => {
  it("gates requests on negotiated capability and throttles duplicates", () => {
    const { ws, sent } = socket();
    useStore.setState({ ws, negotiatedCapabilities: new Set() });
    expect(useStore.getState().listPaneWorkSummaries(SID, PANE)).toBe(false);
    expect(sent).toHaveLength(0);

    useStore.setState({ negotiatedCapabilities: new Set(["pane_work_summary_v1"]) });
    expect(useStore.getState().listPaneWorkSummaries(SID, PANE)).toBe(true);
    expect(useStore.getState().listPaneWorkSummaries(SID, PANE)).toBe(false);
    expect(sent).toEqual([{
      type: "list_pane_work_summaries",
      session_id: SID,
      pane_id: PANE,
      include_current: true,
    }]);
  });

  it("replaces snapshots and merges pane-scoped incremental updates", () => {
    handleServerMessage({
      type: "pane_work_summaries",
      session_id: SID,
      pane_id: PANE,
      availability: "available",
      summaries: [{
        protocol_version: 1,
        session_id: SID,
        pane_id: PANE,
        window_start: "2026-08-11T03:00:00Z",
        window_end: "2026-08-11T06:00:00Z",
        window_kind: "completed",
        status: "queued",
        source_digest: "one",
        source_message_count: 3,
        attempts: 0,
      }],
    }, useStore.setState, useStore.getState);

    handleServerMessage({
      type: "pane_work_summary_updated",
      session_id: SID,
      pane_id: PANE,
      availability: "available",
      summary: {
        protocol_version: 1,
        session_id: SID,
        pane_id: PANE,
        window_start: "2026-08-11T03:00:00Z",
        window_end: "2026-08-11T06:00:00Z",
        window_kind: "completed",
        status: "complete",
        summary: "Implemented and verified the requested work.",
        source_digest: "one",
        source_message_count: 3,
        attempts: 1,
      },
    }, useStore.setState, useStore.getState);

    const cache = useStore.getState().paneWorkSummaries[paneWorkSummaryKey(SID, PANE)];
    expect(cache.summaries).toHaveLength(1);
    expect(cache.summaries[0]).toMatchObject({
      status: "complete",
      summary: "Implemented and verified the requested work.",
    });
  });

  it("clears summary state on logout", () => {
    const close = vi.fn();
    useStore.setState({
      ws: { close } as unknown as WebSocket,
      paneWorkSummaries: {
        [paneWorkSummaryKey(SID, PANE)]: {
          summaries: [],
          availability: "available",
          loading: false,
        },
      },
      negotiatedCapabilities: new Set(["pane_work_summary_v1"]),
    });
    useStore.getState().logout();
    expect(useStore.getState().paneWorkSummaries).toEqual({});
    expect(useStore.getState().negotiatedCapabilities.size).toBe(0);
  });
});
