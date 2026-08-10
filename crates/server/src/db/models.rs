use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ClusterRole {
    Admin,
    #[default]
    User,
}

impl ClusterRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::User => "user",
        }
    }

    pub fn parse(raw: &str) -> Self {
        if raw.trim().eq_ignore_ascii_case("admin") {
            Self::Admin
        } else {
            Self::User
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    Suspended,
    #[default]
    Active,
}

impl AccountStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suspended => "suspended",
        }
    }

    pub fn parse(raw: &str) -> Self {
        if raw.trim().eq_ignore_ascii_case("suspended") {
            Self::Suspended
        } else {
            Self::Active
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProjectLifecycle {
    Suspended,
    /// Internal, irreversible cleanup state. This value is never selectable
    /// from the cluster-administration lifecycle API.
    Deleting,
    #[default]
    Active,
}

impl ProjectLifecycle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Deleting => "deleting",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "active" => Self::Active,
            "suspended" => Self::Suspended,
            "deleting" => Self::Deleting,
            // Unknown persisted lifecycle values must fail closed. Treating
            // them as active would reopen a project written by newer code.
            _ => Self::Deleting,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct User {
    pub id: String,
    pub email: String,
    pub password_hash: String,
    pub created_at: Option<String>,
    #[sqlx(default)]
    pub cluster_role: String,
    #[sqlx(default)]
    pub account_status: String,
}

impl User {
    pub fn role(&self) -> ClusterRole {
        ClusterRole::parse(&self.cluster_role)
    }

    pub fn status(&self) -> AccountStatus {
        AccountStatus::parse(&self.account_status)
    }

    pub fn is_active(&self) -> bool {
        self.status() == AccountStatus::Active
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct MobileDeviceSessionRecord {
    pub id: String,
    pub user_id: String,
    pub installation_id: String,
    pub platform: String,
    pub device_name: Option<String>,
    pub app_version: String,
    pub created_at: String,
    pub last_used_at: String,
    pub expires_at: String,
    pub revoked_at: Option<String>,
    pub revocation_reason: Option<String>,
}

impl MobileDeviceSessionRecord {
    pub fn is_active(&self) -> bool {
        self.revoked_at.is_none()
            && chrono::DateTime::parse_from_rfc3339(&self.expires_at)
                .map(|expires| expires.with_timezone(&chrono::Utc) > chrono::Utc::now())
                .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileRefreshFailure {
    Invalid,
    Expired,
    Revoked,
    Reused,
    InstallationMismatch,
}

#[derive(Debug, Clone, FromRow)]
pub struct MobileNotificationDeliveryRecord {
    pub id: i64,
    pub event_id: String,
    pub push_token_id: String,
    pub token: String,
    pub category: String,
    pub routing_id: String,
    pub session_id: Option<String>,
    pub attempt_count: i64,
    pub provider_ticket_id: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct MobileTaskLaunchRecord {
    pub request_id: String,
    pub user_id: String,
    pub device_session_id: String,
    pub request_fingerprint: String,
    pub machine_id: String,
    pub project_id: String,
    pub status: String,
    pub session_id: Option<String>,
    pub pane_id: Option<i64>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MobileAppVersionCount {
    pub app_version: String,
    pub active_device_sessions: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MobilePersistenceMetrics {
    pub active_device_sessions: i64,
    pub active_push_tokens: i64,
    pub pending_task_launches: i64,
    pub outbox_queued: i64,
    pub outbox_sending: i64,
    pub outbox_ticketed: i64,
    pub outbox_retry: i64,
    pub outbox_permanent_failure: i64,
    pub app_versions: Vec<MobileAppVersionCount>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Project {
    pub id: String,
    pub owner_user_id: String,
    pub lifecycle_status: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl Project {
    pub fn lifecycle(&self) -> ProjectLifecycle {
        ProjectLifecycle::parse(&self.lifecycle_status)
    }
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct ProjectMember {
    pub project_id: String,
    pub user_id: String,
    pub invited_by: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ClusterInvitation {
    pub code: String,
    pub email: String,
    pub created_by: String,
    pub expires_at: String,
    pub redeemed_at: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct AdminAuditEvent {
    pub id: i64,
    pub actor_user_id: String,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub project_id: Option<String>,
    pub details: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDeletionManifest {
    pub project_id: String,
    pub owner_user_id: String,
    pub session_ids: Vec<String>,
    pub affected_user_ids: Vec<String>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct ClusterUserSummary {
    pub id: String,
    pub email: String,
    pub cluster_role: String,
    pub account_status: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct ProjectMemberInfo {
    pub user_id: String,
    pub email: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminProjectSummary {
    pub id: String,
    pub project_name: Option<String>,
    pub hostname: Option<String>,
    pub owner_user_id: String,
    pub owner_email: String,
    pub lifecycle_status: String,
    pub member_count: i64,
    pub active_session_count: i64,
    pub last_activity: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectPolicyOverride {
    pub project_id: String,
    pub team_available: Option<bool>,
    pub allowed_launch_profiles: Option<Vec<String>>,
    pub version: i64,
    pub legacy_imported: bool,
    pub legacy_conflict: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct CliClient {
    pub id: String,
    pub user_id: String,
    pub name: Option<String>,
    pub last_seen: Option<String>,
    pub status: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub cli_client_id: Option<String>,
    pub working_dir: Option<String>,
    pub hostname: Option<String>,
    pub status: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    #[sqlx(default)]
    pub is_paused: bool,
    /// Stable project identity sourced from the CLI's `.apas` file. Backfilled
    /// to `id` for sessions that pre-date the column, so old rows still group
    /// one-session-per-project.
    #[sqlx(default)]
    pub project_id: Option<String>,
    /// Canonical `host/owner/repo` of the project's git `origin` remote, sent by
    /// the CLI in SessionStart. The web sidebar groups sessions by this. `None`
    /// for rows that pre-date the column or projects with no remote.
    #[sqlx(default)]
    pub git_remote: Option<String>,
    /// Raw `origin` URL (cloneable) for this project's repo, sent by the CLI.
    /// Surfaced to the web to prefill the clone URL when creating an instance.
    #[sqlx(default)]
    pub git_remote_url: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub message_type: String,
    pub metadata: Option<String>,
    pub created_at: Option<String>,
}

/// One day-bucketed usage row for a (session, pane). The Overview's
/// lifetime/7-day/today windows are derived by aggregating these rows.
#[derive(Debug, Clone, FromRow)]
pub struct PaneUsageDayRow {
    pub pane_id: i64,
    pub day: String,
    #[sqlx(default)]
    pub prompt_count: i64,
    #[sqlx(default)]
    pub input_tokens: i64,
    #[sqlx(default)]
    pub output_tokens: i64,
    #[sqlx(default)]
    pub cache_read_tokens: i64,
    #[sqlx(default)]
    pub cache_creation_tokens: i64,
    #[sqlx(default)]
    pub total_cost_usd: f64,
    #[sqlx(default)]
    pub num_responses: i64,
    #[sqlx(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct SessionShare {
    pub id: i64,
    pub session_id: String,
    pub user_id: String,
    pub invited_by: String,
    pub role: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct InvitationCode {
    pub code: String,
    pub session_id: String,
    #[sqlx(default)]
    pub project_id: Option<String>,
    pub created_by: String,
    pub expires_at: String,
    pub redeemed_by: Option<String>,
    pub redeemed_at: Option<String>,
    pub created_at: Option<String>,
}
