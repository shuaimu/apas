import { describe, expect, it } from "vitest";
import type { TerminalEvent } from "./terminalBus";
import {
  applyTerminalEvent,
  createTerminalRenderState,
  terminalLifecycleBanner,
} from "./terminalReconciler";

function harness() {
  const writes: number[][] = [];
  let resets = 0;
  return {
    writes,
    get resets() {
      return resets;
    },
    sink: {
      write: (bytes: Uint8Array) => writes.push(Array.from(bytes)),
      reset: () => {
        resets += 1;
      },
    },
  };
}

const bytes = (...values: number[]) => new Uint8Array(values);

describe("terminal instance and snapshot reconciliation", () => {
  it("renders a legacy snapshot and retains unknown lifecycle", () => {
    const state = createTerminalRenderState();
    const io = harness();
    applyTerminalEvent(
      state,
      {
        kind: "snapshot",
        bytes: bytes(1, 2),
        seq: 0,
        truncated: false,
        lifecycle: "unknown",
      },
      io.sink,
    );

    expect(io.writes).toEqual([[1, 2]]);
    expect(state.lifecycle).toBe("unknown");
    expect(state.currentInstanceId).toBeUndefined();
  });

  it("does not duplicate an already-rendered same-instance snapshot", () => {
    const state = createTerminalRenderState();
    const io = harness();
    const snapshot: TerminalEvent = {
      kind: "snapshot",
      bytes: bytes(1, 2),
      seq: 4,
      truncated: false,
      instanceId: "pty-a",
      lifecycle: "running",
    };
    applyTerminalEvent(state, snapshot, io.sink);
    state.snapshotSeen = false;
    applyTerminalEvent(state, { ...snapshot, lifecycle: "disconnected" }, io.sink);

    expect(io.writes).toEqual([[1, 2]]);
    expect(io.resets).toBe(0);
    expect(state.lifecycle).toBe("disconnected");
  });

  it("resets and cumulatively replays when the browser missed frames", () => {
    const state = createTerminalRenderState();
    const io = harness();
    applyTerminalEvent(
      state,
      {
        kind: "snapshot",
        bytes: bytes(1),
        seq: 1,
        truncated: false,
        instanceId: "pty-a",
        lifecycle: "running",
      },
      io.sink,
    );
    state.snapshotSeen = false;
    applyTerminalEvent(
      state,
      {
        kind: "snapshot",
        bytes: bytes(1, 2, 3),
        seq: 3,
        truncated: false,
        instanceId: "pty-a",
        lifecycle: "running",
      },
      io.sink,
    );

    expect(io.resets).toBe(1);
    expect(io.writes).toEqual([[1], [1, 2, 3]]);
    expect(state.lastRenderedSeq).toBe(3);
  });

  it("resets presentation for a replacement instance", () => {
    const state = createTerminalRenderState();
    const io = harness();
    applyTerminalEvent(
      state,
      {
        kind: "snapshot",
        bytes: bytes(1),
        seq: 8,
        truncated: false,
        instanceId: "pty-old",
        lifecycle: "disconnected",
      },
      io.sink,
    );
    applyTerminalEvent(
      state,
      { kind: "state", instanceId: "pty-new", lifecycle: "running" },
      io.sink,
    );
    applyTerminalEvent(
      state,
      { kind: "output", instanceId: "pty-new", bytes: bytes(9), seq: 0 },
      io.sink,
    );

    expect(io.resets).toBe(1);
    expect(io.writes).toEqual([[1], [9]]);
    expect(state.currentInstanceId).toBe("pty-new");
    expect(state.lastRenderedSeq).toBe(0);
  });

  it("ignores delayed output and exit from a replaced instance", () => {
    const state = createTerminalRenderState();
    const io = harness();
    applyTerminalEvent(
      state,
      { kind: "state", instanceId: "pty-new", lifecycle: "running" },
      io.sink,
    );
    state.snapshotSeen = true;

    expect(
      applyTerminalEvent(
        state,
        { kind: "output", instanceId: "pty-old", bytes: bytes(7), seq: 9 },
        io.sink,
      ),
    ).toBe(false);
    expect(
      applyTerminalEvent(
        state,
        { kind: "exited", instanceId: "pty-old", status: "late" },
        io.sink,
      ),
    ).toBe(false);
    expect(io.writes).toEqual([]);
    expect(state.lifecycle).toBe("running");
  });

  it("drops pending frames covered by a snapshot and flushes only its tail", () => {
    const state = createTerminalRenderState();
    const io = harness();
    applyTerminalEvent(
      state,
      { kind: "output", instanceId: "pty-a", bytes: bytes(2), seq: 2 },
      io.sink,
    );
    applyTerminalEvent(
      state,
      { kind: "output", instanceId: "pty-a", bytes: bytes(4), seq: 4 },
      io.sink,
    );
    applyTerminalEvent(
      state,
      {
        kind: "snapshot",
        instanceId: "pty-a",
        bytes: bytes(1, 2, 3),
        seq: 3,
        truncated: false,
        lifecycle: "running",
      },
      io.sink,
    );

    expect(io.writes).toEqual([[1, 2, 3], [4]]);
    expect(state.lastRenderedSeq).toBe(4);
  });
});

describe("terminal lifecycle banners", () => {
  it("renders exited, disconnected, and unknown states even without bytes", () => {
    expect(terminalLifecycleBanner("exited", "status 2")).toContain("status 2");
    expect(terminalLifecycleBanner("disconnected")).toContain("connection interrupted");
    expect(terminalLifecycleBanner("unknown")).toContain("state is unavailable");
    expect(terminalLifecycleBanner("running")).toBeNull();
  });
});
