# APAS team mode — user guide

A short tour of the "team mode" features in APAS. Everything described here
is opt-in — a single pane with no role and no worktree behaves like classic
APAS.

> **Looking for the TODO-driven workflow?** See
> [`todo-driven-workflow.md`](./todo-driven-workflow.md). It's the
> higher-level protocol layered on top of the primitives in this doc:
> a `team-todo.md` queue the Tech Lead drives, with an auto-spawned
> Manager + Tech Lead + Developer + Reviewer team, user-facing
> Approve/Reject buttons in the web Overview, and one PR per contributing
> worker.
> Most users want to read both — this doc covers the pane mechanics
> (worktrees, scratchpad, plan review); that doc covers how the team
> coordinates around shared goals.

## Mental model

- A **pane** is one teammate. Each pane has its own model/provider, optional
  goal, optional git worktree, and its own thinking loop.
- The default managed team has four roles:
  **Manager** talks with the human and keeps `project_goal.md` current,
  **Tech Lead** turns that goal into `team-todo.md` work, **Developer**
  panes implement approved subtasks, and **Reviewer** panes approve or
  reject developer diffs before PRs are opened.
- The **team scratchpad** (`<project>/.apas-team.jsonl`) is the team's shared
  whiteboard. Anything one pane appends, every other pane (and the Overview
  tab) can read. This is also how delegation and review messages flow.
- Tabs at the top of the project are panes. The pinned **Overview** tab at
  the front shows the Manager chat, Project Goal, Team TODO queue, pane
  grid, scratchpad ticker, and provider usage at a glance.

Two ways to get into team mode:

1. **Add more panes** with the `+` button in the tab bar. Each gets its own
   provider/model/role.
2. **Open an existing project** that already has panes saved in `.apas` —
   those auto-restore on CLI boot.

## Setting up a teammate

1. Click `+` → pick a provider (Claude, Codex, etc.).
2. **Tick "Isolated git worktree"** in the `+` dropdown if you want this pane
   working on its own branch — it gets a separate working tree under
   `.apas-worktrees/pane-<id>/` so its changes can't trample anyone else's.
3. Once the pane spawns, click the purple **Role** button in the pane header
   and fill in:
   - **Role** — short noun like `frontend engineer`, `manager`, `reviewer`.
     Recognized keywords (case-insensitive substring): `manager` and
     `reviewer` flip on the orchestration / review system-prompt addenda.
   - **Goal** — one sentence: what's this pane responsible for.
   - **Backstory** — multi-line context: tech preferences, what they should
     and shouldn't do, project conventions.
   - **Plan review** — see the Plan review section below.
4. Save. The role takes effect on the next spawn (close+reopen the tab, or
   reboot the CLI).

## The team scratchpad

`<project>/.apas-team.jsonl` is one JSON record per line:

```json
{"ts": "...", "pane_id": 716, "tags": ["delegate-to:578", "task:abc"], "kind": "delegation", "body": "..."}
```

Common `kind` values: `delegation` (task handoff, usually Tech Lead →
Developer/Reviewer), `reply` (response to a handoff or escalation), `diff`
(announce a diff is ready), `review` (reviewer's verdict), `status`,
`decision`. Tags are free-form strings; the conventions below are what the
agent prompts know about.

Open the amber **Team** button in any pane header to see the live timeline.
Or use the **Overview** tab — it has a scratchpad ticker section with filter
chips per kind.

## Managed team flow

The current team-mode flow splits coordination across Manager, Tech Lead,
Developer, and Reviewer panes instead of having one manager directly hand
work to one worker.

1. **Manager** is the user-facing pane. It chats with the human, helps draft
   or revise `project_goal.md`, and can translate a direct user request into
   an approved Global TODO. Other panes escalate human-facing questions to
   the Manager with `kind: "escalation"` and a `delegate-to:<manager_pane>`
   tag.
   The legacy `manager-directives.jsonl` / `AddManagerDirective` /
   `ManagerDirective` channel is retired; Manager goal edits now flow through
   `project_goal.md` sync via `WebToServer::UpdateProjectGoal`,
   `ServerToCli::UpdateProjectGoal`, `CliToServer::ProjectGoalChanged`, and
   `ServerToWeb::ProjectGoalChanged`.
2. **Tech Lead** reads `project_goal.md`, `team-todo.md`, `.apas`, and
   `.apas-team.jsonl`. It proposes bounded Global TODOs as
   `status: proposed, origin: tech-lead`, waits for the user to approve or
   reject them in Overview, then dispatches approved subtasks with
   `kind: "delegation"` and tags such as
   `delegate-to:<developer_pane>`, `task:TODO-014`, and
   `task:TODO-014-team-mode-v3-guide`.
3. **Developer** panes watch for `delegate-to:<their_pane_id>` records,
   work in their branch/worktree, commit the subtask, and publish
   `kind: "diff"` records tagged with the TODO id. After Reviewer approval,
   the Developer pushes the branch, opens the GitHub PR, and publishes a
   `kind: "decision"` record tagged `pr-opened`.
4. **Reviewer** panes watch `kind: "diff"` records for assigned developer
   panes. They publish `kind: "review"` records tagged
   `approves:<developer_pane>` or `rejects:<developer_pane>` plus the
   relevant `task:TODO-NNN` tag. Rejections go back to the Developer as a
   fresh delegation; approvals let the Developer open the PR.

The Tech Lead records worker `pr-opened` decisions back into `team-todo.md`
as `pr: <pane_id> <url>` lines and tracks PR state. Developers do not merge
their own PRs and do not poll open PRs for comments; the Tech Lead dispatches
new PR comments back to the PR owner with a `pr-comments:<url>` tag.

## Reviewer pattern

Set a pane's role to include `reviewer`. Its system-prompt addendum teaches
it the review loop:

- It subscribes to `kind: "diff"` records on the scratchpad.
- For each new diff, it publishes `kind: "review"` with
  `tags: ["approves:<pane_id>"]` or `["rejects:<pane_id>"]` plus a critique
  in the body.

In the managed team, the Reviewer usually receives review requests from the
Tech Lead after a Developer publishes a diff. The Reviewer still reads the
diff directly from `.apas-team.jsonl`, verifies it against the matching
`team-todo.md` entry, and posts the verdict on the scratchpad.

## Plan review checkpoint

Per-pane policy that holds tool calls until a human approves:

- **Never** (default) — agent runs tools unsupervised.
- **Risky only** — Write / Edit / MultiEdit / NotebookEdit / Bash / Task are
  held; reads and AskUserQuestion go through.
- **Always** — every tool call is held.

Held calls render as orange cards along the bottom of the page with the raw
tool input pretty-printed. Click **Approve** or **Deny**. Useful when you
want to babysit a high-risk pane (e.g. one that pushes to remote) without
sitting there reading every chunk of output.

Change the policy via the Role modal's **Plan review** dropdown.

## Diff review

When a pane has a worktree, a green **Diff** button appears in its header.
Click it for the unified diff vs. the project's main branch. The modal:

- Splits the diff into collapsible per-file sections with `+N/-N` counts.
- Has **Merge & close** (`git merge --no-ff` then remove the pane) and
  **Discard** (force-remove the pane + delete branch).
- Auto-refreshes when the pane's HEAD moves (3-second poll); no need to
  click Refresh.

When you close a pane that owns a worktree, the same three-option flow
appears: **Leave as branch (safe)**, **Merge into current branch**, or
**Discard everything**.

## Overview tab

Pinned at the front of every project. It is the main team-mode control
surface:

1. **Manager chat / default landing** — open or start the Manager pane and
   talk through the project goal without hunting through worker tabs.
2. **Project Goal** — view, edit, or generate `project_goal.md`. The Manager
   and Tech Lead both treat this file as the high-level source of truth.
3. **Team TODO panel** — inspect Global TODOs and per-pane subtasks,
   approve or reject `status: proposed` work, add direct user TODOs, and use
   PR links/status badges once workers open PRs.
4. **Pane grid** — one card per pane: status pill, mode icon, role chip,
   provider, worktree branch + diff stats, last-activity timestamp, and a
   60-bucket last-hour activity sparkline (flat line = wedged, regular bars
   = healthy). Click a card to jump to that pane; click the inline Diff /
   Role buttons to open the modals without switching.
5. **Team scratchpad ticker** — recent records with filter chips per kind,
   useful for seeing delegations, diffs, reviews, decisions, and escalations
   without opening the raw `.apas-team.jsonl` file.
6. **Resource use** — UsageLimitsDisplay per provider, so you can see how
   close any provider is to its quota cap.

## Concrete starter setup

For a new v3 team-mode project:

1. Open the **Overview** tab.
2. Start or open the **Manager** pane. Describe the desired outcome, or ask
   the Manager to scan the repo and draft `project_goal.md`.
3. Start the **Tech Lead** pane. It reads the goal and proposes concrete
   Global TODOs in `team-todo.md`.
4. Approve or reject proposed Global TODOs in the **Team TODO panel**.
5. Keep the default **Developer** and **Reviewer** panes running. Developers
   implement approved subtasks in isolated worktrees; Reviewers approve or
   reject `kind: "diff"` records.
6. Review and merge the GitHub PRs workers open after Reviewer approval.

You can still add manual panes with the `+` button. For a managed Developer,
use an isolated worktree; for a Reviewer, no worktree is usually needed
because it reads diffs and source files.

## Tips

- The role addendum only kicks in on the **next spawn** — close + reopen
  the tab (or reboot the CLI) after editing the role.
- The Team modal scrolls; the Overview tab's scratchpad ticker is for
  faster scanning. Use either.
- If a pane is stuck or looping, the activity sparkline on its Overview
  card is the fastest diagnostic — a flat line for the last few minutes
  means it's wedged.
- The Diff button only shows for panes with `worktree_path` set. If a pane
  has no Diff button and you wanted one, close it and re-add with the
  "Isolated git worktree" checkbox ticked.
- The scratchpad lives at `<project>/.apas-team.jsonl` (sibling file, not
  inside `.apas`). You can `tail -f` it from a shell if you want to watch
  the team chatter outside the web UI.
