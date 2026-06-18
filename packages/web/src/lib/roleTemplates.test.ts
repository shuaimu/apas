import { describe, expect, it } from "vitest";
import { ROLE_TEMPLATES } from "./roleTemplates";

describe("ROLE_TEMPLATES", () => {
  it("scratchpad-writing templates require append-time timestamps", () => {
    for (const id of ["manager", "tech-lead", "developer", "qa", "reviewer", "researcher"]) {
      const template = ROLE_TEMPLATES.find((candidate) => candidate.id === id);
      expect(template, `missing template ${id}`).toBeTruthy();
      expect(template!.backstory).toContain("generate its ts at append time");
      expect(template!.backstory).toContain("TS=$(date -Iseconds)");
      expect(template!.backstory).toContain("never reuse an earlier planning timestamp");
    }
  });

  it("developer template delegates PR state tracking to the Tech Lead", () => {
    const developer = ROLE_TEMPLATES.find((template) => template.id === "developer");
    expect(developer).toBeTruthy();

    const goal = developer!.goal;
    const backstory = developer!.backstory;

    expect(goal).toContain("publish the pr-opened decision");
    expect(goal).toContain("let the Tech Lead track PR state, comments, and team-todo status");
    expect(backstory).toContain("Do NOT idle-poll your own PR state or comments");
    expect(backstory).toContain("Do not edit team-todo.md");
    expect(backstory).toContain("mark the assigned task done yourself");
    expect(backstory).toContain("The Tech Lead owns PR state tracking, team-todo status");
    expect(backstory).toContain("pr-comments:<url>");
    expect(`${goal}\n${backstory}`).not.toContain("wait for the human to merge");
    expect(`${goal}\n${backstory}`).not.toContain("mark the task done");
    expect(backstory).not.toContain("gh pr view <url>");
    expect(backstory).not.toContain("git -C <worktree> checkout master");
  });

  it("manager template turns direct user requests into approved team todos", () => {
    const manager = ROLE_TEMPLATES.find((template) => template.id === "manager");
    expect(manager).toBeTruthy();

    const goal = manager!.goal;
    const backstory = manager!.backstory;
    const text = `${goal}\n${backstory}`;

    expect(text).toContain("team-todo.md");
    expect(text).toContain("status: approved");
    expect(text).toContain("origin: user");
    expect(backstory).toContain("Direct implementation requests");
    expect(backstory).toContain("project_goal.md");
    expect(backstory).toContain("Tech Lead expands it into worker subtasks");
    expect(backstory).toContain("Don't delegate to worker panes yourself");
    expect(backstory).toContain("Never write production code");
    expect(backstory).not.toContain("outside of project_goal.md, you're in the wrong lane");
  });

  it("tech lead template records worker-opened PRs in team todo", () => {
    const techLead = ROLE_TEMPLATES.find((template) => template.id === "tech-lead");
    expect(techLead).toBeTruthy();

    const backstory = techLead!.backstory;

    expect(backstory).toContain('kind: "diff"');
    expect(backstory).toContain("hand off to the Reviewer pane");
    expect(backstory).toContain("worker opens its own PR");
    expect(backstory).toContain('kind: "decision"');
    expect(backstory).toContain("pr-opened");
    expect(backstory).toContain("team-todo.md");
    expect(backstory).toContain("canonical pr: <pane_id> <url>");
    expect(backstory).toContain("pr_open");
    expect(backstory).toContain("PR state refresh");
    expect(backstory).toContain("pr-comments:<url>");
    expect(backstory).not.toContain(
      "escalate to the Manager so the user can review and merge via the GitHub PR",
    );
    expect(backstory).not.toContain("Track each PR's state on the scratchpad");
  });
});
