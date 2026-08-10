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

function seedOverview(paneConfigs: PaneConfig[] = [], teamEnabled = true) {
  const sessionId = "overview-session";
  act(() => {
    useStore.setState({
      sessionId,
      cliClientId: "overview-cli",
      // Team surfaces only render when the project has team mode on, so the
      // composition tests have to opt in explicitly.
      sessions: [
        { id: sessionId, projectId: sessionId, status: "active", isShared: false },
      ] as ReturnType<typeof useStore.getState>["sessions"],
      projectFlags: {
        [sessionId]: {
          autoApproveTodos: false,
          autoMergePrs: false,
          teamEnabled,
          disallowedTabTypes: [],
        },
      },
      projectPolicies: {
        [sessionId]: {
          teamAvailable: teamEnabled,
          allowedLaunchProfiles: ["agent:claude:official:default"],
          version: 3,
          projectSuspended: false,
          noncompliantPaneIds: [],
        },
      },
      paneConfigs,
      paneStatuses: {},
      pausedPanes: [],
      paneMessages: {},
      paneDiffs: {},
      teamRecords: [],
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
  expect(screen.getByText("Terminal work (unmanaged)")).toBeTruthy();
  expect(screen.getByText("Team scratchpad")).toBeTruthy();
  expect(screen.getByText("Delegation board")).toBeTruthy();
  expect(screen.getByText("Resource use")).toBeTruthy();
  expect(screen.getByText(/No suggestions yet/)).toBeTruthy();
  expect(screen.getByText(/No terminal panes/)).toBeTruthy();
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

  it("puts the team switch above every other surface", () => {
    // Placement is the point of this component: buried below the goal bar it
    // read as a broken page rather than a switched-off one. Compared by DOM
    // position rather than text offset — the intro paragraph mentions
    // "Project goal", so a string search matches prose, not the bar.
    seedOverview([], true);
    renderOverview();

    const teamPolicy = screen.getByText("Team mode");
    for (const label of ["Team setup", "Team TODO", "Team (managed)"]) {
      const el = screen.getByText(label);
      expect(
        teamPolicy.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_FOLLOWING,
        `${label} should render after the team policy`,
      ).toBeTruthy();
    }
  });

  describe("team mode off", () => {
    it("hides every team surface and says why", () => {
      seedOverview([], false);

      renderOverview();

      // The Overview still renders, led by the read-only policy status that
      // explains why team surfaces are absent.
      expect(screen.getByText("Team Overview")).toBeTruthy();
      expect(screen.getByText("Unavailable")).toBeTruthy();
      expect(screen.getByText(/disabled by the current cluster policy/)).toBeTruthy();

      expect(screen.queryByText("Team setup")).toBeNull();
      expect(screen.queryByText("Team TODO")).toBeNull();
      expect(screen.queryByText("Team (managed)")).toBeNull();
      // The Tech Lead autonomy card is team-only too — those flags mean
      // nothing without a Tech Lead.
      expect(screen.queryByText("Tech Lead autonomy")).toBeNull();
    });

    it("hides them even when managed panes are still around", () => {
      // A team that was running when the owner flipped the switch: the CLI
      // stops those panes, but they can linger in paneConfigs until it
      // reports back. The UI must not keep offering the team either way.
      seedOverview(
        [pane({ pane_id: 568, label: "Developer", role: "developer", managed: true })],
        false,
      );

      renderOverview();

      expect(screen.queryByText("Team (managed)")).toBeNull();
      expect(screen.queryByTitle("Open Developer")).toBeNull();
      expect(screen.getByText(/disabled by the current cluster policy/)).toBeTruthy();
    });

    it("keeps team surfaces once team mode is on", () => {
      seedOverview([], true);

      renderOverview();

      expect(screen.getByText("Available")).toBeTruthy();
      expect(screen.getByText("Team TODO")).toBeTruthy();
      expect(screen.getByText("Tech Lead autonomy")).toBeTruthy();
    });
  });
});
