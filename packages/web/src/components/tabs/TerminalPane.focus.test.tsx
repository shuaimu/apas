import { render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * Switching tabs must not strand keyboard focus.
 *
 * Visited tabs stay mounted and are hidden with `display: none`, which is
 * deliberate — unmounting would tear down the xterm instance and force a
 * re-attach. But hiding a subtree blurs whatever had focus inside it, and the
 * terminal's setup effect runs only once per pane. So returning to a tab you
 * had already opened left focus on the document body and typing went nowhere.
 */

const term = vi.hoisted(() => ({
  focus: vi.fn(),
  open: vi.fn(),
  write: vi.fn(),
  reset: vi.fn(),
  refresh: vi.fn(),
  dispose: vi.fn(),
  loadAddon: vi.fn(),
  attachCustomKeyEventHandler: vi.fn(),
  onData: vi.fn(() => ({ dispose: vi.fn() })),
  hasSelection: vi.fn(() => false),
  options: {} as Record<string, unknown>,
  cols: 80,
  rows: 24,
}));

// These are all invoked with `new`, so each mock must be a real function; an
// arrow function is not a constructor.
vi.mock("@xterm/xterm", () => ({
  Terminal: function Terminal() {
    return term;
  },
}));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: function FitAddon() {
    return { fit: vi.fn(), activate: vi.fn(), dispose: vi.fn() };
  },
}));
vi.mock("@xterm/addon-clipboard", () => ({
  ClipboardAddon: function ClipboardAddon() {
    return { activate: vi.fn(), dispose: vi.fn() };
  },
}));
vi.mock("@xterm/addon-webgl", () => ({
  WebglAddon: function WebglAddon() {
    return { activate: vi.fn(), dispose: vi.fn(), onContextLoss: vi.fn() };
  },
}));
vi.mock("@xterm/xterm/css/xterm.css", () => ({}));

const storeState = vi.hoisted(() => ({
  attachTerminal: vi.fn(),
  sendTerminalInput: vi.fn(),
  sendTerminalResize: vi.fn(),
  connected: true,
}));

vi.mock("@/lib/store", () => ({
  useStore: (selector: (s: typeof storeState) => unknown) => selector(storeState),
}));

vi.mock("@/lib/useTheme", () => ({
  useTheme: () => ({ theme: "system", isDark: true }),
}));

import { TerminalPane } from "./TerminalPane";

describe("TerminalPane focus across tab switches", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // jsdom has no ResizeObserver, which the pane observes its container with.
    globalThis.ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
  });

  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("focuses the terminal when a pane it had already opened becomes visible again", () => {
    const { rerender } = render(<TerminalPane paneId={7} visible />);
    // Mounting an on-screen pane focuses it; that part already worked.
    expect(term.focus).toHaveBeenCalledTimes(1);

    // Switch away. The pane stays mounted and is hidden by its parent.
    rerender(<TerminalPane paneId={7} visible={false} />);
    expect(term.focus).toHaveBeenCalledTimes(1);

    // Switch back. This is the regression: without the visibility edge the
    // setup effect never re-runs, so nothing ever refocused.
    rerender(<TerminalPane paneId={7} visible />);
    expect(term.focus).toHaveBeenCalledTimes(2);
  });

  it("does not steal focus while the pane stays hidden or stays visible", () => {
    const { rerender } = render(<TerminalPane paneId={7} visible={false} />);
    const afterMount = term.focus.mock.calls.length;

    // Re-renders that do not change visibility must not grab focus, or the
    // pane would fight the user clicking into a composer or dialog.
    rerender(<TerminalPane paneId={7} visible={false} />);
    expect(term.focus).toHaveBeenCalledTimes(afterMount);

    rerender(<TerminalPane paneId={7} visible />);
    expect(term.focus).toHaveBeenCalledTimes(afterMount + 1);
    rerender(<TerminalPane paneId={7} visible />);
    expect(term.focus).toHaveBeenCalledTimes(afterMount + 1);
  });

  it("defaults to visible, so a caller that mounts and unmounts still focuses", () => {
    // The mobile session view renders the pane only while it is open, so it
    // has no visibility flag to pass.
    render(<TerminalPane paneId={7} />);
    expect(term.focus).toHaveBeenCalledTimes(1);
  });
});
