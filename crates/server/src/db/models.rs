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
