"use client";

/**
 * Manager v2 — two-channel manager control panel on the Overview tab.
 *
 * Decoupled from chat. The manager pane runs as a deadloop; the user
 * never chats with it directly. Instead:
 *
 * 1. Project goal → written to `project_goal.md` at the project root.
 *    The deadloop's prompt re-reads this on every iteration so a goal
 *    change takes effect at the next loop boundary (not mid-action).
 * 2. Directives → appended to `manager-directives.jsonl`. Same idea —
 *    the deadloop tails this file on each iteration to pick up
 *    strategy nudges.
 *
 * This deliberately uses two files (not the team scratchpad
 * `.apas-team.jsonl`) so user↔manager comms stay separate from the
 * inter-pane delegation chatter.
 *
 * Manager pane controls:
 *  - **Start manager** — only shown when no manager pane exists.
 *    Spawns a deadloop pane with the Tech Lead template and a fixed
 *    "read goal + tail directives + tail scratchpad + decide" prompt.
 *  - **Pause / Resume** — already on every deadloop card (see PaneGrid).
 *  - **Remove** — hidden on the manager pane (PaneGrid checks role).
 */
import { useEffect, useMemo, useState } from "react";
import { Play, Save, Pause, CheckCircle2, Sparkles } from "lucide-react";
import { useStore } from "@/lib/store";
import { ROLE_TEMPLATES } from "@/lib/roleTemplates";

/**
 * Directive body sent when the user clicks "Auto-generate". Asks the
 * manager to scan project artifacts (README, package metadata, recent
 * commits, docs) and write a concrete project_goal.md so the user
 * doesn't have to type one for an ongoing project.
 */
const AUTO_GENERATE_DIRECTIVE = `[Auto-generate goal request from the user]

Scan this project and write a clear project_goal.md. project_goal.md is yours to maintain — using the Write tool to populate it is expected behaviour, not the "don't write production code" path.

Read, in order:
1. README.md (or README.rst / README.txt) at the project root if present.
2. Whichever build/manifest file describes the project — package.json, Cargo.toml, pyproject.toml, go.mod, CMakeLists.txt, etc.
3. \`git log --oneline -50\` for the recent activity shape.
4. TODO.md / ROADMAP.md / CHANGELOG.md if present.
5. The docs/ folder (skim the index or top file).

Synthesize a 3–7 sentence project_goal.md: what this project IS in one sentence, what's currently in progress (based on recent commits + TODOs), and what the next meaningful milestone looks like. Be concrete — name specific subsystems, modules, or files where useful.

Use Write to overwrite project_goal.md with that text. After the write, briefly summarize what you wrote (one paragraph) so the user can sanity-check it on the next directive turn.`;

/**
 * Composed at pane-creation time. The deadloop runs this prompt every
 * iteration; it tells the agent to re-read the file-based channel state
 * before acting.
 */
const MANAGER_DEADLOOP_PROMPT = `You are the team manager / tech lead, running as an autonomous deadloop.

Every iteration, in order:

1. Read project_goal.md (the project goal). If missing, ask the user via AskUserQuestion and skip the rest of this iteration.
2. Read the last ~10 lines of manager-directives.jsonl (recent user directives, oldest to newest). Treat these as the user's most recent strategy nudges — they win over your own earlier plans.
3. Read the last ~30 records of .apas-team.jsonl (team activity: kind values "delegation" / "reply" / "diff" / "review" / "status" / "decision"). Understand what each worker pane is doing.
4. Decide what to do this iteration:
   - If a worker is blocked or has questions, delegate help, revise the plan, or ask the user via AskUserQuestion.
   - If a worker completed work (kind: "diff"), hand off to the reviewer pane (if one exists) or note the PR is ready for the user to merge.
   - If a user directive needs acting on, do that.
   - If you've taken the same action recently with no new info, just say "Idle; waiting" and end the iteration to avoid spinning.

Delegate via .apas-team.jsonl with kind: "delegation" and tags like ["delegate-to:<pane_id>", "task:<short-id>"]. Workers reply with reply-to:<task_id>.

Do not write production code yourself — your job is design and orchestration. If you find yourself reaching for Write/Edit/Bash, delegate to a worker pane instead.

Do not chat with the user through this pane — they communicate via project_goal.md (slow-changing goal) and manager-directives.jsonl (fast-changing nudges). Use AskUserQuestion when you genuinely need an answer to proceed.`;

export function ProjectGoalBar() {
  const paneConfigs = useStore((s) => s.paneConfigs);
  const pausedPanes = useStore((s) => s.pausedPanes);
  const addPane = useStore((s) => s.addPane);
  const updateProjectGoal = useStore((s) => s.updateProjectGoal);
  const addManagerDirective = useStore((s) => s.addManagerDirective);
  const pausePane = useStore((s) => s.pausePane);
  const resumePane = useStore((s) => s.resumePane);
  const showToast = useStore((s) => s.showToast);

  // Goal text persisted on the CLI host's filesystem; we keep a local
  // mirror that the user is editing. There's no server→web sync for
  // file content yet — once the user clicks Save, the value is the
  // current value. (A v3 could fetch the current file content on
  // attach so users see what's there.)
  const [goalDraft, setGoalDraft] = useState("");
  const [goalDirtySinceSave, setGoalDirtySinceSave] = useState(false);

  const managerPane = useMemo(
    () =>
      paneConfigs.find((p) =>
        (p.role ?? "").toLowerCase().includes("manager"),
      ),
    [paneConfigs],
  );
  const managerPaused = managerPane
    ? pausedPanes.includes(managerPane.pane_id)
    : false;

  // Hydrate the goal draft from the manager pane's `goal` field on first
  // mount — that's the closest mirror we have of the on-disk file
  // (Tech Lead template sets a default goal; once the user edits and
  // saves, project_goal.md is the source of truth on disk but the web
  // doesn't currently re-read it).
  useEffect(() => {
    if (managerPane && !goalDraft && !goalDirtySinceSave) {
      setGoalDraft(managerPane.goal ?? "");
    }
  }, [managerPane, goalDraft, goalDirtySinceSave]);

  const handleStartManager = () => {
    const techLead = ROLE_TEMPLATES.find((t) => t.id === "tech-lead");
    if (!techLead) return;
    const result = addPane(
      "claude",
      "deadloop",
      "Manager",
      MANAGER_DEADLOOP_PROMPT,
      undefined,
      false,
      {
        role: techLead.role,
        goal: goalDraft.trim() || techLead.goal,
        backstory: techLead.backstory,
        planReviewMode: techLead.planReviewMode,
      },
    );
    if (result.success) {
      // Also persist the current goal text to project_goal.md so the
      // deadloop's first iteration finds it.
      if (goalDraft.trim()) {
        updateProjectGoal(goalDraft.trim());
      }
      showToast("Manager started.", "success");
    } else {
      showToast(result.error ?? "Failed to start manager", "error");
    }
  };

  const handleSaveGoal = () => {
    updateProjectGoal(goalDraft);
    setGoalDirtySinceSave(false);
    showToast("Project goal saved.", "success");
  };

  // Auto-generate: if no manager exists, spawn one first, then queue
  // the scan-and-write directive. The deadloop picks it up at the next
  // iteration boundary. Onboards an existing project in one click.
  const handleAutoGenerate = () => {
    if (!managerPane) {
      const techLead = ROLE_TEMPLATES.find((t) => t.id === "tech-lead");
      if (!techLead) {
        showToast("Tech Lead template missing — cannot spawn manager.", "error");
        return;
      }
      const result = addPane(
        "claude",
        "deadloop",
        "Manager",
        MANAGER_DEADLOOP_PROMPT,
        undefined,
        false,
        {
          role: techLead.role,
          goal: techLead.goal,
          backstory: techLead.backstory,
          planReviewMode: techLead.planReviewMode,
        },
      );
      if (!result.success) {
        showToast(result.error ?? "Failed to spawn manager", "error");
        return;
      }
    }
    addManagerDirective(AUTO_GENERATE_DIRECTIVE);
    showToast(
      managerPane
        ? "Manager will scan the project + write the goal at next iteration."
        : "Manager spawning — it will scan + write the goal on its first iteration.",
      "info",
    );
  };

  return (
    <section className="mb-6 rounded border border-violet-300 bg-violet-50 p-4 dark:border-violet-800 dark:bg-violet-950/30">
      <div className="mb-3 flex flex-wrap items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold uppercase tracking-wide text-violet-700 dark:text-violet-300">
          Manager
        </h3>
        {managerPane ? (
          <div className="flex items-center gap-2 text-xs text-violet-600 dark:text-violet-400">
            <span>
              {managerPaused ? (
                <>⏸ paused</>
              ) : (
                <>⏵ running</>
              )}{" "}
              ·{" "}
              <span className="font-mono">
                {managerPane.label || `Pane ${managerPane.pane_id}`}
              </span>
            </span>
            {managerPaused ? (
              <button
                type="button"
                onClick={() => resumePane(managerPane.pane_id)}
                className="flex items-center gap-1 rounded border border-emerald-500 bg-emerald-600 px-2 py-0.5 text-xs font-medium text-white hover:bg-emerald-500"
              >
                <Play className="h-3 w-3" /> Resume
              </button>
            ) : (
              <button
                type="button"
                onClick={() => pausePane(managerPane.pane_id)}
                className="flex items-center gap-1 rounded border border-amber-500 bg-amber-600 px-2 py-0.5 text-xs font-medium text-white hover:bg-amber-500"
              >
                <Pause className="h-3 w-3" /> Pause
              </button>
            )}
          </div>
        ) : (
          <span className="text-xs text-violet-600 dark:text-violet-400">
            no manager yet — start one to begin autonomous orchestration
          </span>
        )}
      </div>

      {/* Goal — slow-changing, overwrites project_goal.md */}
      <div className="mb-4">
        <div className="mb-1 flex items-center justify-between">
          <label className="text-[11px] font-medium uppercase tracking-wide text-violet-700/80 dark:text-violet-300/80">
            Project goal
            <span className="ml-1 text-violet-500/70 dark:text-violet-400/70 normal-case font-normal">
              · written to <span className="font-mono">project_goal.md</span>
            </span>
          </label>
          {goalDirtySinceSave && (
            <span className="text-[10px] text-amber-600 dark:text-amber-400">
              unsaved
            </span>
          )}
        </div>
        <textarea
          value={goalDraft}
          onChange={(e) => {
            setGoalDraft(e.target.value);
            setGoalDirtySinceSave(true);
          }}
          rows={3}
          placeholder="What does the team need to accomplish?"
          className="w-full rounded border border-violet-300 bg-white p-2 text-sm text-gray-900 placeholder-gray-400 dark:border-violet-800 dark:bg-gray-900 dark:text-gray-100"
        />
        <div className="mt-2 flex items-center justify-between gap-2">
          <p className="text-[11px] text-violet-700/80 dark:text-violet-300/80">
            Manager re-reads this file at the start of every loop iteration.
          </p>
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              onClick={handleAutoGenerate}
              className="flex items-center gap-1 rounded border border-indigo-500 bg-indigo-600 px-3 py-1 text-xs font-medium text-white transition-colors hover:bg-indigo-500"
              title="Ask the manager to scan README, build files, recent commits + docs and write a starter project_goal.md. Useful for onboarding an ongoing project."
            >
              <Sparkles className="h-3 w-3" /> Auto-generate
            </button>
            <button
              type="button"
              onClick={handleSaveGoal}
              disabled={!goalDirtySinceSave}
              className="flex items-center gap-1 rounded border border-violet-500 bg-violet-600 px-3 py-1 text-xs font-medium text-white transition-colors hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {goalDirtySinceSave ? (
                <>
                  <Save className="h-3 w-3" /> Save goal
                </>
              ) : (
                <>
                  <CheckCircle2 className="h-3 w-3" /> Saved
                </>
              )}
            </button>
            {!managerPane && (
              <button
                type="button"
                onClick={handleStartManager}
                className="flex items-center gap-1 rounded border border-emerald-500 bg-emerald-600 px-3 py-1 text-xs font-medium text-white transition-colors hover:bg-emerald-500"
              >
                <Play className="h-3 w-3" /> Start manager
              </button>
            )}
          </div>
        </div>
      </div>

    </section>
  );
}
