import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  paneKey,
  useStore,
  type PaneConfig,
  type PlanReviewPendingItem,
} from "@/lib/store";
import { TabbedView } from "./TabbedView";

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getInitialState();
const SESSION_ID = "session-plan-review";
const CLI_CLIENT_ID = "cli-plan-review";
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
    role: overrides.role,
    managed: overrides.managed,
  };
}

function heldTool(
  overrides: Partial<PlanReviewPendingItem> = {},
): PlanReviewPendingItem {
  return {
    paneId: ACTIVE_PANE_ID,
    toolUseId: "toolu_plan_123",
    toolName: "Bash",
    input: { cmd: "npm test", workdir: "packages/web" },
    arrivedAt: 1_786_000_000_000,
    ...overrides,
  };
}

function seedTabbedView({
  pending = [heldTool()],
  answerPlanReview = vi.fn(),
}: {
  pending?: PlanReviewPendingItem[];
  answerPlanReview?: StoreState["answerPlanReview"];
} = {}) {
  const panes = [pane({ pane_id: ACTIVE_PANE_ID, label: "Worker" })];
  localStorage.setItem(activeTabKey(), String(ACTIVE_PANE_ID));

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
      planReviewPending: pending,
      loadPaneMessagesIfNeeded: vi.fn(),
      loadMoreMessages: vi.fn(),
      answerPlanReview,
    });
  });

  return { answerPlanReview };
}

describe("TabbedView plan-review banner", () => {
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

  it("renders held tool details with a readable JSON input preview", () => {
    seedTabbedView();

    render(<TabbedView />);

    expect(
      screen.getByText(
        (_, element) =>
          element?.tagName.toLowerCase() === "h4" &&
          element.textContent === "Plan review: pane 42 wants to call Bash",
      ),
    ).toBeTruthy();
    expect(screen.getByText("held")).toBeTruthy();
    expect(
      screen.getByText(
        (_, element) =>
          element?.tagName.toLowerCase() === "pre" &&
          element.textContent?.includes('"cmd": "npm test"') === true &&
          element.textContent?.includes('"workdir": "packages/web"') === true,
      ),
    ).toBeTruthy();
  });

  it("denies the held tool with the exact tool use id", () => {
    const { answerPlanReview } = seedTabbedView();

    render(<TabbedView />);

    fireEvent.click(screen.getByRole("button", { name: "Deny" }));

    expect(answerPlanReview).toHaveBeenCalledWith("toolu_plan_123", false);
  });

  it("approves the held tool with the exact tool use id", () => {
    const { answerPlanReview } = seedTabbedView();

    render(<TabbedView />);

    fireEvent.click(screen.getByRole("button", { name: "Approve" }));

    expect(answerPlanReview).toHaveBeenCalledWith("toolu_plan_123", true);
  });

  it("does not render the banner when no plan review is pending", () => {
    seedTabbedView({ pending: [] });

    render(<TabbedView />);

    expect(screen.queryByText(/Plan review:/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Deny" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Approve" })).toBeNull();
  });
});
