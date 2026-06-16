import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ProjectGoalBar } from "./ProjectGoalBar";
import { useStore, type PaneConfig } from "@/lib/store";

function seedProjectGoalBar(overrides: Partial<{
  paneConfigs: PaneConfig[];
  pausedPanes: number[];
  addPane: ReturnType<typeof vi.fn>;
  pausePane: ReturnType<typeof vi.fn>;
  resumePane: ReturnType<typeof vi.fn>;
}> = {}) {
  useStore.setState({
    sessionId: "test-session",
    projectGoals: { "test-session": "Ship APAS team mode" },
    paneConfigs: overrides.paneConfigs ?? [],
    pausedPanes: overrides.pausedPanes ?? [],
    addPane: overrides.addPane ?? vi.fn(() => ({ success: true })),
    pausePane: overrides.pausePane ?? vi.fn(),
    resumePane: overrides.resumePane ?? vi.fn(),
    interruptPane: vi.fn(),
    updateProjectGoal: vi.fn(),
    sendMessageToPane: vi.fn(() => ({ success: true })),
    showToast: vi.fn(),
  });
}

describe("ProjectGoalBar team role slots", () => {
  beforeEach(() => {
    seedProjectGoalBar();
  });

  afterEach(() => {
    useStore.setState({
      sessionId: null,
      projectGoals: {},
      paneConfigs: [],
      pausedPanes: [],
    });
  });

  it("renders all canonical roles before launch with provider/model pickers", () => {
    render(<ProjectGoalBar />);

    expect(screen.getByText("Manager")).toBeTruthy();
    expect(screen.getByText("Tech Lead")).toBeTruthy();
    expect(screen.getByText("Developer")).toBeTruthy();
    expect(screen.getByText("Code Reviewer")).toBeTruthy();
    expect(screen.getAllByText("not created")).toHaveLength(4);

    expect(screen.getByLabelText("Manager provider/model")).toHaveProperty(
      "value",
      "claude/official",
    );
    expect(screen.getByLabelText("Developer provider/model")).toHaveProperty(
      "value",
      "claude/official",
    );
  });

  it("launches a missing developer with selected provider/model and template metadata", () => {
    const addPane = vi.fn(() => ({ success: true }));
    seedProjectGoalBar({ addPane });
    render(<ProjectGoalBar />);

    fireEvent.change(screen.getByLabelText("Developer provider/model"), {
      target: { value: "claude/deepseek" },
    });

    const developerSlot = screen
      .getByText("Developer")
      .closest("div[class*='rounded border']");
    const launchButton = developerSlot?.querySelector("button");
    expect(launchButton).toBeTruthy();
    fireEvent.click(launchButton as HTMLButtonElement);

    expect(addPane).toHaveBeenCalledWith(
      "claude",
      "deadloop",
      "Developer",
      undefined,
      "deepseek-v4-pro",
      true,
      expect.objectContaining({
        role: "developer",
        goal: expect.stringContaining("Implement the leaf task"),
        backstory: expect.stringContaining("You are a hands-on implementer"),
        planReviewMode: "never",
      }),
      true,
    );
  });

  it("shows launched slots with pane id and pause/resume controls", () => {
    const resumePane = vi.fn();
    seedProjectGoalBar({
      paneConfigs: [
        {
          pane_id: 77,
          role: "tech lead",
          mode: "deadloop",
          label: "Tech Lead",
          provider: "claude",
          model: "MiniMax-M2.7",
          managed: true,
        } as PaneConfig,
      ],
      pausedPanes: [77],
      resumePane,
    });

    render(<ProjectGoalBar />);

    expect(screen.getByText("paused")).toBeTruthy();
    expect(screen.getByText("77")).toBeTruthy();
    expect(screen.getAllByText("Claude / MiniMax 2.7").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByTitle("Resume Tech Lead"));
    expect(resumePane).toHaveBeenCalledWith(77);
  });
});
