use anyhow::Result;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::path::Path;
use std::time::Duration;

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

        // Enable WAL mode for better concurrency (concurrent reads + writes)
        // Set busy_timeout to wait up to 5s when database is locked
        let database_url = format!("sqlite:{}?mode=rwc", path);
        let pool = SqlitePoolOptions::new()
            .max_connections(32)
            .acquire_timeout(Duration::from_secs(5))
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    sqlx::query("PRAGMA journal_mode=WAL")
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query("PRAGMA busy_timeout=5000")
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query("PRAGMA synchronous=NORMAL")
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            })
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
        let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN is_paused INTEGER DEFAULT 0")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN project_id TEXT")
            .execute(&self.pool)
            .await;
        // Canonical `host/owner/repo` of the project's git remote, used by the
        // web sidebar to group same-repo projects. No backfill: unlike
        // project_id, a NULL git_remote is the intended "(no remote)" value.
        let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN git_remote TEXT")
            .execute(&self.pool)
            .await;
        // Backfill project_id for rows that pre-date the column. Until now,
        // each .apas held a single id used as both project and session id, so
        // the safest backfill is project_id = id — old rows keep their
        // existing one-session-per-project grouping.
        let _ = sqlx::query("UPDATE sessions SET project_id = id WHERE project_id IS NULL")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_sessions_project_id ON sessions(project_id)",
        )
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

        // Per-(session, pane, day) usage counters for the Overview usage panel.
        // Day-bucketed (UTC 'YYYY-MM-DD') so lifetime / last-7-days / today
        // windows are all derivable by aggregating buckets. Counters are
        // additive; no backfill needed (they start at 0).
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS pane_usage_stats (
                session_id TEXT NOT NULL,
                pane_id INTEGER NOT NULL,
                day TEXT NOT NULL,
                prompt_count INTEGER NOT NULL DEFAULT 0,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                total_cost_usd REAL NOT NULL DEFAULT 0,
                num_responses INTEGER NOT NULL DEFAULT 0,
                updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (session_id, pane_id, day)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        let _ = sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_pane_usage_stats_session ON pane_usage_stats(session_id)",
        )
        .execute(&self.pool)
        .await;

        // Session sharing tables
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS session_shares (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                user_id TEXT NOT NULL REFERENCES users(id),
                invited_by TEXT NOT NULL REFERENCES users(id),
                role TEXT NOT NULL DEFAULT 'user',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(session_id, user_id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        let _ =
            sqlx::query("ALTER TABLE session_shares ADD COLUMN role TEXT NOT NULL DEFAULT 'user'")
                .execute(&self.pool)
                .await;
        let _ = sqlx::query(
            "UPDATE session_shares SET role = 'user' WHERE role IS NULL OR trim(role) = ''",
        )
        .execute(&self.pool)
        .await;

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
        sqlx::query("INSERT INTO users (id, email, password_hash) VALUES (?, ?, ?)")
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
        sqlx::query(
            "UPDATE cli_clients SET status = ?, last_seen = CURRENT_TIMESTAMP WHERE id = ?",
        )
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
        //
        // Also update user_id if the existing session owner is a dev/placeholder user
        // (email like 'dev-*@local'). This migrates sessions from temp users to real users.
        // project_id falls back to the session id for older CLIs that don't
        // send it; matches the historical 1:1 mapping.
        let project_id = session
            .project_id
            .clone()
            .unwrap_or_else(|| session.id.clone());
        sqlx::query(
            r#"
            INSERT INTO sessions (id, user_id, cli_client_id, working_dir, hostname, status, project_id, git_remote)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                cli_client_id = excluded.cli_client_id,
                working_dir = excluded.working_dir,
                hostname = excluded.hostname,
                status = excluded.status,
                project_id = excluded.project_id,
                git_remote = excluded.git_remote,
                updated_at = CURRENT_TIMESTAMP,
                user_id = CASE
                    WHEN (SELECT email FROM users WHERE id = sessions.user_id) LIKE 'dev-%@local'
                    THEN excluded.user_id
                    ELSE sessions.user_id
                END
            "#,
        )
        .bind(&session.id)
        .bind(&session.user_id)
        .bind(&session.cli_client_id)
        .bind(&session.working_dir)
        .bind(&session.hostname)
        .bind(&session.status)
        .bind(&project_id)
        .bind(&session.git_remote)
        .execute(&self.pool)
        .await?;

        // Migrate shares from old sessions with same working_dir + hostname to this session.
        // This handles the case where a .apas file is regenerated (new session ID) but
        // shares still reference the old session ID.
        if let (Some(working_dir), Some(hostname)) = (&session.working_dir, &session.hostname) {
            let migrated = sqlx::query(
                r#"
                UPDATE session_shares SET session_id = ?
                WHERE session_id IN (
                    SELECT id FROM sessions
                    WHERE working_dir = ? AND hostname = ? AND id != ?
                )
                AND user_id NOT IN (
                    SELECT user_id FROM session_shares WHERE session_id = ?
                )
                "#,
            )
            .bind(&session.id)
            .bind(working_dir)
            .bind(hostname)
            .bind(&session.id)
            .bind(&session.id)
            .execute(&self.pool)
            .await?;

            if migrated.rows_affected() > 0 {
                tracing::info!(
                    "Migrated {} share(s) from old sessions to new session {} ({}@{})",
                    migrated.rows_affected(),
                    session.id,
                    working_dir,
                    hostname
                );
            }
        }

        Ok(())
    }

    pub async fn update_session_status(&self, id: &str, status: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET status = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(status)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Mark session as inactive and clear cli_client_id (CLI disconnected)
    pub async fn clear_session_cli(&self, id: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET status = 'inactive', cli_client_id = NULL, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_session_paused(&self, id: &str, is_paused: bool) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET is_paused = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(is_paused)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<Session>> {
        let session = sqlx::query_as::<_, Session>(
            "SELECT id, user_id, cli_client_id, working_dir, hostname, status, created_at, updated_at, COALESCE(is_paused, 0) as is_paused, COALESCE(project_id, id) as project_id, git_remote FROM sessions WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(session)
    }

    /// Find the most recently updated different session for the same user/project path/host.
    pub async fn get_latest_project_session_id(
        &self,
        user_id: &str,
        working_dir: Option<&str>,
        hostname: Option<&str>,
        exclude_session_id: &str,
    ) -> Result<Option<String>> {
        let sid = sqlx::query_scalar::<_, String>(
            r#"
            SELECT id
            FROM sessions
            WHERE user_id = ?
              AND COALESCE(working_dir, '') = COALESCE(?, '')
              AND COALESCE(hostname, '') = COALESCE(?, '')
              AND id != ?
            ORDER BY updated_at DESC
            LIMIT 1
            "#,
        )
        .bind(user_id)
        .bind(working_dir)
        .bind(hostname)
        .bind(exclude_session_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(sid)
    }

    pub async fn get_all_sessions(&self) -> Result<Vec<Session>> {
        let sessions = sqlx::query_as::<_, Session>(
            "SELECT id, user_id, cli_client_id, working_dir, hostname, status, created_at, updated_at, COALESCE(is_paused, 0) as is_paused, COALESCE(project_id, id) as project_id, git_remote FROM sessions ORDER BY created_at DESC LIMIT 50",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(sessions)
    }

    pub async fn get_sessions_for_user(&self, user_id: &str) -> Result<Vec<Session>> {
        let sessions = sqlx::query_as::<_, Session>(
            "SELECT id, user_id, cli_client_id, working_dir, hostname, status, created_at, updated_at, COALESCE(is_paused, 0) as is_paused, COALESCE(project_id, id) as project_id, git_remote FROM sessions WHERE user_id = ? ORDER BY created_at DESC LIMIT 50",
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
    pub async fn create_session_share(
        &self,
        session_id: &str,
        user_id: &str,
        invited_by: &str,
    ) -> Result<()> {
        self.create_session_share_with_role(session_id, user_id, invited_by, "user")
            .await
    }

    pub async fn create_session_share_with_role(
        &self,
        session_id: &str,
        user_id: &str,
        invited_by: &str,
        role: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO session_shares (session_id, user_id, invited_by, role) VALUES (?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(user_id)
        .bind(invited_by)
        .bind(role)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_shared_sessions_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<(Session, String, String)>> {
        // Returns sessions shared with this user along with the owner's email and share role
        let rows = sqlx::query(
            r#"
            SELECT s.id, s.user_id, s.cli_client_id, s.working_dir, s.hostname, s.status, s.created_at, s.updated_at, COALESCE(s.is_paused, 0) as is_paused, COALESCE(s.project_id, s.id) as project_id, s.git_remote, u.email, COALESCE(ss.role, 'user') AS role
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
                is_paused: row.get::<i32, _>("is_paused") != 0,
                project_id: row.get("project_id"),
                git_remote: row.get("git_remote"),
            };
            let email: String = row.get("email");
            let role: String = row.get("role");
            results.push((session, email, role));
        }
        Ok(results)
    }

    pub async fn get_session_role_for_user(
        &self,
        session_id: &str,
        user_id: &str,
    ) -> Result<Option<String>> {
        let owner = sqlx::query_scalar::<_, String>("SELECT user_id FROM sessions WHERE id = ?")
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await?;
        if owner.as_deref() == Some(user_id) {
            return Ok(Some("owner".to_string()));
        }

        let role = sqlx::query_scalar::<_, String>(
            "SELECT COALESCE(role, 'user') FROM session_shares WHERE session_id = ? AND user_id = ?",
        )
        .bind(session_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(role)
    }

    pub async fn get_session_share_role(
        &self,
        session_id: &str,
        user_id: &str,
    ) -> Result<Option<String>> {
        let role = sqlx::query_scalar::<_, String>(
            "SELECT COALESCE(role, 'user') FROM session_shares WHERE session_id = ? AND user_id = ?",
        )
        .bind(session_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(role)
    }

    pub async fn update_session_share_role(
        &self,
        session_id: &str,
        user_id: &str,
        role: &str,
    ) -> Result<bool> {
        let result =
            sqlx::query("UPDATE session_shares SET role = ? WHERE session_id = ? AND user_id = ?")
                .bind(role)
                .bind(session_id)
                .bind(user_id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
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
        let result = sqlx::query("DELETE FROM session_shares WHERE session_id = ? AND user_id = ?")
            .bind(session_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn get_session_owner(&self, session_id: &str) -> Result<Option<String>> {
        let owner = sqlx::query_scalar::<_, String>("SELECT user_id FROM sessions WHERE id = ?")
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(owner)
    }

    /// Get session owner info (user_id and email)
    pub async fn get_session_owner_info(
        &self,
        session_id: &str,
    ) -> Result<Option<(String, String)>> {
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
    pub async fn get_session_shares_with_emails(
        &self,
        session_id: &str,
    ) -> Result<Vec<(String, String, Option<String>, String)>> {
        let rows = sqlx::query(
            r#"
            SELECT u.id, u.email, ss.created_at, COALESCE(ss.role, 'user') as role
            FROM session_shares ss
            INNER JOIN users u ON ss.user_id = u.id
            WHERE ss.session_id = ?
            ORDER BY ss.created_at DESC
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .iter()
            .map(|r| {
                use sqlx::Row;
                (
                    r.get("id"),
                    r.get("email"),
                    r.get("created_at"),
                    r.get("role"),
                )
            })
            .collect())
    }

    // ========================================================================
    // Admin Statistics
    // ========================================================================

    /// Get total user count
    pub async fn get_user_count(&self) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }

    /// Get total session count
    pub async fn get_session_count(&self) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sessions")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }

    /// Get active session count (sessions with activity in last 24 hours)
    pub async fn get_active_session_count(&self) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sessions WHERE updated_at > datetime('now', '-1 day')",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    /// Get users created in last 7 days
    pub async fn get_recent_user_count(&self) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM users WHERE created_at > datetime('now', '-7 days')",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    /// Get CLI client count
    pub async fn get_cli_client_count(&self) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cli_clients")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }

    /// Get session share count
    pub async fn get_share_count(&self) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM session_shares")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }

    /// Get recent users (last 10 registered)
    pub async fn get_recent_users(
        &self,
        limit: i32,
    ) -> Result<Vec<(String, String, Option<String>)>> {
        let rows =
            sqlx::query("SELECT id, email, created_at FROM users ORDER BY created_at DESC LIMIT ?")
                .bind(limit)
                .fetch_all(&self.pool)
                .await?;

        Ok(rows
            .iter()
            .map(|r| {
                use sqlx::Row;
                (r.get("id"), r.get("email"), r.get("created_at"))
            })
            .collect())
    }

    /// Get sessions per day for last N days
    pub async fn get_sessions_per_day(&self, days: i32) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query(
            r#"
            SELECT date(created_at) as day, COUNT(*) as count
            FROM sessions
            WHERE created_at > datetime('now', '-' || ? || ' days')
            GROUP BY date(created_at)
            ORDER BY day DESC
            "#,
        )
        .bind(days)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .iter()
            .map(|r| {
                use sqlx::Row;
                (r.get("day"), r.get("count"))
            })
            .collect())
    }

    /// Additively record a usage delta into today's bucket for (session, pane).
    /// `day` is a UTC `YYYY-MM-DD` string. Counters accumulate via ON CONFLICT.
    pub async fn add_pane_usage(
        &self,
        session_id: &str,
        pane_id: i64,
        day: &str,
        delta: &UsageDelta,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO pane_usage_stats
                (session_id, pane_id, day, prompt_count, input_tokens, output_tokens,
                 cache_read_tokens, cache_creation_tokens, total_cost_usd, num_responses, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(session_id, pane_id, day) DO UPDATE SET
                prompt_count = prompt_count + excluded.prompt_count,
                input_tokens = input_tokens + excluded.input_tokens,
                output_tokens = output_tokens + excluded.output_tokens,
                cache_read_tokens = cache_read_tokens + excluded.cache_read_tokens,
                cache_creation_tokens = cache_creation_tokens + excluded.cache_creation_tokens,
                total_cost_usd = total_cost_usd + excluded.total_cost_usd,
                num_responses = num_responses + excluded.num_responses,
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(session_id)
        .bind(pane_id)
        .bind(day)
        .bind(delta.prompt_count)
        .bind(delta.input_tokens)
        .bind(delta.output_tokens)
        .bind(delta.cache_read_tokens)
        .bind(delta.cache_creation_tokens)
        .bind(delta.total_cost_usd)
        .bind(delta.num_responses)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Aggregate usage for the whole project that `session_id` belongs to
    /// (all sessions sharing its project_id), split per pane into the
    /// lifetime / last-7-days / today windows used by the Overview panel.
    pub async fn get_project_usage_stats(
        &self,
        session_id: &str,
    ) -> Result<shared::ProjectUsageStats> {
        let rows = sqlx::query_as::<_, PaneUsageDayRow>(
            r#"
            SELECT pus.pane_id, pus.day, pus.prompt_count, pus.input_tokens, pus.output_tokens,
                   pus.cache_read_tokens, pus.cache_creation_tokens, pus.total_cost_usd,
                   pus.num_responses, pus.updated_at
            FROM pane_usage_stats pus
            JOIN sessions s ON s.id = pus.session_id
            WHERE COALESCE(s.project_id, s.id) =
                  (SELECT COALESCE(project_id, id) FROM sessions WHERE id = ?)
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        // 7-day window is today plus the previous 6 days, inclusive.
        let week_start = (chrono::Utc::now() - chrono::Duration::days(6))
            .format("%Y-%m-%d")
            .to_string();

        use std::collections::BTreeMap;
        let mut by_pane: BTreeMap<i64, shared::PaneUsageStats> = BTreeMap::new();
        let mut project = shared::ProjectUsageStats::default();

        for r in &rows {
            let pane = by_pane
                .entry(r.pane_id)
                .or_insert_with(|| shared::PaneUsageStats {
                    pane_id: r.pane_id as u32,
                    ..Default::default()
                });
            accumulate(&mut pane.lifetime, r);
            accumulate(&mut project.lifetime, r);
            if r.day.as_str() >= week_start.as_str() {
                accumulate(&mut pane.last_7d, r);
                accumulate(&mut project.last_7d, r);
            }
            if r.day == today {
                accumulate(&mut pane.today, r);
                accumulate(&mut project.today, r);
            }
            if let Some(ua) = &r.updated_at {
                // Normalize SQLite's "YYYY-MM-DD HH:MM:SS" to RFC3339 (UTC)
                // BEFORE comparing/storing, so the max comparison stays
                // consistent and clients can parse the timestamp.
                let ua = sqlite_ts_to_rfc3339(ua);
                if pane
                    .last_active
                    .as_deref()
                    .map_or(true, |cur| cur < ua.as_str())
                {
                    pane.last_active = Some(ua.clone());
                }
                if project
                    .last_active
                    .as_deref()
                    .map_or(true, |cur| cur < ua.as_str())
                {
                    project.last_active = Some(ua);
                }
            }
        }

        project.panes = by_pane.into_values().collect();
        Ok(project)
    }
}

/// Additive usage delta applied to a (session, pane, day) bucket.
#[derive(Debug, Clone, Default)]
pub struct UsageDelta {
    pub prompt_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub total_cost_usd: f64,
    pub num_responses: i64,
}

/// SQLite `CURRENT_TIMESTAMP` is `YYYY-MM-DD HH:MM:SS` (UTC, no zone marker).
/// Render it as RFC3339 so the web can parse `last_active` unambiguously.
fn sqlite_ts_to_rfc3339(ts: &str) -> String {
    if ts.len() == 19 && ts.as_bytes().get(10) == Some(&b' ') {
        format!("{}T{}Z", &ts[..10], &ts[11..])
    } else {
        ts.to_string()
    }
}

/// Fold one day-bucket row into a window's running counters (negatives,
/// which never occur in practice, are clamped to 0 before the u64 cast).
fn accumulate(c: &mut shared::UsageCounters, r: &PaneUsageDayRow) {
    c.prompts += r.prompt_count.max(0) as u64;
    c.responses += r.num_responses.max(0) as u64;
    c.input_tokens += r.input_tokens.max(0) as u64;
    c.output_tokens += r.output_tokens.max(0) as u64;
    c.cache_read_tokens += r.cache_read_tokens.max(0) as u64;
    c.cache_creation_tokens += r.cache_creation_tokens.max(0) as u64;
    c.cost_usd += r.total_cost_usd;
}

#[cfg(test)]
mod usage_stats_tests {
    use super::*;

    async fn temp_db() -> Database {
        let dir = std::env::temp_dir().join(format!("apas-usage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp db dir");
        let path = dir.join("apas.db").to_string_lossy().to_string();
        let db = Database::new(&path).await.expect("db");
        db.run_migrations().await.expect("migrations");
        db
    }

    fn session(id: &str, project_id: &str) -> Session {
        Session {
            id: id.to_string(),
            user_id: "u1".to_string(),
            cli_client_id: None,
            working_dir: None,
            hostname: None,
            status: "active".to_string(),
            created_at: None,
            updated_at: None,
            is_paused: false,
            project_id: Some(project_id.to_string()),
            git_remote: None,
        }
    }

    #[tokio::test]
    async fn pane_usage_aggregates_per_window_and_per_pane() {
        let db = temp_db().await;
        let sid = "11111111-1111-1111-1111-111111111111";
        db.create_session(&session(sid, "project-A")).await.unwrap();
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

        // Two additive turns for pane 178 today, plus a prompt.
        db.add_pane_usage(
            sid,
            178,
            &today,
            &UsageDelta {
                input_tokens: 100,
                output_tokens: 50,
                total_cost_usd: 0.02,
                num_responses: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        db.add_pane_usage(
            sid,
            178,
            &today,
            &UsageDelta {
                prompt_count: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        // An old-day bucket for pane 178 — counts toward lifetime only.
        db.add_pane_usage(
            sid,
            178,
            "2020-01-01",
            &UsageDelta {
                input_tokens: 1000,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        // A second pane.
        db.add_pane_usage(
            sid,
            568,
            &today,
            &UsageDelta {
                input_tokens: 10,
                num_responses: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let stats = db.get_project_usage_stats(sid).await.unwrap();
        assert_eq!(stats.panes.len(), 2);

        let p178 = stats.panes.iter().find(|p| p.pane_id == 178).unwrap();
        assert_eq!(p178.lifetime.input_tokens, 1100);
        assert_eq!(p178.lifetime.output_tokens, 50);
        assert_eq!(p178.lifetime.prompts, 1);
        assert_eq!(p178.lifetime.responses, 1);
        // The 2020 bucket is outside today/7d.
        assert_eq!(p178.today.input_tokens, 100);
        assert_eq!(p178.last_7d.input_tokens, 100);
        assert!((p178.lifetime.cost_usd - 0.02).abs() < 1e-9);

        // Project totals sum across both panes.
        assert_eq!(stats.lifetime.input_tokens, 1110);
        assert_eq!(stats.today.input_tokens, 110);
        assert_eq!(stats.lifetime.responses, 2);
        assert_eq!(stats.lifetime.prompts, 1);
    }

    #[tokio::test]
    async fn usage_stats_scope_to_their_project() {
        let db = temp_db().await;
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        db.create_session(&session("s-a", "proj-1")).await.unwrap();
        db.create_session(&session("s-b", "proj-2")).await.unwrap();
        db.add_pane_usage(
            "s-a",
            1,
            &today,
            &UsageDelta {
                input_tokens: 5,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        db.add_pane_usage(
            "s-b",
            1,
            &today,
            &UsageDelta {
                input_tokens: 99,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // s-a's project must not see s-b's usage.
        let a = db.get_project_usage_stats("s-a").await.unwrap();
        assert_eq!(a.lifetime.input_tokens, 5);
    }
}
