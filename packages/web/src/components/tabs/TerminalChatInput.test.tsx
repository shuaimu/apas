import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore } from "@/lib/store";
import { TerminalChatInput } from "./TerminalChatInput";

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getState();

function seed(connected = true) {
  const sendTerminalConversationMessage = vi.fn(() => ({ success: true }));
  act(() => {
    useStore.setState({
      connected,
      sendTerminalConversationMessage: sendTerminalConversationMessage as StoreState["sendTerminalConversationMessage"],
    });
  });
  return sendTerminalConversationMessage;
}

function box(): HTMLTextAreaElement {
  return screen.getByPlaceholderText("Message the agent…") as HTMLTextAreaElement;
}

afterEach(() => {
  vi.restoreAllMocks();
  act(() => {
    useStore.setState({
      connected: false,
      sendTerminalConversationMessage: initialStore.sendTerminalConversationMessage as StoreState["sendTerminalConversationMessage"],
    });
  });
});

describe("TerminalChatInput", () => {
  it("submits a loggable terminal conversation message", () => {
    const send = seed();
    render(<TerminalChatInput paneId={4} />);

    fireEvent.change(box(), { target: { value: "hello agent" } });
    fireEvent.click(screen.getByText("Send"));

    expect(send).toHaveBeenCalledWith(4, "hello agent");
  });

  it("preserves multi-line text for server-side terminal framing", () => {
    const send = seed();
    render(<TerminalChatInput paneId={4} />);

    fireEvent.change(box(), { target: { value: "line one\nline two" } });
    fireEvent.click(screen.getByText("Send"));

    expect(send).toHaveBeenCalledWith(4, "line one\nline two");
  });

  it("sends on Enter and inserts a newline on Shift+Enter", () => {
    const send = seed();
    render(<TerminalChatInput paneId={4} />);
    const el = box();

    fireEvent.change(el, { target: { value: "typed" } });
    fireEvent.keyDown(el, { key: "Enter", shiftKey: true });
    expect(send).not.toHaveBeenCalled();

    fireEvent.keyDown(el, { key: "Enter" });
    expect(send).toHaveBeenCalled();
  });

  it("trims and refuses whitespace-only input", () => {
    const send = seed();
    render(<TerminalChatInput paneId={4} />);

    fireEvent.change(box(), { target: { value: "   " } });
    fireEvent.keyDown(box(), { key: "Enter" });
    expect(send).not.toHaveBeenCalled();

    fireEvent.change(box(), { target: { value: "  padded  " } });
    fireEvent.keyDown(box(), { key: "Enter" });
    expect(send).toHaveBeenCalledWith(4, "padded");
  });

  it("clears the box after sending so the next message starts empty", () => {
    seed();
    render(<TerminalChatInput paneId={4} />);
    fireEvent.change(box(), { target: { value: "one" } });
    fireEvent.keyDown(box(), { key: "Enter" });
    expect(box().value).toBe("");
  });

  it("allows drafting but sends nothing while disconnected", () => {
    const send = seed(false);
    render(<TerminalChatInput paneId={4} />);

    expect(box().disabled).toBe(false);
    fireEvent.change(box(), { target: { value: "kept draft" } });
    fireEvent.keyDown(box(), { key: "Enter" });
    expect(send).not.toHaveBeenCalled();
    expect(box().value).toBe("kept draft");
    expect(screen.queryByText(/Draft while offline/)).toBeNull();
  });

  it("keeps the composer compact without an explanatory note", () => {
    seed();
    render(<TerminalChatInput paneId={4} />);
    expect(screen.queryByText(/Switch to Terminal view/)).toBeNull();
    expect(screen.queryByText(/Sent to the terminal/)).toBeNull();
  });
});
