import { describe, expect, it } from "vitest";
import {
  botPromptForPane,
  CLASSIC_TODO_BOT_LOOP_PROMPT,
  defaultBotPromptForPane,
  deriveInitialActiveTabId,
  OVERVIEW_PANE_ID,
} from "./TabbedView";

describe("deriveInitialActiveTabId", () => {
  const base = {
    activeTabId: null,
    clientChanged: true,
    managerTabId: null,
    paneConfigsLength: 2,
    savedActiveTab: "",
    tabIds: [10, 20],
  };

  it("keeps a valid saved active tab", () => {
    expect(
      deriveInitialActiveTabId({
        ...base,
        managerTabId: 10,
        savedActiveTab: "20",
      }),
    ).toBe(20);
  });

  it("prefers the interactive Manager tab when there is no valid saved tab", () => {
    expect(
      deriveInitialActiveTabId({
        ...base,
        managerTabId: 10,
        savedActiveTab: "",
      }),
    ).toBe(10);
  });

  it("falls back to Overview when there is no Manager and no valid saved tab", () => {
    expect(
      deriveInitialActiveTabId({
        ...base,
        managerTabId: null,
        savedActiveTab: "",
      }),
    ).toBe(OVERVIEW_PANE_ID);
  });

  it("ignores stale saved tabs and still uses Manager or Overview fallback", () => {
    expect(
      deriveInitialActiveTabId({
        ...base,
        managerTabId: 10,
        savedActiveTab: "999",
      }),
    ).toBe(10);

    expect(
      deriveInitialActiveTabId({
        ...base,
        managerTabId: null,
        savedActiveTab: "999",
      }),
    ).toBe(OVERVIEW_PANE_ID);
  });
});

describe("botPromptForPane", () => {
  it("uses explicit saved prompts before managed or classic fallbacks", () => {
    expect(
      botPromptForPane({
        prompt: "Keep doing the custom thing.",
        managed: true,
        role: "developer",
      }),
    ).toBe("Keep doing the custom thing.");
  });

  it("uses role metadata for managed team panes instead of the classic TODO loop", () => {
    const prompt = botPromptForPane({
      managed: true,
      role: "developer",
      goal: "Implement delegated team TODO leaves.",
      backstory: "Stay inside the assigned worktree.",
    });

    expect(prompt).toContain("managed team worker");
    expect(prompt).toContain("Role: developer");
    expect(prompt).toContain("team-todo.md");
    expect(prompt).toContain("Stay inside the assigned worktree.");
    expect(prompt).not.toContain("Work on tasks defined in TODO.md");
  });

  it("keeps the classic TODO.md loop for unmanaged manual bot panes", () => {
    expect(botPromptForPane({ managed: false })).toBe(CLASSIC_TODO_BOT_LOOP_PROMPT);
    expect(CLASSIC_TODO_BOT_LOOP_PROMPT).toContain("TODO.md");
  });

  it("can ignore saved prompts when callers need the default fallback", () => {
    const prompt = defaultBotPromptForPane({
      prompt: "Old saved prompt.",
      managed: true,
      role: "reviewer",
    });

    expect(prompt).toContain("managed team worker");
    expect(prompt).toContain("Role: reviewer");
    expect(prompt).not.toContain("Old saved prompt.");
  });
});
