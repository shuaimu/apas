import { describe, expect, it } from "vitest";
import { extractTimeline } from "./timeline";
import type { Message, OutputType } from "./store";

function message(
  id: string,
  timestamp: string,
  outputType: OutputType,
  content = "",
): Message {
  return {
    id,
    role: "assistant",
    content,
    timestamp: new Date(timestamp),
    outputType,
  };
}

function textMessage(id: string, timestamp: string, content = "plain text"): Message {
  return message(id, timestamp, { type: "text" }, content);
}

function toolUse(
  id: string,
  timestamp: string,
  tool: string,
  input: unknown,
  toolUseId = `${id}-tool`,
): Message {
  return message(
    id,
    timestamp,
    { type: "tool_use", tool, input, toolUseId },
    `Using ${tool}`,
  );
}

function toolResult(
  id: string,
  timestamp: string,
  tool: string,
  success: boolean,
  content: string,
): Message {
  return message(
    id,
    timestamp,
    { type: "tool_result", tool, success },
    content,
  );
}

describe("extractTimeline", () => {
  it("extracts common tool uses with argument summaries and generic fallback truncation", () => {
    const longGenericValue = "x".repeat(80);

    const entries = extractTimeline([
      textMessage("ignored", "2026-06-17T10:00:00Z"),
      toolUse("read", "2026-06-17T10:01:00Z", "Read", {
        file_path: "packages/web/src/lib/timeline.ts",
      }),
      toolUse("bash", "2026-06-17T10:02:00Z", "Bash", {
        command: "npm test -- --runInBand",
      }),
      toolUse("grep", "2026-06-17T10:03:00Z", "Grep", {
        pattern: "tool_result",
      }),
      toolUse("generic", "2026-06-17T10:04:00Z", "CustomTool", {
        retries: 2,
        label: longGenericValue,
      }),
    ]);

    expect(entries.map((entry) => entry.tool)).toEqual([
      "Read",
      "Bash",
      "Grep",
      "CustomTool",
    ]);
    expect(entries.map((entry) => entry.argSummary)).toEqual([
      "packages/web/src/lib/timeline.ts",
      "npm test -- --runInBand",
      "tool_result",
      `${"x".repeat(69)}…`,
    ]);
    expect(entries[3].argSummary).toHaveLength(70);
  });

  it("pairs tool results with success and failure summaries and finishedAt timestamps", () => {
    const readStarted = "2026-06-17T11:00:00Z";
    const bashStarted = "2026-06-17T11:01:00Z";
    const bashFinished = "2026-06-17T11:02:00Z";
    const readFinished = "2026-06-17T11:03:00Z";

    const entries = extractTimeline([
      toolUse("read", readStarted, "Read", { file_path: "README.md" }, "read-1"),
      toolUse("bash", bashStarted, "Bash", { command: "cargo test" }, "bash-1"),
      toolResult("bash-result", bashFinished, "Bash", false, "\n  command failed\nstack"),
      toolResult("read-result", readFinished, "Read", true, "read complete\nmore output"),
    ]);

    expect(entries).toHaveLength(2);
    expect(entries[0]).toMatchObject({
      tool: "Read",
      toolUseId: "read-1",
      ok: true,
      resultBody: "read complete\nmore output",
      resultSummary: "read complete",
      startedAt: new Date(readStarted),
      finishedAt: new Date(readFinished),
    });
    expect(entries[1]).toMatchObject({
      tool: "Bash",
      toolUseId: "bash-1",
      ok: false,
      resultBody: "\n  command failed\nstack",
      resultSummary: "command failed",
      startedAt: new Date(bashStarted),
      finishedAt: new Date(bashFinished),
    });
  });

  it("keeps unmatched tool uses pending and ignores unrelated messages and orphan results", () => {
    const entries = extractTimeline([
      textMessage("text", "2026-06-17T12:00:00Z"),
      message("system", "2026-06-17T12:01:00Z", { type: "system" }, "system"),
      toolResult("orphan", "2026-06-17T12:02:00Z", "Read", true, "unused"),
      toolUse("grep", "2026-06-17T12:03:00Z", "Grep", { pattern: "TODO-101" }),
    ]);

    expect(entries).toHaveLength(1);
    expect(entries[0]).toMatchObject({
      tool: "Grep",
      argSummary: "TODO-101",
    });
    expect(entries[0].ok).toBeUndefined();
    expect(entries[0].resultBody).toBeUndefined();
    expect(entries[0].resultSummary).toBeUndefined();
    expect(entries[0].finishedAt).toBeUndefined();
  });

  it("pairs multiple same-tool results with the most recent unfilled entry first", () => {
    const entries = extractTimeline([
      toolUse("bash-1", "2026-06-17T13:00:00Z", "Bash", {
        command: "first command",
      }),
      toolUse("bash-2", "2026-06-17T13:01:00Z", "Bash", {
        command: "second command",
      }),
      toolResult("result-2", "2026-06-17T13:02:00Z", "Bash", true, "second done"),
      toolResult("result-1", "2026-06-17T13:03:00Z", "Bash", true, "first done"),
    ]);

    expect(entries.map((entry) => entry.argSummary)).toEqual([
      "first command",
      "second command",
    ]);
    expect(entries.map((entry) => entry.resultSummary)).toEqual([
      "first done",
      "second done",
    ]);
    expect(entries[0].finishedAt).toEqual(new Date("2026-06-17T13:03:00Z"));
    expect(entries[1].finishedAt).toEqual(new Date("2026-06-17T13:02:00Z"));
  });
});
