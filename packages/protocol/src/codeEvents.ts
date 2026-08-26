import type {
  ClaudeContentBlock,
  CodeEvent,
  CodeEventKind,
  MessageInfo,
  ServerToWeb,
} from "./generated";

export interface NormalizationContext {
  receivedAt: string;
  sequence: number;
}

function stableHash(value: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}

function compact(value: unknown, maximum = 180): string {
  const text = typeof value === "string" ? value : JSON.stringify(value);
  const normalized = text.replace(/\s+/g, " ").trim();
  return normalized.length > maximum ? `${normalized.slice(0, maximum - 1)}…` : normalized;
}

export function eventIdentity(parts: readonly unknown[]): string {
  return `evt_${stableHash(parts.map((part) => JSON.stringify(part)).join("\u001f"))}`;
}

export function orderingKey(createdAt: string, sequence = 0): string {
  const milliseconds = Date.parse(createdAt);
  const timestamp = Number.isNaN(milliseconds) ? createdAt : new Date(milliseconds).toISOString();
  return `${timestamp}:${sequence.toString().padStart(10, "0")}`;
}

export function interpretMessageRole(message: MessageInfo): CodeEventKind {
  const type = message.message_type.toLowerCase();
  if (message.role === "user") return "instruction";
  if (type.includes("tool")) return "tool";
  if (type.includes("question")) return "question";
  if (type.includes("approval")) return "approval";
  if (type.includes("error")) return "error";
  return "agent_status";
}

function streamSummary(message: Extract<ServerToWeb, { type: "stream_message" }>): {
  kind: CodeEventKind;
  summary: string;
  detail?: unknown;
  attention?: boolean;
} {
  const stream = message.message;
  if (stream.type === "result") {
    return {
      kind: stream.is_error ? "error" : "completed",
      summary: compact(stream.result ?? (stream.is_error ? "Task failed" : "Task completed")),
      detail: stream,
      attention: Boolean(stream.is_error),
    };
  }
  if (stream.type === "system") {
    return { kind: "agent_status", summary: compact(stream.subtype), detail: stream };
  }
  const blocks = stream.message.content;
  const tool = blocks.find((block): block is Extract<ClaudeContentBlock, { type: "tool_use" }> => block.type === "tool_use");
  if (tool) {
    const lowerName = tool.name.toLowerCase();
    if (lowerName.includes("askuserquestion")) return { kind: "question", summary: "Agent asked a question", detail: tool, attention: true };
    if (lowerName.includes("approval")) return { kind: "approval", summary: "Agent requested approval", detail: tool, attention: true };
    if (lowerName.includes("todo")) return { kind: "todo", summary: "Task list updated", detail: tool };
    const command = typeof tool.input === "object" && tool.input !== null && "command" in tool.input
      ? String((tool.input as { command: unknown }).command)
      : "";
    if (lowerName === "bash" && /(^|\s)(test|pytest|cargo test|npm test|pnpm test|vitest|jest)(\s|$)/i.test(command)) {
      return { kind: "test", summary: compact(command), detail: tool };
    }
    return { kind: "tool", summary: `Using ${tool.name}`, detail: tool };
  }
  const failed = blocks.find((block): block is Extract<ClaudeContentBlock, { type: "tool_result" }> => block.type === "tool_result" && Boolean(block.is_error));
  if (failed) return { kind: "error", summary: compact(failed.content), detail: failed, attention: true };
  const text = blocks.find((block): block is Extract<ClaudeContentBlock, { type: "text" }> => block.type === "text");
  return { kind: "agent_status", summary: compact(text?.text ?? stream.type), detail: stream };
}

function makeEvent(
  sessionId: string,
  paneId: number | null | undefined,
  createdAt: string,
  sequence: number,
  kind: CodeEventKind,
  summary: string,
  detail: unknown,
  requiresAttention = false,
  identityHint?: unknown,
): CodeEvent {
  return {
    id: eventIdentity([sessionId, paneId, kind, identityHint ?? detail, createdAt]),
    session_id: sessionId,
    pane_id: paneId ?? undefined,
    ordering_key: orderingKey(createdAt, sequence),
    created_at: createdAt,
    kind,
    summary,
    requires_attention: requiresAttention,
    detail,
  };
}

export function normalizeServerMessage(
  message: ServerToWeb,
  context: NormalizationContext,
): CodeEvent[] {
  const at = "created_at" in message && typeof message.created_at === "string"
    ? message.created_at
    : context.receivedAt;
  switch (message.type) {
    case "output": {
      const output = message.output_type;
      if (typeof output === "object" && output !== null && "approval_request" in output) {
        // Older servers did not attribute approval output to a session. A
        // mobile client must fail closed instead of inventing a cache key that
        // could surface the decision in the wrong project.
        if (!message.session_id) return [];
        const approval = output.approval_request;
        return [makeEvent(
          message.session_id,
          message.pane_id,
          at,
          context.sequence,
          "approval",
          `Approval requested: ${compact(approval.description || approval.tool)}`,
          message,
          true,
          approval.tool_call_id,
        )];
      }
      return [];
    }
    case "user_input":
      return [makeEvent(message.session_id, message.pane_id, at, context.sequence, "instruction", compact(message.text), message, false, message.client_msg_id)];
    case "stream_message": {
      const normalized = streamSummary(message);
      return [makeEvent(message.session_id, message.pane_id, at, context.sequence, normalized.kind, normalized.summary, normalized.detail, normalized.attention)];
    }
    case "session_messages":
      return message.messages.map((stored, index) => {
        const createdAt = stored.created_at ?? at;
        return makeEvent(
          message.session_id,
          stored.pane_id,
          createdAt,
          context.sequence + index,
          interpretMessageRole(stored),
          compact(stored.content),
          stored,
          interpretMessageRole(stored) === "error",
          stored.id,
        );
      });
    case "plan_review_request":
      return [makeEvent(message.session_id, message.pane_id, at, context.sequence, "plan", "Plan review requested", message, true, message.tool_use_id)];
    case "pane_diff":
      return [makeEvent(message.session_id, message.pane_id, at, context.sequence, "diff", message.error ? `Diff failed: ${compact(message.error)}` : "Changes ready to review", message, Boolean(message.error))];
    case "pr_created":
      return [makeEvent(message.session_id, message.pane_id, at, context.sequence, "pull_request", message.error ? `Pull request failed: ${compact(message.error)}` : "Pull request ready", message, Boolean(message.error))];
    case "terminal_state":
    case "terminal_exited":
      return [makeEvent(message.session_id, message.pane_id, at, context.sequence, message.type === "terminal_exited" ? "completed" : "terminal", message.type === "terminal_exited" ? `Terminal exited${message.status ? `: ${message.status}` : ""}` : `Terminal ${message.lifecycle ?? "unknown"}`, message)];
    case "pane_status": {
      const interrupted = message.status?.toLowerCase().includes("interrupt") ?? false;
      // Pane status is transient UI state, not a conversation turn. Keeping
      // every "Thinking"/"Editing" transition in the timeline creates noisy
      // pseudo-messages and leaves a permanent "Pane updated" entry when the
      // status clears. Interruptions remain real activity because they resolve
      // pending attention for that pane.
      return interrupted
        ? [makeEvent(message.session_id, message.pane_id, at, context.sequence, "interrupted", "Agent interrupted", message)]
        : [];
    }
    case "error":
      return [];
    default:
      return [];
  }
}

export function compareCodeEvents(left: CodeEvent, right: CodeEvent): number {
  return left.ordering_key.localeCompare(right.ordering_key) || left.id.localeCompare(right.id);
}

export function applyCodeEvents(current: readonly CodeEvent[], incoming: readonly CodeEvent[]): CodeEvent[] {
  const accepted = new Map(current.map((event) => [event.id, event]));
  for (const event of incoming) {
    if (!accepted.has(event.id)) accepted.set(event.id, event);
  }
  return [...accepted.values()].sort(compareCodeEvents);
}

export function deriveAttention(events: readonly CodeEvent[]): CodeEvent[] {
  const resolved = new Set(
    events
      .filter((event) => event.kind === "completed" || event.kind === "interrupted")
      .map((event) => event.pane_id),
  );
  return events.filter((event) => event.requires_attention && !resolved.has(event.pane_id));
}

export function streamWatermark(events: readonly CodeEvent[]): string | null {
  return events.reduce<string | null>(
    (maximum, event) => (maximum === null || event.ordering_key > maximum ? event.ordering_key : maximum),
    null,
  );
}
