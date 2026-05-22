# APAS team mode — user guide

A short tour of the "team mode" features in APAS. Everything described here
is opt-in — a single pane with no role and no worktree behaves like classic
APAS.

## Mental model

- A **pane** is one teammate. Each pane has its own model/provider, optional
  goal, optional git worktree, and its own thinking loop.
- The **team scratchpad** (`<project>/.apas-team.jsonl`) is the team's shared
  whiteboard. Anything one pane appends, every other pane (and the Overview
  tab) can read. This is also how delegation and review messages flow.
- Tabs at the top of the project are panes. The pinned **Overview** tab at
  the front shows the whole team at a glance.

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

Common `kind` values: `delegation` (manager → worker), `reply` (worker →
manager), `diff` (announce a diff is ready), `review` (reviewer's verdict),
`status`, `decision`. Tags are free-form strings; the conventions below are
what the agent prompts know about.

Open the amber **Team** button in any pane header to see the live timeline.
Or use the **Overview** tab — it has a scratchpad ticker section with filter
chips per kind.

## Manager + worker pattern

Set one pane's role to include `manager` (e.g. `team manager`). On next
spawn that pane gets a system-prompt addendum teaching it the delegation
protocol:

- To delegate work, append a record with `tags: ["delegate-to:<pane_id>"]`
  and `kind: "delegation"`. The receiving pane gets the `body` injected
  straight into its input queue — no user click needed.
- The manager assigns each delegation a task id and includes it as
  `task:<id>`; the worker replies with `tags: ["reply-to:<id>"]` so the pair
  is recoverable later.

The Overview tab's **Delegation board** section shows every recent
delegate/reply pair with status (`awaiting reply`, `replied (+Δt)`,
`untracked` when no task id).

## Reviewer pattern

Set a pane's role to include `reviewer`. Its system-prompt addendum teaches
it the review loop:

- It subscribes to `kind: "diff"` records on the scratchpad.
- For each new diff, it publishes `kind: "review"` with
  `tags: ["approves:<pane_id>"]` or `["rejects:<pane_id>"]` plus a critique
  in the body.

A typical 3-pane setup: **manager** breaks down work + delegates →
**worker** does the work in an isolated worktree, publishes `kind: "diff"`
when ready → **reviewer** approves/rejects on the scratchpad → manager
decides what to do next.

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

Pinned at the front of every project. Four sections:

1. **Pane grid** — one card per pane: status pill, mode icon, role chip,
   provider, worktree branch + diff stats, last-activity timestamp, and a
   60-bucket last-hour activity sparkline (flat line = wedged, regular bars
   = healthy). Click a card to jump to that pane; click the inline Diff /
   Role buttons to open the modals without switching.
2. **Team scratchpad ticker** — last 20 records with filter chips per kind.
3. **Delegation board** — `delegate-to`/`reply-to` pairs with status.
4. **Resource use** — UsageLimitsDisplay per provider, so you can see how
   close any provider is to its quota cap.

## Concrete starter setup

A three-pane scaffold to copy:

- **Pane 2 ("main")** — interactive, no role, no worktree. Your normal pane.
- **Pane 716 ("manager")** — interactive, role: `team manager`, goal:
  "Break down the user's task into small leaf jobs and delegate each to the
  right worker." Plan review: `risky only`.
- **Pane 578 ("worker")** — interactive, role: `frontend engineer`,
  isolated worktree. Plan review: `never`.
- **Pane 849 ("reviewer")** — interactive, role: `code reviewer`, no
  worktree (just reads). Plan review: `never`.

Send your task to the manager pane. The manager will publish
`delegate-to:578` records; the worker picks them up automatically; when the
worker finishes a chunk it publishes `kind: "diff"` and the reviewer
weighs in.

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
