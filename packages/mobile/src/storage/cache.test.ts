import * as SQLite from "expo-sqlite";

import { removeInaccessibleSessions } from "./cache";

jest.mock("expo-sqlite", () => ({
  openDatabaseAsync: jest.fn(),
  deleteDatabaseAsync: jest.fn(async () => undefined),
}));

jest.mock("@/security/credentials", () => ({
  getOrCreateCacheKey: jest.fn(async () => "a".repeat(64)),
}));

const mockRunAsync = jest.fn(async () => ({ changes: 1, lastInsertRowId: 0 }));
const mockDatabase = {
  execAsync: jest.fn(async () => undefined),
  getAllAsync: jest.fn(async () => [
    { session_id: "still-authorized" },
    { session_id: "deleted-or-revoked" },
  ]),
  runAsync: mockRunAsync,
  withTransactionAsync: jest.fn(async (operation: () => Promise<void>) => operation()),
};

describe("encrypted mobile cache authorization cleanup", () => {
  beforeAll(() => {
    jest.mocked(SQLite.openDatabaseAsync).mockResolvedValue(
      mockDatabase as unknown as SQLite.SQLiteDatabase,
    );
  });

  it("transactionally removes summaries, events, and watermarks for missing sessions", async () => {
    await removeInaccessibleSessions(new Set(["still-authorized"]));

    expect(mockDatabase.withTransactionAsync).toHaveBeenCalledTimes(1);
    expect(mockDatabase.execAsync).toHaveBeenCalled();
    expect(mockDatabase.getAllAsync).toHaveBeenCalledWith(
      "SELECT session_id FROM session_summaries",
    );
    expect(mockRunAsync.mock.calls).toEqual([
      ["DELETE FROM session_summaries WHERE session_id = ?", "deleted-or-revoked"],
      ["DELETE FROM code_events WHERE session_id = ?", "deleted-or-revoked"],
      ["DELETE FROM watermarks WHERE stream_key LIKE ?", "deleted-or-revoked:%"],
      ["DELETE FROM conversation_positions WHERE session_id = ?", "deleted-or-revoked"],
      ["DELETE FROM conversation_pane_positions WHERE session_id = ?", "deleted-or-revoked"],
      ["DELETE FROM selected_conversation_panes WHERE session_id = ?", "deleted-or-revoked"],
    ]);
  });
});
