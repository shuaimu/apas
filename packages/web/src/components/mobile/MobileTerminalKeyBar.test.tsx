import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore } from "@/lib/store";
import { MobileTerminalKeyBar } from "./MobileTerminalKeyBar";

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getState();

afterEach(() => {
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

  it("disables every key, with the reason, while the pane cannot take input", () => {
    // Its machine is not running the project: the server would refuse each
    // key, and each refusal would be an error toast.
    const reason = "zoo-006 is not running this project. Start it on zoo-006 first.";
    const sendTerminalInput = vi.fn();
    act(() => {
      useStore.setState({
        sendTerminalInput: sendTerminalInput as StoreState["sendTerminalInput"],
      });
    });
    render(<MobileTerminalKeyBar paneId={17} connected disabledReason={reason} />);

    const toolbar = screen.getByRole("toolbar", { name: "Terminal keys" });
    for (const button of toolbar.querySelectorAll("button")) {
      expect(button.hasAttribute("disabled")).toBe(true);
      expect(button.getAttribute("title")).toBe(reason);
      fireEvent.click(button);
    }
    expect(sendTerminalInput).not.toHaveBeenCalled();
  });
});
