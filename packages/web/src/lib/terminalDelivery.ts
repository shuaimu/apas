/** Terminal writes are unconfirmed until the provider transcript acknowledges them.
 * Keep these separate from chat history: optimistic messages and server echoes
 * are evidence of sending only. Never automatically replay terminal keystrokes.
 */
export interface PendingDelivery {
  id: string;
  sessionId: string;
  paneId: number;
  text: string;
  sentAt: number;
  /** Why the server refused this send (`mutation_ack` for `input`). Its
   *  outcome is known: it is not unconfirmed, and its id is never sent again.
   *  The entry stays, with its text, until the user restores or dismisses it,
   *  or a transcript acknowledgement shows it was delivered after all. */
  error?: string;
}

export const DELIVERY_GRACE_MS = 10_000;
const STORAGE_KEY = "apas_terminal_deliveries";

/** Sends still waiting for the transcript past the grace period. A refused
 *  send is not among them: the server already said what happened to it. */
export function unconfirmedDeliveries(
  pending: PendingDelivery[],
  now: number,
): PendingDelivery[] {
  return pending.filter((entry) => !entry.error && now - entry.sentAt >= DELIVERY_GRACE_MS);
}

/** Sends the server refused, shown at once with its reason. */
export function refusedDeliveries(pending: PendingDelivery[]): PendingDelivery[] {
  return pending.filter((entry) => Boolean(entry.error));
}

export function loadTerminalDeliveries(): PendingDelivery[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const entries: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) || "[]");
    if (!Array.isArray(entries)) return [];
    return entries.filter((entry): entry is PendingDelivery =>
      entry && typeof entry.id === "string" && typeof entry.sessionId === "string"
      && Number.isInteger(entry.paneId) && typeof entry.text === "string"
      && Number.isFinite(entry.sentAt)
      && (entry.error === undefined || typeof entry.error === "string"),
    );
  } catch {
    return [];
  }
}

export function saveTerminalDeliveries(entries: PendingDelivery[]): void {
  if (typeof localStorage === "undefined") return;
  try {
    if (entries.length) localStorage.setItem(STORAGE_KEY, JSON.stringify(entries));
    else localStorage.removeItem(STORAGE_KEY);
  } catch {
    // The in-memory warning still works if browser storage is unavailable.
  }
}
