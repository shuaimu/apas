//! A project's recorded open state: whether the project is open or closed as
//! a whole, who set that, how, when, and for an open the machine the request
//! named.
//!
//! The state is five nullable columns on `projects`. All five `NULL` is the
//! third condition, "not recorded": nobody has opened or closed the project
//! since the server began recording, and the server neither starts nor stops
//! it by itself. It is deliberately not part of `lifecycle_status`, whose
//! parser reads an unknown value as `deleting` and whose triggers reject one.
//!
//! Every write goes through [`Database::write_project_open_state`], so the
//! five columns always change together and `open` is never recorded for a
//! project that is not active.

use anyhow::Result;
use shared::{OpenStateSetKind, ProjectOpenState};
use sqlx::Row;
use std::collections::HashMap;
use uuid::Uuid;

use super::{Database, ProjectLifecycle, SYSTEM_ADMIN_ACTOR};

/// The columns `run_migrations` adds to `projects`, each `TEXT` and nullable.
pub(super) const OPEN_STATE_COLUMNS: [&str; 5] = [
    "open_state",
    "open_state_set_by",
    "open_state_set_kind",
    "open_state_set_at",
    "open_state_machine_id",
];

/// What `open_state_set_by` holds for `actor_user_id`: the account's id, or
/// the empty string for the system administrator, who is not an account.
fn open_state_actor(actor_user_id: &str) -> &str {
    if actor_user_id == SYSTEM_ADMIN_ACTOR {
        ""
    } else {
        actor_user_id
    }
}

/// One project's row as the open state reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectOpenStateRecord {
    pub project_id: String,
    pub owner_user_id: String,
    pub lifecycle: ProjectLifecycle,
    /// `None` is "not recorded", never a value of its own.
    pub state: Option<ProjectOpenState>,
    /// The account that set the state; `Some("")` for the system
    /// administrator. `None` while the state is not recorded.
    pub set_by: Option<String>,
    pub set_kind: Option<OpenStateSetKind>,
    /// RFC 3339, UTC.
    pub set_at: Option<String>,
    /// The machine an open named. It stands in as the one tab owner while
    /// the roster has no tab. Always `None` for a closed project.
    pub machine_id: Option<Uuid>,
}

impl ProjectOpenStateRecord {
    /// The account that set the state, when an account did.
    pub fn set_by_account(&self) -> Option<Uuid> {
        self.set_by
            .as_deref()
            .and_then(|actor| Uuid::parse_str(actor).ok())
    }

    /// The machine that counts as owning the project's tabs while no machine
    /// owns one: the machine the open named, for a project recorded open.
    pub fn stand_in_machine(&self) -> Option<Uuid> {
        (self.state == Some(ProjectOpenState::Open))
            .then_some(self.machine_id)
            .flatten()
    }
}

fn state_text(state: ProjectOpenState) -> &'static str {
    match state {
        ProjectOpenState::Open => "open",
        ProjectOpenState::Closed => "closed",
    }
}

fn kind_text(kind: OpenStateSetKind) -> &'static str {
    match kind {
        OpenStateSetKind::Request => "request",
        OpenStateSetKind::Administrative => "administrative",
    }
}

/// A stored state. A value this server does not know reads as not recorded:
/// that is the one answer on which nothing is started and nothing stopped.
fn parse_state(project_id: &str, raw: Option<String>) -> Option<ProjectOpenState> {
    match raw.as_deref() {
        None => None,
        Some("open") => Some(ProjectOpenState::Open),
        Some("closed") => Some(ProjectOpenState::Closed),
        Some(other) => {
            tracing::warn!(%project_id, value = %other, "unknown stored open state; treating the project as not recorded");
            None
        }
    }
}

fn parse_kind(raw: Option<String>) -> Option<OpenStateSetKind> {
    match raw.as_deref() {
        Some("request") => Some(OpenStateSetKind::Request),
        Some("administrative") => Some(OpenStateSetKind::Administrative),
        _ => None,
    }
}

impl Database {
    /// The project's open state with what deciding on it needs, or `None`
    /// when the server holds no record of the project.
    pub async fn get_project_open_state(
        &self,
        project_id: &str,
    ) -> Result<Option<ProjectOpenStateRecord>> {
        let row = sqlx::query(
            r#"SELECT id, owner_user_id, lifecycle_status, open_state, open_state_set_by,
                      open_state_set_kind, open_state_set_at, open_state_machine_id
               FROM projects WHERE id = ?"#,
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| {
            let state = parse_state(project_id, row.get("open_state"));
            let recorded = state.is_some();
            ProjectOpenStateRecord {
                project_id: row.get("id"),
                owner_user_id: row.get("owner_user_id"),
                lifecycle: ProjectLifecycle::parse(&row.get::<String, _>("lifecycle_status")),
                state,
                set_by: row
                    .get::<Option<String>, _>("open_state_set_by")
                    .filter(|_| recorded),
                set_kind: parse_kind(row.get("open_state_set_kind")).filter(|_| recorded),
                set_at: row
                    .get::<Option<String>, _>("open_state_set_at")
                    .filter(|_| recorded),
                machine_id: row
                    .get::<Option<String>, _>("open_state_machine_id")
                    .filter(|_| state == Some(ProjectOpenState::Open))
                    .and_then(|machine| Uuid::parse_str(&machine).ok()),
            }
        }))
    }

    /// The recorded state of every project that has one, by project id. A
    /// project that is not recorded is absent. For lists that show many
    /// projects at once.
    pub async fn list_recorded_project_open_states(
        &self,
    ) -> Result<HashMap<String, ProjectOpenState>> {
        let rows = sqlx::query("SELECT id, open_state FROM projects WHERE open_state IS NOT NULL")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let project_id: String = row.get("id");
                let state = parse_state(&project_id, row.get("open_state"))?;
                Some((project_id, state))
            })
            .collect())
    }

    /// The one statement that writes a project's open state: the five
    /// columns together, and never `open` for a project that is not active.
    /// A closed project keeps no named machine. False when no row changed:
    /// the project does not exist, or it is not active and `state` is open.
    pub(super) async fn write_project_open_state<'e, E>(
        executor: E,
        project_id: &str,
        state: ProjectOpenState,
        set_by: &str,
        kind: OpenStateSetKind,
        machine_id: Option<&Uuid>,
    ) -> Result<bool>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let machine_id = match state {
            ProjectOpenState::Open => machine_id.map(Uuid::to_string),
            ProjectOpenState::Closed => None,
        };
        let set_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        Ok(sqlx::query(
            r#"UPDATE projects
               SET open_state = ?, open_state_set_by = ?, open_state_set_kind = ?,
                   open_state_set_at = ?, open_state_machine_id = ?
               WHERE id = ? AND (? = 'closed' OR lifecycle_status = 'active')"#,
        )
        .bind(state_text(state))
        .bind(open_state_actor(set_by))
        .bind(kind_text(kind))
        .bind(set_at)
        .bind(machine_id)
        .bind(project_id)
        .bind(state_text(state))
        .execute(executor)
        .await?
        .rows_affected()
            > 0)
    }

    /// Record a project's open state: the state, who set it (`set_by` is an
    /// account id, or [`SYSTEM_ADMIN_ACTOR`]), how, now, and for an open the
    /// machine the request named. `Ok(false)` when the server holds no
    /// record of the project. Recording `open` for a suspended project or
    /// one being deleted is an error and records nothing.
    pub async fn set_project_open_state(
        &self,
        project_id: &str,
        state: ProjectOpenState,
        set_by: &str,
        kind: OpenStateSetKind,
        machine_id: Option<&Uuid>,
    ) -> Result<bool> {
        if Self::write_project_open_state(&self.pool, project_id, state, set_by, kind, machine_id)
            .await?
        {
            return Ok(true);
        }
        let exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM projects WHERE id = ?")
            .bind(project_id)
            .fetch_one(&self.pool)
            .await?
            > 0;
        anyhow::ensure!(!exists, "only an active project can be recorded open");
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::User;

    struct Fixture {
        db: Database,
        dir: std::path::PathBuf,
        owner: String,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    async fn fixture() -> Fixture {
        let dir = std::env::temp_dir().join(format!("apas-open-state-db-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::new(&dir.join("apas.db").to_string_lossy())
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let owner = Uuid::new_v4().to_string();
        db.create_user(&User {
            id: owner.clone(),
            email: format!("{owner}@example.test"),
            password_hash: "hash".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .unwrap();
        Fixture { db, dir, owner }
    }

    async fn project(f: &Fixture) -> String {
        let project_id = Uuid::new_v4().to_string();
        f.db.authorize_project_registration(&project_id, &f.owner)
            .await
            .unwrap();
        project_id
    }

    async fn columns(db: &Database) -> Vec<String> {
        sqlx::query("PRAGMA table_info(projects)")
            .fetch_all(&db.pool)
            .await
            .unwrap()
            .iter()
            .map(|column| column.get::<String, _>("name"))
            .collect()
    }

    /// A database from before the open state gains the five columns at the
    /// next start, with every project not recorded and nothing else about
    /// the project changed; a later start changes nothing again.
    #[tokio::test]
    async fn an_existing_database_gains_the_columns_with_every_project_not_recorded() {
        let f = fixture().await;
        let active = project(&f).await;
        let suspended = project(&f).await;
        assert!(f
            .db
            .set_project_lifecycle(&f.owner, &suspended, ProjectLifecycle::Suspended)
            .await
            .unwrap());
        // The table as an older server left it.
        for column in OPEN_STATE_COLUMNS {
            sqlx::query(&format!("ALTER TABLE projects DROP COLUMN {column}"))
                .execute(&f.db.pool)
                .await
                .unwrap();
        }
        let before = columns(&f.db).await;
        assert!(OPEN_STATE_COLUMNS
            .iter()
            .all(|column| !before.iter().any(|name| name == column)));

        for _ in 0..2 {
            f.db.run_migrations().await.unwrap();
            let after = columns(&f.db).await;
            assert_eq!(after.len(), before.len() + OPEN_STATE_COLUMNS.len());
            for column in OPEN_STATE_COLUMNS {
                assert_eq!(
                    after.iter().filter(|name| name == &column).count(),
                    1,
                    "{column}"
                );
            }
        }
        for (project_id, lifecycle) in [
            (&active, ProjectLifecycle::Active),
            (&suspended, ProjectLifecycle::Suspended),
        ] {
            let record =
                f.db.get_project_open_state(project_id)
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(record.state, None);
            assert_eq!(record.set_by, None);
            assert_eq!(record.set_kind, None);
            assert_eq!(record.set_at, None);
            assert_eq!(record.machine_id, None);
            assert_eq!(record.lifecycle, lifecycle);
            assert_eq!(record.owner_user_id, f.owner);
            assert_eq!(
                f.db.get_project(project_id)
                    .await
                    .unwrap()
                    .unwrap()
                    .lifecycle(),
                lifecycle
            );
        }
        assert!(f
            .db
            .list_recorded_project_open_states()
            .await
            .unwrap()
            .is_empty());
    }

    /// The writer records the five values together, a close keeps no named
    /// machine, the administrator is recorded as no account, and the state
    /// never touches the project's administrative state.
    #[tokio::test]
    async fn the_writer_records_the_state_who_how_when_and_the_named_machine() {
        let f = fixture().await;
        let project_id = project(&f).await;
        let machine = Uuid::new_v4();

        assert!(f
            .db
            .set_project_open_state(
                &project_id,
                ProjectOpenState::Open,
                &f.owner,
                OpenStateSetKind::Request,
                Some(&machine),
            )
            .await
            .unwrap());
        let open =
            f.db.get_project_open_state(&project_id)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(open.state, Some(ProjectOpenState::Open));
        assert_eq!(open.set_by.as_deref(), Some(f.owner.as_str()));
        assert_eq!(open.set_by_account(), Uuid::parse_str(&f.owner).ok());
        assert_eq!(open.set_kind, Some(OpenStateSetKind::Request));
        assert_eq!(open.machine_id, Some(machine));
        assert_eq!(open.stand_in_machine(), Some(machine));
        let set_at = open.set_at.clone().expect("a time");
        assert!(chrono::DateTime::parse_from_rfc3339(&set_at).is_ok());
        assert_eq!(open.lifecycle, ProjectLifecycle::Active);
        assert_eq!(
            f.db.list_recorded_project_open_states().await.unwrap(),
            HashMap::from([(project_id.clone(), ProjectOpenState::Open)])
        );

        assert!(f
            .db
            .set_project_open_state(
                &project_id,
                ProjectOpenState::Closed,
                SYSTEM_ADMIN_ACTOR,
                OpenStateSetKind::Administrative,
                Some(&machine),
            )
            .await
            .unwrap());
        let closed =
            f.db.get_project_open_state(&project_id)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(closed.state, Some(ProjectOpenState::Closed));
        assert_eq!(closed.set_by.as_deref(), Some(""));
        assert_eq!(closed.set_by_account(), None);
        assert_eq!(closed.set_kind, Some(OpenStateSetKind::Administrative));
        assert_eq!(closed.machine_id, None, "a close keeps no named machine");
        assert_eq!(closed.stand_in_machine(), None);
        assert_eq!(closed.lifecycle, ProjectLifecycle::Active);

        // A project the server holds no record of.
        assert!(!f
            .db
            .set_project_open_state(
                &Uuid::new_v4().to_string(),
                ProjectOpenState::Closed,
                &f.owner,
                OpenStateSetKind::Request,
                None,
            )
            .await
            .unwrap());
        assert_eq!(
            f.db.get_project_open_state(&Uuid::new_v4().to_string())
                .await
                .unwrap(),
            None
        );
    }

    /// Only an active project is recorded open. A suspended project and one
    /// being deleted refuse it and keep what they had; a close is accepted in
    /// any administrative state.
    #[tokio::test]
    async fn open_is_refused_for_a_project_that_is_not_active() {
        let f = fixture().await;
        let suspended = project(&f).await;
        let deleting = project(&f).await;
        f.db.set_project_lifecycle(&f.owner, &suspended, ProjectLifecycle::Suspended)
            .await
            .unwrap();
        assert!(f
            .db
            .begin_project_deletion(&f.owner, &deleting, &deleting)
            .await
            .unwrap());
        for project_id in [&suspended, &deleting] {
            let before =
                f.db.get_project_open_state(project_id)
                    .await
                    .unwrap()
                    .unwrap();
            assert!(f
                .db
                .set_project_open_state(
                    project_id,
                    ProjectOpenState::Open,
                    &f.owner,
                    OpenStateSetKind::Request,
                    None,
                )
                .await
                .is_err());
            assert_eq!(
                f.db.get_project_open_state(project_id)
                    .await
                    .unwrap()
                    .unwrap(),
                before
            );
            assert!(f
                .db
                .set_project_open_state(
                    project_id,
                    ProjectOpenState::Closed,
                    &f.owner,
                    OpenStateSetKind::Request,
                    None,
                )
                .await
                .unwrap());
        }
    }

    /// Suspending records the project closed by that actor, as an
    /// administrative stop, in the transaction that suspends it.
    /// Reactivating leaves it closed, and a suspension that changes nothing
    /// records nothing.
    #[tokio::test]
    async fn suspending_records_closed_and_reactivating_leaves_it_closed() {
        let f = fixture().await;
        let project_id = project(&f).await;
        let machine = Uuid::new_v4();
        f.db.set_project_open_state(
            &project_id,
            ProjectOpenState::Open,
            &f.owner,
            OpenStateSetKind::Request,
            Some(&machine),
        )
        .await
        .unwrap();

        assert!(f
            .db
            .set_project_lifecycle(SYSTEM_ADMIN_ACTOR, &project_id, ProjectLifecycle::Suspended)
            .await
            .unwrap());
        let suspended =
            f.db.get_project_open_state(&project_id)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(suspended.state, Some(ProjectOpenState::Closed));
        assert_eq!(suspended.set_by.as_deref(), Some(""));
        assert_eq!(suspended.set_kind, Some(OpenStateSetKind::Administrative));
        assert_eq!(suspended.machine_id, None);
        assert_eq!(suspended.lifecycle, ProjectLifecycle::Suspended);

        // Suspending again changes nothing, so it records nothing.
        assert!(!f
            .db
            .set_project_lifecycle(&f.owner, &project_id, ProjectLifecycle::Suspended)
            .await
            .unwrap());
        assert_eq!(
            f.db.get_project_open_state(&project_id)
                .await
                .unwrap()
                .unwrap(),
            suspended
        );

        assert!(f
            .db
            .set_project_lifecycle(&f.owner, &project_id, ProjectLifecycle::Active)
            .await
            .unwrap());
        let reactivated =
            f.db.get_project_open_state(&project_id)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(reactivated.lifecycle, ProjectLifecycle::Active);
        assert_eq!(reactivated.state, Some(ProjectOpenState::Closed));
        assert_eq!(reactivated.set_at, suspended.set_at);
    }

    /// A project nobody recorded is recorded closed when it is suspended,
    /// and beginning a deletion records the project closed by its owner.
    #[tokio::test]
    async fn beginning_a_deletion_records_closed() {
        let f = fixture().await;
        let untouched = project(&f).await;
        f.db.set_project_lifecycle(&f.owner, &untouched, ProjectLifecycle::Suspended)
            .await
            .unwrap();
        assert_eq!(
            f.db.get_project_open_state(&untouched)
                .await
                .unwrap()
                .unwrap()
                .state,
            Some(ProjectOpenState::Closed)
        );

        let project_id = project(&f).await;
        f.db.set_project_open_state(
            &project_id,
            ProjectOpenState::Open,
            &f.owner,
            OpenStateSetKind::Request,
            None,
        )
        .await
        .unwrap();
        assert!(f
            .db
            .begin_project_deletion(&f.owner, &project_id, &project_id)
            .await
            .unwrap());
        let deleting =
            f.db.get_project_open_state(&project_id)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(deleting.lifecycle, ProjectLifecycle::Deleting);
        assert_eq!(deleting.state, Some(ProjectOpenState::Closed));
        assert_eq!(deleting.set_by.as_deref(), Some(f.owner.as_str()));
        assert_eq!(deleting.set_kind, Some(OpenStateSetKind::Administrative));
        // A repeated request finds it deleting and records nothing more.
        assert!(!f
            .db
            .begin_project_deletion(&f.owner, &project_id, &project_id)
            .await
            .unwrap());
        assert_eq!(
            f.db.get_project_open_state(&project_id)
                .await
                .unwrap()
                .unwrap(),
            deleting
        );
    }

    /// A registration that creates the project's row says so, once; one for
    /// a project that already has a row does not.
    #[tokio::test]
    async fn a_registration_reports_whether_it_created_the_project() {
        let f = fixture().await;
        let project_id = Uuid::new_v4().to_string();
        let (_, created) =
            f.db.authorize_project_registration_reporting_creation(&project_id, &f.owner)
                .await
                .unwrap();
        assert!(created);
        let (_, created) =
            f.db.authorize_project_registration_reporting_creation(&project_id, &f.owner)
                .await
                .unwrap();
        assert!(!created);
        assert_eq!(
            f.db.get_project_open_state(&project_id)
                .await
                .unwrap()
                .unwrap()
                .state,
            None,
            "the registration itself records nothing"
        );
    }
}
