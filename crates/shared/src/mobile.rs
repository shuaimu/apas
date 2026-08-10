//! Versioned public DTOs used by the APAS mobile companion.
//!
//! These types intentionally contain no server-only state. They are exported
//! to JSON Schema and generated into `packages/protocol`, making Rust the
//! source of truth for both native and web TypeScript consumers.

use crate::{MachineWithProjects, PaneKind, PaneMode, Provider, SessionInfo};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const MOBILE_PROTOCOL_MIN_VERSION: u32 = 1;
pub const MOBILE_PROTOCOL_MAX_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Web,
    Mobile,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MobileFeatureFlags {
    #[serde(default)]
    pub bootstrap: bool,
    #[serde(default)]
    pub coding_mutations: bool,
    #[serde(default)]
    pub terminal: bool,
    #[serde(default)]
    pub notifications: bool,
    #[serde(default)]
    pub deep_links: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MobileLoginRequest {
    pub email: String,
    pub password: String,
    pub installation_id: String,
    pub platform: MobilePlatform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    pub app_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MobileRefreshRequest {
    pub refresh_token: String,
    pub installation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MobileLogoutRequest {
    pub refresh_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MobileAuthResponse {
    pub access_token: String,
    pub access_expires_at: String,
    pub refresh_token: String,
    pub refresh_expires_at: String,
    pub device_session_id: Uuid,
    pub user_id: Uuid,
    pub user_email: String,
    pub cluster_role: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MobilePlatform {
    Ios,
    Android,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MobileDeviceSession {
    pub id: Uuid,
    pub installation_id: String,
    pub platform: MobilePlatform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    pub app_version: String,
    pub created_at: String,
    pub last_used_at: String,
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MobileBootstrapResponse {
    pub user_id: Uuid,
    pub user_email: String,
    pub cluster_role: String,
    pub account_status: String,
    pub protocol_min_version: u32,
    pub protocol_max_version: u32,
    pub features: MobileFeatureFlags,
    pub sessions: Vec<MobileSessionSummary>,
    pub machines: Vec<MachineWithProjects>,
    pub launch_targets: Vec<MobileLaunchTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MobileSessionSummary {
    #[serde(flatten)]
    pub session: SessionInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_update_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_user_input_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_summary: Option<String>,
    #[serde(default)]
    pub attention_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MobileLaunchTarget {
    pub machine_id: Uuid,
    pub hostname: String,
    pub project_id: String,
    pub project_name: String,
    pub instance_path: String,
    pub online: bool,
    pub profiles: Vec<MobileLaunchProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MobileLaunchProfile {
    pub key: String,
    pub label: String,
    pub kind: PaneKind,
    pub provider: Provider,
    pub mode: PaneMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CodeEvent {
    pub id: String,
    pub session_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<u32>,
    pub ordering_key: String,
    pub created_at: String,
    pub kind: CodeEventKind,
    pub summary: String,
    #[serde(default)]
    pub requires_attention: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CodeEventKind {
    Instruction,
    AgentStatus,
    Tool,
    Question,
    Approval,
    Plan,
    Todo,
    Test,
    Diff,
    PullRequest,
    Terminal,
    Completed,
    Interrupted,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MobileTaskLaunchRequest {
    pub request_id: Uuid,
    pub machine_id: Uuid,
    pub project_id: String,
    pub instruction: String,
    pub profile_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MobileTaskLaunchResponse {
    pub request_id: Uuid,
    pub session_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<u32>,
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct MobileNotificationPreferences {
    #[serde(default)]
    pub decisions: bool,
    #[serde(default)]
    pub failures: bool,
    #[serde(default)]
    pub pull_requests: bool,
    #[serde(default)]
    pub completions: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MobilePushTokenRequest {
    pub installation_id: String,
    pub platform: MobilePlatform,
    pub token: String,
}
