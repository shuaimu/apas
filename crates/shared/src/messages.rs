use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One published artifact in the team scratchpad (`.apas-team.jsonl`),
/// mirroring the CLI's `crate::scratchpad::TeamRecord`. Separate type
/// here so the wire shape is stable across CLI/server/web even if the
/// CLI's internal helper grows extra columns. Phase 2.2b.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamScratchpadRecord {
    pub ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<u32>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub kind: String,
    pub body: String,
}

/// Per-role provider/model pair the user picks in the "Team setup"
/// card before clicking Start team. Empty fields fall back to the
/// CLI's defaults (Claude / unset model).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamRoleSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Phase 3.2a: per-pane policy for the "editable plan checkpoint"
/// feature. The streaming worker reads this at every turn to decide
/// whether to gate the first tool_use behind a user-approval card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanReviewMode {
    /// Hold every turn's first tool_use until the user approves.
    Always,
    /// Hold only when the turn's first tool_use is in a "risky" set
    /// (Write/Edit/Bash). Read-only tools run through.
    RiskyOnly,
    /// Today's behaviour: no gating.
    #[default]
    Never,
}

/// Last server-observable state of a pty-hosted terminal pane.
///
/// `Disconnected` means the CLI transport went away while the terminal was
/// last known to be running; it does not claim the provider process exited.
/// `Unknown` is the rollout-safe default for peers predating lifecycle
/// reconciliation and after a server restart before the CLI reports state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TerminalLifecycle {
    #[default]
    Unknown,
    Running,
    Disconnected,
    Exited,
}

/// What to do with an isolated git worktree (and its branch) when the pane
/// that owns it is closed. Selected by the web UI before sending
/// `WebToServer::RemovePane` so the CLI knows which git commands to run.
/// Phase 1.1d of the swarm plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneCleanupAction {
    /// `git worktree remove --force <path>` + `git branch -D <branch>`.
    /// Permanently deletes work — pick only if the work is throwaway.
    Discard,
    /// `git -C <project> merge --no-ff <branch>` into the current main-worktree
    /// branch, then remove the worktree + delete the branch. Errors out on
    /// merge conflicts (the user resolves manually with normal git tools).
    MergeAndRemove,
    /// `git worktree remove <path>` (no --force — fails if there are
    /// uncommitted changes, in which case we just clear `worktree_path` and
    /// tell the user to clean up by hand). Branch is left alone so the user
    /// can `git checkout` it for manual review.
    LeaveAsBranch,
}

// ============================================================================
// CLI <-> Server Messages
// ============================================================================

/// Messages sent from CLI client to server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CliToServer {
    /// CLI registers with the server using auth token and version
    Register {
        token: String,
        #[serde(default)]
        version: Option<String>,
    },

    /// CLI starts a local session (hybrid mode)
    SessionStart {
        session_id: Uuid,
        /// Stable project identity from `.apas` (`ProjectMetadata.id`). Older
        /// CLIs may omit this; the server falls back to session_id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project_id: Option<Uuid>,
        working_dir: Option<String>,
        hostname: Option<String>,
        /// Canonical `host/owner/repo` derived from the project's `origin` git
        /// remote, used by the web sidebar to group projects that belong to the
        /// same repo. `None` when there is no remote (or no git). Older CLIs
        /// omit it, so it stays optional and back-compatible.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        git_remote: Option<String>,
        /// Raw `origin` remote URL (scheme/user/auth preserved), used as the
        /// clone URL when creating a new instance of this repo on any machine.
        /// `git_remote` is the lossy grouping key; this is the lossless URL.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        git_remote_url: Option<String>,
        #[serde(default)]
        pane_type: Option<PaneType>,
        /// Pane configurations for this session
        #[serde(default, skip_serializing_if = "Option::is_none")]
        panes: Option<Vec<PaneConfig>>,
    },

    /// Claude output to be forwarded to web client
    Output {
        session_id: Uuid,
        data: String,
        #[serde(default)]
        output_type: OutputType,
        #[serde(default)]
        pane_type: Option<PaneType>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
    },

    /// Session has ended
    SessionEnd { session_id: Uuid, reason: String },

    /// Heartbeat to keep connection alive
    Heartbeat,

    /// Structured message from Claude CLI stream-json output
    StreamMessage {
        session_id: Uuid,
        message: ClaudeStreamMessage,
        #[serde(default)]
        pane_type: Option<PaneType>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
    },

    /// User input/prompt from CLI (to be displayed in web UI)
    UserInput {
        session_id: Uuid,
        text: String,
        #[serde(default)]
        pane_type: Option<PaneType>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
    },

    /// Report deadloop pause status to server (legacy - use PanePaused for new code)
    DeadloopStatus { session_id: Uuid, is_paused: bool },

    /// Report pane pause status to server
    PanePaused {
        session_id: Uuid,
        pane_id: u32,
        is_paused: bool,
    },

    /// Report pane status (e.g., "thinking") for status bar display
    PaneStatus {
        session_id: Uuid,
        #[serde(default)]
        pane_type: PaneType,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
        status: Option<String>,
    },

    /// Report current pane configurations to server
    PaneList {
        session_id: Uuid,
        panes: Vec<PaneConfig>,
    },

    /// Report usage limits for a provider
    UsageLimits {
        #[serde(default)]
        provider: Provider,
        limits: UsageLimits,
    },

    /// One team-scratchpad record (Phase 2.2b). CLI pushes when it
    /// detects new lines in `.apas-team.jsonl` (either appended by an
    /// agent via Bash/Write, or by a future MCP-publish helper). Sent
    /// in batches on attach, then individually as new ones land.
    TeamRecord {
        session_id: Uuid,
        record: TeamScratchpadRecord,
    },

    /// Phase 3.2b2: CLI requests user approval for a held tool_use.
    /// Fired when the pane's `plan_review_mode` says "hold this tool"
    /// per `crate::plan_review::should_hold_tool`. The web UI shows
    /// an Approve / Deny card; the user's answer rides back via
    /// `WebToServer::PlanReviewAnswer`. While held, the agent is
    /// effectively paused on this turn.
    PlanReviewRequest {
        session_id: Uuid,
        pane_id: u32,
        /// claude SDK tool_use_id — used to match the answer back to
        /// the parked request.
        tool_use_id: String,
        /// Tool name as reported by claude (e.g. "Write", "Bash").
        tool_name: String,
        /// The tool's input JSON exactly as claude sent it, so the
        /// web UI can render the would-be call for review.
        input: serde_json::Value,
    },

    /// Diff payload for a pane that owns an isolated worktree. Sent in
    /// response to `ServerToCli::RequestPaneDiff`. `diff` is the unified
    /// patch text (UTF-8, may be empty for "no changes"). `error` is set
    /// instead when something blocked the diff (no worktree, not a git
    /// repo, branch not found, etc.) so the web UI can render it inline
    /// instead of silently dropping the response. Phase 1.2a.
    PaneDiff {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// Result of `ServerToCli::CreatePr` — the CLI pushed the pane's
    /// branch to origin and ran `gh pr create --fill`. `url` is set on
    /// success; `error` is set on failure (no remote, `gh` not installed,
    /// auth missing, no commits, etc.).
    PrCreated {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// v3.1 — current contents of `project_goal.md` on the CLI host.
    /// Pushed (a) at CLI boot once the file exists, and (b) whenever the
    /// file's mtime changes (manager wrote to it, user clicked Save and
    /// CLI persisted it, an outside editor touched it). The web mirrors
    /// the latest value per session and hydrates the textbox when not
    /// being actively edited.
    ProjectGoalChanged {
        session_id: Uuid,
        content: String,
    },

    /// Tech-Lead autonomy flags. `auto_approve_todos` lets the Tech
    /// Lead flip Global TODOs from `proposed` → `approved` without a
    /// human click. `auto_merge_prs` lets it `gh pr merge` (or close
    /// with a rejection comment / post a "work more" review) on PRs
    /// in `pr_open` Globals. Both default false. Pushed at CLI boot
    /// and on every flag mutation so the web mirrors the current
    /// state per session.
    ProjectFlagsChanged {
        session_id: Uuid,
        auto_approve_todos: bool,
        auto_merge_prs: bool,
        /// Whether managed team mode is available for this project. Off for
        /// every project that has not explicitly enabled it, including ones
        /// whose `.apas` predates the field -- `serde(default)` makes absent
        /// mean off, which is the intended migration. Only a project's owner
        /// or admin can change it; see `WebToServer::UpdateProjectFlags`.
        #[serde(default)]
        team_enabled: bool,
        /// Tab types this project refuses to create, as `<kind>:<provider>`
        /// keys (see `tab_type_key`). A *deny* list so that absent means
        /// "everything allowed" — see `tab_type_allowed`. Owner/admin only,
        /// same gate as the flags above.
        #[serde(default)]
        disallowed_tab_types: Vec<String>,
    },

    /// CLI's view of `team-todo.md`. Pushed in response to
    /// `ServerToCli::FetchTeamTodo` and after each `TodoApproval`-driven
    /// mutation. Server forwards to web as `ServerToWeb::TeamTodoState`.
    TeamTodoState {
        session_id: Uuid,
        state: TeamTodoStateMsg,
    },

    /// CLI's view of `suggested-workers.md`. Pushed in response to
    /// `ServerToCli::FetchSuggestedWorkers` and after each
    /// `DismissSuggestion`-driven mutation. Server forwards to web as
    /// `ServerToWeb::SuggestedWorkersState`.
    SuggestedWorkersState {
        session_id: Uuid,
        suggestions: Vec<SuggestedWorkerMsg>,
    },

    /// Raw pty bytes from a [`PaneKind::Terminal`] pane.
    ///
    /// Deliberately NOT `Output`/`StreamMessage`: those are chat records
    /// and get persisted into `data/sessions/<id>/messages.jsonl`, which
    /// raw ANSI would both bloat and break for the message renderer.
    /// Terminal bytes are transient — the server keeps only a bounded
    /// in-memory scrollback ring and never writes them to disk.
    ///
    /// Base64 because a pty read can split both UTF-8 sequences and ANSI
    /// escapes mid-way; the bytes are only reassembled by the terminal
    /// emulator in the browser.
    TerminalOutput {
        session_id: Uuid,
        pane_id: u32,
        /// Identifies one spawned pty process. Optional so a new server can
        /// continue accepting output from CLIs predating reconciliation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance_id: Option<Uuid>,
        data_b64: String,
        /// Monotonic per-pane chunk counter, starting at 0 when the pty
        /// is spawned. Lets the web detect a gap after a reconnect and
        /// lets the server order a snapshot against live output.
        seq: u64,
    },

    /// A terminal pane's child process ended. The pty is gone; the web
    /// shows the status and offers a respawn rather than silently
    /// freezing on the last frame.
    TerminalExited {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance_id: Option<Uuid>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },

    /// Authoritative terminal state, emitted on spawn/exit and for every
    /// configured terminal immediately after each CLI session reconnect.
    TerminalState {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance_id: Option<Uuid>,
        #[serde(default)]
        lifecycle: TerminalLifecycle,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
}

/// Messages sent from server to CLI client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerToCli {
    /// Registration successful
    Registered { cli_id: Uuid },

    /// Registration failed
    RegistrationFailed { reason: String },

    /// Client version is too old
    VersionUnsupported {
        client_version: String,
        min_version: String,
    },

    /// Server refused to start the session (e.g. session_id already owned by
    /// a different user — typically caused by a shared .apas file). The CLI
    /// should surface the reason and exit.
    SessionRejected { session_id: Uuid, reason: String },

    /// New session assigned to this CLI
    SessionAssigned {
        session_id: Uuid,
        working_dir: Option<String>,
    },

    /// User input from web client
    Input {
        session_id: Uuid,
        data: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
    },

    /// Signal to send to Claude process (e.g., SIGINT)
    Signal { session_id: Uuid, signal: String },

    /// Session disconnected from web
    SessionDisconnected { session_id: Uuid },

    /// Heartbeat response
    Heartbeat,

    /// Server asks the CLI to send a fresh `team-todo.md` snapshot. CLI
    /// replies with `CliToServer::TeamTodoState`.
    FetchTeamTodo { session_id: Uuid },

    /// Server forwards a web-side approval / rejection for a Global
    /// TODO. CLI applies it through in-process `team_todo` helpers and
    /// republishes `CliToServer::TeamTodoState`.
    TodoApproval {
        session_id: Uuid,
        todo_id: String,
        /// "approve" | "reject"
        action: String,
    },

    /// Server forwards a web-side request to add a new Global TODO.
    /// CLI picks the next id, writes status=approved, origin=user, and
    /// republishes TeamTodoState.
    AddTodo {
        session_id: Uuid,
        title: String,
        #[serde(default)]
        body: String,
    },

    /// Server asks the CLI to send a fresh `suggested-workers.md`
    /// snapshot. CLI replies with `CliToServer::SuggestedWorkersState`.
    FetchSuggestedWorkers { session_id: Uuid },

    /// Web user dismissed a Manager-proposed worker suggestion. CLI
    /// removes the section from `suggested-workers.md` and republishes
    /// the state.
    DismissSuggestion {
        session_id: Uuid,
        suggestion_id: String,
    },

    /// Flip a pane's `managed` field from false to true. CLI updates
    /// PaneMeta + persists to .apas + re-broadcasts the PaneList.
    /// One-way; there's no demote.
    PromotePaneToManaged {
        session_id: Uuid,
        pane_id: u32,
    },

    /// Pause the deadloop (legacy - use PausePane for new code)
    PauseDeadloop { session_id: Uuid },

    /// Resume the deadloop (legacy - use ResumePane for new code)
    ResumeDeadloop { session_id: Uuid },

    /// Pause a specific pane
    PausePane { session_id: Uuid, pane_id: u32 },

    /// Resume a specific pane
    ResumePane { session_id: Uuid, pane_id: u32 },

    /// Soft-restart a pane's agent: kill the running child process, then
    /// re-spawn the worker with the same config and current agent session id
    /// when available, so `--resume` can preserve prior context.
    /// Lighter-weight than `RebootCli`; targets only one pane.
    RebootPane { session_id: Uuid, pane_id: u32 },

    /// Add a new pane to the session
    AddPane {
        session_id: Uuid,
        pane_config: PaneConfig,
        /// Phase 1.1e: when true, the CLI creates a fresh git worktree
        /// for this pane (path written into the in-memory PaneConfig
        /// before spawn). pane_config.worktree_path is ignored when this
        /// is true — the CLI computes the path.
        #[serde(default)]
        isolated_worktree: bool,
    },

    /// Remove a pane from the session. `cleanup_action` (when Some) is
    /// applied to the pane's isolated worktree before final teardown.
    /// Phase 1.1d.
    RemovePane {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cleanup_action: Option<PaneCleanupAction>,
    },

    /// Start bot (deadloop) on a pane
    StartBot {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_iteration_interval_minutes: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort: Option<String>,
    },

    /// Stop bot on a pane (revert to interactive)
    StopBot { session_id: Uuid, pane_id: u32 },

    /// Reboot the CLI process
    RebootCli { session_id: Uuid },

    /// Request CLI to send its current PaneList
    RequestPaneList { session_id: Uuid },

    /// Update a pane's Claude thinking-effort override without starting a bot,
    /// so the CLI can persist it to the .apas file.
    UpdatePaneEffort {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort: Option<String>,
    },

    /// Update a pane's display label so the CLI can persist it to .apas.
    /// Before this existed, the server handled `WebToServer::UpdatePaneLabel`
    /// purely in its own cache; on next CLI restart the cache was clobbered
    /// by the CLI's PaneList carrying the unchanged on-disk label, and the
    /// rename appeared to silently revert.
    UpdatePaneLabel {
        session_id: Uuid,
        pane_id: u32,
        label: String,
    },

    /// Switch a pane's agent backend (provider + model). Kills the
    /// running agent child immediately so any in-flight turn is
    /// dropped, then respawns the streaming worker with a fresh
    /// session id — the new agent starts with no prior chat context
    /// (user is warned about this in the UI). The on-screen chat
    /// history stays visible.
    ///
    /// `provider: None` keeps the current provider; `model: None`
    /// clears any model override (each provider has its own default).
    /// Sending both lets the web swap providers + reset model in a
    /// single round-trip — useful when moving e.g. from claude to
    /// codex (different binary, different model namespace).
    UpdatePaneModel {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<Provider>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },

    /// Interrupt a pane's agent subprocess (SIGINT). Used to unwedge a turn
    /// stuck in a tool call so the queued user input can be processed.
    InterruptPane { session_id: Uuid, pane_id: u32 },

    /// Forward an AskUserQuestion answer from the web UI down to the CLI's
    /// streaming worker, which writes the matching control_response onto
    /// claude's stdin to complete the canUseTool callback.
    AnswerQuestion {
        session_id: Uuid,
        /// Claude's tool_use_id for the AskUserQuestion call. Must match the
        /// id from the original tool_use block so the CLI can look up the
        /// pending control_request.
        tool_use_id: String,
        /// Map of question text → selected option label(s). Multi-select
        /// values are joined with ", ". Built by the web UI from the user's
        /// selections.
        answers: std::collections::HashMap<String, String>,
    },

    /// Compute and return the current `git diff` for a pane's isolated
    /// worktree branch against the project's HEAD. Phase 1.2a.
    RequestPaneDiff { session_id: Uuid, pane_id: u32 },

    /// Ask the CLI to push the pane's branch to origin and create a
    /// GitHub PR via `gh pr create --fill`. Triggered by the "Create PR"
    /// button on the Diff modal. The CLI responds with
    /// `CliToServer::PrCreated`.
    CreatePr { session_id: Uuid, pane_id: u32 },

    /// Manager v2 — write `goal` into project_goal.md at the project
    /// root (overwriting any existing content).
    UpdateProjectGoal { session_id: Uuid, goal: String },

    /// Toggle the Tech-Lead autonomy flags. Persisted into `.apas` so
    /// they survive a CLI reboot; the Tech Lead re-reads `.apas` each
    /// iteration and unlocks the matching capability when the flag is
    /// true.
    UpdateProjectFlags {
        session_id: Uuid,
        auto_approve_todos: bool,
        auto_merge_prs: bool,
        /// See `CliToServer::ProjectFlagsChanged::team_enabled`. The CLI
        /// persists this to `.apas`, refuses `StartTeam` while it is false,
        /// and stops any running team on a true -> false transition.
        #[serde(default)]
        team_enabled: bool,
        /// Tab types this project refuses to create, as `<kind>:<provider>`
        /// keys (see `tab_type_key`). A *deny* list so that absent means
        /// "everything allowed" — see `tab_type_allowed`. Owner/admin only,
        /// same gate as the flags above.
        #[serde(default)]
        disallowed_tab_types: Vec<String>,
    },

    /// Spawn the default team (Manager, Tech Lead, Reviewer, Developer)
    /// for any role that isn't already present. Triggered by the "Start
    /// team" button on the Overview. Idempotent — extra clicks just
    /// fill in roles the user removed. The four `*_spec` fields carry
    /// the per-role provider/model picks the user made in the Team
    /// setup card; empty fields keep the CLI's defaults.
    StartTeam {
        session_id: Uuid,
        #[serde(default)]
        manager: TeamRoleSpec,
        #[serde(default)]
        tech_lead: TeamRoleSpec,
        #[serde(default)]
        reviewer: TeamRoleSpec,
        #[serde(default)]
        developer: TeamRoleSpec,
    },

    /// Set role/goal/backstory on the named pane and persist to .apas.
    /// Phase 2.1c.
    UpdatePaneRole {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        goal: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backstory: Option<String>,
    },

    /// Forward a user's plan-review verdict to the CLI streaming worker
    /// that parked the tool_use. Phase 3.2b2.
    PlanReviewAnswer {
        session_id: Uuid,
        tool_use_id: String,
        approve: bool,
    },

    /// Set the per-pane plan_review_mode policy and persist to .apas.
    /// Phase 3.2c.
    UpdatePaneReviewMode {
        session_id: Uuid,
        pane_id: u32,
        mode: PlanReviewMode,
    },

    /// v3.2: flip a worker between autonomous (`false`, default) and
    /// manual (`true`) modes. Persisted to `.apas`. The Tech Lead reads
    /// the field when picking delegation targets and skips manual workers.
    UpdatePaneManualMode {
        session_id: Uuid,
        pane_id: u32,
        manual_mode: bool,
    },

    /// Keystrokes for a [`PaneKind::Terminal`] pane, forwarded verbatim
    /// to the pty master. Base64 so control bytes and partial UTF-8 from
    /// the browser survive the JSON hop intact.
    TerminalInput {
        session_id: Uuid,
        pane_id: u32,
        data_b64: String,
    },

    /// Viewport size change for a terminal pane. Applied to the pty via
    /// `TIOCSWINSZ`, which is what makes the hosted TUI re-layout.
    TerminalResize {
        session_id: Uuid,
        pane_id: u32,
        cols: u16,
        rows: u16,
    },
}

// ============================================================================
// Daemon <-> Server Messages
// ============================================================================

/// Messages sent from machine daemon to server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonToServer {
    /// Daemon registers with the server using auth token + machine info
    Register {
        token: String,
        machine: MachineInfo,
        projects: Vec<MachineProjectInfo>,
    },

    /// Periodic heartbeat with latest project states
    Heartbeat { projects: Vec<MachineProjectInfo> },

    /// Update machine metadata (for config changes without reconnect)
    MachineInfoUpdate { machine: MachineInfo },

    /// Result of a `CreateProjectInstance` request. Carries the new project_id
    /// + path on success, or an `error` on failure (clone/auth/dir collision) —
    /// failures for a never-registered project can't be expressed via Heartbeat.
    ProjectInstanceCreated {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

/// Messages sent from server to machine daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerToDaemon {
    /// Registration successful
    Registered { machine_id: Uuid },

    /// Registration failed
    RegistrationFailed { reason: String },

    /// Start APAS CLI for a project on this machine
    StartProjectCli { project_id: String },

    /// Stop APAS CLI for a project on this machine
    StopProjectCli { project_id: String },

    /// Clone a repo into a new instance directory, branch it, register a
    /// `.apas`, and start it. The daemon mints the project_id (so, unlike
    /// StartProjectCli, this carries no project_id) and resolves the dest path
    /// + clone URL machine-side.
    CreateProjectInstance {
        git_remote: String,
        instance_name: String,
        branch: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clone_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
    },

    /// Request a fresh project scan/update push
    RefreshProjects,

    /// Update machine-level MiniMax backend API configuration
    SetMiniMaxConfig {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_base_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key: Option<String>,
        #[serde(default)]
        clear_api_key: bool,
    },

    /// Update machine-level GLM backend API configuration
    SetGlmConfig {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_base_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key: Option<String>,
        #[serde(default)]
        clear_api_key: bool,
    },

    /// Update machine-level DeepSeek backend API configuration. DeepSeek
    /// ships an Anthropic-compatible endpoint at <api_base_url>/anthropic
    /// so it reuses the Claude CLI binary with `ANTHROPIC_BASE_URL` /
    /// `ANTHROPIC_API_KEY` env overrides, same shape as the GLM bridge.
    SetDeepseekConfig {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_base_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key: Option<String>,
        #[serde(default)]
        clear_api_key: bool,
    },

    /// Heartbeat response
    Heartbeat,
}

// ============================================================================
// Web <-> Server Messages
// ============================================================================

/// Messages sent from web client to server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebToServer {
    /// Authenticate with JWT token
    Authenticate { token: String },

    /// List available CLI clients
    ListCliClients,

    /// List daemon-reported machines and projects for this user
    ListMachines,

    /// Start a new session (optionally specify CLI client)
    StartSession { cli_client_id: Option<Uuid> },

    /// Resume an existing session
    ResumeSession { session_id: Uuid },

    /// Attach to observe an existing CLI session (hybrid mode)
    AttachSession { session_id: Uuid },

    /// User input to send to Claude.
    /// When `session_id` is set, the server routes to that exact session
    /// (after verifying this web connection is attached to it). This is the
    /// only safe option for clients that multi-attach: the connection's
    /// last-attached session is non-deterministic when several attaches
    /// race. Omitted = legacy single-attach behaviour.
    Input {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        text: String,
        #[serde(default)]
        pane_type: Option<PaneType>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
        // Client-generated id for this send. The web client retransmits
        // inputs it hasn't seen acked (3s retry + reconnect replay); the
        // server uses this id to drop retransmits of an input it already
        // stored instead of double-storing them.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_msg_id: Option<String>,
    },

    /// Approve a tool call. See `Input::session_id` for the multi-attach rationale.
    Approve {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        tool_call_id: String,
    },

    /// Reject a tool call. See `Input::session_id`.
    Reject {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        tool_call_id: String,
    },

    /// Send signal (e.g., cancel/interrupt). See `Input::session_id`.
    Signal {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        signal: String,
    },

    /// List all sessions (persisted)
    ListSessions,

    /// Get messages for a specific session (with optional pagination)
    GetSessionMessages {
        session_id: Uuid,
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        before_id: Option<String>, // Load messages before this message ID
        #[serde(default)]
        pane_type: Option<PaneType>, // Filter by pane type for per-pane pagination
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>, // Filter by pane ID for per-pane pagination
        // Reconnect catchup: when set, return only messages with
        // `created_at > after_created_at` (sorted ASC, flat across panes,
        // capped at CATCHUP_LIMIT). Reply is flagged `catchup: true` so the
        // client can append rather than replace. before_id / pane filters
        // are ignored when this is set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after_created_at: Option<String>,
        // Per-pane catchup watermarks. When set, return only messages
        // where `created_at > pane_watermarks[pane_id]`. Lets each
        // pane independently request its own tail without overfetching
        // — solves the "fast pane advances the watermark past a slow
        // pane's last-seen" bug that bit `after_created_at` (which
        // used a single per-session timestamp). Reply is flagged
        // `catchup: true`. Mutually exclusive with `after_created_at`;
        // server prefers `pane_watermarks` when both are present.
        // Pane ids ride as JSON object KEYS, which are always strings on the
        // wire. They must be `String` here (not `u32`): `WebToServer` is an
        // internally-tagged enum, and serde's tagged-enum path buffers into a
        // Content value that does NOT coerce string map keys to integers — a
        // `HashMap<u32, _>` here fails with `invalid type: string "2",
        // expected u32`, silently dropping every catchup. The server parses
        // the keys back to u32 when it hands them to storage.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_watermarks: Option<std::collections::HashMap<String, String>>,
    },

    /// Pause the deadloop session (legacy - use PausePane for new code)
    PauseDeadloop {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
    },

    /// Resume the deadloop session (legacy - use ResumePane for new code)
    ResumeDeadloop {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
    },

    /// Pause a specific pane. See `Input::session_id`.
    PausePane {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        pane_id: u32,
    },

    /// Resume a specific pane. See `Input::session_id`.
    ResumePane {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        pane_id: u32,
    },

    /// Soft-restart a single pane's agent on its existing session when
    /// possible (see `ServerToCli::RebootPane`).
    RebootPane {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        pane_id: u32,
    },

    /// Add a new pane
    AddPane {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        provider: Provider,
        mode: PaneMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// When true, the CLI creates an isolated git worktree under
        /// `<project>/.apas-worktrees/pane-<id>` (branch `apas-pane-<id>`)
        /// before spawning, and persists the resulting absolute path on
        /// PaneConfig.worktree_path. Phase 1.1e.
        #[serde(default)]
        isolated_worktree: bool,
        /// Initial role/goal/backstory/plan_review_mode applied to the new
        /// pane BEFORE the first spawn — so a templated worker (Add Worker
        /// modal on Overview) uses the right system prompt immediately
        /// instead of needing a close+reopen. All optional; missing fields
        /// keep the legacy "set via Role modal later" path.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        goal: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backstory: Option<String>,
        #[serde(default)]
        plan_review_mode: PlanReviewMode,
        /// v3.5 — true when this pane is being added as part of the
        /// project team, false when it's a TabBar `+` side chat. See
        /// PaneConfig::managed.
        #[serde(default)]
        managed: bool,
        /// Agent (default) vs Terminal. A Terminal pane ignores `prompt`,
        /// `role`, `goal`, `backstory` and `plan_review_mode` — there is
        /// no system prompt to inject into a pty-hosted TUI.
        #[serde(default)]
        kind: PaneKind,
    },

    /// Remove a pane. When the pane has an isolated worktree assigned
    /// (`PaneConfig.worktree_path` set), `cleanup_action` says what to do
    /// with that worktree and its branch. None = leave the on-disk worktree
    /// and branch alone (legacy behaviour — just unlink the pane from .apas).
    RemovePane {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cleanup_action: Option<PaneCleanupAction>,
    },

    /// Update a pane's custom label
    UpdatePaneLabel {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        pane_id: u32,
        label: String,
    },

    /// Update a pane's Claude thinking-effort override. Persisted in the
    /// project .apas so switching tabs doesn't reset it to default.
    /// `session_id` is optional (see `InterruptPane`): when present the server
    /// validates/auto-attaches this connection to that session, so the change
    /// isn't misrouted to the connection's drifting last-attached session
    /// (which breaks the mobile fan-out); absent = legacy connection-session.
    UpdatePaneEffort {
        #[serde(default)]
        session_id: Option<Uuid>,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort: Option<String>,
    },

    /// Switch a pane's agent backend (provider + model). The CLI kills
    /// the current agent child and respawns with a fresh session id —
    /// the new agent starts with no prior chat context. See
    /// `ServerToCli::UpdatePaneModel` for the full semantics.
    /// `session_id` is optional (see `InterruptPane`): when present the server
    /// validates/auto-attaches this connection to that session, so the switch
    /// isn't misrouted to the connection's drifting last-attached session
    /// (which broke the model selector on mobile); absent = legacy behavior.
    UpdatePaneModel {
        #[serde(default)]
        session_id: Option<Uuid>,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<Provider>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },

    /// Interrupt the agent process running for a pane (e.g. claude wedged on
    /// a long-running Bash tool call). The CLI signals SIGINT to its
    /// subprocess so the current turn aborts and queued input can flow.
    /// `session_id` is optional for backward compat: when present the server
    /// validates/auto-attaches the connection to that session (so "Stop team"
    /// interrupts survive a just-reconnected connection); when absent it falls
    /// back to the connection's currently-attached session.
    InterruptPane {
        #[serde(default)]
        session_id: Option<Uuid>,
        pane_id: u32,
    },

    /// Reorder panes (array of pane_ids in desired order)
    ReorderPanes {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        pane_ids: Vec<u32>,
    },

    /// Start bot (deadloop) on a pane — converts interactive pane to deadloop
    StartBot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_iteration_interval_minutes: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort: Option<String>,
    },

    /// Stop bot on a pane — converts deadloop pane back to interactive
    StopBot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        pane_id: u32,
    },

    /// Reboot the CLI process
    RebootCli {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
    },

    /// Start APAS CLI for a daemon project
    StartMachineProjectCli {
        machine_id: Uuid,
        project_id: String,
    },

    /// Stop APAS CLI for a daemon project
    StopMachineProjectCli {
        machine_id: Uuid,
        project_id: String,
    },

    /// Create a brand-new project instance under a repo on a chosen machine:
    /// the daemon clones `clone_url` into `~/apas_projects/<instance_name>`
    /// (auto-suffixed on collision), checks out a fresh `branch`, registers a
    /// `.apas`, and starts it. `git_remote` is the canonical key (for the
    /// daemon's sibling-URL fallback + naming); `clone_url` is the real URL.
    CreateProjectInstance {
        machine_id: Uuid,
        git_remote: String,
        instance_name: String,
        branch: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clone_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
    },

    /// Update machine-level MiniMax backend API configuration
    SetMachineMiniMaxConfig {
        machine_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_base_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key: Option<String>,
        #[serde(default)]
        clear_api_key: bool,
    },

    /// Update machine-level GLM backend API configuration
    SetMachineGlmConfig {
        machine_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_base_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key: Option<String>,
        #[serde(default)]
        clear_api_key: bool,
    },

    /// Update machine-level DeepSeek backend API configuration
    SetMachineDeepseekConfig {
        machine_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_base_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key: Option<String>,
        #[serde(default)]
        clear_api_key: bool,
    },

    /// Download all session data
    DownloadSession { session_id: Uuid },

    /// Submit answers to a pending AskUserQuestion tool call. The server
    /// relays this to the CLI which writes a control_response onto claude's
    /// stdin so the SDK's canUseTool callback completes with these answers.
    AnswerQuestion {
        /// The session the answered AskUserQuestion belongs to. Optional for
        /// backward compat with older web clients. When present the server
        /// routes the answer deterministically via `resolve_target_session`
        /// instead of falling back to the connection's last-attached session
        /// — the multi-session fan-out drifts that, which misrouted answers
        /// to a different project and left the asking pane stuck.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        tool_use_id: String,
        /// Question text → selected option label(s) joined with ", " for
        /// multi-select.
        answers: std::collections::HashMap<String, String>,
    },

    /// Ask the CLI for the current `git diff` between the pane's worktree
    /// branch and the project's HEAD. Returns via `ServerToWeb::PaneDiff`.
    /// Phase 1.2a.
    RequestPaneDiff {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        pane_id: u32,
    },

    /// Web → server → CLI: push the pane's branch and run
    /// `gh pr create --fill` in its worktree. Result rides
    /// `ServerToWeb::PrCreated`.
    CreatePr {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        pane_id: u32,
    },

    /// Manager v2 — overwrite the project_goal.md file at the project
    /// root with `goal`. The deadloop manager re-reads this file on
    /// every iteration so a goal change takes effect at the next loop
    /// boundary, not mid-iteration.
    UpdateProjectGoal {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        goal: String,
    },

    /// Set Tech-Lead autonomy flags. CLI writes both into `.apas` and
    /// then echoes `CliToServer::ProjectFlagsChanged` so peer web
    /// clients stay in sync.
    UpdateProjectFlags {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        auto_approve_todos: bool,
        auto_merge_prs: bool,
        /// Managed team mode on/off for this project. **Owner/admin only** --
        /// the server drops the whole message from a plain `user`, because
        /// these are project-level policy, not per-seat preferences
        /// (`auto_merge_prs` alone lets the Tech Lead merge PRs unattended).
        #[serde(default)]
        team_enabled: bool,
        /// Tab types this project refuses to create, as `<kind>:<provider>`
        /// keys (see `tab_type_key`). A *deny* list so that absent means
        /// "everything allowed" — see `tab_type_allowed`. Owner/admin only,
        /// same gate as the flags above.
        #[serde(default)]
        disallowed_tab_types: Vec<String>,
    },

    /// Spawn the four default team panes for any role that isn't
    /// already present. Triggered by the Overview "Start team" button.
    /// CLI runs `spawn_missing_team_panes`. Idempotent. Per-role
    /// `*_spec` fields carry the provider/model the user picked in
    /// the Team setup card.
    StartTeam {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        #[serde(default)]
        manager: TeamRoleSpec,
        #[serde(default)]
        tech_lead: TeamRoleSpec,
        #[serde(default)]
        reviewer: TeamRoleSpec,
        #[serde(default)]
        developer: TeamRoleSpec,
    },

    /// Update a pane's role/goal/backstory triple (Phase 2.1c). All three
    /// fields are optional — sending null for any of them clears that
    /// piece. Takes effect on the next pane spawn (close + reopen tab,
    /// or reboot the apas CLI) since claude reads --append-system-prompt
    /// only at launch.
    UpdatePaneRole {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        goal: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backstory: Option<String>,
    },

    /// User's verdict on a held tool_use from Phase 3.2b. `approve = true`
    /// resumes the agent's turn; `approve = false` rejects the tool, which
    /// claude will surface as a tool_result error.
    PlanReviewAnswer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        tool_use_id: String,
        approve: bool,
    },

    /// Update a pane's plan_review_mode (Phase 3.2c). Effective immediately
    /// for future control_requests — the CLI reads the field at decision
    /// time, not at spawn.
    UpdatePaneReviewMode {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        pane_id: u32,
        mode: PlanReviewMode,
    },

    /// v3.2: web → server → CLI: flip a worker between autonomous and
    /// manual modes. CLI updates PaneMeta + PaneConfig + persists to .apas.
    UpdatePaneManualMode {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        pane_id: u32,
        manual_mode: bool,
    },

    /// Ask the server to fetch the current `team-todo.md` for this
    /// session's project. Forwarded as `ServerToCli::FetchTeamTodo`;
    /// CLI replies with `CliToServer::TeamTodoState` which the server
    /// hands back to web as `ServerToWeb::TeamTodoState`.
    FetchTeamTodo { session_id: Uuid },

    /// User approves or rejects a Tech-Lead-proposed Global TODO from
    /// the web Overview tab. action: "approve" | "reject".
    TodoApproval {
        session_id: Uuid,
        todo_id: String,
        action: String,
    },

    /// User adds a new Global TODO from the Overview's + Add TODO
    /// form. CLI assigns the next id (TODO-NNN), writes status=approved,
    /// origin=user, then publishes a fresh TeamTodoState.
    AddTodo {
        session_id: Uuid,
        title: String,
        #[serde(default)]
        body: String,
    },

    /// Web requests a fresh snapshot of `suggested-workers.md`.
    /// Forwarded as `ServerToCli::FetchSuggestedWorkers`; CLI replies
    /// with `CliToServer::SuggestedWorkersState` which the server hands
    /// back to web as `ServerToWeb::SuggestedWorkersState`.
    FetchSuggestedWorkers { session_id: Uuid },

    /// User dismissed a suggested worker. Forwarded as
    /// `ServerToCli::DismissSuggestion`; CLI removes the entry and
    /// republishes the state.
    DismissSuggestion {
        session_id: Uuid,
        suggestion_id: String,
    },

    /// One-way promote: flip an unmanaged side-chat pane to a managed
    /// team member. CLI sets PaneMeta.managed = true and re-broadcasts
    /// the PaneList. There's no demote — keep it simple.
    PromotePaneToManaged {
        session_id: Uuid,
        pane_id: u32,
    },

    /// Keystrokes typed into a terminal pane's xterm.js view. Server
    /// resolves the target session and forwards as
    /// `ServerToCli::TerminalInput`.
    TerminalInput {
        session_id: Uuid,
        pane_id: u32,
        data_b64: String,
    },

    /// The browser's terminal viewport was resized (or first measured).
    /// Forwarded to the CLI so the pty — and therefore the hosted TUI —
    /// matches what the user actually sees.
    TerminalResize {
        session_id: Uuid,
        pane_id: u32,
        cols: u16,
        rows: u16,
    },

    /// Web opened a terminal pane and wants the current scrollback.
    /// Server replies `ServerToWeb::TerminalSnapshot` from its in-memory
    /// ring buffer; no CLI round-trip, so reattach is instant and works
    /// even while the CLI is mid-reconnect.
    TerminalAttach {
        session_id: Uuid,
        pane_id: u32,
    },

    /// Liveness probe from the browser. Server echoes
    /// `ServerToWeb::Heartbeat` so the client can detect silently-stale
    /// connections (mobile OS throttling, NAT timeout, swallowed RST)
    /// without depending on `readyState`.
    Heartbeat,
}

/// Messages sent from server to web client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerToWeb {
    /// Authentication successful
    Authenticated {
        user_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_email: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        server_version: Option<String>,
    },

    /// Authentication failed
    AuthenticationFailed { reason: String },

    /// Session started
    SessionStarted {
        session_id: Uuid,
        #[serde(default)]
        pane_type: Option<PaneType>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
    },

    /// Session status update
    SessionStatus { status: SessionStatus },

    /// Session attached confirmation (includes whether CLI is active)
    SessionAttached {
        session_id: Uuid,
        has_active_cli: bool,
    },

    /// Output from Claude
    Output {
        content: String,
        #[serde(default)]
        output_type: OutputType,
        #[serde(default)]
        pane_type: Option<PaneType>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
    },

    /// Error message
    Error { message: String },

    /// List of available CLI clients
    CliClients { clients: Vec<CliClientInfo> },

    /// List of daemon-reported machines and projects
    Machines { machines: Vec<MachineWithProjects> },

    /// Structured message from Claude CLI stream-json output
    StreamMessage {
        session_id: Uuid,
        message: ClaudeStreamMessage,
        #[serde(default)]
        pane_type: Option<PaneType>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
        // Server's storage timestamp (max created_at of the stored messages
        // produced from this stream event). The web client tracks the max
        // across all stream messages it receives so it can ask the server
        // for `after_created_at = max` on reconnect to fetch the gap that
        // landed while its WS was down. Optional for forward compat with
        // older clients/servers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_at: Option<String>,
    },

    /// List of persisted sessions
    Sessions { sessions: Vec<SessionInfo> },

    /// Messages for a session
    SessionMessages {
        session_id: Uuid,
        messages: Vec<MessageInfo>,
        #[serde(default)]
        has_more: bool, // True if there are older messages to load
        // True when this payload is a reconnect catchup (response to a
        // GetSessionMessages that carried `after_created_at`). Client should
        // append to existing pane state rather than treat as initial load.
        #[serde(default)]
        catchup: bool,
    },

    /// User input/prompt from CLI (displayed in web UI)
    UserInput {
        session_id: Uuid,
        text: String,
        #[serde(default)]
        pane_type: Option<PaneType>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
        // Storage timestamp — same role as on StreamMessage: keeps the web
        // client's reconnect-catchup high-water mark accurate even when a
        // session opens with a user input before any stream activity.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_at: Option<String>,
        // Echoes WebToServer::Input::client_msg_id so the sending client
        // can ack its pending-send queue and dedup echoes by id instead
        // of the content+recency heuristic. None for CLI-originated input.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_msg_id: Option<String>,
    },

    /// Deadloop pause status update (legacy - use PanePaused for new code)
    DeadloopStatus { session_id: Uuid, is_paused: bool },

    /// Pane pause status update
    PanePaused {
        session_id: Uuid,
        pane_id: u32,
        is_paused: bool,
    },

    /// Pane status update (e.g., "thinking") for status bar display.
    /// `session_id` lets the web client drop statuses for sessions it is
    /// not currently viewing — the web is multi-attached to keep background
    /// tabs live, but only the foreground tab should drive the status pill.
    PaneStatus {
        session_id: Uuid,
        #[serde(default)]
        pane_type: PaneType,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
        status: Option<String>,
    },

    /// List of pane configurations for a session
    PaneList {
        session_id: Uuid,
        panes: Vec<PaneConfig>,
    },

    /// Usage limits update from a CLI client
    UsageLimits {
        cli_client_id: Uuid,
        #[serde(default)]
        provider: Provider,
        limits: UsageLimits,
    },

    /// Full session data for download
    SessionDownload {
        session_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project_id: Option<Uuid>,
        messages: Vec<MessageInfo>,
        working_dir: Option<String>,
        hostname: Option<String>,
        created_at: Option<String>,
    },

    /// One team scratchpad record forwarded from the CLI. Phase 2.2b.
    TeamRecord {
        session_id: Uuid,
        record: TeamScratchpadRecord,
    },

    /// Plan-review request forwarded from CLI. Phase 3.2b2.
    PlanReviewRequest {
        session_id: Uuid,
        pane_id: u32,
        tool_use_id: String,
        tool_name: String,
        input: serde_json::Value,
    },

    /// On-demand diff for a pane's isolated worktree branch. Phase 1.2a.
    PaneDiff {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// Forwarded from `CliToServer::PrCreated` after the CLI ran
    /// `gh pr create --fill`. The web shows a toast with the URL or
    /// the error.
    PrCreated {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// Forwarded from `CliToServer::ProjectGoalChanged`. The web caches
    /// this per-session and refreshes the Project goal textbox when the
    /// user isn't actively editing.
    ProjectGoalChanged {
        session_id: Uuid,
        content: String,
    },

    /// Per-project and per-pane usage stats (prompt/token/cost counts) for
    /// the Overview. Pushed after each turn is recorded and replayed on
    /// attach; the server aggregates from the day-bucketed stats table.
    ProjectUsageStats {
        session_id: Uuid,
        stats: ProjectUsageStats,
    },

    /// Result of a web-initiated `CreateProjectInstance`, relayed from the
    /// daemon's ack to the requesting user's web clients as a toast. Distinct
    /// from the generic `Error` variant (which renders in the chat log).
    ProjectInstanceCreated {
        machine_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// Forwarded from `CliToServer::ProjectFlagsChanged`. Web mirrors
    /// the latest Tech-Lead autonomy flags per session for the Overview
    /// toggles.
    ProjectFlagsChanged {
        session_id: Uuid,
        auto_approve_todos: bool,
        auto_merge_prs: bool,
        /// Drives whether the web shows any team surface at all. The CLI
        /// re-broadcasts this every 5s from `.apas`, so a web client that
        /// attaches mid-session hydrates without asking.
        #[serde(default)]
        team_enabled: bool,
        /// Tab types this project refuses to create, as `<kind>:<provider>`
        /// keys (see `tab_type_key`). A *deny* list so that absent means
        /// "everything allowed" — see `tab_type_allowed`. Owner/admin only,
        /// same gate as the flags above.
        #[serde(default)]
        disallowed_tab_types: Vec<String>,
    },

    /// Snapshot of the project's team-todo.md state. Sent in reply to
    /// `WebToServer::FetchTeamTodo` and after every CLI-side mutation
    /// triggered by `WebToServer::TodoApproval`. See
    /// `docs/todo-driven-workflow.md`.
    TeamTodoState {
        session_id: Uuid,
        state: TeamTodoStateMsg,
    },

    /// Snapshot of the project's suggested-workers.md state. Sent in
    /// reply to `WebToServer::FetchSuggestedWorkers` and after every
    /// CLI-side mutation triggered by `WebToServer::DismissSuggestion`.
    SuggestedWorkersState {
        session_id: Uuid,
        suggestions: Vec<SuggestedWorkerMsg>,
    },

    /// Live pty bytes for a terminal pane, fanned out to every web
    /// client attached to the session.
    TerminalOutput {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance_id: Option<Uuid>,
        data_b64: String,
        seq: u64,
    },

    /// Replayed scrollback in answer to `WebToServer::TerminalAttach`.
    ///
    /// `seq` is the sequence of the newest chunk included, so the client
    /// can drop any live `TerminalOutput` it already buffered at or below
    /// it instead of double-rendering. `truncated` is true when the ring
    /// buffer dropped older bytes, which means the replay may begin
    /// partway through an escape sequence — the client writes a reset
    /// first so a clipped sequence can't corrupt the emulator state.
    TerminalSnapshot {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance_id: Option<Uuid>,
        data_b64: String,
        seq: u64,
        #[serde(default)]
        truncated: bool,
        #[serde(default)]
        lifecycle: TerminalLifecycle,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },

    /// A terminal pane's child process ended.
    TerminalExited {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance_id: Option<Uuid>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },

    /// Live terminal lifecycle change. Snapshot carries the same fields for
    /// clients that attach after this event was emitted.
    TerminalState {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance_id: Option<Uuid>,
        #[serde(default)]
        lifecycle: TerminalLifecycle,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },

    /// Echo of `WebToServer::Heartbeat`. The browser's liveness loop
    /// uses any inbound frame (including this one) as proof the WS is
    /// healthy; a missing echo within `livenessMs` triggers reconnect.
    Heartbeat,
}

/// Wire format for a snapshot of `team-todo.md`. Mirrors the CLI's
/// `team_todo::TeamTodo` but with status fields kept as strings so we
/// don't have to ship the enum definitions across crate / language
/// boundaries. The web parses these into UI state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamTodoStateMsg {
    pub globals: Vec<TeamTodoGlobalMsg>,
    pub workers: Vec<TeamTodoWorkerMsg>,
    /// Per-agent scratchpad cursor (RFC3339 timestamp of the last
    /// record acted on). `None` means the cursor file is missing —
    /// either the agent hasn't iterated yet, or it was wiped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tech_lead_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamTodoGlobalMsg {
    pub id: String,
    pub title: String,
    /// proposed | approved | in_progress | under_review | pr_open | done | rejected
    pub status: String,
    /// user | tech-lead
    pub origin: String,
    /// One PR per contributing worker. Empty until any worker's branch
    /// has been pushed and PR'd.
    #[serde(default)]
    pub prs: Vec<PaneTodoPrMsg>,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PaneTodoPrMsg {
    pub pane_id: u32,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation: Option<String>,
}

/// One row in the suggested-workers queue. The Manager pane appends
/// these as `## SUG-NNN — label` sections to `suggested-workers.md`;
/// the Overview renders each as a card with Accept / Dismiss buttons.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SuggestedWorkerMsg {
    pub id: String,
    pub label: String,
    pub role: String,
    pub goal: String,
    pub backstory: String,
    #[serde(default)]
    pub needs_worktree: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamTodoWorkerMsg {
    pub pane_id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_hint: Option<String>,
    pub subtasks: Vec<TeamTodoSubtaskMsg>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamTodoSubtaskMsg {
    pub id: String,
    pub title: String,
    /// pending | in_progress | done | reviewing | revising | approved
    pub status: String,
    pub parent: String,
    pub body: String,
}

/// Information about a persisted session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: Uuid,
    /// Stable project identity from `.apas`. Web UI groups by this.
    /// Falls back to `id` for legacy rows that pre-date the column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Uuid>,
    pub cli_client_id: Option<Uuid>,
    pub working_dir: Option<String>,
    pub hostname: Option<String>,
    /// Canonical `host/owner/repo` of the project's git `origin` remote. The
    /// web sidebar groups projects with the same value under one repo header.
    /// `None`/absent means "no remote" (its own sidebar group).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_remote: Option<String>,
    /// Raw `origin` URL for this project's repo (the cloneable URL). Surfaced
    /// so the web can prefill the clone URL when creating a new instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_remote_url: Option<String>,
    pub status: String,
    pub created_at: Option<String>,
    /// True if this session is shared with the user (not owned)
    #[serde(default)]
    pub is_shared: bool,
    /// Email of the session owner (only set if is_shared is true)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_email: Option<String>,
    /// Share role for this user on the session ("owner", "admin", or "user")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_role: Option<String>,
    /// True if this session has an active CLI client connected
    #[serde(default)]
    pub is_active: bool,
}

/// Aggregated usage counters for a pane or project over one time window.
/// All token counts come from the per-turn Claude/Codex stream `result`
/// usage; `prompts` counts user/loop inputs and `responses` counts completed
/// turns. Fields are snake_case so the wire keys match the web store verbatim.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageCounters {
    #[serde(default)]
    pub prompts: u64,
    #[serde(default)]
    pub responses: u64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_creation_tokens: u64,
    /// Real cost in USD (only Claude transport reports it; 0 otherwise).
    #[serde(default)]
    pub cost_usd: f64,
}

/// Per-pane usage broken down by time window (cumulative lifetime plus the
/// rolling 7-day and today windows derived from day-bucketed rows).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PaneUsageStats {
    pub pane_id: u32,
    #[serde(default)]
    pub lifetime: UsageCounters,
    #[serde(default)]
    pub last_7d: UsageCounters,
    #[serde(default)]
    pub today: UsageCounters,
    /// Most recent activity timestamp (ISO 8601), max over the pane's buckets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_active: Option<String>,
}

/// Project-level usage: the per-pane breakdown plus the project totals
/// (sum over all panes of every session that shares this project_id).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectUsageStats {
    #[serde(default)]
    pub panes: Vec<PaneUsageStats>,
    #[serde(default)]
    pub lifetime: UsageCounters,
    #[serde(default)]
    pub last_7d: UsageCounters,
    #[serde(default)]
    pub today: UsageCounters,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_active: Option<String>,
}

/// Information about a persisted message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageInfo {
    pub id: String,
    pub role: String,
    pub content: String,
    pub message_type: String,
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<u32>,
}

// ============================================================================
// Shared Types
// ============================================================================

/// Machine-level MiniMax backend status safe to expose to web UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MiniMaxBackendInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_configured: bool,
}

/// Machine-level GLM backend status safe to expose to web UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlmBackendInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_configured: bool,
}

/// Machine-level DeepSeek backend status safe to expose to web UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepseekBackendInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_configured: bool,
}

/// Information about a machine reported by a daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineInfo {
    pub machine_id: Uuid,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimax_backend: Option<MiniMaxBackendInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glm_backend: Option<GlmBackendInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepseek_backend: Option<DeepseekBackendInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<String>,
}

/// APAS project discovered on a machine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineProjectInfo {
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub path: String,
    #[serde(default)]
    pub is_running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Resident set size of the headless CLI process, in KiB. Reported by the
    /// daemon from /proc/<pid>/status so the UI can spot runaway memory usage
    /// before the kernel does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_kb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// Machine with its project list
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineWithProjects {
    pub machine: MachineInfo,
    pub projects: Vec<MachineProjectInfo>,
}

/// Pane type for dual-pane mode (legacy - kept for backward compatibility)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PaneType {
    /// Autonomous deadloop worker (left pane)
    #[default]
    Deadloop,
    /// Interactive user session (right pane)
    Interactive,
}

/// Provider for a pane
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    /// `claude-old` / `claude_old` aliases keep panes serialized before the
    /// streaming-only switchover deserializing as the (now-only) Claude
    /// variant. The legacy per-turn `--print` worker has been removed; all
    /// `Provider::Claude` panes use the long-lived stream-json worker.
    #[default]
    #[serde(alias = "claude-old", alias = "claude_old")]
    Claude,
    Codex,
    Minimax,
    Glm,
    Deepseek,
    Opencode,
    #[serde(rename = "cursor-agent")]
    CursorAgent,
}

/// Mode for a pane
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PaneMode {
    Deadloop,
    Interactive,
}

/// How a pane hosts its agent process. Orthogonal to [`Provider`] (which
/// binary) and [`PaneMode`] (how autonomous).
///
/// * [`PaneKind::Agent`] — the original path: the CLI runs the provider
///   headlessly (`claude --print --output-format stream-json`,
///   `codex exec --json`) and parses structured events into
///   `CliToServer::StreamMessage`. Everything team-mode depends on —
///   usage counters, pane status, diffs, plan review, scratchpad
///   publishing, Tech Lead delegation — is built on those events.
/// * [`PaneKind::Terminal`] — the pane instead hosts the provider's real
///   interactive TUI on a pty. Raw bytes flow over the dedicated
///   `Terminal*` messages and are rendered by xterm.js in the browser.
///   Nothing is parsed, so a terminal pane has none of the structured
///   integrations above and is never a delegation target; it is a side
///   chat with a genuine terminal.
///
/// `#[serde(default)]` on `PaneConfig::kind` keeps `.apas` files written
/// before this existed deserializing as `Agent`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum PaneKind {
    #[default]
    Agent,
    Terminal,
}

impl PaneKind {
    pub fn is_terminal(self) -> bool {
        matches!(self, PaneKind::Terminal)
    }
}

/// Canonical key for a "tab type" — the pane kind plus the provider, e.g.
/// `agent:claude`, `terminal:codex`.
///
/// Kind and provider together are what the add-tab menu actually offers, and
/// what a project owner restricts: a claude *agent* tab and a claude
/// *terminal* tab are different capabilities (the terminal one runs the real
/// TUI with permission prompts bypassed), so neither half alone identifies a
/// tab type.
///
/// Derived from the serde names rather than hand-written, so a renamed or
/// added `Provider` variant cannot silently produce a key that no longer
/// matches a stored restriction.
pub fn tab_type_key(kind: PaneKind, provider: Provider) -> String {
    let kind = serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "agent".to_string());
    let provider = serde_json::to_value(provider)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "claude".to_string());
    format!("{kind}:{provider}")
}

/// Every tab type a user can actually be offered, in menu order.
///
/// Deliberately *not* every `Provider`. MiniMax, GLM, and DeepSeek are the
/// claude binary pointed at a different backend, so the add-tab menu offers
/// them as claude **models**, not providers — a `Provider::Minimax` key would
/// be a control that silently does nothing, because those tabs arrive as
/// `provider: claude`. Restricting by model would be a different, larger
/// feature.
///
/// Terminal panes exist only for claude and codex — see
/// `terminal_pane::terminal_binary_for`, which this must stay in step with.
pub fn all_tab_types() -> Vec<String> {
    vec![
        tab_type_key(PaneKind::Agent, Provider::Claude),
        tab_type_key(PaneKind::Agent, Provider::Codex),
        tab_type_key(PaneKind::Agent, Provider::Opencode),
        tab_type_key(PaneKind::Agent, Provider::CursorAgent),
        tab_type_key(PaneKind::Terminal, Provider::Claude),
        tab_type_key(PaneKind::Terminal, Provider::Codex),
    ]
}

/// Whether this project permits creating a tab of the given kind + provider.
///
/// Stored as a *deny* list rather than an allow list on purpose. An allow list
/// read through `#[serde(default)]` would make an absent field mean "nothing
/// permitted", so every project predating the feature would refuse to open any
/// tab at all. Empty deny list = everything allowed = existing projects
/// unaffected, which is the only safe default for a field arriving in an
/// upgrade. It also means a provider added later is permitted until an owner
/// says otherwise, rather than silently disappearing from their menu.
pub fn tab_type_allowed(disallowed: &[String], kind: PaneKind, provider: Provider) -> bool {
    let key = tab_type_key(kind, provider);
    !disallowed.iter().any(|d| d.trim().eq_ignore_ascii_case(&key))
}

/// Configuration for a single pane
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneConfig {
    pub pane_id: u32,
    pub provider: Provider,
    pub mode: PaneMode,
    /// Agent (headless stream-json, the default) vs Terminal (pty-hosted
    /// interactive TUI). See [`PaneKind`].
    #[serde(default)]
    pub kind: PaneKind,
    pub session_id: Uuid, // Provider-specific session for --resume
    #[serde(default)]
    pub is_paused: bool, // Only meaningful for deadloop
    #[serde(default)]
    pub stop_requested: bool, // Graceful stop pending (deadloop will stop after current iteration)
    #[serde(default)]
    pub prompt: Option<String>, // Custom prompt for deadloop
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_iteration_interval_minutes: Option<u64>, // Min time between deadloop iteration starts
    #[serde(default)]
    pub label: Option<String>, // User-facing label like "Deadloop" or "Interactive"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>, // Optional model/backend override (e.g., "o3", "MiniMax-M2.7")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>, // Optional Claude thinking effort override (e.g., "high", "max", "ultracode").
    // `ultracode` is apas-only: it spawns claude with `--effort xhigh` and prepends
    // an `ultracode ` prefix to each user prompt envelope as a workflow trigger.
    /// Absolute path to an isolated git worktree this pane should run in.
    /// When `None`, the pane runs in the project's main working_dir as before
    /// (legacy behaviour, all panes share one tree → potential conflicts).
    /// Phase 1.1 of the swarm plan adds an opt-in path that puts each pane
    /// on its own branch+worktree so parallel work doesn't race; this field
    /// is the persistence hook for that. The worktree itself is created
    /// out-of-band (CLI subcommand / web action) — apas does not touch git
    /// just because the field is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    /// Short role label for the agent in this pane, e.g. "backend
    /// implementer" or "reviewer". When set, gets prepended to claude's
    /// system prompt at spawn so the agent self-identifies. Phase 2.1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// One-line objective the pane is currently working toward, e.g.
    /// "make the auth tests green". Surfaced in the system prompt and
    /// (later) on the pane header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// Free-form additional context appended to the system prompt
    /// (project conventions, constraints, prior decisions). Long-ish
    /// is fine — claude's context window is large.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backstory: Option<String>,
    /// Phase 3.2a: per-pane policy for the editable plan checkpoint.
    /// Default is `Never` (today's behaviour) so existing panes keep
    /// running without prompts.
    #[serde(default)]
    pub plan_review_mode: PlanReviewMode,
    /// v3.2 — worker mode. `false` (default) = **autonomous**: the Tech
    /// Lead may delegate to this pane via `.apas-team.jsonl`. `true` =
    /// **manual**: the worker only takes user chat; the Tech Lead should
    /// skip it when picking a delegation target. Persisted to `.apas`.
    #[serde(default)]
    pub manual_mode: bool,
    /// v3.5 — managed vs. unmanaged. `true` = this pane is part of the
    /// project team, usually created by the Overview Start team role
    /// slots or by accepted worker suggestions / manual managed-worker
    /// flows. Such panes show up on the Overview Pane Grid and the Tech
    /// Lead may consider them for delegation. `false` (default for
    /// backward compat + the TabBar `+` button) = side chat / experiment;
    /// not part of the team queue and never a Tech Lead delegation target.
    #[serde(default)]
    pub managed: bool,
}

/// Legacy pane_id constants
pub const PANE_ID_DEADLOOP: u32 = 1;
pub const PANE_ID_INTERACTIVE: u32 = 2;

impl PaneConfig {
    /// Create default pane configs for a new project (Claude interactive only)
    pub fn defaults() -> Vec<PaneConfig> {
        vec![PaneConfig {
            pane_id: PANE_ID_INTERACTIVE,
            provider: Provider::Claude,
            mode: PaneMode::Interactive,
            kind: PaneKind::Agent,
            session_id: Uuid::new_v4(),
            is_paused: false,
            stop_requested: false,
            prompt: None,
            min_iteration_interval_minutes: None,
            label: Some("Interactive".to_string()),
            model: None,
            effort: None,
            worktree_path: None,
            role: None,
            goal: None,
            backstory: None,
            plan_review_mode: PlanReviewMode::default(),
            manual_mode: false,
            managed: false,
        }]
    }

    /// Map legacy PaneType to numeric pane_id
    pub fn pane_id_from_legacy(pane_type: &PaneType) -> u32 {
        match pane_type {
            PaneType::Deadloop => PANE_ID_DEADLOOP,
            PaneType::Interactive => PANE_ID_INTERACTIVE,
        }
    }

    /// Map numeric pane_id back to legacy PaneType (if applicable)
    pub fn legacy_from_pane_id(pane_id: u32) -> Option<PaneType> {
        match pane_id {
            PANE_ID_DEADLOOP => Some(PaneType::Deadloop),
            PANE_ID_INTERACTIVE => Some(PaneType::Interactive),
            _ => None,
        }
    }
}

/// Type of output content
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OutputType {
    #[default]
    Text,
    Code {
        language: Option<String>,
    },
    ToolUse {
        tool: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool: String,
        success: bool,
    },
    ApprovalRequest {
        tool_call_id: String,
        tool: String,
        description: String,
    },
    System,
    Error,
}

/// Session status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// Waiting for CLI client to connect
    Pending,
    /// CLI client connected, session active
    Connected,
    /// CLI client disconnected
    Disconnected,
    /// Session ended
    Ended,
}

/// Information about a CLI client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliClientInfo {
    pub id: Uuid,
    pub name: Option<String>,
    pub status: CliClientStatus,
    pub last_seen: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Active session ID if the CLI has a local session running
    pub active_session: Option<Uuid>,
}

/// CLI client status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CliClientStatus {
    Online,
    Offline,
    Busy,
}

/// Usage limit information for a time window (5-hour or 7-day)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsageLimitWindow {
    /// Utilization as a fraction (0.0 to 1.0+)
    pub utilization: f64,
    /// When the limit resets (ISO 8601 timestamp)
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "reset_at",
        alias = "resetAt",
        alias = "resetsAt"
    )]
    pub resets_at: Option<String>,
}

/// Usage limits from the provider API/logs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsageLimits {
    /// 5-hour rolling window usage
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "fiveHour")]
    pub five_hour: Option<UsageLimitWindow>,
    /// 7-day (weekly) rolling window usage
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "sevenDay")]
    pub seven_day: Option<UsageLimitWindow>,
    /// When the usage was last fetched (ISO 8601 timestamp)
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "fetchedAt")]
    pub fetched_at: Option<String>,
}

// ============================================================================
// Claude Stream-JSON Message Types
// These match the output format of `claude --output-format stream-json`
// ============================================================================

/// Top-level message from Claude CLI stream-json output
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClaudeStreamMessage {
    /// System initialization message
    System {
        subtype: String,
        session_id: String,
        #[serde(default)]
        tools: Vec<String>,
        #[serde(default)]
        model: String,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(flatten)]
        extra: serde_json::Value,
    },
    /// Assistant (Claude) message with content blocks
    Assistant {
        message: ClaudeAssistantMessage,
        session_id: String,
        #[serde(flatten)]
        extra: serde_json::Value,
    },
    /// User message (typically tool results)
    User {
        message: ClaudeUserMessage,
        session_id: String,
        #[serde(default)]
        tool_use_result: Option<serde_json::Value>,
        #[serde(flatten)]
        extra: serde_json::Value,
    },
    /// Final result message
    Result {
        subtype: String,
        #[serde(default)]
        result: String,
        #[serde(default)]
        total_cost_usd: f64,
        #[serde(default)]
        duration_ms: u64,
        session_id: String,
        #[serde(default)]
        is_error: bool,
        #[serde(flatten)]
        extra: serde_json::Value,
    },
}

/// Claude assistant message structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeAssistantMessage {
    pub content: Vec<ClaudeContentBlock>,
    #[serde(default)]
    pub model: String,
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

/// Claude user message structure (for tool results)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeUserMessage {
    pub content: Vec<ClaudeContentBlock>,
    #[serde(default)]
    pub role: String,
}

/// Content block types in Claude messages
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClaudeContentBlock {
    /// Text content from Claude
    Text { text: String },
    /// Tool use request from Claude
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// Tool result (in user messages)
    /// Note: Claude CLI can send `content` as either a string or an array of
    /// content parts. We use a custom deserializer to handle both.
    ToolResult {
        tool_use_id: String,
        #[serde(deserialize_with = "deserialize_tool_result_content")]
        content: String,
        #[serde(default)]
        is_error: bool,
    },
}

/// Deserialize tool_result content which can be either a string or an array of
/// content parts (e.g. `[{"type":"text","text":"..."}]`).
fn deserialize_tool_result_content<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Array(arr) => {
            // Extract text from content parts like [{"type":"text","text":"..."}]
            let texts: Vec<String> = arr
                .iter()
                .filter_map(|item| item.get("text").and_then(|t| t.as_str()).map(String::from))
                .collect();
            if texts.is_empty() {
                // Fallback: serialize the array as JSON string
                Ok(serde_json::to_string(&serde_json::Value::Array(arr)).unwrap_or_default())
            } else {
                Ok(texts.join("\n"))
            }
        }
        serde_json::Value::Null => Ok(String::new()),
        other => Ok(other.to_string()),
    }
}

// ============================================================================
// Codex Stream-JSON Message Types
// These match the output format of `codex exec --json`
// ============================================================================

/// Top-level message from Codex CLI JSONL output
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CodexStreamMessage {
    /// Thread started — contains the session/thread ID
    #[serde(rename = "thread.started")]
    ThreadStarted { thread_id: String },
    /// Turn started
    #[serde(rename = "turn.started")]
    TurnStarted {},
    /// An item has been completed (message, tool use, tool result, reasoning)
    #[serde(rename = "item.completed")]
    ItemCompleted { item: CodexItem },
    /// Turn completed with usage info
    #[serde(rename = "turn.completed")]
    TurnCompleted {
        #[serde(default)]
        usage: Option<CodexUsage>,
    },
    /// Error message
    #[serde(rename = "error")]
    Error { message: String },
    /// Turn failed with error
    #[serde(rename = "turn.failed")]
    TurnFailed {
        #[serde(default)]
        error: Option<CodexErrorInfo>,
    },
}

/// A completed item from Codex
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexItem {
    pub id: String,
    /// Item type: "reasoning", "agent_message", "tool_use", "tool_result", etc.
    #[serde(rename = "type")]
    pub item_type: String,
    #[serde(default)]
    pub text: Option<String>,
    // tool_use fields
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    // tool_result fields
    #[serde(default)]
    pub output: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

/// Usage information from Codex
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

/// Error info from Codex turn.failed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexErrorInfo {
    #[serde(default)]
    pub message: Option<String>,
}

/// Convert a Codex stream message to a Claude stream message for uniform handling.
/// Returns None for messages that don't map (e.g., thread.started, turn.started).
pub fn convert_codex_to_claude(
    msg: &CodexStreamMessage,
    session_id_str: &str,
) -> Option<ClaudeStreamMessage> {
    match msg {
        CodexStreamMessage::ItemCompleted { item } => {
            match item.item_type.as_str() {
                "agent_message" => {
                    let text = item.text.clone().unwrap_or_default();
                    Some(ClaudeStreamMessage::Assistant {
                        message: ClaudeAssistantMessage {
                            content: vec![ClaudeContentBlock::Text { text }],
                            model: "codex".to_string(),
                            extra: serde_json::Value::Null,
                        },
                        session_id: session_id_str.to_string(),
                        extra: serde_json::Value::Null,
                    })
                }
                "reasoning" => {
                    let text = format!("[Reasoning] {}", item.text.as_deref().unwrap_or(""));
                    Some(ClaudeStreamMessage::Assistant {
                        message: ClaudeAssistantMessage {
                            content: vec![ClaudeContentBlock::Text { text }],
                            model: "codex".to_string(),
                            extra: serde_json::Value::Null,
                        },
                        session_id: session_id_str.to_string(),
                        extra: serde_json::Value::Null,
                    })
                }
                "tool_use" | "function_call" => {
                    let name = item.name.clone().unwrap_or_else(|| "unknown".to_string());
                    let input = item.input.clone().unwrap_or(serde_json::Value::Null);
                    Some(ClaudeStreamMessage::Assistant {
                        message: ClaudeAssistantMessage {
                            content: vec![ClaudeContentBlock::ToolUse {
                                id: item.id.clone(),
                                name,
                                input,
                            }],
                            model: "codex".to_string(),
                            extra: serde_json::Value::Null,
                        },
                        session_id: session_id_str.to_string(),
                        extra: serde_json::Value::Null,
                    })
                }
                "tool_result" | "function_call_output" => {
                    let content = item
                        .output
                        .clone()
                        .or_else(|| item.text.clone())
                        .unwrap_or_default();
                    Some(ClaudeStreamMessage::User {
                        message: ClaudeUserMessage {
                            content: vec![ClaudeContentBlock::ToolResult {
                                tool_use_id: item.id.clone(),
                                content,
                                is_error: false,
                            }],
                            role: "user".to_string(),
                        },
                        session_id: session_id_str.to_string(),
                        tool_use_result: None,
                        extra: serde_json::Value::Null,
                    })
                }
                _ => {
                    // Unknown item type — render as text if it has text
                    if let Some(text) = &item.text {
                        Some(ClaudeStreamMessage::Assistant {
                            message: ClaudeAssistantMessage {
                                content: vec![ClaudeContentBlock::Text { text: text.clone() }],
                                model: "codex".to_string(),
                                extra: serde_json::Value::Null,
                            },
                            session_id: session_id_str.to_string(),
                            extra: serde_json::Value::Null,
                        })
                    } else {
                        None
                    }
                }
            }
        }
        CodexStreamMessage::TurnCompleted { usage } => {
            let (input_tokens, output_tokens, cached_input_tokens) = usage
                .as_ref()
                .map(|u| (u.input_tokens, u.output_tokens, u.cached_input_tokens))
                .unwrap_or((0, 0, 0));
            Some(ClaudeStreamMessage::Result {
                subtype: "success".to_string(),
                result: format!(
                    "Turn completed ({} in, {} out tokens)",
                    input_tokens, output_tokens
                ),
                total_cost_usd: 0.0,
                duration_ms: 0,
                session_id: session_id_str.to_string(),
                is_error: false,
                // Preserve token usage in the same shape the server reads for
                // Claude panes so Codex panes also accumulate token stats.
                // Codex's `input_tokens` is the FULL prompt size and
                // `cached_input_tokens` is a SUBSET of it, whereas Anthropic
                // reports `input_tokens` disjoint from cache reads. Subtract the
                // cached portion so input + cache_read don't double-count (the
                // web sums all token fields for the "Total" column). Codex
                // reports no cache-creation tokens and no cost.
                extra: serde_json::json!({
                    "usage": {
                        "input_tokens": input_tokens.saturating_sub(cached_input_tokens),
                        "output_tokens": output_tokens,
                        "cache_read_input_tokens": cached_input_tokens,
                    }
                }),
            })
        }
        CodexStreamMessage::Error { message } => Some(ClaudeStreamMessage::Result {
            subtype: "error".to_string(),
            result: message.clone(),
            total_cost_usd: 0.0,
            duration_ms: 0,
            session_id: session_id_str.to_string(),
            is_error: true,
            extra: serde_json::Value::Null,
        }),
        CodexStreamMessage::TurnFailed { error } => {
            let msg = error
                .as_ref()
                .and_then(|e| e.message.clone())
                .unwrap_or_else(|| "Turn failed".to_string());
            Some(ClaudeStreamMessage::Result {
                subtype: "error".to_string(),
                result: msg,
                total_cost_usd: 0.0,
                duration_ms: 0,
                session_id: session_id_str.to_string(),
                is_error: true,
                extra: serde_json::Value::Null,
            })
        }
        _ => None, // ThreadStarted, TurnStarted — no display needed
    }
}

// ============================================================================
// Helper implementations
// ============================================================================

impl CliToServer {
    pub fn output(session_id: Uuid, data: impl Into<String>) -> Self {
        Self::Output {
            session_id,
            data: data.into(),
            output_type: OutputType::Text,
            pane_type: None,
            pane_id: None,
        }
    }

    pub fn output_with_type(
        session_id: Uuid,
        data: impl Into<String>,
        output_type: OutputType,
    ) -> Self {
        Self::Output {
            session_id,
            data: data.into(),
            output_type,
            pane_type: None,
            pane_id: None,
        }
    }

    pub fn output_with_pane(
        session_id: Uuid,
        data: impl Into<String>,
        pane_type: PaneType,
    ) -> Self {
        Self::Output {
            session_id,
            data: data.into(),
            output_type: OutputType::Text,
            pane_type: Some(pane_type),
            pane_id: Some(PaneConfig::pane_id_from_legacy(&pane_type)),
        }
    }
}

impl ServerToWeb {
    pub fn output(content: impl Into<String>) -> Self {
        Self::Output {
            content: content.into(),
            output_type: OutputType::Text,
            pane_type: None,
            pane_id: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reboot_pane_docs_do_not_describe_fresh_sessions() {
        let source = include_str!("messages.rs");
        let lines: Vec<&str> = source.lines().collect();
        let target = ["Reboot", "Pane"].concat();
        let forbidden = [
            ["fresh agent", " session"].concat(),
            ["fresh", " session"].concat(),
        ];

        for (idx, line) in lines.iter().enumerate() {
            if !line.contains(&target) {
                continue;
            }
            let start = idx.saturating_sub(4);
            let end = (idx + 5).min(lines.len());
            let window = lines[start..end].join("\n").to_ascii_lowercase();

            for phrase in &forbidden {
                assert!(
                    !window.contains(phrase),
                    "{target} docs near line {} mention `{}`:\n{}",
                    idx + 1,
                    phrase,
                    window
                );
            }
        }
    }

    #[test]
    fn test_cli_to_server_register_serialization() {
        let msg = CliToServer::Register {
            token: "test-token".to_string(),
            version: Some("1.0.0".to_string()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"register\""));
        assert!(json.contains("\"token\":\"test-token\""));

        let deserialized: CliToServer = serde_json::from_str(&json).unwrap();
        match deserialized {
            CliToServer::Register { token, .. } => assert_eq!(token, "test-token"),
            _ => panic!("Expected Register variant"),
        }
    }

    #[test]
    fn test_cli_to_server_session_start_serialization() {
        let session_id = Uuid::new_v4();
        let msg = CliToServer::SessionStart {
            session_id,
            project_id: None,
            working_dir: Some("/home/user/project".to_string()),
            hostname: None,
            git_remote: Some("github.com/shuaimu/apas".to_string()),
            git_remote_url: Some("git@github.com:shuaimu/apas.git".to_string()),
            pane_type: None,
            panes: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"session_start\""));
        assert!(json.contains("\"git_remote_url\":\"git@github.com:shuaimu/apas.git\""));
        assert!(json.contains(&session_id.to_string()));
        assert!(json.contains("\"git_remote\":\"github.com/shuaimu/apas\""));

        let deserialized: CliToServer = serde_json::from_str(&json).unwrap();
        match deserialized {
            CliToServer::SessionStart {
                session_id: sid,
                working_dir,
                git_remote,
                ..
            } => {
                assert_eq!(sid, session_id);
                assert_eq!(working_dir, Some("/home/user/project".to_string()));
                assert_eq!(git_remote, Some("github.com/shuaimu/apas".to_string()));
            }
            _ => panic!("Expected SessionStart variant"),
        }
    }

    #[test]
    fn test_session_start_git_remote_backcompat_and_skip() {
        // None must be omitted from the wire (skip_serializing_if).
        let session_id = Uuid::new_v4();
        let msg = CliToServer::SessionStart {
            session_id,
            project_id: None,
            working_dir: None,
            hostname: None,
            git_remote: None,
            git_remote_url: None,
            pane_type: None,
            panes: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("git_remote"));

        // Legacy CLIs omit git_remote entirely — must still parse to None.
        let legacy = format!(
            r#"{{"type":"session_start","session_id":"{session_id}","working_dir":"/p","hostname":null}}"#
        );
        let parsed: CliToServer = serde_json::from_str(&legacy).unwrap();
        match parsed {
            CliToServer::SessionStart { git_remote, .. } => assert_eq!(git_remote, None),
            _ => panic!("Expected SessionStart variant"),
        }
    }

    #[test]
    fn test_project_usage_stats_serialization() {
        let stats = ProjectUsageStats {
            panes: vec![PaneUsageStats {
                pane_id: 178,
                lifetime: UsageCounters {
                    prompts: 3,
                    responses: 2,
                    input_tokens: 100,
                    output_tokens: 40,
                    cache_read_tokens: 10,
                    cache_creation_tokens: 5,
                    cost_usd: 0.25,
                },
                today: UsageCounters {
                    prompts: 1,
                    ..Default::default()
                },
                last_active: Some("2026-06-29T00:00:00Z".to_string()),
                ..Default::default()
            }],
            lifetime: UsageCounters {
                input_tokens: 100,
                ..Default::default()
            },
            ..Default::default()
        };
        let msg = ServerToWeb::ProjectUsageStats {
            session_id: Uuid::new_v4(),
            stats,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"project_usage_stats\""));
        // snake_case wire keys must match what the web store reads.
        assert!(json.contains("\"input_tokens\":100"));
        assert!(json.contains("\"cache_read_tokens\":10"));
        assert!(json.contains("\"cost_usd\":0.25"));
        assert!(json.contains("\"last_7d\""));

        let round: ServerToWeb = serde_json::from_str(&json).unwrap();
        match round {
            ServerToWeb::ProjectUsageStats { stats, .. } => {
                assert_eq!(stats.panes.len(), 1);
                assert_eq!(stats.panes[0].pane_id, 178);
                assert_eq!(stats.panes[0].lifetime.input_tokens, 100);
            }
            _ => panic!("expected ProjectUsageStats"),
        }
    }

    #[test]
    fn test_codex_turn_completed_preserves_token_usage() {
        let msg = CodexStreamMessage::TurnCompleted {
            usage: Some(CodexUsage {
                input_tokens: 1234,
                cached_input_tokens: 56,
                output_tokens: 78,
            }),
        };
        let converted = convert_codex_to_claude(&msg, "sess").expect("maps to Result");
        match converted {
            ClaudeStreamMessage::Result { extra, .. } => {
                let usage = extra.get("usage").expect("usage present in extra");
                // input_tokens is reported disjoint from the cached subset
                // (1234 full - 56 cached) to match Anthropic's token model.
                assert_eq!(usage.get("input_tokens").and_then(|v| v.as_u64()), Some(1178));
                assert_eq!(usage.get("output_tokens").and_then(|v| v.as_u64()), Some(78));
                assert_eq!(
                    usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()),
                    Some(56)
                );
            }
            _ => panic!("expected Result"),
        }
    }

    #[test]
    fn test_web_input_client_msg_id_roundtrip_and_backcompat() {
        // Old clients omit client_msg_id entirely — must still parse.
        let legacy = r#"{"type":"input","text":"hi","pane_id":205}"#;
        let parsed: WebToServer = serde_json::from_str(legacy).unwrap();
        match parsed {
            WebToServer::Input { client_msg_id, text, .. } => {
                assert_eq!(client_msg_id, None);
                assert_eq!(text, "hi");
            }
            _ => panic!("Expected Input variant"),
        }

        let with_id = r#"{"type":"input","text":"hi","pane_id":205,"client_msg_id":"abc123"}"#;
        let parsed: WebToServer = serde_json::from_str(with_id).unwrap();
        match parsed {
            WebToServer::Input { client_msg_id, .. } => {
                assert_eq!(client_msg_id.as_deref(), Some("abc123"));
            }
            _ => panic!("Expected Input variant"),
        }

        // Echo carries the id back; None must not serialize the key at all
        // (older web bundles ignore unknown keys, but keep the wire tidy).
        let echo = ServerToWeb::UserInput {
            session_id: Uuid::new_v4(),
            text: "hi".to_string(),
            pane_type: None,
            pane_id: Some(205),
            created_at: None,
            client_msg_id: Some("abc123".to_string()),
        };
        let json = serde_json::to_string(&echo).unwrap();
        assert!(json.contains("\"client_msg_id\":\"abc123\""));
        let no_id = ServerToWeb::UserInput {
            session_id: Uuid::new_v4(),
            text: "hi".to_string(),
            pane_type: None,
            pane_id: Some(205),
            created_at: None,
            client_msg_id: None,
        };
        assert!(!serde_json::to_string(&no_id).unwrap().contains("client_msg_id"));
    }

    #[test]
    fn test_cli_to_server_output_helper() {
        let session_id = Uuid::new_v4();
        let msg = CliToServer::output(session_id, "Hello, world!");
        match msg {
            CliToServer::Output {
                session_id: sid,
                data,
                output_type,
                pane_type,
                ..
            } => {
                assert_eq!(sid, session_id);
                assert_eq!(data, "Hello, world!");
                assert_eq!(output_type, OutputType::Text);
                assert_eq!(pane_type, None);
            }
            _ => panic!("Expected Output variant"),
        }
    }

    #[test]
    fn test_server_to_cli_serialization() {
        let cli_id = Uuid::new_v4();
        let msg = ServerToCli::Registered { cli_id };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"registered\""));

        let deserialized: ServerToCli = serde_json::from_str(&json).unwrap();
        match deserialized {
            ServerToCli::Registered { cli_id: cid } => assert_eq!(cid, cli_id),
            _ => panic!("Expected Registered variant"),
        }

        let session_id = Uuid::new_v4();
        let msg = ServerToCli::RebootPane {
            session_id,
            pane_id: 42,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["type"], "reboot_pane");
        assert_eq!(value["session_id"], session_id.to_string());
        assert_eq!(value["pane_id"], 42);

        let deserialized: ServerToCli = serde_json::from_str(&json).unwrap();
        match deserialized {
            ServerToCli::RebootPane {
                session_id: sid,
                pane_id,
            } => {
                assert_eq!(sid, session_id);
                assert_eq!(pane_id, 42);
            }
            _ => panic!("Expected RebootPane variant"),
        }
    }

    #[test]
    fn test_web_to_server_serialization() {
        let msg = WebToServer::Authenticate {
            token: "jwt-token".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"authenticate\""));

        let msg = WebToServer::ListCliClients;
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"list_cli_clients\""));

        let session_id = Uuid::new_v4();
        let msg = WebToServer::RebootPane {
            session_id: Some(session_id),
            pane_id: 42,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["type"], "reboot_pane");
        assert_eq!(value["session_id"], session_id.to_string());
        assert_eq!(value["pane_id"], 42);

        let deserialized: WebToServer = serde_json::from_str(&json).unwrap();
        match deserialized {
            WebToServer::RebootPane {
                session_id: sid,
                pane_id,
            } => {
                assert_eq!(sid, Some(session_id));
                assert_eq!(pane_id, 42);
            }
            _ => panic!("Expected RebootPane variant"),
        }

        let msg = WebToServer::RebootPane {
            session_id: None,
            pane_id: 7,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["type"], "reboot_pane");
        assert!(value.get("session_id").is_none());
        assert_eq!(value["pane_id"], 7);
    }

    #[test]
    fn test_server_to_web_helpers() {
        let msg = ServerToWeb::output("Test output");
        match msg {
            ServerToWeb::Output {
                content,
                output_type,
                pane_type,
                ..
            } => {
                assert_eq!(content, "Test output");
                assert_eq!(output_type, OutputType::Text);
                assert_eq!(pane_type, None);
            }
            _ => panic!("Expected Output variant"),
        }

        let msg = ServerToWeb::error("Something went wrong");
        match msg {
            ServerToWeb::Error { message } => {
                assert_eq!(message, "Something went wrong");
            }
            _ => panic!("Expected Error variant"),
        }
    }

    #[test]
    fn test_output_type_default() {
        let output_type = OutputType::default();
        assert_eq!(output_type, OutputType::Text);
    }

    #[test]
    fn test_default_pane_configs_are_interactive_only() {
        let defaults = PaneConfig::defaults();
        assert_eq!(defaults.len(), 1);

        let pane = &defaults[0];
        assert_eq!(pane.pane_id, PANE_ID_INTERACTIVE);
        assert_eq!(pane.provider, Provider::Claude);
        assert_eq!(pane.mode, PaneMode::Interactive);
        assert_eq!(pane.label.as_deref(), Some("Interactive"));
        assert!(!pane.is_paused);
        assert!(pane.prompt.is_none());
        assert!(pane.model.is_none());
    }

    #[test]
    fn test_deepseek_provider_serializes_as_snake_case() {
        let json = serde_json::to_string(&Provider::Deepseek).unwrap();
        assert_eq!(json, "\"deepseek\"");

        let provider: Provider = serde_json::from_str("\"deepseek\"").unwrap();
        assert_eq!(provider, Provider::Deepseek);

        let pane_json = serde_json::json!({
            "pane_id": 7,
            "provider": "deepseek",
            "mode": "interactive",
            "session_id": Uuid::new_v4(),
        });
        let pane: PaneConfig = serde_json::from_value(pane_json).unwrap();
        assert_eq!(pane.provider, Provider::Deepseek);
    }

    #[test]
    fn test_machine_info_serializes_deepseek_backend() {
        let machine = MachineInfo {
            machine_id: Uuid::new_v4(),
            hostname: "devbox".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            daemon_version: Some("26.06.1".to_string()),
            minimax_backend: None,
            glm_backend: None,
            deepseek_backend: Some(DeepseekBackendInfo {
                api_base_url: Some("https://api.deepseek.com/anthropic".to_string()),
                api_key: None,
                api_key_configured: true,
            }),
            last_seen: None,
        };

        let json = serde_json::to_string(&machine).unwrap();
        assert!(json.contains("\"deepseek_backend\""));
        assert!(json.contains("\"api_base_url\":\"https://api.deepseek.com/anthropic\""));
        assert!(json.contains("\"api_key_configured\":true"));

        let decoded: MachineInfo = serde_json::from_str(&json).unwrap();
        let backend = decoded.deepseek_backend.expect("deepseek backend");
        assert_eq!(
            backend.api_base_url.as_deref(),
            Some("https://api.deepseek.com/anthropic")
        );
        assert!(backend.api_key_configured);
        assert_eq!(backend.api_key, None);
    }

    #[test]
    fn test_output_type_serialization() {
        let json = serde_json::to_string(&OutputType::Text).unwrap();
        assert_eq!(json, "\"text\"");

        let code = OutputType::Code {
            language: Some("rust".to_string()),
        };
        let json = serde_json::to_string(&code).unwrap();
        assert!(json.contains("\"code\""));
        assert!(json.contains("\"language\":\"rust\""));

        let tool_use = OutputType::ToolUse {
            tool: "read_file".to_string(),
            input: serde_json::json!({"path": "/tmp/test.txt"}),
        };
        let json = serde_json::to_string(&tool_use).unwrap();
        assert!(json.contains("\"tool_use\""));
        assert!(json.contains("\"tool\":\"read_file\""));
    }

    #[test]
    fn test_session_status_serialization() {
        let status = SessionStatus::Connected;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"connected\"");

        let status: SessionStatus = serde_json::from_str("\"pending\"").unwrap();
        assert_eq!(status, SessionStatus::Pending);
    }

    #[test]
    fn test_cli_client_status_serialization() {
        let status = CliClientStatus::Online;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"online\"");

        let status: CliClientStatus = serde_json::from_str("\"busy\"").unwrap();
        assert_eq!(status, CliClientStatus::Busy);
    }

    #[test]
    fn test_cli_client_info_serialization() {
        let info = CliClientInfo {
            id: Uuid::new_v4(),
            name: Some("my-laptop".to_string()),
            status: CliClientStatus::Online,
            last_seen: Some(chrono::Utc::now()),
            version: Some("26.04.123".to_string()),
            active_session: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"name\":\"my-laptop\""));
        assert!(json.contains("\"status\":\"online\""));
        assert!(json.contains("\"version\":\"26.04.123\""));

        let deserialized: CliClientInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, Some("my-laptop".to_string()));
        assert_eq!(deserialized.status, CliClientStatus::Online);
        assert_eq!(deserialized.version, Some("26.04.123".to_string()));
    }

    #[test]
    fn test_attach_session_message() {
        let session_id = Uuid::new_v4();
        let msg = WebToServer::AttachSession { session_id };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"attach_session\""));
        assert!(json.contains(&session_id.to_string()));
    }

    #[test]
    fn test_claude_stream_message_system() {
        let json = r#"{"type":"system","subtype":"init","session_id":"abc-123","tools":["Read","Edit"],"model":"claude-opus","cwd":"/home/user"}"#;
        let msg: ClaudeStreamMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClaudeStreamMessage::System {
                subtype,
                tools,
                model,
                ..
            } => {
                assert_eq!(subtype, "init");
                assert_eq!(tools, vec!["Read", "Edit"]);
                assert_eq!(model, "claude-opus");
            }
            _ => panic!("Expected System variant"),
        }
    }

    #[test]
    fn test_claude_stream_message_assistant_text() {
        let json = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello world"}],"model":"claude"},"session_id":"abc-123"}"#;
        let msg: ClaudeStreamMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClaudeStreamMessage::Assistant { message, .. } => {
                assert_eq!(message.content.len(), 1);
                match &message.content[0] {
                    ClaudeContentBlock::Text { text } => assert_eq!(text, "Hello world"),
                    _ => panic!("Expected Text content block"),
                }
            }
            _ => panic!("Expected Assistant variant"),
        }
    }

    #[test]
    fn test_claude_stream_message_assistant_tool_use() {
        let json = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tool-1","name":"Read","input":{"file_path":"/tmp/test.txt"}}],"model":"claude"},"session_id":"abc-123"}"#;
        let msg: ClaudeStreamMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClaudeStreamMessage::Assistant { message, .. } => match &message.content[0] {
                ClaudeContentBlock::ToolUse { id, name, input } => {
                    assert_eq!(id, "tool-1");
                    assert_eq!(name, "Read");
                    assert_eq!(input["file_path"], "/tmp/test.txt");
                }
                _ => panic!("Expected ToolUse content block"),
            },
            _ => panic!("Expected Assistant variant"),
        }
    }

    #[test]
    fn test_claude_stream_message_result() {
        let json = r#"{"type":"result","subtype":"success","result":"Done","total_cost_usd":0.05,"duration_ms":1000,"session_id":"abc-123","is_error":false}"#;
        let msg: ClaudeStreamMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClaudeStreamMessage::Result {
                subtype,
                result,
                total_cost_usd,
                is_error,
                ..
            } => {
                assert_eq!(subtype, "success");
                assert_eq!(result, "Done");
                assert!((total_cost_usd - 0.05).abs() < 0.001);
                assert!(!is_error);
            }
            _ => panic!("Expected Result variant"),
        }
    }

    #[test]
    fn test_cli_to_server_stream_message() {
        let session_id = Uuid::new_v4();
        let stream_msg = ClaudeStreamMessage::Result {
            subtype: "success".to_string(),
            result: "Done".to_string(),
            total_cost_usd: 0.01,
            duration_ms: 500,
            session_id: "test".to_string(),
            is_error: false,
            extra: serde_json::Value::Null,
        };
        let msg = CliToServer::StreamMessage {
            session_id,
            message: stream_msg,
            pane_type: None,
            pane_id: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"stream_message\""));
        assert!(json.contains(&session_id.to_string()));
    }

    /// Every `.apas` written before terminal panes existed lacks `kind`.
    /// Those panes must keep loading as `Agent` — a wrong default here
    /// would try to spawn a pty for every restored pane in the fleet.
    #[test]
    fn pane_config_without_kind_deserializes_as_agent() {
        let legacy = r#"{
            "pane_id": 440,
            "provider": "claude",
            "mode": "interactive",
            "session_id": "52443f74-5819-4502-83fe-db530fe70feb",
            "label": "Tab 440"
        }"#;
        let pane: PaneConfig = serde_json::from_str(legacy).unwrap();
        assert_eq!(pane.kind, PaneKind::Agent);
        assert!(!pane.kind.is_terminal());
    }

    #[test]
    fn terminal_pane_config_round_trips() {
        let legacy = r#"{
            "pane_id": 7,
            "provider": "codex",
            "mode": "interactive",
            "kind": "terminal",
            "session_id": "52443f74-5819-4502-83fe-db530fe70feb"
        }"#;
        let pane: PaneConfig = serde_json::from_str(legacy).unwrap();
        assert_eq!(pane.kind, PaneKind::Terminal);

        let json = serde_json::to_string(&pane).unwrap();
        assert!(json.contains("\"kind\":\"terminal\""));
        let back: PaneConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind, PaneKind::Terminal);
    }

    #[test]
    fn terminal_messages_tag_and_carry_session_id() {
        let session_id = Uuid::new_v4();

        let out = CliToServer::TerminalOutput {
            session_id,
            pane_id: 7,
            instance_id: None,
            data_b64: "aGVsbG8=".to_string(),
            seq: 3,
        };
        let json = serde_json::to_string(&out).unwrap();
        assert!(json.contains("\"type\":\"terminal_output\""));
        assert!(json.contains(&session_id.to_string()));

        // Every web-originated pane control must carry session_id so the
        // server can resolve the right target; a pane_id alone misroutes
        // when several sessions are attached (mobile hits this first).
        for msg in [
            WebToServer::TerminalInput {
                session_id,
                pane_id: 7,
                data_b64: "bHM=".to_string(),
            },
            WebToServer::TerminalResize {
                session_id,
                pane_id: 7,
                cols: 120,
                rows: 40,
            },
            WebToServer::TerminalAttach {
                session_id,
                pane_id: 7,
            },
        ] {
            let json = serde_json::to_string(&msg).unwrap();
            assert!(
                json.contains(&session_id.to_string()),
                "terminal control dropped session_id: {json}"
            );
        }
    }

    #[test]
    fn legacy_terminal_messages_default_lifecycle_and_instance() {
        let output = r#"{
            "type": "terminal_output",
            "session_id": "52443f74-5819-4502-83fe-db530fe70feb",
            "pane_id": 7,
            "data_b64": "aGVsbG8=",
            "seq": 2
        }"#;
        match serde_json::from_str::<CliToServer>(output).unwrap() {
            CliToServer::TerminalOutput { instance_id, .. } => assert!(instance_id.is_none()),
            other => panic!("unexpected variant: {other:?}"),
        }

        let json = r#"{
            "type": "terminal_snapshot",
            "session_id": "52443f74-5819-4502-83fe-db530fe70feb",
            "pane_id": 7,
            "data_b64": "",
            "seq": 0
        }"#;
        match serde_json::from_str::<ServerToWeb>(json).unwrap() {
            ServerToWeb::TerminalSnapshot {
                instance_id,
                truncated,
                lifecycle,
                status,
                ..
            } => {
                assert!(instance_id.is_none());
                assert!(!truncated);
                assert_eq!(lifecycle, TerminalLifecycle::Unknown);
                assert!(status.is_none());
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn terminal_state_messages_round_trip_lifecycle_metadata() {
        let session_id = Uuid::new_v4();
        let instance_id = Uuid::new_v4();
        let cli = CliToServer::TerminalState {
            session_id,
            pane_id: 11,
            instance_id: Some(instance_id),
            lifecycle: TerminalLifecycle::Exited,
            status: Some("exited with status 1".to_string()),
        };
        let json = serde_json::to_string(&cli).unwrap();
        match serde_json::from_str::<CliToServer>(&json).unwrap() {
            CliToServer::TerminalState {
                session_id: got_session,
                pane_id,
                instance_id: got_instance,
                lifecycle,
                status,
            } => {
                assert_eq!(got_session, session_id);
                assert_eq!(pane_id, 11);
                assert_eq!(got_instance, Some(instance_id));
                assert_eq!(lifecycle, TerminalLifecycle::Exited);
                assert_eq!(status.as_deref(), Some("exited with status 1"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }

        let web = ServerToWeb::TerminalState {
            session_id,
            pane_id: 11,
            instance_id: Some(instance_id),
            lifecycle: TerminalLifecycle::Disconnected,
            status: None,
        };
        let json = serde_json::to_string(&web).unwrap();
        match serde_json::from_str::<ServerToWeb>(&json).unwrap() {
            ServerToWeb::TerminalState {
                instance_id: got_instance,
                lifecycle,
                ..
            } => {
                assert_eq!(got_instance, Some(instance_id));
                assert_eq!(lifecycle, TerminalLifecycle::Disconnected);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    // --- tab-type policy -------------------------------------------------

    /// The web's catalog, so the two cannot drift apart unnoticed. Same trick
    /// the CLI already uses for `providerOptions.ts`.
    const WEB_TAB_TYPES_TS: &str = include_str!("../../../packages/web/src/lib/tabTypes.ts");

    #[test]
    fn tab_type_keys_are_kind_then_provider() {
        assert_eq!(tab_type_key(PaneKind::Agent, Provider::Claude), "agent:claude");
        assert_eq!(
            tab_type_key(PaneKind::Terminal, Provider::Codex),
            "terminal:codex"
        );
        // Derived from serde names, so a rename shows up here rather than
        // silently orphaning stored restrictions.
        assert_eq!(
            tab_type_key(PaneKind::Agent, Provider::CursorAgent),
            "agent:cursor-agent"
        );
    }

    #[test]
    fn an_empty_deny_list_allows_everything() {
        // The upgrade path: `.apas` files predating the field deserialize to an
        // empty Vec, which must not lock a project out of opening any tab.
        for key in all_tab_types() {
            let (kind, provider) = key.split_once(':').expect("kind:provider");
            assert!(!kind.is_empty() && !provider.is_empty());
        }
        assert!(tab_type_allowed(&[], PaneKind::Agent, Provider::Claude));
        assert!(tab_type_allowed(&[], PaneKind::Terminal, Provider::Codex));
    }

    #[test]
    fn denying_one_type_leaves_its_sibling_alone() {
        // The distinction the feature exists for: a claude agent tab and a
        // claude terminal tab are different capabilities.
        let deny = vec!["terminal:claude".to_string()];
        assert!(!tab_type_allowed(&deny, PaneKind::Terminal, Provider::Claude));
        assert!(tab_type_allowed(&deny, PaneKind::Agent, Provider::Claude));
        assert!(tab_type_allowed(&deny, PaneKind::Terminal, Provider::Codex));
    }

    #[test]
    fn deny_entries_tolerate_whitespace_and_case() {
        let deny = vec!["  Agent:Claude  ".to_string()];
        assert!(!tab_type_allowed(&deny, PaneKind::Agent, Provider::Claude));
    }

    #[test]
    fn the_catalog_omits_providers_that_are_really_claude_models() {
        // MiniMax/GLM/DeepSeek tabs arrive as `provider: claude`, so offering
        // them as separate tab types would be a checkbox that does nothing.
        let catalog = all_tab_types();
        for absent in ["agent:minimax", "agent:glm", "agent:deepseek"] {
            assert!(
                !catalog.contains(&absent.to_string()),
                "{absent} is not separately creatable"
            );
        }
        assert_eq!(catalog.len(), 6, "catalog: {catalog:?}");
    }

    #[test]
    fn rust_and_web_tab_type_catalogs_agree() {
        for key in all_tab_types() {
            let (kind, provider) = key.split_once(':').expect("kind:provider");
            let needle = format!(r#"kind: "{kind}", provider: "{provider}""#);
            assert!(
                WEB_TAB_TYPES_TS.contains(&needle),
                "tabTypes.ts is missing {key} — the admin UI would not offer it"
            );
        }
        // And nothing extra: a type the web offers but the CLI does not know
        // would be an unenforceable checkbox.
        let web_entries = WEB_TAB_TYPES_TS.matches("{ kind: \"").count();
        assert_eq!(
            web_entries,
            all_tab_types().len(),
            "tabTypes.ts lists {web_entries} types, shared lists {}",
            all_tab_types().len()
        );
    }
}
