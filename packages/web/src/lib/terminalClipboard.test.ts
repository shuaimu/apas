import { ClipboardAddon } from "@xterm/addon-clipboard";
import { Terminal } from "@xterm/xterm";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  handleTerminalClipboardKey,
  PRIMARY_CLIPBOARD_SELECTION,
  SYSTEM_CLIPBOARD_SELECTION,
  WriteOnlyTerminalClipboardProvider,
} from "./terminalClipboard";

function keyEvent(
  key: string,
  modifiers: Partial<Pick<KeyboardEvent, "altKey" | "ctrlKey" | "metaKey">> = {},
): Pick<KeyboardEvent, "altKey" | "ctrlKey" | "key" | "metaKey" | "type"> {
  return {
    altKey: false,
    ctrlKey: false,
    key,
    metaKey: false,
    type: "keydown",
    ...modifiers,
  };
}

describe("handleTerminalClipboardKey", () => {
  it.each([
    ["Ctrl+V", keyEvent("v", { ctrlKey: true })],
    ["Ctrl+Shift+V", keyEvent("V", { ctrlKey: true })],
    ["Cmd+V", keyEvent("v", { metaKey: true })],
  ])("leaves %s to the browser", (_name, event) => {
    expect(handleTerminalClipboardKey(event, false)).toBe(false);
  });

  it("leaves copy to the browser when xterm owns a selection", () => {
    expect(handleTerminalClipboardKey(keyEvent("c", { ctrlKey: true }), true)).toBe(false);
  });

  it("keeps Ctrl+C as terminal interrupt without an xterm selection", () => {
    expect(handleTerminalClipboardKey(keyEvent("c", { ctrlKey: true }), false)).toBe(true);
  });

  it("does not intercept AltGr-like or unrelated terminal keys", () => {
    expect(handleTerminalClipboardKey(keyEvent("v", { altKey: true, ctrlKey: true }), false))
      .toBe(true);
    expect(handleTerminalClipboardKey(keyEvent("x", { ctrlKey: true }), false)).toBe(true);
  });
});

describe("WriteOnlyTerminalClipboardProvider", () => {
  const originalClipboard = navigator.clipboard;
  const originalExecCommand = document.execCommand;

  afterEach(() => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: originalClipboard,
    });
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: originalExecCommand,
    });
    vi.restoreAllMocks();
  });

  it("never exposes browser clipboard contents to a terminal process", () => {
    const readText = vi.fn();
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { readText, writeText: vi.fn() },
    });

    const provider = new WriteOnlyTerminalClipboardProvider();
    expect(provider.readText(SYSTEM_CLIPBOARD_SELECTION)).toBe("");
    expect(readText).not.toHaveBeenCalled();
  });

  it("writes OSC 52 system selections through the browser clipboard API", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });

    const provider = new WriteOnlyTerminalClipboardProvider();
    await provider.writeText(SYSTEM_CLIPBOARD_SELECTION, "Claude selection");

    expect(writeText).toHaveBeenCalledWith("Claude selection");
  });

  it("receives a Claude-style OSC 52 frame through the xterm addon", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    const terminal = new Terminal();
    terminal.loadAddon(
      new ClipboardAddon(undefined, new WriteOnlyTerminalClipboardProvider()),
    );

    await new Promise<void>((resolve) => {
      terminal.write("\x1b]52;c;Q2xhdWRlIHNlbGVjdGlvbg==\x07", resolve);
    });

    expect(writeText).toHaveBeenCalledWith("Claude selection");
    terminal.dispose();
  });

  it("ignores the unsupported X11 primary selection", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });

    const provider = new WriteOnlyTerminalClipboardProvider();
    await provider.writeText(PRIMARY_CLIPBOARD_SELECTION, "not system clipboard");

    expect(writeText).not.toHaveBeenCalled();
  });

  it("falls back to execCommand when the async clipboard write fails", async () => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn().mockRejectedValue(new Error("denied")) },
    });
    const execCommand = vi.fn().mockReturnValue(true);
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: execCommand,
    });

    const provider = new WriteOnlyTerminalClipboardProvider();
    await provider.writeText(SYSTEM_CLIPBOARD_SELECTION, "fallback text");

    expect(execCommand).toHaveBeenCalledWith("copy");
    expect(document.querySelector("textarea")).toBeNull();
  });
});
