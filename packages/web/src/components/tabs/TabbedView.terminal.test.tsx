// A terminal pane takes keystrokes in xterm.js, not in the chat composer.
// Leaving the composer mounted under one isn't just redundant UI: its text
// goes down the agent input path, and a terminal pane has no input channel
// on the CLI side, so the send would trigger the missing-channel pane
// recovery instead of reaching the pty.
import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { paneKey, useStore, type PaneConfig } from "@/lib/store";
import { TabbedView } from "./TabbedView";

const SESSION_ID = "session-terminal";
const CLI_CLIENT_ID = "cli-terminal";
const initialStore = useStore.getInitialState();

// xterm.js needs canvas/WebGL APIs jsdom doesn't provide. The point of
// these tests is the surrounding TabbedView wiring, so stub the pane.
vi.mock("./TerminalPane", () => ({
  TerminalPane: ({ paneId }: { paneId: number }) => (
    <div data-testid={`terminal-pane-${paneId}`} />
  ),
}));

function pane(overrides: Partial<PaneConfig> & Pick<PaneConfig, "pane_id" | "label">): PaneConfig {
  return {
    pane_id: overrides.pane_id,
    provider: overrides.provider ?? "claude",
    mode: overrides.mode ?? "interactive",
    kind: overrides.kind,
    session_id: `${SESSION_ID}-pane-${overrides.pane_id}`,
    is_paused: false,
    label: overrides.label,
  };
}

function seed(panes: PaneConfig[], activePaneId: number) {
  localStorage.setItem(`apas_layout_${CLI_CLIENT_ID}_active_tab`, String(activePaneId));
  act(() => {
    useStore.setState({
      connected: true,
      isAttached: true,
      isDualPane: true,
      sessionId: SESSION_ID,
      cliClientId: CLI_CLIENT_ID,
      messages: [],
      paneConfigs: panes,
      paneMessages: Object.fromEntries(panes.map((p) => [paneKey(p.pane_id), []])),
      paneHasMore: {},
      paneStatuses: {},
      paneModes: {},
      pausedPanes: [],
      loadingMorePane: null,
      hasMoreMessages: false,
      isLoadingMore: false,
      teamRecords: [],
      loadPaneMessagesIfNeeded: vi.fn(),
      loadMoreMessages: vi.fn(),
    });
  });
}

beforeEach(() => {
  localStorage.clear();
  // jsdom has no layout engine; TabBar scrolls the active tab into view.
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(() => {
  act(() => {
    useStore.setState(initialStore, true);
  });
  localStorage.clear();
  vi.clearAllMocks();
});

describe("TabbedView terminal panes", () => {
  it("hides the chat composer on a terminal pane", async () => {
    seed([pane({ pane_id: 7, label: "Codex TTY", kind: "terminal" })], 7);
    render(<TabbedView />);

    expect(await screen.findByTestId("terminal-pane-7")).toBeTruthy();
    expect(screen.queryByPlaceholderText("Type a message...")).toBeNull();
  });

  it("keeps the chat composer on a normal agent pane", async () => {
    seed([pane({ pane_id: 8, label: "Claude 1" })], 8);
    render(<TabbedView />);

    expect(await screen.findByPlaceholderText("Type a message...")).toBeTruthy();
    expect(screen.queryByTestId("terminal-pane-8")).toBeNull();
  });

  it("treats a pane with no kind as an agent pane", async () => {
    // Panes persisted before terminal panes existed have no `kind`; they
    // must keep rendering as chat, not as an empty terminal.
    const legacy = pane({ pane_id: 9, label: "Legacy" });
    delete (legacy as Partial<PaneConfig>).kind;
    seed([legacy], 9);
    render(<TabbedView />);

    expect(await screen.findByPlaceholderText("Type a message...")).toBeTruthy();
    expect(screen.queryByTestId("terminal-pane-9")).toBeNull();
  });
});
