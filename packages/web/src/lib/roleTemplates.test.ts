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

    expect(goal).toContain("mark the task done");
    expect(goal).toContain("let the Tech Lead track PR state and comments");
    expect(backstory).toContain("Do NOT idle-poll your own PR state or comments");
    expect(backstory).toContain("The Tech Lead owns PR state tracking");
    expect(backstory).toContain("pr-comments:<url>");
    expect(`${goal}\n${backstory}`).not.toContain("wait for the human to merge");
    expect(backstory).not.toContain("gh pr view <url>");
    expect(backstory).not.toContain("git -C <worktree> checkout master");
  });
});
