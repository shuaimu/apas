import { createElement } from "react";
import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { buildSuggestWorkersPrompt, OverviewView } from "./OverviewView";
import { useStore, type PaneConfig, type TeamTodoState } from "@/lib/store";

const initialStore = useStore.getState();

function emptyTeamTodo(): TeamTodoState {
  return {
    globals: [],
    workers: [],
    tech_lead_cursor: null,
    reviewer_cursor: null,
  };
}

function pane(
  overrides: Partial<PaneConfig> & Pick<PaneConfig, "pane_id" | "label" | "role" | "managed">,
): PaneConfig {
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
  };
}

function seedOverview(paneConfigs: PaneConfig[] = []) {
  const sessionId = "overview-session";
  act(() => {
    useStore.setState({
      sessionId,
      cliClientId: "overview-cli",
      paneConfigs,
      paneStatuses: {},
      pausedPanes: [],
      paneMessages: {},
      paneDiffs: {},
      teamRecords: [],
      teamTodoState: emptyTeamTodo(),
      teamTodoStates: new Map([[sessionId, emptyTeamTodo()]]),
      suggestedWorkersBySession: new Map([[sessionId, []]]),
      usageLimits: new Map(),
      projectGoals: { [sessionId]: "Ship APAS team mode" },
      addPane: vi.fn(() => ({ success: true })),
      fetchTeamTodo: vi.fn(),
      fetchSuggestedWorkers: vi.fn(),
      sendMessageToPane: vi.fn(() => ({ success: true })),
      showToast: vi.fn(),
      startTeam: vi.fn(),
      updateProjectGoal: vi.fn(),
      pausePane: vi.fn(),
      resumePane: vi.fn(),
      interruptPane: vi.fn(),
      promotePaneToManaged: vi.fn(),
    });
  });
  window.localStorage.removeItem("apas_team_todo_collapsed");
}

function renderOverview() {
  return render(
    createElement(OverviewView, {
      onOpenPane: vi.fn(),
      onOpenDiff: vi.fn(),
      onOpenRole: vi.fn(),
      onPausePane: vi.fn(),
      onResumePane: vi.fn(),
      onRemovePane: vi.fn(),
    }),
  );
}

function expectOverviewSurfaces() {
  expect(screen.getByText("Team Overview")).toBeTruthy();
  expect(screen.getAllByText(/Project goal/).length).toBeGreaterThan(0);
  expect(screen.getByText("Team TODO")).toBeTruthy();
  expect(screen.getByText("Team (managed)")).toBeTruthy();
  expect(screen.getByText("Suggested workers")).toBeTruthy();
  expect(screen.getByText("Side chats (unmanaged)")).toBeTruthy();
  expect(screen.getByText("Team scratchpad")).toBeTruthy();
  expect(screen.getByText("Delegation board")).toBeTruthy();
  expect(screen.getByText("Resource use")).toBeTruthy();
  expect(screen.getByText(/No suggestions yet/)).toBeTruthy();
  expect(screen.getByText(/No side chats/)).toBeTruthy();
  expect(screen.getByText(/No scratchpad records yet/)).toBeTruthy();
  expect(screen.getByText(/No delegations seen yet/)).toBeTruthy();
  expect(screen.getByText(/No usage telemetry yet/)).toBeTruthy();
}

afterEach(() => {
  vi.restoreAllMocks();
  window.localStorage.removeItem("apas_team_todo_collapsed");
  act(() => {
    useStore.setState(initialStore, true);
  });
});

describe("buildSuggestWorkersPrompt", () => {
  it("grounds Manager worker suggestions in team TODO and suggestion context", () => {
    const prompt = buildSuggestWorkersPrompt([
      {
        pane_id: 151,
        provider: "codex",
        mode: "interactive",
        session_id: "manager-session",
        is_paused: false,
        role: "team manager",
        label: "Manager",
        managed: true,
      } as PaneConfig,
      {
        pane_id: 178,
        provider: "codex",
        mode: "deadloop",
        session_id: "tech-lead-session",
        is_paused: false,
        role: "tech lead",
        label: "Tech Lead",
        managed: true,
      } as PaneConfig,
      {
        pane_id: 440,
        provider: "codex",
        mode: "interactive",
        session_id: "scratch-session",
        is_paused: false,
        role: "developer",
        label: "Scratch",
        managed: false,
      } as PaneConfig,
    ]);

    expect(prompt).toContain("project_goal.md");
    expect(prompt).toContain("team-todo.md");
    expect(prompt).toContain("suggested-workers.md");
    expect(prompt).toContain(".apas-team.jsonl");
    expect(prompt).toContain(".apas");
    expect(prompt).toContain("proposed, approved, or in_progress Global TODOs");
    expect(prompt).toContain("Do not duplicate existing managed panes");
    expect(prompt).toContain("Do not duplicate existing suggestions");
    expect(prompt).toContain("pane_id=151 (Manager, manager)");
    expect(prompt).toContain("pane_id=178 (Tech Lead, tech-lead)");
    expect(prompt).not.toContain("pane_id=440");
    expect(prompt).toContain("- needs_worktree: yes | no");
  });
});

describe("OverviewView composition", () => {
  it("renders first-run setup and team surfaces for an active session without managed panes", () => {
    seedOverview();

    renderOverview();

    expect(screen.getByText("Team setup")).toBeTruthy();
    expect(screen.getByText(/No team members yet/)).toBeTruthy();
    expectOverviewSurfaces();
  });

  it("hides first-run setup once a managed pane exists while keeping team surfaces visible", () => {
    seedOverview([
      pane({
        pane_id: 568,
        label: "Developer",
        role: "developer",
        managed: true,
      }),
    ]);

    renderOverview();

    expect(screen.queryByText("Team setup")).toBeNull();
    expect(screen.getByTitle("Open Developer")).toBeTruthy();
    expectOverviewSurfaces();
  });
});
