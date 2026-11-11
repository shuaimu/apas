use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Capability advertised by web, CLI, and daemon peers that understand the
/// server-owned effective project policy protocol.
pub const PROJECT_POLICY_CAPABILITY: &str = "project_policy_v1";
// v2 launches the provider's terminal TUI and carries the first instruction
// as an initial prompt. A v1 CLI would accept the pane but silently drop that
// instruction, so the version bump makes mixed-version rollout fail closed.
pub const MOBILE_TASK_LAUNCH_CAPABILITY: &str = "mobile_task_launch_v2";
/// The connected CLI can host OpenCode's interactive TUI, pass an initial
/// prompt with OpenCode's native flags, and mirror its transcript.
///
/// Kept separate from the generic mobile-launch capability so a rolling
/// server/web deployment cannot route an OpenCode pane to an older CLI that
/// would persist the pane but fail to launch it.
pub const OPENCODE_TERMINAL_CAPABILITY: &str = "terminal_opencode_v1";
/// The connected CLI can host Pi's interactive TUI, pin each pane to an exact
/// Pi session identity, and mirror its session transcript.
///
/// Kept separate from the generic mobile-launch capability so a rolling
/// server/web deployment cannot route a Pi pane to an older CLI that would
/// persist the pane but fail to launch it.
pub const PI_TERMINAL_CAPABILITY: &str = "terminal_pi_v1";
/// The connected CLI can host OMP's interactive TUI, give each pane its own
/// session directory, and mirror the session transcript written there.
///
/// Kept separate from the generic mobile-launch capability so a rolling
/// server/web deployment cannot route an OMP pane to an older CLI that would
/// persist the pane but fail to launch it.
pub const OMP_TERMINAL_CAPABILITY: &str = "terminal_omp_v1";
/// The CLI can deliver scoped structured answers to OMP's native question dialog.
pub const OMP_QUESTION_CAPABILITY: &str = "omp_question_answers_v1";
/// The CLI understands correlated, transport-only reconnect requests.
pub const CLI_LIFECYCLE_CAPABILITY: &str = "cli_lifecycle_v1";
/// The CLI can keep supported terminal providers alive outside the replaceable
/// project controller process and adopt them after a reboot.
pub const PERSISTENT_TERMINAL_HOST_CAPABILITY: &str = "persistent_terminal_host_v1";
pub const PANE_HOST_CLEANUP_ACK_CAPABILITY: &str = "pane_host_cleanup_ack_v1";

/// How long to let a full-screen TUI settle before sending the key that
/// submits what was just typed into it.
///
/// A terminal pane is driven by writing bytes to its pty, and a TUI classifies
/// back-to-back bytes as a *paste*. An Enter inside a paste is text, not a
/// submit, so anything written as one burst lands in the interface and then
/// sits there unsubmitted. Both surfaces that write on a person's behalf pace
/// themselves by this: typed conversation text, and the keystrokes that answer
/// an agent's question.
///
/// One constant because the two paths hit the identical failure and a value
/// this empirical should not be tuned in one place and forgotten in the other.
pub const TERMINAL_SUBMIT_SETTLE_MS: u64 = 100;
/// The daemon implements two-phase, marker-bound, credential-isolated shared
/// project provisioning. Servers must never downgrade a member request when
/// this capability is absent.
pub const SHARED_PROJECT_PROVISIONING_CAPABILITY: &str = "shared_project_provisioning_v1";
/// Existing-directory registration without cloning or starting a runtime.
pub const LOCAL_PROJECT_REGISTRATION_CAPABILITY: &str = "local_project_registration_v1";
/// The CLI/daemon can run one instance of a project per machine, every one of
/// them attached to the project's single session: it tags the panes it owns
/// with its machine id, saves only those into a shared `.apas`, and holds a
/// per-machine claim.
///
/// An instance without it sends its whole roster untagged and assumes it is
/// the project's only runtime, so the server must never attach it beside
/// another machine's instance, route a tab to it by machine, or ask its daemon
/// to clone a checkout that joins an existing project. A host configured with
/// an explicit `daemon.machine_id` must not advertise it either: that setting
/// lives in the shared `config.toml`, so every host on a shared home would
/// report the same machine.
///
/// Web and mobile clients advertise it too, in `Authenticate.capabilities`,
/// to receive [`ServerToWeb::SessionInstances`]; a server that implements
/// instances echoes it in `Authenticated.negotiated_capabilities`, so a client
/// can tell whether that frame will come. The gate is what keeps older native
/// builds connected: they validate every frame against their bundled schema
/// and drop the socket on a variant it does not list.
pub const PROJECT_INSTANCES_CAPABILITY: &str = "project_instances_v1";
/// Canonical non-working pane status used while an agent is blocked on a
/// question only the human can answer. Keep the wire value stable: older web
/// clients safely render it as an ordinary non-empty status, while newer ones
/// classify it separately from active work.
pub const PANE_STATUS_PENDING_ANSWER: &str = "Pending answer";

pub fn pane_status_is_working(status: &str) -> bool {
    status != PANE_STATUS_PENDING_ANSWER
}

/// A native OMP answer. Question identity and selections must not be flattened
/// into question text or comma-separated labels.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OmpQuestionAnswer {
    pub id: String,
    pub selected_options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_input: Option<String>,
}

fn default_true() -> bool {
    true
}

/// One published artifact in the team scratchpad (`.apas-team.jsonl`),
/// mirroring the CLI's `crate::scratchpad::TeamRecord`. Separate type
/// here so the wire shape is stable across CLI/server/web even if the
/// CLI's internal helper grows extra columns. Phase 2.2b.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct TeamRoleSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Phase 3.2a: per-pane policy for the "editable plan checkpoint"
/// feature. The streaming worker reads this at every turn to decide
/// whether to gate the first tool_use behind a user-approval card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, JsonSchema)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TerminalLifecycle {
    #[default]
    Unknown,
    Running,
    Disconnected,
    Exited,
}

/// A project-level lifecycle operation.
///
/// Only reboot is a user decision. Transport recovery used to be a second
/// variant here, driven by a `Reconnect Server` button, and was withdrawn: the
/// CLI already re-dials a lost transport on its own with bounded backoff, and a
/// request to do so has to travel over the very transport that is down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CliLifecycleOperation {
    RebootCli,
}

/// Deserialize a lifecycle operation, tolerating an operation this build no
/// longer knows.
///
/// A browser holding a bundle from before an operation was retired still sends
/// it. Without this the unknown tag fails the whole `WebToServer` decode, so one
/// stale request would take the socket down rather than being ignored — a much
/// worse outcome than dropping the request. Resolution goes through the enum's
/// own `Deserialize` so it cannot drift from `rename_all`.
fn lenient_lifecycle_operation<'de, D>(
    deserializer: D,
) -> Result<Option<CliLifecycleOperation>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::IntoDeserializer;
    let raw = String::deserialize(deserializer)?;
    let parsed: Result<CliLifecycleOperation, serde::de::value::Error> =
        CliLifecycleOperation::deserialize(raw.into_deserializer());
    Ok(parsed.ok())
}

/// Authoritative progress for a correlated lifecycle request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CliLifecyclePhase {
    Accepted,
    Preparing,
    Reconnecting,
    Handoff,
    Reconciling,
    Succeeded,
    Failed,
    TimedOut,
}

impl CliLifecyclePhase {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::TimedOut)
    }
}

/// Whether one configured pane can retain its exact live process during a
/// full CLI replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PanePreservationMode {
    LiveAdoptable,
    RestartRequiredOnCliReboot,
    StructuredPaneMayResume,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PanePreservationInfo {
    pub pane_id: u32,
    pub mode: PanePreservationMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<Uuid>,
}

/// Current lifecycle capabilities and the per-pane reboot consequence list.
/// Empty/default values make the message safe while CLI versions roll out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CliLifecycleInventory {
    #[serde(default)]
    pub persistent_terminal_hosting: bool,
    #[serde(default)]
    pub panes: Vec<PanePreservationInfo>,
}

/// Host-owned terminal continuity metadata reported during reconciliation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TerminalRuntimeReconciliation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<Uuid>,
    #[serde(default)]
    pub oldest_seq: u64,
    #[serde(default)]
    pub current_seq: u64,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub live_adopted: bool,
}

/// What to do with an isolated git worktree (and its branch) when the pane
/// that owns it is closed. Selected by the web UI before sending
/// `WebToServer::RemovePane` so the CLI knows which git commands to run.
/// Phase 1.1d of the swarm plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CliToServer {
    /// CLI registers with the server using auth token and version
    Register {
        token: String,
        #[serde(default)]
        version: Option<String>,
        #[serde(default)]
        capabilities: Vec<String>,
    },

    /// CLI starts a local session (hybrid mode)
    SessionStart {
        session_id: Uuid,
        /// Stable project identity from `.apas` (`ProjectMetadata.id`). Older
        /// CLIs may omit this; the server falls back to session_id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project_id: Option<Uuid>,
        /// Stable daemon identity for a project hosted inside `apas daemon`.
        /// Standalone and older CLIs omit it; current servers can infer a
        /// unique legacy host but fail closed when that inference is ambiguous.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine_id: Option<Uuid>,
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

    /// One-time migration snapshot from a compatible CLI. Once accepted, the
    /// server owns policy and subsequent local `.apas` changes are advisory
    /// only.
    ProjectPolicySnapshot {
        session_id: Uuid,
        team_enabled: bool,
        #[serde(default)]
        disallowed_tab_types: Vec<String>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        runtime: Option<TerminalRuntimeReconciliation>,
    },

    /// Capability and preservation consequence snapshot for project lifecycle
    /// controls. Re-sent after every registration and roster change.
    CliLifecycleInventory {
        session_id: Uuid,
        #[serde(default)]
        inventory: CliLifecycleInventory,
    },

    /// Progress/outcome for a lifecycle request. The server forwards this
    /// only to web connections that still have access to the project.
    CliLifecycleStatus {
        session_id: Uuid,
        request_id: Uuid,
        operation: CliLifecycleOperation,
        phase: CliLifecyclePhase,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inventory: Option<CliLifecycleInventory>,
    },

    /// Result from the CLI's isolated no-tools summary runner.
    PaneWorkSummaryResult {
        result: crate::PaneWorkSummaryGenerationResult,
    },

    /// Native bridge acceptance, not provider-recorded answer confirmation.
    OmpQuestionAnswerResult {
        session_id: Uuid,
        pane_id: u32,
        tool_use_id: String,
        request_id: String,
        accepted: bool,
        pending: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

/// Messages sent from server to CLI client
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
        /// Optional first instruction delivered atomically with pane creation.
        /// This lets retained mobile launch retries deduplicate on pane_id
        /// without risking a duplicate or missing follow-up Input frame.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        initial_input: Option<String>,
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

    /// Forget every pane owned by `machine_id`, a machine with no running
    /// instance that the owner confirmed will not come back. Sent to every
    /// attached instance: one sharing the same `.apas` removes that machine's
    /// entries from it under the project lock, so they are neither restored
    /// nor shown again. A separate checkout's `.apas` holds none of them and
    /// is left alone; if that machine ever returns, its instance reports its
    /// own tabs again, because each instance is authoritative for its panes.
    ForgetMachinePanes { session_id: Uuid, machine_id: Uuid },

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

    /// Correlated lifecycle request used by capability-aware peers. Servers
    /// must never translate an unsupported reconnect into legacy RebootCli.
    CliLifecycleRequest {
        session_id: Uuid,
        request_id: Uuid,
        operation: CliLifecycleOperation,
    },

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

    /// Resolve exactly one current native OMP dialog without terminal input.
    AnswerOmpQuestion {
        session_id: Uuid,
        pane_id: u32,
        tool_use_id: String,
        dialog_id: Uuid,
        request_id: String,
        #[serde(default)]
        answers: Vec<OmpQuestionAnswer>,
        #[serde(default)]
        cancel: bool,
    },

    /// Compute and return the current `git diff` for a pane's isolated
    /// worktree branch against the project's HEAD. Phase 1.2a.
    RequestPaneDiff { session_id: Uuid, pane_id: u32 },

    /// Ask the CLI to push the pane's branch to origin and create a
    /// GitHub PR via `gh pr create --fill`. Triggered by the "Create PR"
    /// button on the Diff modal. The CLI responds with
    /// `CliToServer::PrCreated`.
    CreatePr { session_id: Uuid, pane_id: u32 },

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

    /// Owner-operable workflow settings after cluster policy is split out of
    /// the legacy combined flags payload.
    UpdateProjectOperations {
        session_id: Uuid,
        auto_approve_todos: bool,
        auto_merge_prs: bool,
    },

    /// Server-authoritative capability policy. The CLI persists the version
    /// separately from owner-editable `.apas` state and enforces it at spawn.
    ProjectPolicy {
        session_id: Uuid,
        policy: EffectiveProjectPolicy,
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

    /// Run one bounded stage using the CLI's isolated no-tools summarizer.
    GeneratePaneWorkSummary {
        job: crate::PaneWorkSummaryGenerationJob,
    },
}

/// Which attached instance of a session a [`ServerToCli`] message is for.
///
/// A session can have one project instance attached per machine, each owning
/// the panes tagged with its machine id, so "the session's CLI" no longer
/// names one connection. Sending a pane's input or reboot to whichever
/// instance happens to be primary would act on the wrong machine's process,
/// and sending a policy change to only one instance would leave the others
/// enforcing a stale one. The server therefore routes every message it
/// addresses by session according to this classification.
///
/// [`ServerToCli::scope`] matches every variant without a wildcard arm, so a
/// new variant does not compile until someone decides which class it is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServerToCliScope {
    /// The message is for one existing pane: deliver it to the instance that
    /// owns that pane and to no other, whichever instance is primary. A pane
    /// with no `machine_id`, or one the roster does not list, has no owner
    /// yet and goes to the primary instance as a fallback; see
    /// [`PaneConfig::machine_id`]. The legacy deadloop pause/resume carry no
    /// pane id but act only on [`PANE_ID_DEADLOOP`], so they are in this
    /// class with that id.
    Pane(u32),
    /// `AddPane`: deliver it to the instance on `machine_id`, the
    /// `pane_config.machine_id` the server assigned to the new pane, or to the
    /// primary instance when that is `None`. Unlike [`Self::Process`] this
    /// carries the machine, because the instance that receives the pane is the
    /// one that tags, persists and runs it.
    NewPane { machine_id: Option<Uuid> },
    /// Project-wide state that every instance must hold (policy, flags and
    /// operations updates, roster requests, forgotten machines): fan it out to
    /// every attached instance.
    Project,
    /// Addressed to one instance process rather than to a pane (reboot,
    /// lifecycle, signals): deliver it to the machine the originating request
    /// named explicitly, else the primary instance. The server picks that
    /// machine from the web request, which is why the CLI-bound message itself
    /// carries none.
    Process,
    /// Session-level work aimed at a pane the message does not name, such as
    /// a question or plan answer, or `Input` without a pane id. The server
    /// resolves the target pane itself from its pending decision, so the
    /// caller supplies the target; without one it goes to the primary.
    ///
    /// A CLI applies `Input` without a pane id to [`PANE_ID_INTERACTIVE`]. A
    /// caller with no pending decision to resolve therefore routes it to the
    /// owner of the legacy pane it means (`pane_type` through
    /// [`PaneConfig::pane_id_from_legacy`], else [`PANE_ID_INTERACTIVE`])
    /// before falling back to the primary.
    Unscoped,
    /// Addressed to one CLI connection and never routed by session:
    /// registration, version refusal, session assignment and rejection, and
    /// heartbeats.
    Connection,
}

impl ServerToCli {
    /// How the server routes this message among a session's attached
    /// instances. See [`ServerToCliScope`].
    pub fn scope(&self) -> ServerToCliScope {
        use ServerToCliScope::{Connection, NewPane, Pane, Process, Project, Unscoped};
        match self {
            Self::Registered { .. }
            | Self::RegistrationFailed { .. }
            | Self::VersionUnsupported { .. }
            | Self::SessionRejected { .. }
            | Self::SessionAssigned { .. }
            | Self::Heartbeat => Connection,

            Self::Input {
                pane_id: Some(pane_id),
                ..
            } => Pane(*pane_id),
            Self::Input { pane_id: None, .. }
            | Self::AnswerQuestion { .. }
            | Self::PlanReviewAnswer { .. } => Unscoped,

            Self::PausePane { pane_id, .. }
            | Self::ResumePane { pane_id, .. }
            | Self::RebootPane { pane_id, .. }
            | Self::RemovePane { pane_id, .. }
            | Self::StartBot { pane_id, .. }
            | Self::StopBot { pane_id, .. }
            | Self::UpdatePaneEffort { pane_id, .. }
            | Self::UpdatePaneLabel { pane_id, .. }
            | Self::UpdatePaneModel { pane_id, .. }
            | Self::InterruptPane { pane_id, .. }
            | Self::AnswerOmpQuestion { pane_id, .. }
            | Self::RequestPaneDiff { pane_id, .. }
            | Self::CreatePr { pane_id, .. }
            | Self::UpdatePaneRole { pane_id, .. }
            | Self::UpdatePaneReviewMode { pane_id, .. }
            | Self::UpdatePaneManualMode { pane_id, .. }
            | Self::TerminalInput { pane_id, .. }
            | Self::TerminalResize { pane_id, .. } => Pane(*pane_id),
            // The summarizer runs no tools, but it hands this pane's
            // conversation to a provider under the receiving host's own
            // summary configuration, including whether a provider other than
            // the pane's may see it. That decision belongs to the pane's host.
            Self::GeneratePaneWorkSummary { job } => Pane(job.pane_id),
            // No pane id on the wire, but the CLI pauses or resumes only
            // `PANE_ID_DEADLOOP` (and writes that pane's state), and the web
            // pairs each with a `pause_pane`/`resume_pane` for the same pane.
            // Sent to the primary, they would act on a pane another machine
            // owns.
            Self::PauseDeadloop { .. } | Self::ResumeDeadloop { .. } => Pane(PANE_ID_DEADLOOP),

            Self::AddPane { pane_config, .. } => NewPane {
                machine_id: pane_config.machine_id,
            },

            // `SessionDisconnected` is never sent today; were it sent, a web
            // viewer leaving concerns every instance equally.
            Self::RequestPaneList { .. }
            | Self::UpdateProjectFlags { .. }
            | Self::UpdateProjectOperations { .. }
            | Self::ProjectPolicy { .. }
            | Self::ForgetMachinePanes { .. }
            | Self::SessionDisconnected { .. } => Project,

            Self::Signal { .. } | Self::RebootCli { .. } | Self::CliLifecycleRequest { .. } => {
                Process
            }
        }
    }
}

// ============================================================================
// Daemon <-> Server Messages
// ============================================================================

/// Messages sent from machine daemon to server
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonToServer {
    /// Capability-gated reports from the independent autonomous job host.
    AutonomousJob { report: crate::jobs::JobWorkerReport },

    /// Daemon registers with the server using auth token + machine info
    Register {
        token: String,
        machine: MachineInfo,
        projects: Vec<MachineProjectInfo>,
        #[serde(default)]
        capabilities: Vec<String>,
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

    /// Confirmed host-local registration; the server still finalizes authority.
    LocalProjectRegistered {
        request_id: String,
        result: LocalProjectRegistrationResult,
    },

    /// Result of a marker-bound discard requested after server-side
    /// authorization disappeared during two-phase provisioning.
    ProjectProvisioningDiscarded {
        request_id: String,
        project_id: String,
        discarded: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// Completion of an intentional project-runtime stop. New servers wait
    /// for this before final project deletion so detached pane hosts cannot
    /// outlive the project record that authorized them.
    ProjectRuntimeStopped {
        request_id: Uuid,
        project_id: String,
        success: bool,
        #[serde(default)]
        remaining_pane_hosts: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// Outcome of a `StartProjectCli` that carried a `request_id`, sent only by
    /// daemons advertising [`PROJECT_INSTANCES_CAPABILITY`]. The server waits
    /// on it when a tab is added on a machine whose instance is stopped:
    /// heartbeats carry only a sticky `last_error` that a successful spawn
    /// clears, so they cannot say whether a failure belongs to this attempt or
    /// an earlier one, and several refusals set no error at all.
    ///
    /// `success` = the project now runs in this daemon, including when it
    /// already did. It is the start, not the attachment: the instance still
    /// has to send `SessionStart`. Otherwise `error` says why nothing started,
    /// for example a missing or suspended policy, a live claim held by
    /// another host, an externally started `--headless` run, or a failed
    /// spawn.
    ProjectRuntimeStarted {
        request_id: Uuid,
        project_id: String,
        success: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

/// Messages sent from server to machine daemon
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerToDaemon {
    /// Never sent to a daemon without `omp_job_worker_v1`.
    AutonomousJob { command: crate::jobs::JobWorkerCommand },

    /// Registration successful
    Registered {
        machine_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cluster_owner_user_id: Option<Uuid>,
    },

    /// Registration failed
    RegistrationFailed { reason: String },

    /// Start APAS CLI for a project on this machine
    StartProjectCli {
        project_id: String,
        /// Current server-authoritative policy. A capable daemon refuses a
        /// start command without it, which makes mixed-version rollout fail
        /// closed instead of reviving a suspended project.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        policy: Option<EffectiveProjectPolicy>,
        /// Correlates this start with
        /// [`DaemonToServer::ProjectRuntimeStarted`]. Sent only to daemons
        /// advertising [`PROJECT_INSTANCES_CAPABILITY`]; an older daemon
        /// ignores it and reports nothing beyond its heartbeat. `None` asks for
        /// no report, as every start did before.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<Uuid>,
    },

    /// Stop APAS CLI for a project on this machine
    StopProjectCli {
        project_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<Uuid>,
    },

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
        /// Present for two-phase provisioning. The daemon must use this ID in
        /// local metadata instead of minting an identity of its own.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project_id: Option<String>,
        #[serde(default)]
        provisioning_mode: ProjectProvisioningMode,
        /// When true, `project_id` is an existing project this checkout joins
        /// as another instance: clone without creating a branch, write `.apas`
        /// with that id and zero panes, register, and do not start. Nothing
        /// here carries a tab. The web adds one afterwards with an ordinary
        /// [`WebToServer::AddPane`] naming this machine, which starts the
        /// instance; see [`WebToServer::CreateProjectInstance`]'s
        /// `join_project_id`.
        ///
        /// For that follow-up to find the checkout, the daemon sends a
        /// `Heartbeat` listing it *before* `ProjectInstanceCreated`. The server
        /// handles one daemon's messages in order, so it already lists the
        /// project on this machine when the acknowledgement reaches the web.
        /// Acknowledging first would race the web's next request against the
        /// inventory refresh.
        ///
        /// A separate flag because a present `project_id` already means a
        /// fresh identity reserved for two-phase provisioning. The server only
        /// sends it to daemons advertising [`PROJECT_INSTANCES_CAPABILITY`]: an
        /// older daemon ignores it and, on the owner path, would branch, mint
        /// an unrelated project, and start it.
        #[serde(default)]
        join_existing: bool,
    },

    /// Register an existing directory in place, without starting its project.
    RegisterLocalProject { path: String, request_id: String },

    /// Remove a not-yet-finalized checkout only when its local receipt matches
    /// both opaque IDs. No caller-supplied filesystem path is accepted.
    DiscardProjectProvisioning {
        request_id: String,
        project_id: String,
    },

    /// Replace this daemon, applying an available update first.
    ///
    /// Handled with the same update-then-`exec` path the unattended
    /// self-upgrade uses: `exec` preserves the pid, keeps the process
    /// detached, and skips destructors, so the host record and project claims
    /// are never withdrawn across the replacement.
    RebootDaemon,

    /// Request a fresh project scan/update push
    RefreshProjects,

    /// Compatibility tombstone for older daemons. Upgraded daemons must
    /// reject this inbound mutation and must never emit it.
    #[deprecated(note = "MiniMax support has been retired")]
    #[serde(alias = "set_minimax_config")]
    SetMiniMaxConfig {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_base_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key: Option<String>,
        #[serde(default)]
        clear_api_key: bool,
    },

    /// Compatibility tombstone for older daemons. Upgraded daemons must
    /// reject this inbound mutation and must never emit it.
    #[deprecated(note = "GLM support has been retired")]
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
    /// `ANTHROPIC_API_KEY` environment overrides.
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

/// Strictly redacted mobile renderer health signals. These events contain no
/// project/session/pane identifiers or arbitrary strings by design.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MobileTelemetryEvent {
    TerminalBridgeReady,
    TerminalBridgeRejectedMessage,
    TerminalBridgeCrash,
}

/// Messages sent from web client to server
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebToServer {
    SubscribeJob {
        project_id: String,
        job_id: Uuid,
        after_seq: u64,
    },
    UnsubscribeJob { job_id: Uuid },

    /// Authenticate with JWT token
    Authenticate {
        token: String,
        #[serde(default)]
        capabilities: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_kind: Option<crate::mobile::ClientKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app_version: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        protocol_version: Option<u32>,
    },

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

    /// A deliberate chat-style message for a terminal-hosted agent. Unlike
    /// [`WebToServer::TerminalInput`], this is safe to persist and display in
    /// conversation history because the user submitted it through the
    /// conversation composer rather than typing arbitrary terminal bytes.
    /// The server converts it to terminal input and appends Enter.
    TerminalConversationInput {
        session_id: Uuid,
        pane_id: u32,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_msg_id: Option<String>,
    },

    /// Approve a tool call. See `Input::session_id` for the multi-attach rationale.
    Approve {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        tool_call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
    },

    /// Reject a tool call. See `Input::session_id`.
    Reject {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        tool_call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
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
        /// project team, false for ordinary user-created terminal work. See
        /// PaneConfig::managed.
        #[serde(default)]
        managed: bool,
        /// Agent (default) vs Terminal. A Terminal pane ignores `prompt`,
        /// `role`, `goal`, `backstory` and `plan_review_mode` — there is
        /// no system prompt to inject into a pty-hosted TUI.
        #[serde(default)]
        kind: PaneKind,
        /// Machine whose project instance should run the new pane. `None` =
        /// the session's primary instance, the only target an older server
        /// knows. A machine other than the primary is honoured only when its
        /// daemon advertises [`PROJECT_INSTANCES_CAPABILITY`]; otherwise the
        /// request fails with a correlated reason rather than landing on the
        /// primary, which would run the tab on a machine the user did not pick.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine_id: Option<Uuid>,
        /// Client correlation id. When present the server answers with
        /// [`ServerToWeb::AddPaneResult`] carrying it. An older server ignores
        /// it and answers nothing, so a client that sees no result must report
        /// the launch as unconfirmed, never resend it: a replay can create a
        /// second tab.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
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

    /// Drop every pane owned by `machine_id` from the project roster, after
    /// the owner confirms `Forget <host>'s tabs` for a machine that will not
    /// come back. Tabs never move between machines and a stopped machine's
    /// tabs cannot be closed one by one (their state lives on that machine),
    /// so this is the only way to clear them. Refused while `machine_id` has a
    /// running instance, which is authoritative for its own panes and would
    /// report them again. Running instances receive
    /// [`ServerToCli::ForgetMachinePanes`].
    ForgetMachinePanes {
        session_id: Uuid,
        machine_id: Uuid,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
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
        /// Which of the project's instances to reboot. `None` = the primary
        /// instance, today's single-machine behaviour. Projects run inside
        /// their machine's daemon, so this also picks which host's daemon,
        /// and every project in it, is replaced.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine_id: Option<Uuid>,
    },

    /// Capability-gated project lifecycle operation. `session_id` is
    /// required so multi-attached web connections cannot accidentally target
    /// the last session that happened to attach.
    CliLifecycleRequest {
        session_id: Uuid,
        request_id: Uuid,
        /// `None` when the client asked for an operation this build retired.
        #[serde(deserialize_with = "lenient_lifecycle_operation")]
        operation: Option<CliLifecycleOperation>,
        /// Which of the project's instances the operation is for. `None` = the
        /// primary instance, today's single-machine behaviour. See
        /// `RebootCli::machine_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine_id: Option<Uuid>,
    },

    /// Start APAS CLI for a daemon project
    StartMachineProjectCli {
        machine_id: Uuid,
        project_id: String,
        /// Start the project as a whole rather than on `machine_id` alone,
        /// as the project view's Start means. Tabs never move between
        /// machines, so the server starts an instance on every online machine
        /// that owns tabs of the project and advertises
        /// [`PROJECT_INSTANCES_CAPABILITY`], and on `machine_id` (the view's
        /// resolved machine) only when no machine qualifies. Each machine is
        /// authorized as a single-machine request would be.
        ///
        /// False, and absent from every older client, starts only on
        /// `machine_id`, which is what a per-machine Start on the Machines
        /// page means. The two must stay distinguishable: fanning out on the
        /// plain message would make that per-machine button start other
        /// machines too. An older server ignores the flag and starts only
        /// `machine_id`.
        #[serde(default)]
        all_instances: bool,
    },

    /// Stop APAS CLI for a daemon project
    StopMachineProjectCli {
        machine_id: Uuid,
        project_id: String,
        /// Stop the project as a whole, as the project view's Close means:
        /// every machine with an instance of it attached, each authorized as a
        /// single-machine request would be. False, and absent from every older
        /// client, stops only `machine_id`, as a per-machine Stop on the
        /// Machines page does. An older server ignores the flag and stops only
        /// `machine_id`.
        #[serde(default)]
        all_instances: bool,
    },

    /// Restart the daemon on one machine, applying an available update first.
    ///
    /// Targeted by machine rather than by a project on it: a daemon is
    /// per-machine, and a machine running nothing has no project to route
    /// through. Restarting a daemon does not disturb the projects, panes, or
    /// agents on that host — the daemon owns no long-lived children.
    RebootDaemon { machine_id: Uuid },

    /// Create a brand-new project instance under a repo on a chosen machine:
    /// the daemon clones `clone_url` into `~/apas_projects/<instance_name>`
    /// (auto-suffixed on collision), checks out a fresh `branch`, registers a
    /// `.apas`, and starts it. `git_remote` is the canonical key (for the
    /// daemon's sibling-URL fallback + naming); `clone_url` is the real URL.
    CreateProjectInstance {
        machine_id: Uuid,
        /// Explicit cluster context for shared targets. Omitted by legacy
        /// clients and for compatibility owner flows.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cluster_owner_user_id: Option<String>,
        git_remote: String,
        instance_name: String,
        branch: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        clone_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        /// Clone onto the machine as another instance of this existing project
        /// instead of creating a new project. The checkout takes this project's
        /// identity with no panes, creates no `branch`, and is not started.
        /// Owner path only: the requester must host the project and own the
        /// target machine. An older server ignores the field and creates a
        /// separate project. Clients therefore send it only for a machine
        /// whose entry reports `project_instances_available`, which older
        /// servers never set.
        ///
        /// The tab is a second, client-chained step rather than part of this
        /// request:
        /// 1. Send this with a `request_id`.
        /// 2. Wait for [`ServerToWeb::ProjectInstanceCreated`] carrying that
        ///    `request_id`, `project_id` equal to `join_project_id`, and no
        ///    `error`.
        /// 3. Send [`WebToServer::AddPane`] with this `machine_id` and a fresh
        ///    `request_id`. The server starts the instance, answers
        ///    [`AddPaneStatus::Starting`], then `Routed` or `Failed`.
        ///
        /// "Clone, start and add" is one launch only in the UI. If the client
        /// loses the acknowledgement, the checkout may exist without a tab;
        /// report that as unconfirmed and never resend the clone.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        join_project_id: Option<Uuid>,
    },

    /// Owner-only registration of an existing directory on the selected host.
    RegisterLocalProject {
        machine_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cluster_owner_user_id: Option<String>,
        path: String,
        request_id: String,
    },

    /// Compatibility tombstone for stale web clients. Upgraded servers must
    /// reject this inbound mutation and must never forward its credential.
    #[deprecated(note = "MiniMax support has been retired")]
    #[serde(alias = "set_machine_minimax_config")]
    SetMachineMiniMaxConfig {
        machine_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_base_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key: Option<String>,
        #[serde(default)]
        clear_api_key: bool,
    },

    /// Compatibility tombstone for stale web clients. Upgraded servers must
    /// reject this inbound mutation and must never forward its credential.
    #[deprecated(note = "GLM support has been retired")]
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        /// Question text → selected option label(s) joined with ", " for
        /// multi-select.
        answers: std::collections::HashMap<String, String>,
    },

    /// Lossless OMP submission. Scope and correlation are required, unlike the
    /// legacy Claude answer path.
    AnswerOmpQuestion {
        session_id: Uuid,
        pane_id: u32,
        tool_use_id: String,
        dialog_id: Uuid,
        request_id: String,
        #[serde(default)]
        answers: Vec<OmpQuestionAnswer>,
        #[serde(default)]
        cancel: bool,
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

    UpdateProjectOperations {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
        auto_approve_todos: bool,
        auto_merge_prs: bool,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
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
    TerminalAttach { session_id: Uuid, pane_id: u32 },

    /// Fetch cached summaries and reconcile retained source for one pane.
    ListPaneWorkSummaries {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default)]
        include_current: bool,
    },

    /// Retry a failed/stale window, or refresh the current window when omitted.
    RefreshPaneWorkSummary {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        window_start: Option<chrono::DateTime<chrono::Utc>>,
    },

    /// Redacted native/WebView health event. Accepted only from an
    /// authenticated mobile device session.
    MobileTelemetry { event: MobileTelemetryEvent },

    /// Liveness probe from the browser. Server echoes
    /// `ServerToWeb::Heartbeat` so the client can detect silently-stale
    /// connections (mobile OS throttling, NAT timeout, swallowed RST)
    /// without depending on `readyState`.
    Heartbeat,
}

/// Messages sent from server to web client
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProjectAccessChange {
    Transferred,
    Revoked,
    Deleted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MutationKind {
    Approval,
    Question,
    PlanReview,
    Interrupt,
}

/// Why a requested session attachment was rejected. The session id carried
/// alongside this value lets clients reconcile concurrent foreground and
/// background attachment attempts without treating a global error as the
/// result of whichever navigation happens to be visible.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionAttachmentRejectionReason {
    ProjectAccessRequired,
    HostMachineAccessRequired,
    ProjectUnavailable,
    SessionNotFound,
}

/// Outcome of one correlated [`WebToServer::AddPane`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AddPaneStatus {
    /// Delivered to the target machine's running instance. This is delivery,
    /// not creation: the tab appears when that instance reports it in the
    /// project's pane list.
    Routed,
    /// The target machine has the project but was not running it, so its
    /// instance is being started and the tab will be created when it
    /// attaches. Starting is asynchronous and bounded, so this is never the
    /// last word: `Routed` or `Failed` follows for the same `request_id`.
    /// Pending starts live in server memory and a server restart drops them
    /// without a reply, which a client must show as unconfirmed rather than
    /// resend.
    Starting,
    /// No tab was created; `error` says why.
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerToWeb {
    JobSnapshot { detail: crate::jobs::JobDetail },
    JobEvent { event: crate::jobs::JobEvent },
    JobChanged {
        project_id: String,
        job_id: Uuid,
        state_version: u64,
        last_event_seq: u64,
    },
    JobSubscriptionRejected {
        project_id: String,
        job_id: Uuid,
        code: String,
        message: String,
        resync_required: bool,
    },

    /// Authentication successful
    Authenticated {
        user_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_email: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        server_version: Option<String>,
        #[serde(default)]
        cluster_role: String,
        #[serde(default)]
        account_status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        protocol_version: Option<u32>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        negotiated_capabilities: Vec<String>,
        #[serde(default = "default_true")]
        mutations_allowed: bool,
    },

    /// The peer authenticated, but its protocol cannot safely mutate state.
    /// A mobile client may retain read-only cached/live access while guiding
    /// the user to install a compatible build.
    ProtocolIncompatible {
        minimum_version: u32,
        maximum_version: u32,
        read_only: bool,
        message: String,
    },

    /// Authentication failed
    AuthenticationFailed { reason: String },

    /// Correlated acceptance or rejection for a time-sensitive coding
    /// mutation. `accepted` means the current authorized CLI accepted the
    /// operation for processing; clients never infer success from send().
    MutationAck {
        request_id: String,
        session_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
        mutation: MutationKind,
        accepted: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        /// A fresh native bridge check permits an explicit corrected submission
        /// after rejection. Missing means the question's state is uncertain.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        question_pending: Option<bool>,
    },

    /// Latest connected-CLI lifecycle capabilities and per-pane reboot
    /// preservation consequence list.
    CliLifecycleInventory {
        session_id: Uuid,
        #[serde(default)]
        inventory: CliLifecycleInventory,
        /// The instance that reported this inventory, which the server tags
        /// from the sending attachment. Each instance has its own hosting
        /// capability and panes, so clients keep one inventory per (session,
        /// machine). A per-session one would flip to whichever instance
        /// reported last, and a reboot confirmation would count another
        /// machine's panes. On attach the server replays one per attached
        /// instance. `None` from older servers and for an attachment that
        /// reported no machine, i.e. the session's only instance.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine_id: Option<Uuid>,
    },

    /// Authoritative progress/outcome for one lifecycle request.
    CliLifecycleStatus {
        session_id: Uuid,
        request_id: Uuid,
        operation: CliLifecycleOperation,
        phase: CliLifecyclePhase,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inventory: Option<CliLifecycleInventory>,
        /// The instance the request was for (see
        /// `WebToServer::CliLifecycleRequest::machine_id`), so a carried
        /// `inventory` replaces that machine's entry. `None` as for
        /// [`ServerToWeb::CliLifecycleInventory`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine_id: Option<Uuid>,
    },

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

    /// Correlated rejection for one requested session attachment.
    SessionAttachmentRejected {
        session_id: Uuid,
        reason: SessionAttachmentRejectionReason,
        message: String,
    },

    /// The session's attached project instances. Always the complete set
    /// rather than a delta, so an empty list means no instance is attached.
    ///
    /// Sent only to connections that advertised
    /// [`PROJECT_INSTANCES_CAPABILITY`] in `Authenticate.capabilities`. Older
    /// native builds reject an unknown frame and reconnect, and a replay on
    /// every attach would keep them reconnecting. For an attached connection
    /// that advertised it, the server sends the set right after
    /// `SessionAttached`, and again whenever the set changes: an instance
    /// attaches or detaches, or the primary hands over. A client without the
    /// capability uses `SessionInfo::instances` from the session list and the
    /// pane tags in `PaneList`, which older schemas accept.
    SessionInstances {
        session_id: Uuid,
        instances: Vec<SessionInstanceInfo>,
    },

    /// Output from Claude
    Output {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Uuid>,
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

    /// Project-wide authorization changed outside this socket. Clients use
    /// this as a refresh hint; the server remains the authorization boundary.
    ProjectAccessChanged {
        project_id: String,
        change: ProjectAccessChange,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
    },

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

    /// A provider transcript recorded a web-originated terminal message.
    /// UserInput only acknowledges forwarding; it is not delivery evidence.
    TerminalConversationRecorded {
        session_id: Uuid,
        pane_id: u32,
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

    /// Answer to a [`WebToServer::AddPane`] that carried a `request_id`.
    /// `Starting` is followed by a final `Routed` or `Failed` for the same
    /// request; see [`AddPaneStatus`].
    ///
    /// Sent only to the connection that sent the request, never to the
    /// session's other viewers. Only a client that understands this frame
    /// sends a `request_id`, and older native builds drop their socket on a
    /// frame their schema does not list. If that connection is gone when a
    /// later result is ready, the result is dropped, and the client reports
    /// the launch as unconfirmed.
    AddPaneResult {
        session_id: Uuid,
        request_id: String,
        status: AddPaneStatus,
        /// The machine the tab was routed to, or is waiting for.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine_id: Option<Uuid>,
        /// The pane id the server allocated, once there is one. Ids are
        /// allocated against the whole project, so they are unique across
        /// machines and concurrent requests.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
        /// Why no tab was created; set with `Failed`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// Usage limits update from a CLI client
    UsageLimits {
        cli_client_id: Uuid,
        #[serde(default)]
        provider: Provider,
        limits: UsageLimits,
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

    /// Local registration result after canonical identity and policy finalize.
    LocalProjectRegistered {
        machine_id: Uuid,
        request_id: String,
        result: LocalProjectRegistrationResult,
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

    /// Effective cluster-governed policy for the attached project.
    ProjectPolicyChanged {
        session_id: Uuid,
        policy: EffectiveProjectPolicy,
        #[serde(default)]
        noncompliant_pane_ids: Vec<u32>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        runtime: Option<TerminalRuntimeReconciliation>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        runtime: Option<TerminalRuntimeReconciliation>,
    },

    /// Complete pane-scoped snapshot returned on list/open.
    PaneWorkSummaries {
        session_id: Uuid,
        pane_id: u32,
        #[serde(default)]
        summaries: Vec<crate::PaneWorkSummary>,
        #[serde(default)]
        availability: crate::PaneWorkSummaryAvailability,
    },

    /// Incremental generation/cache state for a single pane window.
    PaneWorkSummaryUpdated {
        session_id: Uuid,
        pane_id: u32,
        summary: crate::PaneWorkSummary,
        #[serde(default)]
        availability: crate::PaneWorkSummaryAvailability,
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct PaneTodoPrMsg {
    pub pane_id: u32,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation: Option<String>,
}

/// One row in the suggested-workers queue. The Manager pane appends
/// these as `## SUG-NNN — label` sections to `suggested-workers.md`;
/// the Overview renders each as a card with Accept / Dismiss buttons.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct SuggestedWorkerMsg {
    pub id: String,
    pub label: String,
    pub role: String,
    pub goal: String,
    pub backstory: String,
    #[serde(default)]
    pub needs_worktree: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct TeamTodoWorkerMsg {
    pub pane_id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_hint: Option<String>,
    pub subtasks: Vec<TeamTodoSubtaskMsg>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct TeamTodoSubtaskMsg {
    pub id: String,
    pub title: String,
    /// pending | in_progress | done | reviewing | revising | approved
    pub status: String,
    pub parent: String,
    pub body: String,
}

/// Information about a persisted session
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
    /// True when at least one pane is currently reporting work. This is
    /// meaningful only while `is_active` is true.
    #[serde(default)]
    pub is_working: bool,
    /// The panes in this session and whether each is working.
    ///
    /// `is_working` above answers "is anything happening here", which cannot
    /// answer "which agent is waiting for me": a project with one busy pane
    /// reads as working, hiding every idle pane in it. Both are derived from
    /// the same pane statuses, so they cannot disagree.
    ///
    /// Defaulted: an older server omits it, and a client must read that as "no
    /// pane detail" rather than as a session with no panes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panes: Vec<MobilePaneSummary>,
    /// The project's attached (running) instances, one per machine, so a
    /// project list can name every host instead of only the primary's
    /// `hostname`. Omitted by older servers and when nothing is attached; in
    /// both cases `hostname`/`working_dir` remain the description to show.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instances: Vec<SessionInstanceInfo>,
}

/// One attached (running) instance of a session's project on one machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionInstanceInfo {
    /// `None` for an attached CLI that reported no machine: standalone and
    /// older CLIs omit it, and the server's legacy inference fails closed
    /// when ambiguous. Such a CLI lacks [`PROJECT_INSTANCES_CAPABILITY`], so
    /// it is always the session's only instance and therefore its primary.
    /// Requests that name no machine reach it, and so do controls for
    /// untagged panes. No machine-addressed request can target it. Listing
    /// it keeps the set complete: a live session never looks as if nothing
    /// were attached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<Uuid>,
    /// Display name of `machine_id`; never used for routing.
    pub hostname: String,
    /// The instance's checkout. Instances on a shared filesystem report the
    /// same path; separate checkouts report their own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// True for the earliest attached instance. It supplies the session's
    /// legacy single-valued `hostname`/`working_dir` and receives every
    /// control that names no machine; when it leaves, the next earliest
    /// becomes primary.
    #[serde(default)]
    pub primary: bool,
    /// Whether the viewer this was sent to may use this instance's runtime:
    /// type into its tabs, add tabs on it, reboot it. The server computes it
    /// per connection, because runtime access is evaluated per machine. On a
    /// third-party host it needs cluster membership that covers this exact
    /// machine, so one viewer can use some of a project's instances and not
    /// others. Clients disable the instance's tabs and its machine choice
    /// when it is `Some(false)`, and say why. They do not offer controls that
    /// can only fail with "Access denied". `None` means not evaluated; treat
    /// the instance as usable and let the server decide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_access: Option<bool>,
}

/// One agent pane, as mobile needs to list it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MobilePaneSummary {
    pub pane_id: u32,
    /// The pane's own label when it has one; the client falls back to its kind
    /// and id, exactly as the session screen does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub kind: PaneKind,
    pub provider: Provider,
    /// Model/backend override used to associate provider-scoped availability
    /// with the same usage account the pane actually consumes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// True while this pane is reporting work, from the same pane statuses the
    /// session-level flag is derived from.
    #[serde(default)]
    pub is_working: bool,
    /// True while this pane is blocked on an unresolved agent question. This
    /// is neither active work nor idle time and takes presentation precedence
    /// over provider availability.
    #[serde(default)]
    pub awaiting_answer: bool,
    /// Most recent working-to-idle transition observed by the server. Older
    /// servers omit it; clients keep those panes visible after timestamped ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_since: Option<String>,
    /// Provider availability is not pane activity. Carry the latest explicit
    /// block separately so shared/mobile snapshots do not turn it into idle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_limited: Option<UsageLimited>,
    /// Machine whose project instance runs this pane; see
    /// [`PaneConfig::machine_id`]. Absent from older servers and for panes
    /// written before multi-machine instances, which no machine owns yet and
    /// whose controls go to the session's primary instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<Uuid>,
    /// Display name for the pane's host badge; see [`PaneConfig::hostname`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
}

/// Aggregated usage counters for a pane or project over one time window.
/// All token counts come from the per-turn Claude/Codex stream `result`
/// usage; `prompts` counts user/loop inputs and `responses` counts completed
/// turns. Fields are snake_case so the wire keys match the web store verbatim.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    /// Sum of provider-reported cost in USD. A zero is meaningful only when
    /// `cost_usd_reported` is true.
    #[serde(default)]
    pub cost_usd: f64,
    /// Whether at least one event in this window supplied an authoritative
    /// cost value. Older peers omit this and therefore read as unknown.
    #[serde(default)]
    pub cost_usd_reported: bool,
}

/// Per-pane usage broken down by time window (cumulative lifetime plus the
/// rolling 7-day and today windows derived from day-bucketed rows).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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

/// Machine-level DeepSeek backend status safe to expose to web UI.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeepseekBackendInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_configured: bool,
}

/// Information about a machine reported by a daemon
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MachineInfo {
    pub machine_id: Uuid,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepseek_backend: Option<DeepseekBackendInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<String>,
}

/// APAS project discovered on a machine
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MachineProjectInfo {
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub path: String,
    /// Server-persisted navigation preference within this machine's cluster.
    /// This is neither a runtime placement lock nor an access grant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_machine_id: Option<Uuid>,
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
    /// Canonical lowercase `host/owner/repo` of the checkout's `origin`,
    /// normalized exactly like `CliToServer::SessionStart::git_remote`. A
    /// registered project that has never started has no session to carry its
    /// remote, so without this the web files it under "(no remote)" instead
    /// of beside the repository's other projects, and `apas register` cannot
    /// match it. `None` without an origin and from older daemons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_remote: Option<String>,
}

/// A registration cannot simultaneously succeed and carry an error.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LocalProjectRegistrationResult {
    Registered { project: MachineProjectInfo },
    Failed { error: String },
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProjectProvisioningMode {
    /// Compatibility behavior for a cluster owner's own machine. This may use
    /// owner-selected paths and credentials already configured on that host.
    #[default]
    TrustedOwner,
    /// Member behavior: canonical public GitHub HTTPS only, managed root only,
    /// no credential helpers, and no automatic runtime start.
    PublicGithub,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MachineClusterAccess {
    #[default]
    Owner,
    Member,
}

/// Machine with its project list
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MachineWithProjects {
    pub machine: MachineInfo,
    pub projects: Vec<MachineProjectInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster_owner_user_id: Option<String>,
    #[serde(default)]
    pub cluster_access: MachineClusterAccess,
    #[serde(default)]
    pub shared_provisioning_available: bool,
    #[serde(default)]
    pub local_project_registration_available: bool,
    /// The machine's daemon is connected and advertises
    /// [`PROJECT_INSTANCES_CAPABILITY`], so a project's tabs can be started on
    /// it beside instances on other machines. Clients offer a machine choice
    /// for new tabs only where this is true. False on a connected daemon
    /// (`daemon_connected`) means an older daemon: reboot it to update. False
    /// from older servers.
    #[serde(default)]
    pub project_instances_available: bool,
    /// The machine's daemon holds a live connection to this server now.
    /// `MachineInfo::last_seen` is stamped on connect, heartbeat and
    /// disconnect alike, so a just-disconnected daemon looks freshly seen.
    /// It cannot separate "offline" from "connected but too old", which a
    /// disabled machine choice has to explain differently. False from older
    /// servers.
    #[serde(default)]
    pub daemon_connected: bool,
}

/// Answer to `GET /projects/by-remote?git_remote=<host/owner/repo>`, with an
/// optional `&project_id=<uuid>`: the caller's existing projects for one
/// repository, so `apas register` can offer to join a checkout to one of them
/// instead of registering an unrelated project for the same repository.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectsByRemoteResponse {
    /// The canonical remote that was looked up.
    pub git_remote: String,
    /// Only projects the caller owns or hosts; empty when none match. The
    /// checkout's own current identity is listed too when its remote
    /// matches, so the CLI excludes its own id before offering a join.
    #[serde(default)]
    pub projects: Vec<ProjectRemoteMatch>,
    /// The project named by the `project_id` query parameter (the checkout's
    /// current local identity), whatever remote its sessions recorded.
    ///
    /// A project with history can be missing from `projects`: an older CLI
    /// never sent its remote, its origin changed, or no daemon has reported
    /// it yet. Zero panes is also a valid state for such a project. Without
    /// this field the CLI cannot tell it from a pristine identity, and
    /// re-identifying it would orphan its history.
    ///
    /// `None` when the query named no id, or when the caller neither owns nor
    /// hosts a project with that id. The CLI re-identifies a checkout only
    /// when it has no panes and the server holds no session for it: no
    /// `local`, or a `local` whose `has_session` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<ProjectRemoteMatch>,
}

/// One existing project whose sessions or registered checkouts carry the
/// requested remote.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectRemoteMatch {
    pub project_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The server holds a session, and so history, for this project. A
    /// registered project that never started has none and still matches.
    #[serde(default)]
    pub has_session: bool,
    /// Where the project is known to be checked out, so the join prompt can
    /// say which project it means (`on <host>:<path>`).
    #[serde(default)]
    pub instances: Vec<ProjectRemoteInstance>,
}

/// One known checkout of a matching project.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectRemoteInstance {
    /// `None` when the checkout is known only from a session recorded without
    /// a daemon machine id, as standalone and older CLIs report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<Uuid>,
    pub hostname: String,
    pub path: String,
    /// An instance of the project is running on this machine now.
    #[serde(default)]
    pub running: bool,
}

/// Pane type for dual-pane mode (legacy - kept for backward compatibility)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneType {
    /// Autonomous deadloop worker (left pane)
    #[default]
    Deadloop,
    /// Interactive user session (right pane)
    Interactive,
}

/// Provider for a pane
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default, JsonSchema)]
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
    /// Decode-only compatibility tombstone. MiniMax is not launchable.
    #[deprecated(note = "MiniMax support has been retired")]
    Minimax,
    /// Decode-only compatibility tombstone. GLM is not launchable.
    #[deprecated(note = "GLM support has been retired")]
    Glm,
    Deepseek,
    Opencode,
    /// Pi coding agent (`pi`): terminal-only in APAS. See
    /// `terminal_pane::terminal_binary_for`.
    Pi,
    /// Oh My Pi (`omp`): a standalone Pi-derived agent, terminal-only in APAS.
    /// Unlike Pi it has no `--session-id`, so a pane is pinned by giving it a
    /// private `--session-dir` instead; see `terminal_pane::omp_session_args`.
    Omp,
    #[serde(rename = "cursor-agent")]
    CursorAgent,
}

/// Mode for a pane
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneMode {
    Deadloop,
    Interactive,
}

/// How a pane hosts its agent process. Orthogonal to [`Provider`] (which
/// binary) and [`PaneMode`] (how autonomous).
///
/// * [`PaneKind::Agent`] — the legacy/managed-team path: the CLI runs the provider
///   headlessly (`claude --print --output-format stream-json`,
///   `codex exec --json`) and parses structured events into
///   `CliToServer::StreamMessage`. Everything team-mode depends on —
///   usage counters, pane status, diffs, plan review, scratchpad
///   publishing, Tech Lead delegation — is built on those events. New
///   unmanaged panes are no longer created with this kind, but existing panes
///   remain compatible and managed team panes continue to use it.
/// * [`PaneKind::Terminal`] — the pane instead hosts the provider's real
///   interactive TUI on a pty. Raw bytes flow over the dedicated
///   `Terminal*` messages and are rendered by xterm.js in the browser.
///   Nothing is parsed, so a terminal pane has none of the structured
///   integrations above and is never a delegation target. This is the normal
///   kind for new user-created Claude, Codex, and OpenCode panes.
///
/// `#[serde(default)]` on `PaneConfig::kind` keeps `.apas` files written
/// before this existed deserializing as `Agent`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default, JsonSchema)]
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
/// Deliberately *not* every `Provider`. The retired compatibility tombstones
/// are never advertised, and DeepSeek is offered as a Claude model because it
/// uses the Anthropic-compatible bridge.
///
/// Terminal panes exist only for claude, codex, opencode, and pi — see
/// `terminal_pane::terminal_binary_for`, which this must stay in step with.
pub fn all_tab_types() -> Vec<String> {
    vec![
        tab_type_key(PaneKind::Terminal, Provider::Claude),
        tab_type_key(PaneKind::Terminal, Provider::Codex),
        tab_type_key(PaneKind::Terminal, Provider::Opencode),
        tab_type_key(PaneKind::Terminal, Provider::Pi),
        tab_type_key(PaneKind::Terminal, Provider::Omp),
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
    !disallowed
        .iter()
        .any(|d| d.trim().eq_ignore_ascii_case(&key))
}

/// A stable, cluster-governed launch capability. Unlike the legacy tab-type
/// key, this identifies the frontend/backend/model combination as well as the
/// pane kind, so administrators can allow Claude Terminal without also
/// allowing every Anthropic-compatible backend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct LaunchProfile {
    pub key: String,
    pub label: String,
    pub kind: PaneKind,
    pub provider: Provider,
    pub backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Canonical model identity for Claude Code running through DeepSeek's
/// Anthropic-compatible endpoint. Pane state and policy keys use this base ID;
/// runtime-only selectors such as `[1m]` must not leak into either.
///
/// DeepSeek's own catalogue is the authority on what this may be. As of
/// 2026-09-11 `GET https://api.deepseek.com/models` lists exactly
/// `deepseek-flash` and `deepseek-v4-pro`, and passing anything else is
/// rejected with a message naming that set. The flash id is deliberately
/// unversioned upstream, so it tracks the current flash release.
pub const DEEPSEEK_FLASH_MODEL: &str = "deepseek-flash";
pub const DEEPSEEK_DEFAULT_MODEL: &str = DEEPSEEK_FLASH_MODEL;

/// The id flash shipped under before DeepSeek dropped the version from it.
/// Still accepted by the API, and still sitting in `.apas` files and stored
/// policy here, so it canonicalizes forward rather than being retired — it
/// names the same offering.
pub const DEEPSEEK_LEGACY_FLASH_MODEL: &str = "deepseek-v4-flash";

/// DeepSeek Pro, withdrawn from APAS. The model still exists upstream; APAS
/// simply no longer offers it, so an existing pane on it reads as an
/// unsupported provider rather than silently becoming a different model with
/// different cost and behaviour.
pub const DEEPSEEK_RETIRED_PRO_MODEL: &str = "deepseek-v4-pro";

/// Whether a model is one of the DeepSeek identities APAS explicitly supports.
pub fn is_supported_deepseek_model(model: &str) -> bool {
    canonical_deepseek_model(model).is_some()
}

/// Whether a model is a DeepSeek identity APAS used to offer and no longer does.
pub fn is_retired_deepseek_model(model: &str) -> bool {
    model
        .trim()
        .eq_ignore_ascii_case(DEEPSEEK_RETIRED_PRO_MODEL)
}

/// Resolve a supported, possibly differently-cased DeepSeek identity to the
/// exact value APAS persists and places in policy keys.
pub fn canonical_deepseek_model(model: &str) -> Option<&'static str> {
    let normalized = model.trim();
    if normalized.eq_ignore_ascii_case(DEEPSEEK_FLASH_MODEL)
        || normalized.eq_ignore_ascii_case(DEEPSEEK_LEGACY_FLASH_MODEL)
    {
        Some(DEEPSEEK_FLASH_MODEL)
    } else {
        None
    }
}

fn launch_profile(
    key: &str,
    label: &str,
    kind: PaneKind,
    provider: Provider,
    backend: &str,
    model: Option<&str>,
) -> LaunchProfile {
    LaunchProfile {
        key: key.to_string(),
        label: label.to_string(),
        kind,
        provider,
        backend: backend.to_string(),
        model: model.map(str::to_string),
    }
}

/// Canonical registry used by migration defaults, server validation, CLI
/// enforcement, and the web admin policy editor. Structured agent panes are a
/// decode/resume compatibility path only and must never be advertised here.
pub fn supported_launch_profiles() -> Vec<LaunchProfile> {
    vec![
        launch_profile(
            "terminal:claude:official:default",
            "Claude Terminal",
            PaneKind::Terminal,
            Provider::Claude,
            "official",
            None,
        ),
        launch_profile(
            "terminal:codex:official:default",
            "Codex Terminal",
            PaneKind::Terminal,
            Provider::Codex,
            "official",
            None,
        ),
        launch_profile(
            "terminal:opencode:official:default",
            "OpenCode Terminal",
            PaneKind::Terminal,
            Provider::Opencode,
            "official",
            None,
        ),
        launch_profile(
            "terminal:pi:official:default",
            "Pi Terminal",
            PaneKind::Terminal,
            Provider::Pi,
            "official",
            None,
        ),
        launch_profile(
            "terminal:omp:official:default",
            "OMP Terminal",
            PaneKind::Terminal,
            Provider::Omp,
            "official",
            None,
        ),
        launch_profile(
            "terminal:claude:deepseek:deepseek-flash",
            "DeepSeek 4.1 Flash Terminal",
            PaneKind::Terminal,
            Provider::Claude,
            "deepseek",
            Some(DEEPSEEK_FLASH_MODEL),
        ),
    ]
}

/// Profile keys that named the same capability under an older model id.
///
/// Stored allowlists are filtered against the supported set, and anything
/// unrecognized is dropped — so renaming flash's key without this would
/// silently strip DeepSeek from every cluster and project that had explicitly
/// allowed it. Retired capabilities are deliberately absent: DeepSeek Pro
/// *should* be dropped.
pub fn renamed_launch_profile_key(key: &str) -> Option<&'static str> {
    match key.trim() {
        "terminal:claude:deepseek:deepseek-v4-flash"
        | "agent:claude:deepseek:deepseek-v4-flash" => {
            Some("terminal:claude:deepseek:deepseek-flash")
        }
        _ => None,
    }
}

fn normalized_model(model: Option<&str>) -> String {
    model
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "default".to_string())
}

/// Whether an explicit provider value is a decode-only retirement tombstone.
#[allow(deprecated)]
pub fn is_retired_provider(provider: Provider) -> bool {
    matches!(provider, Provider::Minimax | Provider::Glm)
}

/// Whether a historical model name selects a retired backend.
pub fn is_retired_model(model: Option<&str>) -> bool {
    let normalized = normalized_model(model);
    normalized.contains("minimax")
        || normalized.starts_with("m2")
        || normalized.starts_with("glm")
        || normalized.contains("glm-")
}

/// Whether a backend name identifies a retired Anthropic-compatible bridge.
pub fn is_retired_backend(backend: &str) -> bool {
    matches!(
        backend.trim().to_ascii_lowercase().as_str(),
        "minimax" | "mini_max" | "m2" | "glm" | "zai" | "z.ai" | "zhipu"
    )
}

/// Whether a stable launch-profile key refers to a retired provider, backend,
/// or model. This accepts historical keys without treating arbitrary
/// substrings in unrelated labels as capabilities.
pub fn is_retired_launch_profile_key(key: &str) -> bool {
    let segments = key.trim().split(':').map(str::trim).collect::<Vec<_>>();
    if segments.len() < 2 {
        return false;
    }

    segments
        .get(1)
        .is_some_and(|provider| is_retired_backend(provider))
        || segments
            .get(2)
            .is_some_and(|backend| is_retired_backend(backend))
        || segments
            .last()
            .is_some_and(|model| is_retired_model(Some(model)) || is_retired_backend(model))
}

/// Whether a launch request names a retired provider through either its
/// explicit provider value or a historical backend-selecting model.
pub fn is_retired_launch(provider: Provider, model: Option<&str>) -> bool {
    is_retired_provider(provider) || is_retired_model(model)
}

/// Derive the registry key for a launch request. Unknown model IDs still get
/// a deterministic key and are therefore denied by an explicit allowlist.
#[allow(deprecated)]
pub fn launch_profile_key(kind: PaneKind, provider: Provider, model: Option<&str>) -> String {
    if is_retired_launch(provider, model) {
        return "unsupported:retired".to_string();
    }
    let kind_name = if kind == PaneKind::Terminal {
        "terminal"
    } else {
        "agent"
    };
    let provider_name = match provider {
        #[allow(deprecated)]
        Provider::Claude | Provider::Deepseek => "claude",
        Provider::Codex => "codex",
        Provider::Opencode => "opencode",
        Provider::Pi => "pi",
        Provider::Omp => "omp",
        Provider::CursorAgent => "cursor-agent",
        Provider::Minimax | Provider::Glm => "unsupported",
    };
    let mut normalized = normalized_model(model);
    if provider == Provider::Deepseek && normalized == "default" {
        normalized = DEEPSEEK_DEFAULT_MODEL.to_string();
    } else if let Some(canonical) = canonical_deepseek_model(&normalized) {
        normalized = canonical.to_string();
    }
    let backend = if normalized.contains("deepseek") || provider == Provider::Deepseek {
        "deepseek"
    } else {
        "official"
    };
    format!("{kind_name}:{provider_name}:{backend}:{normalized}")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct EffectiveProjectPolicy {
    pub team_available: bool,
    pub allowed_launch_profiles: Vec<String>,
    pub version: i64,
    #[serde(default)]
    pub project_suspended: bool,
}

impl Default for EffectiveProjectPolicy {
    fn default() -> Self {
        Self {
            team_available: false,
            allowed_launch_profiles: supported_launch_profiles()
                .into_iter()
                .map(|profile| profile.key)
                .collect(),
            version: 1,
            project_suspended: false,
        }
    }
}

impl EffectiveProjectPolicy {
    pub fn allows(&self, kind: PaneKind, provider: Provider, model: Option<&str>) -> bool {
        !is_retired_launch(provider, model)
            && !self.project_suspended
            && self.allowed_launch_profiles.iter().any(|allowed| {
                allowed.eq_ignore_ascii_case(&launch_profile_key(kind, provider, model))
            })
    }
}

/// Configuration for a single pane
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
    pub model: Option<String>, // Optional model/backend override (e.g., "o3", "deepseek-flash")
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
    /// Lead may consider them for delegation. `false` (the compatibility
    /// default and the value for user-created terminal panes) = not part of
    /// the team queue and never a Tech Lead delegation target.
    #[serde(default)]
    pub managed: bool,
    /// Machine whose project instance runs this pane. A project can run one
    /// instance per machine under one session, and a pane's process, terminal
    /// and controls live on exactly one of them for the pane's lifetime. The
    /// server assigns this from the authenticated instance that reports the
    /// pane, overriding whatever the pane data says, so no instance can claim
    /// another machine's panes.
    ///
    /// `None` = written before multi-machine instances. No machine owns such
    /// a pane yet. Its controls go to the session's primary instance as a
    /// fallback, and the first instance that reports it adopts it. When
    /// instance M reports its roster, the server:
    /// - replaces every pane tagged M, so a pane tagged M that M no longer
    ///   reports is gone;
    /// - tags M on a reported pane whose roster entry is untagged (adoption
    ///   from the `.apas`, surfacing on the server);
    /// - drops and logs a reported pane whose roster entry is tagged for
    ///   another machine;
    /// - leaves alone every untagged pane M did not report.
    ///
    /// The last rule is why "untagged" does not mean "the primary's". A
    /// separate checkout can join, and become primary, while the pre-upgrade
    /// host is stopped. If the primary owned untagged panes, that checkout's
    /// first, empty roster would erase every tab the host still has, with no
    /// stopped badge and nothing to forget. A CLI without
    /// [`PROJECT_INSTANCES_CAPABILITY`] is only ever the session's sole
    /// instance and still replaces the whole roster, as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<Uuid>,
    /// Hostname of `machine_id`, for display only. Hosts get renamed, so
    /// routing and ownership always use `machine_id`. On an untagged pane the
    /// server may keep the host it last recorded the pane under, so the tab is
    /// not shown under whichever instance happens to be primary. It never
    /// implies ownership.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
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
            machine_id: None,
            hostname: None,
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CliClientStatus {
    Online,
    Offline,
    Busy,
}

/// Usage limit information for a time window (5-hour or 7-day)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
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

/// A provider-confirmed usage limit that is currently preventing work.
///
/// This is deliberately separate from utilization: included usage may reach
/// 100% while paid extra usage remains available, in which case the provider
/// is not blocking requests and this field stays absent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct UsageLimited {
    /// Human-readable limiting window, such as "weekly" or "5-hour".
    pub window: String,
    /// When the provider expects work to become available again.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "reset_at",
        alias = "resetAt",
        alias = "resetsAt"
    )]
    pub resets_at: Option<String>,
    /// Model family this limit applies to. Absent means the provider account
    /// is blocked generally. Anthropic currently uses display names such as
    /// "Fable" for model-scoped weekly limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl UsageLimited {
    /// Whether this provider-confirmed block applies to a pane's configured
    /// model. A scoped limit must never make a default/other-model pane look
    /// unavailable merely because both use the same provider account.
    pub fn applies_to_model(&self, pane_model: Option<&str>) -> bool {
        let Some(scope) = self
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return true;
        };
        let Some(pane_model) = pane_model.map(str::trim).filter(|s| !s.is_empty()) else {
            return false;
        };
        let scope = scope.to_ascii_lowercase();
        let pane_model = pane_model.to_ascii_lowercase();
        pane_model == scope || pane_model.contains(&scope) || scope.contains(&pane_model)
    }
}

/// Usage limits from the provider API/logs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
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
    /// Present only when the provider says a usage limit is actively blocking
    /// requests. A full utilization meter alone is not sufficient.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "usageLimited"
    )]
    pub usage_limited: Option<UsageLimited>,
}

// ============================================================================
// Claude Stream-JSON Message Types
// These match the output format of `claude --output-format stream-json`
// ============================================================================

/// Top-level message from Claude CLI stream-json output
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClaudeAssistantMessage {
    pub content: Vec<ClaudeContentBlock>,
    #[serde(default)]
    pub model: String,
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

/// Claude user message structure (for tool results)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClaudeUserMessage {
    pub content: Vec<ClaudeContentBlock>,
    #[serde(default)]
    pub role: String,
}

/// Content block types in Claude messages
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CodexUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

/// Error info from Codex turn.failed
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
            session_id: None,
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
    fn machine_project_preference_is_optional_for_legacy_inventory() {
        let mut project: MachineProjectInfo = serde_json::from_value(serde_json::json!({
            "project_id": "legacy",
            "path": "/legacy"
        }))
        .unwrap();
        assert_eq!(project.preferred_machine_id, None);
        assert!(serde_json::to_value(&project)
            .unwrap()
            .get("preferred_machine_id")
            .is_none());
        let machine_id = Uuid::new_v4();
        project.preferred_machine_id = Some(machine_id);
        let decoded: MachineProjectInfo =
            serde_json::from_value(serde_json::to_value(project).unwrap()).unwrap();
        assert_eq!(decoded.preferred_machine_id, Some(machine_id));
    }

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
            capabilities: Vec::new(),
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
    fn pane_work_summary_message_paths_round_trip() {
        let session_id = Uuid::new_v4();
        let job = crate::PaneWorkSummaryGenerationJob {
            protocol_version: crate::PANE_WORK_SUMMARY_PROTOCOL_VERSION,
            job_id: Uuid::new_v4(),
            session_id,
            pane_id: 9,
            pane_provider: Provider::Claude,
            window_start: "2026-08-11T03:00:00Z".parse().unwrap(),
            window_end: "2026-08-11T06:00:00Z".parse().unwrap(),
            source_digest: "digest".to_string(),
            stage: crate::PaneWorkSummaryStage::Final,
            chunk_index: None,
            chunk_count: None,
            content: "notes".to_string(),
            correction_attempt: false,
        };

        let cli_job = ServerToCli::GeneratePaneWorkSummary { job: job.clone() };
        let decoded: ServerToCli =
            serde_json::from_str(&serde_json::to_string(&cli_job).unwrap()).unwrap();
        assert!(
            matches!(decoded, ServerToCli::GeneratePaneWorkSummary { job: decoded } if decoded == job)
        );

        let web_request = WebToServer::ListPaneWorkSummaries {
            session_id,
            pane_id: 9,
            include_current: true,
        };
        let decoded: WebToServer =
            serde_json::from_str(&serde_json::to_string(&web_request).unwrap()).unwrap();
        assert!(
            matches!(decoded, WebToServer::ListPaneWorkSummaries { session_id: sid, pane_id: 9, include_current: true } if sid == session_id)
        );

        let legacy_request: WebToServer = serde_json::from_value(serde_json::json!({
            "type": "list_pane_work_summaries",
            "session_id": session_id,
            "pane_id": 9
        }))
        .unwrap();
        assert!(matches!(
            legacy_request,
            WebToServer::ListPaneWorkSummaries {
                include_current: false,
                ..
            }
        ));
    }

    #[test]
    fn test_cli_to_server_session_start_serialization() {
        let session_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();
        let msg = CliToServer::SessionStart {
            session_id,
            project_id: None,
            machine_id: Some(machine_id),
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
        assert!(json.contains(&machine_id.to_string()));
        assert!(json.contains("\"git_remote\":\"github.com/shuaimu/apas\""));

        let deserialized: CliToServer = serde_json::from_str(&json).unwrap();
        match deserialized {
            CliToServer::SessionStart {
                session_id: sid,
                machine_id: parsed_machine_id,
                working_dir,
                git_remote,
                ..
            } => {
                assert_eq!(sid, session_id);
                assert_eq!(parsed_machine_id, Some(machine_id));
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
            machine_id: None,
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
                    cost_usd_reported: true,
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
                assert_eq!(
                    usage.get("input_tokens").and_then(|v| v.as_u64()),
                    Some(1178)
                );
                assert_eq!(
                    usage.get("output_tokens").and_then(|v| v.as_u64()),
                    Some(78)
                );
                assert_eq!(
                    usage
                        .get("cache_read_input_tokens")
                        .and_then(|v| v.as_u64()),
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
            WebToServer::Input {
                client_msg_id,
                text,
                ..
            } => {
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
        assert!(!serde_json::to_string(&no_id)
            .unwrap()
            .contains("client_msg_id"));

        let terminal_conversation = WebToServer::TerminalConversationInput {
            session_id: Uuid::new_v4(),
            pane_id: 9,
            text: "hello terminal agent".to_string(),
            client_msg_id: Some("terminal-send-1".to_string()),
        };
        let json = serde_json::to_string(&terminal_conversation).unwrap();
        assert!(json.contains("\"type\":\"terminal_conversation_input\""));
        assert!(json.contains("\"client_msg_id\":\"terminal-send-1\""));
        assert!(matches!(
            serde_json::from_str::<WebToServer>(&json).unwrap(),
            WebToServer::TerminalConversationInput { pane_id: 9, .. }
        ));
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
            capabilities: Vec::new(),
            client_kind: None,
            app_version: None,
            protocol_version: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"authenticate\""));

        let legacy: WebToServer =
            serde_json::from_str(r#"{"type":"authenticate","token":"legacy","capabilities":[]}"#)
                .unwrap();
        match legacy {
            WebToServer::Authenticate {
                client_kind,
                app_version,
                protocol_version,
                ..
            } => {
                assert_eq!(client_kind, None);
                assert_eq!(app_version, None);
                assert_eq!(protocol_version, None);
            }
            _ => panic!("expected authenticate"),
        }

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
    fn session_attachment_rejection_is_correlated_and_typed() {
        let session_id = Uuid::new_v4();
        let message = ServerToWeb::SessionAttachmentRejected {
            session_id,
            reason: SessionAttachmentRejectionReason::HostMachineAccessRequired,
            message: "This project is shared with you, but this hosting machine is not".to_string(),
        };
        let json = serde_json::to_string(&message).unwrap();
        assert!(json.contains("\"type\":\"session_attachment_rejected\""));
        assert!(json.contains("\"reason\":\"host_machine_access_required\""));
        match serde_json::from_str::<ServerToWeb>(&json).unwrap() {
            ServerToWeb::SessionAttachmentRejected {
                session_id: parsed_session_id,
                reason,
                ..
            } => {
                assert_eq!(parsed_session_id, session_id);
                assert_eq!(
                    reason,
                    SessionAttachmentRejectionReason::HostMachineAccessRequired
                );
            }
            other => panic!("unexpected message: {other:?}"),
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
    fn legacy_machine_backend_fields_are_ignored_and_not_reemitted() {
        let machine_id = Uuid::new_v4();
        let json = serde_json::json!({
            "machine_id": machine_id,
            "hostname": "legacy-host",
            "os": "linux",
            "arch": "x86_64",
            "minimax_backend": {"api_key": "must-not-survive"},
            "glm_backend": {"api_key": "must-not-survive"}
        });
        let machine: MachineInfo = serde_json::from_value(json).unwrap();
        let encoded = serde_json::to_value(machine).unwrap();
        assert!(encoded.get("minimax_backend").is_none());
        assert!(encoded.get("glm_backend").is_none());
    }

    #[test]
    #[allow(deprecated)]
    fn legacy_configuration_messages_remain_decodable() {
        let daemon = serde_json::json!({
            "type": "set_minimax_config",
            "api_key": "discard-me",
            "clear_api_key": false
        });
        assert!(matches!(
            serde_json::from_value::<ServerToDaemon>(daemon).unwrap(),
            ServerToDaemon::SetMiniMaxConfig { .. }
        ));

        let web = serde_json::json!({
            "type": "set_machine_glm_config",
            "machine_id": Uuid::new_v4(),
            "api_key": "discard-me",
            "clear_api_key": false
        });
        assert!(matches!(
            serde_json::from_value::<WebToServer>(web).unwrap(),
            WebToServer::SetMachineGlmConfig { .. }
        ));
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
            runtime: Some(TerminalRuntimeReconciliation {
                runtime_id: Some(Uuid::new_v4()),
                oldest_seq: 3,
                current_seq: 9,
                truncated: true,
                live_adopted: true,
            }),
        };
        let json = serde_json::to_string(&cli).unwrap();
        match serde_json::from_str::<CliToServer>(&json).unwrap() {
            CliToServer::TerminalState {
                session_id: got_session,
                pane_id,
                instance_id: got_instance,
                lifecycle,
                status,
                runtime,
            } => {
                assert_eq!(got_session, session_id);
                assert_eq!(pane_id, 11);
                assert_eq!(got_instance, Some(instance_id));
                assert_eq!(lifecycle, TerminalLifecycle::Exited);
                assert_eq!(status.as_deref(), Some("exited with status 1"));
                assert_eq!(runtime.expect("runtime metadata").current_seq, 9);
            }
            other => panic!("unexpected variant: {other:?}"),
        }

        let web = ServerToWeb::TerminalState {
            session_id,
            pane_id: 11,
            instance_id: Some(instance_id),
            lifecycle: TerminalLifecycle::Disconnected,
            status: None,
            runtime: None,
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

    #[test]
    fn lifecycle_protocol_is_correlated_and_legacy_reboot_is_unchanged() {
        let session_id = Uuid::new_v4();
        let request_id = Uuid::new_v4();
        let reboot = WebToServer::CliLifecycleRequest {
            session_id,
            request_id,
            operation: Some(CliLifecycleOperation::RebootCli),
            machine_id: None,
        };
        let json = serde_json::to_string(&reboot).unwrap();
        assert!(json.contains("\"type\":\"cli_lifecycle_request\""));
        assert!(json.contains("\"operation\":\"reboot_cli\""));
        match serde_json::from_str::<WebToServer>(&json).unwrap() {
            WebToServer::CliLifecycleRequest {
                request_id: got,
                operation,
                ..
            } => {
                assert_eq!(got, request_id);
                assert_eq!(operation, Some(CliLifecycleOperation::RebootCli));
            }
            other => panic!("unexpected variant: {other:?}"),
        }

        // A browser still running a bundle from before `reconnect_transport`
        // was retired must not take the socket down: the message has to decode
        // with the operation dropped, not fail outright.
        let stale = format!(
            r#"{{"type":"cli_lifecycle_request","session_id":"{session_id}","request_id":"{request_id}","operation":"reconnect_transport"}}"#
        );
        match serde_json::from_str::<WebToServer>(&stale)
            .expect("a retired operation must not fail the whole decode")
        {
            WebToServer::CliLifecycleRequest { operation, .. } => assert_eq!(operation, None),
            other => panic!("unexpected variant: {other:?}"),
        }

        let legacy = format!(r#"{{"type":"reboot_cli","session_id":"{session_id}"}}"#);
        assert!(matches!(
            serde_json::from_str::<WebToServer>(&legacy).unwrap(),
            WebToServer::RebootCli { .. }
        ));
    }

    #[test]
    fn lifecycle_inventory_defaults_for_rolling_upgrade() {
        let json = r#"{
            "type":"cli_lifecycle_inventory",
            "session_id":"52443f74-5819-4502-83fe-db530fe70feb"
        }"#;
        match serde_json::from_str::<CliToServer>(json).unwrap() {
            CliToServer::CliLifecycleInventory { inventory, .. } => {
                assert!(!inventory.persistent_terminal_hosting);
                assert!(inventory.panes.is_empty());
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn legacy_daemon_project_stop_defaults_to_uncorrelated_cleanup() {
        let legacy = r#"{"type":"stop_project_cli","project_id":"project-a"}"#;
        assert!(matches!(
            serde_json::from_str::<ServerToDaemon>(legacy).unwrap(),
            ServerToDaemon::StopProjectCli {
                project_id,
                request_id: None,
            } if project_id == "project-a"
        ));
    }

    // --- tab-type policy -------------------------------------------------

    /// The web's catalog, so the two cannot drift apart unnoticed. Same trick
    /// the CLI already uses for `providerOptions.ts`.
    const WEB_TAB_TYPES_TS: &str = include_str!("../../../packages/web/src/lib/tabTypes.ts");

    #[test]
    fn tab_type_keys_are_kind_then_provider() {
        assert_eq!(
            tab_type_key(PaneKind::Agent, Provider::Claude),
            "agent:claude"
        );
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
        assert!(!tab_type_allowed(
            &deny,
            PaneKind::Terminal,
            Provider::Claude
        ));
        assert!(tab_type_allowed(&deny, PaneKind::Agent, Provider::Claude));
        assert!(tab_type_allowed(&deny, PaneKind::Terminal, Provider::Codex));
    }

    #[test]
    fn deny_entries_tolerate_whitespace_and_case() {
        let deny = vec!["  Agent:Claude  ".to_string()];
        assert!(!tab_type_allowed(&deny, PaneKind::Agent, Provider::Claude));
    }

    #[test]
    fn user_creatable_catalog_contains_only_supported_terminals() {
        let catalog = all_tab_types();
        assert_eq!(
            catalog,
            vec![
                "terminal:claude".to_string(),
                "terminal:codex".to_string(),
                "terminal:opencode".to_string(),
                "terminal:pi".to_string(),
                "terminal:omp".to_string(),
            ]
        );
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

    #[test]
    fn launch_profile_registry_keys_are_unique_and_derived_from_profiles() {
        let profiles = supported_launch_profiles();
        let keys = profiles
            .iter()
            .map(|profile| profile.key.clone())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(keys.len(), profiles.len());
        for profile in profiles {
            assert_eq!(
                profile.key,
                launch_profile_key(profile.kind, profile.provider, profile.model.as_deref())
            );
        }
    }

    #[test]
    #[allow(deprecated)]
    fn retired_provider_values_deserialize_but_are_never_supported() {
        for (json, provider) in [
            ("\"minimax\"", Provider::Minimax),
            ("\"glm\"", Provider::Glm),
        ] {
            assert_eq!(serde_json::from_str::<Provider>(json).unwrap(), provider);
            assert!(is_retired_provider(provider));
            assert!(!EffectiveProjectPolicy::default().allows(PaneKind::Agent, provider, None));
        }
    }

    #[test]
    fn retirement_classifier_covers_historical_forms() {
        for model in ["MiniMax-M2.7", "minimax-m2", "M2.1", "glm-5.1", "GLM4"] {
            assert!(is_retired_model(Some(model)), "model: {model}");
        }
        for backend in ["minimax", "mini_max", "glm", "z.ai", "zhipu"] {
            assert!(is_retired_backend(backend), "backend: {backend}");
        }
        for key in [
            "agent:claude:minimax:minimax-m2.7",
            "agent:minimax:official:default",
            "agent:claude:glm:glm-5.1",
            "agent:glm:official:default",
        ] {
            assert!(is_retired_launch_profile_key(key), "profile: {key}");
        }
        for supported in [
            "agent:claude:official:default",
            "agent:claude:deepseek:deepseek-v4-pro",
            "agent:claude:deepseek:deepseek-v4-flash",
            "agent:codex:official:o3",
        ] {
            assert!(
                !is_retired_launch_profile_key(supported),
                "profile: {supported}"
            );
        }
    }

    #[test]
    fn supported_profiles_and_default_policy_offer_only_terminal_backends() {
        let profiles = supported_launch_profiles();
        assert_eq!(profiles.len(), 6);
        assert!(profiles
            .iter()
            .all(|profile| profile.kind == PaneKind::Terminal));
        assert_eq!(
            profiles
                .iter()
                .map(|profile| (profile.key.as_str(), profile.label.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("terminal:claude:official:default", "Claude Terminal"),
                ("terminal:codex:official:default", "Codex Terminal"),
                ("terminal:opencode:official:default", "OpenCode Terminal"),
                ("terminal:pi:official:default", "Pi Terminal"),
                ("terminal:omp:official:default", "OMP Terminal"),
                (
                    "terminal:claude:deepseek:deepseek-flash",
                    "DeepSeek 4.1 Flash Terminal",
                ),
            ]
        );
        assert!(profiles.iter().all(|profile| {
            !is_retired_provider(profile.provider)
                && !is_retired_backend(&profile.backend)
                && !is_retired_model(profile.model.as_deref())
                && !is_retired_launch_profile_key(&profile.key)
        }));
        let policy = EffectiveProjectPolicy::default();
        assert!(policy
            .allowed_launch_profiles
            .iter()
            .all(|key| !is_retired_launch_profile_key(key)));
        for expected in [
            "terminal:claude:official:default",
            "terminal:codex:official:default",
            "terminal:opencode:official:default",
            "terminal:pi:official:default",
            "terminal:omp:official:default",
            "terminal:claude:deepseek:deepseek-flash",
        ] {
            assert!(policy
                .allowed_launch_profiles
                .contains(&expected.to_string()));
        }
        assert!(!policy
            .allowed_launch_profiles
            .iter()
            .any(|key| key.contains("deepseek-v4-pro")));
        assert!(policy
            .allowed_launch_profiles
            .iter()
            .all(|key| key.starts_with("terminal:")));
    }

    #[test]
    fn explicit_launch_allowlist_rejects_unknown_models_and_suspension() {
        let mut policy = EffectiveProjectPolicy {
            team_available: true,
            allowed_launch_profiles: vec!["agent:codex:official:default".to_string()],
            version: 4,
            project_suspended: false,
        };
        assert!(policy.allows(PaneKind::Agent, Provider::Codex, None));
        assert!(!policy.allows(PaneKind::Agent, Provider::Claude, Some("future-model")));
        assert!(!policy.allows(PaneKind::Agent, Provider::Claude, Some("deepseek-chat")));
        policy.project_suspended = true;
        assert!(!policy.allows(PaneKind::Agent, Provider::Codex, None));
    }

    #[test]
    #[allow(deprecated)]
    fn deepseek_resolves_to_one_model_and_pro_is_retired() {
        // Flash is the whole DeepSeek offering now, so the bare provider and
        // the explicit model must land on the same key.
        assert_eq!(
            launch_profile_key(PaneKind::Terminal, Provider::Deepseek, None),
            "terminal:claude:deepseek:deepseek-flash"
        );
        assert_eq!(
            launch_profile_key(
                PaneKind::Terminal,
                Provider::Claude,
                Some(DEEPSEEK_FLASH_MODEL)
            ),
            "terminal:claude:deepseek:deepseek-flash"
        );

        // The id flash shipped under before DeepSeek dropped the version
        // canonicalizes forward: it names the same offering, and `.apas` files
        // and stored policy still carry it.
        assert_eq!(
            canonical_deepseek_model(DEEPSEEK_LEGACY_FLASH_MODEL),
            Some(DEEPSEEK_FLASH_MODEL)
        );
        assert_eq!(
            launch_profile_key(
                PaneKind::Terminal,
                Provider::Claude,
                Some(DEEPSEEK_LEGACY_FLASH_MODEL)
            ),
            "terminal:claude:deepseek:deepseek-flash"
        );

        // Pro is withdrawn. It must not canonicalize to flash: that would
        // silently move a pane onto a different model with different cost.
        assert_eq!(canonical_deepseek_model(DEEPSEEK_RETIRED_PRO_MODEL), None);
        assert!(!is_supported_deepseek_model(DEEPSEEK_RETIRED_PRO_MODEL));
        assert!(is_retired_deepseek_model(DEEPSEEK_RETIRED_PRO_MODEL));
        assert!(!is_retired_deepseek_model(DEEPSEEK_FLASH_MODEL));

        // Exactly one DeepSeek profile is offered.
        let deepseek_profiles = supported_launch_profiles()
            .into_iter()
            .filter(|profile| profile.backend == "deepseek")
            .collect::<Vec<_>>();
        assert_eq!(deepseek_profiles.len(), 1);
        assert_eq!(
            deepseek_profiles[0].key,
            "terminal:claude:deepseek:deepseek-flash"
        );
        assert_eq!(
            deepseek_profiles[0].model.as_deref(),
            Some(DEEPSEEK_FLASH_MODEL)
        );
        assert!(!supported_launch_profiles()
            .iter()
            .any(|profile| profile.key.contains("deepseek-v4-pro")));
    }

    #[test]
    fn deepseek_allowlists_authorize_flash_and_never_retired_pro() {
        let flash_only = EffectiveProjectPolicy {
            team_available: false,
            allowed_launch_profiles: vec!["terminal:claude:deepseek:deepseek-flash".to_string()],
            version: 1,
            project_suspended: false,
        };
        assert!(flash_only.allows(
            PaneKind::Terminal,
            Provider::Claude,
            Some(DEEPSEEK_FLASH_MODEL)
        ));
        assert!(flash_only.allows(PaneKind::Terminal, Provider::Deepseek, None));
        // A withdrawn model is not authorized by the surviving profile.
        assert!(!flash_only.allows(
            PaneKind::Terminal,
            Provider::Claude,
            Some(DEEPSEEK_RETIRED_PRO_MODEL)
        ));
        // Nor does the DeepSeek profile leak into the agent kind.
        assert!(!flash_only.allows(
            PaneKind::Agent,
            Provider::Claude,
            Some(DEEPSEEK_FLASH_MODEL)
        ));
    }

    #[test]
    fn the_renamed_flash_key_survives_and_the_retired_one_does_not() {
        // An allowlist naming flash under its old id keeps DeepSeek; the
        // migration filters against the supported set, so without this mapping
        // the capability would be dropped from every policy that had it.
        for legacy in [
            "terminal:claude:deepseek:deepseek-v4-flash",
            "agent:claude:deepseek:deepseek-v4-flash",
        ] {
            assert_eq!(
                renamed_launch_profile_key(legacy),
                Some("terminal:claude:deepseek:deepseek-flash"),
                "{legacy} must carry forward"
            );
        }
        // Pro is meant to be dropped, so it deliberately has no mapping.
        for retired in [
            "terminal:claude:deepseek:deepseek-v4-pro",
            "agent:claude:deepseek:deepseek-v4-pro",
        ] {
            assert_eq!(renamed_launch_profile_key(retired), None);
        }
    }
}

#[cfg(test)]
mod daemon_reboot_tests {
    use super::*;

    /// The request names a machine, not a project. A machine running nothing
    /// still has a daemon worth restarting, so a project-derived route would
    /// be unable to reach it.
    #[test]
    fn a_daemon_reboot_is_addressed_by_machine() {
        let machine_id = Uuid::new_v4();
        let raw = serde_json::to_string(&WebToServer::RebootDaemon { machine_id }).unwrap();
        assert!(raw.contains("reboot_daemon"), "wire tag: {raw}");
        assert!(!raw.contains("project_id"));
        match serde_json::from_str::<WebToServer>(&raw).unwrap() {
            WebToServer::RebootDaemon { machine_id: got } => assert_eq!(got, machine_id),
            other => panic!("expected RebootDaemon, got {other:?}"),
        }
    }

    #[test]
    fn the_daemon_command_round_trips() {
        let raw = serde_json::to_string(&ServerToDaemon::RebootDaemon).unwrap();
        assert!(matches!(
            serde_json::from_str::<ServerToDaemon>(&raw).unwrap(),
            ServerToDaemon::RebootDaemon
        ));
    }

    /// An older daemon never receives this because nothing sends it there;
    /// what matters is that adding it did not disturb the commands it does
    /// receive.
    #[test]
    fn existing_daemon_commands_still_parse() {
        let legacy = r#"{"type":"refresh_projects"}"#;
        assert!(matches!(
            serde_json::from_str::<ServerToDaemon>(legacy).unwrap(),
            ServerToDaemon::RefreshProjects
        ));
    }
}

#[cfg(test)]
mod project_instances_tests {
    use super::*;
    use crate::mobile::MobileSessionSummary;
    use serde_json::json;
    use std::collections::{BTreeSet, HashMap};

    fn pane(pane_id: u32, machine_id: Option<Uuid>) -> PaneConfig {
        let mut pane = PaneConfig::defaults().remove(0);
        pane.pane_id = pane_id;
        pane.machine_id = machine_id;
        pane
    }

    fn wire_tag<T: Serialize>(message: &T) -> String {
        serde_json::to_value(message).unwrap()["type"]
            .as_str()
            .expect("internally tagged message")
            .to_string()
    }

    /// Every `ServerToCli` wire tag, read from the type's own schema so the
    /// list cannot drift from the enum.
    fn every_server_to_cli_tag() -> BTreeSet<String> {
        let schema = serde_json::to_value(schemars::schema_for!(ServerToCli)).unwrap();
        schema["oneOf"]
            .as_array()
            .expect("ServerToCli schema lists its variants")
            .iter()
            .map(|variant| {
                variant["properties"]["type"]["const"]
                    .as_str()
                    .expect("variant tag")
                    .to_string()
            })
            .collect()
    }

    /// One example of every variant with the scope the server routes it by.
    /// The table is the reviewed classification; `scope()` must agree with it.
    fn classification(session_id: Uuid, machine_id: Uuid) -> Vec<(ServerToCli, ServerToCliScope)> {
        use ServerToCliScope::{Connection, NewPane, Pane, Process, Project, Unscoped};
        let summary_job: crate::PaneWorkSummaryGenerationJob = serde_json::from_value(json!({
            "job_id": Uuid::new_v4(),
            "session_id": session_id,
            "pane_id": 9,
            "window_start": "2026-11-11T00:00:00Z",
            "window_end": "2026-11-11T01:00:00Z",
            "source_digest": "digest",
            "content": "conversation",
        }))
        .unwrap();
        let s = session_id;
        vec![
            (
                ServerToCli::Registered {
                    cli_id: Uuid::new_v4(),
                },
                Connection,
            ),
            (
                ServerToCli::RegistrationFailed {
                    reason: "token".into(),
                },
                Connection,
            ),
            (
                ServerToCli::VersionUnsupported {
                    client_version: "26.01.1".into(),
                    min_version: "26.11.0".into(),
                },
                Connection,
            ),
            (
                ServerToCli::SessionRejected {
                    session_id: s,
                    reason: "owner".into(),
                },
                Connection,
            ),
            (
                ServerToCli::SessionAssigned {
                    session_id: s,
                    working_dir: None,
                },
                Connection,
            ),
            (ServerToCli::Heartbeat, Connection),
            (
                ServerToCli::Input {
                    session_id: s,
                    data: "y".into(),
                    pane_id: Some(7),
                },
                Pane(7),
            ),
            (
                ServerToCli::Input {
                    session_id: s,
                    data: "y".into(),
                    pane_id: None,
                },
                Unscoped,
            ),
            (
                ServerToCli::AnswerQuestion {
                    session_id: s,
                    tool_use_id: "toolu_1".into(),
                    answers: HashMap::new(),
                },
                Unscoped,
            ),
            (
                ServerToCli::PlanReviewAnswer {
                    session_id: s,
                    tool_use_id: "toolu_2".into(),
                    approve: true,
                },
                Unscoped,
            ),
            (
                ServerToCli::PausePane {
                    session_id: s,
                    pane_id: 3,
                },
                Pane(3),
            ),
            (
                ServerToCli::ResumePane {
                    session_id: s,
                    pane_id: 3,
                },
                Pane(3),
            ),
            (
                ServerToCli::RebootPane {
                    session_id: s,
                    pane_id: 3,
                },
                Pane(3),
            ),
            (
                ServerToCli::RemovePane {
                    session_id: s,
                    pane_id: 3,
                    cleanup_action: None,
                },
                Pane(3),
            ),
            (
                ServerToCli::StartBot {
                    session_id: s,
                    pane_id: 3,
                    prompt: None,
                    min_iteration_interval_minutes: None,
                    effort: None,
                },
                Pane(3),
            ),
            (
                ServerToCli::StopBot {
                    session_id: s,
                    pane_id: 3,
                },
                Pane(3),
            ),
            (
                ServerToCli::UpdatePaneEffort {
                    session_id: s,
                    pane_id: 3,
                    effort: None,
                },
                Pane(3),
            ),
            (
                ServerToCli::UpdatePaneLabel {
                    session_id: s,
                    pane_id: 3,
                    label: "api".into(),
                },
                Pane(3),
            ),
            (
                ServerToCli::UpdatePaneModel {
                    session_id: s,
                    pane_id: 3,
                    provider: None,
                    model: None,
                },
                Pane(3),
            ),
            (
                ServerToCli::InterruptPane {
                    session_id: s,
                    pane_id: 3,
                },
                Pane(3),
            ),
            (
                ServerToCli::AnswerOmpQuestion {
                    session_id: s,
                    pane_id: 3,
                    tool_use_id: "toolu_3".into(),
                    dialog_id: Uuid::new_v4(),
                    request_id: "answer-1".into(),
                    answers: Vec::new(),
                    cancel: true,
                },
                Pane(3),
            ),
            (
                ServerToCli::RequestPaneDiff {
                    session_id: s,
                    pane_id: 3,
                },
                Pane(3),
            ),
            (
                ServerToCli::CreatePr {
                    session_id: s,
                    pane_id: 3,
                },
                Pane(3),
            ),
            (
                ServerToCli::UpdatePaneRole {
                    session_id: s,
                    pane_id: 3,
                    role: None,
                    goal: None,
                    backstory: None,
                },
                Pane(3),
            ),
            (
                ServerToCli::UpdatePaneReviewMode {
                    session_id: s,
                    pane_id: 3,
                    mode: PlanReviewMode::Always,
                },
                Pane(3),
            ),
            (
                ServerToCli::UpdatePaneManualMode {
                    session_id: s,
                    pane_id: 3,
                    manual_mode: true,
                },
                Pane(3),
            ),
            (
                ServerToCli::TerminalInput {
                    session_id: s,
                    pane_id: 3,
                    data_b64: "eA==".into(),
                },
                Pane(3),
            ),
            (
                ServerToCli::TerminalResize {
                    session_id: s,
                    pane_id: 3,
                    cols: 80,
                    rows: 24,
                },
                Pane(3),
            ),
            (
                ServerToCli::GeneratePaneWorkSummary { job: summary_job },
                Pane(9),
            ),
            (
                ServerToCli::AddPane {
                    session_id: s,
                    pane_config: pane(11, Some(machine_id)),
                    isolated_worktree: false,
                    initial_input: None,
                },
                NewPane {
                    machine_id: Some(machine_id),
                },
            ),
            (
                ServerToCli::AddPane {
                    session_id: s,
                    pane_config: pane(12, None),
                    isolated_worktree: false,
                    initial_input: Some("start".into()),
                },
                NewPane { machine_id: None },
            ),
            (ServerToCli::RequestPaneList { session_id: s }, Project),
            (
                ServerToCli::UpdateProjectFlags {
                    session_id: s,
                    auto_approve_todos: false,
                    auto_merge_prs: false,
                    team_enabled: false,
                    disallowed_tab_types: Vec::new(),
                },
                Project,
            ),
            (
                ServerToCli::UpdateProjectOperations {
                    session_id: s,
                    auto_approve_todos: true,
                    auto_merge_prs: false,
                },
                Project,
            ),
            (
                ServerToCli::ProjectPolicy {
                    session_id: s,
                    policy: EffectiveProjectPolicy::default(),
                },
                Project,
            ),
            (
                ServerToCli::ForgetMachinePanes {
                    session_id: s,
                    machine_id,
                },
                Project,
            ),
            (ServerToCli::SessionDisconnected { session_id: s }, Project),
            (
                ServerToCli::Signal {
                    session_id: s,
                    signal: "SIGINT".into(),
                },
                Process,
            ),
            (
                ServerToCli::PauseDeadloop { session_id: s },
                Pane(PANE_ID_DEADLOOP),
            ),
            (
                ServerToCli::ResumeDeadloop { session_id: s },
                Pane(PANE_ID_DEADLOOP),
            ),
            (ServerToCli::RebootCli { session_id: s }, Process),
            (
                ServerToCli::CliLifecycleRequest {
                    session_id: s,
                    request_id: Uuid::new_v4(),
                    operation: CliLifecycleOperation::RebootCli,
                },
                Process,
            ),
        ]
    }

    /// `scope()` has no wildcard arm, so a new variant cannot compile
    /// unclassified. This pins each existing classification and fails until a
    /// new variant also gets a row here.
    #[test]
    fn every_server_to_cli_variant_has_a_pinned_scope() {
        let table = classification(Uuid::new_v4(), Uuid::new_v4());
        let covered: BTreeSet<String> =
            table.iter().map(|(message, _)| wire_tag(message)).collect();
        assert_eq!(
            covered,
            every_server_to_cli_tag(),
            "every ServerToCli variant needs a row in this table"
        );
        for (message, expected) in &table {
            assert_eq!(message.scope(), *expected, "{}", wire_tag(message));
        }
        let classes: BTreeSet<&str> = table
            .iter()
            .map(|(_, scope)| match scope {
                ServerToCliScope::Pane(_) => "pane",
                ServerToCliScope::NewPane { .. } => "new_pane",
                ServerToCliScope::Project => "project",
                ServerToCliScope::Process => "process",
                ServerToCliScope::Unscoped => "unscoped",
                ServerToCliScope::Connection => "connection",
            })
            .collect();
        assert_eq!(classes.len(), 6, "every class is exercised: {classes:?}");
    }

    /// Approve, reject and typed input reach the CLI as `Input`. With a pane
    /// it must go to that pane's machine; without one the server resolves the
    /// target itself, and falling back to "the pane named 0" would be wrong.
    #[test]
    fn input_is_pane_scoped_only_when_it_names_a_pane() {
        let session_id = Uuid::new_v4();
        let to_pane = ServerToCli::Input {
            session_id,
            data: "n".into(),
            pane_id: Some(42),
        };
        let unaddressed = ServerToCli::Input {
            session_id,
            data: "n".into(),
            pane_id: None,
        };
        assert_eq!(to_pane.scope(), ServerToCliScope::Pane(42));
        assert_eq!(unaddressed.scope(), ServerToCliScope::Unscoped);
    }

    /// The legacy deadloop controls name no pane, yet the CLI applies them to
    /// `PANE_ID_DEADLOOP` alone. Routed as process messages they would reach
    /// the primary even when another machine owns that pane.
    #[test]
    fn legacy_deadloop_controls_follow_the_deadloop_pane() {
        let session_id = Uuid::new_v4();
        assert_eq!(
            ServerToCli::PauseDeadloop { session_id }.scope(),
            ServerToCliScope::Pane(PANE_ID_DEADLOOP)
        );
        assert_eq!(
            ServerToCli::ResumeDeadloop { session_id }.scope(),
            ServerToCliScope::Pane(PANE_ID_DEADLOOP)
        );
    }

    /// The instance that receives `AddPane` tags, persists and runs the pane,
    /// so the target comes from the config the server tagged.
    #[test]
    fn new_panes_go_to_the_machine_the_server_tagged() {
        let session_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();
        let add = |pane_config| ServerToCli::AddPane {
            session_id,
            pane_config,
            isolated_worktree: true,
            initial_input: None,
        };
        assert_eq!(
            add(pane(5, Some(machine_id))).scope(),
            ServerToCliScope::NewPane {
                machine_id: Some(machine_id)
            }
        );
        assert_eq!(
            add(pane(5, None)).scope(),
            ServerToCliScope::NewPane { machine_id: None }
        );
    }

    #[test]
    fn capability_wire_value_is_stable() {
        assert_eq!(PROJECT_INSTANCES_CAPABILITY, "project_instances_v1");
    }

    /// `.apas` and the server's `panes.json` hold `PaneConfig` directly, so an
    /// untagged pane must neither gain keys on rewrite nor fail to load.
    #[test]
    fn pane_machine_tags_are_optional_and_omitted_when_unset() {
        let legacy: PaneConfig = serde_json::from_value(json!({
            "pane_id": 3,
            "provider": "codex",
            "mode": "interactive",
            "kind": "terminal",
            "session_id": Uuid::new_v4(),
        }))
        .unwrap();
        assert_eq!(legacy.machine_id, None);
        assert_eq!(legacy.hostname, None);
        let written = serde_json::to_value(&legacy).unwrap();
        assert!(written.get("machine_id").is_none(), "{written}");
        assert!(written.get("hostname").is_none(), "{written}");

        let machine_id = Uuid::new_v4();
        let tagged = PaneConfig {
            machine_id: Some(machine_id),
            hostname: Some("zoo-006".into()),
            ..legacy
        };
        let decoded: PaneConfig =
            serde_json::from_value(serde_json::to_value(&tagged).unwrap()).unwrap();
        assert_eq!(decoded.machine_id, Some(machine_id));
        assert_eq!(decoded.hostname.as_deref(), Some("zoo-006"));

        let summary: MobilePaneSummary = serde_json::from_value(json!({
            "pane_id": 3,
            "kind": "terminal",
            "provider": "codex",
        }))
        .unwrap();
        assert_eq!(summary.machine_id, None);
        assert_eq!(summary.hostname, None);
        let written = serde_json::to_value(&summary).unwrap();
        assert!(written.get("machine_id").is_none(), "{written}");
        assert!(written.get("hostname").is_none(), "{written}");
    }

    #[test]
    fn add_pane_targets_a_machine_and_correlates_its_result() {
        let session_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();
        let legacy: WebToServer = serde_json::from_value(json!({
            "type": "add_pane",
            "session_id": session_id,
            "provider": "claude",
            "mode": "interactive",
            "kind": "terminal",
        }))
        .unwrap();
        match legacy {
            WebToServer::AddPane {
                machine_id,
                request_id,
                ..
            } => {
                assert_eq!(machine_id, None, "an older client targets the primary");
                assert_eq!(request_id, None);
            }
            other => panic!("expected AddPane, got {other:?}"),
        }
        let targeted: WebToServer = serde_json::from_value(json!({
            "type": "add_pane",
            "session_id": session_id,
            "provider": "claude",
            "mode": "interactive",
            "kind": "terminal",
            "machine_id": machine_id,
            "request_id": "add-1",
        }))
        .unwrap();
        match targeted {
            WebToServer::AddPane {
                machine_id: got,
                request_id,
                ..
            } => {
                assert_eq!(got, Some(machine_id));
                assert_eq!(request_id.as_deref(), Some("add-1"));
            }
            other => panic!("expected AddPane, got {other:?}"),
        }

        for (status, wire) in [
            (AddPaneStatus::Routed, "routed"),
            (AddPaneStatus::Starting, "starting"),
            (AddPaneStatus::Failed, "failed"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), json!(wire));
        }
        let starting = ServerToWeb::AddPaneResult {
            session_id,
            request_id: "add-1".into(),
            status: AddPaneStatus::Starting,
            machine_id: Some(machine_id),
            pane_id: None,
            error: None,
        };
        let value = serde_json::to_value(&starting).unwrap();
        assert_eq!(value["type"], "add_pane_result");
        assert_eq!(value["status"], "starting");
        assert!(value.get("pane_id").is_none(), "{value}");
        assert!(value.get("error").is_none(), "{value}");
        match serde_json::from_value::<ServerToWeb>(value).unwrap() {
            ServerToWeb::AddPaneResult {
                request_id,
                status,
                machine_id: got,
                ..
            } => {
                assert_eq!(request_id, "add-1");
                assert_eq!(status, AddPaneStatus::Starting);
                assert_eq!(got, Some(machine_id));
            }
            other => panic!("expected AddPaneResult, got {other:?}"),
        }
    }

    #[test]
    fn session_instances_are_absent_from_older_servers_and_explicit_when_live() {
        let session_id = Uuid::new_v4();
        let legacy: SessionInfo = serde_json::from_value(json!({
            "id": session_id,
            "cli_client_id": null,
            "working_dir": "/home/u/project",
            "hostname": "zoo-005",
            "status": "active",
            "created_at": null,
        }))
        .unwrap();
        assert!(legacy.instances.is_empty());
        assert!(serde_json::to_value(&legacy)
            .unwrap()
            .get("instances")
            .is_none());

        let secondary: SessionInstanceInfo = serde_json::from_value(json!({
            "machine_id": Uuid::new_v4(),
            "hostname": "zoo-006",
        }))
        .unwrap();
        assert!(!secondary.primary, "primary is opt-in, never assumed");
        assert_eq!(secondary.working_dir, None);
        assert_eq!(
            secondary.runtime_access, None,
            "unevaluated access is unknown, not denied"
        );
        let primary = SessionInstanceInfo {
            machine_id: Some(Uuid::new_v4()),
            hostname: "zoo-005".into(),
            working_dir: Some("/home/u/project".into()),
            primary: true,
            runtime_access: Some(false),
        };

        // The mobile summary flattens SessionInfo; the new list must survive it.
        let summary = MobileSessionSummary {
            session: SessionInfo {
                instances: vec![primary.clone(), secondary.clone()],
                ..legacy
            },
            project_name: None,
            latest_update_at: None,
            last_user_input_at: None,
            latest_summary: None,
            attention_count: 0,
        };
        let decoded: MobileSessionSummary =
            serde_json::from_value(serde_json::to_value(&summary).unwrap()).unwrap();
        assert_eq!(decoded.session.instances, vec![primary, secondary]);

        // The live set is complete, so "nothing attached" is an explicit [].
        let none_attached = ServerToWeb::SessionInstances {
            session_id,
            instances: Vec::new(),
        };
        let value = serde_json::to_value(&none_attached).unwrap();
        assert_eq!(value["type"], "session_instances");
        assert_eq!(value["instances"], json!([]));
    }

    /// A CLI that reports no machine is still attached; listing it keeps the
    /// set complete instead of making a live session look empty.
    #[test]
    fn an_attachment_without_a_machine_is_listed_without_one() {
        let legacy_only = SessionInstanceInfo {
            machine_id: None,
            hostname: "laptop".into(),
            working_dir: Some("/home/u/project".into()),
            primary: true,
            runtime_access: None,
        };
        let value = serde_json::to_value(&legacy_only).unwrap();
        assert!(value.get("machine_id").is_none(), "{value}");
        assert!(value.get("runtime_access").is_none(), "{value}");
        let decoded: SessionInstanceInfo = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, legacy_only);
    }

    #[test]
    fn lifecycle_reports_name_their_instance_when_known() {
        let session_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();
        let legacy: ServerToWeb = serde_json::from_value(json!({
            "type": "cli_lifecycle_inventory",
            "session_id": session_id,
            "inventory": { "persistent_terminal_hosting": true, "panes": [] },
        }))
        .unwrap();
        match legacy {
            ServerToWeb::CliLifecycleInventory { machine_id, .. } => assert_eq!(machine_id, None),
            other => panic!("expected CliLifecycleInventory, got {other:?}"),
        }
        let tagged = ServerToWeb::CliLifecycleStatus {
            session_id,
            request_id: Uuid::new_v4(),
            operation: CliLifecycleOperation::RebootCli,
            phase: CliLifecyclePhase::Accepted,
            message: None,
            inventory: None,
            machine_id: Some(machine_id),
        };
        let value = serde_json::to_value(&tagged).unwrap();
        assert_eq!(value["machine_id"], json!(machine_id));
        match serde_json::from_value::<ServerToWeb>(value).unwrap() {
            ServerToWeb::CliLifecycleStatus {
                machine_id: got, ..
            } => {
                assert_eq!(got, Some(machine_id))
            }
            other => panic!("expected CliLifecycleStatus, got {other:?}"),
        }
    }

    /// The project view's Start/Close act on every instance; the Machines
    /// page's per-machine buttons, and every older client, act on one.
    #[test]
    fn project_wide_start_and_stop_are_opt_in() {
        let machine_id = Uuid::new_v4();
        for kind in ["start_machine_project_cli", "stop_machine_project_cli"] {
            let legacy: WebToServer = serde_json::from_value(json!({
                "type": kind,
                "machine_id": machine_id,
                "project_id": "p",
            }))
            .unwrap();
            let all = match legacy {
                WebToServer::StartMachineProjectCli { all_instances, .. }
                | WebToServer::StopMachineProjectCli { all_instances, .. } => all_instances,
                other => panic!("expected a machine project control, got {other:?}"),
            };
            assert!(!all, "{kind}: an older client means one machine");

            let project_wide: WebToServer = serde_json::from_value(json!({
                "type": kind,
                "machine_id": machine_id,
                "project_id": "p",
                "all_instances": true,
            }))
            .unwrap();
            let all = match project_wide {
                WebToServer::StartMachineProjectCli { all_instances, .. }
                | WebToServer::StopMachineProjectCli { all_instances, .. } => all_instances,
                other => panic!("expected a machine project control, got {other:?}"),
            };
            assert!(all, "{kind}");
        }
    }

    /// A start the server waits on is correlated; an uncorrelated one, which
    /// is every start an older server sends, asks for no report.
    #[test]
    fn daemon_starts_can_be_correlated_with_their_outcome() {
        let legacy: ServerToDaemon = serde_json::from_value(json!({
            "type": "start_project_cli",
            "project_id": "p",
        }))
        .unwrap();
        match legacy {
            ServerToDaemon::StartProjectCli { request_id, .. } => assert_eq!(request_id, None),
            other => panic!("expected StartProjectCli, got {other:?}"),
        }
        let request_id = Uuid::new_v4();
        let correlated = ServerToDaemon::StartProjectCli {
            project_id: "p".into(),
            policy: None,
            request_id: Some(request_id),
        };
        let value = serde_json::to_value(&correlated).unwrap();
        assert_eq!(value["request_id"], json!(request_id));

        let refused = DaemonToServer::ProjectRuntimeStarted {
            request_id,
            project_id: "p".into(),
            success: false,
            error: Some("project is running on zoo-005".into()),
        };
        let value = serde_json::to_value(&refused).unwrap();
        assert_eq!(value["type"], "project_runtime_started");
        match serde_json::from_value::<DaemonToServer>(value).unwrap() {
            DaemonToServer::ProjectRuntimeStarted {
                request_id: got,
                success,
                error,
                ..
            } => {
                assert_eq!(got, request_id);
                assert!(!success);
                assert!(error.is_some());
            }
            other => panic!("expected ProjectRuntimeStarted, got {other:?}"),
        }
        let started: DaemonToServer = serde_json::from_value(json!({
            "type": "project_runtime_started",
            "request_id": request_id,
            "project_id": "p",
            "success": true,
        }))
        .unwrap();
        match started {
            DaemonToServer::ProjectRuntimeStarted { success, error, .. } => {
                assert!(success);
                assert_eq!(error, None);
            }
            other => panic!("expected ProjectRuntimeStarted, got {other:?}"),
        }
    }

    #[test]
    fn machine_entries_report_instance_support_and_canonical_remote() {
        let machine_id = Uuid::new_v4();
        let legacy: MachineWithProjects = serde_json::from_value(json!({
            "machine": {
                "machine_id": machine_id,
                "hostname": "zoo-005",
                "os": "linux",
                "arch": "x86_64",
            },
            "projects": [{ "project_id": "p", "path": "/home/u/p" }],
        }))
        .unwrap();
        assert!(!legacy.project_instances_available);
        assert!(
            !legacy.daemon_connected,
            "an older server reports no connection"
        );
        assert_eq!(legacy.projects[0].git_remote, None);
        let written = serde_json::to_value(&legacy).unwrap();
        assert!(
            written["projects"][0].get("git_remote").is_none(),
            "{written}"
        );
        assert_eq!(written["project_instances_available"], json!(false));

        let current: MachineWithProjects = serde_json::from_value(json!({
            "machine": {
                "machine_id": machine_id,
                "hostname": "zoo-005",
                "os": "linux",
                "arch": "x86_64",
            },
            "projects": [{
                "project_id": "p",
                "path": "/home/u/p",
                "git_remote": "github.com/owner/repo",
            }],
            "project_instances_available": true,
            "daemon_connected": true,
        }))
        .unwrap();
        assert!(current.project_instances_available);
        assert!(current.daemon_connected);
        assert_eq!(
            current.projects[0].git_remote.as_deref(),
            Some("github.com/owner/repo")
        );
    }

    #[test]
    fn process_controls_without_a_machine_target_the_primary() {
        let session_id = Uuid::new_v4();
        let request_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();
        let legacy_reboot = format!(r#"{{"type":"reboot_cli","session_id":"{session_id}"}}"#);
        match serde_json::from_str::<WebToServer>(&legacy_reboot).unwrap() {
            WebToServer::RebootCli { machine_id, .. } => assert_eq!(machine_id, None),
            other => panic!("expected RebootCli, got {other:?}"),
        }
        let legacy_lifecycle = format!(
            r#"{{"type":"cli_lifecycle_request","session_id":"{session_id}","request_id":"{request_id}","operation":"reboot_cli"}}"#
        );
        match serde_json::from_str::<WebToServer>(&legacy_lifecycle).unwrap() {
            WebToServer::CliLifecycleRequest { machine_id, .. } => assert_eq!(machine_id, None),
            other => panic!("expected CliLifecycleRequest, got {other:?}"),
        }

        let targeted = WebToServer::CliLifecycleRequest {
            session_id,
            request_id,
            operation: Some(CliLifecycleOperation::RebootCli),
            machine_id: Some(machine_id),
        };
        match serde_json::from_value::<WebToServer>(serde_json::to_value(&targeted).unwrap())
            .unwrap()
        {
            WebToServer::CliLifecycleRequest {
                machine_id: got, ..
            } => {
                assert_eq!(got, Some(machine_id))
            }
            other => panic!("expected CliLifecycleRequest, got {other:?}"),
        }
        let untargeted = WebToServer::RebootCli {
            session_id: Some(session_id),
            machine_id: None,
        };
        assert!(!serde_json::to_string(&untargeted)
            .unwrap()
            .contains("machine_id"));
    }

    #[test]
    fn clone_to_join_is_opt_in_on_both_hops() {
        let web: WebToServer = serde_json::from_value(json!({
            "type": "create_project_instance",
            "machine_id": Uuid::new_v4(),
            "git_remote": "github.com/owner/repo",
            "instance_name": "repo",
            "branch": "apas/new",
        }))
        .unwrap();
        match web {
            WebToServer::CreateProjectInstance {
                join_project_id, ..
            } => {
                assert_eq!(join_project_id, None)
            }
            other => panic!("expected CreateProjectInstance, got {other:?}"),
        }

        let daemon: ServerToDaemon = serde_json::from_value(json!({
            "type": "create_project_instance",
            "git_remote": "github.com/owner/repo",
            "instance_name": "repo",
            "branch": "apas/new",
        }))
        .unwrap();
        match daemon {
            ServerToDaemon::CreateProjectInstance {
                join_existing,
                project_id,
                ..
            } => {
                assert!(!join_existing);
                assert_eq!(project_id, None);
            }
            other => panic!("expected CreateProjectInstance, got {other:?}"),
        }

        let project_id = Uuid::new_v4().to_string();
        let join = ServerToDaemon::CreateProjectInstance {
            git_remote: "github.com/owner/repo".into(),
            instance_name: "repo".into(),
            branch: String::new(),
            clone_url: Some("https://github.com/owner/repo.git".into()),
            base_path: None,
            request_id: Some("clone-1".into()),
            project_id: Some(project_id.clone()),
            provisioning_mode: ProjectProvisioningMode::TrustedOwner,
            join_existing: true,
        };
        match serde_json::from_value::<ServerToDaemon>(serde_json::to_value(&join).unwrap())
            .unwrap()
        {
            ServerToDaemon::CreateProjectInstance {
                join_existing,
                project_id: got,
                ..
            } => {
                assert!(join_existing);
                assert_eq!(got.as_deref(), Some(project_id.as_str()));
            }
            other => panic!("expected CreateProjectInstance, got {other:?}"),
        }
    }

    #[test]
    fn forget_machine_panes_names_the_session_and_machine_on_both_hops() {
        let session_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();
        let web = WebToServer::ForgetMachinePanes {
            session_id,
            machine_id,
        };
        assert_eq!(wire_tag(&web), "forget_machine_panes");
        match serde_json::from_value::<WebToServer>(serde_json::to_value(&web).unwrap()).unwrap() {
            WebToServer::ForgetMachinePanes {
                session_id: s,
                machine_id: m,
            } => {
                assert_eq!((s, m), (session_id, machine_id))
            }
            other => panic!("expected ForgetMachinePanes, got {other:?}"),
        }
        let cli = ServerToCli::ForgetMachinePanes {
            session_id,
            machine_id,
        };
        assert_eq!(wire_tag(&cli), "forget_machine_panes");
        match serde_json::from_value::<ServerToCli>(serde_json::to_value(&cli).unwrap()).unwrap() {
            ServerToCli::ForgetMachinePanes {
                session_id: s,
                machine_id: m,
            } => {
                assert_eq!((s, m), (session_id, machine_id))
            }
            other => panic!("expected ForgetMachinePanes, got {other:?}"),
        }
    }

    #[test]
    fn projects_by_remote_response_defaults_unknown_details() {
        let project_id = Uuid::new_v4();
        let response: ProjectsByRemoteResponse = serde_json::from_value(json!({
            "git_remote": "github.com/owner/repo",
            "projects": [{
                "project_id": project_id,
                "instances": [{ "hostname": "laptop", "path": "/home/u/repo" }],
            }],
        }))
        .unwrap();
        let found = &response.projects[0];
        assert_eq!(found.project_id, project_id);
        assert_eq!(found.name, None);
        assert!(
            !found.has_session,
            "a never-started registration has no session"
        );
        assert_eq!(found.instances[0].machine_id, None);
        assert!(!found.instances[0].running);

        assert!(response.local.is_none(), "no project_id was asked about");

        let none: ProjectsByRemoteResponse =
            serde_json::from_value(json!({ "git_remote": "github.com/owner/other" })).unwrap();
        assert!(none.projects.is_empty());
        let written = serde_json::to_value(&none).unwrap();
        assert_eq!(
            written["projects"],
            json!([]),
            "an empty match list is explicit"
        );
        assert!(written.get("local").is_none(), "{written}");
    }

    /// A local project whose sessions never recorded the remote matches
    /// nothing, yet has history; `local` is how the CLI learns that.
    #[test]
    fn projects_by_remote_answers_for_the_local_identity_whatever_its_remote() {
        let local_id = Uuid::new_v4();
        let response: ProjectsByRemoteResponse = serde_json::from_value(json!({
            "git_remote": "github.com/owner/repo",
            "projects": [],
            "local": {
                "project_id": local_id,
                "has_session": true,
                "instances": [],
            },
        }))
        .unwrap();
        assert!(response.projects.is_empty());
        let local = response.local.expect("local identity");
        assert_eq!(local.project_id, local_id);
        assert!(local.has_session, "history must block re-identification");
    }
}
