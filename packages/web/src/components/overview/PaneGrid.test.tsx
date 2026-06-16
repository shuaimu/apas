import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PaneGrid } from "./PaneGrid";
import { useStore, type PaneConfig } from "@/lib/store";

const initialStore = useStore.getState();

function pane(overrides: Partial<PaneConfig> & Pick<PaneConfig, "pane_id" | "label" | "role" | "managed">): PaneConfig {
  return {
    pane_id: overrides.pane_id,
    provider: "claude",
    mode: overrides.mode ?? "deadloop",
    session_id: `pane-${overrides.pane_id}`,
    is_paused: false,
    label: overrides.label,
    role: overrides.role,
    managed: overrides.managed,
    model: overrides.model,
    manual_mode: overrides.manual_mode,
  };
}

function seedPaneGrid(paneConfigs: PaneConfig[], promotePaneToManaged = vi.fn()) {
  useStore.setState({
    paneConfigs,
    paneStatuses: {},
    pausedPanes: [],
    paneMessages: {},
    paneDiffs: {},
    promotePaneToManaged,
    updatePaneManualMode: vi.fn(),
    updatePaneModel: vi.fn(),
  });
}

function renderPaneGrid(kind: "managed" | "unmanaged", onRemovePane = vi.fn()) {
  const result = render(
    <PaneGrid
      kind={kind}
      onOpenPane={vi.fn()}
      onOpenDiff={vi.fn()}
      onOpenRole={vi.fn()}
      onPausePane={vi.fn()}
      onResumePane={vi.fn()}
      onRemovePane={onRemovePane}
    />,
  );

  return { ...result, onRemovePane };
}

function card(label: string): HTMLElement {
  return screen.getByTitle(`Open ${label}`);
}

describe("PaneGrid removal controls", () => {
  afterEach(() => {
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("hides Remove for managed coordinators but keeps managed workers removable", () => {
    seedPaneGrid([
      pane({ pane_id: 10, label: "Manager", role: "team manager", managed: true, mode: "interactive" }),
      pane({ pane_id: 11, label: "Tech Lead", role: "tech lead", managed: true }),
      pane({ pane_id: 12, label: "Developer", role: "developer", managed: true }),
    ]);

    const { onRemovePane } = renderPaneGrid("managed");

    expect(within(card("Manager")).queryByText("Remove")).toBeNull();
    expect(within(card("Tech Lead")).queryByText("Remove")).toBeNull();
    expect(within(card("Developer")).getByText("Remove")).toBeTruthy();

    fireEvent.click(within(card("Developer")).getByText("Remove"));

    expect(onRemovePane).toHaveBeenCalledWith(12);
  });

  it("shows Remove for unmanaged side chats with coordinator-like roles", () => {
    seedPaneGrid([
      pane({ pane_id: 20, label: "Side Manager", role: "manager", managed: false }),
      pane({ pane_id: 21, label: "Side Tech Lead", role: "tech lead", managed: false }),
    ]);

    const { onRemovePane } = renderPaneGrid("unmanaged");

    expect(within(card("Side Manager")).getByText("Remove")).toBeTruthy();
    expect(within(card("Side Tech Lead")).getByText("Remove")).toBeTruthy();

    fireEvent.click(within(card("Side Manager")).getByText("Remove"));
    fireEvent.click(within(card("Side Tech Lead")).getByText("Remove"));

    expect(onRemovePane).toHaveBeenCalledWith(20);
    expect(onRemovePane).toHaveBeenCalledWith(21);
  });
});
