/**
 * Transport for `PaneKind: "terminal"` pane bytes.
 *
 * Deliberately NOT zustand state. A full-screen TUI repaints many times a
 * second, and putting those chunks in the store would re-render the whole
 * subscriber tree on every frame. The store's WebSocket handler forwards
 * terminal frames straight here, and only the mounted `TerminalPane` for
 * that pane is woken — React never sees the bytes at all.
 */

export type TerminalEvent =
  | {
      /** Replayed scrollback from the server ring buffer on attach. */
      kind: "snapshot";
      bytes: Uint8Array;
      seq: number;
      /** Older bytes were evicted, so this may start mid-escape-sequence. */
      truncated: boolean;
    }
  | { kind: "output"; bytes: Uint8Array; seq: number }
  | { kind: "exited"; status?: string };

type Listener = (event: TerminalEvent) => void;

const listeners = new Map<number, Set<Listener>>();

/**
 * Listen for one pane's terminal frames. Returns an unsubscribe function.
 */
export function subscribeTerminal(paneId: number, listener: Listener): () => void {
  let set = listeners.get(paneId);
  if (!set) {
    set = new Set();
    listeners.set(paneId, set);
  }
  set.add(listener);
  return () => {
    const current = listeners.get(paneId);
    if (!current) return;
    current.delete(listener);
    if (current.size === 0) listeners.delete(paneId);
  };
}

export function emitTerminal(paneId: number, event: TerminalEvent): void {
  const set = listeners.get(paneId);
  if (!set) return;
  for (const listener of set) {
    try {
      listener(event);
    } catch (err) {
      // One bad listener must not stop the others from getting the frame,
      // or a single unmounting pane could stall every other terminal.
      console.error("terminal listener failed", err);
    }
  }
}

/** True when something is currently rendering this pane. */
export function hasTerminalListener(paneId: number): boolean {
  return (listeners.get(paneId)?.size ?? 0) > 0;
}

/**
 * Decode a base64 frame into raw bytes.
 *
 * `atob` yields a binary string (one char per byte); the char codes are the
 * bytes. We keep them as a Uint8Array rather than a JS string because pty
 * chunks split UTF-8 sequences, and xterm.js reassembles multi-byte
 * characters across writes only when fed bytes.
 */
export function decodeBase64(data: string): Uint8Array {
  const binary = atob(data);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

/** Encode raw bytes (keystrokes) as base64 for the wire. */
export function encodeBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

/** Reset for tests — clears every subscription. */
export function __resetTerminalBus(): void {
  listeners.clear();
}
