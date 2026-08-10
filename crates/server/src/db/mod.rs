use anyhow::Result;
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};
use std::path::Path;
use std::time::Duration;

mod models;

pub use models::*;

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct RetiredProfileMigration {
    cluster_default_changed: bool,
    changed_project_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnershipTransferPolicy {
    ClusterAdministrator,
    CurrentOwner,
}

const LEGACY_PROJECT_ACCESS_MIGRATION: &str = "legacy_project_access_v1";

fn admin_project_name(working_dir: Option<&str>, git_remote: Option<&str>) -> Option<String> {
    working_dir
        .and_then(|path| {
            path.trim()
                .trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .find(|segment| !segment.is_empty())
        })
        .or_else(|| {
            git_remote.and_then(|remote| {
                remote
                    .trim()
                    .trim_end_matches('/')
                    .rsplit('/')
                    .find(|segment| !segment.is_empty())
            })
        })
        .map(|name| name.strip_suffix(".git").unwrap_or(name).to_string())
        .filter(|name| !name.is_empty())
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
                    sqlx::query("PRAGMA foreign_keys=ON")
                        .execute(&mut *conn)
                        .await?;
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
            CREATE TABLE IF NOT EXISTS schema_migrations (
                name TEXT PRIMARY KEY,
                applied_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

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

        let _ =
            sqlx::query("ALTER TABLE users ADD COLUMN cluster_role TEXT NOT NULL DEFAULT 'user'")
                .execute(&self.pool)
                .await;
        let _ = sqlx::query(
            "ALTER TABLE users ADD COLUMN account_status TEXT NOT NULL DEFAULT 'active'",
        )
        .execute(&self.pool)
        .await;
        sqlx::query("UPDATE users SET cluster_role = 'user' WHERE cluster_role NOT IN ('admin', 'user') OR cluster_role IS NULL")
            .execute(&self.pool)
            .await?;
        sqlx::query("UPDATE users SET account_status = 'active' WHERE account_status NOT IN ('active', 'suspended') OR account_status IS NULL")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_users_cluster_role_status ON users(cluster_role, account_status)")
            .execute(&self.pool)
            .await?;
        // SQLite cannot retrofit CHECK constraints with ALTER TABLE. These
        // triggers give upgraded databases the same invariant as a freshly
        // created constrained table without rebuilding `users` (which would
        // be risky while several legacy tables still reference it).
        for statement in [
            r#"CREATE TRIGGER IF NOT EXISTS users_cluster_role_insert
               BEFORE INSERT ON users
               WHEN NEW.cluster_role NOT IN ('admin', 'user')
               BEGIN SELECT RAISE(ABORT, 'invalid cluster_role'); END"#,
            r#"CREATE TRIGGER IF NOT EXISTS users_cluster_role_update
               BEFORE UPDATE OF cluster_role ON users
               WHEN NEW.cluster_role NOT IN ('admin', 'user')
               BEGIN SELECT RAISE(ABORT, 'invalid cluster_role'); END"#,
            r#"CREATE TRIGGER IF NOT EXISTS users_account_status_insert
               BEFORE INSERT ON users
               WHEN NEW.account_status NOT IN ('active', 'suspended')
               BEGIN SELECT RAISE(ABORT, 'invalid account_status'); END"#,
            r#"CREATE TRIGGER IF NOT EXISTS users_account_status_update
               BEFORE UPDATE OF account_status ON users
               WHEN NEW.account_status NOT IN ('active', 'suspended')
               BEGIN SELECT RAISE(ABORT, 'invalid account_status'); END"#,
        ] {
            sqlx::query(statement).execute(&self.pool).await?;
        }

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS mobile_installations (
                installation_id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                platform TEXT NOT NULL CHECK (platform IN ('ios', 'android')),
                device_name TEXT,
                app_version TEXT NOT NULL,
                created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_mobile_installations_user ON mobile_installations(user_id, updated_at DESC)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS mobile_device_sessions (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                installation_id TEXT NOT NULL REFERENCES mobile_installations(installation_id) ON DELETE CASCADE,
                refresh_token_hash TEXT NOT NULL UNIQUE,
                app_version TEXT NOT NULL,
                created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                last_used_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                expires_at DATETIME NOT NULL,
                revoked_at DATETIME,
                revocation_reason TEXT
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_mobile_device_sessions_user_active ON mobile_device_sessions(user_id, revoked_at, expires_at)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_mobile_device_sessions_installation ON mobile_device_sessions(installation_id, created_at DESC)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS mobile_refresh_token_history (
                token_hash TEXT PRIMARY KEY,
                device_session_id TEXT NOT NULL REFERENCES mobile_device_sessions(id) ON DELETE CASCADE,
                consumed_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_mobile_refresh_history_session ON mobile_refresh_token_history(device_session_id, consumed_at DESC)",
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
        // Raw cloneable origin URL (for the create-instance feature).
        let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN git_remote_url TEXT")
            .execute(&self.pool)
            .await;
        // User-driven recency is tracked separately from lifecycle updates so
        // reconnects, pauses, and agent output cannot displace the session the
        // user most recently messaged in mobile session lists.
        let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN last_user_input_at TEXT")
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

        let _ = sqlx::query("ALTER TABLE invitation_codes ADD COLUMN project_id TEXT")
            .execute(&self.pool)
            .await;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                owner_user_id TEXT NOT NULL REFERENCES users(id),
                lifecycle_status TEXT NOT NULL DEFAULT 'active',
                legacy_policy_pending INTEGER NOT NULL DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        let _ = sqlx::query(
            "ALTER TABLE projects ADD COLUMN legacy_policy_pending INTEGER NOT NULL DEFAULT 0",
        )
        .execute(&self.pool)
        .await;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_projects_owner ON projects(owner_user_id)")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_projects_lifecycle ON projects(lifecycle_status)",
        )
        .execute(&self.pool)
        .await?;
        // Recreate these triggers so upgraded installations learn about the
        // internal deletion state; CREATE IF NOT EXISTS would leave the old
        // active/suspended-only trigger in place forever.
        for trigger in ["projects_lifecycle_insert", "projects_lifecycle_update"] {
            sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger}"))
                .execute(&self.pool)
                .await?;
        }
        for statement in [
            r#"CREATE TRIGGER IF NOT EXISTS projects_lifecycle_insert
               BEFORE INSERT ON projects
               WHEN NEW.lifecycle_status NOT IN ('active', 'suspended', 'deleting')
               BEGIN SELECT RAISE(ABORT, 'invalid project lifecycle_status'); END"#,
            r#"CREATE TRIGGER IF NOT EXISTS projects_lifecycle_update
               BEFORE UPDATE OF lifecycle_status ON projects
               WHEN NEW.lifecycle_status NOT IN ('active', 'suspended', 'deleting')
               BEGIN SELECT RAISE(ABORT, 'invalid project lifecycle_status'); END"#,
        ] {
            sqlx::query(statement).execute(&self.pool).await?;
        }

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS project_members (
                project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                invited_by TEXT NOT NULL REFERENCES users(id),
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (project_id, user_id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_project_members_user ON project_members(user_id, project_id)")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS cluster_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                team_available INTEGER NOT NULL DEFAULT 0,
                allowed_launch_profiles TEXT NOT NULL,
                version INTEGER NOT NULL DEFAULT 1,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        let default_profiles = serde_json::to_string(
            &shared::supported_launch_profiles()
                .into_iter()
                .map(|profile| profile.key)
                .collect::<Vec<_>>(),
        )?;
        sqlx::query(
            "INSERT OR IGNORE INTO cluster_settings (id, team_available, allowed_launch_profiles, version) VALUES (1, 0, ?, 1)",
        )
        .bind(default_profiles)
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS project_policy_overrides (
                project_id TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
                team_available INTEGER,
                allowed_launch_profiles TEXT,
                version INTEGER NOT NULL DEFAULT 1,
                legacy_imported INTEGER NOT NULL DEFAULT 0,
                legacy_conflict TEXT,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        let retirement = self.normalize_retired_provider_profiles().await?;
        if retirement.cluster_default_changed || !retirement.changed_project_ids.is_empty() {
            tracing::info!(
                cluster_default_changed = retirement.cluster_default_changed,
                changed_project_count = retirement.changed_project_ids.len(),
                changed_project_ids = ?retirement.changed_project_ids,
                "removed retired provider profiles from persisted launch policy"
            );
        }

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS cluster_invitations (
                code TEXT PRIMARY KEY,
                email TEXT NOT NULL,
                created_by TEXT NOT NULL REFERENCES users(id),
                expires_at DATETIME NOT NULL,
                redeemed_at DATETIME,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_cluster_invitations_email ON cluster_invitations(email)")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS admin_audit_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                actor_user_id TEXT NOT NULL REFERENCES users(id),
                action TEXT NOT NULL,
                target_type TEXT NOT NULL,
                target_id TEXT NOT NULL,
                details TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        let _ = sqlx::query("ALTER TABLE admin_audit_events ADD COLUMN project_id TEXT")
            .execute(&self.pool)
            .await;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_admin_audit_created ON admin_audit_events(created_at DESC, id DESC)")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_admin_audit_project ON admin_audit_events(project_id, id)")
            .execute(&self.pool)
            .await?;

        // Mobile notification state is installation-scoped. Device-session
        // revocation removes push reachability immediately through the trigger
        // below, regardless of which credential-revocation path fired.
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS mobile_push_tokens (
                id TEXT PRIMARY KEY,
                installation_id TEXT NOT NULL REFERENCES mobile_installations(installation_id) ON DELETE CASCADE,
                device_session_id TEXT NOT NULL REFERENCES mobile_device_sessions(id) ON DELETE CASCADE,
                platform TEXT NOT NULL CHECK (platform IN ('ios', 'android')),
                token TEXT NOT NULL,
                token_hash TEXT NOT NULL UNIQUE,
                created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                retired_at DATETIME,
                retirement_reason TEXT
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_mobile_push_installation_active ON mobile_push_tokens(installation_id, retired_at)")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_mobile_push_session ON mobile_push_tokens(device_session_id)")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS mobile_notification_preferences (
                installation_id TEXT PRIMARY KEY REFERENCES mobile_installations(installation_id) ON DELETE CASCADE,
                decisions INTEGER NOT NULL DEFAULT 1,
                failures INTEGER NOT NULL DEFAULT 1,
                pull_requests INTEGER NOT NULL DEFAULT 1,
                completions INTEGER NOT NULL DEFAULT 0,
                updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS mobile_notification_events (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
                session_id TEXT,
                pane_id INTEGER,
                category TEXT NOT NULL CHECK (category IN ('decision', 'failure', 'pull_request', 'completion')),
                routing_id TEXT NOT NULL,
                dedupe_key TEXT NOT NULL UNIQUE,
                created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_mobile_notification_events_user_created ON mobile_notification_events(user_id, created_at)")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS mobile_notification_deliveries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT NOT NULL REFERENCES mobile_notification_events(id) ON DELETE CASCADE,
                push_token_id TEXT NOT NULL REFERENCES mobile_push_tokens(id) ON DELETE CASCADE,
                status TEXT NOT NULL DEFAULT 'queued',
                attempt_count INTEGER NOT NULL DEFAULT 0,
                next_attempt_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                provider_ticket_id TEXT,
                provider_error TEXT,
                created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(event_id, push_token_id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_mobile_notification_delivery_retry ON mobile_notification_deliveries(status, next_attempt_at, id)")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            r#"
            CREATE TRIGGER IF NOT EXISTS mobile_device_session_push_cleanup
            AFTER UPDATE OF revoked_at ON mobile_device_sessions
            WHEN NEW.revoked_at IS NOT NULL
            BEGIN
                DELETE FROM mobile_push_tokens WHERE device_session_id = NEW.id;
            END
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            r#"
            CREATE TRIGGER IF NOT EXISTS mobile_notification_session_cleanup
            AFTER DELETE ON sessions
            BEGIN
                DELETE FROM mobile_notification_events WHERE session_id = OLD.id;
            END
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Mobile task launch results are retained independently of an HTTP
        // connection. The instruction is deliberately not stored: the keyed
        // request fingerprint is enough to reject request-id reuse without
        // adding prompt content to the control-plane database.
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS mobile_task_launches (
                request_id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                device_session_id TEXT NOT NULL REFERENCES mobile_device_sessions(id) ON DELETE CASCADE,
                request_fingerprint TEXT NOT NULL,
                machine_id TEXT NOT NULL,
                project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'completed', 'failed')),
                session_id TEXT,
                pane_id INTEGER,
                error_message TEXT,
                created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_mobile_task_launch_user_created ON mobile_task_launches(user_id, created_at DESC)",
        )
        .execute(&self.pool)
        .await?;

        // Canonicalize all historical runtime sessions into durable projects.
        // The earliest session owns the project; other historical owners keep
        // access as ordinary members and are surfaced in the audit log.
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO projects (id, owner_user_id, lifecycle_status, legacy_policy_pending, created_at, updated_at)
            SELECT project_id, user_id, 'active', 1, created_at, updated_at
            FROM (
                SELECT COALESCE(project_id, id) AS project_id, user_id, created_at, updated_at,
                       ROW_NUMBER() OVER (
                           PARTITION BY COALESCE(project_id, id)
                           ORDER BY created_at ASC, id ASC
                       ) AS project_rank
                FROM sessions
            ) ranked
            WHERE project_rank = 1
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Make conflicting historical owners visible to administrators. The
        // access import itself is versioned below so runtime history cannot
        // keep acting as an authorization source after the initial upgrade.
        sqlx::query(
            r#"
            INSERT INTO admin_audit_events
                (actor_user_id, action, target_type, target_id, details)
            SELECT p.owner_user_id, 'migration.conflicting_project_owners',
                   'project', p.id,
                   json_object(
                       'selected_owner_user_id', p.owner_user_id,
                       'historical_owner_count', COUNT(DISTINCT s.user_id)
                   )
            FROM projects p
            JOIN sessions s ON COALESCE(s.project_id, s.id) = p.id
            GROUP BY p.id, p.owner_user_id
            HAVING COUNT(DISTINCT s.user_id) > 1
               AND NOT EXISTS (
                   SELECT 1 FROM admin_audit_events a
                   WHERE a.action = 'migration.conflicting_project_owners'
                     AND a.target_type = 'project'
                     AND a.target_id = p.id
               )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            UPDATE invitation_codes
            SET project_id = (
                SELECT COALESCE(s.project_id, s.id)
                FROM sessions s
                WHERE s.id = invitation_codes.session_id
            )
            WHERE project_id IS NULL
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_invitation_codes_project ON invitation_codes(project_id)")
            .execute(&self.pool)
            .await?;

        // Project-level admin is deliberately not carried forward. Keep a
        // migration audit trail without elevating any account cluster-wide.
        sqlx::query(
            r#"
            INSERT INTO admin_audit_events (actor_user_id, action, target_type, target_id, details)
            SELECT p.owner_user_id, 'migration.legacy_project_admin_downgraded',
                   'project_member', ss.user_id,
                   json_object('project_id', p.id, 'legacy_role', 'admin')
            FROM session_shares ss
            JOIN sessions s ON s.id = ss.session_id
            JOIN projects p ON p.id = COALESCE(s.project_id, s.id)
            WHERE lower(trim(COALESCE(ss.role, 'user'))) = 'admin'
              AND NOT EXISTS (
                  SELECT 1 FROM admin_audit_events a
                  WHERE a.action = 'migration.legacy_project_admin_downgraded'
                    AND a.target_id = ss.user_id
                    AND json_extract(a.details, '$.project_id') = p.id
              )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Give every deterministically project-associated legacy audit event
        // an explicit key after all migration-generated events have landed.
        // The CASE wrapper keeps malformed historical JSON from aborting an
        // otherwise additive migration.
        sqlx::query(
            "UPDATE admin_audit_events SET project_id = target_id WHERE project_id IS NULL AND target_type = 'project'",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            r#"
            UPDATE admin_audit_events
            SET project_id = json_extract(
                CASE WHEN json_valid(details) THEN details ELSE '{}' END,
                '$.project_id'
            )
            WHERE project_id IS NULL
              AND json_type(
                    CASE WHEN json_valid(details) THEN details ELSE '{}' END,
                    '$.project_id'
                  ) = 'text'
            "#,
        )
        .execute(&self.pool)
        .await?;

        self.migrate_legacy_project_access_once().await?;

        tracing::info!("Database migrations completed");
        Ok(())
    }

    /// Import access from the pre-project session tables exactly once.
    ///
    /// Historical sessions are evidence for the initial migration, not an
    /// ongoing authorization source. Rerunning these inserts after a user
    /// leaves or is removed would silently restore their project membership.
    /// The marker and the data changes share one transaction, so a failed
    /// migration can be retried without leaving a false completion record.
    async fn migrate_legacy_project_access_once(&self) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let should_run = sqlx::query("INSERT OR IGNORE INTO schema_migrations (name) VALUES (?)")
            .bind(LEGACY_PROJECT_ACCESS_MIGRATION)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;

        if !should_run {
            tx.commit().await?;
            return Ok(false);
        }

        sqlx::query(
            r#"
            INSERT OR IGNORE INTO project_members (project_id, user_id, invited_by, created_at)
            SELECT DISTINCT COALESCE(s.project_id, s.id), s.user_id, p.owner_user_id, s.created_at
            FROM sessions s
            JOIN projects p ON p.id = COALESCE(s.project_id, s.id)
            WHERE s.user_id != p.owner_user_id
            "#,
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            INSERT OR IGNORE INTO project_members (project_id, user_id, invited_by, created_at)
            SELECT DISTINCT COALESCE(s.project_id, s.id), ss.user_id, ss.invited_by, ss.created_at
            FROM session_shares ss
            JOIN sessions s ON s.id = ss.session_id
            JOIN projects p ON p.id = COALESCE(s.project_id, s.id)
            WHERE ss.user_id != p.owner_user_id
            "#,
        )
        .execute(&mut *tx)
        .await?;

        // Installations that already ran the formerly-unbounded backfill may
        // contain resurrected rows. The audit log is authoritative for these
        // rows: remove a membership only when its latest access-changing event
        // is a departure/removal. A later invitation, explicit add, or owner
        // transfer therefore remains intact.
        let repaired = sqlx::query(
            r#"
            DELETE FROM project_members
            WHERE EXISTS (
                SELECT 1
                FROM admin_audit_events departure
                WHERE departure.project_id = project_members.project_id
                  AND departure.target_id = project_members.user_id
                  AND departure.action IN ('project.member_left', 'project.member_removed')
                  AND departure.id = (
                      SELECT MAX(event.id)
                      FROM admin_audit_events event
                      WHERE event.project_id = project_members.project_id
                        AND (
                            (
                                event.action IN (
                                    'project.member_added',
                                    'project.member_left',
                                    'project.member_removed'
                                )
                                AND (
                                    event.target_id = project_members.user_id
                                    OR (
                                        event.action = 'project.member_added'
                                        AND json_extract(
                                            CASE WHEN json_valid(event.details)
                                                 THEN event.details ELSE '{}' END,
                                            '$.user_id'
                                        ) = project_members.user_id
                                    )
                                )
                            )
                            OR (
                                event.action = 'project.owner_transferred'
                                AND (
                                    json_extract(
                                        CASE WHEN json_valid(event.details)
                                             THEN event.details ELSE '{}' END,
                                        '$.from'
                                    ) = project_members.user_id
                                    OR json_extract(
                                        CASE WHEN json_valid(event.details)
                                             THEN event.details ELSE '{}' END,
                                        '$.to'
                                    ) = project_members.user_id
                                )
                            )
                        )
                  )
            )
            "#,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();

        tx.commit().await?;
        tracing::info!(
            migration = LEGACY_PROJECT_ACCESS_MIGRATION,
            repaired_memberships = repaired,
            "completed one-time legacy project access migration"
        );
        Ok(true)
    }

    /// Remove decode-only provider profiles from persisted policy without
    /// changing the meaning or order of any remaining entry. Each changed row
    /// receives a distinct cluster-monotonic version; already-clean rows are
    /// untouched, which makes the migration idempotent.
    async fn normalize_retired_provider_profiles(&self) -> Result<RetiredProfileMigration> {
        fn filtered_profiles(raw: &str) -> Option<Vec<String>> {
            let profiles = serde_json::from_str::<Vec<String>>(raw).ok()?;
            let filtered = profiles
                .iter()
                .filter(|profile| !shared::is_retired_launch_profile_key(profile))
                .cloned()
                .collect::<Vec<_>>();
            (filtered != profiles).then_some(filtered)
        }

        let mut tx = self.pool.begin().await?;
        let mut next_version = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COALESCE(MAX(version), 0) FROM (
                SELECT version FROM cluster_settings WHERE id = 1
                UNION ALL
                SELECT version FROM project_policy_overrides
            )
            "#,
        )
        .fetch_one(&mut *tx)
        .await?;
        let mut result = RetiredProfileMigration::default();

        let cluster =
            sqlx::query("SELECT allowed_launch_profiles FROM cluster_settings WHERE id = 1")
                .fetch_one(&mut *tx)
                .await?;
        let cluster_raw: String = cluster.get("allowed_launch_profiles");
        if let Some(filtered) = filtered_profiles(&cluster_raw) {
            next_version += 1;
            sqlx::query(
                "UPDATE cluster_settings SET allowed_launch_profiles = ?, version = ?, updated_at = CURRENT_TIMESTAMP WHERE id = 1",
            )
            .bind(serde_json::to_string(&filtered)?)
            .bind(next_version)
            .execute(&mut *tx)
            .await?;
            result.cluster_default_changed = true;
        }

        let overrides = sqlx::query(
            "SELECT project_id, allowed_launch_profiles FROM project_policy_overrides WHERE allowed_launch_profiles IS NOT NULL ORDER BY project_id",
        )
        .fetch_all(&mut *tx)
        .await?;
        for row in overrides {
            let project_id: String = row.get("project_id");
            let raw: String = row.get("allowed_launch_profiles");
            let Some(filtered) = filtered_profiles(&raw) else {
                continue;
            };
            next_version += 1;
            sqlx::query(
                "UPDATE project_policy_overrides SET allowed_launch_profiles = ?, version = ?, updated_at = CURRENT_TIMESTAMP WHERE project_id = ?",
            )
            .bind(serde_json::to_string(&filtered)?)
            .bind(next_version)
            .bind(&project_id)
            .execute(&mut *tx)
            .await?;
            result.changed_project_ids.push(project_id);
        }

        tx.commit().await?;
        Ok(result)
    }

    // User operations
    pub async fn create_user(&self, user: &User) -> Result<()> {
        sqlx::query("INSERT INTO users (id, email, password_hash, cluster_role, account_status) VALUES (?, ?, ?, ?, ?)")
            .bind(&user.id)
            .bind(&user.email)
            .bind(&user.password_hash)
            .bind(if user.cluster_role.is_empty() { "user" } else { &user.cluster_role })
            .bind(if user.account_status.is_empty() { "active" } else { &user.account_status })
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_all_users(&self) -> Result<Vec<User>> {
        let users = sqlx::query_as::<_, User>("SELECT id, email, password_hash, created_at, cluster_role, account_status FROM users ORDER BY email")
            .fetch_all(&self.pool)
            .await?;
        Ok(users)
    }

    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<User>> {
        let user = sqlx::query_as::<_, User>(
            "SELECT id, email, password_hash, created_at, cluster_role, account_status FROM users WHERE email = ?",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        Ok(user)
    }

    pub async fn get_user_by_id(&self, id: &str) -> Result<Option<User>> {
        let user = sqlx::query_as::<_, User>(
            "SELECT id, email, password_hash, created_at, cluster_role, account_status FROM users WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(user)
    }

    pub async fn update_user_password(&self, email: &str, password_hash: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let user_id = sqlx::query_scalar::<_, String>("SELECT id FROM users WHERE email = ?")
            .bind(email)
            .fetch_optional(&mut *tx)
            .await?;
        let result = sqlx::query("UPDATE users SET password_hash = ? WHERE email = ?")
            .bind(password_hash)
            .bind(email)
            .execute(&mut *tx)
            .await?;
        if let Some(user_id) = user_id {
            sqlx::query(
                "UPDATE mobile_device_sessions SET revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP), revocation_reason = COALESCE(revocation_reason, 'password_reset') WHERE user_id = ? AND revoked_at IS NULL",
            )
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(result.rows_affected() > 0)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_mobile_device_session(
        &self,
        id: &str,
        user_id: &str,
        installation_id: &str,
        platform: &str,
        device_name: Option<&str>,
        app_version: &str,
        refresh_token_hash: &str,
        expires_at: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let existing_user = sqlx::query_scalar::<_, String>(
            "SELECT user_id FROM mobile_installations WHERE installation_id = ?",
        )
        .bind(installation_id)
        .fetch_optional(&mut *tx)
        .await?;
        anyhow::ensure!(
            existing_user.as_deref().is_none() || existing_user.as_deref() == Some(user_id),
            "installation is already registered to another user"
        );
        sqlx::query(
            r#"
            INSERT INTO mobile_installations
                (installation_id, user_id, platform, device_name, app_version)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(installation_id) DO UPDATE SET
                platform = excluded.platform,
                device_name = excluded.device_name,
                app_version = excluded.app_version,
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(installation_id)
        .bind(user_id)
        .bind(platform)
        .bind(device_name)
        .bind(app_version)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE mobile_device_sessions SET revoked_at = CURRENT_TIMESTAMP, revocation_reason = 'superseded_login' WHERE installation_id = ? AND revoked_at IS NULL",
        )
        .bind(installation_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO mobile_device_sessions
                (id, user_id, installation_id, refresh_token_hash, app_version, expires_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id)
        .bind(user_id)
        .bind(installation_id)
        .bind(refresh_token_hash)
        .bind(app_version)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_mobile_device_session(
        &self,
        id: &str,
    ) -> Result<Option<MobileDeviceSessionRecord>> {
        Ok(sqlx::query_as::<_, MobileDeviceSessionRecord>(
            r#"
            SELECT s.id, s.user_id, s.installation_id, i.platform, i.device_name,
                   s.app_version, s.created_at, s.last_used_at, s.expires_at,
                   s.revoked_at, s.revocation_reason
            FROM mobile_device_sessions s
            JOIN mobile_installations i ON i.installation_id = s.installation_id
            WHERE s.id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn list_mobile_device_sessions(
        &self,
        user_id: &str,
    ) -> Result<Vec<MobileDeviceSessionRecord>> {
        Ok(sqlx::query_as::<_, MobileDeviceSessionRecord>(
            r#"
            SELECT s.id, s.user_id, s.installation_id, i.platform, i.device_name,
                   s.app_version, s.created_at, s.last_used_at, s.expires_at,
                   s.revoked_at, s.revocation_reason
            FROM mobile_device_sessions s
            JOIN mobile_installations i ON i.installation_id = s.installation_id
            WHERE s.user_id = ?
            ORDER BY s.created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn is_mobile_device_session_active(&self, id: &str, user_id: &str) -> Result<bool> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM mobile_device_sessions WHERE id = ? AND user_id = ? AND revoked_at IS NULL AND datetime(expires_at) > datetime('now')",
        )
        .bind(id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count == 1)
    }

    pub async fn mobile_refresh_token_matches(
        &self,
        session_id: &str,
        refresh_token_hash: &str,
    ) -> Result<bool> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM mobile_device_sessions WHERE id = ? AND refresh_token_hash = ? AND revoked_at IS NULL AND datetime(expires_at) > datetime('now')",
        )
        .bind(session_id)
        .bind(refresh_token_hash)
        .fetch_one(&self.pool)
        .await?
            == 1)
    }

    pub async fn rotate_mobile_refresh_token(
        &self,
        current_hash: &str,
        installation_id: &str,
        new_hash: &str,
    ) -> Result<Result<MobileDeviceSessionRecord, MobileRefreshFailure>> {
        let mut tx = self.pool.begin().await?;
        // Make the compare-and-swap the transaction's first statement. Two
        // deferred SQLite transactions that both read before writing can form
        // snapshots that cannot be upgraded, yielding SQLITE_BUSY immediately
        // despite busy_timeout. Beginning with the write serializes refresh
        // races and lets the loser inspect the winner's committed history.
        let updated = sqlx::query(
            r#"UPDATE mobile_device_sessions
               SET refresh_token_hash = ?, last_used_at = CURRENT_TIMESTAMP
               WHERE refresh_token_hash = ?
                 AND installation_id = ?
                 AND revoked_at IS NULL
                 AND datetime(expires_at) > datetime('now')"#,
        )
        .bind(new_hash)
        .bind(current_hash)
        .bind(installation_id)
        .execute(&mut *tx)
        .await?;

        if updated.rows_affected() == 1 {
            let session_id = sqlx::query_scalar::<_, String>(
                "SELECT id FROM mobile_device_sessions WHERE refresh_token_hash = ?",
            )
            .bind(new_hash)
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO mobile_refresh_token_history (token_hash, device_session_id) VALUES (?, ?)",
            )
            .bind(current_hash)
            .bind(&session_id)
            .execute(&mut *tx)
            .await?;
            let record = sqlx::query_as::<_, MobileDeviceSessionRecord>(
                r#"
                SELECT s.id, s.user_id, s.installation_id, i.platform, i.device_name,
                       s.app_version, s.created_at, s.last_used_at, s.expires_at,
                       s.revoked_at, s.revocation_reason
                FROM mobile_device_sessions s
                JOIN mobile_installations i ON i.installation_id = s.installation_id
                WHERE s.id = ?
                "#,
            )
            .bind(&session_id)
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(Ok(record));
        }

        let row: Option<(String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT id, installation_id, revoked_at, expires_at FROM mobile_device_sessions WHERE refresh_token_hash = ?",
        )
        .bind(current_hash)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((session_id, bound_installation, revoked_at, expires_at)) = row {
            if bound_installation != installation_id {
                sqlx::query(
                    "UPDATE mobile_device_sessions SET revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP), revocation_reason = 'installation_mismatch' WHERE id = ?",
                )
                .bind(&session_id)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                return Ok(Err(MobileRefreshFailure::InstallationMismatch));
            }
            if revoked_at.is_some() {
                tx.commit().await?;
                return Ok(Err(MobileRefreshFailure::Revoked));
            }
            let expires = chrono::DateTime::parse_from_rfc3339(&expires_at)
                .map(|value| value.with_timezone(&chrono::Utc))
                .ok();
            if expires.is_none_or(|value| value <= chrono::Utc::now()) {
                sqlx::query(
                    "UPDATE mobile_device_sessions SET revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP), revocation_reason = 'expired' WHERE id = ?",
                )
                .bind(&session_id)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                return Ok(Err(MobileRefreshFailure::Expired));
            }

            // A matching active row that lost the compare-and-swap is a
            // concurrent-use signal. Revoke it rather than guessing which
            // caller should retain the credential.
            sqlx::query(
                "UPDATE mobile_device_sessions SET revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP), revocation_reason = 'concurrent_refresh_reuse' WHERE id = ?",
            )
            .bind(&session_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(Err(MobileRefreshFailure::Reused));
        }

        let reused_session = sqlx::query_scalar::<_, String>(
            "SELECT device_session_id FROM mobile_refresh_token_history WHERE token_hash = ?",
        )
        .bind(current_hash)
        .fetch_optional(&mut *tx)
        .await?;
        let failure = if let Some(session_id) = reused_session {
            sqlx::query(
                "UPDATE mobile_device_sessions SET revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP), revocation_reason = 'refresh_token_reuse' WHERE id = ?",
            )
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
            MobileRefreshFailure::Reused
        } else {
            MobileRefreshFailure::Invalid
        };
        tx.commit().await?;
        Ok(Err(failure))
    }

    pub async fn revoke_mobile_device_session(
        &self,
        actor_user_id: &str,
        session_id: &str,
        allow_any_user: bool,
        reason: &str,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let owner = sqlx::query_scalar::<_, String>(
            "SELECT user_id FROM mobile_device_sessions WHERE id = ?",
        )
        .bind(session_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(owner) = owner else {
            tx.commit().await?;
            return Ok(false);
        };
        anyhow::ensure!(
            allow_any_user || owner == actor_user_id,
            "device session access denied"
        );
        let changed = sqlx::query(
            "UPDATE mobile_device_sessions SET revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP), revocation_reason = COALESCE(revocation_reason, ?) WHERE id = ?",
        )
        .bind(reason)
        .bind(session_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        if changed {
            Self::insert_audit_tx(
                &mut tx,
                actor_user_id,
                "mobile.device_session_revoked",
                "mobile_device_session",
                session_id,
                Some(serde_json::json!({ "reason": reason, "owner_user_id": owner })),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(changed)
    }

    pub async fn revoke_mobile_device_sessions_for_user(
        &self,
        user_id: &str,
        reason: &str,
    ) -> Result<u64> {
        Ok(sqlx::query(
            "UPDATE mobile_device_sessions SET revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP), revocation_reason = COALESCE(revocation_reason, ?) WHERE user_id = ? AND revoked_at IS NULL",
        )
        .bind(reason)
        .bind(user_id)
        .execute(&self.pool)
        .await?
        .rows_affected())
    }

    pub async fn register_mobile_push_token(
        &self,
        token_id: &str,
        user_id: &str,
        device_session_id: &str,
        installation_id: &str,
        platform: &str,
        token: &str,
        token_hash: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let allowed = sqlx::query_scalar::<_, i64>(
            r#"SELECT COUNT(*) FROM mobile_device_sessions
               WHERE id = ? AND user_id = ? AND installation_id = ?
                 AND revoked_at IS NULL AND datetime(expires_at) > datetime('now')"#,
        )
        .bind(device_session_id)
        .bind(user_id)
        .bind(installation_id)
        .fetch_one(&mut *tx)
        .await?
            > 0;
        anyhow::ensure!(
            allowed,
            "mobile device session does not own this installation"
        );
        sqlx::query(
            "UPDATE mobile_push_tokens SET retired_at = COALESCE(retired_at, CURRENT_TIMESTAMP), retirement_reason = COALESCE(retirement_reason, 'rotated') WHERE installation_id = ? AND token_hash != ? AND retired_at IS NULL",
        )
        .bind(installation_id)
        .bind(token_hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO mobile_push_tokens
                 (id, installation_id, device_session_id, platform, token, token_hash)
               VALUES (?, ?, ?, ?, ?, ?)
               ON CONFLICT(token_hash) DO UPDATE SET
                 installation_id = excluded.installation_id,
                 device_session_id = excluded.device_session_id,
                 platform = excluded.platform,
                 token = excluded.token,
                 retired_at = NULL,
                 retirement_reason = NULL,
                 updated_at = CURRENT_TIMESTAMP"#,
        )
        .bind(token_id)
        .bind(installation_id)
        .bind(device_session_id)
        .bind(platform)
        .bind(token)
        .bind(token_hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT OR IGNORE INTO mobile_notification_preferences(installation_id) VALUES (?)",
        )
        .bind(installation_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn claim_mobile_task_launch(
        &self,
        request_id: &str,
        user_id: &str,
        device_session_id: &str,
        request_fingerprint: &str,
        machine_id: &str,
        project_id: &str,
    ) -> Result<MobileTaskLaunchRecord> {
        let mut tx = self.pool.begin().await?;
        let authorized_device = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM mobile_device_sessions WHERE id = ? AND user_id = ? AND revoked_at IS NULL AND datetime(expires_at) > datetime('now')",
        )
        .bind(device_session_id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?
            == 1;
        anyhow::ensure!(authorized_device, "mobile device session is unavailable");
        sqlx::query(
            r#"INSERT OR IGNORE INTO mobile_task_launches
                 (request_id, user_id, device_session_id, request_fingerprint, machine_id, project_id)
               VALUES (?, ?, ?, ?, ?, ?)"#,
        )
        .bind(request_id)
        .bind(user_id)
        .bind(device_session_id)
        .bind(request_fingerprint)
        .bind(machine_id)
        .bind(project_id)
        .execute(&mut *tx)
        .await?;
        let record = sqlx::query_as::<_, MobileTaskLaunchRecord>(
            r#"SELECT request_id, user_id, device_session_id, request_fingerprint,
                      machine_id, project_id, status, session_id, pane_id, error_message
               FROM mobile_task_launches WHERE request_id = ?"#,
        )
        .bind(request_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(record)
    }

    pub async fn complete_mobile_task_launch(
        &self,
        request_id: &str,
        user_id: &str,
        session_id: &str,
        pane_id: u32,
    ) -> Result<bool> {
        Ok(sqlx::query(
            r#"UPDATE mobile_task_launches
               SET status = 'completed', session_id = ?, pane_id = ?, error_message = NULL,
                   updated_at = CURRENT_TIMESTAMP
               WHERE request_id = ? AND user_id = ? AND status = 'pending'"#,
        )
        .bind(session_id)
        .bind(i64::from(pane_id))
        .bind(request_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1)
    }

    pub async fn fail_mobile_task_launch(
        &self,
        request_id: &str,
        user_id: &str,
        error_message: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"UPDATE mobile_task_launches
               SET status = 'failed', error_message = ?, updated_at = CURRENT_TIMESTAMP
               WHERE request_id = ? AND user_id = ? AND status = 'pending'"#,
        )
        .bind(error_message)
        .bind(request_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mobile_notification_preferences(
        &self,
        user_id: &str,
        device_session_id: &str,
    ) -> Result<Option<shared::MobileNotificationPreferences>> {
        let row = sqlx::query(
            r#"SELECT p.decisions, p.failures, p.pull_requests, p.completions
               FROM mobile_notification_preferences p
               JOIN mobile_device_sessions s ON s.installation_id = p.installation_id
               WHERE s.id = ? AND s.user_id = ? AND s.revoked_at IS NULL"#,
        )
        .bind(device_session_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| shared::MobileNotificationPreferences {
            decisions: row.get::<i64, _>("decisions") != 0,
            failures: row.get::<i64, _>("failures") != 0,
            pull_requests: row.get::<i64, _>("pull_requests") != 0,
            completions: row.get::<i64, _>("completions") != 0,
        }))
    }

    pub async fn update_mobile_notification_preferences(
        &self,
        user_id: &str,
        device_session_id: &str,
        preferences: &shared::MobileNotificationPreferences,
    ) -> Result<bool> {
        let installation_id = sqlx::query_scalar::<_, String>(
            "SELECT installation_id FROM mobile_device_sessions WHERE id = ? AND user_id = ? AND revoked_at IS NULL",
        )
        .bind(device_session_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(installation_id) = installation_id else {
            return Ok(false);
        };
        sqlx::query(
            r#"INSERT INTO mobile_notification_preferences
                 (installation_id, decisions, failures, pull_requests, completions)
               VALUES (?, ?, ?, ?, ?)
               ON CONFLICT(installation_id) DO UPDATE SET
                 decisions = excluded.decisions,
                 failures = excluded.failures,
                 pull_requests = excluded.pull_requests,
                 completions = excluded.completions,
                 updated_at = CURRENT_TIMESTAMP"#,
        )
        .bind(&installation_id)
        .bind(preferences.decisions)
        .bind(preferences.failures)
        .bind(preferences.pull_requests)
        .bind(preferences.completions)
        .execute(&self.pool)
        .await?;
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn enqueue_mobile_notification_event(
        &self,
        event_id: &str,
        user_id: &str,
        project_id: &str,
        session_id: Option<&str>,
        pane_id: Option<u32>,
        category: &str,
        routing_id: &str,
        dedupe_key: &str,
    ) -> Result<bool> {
        anyhow::ensure!(
            matches!(
                category,
                "decision" | "failure" | "pull_request" | "completion"
            ),
            "invalid notification category"
        );
        anyhow::ensure!(
            !routing_id.is_empty() && routing_id.len() <= 200,
            "invalid notification routing identifier"
        );
        let mut tx = self.pool.begin().await?;
        let authorized = sqlx::query_scalar::<_, i64>(
            r#"SELECT COUNT(*)
               FROM users u
               JOIN projects p ON p.id = ?
               LEFT JOIN project_members pm ON pm.project_id = p.id AND pm.user_id = u.id
               WHERE u.id = ? AND u.account_status = 'active' AND p.lifecycle_status = 'active'
                 AND (p.owner_user_id = u.id OR pm.user_id IS NOT NULL)
                 AND (? IS NULL OR EXISTS (
                   SELECT 1 FROM sessions s
                   WHERE s.id = ? AND COALESCE(s.project_id, s.id) = p.id
                 ))"#,
        )
        .bind(project_id)
        .bind(user_id)
        .bind(session_id)
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await?
            > 0;
        if !authorized {
            tx.commit().await?;
            return Ok(false);
        }
        let inserted = sqlx::query(
            r#"INSERT OR IGNORE INTO mobile_notification_events
                 (id, user_id, project_id, session_id, pane_id, category, routing_id, dedupe_key)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(event_id)
        .bind(user_id)
        .bind(project_id)
        .bind(session_id)
        .bind(pane_id.map(i64::from))
        .bind(category)
        .bind(routing_id)
        .bind(dedupe_key)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        if !inserted {
            tx.commit().await?;
            return Ok(false);
        }
        sqlx::query(
            r#"INSERT OR IGNORE INTO mobile_notification_deliveries(event_id, push_token_id)
               SELECT ?, t.id
               FROM mobile_push_tokens t
               JOIN mobile_installations i ON i.installation_id = t.installation_id
               JOIN mobile_device_sessions s ON s.id = t.device_session_id
               JOIN mobile_notification_preferences p ON p.installation_id = i.installation_id
               WHERE i.user_id = ? AND t.retired_at IS NULL AND s.revoked_at IS NULL
                 AND datetime(s.expires_at) > datetime('now')
                 AND CASE ?
                   WHEN 'decision' THEN p.decisions
                   WHEN 'failure' THEN p.failures
                   WHEN 'pull_request' THEN p.pull_requests
                   WHEN 'completion' THEN p.completions
                   ELSE 0
                 END = 1"#,
        )
        .bind(event_id)
        .bind(user_id)
        .bind(category)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn claim_mobile_notification_deliveries(
        &self,
        limit: usize,
    ) -> Result<Vec<MobileNotificationDeliveryRecord>> {
        let mut tx = self.pool.begin().await?;
        // Authorization may change after enqueue and before the provider
        // worker claims a delivery. Erase stale logical events first so
        // suspended accounts, lost project membership, and deleted sessions
        // can never receive a delayed notification or leave a queued backlog.
        sqlx::query(
            r#"DELETE FROM mobile_notification_events
               WHERE NOT EXISTS (
                   SELECT 1 FROM users u
                   WHERE u.id = mobile_notification_events.user_id
                     AND u.account_status = 'active'
               )
               OR (
                   project_id IS NOT NULL AND NOT EXISTS (
                       SELECT 1 FROM projects p
                       WHERE p.id = mobile_notification_events.project_id
                         AND p.lifecycle_status = 'active'
                         AND (
                             p.owner_user_id = mobile_notification_events.user_id
                             OR EXISTS (
                                 SELECT 1 FROM project_members pm
                                 WHERE pm.project_id = p.id
                                   AND pm.user_id = mobile_notification_events.user_id
                             )
                         )
                   )
               )
               OR (
                   session_id IS NOT NULL AND NOT EXISTS (
                       SELECT 1 FROM sessions s
                       WHERE s.id = mobile_notification_events.session_id
                         AND (
                             mobile_notification_events.project_id IS NULL
                             OR COALESCE(s.project_id, s.id) = mobile_notification_events.project_id
                         )
                   )
               )"#,
        )
        .execute(&mut *tx)
        .await?;
        let rows = sqlx::query_as::<_, MobileNotificationDeliveryRecord>(
            r#"SELECT d.id, d.event_id, d.push_token_id, t.token, e.category,
                      e.routing_id, e.session_id, d.attempt_count, d.provider_ticket_id
               FROM mobile_notification_deliveries d
               JOIN mobile_notification_events e ON e.id = d.event_id
               JOIN mobile_push_tokens t ON t.id = d.push_token_id
               JOIN users u ON u.id = e.user_id
               LEFT JOIN projects p ON p.id = e.project_id
               WHERE d.status IN ('queued', 'retry')
                 AND datetime(d.next_attempt_at) <= datetime('now')
                 AND t.retired_at IS NULL AND u.account_status = 'active'
                 AND (e.project_id IS NULL OR p.lifecycle_status = 'active')
               ORDER BY d.id LIMIT ?"#,
        )
        .bind(limit.min(100) as i64)
        .fetch_all(&mut *tx)
        .await?;
        for row in &rows {
            sqlx::query(
                "UPDATE mobile_notification_deliveries SET status = 'sending', attempt_count = attempt_count + 1, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
            )
            .bind(row.id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(rows)
    }

    pub async fn pending_mobile_notification_receipts(
        &self,
        limit: usize,
    ) -> Result<Vec<MobileNotificationDeliveryRecord>> {
        Ok(sqlx::query_as::<_, MobileNotificationDeliveryRecord>(
            r#"SELECT d.id, d.event_id, d.push_token_id, t.token, e.category,
                      e.routing_id, e.session_id, d.attempt_count, d.provider_ticket_id
               FROM mobile_notification_deliveries d
               JOIN mobile_notification_events e ON e.id = d.event_id
               JOIN mobile_push_tokens t ON t.id = d.push_token_id
               WHERE d.status = 'ticketed' AND d.provider_ticket_id IS NOT NULL
                 AND datetime(d.updated_at) <= datetime('now', '-15 seconds')
               ORDER BY d.id LIMIT ?"#,
        )
        .bind(limit.min(100) as i64)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn mark_mobile_delivery_ticketed(&self, id: i64, ticket_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE mobile_notification_deliveries SET status = 'ticketed', provider_ticket_id = ?, provider_error = NULL, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(ticket_id)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_mobile_delivery_delivered(&self, id: i64) -> Result<()> {
        sqlx::query(
            "UPDATE mobile_notification_deliveries SET status = 'delivered', provider_error = NULL, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn retry_mobile_delivery(
        &self,
        id: i64,
        error: &str,
        delay_seconds: u64,
    ) -> Result<()> {
        let modifier = format!("+{} seconds", delay_seconds.min(3600));
        sqlx::query(
            "UPDATE mobile_notification_deliveries SET status = 'retry', provider_error = ?, provider_ticket_id = NULL, next_attempt_at = datetime('now', ?), updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(error)
        .bind(modifier)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn retire_mobile_push_token(
        &self,
        token_id: &str,
        delivery_id: i64,
        reason: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE mobile_push_tokens SET retired_at = COALESCE(retired_at, CURRENT_TIMESTAMP), retirement_reason = COALESCE(retirement_reason, ?) WHERE id = ?",
        )
        .bind(reason)
        .bind(token_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE mobile_notification_deliveries SET status = 'permanent_failure', provider_error = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(reason)
        .bind(delivery_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn recover_mobile_notification_outbox(&self) -> Result<u64> {
        Ok(sqlx::query(
            "UPDATE mobile_notification_deliveries SET status = 'retry', next_attempt_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP WHERE status = 'sending'",
        )
        .execute(&self.pool)
        .await?
            .rows_affected())
    }

    /// Persistent gauges for the admin-only mobile operational view. The
    /// result intentionally contains counts and app versions only—never raw
    /// credentials, installation identifiers, push tokens, or project data.
    pub async fn mobile_persistence_metrics(&self) -> Result<MobilePersistenceMetrics> {
        let active_device_sessions = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM mobile_device_sessions WHERE revoked_at IS NULL AND datetime(expires_at) > datetime('now')",
        )
        .fetch_one(&self.pool)
        .await?;
        let active_push_tokens = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM mobile_push_tokens WHERE retired_at IS NULL",
        )
        .fetch_one(&self.pool)
        .await?;
        let pending_task_launches = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM mobile_task_launches WHERE status = 'pending'",
        )
        .fetch_one(&self.pool)
        .await?;
        let outbox = sqlx::query_as::<_, (String, i64)>(
            "SELECT status, COUNT(*) FROM mobile_notification_deliveries GROUP BY status",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
        let app_versions = sqlx::query_as::<_, (String, i64)>(
            r#"SELECT app_version, COUNT(*)
               FROM mobile_device_sessions
               WHERE revoked_at IS NULL AND datetime(expires_at) > datetime('now')
               GROUP BY app_version ORDER BY COUNT(*) DESC, app_version"#,
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(
            |(app_version, active_device_sessions)| MobileAppVersionCount {
                app_version,
                active_device_sessions,
            },
        )
        .collect();
        Ok(MobilePersistenceMetrics {
            active_device_sessions,
            active_push_tokens,
            pending_task_launches,
            outbox_queued: *outbox.get("queued").unwrap_or(&0),
            outbox_sending: *outbox.get("sending").unwrap_or(&0),
            outbox_ticketed: *outbox.get("ticketed").unwrap_or(&0),
            outbox_retry: *outbox.get("retry").unwrap_or(&0),
            outbox_permanent_failure: *outbox.get("permanent_failure").unwrap_or(&0),
            app_versions,
        })
    }

    pub async fn active_project_user_ids(&self, project_id: &str) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar::<_, String>(
            r#"SELECT u.id
               FROM users u
               WHERE u.account_status = 'active' AND u.id IN (
                   SELECT owner_user_id FROM projects
                   WHERE id = ? AND lifecycle_status = 'active'
                   UNION
                   SELECT user_id FROM project_members WHERE project_id = ?
               )
               ORDER BY u.id"#,
        )
        .bind(project_id)
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn bootstrap_cluster_admin(&self, email: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let active_admins = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM users WHERE cluster_role = 'admin' AND account_status = 'active'",
        )
        .fetch_one(&mut *tx)
        .await?;
        if active_admins > 0 {
            tx.commit().await?;
            return Ok(false);
        }
        let result = sqlx::query(
            "UPDATE users SET cluster_role = 'admin', account_status = 'active' WHERE lower(email) = lower(?)",
        )
        .bind(email.trim())
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() > 0 {
            let admin_id = sqlx::query_scalar::<_, String>(
                "SELECT id FROM users WHERE lower(email) = lower(?)",
            )
            .bind(email.trim())
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO admin_audit_events (actor_user_id, action, target_type, target_id, details) VALUES (?, 'migration.bootstrap_cluster_admin', 'user', ?, ?)",
            )
            .bind(&admin_id)
            .bind(&admin_id)
            .bind(serde_json::json!({ "email": email.trim() }).to_string())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_cluster_users(
        &self,
        search: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ClusterUserSummary>> {
        let pattern = format!("%{}%", search.unwrap_or_default().trim());
        Ok(sqlx::query_as::<_, ClusterUserSummary>(
            r#"
            SELECT id, email, cluster_role, account_status, created_at
            FROM users
            WHERE (? = '%%' OR lower(email) LIKE lower(?))
            ORDER BY email
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(&pattern)
        .bind(&pattern)
        .bind(limit.clamp(1, 200))
        .bind(offset.max(0))
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn update_cluster_user_role(
        &self,
        actor_user_id: &str,
        target_user_id: &str,
        role: ClusterRole,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let current: Option<(String, String)> =
            sqlx::query_as("SELECT cluster_role, account_status FROM users WHERE id = ?")
                .bind(target_user_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((current_role, current_status)) = current else {
            tx.commit().await?;
            return Ok(false);
        };
        if current_role == "admin" && role != ClusterRole::Admin && current_status == "active" {
            let admins = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM users WHERE cluster_role = 'admin' AND account_status = 'active'",
            )
            .fetch_one(&mut *tx)
            .await?;
            anyhow::ensure!(
                admins > 1,
                "cannot demote the last active cluster administrator"
            );
        }
        let changed = sqlx::query("UPDATE users SET cluster_role = ? WHERE id = ?")
            .bind(role.as_str())
            .bind(target_user_id)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
        if changed {
            Self::insert_audit_tx(
                &mut tx,
                actor_user_id,
                "cluster_user.role_changed",
                "user",
                target_user_id,
                Some(serde_json::json!({ "from": current_role, "to": role.as_str() })),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(changed)
    }

    pub async fn update_cluster_user_status(
        &self,
        actor_user_id: &str,
        target_user_id: &str,
        status: AccountStatus,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let current: Option<(String, String)> =
            sqlx::query_as("SELECT cluster_role, account_status FROM users WHERE id = ?")
                .bind(target_user_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((role, current_status)) = current else {
            tx.commit().await?;
            return Ok(false);
        };
        if role == "admin" && current_status == "active" && status == AccountStatus::Suspended {
            let admins = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM users WHERE cluster_role = 'admin' AND account_status = 'active'",
            )
            .fetch_one(&mut *tx)
            .await?;
            anyhow::ensure!(
                admins > 1,
                "cannot suspend the last active cluster administrator"
            );
        }
        let changed = sqlx::query("UPDATE users SET account_status = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(target_user_id)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
        if changed && status == AccountStatus::Suspended {
            sqlx::query(
                "UPDATE mobile_device_sessions SET revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP), revocation_reason = COALESCE(revocation_reason, 'account_suspended') WHERE user_id = ? AND revoked_at IS NULL",
            )
            .bind(target_user_id)
            .execute(&mut *tx)
            .await?;
        }
        if changed {
            Self::insert_audit_tx(
                &mut tx,
                actor_user_id,
                "cluster_user.status_changed",
                "user",
                target_user_id,
                Some(serde_json::json!({ "from": current_status, "to": status.as_str() })),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(changed)
    }

    pub async fn create_cluster_invitation(&self, invitation: &ClusterInvitation) -> Result<()> {
        sqlx::query(
            "INSERT INTO cluster_invitations (code, email, created_by, expires_at) VALUES (?, lower(?), ?, ?)",
        )
        .bind(&invitation.code)
        .bind(invitation.email.trim())
        .bind(&invitation.created_by)
        .bind(&invitation.expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_cluster_invitation(&self, code: &str) -> Result<Option<ClusterInvitation>> {
        Ok(sqlx::query_as::<_, ClusterInvitation>(
            "SELECT code, email, created_by, expires_at, redeemed_at, created_at FROM cluster_invitations WHERE code = ?",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn redeem_cluster_invitation(&self, code: &str) -> Result<bool> {
        Ok(sqlx::query(
            "UPDATE cluster_invitations SET redeemed_at = CURRENT_TIMESTAMP WHERE code = ? AND redeemed_at IS NULL",
        )
        .bind(code)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }

    /// Atomically consume an administrator-created invitation and create its
    /// active cluster-user account. If another registration wins the code or
    /// user insertion fails, neither half is committed.
    pub async fn create_user_redeeming_cluster_invitation(
        &self,
        user: &User,
        code: &str,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let consumed = sqlx::query(
            r#"
            UPDATE cluster_invitations
            SET redeemed_at = CURRENT_TIMESTAMP
            WHERE code = ?
              AND redeemed_at IS NULL
              AND lower(email) = lower(?)
              AND datetime(expires_at) > CURRENT_TIMESTAMP
            "#,
        )
        .bind(code)
        .bind(user.email.trim())
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        if !consumed {
            tx.rollback().await?;
            return Ok(false);
        }
        let actor_user_id = sqlx::query_scalar::<_, String>(
            "SELECT created_by FROM cluster_invitations WHERE code = ?",
        )
        .bind(code)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO users (id, email, password_hash, cluster_role, account_status) VALUES (?, ?, ?, 'user', 'active')")
            .bind(&user.id)
            .bind(user.email.trim())
            .bind(&user.password_hash)
            .execute(&mut *tx)
            .await?;
        Self::insert_audit_tx(
            &mut tx,
            &actor_user_id,
            "cluster_user.created",
            "user",
            &user.id,
            Some(serde_json::json!({ "email": user.email })),
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    async fn insert_audit_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        actor_user_id: &str,
        action: &str,
        target_type: &str,
        target_id: &str,
        details: Option<serde_json::Value>,
    ) -> Result<()> {
        let project_id = if target_type == "project" {
            Some(target_id.to_string())
        } else {
            details
                .as_ref()
                .and_then(|value| value.get("project_id"))
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
        };
        sqlx::query(
            "INSERT INTO admin_audit_events (actor_user_id, action, target_type, target_id, project_id, details) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(actor_user_id)
        .bind(action)
        .bind(target_type)
        .bind(target_id)
        .bind(project_id)
        .bind(details.map(|value| value.to_string()))
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn record_audit(
        &self,
        actor_user_id: &str,
        action: &str,
        target_type: &str,
        target_id: &str,
        details: Option<serde_json::Value>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        Self::insert_audit_tx(
            &mut tx,
            actor_user_id,
            action,
            target_type,
            target_id,
            details,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_audit_events(&self, limit: i64, offset: i64) -> Result<Vec<AdminAuditEvent>> {
        Ok(sqlx::query_as::<_, AdminAuditEvent>(
            "SELECT id, actor_user_id, action, target_type, target_id, project_id, details, created_at FROM admin_audit_events ORDER BY id DESC LIMIT ? OFFSET ?",
        )
        .bind(limit.clamp(1, 200))
        .bind(offset.max(0))
        .fetch_all(&self.pool)
        .await?)
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
            "INSERT OR IGNORE INTO projects (id, owner_user_id, lifecycle_status) VALUES (?, ?, 'active')",
        )
        .bind(&project_id)
        .bind(&session.user_id)
        .execute(&self.pool)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO sessions (id, user_id, cli_client_id, working_dir, hostname, status, project_id, git_remote, git_remote_url)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                cli_client_id = excluded.cli_client_id,
                working_dir = excluded.working_dir,
                hostname = excluded.hostname,
                status = excluded.status,
                project_id = excluded.project_id,
                git_remote = excluded.git_remote,
                git_remote_url = excluded.git_remote_url,
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
        .bind(&session.git_remote_url)
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

    pub async fn get_project(&self, project_id: &str) -> Result<Option<Project>> {
        Ok(sqlx::query_as::<_, Project>(
            "SELECT id, owner_user_id, lifecycle_status, created_at, updated_at FROM projects WHERE id = ?",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn get_project_for_session(&self, session_id: &str) -> Result<Option<Project>> {
        Ok(sqlx::query_as::<_, Project>(
            r#"
            SELECT p.id, p.owner_user_id, p.lifecycle_status, p.created_at, p.updated_at
            FROM sessions s
            JOIN projects p ON p.id = COALESCE(s.project_id, s.id)
            WHERE s.id = ?
            "#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn authorize_project_registration(
        &self,
        project_id: &str,
        user_id: &str,
    ) -> Result<Project> {
        let user = self
            .get_user_by_id(user_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("cluster account not found"))?;
        anyhow::ensure!(user.is_active(), "cluster account is suspended");
        sqlx::query(
            "INSERT OR IGNORE INTO projects (id, owner_user_id, lifecycle_status) VALUES (?, ?, 'active')",
        )
        .bind(project_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        let project = self
            .get_project(project_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("project could not be created"))?;
        match project.lifecycle() {
            ProjectLifecycle::Active => {}
            ProjectLifecycle::Suspended => anyhow::bail!("project is suspended"),
            ProjectLifecycle::Deleting => anyhow::bail!("project deletion is in progress"),
        }
        let has_access = project.owner_user_id == user_id
            || sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM project_members WHERE project_id = ? AND user_id = ?",
            )
            .bind(project_id)
            .bind(user_id)
            .fetch_one(&self.pool)
            .await?
                > 0;
        anyhow::ensure!(has_access, "user is not a member of this project");
        Ok(project)
    }

    pub async fn get_project_role_for_user(
        &self,
        project_id: &str,
        user_id: &str,
    ) -> Result<Option<String>> {
        let owner =
            sqlx::query_scalar::<_, String>("SELECT owner_user_id FROM projects WHERE id = ?")
                .bind(project_id)
                .fetch_optional(&self.pool)
                .await?;
        if owner.as_deref() == Some(user_id) {
            return Ok(Some("owner".to_string()));
        }
        let member = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM project_members WHERE project_id = ? AND user_id = ?",
        )
        .bind(project_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok((member > 0).then(|| "user".to_string()))
    }

    pub async fn list_project_members(&self, project_id: &str) -> Result<Vec<ProjectMemberInfo>> {
        Ok(sqlx::query_as::<_, ProjectMemberInfo>(
            r#"
            SELECT u.id AS user_id, u.email, pm.created_at
            FROM project_members pm
            JOIN users u ON u.id = pm.user_id
            WHERE pm.project_id = ?
            ORDER BY u.email
            "#,
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn add_project_member(
        &self,
        actor_user_id: &str,
        project_id: &str,
        user_id: &str,
    ) -> Result<bool> {
        let user = self
            .get_user_by_id(user_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("cluster account not found"))?;
        anyhow::ensure!(user.is_active(), "target cluster account is suspended");
        let project = self
            .get_project(project_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("project not found"))?;
        anyhow::ensure!(
            project.lifecycle() == ProjectLifecycle::Active,
            "project is not active"
        );
        anyhow::ensure!(
            project.owner_user_id != user_id,
            "owner is already part of the project"
        );
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query(
            "INSERT OR IGNORE INTO project_members (project_id, user_id, invited_by) VALUES (?, ?, ?)",
        )
        .bind(project_id)
        .bind(user_id)
        .bind(actor_user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        if changed {
            Self::insert_audit_tx(
                &mut tx,
                actor_user_id,
                "project.member_added",
                "project_member",
                user_id,
                Some(serde_json::json!({ "project_id": project_id })),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(changed)
    }

    pub async fn remove_project_member(
        &self,
        actor_user_id: &str,
        project_id: &str,
        user_id: &str,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let lifecycle =
            sqlx::query_scalar::<_, String>("SELECT lifecycle_status FROM projects WHERE id = ?")
                .bind(project_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| anyhow::anyhow!("project not found"))?;
        anyhow::ensure!(
            ProjectLifecycle::parse(&lifecycle) != ProjectLifecycle::Deleting,
            "project deletion is in progress"
        );
        let changed =
            sqlx::query("DELETE FROM project_members WHERE project_id = ? AND user_id = ?")
                .bind(project_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                > 0;
        if changed {
            sqlx::query(
                "DELETE FROM session_shares WHERE user_id = ? AND session_id IN (SELECT id FROM sessions WHERE COALESCE(project_id, id) = ?)",
            )
            .bind(user_id)
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
            Self::insert_audit_tx(
                &mut tx,
                actor_user_id,
                "project.member_removed",
                "project_member",
                user_id,
                Some(serde_json::json!({ "project_id": project_id })),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(changed)
    }

    pub async fn transfer_project_ownership(
        &self,
        actor_user_id: &str,
        project_id: &str,
        new_owner_user_id: &str,
    ) -> Result<bool> {
        self.transfer_project_ownership_with_policy(
            actor_user_id,
            project_id,
            new_owner_user_id,
            OwnershipTransferPolicy::ClusterAdministrator,
        )
        .await
    }

    pub async fn transfer_project_ownership_by_owner(
        &self,
        actor_user_id: &str,
        project_id: &str,
        new_owner_user_id: &str,
    ) -> Result<bool> {
        self.transfer_project_ownership_with_policy(
            actor_user_id,
            project_id,
            new_owner_user_id,
            OwnershipTransferPolicy::CurrentOwner,
        )
        .await
    }

    async fn transfer_project_ownership_with_policy(
        &self,
        actor_user_id: &str,
        project_id: &str,
        new_owner_user_id: &str,
        policy: OwnershipTransferPolicy,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let project = sqlx::query_as::<_, (String, String)>(
            "SELECT owner_user_id, lifecycle_status FROM projects WHERE id = ?",
        )
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((old_owner, lifecycle_raw)) = project else {
            tx.commit().await?;
            return Ok(false);
        };
        let lifecycle = ProjectLifecycle::parse(&lifecycle_raw);
        anyhow::ensure!(
            lifecycle != ProjectLifecycle::Deleting,
            "project deletion is in progress"
        );
        if policy == OwnershipTransferPolicy::CurrentOwner {
            anyhow::ensure!(
                old_owner == actor_user_id,
                "only the project owner can transfer ownership"
            );
            anyhow::ensure!(
                lifecycle == ProjectLifecycle::Active,
                "ownership can only be transferred for an active project"
            );
        }
        if old_owner == new_owner_user_id {
            tx.commit().await?;
            return Ok(false);
        }
        let target_status =
            sqlx::query_scalar::<_, String>("SELECT account_status FROM users WHERE id = ?")
                .bind(new_owner_user_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| anyhow::anyhow!("target cluster account not found"))?;
        anyhow::ensure!(
            AccountStatus::parse(&target_status) == AccountStatus::Active,
            "target cluster account is suspended"
        );
        if policy == OwnershipTransferPolicy::CurrentOwner {
            let is_member = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM project_members WHERE project_id = ? AND user_id = ?",
            )
            .bind(project_id)
            .bind(new_owner_user_id)
            .fetch_one(&mut *tx)
            .await?
                > 0;
            anyhow::ensure!(
                is_member,
                "ownership can only be transferred to an existing project user"
            );
        }
        let changed = sqlx::query(
            "UPDATE projects SET owner_user_id = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ? AND owner_user_id = ? AND lifecycle_status != 'deleting'",
        )
        .bind(new_owner_user_id)
        .bind(project_id)
        .bind(&old_owner)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        anyhow::ensure!(changed, "project ownership changed concurrently");
        sqlx::query("DELETE FROM project_members WHERE project_id = ? AND user_id = ?")
            .bind(project_id)
            .bind(new_owner_user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT OR IGNORE INTO project_members (project_id, user_id, invited_by) VALUES (?, ?, ?)",
        )
        .bind(project_id)
        .bind(&old_owner)
        .bind(actor_user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "DELETE FROM session_shares WHERE user_id = ? AND session_id IN (SELECT id FROM sessions WHERE COALESCE(project_id, id) = ?)",
        )
        .bind(new_owner_user_id)
        .bind(project_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO session_shares (session_id, user_id, invited_by, role)
            SELECT id, ?, ?, 'user'
            FROM sessions
            WHERE COALESCE(project_id, id) = ?
            "#,
        )
        .bind(&old_owner)
        .bind(actor_user_id)
        .bind(project_id)
        .execute(&mut *tx)
        .await?;
        Self::insert_audit_tx(
            &mut tx,
            actor_user_id,
            "project.owner_transferred",
            "project",
            project_id,
            Some(serde_json::json!({ "from": old_owner, "to": new_owner_user_id })),
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn leave_project(&self, user_id: &str, project_id: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let project = sqlx::query_as::<_, (String, String)>(
            "SELECT owner_user_id, lifecycle_status FROM projects WHERE id = ?",
        )
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("project not found"))?;
        anyhow::ensure!(
            ProjectLifecycle::parse(&project.1) != ProjectLifecycle::Deleting,
            "project deletion is in progress"
        );
        anyhow::ensure!(
            project.0 != user_id,
            "the project owner must transfer ownership or delete the project"
        );
        let changed =
            sqlx::query("DELETE FROM project_members WHERE project_id = ? AND user_id = ?")
                .bind(project_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                > 0;
        if changed {
            sqlx::query(
                "DELETE FROM session_shares WHERE user_id = ? AND session_id IN (SELECT id FROM sessions WHERE COALESCE(project_id, id) = ?)",
            )
            .bind(user_id)
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
            Self::insert_audit_tx(
                &mut tx,
                user_id,
                "project.member_left",
                "project_member",
                user_id,
                Some(serde_json::json!({ "project_id": project_id })),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(changed)
    }

    pub async fn set_project_lifecycle(
        &self,
        actor_user_id: &str,
        project_id: &str,
        lifecycle: ProjectLifecycle,
    ) -> Result<bool> {
        anyhow::ensure!(
            lifecycle != ProjectLifecycle::Deleting,
            "deleting is an internal project lifecycle"
        );
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query(
            "UPDATE projects SET lifecycle_status = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ? AND lifecycle_status != ? AND lifecycle_status != 'deleting'",
        )
        .bind(lifecycle.as_str())
        .bind(project_id)
        .bind(lifecycle.as_str())
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        if changed {
            Self::insert_audit_tx(
                &mut tx,
                actor_user_id,
                "project.lifecycle_changed",
                "project",
                project_id,
                Some(serde_json::json!({ "status": lifecycle.as_str() })),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(changed)
    }

    /// Irreversibly move an owned project into its durable cleanup state.
    /// Returns `false` when the project was already deleting.
    pub async fn begin_project_deletion(
        &self,
        actor_user_id: &str,
        project_id: &str,
        confirmation: &str,
    ) -> Result<bool> {
        anyhow::ensure!(
            confirmation == project_id,
            "project deletion confirmation does not match"
        );
        let mut tx = self.pool.begin().await?;
        let project = sqlx::query_as::<_, (String, String)>(
            "SELECT owner_user_id, lifecycle_status FROM projects WHERE id = ?",
        )
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("project not found"))?;
        anyhow::ensure!(
            project.0 == actor_user_id,
            "only the project owner can delete the project"
        );
        let lifecycle = ProjectLifecycle::parse(&project.1);
        if lifecycle == ProjectLifecycle::Deleting {
            tx.commit().await?;
            return Ok(false);
        }
        anyhow::ensure!(
            matches!(
                lifecycle,
                ProjectLifecycle::Active | ProjectLifecycle::Suspended
            ),
            "project cannot be deleted from its current lifecycle"
        );
        let changed = sqlx::query(
            "UPDATE projects SET lifecycle_status = 'deleting', updated_at = CURRENT_TIMESTAMP WHERE id = ? AND owner_user_id = ? AND lifecycle_status IN ('active', 'suspended')",
        )
        .bind(project_id)
        .bind(actor_user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        anyhow::ensure!(changed, "project deletion state changed concurrently");
        tx.commit().await?;
        Ok(true)
    }

    pub async fn get_project_deletion_manifest(
        &self,
        project_id: &str,
    ) -> Result<Option<ProjectDeletionManifest>> {
        let project = sqlx::query_as::<_, (String, String)>(
            "SELECT owner_user_id, lifecycle_status FROM projects WHERE id = ?",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some((owner_user_id, lifecycle)) = project else {
            return Ok(None);
        };
        anyhow::ensure!(
            ProjectLifecycle::parse(&lifecycle) == ProjectLifecycle::Deleting,
            "project deletion has not started"
        );
        let session_ids = sqlx::query_scalar::<_, String>(
            "SELECT id FROM sessions WHERE COALESCE(project_id, id) = ? ORDER BY id",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        let affected_user_ids = sqlx::query_scalar::<_, String>(
            r#"
            SELECT user_id FROM (
                SELECT owner_user_id AS user_id FROM projects WHERE id = ?
                UNION
                SELECT user_id FROM project_members WHERE project_id = ?
                UNION
                SELECT user_id FROM sessions WHERE COALESCE(project_id, id) = ?
                UNION
                SELECT session_shares.user_id
                FROM session_shares
                JOIN sessions ON sessions.id = session_shares.session_id
                WHERE COALESCE(sessions.project_id, sessions.id) = ?
            ) ORDER BY user_id
            "#,
        )
        .bind(project_id)
        .bind(project_id)
        .bind(project_id)
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(Some(ProjectDeletionManifest {
            project_id: project_id.to_string(),
            owner_user_id,
            session_ids,
            affected_user_ids,
        }))
    }

    pub async fn list_deleting_project_ids(&self) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar::<_, String>(
            "SELECT id FROM projects WHERE lifecycle_status = 'deleting' ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_project_deletion_status(
        &self,
        project_id: &str,
    ) -> Result<Option<ProjectLifecycle>> {
        Ok(
            sqlx::query_scalar::<_, String>("SELECT lifecycle_status FROM projects WHERE id = ?")
                .bind(project_id)
                .fetch_optional(&self.pool)
                .await?
                .map(|raw| ProjectLifecycle::parse(&raw)),
        )
    }

    pub async fn get_project_id_for_session(&self, session_id: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar::<_, String>(
            "SELECT COALESCE(project_id, id) FROM sessions WHERE id = ?",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Final idempotent relational phase of permanent deletion. File-backed
    /// artifacts must be removed first while the session rows still form the
    /// durable cleanup manifest.
    pub async fn delete_project_records(&self, project_id: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let lifecycle =
            sqlx::query_scalar::<_, String>("SELECT lifecycle_status FROM projects WHERE id = ?")
                .bind(project_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some(lifecycle) = lifecycle else {
            tx.commit().await?;
            return Ok(false);
        };
        anyhow::ensure!(
            ProjectLifecycle::parse(&lifecycle) == ProjectLifecycle::Deleting,
            "project deletion has not started"
        );

        for statement in [
            "DELETE FROM messages WHERE session_id IN (SELECT id FROM sessions WHERE COALESCE(project_id, id) = ?)",
            "DELETE FROM pane_usage_stats WHERE session_id IN (SELECT id FROM sessions WHERE COALESCE(project_id, id) = ?)",
            "DELETE FROM session_shares WHERE session_id IN (SELECT id FROM sessions WHERE COALESCE(project_id, id) = ?)",
            "DELETE FROM invitation_codes WHERE project_id = ? OR session_id IN (SELECT id FROM sessions WHERE COALESCE(project_id, id) = ?)",
        ] {
            let mut query = sqlx::query(statement).bind(project_id);
            if statement.starts_with("DELETE FROM invitation_codes") {
                query = query.bind(project_id);
            }
            query.execute(&mut *tx).await?;
        }
        sqlx::query("DELETE FROM sessions WHERE COALESCE(project_id, id) = ?")
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM project_policy_overrides WHERE project_id = ?")
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM project_members WHERE project_id = ?")
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM admin_audit_events WHERE project_id = ?")
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
        let deleted =
            sqlx::query("DELETE FROM projects WHERE id = ? AND lifecycle_status = 'deleting'")
                .bind(project_id)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                > 0;
        tx.commit().await?;
        Ok(deleted)
    }

    pub async fn list_admin_projects(
        &self,
        search: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AdminProjectSummary>> {
        let pattern = format!("%{}%", search.unwrap_or_default().trim());
        let rows = sqlx::query(
            r#"
            WITH ranked_sessions AS (
                SELECT COALESCE(project_id, id) AS canonical_project_id,
                       working_dir, hostname, git_remote,
                       ROW_NUMBER() OVER (
                           PARTITION BY COALESCE(project_id, id)
                           ORDER BY CASE WHEN status = 'active' THEN 0 ELSE 1 END,
                                    COALESCE(updated_at, created_at) DESC,
                                    id DESC
                       ) AS session_rank
                FROM sessions
            )
            SELECT p.id, p.owner_user_id, u.email AS owner_email, p.lifecycle_status,
                   (SELECT COUNT(*) FROM project_members pm WHERE pm.project_id = p.id) AS member_count,
                   (SELECT COUNT(*) FROM sessions s WHERE COALESCE(s.project_id, s.id) = p.id AND s.status = 'active') AS active_session_count,
                   (SELECT MAX(s.updated_at) FROM sessions s WHERE COALESCE(s.project_id, s.id) = p.id) AS last_activity,
                   p.created_at, rs.working_dir, rs.hostname, rs.git_remote
            FROM projects p
            JOIN users u ON u.id = p.owner_user_id
            LEFT JOIN ranked_sessions rs
              ON rs.canonical_project_id = p.id AND rs.session_rank = 1
            WHERE (? = '%%'
                   OR lower(p.id) LIKE lower(?)
                   OR lower(u.email) LIKE lower(?)
                   OR lower(COALESCE(rs.working_dir, '')) LIKE lower(?)
                   OR lower(COALESCE(rs.hostname, '')) LIKE lower(?))
            ORDER BY COALESCE(last_activity, p.created_at) DESC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .bind(limit.clamp(1, 200))
        .bind(offset.max(0))
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| {
                let working_dir = row.get::<Option<String>, _>("working_dir");
                let git_remote = row.get::<Option<String>, _>("git_remote");
                AdminProjectSummary {
                    id: row.get("id"),
                    project_name: admin_project_name(working_dir.as_deref(), git_remote.as_deref()),
                    hostname: row.get("hostname"),
                    owner_user_id: row.get("owner_user_id"),
                    owner_email: row.get("owner_email"),
                    lifecycle_status: row.get("lifecycle_status"),
                    member_count: row.get("member_count"),
                    active_session_count: row.get("active_session_count"),
                    last_activity: row.get("last_activity"),
                    created_at: row.get("created_at"),
                }
            })
            .collect())
    }

    pub async fn list_project_ids(&self) -> Result<Vec<String>> {
        Ok(
            sqlx::query_scalar::<_, String>("SELECT id FROM projects ORDER BY id")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn get_cluster_default_policy(&self) -> Result<shared::EffectiveProjectPolicy> {
        let (team_available, profiles_json, version) =
            sqlx::query_as::<_, (i64, String, i64)>(
                "SELECT team_available, allowed_launch_profiles, version FROM cluster_settings WHERE id = 1",
            )
            .fetch_one(&self.pool)
            .await?;
        Ok(shared::EffectiveProjectPolicy {
            team_available: team_available != 0,
            allowed_launch_profiles: serde_json::from_str(&profiles_json).unwrap_or_else(|_| {
                shared::EffectiveProjectPolicy::default().allowed_launch_profiles
            }),
            version,
            project_suspended: false,
        })
    }

    pub async fn set_cluster_default_policy(
        &self,
        actor_user_id: &str,
        team_available: bool,
        allowed_launch_profiles: Vec<String>,
    ) -> Result<shared::EffectiveProjectPolicy> {
        anyhow::ensure!(
            allowed_launch_profiles
                .iter()
                .all(|profile| !shared::is_retired_launch_profile_key(profile)),
            "policy contains a retired launch profile"
        );
        let supported = shared::supported_launch_profiles()
            .into_iter()
            .map(|profile| profile.key)
            .collect::<std::collections::HashSet<_>>();
        anyhow::ensure!(
            allowed_launch_profiles
                .iter()
                .all(|profile| supported.contains(profile)),
            "policy contains an unsupported launch profile"
        );
        let profiles_json = serde_json::to_string(&allowed_launch_profiles)?;
        let mut tx = self.pool.begin().await?;
        let next_version = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT MAX(version) + 1 FROM (
                SELECT version FROM cluster_settings WHERE id = 1
                UNION ALL
                SELECT version FROM project_policy_overrides
            )
            "#,
        )
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE cluster_settings SET team_available = ?, allowed_launch_profiles = ?, version = ?, updated_at = CURRENT_TIMESTAMP WHERE id = 1",
        )
        .bind(i64::from(team_available))
        .bind(profiles_json)
        .bind(next_version)
        .execute(&mut *tx)
        .await?;
        Self::insert_audit_tx(
            &mut tx,
            actor_user_id,
            "cluster.policy_changed",
            "cluster",
            "default",
            Some(serde_json::json!({
                "team_available": team_available,
                "allowed_launch_profiles": allowed_launch_profiles,
                "version": next_version,
            })),
        )
        .await?;
        tx.commit().await?;
        self.get_cluster_default_policy().await
    }

    pub async fn get_effective_project_policy(
        &self,
        project_id: &str,
    ) -> Result<shared::EffectiveProjectPolicy> {
        use sqlx::Row;
        let row = sqlx::query(
            r#"
            SELECT cs.team_available AS default_team, cs.allowed_launch_profiles AS default_profiles,
                   cs.version AS default_version, p.lifecycle_status,
                   ppo.team_available AS override_team,
                   ppo.allowed_launch_profiles AS override_profiles,
                   ppo.version AS override_version
            FROM projects p
            CROSS JOIN cluster_settings cs
            LEFT JOIN project_policy_overrides ppo ON ppo.project_id = p.id
            WHERE p.id = ? AND cs.id = 1
            "#,
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("project not found"))?;
        let default_profiles: String = row.get("default_profiles");
        let override_profiles: Option<String> = row.get("override_profiles");
        let profiles_json = override_profiles.as_ref().unwrap_or(&default_profiles);
        let allowed_launch_profiles = serde_json::from_str::<Vec<String>>(profiles_json)
            .unwrap_or_else(|_| shared::EffectiveProjectPolicy::default().allowed_launch_profiles);
        let default_team = row.get::<i64, _>("default_team") != 0;
        let override_team = row
            .try_get::<Option<i64>, _>("override_team")
            .unwrap_or(None);
        let default_version: i64 = row.get("default_version");
        let override_version = row
            .try_get::<Option<i64>, _>("override_version")
            .unwrap_or(None);
        Ok(shared::EffectiveProjectPolicy {
            team_available: override_team
                .map(|value| value != 0)
                .unwrap_or(default_team),
            allowed_launch_profiles,
            version: override_version
                .unwrap_or(default_version)
                .max(default_version),
            project_suspended: row.get::<String, _>("lifecycle_status") == "suspended",
        })
    }

    pub async fn get_project_policy_override(
        &self,
        project_id: &str,
    ) -> Result<Option<ProjectPolicyOverride>> {
        use sqlx::Row;
        let row = sqlx::query(
            r#"
            SELECT project_id, team_available, allowed_launch_profiles,
                   version, legacy_imported, legacy_conflict
            FROM project_policy_overrides WHERE project_id = ?
            "#,
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| ProjectPolicyOverride {
            project_id: row.get("project_id"),
            team_available: row
                .get::<Option<i64>, _>("team_available")
                .map(|value| value != 0),
            allowed_launch_profiles: row
                .get::<Option<String>, _>("allowed_launch_profiles")
                .and_then(|raw| serde_json::from_str(&raw).ok()),
            version: row.get("version"),
            legacy_imported: row.get::<i64, _>("legacy_imported") != 0,
            legacy_conflict: row.get("legacy_conflict"),
        }))
    }

    pub async fn set_project_policy_override(
        &self,
        actor_user_id: &str,
        project_id: &str,
        team_available: Option<bool>,
        allowed_launch_profiles: Option<Vec<String>>,
    ) -> Result<shared::EffectiveProjectPolicy> {
        let lifecycle = self
            .get_project(project_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("project not found"))?
            .lifecycle();
        anyhow::ensure!(
            lifecycle != ProjectLifecycle::Deleting,
            "project deletion is in progress"
        );
        if let Some(profiles) = &allowed_launch_profiles {
            anyhow::ensure!(
                profiles
                    .iter()
                    .all(|profile| !shared::is_retired_launch_profile_key(profile)),
                "policy contains a retired launch profile"
            );
            let supported = shared::supported_launch_profiles()
                .into_iter()
                .map(|profile| profile.key)
                .collect::<std::collections::HashSet<_>>();
            anyhow::ensure!(
                profiles.iter().all(|profile| supported.contains(profile)),
                "policy contains an unsupported launch profile"
            );
        }
        let profiles_json = allowed_launch_profiles
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let mut tx = self.pool.begin().await?;
        let next_version = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT MAX(version) + 1 FROM (
                SELECT version FROM cluster_settings WHERE id = 1
                UNION ALL
                SELECT version FROM project_policy_overrides WHERE project_id = ?
            )
            "#,
        )
        .bind(project_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO project_policy_overrides
                (project_id, team_available, allowed_launch_profiles, version, updated_at)
            VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(project_id) DO UPDATE SET
                team_available = excluded.team_available,
                allowed_launch_profiles = excluded.allowed_launch_profiles,
                version = excluded.version,
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(project_id)
        .bind(team_available.map(i64::from))
        .bind(&profiles_json)
        .bind(next_version)
        .execute(&mut *tx)
        .await?;
        Self::insert_audit_tx(
            &mut tx,
            actor_user_id,
            "project.policy_changed",
            "project",
            project_id,
            Some(serde_json::json!({
                "team_available": team_available,
                "allowed_launch_profiles": allowed_launch_profiles,
                "version": next_version,
            })),
        )
        .await?;
        tx.commit().await?;
        self.get_effective_project_policy(project_id).await
    }

    pub async fn import_legacy_project_policy(
        &self,
        project_id: &str,
        team_available: bool,
        disallowed_tab_types: &[String],
    ) -> Result<shared::EffectiveProjectPolicy> {
        let allowed = shared::supported_launch_profiles()
            .into_iter()
            .filter(|profile| {
                let tab_key = shared::tab_type_key(profile.kind, profile.provider);
                !disallowed_tab_types
                    .iter()
                    .any(|blocked| blocked.trim().eq_ignore_ascii_case(&tab_key))
            })
            .map(|profile| profile.key)
            .collect::<Vec<_>>();
        let snapshot = serde_json::json!({
            "team_available": team_available,
            "allowed_launch_profiles": allowed,
        })
        .to_string();
        let mut tx = self.pool.begin().await?;
        let existing: Option<(i64, Option<i64>, Option<String>)> = sqlx::query_as(
            "SELECT legacy_imported, team_available, allowed_launch_profiles FROM project_policy_overrides WHERE project_id = ?",
        )
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((imported, old_team, old_profiles)) = existing {
            let old_snapshot = serde_json::json!({
                "team_available": old_team.map(|value| value != 0),
                "allowed_launch_profiles": old_profiles.and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok()),
            })
            .to_string();
            // The server is authoritative after either the first import or an
            // explicit administrator override. Never let a reconnecting CLI
            // overwrite that state; retain a differing legacy snapshot only
            // as migration/conflict metadata for administrators.
            if old_snapshot != snapshot || imported == 0 {
                sqlx::query(
                    "UPDATE project_policy_overrides SET legacy_conflict = ?, updated_at = CURRENT_TIMESTAMP WHERE project_id = ?",
                )
                .bind(snapshot)
                .bind(project_id)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
            return self.get_effective_project_policy(project_id).await;
        }
        let import_pending =
            sqlx::query_scalar::<_, i64>("SELECT legacy_policy_pending FROM projects WHERE id = ?")
                .bind(project_id)
                .fetch_optional(&mut *tx)
                .await?
                .unwrap_or(0)
                != 0;
        if !import_pending {
            tx.commit().await?;
            return self.get_effective_project_policy(project_id).await;
        }
        let allowed_json = serde_json::to_string(&allowed)?;
        sqlx::query(
            r#"
            INSERT INTO project_policy_overrides
                (project_id, team_available, allowed_launch_profiles, version, legacy_imported, updated_at)
            VALUES (?, ?, ?, 2, 1, CURRENT_TIMESTAMP)
            ON CONFLICT(project_id) DO UPDATE SET
                team_available = excluded.team_available,
                allowed_launch_profiles = excluded.allowed_launch_profiles,
                version = MAX(project_policy_overrides.version + 1, excluded.version),
                legacy_imported = 1,
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(project_id)
        .bind(i64::from(team_available))
        .bind(allowed_json)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE projects SET legacy_policy_pending = 0, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(project_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.get_effective_project_policy(project_id).await
    }

    pub async fn update_session_status(&self, id: &str, status: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET status = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(status)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn record_session_user_input(&self, id: &str, created_at: &str) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE sessions
            SET last_user_input_at = CASE
                    WHEN last_user_input_at IS NULL OR last_user_input_at < ? THEN ?
                    ELSE last_user_input_at
                END,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?
            "#,
        )
        .bind(created_at)
        .bind(created_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_session_last_user_input_at(&self, id: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar::<_, Option<String>>(
            "SELECT last_user_input_at FROM sessions WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .flatten())
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
            "SELECT id, user_id, cli_client_id, working_dir, hostname, status, created_at, updated_at, COALESCE(is_paused, 0) as is_paused, COALESCE(project_id, id) as project_id, git_remote, git_remote_url FROM sessions WHERE id = ?",
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
            "SELECT id, user_id, cli_client_id, working_dir, hostname, status, created_at, updated_at, COALESCE(is_paused, 0) as is_paused, COALESCE(project_id, id) as project_id, git_remote, git_remote_url FROM sessions ORDER BY created_at DESC LIMIT 50",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(sessions)
    }

    pub async fn get_sessions_for_user(&self, user_id: &str) -> Result<Vec<Session>> {
        let sessions = sqlx::query_as::<_, Session>(
            r#"
            SELECT s.id, p.owner_user_id AS user_id, s.cli_client_id, s.working_dir,
                   s.hostname, s.status, s.created_at, s.updated_at,
                   COALESCE(s.is_paused, 0) AS is_paused,
                   COALESCE(s.project_id, s.id) AS project_id,
                   s.git_remote, s.git_remote_url
            FROM sessions s
            JOIN projects p ON p.id = COALESCE(s.project_id, s.id)
            WHERE p.owner_user_id = ?
            ORDER BY s.created_at DESC
            LIMIT 50
            "#,
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
        let project_id = match &code.project_id {
            Some(project_id) => Some(project_id.clone()),
            None => {
                sqlx::query_scalar::<_, String>(
                    "SELECT COALESCE(project_id, id) FROM sessions WHERE id = ?",
                )
                .bind(&code.session_id)
                .fetch_optional(&self.pool)
                .await?
            }
        };
        sqlx::query(
            "INSERT INTO invitation_codes (code, session_id, project_id, created_by, expires_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&code.code)
        .bind(&code.session_id)
        .bind(project_id)
        .bind(&code.created_by)
        .bind(&code.expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_invitation_code(&self, code: &str) -> Result<Option<InvitationCode>> {
        let invitation = sqlx::query_as::<_, InvitationCode>(
            "SELECT code, session_id, project_id, created_by, expires_at, redeemed_by, redeemed_at, created_at FROM invitation_codes WHERE code = ?",
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

    pub async fn redeem_project_invitation(&self, code: &str, user_id: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let invitation = sqlx::query_as::<_, (String, Option<String>, String)>(
            "SELECT session_id, project_id, created_by FROM invitation_codes WHERE code = ?",
        )
        .bind(code)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((session_id, stored_project_id, invited_by)) = invitation else {
            tx.rollback().await?;
            return Ok(false);
        };
        let project_id = match stored_project_id {
            Some(project_id) => project_id,
            None => {
                sqlx::query_scalar::<_, String>(
                    "SELECT COALESCE(project_id, id) FROM sessions WHERE id = ?",
                )
                .bind(&session_id)
                .fetch_one(&mut *tx)
                .await?
            }
        };
        let eligible = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*) FROM projects p, users u
            WHERE p.id = ? AND p.lifecycle_status = 'active'
              AND u.id = ? AND u.account_status = 'active'
            "#,
        )
        .bind(&project_id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?
            > 0;
        if !eligible {
            tx.rollback().await?;
            return Ok(false);
        }
        let consumed = sqlx::query(
            r#"
            UPDATE invitation_codes
            SET redeemed_by = ?, redeemed_at = CURRENT_TIMESTAMP
            WHERE code = ? AND redeemed_by IS NULL
              AND datetime(expires_at) > CURRENT_TIMESTAMP
            "#,
        )
        .bind(user_id)
        .bind(code)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        if !consumed {
            tx.rollback().await?;
            return Ok(false);
        }
        let added = sqlx::query(
            "INSERT OR IGNORE INTO project_members (project_id, user_id, invited_by) VALUES (?, ?, ?)",
        )
        .bind(&project_id)
        .bind(user_id)
        .bind(&invited_by)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        if added {
            Self::insert_audit_tx(
                &mut tx,
                &invited_by,
                "project.member_added",
                "project",
                &project_id,
                Some(serde_json::json!({ "user_id": user_id, "via": "invitation" })),
            )
            .await?;
        }
        sqlx::query("DELETE FROM invitation_codes WHERE code = ?")
            .bind(code)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
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
        anyhow::ensure!(
            role.trim().eq_ignore_ascii_case("user"),
            "invalid project role: only 'user' is assignable"
        );
        let project_id = sqlx::query_scalar::<_, String>(
            "SELECT COALESCE(project_id, id) FROM sessions WHERE id = ?",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("project session not found"))?;
        self.add_project_member(invited_by, &project_id, user_id)
            .await?;
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
        // Compatibility shape for the web session list, now sourced from
        // canonical project membership rather than per-session role rows.
        let rows = sqlx::query(
            r#"
            SELECT s.id, p.owner_user_id AS user_id, s.cli_client_id, s.working_dir, s.hostname, s.status, s.created_at, s.updated_at, COALESCE(s.is_paused, 0) as is_paused, COALESCE(s.project_id, s.id) as project_id, s.git_remote, s.git_remote_url, u.email, 'user' AS role
            FROM sessions s
            INNER JOIN projects p ON p.id = COALESCE(s.project_id, s.id)
            INNER JOIN project_members pm ON pm.project_id = p.id
            INNER JOIN users u ON p.owner_user_id = u.id
            WHERE pm.user_id = ?
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
                git_remote_url: row.get("git_remote_url"),
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
        let project_id = sqlx::query_scalar::<_, String>(
            "SELECT COALESCE(project_id, id) FROM sessions WHERE id = ?",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;
        match project_id {
            Some(project_id) => self.get_project_role_for_user(&project_id, user_id).await,
            None => Ok(None),
        }
    }

    pub async fn get_session_share_role(
        &self,
        session_id: &str,
        user_id: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .get_session_role_for_user(session_id, user_id)
            .await?
            .filter(|role| role == "user"))
    }

    pub async fn update_session_share_role(
        &self,
        session_id: &str,
        user_id: &str,
        role: &str,
    ) -> Result<bool> {
        anyhow::ensure!(
            role.trim().eq_ignore_ascii_case("user"),
            "invalid project role: only 'user' is assignable"
        );
        Ok(self
            .get_session_share_role(session_id, user_id)
            .await?
            .is_some())
    }

    pub async fn check_session_access(&self, session_id: &str, user_id: &str) -> Result<bool> {
        let result = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM sessions s
            JOIN projects p ON p.id = COALESCE(s.project_id, s.id)
            WHERE s.id = ?
              AND p.lifecycle_status = 'active'
              AND (
                  p.owner_user_id = ? OR EXISTS (
                      SELECT 1 FROM project_members pm
                      WHERE pm.project_id = p.id AND pm.user_id = ?
                  )
              )
            "#,
        )
        .bind(session_id)
        .bind(user_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(result > 0)
    }

    pub async fn delete_session_share(&self, session_id: &str, user_id: &str) -> Result<bool> {
        let project_id = sqlx::query_scalar::<_, String>(
            "SELECT COALESCE(project_id, id) FROM sessions WHERE id = ?",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(project_id) = project_id else {
            return Ok(false);
        };
        let removed = self
            .remove_project_member(
                &self
                    .get_project(&project_id)
                    .await?
                    .map(|project| project.owner_user_id)
                    .unwrap_or_default(),
                &project_id,
                user_id,
            )
            .await?;
        sqlx::query(
            "DELETE FROM session_shares WHERE user_id = ? AND session_id IN (SELECT id FROM sessions WHERE COALESCE(project_id, id) = ?)",
        )
        .bind(user_id)
        .bind(&project_id)
        .execute(&self.pool)
        .await?;
        Ok(removed)
    }

    pub async fn get_session_owner(&self, session_id: &str) -> Result<Option<String>> {
        let owner = sqlx::query_scalar::<_, String>(
            r#"
            SELECT p.owner_user_id
            FROM sessions s JOIN projects p ON p.id = COALESCE(s.project_id, s.id)
            WHERE s.id = ?
            "#,
        )
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
            INNER JOIN projects p ON p.id = COALESCE(s.project_id, s.id)
            INNER JOIN users u ON p.owner_user_id = u.id
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
            SELECT u.id, u.email, pm.created_at, 'user' as role
            FROM sessions s
            INNER JOIN projects p ON p.id = COALESCE(s.project_id, s.id)
            INNER JOIN project_members pm ON pm.project_id = p.id
            INNER JOIN users u ON pm.user_id = u.id
            WHERE s.id = ?
            ORDER BY pm.created_at DESC
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
mod mobile_notification_tests {
    use super::*;

    async fn database() -> Database {
        let dir = std::env::temp_dir().join(format!(
            "apas-mobile-notifications-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::new(&dir.join("apas.db").to_string_lossy())
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        db.create_user(&User {
            id: "mobile-user".to_string(),
            email: "mobile-notifications@example.test".to_string(),
            password_hash: "hash".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .unwrap();
        db.create_mobile_device_session(
            "device-session",
            "mobile-user",
            "installation",
            "ios",
            Some("Phone"),
            "0.1.0",
            "refresh-hash",
            &(chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339(),
        )
        .await
        .unwrap();
        db
    }

    #[tokio::test]
    async fn registration_rotation_preferences_and_revocation_cleanup_are_atomic() {
        let db = database().await;
        db.register_mobile_push_token(
            "push-1",
            "mobile-user",
            "device-session",
            "installation",
            "ios",
            "ExponentPushToken[first]",
            "token-hash-1",
        )
        .await
        .unwrap();
        db.register_mobile_push_token(
            "push-2",
            "mobile-user",
            "device-session",
            "installation",
            "ios",
            "ExponentPushToken[second]",
            "token-hash-2",
        )
        .await
        .unwrap();
        db.create_mobile_device_session(
            "device-session-2",
            "mobile-user",
            "installation-2",
            "android",
            Some("Tablet"),
            "0.1.0",
            "refresh-hash-2",
            &(chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339(),
        )
        .await
        .unwrap();
        db.register_mobile_push_token(
            "push-3",
            "mobile-user",
            "device-session-2",
            "installation-2",
            "android",
            "ExponentPushToken[third]",
            "token-hash-3",
        )
        .await
        .unwrap();
        let active = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM mobile_push_tokens WHERE retired_at IS NULL",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(active, 2);

        let preferences = shared::MobileNotificationPreferences {
            decisions: false,
            failures: true,
            pull_requests: false,
            completions: true,
        };
        assert!(db
            .update_mobile_notification_preferences("mobile-user", "device-session", &preferences,)
            .await
            .unwrap());
        assert!(
            db.update_mobile_notification_preferences(
                "mobile-user",
                "device-session-2",
                &preferences,
            )
            .await
            .unwrap()
        );
        assert_eq!(
            db.mobile_notification_preferences("mobile-user", "device-session")
                .await
                .unwrap()
                .unwrap()
                .completions,
            true
        );

        db.authorize_project_registration("mobile-project", "mobile-user")
            .await
            .unwrap();
        db.create_session(&Session {
            id: "mobile-session".to_string(),
            user_id: "mobile-user".to_string(),
            cli_client_id: None,
            working_dir: None,
            hostname: None,
            status: "active".to_string(),
            created_at: None,
            updated_at: None,
            is_paused: false,
            project_id: Some("mobile-project".to_string()),
            git_remote: None,
            git_remote_url: None,
        })
        .await
        .unwrap();
        assert!(db
            .enqueue_mobile_notification_event(
                "event-1",
                "mobile-user",
                "mobile-project",
                Some("mobile-session"),
                Some(1),
                "completion",
                "opaque-route",
                "dedupe-1",
            )
            .await
            .unwrap());
        assert!(!db
            .enqueue_mobile_notification_event(
                "event-2",
                "mobile-user",
                "mobile-project",
                Some("mobile-session"),
                Some(1),
                "completion",
                "opaque-route",
                "dedupe-1",
            )
            .await
            .unwrap());
        let deliveries = db.claim_mobile_notification_deliveries(100).await.unwrap();
        assert_eq!(deliveries.len(), 2);
        assert_eq!(db.recover_mobile_notification_outbox().await.unwrap(), 2);

        let deliveries = db.claim_mobile_notification_deliveries(100).await.unwrap();
        assert_eq!(deliveries.len(), 2);
        db.mark_mobile_delivery_ticketed(deliveries[0].id, "ticket-1")
            .await
            .unwrap();
        sqlx::query(
            "UPDATE mobile_notification_deliveries SET updated_at = datetime('now', '-20 seconds') WHERE id = ?",
        )
        .bind(deliveries[0].id)
        .execute(&db.pool)
        .await
        .unwrap();
        let receipts = db.pending_mobile_notification_receipts(100).await.unwrap();
        assert_eq!(receipts.len(), 1);
        db.mark_mobile_delivery_delivered(receipts[0].id)
            .await
            .unwrap();

        db.retry_mobile_delivery(deliveries[1].id, "transient", 0)
            .await
            .unwrap();
        let retry = db.claim_mobile_notification_deliveries(100).await.unwrap();
        assert_eq!(retry.len(), 1);
        assert!(retry[0].attempt_count >= 2);
        db.retire_mobile_push_token(&retry[0].push_token_id, retry[0].id, "DeviceNotRegistered")
            .await
            .unwrap();
        let invalid_status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM mobile_notification_deliveries WHERE id = ?",
        )
        .bind(retry[0].id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(invalid_status, "permanent_failure");

        sqlx::query("DELETE FROM sessions WHERE id = 'mobile-session'")
            .execute(&db.pool)
            .await
            .unwrap();
        let session_events =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM mobile_notification_events")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        let session_deliveries =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM mobile_notification_deliveries")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!((session_events, session_deliveries), (0, 0));

        sqlx::query("DELETE FROM projects WHERE id = 'mobile-project'")
            .execute(&db.pool)
            .await
            .unwrap();
        let events =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM mobile_notification_events")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        let deliveries =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM mobile_notification_deliveries")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!((events, deliveries), (0, 0));

        db.revoke_mobile_device_session("mobile-user", "device-session", false, "test_revocation")
            .await
            .unwrap();
        // Revoking one device session deletes exactly that session's push
        // tokens, via the mobile_device_session_push_cleanup trigger. Scope the
        // check to the revoked session: a bare global count cannot tell
        // "this device went dark" apart from "every device this user owns went
        // dark", and the latter would be a bug, not a pass.
        let revoked_session_tokens = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM mobile_push_tokens WHERE device_session_id = 'device-session'",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(revoked_session_tokens, 0);
        // Deliberately not a global "active tokens" count. One token was already
        // retired above as DeviceNotRegistered, and which one that is depends on
        // the order claim_mobile_notification_deliveries hands back the two
        // deliveries — so a global count is legitimately 0 or 1 and asserting
        // either makes the test order-dependent. Scoping to the sibling session
        // states the property that actually matters and holds every run.
        let sibling_tokens = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM mobile_push_tokens WHERE device_session_id = 'device-session-2'",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(
            sibling_tokens, 1,
            "revoking one device session must not touch another's push tokens"
        );
        db.revoke_mobile_device_session("mobile-user", "device-session-2", false, "test_logout")
            .await
            .unwrap();
        let remaining = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM mobile_push_tokens")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[tokio::test]
    async fn queued_notifications_are_erased_after_membership_loss() {
        let db = database().await;
        db.create_user(&User {
            id: "project-owner".to_string(),
            email: "notification-owner@example.test".to_string(),
            password_hash: "hash".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .unwrap();
        db.authorize_project_registration("shared-project", "project-owner")
            .await
            .unwrap();
        db.add_project_member("project-owner", "shared-project", "mobile-user")
            .await
            .unwrap();
        db.create_session(&Session {
            id: "shared-session".to_string(),
            user_id: "project-owner".to_string(),
            cli_client_id: None,
            working_dir: None,
            hostname: None,
            status: "active".to_string(),
            created_at: None,
            updated_at: None,
            is_paused: false,
            project_id: Some("shared-project".to_string()),
            git_remote: None,
            git_remote_url: None,
        })
        .await
        .unwrap();
        db.register_mobile_push_token(
            "member-push",
            "mobile-user",
            "device-session",
            "installation",
            "ios",
            "ExponentPushToken[member]",
            "member-token-hash",
        )
        .await
        .unwrap();
        assert!(db
            .enqueue_mobile_notification_event(
                "membership-event",
                "mobile-user",
                "shared-project",
                Some("shared-session"),
                Some(1),
                "decision",
                "opaque-membership-route",
                "membership-dedupe",
            )
            .await
            .unwrap());

        db.remove_project_member("project-owner", "shared-project", "mobile-user")
            .await
            .unwrap();
        assert!(db
            .claim_mobile_notification_deliveries(100)
            .await
            .unwrap()
            .is_empty());
        let events =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM mobile_notification_events")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        let deliveries =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM mobile_notification_deliveries")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!((events, deliveries), (0, 0));
    }

    #[tokio::test]
    async fn notification_outbox_recovers_and_drains_a_bounded_load_after_reopen() {
        const EVENTS: usize = 1_000;
        let dir =
            std::env::temp_dir().join(format!("apas-mobile-outbox-load-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("apas.db").to_string_lossy().into_owned();
        let db = Database::new(&path).await.unwrap();
        db.run_migrations().await.unwrap();
        db.create_user(&User {
            id: "outbox-user".to_string(),
            email: "mobile-outbox@example.test".to_string(),
            password_hash: "hash".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .unwrap();
        db.create_mobile_device_session(
            "outbox-device",
            "outbox-user",
            "outbox-installation",
            "android",
            None,
            "load-test",
            "refresh-hash",
            &(chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339(),
        )
        .await
        .unwrap();
        db.register_mobile_push_token(
            "outbox-push",
            "outbox-user",
            "outbox-device",
            "outbox-installation",
            "android",
            "ExponentPushToken[outbox-load]",
            "outbox-token-hash",
        )
        .await
        .unwrap();
        db.authorize_project_registration("outbox-project", "outbox-user")
            .await
            .unwrap();

        let started = std::time::Instant::now();
        for index in 0..EVENTS {
            assert!(db
                .enqueue_mobile_notification_event(
                    &format!("load-event-{index}"),
                    "outbox-user",
                    "outbox-project",
                    None,
                    None,
                    "failure",
                    &format!("route-{index}"),
                    &format!("dedupe-{index}"),
                )
                .await
                .unwrap());
        }
        let stranded = db.claim_mobile_notification_deliveries(100).await.unwrap();
        assert_eq!(stranded.len(), 100);
        drop(db);

        let reopened = Database::new(&path).await.unwrap();
        reopened.run_migrations().await.unwrap();
        assert_eq!(
            reopened.recover_mobile_notification_outbox().await.unwrap(),
            100
        );
        let mut drained = 0;
        loop {
            let deliveries = reopened
                .claim_mobile_notification_deliveries(100)
                .await
                .unwrap();
            if deliveries.is_empty() {
                break;
            }
            assert!(deliveries.len() <= 100);
            drained += deliveries.len();
            for delivery in deliveries {
                reopened
                    .mark_mobile_delivery_delivered(delivery.id)
                    .await
                    .unwrap();
            }
        }
        assert_eq!(drained, EVENTS);
        eprintln!(
            "recovered and drained {EVENTS} notification deliveries in {:?}",
            started.elapsed()
        );
    }
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
        db.create_user(&User {
            id: "u1".to_string(),
            email: "usage@example.test".to_string(),
            password_hash: "hash".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .expect("usage test user");
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
            git_remote_url: None,
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

#[cfg(test)]
mod cluster_administration_tests {
    use super::*;

    async fn database(name: &str) -> Database {
        let dir = std::env::temp_dir().join(format!("apas-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp db dir");
        Database::new(&dir.join("apas.db").to_string_lossy())
            .await
            .expect("database")
    }

    fn user(id: &str, email: &str, role: &str) -> User {
        User {
            id: id.to_string(),
            email: email.to_string(),
            password_hash: "hash".to_string(),
            created_at: None,
            cluster_role: role.to_string(),
            account_status: "active".to_string(),
        }
    }

    #[tokio::test]
    async fn legacy_rows_migrate_to_canonical_projects_and_user_roles() {
        let db = database("canonical-migration").await;
        sqlx::query(
            "CREATE TABLE users (id TEXT PRIMARY KEY, email TEXT UNIQUE NOT NULL, password_hash TEXT NOT NULL, created_at DATETIME DEFAULT CURRENT_TIMESTAMP)",
        )
        .execute(&db.pool)
        .await
        .unwrap();
        for (id, email) in [
            ("u1", "owner@test"),
            ("u2", "other@test"),
            ("u3", "shared@test"),
        ] {
            sqlx::query("INSERT INTO users (id, email, password_hash) VALUES (?, ?, 'hash')")
                .bind(id)
                .bind(email)
                .execute(&db.pool)
                .await
                .unwrap();
        }
        sqlx::query(
            r#"CREATE TABLE sessions (
                id TEXT PRIMARY KEY, user_id TEXT NOT NULL, cli_client_id TEXT,
                working_dir TEXT, hostname TEXT, status TEXT,
                created_at DATETIME, updated_at DATETIME, project_id TEXT
            )"#,
        )
        .execute(&db.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO sessions (id, user_id, project_id, created_at, updated_at) VALUES ('s1', 'u1', 'project-a', '2025-01-01', '2025-01-01'), ('s2', 'u2', 'project-a', '2025-02-01', '2025-02-01')")
            .execute(&db.pool)
            .await
            .unwrap();
        sqlx::query(
            r#"CREATE TABLE session_shares (
                id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL,
                user_id TEXT NOT NULL, invited_by TEXT NOT NULL,
                role TEXT NOT NULL DEFAULT 'user', created_at DATETIME,
                UNIQUE(session_id, user_id)
            )"#,
        )
        .execute(&db.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO session_shares (session_id, user_id, invited_by, role) VALUES ('s1', 'u3', 'u1', 'admin'), ('s2', 'u3', 'u2', 'admin')")
            .execute(&db.pool)
            .await
            .unwrap();
        sqlx::query(
            r#"CREATE TABLE invitation_codes (
                code TEXT PRIMARY KEY, session_id TEXT NOT NULL, created_by TEXT NOT NULL,
                expires_at DATETIME NOT NULL, redeemed_by TEXT, redeemed_at DATETIME,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"#,
        )
        .execute(&db.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO invitation_codes (code, session_id, created_by, expires_at) VALUES ('invite', 's2', 'u2', '2099-01-01T00:00:00Z')")
            .execute(&db.pool)
            .await
            .unwrap();

        db.run_migrations().await.unwrap();

        let owner = db.get_project("project-a").await.unwrap().unwrap();
        assert_eq!(owner.owner_user_id, "u1");
        let members = db.list_project_members("project-a").await.unwrap();
        assert_eq!(
            members
                .iter()
                .filter(|member| member.user_id == "u2")
                .count(),
            1
        );
        assert_eq!(
            members
                .iter()
                .filter(|member| member.user_id == "u3")
                .count(),
            1
        );
        assert!(members.iter().all(|member| member.user_id != "u1"));
        let invitation = db.get_invitation_code("invite").await.unwrap().unwrap();
        assert_eq!(invitation.project_id.as_deref(), Some("project-a"));
        let migrated_user = db.get_user_by_id("u3").await.unwrap().unwrap();
        assert_eq!(migrated_user.cluster_role, "user");
        assert_eq!(migrated_user.account_status, "active");
        let actions = db
            .list_audit_events(50, 0)
            .await
            .unwrap()
            .into_iter()
            .map(|event| event.action)
            .collect::<Vec<_>>();
        assert!(actions.contains(&"migration.conflicting_project_owners".to_string()));
        assert!(actions.contains(&"migration.legacy_project_admin_downgraded".to_string()));

        let imported = db
            .import_legacy_project_policy("project-a", true, &["terminal:codex".to_string()])
            .await
            .unwrap();
        assert!(imported.team_available);
        assert!(!imported.allows(shared::PaneKind::Terminal, shared::Provider::Codex, None,));
    }

    #[tokio::test]
    async fn legacy_access_upgrade_repairs_a_membership_resurrected_after_leave() {
        let db = database("legacy-access-repair").await;
        db.run_migrations().await.unwrap();
        db.create_user(&user("original-owner", "original@test", "user"))
            .await
            .unwrap();
        db.create_user(&user("new-owner", "new@test", "user"))
            .await
            .unwrap();
        db.authorize_project_registration("project-a", "original-owner")
            .await
            .unwrap();
        db.add_project_member("original-owner", "project-a", "new-owner")
            .await
            .unwrap();
        db.create_session(&Session {
            id: "historical-session".to_string(),
            user_id: "original-owner".to_string(),
            cli_client_id: None,
            working_dir: None,
            hostname: None,
            status: "active".to_string(),
            created_at: None,
            updated_at: None,
            is_paused: false,
            project_id: Some("project-a".to_string()),
            git_remote: None,
            git_remote_url: None,
        })
        .await
        .unwrap();
        assert!(db
            .transfer_project_ownership_by_owner("original-owner", "project-a", "new-owner")
            .await
            .unwrap());
        assert!(db
            .leave_project("original-owner", "project-a")
            .await
            .unwrap());

        // Simulate an installation upgrading from the old unbounded
        // backfill: the stale membership has already been recreated, while
        // the new migration marker does not exist yet.
        sqlx::query(
            "INSERT INTO project_members (project_id, user_id, invited_by) VALUES ('project-a', 'original-owner', 'new-owner')",
        )
        .execute(&db.pool)
        .await
        .unwrap();
        sqlx::query("DELETE FROM schema_migrations WHERE name = ?")
            .bind(LEGACY_PROJECT_ACCESS_MIGRATION)
            .execute(&db.pool)
            .await
            .unwrap();

        db.run_migrations().await.unwrap();
        assert!(db
            .get_project_role_for_user("project-a", "original-owner")
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM schema_migrations WHERE name = ?",)
                .bind(LEGACY_PROJECT_ACCESS_MIGRATION)
                .fetch_one(&db.pool)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn bootstrap_and_last_active_admin_guards_are_transactional() {
        let db = database("admin-guard").await;
        db.run_migrations().await.unwrap();
        db.create_user(&user("u1", "first@test", "user"))
            .await
            .unwrap();
        db.create_user(&user("u2", "second@test", "user"))
            .await
            .unwrap();
        assert!(db.bootstrap_cluster_admin("first@test").await.unwrap());
        assert!(!db.bootstrap_cluster_admin("second@test").await.unwrap());
        assert!(db
            .update_cluster_user_role("u1", "u1", ClusterRole::User)
            .await
            .unwrap_err()
            .to_string()
            .contains("last active"));
        assert!(db
            .update_cluster_user_status("u1", "u1", AccountStatus::Suspended)
            .await
            .unwrap_err()
            .to_string()
            .contains("last active"));
        db.update_cluster_user_role("u1", "u2", ClusterRole::Admin)
            .await
            .unwrap();
        assert!(db
            .update_cluster_user_status("u2", "u1", AccountStatus::Suspended)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn policy_inheritance_versions_and_legacy_conflicts_preserve_server_state() {
        let db = database("policy").await;
        db.run_migrations().await.unwrap();
        db.create_user(&user("admin", "admin@test", "admin"))
            .await
            .unwrap();
        db.authorize_project_registration("project-a", "admin")
            .await
            .unwrap();

        let initial = db.get_effective_project_policy("project-a").await.unwrap();
        assert!(!initial.team_available);
        let defaults = db
            .set_cluster_default_policy(
                "admin",
                true,
                vec!["agent:codex:official:default".to_string()],
            )
            .await
            .unwrap();
        assert!(defaults.version > initial.version);
        let inherited = db.get_effective_project_policy("project-a").await.unwrap();
        assert!(inherited.team_available);
        assert!(inherited.allows(shared::PaneKind::Agent, shared::Provider::Codex, None));
        assert!(!inherited.allows(shared::PaneKind::Agent, shared::Provider::Claude, None));

        // A project created after the migration inherits server defaults;
        // its ordinary `.apas` snapshot must not become an implicit override.
        let ignored = db
            .import_legacy_project_policy("project-a", false, &[])
            .await
            .unwrap();
        assert_eq!(ignored, inherited);

        let overridden = db
            .set_project_policy_override(
                "admin",
                "project-a",
                Some(false),
                Some(vec!["agent:claude:official:default".to_string()]),
            )
            .await
            .unwrap();
        assert!(overridden.version > inherited.version);
        assert!(!overridden.team_available);
        assert!(overridden.allows(shared::PaneKind::Agent, shared::Provider::Claude, None));

        let after_legacy = db
            .import_legacy_project_policy("project-a", true, &[])
            .await
            .unwrap();
        assert_eq!(after_legacy, overridden);
        let conflict = sqlx::query_scalar::<_, String>(
            "SELECT legacy_conflict FROM project_policy_overrides WHERE project_id = 'project-a'",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert!(conflict.contains("team_available"));
    }

    #[tokio::test]
    async fn retired_profile_migration_is_ordered_versioned_and_idempotent() {
        let db = database("retired-policy").await;
        db.run_migrations().await.unwrap();
        db.create_user(&user("admin", "admin@test", "admin"))
            .await
            .unwrap();
        for project_id in ["clean", "mixed", "retired-only"] {
            db.authorize_project_registration(project_id, "admin")
                .await
                .unwrap();
        }

        let retired_minimax = "agent:claude:minimax:minimax-m2.7";
        let retired_glm = "agent:claude:glm:glm-5.1";
        let claude = "agent:claude:official:default";
        let codex = "agent:codex:official:default";
        sqlx::query(
            "UPDATE cluster_settings SET allowed_launch_profiles = ?, version = 4 WHERE id = 1",
        )
        .bind(serde_json::to_string(&vec![claude, retired_minimax, codex]).unwrap())
        .execute(&db.pool)
        .await
        .unwrap();
        for (project_id, profiles, version) in [
            ("clean", vec![codex], 6_i64),
            (
                "mixed",
                vec![retired_glm, claude, retired_minimax, codex],
                7,
            ),
            ("retired-only", vec![retired_minimax, retired_glm], 5),
        ] {
            sqlx::query(
                "INSERT INTO project_policy_overrides (project_id, allowed_launch_profiles, version) VALUES (?, ?, ?)",
            )
            .bind(project_id)
            .bind(serde_json::to_string(&profiles).unwrap())
            .bind(version)
            .execute(&db.pool)
            .await
            .unwrap();
        }

        let changed = db.normalize_retired_provider_profiles().await.unwrap();
        assert!(changed.cluster_default_changed);
        assert_eq!(
            changed.changed_project_ids,
            vec!["mixed".to_string(), "retired-only".to_string()]
        );

        let (cluster_json, cluster_version) = sqlx::query_as::<_, (String, i64)>(
            "SELECT allowed_launch_profiles, version FROM cluster_settings WHERE id = 1",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&cluster_json).unwrap(),
            vec![claude.to_string(), codex.to_string()]
        );
        assert_eq!(cluster_version, 8);

        let rows = sqlx::query(
            "SELECT project_id, allowed_launch_profiles, version FROM project_policy_overrides ORDER BY project_id",
        )
        .fetch_all(&db.pool)
        .await
        .unwrap();
        let stored = rows
            .into_iter()
            .map(|row| {
                (
                    row.get::<String, _>("project_id"),
                    serde_json::from_str::<Vec<String>>(
                        &row.get::<String, _>("allowed_launch_profiles"),
                    )
                    .unwrap(),
                    row.get::<i64, _>("version"),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(stored[0], ("clean".to_string(), vec![codex.to_string()], 6));
        assert_eq!(
            stored[1],
            (
                "mixed".to_string(),
                vec![claude.to_string(), codex.to_string()],
                9
            )
        );
        assert_eq!(stored[2], ("retired-only".to_string(), vec![], 10));

        let second = db.normalize_retired_provider_profiles().await.unwrap();
        assert_eq!(second, RetiredProfileMigration::default());
        let versions = sqlx::query_scalar::<_, i64>(
            "SELECT SUM(version) FROM cluster_settings UNION ALL SELECT SUM(version) FROM project_policy_overrides",
        )
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert_eq!(versions, vec![8, 25]);
    }

    #[tokio::test]
    async fn policy_mutations_reject_retired_and_unknown_profiles() {
        let db = database("retired-policy-validation").await;
        db.run_migrations().await.unwrap();
        db.create_user(&user("admin", "admin@test", "admin"))
            .await
            .unwrap();
        db.authorize_project_registration("project-a", "admin")
            .await
            .unwrap();

        let retired = vec!["agent:claude:glm:glm-5.1".to_string()];
        assert!(db
            .set_cluster_default_policy("admin", true, retired.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("retired"));
        assert!(db
            .set_project_policy_override("admin", "project-a", None, Some(retired))
            .await
            .unwrap_err()
            .to_string()
            .contains("retired"));
        assert!(db
            .set_cluster_default_policy(
                "admin",
                true,
                vec!["agent:claude:official:unknown".to_string()],
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("unsupported"));
    }

    #[tokio::test]
    async fn canonical_membership_controls_all_instances_and_suspension() {
        let db = database("access-matrix").await;
        db.run_migrations().await.unwrap();
        db.create_user(&user("owner", "owner@test", "user"))
            .await
            .unwrap();
        db.create_user(&user("member", "member@test", "user"))
            .await
            .unwrap();
        db.create_user(&user("stranger", "stranger@test", "admin"))
            .await
            .unwrap();
        db.authorize_project_registration("project-a", "owner")
            .await
            .unwrap();
        db.add_project_member("owner", "project-a", "member")
            .await
            .unwrap();
        for id in ["instance-a", "instance-b"] {
            db.create_session(&Session {
                id: id.to_string(),
                user_id: "owner".to_string(),
                cli_client_id: None,
                working_dir: None,
                hostname: None,
                status: "active".to_string(),
                created_at: None,
                updated_at: None,
                is_paused: false,
                project_id: Some("project-a".to_string()),
                git_remote: None,
                git_remote_url: None,
            })
            .await
            .unwrap();
            assert!(db.check_session_access(id, "owner").await.unwrap());
            assert!(db.check_session_access(id, "member").await.unwrap());
            // Cluster administration never implies content-plane access.
            assert!(!db.check_session_access(id, "stranger").await.unwrap());
        }
        db.set_project_lifecycle("stranger", "project-a", ProjectLifecycle::Suspended)
            .await
            .unwrap();
        assert!(!db
            .check_session_access("instance-a", "owner")
            .await
            .unwrap());
        assert!(db
            .authorize_project_registration("project-a", "owner")
            .await
            .unwrap_err()
            .to_string()
            .contains("suspended"));
    }

    #[tokio::test]
    async fn admin_project_inventory_includes_session_name_and_host() {
        let db = database("admin-project-metadata").await;
        db.run_migrations().await.unwrap();
        db.create_user(&user("owner", "owner@test", "admin"))
            .await
            .unwrap();
        db.authorize_project_registration("project-a", "owner")
            .await
            .unwrap();

        for (id, working_dir, hostname, status) in [
            (
                "current-session",
                "/home/users/shuai/mako-soumojit/",
                "zoo-002",
                "active",
            ),
            (
                "newer-archived-session",
                "/tmp/archived-name",
                "old-host",
                "completed",
            ),
        ] {
            db.create_session(&Session {
                id: id.to_string(),
                user_id: "owner".to_string(),
                cli_client_id: None,
                working_dir: Some(working_dir.to_string()),
                hostname: Some(hostname.to_string()),
                status: status.to_string(),
                created_at: None,
                updated_at: None,
                is_paused: false,
                project_id: Some("project-a".to_string()),
                git_remote: Some("github.com/example/fallback-name".to_string()),
                git_remote_url: None,
            })
            .await
            .unwrap();
        }

        let projects = db.list_admin_projects(None, 50, 0).await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].project_name.as_deref(), Some("mako-soumojit"));
        assert_eq!(projects[0].hostname.as_deref(), Some("zoo-002"));
        assert_eq!(
            db.list_admin_projects(Some("mako-soumojit"), 50, 0)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            db.list_admin_projects(Some("zoo-002"), 50, 0)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn cluster_invitation_redemption_is_single_use_and_atomic() {
        let db = database("cluster-invitation").await;
        db.run_migrations().await.unwrap();
        db.create_user(&user("admin", "admin@test", "admin"))
            .await
            .unwrap();
        db.create_cluster_invitation(&ClusterInvitation {
            code: "invite-once".to_string(),
            email: "new@test".to_string(),
            created_by: "admin".to_string(),
            expires_at: (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            redeemed_at: None,
            created_at: None,
        })
        .await
        .unwrap();
        assert!(!db
            .create_user_redeeming_cluster_invitation(
                &user("wrong", "wrong@test", "user"),
                "invite-once",
            )
            .await
            .unwrap());
        assert!(db.get_user_by_id("wrong").await.unwrap().is_none());
        assert!(db
            .create_user_redeeming_cluster_invitation(
                &user("new", "new@test", "user"),
                "invite-once",
            )
            .await
            .unwrap());
        assert!(!db
            .create_user_redeeming_cluster_invitation(
                &user("duplicate", "new@test", "user"),
                "invite-once",
            )
            .await
            .unwrap());
        assert!(db
            .list_audit_events(20, 0)
            .await
            .unwrap()
            .iter()
            .any(|event| event.action == "cluster_user.created" && event.target_id == "new"));
    }

    #[tokio::test]
    async fn project_invitation_redeems_once_into_canonical_membership() {
        let db = database("project-invitation").await;
        db.run_migrations().await.unwrap();
        db.create_user(&user("owner", "owner@test", "user"))
            .await
            .unwrap();
        db.create_user(&user("member", "member@test", "user"))
            .await
            .unwrap();
        db.authorize_project_registration("project-a", "owner")
            .await
            .unwrap();
        for id in ["instance-a", "instance-b"] {
            db.create_session(&Session {
                id: id.to_string(),
                user_id: "owner".to_string(),
                cli_client_id: None,
                working_dir: None,
                hostname: None,
                status: "active".to_string(),
                created_at: None,
                updated_at: None,
                is_paused: false,
                project_id: Some("project-a".to_string()),
                git_remote: None,
                git_remote_url: None,
            })
            .await
            .unwrap();
        }
        db.create_invitation_code(&InvitationCode {
            code: "project-invite".to_string(),
            session_id: "instance-a".to_string(),
            project_id: Some("project-a".to_string()),
            created_by: "owner".to_string(),
            expires_at: (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            redeemed_by: None,
            redeemed_at: None,
            created_at: None,
        })
        .await
        .unwrap();
        assert!(db
            .redeem_project_invitation("project-invite", "member")
            .await
            .unwrap());
        assert!(!db
            .redeem_project_invitation("project-invite", "member")
            .await
            .unwrap());
        assert!(db
            .check_session_access("instance-a", "member")
            .await
            .unwrap());
        assert!(db
            .check_session_access("instance-b", "member")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn deleting_lifecycle_is_internal_fail_closed_and_restart_discoverable() {
        assert_eq!(
            ProjectLifecycle::parse("future-value"),
            ProjectLifecycle::Deleting
        );
        let db = database("deleting-lifecycle").await;
        db.run_migrations().await.unwrap();
        db.create_user(&user("owner", "owner@test", "user"))
            .await
            .unwrap();
        db.create_user(&user("member", "member@test", "user"))
            .await
            .unwrap();
        db.create_user(&user("legacy-share", "legacy-share@test", "user"))
            .await
            .unwrap();
        db.authorize_project_registration("project-a", "owner")
            .await
            .unwrap();
        db.add_project_member("owner", "project-a", "member")
            .await
            .unwrap();
        db.create_session(&Session {
            id: "session-a".to_string(),
            user_id: "owner".to_string(),
            cli_client_id: None,
            working_dir: None,
            hostname: None,
            status: "active".to_string(),
            created_at: None,
            updated_at: None,
            is_paused: false,
            project_id: Some("project-a".to_string()),
            git_remote: None,
            git_remote_url: None,
        })
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO session_shares (session_id, user_id, invited_by, role) VALUES (?, ?, ?, ?)",
        )
        .bind("session-a")
        .bind("legacy-share")
        .bind("owner")
        .bind("user")
        .execute(&db.pool)
        .await
        .unwrap();

        assert!(db
            .begin_project_deletion("member", "project-a", "project-a")
            .await
            .unwrap_err()
            .to_string()
            .contains("owner"));
        assert!(db
            .begin_project_deletion("owner", "project-a", "wrong")
            .await
            .unwrap_err()
            .to_string()
            .contains("confirmation"));
        assert!(db
            .begin_project_deletion("owner", "project-a", "project-a")
            .await
            .unwrap());
        assert!(!db
            .begin_project_deletion("owner", "project-a", "project-a")
            .await
            .unwrap());
        assert_eq!(
            db.get_project_deletion_status("project-a").await.unwrap(),
            Some(ProjectLifecycle::Deleting)
        );
        assert_eq!(
            db.list_deleting_project_ids().await.unwrap(),
            vec!["project-a".to_string()]
        );
        let manifest = db
            .get_project_deletion_manifest("project-a")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(manifest.session_ids, vec!["session-a".to_string()]);
        assert_eq!(
            manifest.affected_user_ids,
            vec![
                "legacy-share".to_string(),
                "member".to_string(),
                "owner".to_string()
            ]
        );
        assert!(db
            .authorize_project_registration("project-a", "owner")
            .await
            .unwrap_err()
            .to_string()
            .contains("deletion"));
        assert!(!db
            .set_project_lifecycle("owner", "project-a", ProjectLifecycle::Active)
            .await
            .unwrap());
        assert!(db
            .set_project_lifecycle("owner", "project-a", ProjectLifecycle::Deleting)
            .await
            .unwrap_err()
            .to_string()
            .contains("internal"));
        assert!(sqlx::query(
            "UPDATE projects SET lifecycle_status = 'invalid' WHERE id = 'project-a'",
        )
        .execute(&db.pool)
        .await
        .is_err());

        db.authorize_project_registration("project-b", "owner")
            .await
            .unwrap();
        db.set_project_lifecycle("owner", "project-b", ProjectLifecycle::Suspended)
            .await
            .unwrap();
        assert!(db
            .begin_project_deletion("owner", "project-b", "project-b")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn project_audit_keys_are_populated_and_legacy_rows_are_backfilled() {
        let db = database("audit-project-key").await;
        db.run_migrations().await.unwrap();
        db.create_user(&user("owner", "owner@test", "user"))
            .await
            .unwrap();
        db.authorize_project_registration("project-a", "owner")
            .await
            .unwrap();
        db.record_audit(
            "owner",
            "project.member_test",
            "project_member",
            "owner",
            Some(serde_json::json!({ "project_id": "project-a" })),
        )
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO admin_audit_events (actor_user_id, action, target_type, target_id, details) VALUES ('owner', 'legacy.project', 'project', 'project-a', 'not-json')",
        )
        .execute(&db.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO admin_audit_events (actor_user_id, action, target_type, target_id, details) VALUES ('owner', 'legacy.member', 'project_member', 'owner', '{\"project_id\":\"project-a\"}')",
        )
        .execute(&db.pool)
        .await
        .unwrap();
        db.run_migrations().await.unwrap();

        let events = db.list_audit_events(20, 0).await.unwrap();
        for action in ["project.member_test", "legacy.project", "legacy.member"] {
            assert_eq!(
                events
                    .iter()
                    .find(|event| event.action == action)
                    .and_then(|event| event.project_id.as_deref()),
                Some("project-a")
            );
        }
    }

    #[tokio::test]
    async fn owner_transfer_and_member_departure_sync_project_and_legacy_roles() {
        let db = database("owner-transfer-leave").await;
        db.run_migrations().await.unwrap();
        for (id, role) in [
            ("owner", "user"),
            ("member", "user"),
            ("outsider", "user"),
            ("admin", "admin"),
        ] {
            db.create_user(&user(id, &format!("{id}@test"), role))
                .await
                .unwrap();
        }
        db.authorize_project_registration("project-a", "owner")
            .await
            .unwrap();
        db.add_project_member("owner", "project-a", "member")
            .await
            .unwrap();
        for id in ["instance-a", "instance-b"] {
            db.create_session(&Session {
                id: id.to_string(),
                user_id: "owner".to_string(),
                cli_client_id: None,
                working_dir: None,
                hostname: None,
                status: "active".to_string(),
                created_at: None,
                updated_at: None,
                is_paused: false,
                project_id: Some("project-a".to_string()),
                git_remote: None,
                git_remote_url: None,
            })
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO session_shares (session_id, user_id, invited_by, role) VALUES (?, 'member', 'owner', 'user')",
            )
            .bind(id)
            .execute(&db.pool)
            .await
            .unwrap();
        }

        assert!(db
            .transfer_project_ownership_by_owner("member", "project-a", "member")
            .await
            .unwrap_err()
            .to_string()
            .contains("owner"));
        assert!(db
            .transfer_project_ownership_by_owner("owner", "project-a", "outsider")
            .await
            .unwrap_err()
            .to_string()
            .contains("existing project user"));
        sqlx::query("UPDATE users SET account_status = 'suspended' WHERE id = 'member'")
            .execute(&db.pool)
            .await
            .unwrap();
        assert!(db
            .transfer_project_ownership_by_owner("owner", "project-a", "member")
            .await
            .unwrap_err()
            .to_string()
            .contains("suspended"));
        sqlx::query("UPDATE users SET account_status = 'active' WHERE id = 'member'")
            .execute(&db.pool)
            .await
            .unwrap();

        assert!(db
            .transfer_project_ownership_by_owner("owner", "project-a", "member")
            .await
            .unwrap());
        assert_eq!(
            db.get_project_role_for_user("project-a", "member")
                .await
                .unwrap()
                .as_deref(),
            Some("owner")
        );
        assert_eq!(
            db.get_project_role_for_user("project-a", "owner")
                .await
                .unwrap()
                .as_deref(),
            Some("user")
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM session_shares WHERE user_id = 'member'",
            )
            .fetch_one(&db.pool)
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM session_shares WHERE user_id = 'owner'",
            )
            .fetch_one(&db.pool)
            .await
            .unwrap(),
            2
        );
        assert!(db
            .leave_project("member", "project-a")
            .await
            .unwrap_err()
            .to_string()
            .contains("owner"));
        assert!(db.leave_project("owner", "project-a").await.unwrap());
        assert!(db
            .get_project_role_for_user("project-a", "owner")
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM session_shares WHERE user_id = 'owner'",
            )
            .fetch_one(&db.pool)
            .await
            .unwrap(),
            0
        );

        // A server restart reruns schema setup, but historical session
        // ownership must never recreate an intentionally removed membership.
        db.run_migrations().await.unwrap();
        assert!(db
            .get_project_role_for_user("project-a", "owner")
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM session_shares WHERE user_id = 'owner'",
            )
            .fetch_one(&db.pool)
            .await
            .unwrap(),
            0
        );

        // Administrator policy remains broader than owner policy.
        assert!(db
            .transfer_project_ownership("admin", "project-a", "outsider")
            .await
            .unwrap());
        assert_eq!(
            db.get_project_role_for_user("project-a", "outsider")
                .await
                .unwrap()
                .as_deref(),
            Some("owner")
        );
    }

    #[tokio::test]
    async fn concurrent_transfer_and_departure_preserve_single_owner() {
        let db = database("transfer-leave-race").await;
        db.run_migrations().await.unwrap();
        db.create_user(&user("owner", "owner@test", "user"))
            .await
            .unwrap();
        db.create_user(&user("member", "member@test", "user"))
            .await
            .unwrap();
        db.authorize_project_registration("project-a", "owner")
            .await
            .unwrap();
        db.add_project_member("owner", "project-a", "member")
            .await
            .unwrap();

        let transfer_db = db.clone();
        let leave_db = db.clone();
        let (_transfer, _leave) = tokio::join!(
            transfer_db.transfer_project_ownership_by_owner("owner", "project-a", "member"),
            leave_db.leave_project("member", "project-a")
        );

        let project = db.get_project("project-a").await.unwrap().unwrap();
        assert!(matches!(project.owner_user_id.as_str(), "owner" | "member"));
        let owner_membership = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM project_members WHERE project_id = 'project-a' AND user_id = ?",
        )
        .bind(&project.owner_user_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(
            owner_membership, 0,
            "the sole owner is never duplicated as a user"
        );
    }
}

#[cfg(test)]
mod mobile_task_launch_tests {
    use super::*;

    #[tokio::test]
    async fn launch_claims_retain_one_result_and_cascade_without_prompt_storage() {
        let dir =
            std::env::temp_dir().join(format!("apas-mobile-task-launch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::new(&dir.join("apas.db").to_string_lossy())
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let user_id = uuid::Uuid::new_v4().to_string();
        db.create_user(&User {
            id: user_id.clone(),
            email: "mobile-launch@test".to_string(),
            password_hash: "hash".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .unwrap();
        db.authorize_project_registration("project-a", &user_id)
            .await
            .unwrap();
        let device_session_id = uuid::Uuid::new_v4().to_string();
        db.create_mobile_device_session(
            &device_session_id,
            &user_id,
            "installation-a",
            "ios",
            Some("phone"),
            "1.0.0",
            "refresh-hash",
            &(chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339(),
        )
        .await
        .unwrap();
        let request_id = uuid::Uuid::new_v4().to_string();
        let first = db
            .claim_mobile_task_launch(
                &request_id,
                &user_id,
                &device_session_id,
                "keyed-fingerprint",
                &uuid::Uuid::new_v4().to_string(),
                "project-a",
            )
            .await
            .unwrap();
        assert_eq!(first.status, "pending");
        assert!(db
            .complete_mobile_task_launch(
                &request_id,
                &user_id,
                &uuid::Uuid::new_v4().to_string(),
                42
            )
            .await
            .unwrap());
        assert!(!db
            .complete_mobile_task_launch(
                &request_id,
                &user_id,
                &uuid::Uuid::new_v4().to_string(),
                99
            )
            .await
            .unwrap());
        let retained = db
            .claim_mobile_task_launch(
                &request_id,
                &user_id,
                &device_session_id,
                "different-fingerprint",
                &uuid::Uuid::new_v4().to_string(),
                "project-a",
            )
            .await
            .unwrap();
        assert_eq!(retained.status, "completed");
        assert_eq!(retained.request_fingerprint, "keyed-fingerprint");
        assert_eq!(retained.pane_id, Some(42));

        let columns = sqlx::query("PRAGMA table_info(mobile_task_launches)")
            .fetch_all(&db.pool)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect::<Vec<_>>();
        assert!(!columns
            .iter()
            .any(|name| name.contains("instruction") || name.contains("prompt")));

        sqlx::query("DELETE FROM projects WHERE id = 'project-a'")
            .execute(&db.pool)
            .await
            .unwrap();
        let remaining = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM mobile_task_launches WHERE request_id = ?",
        )
        .bind(&request_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(remaining, 0);
    }
}
