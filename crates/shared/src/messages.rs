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
    DeadloopStatus {
        session_id: Uuid,
        is_paused: bool,
    },

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

    /// Report usage limits from the Anthropic API
    UsageLimits { limits: UsageLimits },
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

    /// New session assigned to this CLI
    SessionAssigned { session_id: Uuid, working_dir: Option<String> },

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
    AddPane { session_id: Uuid, pane_config: PaneConfig },

    /// Remove a pane from the session
    RemovePane { session_id: Uuid, pane_id: u32 },

    /// Reboot the CLI process
    RebootCli { session_id: Uuid },
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
    },

    /// Remove a pane
    RemovePane { pane_id: u32 },

    /// Reboot the CLI process
    RebootCli,

    /// Download all session data
    DownloadSession { session_id: Uuid },
}

/// Messages sent from server to web client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerToWeb {
    /// Authentication successful
    Authenticated { user_id: Uuid },

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
    DeadloopStatus {
        session_id: Uuid,
        is_paused: bool,
    },

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
        limits: UsageLimits,
    },

    /// Full session data for download
    SessionDownload {
        session_id: Uuid,
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Claude,
    Codex,
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
    pub prompt: Option<String>, // Custom prompt for deadloop
    #[serde(default)]
    pub label: Option<String>, // User-facing label like "Deadloop" or "Interactive"
}

/// Legacy pane_id constants
pub const PANE_ID_DEADLOOP: u32 = 1;
pub const PANE_ID_INTERACTIVE: u32 = 2;

impl PaneConfig {
    /// Create default pane configs (Claude deadloop + Claude interactive)
    pub fn defaults() -> Vec<PaneConfig> {
        vec![
            PaneConfig {
                pane_id: PANE_ID_DEADLOOP,
                provider: Provider::Claude,
                mode: PaneMode::Deadloop,
                session_id: Uuid::new_v4(),
                is_paused: false,
                prompt: None,
                label: Some("Deadloop".to_string()),
            },
            PaneConfig {
                pane_id: PANE_ID_INTERACTIVE,
                provider: Provider::Claude,
                mode: PaneMode::Interactive,
                session_id: Uuid::new_v4(),
                is_paused: false,
                prompt: None,
                label: Some("Interactive".to_string()),
            },
        ]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
}

/// Usage limits from the Anthropic API
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsageLimits {
    /// 5-hour rolling window usage
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<UsageLimitWindow>,
    /// 7-day (weekly) rolling window usage
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day: Option<UsageLimitWindow>,
    /// When the usage was last fetched (ISO 8601 timestamp)
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    Text {
        text: String,
    },
    /// Tool use request from Claude
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// Tool result (in user messages)
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
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

    pub fn output_with_type(session_id: Uuid, data: impl Into<String>, output_type: OutputType) -> Self {
        Self::Output {
            session_id,
            data: data.into(),
            output_type,
            pane_type: None,
            pane_id: None,
        }
    }

    pub fn output_with_pane(session_id: Uuid, data: impl Into<String>, pane_type: PaneType) -> Self {
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
            CliToServer::SessionStart { session_id: sid, working_dir, .. } => {
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
            CliToServer::Output { session_id: sid, data, output_type, pane_type, .. } => {
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
            ServerToWeb::Output { content, output_type, pane_type, .. } => {
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
            active_session: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"name\":\"my-laptop\""));
        assert!(json.contains("\"status\":\"online\""));

        let deserialized: CliClientInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, Some("my-laptop".to_string()));
        assert_eq!(deserialized.status, CliClientStatus::Online);
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
            ClaudeStreamMessage::System { subtype, tools, model, .. } => {
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
            ClaudeStreamMessage::Assistant { message, .. } => {
                match &message.content[0] {
                    ClaudeContentBlock::ToolUse { id, name, input } => {
                        assert_eq!(id, "tool-1");
                        assert_eq!(name, "Read");
                        assert_eq!(input["file_path"], "/tmp/test.txt");
                    }
                    _ => panic!("Expected ToolUse content block"),
                }
            }
            _ => panic!("Expected Assistant variant"),
        }
    }

    #[test]
    fn test_claude_stream_message_result() {
        let json = r#"{"type":"result","subtype":"success","result":"Done","total_cost_usd":0.05,"duration_ms":1000,"session_id":"abc-123","is_error":false}"#;
        let msg: ClaudeStreamMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClaudeStreamMessage::Result { subtype, result, total_cost_usd, is_error, .. } => {
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
