import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { TeamSetupCard } from "./TeamSetupCard";
import { DEEPSEEK_DEFAULT_MODEL } from "@/lib/providerOptions";
import { useStore, type PaneConfig } from "@/lib/store";

const initialStore = useStore.getState();

type StoreState = ReturnType<typeof useStore.getState>;

function managedPane(overrides: Pick<PaneConfig, "pane_id" | "label" | "role">): PaneConfig {
  return {
    pane_id: overrides.pane_id,
    provider: "claude",
    mode: "deadloop",
    session_id: `pane-${overrides.pane_id}`,
    is_paused: false,
    label: overrides.label,
    role: overrides.role,
    managed: true,
  };
}

function seedTeamSetupCard(overrides: Partial<{
  paneConfigs: PaneConfig[];
  startTeam: ReturnType<typeof vi.fn>;
}> = {}) {
  const startTeam = overrides.startTeam ?? vi.fn();

  act(() => {
    useStore.setState({
      paneConfigs: overrides.paneConfigs ?? [],
      startTeam: startTeam as StoreState["startTeam"],
    });
  });

  return { startTeam };
}

function roleSelects(): HTMLSelectElement[] {
  return screen.getAllByTitle("Agent frontend × API backend") as HTMLSelectElement[];
}

describe("TeamSetupCard", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("renders all canonical roles and Start team before managed panes exist", () => {
    seedTeamSetupCard();

    render(<TeamSetupCard />);

    expect(screen.getByText("Team setup")).toBeTruthy();
    expect(screen.getByText("Manager")).toBeTruthy();
    expect(screen.getByText("Tech Lead")).toBeTruthy();
    expect(screen.getByText("Reviewer")).toBeTruthy();
    expect(screen.getByText("Developer")).toBeTruthy();
    expect(screen.getByRole("button", { name: /start team/i })).toBeTruthy();
    expect(screen.getAllByText("Not created")).toHaveLength(4);
    expect(roleSelects().map((select) => select.value)).toEqual([
      "claude/official",
      "claude/official",
      "claude/official",
      "claude/official",
    ]);
  });

  it("starts the team with selected provider and model specs per role", () => {
    const { startTeam } = seedTeamSetupCard();
    render(<TeamSetupCard />);

    const [manager, techLead, reviewer, developer] = roleSelects();
    fireEvent.change(techLead, { target: { value: "claude/deepseek" } });
    fireEvent.change(developer, { target: { value: "codex/official" } });

    fireEvent.click(screen.getByRole("button", { name: /start team/i }));

    expect(manager.value).toBe("claude/official");
    expect(reviewer.value).toBe("claude/official");
    expect(startTeam).toHaveBeenCalledWith({
      manager: { provider: "claude", model: null },
      techLead: { provider: "claude", model: DEEPSEEK_DEFAULT_MODEL },
      reviewer: { provider: "claude", model: null },
      developer: { provider: "codex", model: null },
    });
  });

  it.each([
    managedPane({ pane_id: 10, label: "Manager", role: "team manager" }),
    managedPane({ pane_id: 11, label: "Developer", role: "developer" }),
  ])("hides once managed team pane $label exists", (pane) => {
    seedTeamSetupCard({ paneConfigs: [pane] });

    render(<TeamSetupCard />);

    expect(screen.queryByText("Team setup")).toBeNull();
    expect(screen.queryByRole("button", { name: /start team/i })).toBeNull();
  });
});
