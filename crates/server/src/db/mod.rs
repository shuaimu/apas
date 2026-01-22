use anyhow::Result;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::path::Path;

mod models;

pub use models::*;

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn new(path: &str) -> Result<Self> {
        // Ensure the directory exists
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent)?;
        }

        let database_url = format!("sqlite:{}?mode=rwc", path);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await?;

        Ok(Self { pool })
    }

    pub async fn run_migrations(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                email TEXT UNIQUE NOT NULL,
                password_hash TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS cli_clients (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL REFERENCES users(id),
                name TEXT,
                last_seen DATETIME,
                status TEXT DEFAULT 'offline',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                cli_client_id TEXT,
                working_dir TEXT,
                hostname TEXT,
                status TEXT DEFAULT 'pending',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Add columns if they don't exist (migration for existing DBs)
        let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN working_dir TEXT")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN hostname TEXT")
            .execute(&self.pool)
            .await;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id),
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                message_type TEXT DEFAULT 'text',
                metadata TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Session sharing tables
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS session_shares (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                user_id TEXT NOT NULL REFERENCES users(id),
                invited_by TEXT NOT NULL REFERENCES users(id),
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(session_id, user_id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS invitation_codes (
                code TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                created_by TEXT NOT NULL REFERENCES users(id),
                expires_at DATETIME NOT NULL,
                redeemed_by TEXT REFERENCES users(id),
                redeemed_at DATETIME,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        tracing::info!("Database migrations completed");
        Ok(())
    }

    // User operations
    pub async fn create_user(&self, user: &User) -> Result<()> {
        sqlx::query(
            "INSERT INTO users (id, email, password_hash) VALUES (?, ?, ?)",
        )
        .bind(&user.id)
        .bind(&user.email)
        .bind(&user.password_hash)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_all_users(&self) -> Result<Vec<User>> {
        let users = sqlx::query_as::<_, User>("SELECT * FROM users ORDER BY email")
            .fetch_all(&self.pool)
            .await?;
        Ok(users)
    }

    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<User>> {
        let user = sqlx::query_as::<_, User>(
            "SELECT id, email, password_hash, created_at FROM users WHERE email = ?",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        Ok(user)
    }

    pub async fn get_user_by_id(&self, id: &str) -> Result<Option<User>> {
        let user = sqlx::query_as::<_, User>(
            "SELECT id, email, password_hash, created_at FROM users WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(user)
    }

    pub async fn update_user_password(&self, email: &str, password_hash: &str) -> Result<bool> {
        let result = sqlx::query("UPDATE users SET password_hash = ? WHERE email = ?")
            .bind(password_hash)
            .bind(email)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    // CLI client operations
    pub async fn upsert_cli_client(&self, client: &CliClient) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO cli_clients (id, user_id, name, last_seen, status)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                last_seen = excluded.last_seen,
                status = excluded.status
            "#,
        )
        .bind(&client.id)
        .bind(&client.user_id)
        .bind(&client.name)
        .bind(&client.last_seen)
        .bind(&client.status)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_cli_clients_for_user(&self, user_id: &str) -> Result<Vec<CliClient>> {
        let clients = sqlx::query_as::<_, CliClient>(
            "SELECT id, user_id, name, last_seen, status, created_at FROM cli_clients WHERE user_id = ?",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(clients)
    }

    pub async fn update_cli_client_status(&self, id: &str, status: &str) -> Result<()> {
        sqlx::query("UPDATE cli_clients SET status = ?, last_seen = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(status)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // Session operations
    pub async fn create_session(&self, session: &Session) -> Result<()> {
        // Use UPSERT (ON CONFLICT DO UPDATE) instead of INSERT OR REPLACE
        // INSERT OR REPLACE triggers ON DELETE CASCADE, which deletes session_shares
        sqlx::query(
            r#"
            INSERT INTO sessions (id, user_id, cli_client_id, working_dir, hostname, status)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                cli_client_id = excluded.cli_client_id,
                working_dir = excluded.working_dir,
                hostname = excluded.hostname,
                status = excluded.status,
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(&session.id)
        .bind(&session.user_id)
        .bind(&session.cli_client_id)
        .bind(&session.working_dir)
        .bind(&session.hostname)
        .bind(&session.status)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_session_status(&self, id: &str, status: &str) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET status = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(status)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<Session>> {
        let session = sqlx::query_as::<_, Session>(
            "SELECT id, user_id, cli_client_id, working_dir, hostname, status, created_at, updated_at FROM sessions WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(session)
    }

    pub async fn get_all_sessions(&self) -> Result<Vec<Session>> {
        let sessions = sqlx::query_as::<_, Session>(
            "SELECT id, user_id, cli_client_id, working_dir, hostname, status, created_at, updated_at FROM sessions ORDER BY created_at DESC LIMIT 50",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(sessions)
    }

    pub async fn get_sessions_for_user(&self, user_id: &str) -> Result<Vec<Session>> {
        let sessions = sqlx::query_as::<_, Session>(
            "SELECT id, user_id, cli_client_id, working_dir, hostname, status, created_at, updated_at FROM sessions WHERE user_id = ? ORDER BY created_at DESC LIMIT 50",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(sessions)
    }

    // Message operations
    pub async fn save_message(&self, message: &Message) -> Result<()> {
        sqlx::query(
            "INSERT INTO messages (id, session_id, role, content, message_type, metadata) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&message.id)
        .bind(&message.session_id)
        .bind(&message.role)
        .bind(&message.content)
        .bind(&message.message_type)
        .bind(&message.metadata)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_messages_for_session(&self, session_id: &str) -> Result<Vec<Message>> {
        let messages = sqlx::query_as::<_, Message>(
            "SELECT id, session_id, role, content, message_type, metadata, created_at FROM messages WHERE session_id = ? ORDER BY created_at ASC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(messages)
    }

    // Invitation code operations
    pub async fn create_invitation_code(&self, code: &InvitationCode) -> Result<()> {
        sqlx::query(
            "INSERT INTO invitation_codes (code, session_id, created_by, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&code.code)
        .bind(&code.session_id)
        .bind(&code.created_by)
        .bind(&code.expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_invitation_code(&self, code: &str) -> Result<Option<InvitationCode>> {
        let invitation = sqlx::query_as::<_, InvitationCode>(
            "SELECT code, session_id, created_by, expires_at, redeemed_by, redeemed_at, created_at FROM invitation_codes WHERE code = ?",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await?;
        Ok(invitation)
    }

    pub async fn redeem_invitation_code(&self, code: &str, user_id: &str) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE invitation_codes SET redeemed_by = ?, redeemed_at = CURRENT_TIMESTAMP WHERE code = ? AND redeemed_by IS NULL",
        )
        .bind(user_id)
        .bind(code)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_invitation_code(&self, code: &str) -> Result<()> {
        sqlx::query("DELETE FROM invitation_codes WHERE code = ?")
            .bind(code)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // Session share operations
    pub async fn create_session_share(&self, session_id: &str, user_id: &str, invited_by: &str) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO session_shares (session_id, user_id, invited_by) VALUES (?, ?, ?)",
        )
        .bind(session_id)
        .bind(user_id)
        .bind(invited_by)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_shared_sessions_for_user(&self, user_id: &str) -> Result<Vec<(Session, String)>> {
        // Returns sessions shared with this user along with the owner's email
        let rows = sqlx::query(
            r#"
            SELECT s.id, s.user_id, s.cli_client_id, s.working_dir, s.hostname, s.status, s.created_at, s.updated_at, u.email
            FROM sessions s
            INNER JOIN session_shares ss ON s.id = ss.session_id
            INNER JOIN users u ON s.user_id = u.id
            WHERE ss.user_id = ?
            ORDER BY s.created_at DESC
            LIMIT 50
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        let mut results = Vec::new();
        for row in rows {
            use sqlx::Row;
            let session = Session {
                id: row.get("id"),
                user_id: row.get("user_id"),
                cli_client_id: row.get("cli_client_id"),
                working_dir: row.get("working_dir"),
                hostname: row.get("hostname"),
                status: row.get("status"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            };
            let email: String = row.get("email");
            results.push((session, email));
        }
        Ok(results)
    }

    pub async fn check_session_access(&self, session_id: &str, user_id: &str) -> Result<bool> {
        // Check if user owns the session or has shared access
        let result = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*) FROM (
                SELECT 1 FROM sessions WHERE id = ? AND user_id = ?
                UNION ALL
                SELECT 1 FROM session_shares WHERE session_id = ? AND user_id = ?
            )
            "#,
        )
        .bind(session_id)
        .bind(user_id)
        .bind(session_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(result > 0)
    }

    pub async fn delete_session_share(&self, session_id: &str, user_id: &str) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM session_shares WHERE session_id = ? AND user_id = ?",
        )
        .bind(session_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn get_session_owner(&self, session_id: &str) -> Result<Option<String>> {
        let owner = sqlx::query_scalar::<_, String>(
            "SELECT user_id FROM sessions WHERE id = ?",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(owner)
    }

    /// Get session owner info (user_id and email)
    pub async fn get_session_owner_info(&self, session_id: &str) -> Result<Option<(String, String)>> {
        let row = sqlx::query(
            r#"
            SELECT u.id, u.email
            FROM sessions s
            INNER JOIN users u ON s.user_id = u.id
            WHERE s.id = ?
            "#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| {
            use sqlx::Row;
            (r.get("id"), r.get("email"))
        }))
    }

    /// Get all users who have shared access to a session (with their emails)
    pub async fn get_session_shares_with_emails(&self, session_id: &str) -> Result<Vec<(String, String, Option<String>)>> {
        let rows = sqlx::query(
            r#"
            SELECT u.id, u.email, ss.created_at
            FROM session_shares ss
            INNER JOIN users u ON ss.user_id = u.id
            WHERE ss.session_id = ?
            ORDER BY ss.created_at DESC
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.iter().map(|r| {
            use sqlx::Row;
            (r.get("id"), r.get("email"), r.get("created_at"))
        }).collect())
    }
}
