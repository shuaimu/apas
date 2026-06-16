import { describe, expect, it } from "vitest";
import { ROLE_TEMPLATES } from "./roleTemplates";

describe("ROLE_TEMPLATES", () => {
  it("developer template cleans the worktree only after merge", () => {
    const developer = ROLE_TEMPLATES.find((template) => template.id === "developer");
    expect(developer).toBeTruthy();

    const backstory = developer!.backstory;
    const openPos = backstory.indexOf("OPEN");
    const mergedPos = backstory.indexOf("MERGED");
    const checkoutPos = backstory.indexOf("git -C <worktree> checkout master");
    const pullPos = backstory.indexOf("git -C <worktree> pull --ff-only origin master");
    const deletePos = backstory.indexOf("git -C <worktree> branch -D <branch>");
    const closedPos = backstory.indexOf("CLOSED");

    expect(openPos).toBeGreaterThanOrEqual(0);
    expect(mergedPos).toBeGreaterThan(openPos);
    expect(checkoutPos).toBeGreaterThan(mergedPos);
    expect(pullPos).toBeGreaterThan(checkoutPos);
    expect(deletePos).toBeGreaterThan(pullPos);
    expect(closedPos).toBeGreaterThan(deletePos);
  });
});
