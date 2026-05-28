# Tech-Lead-Driven Workflow (TODO doc + approval gate + PR review)

Status: **draft** — written to capture the workflow the user described in
chat so we can iterate on the design before / during implementation.

Complements `docs/team-mode.md` (which describes the lower-level
scratchpad / delegation primitives). This doc is the higher-level
*protocol* layered on top of those primitives.

## Motivation

Today's flow:
- Manager and Tech Lead pick up user requests ad-hoc.
- Tech Lead delegates via `.apas-team.jsonl` records.
- Workers commit directly to their worktrees; the user has no single
  "queue of things the team intends to do" to look at.
- Code review is opt-in: a reviewer pane only triggers if Tech Lead
  remembers to ask for one.
- Nothing forces a PR boundary — the team's work can land on main with
  no explicit user gate.

What we want instead:
- One **TODO document** the team maintains as the single source of
  truth for "what we're doing."
- A **user-approval gate** before the team spends tokens on
  Tech-Lead-originated work.
- A **per-worker section** so each worker has a clear queue.
- A **mandatory code-review loop** before anything turns into a PR.
- Each global TODO item maps 1:1 to a **PR the user can review**.

## Roles

| Role | Pane | What's new in this workflow |
|---|---|---|
| **User** | (browser) | Approves Tech-Lead-proposed TODOs; reviews PRs at the end |
| **Manager** | user-facing | Surfaces TODO proposals to the user; relays approval back to Tech Lead |
| **Tech Lead** | deadloop | Owns the TODO doc. Proposes / accepts items, expands them, dispatches per-worker subtasks, gates on Reviewer's verdict before opening PRs |
| **Worker** | deadloop, isolated worktree | Reads its own section; ships work + publishes `kind: "diff"` records when done |
| **Reviewer** | deadloop | A regular worker pane with role `reviewer`. Tech Lead delegates to it via the standard `.apas-team.jsonl` channel when workers report done; Reviewer iterates with workers (also via standard delegations) until approved. Set up via Overview's **+Worker** modal. |

## The TODO document

Path: **`<project>/team-todo.md`** (Markdown, human-editable, machine-parseable).

Structure: one top-level "Global TODOs" section followed by per-worker
sections. Items are flat task entries with structured metadata blocks
so both humans and the Tech Lead can read/edit reliably.

```markdown
# Team TODO

## Global TODOs

### [TODO-001] Switch the auth middleware to JWT
status: approved
origin: user
pr: (not yet)

The current session-cookie middleware is being deprecated. Replace
with the JWT validator from the shared auth crate.

### [TODO-003] Add streaming-response to /v1/chat (multi-worker example)
status: pr_open
origin: user
pr: 578 https://github.com/foo/bar/pull/42
pr: 612 https://github.com/foo/bar/pull/43

Backend (pane 578) wires SSE through the route; frontend (pane 612)
updates the chat client to consume SSE. Two PRs for one TODO — the
user reviews them on GitHub side-by-side.

### [TODO-002] Add streaming-response support to /v1/chat
status: proposed
origin: tech-lead
pr: (not yet)

While auditing the API I noticed callers ask for streaming via the
`stream=true` query param but we always send the full response in one
shot. I'd like to wire SSE through the route — estimated 1-2 days, one
backend worker.

## pane:578 — backend-engineer

### [TODO-001 · backend-1] Replace middleware in src/auth/middleware.rs
status: in_progress
parent: TODO-001

- Remove `SessionCookieMiddleware`
- Wire `JwtValidator::from_env()` at the same place
- Update tests in src/auth/tests/middleware.rs

### [TODO-001 · backend-2] Update the integration test harness
status: pending
parent: TODO-001

The test fixtures bake session cookies in; replace with a JWT minter.

## pane:612 — frontend-engineer

(no items yet)
```

### Field reference

**Global TODO**:
- `status`: `proposed` | `approved` | `in_progress` | `under_review` | `pr_open` | `done` | `rejected`
- `origin`: `user` | `tech-lead`
- `pr`: zero or more lines, each `pr: <pane_id> <github_pr_url>`. One per contributing worker pane. `pr: (not yet)` means none yet (functionally equivalent to omitting the line). The Global flips to `pr_open` when *every* contributing worker has a PR line, and to `done` when every URL's GitHub state is `MERGED`.

**Worker subtask**:
- `status`: `pending` | `in_progress` | `done` | `reviewing` | `revising` | `approved`
- `parent`: parent global TODO id

Both kinds use bracket IDs (`[TODO-NNN]`, `[TODO-NNN · slug]`) so links
work in plain text and in any future card UI.

## Lifecycle

### Global TODO state machine

```
                ┌──────────────┐
   tech-lead → │  proposed    │ ←──── user rejects ──┐
                └──────┬───────┘                       │
                       │ user approves                 │
                       ▼                               │
   user ───────→ ┌──────────────┐ ────── tech-lead drops ─┐
                │   approved   │                            │
                └──────┬───────┘                            │
                       │ tech-lead expands                  │
                       ▼                                    │
                ┌──────────────┐                            │
                │ in_progress  │                            │
                └──────┬───────┘                            │
                       │ all subtasks done                  │
                       ▼                                    │
                ┌──────────────┐ ←── reviewer rejects ──┐  │
                │ under_review │                          │ │
                └──────┬───────┘                          │ │
                       │ reviewer approves                │ │
                       ▼                                  │ │
                ┌──────────────┐                          │ │
                │   pr_open    │                          │ │
                └──────┬───────┘                          │ │
                       │ user merges                      │ │
                       ▼                                  │ │
                ┌──────────────┐                          │ │
                │    done      │                          │ │
                └──────────────┘                          │ │
                                                          │ │
                ┌──────────────┐                          │ │
                │   rejected   │ ←────────────────────────┘ │
                └──────────────┘                            │
                                  (or back to in_progress) ─┘
```

### Worker subtask state machine

```
pending → in_progress → done → reviewing ⇄ revising → approved
```

When a worker publishes its diff, the subtask flips `done` → `reviewing`.
Reviewer pushes feedback → `revising`. Worker fixes → `reviewing` again.
Loop until `approved`. Once every subtask under a global TODO is
`approved`, the global TODO flips `under_review` → `pr_open`.

## Editing the doc — agents do it directly

`team-todo.md` is the source of truth. Everyone with file access reads
and edits it directly — the Tech Lead via Read/Edit, the Manager via
the same tools, the user in their editor. The parser is forgiving;
malformed entries are skipped rather than crashing the parse.

There is no `apas todo` CLI surface. Operations that need more than
text manipulation — PR opening, merge-status checking — happen via the
agent's existing Bash tool. The Tech Lead's deadloop prompt spells out
the `git push` / `gh pr create --fill` / `gh pr view --json state`
recipe; the resulting URL / state goes back into `team-todo.md` via
the Edit tool.

There is one in-process write path the CLI provides: when the user
clicks **Approve** / **Reject** in the Web Overview, the web sends
`WebToServer::TodoApproval` → server forwards as
`ServerToCli::TodoApproval` → CLI's handler calls
`team_todo::set_global_status` + atomic save in-process. This is the
only mutation that doesn't originate from an agent; everything else
is a file edit by Tech Lead or Manager.

The `team_todo` Rust library (parser, serializer, atomic file ops, the
state-machine setters) remains in `crates/client-cli/src/team_todo.rs`
because the in-process approval handler and the `WebToServer` wire
format use it. Agents never call it directly.

## How the pieces fit

### Tech Lead loop

Each iteration the Tech Lead:

1. Reads `team-todo.md` + `project_goal.md` + recent `.apas-team.jsonl`.
2. Checks Global TODOs:
   - For `proposed` items: do nothing (waiting for user). Escalate to
     Manager if the queue is too cold (so user knows there's something to
     review).
   - For `approved` items with no subtasks: **expand** — break into 1-N
     worker subtasks, write them into the appropriate worker sections,
     mark global as `in_progress`.
   - For `in_progress` items: poll subtasks. When all `approved`, flip
     to `under_review` and post a `delegate-to:<reviewer_pane_id>` record
     asking the project's Reviewer pane to take it.
3. For each Worker section: dispatch the next `pending` subtask to that
   worker via `.apas-team.jsonl` (existing `delegate-to:<pane_id>`
   protocol). Mark the subtask `in_progress`.
4. Watch for Reviewer verdicts (`kind: "review"`). On approve → flip
   subtask to `approved`. The Reviewer handles `reject` directly with
   the worker via a normal `delegate-to:<worker>` record; the Tech Lead
   just marks the subtask `revising` and waits.
5. On a global TODO hitting `under_review` AND all subtasks `approved` →
   `apas todo open-pr TODO-NNN` (runs `gh pr create` and updates the doc).
6. Periodically (~every 10 iterations): `apas todo refresh-pr-status`
   to flip merged PRs from `pr_open` to `done`.

If the Tech Lead would suggest something to add to the global queue,
it appends it as `status: proposed, origin: tech-lead` and posts a
`kind: "todo-proposal"` record so the Manager surfaces it.

### Manager loop

The Manager is reactive — it acts on user messages typed in its chat.
Most user requests fall into one of these patterns:

- **"Do X" / new work.** Add a Global TODO with
  `status: approved, origin: user` directly. The Tech Lead picks it up
  next iteration. (Don't relay through the Tech Lead — adding the TODO
  is one step instead of two and shows up in the Overview immediately.)
- **"What's happening?"** Read `team-todo.md` + recent scratchpad +
  `project_goal.md` and summarize.
- **"Approve / reject TODO-NNN"** typed in chat. Flip the TODO's
  `status:` line in `team-todo.md`. (The web Overview Approve / Reject
  buttons hit the same path; the Manager doesn't need to do anything
  for that case.)
- **Strategic / vision change.** Update `project_goal.md`. The Tech
  Lead reads it each iteration.
- **Quick ad-hoc question for the Tech Lead.** Delegate via
  `.apas-team.jsonl` with `delegate-to:<tech_lead_pane_id>`.

Proactively: if a `status: proposed, origin: tech-lead` TODO has been
sitting more than ~30 min without action, surface it in chat
("Tech Lead proposed TODO-002 (<title>). Approve?").

### Worker loop

Same as today (`/loop` deadloop, reads delegations from
`.apas-team.jsonl`). New: each delegation now references a
`task:<TODO-NNN · slug>` tag so it can be paired back to its subtask
entry in the TODO doc.

### Reviewer loop

- Reviewer is a regular worker pane (role contains `reviewer`).
- Tech Lead delegates to it on `.apas-team.jsonl` with
  `tags: ["delegate-to:<reviewer_pane_id>", "task:TODO-NNN"]` and a
  body that names the TODO + worker pane ids whose diffs to review.
- Reviewer reads each worker's `kind: "diff"` record, evaluates,
  posts `kind: "review"` with `approves:<pane_id>` or
  `rejects:<pane_id>` plus a short critique.
- For a reject, the Reviewer dispatches the fix directly back to the
  worker via a normal `delegate-to:<worker_pane_id>` record. Worker
  iterates → new diff → next review. Loop is self-driving; Tech Lead
  just watches for the final approval.
- No reap. The Reviewer pane stays put across TODOs (it's a long-lived
  member of the team, just like any other worker).

## Worker → section mapping

A worker section is identified by `pane:<pane_id>` (stable, machine-
resolvable from `.apas`). The header includes the pane's role as a
human-readable hint, e.g. `## pane:578 — backend-engineer`. New
workers added later in the project's life get a fresh empty section
appended on first delegation.

## Locked-in decisions

Q1. **Format**: Markdown only. Tech Lead edits it via line-rewrites;
the user can hand-edit too. No alternative serialization.

Q2. **Approval surface**: Both — Approve/Reject buttons in the web
Overview tab AND chat-parsed approvals in the Manager pane. Both paths
route through the same `WebToServer::TodoApproval` / Manager-edit code
that flips the TODO's `status` field.

Q3. **Reviewer lifecycle**: The Reviewer is a regular worker pane
(role `reviewer`) the user sets up once via Overview's **+Worker**
modal. Tech Lead delegates to it like any other worker; Reviewer
iterates with workers via the same `.apas-team.jsonl` delegate-to
protocol. No special spawn machinery; no special reap. If no reviewer
pane exists when a TODO hits `under_review`, the Tech Lead escalates
to the Manager so the human sets one up. _(Earlier draft proposed
auto-spawning a per-TODO Reviewer; superseded by the uniform-worker
model — fewer moving parts.)_

Q4. **PR opening**: Tech Lead auto-runs `gh pr create` the moment all
subtasks are `approved`. Writes the URL into the `pr:` field and flips
`under_review` → `pr_open`.

## Implementation phases

Each phase is independently shippable.

### Phase 1 — TODO doc + Tech Lead expand/dispatch loop

- Define the Markdown schema (`team-todo.md`).
- Parser + serializer in `crates/client-cli/src/team_todo.rs` (Rust;
  shared types for parsing/writing entries).
- Tech Lead system prompt: read the doc each iteration, expand the
  oldest `approved` item, dispatch first `pending` subtask per worker.
- Tests: parser round-trips, expand-and-dispatch sequence with stub
  panes.

### Phase 2 — Approval gate

- Tech Lead's `propose` path: append a `proposed/tech-lead` entry +
  publish `kind: "todo-proposal"` on the scratchpad.
- Manager's system prompt: surface unactioned proposals; parse user
  decisions; edit the doc.
- Optional web UI: render proposals in Overview; Approve/Reject
  buttons that re-use the same edit path via a new
  `WebToServer::TodoApproval` message.

### Phase 3 — Code review loop

- Reviewer is a regular worker pane with `role: "reviewer"`. Tech Lead
  delegates to it on `.apas-team.jsonl` when ready_for_review; no
  special spawn machinery.
- Reviewer's system-prompt addendum (already in
  `crates/client-cli/src/role.rs` as `REVIEWER_NOTE`) teaches the
  receive-delegation / review-diffs / delegate-fixes-back loop.
- Tech Lead's `under_review` transition is just
  `apas todo set-status TODO-NNN under_review` plus the delegation
  record. The reviewer iterates with workers until done; Tech Lead
  watches for the final `approves:<worker>` records.

### Phase 4 — PR handoff

- `gh pr create` from the Tech Lead, writing the URL into the `pr:`
  field. Wire the existing `CreatePr` plumbing (`ServerToCli::CreatePr`)
  to read this field for the PR title/body.
- Once the user merges and the PR is closed, a daily sweep in the Tech
  Lead loop flips `pr_open` → `done`.

### Phase 5 — Migration / Polish

- One-time migration: on Tech Lead boot, if `team-todo.md` is missing
  but `.apas-team.jsonl` shows recent delegations, seed an empty doc
  with a "## pane:X — <role>" stub for each known worker.
- Web Overview: a TODO panel that renders the doc as a checklist with
  status badges. Pure presentation — Tech Lead remains the only writer.

## Non-goals

- Re-architecting `.apas-team.jsonl`. It's still the substrate for
  delegations / replies / reviews. The TODO doc layers a structured
  plan view on top.
- Replacing the Manager. The Manager still owns conversation with the
  user; this protocol just gives it a structured queue to surface.
- Integration-branch merging of worker branches. A multi-worker
  Global TODO ships as **N PRs** (one per worker), not one merged PR.
  Workers each have their own isolated worktree on their own branch;
  combining them into a single integration branch would mean Tech
  Lead doing `git merge` dances and resolving conflicts on the user's
  behalf, which is more risk than benefit. The user reviews multiple
  related PRs in GitHub.
