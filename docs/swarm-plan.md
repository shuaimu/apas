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
- [ ] **1.1d — cleanup on pane removal** (next): prompt for "discard / merge
  to current branch / leave as branch for manual review."
- [ ] **1.1e — web UI toggle on Add Pane**: checkbox "isolated
  worktree", calls 1.1c under the hood.

1.2  **Diff-review surface** in the web UI.
- Server-side: per pane, watch `git diff <branch>..HEAD` on commit hook (or
  poll the working tree). Emit `ServerToWeb::PaneDiff` with stats + maybe
  a truncated patch.
- Web: a "Diff" toggle in the pane header that expands to a syntax-highlighted
  diff view (reuse `CodeBlock`).
- Buttons: "Merge to current branch" (delegates to the manager pane or a
  user-confirmed `git merge`), "Discard", "Open in $EDITOR".

### Phase 2 — coordination primitives

2.1  **Role / goal / backstory** on each pane.
- New fields in `PaneConfig`:
  ```rust
  role: Option<String>,         // e.g. "backend implementer"
  goal: Option<String>,         // e.g. "make auth tests green"
  backstory: Option<String>,    // appended to system prompt
  ```
- Wire via `--append-system-prompt` on claude launch. (For codex/others,
  best-effort via env or system message.)
- Replaces the current "raw prompt" field for deadloops with three smaller
  inputs (and persists alongside it; old prompt still works).
- Web UI: a "Role" tab in the pane settings drawer with three short textareas.

2.2  **Project-scoped team scratchpad** (`.apas/team.jsonl`).
- Append-only JSONL, one record per published artifact:
  ```json
  {"ts":"...","pane_id":42,"tags":["pr-review","auth-module"],
   "kind":"diff","body":"..."}
  ```
- Each pane's system-prompt-append includes a one-liner reminding it of the
  file + the tags it should `tail -f`.
- Web UI: a global "Team" tab next to the per-pane chats that renders the
  scratchpad as a timeline.

### Phase 3 — orchestration

3.1  **Manager pane** (orchestrator).
- A pane with `role: "manager"` runs an MCP server (via `--mcp-config`) that
  exposes `delegate(target_pane_id, task, expected_artifacts)`. The MCP
  implementation routes the task into the target pane's input queue (we
  already have per-pane input channels in `dual_pane.rs`).
- Workers reply by publishing to `team.jsonl` with `tags: ["reply-to:<task_id>"]`;
  manager polls.
- Optional convention: only manager panes are auto-approved to write to
  `team.jsonl`; workers go through manager.

3.2  **Editable plan checkpoint** (per-pane policy).
- New `PaneConfig.plan_review_mode: "always" | "risky-only" | "never"`.
- When `always`: every new turn from claude is wrapped in an
  `AskUserQuestion`-style card showing the plan text (extracted from the
  first assistant message before any tool_use). User clicks
  Approve / Edit / Reject before tools execute.
- When `risky-only`: only fires for turns containing `Write/Edit/Bash` tools.
- When `never`: today's behaviour.
- Persisted alongside the plan + final diff for review history.

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
