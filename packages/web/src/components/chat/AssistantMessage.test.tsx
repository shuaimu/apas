import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AssistantMessage } from "./AssistantMessage";
import type { Message } from "@/lib/store";

function assistantMessage(overrides: Partial<Message>): Message {
  return {
    id: "msg-1",
    role: "assistant",
    content: "",
    timestamp: new Date("2026-06-17T12:00:00Z"),
    ...overrides,
  };
}

describe("AssistantMessage tool dispatch", () => {
  it("routes AskUserQuestion tool_use output to AskUserQuestionCard", () => {
    render(
      <AssistantMessage
        message={assistantMessage({
          outputType: {
            type: "tool_use",
            tool: "AskUserQuestion",
            toolUseId: "tool-question",
            input: {
              questions: [
                {
                  question: "Which path should we take?",
                  options: [
                    { label: "Safe path" },
                    { label: "Fast path" },
                  ],
                },
              ],
            },
          },
        })}
      />,
    );

    expect(screen.getByText("Claude is asking")).toBeTruthy();
    expect(screen.getByText("Which path should we take?")).toBeTruthy();
    expect(screen.getByRole("button", { name: /safe path/i })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /using askuserquestion/i })).toBeNull();
  });

  it("routes generic tool_use output to ToolCard", () => {
    const { container } = render(
      <AssistantMessage
        message={assistantMessage({
          outputType: {
            type: "tool_use",
            tool: "Bash",
            input: "npm test",
          },
        })}
      />,
    );

    expect(screen.getByRole("button", { name: /using bash/i })).toBeTruthy();
    expect(container.textContent).not.toContain("npm test");

    fireEvent.click(screen.getByRole("button", { name: /using bash/i }));

    expect(screen.getByText("npm test")).toBeTruthy();
  });

  it("hides AskUserQuestion tool_result output", () => {
    render(
      <AssistantMessage
        message={assistantMessage({
          content: "raw AskUserQuestion result payload",
          outputType: {
            type: "tool_result",
            tool: "AskUserQuestion",
            success: true,
          },
        })}
      />,
    );

    expect(screen.queryByText("raw AskUserQuestion result payload")).toBeNull();
    expect(screen.queryByRole("button", { name: /askuserquestion succeeded/i })).toBeNull();
  });

  it("routes successful generic tool_result output to ToolCard", () => {
    render(
      <AssistantMessage
        message={assistantMessage({
          content: "command output",
          outputType: {
            type: "tool_result",
            tool: "Bash",
            success: true,
          },
        })}
      />,
    );

    expect(screen.getByRole("button", { name: /bash succeeded/i })).toBeTruthy();
    expect(screen.queryByText("command output")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /bash succeeded/i }));

    expect(screen.getByText("command output")).toBeTruthy();
  });

  it("routes failed generic tool_result output to ToolCard", () => {
    render(
      <AssistantMessage
        message={assistantMessage({
          content: "permission denied",
          outputType: {
            type: "tool_result",
            tool: "Edit",
            success: false,
          },
        })}
      />,
    );

    expect(screen.getByRole("button", { name: /edit failed/i })).toBeTruthy();
    expect(screen.queryByText("permission denied")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /edit failed/i }));

    expect(screen.getByText("permission denied")).toBeTruthy();
  });
});
