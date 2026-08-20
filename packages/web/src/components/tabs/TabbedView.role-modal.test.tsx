import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { paneKey, useStore, type PaneConfig, type PlanReviewMode } from "@/lib/store";
import { TabbedView } from "./TabbedView";

type StoreState = ReturnType<typeof useStore.getState>;

const initialStore = useStore.getInitialState();
const SESSION_ID = "session-role-modal";
const CLI_CLIENT_ID = "cli-role-modal";
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
    goal: overrides.goal,
    backstory: overrides.backstory,
    plan_review_mode: overrides.plan_review_mode,
    managed: overrides.managed,
  };
}

function seedTabbedView({
  panes = [
    pane({
      pane_id: ACTIVE_PANE_ID,
      label: "Worker One",
      role: "existing role",
      goal: "Existing goal",
      backstory: "Existing backstory",
      plan_review_mode: "risky_only",
    }),
  ],
  updatePaneLabel = vi.fn(),
  updatePaneRole = vi.fn(),
  updatePaneReviewMode = vi.fn(),
}: {
  panes?: PaneConfig[];
  updatePaneLabel?: StoreState["updatePaneLabel"];
  updatePaneRole?: StoreState["updatePaneRole"];
  updatePaneReviewMode?: StoreState["updatePaneReviewMode"];
} = {}) {
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
      loadPaneMessagesIfNeeded: vi.fn(),
      loadMoreMessages: vi.fn(),
      updatePaneLabel,
      updatePaneRole,
      updatePaneReviewMode,
    });
  });

  return { updatePaneLabel, updatePaneRole, updatePaneReviewMode };
}

async function openRoleModal() {
  render(<TabbedView />);

  fireEvent.click(await screen.findByRole("button", { name: "Role" }));

  expect(screen.getByText(/Role .* Goal .* Backstory/)).toBeTruthy();
}

function roleFields() {
  return {
    name: screen.getByLabelText(/Name/) as HTMLInputElement,
    role: screen.getByLabelText(/Role/) as HTMLInputElement,
    goal: screen.getByLabelText(/Goal/) as HTMLTextAreaElement,
    backstory: screen.getByLabelText(/Backstory/) as HTMLTextAreaElement,
    mode: screen.getByLabelText(/Plan review/) as HTMLSelectElement,
  };
}

describe("TabbedView role settings modal", () => {
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

  it("hydrates role settings and saves edited metadata", async () => {
    const { updatePaneLabel, updatePaneRole, updatePaneReviewMode } = seedTabbedView();

    await openRoleModal();

    const fields = roleFields();
    expect(fields.name.value).toBe("Worker One");
    expect(fields.role.value).toBe("existing role");
    expect(fields.goal.value).toBe("Existing goal");
    expect(fields.backstory.value).toBe("Existing backstory");
    expect(fields.mode.value).toBe("risky_only");

    fireEvent.change(fields.name, { target: { value: "  Renamed Worker  " } });
    fireEvent.change(fields.role, { target: { value: "edited role" } });
    fireEvent.change(fields.goal, { target: { value: "Edited goal" } });
    fireEvent.change(fields.backstory, { target: { value: "Edited backstory" } });
    fireEvent.change(fields.mode, { target: { value: "always" satisfies PlanReviewMode } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(updatePaneLabel).toHaveBeenCalledWith(ACTIVE_PANE_ID, "Renamed Worker");
    expect(updatePaneRole).toHaveBeenCalledWith(
      ACTIVE_PANE_ID,
      "edited role",
      "Edited goal",
      "Edited backstory",
    );
    expect(updatePaneReviewMode).toHaveBeenCalledWith(ACTIVE_PANE_ID, "always");
    expect(screen.queryByText(/Role .* Goal .* Backstory/)).toBeNull();
  });

  it("does not update the pane label when the trimmed label is unchanged", async () => {
    const { updatePaneLabel, updatePaneRole, updatePaneReviewMode } = seedTabbedView();

    await openRoleModal();

    fireEvent.change(roleFields().name, { target: { value: "  Worker One  " } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(updatePaneLabel).not.toHaveBeenCalled();
    expect(updatePaneRole).toHaveBeenCalledWith(
      ACTIVE_PANE_ID,
      "existing role",
      "Existing goal",
      "Existing backstory",
    );
    expect(updatePaneReviewMode).toHaveBeenCalledWith(ACTIVE_PANE_ID, "risky_only");
  });

  it("clears role fields and resets plan review mode", async () => {
    seedTabbedView();

    await openRoleModal();

    fireEvent.click(screen.getByRole("button", { name: /Clear/ }));

    const fields = roleFields();
    expect(fields.role.value).toBe("");
    expect(fields.goal.value).toBe("");
    expect(fields.backstory.value).toBe("");
    expect(fields.mode.value).toBe("never");
  });

  it("closes from Cancel or backdrop without saving", async () => {
    const { updatePaneLabel, updatePaneRole, updatePaneReviewMode } = seedTabbedView();

    await openRoleModal();
    fireEvent.change(roleFields().role, { target: { value: "discarded role" } });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(updatePaneLabel).not.toHaveBeenCalled();
    expect(updatePaneRole).not.toHaveBeenCalled();
    expect(updatePaneReviewMode).not.toHaveBeenCalled();
    expect(screen.queryByText(/Role .* Goal .* Backstory/)).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Role" }));
    const heading = screen.getByText(/Role .* Goal .* Backstory/);
    const panel = heading.closest("div");
    expect(panel?.parentElement).toBeTruthy();

    fireEvent.click(panel?.parentElement as HTMLElement);

    await waitFor(() => {
      expect(screen.queryByText(/Role .* Goal .* Backstory/)).toBeNull();
    });
    expect(updatePaneLabel).not.toHaveBeenCalled();
    expect(updatePaneRole).not.toHaveBeenCalled();
    expect(updatePaneReviewMode).not.toHaveBeenCalled();
  });
});
