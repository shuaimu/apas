import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CodeBlock } from "./CodeBlock";

vi.mock("react-syntax-highlighter", async () => {
  const React = await import("react");

  return {
    Prism: ({
      children,
      language,
    }: {
      children: string;
      language?: string;
    }) =>
      React.createElement(
        "pre",
        {
          "data-language": language,
          "data-testid": "syntax-highlighter",
        },
        children,
      ),
  };
});

vi.mock("react-syntax-highlighter/dist/esm/styles/prism", () => ({
  oneDark: {},
}));

function mockClipboard() {
  const writeText = vi.fn(() => Promise.resolve());

  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText },
  });

  return writeText;
}

describe("CodeBlock", () => {
  beforeEach(() => {
    mockClipboard();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.clearAllMocks();
  });

  it("renders the requested language and code content", () => {
    const code = 'console.log("hello");';

    render(<CodeBlock code={code} language="typescript" />);

    expect(screen.getByText("typescript")).toBeTruthy();
    expect(screen.getByTestId("syntax-highlighter").textContent).toBe(code);
    expect(screen.getByTestId("syntax-highlighter").dataset.language).toBe(
      "typescript",
    );
  });

  it("defaults the language to text", () => {
    render(<CodeBlock code="plain output" />);

    expect(screen.getByText("text")).toBeTruthy();
    expect(screen.getByTestId("syntax-highlighter").dataset.language).toBe(
      "text",
    );
  });

  it("copies exact code and resets the copied state after the timeout", async () => {
    vi.useFakeTimers();
    const writeText = mockClipboard();
    const code = "cargo test --all-targets";

    render(<CodeBlock code={code} language="bash" />);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Copy" }));
    });

    expect(writeText).toHaveBeenCalledTimes(1);
    expect(writeText).toHaveBeenCalledWith(code);
    expect(screen.getByRole("button", { name: "Copied!" })).toBeTruthy();

    act(() => {
      vi.advanceTimersByTime(2000);
    });

    expect(screen.getByRole("button", { name: "Copy" })).toBeTruthy();
  });
});
