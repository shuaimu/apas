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
    promotePaneToManaged,
    updatePaneManualMode: vi.fn(),
    updatePaneModel,
  });
  return { promotePaneToManaged, updatePaneModel };
}

function renderPaneGrid(kind: "managed" | "unmanaged", onRemovePane = vi.fn(), onOpenPane = vi.fn()) {
  const result = render(
    <PaneGrid
      kind={kind}
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

function claudeModelSelect(label: string): HTMLSelectElement {
  return within(card(label)).getByTitle(/Claude model/) as HTMLSelectElement;
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

  it("points the empty managed-team copy at Start team and side-chat promotion", () => {
    seedPaneGrid([]);

    renderPaneGrid("managed");

    const emptyState = screen.getByText(/No team members yet/);
    expect(emptyState.textContent).toContain("Start team");
    expect(emptyState.textContent).toContain("promote an existing side chat");
    expect(emptyState.textContent).not.toContain("+ Add Worker");
  });
});

describe("PaneGrid removal controls", () => {
  afterEach(() => {
    vi.restoreAllMocks();
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

describe("PaneGrid agent and model switchers", () => {
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

    const managed = renderPaneGrid("managed");
    const managedSelect = agentSelect("Managed Claude");

    expect(optionLabels(managedSelect)).toEqual([
      "Claude / Official",
      "Claude / MiniMax 2.7",
      "Claude / GLM 5.1",
      "Claude / DeepSeek",
      "Codex / Official",
      "OpenCode / Official",
      "Cursor / Official",
    ]);
    expect(managedSelect.value).toBe("claude/official");

    fireEvent.click(managedSelect);

    expect(managed.onOpenPane).not.toHaveBeenCalled();

    managed.unmount();

    const unmanaged = renderPaneGrid("unmanaged");
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
    const { onOpenPane } = renderPaneGrid("managed");

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
    renderPaneGrid("managed");

    fireEvent.change(agentSelect("DeepSeek Worker"), { target: { value: "claude/deepseek" } });

    expect(updatePaneModel).toHaveBeenCalledWith(42, DEEPSEEK_DEFAULT_MODEL, "claude");
  });

  it("does not change provider when confirmation is cancelled", () => {
    const { updatePaneModel } = seedPaneGrid([
      pane({ pane_id: 41, label: "Cancelled Worker", role: "developer", managed: true }),
    ]);
    vi.spyOn(window, "confirm").mockReturnValue(false);
    renderPaneGrid("managed");

    fireEvent.change(agentSelect("Cancelled Worker"), { target: { value: "codex/official" } });

    expect(updatePaneModel).not.toHaveBeenCalled();
    expect(useStore.getState().paneConfigs[0]).toMatchObject({
      pane_id: 41,
      provider: "claude",
      model: undefined,
    });
  });

  it("updates Claude model selections and clears Default to null", () => {
    const { updatePaneModel } = seedPaneGrid([
      pane({ pane_id: 50, label: "Claude Default", role: "developer", managed: true }),
      pane({ pane_id: 51, label: "Claude Sonnet", role: "developer", managed: true, model: "sonnet" }),
    ]);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    renderPaneGrid("managed");

    fireEvent.change(claudeModelSelect("Claude Default"), { target: { value: "opus" } });
    fireEvent.change(claudeModelSelect("Claude Sonnet"), { target: { value: "default" } });

    expect(updatePaneModel).toHaveBeenNthCalledWith(1, 50, "opus");
    expect(updatePaneModel).toHaveBeenNthCalledWith(2, 51, null);
  });

  it("does not render the Claude model selector for non-Claude panes", () => {
    seedPaneGrid([
      pane({ pane_id: 60, label: "Codex Worker", role: "developer", managed: true, provider: "codex" }),
    ]);

    renderPaneGrid("managed");

    expect(within(card("Codex Worker")).queryByTitle(/Claude model/)).toBeNull();
  });
});
