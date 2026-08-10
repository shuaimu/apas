"use client";

import { useCallback, useEffect, useState } from "react";

/**
 * How a `PaneKind: "terminal"` pane is displayed.
 *
 *  - `terminal` — the raw pty through xterm.js. What the pane actually is.
 *  - `conversation` — the same structured chat view an agent pane gets, built
 *    from the turns the CLI reads out of the provider's own transcript.
 *
 * The second view only became possible once terminal panes had history at all:
 * their turns now arrive as ordinary pane messages, so the existing
 * `MessagePane` renders them with no special casing.
 *
 * Note the two views are not equivalent and the difference is not cosmetic.
 * The terminal is live and interactive; the conversation view is a *reading*
 * of the transcript, so it lags by up to one poll interval and shows only
 * user/assistant turns. Its companion composer writes messages into the pty,
 * which makes it the practical mobile control surface while raw terminal stays
 * available for menus, modifier keys, and troubleshooting.
 */
export type TerminalViewMode = "terminal" | "conversation";

const STORAGE_KEY = "apas_terminal_view_mode";

/**
 * Persisted per pane rather than globally: someone watching one agent work
 * while reading another's transcript is the normal case, and a single global
 * flag would fight them on every tab switch.
 */
function readAll(): Record<string, TerminalViewMode> {
  if (typeof window === "undefined") return {};
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    return raw ? (JSON.parse(raw) as Record<string, TerminalViewMode>) : {};
  } catch {
    // Corrupt or unavailable storage must not take the pane down with it.
    return {};
  }
}

function writeAll(map: Record<string, TerminalViewMode>) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(map));
  } catch {
    // Private mode / quota. The choice just won't survive a reload.
  }
}

/** Key by session too, so the same pane id in another project is independent. */
export function viewModeKey(sessionId: string | null, paneId: number): string {
  return `${sessionId ?? "none"}:${paneId}`;
}

/**
 * Current view for a pane plus a setter. Defaults to `terminal` — that is what
 * the pane *is*, and defaulting to a lagging read-only view would look like the
 * terminal had broken.
 */
export function useTerminalViewMode(
  sessionId: string | null,
  paneId: number,
): [TerminalViewMode, (mode: TerminalViewMode) => void] {
  const { modeForPane, setModeForPane } = useTerminalViewModes(sessionId);
  const mode = modeForPane(paneId);

  const setMode = useCallback(
    (next: TerminalViewMode) => {
      setModeForPane(paneId, next);
    },
    [paneId, setModeForPane],
  );

  return [mode, setMode];
}

/**
 * Controls every terminal pane in one component without calling a hook inside
 * a dynamic pane list. Each mounted pane keeps its own mode while a shared
 * toolbar can read and update whichever pane is active.
 */
export function useTerminalViewModes(sessionId: string | null): {
  modeForPane: (paneId: number) => TerminalViewMode;
  setModeForPane: (paneId: number, mode: TerminalViewMode) => void;
} {
  // Starts empty and syncs after mount: reading localStorage during render
  // would mismatch the server-rendered HTML and trip a hydration error.
  const [modes, setModes] = useState<Record<string, TerminalViewMode>>({});

  useEffect(() => {
    setModes(readAll());
  }, []);

  const modeForPane = useCallback(
    (paneId: number): TerminalViewMode => {
      const stored = modes[viewModeKey(sessionId, paneId)];
      return stored === "conversation" ? "conversation" : "terminal";
    },
    [modes, sessionId],
  );

  const setModeForPane = useCallback(
    (paneId: number, next: TerminalViewMode) => {
      const key = viewModeKey(sessionId, paneId);
      setModes((previous) => ({ ...previous, [key]: next }));
      const all = readAll();
      all[key] = next;
      writeAll(all);
    },
    [sessionId],
  );

  return { modeForPane, setModeForPane };
}
