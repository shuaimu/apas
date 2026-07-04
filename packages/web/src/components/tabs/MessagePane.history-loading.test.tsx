import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useStore, type Message } from "@/lib/store";
import { MessagePane } from "./TabbedView";

const initialStore = useStore.getInitialState();
const PANE_ID = 88;

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

describe("MessagePane history-loading placeholder", () => {
  beforeEach(() => {
    Element.prototype.scrollIntoView = vi.fn();
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      callback(0);
      return 0;
    });
    act(() => {
      useStore.setState({ sessionId: "session-history-loading" });
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

  it("shows the fetching-earlier-history placeholder while paging older history", () => {
    render(
      <MessagePane
        paneId={PANE_ID}
        messages={makeMessages(5)}
        onLoadMore={vi.fn()}
        isLoading={true}
        hasMore={true}
        isActive={true}
      />,
    );

    expect(screen.getByTestId(`history-loading-${PANE_ID}`)).toBeTruthy();
    expect(screen.getByText(/Fetching earlier history/)).toBeTruthy();
    // The old messages stay rendered above the placeholder.
    expect(screen.getByText("message 1")).toBeTruthy();
  });

  it("hides the placeholder when not loading", () => {
    render(
      <MessagePane
        paneId={PANE_ID}
        messages={makeMessages(5)}
        onLoadMore={vi.fn()}
        isLoading={false}
        hasMore={true}
        isActive={true}
      />,
    );

    expect(screen.queryByTestId(`history-loading-${PANE_ID}`)).toBeNull();
    expect(screen.queryByText(/Fetching earlier history/)).toBeNull();
  });
});
