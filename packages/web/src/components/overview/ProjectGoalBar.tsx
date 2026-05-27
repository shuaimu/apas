"use client";

/**
 * v3 — Manager + Tech Lead control panel on the Overview tab.
 *
 * Two roles, two spawns:
 *   - **Manager** (interactive) — user-facing chat. Owns project_goal.md.
 *     Required for the team to function; spawn first.
 *   - **Tech Lead** (deadloop) — optional autonomous orchestrator.
 *     Spawn when you want the team to grind while you're away.
 *
 * Project goal text is persisted to `project_goal.md` via the existing
 * UpdateProjectGoal wire. The Manager re-reads the file as needed and
 * also takes user chat as the primary signal.
 *
 * Auto-generate sends a chat message to the Manager asking it to scan
 * the project and populate project_goal.md itself — useful for
 * onboarding an ongoing project.
 */
import { useEffect, useMemo, useState } from "react";
import { Play, Save, Pause, CheckCircle2, Sparkles, Bot } from "lucide-react";
import { useStore, type PaneConfig } from "@/lib/store";
import { ROLE_TEMPLATES } from "@/lib/roleTemplates";

/**
 * Deadloop prompt for the Tech Lead. Read at every iteration; tells the
 * agent what to do each tick.
 */
const TECH_LEAD_DEADLOOP_PROMPT = `You are this project's Tech Lead, running as an autonomous deadloop.

Every iteration, in order:

1. Read project_goal.md (the project goal). If missing, escalate to the Manager via .apas-team.jsonl (kind: "escalation") and end the iteration.
2. Read the last ~30 records of .apas-team.jsonl. Pay attention to:
   - Delegations from the Manager (records with tags containing "delegate-to:<your_pane_id>") — treat these as priority goal updates from the human.
   - Worker activity (kind: "diff" / "reply" / "status" from worker panes).
3. Decide what to do this iteration:
   - If the Manager just delegated something to you, plan how to break it down and delegate to workers.
   - If a worker is blocked or has questions, delegate help or revise the plan.
   - If a worker completed work (kind: "diff"), hand off to a Reviewer pane if one exists, or escalate via "escalation" so the Manager can surface the PR to the human.
   - If you've taken the same action recently with no new info, just say "Idle; waiting" and end the iteration to avoid spinning.

Delegate to workers via .apas-team.jsonl with kind: "delegation" and tags ["delegate-to:<worker_pane_id>", "task:<short-id>"]. Workers reply via reply-to:<task_id>.

Do not chat with the human directly — that's the Manager's job. If you need to ask the human something, escalate via kind: "escalation" and let the Manager surface it.

Do not write production code yourself — your job is design and orchestration. If you find yourself reaching for Write/Edit/Bash on production files, delegate to a worker pane instead.`;

const AUTO_GENERATE_CHAT_MESSAGE = `Please scan this project and write a starter project_goal.md.

Read, in order:
1. README.md at the project root (if present).
2. Whichever build/manifest file describes the project — package.json, Cargo.toml, pyproject.toml, go.mod, CMakeLists.txt, etc.
3. \`git log --oneline -50\` for the recent activity shape.
4. TODO.md / ROADMAP.md / CHANGELOG.md if present.
5. The docs/ folder (skim the index or top file).

Then use Write to overwrite project_goal.md with a 3–7 sentence goal: what this project IS in one sentence, what's currently in progress (based on recent commits + TODOs), and what the next meaningful milestone looks like. Be concrete — name specific subsystems, modules, or files where useful.

After the write, briefly summarize what you wrote so I can sanity-check it.`;

function isManagerPane(p: PaneConfig): boolean {
  const lower = (p.role ?? "").toLowerCase();
  return (
    lower.includes("manager") &&
    !lower.includes("tech lead") &&
    p.mode === "interactive"
  );
}

function isTechLeadPane(p: PaneConfig): boolean {
  const lower = (p.role ?? "").toLowerCase();
  return lower.includes("tech lead") && p.mode === "deadloop";
}

export function ProjectGoalBar() {
  const paneConfigs = useStore((s) => s.paneConfigs);
  const pausedPanes = useStore((s) => s.pausedPanes);
  const sessionId = useStore((s) => s.sessionId);
  const projectGoalFromCli = useStore((s) =>
    sessionId ? s.projectGoals[sessionId] : undefined,
  );
  const addPane = useStore((s) => s.addPane);
  const updateProjectGoal = useStore((s) => s.updateProjectGoal);
  const sendMessageToPane = useStore((s) => s.sendMessageToPane);
  const pausePane = useStore((s) => s.pausePane);
  const resumePane = useStore((s) => s.resumePane);
  const showToast = useStore((s) => s.showToast);

  // Local mirror of the on-disk project_goal.md. The CLI polls the file's
  // mtime every 3s and pushes ProjectGoalChanged when it changes; we
  // hydrate `goalDraft` from that whenever the user isn't actively
  // editing (`goalDirtySinceSave` tracks the "I'm editing now" state so
  // server pushes don't clobber the user's in-progress typing).
  const [goalDraft, setGoalDraft] = useState("");
  const [goalDirtySinceSave, setGoalDirtySinceSave] = useState(false);
  // Queue an auto-generate chat message if the Manager doesn't exist
  // yet at click-time. Sent once the Manager appears in paneConfigs.
  const [pendingAutoGenerate, setPendingAutoGenerate] = useState(false);

  const managerPane = useMemo(
    () => paneConfigs.find(isManagerPane),
    [paneConfigs],
  );
  const techLeadPane = useMemo(
    () => paneConfigs.find(isTechLeadPane),
    [paneConfigs],
  );

  const managerPaused = managerPane
    ? pausedPanes.includes(managerPane.pane_id)
    : false;
  const techLeadPaused = techLeadPane
    ? pausedPanes.includes(techLeadPane.pane_id)
    : false;

  // Hydrate goalDraft from the CLI's mirror of project_goal.md (pushed
  // via ProjectGoalChanged whenever the file's mtime changes). Skip the
  // sync when the user is mid-edit so an arriving push doesn't clobber
  // typing. We deliberately do NOT hydrate from pane.goal — that's the
  // agent's role-description, not the project's goal.
  useEffect(() => {
    if (goalDirtySinceSave) return;
    if (projectGoalFromCli === undefined) return;
    setGoalDraft(projectGoalFromCli);
  }, [projectGoalFromCli, goalDirtySinceSave]);

  // After "Auto-generate" with no Manager yet, wait for the Manager to
  // appear, then route the scan-and-write message.
  useEffect(() => {
    if (!pendingAutoGenerate || !managerPane) return;
    const result = sendMessageToPane(AUTO_GENERATE_CHAT_MESSAGE, managerPane.pane_id);
    if (result.success) {
      setPendingAutoGenerate(false);
      showToast("Manager scanning the project — watch the chat for progress.", "info");
    } else {
      showToast(result.error ?? "Failed to reach Manager", "error");
      setPendingAutoGenerate(false);
    }
  }, [pendingAutoGenerate, managerPane, sendMessageToPane, showToast]);

  const handleStartManager = () => {
    const manager = ROLE_TEMPLATES.find((t) => t.id === "manager");
    if (!manager) {
      showToast("Manager template missing — cannot spawn.", "error");
      return;
    }
    const result = addPane(
      "claude",
      "interactive",
      "Manager",
      undefined,
      undefined,
      false,
      {
        role: manager.role,
        goal: goalDraft.trim() || manager.goal,
        backstory: manager.backstory,
        planReviewMode: manager.planReviewMode,
      },
    );
    if (result.success) {
      if (goalDraft.trim()) updateProjectGoal(goalDraft.trim());
      showToast("Manager started — chat with them on the left.", "success");
    } else {
      showToast(result.error ?? "Failed to start Manager", "error");
    }
  };

  const handleStartTechLead = () => {
    const techLead = ROLE_TEMPLATES.find((t) => t.id === "tech-lead");
    if (!techLead) {
      showToast("Tech Lead template missing — cannot spawn.", "error");
      return;
    }
    const result = addPane(
      "claude",
      "deadloop",
      "Tech Lead",
      TECH_LEAD_DEADLOOP_PROMPT,
      undefined,
      false,
      {
        role: techLead.role,
        goal: techLead.goal,
        backstory: techLead.backstory,
        planReviewMode: techLead.planReviewMode,
      },
    );
    if (result.success) {
      showToast("Tech Lead started — autonomous orchestration online.", "success");
    } else {
      showToast(result.error ?? "Failed to start Tech Lead", "error");
    }
  };

  const handleSaveGoal = () => {
    updateProjectGoal(goalDraft);
    setGoalDirtySinceSave(false);
    showToast("Project goal saved.", "success");
  };

  // Auto-generate: if no Manager exists, spawn one and queue the
  // scan-and-write message. Otherwise route immediately.
  const handleAutoGenerate = () => {
    if (!managerPane) {
      handleStartManager();
      setPendingAutoGenerate(true);
      showToast("Manager spawning — will scan + write the goal on first turn.", "info");
      return;
    }
    const result = sendMessageToPane(AUTO_GENERATE_CHAT_MESSAGE, managerPane.pane_id);
    if (result.success) {
      showToast("Manager scanning the project — watch the chat for progress.", "info");
    } else {
      showToast(result.error ?? "Failed to reach Manager", "error");
    }
  };

  return (
    <section className="mb-6 rounded border border-violet-300 bg-violet-50 p-4 dark:border-violet-800 dark:bg-violet-950/30">
      <div className="mb-3 flex flex-wrap items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold uppercase tracking-wide text-violet-700 dark:text-violet-300">
          Team
        </h3>
        <div className="flex flex-wrap items-center gap-3 text-xs">
          {/* Manager status */}
          {managerPane ? (
            <span className="flex items-center gap-1 text-violet-600 dark:text-violet-400">
              <span>
                💬 Manager{" "}
                {managerPaused ? "⏸ paused" : "⏵ running"} ·{" "}
                <span className="font-mono">
                  {managerPane.label || `pane ${managerPane.pane_id}`}
                </span>
              </span>
              {managerPaused ? (
                <button
                  type="button"
                  onClick={() => resumePane(managerPane.pane_id)}
                  className="rounded border border-emerald-500 bg-emerald-600 px-1.5 py-0.5 text-[11px] text-white hover:bg-emerald-500"
                >
                  resume
                </button>
              ) : (
                <button
                  type="button"
                  onClick={() => pausePane(managerPane.pane_id)}
                  className="rounded border border-amber-500 bg-amber-600 px-1.5 py-0.5 text-[11px] text-white hover:bg-amber-500"
                >
                  pause
                </button>
              )}
            </span>
          ) : (
            <span className="text-violet-600 dark:text-violet-400">
              💬 no Manager — start one to chat with the team
            </span>
          )}
          {/* Tech Lead status */}
          {techLeadPane ? (
            <span className="flex items-center gap-1 text-indigo-600 dark:text-indigo-400">
              <span>
                🧭 Tech Lead{" "}
                {techLeadPaused ? "⏸ paused" : "⏵ running"} ·{" "}
                <span className="font-mono">
                  {techLeadPane.label || `pane ${techLeadPane.pane_id}`}
                </span>
              </span>
              {techLeadPaused ? (
                <button
                  type="button"
                  onClick={() => resumePane(techLeadPane.pane_id)}
                  className="rounded border border-emerald-500 bg-emerald-600 px-1.5 py-0.5 text-[11px] text-white hover:bg-emerald-500"
                >
                  resume
                </button>
              ) : (
                <button
                  type="button"
                  onClick={() => pausePane(techLeadPane.pane_id)}
                  className="rounded border border-amber-500 bg-amber-600 px-1.5 py-0.5 text-[11px] text-white hover:bg-amber-500"
                >
                  pause
                </button>
              )}
            </span>
          ) : (
            <span className="text-indigo-600 dark:text-indigo-400">
              🧭 no Tech Lead — optional autonomous orchestration
            </span>
          )}
        </div>
      </div>

      {/* Goal — overwrites project_goal.md */}
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
          placeholder="What does the team need to accomplish? (Manager keeps this in sync from chat.)"
          className="w-full rounded border border-violet-300 bg-white p-2 text-sm text-gray-900 placeholder-gray-400 dark:border-violet-800 dark:bg-gray-900 dark:text-gray-100"
        />
        <div className="mt-2 flex items-center justify-between gap-2">
          <p className="text-[11px] text-violet-700/80 dark:text-violet-300/80">
            Tech Lead re-reads this file at every loop iteration.
          </p>
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              onClick={handleAutoGenerate}
              className="flex items-center gap-1 rounded border border-indigo-500 bg-indigo-600 px-3 py-1 text-xs font-medium text-white transition-colors hover:bg-indigo-500"
              title="Ask the Manager to scan README, build files, recent commits + docs and write a starter project_goal.md."
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
                className="flex items-center gap-1 rounded border border-violet-500 bg-violet-600 px-3 py-1 text-xs font-medium text-white transition-colors hover:bg-violet-500"
              >
                <Play className="h-3 w-3" /> Start Manager
              </button>
            )}
            {managerPane && !techLeadPane && (
              <button
                type="button"
                onClick={handleStartTechLead}
                className="flex items-center gap-1 rounded border border-indigo-500 bg-indigo-600 px-3 py-1 text-xs font-medium text-white transition-colors hover:bg-indigo-500"
                title="Optional autonomous orchestrator — workers will get delegations even when you're away."
              >
                <Bot className="h-3 w-3" /> Start Tech Lead
              </button>
            )}
          </div>
        </div>
      </div>
    </section>
  );
}
