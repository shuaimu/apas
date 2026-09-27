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

describe("parsed terminal checkpoints", () => {
  const snapshot: TerminalEvent = { kind: "snapshot", instanceId: "pty-a", seq: 10, bytes: bytes(1), truncated: false, lifecycle: "running", screen: { cols: 80, rows: 24, checkpointSeq: 10 } };
  it("restores dimensions before bytes and consumes a continuous checkpoint without repainting", () => {
    const state = createTerminalRenderState();
    const calls: unknown[] = [];
    const sink = { reset: () => calls.push("reset"), resize: (cols: number, rows: number) => calls.push([cols, rows]), write: (data: Uint8Array) => calls.push(Array.from(data)) };
    applyTerminalEvent(state, snapshot, sink);
    expect(calls).toEqual(["reset", [80, 24], [1]]);
    calls.length = 0;
    applyTerminalEvent(state, { ...snapshot, seq: 11, screen: { cols: 80, rows: 24, checkpointSeq: 11 } }, sink);
    expect(calls).toEqual([]);
    expect(state.lastRenderedSeq).toBe(11);
  });
  it("buffers a gap, refuses stale snapshots, and resumes only after complete recovery", () => {
    const state = createTerminalRenderState();
    const io = harness();
    applyTerminalEvent(state, snapshot, io.sink);
    applyTerminalEvent(state, { kind: "output", instanceId: "pty-a", seq: 12, bytes: bytes(2) }, io.sink);
    expect(state.needsSnapshot).toBe(true);
    expect(io.writes).toEqual([[1]]);
    expect(applyTerminalEvent(state, { ...snapshot, seq: 9 }, io.sink)).toBe(false);
    applyTerminalEvent(state, { ...snapshot, seq: 13, bytes: bytes(3), screen: { cols: 80, rows: 24, checkpointSeq: 13 } }, io.sink);
    expect(io.writes).toEqual([[1], [3]]);
    expect(state.needsSnapshot).toBe(false);
    expect(state.pending).toEqual([]);
  });
  it("never revives a replaced process from a delayed snapshot", () => {
    const state = createTerminalRenderState();
    const io = harness();
    applyTerminalEvent(state, snapshot, io.sink);
    applyTerminalEvent(state, { kind: "state", instanceId: "pty-b", lifecycle: "running" }, io.sink);
    expect(applyTerminalEvent(state, snapshot, io.sink)).toBe(false);
    expect(state.currentInstanceId).toBe("pty-b");
  });
  it("accepts an empty process's sequence-zero checkpoint after a legacy empty response", () => {
    const state = createTerminalRenderState();
    const io = harness();
    applyTerminalEvent(state, { ...snapshot, bytes: bytes(), seq: 0, screen: undefined }, io.sink);
    applyTerminalEvent(state, { ...snapshot, seq: 0, screen: { cols: 80, rows: 24, checkpointSeq: 0 } }, io.sink);
    expect(state.screen).toEqual({ cols: 80, rows: 24, checkpointSeq: 0 });
    expect(state.limitedRecovery).toBe(false);
  });
});
