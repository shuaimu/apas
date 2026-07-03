import { describe, expect, it } from "vitest";
import { groupMessagesForRender } from "./ToolGroupCard";
import type { Message, OutputType } from "@/lib/store";

function message(
  id: string,
  outputType: OutputType | undefined,
  content = "",
): Message {
  return {
    id,
    role: "assistant",
    content,
    timestamp: new Date("2026-07-03T10:00:00Z"),
    outputType,
  };
}

function textMessage(id: string, content = "plain text"): Message {
  return message(id, { type: "text" }, content);
}

function toolUse(id: string, tool: string): Message {
  return message(id, { type: "tool_use", tool, input: {}, toolUseId: `${id}-tu` });
}

function toolResult(id: string, tool: string, success = true): Message {
  return message(id, { type: "tool_result", tool, success }, "ok");
}

describe("groupMessagesForRender", () => {
  it("folds a single use+result pair into a tool group", () => {
    const items = groupMessagesForRender([
      textMessage("t1"),
      toolUse("u1", "Edit"),
      toolResult("r1", "Edit"),
      textMessage("t2"),
    ]);

    expect(items.map((i) => i.kind)).toEqual(["message", "tool-group", "message"]);
    const group = items[1];
    if (group.kind !== "tool-group") throw new Error("expected tool-group");
    expect(group.items.map((m) => m.id)).toEqual(["u1", "r1"]);
  });

  it("keeps a lone in-flight tool_use inline", () => {
    const items = groupMessagesForRender([
      textMessage("t1"),
      toolUse("u1", "Bash"),
    ]);

    expect(items.map((i) => i.kind)).toEqual(["message", "message"]);
  });

  it("splits runs on interleaved text and folds each side independently", () => {
    const items = groupMessagesForRender([
      toolUse("u1", "Read"),
      toolResult("r1", "Read"),
      textMessage("t1"),
      toolUse("u2", "Edit"),
      toolResult("r2", "Edit"),
      toolUse("u3", "Bash"),
      toolResult("r3", "Bash"),
    ]);

    expect(items.map((i) => i.kind)).toEqual(["tool-group", "message", "tool-group"]);
    const second = items[2];
    if (second.kind !== "tool-group") throw new Error("expected tool-group");
    expect(second.items.map((m) => m.id)).toEqual(["u2", "r2", "u3", "r3"]);
  });

  it("never folds AskUserQuestion cards", () => {
    const items = groupMessagesForRender([
      toolUse("u1", "AskUserQuestion"),
      toolResult("r1", "AskUserQuestion"),
    ]);

    expect(items.map((i) => i.kind)).toEqual(["message", "message"]);
  });
});
