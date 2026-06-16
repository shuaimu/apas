import { describe, expect, it } from "vitest";
import { buildSuggestWorkersPrompt } from "./OverviewView";
import type { PaneConfig } from "@/lib/store";

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
