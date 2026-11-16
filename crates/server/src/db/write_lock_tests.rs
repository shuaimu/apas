//! Audit writers while another connection writes.
//!
//! A deferred transaction that reads before it writes is refused the write
//! lock at once, whatever `busy_timeout` is, when another connection holds
//! it. An open or close audit row was lost that way while a daemon
//! registered. Every writer here reads first, so each test holds the write
//! lock on a second connection, starts the writer, lets the lock go a moment
//! later, and expects the writer to have waited and its audit row to exist.

use super::*;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, SqliteConnection};
use std::future::Future;
use std::path::PathBuf;
use std::time::Instant;

/// How long the other connection keeps the write lock after the writer under
/// test has started. Far below the pool's five-second `busy_timeout`.
const HELD: Duration = Duration::from_millis(300);

struct Fixture {
    db: Database,
    path: PathBuf,
}

fn user(id: &str) -> User {
    User {
        id: id.to_string(),
        email: format!("{id}@test"),
        password_hash: "hash".to_string(),
        created_at: None,
        cluster_role: "user".to_string(),
        account_status: "active".to_string(),
    }
}

/// A migrated database with the accounts `owner` and `member`, and
/// `project-a` owned by `owner` and hosted in the owner's cluster.
async fn fixture(name: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("apas-write-lock-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp db dir");
    let path = dir.join("apas.db");
    let db = Database::new(&path.to_string_lossy())
        .await
        .expect("database");
    db.run_migrations().await.unwrap();
    for id in ["owner", "member"] {
        db.create_user(&user(id)).await.unwrap();
    }
    db.authorize_project_registration("project-a", "owner")
        .await
        .unwrap();
    Fixture { db, path }
}

/// A connection outside the pool that never waits for a lock, so a refusal
/// is seen at once.
async fn second_connection(fixture: &Fixture) -> SqliteConnection {
    SqliteConnectOptions::new()
        .filename(&fixture.path)
        .busy_timeout(Duration::ZERO)
        .connect()
        .await
        .expect("second connection")
}

/// Run `write` while a second connection holds the database's write lock.
/// That connection took the lock before `write` started and commits `HELD`
/// later, so `write` asks for the lock while it is taken.
async fn while_another_connection_writes<T>(
    fixture: &Fixture,
    write: impl Future<Output = T>,
) -> T {
    let mut other = second_connection(fixture).await;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut other)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO admin_audit_events (actor_user_id, action, target_type, target_id) VALUES ('owner', 'test.other_writer', 'test', 'other')",
    )
    .execute(&mut other)
    .await
    .unwrap();
    let started = Instant::now();
    let (result, released_after) = tokio::join!(write, async {
        tokio::time::sleep(HELD).await;
        sqlx::query("COMMIT").execute(&mut other).await.unwrap();
        started.elapsed()
    });
    // The writer finished after the lock was released, not before: it
    // waited for the lock instead of slipping in ahead of it.
    assert!(started.elapsed() >= released_after);
    assert_eq!(audit_rows(&fixture.db, "test.other_writer").await, 1);
    result
}

async fn audit_rows(db: &Database, action: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM admin_audit_events WHERE action = ?")
        .bind(action)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn begin_write_holds_the_write_lock_until_it_commits() {
    let fixture = fixture("begin-write").await;
    let mut other = second_connection(&fixture).await;

    let tx = fixture.db.begin_write().await.unwrap();
    let refused = sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut other)
        .await
        .expect_err("the transaction holds the write lock");
    assert!(
        refused.to_string().contains("database is locked"),
        "{refused}"
    );
    tx.commit().await.unwrap();

    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut other)
        .await
        .expect("the write lock is free after the commit");
    sqlx::query("ROLLBACK").execute(&mut other).await.unwrap();
    // Taking the lock wrote nothing.
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM admin_audit_events")
            .fetch_one(&fixture.db.pool)
            .await
            .unwrap(),
        0
    );
}

/// The lost row of the smoke run: a close audited by an account, filed under
/// its project, while another connection was writing.
#[tokio::test]
async fn an_open_state_audit_row_is_written_while_another_connection_writes() {
    let fixture = fixture("record-audit").await;
    let db = &fixture.db;

    while_another_connection_writes(
        &fixture,
        db.record_audit(
            "owner",
            "project.closed",
            "project",
            "project-a",
            Some(serde_json::json!({ "kind": "request", "source": "web", "machines": [] })),
        ),
    )
    .await
    .expect("the audit write waits for the other writer");

    let events = db.list_audit_events(20, 0).await.unwrap();
    let closed = events
        .iter()
        .find(|event| event.action == "project.closed")
        .expect("the close is in the audit history");
    assert_eq!(closed.actor_user_id, "owner");
    assert_eq!(closed.project_id.as_deref(), Some("project-a"));
    assert_eq!(closed.cluster_user_id.as_deref(), Some("owner"));
}

/// The same through the open-state code, which only logs a failed audit
/// write: nothing else would notice the missing row.
#[tokio::test]
async fn a_close_is_audited_through_the_open_state_code_while_another_connection_writes() {
    let fixture = fixture("open-state-audit").await;
    let mut config = crate::config::Config::default();
    config.database.path = fixture.path.to_string_lossy().into_owned();
    let state = crate::state::AppState::new(fixture.db.clone(), config);

    while_another_connection_writes(
        &fixture,
        crate::open_state::audit_open_state(
            &state,
            "owner",
            "project-a",
            shared::ProjectOpenState::Closed,
            shared::OpenStateSetKind::Request,
            "web",
            &[],
        ),
    )
    .await;

    let events = fixture.db.list_audit_events(20, 0).await.unwrap();
    let closed = events
        .iter()
        .find(|event| event.action == "project.closed")
        .expect("the close is in the audit history");
    assert_eq!(closed.project_id.as_deref(), Some("project-a"));
    let details: serde_json::Value =
        serde_json::from_str(closed.details.as_deref().expect("details")).unwrap();
    assert_eq!(details["kind"], "request");
    assert_eq!(details["source"], "web");
}

#[tokio::test]
async fn a_cluster_audit_row_is_written_while_another_connection_writes() {
    let fixture = fixture("record-cluster-audit").await;
    let db = &fixture.db;

    // A cluster target makes the write look the cluster's account up first.
    while_another_connection_writes(
        &fixture,
        db.record_audit("owner", "cluster.test", "cluster", "owner", None),
    )
    .await
    .expect("the audit write waits for the other writer");

    assert_eq!(audit_rows(db, "cluster.test").await, 1);
}

#[tokio::test]
async fn revoking_a_device_session_is_audited_while_another_connection_writes() {
    let fixture = fixture("revoke-device").await;
    let db = &fixture.db;
    db.create_mobile_device_session(
        "device-session",
        "owner",
        "installation",
        "ios",
        Some("Phone"),
        "0.1.0",
        "refresh-hash",
        &(chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339(),
    )
    .await
    .unwrap();

    let revoked = while_another_connection_writes(
        &fixture,
        db.revoke_mobile_device_session("owner", "device-session", false, "user_request"),
    )
    .await
    .expect("the revocation waits for the other writer");

    assert!(revoked);
    assert_eq!(audit_rows(db, "mobile.device_session_revoked").await, 1);
}

#[tokio::test]
async fn suspending_an_account_is_audited_while_another_connection_writes() {
    let fixture = fixture("account-status").await;
    let db = &fixture.db;

    let changed = while_another_connection_writes(
        &fixture,
        db.update_cluster_user_status(SYSTEM_ADMIN_ACTOR, "member", AccountStatus::Suspended),
    )
    .await
    .expect("the status change waits for the other writer");

    assert!(changed);
    assert_eq!(audit_rows(db, "cluster_user.status_changed").await, 1);
}

#[tokio::test]
async fn removing_a_project_member_is_audited_while_another_connection_writes() {
    let fixture = fixture("remove-member").await;
    let db = &fixture.db;
    assert!(db
        .add_project_member("owner", "project-a", "member")
        .await
        .unwrap());

    let removed = while_another_connection_writes(
        &fixture,
        db.remove_project_member("owner", "project-a", "member"),
    )
    .await
    .expect("the removal waits for the other writer");

    assert!(removed);
    assert_eq!(audit_rows(db, "project.member_removed").await, 1);
}

#[tokio::test]
async fn transferring_a_project_is_audited_while_another_connection_writes() {
    let fixture = fixture("transfer").await;
    let db = &fixture.db;
    assert!(db
        .add_project_member("owner", "project-a", "member")
        .await
        .unwrap());

    let transferred = while_another_connection_writes(
        &fixture,
        db.transfer_project_ownership("owner", "project-a", "member"),
    )
    .await
    .expect("the transfer waits for the other writer");

    assert!(transferred);
    assert_eq!(audit_rows(db, "project.owner_transferred").await, 1);
}

#[tokio::test]
async fn leaving_a_project_is_audited_while_another_connection_writes() {
    let fixture = fixture("leave").await;
    let db = &fixture.db;
    assert!(db
        .add_project_member("owner", "project-a", "member")
        .await
        .unwrap());

    let left = while_another_connection_writes(&fixture, db.leave_project("member", "project-a"))
        .await
        .expect("leaving waits for the other writer");

    assert!(left);
    assert_eq!(audit_rows(db, "project.member_left").await, 1);
}

#[tokio::test]
async fn a_cluster_default_policy_change_is_audited_while_another_connection_writes() {
    let fixture = fixture("cluster-policy").await;
    let db = &fixture.db;

    while_another_connection_writes(
        &fixture,
        db.set_cluster_default_policy("owner", "owner", None, None),
    )
    .await
    .expect("the policy change waits for the other writer");

    assert_eq!(audit_rows(db, "cluster.default_policy_changed").await, 1);
}

#[tokio::test]
async fn a_deployment_policy_change_is_audited_while_another_connection_writes() {
    let fixture = fixture("deployment-policy").await;
    let db = &fixture.db;
    let current = db.get_deployment_default_policy().await.unwrap();

    let saved = while_another_connection_writes(
        &fixture,
        db.set_deployment_default_policy(
            SYSTEM_ADMIN_ACTOR,
            current.team_available,
            current.allowed_launch_profiles.clone(),
        ),
    )
    .await
    .expect("the policy change waits for the other writer");

    assert_eq!(saved.version, current.version + 1);
    assert_eq!(audit_rows(db, "cluster.policy_changed").await, 1);
}

#[tokio::test]
async fn a_project_policy_change_is_audited_while_another_connection_writes() {
    let fixture = fixture("project-policy").await;
    let db = &fixture.db;

    while_another_connection_writes(
        &fixture,
        db.set_project_policy_override("owner", "project-a", None, None),
    )
    .await
    .expect("the policy change waits for the other writer");

    assert_eq!(audit_rows(db, "project.policy_changed").await, 1);
}

#[tokio::test]
async fn redeeming_a_project_invitation_is_audited_while_another_connection_writes() {
    let fixture = fixture("redeem-invitation").await;
    let db = &fixture.db;
    db.create_session(&Session {
        id: "instance-a".to_string(),
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

    let redeemed = while_another_connection_writes(
        &fixture,
        db.redeem_project_invitation("project-invite", "member"),
    )
    .await
    .expect("the redemption waits for the other writer");

    assert!(redeemed);
    assert_eq!(audit_rows(db, "project.member_added").await, 1);
}

#[tokio::test]
async fn inviting_to_a_cluster_is_audited_while_another_connection_writes() {
    let fixture = fixture("cluster-invitation").await;
    let db = &fixture.db;
    let expires_at = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();

    while_another_connection_writes(
        &fixture,
        db.create_shared_cluster_invitation(
            "invitation-1",
            "token-hash",
            "owner",
            "member@test",
            &expires_at,
        ),
    )
    .await
    .expect("the invitation waits for the other writer");

    assert_eq!(audit_rows(db, "cluster.invitation_created").await, 1);
}

#[tokio::test]
async fn accepting_a_cluster_invitation_is_audited_while_another_connection_writes() {
    let fixture = fixture("cluster-invitation-accepted").await;
    let db = &fixture.db;
    let expires_at = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    db.create_shared_cluster_invitation(
        "invitation-1",
        "token-hash",
        "owner",
        "member@test",
        &expires_at,
    )
    .await
    .unwrap();

    let membership = while_another_connection_writes(
        &fixture,
        db.accept_shared_cluster_invitation("token-hash", "member"),
    )
    .await
    .expect("the acceptance waits for the other writer");

    assert!(membership.is_some());
    assert_eq!(audit_rows(db, "cluster.member_joined").await, 1);
}

#[tokio::test]
async fn adding_a_cluster_member_is_audited_while_another_connection_writes() {
    let fixture = fixture("cluster-member").await;
    let db = &fixture.db;

    while_another_connection_writes(
        &fixture,
        db.add_cluster_member("owner", "member@test", None, None),
    )
    .await
    .expect("adding the member waits for the other writer");

    assert_eq!(audit_rows(db, "cluster.member_added").await, 1);
}

#[tokio::test]
async fn finishing_a_provisioned_project_is_audited_while_another_connection_writes() {
    let fixture = fixture("provisioning").await;
    let db = &fixture.db;
    db.add_cluster_member("owner", "member@test", None, None)
        .await
        .unwrap();
    db.claim_project_provisioning(
        "request",
        "member",
        "owner",
        "machine-1",
        "fingerprint",
        "github.com/example/repo",
        "https://github.com/example/repo.git",
        "repo",
        "member-work",
        "project-b",
    )
    .await
    .unwrap();

    let finished = while_another_connection_writes(
        &fixture,
        db.finalize_project_provisioning("request", "member"),
    )
    .await
    .expect("finishing waits for the other writer");

    assert_eq!(
        finished.map(|request| request.status).as_deref(),
        Some("completed")
    );
    assert_eq!(audit_rows(db, "project.provisioned").await, 1);
}
