# Tech-Lead-Driven Workflow (TODO doc + approval gate + PR review)

Status: **implemented and evolving** — this is the operating protocol
for APAS team-mode TODO orchestration. The core workflow is in use; new
TODO-backed refinements should update this document when they change the
state machine or role responsibilities.

Complements `docs/team-mode.md` (which describes the lower-level
scratchpad / delegation primitives). This doc is the higher-level
*protocol* layered on top of those primitives.

## Motivation

APAS team mode coordinates autonomous panes through three durable files:

- `project_goal.md` captures the current product direction in the
  Manager's lane.
- `team-todo.md` is the structured queue the Tech Lead owns: proposed
  work, approved Globals, per-worker subtasks, review state, and PR
  links all live here.
- `.apas-team.jsonl` remains the append-only bus for delegations, diffs,
  reviews, decisions, and escalations.

That split gives the user a visible approval surface before
Tech-Lead-originated work consumes tokens, gives each worker a concrete
subtask queue, requires Reviewer approval before PR handoff, and maps
each Global TODO to one or more worker-opened PRs the user can review in
GitHub.

## Roles

| Role | Pane | What's new in this workflow |
|---|---|---|
| **User** | (browser) | Approves Tech-Lead-proposed TODOs; reviews PRs at the end |
| **Manager** | user-facing | Surfaces TODO proposals to the user and turns direct user requests into approved Global TODOs |
| **Tech Lead** | deadloop | Owns the TODO doc. Proposes items, expands approved work, dispatches per-worker subtasks, delegates review, records worker-opened PRs, and refreshes PR state |
| **Worker** | deadloop, isolated worktree | Reads its own section; ships work, publishes `kind: "diff"` records, and opens its own PR once the Reviewer approves |
| **Reviewer** | deadloop | The default managed Reviewer pane/slot with role `reviewer`. Tech Lead delegates to it via the standard `.apas-team.jsonl` channel when worker diffs are ready; Reviewer iterates with workers (also via standard delegations) until approved. Users can still add extra reviewer panes manually. |

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
- `status`: `pending` | `in_progress` | `reviewing` | `revising` | `approved` | `done`
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
                       │ worker diffs ready                 │
                       ▼                                    │
                ┌──────────────┐ ←── reviewer rejects ──┐  │
                │ under_review │                          │ │
                └──────┬───────┘                          │ │
                       │ workers open PRs                 │ │
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
pending → in_progress → reviewing ⇄ revising → approved → done
```

When a worker publishes its diff, the subtask flips `in_progress` →
`reviewing`. Reviewer pushes feedback → `revising`. Worker fixes →
`reviewing` again. Loop until `approved`. Once approved, the worker
opens its own PR and publishes a `kind: "decision"` record tagged
`pr-opened`; the Tech Lead records the `pr: <pane_id> <url>` line and
the subtask is complete. Once every contributing worker has a PR line,
the global TODO flips `under_review` → `pr_open`.

## Scratchpad cursors

Tech Lead and Reviewer both iterate on a self-paced /loop cadence
(roughly every 2 min). A busy team can easily generate more than that
many records between iterations, so a fixed-window `tail -n 30` will
miss things. Both agents instead maintain a per-pane cursor file:

- `.apas-tech-lead-cursor` — single-line file holding the `ts` of the
  last scratchpad record the Tech Lead processed/scanned.
- `.apas-reviewer-cursor` — same idea for the Reviewer.

Each iteration the agent reads the cursor, queries records strictly
newer (`jq -c 'select(.ts > "<cursor>")' .apas-team.jsonl`), acts,
then writes back the newest `ts` it processed/scanned after handling any
directed work. Self-authored records and delegations to other panes can
be ignored, but they still count as successfully scanned and should
advance the scratchpad cursor so they are not re-read every loop. First
run (cursor file missing) falls back to `tail -n 50` as catch-up.

Because those cursors compare the record's `ts`, every writer must stamp
scratchpad records at append time. Generate the timestamp immediately
before writing the JSON line (for example with `date -Iseconds`) and do
not reuse an earlier planning timestamp; otherwise a record appended
late with an older `ts` can fall behind another pane's cursor and be
skipped.

Both files are git-ignored. Re-processing on cursor loss is safe
because every action also updates `team-todo.md` — the state machine
deduplicates idempotently.

## Editing the doc — agents do it directly

`team-todo.md` is the source of truth. Everyone with file access reads
and edits it directly — the Tech Lead via Read/Edit, the Manager via
the same tools, the user in their editor. The parser is forgiving;
malformed entries are skipped rather than crashing the parse.

There is no `apas todo` CLI surface. Operations that need more than
text manipulation happen via the agents' existing Bash tools: workers
run `git push` / `gh pr create --fill` after Reviewer approval, and the
Tech Lead runs `gh pr view --json state` when refreshing open PRs. PR
URLs and merge state go back into `team-todo.md` via the Edit tool.

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
   - For `approved` items with no subtasks: apply backlog backpressure
     before expanding. Count managed Developer panes and their `pending`,
     `in_progress`, and `revising` subtasks. Available capacity is the
     number of managed Developers with none of those active subtasks, and
     the queue limit allows one additional `pending` subtask per managed
     Developer across the whole queue. Expand only while new subtasks fit
     within available capacity plus remaining queue slots; write those
     subtasks into the appropriate worker sections and mark that Global as
     `in_progress`. Leave the remaining approved Globals as `approved`
     with no subtasks until worker capacity opens.
   - For `in_progress` items: dispatch pending subtasks. When the
     relevant worker diffs are ready, flip to `under_review` and post a
     `delegate-to:<reviewer_pane_id>` record asking the project's
     managed Reviewer pane to evaluate them. If no managed Reviewer
     exists, escalate to the Manager/human to start or add one.
   - For `pr_open` items: refresh the recorded PR URLs periodically and
     flip to `done` only after every PR is merged.
3. For each Worker section: dispatch the next `pending` subtask to that
   worker via `.apas-team.jsonl` (existing `delegate-to:<pane_id>`
   protocol). Mark the subtask `in_progress`.
4. Watch for Reviewer verdicts (`kind: "review"`). On reject, mark the
   affected subtask `revising`; the Reviewer sends the fix delegation
   directly to the worker. On approve, the worker opens its own PR.
5. Watch for worker `kind: "decision"` records tagged `pr-opened`.
   Record each URL as `pr: <pane_id> <github_pr_url>` on the Global
   TODO. When every contributing worker has a PR line, flip the Global
   TODO to `pr_open`.
6. Periodically (~every 10 iterations, or when a PR is stale): refresh
   each recorded PR with `gh pr view <url> --json state`. Flip
   `pr_open` → `done` only after every PR is merged; escalate if a PR is
   closed without merge.

If the Tech Lead would suggest something to add to the global queue,
it appends it as `status: proposed, origin: tech-lead`. The Manager and
Overview surface proposals by reading `team-todo.md`; no scratchpad
proposal record is required.

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

When a worker finishes the assigned code/doc change, it commits on its
own branch and publishes a `kind: "diff"` record. After the Reviewer
posts `approves:<worker_pane_id>`, the worker runs `git push -u origin
<branch>` and `gh pr create --fill`, then publishes a
`kind: "decision"` record tagged `pr-opened` with the PR URL. The Tech
Lead records that URL on the Global TODO; the worker does not create or
edit the Global TODO's `pr:` lines itself.

### Reviewer loop

- Reviewer is normally the default managed Reviewer pane/slot (role
  contains `reviewer`). Extra manually added reviewer panes can still be
  used if the Tech Lead explicitly targets them.
- Tech Lead delegates to it on `.apas-team.jsonl` with
  `tags: ["delegate-to:<reviewer_pane_id>", "task:TODO-NNN"]` and a
  body that names the TODO + worker pane ids whose diffs to review.
- Reviewer reads each worker's `kind: "diff"` record, evaluates,
  posts `kind: "review"` with `approves:<pane_id>` or
  `rejects:<pane_id>` plus a short critique.
- For a reject, the Reviewer dispatches the fix directly back to the
  worker via a normal `delegate-to:<worker_pane_id>` record. Worker
  iterates → new diff → next review. Loop is self-driving; Tech Lead
  marks the subtask `revising` and watches for the final approval.
- For an approval, the Reviewer is done; the worker opens its own PR and
  the Tech Lead records the resulting `pr-opened` decision.
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

Q3. **Reviewer lifecycle**: The Reviewer is the default managed
Reviewer pane/slot (role `reviewer`) in the team. Tech Lead delegates
to it like any other worker; Reviewer iterates with workers via the
same `.apas-team.jsonl` delegate-to protocol. Users can still add extra
reviewer panes manually. No special per-TODO spawn machinery; no
special reap. If no managed Reviewer pane exists when a TODO hits
`under_review`, the Tech Lead escalates to the Manager/human to start
or add one. _(Earlier draft proposed auto-spawning a per-TODO
Reviewer; superseded by the long-lived team-slot model — fewer moving
parts.)_

Q4. **PR opening**: Workers open PRs themselves after Reviewer approval.
Each worker publishes `kind: "decision"` with `pr-opened`; the Tech Lead
writes the URL into the matching Global TODO's `pr:` field. A
multi-worker Global TODO reaches `pr_open` only after every contributing
worker has a recorded PR.

## Implementation history

The workflow shipped incrementally. These notes are architecture
breadcrumbs, not an active implementation plan:

### TODO doc + Tech Lead expand/dispatch loop

- `team-todo.md` defines the Markdown schema and remains the source of
  truth for Global TODOs and worker subtasks.
- Parser, serializer, atomic save, and state-machine helpers live in
  `crates/client-cli/src/team_todo.rs`.
- The Tech Lead prompt reads the doc each iteration, expands approved
  Globals subject to backlog backpressure, and dispatches pending
  subtasks to available workers.
- Regression coverage protects parser round-trips and representative
  expand/dispatch behavior.

### Approval gate

- Tech-Lead-originated work enters as `status: proposed, origin:
  tech-lead`.
- The Manager surfaces unactioned proposals and can translate direct
  user requests into `status: approved, origin: user` Globals.
- The Web Overview renders proposal controls; Approve/Reject sends
  `WebToServer::TodoApproval`, which the CLI applies through the same
  `team_todo` state-machine path.

### Code review loop

- Reviewer is the default managed team slot with `role: "reviewer"`.
  Tech Lead delegates to it on `.apas-team.jsonl` when a Global TODO
  enters `under_review`; no per-TODO spawn machinery. If no managed
  Reviewer exists, Tech Lead escalates to the Manager/human to start or
  add one.
- Reviewer's system-prompt addendum (already in
  `crates/client-cli/src/role.rs` as `REVIEWER_NOTE`) teaches the
  receive-delegation / review-diffs / delegate-fixes-back loop.
- Tech Lead's `under_review` transition is a `team-todo.md` edit plus
  the delegation record. The reviewer iterates with workers until
  approved; Tech Lead watches for final `approves:<worker>` records and
  then for worker `pr-opened` decisions.
- Reviewer posts `approves:<worker>` / `rejects:<worker>` verdicts.
  Rejects go directly back to the worker as normal delegations; approvals
  let workers open their PRs.

### PR handoff

- Workers run `git push -u origin <branch>` and `gh pr create --fill`
  after Reviewer approval, then publish `kind: "decision"` with
  `pr-opened`.
- Tech Lead records each worker URL as `pr: <pane_id> <url>` and flips
  the Global TODO to `pr_open` once all contributing workers have PRs.
- Tech Lead owns PR state refresh and PR-comment dispatch; workers do
  not idle-poll their own PRs.

### Migration / UI polish

- On boot, the team stack can seed missing worker sections from the
  current pane roster so `team-todo.md` stays navigable.
- The Web Overview renders the TODO document as a checklist with status
  badges, proposal controls, PR links, and subtask lifecycle state.
  Agents still mutate the document directly for orchestration state.

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
