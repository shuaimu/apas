import type { CodeEvent, ServerToWeb } from "./generated.js";
import { describe, expect, it } from "vitest";
import {
  applyCodeEvents,
  deriveAttention,
  normalizeServerMessage,
  orderingKey,
  streamWatermark,
} from "./codeEvents.js";

const context = { receivedAt: "2026-08-08T12:00:00.000Z", sequence: 7 };

describe("CodeEvent helpers", () => {
  it("normalizes a user instruction with a stable client identity", () => {
    const message: ServerToWeb = {
      type: "user_input",
      session_id: "58dbd62d-c40f-4b5b-a1d4-96aca52ea595",
      pane_id: 2,
      text: "Fix the failing test",
      client_msg_id: "send-1",
      created_at: "2026-08-08T11:59:00Z",
    };
    const first = normalizeServerMessage(message, context)[0];
    const replay = normalizeServerMessage(message, { ...context, sequence: 99 })[0];
    expect(first.kind).toBe("instruction");
    expect(first.id).toBe(replay.id);
  });

  it("maps plan and pull-request outcomes to mobile activity", () => {
    expect(normalizeServerMessage({
      type: "plan_review_request",
      session_id: "58dbd62d-c40f-4b5b-a1d4-96aca52ea595",
      pane_id: 1,
      tool_name: "ExitPlanMode",
      tool_use_id: "tool-1",
      input: {},
    }, context)[0]).toMatchObject({ kind: "plan", requires_attention: true });
    expect(normalizeServerMessage({
      type: "pr_created",
      session_id: "58dbd62d-c40f-4b5b-a1d4-96aca52ea595",
      pane_id: 1,
      url: "https://github.com/example/repo/pull/1",
    }, context)[0].kind).toBe("pull_request");
  });

  it("keeps transient working status out of conversation history", () => {
    expect(normalizeServerMessage({
      type: "pane_status",
      session_id: "58dbd62d-c40f-4b5b-a1d4-96aca52ea595",
      pane_id: 9,
      status: "Working...",
    }, context)).toEqual([]);
    expect(normalizeServerMessage({
      type: "pane_status",
      session_id: "58dbd62d-c40f-4b5b-a1d4-96aca52ea595",
      pane_id: 9,
      status: null,
    }, context)).toEqual([]);
  });

  it("keeps approval routing on the exact session and pane", () => {
    expect(normalizeServerMessage({
      type: "output",
      session_id: "58dbd62d-c40f-4b5b-a1d4-96aca52ea595",
      pane_id: 9,
      content: "Approve deployment?",
      output_type: {
        approval_request: {
          tool_call_id: "approval-1",
          tool: "Bash",
          description: "Deploy",
        },
      },
    }, context)[0]).toMatchObject({
      session_id: "58dbd62d-c40f-4b5b-a1d4-96aca52ea595",
      pane_id: 9,
      kind: "approval",
      requires_attention: true,
    });
    expect(normalizeServerMessage({
      type: "output",
      pane_id: 9,
      content: "Legacy unattributed approval",
      output_type: {
        approval_request: {
          tool_call_id: "approval-legacy",
          tool: "Bash",
          description: "Deploy",
        },
      },
    }, context)).toEqual([]);
  });

  it("deduplicates and keeps server ordering", () => {
    const event = (id: string, sequence: number): CodeEvent => ({
      id,
      session_id: "58dbd62d-c40f-4b5b-a1d4-96aca52ea595",
      ordering_key: orderingKey("2026-08-08T12:00:00Z", sequence),
      created_at: "2026-08-08T12:00:00Z",
      kind: "agent_status",
      summary: id,
    });
    const accepted = applyCodeEvents([event("later", 2)], [event("earlier", 1), event("later", 2)]);
    expect(accepted.map((item) => item.id)).toEqual(["earlier", "later"]);
    expect(streamWatermark(accepted)).toBe(event("later", 2).ordering_key);
  });

  it("merges an older pagination page without duplicating the overlap", () => {
    const event = (id: string, key: string): CodeEvent => ({
      id,
      session_id: "58dbd62d-c40f-4b5b-a1d4-96aca52ea595",
      ordering_key: key,
      created_at: `2026-08-08T12:00:0${key}Z`,
      kind: "agent_status",
      summary: id,
    });
    const current = [event("third", "3"), event("fourth", "4")];
    const olderPage = [event("first", "1"), event("second", "2"), event("third", "3")];
    expect(applyCodeEvents(current, olderPage).map((item) => item.id)).toEqual([
      "first",
      "second",
      "third",
      "fourth",
    ]);
  });

  it("reduces a high-volume replay deterministically", () => {
    const incoming: CodeEvent[] = Array.from({ length: 5_000 }, (_, index) => ({
      id: `event-${index}`,
      session_id: "58dbd62d-c40f-4b5b-a1d4-96aca52ea595",
      ordering_key: index.toString().padStart(8, "0"),
      created_at: "2026-08-08T12:00:00Z",
      kind: "agent_status",
      summary: `Event ${index}`,
    }));
    const accepted = applyCodeEvents([], [...incoming, ...incoming.slice(4_500)]);
    expect(accepted).toHaveLength(5_000);
    expect(accepted.at(-1)?.id).toBe("event-4999");
  });

  it("derives unresolved attention", () => {
    const events: CodeEvent[] = [
      { id: "question", session_id: "s", pane_id: 1, ordering_key: "1", created_at: "now", kind: "question", summary: "Choose", requires_attention: true },
      { id: "done", session_id: "s", pane_id: 2, ordering_key: "2", created_at: "now", kind: "completed", summary: "Done" },
      { id: "error", session_id: "s", pane_id: 2, ordering_key: "3", created_at: "now", kind: "error", summary: "Failed", requires_attention: true },
    ];
    expect(deriveAttention(events).map((event) => event.id)).toEqual(["question"]);
  });
});
