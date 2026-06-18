import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ToolCard } from "./ToolCard";

describe("ToolCard", () => {
  it("starts structured tool-use input collapsed and expands pretty-printed JSON", () => {
    const { container } = render(
      <ToolCard
        type="use"
        tool="Read"
        input={{ file_path: "/tmp/report.md", limit: 25 }}
      />,
    );

    expect(screen.getByRole("button", { name: /using read/i })).toBeTruthy();
    expect(container.textContent).not.toContain("file_path");

    fireEvent.click(screen.getByRole("button", { name: /using read/i }));

    expect(container.textContent).toContain('"file_path": "/tmp/report.md"');
    expect(container.textContent).toContain('"limit": 25');
  });

  it("expands string tool-use input as raw text", () => {
    render(
      <ToolCard
        type="use"
        tool="Bash"
        input="npm test -- --runInBand"
      />,
    );

    expect(screen.queryByText("npm test -- --runInBand")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /using bash/i }));

    expect(screen.getByText("npm test -- --runInBand")).toBeTruthy();
  });

  it("renders successful tool results with expandable result text", () => {
    render(
      <ToolCard
        type="result"
        tool="Bash"
        success={true}
        result="All tests passed"
      />,
    );

    expect(screen.getByRole("button", { name: /bash succeeded/i })).toBeTruthy();
    expect(screen.queryByText("All tests passed")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /bash succeeded/i }));

    expect(screen.getByText("All tests passed")).toBeTruthy();
  });

  it("renders failed tool results with expandable result text", () => {
    render(
      <ToolCard
        type="result"
        tool="Edit"
        success={false}
        result="Permission denied"
      />,
    );

    expect(screen.getByRole("button", { name: /edit failed/i })).toBeTruthy();
    expect(screen.queryByText("Permission denied")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /edit failed/i }));

    expect(screen.getByText("Permission denied")).toBeTruthy();
  });

  it("renders unknown tool names without crashing", () => {
    const { container } = render(
      <ToolCard
        type="use"
        tool="MysteryTool"
        input={{ payload: "ok" }}
      />,
    );

    expect(screen.getByRole("button", { name: /using mysterytool/i })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /using mysterytool/i }));

    expect(container.textContent).toContain('"payload": "ok"');
  });
});
