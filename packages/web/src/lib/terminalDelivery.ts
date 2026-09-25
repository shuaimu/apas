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
}

export const DELIVERY_GRACE_MS = 10_000;
const STORAGE_KEY = "apas_terminal_deliveries";

export function unconfirmedDeliveries(
  pending: PendingDelivery[],
  now: number,
): PendingDelivery[] {
  return pending.filter((entry) => now - entry.sentAt >= DELIVERY_GRACE_MS);
}

export function loadTerminalDeliveries(): PendingDelivery[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const entries: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) || "[]");
    if (!Array.isArray(entries)) return [];
    return entries.filter((entry): entry is PendingDelivery =>
      entry && typeof entry.id === "string" && typeof entry.sessionId === "string"
      && Number.isInteger(entry.paneId) && typeof entry.text === "string"
      && Number.isFinite(entry.sentAt),
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
