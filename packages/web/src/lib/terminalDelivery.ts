/**
 * Did a message typed into a terminal pane's conversation actually land?
 *
 * Text typed here is written straight into the pty. A successful write proves
 * only that bytes were accepted — the provider may be mid-turn, sitting in a
 * menu, or still at a startup prompt ("resume a previous session?"), in which
 * case the keystrokes do something else entirely or nothing at all. The sender
 * sees no difference.
 *
 * So delivery is confirmed the same way an answered question is: by reading
 * what the provider recorded, never by the fact that we wrote. A message that
 * comes back as a user turn landed. One that never does did not, and after a
 * grace period the person who sent it should be told rather than left assuming.
 */

export interface PendingDelivery {
  paneId: number;
  text: string;
  /** Epoch millis, so a turn recorded before we sent cannot confirm us. */
  sentAt: number;
}

export interface DeliveryTurn {
  role: string;
  content: string;
  timestampMs: number;
}

/**
 * How long to wait before calling a message unconfirmed.
 *
 * The transcript is polled, so confirmation legitimately lags by a poll or so.
 * Too short and every message flickers a warning; too long and a person keeps
 * typing into a menu. Ten seconds is comfortably more than a poll and well
 * inside the time it takes to wonder why nothing happened.
 */
export const DELIVERY_GRACE_MS = 10_000;

function normalize(value: string): string {
  return value.trim().replace(/\s+/g, " ");
}

/**
 * Whether a recorded turn accounts for this message.
 *
 * Containment rather than equality: a provider may record the text with its own
 * framing around it, and a false "confirmed" is a far smaller harm than a false
 * "not confirmed" on every message.
 */
export function isDeliveryConfirmed(
  pending: PendingDelivery,
  turns: DeliveryTurn[],
): boolean {
  const wanted = normalize(pending.text);
  if (wanted.length === 0) return true;
  return turns.some(
    (turn) =>
      turn.role === "user" &&
      turn.timestampMs >= pending.sentAt &&
      normalize(turn.content).includes(wanted),
  );
}

/**
 * The messages that have neither been recorded nor are still within grace.
 *
 * Anything still inside the grace window is omitted: it is not yet evidence of
 * anything, and warning about it would train people to ignore the warning.
 */
export function unconfirmedDeliveries(
  pending: PendingDelivery[],
  turns: DeliveryTurn[],
  now: number,
  graceMs: number = DELIVERY_GRACE_MS,
): PendingDelivery[] {
  return pending.filter(
    (entry) =>
      now - entry.sentAt >= graceMs && !isDeliveryConfirmed(entry, turns),
  );
}

/** Drop what has been confirmed, so the list cannot grow without bound. */
export function pruneConfirmed(
  pending: PendingDelivery[],
  turns: DeliveryTurn[],
): PendingDelivery[] {
  return pending.filter((entry) => !isDeliveryConfirmed(entry, turns));
}
