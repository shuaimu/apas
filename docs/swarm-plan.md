# APAS Swarm/Team Evolution Plan

## Goal

Evolve APAS from "many parallel claude/codex panes that don't know about each
other" into a coordinated autonomous-development team with first-class human
review checkpoints.

## What we already have (do not rebuild)

- Multiple concurrent panes per project, per-pane provider/model/effort
  (`crates/client-cli/src/mode/dual_pane.rs`, `crates/shared/src/messages.rs`).
- Autonomous deadloop (/loop) with per-pane prompt and min-interval persisted
  in `.apas`.
- `AskUserQuestion` human-gate (web card → `apply_flag_settings`-style
  control_request path) — `packages/web/src/components/tools/AskUserQuestionCard.tsx`,
  `crates/client-cli/src/mode/dual_pane.rs:try_handle_control_request`.
- Pane pause/resume/interrupt, all wired through `ServerToCli`.
- Live multi-session push: every session the web client has ever attached to
  in this load streams in parallel; background tabs update via `sessionCache`
  (in-memory + IndexedDB) plus a sidebar "new activity" dot.
- Per-pane label, optional model override, optional effort override; live
  effort change via `apply_flag_settings`.

## What's missing (gap analysis from the research)

1. **Git isolation**: panes share one working tree → silent races.
2. **Inter-pane state**: no shared scratchpad, message pool, or inbox.
3. **Roles**: `Provider` is the closest thing to a role; no goal/backstory.
4. **Plan-before-execute checkpoint** (Devin / Copilot Workspace).
5. **PR-style diff review surface** inside APAS (Cursor 3 pattern).
6. **Orchestrator/manager pane** (claude-swarm / CrewAI hierarchical).
7. **Tool-level approval policy** (AutoGen `human_input_mode`).
8. **Reviewer / judge pane** (kyegomez/swarms v10).
9. **Distilled "what's happening now"** per pane (OpenHands timeline / Cursor
   status pill).

## Research references

- Survey notes & links live in this doc's commit message and in the chat
  history that produced it. Key transplants chosen:
  - **claude-swarm** (parruda) — declarative topology, worktree-per-agent,
    MCP-as-tool delegation. BSL-licensed, optional to vendor.
  - **CrewAI** — role / goal / backstory triple as a friendlier system-prompt UI.
  - **Devin + Copilot Workspace** — editable plan as a first-class object
    before code runs.
  - **Cursor background agents** — branch-per-agent + auto-PR + in-IDE diff
    review.
  - **MetaGPT** — project-scoped shared message pool with publish/subscribe.
  - **AutoGen** — `human_input_mode` per agent.
  - **kyegomez/swarms v10** — judge / auto-reviewer with structured critique.
  - **OpenHands** — visible Think stream separate from tool calls.

## Phases (ship in this order)

### Phase 1 — make parallel panes safe (foundation)

**Why first**: every later phase assumes panes can't trash each other's work.

1.1  **Worktree-per-pane**.
- [x] **1.1a — data model** (commit pending): `worktree_path:
  Option<String>` added to `PaneConfig` (persisted in `.apas`) and
  mirrored on the in-memory `PaneMeta`. All 13 PaneMeta construction
  sites and all PaneConfig sites pass `worktree_path` through;
  defaults to `None` (legacy behaviour preserved). Restore-from-.apas
  threads it into the `tabs_to_restore` tuple. No runtime behaviour
  change yet — sets up the field so the next leaf can read it.
- [x] **1.1b — spawn cwd**: threaded `worktree_path: Option<String>`
  through `run_pane_session{,_streaming}` and
  `run_deadloop_session{,_inner,_streaming}`. Each leaf computes
  `effective_dir = worktree_path.as_deref().unwrap_or(working_dir)` and
  uses it for `.current_dir()` plus the session-jsonl tailer
  (`tail_session_jsonl`), background-task watcher
  (`poll_background_tasks`), and the `session_jsonl_exists` first-spawn
  probe. All 7 call sites in `dual_pane.rs` now look up
  `meta.worktree_path` from `pane_metas` alongside the existing per-pane
  slots. Default `None` preserves legacy behaviour. Behaviour change:
  when a worktree is set, claude's cwd, its session jsonl path, and the
  auto-wake tmp dir all use the worktree path — they MUST agree because
  claude keys its jsonl by encoded-cwd.
- [x] **1.1c — opt-in CLI subcommand**: shipped `apas worktree {add,remove,list}`.
  `add <pane-id> [branch] [--path <p>]` runs `git -C <project> worktree add
  <path> -b <branch>`, canonicalizes the resulting path, and writes it into
  the matching `PaneConfig.worktree_path` in `.apas`. Defaults: branch
  `apas-pane-<id>`, path `<project>/.apas-worktrees/pane-<id>`. Deviation
  from plan: did NOT implement auto-restart of the running pane — instead
  the command prints a "close + re-add the tab, or reboot" hint. Live
  restart is a possible later leaf if the manual step proves annoying.
  Also exposed `remove <pane-id>` (clears the assignment but leaves the
  git worktree intact) and `list` (shows current assignments).
- [x] **1.1d — cleanup on pane removal**: shipped the three-option flow.
  Closing a pane that owns a worktree opens a modal in the web UI
  (`WorktreeCleanupModal`) with: "leave as branch (safe)", "merge into
  current branch then remove", and "discard everything (with second
  confirm)". The chosen action rides on `WebToServer::RemovePane` as a
  new `cleanup_action` field, forwarded through `ServerToCli::RemovePane`
  and into the existing `TuiEvent::CloseTab`. The CLI's CloseTab handler
  reads `meta.worktree_path` (Phase 1.1a) before tearing down, then —
  after killing the agent and removing the pane from in-memory state —
  calls `worktree::cleanup_on_close` which runs the matching git
  plumbing. `leave_as_branch` uses `git worktree remove` without
  `--force` so uncommitted changes are preserved with a hand-off
  message; `discard` uses `--force` and deletes the branch; merge does
  `git merge --no-ff` first and aborts on conflict. Covered by 4 new
  unit tests (clean leave, dirty leave, discard, merge bringing
  commits into HEAD).
- [x] **1.1e — web UI toggle on Add Pane**: the "+" dropdown in TabBar
  now has an "Isolated git worktree" checkbox at the top. When checked,
  the next provider click sends `isolated_worktree: true` on the
  `WebToServer::AddPane`; the server forwards it as a new field on
  `ServerToCli::AddPane`; the CLI's AddPane handler (in
  `run_server_connection`) calls `worktree::create_for_pane` — a new
  shared helper extracted from `worktree::add` — and threads the
  resulting absolute path into `TuiEvent::AddTabWithConfig.worktree_path`,
  which now also persists into `PaneMeta.worktree_path`. The checkbox
  resets to off after each tab creation so a second tab doesn't silently
  inherit the choice. Failure to create the worktree (not a git repo,
  path collision, etc.) is surfaced as a system message in the new pane
  and the spawn falls back to the shared cwd — so the user always gets
  a working pane.

Phase 1.1 is complete.

1.2  **Diff-review surface** in the web UI. Sub-split per the
"break chunky leaves into smaller ones" rule:
- [x] **1.2a — on-demand diff**: shipped the smallest viable diff
  surface. New wire types `WebToServer::RequestPaneDiff`,
  `ServerToCli::RequestPaneDiff`, `CliToServer::PaneDiff`,
  `ServerToWeb::PaneDiff` (carries `branch`, `base`, `diff`, `error`).
  CLI helper `worktree::compute_pane_diff(project_dir, worktree_path)`
  runs `git diff HEAD...<branch>` (three-dot, so it's "changes on the
  worktree branch since divergence" — the intuitive answer for "what
  did this pane change?"). Web UI: green "Diff" button appears in the
  pane header next to Interrupt when the active pane has a
  worktree_path. Click opens a modal with the patch text + a Refresh
  button. No polling, no syntax highlighting yet — those are 1.2b/c.
  Two new unit tests (happy-path diff with content, error path when
  no worktree).
- [x] **1.2b — auto-refresh**: a single shared poller thread (spawned
  from `dual_pane::run_inner`, 3-second tick) scans `pane_metas` for
  panes with `worktree_path`, runs `git rev-parse HEAD` in each, and
  re-emits `PaneDiff` only when the SHA differs from the previous
  tick. New panes are picked up automatically on the next scan; closed
  panes have their `last_seen` entry reaped so the state map stays
  bounded. Helper `worktree::poll_changed_diffs(project, state, panes)`
  is exposed so the loop body is testable in isolation — covered by a
  new unit test that walks the (baseline → no change → commit → reap)
  cycle.
- [x] **1.2c — syntax-highlighted view**: the modal now splits the
  unified diff on `diff --git a/<path> b/<path>` headers and renders
  each file as a collapsible section. Header shows the path + per-file
  `+N / -N` line counts (handy when collapsed). Each section uses the
  existing `CodeBlock` component with `language="diff"` so the patch
  gets Prism syntax highlighting + a copy button for free.
- [x] **1.2d — action buttons in the diff view**: PaneDiffModal now
  has "Merge & close" (green) and "Discard" (red) buttons in the
  header. Each shows a confirm dialog, then calls the existing
  `removePane(paneId, action)` from 1.1d — so the entire backend git
  plumbing is shared with the on-close cleanup prompt. "Open in
  $EDITOR" was dropped from scope: the worktree lives on the CLI host,
  which is often a different machine from the user's browser, making
  remote-editor a brittle UX. The path is already shown in the Diff
  button's tooltip so the user can `$EDITOR` it manually when local.

Phase 1.2 is complete.

### Phase 2 — coordination primitives

2.1  **Role / goal / backstory** on each pane. Sub-split:
- [x] **2.1a — data model**: added `role`, `goal`, `backstory:
  Option<String>` to `shared::PaneConfig` (persisted, default-None via
  serde) and mirrored them on the in-memory `PaneMeta`. Threaded
  through every PaneConfig/PaneMeta construction site (14 PaneMeta
  sites, 5 PaneConfig sites across dual_pane.rs, project.rs, ws_web.rs,
  messages.rs). `tabs_to_restore` tuple grew from 11 to 14 elements so
  restore-from-.apas round-trips the three new fields. No runtime
  behaviour change yet — sets up the fields for 2.1b's
  --append-system-prompt wiring.
- [x] **2.1b — claude launch wiring**: new `crate::role` module exposes
  `compose_system_prompt(role, goal, backstory) -> Option<String>` that
  joins set fields into a markdown-styled block (`# Role\n…\n\n# Goal\n…`
  etc.) and returns None when all three are empty. Spawn functions
  (`run_pane_session{,_streaming}`,
  `run_deadloop_session{,_inner,_streaming}`) now take an extra
  `system_prompt: Option<String>` argument; the streaming claude spawn
  pushes `--append-system-prompt <prompt>` when Some, the legacy
  non-claude path explicitly suppresses the param with a comment
  pointing at this leaf. All 7 call sites compose from
  `meta.role/goal/backstory` via the new helper and pass through. 4 new
  unit tests in `role.rs` cover the empty / single / all-three / partial
  cases.
- [x] **2.1c — web UI Role drawer**: purple "Role" button in the pane
  header (next to Diff) opens a modal with three inputs (role + goal:
  text inputs; backstory: 6-row textarea). Save sends new
  `WebToServer::UpdatePaneRole { pane_id, role, goal, backstory }` →
  `ServerToCli::UpdatePaneRole` → CLI updates `PaneMeta.role/goal/
  backstory`, persists via `save_pane_configs`, and emits a system
  message into the pane reminding the user the change takes effect on
  next spawn. The modal pre-populates from the current values in
  `PaneConfig` (which is sync'd from PaneMeta via PaneList).

Phase 2.1 is complete.

2.2  **Project-scoped team scratchpad** (`.apas/team.jsonl`). Sub-split:
- [x] **2.2a — data model + CLI helpers**: new `crate::scratchpad`
  module with `TeamRecord { ts, pane_id, tags, kind, body }`, an
  `append()` helper, `read_all()`, and `read_filtered_by_tags()`.
  Deviation from plan: `.apas` is a *file* (project metadata), so the
  scratchpad lives at `<project>/.apas-team.jsonl` (sibling) rather
  than `<project>/.apas/team.jsonl` (would conflict with the file).
  Documented in the path-resolution helper so a future leaf that
  migrates `.apas` to a directory has one place to change. Four unit
  tests cover round-trip, tag filter, missing-file → empty, and
  malformed-line skipping.
- [x] **2.2b — wire + web UI Team timeline**: new shared
  `TeamScratchpadRecord` type +
  `CliToServer::TeamRecord` / `ServerToWeb::TeamRecord` envelopes.
  CLI background watcher polls `.apas-team.jsonl` every 2s by file
  size; on growth (or on CLI startup) it emits each new record
  upstream. Web store accumulates them in `teamRecords` and an
  amber "Team" button in the header opens a modal with the timeline
  (kind/ts/pane/tags chips + body in monospace). Deviation from the
  plan: shipped as a modal rather than a full "Team tab" alongside
  the per-pane chats — cheaper, can be promoted later if it
  warrants the layout work.
- [x] **2.2c — system-prompt mention**: `role::compose_system_prompt`
  now appends a static "# Team scratchpad" section after the
  role/goal/backstory blocks, telling the agent about
  `.apas-team.jsonl`, showing the JSON line shape (with the known
  `kind` values), and suggesting `tail -f` / Write to consume and
  publish. Rides along only when at least one of role/goal/backstory
  is set — without those, no system prompt is emitted at all, so the
  note stays quiet for panes that haven't opted in. Existing role
  tests updated to assert structural properties (section ordering,
  scratchpad note presence) rather than exact strings.

Phase 2.2 is complete.

### Phase 3 — orchestration

3.1  **Manager pane** (orchestrator). Sub-split (see
`docs/dev/3.1-delegation-via-scratchpad.md` for the
file-based-vs-MCP decision):
- [x] **3.1a — scratchpad-based delegation routing**: extended the
  Phase 2.2b scratchpad watcher: for each NEW record (never history)
  whose tags contain `delegate-to:<pane_id>`, the watcher sends the
  record's `body` into the target pane's input channel via the same
  channel that handles user input (with `from_tui=false`). New
  testable helper `scratchpad::delegate_target_pane(record)` parses
  the first `delegate-to:<u32>` tag and returns the id; unit-tested
  for happy path, no-match, and malformed-suffix cases. Lookup fails
  gracefully (log + skip) when the target pane has no input channel
  (e.g. delegating to a pane that was closed). See
  `docs/dev/3.1-delegation-via-scratchpad.md` for the
  file-based-vs-MCP rationale.
- [x] **3.1b — manager system-prompt addendum**: when a pane's role
  contains "manager" (case-insensitive), `compose_system_prompt`
  appends a "# Manager protocol" section after the scratchpad note,
  teaching the agent the `delegate-to:<pane_id>` and
  `reply-to:<task_id>` tag conventions and pointing it at the project
  `.apas` file to discover available workers. Deviation: did NOT
  enumerate sibling panes inline — would require plumbing siblings
  into compose_system_prompt and would rot as tabs come and go. Three
  new role tests: addendum presence on manager-role, case-insensitive
  substring detection, and that non-manager roles don't get it.
- [ ] **3.1c (optional) — MCP delegate tool**: replace the JSONL
  convention with a proper MCP server when the simpler approach
  starts feeling limiting. Deferred until needed.

3.2  **Editable plan checkpoint** (per-pane policy). Sub-split:
- [x] **3.2a — data model**: new `shared::PlanReviewMode` enum
  (`Always` / `RiskyOnly` / `Never`, default `Never`) added to
  `PaneConfig` (persisted, default via `#[serde(default)]`) and
  mirrored on `PaneMeta`. Threaded through every PaneConfig (15 sites
  total: messages/project/ws_web + 13 dual_pane bulk-patched) and
  through the `tabs_to_restore` tuple (now 15 elements) so the value
  survives CLI restarts. `build_pane_list` reads it from PaneMeta;
  the synthetic PaneConfig path defaults to `Never`. No runtime
  behaviour change — sets up 3.2b's gating logic.
- [ ] **3.2b — gating logic**: when `always` (or `risky-only` and the
  pending tool_use is Write/Edit/Bash), the streaming worker holds
  the tool-use until the user approves via a new card. Effectively
  the AskUserQuestion plumbing (Phase 1's permission_prompt path)
  with a different card kind.
- [ ] **3.2c — web UI mode picker**: extend the Role drawer (or add
  a separate dropdown) so the user can change plan_review_mode per
  pane.

3.3  **Judge / auto-reviewer pane** (optional).
- A pane with `role: "reviewer"` subscribes to `team.jsonl` for `kind: "diff"`
  events from other panes, reads the diff, and publishes back a review
  (`kind: "review"`, `tags: ["approves:<task_id>"]` or `["rejects:..."]`).
- The plan-review card (3.2) renders the judge's verdict inline so the human
  can rubber-stamp common cases.

### Phase 4 — UX polish

4.1  **Distilled "what's happening" pill** per pane.
- Derive from the most recent `tool_use` block: "Running tests…",
  "Editing `src/foo.rs`…", "Waiting on review".
- Reuses today's `PaneStatus` plumbing; just a new text-generation rule.

4.2  **Action/observation timeline** (sidebar, collapsible).
- Per-turn summary: "tool: Bash; args: …; → 14 failures".
- Toggle between this and the raw chat. Same data, different view.

4.3  **Tool-level approval policy** per pane.
- `PaneConfig.tool_approval_mode: "never" | "risky-only" | "always"`.
- Default `risky-only` for new manager panes, `never` for low-stakes workers.
- Reuses the existing `--permission-prompt-tool stdio` plumbing.

## Tradeoffs to revisit at each phase

- Worktree-per-pane complicates the user's mental model — provide an explicit
  "merge to main" UX in the diff view.
- Manager pane adds an LLM round-trip per delegated task; not free.
- Plan-review checkpoint adds clicks; make per-pane dismissible.
- Vendoring claude-swarm could save weeks at the cost of a heavier
  dependency footprint.

## Operating principle for the self-paced loop

- One leaf task per iteration. Ship → test → commit → push.
- Open PR-style branch for each phase (`apas-swarm/phase-1.1-worktree-per-pane`,
  etc.) — even if we merge fast, the branch history is the review trail.
- Update this doc after each leaf: cross off what's done, note any deviations
  from the original plan inline.
- When uncertain about UX, use `AskUserQuestion`. When uncertain about
  architecture, write a one-paragraph note in `docs/dev/` and proceed with
  the simplest version.
