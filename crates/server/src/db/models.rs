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

#[derive(Debug, Clone, FromRow)]
pub struct SessionShare {
    pub id: i64,
    pub session_id: String,
    pub user_id: String,
    pub invited_by: String,
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
