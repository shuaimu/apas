import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { Message, OutputType } from "@/lib/store";
import { AUTOPILOT_ITERATION_TOOL_NAME } from "@/lib/autopilot";
import { AssistantMessage } from "./AssistantMessage";
import { groupMessagesForRender, isToolLikeMessage } from "./ToolGroupCard";

const AUTOPILOT = "60000000-0000-4000-8000-000000000001";

function message(id: string, outputType: OutputType, content = ""): Message {
  return { id, role: "assistant", content, timestamp: new Date("2026-11-13T14:15:00Z"), outputType };
}

function marker(id: string, input: Record<string, unknown>): Message {
  return message(id, {
    type: "tool_use",
    tool: AUTOPILOT_ITERATION_TOOL_NAME,
    input: { autopilot_id: AUTOPILOT, ...input },
    toolUseId: `${id}-tu`,
  });
}

describe("the conversation's autopilot iteration marker", () => {
  it("says which iteration starts, with and without an iteration limit", () => {
    const { unmount } = render(<AssistantMessage message={marker("m3", { iteration: 3, limit: 20 })} />);
    expect(screen.getByTestId("autopilot-iteration-marker").textContent).toContain("Autopilot · iteration 3 of 20");
    unmount();

    render(<AssistantMessage message={marker("m3", { iteration: 3 })} />);
    const line = screen.getByTestId("autopilot-iteration-marker");
    expect(line.textContent).toContain("Autopilot · iteration 3");
    expect(line.textContent).not.toContain(" of ");
  });

  it("is a line of its own: no tool card, no message bubble, no prompt on a later marker", () => {
    const { container } = render(<AssistantMessage message={marker("m3", { iteration: 3, limit: 20 })} />);
    const line = screen.getByTestId("autopilot-iteration-marker");
    // The marker is the whole message: nothing wraps it as the agent's words
    // or as a call to a tool named ApasAutopilotIteration.
    expect(container.firstElementChild).toBe(line);
    expect(container.textContent).not.toContain(AUTOPILOT_ITERATION_TOOL_NAME);
    expect(container.textContent).not.toContain(AUTOPILOT);
    expect(container.querySelector("details")).toBeNull();
    expect(container.textContent).not.toMatch(/bot|worker/i);
  });

  it("opens the prompt behind a marker that carries one", () => {
    render(<AssistantMessage message={marker("m1", {
      iteration: 1,
      limit: 20,
      prompt: "Check the build and fix what fails.",
    })} />);
    const line = screen.getByTestId("autopilot-iteration-marker");
    const details = line.querySelector("details") as HTMLDetailsElement;
    expect(details.open).toBe(false);
    fireEvent.click(screen.getByText("Prompt"));
    expect(details.textContent).toContain("Check the build and fix what fails.");
  });

  it("falls back to the generic tool entry for a block it cannot read", () => {
    render(<AssistantMessage message={marker("bad", { limit: 20 })} />);
    expect(screen.queryByTestId("autopilot-iteration-marker")).toBeNull();
  });

  it("is never folded into a tool-call group", () => {
    const toolUse = (id: string) => message(id, { type: "tool_use", tool: "Bash", input: {}, toolUseId: `${id}-tu` });
    const toolResult = (id: string) => message(id, { type: "tool_result", tool: "Bash", success: true }, "ok");
    expect(isToolLikeMessage(marker("m1", { iteration: 1 }))).toBe(false);
    expect(isToolLikeMessage(toolUse("u0"))).toBe(true);

    // Two iterations that each called a tool, then one that called none.
    const items = groupMessagesForRender([
      marker("m1", { iteration: 1 }),
      toolUse("u1"),
      toolResult("r1"),
      marker("m2", { iteration: 2 }),
      toolUse("u2"),
      toolResult("r2"),
      marker("m3", { iteration: 3 }),
      marker("m4", { iteration: 4 }),
    ]);
    expect(items.map((item) => item.kind === "message" ? item.message.id : item.items.map((m) => m.id).join("+")))
      .toEqual(["m1", "u1+r1", "m2", "u2+r2", "m3", "m4"]);
  });
});
