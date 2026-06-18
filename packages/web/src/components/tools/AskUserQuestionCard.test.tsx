import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AskUserQuestionCard } from "./AskUserQuestionCard";
import { useStore } from "@/lib/store";

const initialStore = useStore.getState();

type StoreState = ReturnType<typeof useStore.getState>;

function seedQuestionStore(overrides: Partial<{
  answeredQuestions: Map<string, Record<string, string>>;
  answerQuestion: ReturnType<typeof vi.fn>;
}> = {}) {
  const answerQuestion = overrides.answerQuestion ?? vi.fn();

  act(() => {
    useStore.setState({
      answeredQuestions: overrides.answeredQuestions ?? new Map(),
      answerQuestion: answerQuestion as StoreState["answerQuestion"],
    });
  });

  return { answerQuestion };
}

describe("AskUserQuestionCard", () => {
  beforeEach(() => {
    Element.prototype.scrollIntoView = vi.fn();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("renders the fallback for empty or invalid question input", () => {
    const { rerender } = render(
      <AskUserQuestionCard toolUseId="tool-empty" input={{ questions: [] }} />,
    );

    expect(screen.getByText("Question received but no options were provided.")).toBeTruthy();

    rerender(<AskUserQuestionCard toolUseId="tool-invalid" input="not-a-question" />);

    expect(screen.getByText("Question received but no options were provided.")).toBeTruthy();
  });

  it("submits a single-select answer keyed by the exact question text", () => {
    const { answerQuestion } = seedQuestionStore();

    render(
      <AskUserQuestionCard
        toolUseId="tool-single"
        input={{
          questions: [
            {
              question: "Which rollout path should we take?",
              header: "Release",
              options: [
                { label: "Safe path", description: "Lower risk" },
                { label: "Fast path", description: "Ship immediately" },
              ],
            },
          ],
        }}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /fast path/i }));
    fireEvent.click(screen.getByRole("button", { name: /submit/i }));

    expect(answerQuestion).toHaveBeenCalledWith("tool-single", {
      "Which rollout path should we take?": "Fast path",
    });
  });

  it("requires trimmed Other text for multi-select answers and submits labels plus free text", () => {
    const { answerQuestion } = seedQuestionStore();

    render(
      <AskUserQuestionCard
        toolUseId="tool-multi"
        input={{
          questions: [
            {
              question: "Which tasks should be included?",
              multiSelect: true,
              options: [
                { label: "Write tests" },
                { label: "Update docs" },
              ],
            },
          ],
        }}
      />,
    );

    const submit = screen.getByRole("button", { name: /submit/i }) as HTMLButtonElement;

    expect(submit.disabled).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: /write tests/i }));
    expect(submit.disabled).toBe(false);

    fireEvent.click(screen.getByRole("button", { name: /other/i }));
    expect(submit.disabled).toBe(true);

    const otherInput = screen.getByPlaceholderText(/type your answer/i);
    fireEvent.change(otherInput, { target: { value: "   " } });
    expect(submit.disabled).toBe(true);

    fireEvent.change(otherInput, { target: { value: "Run smoke checks  " } });
    expect(submit.disabled).toBe(false);

    fireEvent.click(submit);

    expect(answerQuestion).toHaveBeenCalledWith("tool-multi", {
      "Which tasks should be included?": "Write tests, Run smoke checks",
    });
  });

  it("renders submitted answers from the store without editable controls", () => {
    seedQuestionStore({
      answeredQuestions: new Map([
        ["tool-submitted", { "Pick an option": "Already answered" }],
      ]),
    });

    render(
      <AskUserQuestionCard
        toolUseId="tool-submitted"
        input={{
          questions: [
            {
              question: "Pick an option",
              options: [
                { label: "First option" },
                { label: "Second option" },
              ],
            },
          ],
        }}
      />,
    );

    expect(screen.getByText("Answer Submitted")).toBeTruthy();
    expect(screen.getByText("Your answer")).toBeTruthy();
    expect(screen.getByText("Already answered")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /submit/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /first option/i })).toBeNull();
  });
});
