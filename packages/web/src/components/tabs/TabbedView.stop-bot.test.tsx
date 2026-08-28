import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { paneKey, useStore, type PaneConfig } from "@/lib/store";
import { TabbedView } from "./TabbedView";

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getInitialState();
const SESSION_ID = "session-stop-bot";
const CLI_CLIENT_ID = "cli-stop-bot";
const ACTIVE_PANE_ID = 42;

function activeTabKey(): string {
  return `apas_layout_${CLI_CLIENT_ID}_active_tab`;
}

function pane(overrides: Partial<PaneConfig> & Pick<PaneConfig, "pane_id">): PaneConfig {
  return {
    pane_id: overrides.pane_id,
    provider: overrides.provider ?? "claude",
    mode: overrides.mode ?? "interactive",
    session_id: overrides.session_id ?? `${SESSION_ID}-pane-${overrides.pane_id}`,
    is_paused: overrides.is_paused ?? false,
    label: overrides.label ?? `Pane ${overrides.pane_id}`,
    stop_requested: overrides.stop_requested,
    role: overrides.role,
    managed: overrides.managed,
  };
}

function seedTabbedView({
  activePaneId = ACTIVE_PANE_ID,
  panes = [pane({ pane_id: ACTIVE_PANE_ID, label: "Worker" })],
  stopBot = vi.fn(),
}: {
  activePaneId?: number;
  panes?: PaneConfig[];
  stopBot?: StoreState["stopBot"];
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
      projectPolicies: {
        [SESSION_ID]: {
          teamAvailable: true,
          allowedLaunchProfiles: ["terminal:claude:official:default"],
          version: 1,
          projectSuspended: false,
          noncompliantPaneIds: [],
        },
      },
      loadPaneMessagesIfNeeded: vi.fn(),
      loadMoreMessages: vi.fn(),
      stopBot,
    });
  });

  return { stopBot };
}

describe("TabbedView bot stop controls", () => {
  beforeEach(() => {
    Element.prototype.scrollIntoView = vi.fn();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    localStorage.clear();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("renders graceful Stop Bot controls for a running bot pane", async () => {
    const { stopBot } = seedTabbedView({
      panes: [
        pane({
          pane_id: ACTIVE_PANE_ID,
          label: "Worker",
          mode: "deadloop",
          stop_requested: false,
        }),
      ],
    });

    render(<TabbedView />);

    const stopButton = await screen.findByRole("button", { name: "Stop Bot" });
    expect(stopButton.getAttribute("title")).toBe(
      "Stop after current work finishes (click again to force stop)",
    );
    expect(
      screen.getByText(
        "Bot is running autonomously. Click Stop Bot to switch to interactive mode after current work finishes.",
      ),
    ).toBeTruthy();

    fireEvent.click(stopButton);

    expect(stopBot).toHaveBeenCalledWith(ACTIVE_PANE_ID);
  });

  it("renders Force Stop controls when a stop is already requested", async () => {
    const { stopBot } = seedTabbedView({
      panes: [
        pane({
          pane_id: ACTIVE_PANE_ID,
          label: "Worker",
          mode: "deadloop",
          stop_requested: true,
        }),
      ],
    });

    render(<TabbedView />);

    const forceStopButton = await screen.findByRole("button", { name: "Force Stop" });
    expect(forceStopButton.getAttribute("title")).toContain("Force stop immediately");
    expect(screen.getByText(/Stop requested .* Click Force Stop to kill immediately\./)).toBeTruthy();

    fireEvent.click(forceStopButton);

    expect(stopBot).toHaveBeenCalledWith(ACTIVE_PANE_ID);
  });

  it("does not render stop controls for interactive panes or Overview", async () => {
    seedTabbedView({
      panes: [
        pane({ pane_id: ACTIVE_PANE_ID, label: "Interactive worker", mode: "interactive" }),
        pane({ pane_id: 77, label: "Bot worker", mode: "deadloop" }),
      ],
    });

    render(<TabbedView />);

    await screen.findByRole("button", { name: "Bot" });
    expect(screen.queryByRole("button", { name: "Stop Bot" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Force Stop" })).toBeNull();

    fireEvent.click(screen.getByText("Overview"));

    await waitFor(() => {
      expect(screen.queryByRole("button", { name: "Stop Bot" })).toBeNull();
      expect(screen.queryByRole("button", { name: "Force Stop" })).toBeNull();
    });
  });
});
