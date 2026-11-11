import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { handleServerMessage, useStore } from "./store";
import { loadOmpQuestionStates, ompQuestionKey, type OmpQuestionResult } from "./ompQuestions";

const initial = useStore.getState();
const sid = "11111111-1111-4111-8111-111111111111";
const pane = 7;
const toolId = "omp:7:native-session:native-tool";
const dialogId = "22222222-2222-4222-8222-222222222222";
const key = ompQuestionKey(sid, pane, toolId);
const input = { provider: "omp", questions: [{ id: "native-id", question: "Pick", options: [{ label: "one, two" }], multiSelect: true }], omp: { dialog_id: dialogId } };
let sent: Record<string, unknown>[];

function receive(frame: Record<string, unknown>) {
  handleServerMessage(frame, useStore.setState, useStore.getState);
}

function ask(dialog: string | null = dialogId, sessionId = sid, paneId = pane, createdAt = "2026-10-10T12:00:00Z") {
  receive({ type: "stream_message", session_id: sessionId, pane_id: paneId, created_at: createdAt,
    message: { type: "assistant", message: { content: [{ type: "tool_use", name: "AskUserQuestion", id: toolId, input: { ...input, omp: { dialog_id: dialog } } }] } },
  });
}

function result(status: OmpQuestionResult["status"], label = "Terminal choice"): string {
  return JSON.stringify({ type: "omp_question_result", status, results: status === "answered" ? [{ id: "native-id", question: "Pick", options: [label], multi: true, selectedOptions: [label], customInput: " native text " }] : [] });
}

function recorded(content: string) {
  receive({ type: "stream_message", session_id: sid, pane_id: pane,
    message: { type: "user", message: { content: [{ type: "tool_result", tool_use_id: toolId, content, is_error: false }] } },
  });
}

function submit(cancel = false) {
  useStore.getState().answerOmpQuestion({ sessionId: sid, paneId: pane, toolUseId: toolId, dialogId, answers: [{ id: "native-id", selectedOptions: ["one, two"], customInput: " literal, text " }], cancel });
}

beforeEach(() => {
  vi.useFakeTimers();
  localStorage.clear();
  sent = [];
  useStore.setState({ ...initial, sessionId: sid, paneMessages: {}, messages: [], sessionCache: new Map(), ompQuestions: new Map(), answeredQuestions: new Map(), pendingAnswers: [], isDualPane: true,
    ws: { readyState: WebSocket.OPEN, send: (raw: string) => sent.push(JSON.parse(raw)) } as unknown as WebSocket,
  }, true);
});

afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
  useStore.setState(initial, true);
  localStorage.clear();
});

describe("OMP question state and exact wire routing", () => {
  it("sends explicit origin even when the foreground session differs, without legacy optimism or replay queue", () => {
    ask();
    useStore.setState({ sessionId: "different-foreground" });
    submit();
    expect(sent).toHaveLength(1);
    expect(sent[0]).toEqual({ type: "answer_omp_question", session_id: sid, pane_id: pane, tool_use_id: toolId, dialog_id: dialogId, request_id: expect.any(String), answers: [{ id: "native-id", selectedOptions: ["one, two"], customInput: " literal, text " }], cancel: false });
    expect(useStore.getState().ompQuestions.get(key)?.phase).toBe("submitting");
    expect(useStore.getState().answeredQuestions.size).toBe(0);
    expect(useStore.getState().pendingAnswers).toEqual([]);
    useStore.getState().answerQuestion(toolId, { Pick: "bad legacy answer" });
    expect(sent).toHaveLength(1);
  });

  it("correlates acknowledgements and permits only explicit retry after a confirmed pending rejection", () => {
    ask(); submit();
    const ack = { type: "mutation_ack", mutation: "question", session_id: sid, pane_id: pane, request_id: sent[0].request_id };
    receive({ ...ack, request_id: "stale", accepted: true });
    receive({ ...ack, pane_id: pane + 1, accepted: true });
    expect(useStore.getState().ompQuestions.get(key)?.phase).toBe("submitting");
    receive({ ...ack, accepted: false, question_pending: true, error: "Choose a valid option" });
    expect(useStore.getState().ompQuestions.get(key)?.phase).toBe("pending");
    expect(sent).toHaveLength(1);
    submit();
    expect(sent).toHaveLength(2);
    expect(sent[1].request_id).not.toBe(sent[0].request_id);
    receive({ ...ack, accepted: true });
    expect(useStore.getState().ompQuestions.get(key)?.phase).toBe("submitting");
    receive({ ...ack, request_id: sent[1].request_id, accepted: true });
    expect(useStore.getState().ompQuestions.get(key)?.phase).toBe("accepted");
    expect(useStore.getState().ompQuestions.get(key)?.result).toBeUndefined();
    submit();
    expect(sent).toHaveLength(2);
    recorded(result("answered"));
    expect(useStore.getState().ompQuestions.get(key)?.result?.results[0].selectedOptions).toEqual(["Terminal choice"]);
    receive({ ...ack, request_id: sent[1].request_id, accepted: false, question_pending: true });
    expect(useStore.getState().ompQuestions.get(key)?.phase).toBe("answered");
  });

  it("times out to uncertainty, preserves uncertainty across refresh, and never automatically resends", () => {
    ask(); submit();
    vi.advanceTimersByTime(15_000);
    expect(useStore.getState().ompQuestions.get(key)?.phase).toBe("uncertain");
    useStore.setState({ ompQuestions: loadOmpQuestionStates() });
    ask(); submit();
    receive({ type: "authenticated", user_id: "user" });
    receive({ type: "session_attached", session_id: sid, has_active_cli: true });
    vi.advanceTimersByTime(60_000);
    expect(sent.filter((frame) => frame.type === "answer_omp_question" || frame.type === "answer_question")).toHaveLength(1);
    expect(useStore.getState().ompQuestions.get(key)?.phase).toBe("uncertain");
    recorded(result("answered"));
    expect(loadOmpQuestionStates().size).toBe(0);
  });

  it("cancels with no answers and waits for the native cancellation", () => {
    ask(); submit(true);
    expect(sent[0]).toMatchObject({ cancel: true, answers: [] });
    expect(useStore.getState().ompQuestions.get(key)?.phase).toBe("submitting");
    recorded(result("cancelled"));
    expect(useStore.getState().ompQuestions.get(key)?.phase).toBe("cancelled");
    expect(useStore.getState().answeredQuestions.size).toBe(0);
  });

  it("upserts live metadata and results without duplicate cards, scoped by pane and session", () => {
    ask(null); ask(); ask();
    expect(useStore.getState().paneMessages[String(pane)]).toHaveLength(1);
    expect(useStore.getState().ompQuestions.get(key)?.dialogId).toBe(dialogId);
    ask(null, sid, pane, "2026-10-10T12:01:00Z");
    submit();
    expect(sent).toHaveLength(0);
    expect(useStore.getState().paneMessages[String(pane)]).toHaveLength(1);
    ask(dialogId, sid, pane + 1);
    expect(useStore.getState().paneMessages[String(pane + 1)]).toHaveLength(1);
    recorded(result("unavailable"));
    recorded(result("answered"));
    recorded(result("unavailable"));
    recorded(result("answered"));
    expect(useStore.getState().paneMessages[String(pane)]).toHaveLength(2);
    expect(useStore.getState().ompQuestions.get(key)?.phase).toBe("answered");
  });

  it("recovers pending metadata and authoritative answers from history and catchup", () => {
    const row = (id: string, messageType: string, content: unknown, createdAt: string) => ({ id, pane_id: pane, role: messageType === "tool_result" ? "tool" : "assistant", message_type: messageType, content: JSON.stringify(content), created_at: createdAt });
    const question = { id: toolId, name: "AskUserQuestion", input };
    const rows = [row("q-old", "tool_use", { ...question, input: { ...input, omp: { dialog_id: null } } }, "2026-10-10T12:00:00Z"), row("q-new", "tool_use", question, "2026-10-10T12:01:00Z")];
    receive({ type: "session_messages", session_id: sid, messages: rows, has_more: false });
    expect(useStore.getState().paneMessages[String(pane)]).toHaveLength(1);
    expect(useStore.getState().ompQuestions.get(key)?.dialogId).toBe(dialogId);
    submit();
    useStore.setState({ ompQuestions: loadOmpQuestionStates() });
    const answer = row("result", "tool_result", { tool_use_id: toolId, content: result("answered", "Actual native result"), is_error: false }, "2026-10-10T12:02:00Z");
    receive({ type: "session_messages", session_id: sid, messages: [...rows, answer], catchup: true });
    receive({ type: "session_messages", session_id: sid, messages: [...rows, answer], catchup: true });
    expect(useStore.getState().paneMessages[String(pane)]).toHaveLength(2);
    expect(useStore.getState().ompQuestions.get(key)?.result?.results[0].selectedOptions).toEqual(["Actual native result"]);
    expect(useStore.getState().pendingAnswers).toEqual([]);
  });

  it("recovers result-only history before the matching question page", () => {
    receive({ type: "session_messages", session_id: sid, messages: [{ id: "answer-first", pane_id: pane, role: "tool", message_type: "tool_result", content: JSON.stringify({ tool_use_id: toolId, content: result("answered"), is_error: false }), created_at: "2026-10-10T12:03:00Z" }] });
    ask();
    expect(useStore.getState().ompQuestions.get(key)?.phase).toBe("answered");
    submit();
    expect(sent).toHaveLength(0);
  });

  it("reopens a branch-invalidated question only with newer verified dialog metadata, never a recorded native answer", () => {
    ask();
    receive({ type: "stream_message", session_id: sid, pane_id: pane, created_at: "2026-10-10T12:01:00Z",
      message: { type: "user", message: { content: [{ type: "tool_result", tool_use_id: toolId, content: result("unavailable") }] } },
    });
    ask(null, sid, pane, "2026-10-10T12:02:00Z");
    expect(useStore.getState().ompQuestions.get(key)?.phase).toBe("unavailable");
    ask(dialogId, sid, pane, "2026-10-10T12:03:00Z");
    expect(useStore.getState().ompQuestions.get(key)?.phase).toBe("pending");
    expect(useStore.getState().ompQuestions.get(key)?.result).toBeUndefined();
    recorded(result("answered"));
    ask(dialogId, sid, pane, "2026-10-10T12:04:00Z");
    expect(useStore.getState().ompQuestions.get(key)?.phase).toBe("answered");
  });

  it("clears submission state when its project is removed and on logout", () => {
    ask(); submit();
    useStore.setState({ sessions: [{ id: sid, projectId: "project", status: "active" }] });
    useStore.getState().forgetProject("project");
    expect(useStore.getState().ompQuestions.size).toBe(0);
    expect(loadOmpQuestionStates().size).toBe(0);
    ask();
    useStore.setState({ ws: null });
    useStore.getState().logout();
    expect(useStore.getState().ompQuestions.size).toBe(0);
    expect(loadOmpQuestionStates().size).toBe(0);
  });

  it("does not send stale dialogs or disconnected answers and clears removed pane state", () => {
    ask();
    useStore.getState().answerOmpQuestion({ sessionId: sid, paneId: pane, toolUseId: toolId, dialogId: "stale-dialog", answers: [] });
    expect(sent).toHaveLength(0);
    useStore.setState({ ws: null });
    submit();
    expect(useStore.getState().ompQuestions.get(key)?.phase).toBe("uncertain");
    expect(sent).toHaveLength(0);
    receive({ type: "pane_list", session_id: sid, panes: [] });
    expect(useStore.getState().ompQuestions.size).toBe(0);
    expect(loadOmpQuestionStates().size).toBe(0);
  });
});
