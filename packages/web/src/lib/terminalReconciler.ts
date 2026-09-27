import type { TerminalEvent, TerminalLifecycle } from "./terminalBus";

type PendingOutput = Extract<TerminalEvent, { kind: "output" }>;

export interface TerminalRenderState {
  currentInstanceId?: string;
  lastRenderedSeq?: number;
  snapshotSeen: boolean;
  pending: PendingOutput[];
  lifecycle?: TerminalLifecycle;
  status?: string;
}

export interface TerminalSink {
  write(bytes: Uint8Array): void;
  reset(): void;
}

export function createTerminalRenderState(): TerminalRenderState {
  return { snapshotSeen: false, pending: [] };
}

function conflicts(current: string | undefined, incoming: string | undefined): boolean {
  return current !== undefined && incoming !== undefined && current !== incoming;
}

function adoptInstance(
  state: TerminalRenderState,
  instanceId: string,
  sink: TerminalSink,
): void {
  if (state.currentInstanceId !== instanceId) {
    if (state.currentInstanceId !== undefined || state.lastRenderedSeq !== undefined) {
      sink.reset();
    }
    state.currentInstanceId = instanceId;
    state.lastRenderedSeq = undefined;
  }
}

function applyLifecycle(
  state: TerminalRenderState,
  lifecycle: TerminalLifecycle,
  status?: string,
): void {
  state.lifecycle = lifecycle;
  state.status = status;
}

/**
 * Apply one server event to the mounted emulator without duplicating
 * cumulative snapshots or allowing a replaced PTY's delayed live events to
 * corrupt the new screen. The mutable state belongs in a React ref so raw
 * terminal traffic never enters global application state.
 */
export function applyTerminalEvent(
  state: TerminalRenderState,
  event: TerminalEvent,
  sink: TerminalSink,
): boolean {
  if (event.kind === "snapshot") {
    const replacement = conflicts(state.currentInstanceId, event.instanceId)
      || (state.currentInstanceId === undefined
        && event.instanceId !== undefined
        && state.lastRenderedSeq !== undefined);

    if (replacement && event.instanceId !== undefined) {
      adoptInstance(state, event.instanceId, sink);
    } else if (state.currentInstanceId === undefined && event.instanceId !== undefined) {
      state.currentInstanceId = event.instanceId;
    }

    applyLifecycle(state, event.lifecycle, event.status);

    const snapshotIsAhead =
      state.lastRenderedSeq !== undefined && event.seq > state.lastRenderedSeq;
    const replay =
      replacement || state.lastRenderedSeq === undefined || snapshotIsAhead;
    if (replay) {
      // Snapshots are cumulative. If this browser missed any frame, reset and
      // rebuild instead of appending the same full-screen history twice.
      if (!replacement && (snapshotIsAhead || event.truncated)) sink.reset();
      if (event.bytes.length > 0) sink.write(event.bytes);
      state.lastRenderedSeq = event.seq;
    }

    for (const pending of state.pending) {
      if (conflicts(state.currentInstanceId, pending.instanceId)) continue;
      if (state.currentInstanceId === undefined && pending.instanceId !== undefined) {
        state.currentInstanceId = pending.instanceId;
      }
      if (state.lastRenderedSeq === undefined || pending.seq > state.lastRenderedSeq) {
        sink.write(pending.bytes);
        state.lastRenderedSeq = pending.seq;
      }
    }
    state.pending = [];
    state.snapshotSeen = true;
    return true;
  }

  if (event.kind === "state") {
    // The server only fans accepted state transitions, so a changed instance
    // here is the authoritative replacement boundary.
    if (event.instanceId !== undefined) {
      adoptInstance(state, event.instanceId, sink);
      state.pending = state.pending.filter(
        (pending) => !conflicts(event.instanceId, pending.instanceId),
      );
    }
    applyLifecycle(state, event.lifecycle, event.status);
    return true;
  }

  if (event.kind === "exited") {
    if (conflicts(state.currentInstanceId, event.instanceId)) return false;
    if (state.currentInstanceId === undefined && event.instanceId !== undefined) {
      adoptInstance(state, event.instanceId, sink);
    }
    applyLifecycle(state, "exited", event.status);
    return true;
  }

  if (!state.snapshotSeen) {
    state.pending.push(event);
    return true;
  }
  if (conflicts(state.currentInstanceId, event.instanceId)) return false;
  if (state.currentInstanceId === undefined && event.instanceId !== undefined) {
    adoptInstance(state, event.instanceId, sink);
  }
  if (state.lastRenderedSeq !== undefined && event.seq <= state.lastRenderedSeq) {
    return false;
  }
  sink.write(event.bytes);
  state.lastRenderedSeq = event.seq;
  return true;
}

export function terminalLifecycleBanner(
  lifecycle: TerminalLifecycle | undefined,
  status?: string,
): string | null {
  switch (lifecycle) {
    case "exited":
      return `Process ended (${status ?? "status unavailable"}). Reboot the pane to start a new one.`;
    case "disconnected":
      return "CLI connection interrupted. Showing retained terminal output while it reconnects.";
    case "unknown":
      return "Terminal process state is unavailable. Reconnect or reboot the pane if it remains unresponsive.";
    case "running":
    case undefined:
      return null;
  }
}
