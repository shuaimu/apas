import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AskUserQuestionCard } from "./AskUserQuestionCard";
import { AssistantMessage } from "../chat/AssistantMessage";
import { handleServerMessage, useStore } from "@/lib/store";
import { ompQuestionKey } from "@/lib/ompQuestions";

const initial = useStore.getState();
const sessionId = "11111111-1111-4111-8111-111111111111";
const paneId = 8;
const toolUseId = "omp:8:native:ask";
const dialogId = "22222222-2222-4222-8222-222222222222";
const questions = [
  { id: "single", question: "Same question?", recommended: 0, options: [{ label: "Alpha, beta", description: "Keep the comma", preview: "<script>plain text</script>" }, { label: "Gamma" }] },
  { id: "multi", question: "Same question?", multiSelect: true, options: [{ label: "Delta" }, { label: "Epsilon" }] },
];
let sent: Record<string, unknown>[];

function receive(frame: Record<string, unknown>) {
  act(() => handleServerMessage(frame, useStore.setState, useStore.getState));
}

function questionInput(dialog: string | null = dialogId) {
  return { provider: "omp", omp: { dialog_id: dialog }, questions };
}

function seed(dialog: string | null = dialogId) {
  receive({ type: "stream_message", session_id: sessionId, pane_id: paneId,
    message: { type: "assistant", message: { content: [{ type: "tool_use", name: "AskUserQuestion", id: toolUseId, input: questionInput(dialog) }] } },
  });
}

function nativeResult(status: "answered" | "cancelled" | "unavailable") {
  receive({ type: "stream_message", session_id: sessionId, pane_id: paneId,
    message: { type: "user", message: { content: [{ type: "tool_result", tool_use_id: toolUseId, content: JSON.stringify({ type: "omp_question_result", status, results: status === "answered" ? [
      { id: "single", question: "Same question?", options: ["Gamma"], multi: false, selectedOptions: ["Gamma"], customInput: "Terminal custom" },
      { id: "multi", question: "Same question?", options: [], multi: true, selectedOptions: [], note: "None requested" },
    ] : [] }) }] } },
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  localStorage.clear();
  sent = [];
  useStore.setState({ ...initial, sessionId, messages: [], paneMessages: {}, ompQuestions: new Map(), answeredQuestions: new Map(), pendingAnswers: [],
    ws: { readyState: WebSocket.OPEN, send: (raw: string) => sent.push(JSON.parse(raw)) } as unknown as WebSocket,
  }, true);
});

afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
  act(() => useStore.setState(initial, true));
  localStorage.clear();
});

describe("native OMP question interactions", () => {
  it("renders through the desktop renderer with explicit scope and sends duplicate text, comma labels, multiple selections and separate literal custom input losslessly", () => {
    seed();
    const message = useStore.getState().paneMessages[String(paneId)][0];
    const { container } = render(<AssistantMessage message={message} />);
    expect(container.querySelector("script")).toBeNull();
    expect(screen.getByText("<script>plain text</script>")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Alpha, beta/ }));
    fireEvent.click(screen.getByRole("button", { name: "Delta" }));
    fireEvent.click(screen.getByRole("button", { name: "Epsilon" }));
    fireEvent.change(screen.getByRole("textbox", { name: /question 1:/ }), { target: { value: "  literal, custom  " } });
    fireEvent.change(screen.getByRole("textbox", { name: /question 2:/ }), { target: { value: "Second custom" } });
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({ type: "answer_omp_question", session_id: sessionId, pane_id: paneId, tool_use_id: toolUseId, dialog_id: dialogId, answers: [
      { id: "single", selectedOptions: ["Alpha, beta"], customInput: "  literal, custom  " },
      { id: "multi", selectedOptions: ["Delta", "Epsilon"], customInput: "Second custom" },
    ] });
    expect(screen.getByRole("status").textContent).toContain("not yet confirmed");
    expect(screen.queryByText("Answer recorded by OMP")).toBeNull();
    expect(screen.getByRole<HTMLButtonElement>("button", { name: "Submit" }).disabled).toBe(true);
    receive({ type: "mutation_ack", mutation: "question", session_id: sessionId, pane_id: paneId, request_id: sent[0].request_id, accepted: true });
    expect(screen.getByRole("status").textContent).toContain("waiting for recorded result");
    nativeResult("answered");
    expect(screen.getByText("Answer recorded by OMP")).toBeTruthy();
    expect(screen.getByText("Terminal custom")).toBeTruthy();
    expect(screen.getByText("No options selected")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Submit" })).toBeNull();
    expect(screen.queryByDisplayValue("  literal, custom  ")).toBeNull();
  });

  it("allows custom-only single answers and empty native multi-selection", () => {
    seed();
    render(<AskUserQuestionCard toolUseId={toolUseId} sessionId={sessionId} paneId={paneId} input={questionInput()} />);
    const submit = screen.getByRole<HTMLButtonElement>("button", { name: "Submit" });
    expect(submit.disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Gamma" }));
    fireEvent.click(screen.getByRole("button", { name: "Gamma" }));
    fireEvent.change(screen.getByRole("textbox", { name: /question 1:/ }), { target: { value: " custom only " } });
    expect(submit.disabled).toBe(false);
    fireEvent.click(submit);
    expect(sent[0].answers).toEqual([
      { id: "single", selectedOptions: [], customInput: " custom only " },
      { id: "multi", selectedOptions: [] },
    ]);
  });

  it("keeps draft selections attached to native IDs when duplicate-text questions reorder", () => {
    seed();
    const { rerender } = render(<AskUserQuestionCard toolUseId={toolUseId} sessionId={sessionId} paneId={paneId} input={questionInput()} />);
    fireEvent.click(screen.getByRole("button", { name: "Gamma" }));
    fireEvent.click(screen.getByRole("button", { name: "Delta" }));
    rerender(<AskUserQuestionCard toolUseId={toolUseId} sessionId={sessionId} paneId={paneId} input={{ ...questionInput(), questions: [...questions].reverse() }} />);
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    expect(sent[0].answers).toEqual([{ id: "multi", selectedOptions: ["Delta"] }, { id: "single", selectedOptions: ["Gamma"] }]);
  });

  it.each(["cancelled", "unavailable"] as const)("renders authoritative %s without claiming an answer was recorded", (status) => {
    seed();
    render(<AskUserQuestionCard toolUseId={toolUseId} sessionId={sessionId} paneId={paneId} input={questionInput()} />);
    fireEvent.click(screen.getByRole("button", { name: "Cancel question" }));
    expect(sent[0]).toMatchObject({ cancel: true, answers: [] });
    expect(screen.getByRole("status").textContent).toContain("not yet confirmed");
    nativeResult(status);
    expect(screen.getByRole("status").textContent).toContain(status === "cancelled" ? "Question cancelled" : "Question no longer available");
    expect(screen.queryByText("Answer recorded by OMP")).toBeNull();
    expect(screen.queryByRole("button", { name: "Cancel question" })).toBeNull();
  });

  it("shows read-only terminal fallback without a bridge or explicit routing, then enables a recovered bridge", () => {
    seed(null);
    const { rerender } = render(<AskUserQuestionCard toolUseId={toolUseId} sessionId={sessionId} paneId={paneId} input={questionInput(null)} />);
    expect(screen.getByRole("status").textContent).toContain("Answer in the terminal");
    expect(screen.getByRole<HTMLButtonElement>("button", { name: "Gamma" }).disabled).toBe(true);
    seed();
    rerender(<AskUserQuestionCard toolUseId={toolUseId} input={questionInput()} />);
    expect(screen.getByRole<HTMLButtonElement>("button", { name: "Gamma" }).disabled).toBe(true);
    rerender(<AskUserQuestionCard toolUseId={toolUseId} sessionId={sessionId} paneId={paneId} input={questionInput()} />);
    expect(screen.getByRole<HTMLButtonElement>("button", { name: "Gamma" }).disabled).toBe(false);
    expect(sent).toHaveLength(0);
  });

  it("displays a pending rejection for manual correction without resending", () => {
    seed();
    render(<AskUserQuestionCard toolUseId={toolUseId} sessionId={sessionId} paneId={paneId} input={questionInput()} />);
    fireEvent.click(screen.getByRole("button", { name: "Gamma" }));
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));
    receive({ type: "mutation_ack", mutation: "question", session_id: sessionId, pane_id: paneId, request_id: sent[0].request_id, accepted: false, question_pending: true, error: "Native question remains open" });
    expect(screen.getByText("Native question remains open")).toBeTruthy();
    expect(screen.getByRole<HTMLButtonElement>("button", { name: "Submit" }).disabled).toBe(false);
    expect(sent).toHaveLength(1);
    expect(useStore.getState().ompQuestions.get(ompQuestionKey(sessionId, paneId, toolUseId))?.phase).toBe("pending");
  });
});
