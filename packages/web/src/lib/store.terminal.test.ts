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
  useStore.setState({ ws: null, sessionId: null });
});

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
      { kind: "output", bytes: new Uint8Array([0x68, 0x69]), seq: 4 },
    ]);
    // The store must be untouched — a pty repaints many times a second and
    // storing frames would re-render every subscriber on each one.
    expect(useStore.getState()).toBe(before);
  });

  it("routes terminal_snapshot with its truncated flag", () => {
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
      },
      useStore.setState,
      useStore.getState,
    );

    expect(seen[0]).toMatchObject({ kind: "snapshot", seq: 9, truncated: true });
  });

  it("defaults snapshot truncated to false when the server omits it", () => {
    const seen: TerminalEvent[] = [];
    subscribeTerminal(PANE, (e) => seen.push(e));

    handleServerMessage(
      { type: "terminal_snapshot", session_id: SID, pane_id: PANE, data_b64: "", seq: 0 },
      useStore.setState,
      useStore.getState,
    );

    expect(seen[0]).toMatchObject({ kind: "snapshot", truncated: false });
  });

  it("routes terminal_exited with its status", () => {
    const seen: TerminalEvent[] = [];
    subscribeTerminal(PANE, (e) => seen.push(e));

    handleServerMessage(
      { type: "terminal_exited", session_id: SID, pane_id: PANE, status: "exited with status 0" },
      useStore.setState,
      useStore.getState,
    );

    expect(seen[0]).toEqual({ kind: "exited", status: "exited with status 0" });
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
    useStore.setState({ ws, sessionId: SID, isAttached: true });

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
    useStore.setState({ ws, sessionId: SID, isAttached: true });

    useStore.getState().addPane("claude", "interactive", "Claude 2", undefined, undefined, undefined, undefined, true);

    expect(sent[0]).toMatchObject({ kind: "agent", managed: true });
  });
});
