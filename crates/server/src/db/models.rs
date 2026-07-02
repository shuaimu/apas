use sqlx::FromRow;

#[derive(Debug, Clone, FromRow)]
pub struct User {
    pub id: String,
    pub email: String,
    pub password_hash: String,
    pub created_at: Option<String>,
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
    pub created_by: String,
    pub expires_at: String,
    pub redeemed_by: Option<String>,
    pub redeemed_at: Option<String>,
    pub created_at: Option<String>,
}
