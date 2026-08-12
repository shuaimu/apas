import * as SQLite from "expo-sqlite";
import {
  validateServerMessage,
  type CodeEvent,
  type MobileSessionSummary,
  type PaneWorkSummary,
} from "@apas/protocol";

import { getOrCreateCacheKey } from "@/security/credentials";
import type { PaneWorkSummaryAvailability } from "@/state/store";

const DATABASE_NAME = "apas-code.db";
let databasePromise: Promise<SQLite.SQLiteDatabase> | null = null;

interface SessionRow {
  payload: string;
}

interface EventRow {
  payload: string;
}

interface PaneWorkSummaryRow {
  payload: string;
  updated_at: string;
}

export const MAX_PANE_WORK_SUMMARY_WINDOWS = 56;

export interface CachedPaneWorkSummarySnapshot {
  sessionId: string;
  paneId: number;
  summaries: PaneWorkSummary[];
  availability: PaneWorkSummaryAvailability;
  updatedAt: string;
}

export interface CachedSnapshot {
  sessions: MobileSessionSummary[];
  events: CodeEvent[];
  watermarks: Record<string, string>;
  updatedAt: string | null;
}

export interface ConversationPosition {
  offset: number;
  followNewest: boolean;
}

export async function openCache(): Promise<SQLite.SQLiteDatabase> {
  databasePromise ??= (async () => {
    const db = await SQLite.openDatabaseAsync(DATABASE_NAME);
    const key = await getOrCreateCacheKey();
    if (!/^[0-9a-f]{64}$/.test(key)) throw new Error("Invalid cache encryption key");
    await db.execAsync(`PRAGMA key = \"x'${key}'\";`);
    await db.execAsync(`
      PRAGMA journal_mode = WAL;
      PRAGMA foreign_keys = ON;
      CREATE TABLE IF NOT EXISTS cache_meta (
        key TEXT PRIMARY KEY NOT NULL,
        value TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS session_summaries (
        session_id TEXT PRIMARY KEY NOT NULL,
        payload TEXT NOT NULL,
        updated_at TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS code_events (
        event_id TEXT PRIMARY KEY NOT NULL,
        session_id TEXT NOT NULL,
        pane_id INTEGER,
        ordering_key TEXT NOT NULL,
        payload TEXT NOT NULL,
        accepted_at TEXT NOT NULL
      );
      CREATE INDEX IF NOT EXISTS idx_code_events_session_order
        ON code_events(session_id, ordering_key);
      CREATE TABLE IF NOT EXISTS watermarks (
        stream_key TEXT PRIMARY KEY NOT NULL,
        ordering_key TEXT NOT NULL,
        accepted_at TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS task_drafts (
        draft_key TEXT PRIMARY KEY NOT NULL,
        payload TEXT NOT NULL,
        updated_at TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS conversation_positions (
        session_id TEXT PRIMARY KEY NOT NULL,
        offset REAL NOT NULL,
        follow_newest INTEGER NOT NULL,
        updated_at TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS conversation_pane_positions (
        session_id TEXT NOT NULL,
        pane_id INTEGER NOT NULL,
        offset REAL NOT NULL,
        follow_newest INTEGER NOT NULL,
        updated_at TEXT NOT NULL,
        PRIMARY KEY(session_id, pane_id)
      );
      CREATE TABLE IF NOT EXISTS selected_conversation_panes (
        session_id TEXT PRIMARY KEY NOT NULL,
        pane_id INTEGER NOT NULL,
        updated_at TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS pane_work_summary_snapshots (
        session_id TEXT NOT NULL,
        pane_id INTEGER NOT NULL,
        payload TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        PRIMARY KEY(session_id, pane_id)
      );
      PRAGMA user_version = 4;
    `);
    return db;
  })();
  return databasePromise;
}

export async function replaceSessionSummaries(sessions: MobileSessionSummary[]): Promise<void> {
  const db = await openCache();
  const now = new Date().toISOString();
  await db.withTransactionAsync(async () => {
    await db.runAsync("DELETE FROM session_summaries");
    for (const session of sessions) {
      await db.runAsync(
        "INSERT INTO session_summaries(session_id, payload, updated_at) VALUES (?, ?, ?)",
        session.id,
        JSON.stringify(session),
        now,
      );
    }
    await db.runAsync(
      "INSERT OR REPLACE INTO cache_meta(key, value) VALUES ('sessions_updated_at', ?)",
      now,
    );
  });
}

export async function acceptEvents(events: CodeEvent[]): Promise<void> {
  if (events.length === 0) return;
  const db = await openCache();
  const acceptedAt = new Date().toISOString();
  await db.withTransactionAsync(async () => {
    for (const event of events) {
      const inserted = await db.runAsync(
        `INSERT OR IGNORE INTO code_events
          (event_id, session_id, pane_id, ordering_key, payload, accepted_at)
          VALUES (?, ?, ?, ?, ?, ?)`,
        event.id,
        event.session_id,
        event.pane_id ?? null,
        event.ordering_key,
        JSON.stringify(event),
        acceptedAt,
      );
      if (inserted.changes === 0) continue;
      const streamKey = `${event.session_id}:${event.pane_id ?? "all"}`;
      await db.runAsync(
        `INSERT INTO watermarks(stream_key, ordering_key, accepted_at) VALUES (?, ?, ?)
         ON CONFLICT(stream_key) DO UPDATE SET
           ordering_key = MAX(watermarks.ordering_key, excluded.ordering_key),
           accepted_at = excluded.accepted_at`,
        streamKey,
        event.created_at,
        acceptedAt,
      );
    }
  });
}

export async function readCachedSnapshot(sessionId?: string): Promise<CachedSnapshot> {
  const db = await openCache();
  const sessions = await db.getAllAsync<SessionRow>(
    "SELECT payload FROM session_summaries ORDER BY updated_at DESC",
  );
  const events = sessionId
    ? await db.getAllAsync<EventRow>(
        "SELECT payload FROM code_events WHERE session_id = ? ORDER BY ordering_key ASC",
        sessionId,
      )
    : [];
  const watermarkRows = await db.getAllAsync<{ stream_key: string; ordering_key: string }>(
    "SELECT stream_key, ordering_key FROM watermarks",
  );
  const updated = await db.getFirstAsync<{ value: string }>(
    "SELECT value FROM cache_meta WHERE key = 'sessions_updated_at'",
  );
  return {
    sessions: sessions.map((row) => JSON.parse(row.payload) as MobileSessionSummary),
    events: events.map((row) => JSON.parse(row.payload) as CodeEvent),
    watermarks: Object.fromEntries(watermarkRows.map((row) => [row.stream_key, row.ordering_key])),
    updatedAt: updated?.value ?? null,
  };
}

export async function removeInaccessibleSessions(allowedSessionIds: Set<string>): Promise<void> {
  const db = await openCache();
  const rows = await db.getAllAsync<{ session_id: string }>("SELECT session_id FROM session_summaries");
  const summaryRows = await db.getAllAsync<{ session_id: string }>(
    "SELECT DISTINCT session_id FROM pane_work_summary_snapshots",
  );
  await db.withTransactionAsync(async () => {
    for (const sessionId of new Set([...rows, ...summaryRows].map((row) => row.session_id))) {
      if (allowedSessionIds.has(sessionId)) continue;
      await db.runAsync("DELETE FROM session_summaries WHERE session_id = ?", sessionId);
      await db.runAsync("DELETE FROM code_events WHERE session_id = ?", sessionId);
      await db.runAsync("DELETE FROM watermarks WHERE stream_key LIKE ?", `${sessionId}:%`);
      await db.runAsync("DELETE FROM conversation_positions WHERE session_id = ?", sessionId);
      await db.runAsync("DELETE FROM conversation_pane_positions WHERE session_id = ?", sessionId);
      await db.runAsync("DELETE FROM selected_conversation_panes WHERE session_id = ?", sessionId);
      await db.runAsync("DELETE FROM pane_work_summary_snapshots WHERE session_id = ?", sessionId);
    }
  });
}

export async function writePaneWorkSummarySnapshot(
  sessionId: string,
  paneId: number,
  summaries: PaneWorkSummary[],
  availability: PaneWorkSummaryAvailability,
  updatedAt = new Date().toISOString(),
): Promise<void> {
  const bounded = boundPaneWorkSummaries(sessionId, paneId, summaries);
  const db = await openCache();
  await db.runAsync(
    `INSERT INTO pane_work_summary_snapshots(session_id, pane_id, payload, updated_at)
     VALUES (?, ?, ?, ?)
     ON CONFLICT(session_id, pane_id) DO UPDATE SET
       payload = excluded.payload,
       updated_at = excluded.updated_at`,
    sessionId,
    paneId,
    JSON.stringify({ summaries: bounded, availability }),
    updatedAt,
  );
}

export async function readPaneWorkSummarySnapshot(
  sessionId: string,
  paneId: number,
): Promise<CachedPaneWorkSummarySnapshot | null> {
  const db = await openCache();
  const row = await db.getFirstAsync<PaneWorkSummaryRow>(
    "SELECT payload, updated_at FROM pane_work_summary_snapshots WHERE session_id = ? AND pane_id = ?",
    sessionId,
    paneId,
  );
  if (!row) return null;
  try {
    const payload = JSON.parse(row.payload) as { summaries?: unknown; availability?: unknown };
    const message = {
      type: "pane_work_summaries",
      session_id: sessionId,
      pane_id: paneId,
      summaries: payload.summaries,
      availability: payload.availability,
    };
    if (!validateServerMessage(message).valid || !Array.isArray(payload.summaries)) return null;
    return {
      sessionId,
      paneId,
      summaries: boundPaneWorkSummaries(sessionId, paneId, payload.summaries as PaneWorkSummary[]),
      availability: (payload.availability ?? "unknown") as PaneWorkSummaryAvailability,
      updatedAt: row.updated_at,
    };
  } catch {
    return null;
  }
}

export function boundPaneWorkSummaries(
  sessionId: string,
  paneId: number,
  summaries: PaneWorkSummary[],
): PaneWorkSummary[] {
  const byWindow = new Map<string, PaneWorkSummary>();
  for (const summary of summaries) {
    if (summary.session_id !== sessionId || summary.pane_id !== paneId) continue;
    byWindow.set(summary.window_start, summary);
  }
  return [...byWindow.values()]
    .sort((left, right) => right.window_start.localeCompare(left.window_start))
    .slice(0, MAX_PANE_WORK_SUMMARY_WINDOWS);
}

export async function saveTaskDraft(draftKey: string, value: unknown): Promise<void> {
  const db = await openCache();
  await db.runAsync(
    `INSERT INTO task_drafts(draft_key, payload, updated_at) VALUES (?, ?, ?)
     ON CONFLICT(draft_key) DO UPDATE SET payload = excluded.payload, updated_at = excluded.updated_at`,
    draftKey,
    JSON.stringify(value),
    new Date().toISOString(),
  );
}

export async function loadTaskDraft<T>(draftKey: string): Promise<T | null> {
  const db = await openCache();
  const row = await db.getFirstAsync<{ payload: string }>(
    "SELECT payload FROM task_drafts WHERE draft_key = ?",
    draftKey,
  );
  return row ? (JSON.parse(row.payload) as T) : null;
}

export async function deleteTaskDraft(draftKey: string): Promise<void> {
  const db = await openCache();
  await db.runAsync("DELETE FROM task_drafts WHERE draft_key = ?", draftKey);
}

export async function saveConversationPosition(
  sessionId: string,
  paneId: number,
  position: ConversationPosition,
): Promise<void> {
  if (!Number.isInteger(paneId) || paneId < 0 || !Number.isFinite(position.offset) || position.offset < 0) return;
  const db = await openCache();
  await db.runAsync(
    `INSERT INTO conversation_pane_positions(session_id, pane_id, offset, follow_newest, updated_at)
     VALUES (?, ?, ?, ?, ?)
     ON CONFLICT(session_id, pane_id) DO UPDATE SET
       offset = excluded.offset,
       follow_newest = excluded.follow_newest,
       updated_at = excluded.updated_at`,
    sessionId,
    paneId,
    position.offset,
    position.followNewest ? 1 : 0,
    new Date().toISOString(),
  );
}

export async function readConversationPosition(sessionId: string, paneId: number): Promise<ConversationPosition | null> {
  const db = await openCache();
  const row = await db.getFirstAsync<{ offset: number; follow_newest: number }>(
    "SELECT offset, follow_newest FROM conversation_pane_positions WHERE session_id = ? AND pane_id = ?",
    sessionId,
    paneId,
  );
  if (!row || !Number.isFinite(row.offset) || row.offset < 0) return null;
  return { offset: row.offset, followNewest: row.follow_newest === 1 };
}

export async function saveSelectedConversationPane(sessionId: string, paneId: number): Promise<void> {
  if (!Number.isInteger(paneId) || paneId < 0) return;
  const db = await openCache();
  await db.runAsync(
    `INSERT INTO selected_conversation_panes(session_id, pane_id, updated_at) VALUES (?, ?, ?)
     ON CONFLICT(session_id) DO UPDATE SET pane_id = excluded.pane_id, updated_at = excluded.updated_at`,
    sessionId,
    paneId,
    new Date().toISOString(),
  );
}

export async function readSelectedConversationPane(sessionId: string): Promise<number | null> {
  const db = await openCache();
  const row = await db.getFirstAsync<{ pane_id: number }>(
    "SELECT pane_id FROM selected_conversation_panes WHERE session_id = ?",
    sessionId,
  );
  return row && Number.isInteger(row.pane_id) && row.pane_id >= 0 ? row.pane_id : null;
}

export async function wipeCache(): Promise<void> {
  if (databasePromise) {
    const db = await databasePromise;
    await db.closeAsync();
    databasePromise = null;
  }
  await SQLite.deleteDatabaseAsync(DATABASE_NAME).catch(() => undefined);
}
