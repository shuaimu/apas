import { describe, expect, it } from "vitest";

import {
  clearTerminalForAccessLoss,
  initialTerminalState,
  markTerminalTransportDisconnected,
  reconcileTerminal,
} from "./terminal.js";

const session = "58dbd62d-c40f-4b5b-a1d4-96aca52ea595";
const instance = "14a1094b-6862-43ec-81ae-d38069927aa3";

describe("terminal reconciliation", () => {
  it("accepts a snapshot before contiguous live output", () => {
    const snapshot = reconcileTerminal(initialTerminalState(), { type: "terminal_snapshot", session_id: session, pane_id: 2, instance_id: instance, seq: 10, data_b64: "YQ==", lifecycle: "running" });
    const output = reconcileTerminal(snapshot.state, { type: "terminal_output", session_id: session, pane_id: 2, instance_id: instance, seq: 11, data_b64: "Yg==" });
    expect(snapshot.action).toBe("snapshot");
    expect(output.action).toBe("output");
    expect(output.state.sequence).toBe(11);
  });

  it("ignores duplicates and requests a snapshot for gaps or restarts", () => {
    const ready = reconcileTerminal(initialTerminalState(), { type: "terminal_snapshot", session_id: session, pane_id: 2, instance_id: instance, seq: 4, data_b64: "", lifecycle: "running" }).state;
    expect(reconcileTerminal(ready, { type: "terminal_output", session_id: session, pane_id: 2, instance_id: instance, seq: 4, data_b64: "" }).action).toBe("ignore");
    expect(reconcileTerminal(ready, { type: "terminal_output", session_id: session, pane_id: 2, instance_id: instance, seq: 6, data_b64: "" }).state.needsSnapshot).toBe(true);
    expect(reconcileTerminal(ready, { type: "terminal_state", session_id: session, pane_id: 2, instance_id: "a0ff1cda-67d1-48cd-8ada-504704c25242", lifecycle: "running" }).action).toBe("reset");
  });

  it("retains lifecycle on disconnect but clears state on access loss", () => {
    const state = { ...initialTerminalState(), lifecycle: "running" as const, needsSnapshot: false };
    expect(markTerminalTransportDisconnected(state).lifecycle).toBe("disconnected");
    const cleared = clearTerminalForAccessLoss(state);
    expect(cleared.lifecycle).toBe("unknown");
    expect(cleared.generation).toBe(1);
  });
});
