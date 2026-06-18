import { act, createEvent, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore } from "@/lib/store";
import { InputBox } from "./InputBox";

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getInitialState();

function renderInputBox(connected = true) {
  const sendMessage = vi.fn();

  act(() => {
    useStore.setState({
      connected,
      sendMessage: sendMessage as StoreState["sendMessage"],
    });
  });

  render(<InputBox />);

  return {
    sendMessage,
    textarea: screen.getByRole("textbox") as HTMLTextAreaElement,
    sendButton: screen.getByRole("button", {
      name: "Send message",
    }) as HTMLButtonElement,
  };
}

describe("InputBox", () => {
  afterEach(() => {
    vi.clearAllMocks();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("sends trimmed text on click and clears the textarea", () => {
    const { sendMessage, sendButton, textarea } = renderInputBox();

    fireEvent.change(textarea, { target: { value: "  Ship it  " } });
    fireEvent.click(sendButton);

    expect(sendMessage).toHaveBeenCalledTimes(1);
    expect(sendMessage).toHaveBeenCalledWith("Ship it");
    expect(textarea.value).toBe("");
  });

  it("sends once on Enter and clears the textarea", () => {
    const { sendMessage, textarea } = renderInputBox();

    fireEvent.change(textarea, { target: { value: "send from keyboard" } });
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter" });

    expect(sendMessage).toHaveBeenCalledTimes(1);
    expect(sendMessage).toHaveBeenCalledWith("send from keyboard");
    expect(textarea.value).toBe("");
  });

  it("keeps Shift+Enter as a newline without sending", () => {
    const { sendMessage, textarea } = renderInputBox();

    fireEvent.change(textarea, { target: { value: "first line" } });
    const shiftEnter = createEvent.keyDown(textarea, {
      key: "Enter",
      code: "Enter",
      shiftKey: true,
    });
    fireEvent(textarea, shiftEnter);
    fireEvent.change(textarea, { target: { value: "first line\n" } });

    expect(shiftEnter.defaultPrevented).toBe(false);
    expect(sendMessage).not.toHaveBeenCalled();
    expect(textarea.value).toBe("first line\n");
  });

  it("disables input while disconnected and shows the connect placeholder", () => {
    const { sendButton, textarea } = renderInputBox(false);

    expect(textarea.placeholder).toBe("Connect to start chatting");
    expect(textarea.disabled).toBe(true);
    expect(sendButton.disabled).toBe(true);
  });

  it("keeps whitespace-only input disabled and does not send", () => {
    const { sendMessage, sendButton, textarea } = renderInputBox();

    fireEvent.change(textarea, { target: { value: "   " } });
    fireEvent.click(sendButton);

    expect(sendButton.disabled).toBe(true);
    expect(sendMessage).not.toHaveBeenCalled();
  });
});
