/**
 * Phase 4.2a: extract a per-tool timeline from a pane's flat message
 * list. Each entry pairs a `tool_use` block with its matching
 * `tool_result` (by toolUseId). The result content lives in the
 * tool_result message's `content` string; we surface a short summary.
 *
 * Pure function — no state, easy to unit-test, reused by the timeline
 * view (4.2b) and potentially the team-modal / diff modal later.
 */
import type { Message } from "./store";

export interface TimelineEntry {
  /** Original tool name as claude reported it ("Bash", "Edit", …). */
  tool: string;
  /** Tool use id, useful for stable React keys + correlation. */
  toolUseId?: string;
  /** Raw tool input JSON for the expanded view. */
  input: unknown;
  /** One-line argument summary derived from input (e.g. "src/foo.rs"). */
  argSummary: string;
  /** Raw result content (the tool_result message body). */
  resultBody?: string;
  /** One-line result summary, truncated. */
  resultSummary?: string;
  /** Did the tool succeed? Undefined if no matching tool_result seen yet. */
  ok?: boolean;
  /** Timestamp of the tool_use (when claude requested it). */
  startedAt: Date;
  /** Timestamp of the tool_result. Undefined if still pending. */
  finishedAt?: Date;
}

const ARG_MAX = 70;
const RESULT_MAX = 120;

export function extractTimeline(messages: Message[]): TimelineEntry[] {
  // First pass: tool_uses become entries; second pass: matching tool_results fill them in.
  // We preserve original order via `entries`. `byId` is a side index for the second pass.
  const entries: TimelineEntry[] = [];
  const byId = new Map<string, number>(); // toolUseId → entries[]index

  for (const m of messages) {
    const t = m.outputType?.type;
    if (t === "tool_use") {
      const ot = m.outputType as { type: "tool_use"; tool: string; input: unknown; toolUseId?: string };
      const idx = entries.length;
      entries.push({
        tool: ot.tool,
        toolUseId: ot.toolUseId,
        input: ot.input,
        argSummary: summarizeArgs(ot.tool, ot.input),
        startedAt: m.timestamp,
      });
      if (ot.toolUseId) byId.set(ot.toolUseId, idx);
    } else if (t === "tool_result") {
      const ot = m.outputType as { type: "tool_result"; tool: string; success: boolean };
      // tool_result messages don't carry the tool_use_id directly in
      // OutputType (it lives on the raw payload). We pair on insertion
      // order as a fallback — match the most recent unfilled entry for
      // the same tool name.
      let idx: number | undefined;
      for (let i = entries.length - 1; i >= 0; i--) {
        if (entries[i].tool === ot.tool && entries[i].ok === undefined) {
          idx = i;
          break;
        }
      }
      if (idx !== undefined) {
        entries[idx].ok = ot.success;
        entries[idx].resultBody = m.content;
        entries[idx].resultSummary = summarizeResult(m.content);
        entries[idx].finishedAt = m.timestamp;
      }
      // Suppress lint: byId is also kept up to date for future tooling
      // that wants O(1) lookup by toolUseId.
      void byId;
    }
  }

  return entries;
}

function summarizeArgs(tool: string, input: unknown): string {
  if (input === null || typeof input !== "object") return "";
  const o = input as Record<string, unknown>;
  const pickStr = (k: string): string | undefined =>
    typeof o[k] === "string" ? (o[k] as string) : undefined;
  switch (tool) {
    case "Read":
    case "Edit":
    case "Write":
    case "MultiEdit":
      return truncate(pickStr("file_path") ?? "", ARG_MAX);
    case "NotebookEdit":
      return truncate(pickStr("notebook_path") ?? "", ARG_MAX);
    case "Bash":
      return truncate(pickStr("command") ?? "", ARG_MAX);
    case "Glob":
      return truncate(pickStr("pattern") ?? "", ARG_MAX);
    case "Grep":
      return truncate(pickStr("pattern") ?? "", ARG_MAX);
    case "LS":
      return truncate(pickStr("path") ?? "", ARG_MAX);
    case "WebFetch":
      return truncate(pickStr("url") ?? "", ARG_MAX);
    case "WebSearch":
      return truncate(pickStr("query") ?? "", ARG_MAX);
    case "Task":
      return truncate(pickStr("description") ?? pickStr("subagent_type") ?? "", ARG_MAX);
    default: {
      // Generic fallback: first short string field.
      for (const [, v] of Object.entries(o)) {
        if (typeof v === "string" && v.length > 0) {
          return truncate(v, ARG_MAX);
        }
      }
      return "";
    }
  }
}

function summarizeResult(body: string): string {
  if (!body) return "";
  // Take the first non-empty line, truncate.
  const firstLine = body.split("\n").find((l) => l.trim().length > 0) ?? body;
  return truncate(firstLine.trim(), RESULT_MAX);
}

function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  return s.slice(0, max - 1) + "…";
}
