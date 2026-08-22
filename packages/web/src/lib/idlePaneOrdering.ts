import type { SessionPaneSummary } from "@/lib/store";

export interface IdlePaneEntry {
  session: { id: string };
  pane: Pick<SessionPaneSummary, "pane_id" | "idle_since">;
}

function idleSinceMs(value: string | null | undefined): number | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
}

/** Newest known idle transition first; legacy/invalid timestamps sort last. */
export function compareRecentlyIdle(
  left: IdlePaneEntry,
  right: IdlePaneEntry,
): number {
  const leftAt = idleSinceMs(left.pane.idle_since);
  const rightAt = idleSinceMs(right.pane.idle_since);

  if (leftAt !== null && rightAt !== null && leftAt !== rightAt) {
    return rightAt - leftAt;
  }
  if (leftAt !== null && rightAt === null) return -1;
  if (leftAt === null && rightAt !== null) return 1;

  return left.session.id.localeCompare(right.session.id)
    || left.pane.pane_id - right.pane.pane_id;
}
