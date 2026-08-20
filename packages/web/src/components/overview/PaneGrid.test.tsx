import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PaneGrid } from "./PaneGrid";
import { useStore, type PaneConfig } from "@/lib/store";
import { DEEPSEEK_DEFAULT_MODEL } from "@/lib/providerOptions";

const initialStore = useStore.getState();

function pane(overrides: Partial<PaneConfig> & Pick<PaneConfig, "pane_id" | "label" | "role" | "managed">): PaneConfig {
  return {
    pane_id: overrides.pane_id,
    provider: overrides.provider ?? "claude",
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
  const updatePaneModel = vi.fn();
  useStore.setState({
    paneConfigs,
    paneStatuses: {},
    pausedPanes: [],
    paneMessages: {},
    paneDiffs: {},
    updatePaneManualMode: vi.fn(),
    updatePaneModel,
  });
  return { promotePaneToManaged, updatePaneModel };
}

function renderPaneGrid(onRemovePane = vi.fn(), onOpenPane = vi.fn()) {
  const result = render(
    <PaneGrid
      onOpenPane={onOpenPane}
      onOpenDiff={vi.fn()}
      onOpenRole={vi.fn()}
      onPausePane={vi.fn()}
      onResumePane={vi.fn()}
      onRemovePane={onRemovePane}
    />,
  );

  return { ...result, onOpenPane, onRemovePane };
}

function card(label: string): HTMLElement {
  return screen.getByTitle(`Open ${label}`);
}

function agentSelect(label: string): HTMLSelectElement {
  return within(card(label)).getByTitle(/Agent frontend \/ API backend/) as HTMLSelectElement;
}

function optionLabels(select: HTMLSelectElement): string[] {
  return Array.from(select.options).map((option) => option.textContent ?? "");
}

describe("PaneGrid empty states", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("shows Remove for unmanaged side chats with coordinator-like roles", () => {
    seedPaneGrid([
      pane({ pane_id: 20, label: "Side Manager", role: "manager", managed: false }),
      pane({ pane_id: 21, label: "Side Tech Lead", role: "tech lead", managed: false }),
    ]);

    const { onRemovePane } = renderPaneGrid();

    expect(within(card("Side Manager")).getByText("Remove")).toBeTruthy();
    expect(within(card("Side Tech Lead")).getByText("Remove")).toBeTruthy();

    fireEvent.click(within(card("Side Manager")).getByText("Remove"));
    fireEvent.click(within(card("Side Tech Lead")).getByText("Remove"));

    expect(onRemovePane).toHaveBeenCalledWith(20);
    expect(onRemovePane).toHaveBeenCalledWith(21);
  });
});

describe("PaneGrid existing-pane selectors", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("renders supported agent backend options for managed and unmanaged panes without opening cards", () => {
    seedPaneGrid([
      pane({ pane_id: 30, label: "Managed Claude", role: "developer", managed: true }),
      pane({ pane_id: 31, label: "Side Codex", role: "side chat", managed: false, provider: "codex" }),
    ]);

    const managed = renderPaneGrid();
    const managedSelect = agentSelect("Managed Claude");

    expect(optionLabels(managedSelect)).toEqual([
      "Claude / Official",
      "Claude / DeepSeek",
      "Codex / Official",
      "OpenCode / Official",
      "Cursor / Official",
    ]);
    expect(managedSelect.value).toBe("claude/official");

    fireEvent.click(managedSelect);

    expect(managed.onOpenPane).not.toHaveBeenCalled();

    managed.unmount();

    const unmanaged = renderPaneGrid();
    const unmanagedSelect = agentSelect("Side Codex");

    expect(optionLabels(unmanagedSelect)).toContain("Codex / Official");
    expect(unmanagedSelect.value).toBe("codex/official");

    fireEvent.click(unmanagedSelect);

    expect(unmanaged.onOpenPane).not.toHaveBeenCalled();
  });

  it("changes provider after confirmation without opening the pane", () => {
    const { updatePaneModel } = seedPaneGrid([
      pane({ pane_id: 40, label: "Provider Worker", role: "developer", managed: true }),
    ]);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const { onOpenPane } = renderPaneGrid();

    fireEvent.change(agentSelect("Provider Worker"), { target: { value: "codex/official" } });

    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("Switch agent to Codex / Official?"));
    expect(updatePaneModel).toHaveBeenCalledWith(40, null, "codex");
    expect(onOpenPane).not.toHaveBeenCalled();
  });

  it("uses the shared DeepSeek default when switching provider", () => {
    const { updatePaneModel } = seedPaneGrid([
      pane({ pane_id: 42, label: "DeepSeek Worker", role: "developer", managed: true }),
    ]);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    renderPaneGrid();

    fireEvent.change(agentSelect("DeepSeek Worker"), { target: { value: "claude/deepseek" } });

    expect(updatePaneModel).toHaveBeenCalledWith(42, DEEPSEEK_DEFAULT_MODEL, "claude");
  });

  it("does not change provider when confirmation is cancelled", () => {
    const { updatePaneModel } = seedPaneGrid([
      pane({ pane_id: 41, label: "Cancelled Worker", role: "developer", managed: true }),
    ]);
    vi.spyOn(window, "confirm").mockReturnValue(false);
    renderPaneGrid();

    fireEvent.change(agentSelect("Cancelled Worker"), { target: { value: "codex/official" } });

    expect(updatePaneModel).not.toHaveBeenCalled();
    expect(useStore.getState().paneConfigs[0]).toMatchObject({
      pane_id: 41,
      provider: "claude",
      model: undefined,
    });
  });

  it("renders no model selector and preserves launch models for managed and unmanaged panes", () => {
    const { updatePaneModel } = seedPaneGrid([
      pane({ pane_id: 51, label: "Claude Sonnet", role: "developer", managed: true, model: "sonnet" }),
      pane({
        pane_id: 52,
        label: "Side Claude",
        role: "side chat",
        managed: false,
        model: "claude-fable-5",
      }),
    ]);
    const managed = renderPaneGrid();

    expect(within(card("Claude Sonnet")).queryByTitle(/Claude model/)).toBeNull();
    expect(within(card("Claude Sonnet")).getAllByRole("combobox")).toHaveLength(1);
    expect(agentSelect("Claude Sonnet").value).toBe("claude/official");

    managed.unmount();
    renderPaneGrid();

    expect(within(card("Side Claude")).queryByTitle(/Claude model/)).toBeNull();
    expect(within(card("Side Claude")).getAllByRole("combobox")).toHaveLength(1);
    expect(agentSelect("Side Claude").value).toBe("claude/official");
    expect(updatePaneModel).not.toHaveBeenCalled();
    expect(useStore.getState().paneConfigs).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ pane_id: 51, model: "sonnet" }),
        expect.objectContaining({ pane_id: 52, model: "claude-fable-5" }),
      ]),
    );
  });

  it("renders a retired historical pane as unsupported and read-only", () => {
    const { updatePaneModel, promotePaneToManaged } = seedPaneGrid([
      pane({
        pane_id: 61,
        label: "Historical Worker",
        role: "developer",
        managed: false,
        provider: "claude",
        model: "glm-5.1",
      }),
    ]);
    const { onRemovePane } = renderPaneGrid();
    const retiredCard = card("Historical Worker");

    expect(within(retiredCard).getByText("Unsupported provider")).toBeTruthy();
    expect(within(retiredCard).queryByRole("combobox")).toBeNull();
    expect(within(retiredCard).queryByText("Role")).toBeNull();
    expect(within(retiredCard).queryByText("Resume")).toBeNull();
    expect(within(retiredCard).queryByText("+ Add to team")).toBeNull();
    fireEvent.click(within(retiredCard).getByText("Remove"));
    expect(onRemovePane).toHaveBeenCalledWith(61);
    expect(updatePaneModel).not.toHaveBeenCalled();
    expect(promotePaneToManaged).not.toHaveBeenCalled();
  });
});
