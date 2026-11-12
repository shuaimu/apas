import { act, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emitTerminal } from "@/lib/terminalBus";

/**
 * A terminal tab whose machine is not running the project, or that this
 * viewer may not use, keeps its xterm so the retained screen still reads. It
 * must not send keystrokes or sizes then: the server refuses each one, and
 * every refusal becomes an error toast — two of them just for opening the
 * tab, then one per key.
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
  onData: vi.fn(),
  hasSelection: vi.fn(() => false),
  options: {} as Record<string, unknown>,
  cols: 80,
  rows: 24,
}));

// Invoked with `new`, so each mock must be a real function.
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

const STOPPED = "zoo-006 is not running this project. Start it on zoo-006 first.";

/** The keystroke handler the pane registered with xterm. */
let typed: (data: string) => void;
/** The pane's ResizeObserver callback: a layout change. */
let resized: () => void;

describe("TerminalPane input for a tab that cannot take it", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    term.cols = 80;
    term.rows = 24;
    term.onData.mockImplementation((handler: (data: string) => void) => {
      typed = handler;
      return { dispose: vi.fn() };
    });
    globalThis.ResizeObserver = class {
      constructor(callback: () => void) {
        resized = callback;
      }
      observe() {}
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
    // jsdom lays everything out at 0x0, which the pane treats as hidden.
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(640);
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(384);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    document.body.innerHTML = "";
  });

  it("still attaches, but sends no size or keystroke while its machine cannot take them", () => {
    const { container } = render(<TerminalPane paneId={11} inputDisabledReason={STOPPED} />);
    // The retained screen is still requested.
    expect(storeState.attachTerminal).toHaveBeenCalledWith(11);

    term.cols = 120;
    resized();
    typed("a");
    typed("\r");

    expect(storeState.sendTerminalResize).not.toHaveBeenCalled();
    expect(storeState.sendTerminalInput).not.toHaveBeenCalled();
    expect(container.querySelector(`[title="${STOPPED}"]`)).toBeTruthy();
  });

  it("reports its size once the machine runs the project again, then takes keys", () => {
    const { rerender } = render(<TerminalPane paneId={11} inputDisabledReason={STOPPED} />);
    expect(storeState.sendTerminalResize).not.toHaveBeenCalled();

    rerender(<TerminalPane paneId={11} />);
    // The size withheld meanwhile goes out now, once.
    expect(storeState.sendTerminalResize).toHaveBeenCalledTimes(1);
    expect(storeState.sendTerminalResize).toHaveBeenCalledWith(11, 80, 24);
    // The terminal was not recreated, so its scrollback survives.
    expect(term.open).toHaveBeenCalledTimes(1);
    expect(term.dispose).not.toHaveBeenCalled();

    typed("a");
    expect(storeState.sendTerminalInput).toHaveBeenCalledWith(11, "a");
  });

  it("stops sending as soon as its machine stops", () => {
    const { rerender } = render(<TerminalPane paneId={11} />);
    expect(storeState.sendTerminalResize).toHaveBeenCalledWith(11, 80, 24);
    typed("a");
    expect(storeState.sendTerminalInput).toHaveBeenCalledWith(11, "a");
    vi.clearAllMocks();

    rerender(<TerminalPane paneId={11} inputDisabledReason={STOPPED} />);
    // The refusal banner that arrives with it resizes the terminal.
    term.rows = 20;
    resized();
    typed("b");

    expect(storeState.sendTerminalResize).not.toHaveBeenCalled();
    expect(storeState.sendTerminalInput).not.toHaveBeenCalled();
    expect(term.dispose).not.toHaveBeenCalled();
  });

  // A stopped machine's terminals are disconnected, under a banner saying the
  // machine is not running the project: the line above the terminal must not
  // promise a reconnect.
  it("does not promise a reconnect while its machine is not running the project", () => {
    const { container, rerender } = render(
      <TerminalPane paneId={11} inputDisabledReason={STOPPED} />,
    );
    act(() => emitTerminal(11, snapshot("running", "retained screen")));
    act(() => emitTerminal(11, { kind: "state", lifecycle: "disconnected" }));
    expect(container.textContent).toContain("Showing retained terminal output.");
    expect(container.textContent).not.toContain("reconnects");

    // A tab that can be used keeps the reconnect wording.
    rerender(<TerminalPane paneId={11} />);
    expect(container.textContent).toContain("while it reconnects");
  });

  // After a server restart the server retains nothing for such a tab: a
  // terminal opened then is blank, while one that stayed open keeps its screen.
  it("says it shows retained output only while it shows some", () => {
    const open = render(<TerminalPane paneId={12} inputDisabledReason={STOPPED} />);
    act(() => emitTerminal(12, snapshot("running", "screen before the restart", 4)));
    act(() => emitTerminal(12, snapshot("unknown", "", 0)));
    expect(open.container.textContent).toContain("Showing retained terminal output.");
    open.unmount();

    const fresh = render(<TerminalPane paneId={13} inputDisabledReason={STOPPED} />);
    act(() => emitTerminal(13, snapshot("unknown", "", 0)));
    expect(fresh.container.textContent).not.toContain("retained");
    act(() => emitTerminal(13, { kind: "state", lifecycle: "disconnected" }));
    expect(fresh.container.textContent).not.toContain("retained");
  });
});

function snapshot(
  lifecycle: "running" | "unknown",
  text: string,
  seq = 1,
): Parameters<typeof emitTerminal>[1] {
  return {
    kind: "snapshot",
    bytes: new TextEncoder().encode(text),
    seq,
    truncated: false,
    instanceId: lifecycle === "running" ? "pty-1" : undefined,
    lifecycle,
  };
}
