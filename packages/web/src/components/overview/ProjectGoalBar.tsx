"use client";

/**
 * v3 — canonical team control panel on the Overview tab.
 *
 * Renders each canonical team role as a slot before launch, so the user can
 * choose the agent provider/model before creating Manager, Tech Lead,
 * Developer, or Reviewer panes.
 *
 * Project goal text is persisted to `project_goal.md` via the existing
 * UpdateProjectGoal wire. The Manager re-reads the file as needed and
 * also takes user chat as the primary signal.
 *
 * Auto-generate sends a chat message to the Manager asking it to scan
 * the project and populate project_goal.md itself — useful for
 * onboarding an ongoing project.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import {
  Play,
  Save,
  Pause,
  CheckCircle2,
  Sparkles,
  Bot,
  ChevronDown,
  ChevronUp,
} from "lucide-react";
import { useStore, type PaneConfig } from "@/lib/store";
import {
  canonicalTeamRoleTemplates,
  type RoleTemplate,
} from "@/lib/roleTemplates";
import {
  PROVIDER_MODEL_OPTIONS,
  findProviderModelOption,
  providerModelValue,
} from "@/lib/providerOptions";

/**
 * Deadloop prompt for the Tech Lead. Read at every iteration; tells the
 * agent what to do each tick.
 */
const TECH_LEAD_DEADLOOP_PROMPT = `You are this project's Tech Lead, running as an autonomous deadloop.

Every iteration, in order:

1. Read project_goal.md and team-todo.md (the doc IS the source of truth — read with the Read tool, mutate with Write/Edit). Also read the autonomy flags every iteration with \`jq '.auto_approve_todos // false, .auto_merge_prs // false' .apas\`; they default to false and gate auto-approval and auto-merge behavior. If project_goal.md is missing, escalate to the Manager via .apas-team.jsonl (kind: "escalation") and end the iteration.
2. Walk the Global TODOs in team-todo.md and act on each:
   - status: approved with no subtasks — apply backlog backpressure before expanding. Count managed developer panes and their pending / in_progress / revising subtasks. Available managed developer capacity is the number of developers with none of those subtasks. The explicit queue limit is one additional pending subtask per managed developer across the whole queue. Expand only while new subtasks fit within available managed developer capacity plus remaining queue slots; leave the remainder approved with no subtasks so user-approved backlog state is preserved. Do not create more queued subtasks than this configured capacity.
   - status: in_progress — dispatch pending subtasks to their worker via .apas-team.jsonl (kind: "delegation", tags ["delegate-to:<pane_id>", "task:<subtask_id>"]). When every contributing subtask is reviewing / approved / done, flip the Global to under_review AND delegate to the Reviewer with a .apas-team.jsonl kind: "delegation" record tagged ["delegate-to:<reviewer_pane_id>", "task:TODO-NNN"] plus review-worker:<pane_id> tags for the workers being reviewed.
   - Orphan PR reconciliation — if a Global is under_review or accidentally left in a legacy status: done / bare pr: https://github.com/.../pull/N shape, recover only when a contributing pane subtask contains clear evidence like PR opened ... https://github.com/.../pull/N. Normalize the Global to status: pr_open, write canonical pr: <pane_id> <url> lines for the evidenced pane PRs, and leave a short audit note in the subtask/body explaining the recovery. Do not guess or invent PR URLs; without explicit pane/subtask evidence, leave the Global unchanged and wait for a pr-opened decision or escalate.
   - status: pr_open — skip settled done / rejected Globals, then re-check the PR state with gh pr view --json state. Maintain .apas-tech-lead-pr-comments.json as a per-PR comment cursor; for each pr: <pane_id> <url> line, run \`gh pr view <url> --json comments,reviews\`, filter comments/reviews to entries with createdAt > cursor[url], and if any new reviewer comment exists, dispatch one kind: "delegation" record to the PR owner pane with tags ["delegate-to:<pane_id>", "task:<TODO-NNN>", "pr-comments:<url>"]. Advance cursor[url] only after successful fetches; if gh fails, leave the cursor unchanged and retry next iteration. If auto_merge_prs is true, merge with \`gh pr merge <url> --squash --auto\` only when there is a local Reviewer approval record, reviewDecision is not CHANGES_REQUESTED, mergeable == "MERGEABLE", and CI is green with no pending checks. If mergeable == "CONFLICTING", leave the Global pr_open, do not close the PR, and send one pr-comments:<url> delegation to the original owner from the pr: <pane_id> <url> line asking them to rebase/merge the current default branch, resolve conflicts, rerun verification, and push the same branch; set the owner subtask to revising only if it is not already revising / in_progress, and avoid duplicate conflict delegations. If \`gh pr merge <url> --squash --auto\` fails with \`enablePullRequestAutoMerge\` or "Auto merge is not allowed for this repository", re-check with \`gh pr view <url> --json state,statusCheckRollup,reviewDecision,mergeable\`; only if the re-check still has local Reviewer approval, state == "OPEN", mergeable == "MERGEABLE", reviewDecision is not CHANGES_REQUESTED, and CI is clean with no stale or long-pending checks, run \`gh pr merge <url> --squash\` without \`--auto\`, then refresh PR state before marking done. If any gate is not clean, leave the PR open and use the existing coalesced escalation/defer behavior; never use this fallback for CONFLICTING, UNKNOWN, failing checks, stale checks, or missing Reviewer approval; do not close or repeatedly escalate solely because auto-merge is disabled; otherwise leave the PR alone.
3. Read scratchpad records since your last iteration (cursor at .apas-tech-lead-cursor). After each successful scratchpad scan, advance .apas-tech-lead-cursor to the newest scratchpad record you successfully scanned/processed, including ignored records and records that require no action, so no-op records are not reread forever. Look for worker replies, reviewer verdicts, and Manager delegations directed at you (delegate-to:<your_pane_id>). When a worker publishes kind: "diff" with tags including task:TODO-NNN, record the branch/commit details in the matching pane subtask in team-todo.md and set that pane subtask to status: reviewing; do not request Reviewer review for that single diff yet. After recording diff records, check all contributing subtasks for that Global: only when every contributor is reviewing / approved / done, flip the Global to under_review and append one Reviewer delegation with .apas-team.jsonl kind: "delegation" tags ["delegate-to:<reviewer_pane_id>", "task:TODO-NNN"] plus review-worker:<pane_id> tags for the workers being reviewed. When a worker publishes a worker-owned kind: "decision" with tags including "pr-opened", record pr: <pane_id> <url> lines on the matching Global (the worker opens its own PR now; you just record). When every contributing worker has its pr: line, flip the Global to pr_open.
4. **Survey + propose new work.** Every iteration, unconditionally take a pass at proposing follow-on work — this is your standing job, not a fallback.
   - If an origin remote exists, fetch remote metadata before the survey so recently merged PRs are visible. Include remote/default-branch drift by checking origin/HEAD and falling back to origin/master when needed.
   - If the checkout is clean and the local default branch is fast-forwardable, you may fast-forward it before reading survey files. If the checkout is dirty or non-fast-forward, preserve the worktree: do NOT pull, reset, checkout, or otherwise mutate it.
   - Scan the codebase shape: git log --oneline -20, git status, top of project_goal.md and README/CLAUDE. When local README.md or CLAUDE.md may be stale, read those key files from the remote default branch instead with git show origin/HEAD:README.md / git show origin/HEAD:CLAUDE.md, falling back to origin/master.
   - Checkout-drift escalations must be based on a fresh status snapshot: immediately before posting any checkout-conflict or dirty-worktree escalation, rerun git status --short --branch and base the escalation only on that latest output. If the latest status contradicts an earlier snapshot, avoid escalating stale evidence; if you already posted one, post one concise correction with the current evidence.
   - Scan the existing team-todo.md so you don't re-propose near-duplicates and so you can build follow-ons off recently-done entries.
   - Propose 1–3 new Global TODOs each iteration. Append under ## Global TODOs with ### [TODO-NNN] short title, status: proposed, origin: tech-lead, plus a body that cites specific files / recent commits / the gap you're filling.
   - Hard rules: before proposing, count all status: proposed Globals across origins in team-todo.md. Cap at 10 outstanding proposed Globals; if the count is already 10 or more, skip the entire proposal step this iteration so the user can triage the existing queue. Otherwise cap at 3/iter, skip near-duplicates of proposed/approved/in_progress entries (done/rejected are NOT duplicates — v2 proposals are fine), skip if project_goal.md is empty (escalate instead), proposals must name files and a deliverable (not "polish the UI").
   - Proposed entries surface in the Overview's TODO panel for the user to Approve / Reject directly.

CRITICAL: every new Global TODO you write MUST start at status: proposed. The user reads project_goal.md as direction, NOT as pre-approval for specific TODOs. The web user (Approve / Add TODO buttons) and the Manager pane (origin: user, from direct chat) may introduce non-proposed Globals. You may move an existing proposal from proposed -> approved only when auto_approve_todos is true and the proposal is concrete, bounded, aligned with project_goal.md, and not a duplicate. You may always flip status on existing entries through the normal workflow (approved → in_progress → under_review → pr_open → done).

If you've taken the same action recently with no new info, just say "Idle; waiting" and end the iteration to avoid spinning.

Do not chat with the human directly — that's the Manager's job. Escalate via kind: "escalation" on .apas-team.jsonl if you need them.

Do not write production code yourself — your job is design and orchestration. If you find yourself reaching for Write/Edit/Bash on production files (other than team-todo.md and .apas-team.jsonl), delegate to a worker pane instead.`;

const AUTO_GENERATE_CHAT_MESSAGE = `Please scan this project and write a starter project_goal.md.

Read, in order:
1. Existing project_goal.md, if present, so you preserve any user-stated direction.
2. team-todo.md, especially Global TODOs that are proposed, approved, in_progress, pr_open, or recently done.
3. The last ~50 records of .apas-team.jsonl, focusing on delegations, diffs, reviews, decisions, escalations, and pr-opened records.
4. .apas, specifically the managed pane roster and the Manager / Tech Lead / Developer / Reviewer roles currently configured.
5. README.md and CLAUDE.md at the project root, if present.
6. docs/team-mode.md and docs/todo-driven-workflow.md, if present, because they describe the current team-mode workflow.
7. Whichever build/manifest file describes the project — package.json, Cargo.toml, pyproject.toml, go.mod, CMakeLists.txt, etc.
8. \`git log --oneline -50\` for the recent activity shape.
9. TODO.md / ROADMAP.md / CHANGELOG.md only as legacy fallback context if the team-mode sources above are missing or thin.

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

function roleText(p: PaneConfig): string {
  return `${p.role ?? ""} ${p.label ?? ""}`.toLowerCase();
}

function isPaneForTemplate(p: PaneConfig, template: RoleTemplate): boolean {
  if (p.managed !== true) return false;
  const lower = roleText(p);
  switch (template.id) {
    case "manager":
      return (
        p.mode === "interactive" &&
        lower.includes("manager") &&
        !lower.includes("tech lead")
      );
    case "tech-lead":
      return p.mode === "deadloop" && lower.includes("tech lead");
    case "developer":
      return p.mode === "deadloop" && lower.includes("developer");
    case "reviewer":
      return (
        p.mode === "deadloop" &&
        (lower.includes("code reviewer") || lower.includes("reviewer"))
      );
    default:
      return lower.includes(template.role.toLowerCase());
  }
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
  const interruptPane = useStore((s) => s.interruptPane);
  const showToast = useStore((s) => s.showToast);

  // Local mirror of the on-disk project_goal.md. The CLI polls the file's
  // mtime every 3s and pushes ProjectGoalChanged when it changes; we
  // hydrate `goalDraft` from that whenever the user isn't actively
  // editing (`goalDirtySinceSave` tracks the "I'm editing now" state so
  // server pushes don't clobber the user's in-progress typing).
  const [goalDraft, setGoalDraft] = useState("");
  const [goalDirtySinceSave, setGoalDirtySinceSave] = useState(false);
  // Collapsed = small fixed cap with internal scroll; expanded = auto-grow
  // up to 60vh. Default collapsed so a long goal doesn't dominate the
  // Overview. Toggle hidden when the content fits in the collapsed cap.
  const [goalExpanded, setGoalExpanded] = useState(false);
  const [goalOverflows, setGoalOverflows] = useState(false);
  const GOAL_COLLAPSED_PX = 110; // ~5 lines at default font size
  // Queue an auto-generate chat message if the Manager doesn't exist
  // yet at click-time. Sent once the Manager appears in paneConfigs.
  const [pendingAutoGenerate, setPendingAutoGenerate] = useState(false);
  const [slotSelections, setSlotSelections] = useState<Record<string, string>>({});

  const managerPane = useMemo(
    () => paneConfigs.find(isManagerPane),
    [paneConfigs],
  );
  const teamRoleTemplates = useMemo(() => canonicalTeamRoleTemplates(), []);
  const teamRoleSlots = useMemo(
    () =>
      teamRoleTemplates.map((template) => ({
        template,
        pane: paneConfigs.find((p) => isPaneForTemplate(p, template)),
      })),
    [paneConfigs, teamRoleTemplates],
  );

  // Team controls operate on every `managed: true` pane in the project
  // (Manager + Tech Lead + Reviewer + Developer + any user-added
  // workers). Pause/Resume only applies to deadloop panes — interactive
  // ones (the Manager) are always idle-waiting-for-input, so pausing
  // them is a no-op. Stop fans out to every managed pane regardless of
  // mode: interrupt is safe on idle panes too.
  const managedPanes = useMemo(
    () => paneConfigs.filter((p) => p.managed === true),
    [paneConfigs],
  );
  const managedDeadloopPanes = useMemo(
    () => managedPanes.filter((p) => p.mode === "deadloop"),
    [managedPanes],
  );
  // "Team paused" = every managed deadloop pane is in pausedPanes.
  // Empty deadloop set → considered not paused (button stays "Pause team"
  // but disabled below since there's nothing to pause).
  const teamAllPaused =
    managedDeadloopPanes.length > 0 &&
    managedDeadloopPanes.every((p) => pausedPanes.includes(p.pane_id));
  const teamHasAnyDeadloop = managedDeadloopPanes.length > 0;
  const teamHasAnyManaged = managedPanes.length > 0;

  const handlePauseTeam = () => {
    for (const p of managedDeadloopPanes) {
      if (!pausedPanes.includes(p.pane_id)) pausePane(p.pane_id);
    }
    showToast(`Paused ${managedDeadloopPanes.length} team worker(s)`, "info");
  };
  const handleResumeTeam = () => {
    for (const p of managedDeadloopPanes) {
      if (pausedPanes.includes(p.pane_id)) resumePane(p.pane_id);
    }
    showToast(`Resumed ${managedDeadloopPanes.length} team worker(s)`, "info");
  };
  const handleStopTeam = () => {
    if (
      typeof window !== "undefined" &&
      !window.confirm(
        `Stop ${managedPanes.length} managed team pane(s)? Each pane's in-flight turn is interrupted; deadloop workers are also paused so they don't auto-restart on the next file event. Click "Resume team" later to bring them back online.`,
      )
    ) {
      return;
    }
    // Interrupt every managed pane (kills the in-flight turn on
    // deadloop + interactive workers alike). Then pause every managed
    // deadloop pane so they stay quiet — without the pause the loop
    // would tick again as soon as the file watcher fires on any
    // sibling pane's write, defeating "stop".
    for (const p of managedPanes) {
      interruptPane(p.pane_id);
    }
    for (const p of managedDeadloopPanes) {
      if (!pausedPanes.includes(p.pane_id)) pausePane(p.pane_id);
    }
    showToast(
      `Interrupted ${managedPanes.length} pane(s) and paused ${managedDeadloopPanes.length} deadloop worker(s) — click Resume team to bring them back`,
      "info",
    );
  };

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

  // Auto-grow the textarea to fit its content. Two caps: a small
  // `GOAL_COLLAPSED_PX` for the default collapsed state (long goal stays
  // out of the way), and ~60vh when the user clicks Expand. Past either
  // cap the textarea becomes internally scrollable. The `goalOverflows`
  // flag drives whether to show the toggle at all — no point in offering
  // Expand on a 2-line goal.
  const goalTextareaRef = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    const ta = goalTextareaRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    const naturalPx = ta.scrollHeight;
    setGoalOverflows(naturalPx > GOAL_COLLAPSED_PX);
    const capPx = goalExpanded
      ? Math.floor(window.innerHeight * 0.6)
      : GOAL_COLLAPSED_PX;
    ta.style.height = `${Math.min(naturalPx, capPx)}px`;
  }, [goalDraft, goalExpanded]);

  // v3.4 — auto-spawn responsibility moved to the CLI (see
  // dual_pane.rs run_inner). Web no longer auto-spawns on attach to
  // avoid the race where CLI auto-spawn and web auto-spawn both fire
  // before the first PaneList arrives, producing duplicate Manager
  // panes. The team role slots below stay as manual launch controls.

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

  const selectedProviderOption = (template: RoleTemplate) => {
    const value =
      slotSelections[template.id] ??
      providerModelValue(
        template.recommendedProvider ?? "claude",
        template.recommendedModel,
      );
    return findProviderModelOption(value);
  };

  const launchTeamRole = (template: RoleTemplate): boolean => {
    const picked = selectedProviderOption(template);
    const mode = template.teamMode ?? (template.id === "manager" ? "interactive" : "deadloop");
    const prompt = template.id === "tech-lead" ? TECH_LEAD_DEADLOOP_PROMPT : undefined;
    const goal =
      template.id === "manager"
        ? goalDraft.trim() || template.goal
        : template.goal;
    const result = addPane(
      picked.provider,
      mode,
      template.label,
      prompt,
      picked.model,
      template.isolatedWorktree === true,
      {
        role: template.role,
        goal,
        backstory: template.backstory,
        planReviewMode: template.planReviewMode,
      },
      true, // managed — part of the team
    );
    if (result.success) {
      if (template.id === "manager" && goalDraft.trim()) updateProjectGoal(goalDraft.trim());
      showToast(`${template.label} launched.`, "success");
      return true;
    } else {
      showToast(result.error ?? `Failed to launch ${template.label}`, "error");
      return false;
    }
  };

  const handleStartManager = () => {
    const manager = teamRoleTemplates.find((t) => t.id === "manager");
    if (!manager) {
      showToast("Manager template missing — cannot spawn.", "error");
      return false;
    }
    return launchTeamRole(manager);
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
      if (handleStartManager()) {
        setPendingAutoGenerate(true);
        showToast("Manager spawning — will scan + write the goal on first turn.", "info");
      }
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
          {/* Team-wide controls — fan out to every managed pane. Pause
              only touches deadloops (interactive panes can't be paused
              meaningfully); Stop interrupts every managed pane's
              in-flight turn. */}
          {teamHasAnyManaged && (
            <span className="flex items-center gap-1">
              {teamAllPaused ? (
                <button
                  type="button"
                  onClick={handleResumeTeam}
                  disabled={!teamHasAnyDeadloop}
                  className="rounded border border-emerald-500 bg-emerald-600 px-2 py-0.5 text-[11px] text-white hover:bg-emerald-500 disabled:cursor-not-allowed disabled:opacity-50"
                  title="Resume all managed deadloop workers"
                >
                  Resume team
                </button>
              ) : (
                <button
                  type="button"
                  onClick={handlePauseTeam}
                  disabled={!teamHasAnyDeadloop}
                  className="rounded border border-amber-500 bg-amber-600 px-2 py-0.5 text-[11px] text-white hover:bg-amber-500 disabled:cursor-not-allowed disabled:opacity-50"
                  title="Pause all managed deadloop workers (current iteration finishes, next one waits)"
                >
                  Pause team
                </button>
              )}
              <button
                type="button"
                onClick={handleStopTeam}
                className="rounded border border-rose-500 bg-rose-600 px-2 py-0.5 text-[11px] text-white hover:bg-rose-500"
                title="Interrupt every managed pane's current turn AND pause the deadloop workers so they stay quiet until you click Resume team."
              >
                Stop team
              </button>
            </span>
          )}
        </div>
      </div>

      <div className="mb-4 grid grid-cols-1 gap-2 lg:grid-cols-2">
        {teamRoleSlots.map(({ template, pane }) => {
          const selected = selectedProviderOption(template);
          return (
            <TeamRoleSlot
              key={template.id}
              template={template}
              pane={pane}
              paused={pane ? pausedPanes.includes(pane.pane_id) : false}
              selectionValue={selected.value}
              onSelectionChange={(value) =>
                setSlotSelections((current) => ({
                  ...current,
                  [template.id]: value,
                }))
              }
              onLaunch={() => launchTeamRole(template)}
              onPause={() => pane && pausePane(pane.pane_id)}
              onResume={() => pane && resumePane(pane.pane_id)}
            />
          );
        })}
      </div>

      {/* Goal — overwrites project_goal.md */}
      <div className="mb-4">
        <div className="mb-1 flex items-center justify-between gap-2">
          <label className="text-[11px] font-medium uppercase tracking-wide text-violet-700/80 dark:text-violet-300/80">
            Project goal
            <span className="ml-1 text-violet-500/70 dark:text-violet-400/70 normal-case font-normal">
              · written to <span className="font-mono">project_goal.md</span>
            </span>
          </label>
          <div className="flex items-center gap-2">
            {goalDirtySinceSave && (
              <span className="text-[10px] text-amber-600 dark:text-amber-400">
                unsaved
              </span>
            )}
            {goalOverflows && (
              <button
                type="button"
                onClick={() => setGoalExpanded((v) => !v)}
                className="flex items-center gap-0.5 rounded border border-violet-300 bg-white px-1.5 py-0.5 text-[10px] font-medium text-violet-700 hover:bg-violet-100 dark:border-violet-700 dark:bg-gray-900 dark:text-violet-300 dark:hover:bg-violet-900/40"
                title={goalExpanded ? "Collapse goal text" : "Expand goal text"}
              >
                {goalExpanded ? (
                  <>
                    <ChevronUp className="h-3 w-3" /> Shrink
                  </>
                ) : (
                  <>
                    <ChevronDown className="h-3 w-3" /> Expand
                  </>
                )}
              </button>
            )}
          </div>
        </div>
        <textarea
          ref={goalTextareaRef}
          value={goalDraft}
          onChange={(e) => {
            setGoalDraft(e.target.value);
            setGoalDirtySinceSave(true);
          }}
          rows={3}
          placeholder="What does the team need to accomplish? (Manager keeps this in sync from chat.)"
          className="w-full overflow-y-auto rounded border border-violet-300 bg-white p-2 text-sm text-gray-900 placeholder-gray-400 dark:border-violet-800 dark:bg-gray-900 dark:text-gray-100"
          style={{ resize: "none" }}
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
              title="Ask the Manager to scan team-mode sources, recent commits, and docs before writing a starter project_goal.md."
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
          </div>
        </div>
      </div>
    </section>
  );
}

function TeamRoleSlot({
  template,
  pane,
  paused,
  selectionValue,
  onSelectionChange,
  onLaunch,
  onPause,
  onResume,
}: {
  template: RoleTemplate;
  pane?: PaneConfig;
  paused: boolean;
  selectionValue: string;
  onSelectionChange: (value: string) => void;
  onLaunch: () => void;
  onPause: () => void;
  onResume: () => void;
}) {
  const launched = pane !== undefined;
  const currentOption = launched
    ? findProviderModelOption(providerModelValue(pane.provider, pane.model))
    : findProviderModelOption(selectionValue);
  const statusLabel = !pane
    ? "not created"
    : pane.mode === "deadloop"
      ? paused
        ? "paused"
        : "running"
      : "ready";

  return (
    <div className="flex flex-col gap-2 rounded border border-violet-200 bg-white/80 p-3 dark:border-violet-800 dark:bg-gray-950/50">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-base leading-none" aria-hidden="true">
              {template.glyph}
            </span>
            <span className="font-semibold text-gray-900 dark:text-gray-100">
              {template.label}
            </span>
            <span
              className={`rounded px-1.5 py-0.5 text-[10px] font-medium ${
                launched
                  ? paused
                    ? "bg-amber-100 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300"
                    : "bg-emerald-100 text-emerald-800 dark:bg-emerald-950/40 dark:text-emerald-300"
                  : "bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-300"
              }`}
            >
              {statusLabel}
            </span>
            {template.isolatedWorktree && (
              <span className="rounded bg-sky-100 px-1.5 py-0.5 text-[10px] font-medium text-sky-700 dark:bg-sky-950/40 dark:text-sky-300">
                isolated
              </span>
            )}
          </div>
          <div className="mt-1 truncate text-[11px] text-gray-600 dark:text-gray-400">
            {pane ? (
              <>
                pane <span className="font-mono">{pane.pane_id}</span>
                {" · "}
                <span className="font-mono">{pane.label || template.label}</span>
                {" · "}
                {currentOption.label}
              </>
            ) : (
              currentOption.label
            )}
          </div>
        </div>

        {pane && pane.mode === "deadloop" && (
          paused ? (
            <button
              type="button"
              onClick={onResume}
              className="flex shrink-0 items-center gap-1 rounded border border-emerald-500 bg-emerald-600 px-2 py-1 text-[11px] font-medium text-white hover:bg-emerald-500"
              title={`Resume ${template.label}`}
            >
              <Play className="h-3 w-3" /> Resume
            </button>
          ) : (
            <button
              type="button"
              onClick={onPause}
              className="flex shrink-0 items-center gap-1 rounded border border-amber-500 bg-amber-600 px-2 py-1 text-[11px] font-medium text-white hover:bg-amber-500"
              title={`Pause ${template.label}`}
            >
              <Pause className="h-3 w-3" /> Pause
            </button>
          )
        )}
      </div>

      {!pane && (
        <div className="flex flex-wrap items-end gap-2">
          <label className="flex min-w-[13rem] flex-1 flex-col gap-1 text-[11px] font-medium text-gray-700 dark:text-gray-300">
            Provider/model
            <select
              value={selectionValue}
              onChange={(e) => onSelectionChange(e.target.value)}
              aria-label={`${template.label} provider/model`}
              className="rounded border border-violet-300 bg-white px-2 py-1 text-xs font-mono text-gray-900 dark:border-violet-800 dark:bg-gray-900 dark:text-gray-100"
            >
              {PROVIDER_MODEL_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            onClick={onLaunch}
            className="flex items-center gap-1 rounded border border-violet-500 bg-violet-600 px-3 py-1 text-xs font-medium text-white transition-colors hover:bg-violet-500"
          >
            {template.id === "tech-lead" ? (
              <Bot className="h-3 w-3" />
            ) : (
              <Play className="h-3 w-3" />
            )}
            Launch
          </button>
        </div>
      )}
    </div>
  );
}
