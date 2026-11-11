import type { Message } from "./store";

export interface OmpQuestionAnswer {
  id: string;
  selectedOptions: string[];
  customInput?: string;
}

export interface OmpQuestionResult {
  type: "omp_question_result";
  status: "answered" | "cancelled" | "unavailable";
  results: (OmpQuestionAnswer & {
    question: string;
    options: string[];
    multi: boolean;
    note?: string;
    timedOut?: boolean;
  })[];
  message?: string;
}

export interface OmpQuestionState {
  sessionId: string;
  paneId: number;
  toolUseId: string;
  updatedAt?: number;
  dialogId?: string;
  requestId?: string;
  phase: "pending" | "submitting" | "accepted" | "uncertain" | "answered" | "cancelled" | "unavailable";
  error?: string;
  result?: OmpQuestionResult;
}

export interface AnswerOmpQuestionRequest {
  sessionId: string;
  paneId: number;
  toolUseId: string;
  dialogId: string;
  answers: OmpQuestionAnswer[];
  cancel?: boolean;
}

export interface AnswerOmpQuestionMessage {
  type: "answer_omp_question";
  session_id: string;
  pane_id: number;
  tool_use_id: string;
  dialog_id: string;
  request_id: string;
  answers: OmpQuestionAnswer[];
  cancel: boolean;
}

export interface OmpQuestionMutationAck {
  type: "mutation_ack";
  mutation: "question";
  session_id: string;
  pane_id?: number;
  request_id: string;
  accepted: boolean;
  question_pending?: boolean;
  error?: string;
}

export function ompQuestionKey(sessionId: string, paneId: number, toolUseId: string): string {
  return JSON.stringify([sessionId, paneId, toolUseId]);
}

export function isOmpQuestionInput(input: unknown): input is { provider: "omp"; omp?: { dialog_id?: string | null } } {
  return !!input && typeof input === "object" && "provider" in input && input.provider === "omp";
}

export function parseOmpQuestionResult(content: unknown): OmpQuestionResult | undefined {
  if (typeof content !== "string") return;
  try {
    const value = JSON.parse(content);
    if (value?.type !== "omp_question_result" || !["answered", "cancelled", "unavailable"].includes(value.status) || !Array.isArray(value.results)) return;
    if (value.message !== undefined && typeof value.message !== "string") return;
    if (!value.results.every((row: OmpQuestionResult["results"][number]) =>
      row && typeof row.id === "string" && typeof row.question === "string" &&
      Array.isArray(row.options) && typeof row.multi === "boolean" &&
      Array.isArray(row.selectedOptions) && row.selectedOptions.every((label) => typeof label === "string") &&
      (row.customInput === undefined || typeof row.customInput === "string") &&
      (row.note === undefined || typeof row.note === "string") &&
      (row.timedOut === undefined || typeof row.timedOut === "boolean"))) return;
    return value as OmpQuestionResult;
  } catch {
    return;
  }
}

// Both live replay and history use stable native identity, never the storage or
// browser-generated message id. Keep the first card's position and React key.
export function mergeOmpQuestionMessages(messages: Message[], incoming?: Message): Message[] {
  if (incoming) {
    const output = incoming.outputType;
    const native = output?.type === "tool_use"
      ? isOmpQuestionInput(output.input)
      : output?.type === "tool_result" && !!parseOmpQuestionResult(incoming.content);
    if (!native) return [...messages, incoming];
    messages = [...messages, incoming];
  }
  const positions = new Map<string, number>();
  const merged: Message[] = [];
  for (const message of messages) {
    const output = message.outputType;
    const native = output?.type === "tool_use"
      ? isOmpQuestionInput(output.input)
      : output?.type === "tool_result" && !!parseOmpQuestionResult(message.content);
    const key = native && output && "toolUseId" in output && output.toolUseId
      ? JSON.stringify([message.sessionId, message.paneId, output.type, output.toolUseId])
      : undefined;
    const index = key === undefined ? undefined : positions.get(key);
    if (index === undefined) {
      if (key !== undefined) positions.set(key, merged.length);
      merged.push(message);
    } else {
      const previous = merged[index];
      const previousResult = parseOmpQuestionResult(previous.content);
      const nextResult = parseOmpQuestionResult(message.content);
      // A lifecycle invalidation must never overwrite the actual native result.
      if (nextResult?.status === "unavailable" && previousResult && previousResult.status !== "unavailable") continue;
      if (message.timestamp >= previous.timestamp || (nextResult && nextResult.status !== "unavailable")) {
        merged[index] = { ...message, id: previous.id };
      }
    }
  }
  return merged;
}

const STORAGE_KEY = "apas_omp_question_submissions";

// Persist delivery uncertainty, not an answer or replayable payload. Authoritative
// results are recovered from history. A refresh can never retransmit a native ask.
export function saveOmpQuestionStates(states: Map<string, OmpQuestionState>): void {
  if (typeof localStorage === "undefined") return;
  try {
    const pending = [...states].filter(([, state]) => state.requestId && !state.result);
    if (pending.length) localStorage.setItem(STORAGE_KEY, JSON.stringify(pending));
    else localStorage.removeItem(STORAGE_KEY);
  } catch { /* Browser storage may be unavailable. */ }
}

export function loadOmpQuestionStates(): Map<string, OmpQuestionState> {
  if (typeof localStorage === "undefined") return new Map();
  try {
    const stored: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "[]");
    if (!Array.isArray(stored)) return new Map();
    const states = new Map<string, OmpQuestionState>();
    for (const entry of stored) {
      if (!Array.isArray(entry)) continue;
      const state = entry[1] as OmpQuestionState | undefined;
      if (!state || typeof state.sessionId !== "string" || typeof state.paneId !== "number" || typeof state.toolUseId !== "string" || typeof state.requestId !== "string") continue;
      states.set(ompQuestionKey(state.sessionId, state.paneId, state.toolUseId), {
        sessionId: state.sessionId, paneId: state.paneId, toolUseId: state.toolUseId,
        requestId: state.requestId,
        dialogId: typeof state.dialogId === "string" ? state.dialogId : undefined,
        updatedAt: typeof state.updatedAt === "number" ? state.updatedAt : undefined,
        phase: state.phase === "accepted" ? "accepted" : "uncertain",
      });
    }
    return states;
  } catch {
    return new Map();
  }
}
