import * as SQLite from "expo-sqlite";

import type { PaneWorkSummary } from "@apas/protocol";

import {
  MAX_PANE_WORK_SUMMARY_WINDOWS,
  boundPaneWorkSummaries,
  readPaneWorkSummarySnapshot,
  removeInaccessibleSessions,
  wipeCache,
  writePaneWorkSummarySnapshot,
} from "./cache";

jest.mock("expo-sqlite", () => ({
  openDatabaseAsync: jest.fn(),
  deleteDatabaseAsync: jest.fn(async () => undefined),
}));

jest.mock("@/security/credentials", () => ({
  getOrCreateCacheKey: jest.fn(async () => "a".repeat(64)),
}));

const mockRunAsync = jest.fn(async (..._args: unknown[]) => ({ changes: 1, lastInsertRowId: 0 }));
const mockGetFirstAsync = jest.fn(async (..._args: unknown[]): Promise<unknown> => null);
const mockDatabase = {
  execAsync: jest.fn(async () => undefined),
  getAllAsync: jest.fn(async () => [
    { session_id: "still-authorized" },
    { session_id: "deleted-or-revoked" },
  ]),
  getFirstAsync: mockGetFirstAsync,
  runAsync: mockRunAsync,
  withTransactionAsync: jest.fn(async (operation: () => Promise<void>) => operation()),
  closeAsync: jest.fn(async () => undefined),
};

const sessionId = "58dbd62d-c40f-4b5b-a1d4-96aca52ea595";

function summary(index: number, paneId = 3): PaneWorkSummary {
  const hour = String(index % 24).padStart(2, "0");
  const day = String(1 + Math.floor(index / 24)).padStart(2, "0");
  return {
    session_id: sessionId,
    pane_id: paneId,
    window_start: `2026-08-${day}T${hour}:00:00Z`,
    window_end: `2026-08-${day}T${hour}:59:59Z`,
    status: "complete",
    summary: `Window ${index}`,
  };
}

describe("encrypted mobile cache authorization cleanup", () => {
  beforeAll(() => {
    jest.mocked(SQLite.openDatabaseAsync).mockResolvedValue(
      mockDatabase as unknown as SQLite.SQLiteDatabase,
    );
  });

  beforeEach(() => {
    mockRunAsync.mockClear();
    mockGetFirstAsync.mockReset().mockResolvedValue(null);
  });

  it("transactionally removes summaries, events, and watermarks for missing sessions", async () => {
    await removeInaccessibleSessions(new Set(["still-authorized"]));

    expect(mockDatabase.withTransactionAsync).toHaveBeenCalledTimes(1);
    expect(mockDatabase.execAsync).toHaveBeenCalled();
    expect(mockDatabase.getAllAsync).toHaveBeenCalledWith(
      "SELECT session_id FROM session_summaries",
    );
    expect(mockDatabase.getAllAsync).toHaveBeenCalledWith(
      "SELECT DISTINCT session_id FROM pane_work_summary_snapshots",
    );
    expect(mockRunAsync.mock.calls).toEqual([
      ["DELETE FROM session_summaries WHERE session_id = ?", "deleted-or-revoked"],
      ["DELETE FROM code_events WHERE session_id = ?", "deleted-or-revoked"],
      ["DELETE FROM watermarks WHERE stream_key LIKE ?", "deleted-or-revoked:%"],
      ["DELETE FROM conversation_positions WHERE session_id = ?", "deleted-or-revoked"],
      ["DELETE FROM conversation_pane_positions WHERE session_id = ?", "deleted-or-revoked"],
      ["DELETE FROM selected_conversation_panes WHERE session_id = ?", "deleted-or-revoked"],
      ["DELETE FROM pane_work_summary_snapshots WHERE session_id = ?", "deleted-or-revoked"],
    ]);
  });

  it("bounds, deduplicates, and isolates summary windows before persistence", async () => {
    const summaries = [...Array.from({ length: 60 }, (_, index) => summary(index)), summary(59), summary(1, 4)];
    expect(boundPaneWorkSummaries(sessionId, 3, summaries)).toHaveLength(MAX_PANE_WORK_SUMMARY_WINDOWS);

    await writePaneWorkSummarySnapshot(sessionId, 3, summaries, "available", "2026-08-12T12:00:00Z");
    const write = mockRunAsync.mock.calls.at(-1);
    expect(write?.slice(1, 3)).toEqual([sessionId, 3]);
    const payload = JSON.parse(String(write?.[3])) as { summaries: PaneWorkSummary[] };
    expect(payload.summaries).toHaveLength(MAX_PANE_WORK_SUMMARY_WINDOWS);
    expect(payload.summaries.every((item) => item.pane_id === 3)).toBe(true);
  });

  it("rejects malformed cached snapshots", async () => {
    mockGetFirstAsync.mockResolvedValueOnce({ payload: "{not-json", updated_at: "2026-08-12T12:00:00Z" });
    await expect(readPaneWorkSummarySnapshot(sessionId, 3)).resolves.toBeNull();
  });

  it("reads only records matching the requested pane", async () => {
    mockGetFirstAsync.mockResolvedValueOnce({
      payload: JSON.stringify({ summaries: [summary(1), summary(2, 4)], availability: "available" }),
      updated_at: "2026-08-12T12:00:00Z",
    });
    const cached = await readPaneWorkSummarySnapshot(sessionId, 3);
    expect(cached?.summaries.map((item) => item.pane_id)).toEqual([3]);
    expect(mockGetFirstAsync).toHaveBeenCalledWith(
      "SELECT payload, updated_at FROM pane_work_summary_snapshots WHERE session_id = ? AND pane_id = ?",
      sessionId,
      3,
    );
  });

  it("erases the encrypted database on a full cache wipe", async () => {
    await wipeCache();
    expect(mockDatabase.closeAsync).toHaveBeenCalled();
    expect(SQLite.deleteDatabaseAsync).toHaveBeenCalledWith("apas-code.db");
  });
});
