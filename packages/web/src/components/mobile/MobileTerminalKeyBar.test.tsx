import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "@/lib/store";
import { MobileTerminalKeyBar } from "./MobileTerminalKeyBar";
import { registerTerminalControls } from "@/lib/terminalControls";
let unregister: () => void;
beforeEach(() => { unregister = registerTerminalControls(17, (control) => {
  if (control.kind === "key") useStore.getState().sendTerminalInput(17, control.data);
  return true;
}); });

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getState();

afterEach(() => {
  unregister();
  vi.restoreAllMocks();
  act(() => {
    useStore.setState({
      sendTerminalInput: initialStore.sendTerminalInput as StoreState["sendTerminalInput"],
    });
  });
});

describe("MobileTerminalKeyBar", () => {
  it("sends byte-exact terminal control sequences to the selected pane", () => {
    const sendTerminalInput = vi.fn();
    act(() => {
      useStore.setState({
        sendTerminalInput: sendTerminalInput as StoreState["sendTerminalInput"],
      });
    });
    render(<MobileTerminalKeyBar paneId={17} connected />);

    const expected = [
      ["Send Escape", "\x1b"],
      ["Send Arrow Up", "\x1b[A"],
      ["Send Arrow Down", "\x1b[B"],
      ["Send Enter", "\r"],
      ["Send Tab", "\t"],
      ["Send Ctrl-C", "\x03"],
    ] as const;

    for (const [name, data] of expected) {
      fireEvent.click(screen.getByRole("button", { name }));
      expect(sendTerminalInput).toHaveBeenLastCalledWith(17, data);
    }
    expect(sendTerminalInput).toHaveBeenCalledTimes(expected.length);
  });

  it("disables every key while disconnected", () => {
    const sendTerminalInput = vi.fn();
    act(() => {
      useStore.setState({
        sendTerminalInput: sendTerminalInput as StoreState["sendTerminalInput"],
      });
    });
    render(<MobileTerminalKeyBar paneId={17} connected={false} />);

    const toolbar = screen.getByRole("toolbar", { name: "Terminal keys" });
    for (const button of toolbar.querySelectorAll("button")) {
      expect(button.hasAttribute("disabled")).toBe(true);
      fireEvent.click(button);
    }
    expect(sendTerminalInput).not.toHaveBeenCalled();
  });
});

it("keeps a staged draft when the terminal is restoring and pastes without submitting once ready", () => {
  sessionStorage.clear();
  const paste = vi.fn(() => false);
  unregister();
  unregister = registerTerminalControls(17, paste);
  render(<MobileTerminalKeyBar paneId={17} connected />);
  fireEvent.click(screen.getByRole("button", { name: "Text" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Terminal text draft" }), { target: { value: "line one\nline two" } });
  fireEvent.click(screen.getByRole("button", { name: "Paste into terminal" }));
  expect((screen.getByRole("textbox", { name: "Terminal text draft" }) as HTMLTextAreaElement).value).toBe("line one\nline two");
  paste.mockReturnValue(true);
  fireEvent.click(screen.getByRole("button", { name: "Paste into terminal" }));
  expect(paste).toHaveBeenLastCalledWith({ kind: "paste", text: "line one\nline two" });
  expect(screen.queryByRole("textbox", { name: "Terminal text draft" })).toBeNull();
  expect(paste.mock.calls).toHaveLength(2);
});
