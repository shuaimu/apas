import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useStore, type Message } from "@/lib/store";
import { MessageList } from "./MessageList";

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getInitialState();
let sessionCounter = 0;

function message(overrides: Partial<Message>): Message {
  return {
    id: "msg",
    role: "assistant",
    content: "",
    timestamp: new Date(2026, 5, 17, 9, 0),
    outputType: { type: "text" },
    ...overrides,
  };
}

function seedMessageList(overrides: {
  messages?: Message[];
  hasMoreMessages?: boolean;
  isLoadingMore?: boolean;
  loadMoreMessages?: ReturnType<typeof vi.fn>;
} = {}) {
  const loadMoreMessages = overrides.loadMoreMessages ?? vi.fn();

  act(() => {
    useStore.setState({
      messages: overrides.messages ?? [],
      sessionId: `session-message-list-${sessionCounter++}`,
      hasMoreMessages: overrides.hasMoreMessages ?? false,
      isLoadingMore: overrides.isLoadingMore ?? false,
      loadMoreMessages: loadMoreMessages as StoreState["loadMoreMessages"],
    });
  });

  const result = render(<MessageList />);
  return { ...result, loadMoreMessages };
}

function setScrollMetrics(
  element: Element,
  metrics: { scrollTop: number; scrollHeight: number; clientHeight: number },
) {
  Object.defineProperty(element, "scrollTop", {
    configurable: true,
    value: metrics.scrollTop,
    writable: true,
  });
  Object.defineProperty(element, "scrollHeight", {
    configurable: true,
    value: metrics.scrollHeight,
  });
  Object.defineProperty(element, "clientHeight", {
    configurable: true,
    value: metrics.clientHeight,
  });
}

describe("MessageList", () => {
  beforeEach(() => {
    Object.defineProperty(Element.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn(),
    });
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      callback(0);
      return 0;
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.clearAllMocks();
    delete (Element.prototype as Element & { scrollIntoView?: unknown })
      .scrollIntoView;
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("renders the empty state", () => {
    seedMessageList();

    expect(screen.getByText("No messages yet")).toBeTruthy();
    expect(screen.getByText("Start a conversation with Claude")).toBeTruthy();
  });

  it("renders user, assistant, and system messages in order", () => {
    const { container } = seedMessageList({
      messages: [
        message({ id: "user-1", role: "user", content: "User asks first" }),
        message({
          id: "assistant-1",
          role: "assistant",
          content: "Assistant replies second",
        }),
        message({
          id: "system-1",
          role: "system",
          content: "System notes third",
        }),
      ],
    });

    const transcript = container.textContent ?? "";

    expect(transcript.indexOf("User asks first")).toBeLessThan(
      transcript.indexOf("Assistant replies second"),
    );
    expect(transcript.indexOf("Assistant replies second")).toBeLessThan(
      transcript.indexOf("System notes third"),
    );
  });

  it("renders the load-more hint when more messages are available", () => {
    seedMessageList({
      hasMoreMessages: true,
      messages: [message({ id: "msg-1", content: "Existing message" })],
    });

    expect(screen.getByText("Scroll up to load more")).toBeTruthy();
    expect(screen.queryByText("Loading older messages...")).toBeNull();
  });

  it("renders the loading older messages state", () => {
    seedMessageList({
      hasMoreMessages: true,
      isLoadingMore: true,
      messages: [message({ id: "msg-1", content: "Existing message" })],
    });

    expect(screen.getByText("Loading older messages...")).toBeTruthy();
    expect(screen.queryByText("Scroll up to load more")).toBeNull();
  });

  it("loads more once when scrolled near the top", () => {
    const loadMoreMessages = vi.fn();
    const { container } = seedMessageList({
      hasMoreMessages: true,
      loadMoreMessages,
      messages: [message({ id: "msg-1", content: "Existing message" })],
    });
    const scrollContainer = container.firstElementChild;

    expect(scrollContainer).toBeTruthy();
    setScrollMetrics(scrollContainer!, {
      clientHeight: 400,
      scrollHeight: 1200,
      scrollTop: 20,
    });

    fireEvent.scroll(scrollContainer!);

    expect(loadMoreMessages).toHaveBeenCalledTimes(1);
  });
});
