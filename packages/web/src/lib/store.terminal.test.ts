// Terminal-pane wire contract: server frames land on the terminalBus (not
// in zustand state), and every outbound control carries session_id so the
// server can resolve the right target. Getting the latter wrong is how
// pane controls silently misrouted on mobile before.
import { describe, it, expect, beforeEach, vi } from "vitest";
import { useStore, handleServerMessage } from "./store";
import {
  subscribeTerminal,
  __resetTerminalBus,
  encodeBase64,
  type TerminalEvent,
} from "./terminalBus";

const SID = "11111111-1111-4111-8111-111111111111";
const PANE = 7;

function fakeSocket() {
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
  __resetTerminalBus();
  useStore.setState({ ws: null, sessionId: null, projectPolicies: {} });
});

const permissivePolicy = {
  teamAvailable: true,
  allowedLaunchProfiles: [
    "terminal:codex:official:default",
    "agent:claude:official:default",
  ],
  version: 1,
  projectSuspended: false,
  noncompliantPaneIds: [],
};

describe("terminal server frames", () => {
  it("routes terminal_output to the bus, not into store state", () => {
    const seen: TerminalEvent[] = [];
    subscribeTerminal(PANE, (e) => seen.push(e));
    const before = useStore.getState();

    handleServerMessage(
      { type: "terminal_output", session_id: SID, pane_id: PANE, data_b64: "aGk=", seq: 4 },
      useStore.setState,
      useStore.getState,
    );

    expect(seen).toEqual([
      { kind: "output", bytes: new Uint8Array([0x68, 0x69]), seq: 4, instanceId: undefined },
    ]);
    // The store must be untouched — a pty repaints many times a second and
    // storing frames would re-render every subscriber on each one.
    expect(useStore.getState()).toBe(before);
  });

  it("routes terminal_snapshot with instance and lifecycle metadata", () => {
    const seen: TerminalEvent[] = [];
    subscribeTerminal(PANE, (e) => seen.push(e));

    handleServerMessage(
      {
        type: "terminal_snapshot",
        session_id: SID,
        pane_id: PANE,
        data_b64: "aGk=",
        seq: 9,
        truncated: true,
        instance_id: "pty-a",
        lifecycle: "disconnected",
        status: "network unavailable",
      },
      useStore.setState,
      useStore.getState,
    );

    expect(seen[0]).toMatchObject({
      kind: "snapshot",
      seq: 9,
      truncated: true,
      instanceId: "pty-a",
      lifecycle: "disconnected",
      status: "network unavailable",
    });
  });

  it("defaults snapshot truncated to false when the server omits it", () => {
    const seen: TerminalEvent[] = [];
    subscribeTerminal(PANE, (e) => seen.push(e));

    handleServerMessage(
      { type: "terminal_snapshot", session_id: SID, pane_id: PANE, data_b64: "", seq: 0 },
      useStore.setState,
      useStore.getState,
    );

    expect(seen[0]).toMatchObject({
      kind: "snapshot",
      truncated: false,
      lifecycle: "unknown",
      instanceId: undefined,
    });
  });

  it("routes terminal_exited with its status", () => {
    const seen: TerminalEvent[] = [];
    subscribeTerminal(PANE, (e) => seen.push(e));

    handleServerMessage(
      { type: "terminal_exited", session_id: SID, pane_id: PANE, status: "exited with status 0" },
      useStore.setState,
      useStore.getState,
    );

    expect(seen[0]).toEqual({
      kind: "exited",
      instanceId: undefined,
      status: "exited with status 0",
    });
  });

  it("routes lifecycle-only state events and defaults invalid state to unknown", () => {
    const seen: TerminalEvent[] = [];
    subscribeTerminal(PANE, (e) => seen.push(e));

    handleServerMessage(
      {
        type: "terminal_state",
        session_id: SID,
        pane_id: PANE,
        instance_id: "pty-a",
        lifecycle: "running",
      },
      useStore.setState,
      useStore.getState,
    );
    handleServerMessage(
      { type: "terminal_state", session_id: SID, pane_id: PANE, lifecycle: "future_state" },
      useStore.setState,
      useStore.getState,
    );

    expect(seen).toEqual([
      { kind: "state", instanceId: "pty-a", lifecycle: "running", status: undefined },
      { kind: "state", instanceId: undefined, lifecycle: "unknown", status: undefined },
    ]);
  });

  it("ignores frames missing pane_id rather than throwing", () => {
    expect(() =>
      handleServerMessage(
        { type: "terminal_output", session_id: SID, data_b64: "aGk=", seq: 1 },
        useStore.setState,
        useStore.getState,
      ),
    ).not.toThrow();
  });
});

describe("terminal outbound controls", () => {
  it("attach/input/resize all carry session_id and pane_id", () => {
    const { ws, sent } = fakeSocket();
    useStore.setState({ ws, sessionId: SID });

    useStore.getState().attachTerminal(PANE);
    useStore.getState().sendTerminalInput(PANE, "ls\r");
    useStore.getState().sendTerminalResize(PANE, 120, 40);

    expect(sent).toHaveLength(3);
    for (const frame of sent) {
      expect(frame.session_id).toBe(SID);
      expect(frame.pane_id).toBe(PANE);
    }
    expect(sent.map((f) => f.type)).toEqual([
      "terminal_attach",
      "terminal_input",
      "terminal_resize",
    ]);
  });

  it("base64-encodes keystrokes including control bytes", () => {
    const { ws, sent } = fakeSocket();
    useStore.setState({ ws, sessionId: SID });

    // Ctrl-C — the single most important byte to deliver intact.
    useStore.getState().sendTerminalInput(PANE, "\x03");

    expect(sent[0].data_b64).toBe(encodeBase64(new Uint8Array([0x03])));
  });

  it("sends nothing when there is no session or socket", () => {
    const { ws, sent } = fakeSocket();

    useStore.setState({ ws, sessionId: null });
    useStore.getState().attachTerminal(PANE);
    expect(sent).toHaveLength(0);

    useStore.setState({ ws: null, sessionId: SID });
    expect(() => useStore.getState().sendTerminalInput(PANE, "x")).not.toThrow();
    expect(sent).toHaveLength(0);
  });

  it("addPane forces a terminal pane to be unmanaged", () => {
    const { ws, sent } = fakeSocket();
    useStore.setState({
      ws,
      sessionId: SID,
      isAttached: true,
      projectPolicies: { [SID]: permissivePolicy },
    });

    // Terminal panes publish no stream events, so the Tech Lead must never
    // treat one as a delegation target even if a caller asks for managed.
    const result = useStore
      .getState()
      .addPane("codex", "interactive", "Codex TTY 2", undefined, undefined, undefined, undefined, true, "terminal");

    expect(result.success).toBe(true);
    expect(sent[0]).toMatchObject({ type: "add_pane", kind: "terminal", managed: false });
  });

  it("addPane defaults to an agent pane and preserves managed", () => {
    const { ws, sent } = fakeSocket();
    useStore.setState({
      ws,
      sessionId: SID,
      isAttached: true,
      projectPolicies: { [SID]: permissivePolicy },
    });

    useStore.getState().addPane("claude", "interactive", "Claude 2", undefined, undefined, undefined, undefined, true);

    expect(sent[0]).toMatchObject({ kind: "agent", managed: true });
  });

  it("fails closed before policy arrives and rejects disallowed profiles", () => {
    const { ws, sent } = fakeSocket();
    useStore.setState({ ws, sessionId: SID, isAttached: true, projectPolicies: {} });
    const pending = useStore.getState().addPane("codex", "interactive");
    expect(pending.success).toBe(false);
    expect(pending.error).toMatch(/authoritative cluster policy/);

    useStore.setState({
      projectPolicies: {
        [SID]: {
          ...permissivePolicy,
          allowedLaunchProfiles: ["agent:claude:official:default"],
        },
      },
    });
    const denied = useStore.getState().addPane("codex", "interactive");
    expect(denied.success).toBe(false);
    expect(denied.error).toMatch(/disabled by cluster policy v1/);
    expect(sent).toHaveLength(0);
  });
});
