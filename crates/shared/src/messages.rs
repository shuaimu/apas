use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// CLI <-> Server Messages
// ============================================================================

/// Messages sent from CLI client to server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CliToServer {
    /// CLI registers with the server using auth token
    Register { token: String },

    /// CLI starts a local session (hybrid mode)
    SessionStart {
        session_id: Uuid,
        working_dir: Option<String>,
    },

    /// Claude output to be forwarded to web client
    Output {
        session_id: Uuid,
        data: String,
        #[serde(default)]
        output_type: OutputType,
    },

    /// Session has ended
    SessionEnd { session_id: Uuid, reason: String },

    /// Heartbeat to keep connection alive
    Heartbeat,

    /// Structured message from Claude CLI stream-json output
    StreamMessage {
        session_id: Uuid,
        message: ClaudeStreamMessage,
    },

    /// User input/prompt from CLI (to be displayed in web UI)
    UserInput {
        session_id: Uuid,
        text: String,
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

    /// New session assigned to this CLI
    SessionAssigned { session_id: Uuid, working_dir: Option<String> },

    /// User input from web client
    Input { session_id: Uuid, data: String },

    /// Signal to send to Claude process (e.g., SIGINT)
    Signal { session_id: Uuid, signal: String },

    /// Session disconnected from web
    SessionDisconnected { session_id: Uuid },

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

    /// Start a new session (optionally specify CLI client)
    StartSession { cli_client_id: Option<Uuid> },

    /// Resume an existing session
    ResumeSession { session_id: Uuid },

    /// Attach to observe an existing CLI session (hybrid mode)
    AttachSession { session_id: Uuid },

    /// User input to send to Claude
    Input { text: String },

    /// Approve a tool call
    Approve { tool_call_id: String },

    /// Reject a tool call
    Reject { tool_call_id: String },

    /// Send signal (e.g., cancel/interrupt)
    Signal { signal: String },

    /// List all sessions (persisted)
    ListSessions,

    /// Get messages for a specific session
    GetSessionMessages { session_id: Uuid },
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
    SessionStarted { session_id: Uuid },

    /// Session status update
    SessionStatus { status: SessionStatus },

    /// Output from Claude
    Output {
        content: String,
        #[serde(default)]
        output_type: OutputType,
    },

    /// Error message
    Error { message: String },

    /// List of available CLI clients
    CliClients { clients: Vec<CliClientInfo> },

    /// Structured message from Claude CLI stream-json output
    StreamMessage {
        session_id: Uuid,
        message: ClaudeStreamMessage,
    },

    /// List of persisted sessions
    Sessions { sessions: Vec<SessionInfo> },

    /// Messages for a session
    SessionMessages { session_id: Uuid, messages: Vec<MessageInfo> },

    /// User input/prompt from CLI (displayed in web UI)
    UserInput {
        session_id: Uuid,
        text: String,
    },
}

/// Information about a persisted session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: Uuid,
    pub cli_client_id: Option<Uuid>,
    pub working_dir: Option<String>,
    pub status: String,
    pub created_at: Option<String>,
}

/// Information about a persisted message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageInfo {
    pub id: String,
    pub role: String,
    pub content: String,
    pub message_type: String,
    pub created_at: Option<String>,
}

// ============================================================================
// Shared Types
// ============================================================================

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
        }
    }

    pub fn output_with_type(session_id: Uuid, data: impl Into<String>, output_type: OutputType) -> Self {
        Self::Output {
            session_id,
            data: data.into(),
            output_type,
        }
    }
}

impl ServerToWeb {
    pub fn output(content: impl Into<String>) -> Self {
        Self::Output {
            content: content.into(),
            output_type: OutputType::Text,
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
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"register\""));
        assert!(json.contains("\"token\":\"test-token\""));

        let deserialized: CliToServer = serde_json::from_str(&json).unwrap();
        match deserialized {
            CliToServer::Register { token } => assert_eq!(token, "test-token"),
            _ => panic!("Expected Register variant"),
        }
    }

    #[test]
    fn test_cli_to_server_session_start_serialization() {
        let session_id = Uuid::new_v4();
        let msg = CliToServer::SessionStart {
            session_id,
            working_dir: Some("/home/user/project".to_string()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"session_start\""));
        assert!(json.contains(&session_id.to_string()));

        let deserialized: CliToServer = serde_json::from_str(&json).unwrap();
        match deserialized {
            CliToServer::SessionStart { session_id: sid, working_dir } => {
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
            CliToServer::Output { session_id: sid, data, output_type } => {
                assert_eq!(sid, session_id);
                assert_eq!(data, "Hello, world!");
                assert_eq!(output_type, OutputType::Text);
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
            ServerToWeb::Output { content, output_type } => {
                assert_eq!(content, "Test output");
                assert_eq!(output_type, OutputType::Text);
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
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"stream_message\""));
        assert!(json.contains(&session_id.to_string()));
    }
}
