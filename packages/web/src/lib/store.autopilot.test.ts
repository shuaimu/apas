import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { validateClientMessage, validateServerMessage } from "@apas/protocol";
import { useStore, type PaneConfig, type SessionInfo } from "./store";
import {
  autopilotControlModel,
  autopilotRequestNotice,
  paneAutopilotKey,
  TAB_AUTOPILOT_CAPABILITY,
  type AutopilotRequest,
} from "./autopilot";

/**
 * Autopilot in the store: what a tab reports, and requests that are sent once
 * and never again. These drive the real socket handler with the protocol's
 * golden frames, and check every frame the store sends against the schema the
 * server parses.
 */

const SID = "20000000-0000-4000-8000-000000000001";
const OTHER_SID = "20000000-0000-4000-8000-000000000002";
const AUTOPILOT = "60000000-0000-4000-8000-000000000001";

type MockWs = {
  send: Mock;
  onmessage?: (event: MessageEvent) => void;
  onclose?: (event: CloseEvent) => void;
};

function fixture(name: string): Record<string, unknown> {
  return JSON.parse(readFileSync(resolve(__dirname, "../../../protocol/fixtures", name), "utf8"));
}

async function openSocket(): Promise<MockWs> {
  localStorage.setItem("apas_token", "test-token");
  useStore.setState({ ws: null });
  useStore.getState().connect();
  await new Promise((done) => setTimeout(done, 10));
  return useStore.getState().ws as unknown as MockWs;
}

function deliver(ws: MockWs, message: Record<string, unknown>) {
  ws.onmessage?.(new MessageEvent("message", { data: JSON.stringify(message) }));
}

function sentFrames(ws: MockWs, type?: string): Record<string, unknown>[] {
  return ws.send.mock.calls
    .map(([raw]) => JSON.parse(String(raw)) as Record<string, unknown>)
    .filter((frame) => !type || frame.type === type);
}

function negotiate(capable = true) {
  useStore.setState({
    negotiatedCapabilities: new Set(capable ? [TAB_AUTOPILOT_CAPABILITY] : []),
  });
}

function entry(paneId: number, sessionId = SID) {
  return useStore.getState().paneAutopilot[paneAutopilotKey(sessionId, paneId)];
}

function requests(): AutopilotRequest[] {
  return Object.values(useStore.getState().autopilotRequests);
}

function ompTab(paneId: number): PaneConfig {
  return {
    pane_id: paneId,
    provider: "omp",
    mode: "interactive",
    kind: "terminal",
    session_id: `conversation-${paneId}`,
    is_paused: false,
    label: `OMP ${paneId}`,
  };
}

function receipt(requestId: string, outcome: string, reason?: string): Record<string, unknown> {
  return { type: "pane_autopilot_receipt", session_id: SID, pane_id: 4, request_id: requestId, outcome, reason };
}

const initialState = useStore.getState();

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  useStore.setState(initialState, true);
  useStore.setState({ sessionId: SID, isAttached: true, paneAutopilot: {}, autopilotRequests: {}, toasts: [] });
});

afterEach(() => {
  vi.useRealTimers();
  useStore.getState().disconnect();
});

describe("negotiation", () => {
  it("advertises tab_autopilot_v1 and knows whether the server negotiated it", async () => {
    const ws = await openSocket();
    const [authenticate] = sentFrames(ws, "authenticate");
    expect(authenticate.capabilities).toContain(TAB_AUTOPILOT_CAPABILITY);

    deliver(ws, { type: "authenticated", user_id: "u1", negotiated_capabilities: [TAB_AUTOPILOT_CAPABILITY] });
    expect(useStore.getState().negotiatedCapabilities.has(TAB_AUTOPILOT_CAPABILITY)).toBe(true);

    deliver(ws, { type: "authenticated", user_id: "u1", negotiated_capabilities: ["project_instances_v1"] });
    expect(useStore.getState().negotiatedCapabilities.has(TAB_AUTOPILOT_CAPABILITY)).toBe(false);
  });

  it("sends nothing to a server that did not negotiate autopilot", async () => {
    const ws = await openSocket();
    negotiate(false);
    const result = useStore.getState().controlPaneAutopilot(SID, 4, { kind: "stop", now: false });
    expect(result).toEqual({ success: false, error: "This APAS server does not support autopilot." });
    expect(sentFrames(ws, "control_pane_autopilot")).toEqual([]);
    expect(requests()).toEqual([]);
  });
});

describe("what a tab reports", () => {
  it("holds each tab's state and availability from pane_autopilot_state", async () => {
    const ws = await openSocket();
    negotiate();
    for (const name of [
      "server-pane-autopilot-state.json",
      "server-pane-autopilot-state-unavailable.json",
    ]) {
      const frame = fixture(name);
      expect(validateServerMessage(frame).valid).toBe(true);
      deliver(ws, frame);
    }

    expect(entry(4)).toMatchObject({ support: "supported", reportedAt: "2026-11-13T14:16:05Z" });
    expect(entry(4)?.state?.autopilot).toMatchObject({
      autopilot_id: AUTOPILOT,
      iterations: 2,
      condition: { kind: "waiting", detail: "next_iteration" },
    });
    expect(entry(5)).toEqual({ support: "instance_update_required", reportedAt: undefined });

    // The next report replaces the last: here the autopilot has ended.
    deliver(ws, fixture("server-pane-autopilot-state-ended.json"));
    expect(entry(4)?.state?.autopilot).toBeUndefined();
    expect(entry(4)?.state?.last_ended?.reason).toBe("conversation_changed");
  });

  it("reads a tab with nothing held as not reported, never as off", () => {
    negotiate();
    expect(entry(9)).toBeUndefined();
    const model = autopilotControlModel({ entry: entry(9), connected: true });
    expect(model.notReported).toBe(true);
    expect(model.canStart).toBe(false);
    expect(model.actions).toEqual(["stop", "stop_now"]);
  });

  it("keeps state for a session other than the one on screen", async () => {
    const ws = await openSocket();
    negotiate();
    deliver(ws, { ...fixture("server-pane-autopilot-state.json"), session_id: OTHER_SID });
    expect(entry(4, OTHER_SID)?.state?.autopilot?.autopilot_id).toBe(AUTOPILOT);
    expect(entry(4)).toBeUndefined();
  });

  it("updates the session list's block, which is what lists read", async () => {
    const ws = await openSocket();
    negotiate();
    const listed = fixture("server-sessions-autopilot.json");
    expect(validateServerMessage(listed).valid).toBe(true);
    deliver(ws, listed);
    const panes = () => useStore.getState().sessions.find((session) => session.id === SID)?.panes ?? [];
    expect(panes().find((pane) => pane.pane_id === 3)?.autopilot)
      .toMatchObject({ support: "supported", condition: "waiting", iteration_limit: 20 });
    expect(panes().find((pane) => pane.pane_id === 4)?.autopilot).toEqual({ support: "needs_agent_reboot" });

    deliver(ws, fixture("server-pane-autopilot-state.json"));
    const summary = panes().find((pane) => pane.pane_id === 4)?.autopilot;
    expect(summary).toMatchObject({
      support: "supported",
      condition: "waiting",
      detail: "next_iteration",
      next_iteration_at: "2026-11-13T14:30:00Z",
      iterations: 2,
      iteration_limit: 20,
      omp_goal: "paused",
    });
    // A list never learns the prompt.
    expect(JSON.stringify(summary)).not.toContain("Check the build");
    // The other tab's block is untouched.
    expect(panes().find((pane) => pane.pane_id === 3)?.autopilot?.condition).toBe("waiting");
  });

  it("prunes state and requests with the roster", async () => {
    const ws = await openSocket();
    negotiate();
    deliver(ws, fixture("server-pane-autopilot-state.json"));
    deliver(ws, fixture("server-pane-autopilot-state-unavailable.json"));
    deliver(ws, { ...fixture("server-pane-autopilot-state.json"), session_id: OTHER_SID });
    useStore.getState().controlPaneAutopilot(SID, 5, { kind: "stop", now: false });
    useStore.getState().controlPaneAutopilot(SID, 4, { kind: "stop", now: false });

    deliver(ws, { type: "pane_list", session_id: SID, panes: [ompTab(4)] });

    expect(entry(4)).toBeDefined();
    // Pane ids are reused: a later tab 5 must not inherit this one's state.
    expect(entry(5)).toBeUndefined();
    expect(requests().map((request) => request.paneId)).toEqual([4]);
    // Another session's roster is not this one's.
    expect(entry(4, OTHER_SID)).toBeDefined();

    // A roster for a session in the background prunes that session only.
    deliver(ws, { type: "pane_list", session_id: OTHER_SID, panes: [] });
    expect(entry(4, OTHER_SID)).toBeUndefined();
    expect(entry(4)).toBeDefined();
  });

  it("drops every state with the socket, so tabs read as not reported until it is replayed", async () => {
    const ws = await openSocket();
    negotiate();
    deliver(ws, fixture("server-pane-autopilot-state.json"));
    ws.onclose?.(new CloseEvent("close", { code: 1000 }));
    expect(useStore.getState().paneAutopilot).toEqual({});
  });
});

describe("requests", () => {
  it("sends each action once, as a frame the protocol accepts", async () => {
    const ws = await openSocket();
    negotiate();
    const { controlPaneAutopilot } = useStore.getState();
    const spec = {
      prompt: "Check the build and fix what fails.",
      repeat: { kind: "every" as const, seconds: 900 },
      iteration_limit: { kind: "at_most" as const, value: 20 },
      time_limit: { kind: "none" as const },
      end_without_tool_call: false,
    };
    expect(controlPaneAutopilot(SID, 4, { kind: "start", spec }).success).toBe(true);
    expect(controlPaneAutopilot(SID, 4, { kind: "pause", autopilot_id: AUTOPILOT }).success).toBe(true);
    expect(controlPaneAutopilot(SID, 4, { kind: "resume", autopilot_id: AUTOPILOT }).success).toBe(true);
    expect(controlPaneAutopilot(SID, 4, { kind: "stop", autopilot_id: AUTOPILOT, now: false }).success).toBe(true);
    expect(controlPaneAutopilot(SID, 4, { kind: "stop", now: true }).success).toBe(true);

    const frames = sentFrames(ws, "control_pane_autopilot");
    expect(frames).toHaveLength(5);
    for (const frame of frames) {
      const result = validateClientMessage(frame);
      expect(result.errors).toEqual([]);
      // Every request names its session and its tab.
      expect(frame).toMatchObject({ session_id: SID, pane_id: 4 });
      expect(typeof frame.request_id).toBe("string");
    }
    expect(new Set(frames.map((frame) => frame.request_id)).size).toBe(5);
    // The start is the golden frame, apart from its request id.
    expect(frames[0].action).toEqual(fixture("web-control-pane-autopilot-start.json").action);
    expect(frames[2].action).toEqual(fixture("web-control-pane-autopilot-resume.json").action);
    expect(frames[4].action).toEqual({ kind: "stop", now: true });

    expect(requests().map((request) => [request.kind, request.status])).toEqual([
      ["start", "pending"],
      ["pause", "pending"],
      ["resume", "pending"],
      ["stop", "pending"],
      ["stop_now", "pending"],
    ]);
  });

  it("queues nothing while the page is offline", () => {
    negotiate();
    useStore.setState({ ws: null });
    const result = useStore.getState().controlPaneAutopilot(SID, 4, { kind: "stop", now: true });
    expect(result).toEqual({ success: false, error: "Not connected to the server." });
    expect(requests()).toEqual([]);
  });

  it("keeps an accepted request until the tab reports, and changes no state by itself", async () => {
    const ws = await openSocket();
    negotiate();
    deliver(ws, fixture("server-pane-autopilot-state.json"));
    const { requestId } = useStore.getState().controlPaneAutopilot(SID, 4, { kind: "stop", now: false });

    deliver(ws, receipt(requestId!, "accepted"));
    expect(requests()).toMatchObject([{ requestId, status: "accepted" }]);
    // Accepted means the tab took the request. What is displayed is still
    // what the tab last reported.
    expect(entry(4)?.state?.autopilot?.condition.kind).toBe("waiting");

    deliver(ws, fixture("server-pane-autopilot-state-ended.json"));
    expect(requests()).toEqual([]);
    expect(entry(4)?.state?.last_ended).toBeDefined();
  });

  it("forgets an accepted request by itself when the tab's report is slow", async () => {
    const ws = await openSocket();
    negotiate();
    vi.useFakeTimers();
    const { requestId } = useStore.getState().controlPaneAutopilot(SID, 4, { kind: "pause" });
    deliver(ws, receipt(requestId!, "accepted"));
    expect(requests()).toHaveLength(1);
    vi.advanceTimersByTime(6_000);
    expect(requests()).toEqual([]);
  });

  it("keeps a refusal with its reason until it is dismissed", async () => {
    const ws = await openSocket();
    negotiate();
    const { requestId } = useStore.getState().controlPaneAutopilot(SID, 4, {
      kind: "start",
      spec: { prompt: "go", repeat: { kind: "after_each_turn" }, iteration_limit: { kind: "none" }, time_limit: { kind: "none" } },
    });
    const refused = { ...fixture("server-pane-autopilot-receipt.json"), request_id: requestId };
    expect(validateServerMessage(refused).valid).toBe(true);
    deliver(ws, refused);

    expect(requests()).toMatchObject([{
      requestId,
      status: "refused",
      reason: "This tab is already on autopilot. Stop it first.",
    }]);
    expect(useStore.getState().toasts.map((toast) => toast.message))
      .toContain("This tab is already on autopilot. Stop it first.");

    useStore.getState().dismissAutopilotRequest(requestId!);
    expect(requests()).toEqual([]);
  });

  it("shows a request that was not confirmed and never sends it again", async () => {
    const ws = await openSocket();
    negotiate();
    const { requestId } = useStore.getState().controlPaneAutopilot(SID, 4, { kind: "stop", now: false });
    deliver(ws, receipt(requestId!, "uncertain"));

    expect(requests()).toMatchObject([{ requestId, status: "unconfirmed", cause: "uncertain" }]);
    expect(autopilotRequestNotice(requests()[0]).detail).toContain("It was not sent again.");
    expect(sentFrames(ws, "control_pane_autopilot")).toHaveLength(1);

    // It stays visible across later reports and receipts for other requests,
    // and nothing sends it again.
    deliver(ws, fixture("server-pane-autopilot-state.json"));
    deliver(ws, receipt("someone-else", "accepted"));
    expect(requests()).toMatchObject([{ requestId, status: "unconfirmed" }]);
    expect(sentFrames(ws, "control_pane_autopilot")).toHaveLength(1);
  });

  it("reports a request as not confirmed when no receipt arrives", async () => {
    const ws = await openSocket();
    negotiate();
    vi.useFakeTimers();
    const { requestId } = useStore.getState().controlPaneAutopilot(SID, 4, { kind: "stop", now: true });
    vi.advanceTimersByTime(15_000);
    expect(requests()).toMatchObject([{ requestId, kind: "stop_now", status: "unconfirmed", cause: "timeout" }]);
    expect(sentFrames(ws, "control_pane_autopilot")).toHaveLength(1);

    // A receipt that arrives late still says what happened.
    deliver(ws, receipt(requestId!, "refused", "zoo-005 is not running this project. Open the project again to start it there."));
    expect(requests()).toMatchObject([{
      status: "refused",
      reason: "zoo-005 is not running this project. Open the project again to start it there.",
    }]);
  });

  it("does not resend a request whose connection dropped, on that socket or the next", async () => {
    const first = await openSocket();
    negotiate();
    const { requestId } = useStore.getState().controlPaneAutopilot(SID, 4, { kind: "pause" });
    first.onclose?.(new CloseEvent("close", { code: 1000 }));
    expect(requests()).toMatchObject([{ requestId, status: "unconfirmed", cause: "connection" }]);

    const second = await openSocket();
    deliver(second, { type: "authenticated", user_id: "u1", negotiated_capabilities: [TAB_AUTOPILOT_CAPABILITY] });
    deliver(second, { type: "session_attached", session_id: SID, has_active_cli: true });
    expect(sentFrames(second, "control_pane_autopilot")).toEqual([]);
    expect(requests()).toMatchObject([{ requestId, status: "unconfirmed" }]);
  });

  it("ignores a receipt for another tab or a request it never sent", async () => {
    const ws = await openSocket();
    negotiate();
    const { requestId } = useStore.getState().controlPaneAutopilot(SID, 4, { kind: "pause" });
    deliver(ws, { ...receipt(requestId!, "refused", "no"), pane_id: 5 });
    deliver(ws, receipt("unknown", "refused", "no"));
    expect(requests()).toMatchObject([{ requestId, status: "pending" }]);
  });

  it("forgets states and requests with their project", async () => {
    const ws = await openSocket();
    negotiate();
    const session: SessionInfo = { id: SID, projectId: "project-1", status: "active" };
    useStore.setState({ sessions: [session] });
    deliver(ws, fixture("server-pane-autopilot-state.json"));
    useStore.getState().controlPaneAutopilot(SID, 4, { kind: "pause" });
    useStore.getState().forgetProject("project-1");
    expect(useStore.getState().paneAutopilot).toEqual({});
    expect(requests()).toEqual([]);
  });
});
