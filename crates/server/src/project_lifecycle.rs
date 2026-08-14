use anyhow::{Context, Result};
use shared::ProjectAccessChange;
use std::time::Duration;
use uuid::Uuid;

use crate::state::AppState;

/// Complete one project's irreversible cleanup. The database project/session
/// rows remain the recovery manifest until file deletion has succeeded.
pub async fn cleanup_project(state: &AppState, project_id: &str) -> Result<()> {
    let guard = state.project_deletion_guard(project_id).await;
    let Some(manifest) = state.db.get_project_deletion_manifest(project_id).await? else {
        return Ok(());
    };
    let session_ids = manifest
        .session_ids
        .iter()
        .map(|id| Uuid::parse_str(id).with_context(|| "invalid persisted session identity"))
        .collect::<Result<Vec<_>>>()?;
    let affected_user_ids = manifest
        .affected_user_ids
        .iter()
        .filter_map(|id| Uuid::parse_str(id).ok())
        .collect::<Vec<_>>();

    state
        .sessions
        .confirm_project_runtime_stopped(project_id)
        .await?;
    state
        .sessions
        .purge_project_state(project_id, &affected_user_ids)
        .await;
    state.storage.delete_session_dirs(&session_ids).await?;
    state.db.delete_project_records(project_id).await?;

    for user_id in &affected_user_ids {
        state.sessions.notify_project_access_changed(
            user_id,
            project_id,
            ProjectAccessChange::Deleted,
            None,
        );
    }
    state.forget_project_mutation_gate(project_id);
    drop(guard);
    Ok(())
}

/// Shield cleanup from an HTTP client disconnect and retry transient storage
/// failures a bounded number of times. Durable `deleting` rows are recovered
/// again on the next server start if all attempts fail.
pub fn schedule_project_cleanup(state: AppState, project_id: String) {
    tokio::spawn(async move {
        for attempt in 0..3_u64 {
            match cleanup_project(&state, &project_id).await {
                Ok(()) => return,
                Err(_) => {
                    tracing::warn!(
                        attempt = attempt + 1,
                        "project deletion cleanup remains incomplete"
                    );
                    if attempt < 2 {
                        tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                    }
                }
            }
        }
    });
}

/// Resume every interrupted deletion before normal traffic is accepted.
pub async fn recover_interrupted_deletions(state: &AppState) -> Result<()> {
    for project_id in state.db.list_deleting_project_ids().await? {
        if cleanup_project(state, &project_id).await.is_err() {
            tracing::warn!("startup project deletion recovery remains incomplete");
            schedule_project_cleanup(state.clone(), project_id);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::{InvitationCode, Message, Session, UsageDelta, User},
        storage::StoredMessage,
    };
    use sqlx::SqlitePool;
    use tokio::sync::mpsc;

    struct Fixture {
        state: AppState,
        pool: SqlitePool,
        root: std::path::PathBuf,
        owner: Uuid,
        member: Uuid,
        project: Uuid,
        sessions: [Uuid; 2],
    }

    async fn fixture() -> Fixture {
        let root = std::env::temp_dir().join(format!("apas-deletion-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db_path = root.join("apas.db");
        let db = crate::db::Database::new(&db_path.to_string_lossy())
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let mut config = crate::config::Config::default();
        config.database.path = db_path.to_string_lossy().into_owned();
        let state = AppState::new(db, config);
        let pool = SqlitePool::connect(&format!("sqlite://{}", db_path.display()))
            .await
            .unwrap();
        let owner = Uuid::new_v4();
        let member = Uuid::new_v4();
        let project = Uuid::new_v4();
        let sessions = [Uuid::new_v4(), Uuid::new_v4()];
        for (id, email) in [(owner, "owner@test"), (member, "member@test")] {
            state
                .db
                .create_user(&User {
                    id: id.to_string(),
                    email: email.to_string(),
                    password_hash: "hash".to_string(),
                    created_at: None,
                    cluster_role: "user".to_string(),
                    account_status: "active".to_string(),
                })
                .await
                .unwrap();
        }
        state
            .db
            .authorize_project_registration(&project.to_string(), &owner.to_string())
            .await
            .unwrap();
        for session_id in sessions {
            state
                .db
                .create_session(&Session {
                    id: session_id.to_string(),
                    user_id: owner.to_string(),
                    cli_client_id: None,
                    working_dir: Some("/work/project".to_string()),
                    hostname: Some("host".to_string()),
                    status: "active".to_string(),
                    created_at: None,
                    updated_at: None,
                    is_paused: false,
                    project_id: Some(project.to_string()),
                    git_remote: None,
                    git_remote_url: None,
                })
                .await
                .unwrap();
        }
        state
            .db
            .add_project_member(
                &owner.to_string(),
                &project.to_string(),
                &member.to_string(),
            )
            .await
            .unwrap();
        Fixture {
            state,
            pool,
            root,
            owner,
            member,
            project,
            sessions,
        }
    }

    async fn count(pool: &SqlitePool, query: &str, value: &str) -> i64 {
        sqlx::query_scalar(query)
            .bind(value)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn cleanup_erases_every_managed_artifact_and_allows_fresh_registration() {
        let f = fixture().await;
        for (index, session_id) in f.sessions.iter().enumerate() {
            f.state
                .db
                .save_message(&Message {
                    id: Uuid::new_v4().to_string(),
                    session_id: session_id.to_string(),
                    role: "assistant".to_string(),
                    content: format!("secret-{index}"),
                    message_type: "text".to_string(),
                    metadata: None,
                    created_at: None,
                })
                .await
                .unwrap();
            f.state
                .db
                .add_pane_usage(
                    &session_id.to_string(),
                    1,
                    "2026-08-08",
                    &UsageDelta {
                        prompt_count: 1,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            f.state
                .db
                .create_session_share_with_role(
                    &session_id.to_string(),
                    &f.member.to_string(),
                    &f.owner.to_string(),
                    "user",
                )
                .await
                .unwrap();
            f.state
                .storage
                .append_message(
                    session_id,
                    &StoredMessage {
                        id: Uuid::new_v4().to_string(),
                        role: "assistant".to_string(),
                        content: "file secret".to_string(),
                        message_type: "text".to_string(),
                        created_at: chrono::Utc::now().to_rfc3339(),
                        pane_type: Some("1".to_string()),
                    },
                )
                .await
                .unwrap();
            tokio::fs::write(
                f.root
                    .join("sessions")
                    .join(session_id.to_string())
                    .join("messages.jsonl.gc.tmp"),
                b"temporary secret",
            )
            .await
            .unwrap();
        }
        f.state
            .db
            .create_invitation_code(&InvitationCode {
                code: "PROJECT-CODE".to_string(),
                session_id: f.sessions[0].to_string(),
                project_id: Some(f.project.to_string()),
                created_by: f.owner.to_string(),
                expires_at: (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
                redeemed_by: None,
                redeemed_at: None,
                created_at: None,
            })
            .await
            .unwrap();
        f.state
            .db
            .set_project_policy_override(
                &f.owner.to_string(),
                &f.project.to_string(),
                Some(true),
                None,
            )
            .await
            .unwrap();

        let cli_id = Uuid::new_v4();
        let (cli_tx, _cli_rx) = mpsc::channel(8);
        f.state.sessions.register_cli(cli_id, f.owner, cli_tx, None);
        f.state
            .sessions
            .create_cli_session(f.sessions[0], cli_id, None, None);
        f.state
            .sessions
            .set_session_project(f.sessions[0], f.project.to_string());
        f.state
            .sessions
            .append_terminal_output(&f.sessions[0], 1, None, b"terminal secret", 1);
        f.state
            .sessions
            .record_input_id(f.sessions[0], "input-id".to_string(), "time".to_string());

        f.state
            .db
            .begin_project_deletion(
                &f.owner.to_string(),
                &f.project.to_string(),
                &f.project.to_string(),
            )
            .await
            .unwrap();
        cleanup_project(&f.state, &f.project.to_string())
            .await
            .unwrap();

        assert!(f
            .state
            .db
            .get_project(&f.project.to_string())
            .await
            .unwrap()
            .is_none());
        assert!(f
            .state
            .db
            .list_admin_projects(None, 100, 0)
            .await
            .unwrap()
            .is_empty());
        for session_id in f.sessions {
            let sid = session_id.to_string();
            assert_eq!(
                count(&f.pool, "SELECT COUNT(*) FROM sessions WHERE id = ?", &sid).await,
                0
            );
            assert_eq!(
                count(
                    &f.pool,
                    "SELECT COUNT(*) FROM messages WHERE session_id = ?",
                    &sid
                )
                .await,
                0
            );
            assert_eq!(
                count(
                    &f.pool,
                    "SELECT COUNT(*) FROM pane_usage_stats WHERE session_id = ?",
                    &sid
                )
                .await,
                0
            );
            assert_eq!(
                count(
                    &f.pool,
                    "SELECT COUNT(*) FROM session_shares WHERE session_id = ?",
                    &sid
                )
                .await,
                0
            );
            assert!(!f.root.join("sessions").join(&sid).exists());
            assert!(f.state.sessions.project_for_session(&session_id).is_none());
            assert!(f.state.sessions.terminal_snapshot(&session_id, 1).is_none());
        }
        for table in [
            "project_members",
            "project_policy_overrides",
            "admin_audit_events",
        ] {
            let query = format!("SELECT COUNT(*) FROM {table} WHERE project_id = ?");
            assert_eq!(count(&f.pool, &query, &f.project.to_string()).await, 0);
        }
        assert_eq!(
            count(
                &f.pool,
                "SELECT COUNT(*) FROM invitation_codes WHERE project_id = ?",
                &f.project.to_string(),
            )
            .await,
            0
        );
        assert!(!f.state.sessions.is_cli_connected(&cli_id));

        f.state
            .db
            .authorize_project_registration(&f.project.to_string(), &f.owner.to_string())
            .await
            .unwrap();
        assert_eq!(
            f.state
                .db
                .get_project_role_for_user(&f.project.to_string(), &f.member.to_string())
                .await
                .unwrap(),
            None
        );
        assert!(f
            .state
            .storage
            .get_messages(&f.sessions[0])
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn recovery_retries_file_and_database_failures_without_exposing_project() {
        let f = fixture().await;
        let blocked_dir = f.root.join("sessions").join(f.sessions[0].to_string());
        std::fs::create_dir_all(blocked_dir.parent().unwrap()).unwrap();
        std::fs::write(&blocked_dir, b"not a directory").unwrap();
        f.state
            .db
            .begin_project_deletion(
                &f.owner.to_string(),
                &f.project.to_string(),
                &f.project.to_string(),
            )
            .await
            .unwrap();
        assert!(cleanup_project(&f.state, &f.project.to_string())
            .await
            .is_err());
        assert!(f
            .state
            .active_session_operation(&f.sessions[0].to_string())
            .await
            .is_err());
        std::fs::remove_file(&blocked_dir).unwrap();

        sqlx::query(
            "CREATE TRIGGER fail_project_delete BEFORE DELETE ON projects BEGIN SELECT RAISE(ABORT, 'injected failure'); END",
        )
        .execute(&f.pool)
        .await
        .unwrap();
        assert!(cleanup_project(&f.state, &f.project.to_string())
            .await
            .is_err());
        assert!(
            !blocked_dir.exists(),
            "file cleanup is idempotently complete"
        );
        assert_eq!(
            f.state
                .db
                .get_project_deletion_status(&f.project.to_string())
                .await
                .unwrap(),
            Some(crate::db::ProjectLifecycle::Deleting)
        );
        sqlx::query("DROP TRIGGER fail_project_delete")
            .execute(&f.pool)
            .await
            .unwrap();

        recover_interrupted_deletions(&f.state).await.unwrap();
        assert!(f
            .state
            .db
            .get_project(&f.project.to_string())
            .await
            .unwrap()
            .is_none());
        cleanup_project(&f.state, &f.project.to_string())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn deletion_drains_an_inflight_writer_and_rejects_late_writers() {
        let f = fixture().await;
        let operation_guard = f
            .state
            .project_operation_guard(&f.project.to_string())
            .await;
        f.state
            .db
            .begin_project_deletion(
                &f.owner.to_string(),
                &f.project.to_string(),
                &f.project.to_string(),
            )
            .await
            .unwrap();
        f.state
            .storage
            .append_message(
                &f.sessions[0],
                &StoredMessage {
                    id: Uuid::new_v4().to_string(),
                    role: "assistant".to_string(),
                    content: "in flight".to_string(),
                    message_type: "text".to_string(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    pane_type: None,
                },
            )
            .await
            .unwrap();

        let state = f.state.clone();
        let project_id = f.project.to_string();
        let cleanup = tokio::spawn(async move { cleanup_project(&state, &project_id).await });
        tokio::task::yield_now().await;
        assert!(
            !cleanup.is_finished(),
            "cleanup waits for the shared writer"
        );
        drop(operation_guard);
        cleanup.await.unwrap().unwrap();

        assert!(!f
            .root
            .join("sessions")
            .join(f.sessions[0].to_string())
            .exists());
        assert!(f
            .state
            .active_session_operation(&f.sessions[0].to_string())
            .await
            .is_err());
    }
}
