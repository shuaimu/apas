use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

    /// Pause the deadloop (legacy - use PausePane for new code)
    PauseDeadloop { session_id: Uuid },

    /// Resume the deadloop (legacy - use ResumePane for new code)
    ResumeDeadloop { session_id: Uuid },

    /// Pause a specific pane
    PausePane { session_id: Uuid, pane_id: u32 },

    /// Resume a specific pane
    ResumePane { session_id: Uuid, pane_id: u32 },

    /// Add a new pane to the session
    AddPane {
        session_id: Uuid,
        pane_config: PaneConfig,
    },

    /// Remove a pane from the session
    RemovePane { session_id: Uuid, pane_id: u32 },

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

    /// User input to send to Claude
    Input {
        text: String,
        #[serde(default)]
        pane_type: Option<PaneType>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
    },

    /// Approve a tool call
    Approve { tool_call_id: String },

    /// Reject a tool call
    Reject { tool_call_id: String },

    /// Send signal (e.g., cancel/interrupt)
    Signal { signal: String },

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
    },

    /// Pause the deadloop session (legacy - use PausePane for new code)
    PauseDeadloop,

    /// Resume the deadloop session (legacy - use ResumePane for new code)
    ResumeDeadloop,

    /// Pause a specific pane
    PausePane { pane_id: u32 },

    /// Resume a specific pane
    ResumePane { pane_id: u32 },

    /// Add a new pane
    AddPane {
        provider: Provider,
        mode: PaneMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },

    /// Remove a pane
    RemovePane { pane_id: u32 },

    /// Update a pane's custom label
    UpdatePaneLabel {
        pane_id: u32,
        label: String,
    },

    /// Update a pane's Claude thinking-effort override. Persisted in the
    /// project .apas so switching tabs doesn't reset it to default.
    UpdatePaneEffort {
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort: Option<String>,
    },

    /// Interrupt the agent process running for a pane (e.g. claude wedged on
    /// a long-running Bash tool call). The CLI signals SIGINT to its
    /// subprocess so the current turn aborts and queued input can flow.
    InterruptPane { pane_id: u32 },

    /// Reorder panes (array of pane_ids in desired order)
    ReorderPanes { pane_ids: Vec<u32> },

    /// Start bot (deadloop) on a pane — converts interactive pane to deadloop
    StartBot {
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_iteration_interval_minutes: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort: Option<String>,
    },

    /// Stop bot on a pane — converts deadloop pane back to interactive
    StopBot { pane_id: u32 },

    /// Reboot the CLI process
    RebootCli,

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

    /// Download all session data
    DownloadSession { session_id: Uuid },

    /// Submit answers to a pending AskUserQuestion tool call. The server
    /// relays this to the CLI which writes a control_response onto claude's
    /// stdin so the SDK's canUseTool callback completes with these answers.
    AnswerQuestion {
        tool_use_id: String,
        /// Question text → selected option label(s) joined with ", " for
        /// multi-select.
        answers: std::collections::HashMap<String, String>,
    },
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
    },

    /// List of persisted sessions
    Sessions { sessions: Vec<SessionInfo> },

    /// Messages for a session
    SessionMessages {
        session_id: Uuid,
        messages: Vec<MessageInfo>,
        #[serde(default)]
        has_more: bool, // True if there are older messages to load
    },

    /// User input/prompt from CLI (displayed in web UI)
    UserInput {
        session_id: Uuid,
        text: String,
        #[serde(default)]
        pane_type: Option<PaneType>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u32>,
    },

    /// Deadloop pause status update (legacy - use PanePaused for new code)
    DeadloopStatus { session_id: Uuid, is_paused: bool },

    /// Pane pause status update
    PanePaused {
        session_id: Uuid,
        pane_id: u32,
        is_paused: bool,
    },

    /// Pane status update (e.g., "thinking") for status bar display
    PaneStatus {
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

/// Configuration for a single pane
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneConfig {
    pub pane_id: u32,
    pub provider: Provider,
    pub mode: PaneMode,
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
    pub effort: Option<String>, // Optional Claude thinking effort override (e.g., "high", "max")
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
            session_id: Uuid::new_v4(),
            is_paused: false,
            stop_requested: false,
            prompt: None,
            min_iteration_interval_minutes: None,
            label: Some("Interactive".to_string()),
            model: None,
            effort: None,
            worktree_path: None,
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
            let (input_tokens, output_tokens) = usage
                .as_ref()
                .map(|u| (u.input_tokens, u.output_tokens))
                .unwrap_or((0, 0));
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
                extra: serde_json::Value::Null,
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
            pane_type: None,
            panes: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"session_start\""));
        assert!(json.contains(&session_id.to_string()));

        let deserialized: CliToServer = serde_json::from_str(&json).unwrap();
        match deserialized {
            CliToServer::SessionStart {
                session_id: sid,
                working_dir,
                ..
            } => {
                assert_eq!(sid, session_id);
                assert_eq!(working_dir, Some("/home/user/project".to_string()));
            }
            _ => panic!("Expected SessionStart variant"),
        }
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
}
