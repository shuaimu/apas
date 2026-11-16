import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PaneGrid } from "./PaneGrid";
import { useStore, type PaneConfig } from "@/lib/store";
import {
  DEEPSEEK_DEFAULT_MODEL,
  DEEPSEEK_FLASH_MODEL,
  DEEPSEEK_RETIRED_PRO_MODEL,
} from "@/lib/providerOptions";

const initialStore = useStore.getState();

function pane(overrides: Partial<PaneConfig> & Pick<PaneConfig, "pane_id" | "label" | "role" | "managed">): PaneConfig {
  return {
    pane_id: overrides.pane_id,
    provider: overrides.provider ?? "claude",
    mode: overrides.mode ?? "interactive",
    session_id: `pane-${overrides.pane_id}`,
    is_paused: false,
    label: overrides.label,
    role: overrides.role,
    managed: overrides.managed,
    model: overrides.model,
    manual_mode: overrides.manual_mode,
  };
}

function seedPaneGrid(
  paneConfigs: PaneConfig[],
  promotePaneToManaged = vi.fn(),
  paneStatuses: Record<string, string | null> = {},
) {
  const updatePaneModel = vi.fn();
  useStore.setState({
    paneConfigs,
    paneStatuses,
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
      onRemovePane={onRemovePane}
    />,
  );

  return { ...result, onOpenPane, onRemovePane };
}

function card(label: string): HTMLElement {
  return screen.getByTitle(`Open ${label}`);
}

function closeButton(label: string): HTMLElement {
  return within(card(label)).getByRole("button", { name: `Close tab ${label}` });
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

  it("offers Close tab for unmanaged side chats with coordinator-like roles", () => {
    seedPaneGrid([
      pane({ pane_id: 20, label: "Side Manager", role: "manager", managed: false }),
      pane({ pane_id: 21, label: "Side Tech Lead", role: "tech lead", managed: false }),
    ]);

    const { onRemovePane, onOpenPane } = renderPaneGrid();

    expect(closeButton("Side Manager").textContent).toBe("Close tab");
    expect(closeButton("Side Tech Lead").textContent).toBe("Close tab");

    fireEvent.click(closeButton("Side Manager"));
    fireEvent.click(closeButton("Side Tech Lead"));

    expect(onRemovePane).toHaveBeenCalledWith(20);
    expect(onRemovePane).toHaveBeenCalledWith(21);
    // Closing a tab from its card does not also open it.
    expect(onOpenPane).not.toHaveBeenCalled();
  });

  it("opens a tab from its name, which is a button, and from anywhere on its card", () => {
    seedPaneGrid([pane({ pane_id: 24, label: "Docs", role: undefined, managed: false })]);
    const { onOpenPane } = renderPaneGrid();

    const name = screen.getByRole("button", { name: "Open Docs" });
    expect(name.textContent).toBe("Docs");
    fireEvent.click(name);
    expect(onOpenPane).toHaveBeenCalledTimes(1);
    expect(onOpenPane).toHaveBeenLastCalledWith(24);

    fireEvent.click(card("Docs"));
    expect(onOpenPane).toHaveBeenCalledTimes(2);
  });

  it("gives every card a status line, and says when a tab last worked", () => {
    seedPaneGrid(
      [
        pane({ pane_id: 25, label: "Busy", role: undefined, managed: false }),
        pane({ pane_id: 26, label: "Quiet", role: undefined, managed: false }),
        pane({ pane_id: 27, label: "Reported", role: undefined, managed: false }),
      ],
      vi.fn(),
      { "25": "Working..." },
    );
    const counters = { prompts: 0, responses: 0, input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_creation_tokens: 0, cost_usd: 0 };
    useStore.setState({
      sessionId: "S",
      // The server's record covers a tab whose messages this browser never loaded.
      usageStats: { S: { panes: [{ pane_id: 27, lifetime: counters, last_7d: counters, today: counters, last_active: new Date(Date.now() - 5 * 60_000).toISOString() }], lifetime: counters, last_7d: counters, today: counters } },
    });
    renderPaneGrid();

    expect(within(card("Busy")).getByText("Working...")).toBeTruthy();
    expect(within(card("Quiet")).getByText("Idle")).toBeTruthy();
    expect(within(card("Quiet")).getByText(/No activity yet/)).toBeTruthy();
    expect(within(card("Reported")).getByText(/Last active 5m ago/)).toBeTruthy();
  });

  it("lists the cards in the tab bar's order, whatever their ids or states", () => {
    // The roster's order is the tab bar's. Ids are not creation order, and a
    // working tab keeps its place.
    seedPaneGrid(
      [
        pane({ pane_id: 612, label: "First in tab bar", role: undefined, managed: false }),
        pane({ pane_id: 265, label: "Second", role: undefined, managed: false }),
        pane({ pane_id: 44, label: "Third", role: undefined, managed: false }),
      ],
      vi.fn(),
      { "44": "Working..." },
    );
    renderPaneGrid();

    expect(screen.getAllByRole("heading", { level: 4 }).map((heading) => heading.textContent))
      .toEqual(["First in tab bar", "Second", "Third"]);
  });

  it("never shows a negative time when the server's clock is ahead of this browser's", () => {
    seedPaneGrid([
      pane({ pane_id: 28, label: "Ahead", role: undefined, managed: false }),
      pane({ pane_id: 29, label: "Recent", role: undefined, managed: false }),
    ]);
    const counters = { prompts: 0, responses: 0, input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_creation_tokens: 0, cost_usd: 0 };
    const usage = (pane_id: number, offsetMs: number) => ({
      pane_id,
      lifetime: counters,
      last_7d: counters,
      today: counters,
      last_active: new Date(Date.now() + offsetMs).toISOString(),
    });
    useStore.setState({
      sessionId: "S",
      usageStats: { S: { panes: [usage(28, 90_000), usage(29, -44_000)], lifetime: counters, last_7d: counters, today: counters } },
    });
    renderPaneGrid();

    // Same words as the usage table's row for the same tab.
    expect(within(card("Ahead")).getByText("· Last active just now")).toBeTruthy();
    expect(within(card("Recent")).getByText("· Last active just now")).toBeTruthy();
    expect(card("Ahead").textContent).not.toMatch(/-\d/);
  });

  it("says what a terminal tab runs", () => {
    seedPaneGrid([
      { ...pane({ pane_id: 28, label: "Flash", role: undefined, managed: false, model: DEEPSEEK_FLASH_MODEL }), kind: "terminal" },
      { ...pane({ pane_id: 29, label: "Shell", role: undefined, managed: false, provider: "omp" }), kind: "terminal" },
    ]);
    renderPaneGrid();

    expect(within(card("Flash")).getByText("Claude / DeepSeek 4.1 Flash")).toBeTruthy();
    expect(within(card("Shell")).getByText("OMP")).toBeTruthy();
    expect(within(card("Shell")).queryByRole("combobox")).toBeNull();
  });

  it("points a project without tabs at the tab bar", () => {
    seedPaneGrid([]);
    renderPaneGrid();
    expect(screen.getByText(/No tabs yet/)).toBeTruthy();
  });

  it("labels a blocked pane as pending answer rather than thinking", () => {
    seedPaneGrid(
      [pane({ pane_id: 22, label: "Needs answer", role: "developer", managed: false })],
      vi.fn(),
      { "22": "Pending answer" },
    );

    renderPaneGrid();
    expect(within(card("Needs answer")).getByLabelText("pending answer")).toBeTruthy();
    expect(within(card("Needs answer")).queryByLabelText("thinking")).toBeNull();
  });

  it("keeps legacy roles and history without bot or team execution controls", () => {
    seedPaneGrid([
      pane({ pane_id: 23, label: "Earlier coordinator", role: "tech lead", managed: true, mode: "deadloop" }),
    ]);
    const { onOpenPane, onRemovePane } = renderPaneGrid();
    expect(screen.queryByText("Pause")).toBeNull();
    expect(screen.queryByText("Resume")).toBeNull();
    expect(screen.queryByTitle(/Click to flip/)).toBeNull();
    expect(screen.queryByText("+ Add to team")).toBeNull();
    fireEvent.click(card("Earlier coordinator"));
    expect(onOpenPane).toHaveBeenCalledWith(23);
    fireEvent.click(closeButton("Earlier coordinator"));
    expect(onRemovePane).toHaveBeenCalledWith(23);
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
      "Claude / DeepSeek 4.1 Flash",
      "Codex / Official",
      "OpenCode",
      "Cursor",
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

    fireEvent.change(agentSelect("DeepSeek Worker"), {
      target: { value: "claude/deepseek-flash" },
    });

    expect(updatePaneModel).toHaveBeenCalledWith(42, DEEPSEEK_DEFAULT_MODEL, "claude");
  });

  it("marks a pane on the withdrawn DeepSeek model as unsupported", () => {
    seedPaneGrid([
      pane({
        pane_id: 43,
        label: "DeepSeek Pro Worker",
        role: "developer",
        managed: false,
        model: DEEPSEEK_RETIRED_PRO_MODEL,
      }),
    ]);
    renderPaneGrid();

    // The established treatment for a retired model: the pane keeps its
    // history and identity, is flagged, and offers no backend selector, so it
    // can never be silently resumed on a model APAS no longer supports.
    expect(screen.getByLabelText("unsupported provider")).toBeTruthy();
    expect(screen.queryByTitle(/Agent frontend \/ API backend/)).toBeNull();
  });


  it("filters the surviving DeepSeek profile through launch-profile policy", () => {
    seedPaneGrid([
      pane({ pane_id: 44, label: "Policy Worker", role: "side chat", managed: false }),
    ]);
    useStore.setState({
      sessionId: "policy-session",
      projectPolicies: {
        "policy-session": {
          teamAvailable: false,
          // Deliberately the pre-rename key: a policy the server has not
          // migrated yet must still offer the capability it names.
          allowedLaunchProfiles: ["agent:claude:deepseek:deepseek-v4-flash"],
          version: 3,
          projectSuspended: false,
          noncompliantPaneIds: [],
        },
      },
    });
    renderPaneGrid();

    const select = agentSelect("Policy Worker");
    expect(optionLabels(select)).toContain("Claude / DeepSeek 4.1 Flash");
    // The withdrawn variant is not offered under any label.
    expect(optionLabels(select).some((label) => /DeepSeek Pro/i.test(label))).toBe(false);
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
    fireEvent.click(closeButton("Historical Worker"));
    expect(onRemovePane).toHaveBeenCalledWith(61);
    expect(updatePaneModel).not.toHaveBeenCalled();
    expect(promotePaneToManaged).not.toHaveBeenCalled();
  });
});

describe("PaneGrid tabs that are not running", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  const grid = (props: Partial<Parameters<typeof PaneGrid>[0]>) => render(
    <PaneGrid onOpenPane={vi.fn()} onOpenDiff={vi.fn()} onOpenRole={vi.fn()} onRemovePane={vi.fn()} {...props} />,
  );
  const panes = [
    pane({ pane_id: 1, label: "claude-005", role: undefined, managed: false }),
    pane({ pane_id: 2, label: "claude-006", role: undefined, managed: false }),
  ];

  it("says Not running on its machine instead of Idle, as the sidebar's dots do", () => {
    // A status the tab reported before its machine stopped is stale.
    seedPaneGrid(panes, vi.fn(), { "2": "Thinking" });
    grid({
      tabHosts: new Map([
        [1, { hostname: "zoo-005", state: "running", badge: true }],
        [2, { hostname: "zoo-006", state: "stopped", badge: true }],
      ]),
    });
    expect(within(card("claude-005")).getByText("Idle")).toBeTruthy();
    expect(within(card("claude-005")).getByRole("img", { name: "idle" })).toBeTruthy();
    expect(within(card("claude-006")).getByText("Not running on zoo-006")).toBeTruthy();
    expect(within(card("claude-006")).getByRole("img", { name: "not running" })).toBeTruthy();
    expect(within(card("claude-006")).queryByText("Thinking")).toBeNull();
    expect(within(card("claude-006")).queryByText("Idle")).toBeNull();
  });

  it("says Not running on every card of a project that runs nowhere", () => {
    seedPaneGrid(panes);
    // A closed project on one machine: nothing is known per tab.
    grid({ projectRunning: false });
    for (const label of ["claude-005", "claude-006"]) {
      expect(within(card(label)).getByText("Not running")).toBeTruthy();
      expect(within(card(label)).queryByText("Idle")).toBeNull();
    }
  });

  it("says a tab this viewer may not use has no access, and keeps Idle without any report", () => {
    seedPaneGrid(panes);
    const view = grid({ tabHosts: new Map([[2, { hostname: "zoo-006", state: "no_access", badge: true }]]) });
    expect(within(card("claude-006")).getByText("No access to zoo-006")).toBeTruthy();
    expect(within(card("claude-005")).getByText("Idle")).toBeTruthy();
    view.unmount();

    grid({});
    expect(within(card("claude-005")).getByText("Idle")).toBeTruthy();
    expect(within(card("claude-006")).getByText("Idle")).toBeTruthy();
  });
});
