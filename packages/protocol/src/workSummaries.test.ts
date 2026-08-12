import { describe, expect, it } from "vitest";
import type { ServerToWeb, WebToServer } from "./generated.js";
import { validateClientMessage, validateServerMessage } from "./validators.js";

const sessionId = "58dbd62d-c40f-4b5b-a1d4-96aca52ea595";
const summary = {
  protocol_version: 1,
  session_id: sessionId,
  pane_id: 7,
  window_start: "2026-08-12T00:00:00Z",
  window_end: "2026-08-12T03:00:00Z",
  window_kind: "completed" as const,
  status: "complete" as const,
  summary: "Implemented and verified the selected pane feature while preserving authorization and existing conversation behavior for attached users across clients.",
  source_digest: "a".repeat(64),
  source_message_count: 12,
  attempts: 1,
};

describe("pane work summary protocol", () => {
  it("accepts exact pane list and window refresh requests", () => {
    const list: WebToServer = {
      type: "list_pane_work_summaries",
      session_id: sessionId,
      pane_id: 7,
      include_current: true,
    };
    const refresh: WebToServer = {
      type: "refresh_pane_work_summary",
      session_id: sessionId,
      pane_id: 7,
      window_start: "2026-08-09T21:00:00Z",
    };
    expect(validateClientMessage(list)).toMatchObject({ valid: true, errors: [] });
    expect(validateClientMessage(refresh)).toMatchObject({ valid: true, errors: [] });
  });

  it("accepts authoritative snapshots and incremental updates", () => {
    const snapshot: ServerToWeb = {
      type: "pane_work_summaries",
      session_id: sessionId,
      pane_id: 7,
      summaries: [summary],
      availability: "available",
    };
    const update: ServerToWeb = {
      type: "pane_work_summary_updated",
      session_id: sessionId,
      pane_id: 7,
      summary,
      availability: "available",
    };
    expect(validateServerMessage(snapshot)).toMatchObject({ valid: true, errors: [] });
    expect(validateServerMessage(update)).toMatchObject({ valid: true, errors: [] });
  });
});
