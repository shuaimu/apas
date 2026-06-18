import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { paneKey, useStore, type PaneConfig } from "@/lib/store";
import { TabbedView } from "./TabbedView";

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getInitialState();
const SESSION_ID = "session-cleanup-dialog";
const CLI_CLIENT_ID = "cli-cleanup-dialog";
const WORKTREE_PANE_ID = 42;
const REMAINING_PANE_ID = 7;
const WORKTREE_PATH = "/tmp/apas-worktree-feature";

function activeTabKey(): string {
  return `apas_layout_${CLI_CLIENT_ID}_active_tab`;
}

function pane(overrides: Partial<PaneConfig> & Pick<PaneConfig, "pane_id" | "label">): PaneConfig {
  return {
    pane_id: overrides.pane_id,
    provider: overrides.provider ?? "claude",
    mode: overrides.mode ?? "interactive",
    session_id: overrides.session_id ?? `${SESSION_ID}-pane-${overrides.pane_id}`,
    is_paused: overrides.is_paused ?? false,
    label: overrides.label,
    worktree_path: overrides.worktree_path,
    role: overrides.role,
    managed: overrides.managed,
  };
}

function seedTabbedView(removePane = vi.fn()) {
  const panes = [
    pane({
      pane_id: WORKTREE_PANE_ID,
      label: "Worktree pane",
      worktree_path: WORKTREE_PATH,
    }),
    pane({ pane_id: REMAINING_PANE_ID, label: "Remaining pane" }),
  ];

  localStorage.setItem(activeTabKey(), String(WORKTREE_PANE_ID));

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
      removePane: removePane as StoreState["removePane"],
    });
  });

  return { removePane };
}

function tabButton(label: string): HTMLElement {
  const button = screen.getByText(label).closest("button");
  expect(button).toBeTruthy();
  return button as HTMLElement;
}

async function openCleanupDialog(removePane = vi.fn()) {
  seedTabbedView(removePane);
  render(<TabbedView />);

  await waitFor(() => {
    expect(tabButton("Worktree pane").className).toContain("border-b-blue-500");
  });

  fireEvent.click(within(tabButton("Worktree pane")).getByTitle("Close tab"));

  expect(screen.getByText("Close pane with isolated worktree")).toBeTruthy();
  expect(screen.getByText(WORKTREE_PATH)).toBeTruthy();
  expect(removePane).not.toHaveBeenCalled();

  return { removePane };
}

describe("TabbedView isolated worktree cleanup dialog", () => {
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

  it("opens the cleanup dialog instead of immediately removing an isolated-worktree pane", async () => {
    await openCleanupDialog();
  });

  it("leaves the branch, removes the pane with the safe action, and selects a remaining tab", async () => {
    const { removePane } = await openCleanupDialog();

    fireEvent.click(screen.getByRole("button", { name: /Leave as branch/ }));

    expect(removePane).toHaveBeenCalledWith(WORKTREE_PANE_ID, "leave_as_branch");
    expect(localStorage.getItem(activeTabKey())).toBe(String(REMAINING_PANE_ID));
    expect(screen.queryByText("Close pane with isolated worktree")).toBeNull();
  });

  it("passes the merge-and-remove cleanup action", async () => {
    const { removePane } = await openCleanupDialog();

    fireEvent.click(screen.getByRole("button", { name: /Merge into current branch/ }));

    expect(removePane).toHaveBeenCalledWith(WORKTREE_PANE_ID, "merge_and_remove");
    expect(screen.queryByText("Close pane with isolated worktree")).toBeNull();
  });

  it("requires confirmation before discarding the worktree and branch", async () => {
    const confirm = vi.spyOn(window, "confirm")
      .mockReturnValueOnce(false)
      .mockReturnValueOnce(true);
    const { removePane } = await openCleanupDialog();

    fireEvent.click(screen.getByRole("button", { name: /Discard everything/ }));

    expect(confirm).toHaveBeenCalledWith(
      "Permanently discard the worktree AND its branch? This cannot be undone.",
    );
    expect(removePane).not.toHaveBeenCalled();
    expect(screen.getByText("Close pane with isolated worktree")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /Discard everything/ }));

    expect(removePane).toHaveBeenCalledWith(WORKTREE_PANE_ID, "discard");
    expect(screen.queryByText("Close pane with isolated worktree")).toBeNull();
  });

  it("closes from Cancel or backdrop without removing the pane", async () => {
    const { removePane } = await openCleanupDialog();

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(removePane).not.toHaveBeenCalled();
    expect(screen.queryByText("Close pane with isolated worktree")).toBeNull();

    fireEvent.click(within(tabButton("Worktree pane")).getByTitle("Close tab"));
    const heading = screen.getByText("Close pane with isolated worktree");
    const panel = heading.closest("div");
    expect(panel?.parentElement).toBeTruthy();

    fireEvent.click(panel?.parentElement as HTMLElement);

    expect(removePane).not.toHaveBeenCalled();
    expect(screen.queryByText("Close pane with isolated worktree")).toBeNull();
  });
});
