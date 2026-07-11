import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useStore, type Message } from "@/lib/store";
import { computeHiddenCount, INITIAL_RENDER_CAP, MessagePane, RENDER_CAP_STEP } from "./TabbedView";

const initialStore = useStore.getInitialState();
const PANE_ID = 77;

function makeMessages(count: number): Message[] {
  return Array.from({ length: count }, (_, i) => {
    const index = i + 1;
    return {
      id: `message-${index}`,
      role: "user",
      content: `message ${index}`,
      timestamp: new Date(Date.UTC(2026, 0, 1, 0, 0, i)),
      outputType: { type: "text" },
    };
  });
}

function setScrollMetric(
  element: HTMLElement,
  key: "scrollTop" | "scrollHeight" | "clientHeight",
  value: number,
): void {
  Object.defineProperty(element, key, {
    configurable: true,
    writable: true,
    value,
  });
}

describe("MessagePane render cap", () => {
  beforeEach(() => {
    Element.prototype.scrollIntoView = vi.fn();
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      callback(0);
      return 0;
    });
    act(() => {
      useStore.setState({
        sessionId: "session-render-cap",
      });
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    localStorage.clear();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("initially mounts the newest messages and reveals one local chunk on demand", () => {
    const messages = makeMessages(INITIAL_RENDER_CAP + RENDER_CAP_STEP + 40);

    render(
      <MessagePane
        paneId={PANE_ID}
        messages={messages}
        isActive={true}
      />,
    );

    expect(screen.queryByText("message 90")).toBeNull();
    expect(screen.getByText("message 91")).toBeTruthy();
    expect(screen.getByText("message 120")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Show earlier messages (90)" }),
    ).toBeTruthy();

    fireEvent.click(
      screen.getByRole("button", { name: "Show earlier messages (90)" }),
    );

    expect(screen.queryByText("message 40")).toBeNull();
    expect(screen.getByText("message 41")).toBeTruthy();
    expect(screen.getByText("message 120")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Show earlier messages (40)" }),
    ).toBeTruthy();
  });

  it("reveals cached local backlog on scroll before paging the server", () => {
    let now = 1000;
    vi.spyOn(Date, "now").mockImplementation(() => now);
    const onLoadMore = vi.fn();

    render(
      <MessagePane
        paneId={PANE_ID}
        messages={makeMessages(INITIAL_RENDER_CAP + RENDER_CAP_STEP)}
        onLoadMore={onLoadMore}
        hasMore={true}
        isActive={true}
      />,
    );

    const scroller = screen.getByTestId(`message-pane-${PANE_ID}`);
    setScrollMetric(scroller, "scrollTop", 0);
    setScrollMetric(scroller, "scrollHeight", 1000);
    setScrollMetric(scroller, "clientHeight", 500);

    fireEvent.scroll(scroller);

    expect(onLoadMore).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: /Show earlier messages/ })).toBeNull();
    expect(screen.getByText("message 1")).toBeTruthy();
    expect(screen.getByText("message 80")).toBeTruthy();

    now = 1300;
    setScrollMetric(scroller, "scrollTop", 0);
    fireEvent.scroll(scroller);

    expect(onLoadMore).toHaveBeenCalledOnce();
  });
});

describe("computeHiddenCount — tool messages don't consume the reveal budget", () => {
  const textMsg = (id: string): Message => ({
    id, role: "assistant", content: id, timestamp: new Date(), outputType: { type: "text" },
  });
  const toolMsg = (id: string, kind: "tool_use" | "tool_result"): Message => ({
    id, role: "assistant", content: id, timestamp: new Date(),
    outputType: kind === "tool_use"
      ? { type: "tool_use", tool: "Bash", input: {} }
      : { type: "tool_result", tool: "Bash", success: true },
  });
  const askMsg = (id: string): Message => ({
    id, role: "assistant", content: id, timestamp: new Date(),
    outputType: { type: "tool_use", tool: "AskUserQuestion", input: {} },
  });

  it("counts only non-tool messages toward the cap", () => {
    const msgs = [
      textMsg("t0"), toolMsg("u1", "tool_use"), toolMsg("r2", "tool_result"),
      textMsg("t3"), toolMsg("u4", "tool_use"), toolMsg("r5", "tool_result"),
      textMsg("t6"),
    ];
    // Budget of 2 text messages -> show from t3 onward (t3, its tools, t6).
    // Old count-everything behavior would have hidden 5, leaving 1 text.
    expect(computeHiddenCount(msgs, 2)).toBe(3);
  });

  it("shows everything when non-tool messages are fewer than the cap", () => {
    const msgs = [textMsg("t0"), toolMsg("u1", "tool_use"), textMsg("t2")];
    expect(computeHiddenCount(msgs, 30)).toBe(0);
  });

  it("keeps AskUserQuestion counted as a normal (budget-consuming) message", () => {
    const msgs = [textMsg("t0"), askMsg("ask1"), textMsg("t2")];
    expect(computeHiddenCount(msgs, 1)).toBe(2); // only t2
    expect(computeHiddenCount(msgs, 2)).toBe(1); // ask1 + t2
  });
});
