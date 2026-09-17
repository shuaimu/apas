import { beforeEach, describe, expect, it } from "vitest";
import { paneKey, useStore, type PaneConfig } from "./store";

/**
 * Closing the last pane of a project.
 *
 * `paneConfigs.length === 0` cannot say whether a roster arrived and is empty
 * or has not arrived at all, and the two need opposite handling: before one
 * arrives the UI synthesizes tabs from message history, whereas an authoritative
 * empty roster means the project genuinely has no panes. `paneListReceived` is
 * what separates them.
 */

const SESSION = "session-roster";

function pane(pane_id: number): PaneConfig {
  return {
    pane_id,
    provider: "claude",
    mode: "interactive",
    session_id: `pane-${pane_id}`,
  } as PaneConfig;
}

/// Drive the real websocket handler, the way the other store tests do —
/// `handleServerMessage` closes over the store's get/set and is not callable
/// on its own.
async function connect() {
  // `connect()` bails without a stored token.
  localStorage.setItem("apas_token", "test-token");
  useStore.getState().connect();
  await new Promise((resolve) => setTimeout(resolve, 10));
  useStore.setState({ isAuthenticated: true, sessionId: SESSION });
}

function sendRoster(panes: PaneConfig[]) {
  const ws = useStore.getState().ws as unknown as {
    onmessage?: (event: MessageEvent) => void;
  };
  ws.onmessage?.(
    new MessageEvent("message", {
      data: JSON.stringify({ type: "pane_list", session_id: SESSION, panes }),
    }),
  );
}

describe("authoritative pane roster", () => {
  beforeEach(async () => {
    useStore.setState({
      connected: false,
      isAuthenticated: false,
      sessionId: SESSION,
      ws: null,
      paneConfigs: [],
      paneListReceived: false,
      paneMessages: {},
      paneHasMore: {},
      paneStatuses: {},
      paneModes: {},
      pausedPanes: [],
      pendingLabels: [],
    });
    await connect();
  });

  it("starts out not having received a roster", () => {
    expect(useStore.getState().paneListReceived).toBe(false);
  });

  it("records an empty roster as received, not as silence", () => {
    sendRoster([]);

    const state = useStore.getState();
    expect(state.paneConfigs).toEqual([]);
    // The whole point: an empty list is an answer. Without this the UI cannot
    // tell a project with no panes from one whose CLI has not reported yet,
    // and rebuilds the closed pane as a ghost tab from its leftover messages.
    expect(state.paneListReceived).toBe(true);
  });

  it("forgets a closed pane's messages and status, and keeps the survivors'", () => {
    sendRoster([pane(7), pane(9)]);
    useStore.setState({
      paneMessages: {
        [paneKey(7)]: [{ id: "m7", type: "assistant", content: "seven" }],
        [paneKey(9)]: [{ id: "m9", type: "assistant", content: "nine" }],
      } as never,
      paneStatuses: { [paneKey(7)]: "Working…", [paneKey(9)]: "Working…" },
      paneHasMore: { [paneKey(7)]: true, [paneKey(9)]: true },
    });

    // Close pane 7.
    sendRoster([pane(9)]);

    const state = useStore.getState();
    expect(state.paneMessages[paneKey(7)]).toBeUndefined();
    expect(state.paneStatuses[paneKey(7)]).toBeUndefined();
    expect(state.paneHasMore[paneKey(7)]).toBeUndefined();
    // A roster broadcast fires on every start/stop transition, so a surviving
    // pane's conversation must never be disturbed by one.
    expect(state.paneMessages[paneKey(9)]).toHaveLength(1);
    expect(state.paneStatuses[paneKey(9)]).toBe("Working…");
  });

  it("leaves nothing behind when the last pane is closed", () => {
    sendRoster([pane(7)]);
    useStore.setState({
      paneMessages: {
        [paneKey(7)]: [{ id: "m7", type: "assistant", content: "seven" }],
      } as never,
      paneStatuses: { [paneKey(7)]: "Working…" },
    });

    sendRoster([]);

    const state = useStore.getState();
    expect(state.paneConfigs).toEqual([]);
    expect(state.paneListReceived).toBe(true);
    // Leftovers here are what tab synthesis unions into a ghost tab.
    expect(Object.keys(state.paneMessages)).toEqual([]);
    expect(Object.keys(state.paneStatuses)).toEqual([]);
  });

  it("does not churn identity when the roster is unchanged", () => {
    sendRoster([pane(7)]);
    const messages = { [paneKey(7)]: [] };
    useStore.setState({ paneMessages: messages as never });

    sendRoster([pane(7)]);

    // Subscribers compare by reference; a roster broadcast that drops nothing
    // must not look like a change to every pane's message list.
    expect(useStore.getState().paneMessages).toBe(messages);
  });
});
