import type { ServerToWeb } from "@apas/protocol";

type TerminalMessage = Extract<
  ServerToWeb,
  { type: "terminal_snapshot" | "terminal_output" | "terminal_state" | "terminal_exited" }
>;

type Listener = (message: TerminalMessage) => void;
const listeners = new Set<Listener>();

export function publishTerminalMessage(message: ServerToWeb): void {
  if (!["terminal_snapshot", "terminal_output", "terminal_state", "terminal_exited"].includes(message.type)) return;
  for (const listener of listeners) listener(message as TerminalMessage);
}

export function subscribeTerminalMessages(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
