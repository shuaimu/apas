/**
 * Which pane a mobile session screen opens on.
 *
 * Lives outside the session screen because two surfaces need it: the screen
 * reads it on entry, and the home writes it when a row names a specific agent.
 * Keeping the key in one place is the point — spelled twice, the two would
 * eventually disagree and the write would silently do nothing.
 */
const MOBILE_SELECTED_PANE_PREFIX = "apas_mobile_selected_pane:";

export function readSelectedPane(sessionId: string): number | null {
  try {
    const raw = window.localStorage.getItem(`${MOBILE_SELECTED_PANE_PREFIX}${sessionId}`);
    if (raw === null) return null;
    const value = Number(raw);
    return Number.isInteger(value) && value >= 0 ? value : null;
  } catch {
    return null;
  }
}

export function writeSelectedPane(sessionId: string, paneId: number) {
  try {
    window.localStorage.setItem(`${MOBILE_SELECTED_PANE_PREFIX}${sessionId}`, String(paneId));
  } catch {
    // A private-mode browser refusing storage costs the remembered pane, not
    // the navigation.
  }
}
