import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore } from "@/lib/store";
import { TerminalChatInput } from "./TerminalChatInput";

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getState();

function seed(connected = true) {
  const sendTerminalInput = vi.fn();
  act(() => {
    useStore.setState({
      connected,
      sendTerminalInput: sendTerminalInput as StoreState["sendTerminalInput"],
    });
  });
  return sendTerminalInput;
}

function box(): HTMLTextAreaElement {
  return screen.getByPlaceholderText(/Message the agent|Disconnected/) as HTMLTextAreaElement;
}

afterEach(() => {
  vi.restoreAllMocks();
  act(() => {
    useStore.setState({
      connected: false,
      sendTerminalInput: initialStore.sendTerminalInput as StoreState["sendTerminalInput"],
    });
  });
});

describe("TerminalChatInput", () => {
  it("sends the text then a separate carriage return", () => {
    // The CR is separate so the TUI sees a deliberate submit after the text
    // has landed, rather than a newline buried in the payload.
    const send = seed();
    render(<TerminalChatInput paneId={4} />);

    fireEvent.change(box(), { target: { value: "hello agent" } });
    fireEvent.click(screen.getByText("Send"));

    expect(send.mock.calls).toEqual([
      [4, "hello agent"],
      [4, "\r"],
    ]);
  });

  it("wraps multi-line text in a bracketed paste", () => {
    // Without this the TUI treats the first newline as "submit", firing line
    // one as a whole message and leaving the rest behind.
    const send = seed();
    render(<TerminalChatInput paneId={4} />);

    fireEvent.change(box(), { target: { value: "line one\nline two" } });
    fireEvent.click(screen.getByText("Send"));

    expect(send.mock.calls[0]).toEqual([4, "\x1b[200~line one\nline two\x1b[201~"]);
    expect(send.mock.calls[1]).toEqual([4, "\r"]);
  });

  it("does NOT bracket single-line text", () => {
    // A TUI that never enabled bracketed paste (DECSET 2004) would show the
    // wrapper as literal keystrokes. Keeping the common case unwrapped means
    // it cannot be corrupted by that.
    const send = seed();
    render(<TerminalChatInput paneId={4} />);

    fireEvent.change(box(), { target: { value: "plain" } });
    fireEvent.click(screen.getByText("Send"));

    expect(String(send.mock.calls[0][1])).not.toContain("\x1b[200~");
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
    expect(send.mock.calls[0]).toEqual([4, "padded"]);
  });

  it("clears the box after sending so the next message starts empty", () => {
    seed();
    render(<TerminalChatInput paneId={4} />);
    fireEvent.change(box(), { target: { value: "one" } });
    fireEvent.keyDown(box(), { key: "Enter" });
    expect(box().value).toBe("");
  });

  it("sends nothing while disconnected", () => {
    // Silently dropping into a dead socket would look like the agent ignored
    // the message.
    const send = seed(false);
    render(<TerminalChatInput paneId={4} />);

    expect(box().disabled).toBe(true);
    fireEvent.change(box(), { target: { value: "lost" } });
    fireEvent.keyDown(box(), { key: "Enter" });
    expect(send).not.toHaveBeenCalled();
  });

  it("says where the live state is, since this sends blind", () => {
    seed();
    render(<TerminalChatInput paneId={4} />);
    expect(screen.getByText(/Switch to Terminal view/)).toBeTruthy();
  });
});
