import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { paneKey, useStore, type PaneConfig } from "@/lib/store";
import {
  botPromptForPane,
  CLASSIC_TODO_BOT_LOOP_PROMPT,
  confirmedPaneRebootTarget,
  defaultBotPromptForPane,
  deriveInitialActiveTabId,
  lazyPaneMessageLoadTargets,
  OVERVIEW_PANE_ID,
  requestConfirmedPaneReboot,
  shouldShowPaneRebootButton,
  TabbedView,
} from "./TabbedView";

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getInitialState();
const DOWNLOAD_SESSION_ID = "session-tabbed-download";
const DOWNLOAD_CLI_CLIENT_ID = "cli-tabbed-download";
const DOWNLOAD_PANE_ID = 42;

function activeTabKey(): string {
  return `apas_layout_${DOWNLOAD_CLI_CLIENT_ID}_active_tab`;
}

function downloadPane(overrides: Partial<PaneConfig> & Pick<PaneConfig, "pane_id" | "label">): PaneConfig {
  return {
    pane_id: overrides.pane_id,
    provider: overrides.provider ?? "claude",
    mode: overrides.mode ?? "interactive",
    session_id: overrides.session_id ?? `${DOWNLOAD_SESSION_ID}-pane-${overrides.pane_id}`,
    is_paused: overrides.is_paused ?? false,
    label: overrides.label,
    role: overrides.role,
    managed: overrides.managed,
  };
}

function seedDownloadTabbedView({
  downloadSession = vi.fn(),
  rebootPane = vi.fn(),
  requestPaneDiff = vi.fn(),
}: {
  downloadSession?: StoreState["downloadSession"];
  rebootPane?: StoreState["rebootPane"];
  requestPaneDiff?: StoreState["requestPaneDiff"];
} = {}) {
  const panes = [downloadPane({ pane_id: DOWNLOAD_PANE_ID, label: "Worker" })];
  localStorage.setItem(activeTabKey(), String(DOWNLOAD_PANE_ID));

  act(() => {
    useStore.setState({
      connected: true,
      isAttached: true,
      isDualPane: true,
      sessionId: DOWNLOAD_SESSION_ID,
      cliClientId: DOWNLOAD_CLI_CLIENT_ID,
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
      downloadSession,
      rebootPane,
      requestPaneDiff,
    });
  });

  return { downloadSession, rebootPane, requestPaneDiff };
}

describe("deriveInitialActiveTabId", () => {
  const base = {
    activeTabId: null,
    clientChanged: true,
    managerTabId: null,
    paneConfigsLength: 2,
    savedActiveTab: "",
    tabIds: [10, 20],
  };

  it("keeps a valid saved active tab", () => {
    expect(
      deriveInitialActiveTabId({
        ...base,
        managerTabId: 10,
        savedActiveTab: "20",
      }),
    ).toBe(20);
  });

  it("prefers the interactive Manager tab when there is no valid saved tab", () => {
    expect(
      deriveInitialActiveTabId({
        ...base,
        managerTabId: 10,
        savedActiveTab: "",
      }),
    ).toBe(10);
  });

  it("falls back to Overview when there is no Manager and no valid saved tab", () => {
    expect(
      deriveInitialActiveTabId({
        ...base,
        managerTabId: null,
        savedActiveTab: "",
      }),
    ).toBe(OVERVIEW_PANE_ID);
  });

  it("ignores stale saved tabs and still uses Manager or Overview fallback", () => {
    expect(
      deriveInitialActiveTabId({
        ...base,
        managerTabId: 10,
        savedActiveTab: "999",
      }),
    ).toBe(10);

    expect(
      deriveInitialActiveTabId({
        ...base,
        managerTabId: null,
        savedActiveTab: "999",
      }),
    ).toBe(OVERVIEW_PANE_ID);
  });
});

describe("lazyPaneMessageLoadTargets", () => {
  it("requests only the active real pane", () => {
    expect(
      lazyPaneMessageLoadTargets({
        activeTabId: 20,
        tabIds: [10, 20, 30],
      }),
    ).toEqual([20]);
  });

  it("skips Overview, stale active tabs, and null active tabs", () => {
    expect(
      lazyPaneMessageLoadTargets({
        activeTabId: OVERVIEW_PANE_ID,
        tabIds: [10, 20],
      }),
    ).toEqual([]);
    expect(
      lazyPaneMessageLoadTargets({
        activeTabId: 99,
        tabIds: [10, 20],
      }),
    ).toEqual([]);
    expect(
      lazyPaneMessageLoadTargets({
        activeTabId: null,
        tabIds: [10, 20],
      }),
    ).toEqual([]);
  });
});

describe("TabbedView session download actions", () => {
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

  it("calls downloadSession from the toolbar without opening Team or active-tab actions", () => {
    const downloadSession = vi.fn();
    const rebootPane = vi.fn();
    const requestPaneDiff = vi.fn();
    seedDownloadTabbedView({ downloadSession, rebootPane, requestPaneDiff });

    render(<TabbedView />);

    expect(screen.getByRole("button", { name: "Team" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Download" }));

    expect(downloadSession).toHaveBeenCalledOnce();
    expect(screen.queryByText(/Team scratchpad/)).toBeNull();
    expect(rebootPane).not.toHaveBeenCalled();
    expect(requestPaneDiff).not.toHaveBeenCalled();
    expect(localStorage.getItem(activeTabKey())).toBe(String(DOWNLOAD_PANE_ID));
  });
});

describe("pane reboot controls", () => {
  it("hides the per-pane reboot button on Overview and null tabs", () => {
    expect(shouldShowPaneRebootButton(OVERVIEW_PANE_ID)).toBe(false);
    expect(shouldShowPaneRebootButton(null)).toBe(false);
    expect(shouldShowPaneRebootButton(42)).toBe(true);
  });

  it("returns a real pane target only after confirmation", () => {
    const confirm = vi.fn(() => true);

    expect(confirmedPaneRebootTarget(42, confirm)).toBe(42);
    expect(confirm).toHaveBeenCalledOnce();

    expect(confirmedPaneRebootTarget(OVERVIEW_PANE_ID, confirm)).toBeNull();
    expect(confirmedPaneRebootTarget(null, confirm)).toBeNull();
    expect(confirm).toHaveBeenCalledOnce();

    expect(confirmedPaneRebootTarget(7, vi.fn(() => false))).toBeNull();
  });

  it("invokes rebootPane for real panes only after confirmation", () => {
    const rebootPane = vi.fn();
    const confirm = vi.fn(() => true);

    requestConfirmedPaneReboot(42, confirm, rebootPane);

    expect(confirm).toHaveBeenCalledOnce();
    expect(rebootPane).toHaveBeenCalledWith(42);

    requestConfirmedPaneReboot(OVERVIEW_PANE_ID, confirm, rebootPane);
    requestConfirmedPaneReboot(null, confirm, rebootPane);
    expect(confirm).toHaveBeenCalledOnce();
    expect(rebootPane).toHaveBeenCalledOnce();

    requestConfirmedPaneReboot(7, vi.fn(() => false), rebootPane);
    expect(rebootPane).toHaveBeenCalledOnce();
  });
});

describe("botPromptForPane", () => {
  it("uses explicit saved prompts before managed or classic fallbacks", () => {
    expect(
      botPromptForPane({
        prompt: "Keep doing the custom thing.",
        managed: true,
        role: "developer",
      }),
    ).toBe("Keep doing the custom thing.");
  });

  it("uses role metadata for managed team panes instead of the classic TODO loop", () => {
    const prompt = botPromptForPane({
      managed: true,
      role: "developer",
      goal: "Implement delegated team TODO leaves.",
      backstory: "Stay inside the assigned worktree.",
    });

    expect(prompt).toContain("managed team worker");
    expect(prompt).toContain("Role: developer");
    expect(prompt).toContain("team-todo.md");
    expect(prompt).toContain("Stay inside the assigned worktree.");
    expect(prompt).not.toContain("Work on tasks defined in TODO.md");
  });

  it("keeps the classic TODO.md loop for unmanaged manual bot panes", () => {
    expect(botPromptForPane({ managed: false })).toBe(CLASSIC_TODO_BOT_LOOP_PROMPT);
    expect(CLASSIC_TODO_BOT_LOOP_PROMPT).toContain("TODO.md");
  });

  it("can ignore saved prompts when callers need the default fallback", () => {
    const prompt = defaultBotPromptForPane({
      prompt: "Old saved prompt.",
      managed: true,
      role: "reviewer",
    });

    expect(prompt).toContain("managed team worker");
    expect(prompt).toContain("Role: reviewer");
    expect(prompt).not.toContain("Old saved prompt.");
  });
});
