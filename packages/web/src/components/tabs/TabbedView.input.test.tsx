import { act, createEvent, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { paneKey, useStore, type PaneConfig } from "@/lib/store";
import { OVERVIEW_PANE_ID, TabbedView } from "./TabbedView";

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getInitialState();
const SESSION_ID = "session-tabbed-input";
const CLI_CLIENT_ID = "cli-tabbed-input";
const ACTIVE_PANE_ID = 42;

function draftKey(sessionId = SESSION_ID, paneId = ACTIVE_PANE_ID): string {
  return `apas_input_draft_${sessionId}_${paneId}`;
}

function activeTabKey(cliClientId = CLI_CLIENT_ID): string {
  return `apas_layout_${cliClientId}_active_tab`;
}

function pane(overrides: Partial<PaneConfig> & Pick<PaneConfig, "pane_id">): PaneConfig {
  return {
    pane_id: overrides.pane_id,
    provider: overrides.provider ?? "claude",
    mode: overrides.mode ?? "interactive",
    session_id: overrides.session_id ?? `${SESSION_ID}-pane-${overrides.pane_id}`,
    is_paused: overrides.is_paused ?? false,
    label: overrides.label ?? `Pane ${overrides.pane_id}`,
    role: overrides.role,
    managed: overrides.managed,
    stop_requested: overrides.stop_requested,
  };
}

function seedTabbedView({
  activePaneId = ACTIVE_PANE_ID,
  panes = [pane({ pane_id: ACTIVE_PANE_ID, label: "Worker" })],
  sendMessageToPane = vi.fn(() => ({ success: true })),
}: {
  activePaneId?: number;
  panes?: PaneConfig[];
  sendMessageToPane?: StoreState["sendMessageToPane"];
} = {}) {
  localStorage.setItem(activeTabKey(), String(activePaneId));

  act(() => {
    useStore.setState({
      connected: true,
      isAttached: true,
      isDualPane: true,
      sessionId: SESSION_ID,
      cliClientId: CLI_CLIENT_ID,
      messages: [],
      paneConfigs: panes,
      paneMessages: Object.fromEntries(panes.map((item) => [paneKey(item.pane_id), []])),
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
      sendMessageToPane,
    });
  });

  return { sendMessageToPane };
}

async function activeInput(): Promise<HTMLTextAreaElement> {
  const textarea = await screen.findByPlaceholderText("Type a message...");
  return textarea as HTMLTextAreaElement;
}

describe("TabbedView pane input drafts", () => {
  beforeEach(() => {
    Element.prototype.scrollIntoView = vi.fn();
  });

  afterEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("restores the active pane draft from its session and pane localStorage key", async () => {
    localStorage.setItem(draftKey(), "restored draft");
    seedTabbedView();

    render(<TabbedView />);

    const textarea = await activeInput();
    await waitFor(() => {
      expect(textarea.value).toBe("restored draft");
    });
  });

  it("persists a real pane draft and clears it after a successful send", async () => {
    const { sendMessageToPane } = seedTabbedView();
    render(<TabbedView />);

    const textarea = await activeInput();
    fireEvent.change(textarea, { target: { value: "  send this draft  " } });

    expect(localStorage.getItem(draftKey())).toBe("  send this draft  ");

    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    expect(sendMessageToPane).toHaveBeenCalledWith("send this draft", ACTIVE_PANE_ID);
    await waitFor(() => {
      expect(textarea.value).toBe("");
    });
    expect(localStorage.getItem(draftKey())).toBeNull();
  });

  it("sends on Enter but lets Shift+Enter preserve a newline without sending", async () => {
    const { sendMessageToPane } = seedTabbedView();
    render(<TabbedView />);

    const textarea = await activeInput();
    fireEvent.change(textarea, { target: { value: "send from keyboard" } });

    const enter = createEvent.keyDown(textarea, { key: "Enter", code: "Enter" });
    fireEvent(textarea, enter);

    expect(enter.defaultPrevented).toBe(true);
    expect(sendMessageToPane).toHaveBeenCalledWith("send from keyboard", ACTIVE_PANE_ID);
    await waitFor(() => {
      expect(textarea.value).toBe("");
    });

    fireEvent.change(textarea, { target: { value: "first line" } });
    const shiftEnter = createEvent.keyDown(textarea, {
      key: "Enter",
      code: "Enter",
      shiftKey: true,
    });
    fireEvent(textarea, shiftEnter);
    fireEvent.change(textarea, { target: { value: "first line\n" } });

    expect(shiftEnter.defaultPrevented).toBe(false);
    expect(sendMessageToPane).toHaveBeenCalledTimes(1);
    expect(textarea.value).toBe("first line\n");
    expect(localStorage.getItem(draftKey())).toBe("first line\n");
  });

  it("shows failed send errors while keeping the draft intact", async () => {
    const sendMessageToPane = vi.fn(() => ({
      success: false,
      error: "Pane worker unavailable",
    }));
    seedTabbedView({ sendMessageToPane });
    render(<TabbedView />);

    const textarea = await activeInput();
    fireEvent.change(textarea, { target: { value: "keep this draft" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    expect(await screen.findByText("Pane worker unavailable")).toBeTruthy();
    expect(textarea.value).toBe("keep this draft");
    expect(localStorage.getItem(draftKey())).toBe("keep this draft");
  });

  it("hides input on Overview and shows bot status instead of input for deadloop panes", async () => {
    seedTabbedView({
      panes: [
        pane({ pane_id: ACTIVE_PANE_ID, label: "Worker" }),
        pane({ pane_id: 77, label: "Reviewer", mode: "deadloop" }),
      ],
    });
    render(<TabbedView />);

    await activeInput();

    fireEvent.click(screen.getByText("Overview"));
    await waitFor(() => {
      expect(screen.queryByPlaceholderText("Type a message...")).toBeNull();
    });

    fireEvent.click(screen.getByText("Reviewer (Bot)"));
    await waitFor(() => {
      expect(screen.queryByPlaceholderText("Type a message...")).toBeNull();
      expect(
        screen.getByText("Bot is running autonomously. Click Stop Bot to switch to interactive mode after current work finishes."),
      ).toBeTruthy();
    });
  });
});
