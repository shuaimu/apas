import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ProjectGoalBar } from "./ProjectGoalBar";
import { CLAUDE_FABLE_MODEL } from "@/lib/providerOptions";
import { useStore, type PaneConfig } from "@/lib/store";

const DEFAULT_SESSION_ID = "test-session";
const PROJECT_GOAL_SESSION_ID = "session-project-goal";

function seedProjectGoalBar(overrides: Partial<{
  sessionId: string;
  projectGoals: Record<string, string>;
  paneConfigs: PaneConfig[];
  pausedPanes: number[];
  addPane: ReturnType<typeof vi.fn>;
  updateProjectGoal: ReturnType<typeof vi.fn>;
  pausePane: ReturnType<typeof vi.fn>;
  resumePane: ReturnType<typeof vi.fn>;
  sendMessageToPane: ReturnType<typeof vi.fn>;
}> = {}) {
  const sessionId = overrides.sessionId ?? DEFAULT_SESSION_ID;

  act(() => {
    useStore.setState({
      sessionId,
      projectGoals: overrides.projectGoals ?? { [sessionId]: "Ship APAS team mode" },
      paneConfigs: overrides.paneConfigs ?? [],
      pausedPanes: overrides.pausedPanes ?? [],
      addPane: overrides.addPane ?? vi.fn(() => ({ success: true })),
      updateProjectGoal: overrides.updateProjectGoal ?? vi.fn(),
      pausePane: overrides.pausePane ?? vi.fn(),
      resumePane: overrides.resumePane ?? vi.fn(),
      interruptPane: vi.fn(),
      sendMessageToPane: overrides.sendMessageToPane ?? vi.fn(() => ({ success: true })),
      showToast: vi.fn(),
    });
  });
}

function roleSlot(label: string): HTMLElement {
  const labelElement =
    screen
      .getAllByText(label)
      .find((element) => element.className.includes("font-semibold")) ??
    screen.getByText(label);
  const slot = labelElement.closest("div[class*='rounded border']");
  expect(slot).toBeTruthy();
  return slot as HTMLElement;
}

describe("ProjectGoalBar team role slots", () => {
  beforeEach(() => {
    seedProjectGoalBar();
  });

  afterEach(() => {
    act(() => {
      useStore.setState({
        sessionId: null,
        projectGoals: {},
        paneConfigs: [],
        pausedPanes: [],
      });
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

  it("launches a missing developer with the shared Claude Fable option", () => {
    const addPane = vi.fn(() => ({ success: true }));
    seedProjectGoalBar({ addPane });
    render(<ProjectGoalBar />);

    fireEvent.change(screen.getByLabelText("Developer provider/model"), {
      target: { value: "claude/fable" },
    });

    const developerSlot = roleSlot("Developer");
    const launchButton = within(developerSlot).getByRole("button", {
      name: /launch/i,
    });
    fireEvent.click(launchButton);

    expect(addPane).toHaveBeenCalledWith(
      "claude",
      "deadloop",
      "Developer",
      undefined,
      CLAUDE_FABLE_MODEL,
      true,
      expect.objectContaining({
        role: "developer",
        goal: expect.stringContaining("Implement the leaf task"),
        planReviewMode: "never",
      }),
      true,
    );
  });

  it("launches a missing Manager with the current goal and managed role metadata", async () => {
    const addPane = vi.fn(() => ({ success: true }));
    const updateProjectGoal = vi.fn();
    seedProjectGoalBar({ addPane, updateProjectGoal });
    render(<ProjectGoalBar />);

    const textarea = screen.getByPlaceholderText(
      /What does the team need to accomplish/,
    ) as HTMLTextAreaElement;
    await waitFor(() => {
      expect(textarea.value).toBe("Ship APAS team mode");
    });

    fireEvent.click(within(roleSlot("Manager")).getByRole("button", { name: /launch/i }));

    expect(addPane).toHaveBeenCalledWith(
      "claude",
      "interactive",
      "Manager",
      undefined,
      undefined,
      false,
      expect.objectContaining({
        role: "team manager",
        goal: "Ship APAS team mode",
        backstory: expect.stringContaining("user-facing role"),
        planReviewMode: "never",
      }),
      true,
    );
    expect(updateProjectGoal).toHaveBeenCalledWith("Ship APAS team mode");
  });

  it("launches a missing Tech Lead with remote-aware survey prompt", () => {
    const addPane = vi.fn(() => ({ success: true }));
    seedProjectGoalBar({ addPane });
    render(<ProjectGoalBar />);

    const techLeadSlot = screen
      .getByText("Tech Lead")
      .closest("div[class*='rounded border']");
    const launchButton = techLeadSlot?.querySelector("button");
    expect(launchButton).toBeTruthy();
    fireEvent.click(launchButton as HTMLButtonElement);

    expect(addPane).toHaveBeenCalledTimes(1);
    const prompt = addPane.mock.calls[0]?.[3];
    expect(prompt).toEqual(expect.stringContaining("fetch remote metadata"));
    expect(prompt).toEqual(expect.stringContaining("remote/default-branch drift"));
    expect(prompt).toEqual(expect.stringContaining("origin/HEAD"));
    expect(prompt).toEqual(expect.stringContaining("origin/master"));
    expect(prompt).toEqual(expect.stringContaining("preserve the worktree"));
    expect(prompt).toEqual(expect.stringContaining("git show origin/HEAD:README.md"));
    expect(prompt).toEqual(expect.stringContaining("git show origin/HEAD:CLAUDE.md"));
    expect(prompt).toEqual(expect.stringContaining("Checkout-drift escalations must be based on a fresh status snapshot"));
    expect(prompt).toEqual(expect.stringContaining("checkout-conflict or dirty-worktree escalation"));
    expect(prompt).toEqual(expect.stringContaining("git status --short --branch"));
    expect(prompt).toEqual(expect.stringContaining("base the escalation only on that latest output"));
    expect(prompt).toEqual(expect.stringContaining("latest status contradicts an earlier snapshot"));
    expect(prompt).toEqual(expect.stringContaining("avoid escalating stale evidence"));
    expect(prompt).toEqual(expect.stringContaining("post one concise correction with the current evidence"));
    expect(prompt).toEqual(expect.stringContaining("backlog backpressure"));
    expect(prompt).toEqual(expect.stringContaining("Available managed developer capacity"));
    expect(prompt).toEqual(expect.stringContaining("explicit queue limit"));
    expect(prompt).toEqual(expect.stringContaining("user-approved backlog state is preserved"));
    expect(prompt).toEqual(expect.stringContaining("configured capacity"));
    const promptText = String(prompt);
    expect(promptText).toContain("count all status: proposed Globals across origins");
    expect(promptText).toContain("Cap at 10 outstanding proposed Globals");
    expect(promptText).toContain("count is already 10 or more");
    expect(promptText).toContain("skip the entire proposal step");
    expect(promptText).toContain("user can triage the existing queue");
    expect(promptText).toContain("Otherwise cap at 3/iter");
    expect(promptText).toContain('kind: "diff"');
    expect(promptText).toContain("record the branch/commit details");
    expect(promptText).toContain("team-todo.md");
    expect(promptText).toContain("status: reviewing");
    expect(promptText).toContain("reviewing / approved / done");
    expect(promptText).toContain("under_review");
    expect(promptText).toContain(".apas-tech-lead-cursor");
    expect(promptText).toContain("After each successful scratchpad scan");
    expect(promptText).toContain("newest scratchpad record");
    expect(promptText).toContain("successfully scanned/processed");
    expect(promptText).toContain("ignored records");
    expect(promptText).toContain("records that require no action");
    expect(promptText).toContain("no-op records are not reread forever");
    expect(promptText).toContain('tags ["delegate-to:<reviewer_pane_id>", "task:TODO-NNN"]');
    expect(promptText).toContain("review-worker:<pane_id>");
    expect(promptText).toContain("do not request Reviewer review for that single diff yet");
    const diffRecordIndex = promptText.indexOf('When a worker publishes kind: "diff"');
    const allContributorsIndex = promptText.indexOf(
      "only when every contributor is reviewing / approved / done",
    );
    const reviewerDelegationIndex = promptText.indexOf(
      'tags ["delegate-to:<reviewer_pane_id>", "task:TODO-NNN"]',
      diffRecordIndex,
    );
    expect(diffRecordIndex).toBeGreaterThanOrEqual(0);
    expect(allContributorsIndex).toBeGreaterThan(diffRecordIndex);
    expect(reviewerDelegationIndex).toBeGreaterThan(allContributorsIndex);
    expect(promptText).toContain("Orphan PR reconciliation");
    expect(promptText).toContain("legacy status: done / bare pr: https://github.com/.../pull/N shape");
    expect(promptText).toContain("contributing pane subtask contains clear evidence");
    expect(promptText).toContain("PR opened ... https://github.com/.../pull/N");
    expect(promptText).toContain("canonical pr: <pane_id> <url> lines");
    expect(promptText).toContain("Do not guess or invent PR URLs");
    expect(promptText).toContain("without explicit pane/subtask evidence");
    expect(promptText).toContain(
      "jq '.auto_approve_todos // false, .auto_merge_prs // false' .apas",
    );
    expect(promptText).toContain("auto_approve_todos");
    expect(promptText).toContain("auto_merge_prs");
    expect(promptText).toContain("proposed -> approved");
    expect(promptText).toContain("auto_approve_todos is true");
    expect(promptText).toContain("concrete, bounded, aligned with project_goal.md");
    expect(promptText).toContain("not a duplicate");
    expect(promptText).toContain("gh pr merge <url> --squash --auto");
    expect(promptText).toContain("local Reviewer approval record");
    expect(promptText).toContain("reviewDecision is not CHANGES_REQUESTED");
    expect(promptText).toContain('mergeable == "MERGEABLE"');
    expect(promptText).toContain("CI is green with no pending checks");
    expect(promptText).toContain('mergeable == "CONFLICTING"');
    expect(promptText).toContain("leave the Global pr_open");
    expect(promptText).toContain("do not close the PR");
    expect(promptText).toContain("pr-comments:<url> delegation");
    expect(promptText).toContain("original owner from the pr: <pane_id> <url> line");
    expect(promptText).toContain("rebase/merge the current default branch");
    expect(promptText).toContain("resolve conflicts");
    expect(promptText).toContain("rerun verification");
    expect(promptText).toContain("push the same branch");
    expect(promptText).toContain("not already revising / in_progress");
    expect(promptText).toContain("avoid duplicate conflict delegations");
    expect(promptText).toContain("enablePullRequestAutoMerge");
    expect(promptText).toContain("Auto merge is not allowed for this repository");
    expect(promptText).toContain(
      "gh pr view <url> --json state,statusCheckRollup,reviewDecision,mergeable",
    );
    expect(promptText).toContain('state == "OPEN"');
    expect(promptText).toContain("CI is clean with no stale or long-pending checks");
    expect(promptText).toContain("gh pr merge <url> --squash");
    expect(promptText).toContain("without `--auto`");
    expect(promptText).toContain("refresh PR state before marking done");
    expect(promptText).toContain("never use this fallback for CONFLICTING, UNKNOWN");
    expect(promptText).toContain(
      "do not close or repeatedly escalate solely because auto-merge is disabled",
    );
    expect(promptText).toContain(".apas-tech-lead-pr-comments.json");
    expect(promptText).toContain("gh pr view <url> --json comments,reviews");
    expect(promptText).toContain("createdAt > cursor[url]");
    expect(promptText).toContain("pr-comments:<url>");
    expect(promptText).toContain("Advance cursor[url] only after successful fetches");
    expect(promptText).toContain("skip settled done / rejected Globals");
    expect(promptText).toContain("worker-owned kind: \"decision\"");
    expect(promptText).toContain("tags including \"pr-opened\"");
    expect(promptText).toContain("record pr: <pane_id> <url> lines");
    expect(promptText).toContain("skip proposal creation");
    expect(promptText).toContain("escalate to the Manager");
    expect(promptText).toContain("project_goal.md is empty or trivially short (<200 chars)");
    expect(promptText).not.toContain("may NEVER write 'approved' yourself");
    expect(addPane).toHaveBeenCalledWith(
      "claude",
      "deadloop",
      "Tech Lead",
      expect.stringContaining("team-todo.md"),
      undefined,
      false,
      expect.objectContaining({
        role: "tech lead",
        goal: expect.stringContaining("Autonomous orchestrator"),
        backstory: expect.stringContaining("Tech Lead"),
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

  it("asks the Manager to scan team-mode sources when auto-generating the goal", () => {
    const sendMessageToPane = vi.fn(() => ({ success: true }));
    seedProjectGoalBar({
      sendMessageToPane,
      paneConfigs: [
        {
          pane_id: 42,
          role: "manager",
          mode: "interactive",
          label: "Manager",
          provider: "claude",
          model: "official",
          managed: true,
        } as PaneConfig,
      ],
    });

    render(<ProjectGoalBar />);
    fireEvent.click(screen.getByText("Auto-generate"));

    expect(sendMessageToPane).toHaveBeenCalledTimes(1);
    const [prompt, paneId] = sendMessageToPane.mock.calls[0] ?? [];
    expect(paneId).toBe(42);
    expect(prompt).toEqual(expect.stringContaining("team-todo.md"));
    expect(prompt).toEqual(expect.stringContaining(".apas-team.jsonl"));
    expect(prompt).toEqual(expect.stringContaining(".apas"));
    expect(prompt).toEqual(expect.stringContaining("docs/team-mode.md"));
    expect(prompt).toEqual(expect.stringContaining("docs/todo-driven-workflow.md"));
    expect(prompt).toEqual(expect.stringContaining("TODO.md / ROADMAP.md / CHANGELOG.md only as legacy fallback context"));
    expect(String(prompt).indexOf("team-todo.md")).toBeLessThan(
      String(prompt).indexOf("TODO.md / ROADMAP.md"),
    );
  });

  it("launches Manager and queues auto-generate when no Manager exists yet", async () => {
    const addPane = vi.fn(() => ({ success: true }));
    const sendMessageToPane = vi.fn(() => ({ success: true }));
    seedProjectGoalBar({ addPane, sendMessageToPane });
    render(<ProjectGoalBar />);

    const textarea = screen.getByPlaceholderText(
      /What does the team need to accomplish/,
    ) as HTMLTextAreaElement;
    await waitFor(() => {
      expect(textarea.value).toBe("Ship APAS team mode");
    });

    fireEvent.click(screen.getByText("Auto-generate"));

    expect(addPane).toHaveBeenCalledWith(
      "claude",
      "interactive",
      "Manager",
      undefined,
      undefined,
      false,
      expect.objectContaining({
        role: "team manager",
        goal: "Ship APAS team mode",
      }),
      true,
    );
    expect(sendMessageToPane).not.toHaveBeenCalled();

    act(() => {
      useStore.setState({
        paneConfigs: [
          {
            pane_id: 42,
            role: "manager",
            mode: "interactive",
            label: "Manager",
            provider: "claude",
            model: "official",
            managed: true,
          } as PaneConfig,
        ],
      });
    });

    await waitFor(() => {
      expect(sendMessageToPane).toHaveBeenCalledWith(
        expect.stringContaining("starter project_goal.md"),
        42,
      );
    });
  });

  it("does not show duplicate launches for existing Manager and Tech Lead panes", () => {
    const addPane = vi.fn(() => ({ success: true }));
    const pausePane = vi.fn();
    seedProjectGoalBar({
      addPane,
      pausePane,
      paneConfigs: [
        {
          pane_id: 41,
          role: "manager",
          mode: "interactive",
          label: "Manager",
          provider: "claude",
          model: "official",
          managed: true,
        } as PaneConfig,
        {
          pane_id: 77,
          role: "tech lead",
          mode: "deadloop",
          label: "Tech Lead",
          provider: "claude",
          model: "official",
          managed: true,
        } as PaneConfig,
      ],
    });

    render(<ProjectGoalBar />);

    expect(within(roleSlot("Manager")).queryByRole("button", { name: /launch/i })).toBeNull();
    const techLeadSlot = roleSlot("Tech Lead");
    expect(within(techLeadSlot).queryByRole("button", { name: /launch/i })).toBeNull();

    fireEvent.click(within(techLeadSlot).getByTitle("Pause Tech Lead"));

    expect(pausePane).toHaveBeenCalledWith(77);
    expect(addPane).not.toHaveBeenCalled();
  });

  it("hydrates from projectGoals without clobbering a dirty edit", async () => {
    seedProjectGoalBar({
      sessionId: PROJECT_GOAL_SESSION_ID,
      projectGoals: { [PROJECT_GOAL_SESSION_ID]: "goal from server" },
    });

    render(<ProjectGoalBar />);

    const textarea = screen.getByPlaceholderText(
      /What does the team need to accomplish/,
    ) as HTMLTextAreaElement;

    await waitFor(() => {
      expect(textarea.value).toBe("goal from server");
    });

    act(() => {
      useStore.setState({
        projectGoals: { [PROJECT_GOAL_SESSION_ID]: "updated server goal" },
      });
    });

    await waitFor(() => {
      expect(textarea.value).toBe("updated server goal");
    });

    fireEvent.change(textarea, { target: { value: "local draft" } });
    expect(textarea.value).toBe("local draft");

    act(() => {
      useStore.setState({
        projectGoals: { [PROJECT_GOAL_SESSION_ID]: "late server goal" },
      });
    });

    await waitFor(() => {
      expect(textarea.value).toBe("local draft");
    });
  });
});
