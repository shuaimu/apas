// IndexedDB-backed persistent layer for the per-session message cache.
//
// In-memory `sessionCache` in store.ts gives us instant tab switching
// within a single page load. This module extends that so the cache also
// survives page reloads / new tabs — the snapshot is mirrored into
// IndexedDB on every "leave a session", and on app boot we hydrate the
// in-memory Map from disk.
//
// Why IndexedDB, not localStorage:
//   * localStorage caps at ~5 MB per origin and serializes synchronously,
//     blocking the main thread. Long sessions can easily exceed that.
//   * IndexedDB stores structured-clonable values directly (Map, Date,
//     nested arrays) — no JSON.stringify, no Date->string lossy round
//     trip.
//
// We use idb-keyval (a tiny wrapper) with a custom store name so we
// don't collide with anything else the app might ever stash in IDB.

import {
  clear,
  createStore,
  del,
  entries,
  get,
  set,
  type UseStore,
} from "idb-keyval";
import type { SessionCacheEntry } from "./store";

const DB_NAME = "apas-cache";
const STORE_NAME = "session-snapshots";

// Lazily construct the store handle so SSR / no-window contexts don't
// crash on module load. (Next.js may evaluate this file server-side
// during static page collection; idb-keyval throws if `indexedDB` is
// missing.)
let cachedStore: UseStore | null = null;
function getStore(): UseStore | null {
  if (typeof window === "undefined") return null;
  if (typeof indexedDB === "undefined") return null;
  if (cachedStore) return cachedStore;
  try {
    cachedStore = createStore(DB_NAME, STORE_NAME);
  } catch (e) {
    console.warn("[sessionCacheDb] createStore failed", e);
    return null;
  }
  return cachedStore;
}

/**
 * Read every persisted snapshot. Used once on app boot to hydrate the
 * in-memory cache. Returns an empty Map on any error so callers don't
 * need to handle absent / quota-exceeded / blocked situations.
 */
export async function loadAllSnapshots(): Promise<Map<string, SessionCacheEntry>> {
  const store = getStore();
  if (!store) return new Map();
  try {
    const all = await entries<string, SessionCacheEntry>(store);
    const out = new Map<string, SessionCacheEntry>();
    for (const [key, value] of all) {
      if (typeof key === "string" && value && typeof value === "object") {
        out.set(key, value);
      }
    }
    return out;
  } catch (e) {
    console.warn("[sessionCacheDb] loadAllSnapshots failed", e);
    return new Map();
  }
}

/**
 * Read a single snapshot by session id — exposed for the (rare) case
 * where someone navigates directly to a session whose hydration hasn't
 * completed yet, but isn't wired in by default since boot-time
 * hydration covers the normal flow.
 */
export async function loadSnapshot(
  sessionId: string,
): Promise<SessionCacheEntry | undefined> {
  const store = getStore();
  if (!store) return undefined;
  try {
    return await get<SessionCacheEntry>(sessionId, store);
  } catch (e) {
    console.warn("[sessionCacheDb] loadSnapshot failed", e);
    return undefined;
  }
}

/**
 * Persist a snapshot. Fire-and-forget — callers don't await; failure
 * just means the next page reload won't have this entry, which is a
 * graceful degradation back to "fetch from server."
 */
export function saveSnapshot(
  sessionId: string,
  entry: SessionCacheEntry,
): void {
  const store = getStore();
  if (!store) return;
  set(sessionId, entry, store).catch((e) => {
    console.warn("[sessionCacheDb] saveSnapshot failed", e);
  });
}

/**
 * Drop one snapshot from disk. Used by the in-memory LRU eviction so
 * the disk cache doesn't grow unbounded across long project-hopping
 * sessions. Fire-and-forget.
 */
export function deleteSnapshot(sessionId: string): void {
  const store = getStore();
  if (!store) return;
  del(sessionId, store).catch((e) => {
    console.warn("[sessionCacheDb] deleteSnapshot failed", e);
  });
}

/**
 * Drop every persisted snapshot. Wired into the Settings → Clear local cache
 * button so a user stuck with a partial bucket (lost stream_message + the
 * trust-local rule) can recover without DevTools.
 */
export async function clearAllSnapshots(): Promise<void> {
  const store = getStore();
  if (!store) return;
  try {
    await clear(store);
  } catch (e) {
    console.warn("[sessionCacheDb] clearAllSnapshots failed", e);
  }
}
