"use client";

/**
 * Manager v1 — "Project goal" input at the top of the Overview tab.
 *
 * Finds (or creates) a Tech-Lead pane and routes the user's goal to it.
 * The manager pane uses the existing Phase 3.1b manager addendum + the
 * Phase 3.1a `delegate-to:` scratchpad protocol to drive worker panes.
 *
 * Drive model in v1: USER input drives the manager (manager is interactive,
 * not deadloop). v2 will optionally enable deadloop self-pacing once we see
 * whether the autonomous-grind mode is worth the token cost.
 */
import { useEffect, useMemo, useState } from "react";
import { ChevronsRight } from "lucide-react";
import { useStore, type PaneConfig } from "@/lib/store";
import { ROLE_TEMPLATES } from "@/lib/roleTemplates";

export function ProjectGoalBar() {
  const paneConfigs = useStore((s) => s.paneConfigs);
  const addPane = useStore((s) => s.addPane);
  const sendMessageToPane = useStore((s) => s.sendMessageToPane);
  const updatePaneRole = useStore((s) => s.updatePaneRole);
  const updatePaneReviewMode = useStore((s) => s.updatePaneReviewMode);
  const showToast = useStore((s) => s.showToast);

  const [goal, setGoal] = useState("");
  const [pending, setPending] = useState<string | null>(null);
  // Pane IDs that existed BEFORE the user clicked "Create manager and
  // start". Used to identify the new pane in the fallback path when the
  // CLI is on a pre-26.05.65 binary and didn't honor role/goal/backstory
  // on AddPane (so the role-substring match returns nothing).
  const [knownIdsAtCreate, setKnownIdsAtCreate] = useState<Set<number> | null>(
    null,
  );

  // First pane whose role contains "manager" (case-insensitive). The same
  // substring match that activates the manager system-prompt addendum in
  // role::compose_system_prompt, so the UI and the prompt stay aligned.
  const managerPane = useMemo(
    () =>
      paneConfigs.find((p) =>
        (p.role ?? "").toLowerCase().includes("manager"),
      ),
    [paneConfigs],
  );

  // After "Create manager and start", wait for the new pane to appear in
  // paneConfigs, then route the queued goal there.
  useEffect(() => {
    if (!pending) return;
    // Prefer a pane with proper "manager" role (CLI >= 26.05.65 path).
    // Fall back to the newest pane we created (when CLI < 26.05.65 dropped
    // role/goal/backstory from AddPane). The fallback also patches the
    // role retroactively via UpdatePaneRole so the next spawn picks up
    // the manager addendum without the user re-creating the pane.
    let candidate = managerPane;
    let needsRolePatch = false;
    if (!candidate && knownIdsAtCreate) {
      const fresh = paneConfigs.find(
        (p) => !knownIdsAtCreate.has(p.pane_id),
      );
      if (fresh) {
        candidate = fresh;
        needsRolePatch = true;
      }
    }
    if (!candidate) return;

    const techLead = ROLE_TEMPLATES.find((t) => t.id === "tech-lead");
    if (needsRolePatch && techLead) {
      updatePaneRole(candidate.pane_id, techLead.role, techLead.goal, techLead.backstory);
      updatePaneReviewMode(candidate.pane_id, techLead.planReviewMode);
    }

    // Message body: when the CLI didn't apply the manager addendum
    // (needsRolePatch), inline the protocol so the agent knows what to
    // do on this very first message. On properly-set-up panes, the
    // addendum from compose_system_prompt is already in the system
    // prompt and we don't repeat ourselves.
    const protocolHint = needsRolePatch
      ? "[Note: your role wasn't applied at spawn (CLI predates the AddPane wire change). You are the team manager / tech lead — break this goal into small leaves, delegate each to a worker pane via .apas-team.jsonl with tags delegate-to:<pane_id> and task:<id>, and track replies via reply-to:<id>. Don't write production code yourself. Reboot the CLI to get the proper manager addendum on next spawn.]\n\n"
      : "";
    const message =
      protocolHint + composeManagerMessage(paneConfigs, candidate, pending);
    const result = sendMessageToPane(message, candidate.pane_id);
    if (result.success) {
      showToast(
        needsRolePatch
          ? "Goal sent — role patched retroactively. Reboot the CLI for the proper addendum."
          : "Goal sent to manager.",
        needsRolePatch ? "info" : "success",
      );
      setPending(null);
      setKnownIdsAtCreate(null);
    } else {
      showToast(result.error ?? "Failed to reach manager", "error");
    }
  }, [
    pending,
    managerPane,
    paneConfigs,
    knownIdsAtCreate,
    sendMessageToPane,
    updatePaneRole,
    updatePaneReviewMode,
    showToast,
  ]);

  const handleSubmit = () => {
    const text = goal.trim();
    if (!text) return;
    if (managerPane) {
      const message = composeManagerMessage(paneConfigs, managerPane, text);
      const result = sendMessageToPane(message, managerPane.pane_id);
      if (result.success) {
        showToast("Goal sent to manager.", "success");
        setGoal("");
      } else {
        showToast(result.error ?? "Failed to reach manager", "error");
      }
      return;
    }
    // No manager pane yet — create one from the Tech Lead template and
    // queue the goal for delivery once the pane appears.
    const techLead = ROLE_TEMPLATES.find((t) => t.id === "tech-lead");
    if (!techLead) {
      showToast("Tech Lead template missing — cannot create manager.", "error");
      return;
    }
    // Snapshot the current pane IDs so the useEffect fallback can find
    // the new one even if the CLI doesn't apply our requested role.
    setKnownIdsAtCreate(new Set(paneConfigs.map((p) => p.pane_id)));
    const result = addPane(
      "claude",
      "interactive",
      "Manager",
      undefined,
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
      setPending(text);
      setGoal("");
      showToast("Manager spawning — goal queued.", "info");
    } else {
      setKnownIdsAtCreate(null);
      showToast(result.error ?? "Failed to create manager", "error");
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Cmd/Ctrl+Enter submits — Enter alone inserts a newline so multi-line
    // goals don't accidentally submit halfway through typing.
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      handleSubmit();
    }
  };

  const buttonLabel = managerPane
    ? "Send to manager"
    : pending
      ? "Waiting for manager…"
      : "Create manager and start";

  return (
    <section className="mb-6 rounded border border-violet-300 bg-violet-50 p-4 dark:border-violet-800 dark:bg-violet-950/30">
      <div className="mb-2 flex flex-wrap items-baseline gap-2">
        <h3 className="text-sm font-semibold uppercase tracking-wide text-violet-700 dark:text-violet-300">
          Project goal
        </h3>
        {managerPane ? (
          <span className="text-xs text-violet-600 dark:text-violet-400">
            → routed to{" "}
            <span className="font-mono">{managerPane.label || `Pane ${managerPane.pane_id}`}</span>{" "}
            (manager)
          </span>
        ) : (
          <span className="text-xs text-violet-600 dark:text-violet-400">
            no manager pane yet — submitting will spawn one with the Tech Lead template
          </span>
        )}
      </div>
      <textarea
        value={goal}
        onChange={(e) => setGoal(e.target.value)}
        onKeyDown={handleKeyDown}
        rows={3}
        placeholder="What does the team need to accomplish? (Cmd/Ctrl+Enter to send)"
        className="w-full rounded border border-violet-300 bg-white p-2 text-sm text-gray-900 placeholder-gray-400 dark:border-violet-800 dark:bg-gray-900 dark:text-gray-100"
      />
      <div className="mt-2 flex items-center justify-between gap-2">
        <p className="text-[11px] text-violet-700/80 dark:text-violet-300/80">
          The manager will delegate to worker panes via{" "}
          <span className="font-mono">.apas-team.jsonl</span>. Add workers from
          the Add worker button or let the manager describe the team it needs.
        </p>
        <button
          type="button"
          onClick={handleSubmit}
          disabled={!goal.trim() || !!pending}
          className="flex items-center gap-1.5 rounded border border-violet-500 bg-violet-600 px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50"
        >
          <ChevronsRight className="h-4 w-4" />
          {buttonLabel}
        </button>
      </div>
    </section>
  );
}

/** Builds the "[Current team: ...]" header that rides along with each
 *  goal message so the manager has up-to-date sibling info without needing
 *  a respawn. The manager addendum from Phase 3.1b is generic — this is the
 *  per-message snapshot of who's currently on the roster. */
function composeManagerMessage(
  panes: PaneConfig[],
  managerPane: PaneConfig,
  userText: string,
): string {
  const others = panes
    .filter((p) => p.pane_id !== managerPane.pane_id)
    .sort((a, b) => a.pane_id - b.pane_id);
  let roster: string;
  if (others.length === 0) {
    roster =
      "[Current team: no worker panes yet. If you need workers (developer / qa / reviewer / researcher / devops), describe the team you want and ask the user to add them — or ask the user directly via AskUserQuestion.]";
  } else {
    const lines = others
      .map((p) => {
        const label = p.label || `Pane ${p.pane_id}`;
        const role = p.role ? `, role: ${p.role}` : "";
        const goal = p.goal ? `, owns: ${truncate(p.goal, 120)}` : "";
        const wt = p.worktree_path ? ", isolated worktree" : "";
        return `  - pane_id=${p.pane_id} (${label}${role}${goal}${wt})`;
      })
      .join("\n");
    roster = `[Current team:\n${lines}\nUse delegate-to:<pane_id> on .apas-team.jsonl to assign work.]`;
  }
  return `${roster}\n\nUSER PROJECT GOAL:\n${userText}`;
}

function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  return `${s.slice(0, max - 1)}…`;
}
