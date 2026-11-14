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

  // The server refuses a new conversation-only pane whether or not the request
  // carries the legacy `managed` marker, so the browser says so first. That
  // marker used to exempt the pane while team mode was available.
  it.each([
    ["without the managed marker", false, false],
    ["with the managed marker", true, false],
    ["with the managed marker and the retained team field set", true, true],
  ])("rejects a new conversation-only pane %s", (_name, managed, teamAvailable) => {
    const { ws, sent } = fakeSocket();
    useStore.setState({
      ws,
      sessionId: SID,
      isAttached: true,
      projectPolicies: { [SID]: { ...permissivePolicy, teamAvailable } },
    });

    const result = useStore
      .getState()
      .addPane("claude", "interactive", "Legacy", undefined, undefined, undefined, undefined, managed, "agent");

    expect(result.success).toBe(false);
    expect(result.error).toMatch(/Conversation-only panes are retired/);
    expect(sent).toHaveLength(0);
  });

  // `team_available` survives on the wire for older builds and decides
  // nothing: the same requests get the same answers whatever it says.
  it.each([false, true])("treats a tab marked managed like any other when teamAvailable is %s", (teamAvailable) => {
    const { ws, sent } = fakeSocket();
    const showToast = vi.fn();
    useStore.setState({
      ws,
      sessionId: SID,
      isAttached: true,
      showToast,
      projectPolicies: { [SID]: { ...permissivePolicy, teamAvailable } },
      paneConfigs: [
        {
          pane_id: PANE,
          provider: "codex",
          mode: "interactive",
          kind: "terminal",
          session_id: "conversation",
          is_paused: false,
          stop_requested: false,
          managed: true,
        },
      ],
    });

    useStore.getState().resumePane(PANE);
    useStore.getState().rebootPane(PANE);
    const added = useStore
      .getState()
      .addPane("codex", "interactive", "Codex 2", undefined, undefined, undefined, undefined, true, "terminal");

    expect(showToast).not.toHaveBeenCalled();
    expect(added.success).toBe(true);
    expect(sent.map((frame) => frame.type)).toEqual(["resume_pane", "reboot_pane", "add_pane"]);
  });

  it("refuses a Pi tab with the sentence that says what to use, and still adds an OMP tab", () => {
    const { ws, sent } = fakeSocket();
    useStore.setState({
      ws,
      sessionId: SID,
      isAttached: true,
      projectPolicies: {
        [SID]: {
          ...permissivePolicy,
          // A policy stored before the retirement can still list Pi.
          allowedLaunchProfiles: [
            "terminal:pi:official:default",
            "terminal:omp:official:default",
          ],
        },
      },
    });

    const refused = useStore
      .getState()
      .addPane("pi", "interactive", "Pi 1", undefined, undefined, undefined, undefined, false, "terminal");
    expect(refused).toEqual({ success: false, error: "Pi is no longer supported. Use OMP instead." });
    expect(sent).toHaveLength(0);

    const added = useStore
      .getState()
      .addPane("omp", "interactive", "OMP 1", undefined, undefined, undefined, undefined, false, "terminal");
    expect(added.success).toBe(true);
    expect(sent[0]).toMatchObject({ type: "add_pane", kind: "terminal", provider: "omp" });
  });

  it("sends nothing to a saved Pi tab, and still sends to an OMP tab", () => {
    const { ws, sent } = fakeSocket();
    const toasts: string[] = [];
    const tab = (pane_id: number, provider: "pi" | "omp") => ({
      pane_id,
      provider,
      mode: "interactive" as const,
      kind: "terminal" as const,
      session_id: `${provider}-${pane_id}`,
      is_paused: false,
      label: `${provider} ${pane_id}`,
    });
    useStore.setState({
      ws,
      sessionId: SID,
      isAttached: true,
      paneConfigs: [tab(77, "pi"), tab(78, "omp")],
      pendingTerminalDeliveries: [],
      projectPolicies: {
        [SID]: { ...permissivePolicy, allowedLaunchProfiles: ["terminal:omp:official:default"] },
      },
      showToast: (message: string) => { toasts.push(message); },
    });

    // The phone's composer and the conversation view both send this way. A
    // message to a tab with no agent would stay unconfirmed forever.
    expect(useStore.getState().sendTerminalConversationMessage(77, "hello pi")).toEqual({
      success: false,
      error: "Pi is no longer supported. Use OMP instead. This tab is read-only.",
    });
    useStore.getState().sendTerminalInput(77, "x");
    expect(sent).toHaveLength(0);
    expect(useStore.getState().pendingTerminalDeliveries).toHaveLength(0);

    // A Pi tab cannot become an OMP tab: the server and its CLI refuse too.
    useStore.getState().updatePaneModel(77, null, "omp");
    expect(sent).toHaveLength(0);
    expect(toasts).toEqual([
      "Tab 77 uses Pi, which is no longer supported. Close it and open an OMP tab.",
    ]);

    expect(useStore.getState().sendTerminalConversationMessage(78, "hello omp").success).toBe(true);
    useStore.getState().sendTerminalInput(78, "x");
    expect(sent.map((frame) => [frame.type, frame.pane_id])).toEqual([
      ["terminal_conversation_input", 78],
      ["terminal_input", 78],
    ]);
  });

  it("fails closed before policy arrives and rejects disallowed profiles", () => {
    const { ws, sent } = fakeSocket();
    useStore.setState({ ws, sessionId: SID, isAttached: true, projectPolicies: {} });
    const pending = useStore.getState().addPane("codex", "interactive", undefined, undefined, undefined, undefined, undefined, false, "terminal");
    expect(pending.success).toBe(false);
    expect(pending.error).toMatch(/authoritative cluster policy/);

    useStore.setState({
      projectPolicies: {
        [SID]: {
          ...permissivePolicy,
          allowedLaunchProfiles: ["terminal:claude:official:default"],
        },
      },
    });
    const denied = useStore.getState().addPane("codex", "interactive", undefined, undefined, undefined, undefined, undefined, false, "terminal");
    expect(denied.success).toBe(false);
    expect(denied.error).toMatch(/disabled by cluster policy v1/);
    expect(sent).toHaveLength(0);
  });
});
