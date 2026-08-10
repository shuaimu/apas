import type { ServerToWeb } from "./generated";

export type TerminalLifecycle = "unknown" | "running" | "disconnected" | "exited";

export interface TerminalReconciliationState {
  instanceId: string | null;
  sequence: number;
  lifecycle: TerminalLifecycle;
  status: string | null;
  needsSnapshot: boolean;
  generation: number;
}

export interface TerminalReconciliationResult {
  state: TerminalReconciliationState;
  action: "ignore" | "reset" | "snapshot" | "output" | "lifecycle";
  dataBase64?: string;
  truncated?: boolean;
}

export const initialTerminalState = (): TerminalReconciliationState => ({
  instanceId: null,
  sequence: 0,
  lifecycle: "unknown",
  status: null,
  needsSnapshot: true,
  generation: 0,
});

export const terminalTheme = {
  background: "#08080b",
  foreground: "#f2f2f4",
  cursor: "#8b80ff",
  selectionBackground: "#4a426f88",
  black: "#18181c",
  red: "#ff6b6b",
  green: "#51cf8a",
  yellow: "#f7c948",
  blue: "#75a7ff",
  magenta: "#c792ea",
  cyan: "#65d1d1",
  white: "#e8e8ec",
} as const;

type TerminalServerMessage = Extract<
  ServerToWeb,
  { type: "terminal_snapshot" | "terminal_output" | "terminal_state" | "terminal_exited" }
>;

export function reconcileTerminal(
  current: TerminalReconciliationState,
  message: TerminalServerMessage,
): TerminalReconciliationResult {
  const instanceId = message.instance_id ?? null;
  if (message.type === "terminal_snapshot") {
    const reset = current.instanceId !== null && current.instanceId !== instanceId;
    return {
      state: {
        instanceId,
        sequence: message.seq,
        lifecycle: message.lifecycle ?? "unknown",
        status: message.status ?? null,
        needsSnapshot: false,
        generation: reset ? current.generation + 1 : current.generation,
      },
      action: "snapshot",
      dataBase64: message.data_b64,
      truncated: message.truncated ?? false,
    };
  }
  if (message.type === "terminal_output") {
    if (current.needsSnapshot || current.instanceId !== instanceId) {
      return { state: { ...current, needsSnapshot: true }, action: "ignore" };
    }
    if (message.seq <= current.sequence) return { state: current, action: "ignore" };
    if (message.seq !== current.sequence + 1) {
      return { state: { ...current, needsSnapshot: true }, action: "ignore" };
    }
    return {
      state: { ...current, sequence: message.seq },
      action: "output",
      dataBase64: message.data_b64,
    };
  }
  const lifecycle: TerminalLifecycle = message.type === "terminal_exited"
    ? "exited"
    : (message.lifecycle ?? "unknown");
  const instanceChanged = current.instanceId !== null && current.instanceId !== instanceId;
  return {
    state: {
      ...current,
      instanceId,
      lifecycle,
      status: message.status ?? null,
      needsSnapshot: current.needsSnapshot || instanceChanged,
      generation: instanceChanged ? current.generation + 1 : current.generation,
    },
    action: instanceChanged ? "reset" : "lifecycle",
  };
}

export function markTerminalTransportDisconnected(
  current: TerminalReconciliationState,
): TerminalReconciliationState {
  return {
    ...current,
    lifecycle: current.lifecycle === "exited" ? "exited" : "disconnected",
    needsSnapshot: true,
  };
}

export function clearTerminalForAccessLoss(
  current: TerminalReconciliationState,
): TerminalReconciliationState {
  return { ...initialTerminalState(), generation: current.generation + 1 };
}
