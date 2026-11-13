//! Durable worker state shares the bounded-job write reservation. Current context is
//! independent of replay retention; immutable execution links retain cleanup ownership.
use super::{
    jobs::{self, JobTx},
    Database,
};
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use shared::*;
use sqlx::{sqlite::SqliteRow, Row};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct StoredWorker {
    pub detail: WorkerDetail,
    pub schedule_generation: u64,
    pub grant_active: bool,
    pub stop_requested: bool,
    pub accounting_at: String,
}

const SELECT_WORKER: &str = "SELECT w.document,w.schedule_generation,w.grant_active,w.stop_requested,w.accounting_at,w.budget,m.document AS mandate,c.document AS checkpoint FROM autonomous_workers w JOIN autonomous_worker_mandates m ON m.worker_id=w.id AND m.revision=w.mandate_revision LEFT JOIN autonomous_worker_checkpoints c ON c.id=w.checkpoint_id";
const EVENT_RETENTION: u64 = 2000;
const HISTORY_PAGE: usize = 50;

fn decode_worker(row: SqliteRow) -> Result<StoredWorker> {
    Ok(StoredWorker {
        detail: WorkerDetail {
            worker: serde_json::from_str(row.get("document"))?,
            mandate: serde_json::from_str(row.get("mandate"))?,
            checkpoint: row
                .get::<Option<&str>, _>("checkpoint")
                .map(serde_json::from_str)
                .transpose()?,
            budget: serde_json::from_str(row.get("budget"))?,
            allowed_actions: WorkerAllowedActions::default(),
            current_job: None,
        },
        schedule_generation: row.get::<i64, _>("schedule_generation") as u64,
        grant_active: row.get("grant_active"),
        stop_requested: row.get("stop_requested"),
        accounting_at: row.get("accounting_at"),
    })
}

fn timestamp(value: &str) -> Result<i64> {
    Ok(DateTime::parse_from_rfc3339(value)
        .context("invalid_worker_timestamp")?
        .timestamp_millis())
}

fn timestamp_string(value: i64) -> Result<String> {
    Ok(DateTime::<Utc>::from_timestamp_millis(value)
        .context("invalid_worker_timestamp")?
        .to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn enum_name<T: serde::Serialize>(value: T) -> Result<String> {
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_owned)
        .context("invalid_worker_enum")
}

impl Database {
    pub async fn migrate_workers(&self) -> Result<()> {
        let mut tx = self.job_transaction().await?;
        for statement in [
            "CREATE TABLE IF NOT EXISTS autonomous_workers(id TEXT PRIMARY KEY,project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,state TEXT NOT NULL,desired_state TEXT NOT NULL,grant_active INTEGER NOT NULL,stop_requested INTEGER NOT NULL,schedule_generation INTEGER NOT NULL,mandate_revision INTEGER NOT NULL,state_version INTEGER NOT NULL,high_watermark INTEGER NOT NULL,next_check_at INTEGER,active_job_id TEXT,checkpoint_id TEXT,accounting_at TEXT NOT NULL,created_at TEXT NOT NULL,document TEXT NOT NULL,budget TEXT NOT NULL)",
            "CREATE INDEX IF NOT EXISTS autonomous_workers_due ON autonomous_workers(next_check_at,id) WHERE desired_state='active' AND grant_active=1 AND stop_requested=0 AND active_job_id IS NULL AND state IN ('queued','idle','waiting_budget')",
            "CREATE INDEX IF NOT EXISTS autonomous_workers_project ON autonomous_workers(project_id,created_at,id)",
            "CREATE INDEX IF NOT EXISTS autonomous_workers_history ON autonomous_workers(created_at,id)",
            "CREATE INDEX IF NOT EXISTS autonomous_workers_state ON autonomous_workers(state,created_at,id)",
            "CREATE INDEX IF NOT EXISTS autonomous_workers_project_state ON autonomous_workers(project_id,state,created_at,id)",
            "CREATE INDEX IF NOT EXISTS autonomous_workers_attention ON autonomous_workers(created_at,id) WHERE json_extract(document,'$.attention')=1",
            "CREATE INDEX IF NOT EXISTS autonomous_workers_active ON autonomous_workers(active_job_id) WHERE active_job_id IS NOT NULL",
            "CREATE TABLE IF NOT EXISTS autonomous_worker_mandates(worker_id TEXT NOT NULL REFERENCES autonomous_workers(id) ON DELETE CASCADE,revision INTEGER NOT NULL,document TEXT NOT NULL,PRIMARY KEY(worker_id,revision))",
            "CREATE TRIGGER IF NOT EXISTS autonomous_worker_mandate_immutable BEFORE UPDATE ON autonomous_worker_mandates BEGIN SELECT RAISE(ABORT,'immutable_worker_mandate'); END",
            "CREATE TABLE IF NOT EXISTS autonomous_worker_checkpoints(id TEXT PRIMARY KEY,worker_id TEXT NOT NULL REFERENCES autonomous_workers(id) ON DELETE CASCADE,run_id TEXT NOT NULL,document TEXT NOT NULL)",
            "CREATE INDEX IF NOT EXISTS autonomous_worker_checkpoints_worker ON autonomous_worker_checkpoints(worker_id)",
            "CREATE TRIGGER IF NOT EXISTS autonomous_worker_checkpoint_immutable BEFORE UPDATE ON autonomous_worker_checkpoints BEGIN SELECT RAISE(ABORT,'immutable_worker_checkpoint'); END",
            "CREATE TABLE IF NOT EXISTS autonomous_worker_items(id TEXT PRIMARY KEY,worker_id TEXT NOT NULL REFERENCES autonomous_workers(id) ON DELETE CASCADE,revision INTEGER NOT NULL,state TEXT NOT NULL,active INTEGER NOT NULL,priority INTEGER NOT NULL,attempts INTEGER NOT NULL,encoded_bytes INTEGER NOT NULL,created_at TEXT NOT NULL,document TEXT NOT NULL)",
            "CREATE INDEX IF NOT EXISTS autonomous_worker_items_active ON autonomous_worker_items(worker_id,active,priority DESC,id)",
            "CREATE INDEX IF NOT EXISTS autonomous_worker_items_history ON autonomous_worker_items(worker_id,created_at DESC,id DESC)",
            "CREATE INDEX IF NOT EXISTS autonomous_worker_items_status ON autonomous_worker_items(worker_id,state,created_at DESC,id DESC)",
            "CREATE TABLE IF NOT EXISTS autonomous_worker_executions(worker_id TEXT NOT NULL REFERENCES autonomous_workers(id) ON DELETE CASCADE,job_id TEXT PRIMARY KEY REFERENCES autonomous_jobs(id) ON DELETE CASCADE,run_id TEXT UNIQUE NOT NULL REFERENCES autonomous_job_runs(id) ON DELETE CASCADE,reservation_id TEXT UNIQUE NOT NULL,purpose TEXT NOT NULL,item_id TEXT,mandate_revision INTEGER NOT NULL,active INTEGER NOT NULL DEFAULT 1,created_at TEXT NOT NULL)",
            "CREATE UNIQUE INDEX IF NOT EXISTS autonomous_worker_one_execution ON autonomous_worker_executions(worker_id) WHERE active=1",
            "CREATE INDEX IF NOT EXISTS autonomous_worker_execution_history ON autonomous_worker_executions(worker_id,created_at DESC,run_id DESC)",
            "CREATE INDEX IF NOT EXISTS autonomous_worker_execution_item ON autonomous_worker_executions(worker_id,item_id)",
            "CREATE TRIGGER IF NOT EXISTS autonomous_worker_execution_immutable BEFORE UPDATE ON autonomous_worker_executions WHEN OLD.worker_id IS NOT NEW.worker_id OR OLD.job_id IS NOT NEW.job_id OR OLD.run_id IS NOT NEW.run_id OR OLD.reservation_id IS NOT NEW.reservation_id OR OLD.purpose IS NOT NEW.purpose OR OLD.item_id IS NOT NEW.item_id OR OLD.mandate_revision IS NOT NEW.mandate_revision OR OLD.created_at IS NOT NEW.created_at OR (OLD.active=0 AND NEW.active=1) BEGIN SELECT RAISE(ABORT,'immutable_worker_execution'); END",
            "CREATE TABLE IF NOT EXISTS autonomous_worker_reservations(id TEXT PRIMARY KEY,worker_id TEXT NOT NULL REFERENCES autonomous_workers(id) ON DELETE CASCADE,job_id TEXT UNIQUE NOT NULL REFERENCES autonomous_worker_executions(job_id) ON DELETE CASCADE,run_id TEXT UNIQUE NOT NULL,limits TEXT NOT NULL,usage TEXT NOT NULL,stopped INTEGER NOT NULL DEFAULT 0,resolved INTEGER NOT NULL DEFAULT 0,uncertain INTEGER NOT NULL DEFAULT 1,created_at INTEGER NOT NULL)",
            "CREATE INDEX IF NOT EXISTS autonomous_worker_reservations_open ON autonomous_worker_reservations(worker_id,resolved)",
            "CREATE TRIGGER IF NOT EXISTS autonomous_worker_reservation_immutable BEFORE UPDATE ON autonomous_worker_reservations WHEN OLD.id IS NOT NEW.id OR OLD.worker_id IS NOT NEW.worker_id OR OLD.job_id IS NOT NEW.job_id OR OLD.run_id IS NOT NEW.run_id OR OLD.limits IS NOT NEW.limits OR OLD.created_at IS NOT NEW.created_at BEGIN SELECT RAISE(ABORT,'immutable_worker_reservation'); END",
            "CREATE TABLE IF NOT EXISTS autonomous_worker_usage(id INTEGER PRIMARY KEY,worker_id TEXT NOT NULL REFERENCES autonomous_workers(id) ON DELETE CASCADE,reservation_id TEXT NOT NULL REFERENCES autonomous_worker_reservations(id) ON DELETE CASCADE,accounted_at INTEGER NOT NULL,runs INTEGER NOT NULL,document TEXT NOT NULL)",
            "CREATE INDEX IF NOT EXISTS autonomous_worker_usage_window ON autonomous_worker_usage(worker_id,accounted_at,id)",
            "CREATE INDEX IF NOT EXISTS autonomous_worker_usage_reservation ON autonomous_worker_usage(reservation_id,accounted_at)",
            "CREATE TRIGGER IF NOT EXISTS autonomous_worker_usage_immutable BEFORE UPDATE ON autonomous_worker_usage BEGIN SELECT RAISE(ABORT,'immutable_worker_usage'); END",
            "CREATE TABLE IF NOT EXISTS autonomous_worker_events(worker_id TEXT NOT NULL REFERENCES autonomous_workers(id) ON DELETE CASCADE,seq INTEGER NOT NULL,document TEXT NOT NULL,PRIMARY KEY(worker_id,seq))",
            "CREATE TABLE IF NOT EXISTS autonomous_worker_mutations(actor TEXT NOT NULL,request_id TEXT NOT NULL,project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,fingerprint TEXT NOT NULL,document TEXT NOT NULL,PRIMARY KEY(actor,request_id))",
            "CREATE TABLE IF NOT EXISTS autonomous_worker_previews(id TEXT PRIMARY KEY,actor TEXT NOT NULL,project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,expires_at TEXT NOT NULL,document TEXT NOT NULL)",
            "CREATE INDEX IF NOT EXISTS autonomous_worker_previews_expiry ON autonomous_worker_previews(expires_at)",
            "CREATE TABLE IF NOT EXISTS autonomous_worker_results(run_id TEXT PRIMARY KEY REFERENCES autonomous_job_runs(id) ON DELETE CASCADE,worker_id TEXT NOT NULL REFERENCES autonomous_workers(id) ON DELETE CASCADE,document TEXT NOT NULL,applied INTEGER NOT NULL)",
            "CREATE TRIGGER IF NOT EXISTS autonomous_worker_result_immutable BEFORE UPDATE ON autonomous_worker_results WHEN OLD.run_id IS NOT NEW.run_id OR OLD.worker_id IS NOT NEW.worker_id OR OLD.document IS NOT NEW.document BEGIN SELECT RAISE(ABORT,'immutable_worker_result'); END",
            "CREATE TABLE IF NOT EXISTS autonomous_worker_notification_pending(worker_id TEXT NOT NULL REFERENCES autonomous_workers(id) ON DELETE CASCADE,seq INTEGER NOT NULL,document TEXT NOT NULL,PRIMARY KEY(worker_id,seq))",
            "INSERT OR IGNORE INTO schema_migrations(name) VALUES('autonomous_workers_v1')",
        ] {
            sqlx::query(statement).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn worker(&self, id: Uuid) -> Result<Option<StoredWorker>> {
        sqlx::query(&format!("{SELECT_WORKER} WHERE w.id=?"))
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .map(decode_worker)
            .transpose()
    }

    /// Lifecycle/authority inventory intentionally has no presentation limit.
    pub async fn all_workers(&self) -> Result<Vec<StoredWorker>> {
        sqlx::query(&format!("{SELECT_WORKER} ORDER BY w.created_at,w.id"))
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(decode_worker)
            .collect()
    }

    /// Apply project and presentation filters before the bounded keyset page.
    /// Global callers must still authorize each row before returning its content.
    pub async fn worker_page(
        &self,
        project: Option<&str>,
        cursor: Option<&str>,
        status: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredWorker>> {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(SELECT_WORKER);
        query.push(" WHERE 1=1");
        if let Some(project) = project {
            query.push(" AND w.project_id=").push_bind(project);
        }
        if let Some(status) = status {
            if status == "attention" {
                query.push(" AND json_extract(w.document,'$.attention')=1");
            } else {
                serde_json::from_value::<WorkerState>(serde_json::Value::String(status.to_owned()))
                    .context("invalid_worker_status")?;
                query.push(" AND w.state=").push_bind(status);
            }
        }
        if let Some(cursor) = cursor {
            let (created, id) = cursor.rsplit_once('|').context("invalid_cursor")?;
            timestamp(created).context("invalid_cursor")?;
            Uuid::parse_str(id).context("invalid_cursor")?;
            query
                .push(" AND (w.created_at,w.id)<(")
                .push_bind(created)
                .push(",")
                .push_bind(id)
                .push(")");
        }
        query
            .push(" ORDER BY w.created_at DESC,w.id DESC LIMIT ")
            .push_bind(limit.clamp(1, 256) as i64);
        query
            .build()
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(decode_worker)
            .collect()
    }

    pub async fn due_workers(&self, now: &str, limit: usize) -> Result<Vec<StoredWorker>> {
        sqlx::query(&format!("{SELECT_WORKER} WHERE w.desired_state='active' AND w.grant_active=1 AND w.stop_requested=0 AND w.active_job_id IS NULL AND w.state IN ('queued','idle','waiting_budget') AND w.next_check_at<=? AND NOT EXISTS(SELECT 1 FROM autonomous_worker_executions e WHERE e.worker_id=w.id AND e.active=1) ORDER BY w.next_check_at,w.id LIMIT ?"))
            .bind(timestamp(now)?).bind(limit.clamp(1, 256) as i64)
            .fetch_all(&self.pool).await?.into_iter().map(decode_worker).collect()
    }

    pub async fn worker_preview(&self, actor: Uuid, id: Uuid) -> Result<Option<WorkerPreview>> {
        let document: Option<String> = sqlx::query_scalar(
            "SELECT document FROM autonomous_worker_previews WHERE id=? AND actor=?",
        )
        .bind(id.to_string())
        .bind(actor.to_string())
        .fetch_optional(&self.pool)
        .await?;
        document
            .map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    }

    pub async fn store_worker_preview(&self, actor: Uuid, preview: &WorkerPreview) -> Result<()> {
        sqlx::query("INSERT INTO autonomous_worker_previews(id,actor,project_id,expires_at,document) VALUES(?,?,?,?,?)")
            .bind(preview.id.to_string()).bind(actor.to_string()).bind(&preview.project_id)
            .bind(&preview.expires_at).bind(serde_json::to_string(preview)?).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn worker_receipt(
        &self,
        actor: Uuid,
        request: Uuid,
    ) -> Result<Option<WorkerMutationReceipt>> {
        let document: Option<String> = sqlx::query_scalar(
            "SELECT document FROM autonomous_worker_mutations WHERE actor=? AND request_id=?",
        )
        .bind(actor.to_string())
        .bind(request.to_string())
        .fetch_optional(&self.pool)
        .await?;
        document
            .map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    }

    pub async fn worker_events(
        &self,
        id: Uuid,
        after: u64,
        limit: usize,
    ) -> Result<WorkerEventsResponse> {
        let mut tx = self.pool.begin().await?;
        let stored = load(&mut tx, id).await?;
        let high_watermark = stored.detail.worker.high_watermark;
        let first: Option<i64> =
            sqlx::query_scalar("SELECT MIN(seq) FROM autonomous_worker_events WHERE worker_id=?")
                .bind(id.to_string())
                .fetch_one(&mut *tx)
                .await?;
        let limit = limit.clamp(1, 200);
        let rows = sqlx::query("SELECT document FROM autonomous_worker_events WHERE worker_id=? AND seq>? ORDER BY seq LIMIT ?")
            .bind(id.to_string()).bind(i64::try_from(after).unwrap_or(i64::MAX)).bind((limit + 1) as i64)
            .fetch_all(&mut *tx).await?;
        let mut events: Vec<WorkerEvent> = rows
            .into_iter()
            .map(|r| serde_json::from_str(r.get("document")))
            .collect::<Result<_, _>>()?;
        let has_more = events.len() > limit;
        events.truncate(limit);
        let resync_required = after > high_watermark
            || first
                .map(|n| after.saturating_add(1) < n as u64)
                .unwrap_or(after < high_watermark)
            || events
                .first()
                .is_some_and(|e| e.seq != after.saturating_add(1))
            || events.windows(2).any(|p| p[1].seq != p[0].seq + 1)
            || (!has_more && events.last().map_or(after, |event| event.seq) < high_watermark);
        tx.commit().await?;
        Ok(WorkerEventsResponse {
            events,
            high_watermark,
            has_more,
            resync_required,
        })
    }

    pub async fn pending_worker_notifications(&self, limit: usize) -> Result<Vec<WorkerEvent>> {
        sqlx::query(
            "SELECT document FROM autonomous_worker_notification_pending ORDER BY rowid LIMIT ?",
        )
        .bind(limit.clamp(1, 100) as i64)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| serde_json::from_str(row.get("document")).map_err(Into::into))
        .collect()
    }

    pub async fn acknowledge_worker_notification(&self, worker: Uuid, seq: u64) -> Result<()> {
        sqlx::query(
            "DELETE FROM autonomous_worker_notification_pending WHERE worker_id=? AND seq=?",
        )
        .bind(worker.to_string())
        .bind(i64::try_from(seq)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn worker_runs(
        &self,
        worker: Uuid,
        cursor: Option<&str>,
    ) -> Result<WorkerRunsResponse> {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT e.job_id,e.item_id,e.purpose,e.mandate_revision,e.created_at,e.run_id,r.document FROM autonomous_worker_executions e JOIN autonomous_job_runs r ON r.id=e.run_id WHERE e.worker_id=");
        query.push_bind(worker.to_string());
        if let Some(cursor) = cursor {
            let (created, id) = cursor.rsplit_once('|').context("invalid_cursor")?;
            Uuid::parse_str(id).context("invalid_cursor")?;
            query
                .push(" AND (e.created_at,e.run_id)<(")
                .push_bind(created)
                .push(",")
                .push_bind(id)
                .push(")");
        }
        query.push(" ORDER BY e.created_at DESC,e.run_id DESC LIMIT 51");
        let rows = query.build().fetch_all(&self.pool).await?;
        let next_cursor = (rows.len() > HISTORY_PAGE).then(|| {
            let row = &rows[HISTORY_PAGE - 1];
            format!(
                "{}|{}",
                row.get::<&str, _>("created_at"),
                row.get::<&str, _>("run_id")
            )
        });
        let runs = rows
            .into_iter()
            .take(HISTORY_PAGE)
            .map(|row| {
                Ok(WorkerRunSummary {
                    worker_id: worker,
                    item_id: row
                        .get::<Option<&str>, _>("item_id")
                        .map(Uuid::parse_str)
                        .transpose()?,
                    job_id: Uuid::parse_str(row.get("job_id"))?,
                    purpose: serde_json::from_value(serde_json::Value::String(row.get("purpose")))?,
                    mandate_revision: row.get::<i64, _>("mandate_revision") as u64,
                    run: serde_json::from_str(row.get("document"))?,
                })
            })
            .collect::<Result<_>>()?;
        Ok(WorkerRunsResponse { runs, next_cursor })
    }

    pub async fn worker_run(&self, worker: Uuid, run: Uuid) -> Result<Option<Uuid>> {
        let job: Option<String> = sqlx::query_scalar(
            "SELECT job_id FROM autonomous_worker_executions WHERE worker_id=? AND run_id=?",
        )
        .bind(worker.to_string())
        .bind(run.to_string())
        .fetch_optional(&self.pool)
        .await?;
        job.map(|id| Uuid::parse_str(&id).map_err(Into::into))
            .transpose()
    }

    pub async fn worker_items(
        &self,
        worker: Uuid,
        cursor: Option<&str>,
        status: Option<&str>,
    ) -> Result<WorkerItemsResponse> {
        let mut tx = self.pool.begin().await?;
        let stored = load(&mut tx, worker).await?;
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT document,created_at,id FROM autonomous_worker_items WHERE worker_id=",
        );
        query.push_bind(worker.to_string());
        if let Some(status) = status {
            if status == "active" {
                query.push(" AND active=1");
            } else {
                serde_json::from_value::<WorkerItemState>(serde_json::Value::String(
                    status.to_owned(),
                ))
                .context("invalid_item_status")?;
                query.push(" AND state=").push_bind(status);
            }
        }
        if let Some(cursor) = cursor {
            let (created, id) = cursor.rsplit_once('|').context("invalid_cursor")?;
            Uuid::parse_str(id).context("invalid_cursor")?;
            query
                .push(" AND (created_at,id)<(")
                .push_bind(created)
                .push(",")
                .push_bind(id)
                .push(")");
        }
        query.push(" ORDER BY created_at DESC,id DESC LIMIT 51");
        let rows = query.build().fetch_all(&mut *tx).await?;
        let next_cursor = (rows.len() > HISTORY_PAGE).then(|| {
            let row = &rows[HISTORY_PAGE - 1];
            format!(
                "{}|{}",
                row.get::<&str, _>("created_at"),
                row.get::<&str, _>("id")
            )
        });
        let items = rows
            .into_iter()
            .take(HISTORY_PAGE)
            .map(|r| serde_json::from_str(r.get("document")))
            .collect::<Result<_, _>>()?;
        tx.commit().await?;
        Ok(WorkerItemsResponse {
            items,
            backlog_revision: stored.detail.worker.backlog_revision,
            next_cursor,
        })
    }
}

pub async fn load(tx: &mut JobTx<'_>, id: Uuid) -> Result<StoredWorker> {
    decode_worker(
        sqlx::query(&format!("{SELECT_WORKER} WHERE w.id=?"))
            .bind(id.to_string())
            .fetch_optional(&mut **tx)
            .await?
            .context("worker_not_found")?,
    )
}

pub async fn save(tx: &mut JobTx<'_>, value: &StoredWorker) -> Result<()> {
    let w = &value.detail.worker;
    ensure!(
        w.mandate_revision == value.detail.mandate.revision,
        "mandate_revision_conflict"
    );
    let document = serde_json::to_string(w)?;
    if let Some(old) = sqlx::query("SELECT document,state_version,high_watermark,schedule_generation,accounting_at FROM autonomous_workers WHERE id=?")
        .bind(w.id.to_string()).fetch_optional(&mut **tx).await? {
        let previous: WorkerSummary = serde_json::from_str(old.get("document"))?;
        ensure!(previous.project_id == w.project_id && previous.created_at == w.created_at, "immutable_worker_identity");
        ensure!(w.mandate_revision >= previous.mandate_revision
            && w.mandate_revision <= previous.mandate_revision.saturating_add(1)
            && w.backlog_revision >= previous.backlog_revision, "mandate_revision_conflict");
        let version = old.get::<i64, _>("state_version") as u64;
        ensure!(w.state_version == version && old.get::<&str, _>("document") == document
            || w.state_version == version.checked_add(1).context("worker_version_exhausted")?, "version_conflict");
        ensure!(w.high_watermark >= old.get::<i64, _>("high_watermark") as u64
            && value.schedule_generation >= old.get::<i64, _>("schedule_generation") as u64, "version_conflict");
        ensure!(timestamp(&value.accounting_at)? >= timestamp(old.get("accounting_at"))?, "accounting_clock_regression");
    }
    let mandate = serde_json::to_string(&value.detail.mandate)?;
    let previous: Option<String> = sqlx::query_scalar(
        "SELECT document FROM autonomous_worker_mandates WHERE worker_id=? AND revision=?",
    )
    .bind(w.id.to_string())
    .bind(i64::try_from(w.mandate_revision)?)
    .fetch_optional(&mut **tx)
    .await?;
    ensure!(
        previous.as_ref().is_none_or(|old| old == &mandate),
        "immutable_worker_mandate"
    );
    sqlx::query("INSERT INTO autonomous_workers(id,project_id,state,desired_state,grant_active,stop_requested,schedule_generation,mandate_revision,state_version,high_watermark,next_check_at,active_job_id,checkpoint_id,accounting_at,created_at,document,budget) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET state=excluded.state,desired_state=excluded.desired_state,grant_active=excluded.grant_active,stop_requested=excluded.stop_requested,schedule_generation=excluded.schedule_generation,mandate_revision=excluded.mandate_revision,state_version=excluded.state_version,high_watermark=excluded.high_watermark,next_check_at=excluded.next_check_at,active_job_id=excluded.active_job_id,checkpoint_id=excluded.checkpoint_id,accounting_at=excluded.accounting_at,document=excluded.document,budget=excluded.budget")
        .bind(w.id.to_string()).bind(&w.project_id).bind(enum_name(w.state)?).bind(enum_name(w.desired_state)?)
        .bind(value.grant_active).bind(value.stop_requested).bind(i64::try_from(value.schedule_generation)?)
        .bind(i64::try_from(w.mandate_revision)?).bind(i64::try_from(w.state_version)?).bind(i64::try_from(w.high_watermark)?)
        .bind(w.next_check_at.as_deref().map(timestamp).transpose()?).bind(w.active_job_id.map(|id| id.to_string()))
        .bind(value.detail.checkpoint.as_ref().map(|c| c.id.to_string())).bind(&value.accounting_at).bind(&w.created_at)
        .bind(document).bind(serde_json::to_string(&value.detail.budget)?).execute(&mut **tx).await?;
    if previous.is_none() {
        sqlx::query(
            "INSERT INTO autonomous_worker_mandates(worker_id,revision,document) VALUES(?,?,?)",
        )
        .bind(w.id.to_string())
        .bind(i64::try_from(w.mandate_revision)?)
        .bind(mandate)
        .execute(&mut **tx)
        .await?;
    }
    if let Some(checkpoint) = &value.detail.checkpoint {
        ensure!(checkpoint.worker_id == w.id, "checkpoint_identity_conflict");
        let document = serde_json::to_string(checkpoint)?;
        ensure!(
            document.len() <= WORKER_MAX_RESULT_BYTES,
            "checkpoint_too_large"
        );
        let previous: Option<String> =
            sqlx::query_scalar("SELECT document FROM autonomous_worker_checkpoints WHERE id=?")
                .bind(checkpoint.id.to_string())
                .fetch_optional(&mut **tx)
                .await?;
        if let Some(previous) = previous {
            ensure!(previous == document, "immutable_worker_checkpoint");
        } else {
            sqlx::query("INSERT INTO autonomous_worker_checkpoints(id,worker_id,run_id,document) VALUES(?,?,?,?)")
                .bind(checkpoint.id.to_string()).bind(w.id.to_string()).bind(checkpoint.run_id.to_string()).bind(document)
                .execute(&mut **tx).await?;
        }
    }
    Ok(())
}

pub async fn append(
    tx: &mut JobTx<'_>,
    worker: &mut StoredWorker,
    kind: WorkerEventKind,
    summary: &str,
) -> Result<WorkerEvent> {
    ensure!(summary.len() <= 8192, "event_too_large");
    let w = &mut worker.detail.worker;
    let notify = if w.active_job_id.is_none()
        && matches!(
            w.state,
            WorkerState::Blocked | WorkerState::RecoveryRequired
        )
        && matches!(
            kind,
            WorkerEventKind::StateChanged | WorkerEventKind::Review
        ) {
        let previous: Option<String> =
            sqlx::query_scalar("SELECT document FROM autonomous_workers WHERE id=?")
                .bind(w.id.to_string())
                .fetch_optional(&mut **tx)
                .await?;
        previous
            .map(|document| serde_json::from_str::<WorkerSummary>(&document))
            .transpose()?
            .is_none_or(|previous| previous.state != w.state || previous.reason != w.reason)
    } else {
        false
    };
    w.high_watermark = w
        .high_watermark
        .checked_add(1)
        .context("worker_version_exhausted")?;
    w.state_version = w
        .state_version
        .checked_add(1)
        .context("worker_version_exhausted")?;
    w.updated_at = Utc::now().to_rfc3339();
    let event = WorkerEvent {
        worker_id: w.id,
        project_id: w.project_id.clone(),
        seq: w.high_watermark,
        state_version: w.state_version,
        kind,
        summary: summary.to_owned(),
        job_id: w.active_job_id,
        run_id: w.active_run_id,
        question_id: None,
        created_at: w.updated_at.clone(),
    };
    save(tx, worker).await?;
    sqlx::query("INSERT INTO autonomous_worker_events(worker_id,seq,document) VALUES(?,?,?)")
        .bind(event.worker_id.to_string())
        .bind(i64::try_from(event.seq)?)
        .bind(serde_json::to_string(&event)?)
        .execute(&mut **tx)
        .await?;
    if notify {
        sqlx::query("INSERT INTO autonomous_worker_notification_pending(worker_id,seq,document) VALUES(?,?,?)")
            .bind(event.worker_id.to_string()).bind(i64::try_from(event.seq)?).bind(serde_json::to_string(&event)?)
            .execute(&mut **tx).await?;
    }
    sqlx::query("DELETE FROM autonomous_worker_events WHERE worker_id=? AND seq<=?")
        .bind(event.worker_id.to_string())
        .bind(event.seq.saturating_sub(EVENT_RETENTION) as i64)
        .execute(&mut **tx)
        .await?;
    Ok(event)
}

pub async fn receipt(
    tx: &mut JobTx<'_>,
    actor: Uuid,
    request: Uuid,
    fingerprint: &str,
) -> Result<Option<WorkerMutationReceipt>> {
    sqlx::query("SELECT fingerprint,document FROM autonomous_worker_mutations WHERE actor=? AND request_id=?")
        .bind(actor.to_string()).bind(request.to_string()).fetch_optional(&mut **tx).await?
        .map(|row| {
            ensure!(row.get::<&str, _>("fingerprint") == fingerprint, "request_conflict");
            Ok(serde_json::from_str(row.get("document"))?)
        }).transpose()
}

pub async fn save_receipt(
    tx: &mut JobTx<'_>,
    actor: Uuid,
    fingerprint: &str,
    value: &WorkerMutationReceipt,
) -> Result<()> {
    if let Some(previous) = receipt(tx, actor, value.request_id, fingerprint).await? {
        ensure!(
            serde_json::to_value(previous)? == serde_json::to_value(value)?,
            "request_conflict"
        );
        return Ok(());
    }
    sqlx::query("INSERT INTO autonomous_worker_mutations(actor,request_id,project_id,fingerprint,document) VALUES(?,?,?,?,?)")
        .bind(actor.to_string()).bind(value.request_id.to_string()).bind(&value.project_id)
        .bind(fingerprint).bind(serde_json::to_string(value)?).execute(&mut **tx).await?;
    Ok(())
}

/// Complete bounded active context, never a truncated history page.
pub async fn items(tx: &mut JobTx<'_>, worker: Uuid) -> Result<Vec<WorkerItem>> {
    sqlx::query("SELECT document FROM autonomous_worker_items WHERE worker_id=? AND active=1 ORDER BY priority DESC,id")
        .bind(worker.to_string()).fetch_all(&mut **tx).await?.into_iter()
        .map(|row| serde_json::from_str(row.get("document")).map_err(Into::into)).collect()
}

pub async fn save_item(tx: &mut JobTx<'_>, value: &WorkerItem) -> Result<()> {
    ensure!(
        value.item.is_valid() && !value.id.is_nil(),
        "invalid_worker_item"
    );
    let document = serde_json::to_string(value)?;
    let active = !matches!(
        value.state,
        WorkerItemState::Done | WorkerItemState::Dismissed
    );
    if let Some(row) = sqlx::query("SELECT document FROM autonomous_worker_items WHERE id=?")
        .bind(value.id.to_string())
        .fetch_optional(&mut **tx)
        .await?
    {
        let old: WorkerItem = serde_json::from_str(row.get("document"))?;
        ensure!(
            old.worker_id == value.worker_id
                && old.origin == value.origin
                && old.created_at == value.created_at,
            "immutable_item_identity"
        );
        ensure!(value.attempts >= old.attempts, "item_attempt_regression");
        ensure!(
            value.revision >= old.revision && value.revision <= old.revision.saturating_add(1),
            "item_revision_conflict"
        );
        if old.item != value.item {
            ensure!(
                value.revision == old.revision.saturating_add(1),
                "item_revision_conflict"
            );
            ensure!(
                !matches!(
                    old.state,
                    WorkerItemState::Selected
                        | WorkerItemState::Running
                        | WorkerItemState::AwaitingReview
                ),
                "item_claimed"
            );
        }
    } else {
        ensure!(value.revision == 1, "item_revision_conflict");
    }
    if active {
        let row = sqlx::query("SELECT COUNT(*) AS count,COALESCE(SUM(encoded_bytes),0) AS bytes FROM autonomous_worker_items WHERE worker_id=? AND active=1 AND id<>?")
            .bind(value.worker_id.to_string()).bind(value.id.to_string()).fetch_one(&mut **tx).await?;
        ensure!(
            (row.get::<i64, _>("count") as usize) < WORKER_MAX_BACKLOG_ITEMS,
            "worker_backlog_full"
        );
        ensure!(
            (row.get::<i64, _>("bytes") as usize).saturating_add(document.len())
                <= WORKER_MAX_CONTEXT_BYTES,
            "worker_backlog_too_large"
        );
    }
    sqlx::query("INSERT INTO autonomous_worker_items(id,worker_id,revision,state,active,priority,attempts,encoded_bytes,created_at,document) VALUES(?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,state=excluded.state,active=excluded.active,priority=excluded.priority,attempts=excluded.attempts,encoded_bytes=excluded.encoded_bytes,document=excluded.document")
        .bind(value.id.to_string()).bind(value.worker_id.to_string()).bind(i64::try_from(value.revision)?)
        .bind(enum_name(value.state)?).bind(active).bind(value.item.priority).bind(i64::from(value.attempts))
        .bind(document.len() as i64).bind(&value.created_at).bind(document).execute(&mut **tx).await?;
    Ok(())
}

/// Advance the durable accounting clock even for a duplicate report. A backward
/// wall clock can delay replenishment, never make already charged usage disappear.
async fn accounting_time(tx: &mut JobTx<'_>, worker: &StoredWorker, now: &str) -> Result<i64> {
    let persisted: String =
        sqlx::query_scalar("SELECT accounting_at FROM autonomous_workers WHERE id=?")
            .bind(worker.detail.worker.id.to_string())
            .fetch_one(&mut **tx)
            .await?;
    Ok(timestamp(now)?
        .max(timestamp(&worker.accounting_at)?)
        .max(timestamp(&persisted)?))
}

async fn advance_clock(tx: &mut JobTx<'_>, worker: &mut StoredWorker, at: i64) -> Result<()> {
    worker.accounting_at = timestamp_string(at)?;
    sqlx::query("UPDATE autonomous_workers SET accounting_at=? WHERE id=?")
        .bind(&worker.accounting_at)
        .bind(worker.detail.worker.id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub async fn reserve(
    tx: &mut JobTx<'_>,
    worker: &mut StoredWorker,
    reservation: Uuid,
    job: Uuid,
    run: Uuid,
    limits: &JobLimits,
    now: &str,
) -> Result<()> {
    ensure!(
        limits.fits_within(&worker.detail.mandate.spec.limits.per_run) && limits.attempts == 1,
        "worker_run_limits_exceeded"
    );
    let stored = jobs::load(tx, job).await?;
    let context = stored
        .detail
        .task
        .worker
        .as_ref()
        .context("worker_context_missing")?;
    ensure!(
        context.worker_id == worker.detail.worker.id
            && context.reservation_id == reservation
            && context.mandate_revision == worker.detail.mandate.revision
            && context.backlog_revision == worker.detail.worker.backlog_revision
            && context.schedule_generation == worker.schedule_generation
            && context.mandate == worker.detail.mandate.spec
            && stored.detail.job.project_id == worker.detail.worker.project_id
            && stored.detail.job.current_run_id == run
            && stored.detail.job.limits == *limits
            && stored.detail.runs.iter().any(|r| r.id == run),
        "worker_execution_context_conflict"
    );
    if let Some(row) = sqlx::query(
        "SELECT worker_id,job_id,run_id,limits FROM autonomous_worker_reservations WHERE id=?",
    )
    .bind(reservation.to_string())
    .fetch_optional(&mut **tx)
    .await?
    {
        ensure!(
            row.get::<&str, _>("worker_id") == worker.detail.worker.id.to_string()
                && row.get::<&str, _>("job_id") == job.to_string()
                && row.get::<&str, _>("run_id") == run.to_string()
                && serde_json::from_str::<JobLimits>(row.get("limits"))? == *limits,
            "reservation_conflict"
        );
        return Ok(());
    }
    let persisted = load(tx, worker.detail.worker.id).await?;
    ensure!(
        persisted.schedule_generation == worker.schedule_generation
            && persisted.detail.worker.state_version == worker.detail.worker.state_version
            && persisted.detail.worker.mandate_revision == context.mandate_revision
            && persisted.detail.worker.backlog_revision == context.backlog_revision,
        "worker_execution_context_conflict"
    );
    ensure!(
        persisted.grant_active
            && !persisted.stop_requested
            && persisted.detail.worker.desired_state == WorkerDesiredState::Active
            && persisted.detail.worker.active_job_id.is_none()
            && matches!(
                persisted.detail.worker.state,
                WorkerState::Queued | WorkerState::Idle | WorkerState::WaitingBudget
            ),
        "worker_not_active"
    );
    ensure!(
        matches!(
            (context.purpose, context.selected_item_id),
            (WorkerRunPurpose::Inspection, None) | (WorkerRunPurpose::Implementation, Some(_))
        ),
        "worker_execution_context_conflict"
    );
    ensure!(
        worker.grant_active
            && !worker.stop_requested
            && worker.detail.worker.desired_state == WorkerDesiredState::Active,
        "worker_not_active"
    );
    if let Some(item) = context.selected_item_id {
        let document: String = sqlx::query_scalar(
            "SELECT document FROM autonomous_worker_items WHERE worker_id=? AND id=?",
        )
        .bind(context.worker_id.to_string())
        .bind(item.to_string())
        .fetch_optional(&mut **tx)
        .await?
        .context("item_not_found")?;
        let selected: WorkerItem = serde_json::from_str(&document)?;
        let admitted = context
            .items
            .iter()
            .find(|i| i.id == item)
            .context("item_context_missing")?;
        ensure!(
            selected.revision == admitted.revision && selected.item == admitted.item,
            "item_revision_conflict"
        );
        let previous_attempts: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM autonomous_worker_executions WHERE worker_id=? AND item_id=?",
        )
        .bind(context.worker_id.to_string())
        .bind(item.to_string())
        .fetch_one(&mut **tx)
        .await?;
        ensure!(
            i64::from(selected.attempts) == previous_attempts.saturating_add(1)
                && selected.attempts <= worker.detail.mandate.spec.limits.attempts_per_item,
            "item_attempts_exhausted"
        );
    }
    let at = accounting_time(tx, worker, now).await?;
    let available = budget(tx, worker, now).await?;
    ensure!(
        !available.accounting_uncertain,
        "worker_accounting_uncertain"
    );
    let bound = &worker.detail.mandate.spec.limits;
    ensure!(
        available
            .usage
            .elapsed_seconds
            .saturating_add(available.reserved_elapsed_seconds)
            .saturating_add(limits.elapsed_seconds)
            <= bound.elapsed_seconds_24h
            && available
                .usage
                .spend_micro_usd
                .unwrap_or(0)
                .saturating_add(available.reserved_spend_micro_usd)
                .saturating_add(limits.spend_micro_usd)
                <= bound.spend_micro_usd_24h
            && available.runs.saturating_add(available.reserved_runs) < bound.runs_24h,
        "worker_budget_exhausted"
    );
    sqlx::query("INSERT INTO autonomous_worker_executions(worker_id,job_id,run_id,reservation_id,purpose,item_id,mandate_revision,active,created_at) VALUES(?,?,?,?,?,?,?,1,?)")
        .bind(context.worker_id.to_string()).bind(job.to_string()).bind(run.to_string()).bind(reservation.to_string())
        .bind(enum_name(context.purpose)?).bind(context.selected_item_id.map(|id| id.to_string()))
        .bind(i64::try_from(context.mandate_revision)?).bind(timestamp_string(at)?).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO autonomous_worker_reservations(id,worker_id,job_id,run_id,limits,usage,created_at) VALUES(?,?,?,?,?,?,?)")
        .bind(reservation.to_string()).bind(context.worker_id.to_string()).bind(job.to_string()).bind(run.to_string())
        .bind(serde_json::to_string(limits)?).bind(serde_json::to_string(&JobUsage::default())?).bind(at).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO autonomous_worker_usage(worker_id,reservation_id,accounted_at,runs,document) VALUES(?,?,?,1,?)")
        .bind(context.worker_id.to_string()).bind(reservation.to_string()).bind(at)
        .bind(serde_json::to_string(&JobUsage { spend_micro_usd: Some(0), ..JobUsage::default() })?).execute(&mut **tx).await?;
    advance_clock(tx, worker, at).await?;
    worker.detail.budget = budget(tx, worker, now).await?;
    Ok(())
}

/// Cumulative host usage includes root and children. Only positive deltas are
/// charged, at reconciliation time; repeated or reordered reports cannot refund.
pub async fn reconcile_usage(
    tx: &mut JobTx<'_>,
    worker: &mut StoredWorker,
    reservation: Uuid,
    usage: &JobUsage,
    stopped: bool,
    now: &str,
) -> Result<()> {
    let row = sqlx::query("SELECT usage,stopped,uncertain,resolved FROM autonomous_worker_reservations WHERE id=? AND worker_id=?")
        .bind(reservation.to_string()).bind(worker.detail.worker.id.to_string()).fetch_optional(&mut **tx).await?.context("reservation_not_found")?;
    let old: JobUsage = serde_json::from_str(row.get("usage"))?;
    let merged = JobUsage {
        elapsed_seconds: old.elapsed_seconds.max(usage.elapsed_seconds),
        input_tokens: old.input_tokens.max(usage.input_tokens),
        output_tokens: old.output_tokens.max(usage.output_tokens),
        spend_micro_usd: match (old.spend_micro_usd, usage.spend_micro_usd) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        },
        estimated: old.estimated || usage.estimated,
    };
    let dominates = usage.elapsed_seconds >= old.elapsed_seconds
        && usage.input_tokens >= old.input_tokens
        && usage.output_tokens >= old.output_tokens
        && usage.spend_micro_usd.is_some()
        && usage.spend_micro_usd.unwrap_or(0) >= old.spend_micro_usd.unwrap_or(0);
    let uncertain = if dominates {
        false
    } else {
        row.get::<bool, _>("uncertain") || usage.spend_micro_usd.is_none()
    };
    let stopped = stopped || row.get::<bool, _>("stopped");
    let mut delta = JobUsage {
        elapsed_seconds: merged.elapsed_seconds - old.elapsed_seconds,
        input_tokens: merged.input_tokens - old.input_tokens,
        output_tokens: merged.output_tokens - old.output_tokens,
        spend_micro_usd: Some(
            merged
                .spend_micro_usd
                .unwrap_or(0)
                .saturating_sub(old.spend_micro_usd.unwrap_or(0)),
        ),
        estimated: merged.estimated,
    };
    let at = accounting_time(tx, worker, now).await?;
    let mut runs = 0;
    if stopped && !uncertain && !row.get::<bool, _>("resolved") {
        // Until final stop/accounting the entire obligation is held. If it
        // crossed a window boundary, charge its aged portion at reconciliation
        // rather than releasing that obligation into an immediately empty window.
        let window = at.saturating_sub((WORKER_BUDGET_WINDOW_SECONDS * 1000) as i64);
        let rows = sqlx::query("SELECT document,runs FROM autonomous_worker_usage WHERE reservation_id=? AND accounted_at>?")
            .bind(reservation.to_string()).bind(window).fetch_all(&mut **tx).await?;
        let mut in_window = JobUsage {
            spend_micro_usd: Some(0),
            ..JobUsage::default()
        };
        let mut charged_runs = 0u32;
        for row in rows {
            add_usage(
                &mut in_window,
                &serde_json::from_str::<JobUsage>(row.get("document"))?,
            );
            charged_runs = charged_runs.saturating_add(row.get::<i64, _>("runs") as u32);
        }
        delta.elapsed_seconds = merged
            .elapsed_seconds
            .saturating_sub(in_window.elapsed_seconds);
        delta.input_tokens = merged.input_tokens.saturating_sub(in_window.input_tokens);
        delta.output_tokens = merged.output_tokens.saturating_sub(in_window.output_tokens);
        delta.spend_micro_usd = Some(
            merged
                .spend_micro_usd
                .unwrap_or(0)
                .saturating_sub(in_window.spend_micro_usd.unwrap_or(0)),
        );
        runs = 1u32.saturating_sub(charged_runs);
    }
    if runs > 0
        || delta.elapsed_seconds > 0
        || delta.input_tokens > 0
        || delta.output_tokens > 0
        || delta.spend_micro_usd != Some(0)
        || (merged.estimated && !old.estimated)
    {
        sqlx::query("INSERT INTO autonomous_worker_usage(worker_id,reservation_id,accounted_at,runs,document) VALUES(?,?,?,?,?)")
            .bind(worker.detail.worker.id.to_string()).bind(reservation.to_string()).bind(at).bind(i64::from(runs))
            .bind(serde_json::to_string(&delta)?).execute(&mut **tx).await?;
    }
    sqlx::query("UPDATE autonomous_worker_reservations SET usage=?,stopped=?,resolved=?,uncertain=? WHERE id=?")
        .bind(serde_json::to_string(&merged)?).bind(stopped).bind(stopped && !uncertain).bind(uncertain)
        .bind(reservation.to_string()).execute(&mut **tx).await?;
    advance_clock(tx, worker, at).await?;
    worker.detail.budget = budget(tx, worker, now).await?;
    Ok(())
}

fn add_usage(total: &mut JobUsage, value: &JobUsage) {
    total.elapsed_seconds = total.elapsed_seconds.saturating_add(value.elapsed_seconds);
    total.input_tokens = total.input_tokens.saturating_add(value.input_tokens);
    total.output_tokens = total.output_tokens.saturating_add(value.output_tokens);
    total.spend_micro_usd = Some(
        total
            .spend_micro_usd
            .unwrap_or(0)
            .saturating_add(value.spend_micro_usd.unwrap_or(0)),
    );
    total.estimated |= value.estimated;
}

/// Lifetime item consumption comes from cumulative reservation counters, not the
/// rolling debit ledger (which may conservatively re-age unresolved obligations).
pub async fn item_usage(tx: &mut JobTx<'_>, worker: Uuid, item: Uuid) -> Result<JobUsage> {
    let rows = sqlx::query("SELECT r.usage,r.uncertain FROM autonomous_worker_executions e JOIN autonomous_worker_reservations r ON r.id=e.reservation_id WHERE e.worker_id=? AND e.item_id=?")
        .bind(worker.to_string()).bind(item.to_string()).fetch_all(&mut **tx).await?;
    let mut total = JobUsage {
        spend_micro_usd: Some(0),
        ..JobUsage::default()
    };
    let mut uncertain = false;
    for row in rows {
        let usage: JobUsage = serde_json::from_str(row.get("usage"))?;
        uncertain |= row.get::<bool, _>("uncertain") || usage.spend_micro_usd.is_none();
        add_usage(&mut total, &usage);
    }
    if uncertain {
        total.spend_micro_usd = None;
    }
    Ok(total)
}

pub async fn budget(tx: &mut JobTx<'_>, worker: &StoredWorker, now: &str) -> Result<WorkerBudget> {
    let at = accounting_time(tx, worker, now).await?;
    let window = at.saturating_sub((WORKER_BUDGET_WINDOW_SECONDS * 1000) as i64);
    let mut result = WorkerBudget {
        usage: JobUsage {
            spend_micro_usd: Some(0),
            ..JobUsage::default()
        },
        window_started_at: timestamp_string(window)?,
        ..WorkerBudget::default()
    };
    let rows = sqlx::query("SELECT reservation_id,accounted_at,runs,document FROM autonomous_worker_usage WHERE worker_id=? AND accounted_at>? ORDER BY accounted_at,id")
        .bind(worker.detail.worker.id.to_string()).bind(window).fetch_all(&mut **tx).await?;
    let mut expiring = Vec::with_capacity(rows.len());
    let mut charged = std::collections::HashMap::<String, (JobUsage, u32)>::new();
    for row in rows {
        let usage: JobUsage = serde_json::from_str(row.get("document"))?;
        let runs = row.get::<i64, _>("runs") as u32;
        add_usage(&mut result.usage, &usage);
        result.runs = result.runs.saturating_add(runs);
        let entry = charged.entry(row.get("reservation_id")).or_insert_with(|| {
            (
                JobUsage {
                    spend_micro_usd: Some(0),
                    ..JobUsage::default()
                },
                0,
            )
        });
        add_usage(&mut entry.0, &usage);
        entry.1 = entry.1.saturating_add(runs);
        expiring.push((row.get::<i64, _>("accounted_at"), usage, runs));
    }
    let reservations = sqlx::query("SELECT id,limits,usage,uncertain FROM autonomous_worker_reservations WHERE worker_id=? AND resolved=0")
        .bind(worker.detail.worker.id.to_string()).fetch_all(&mut **tx).await?;
    let unresolved = !reservations.is_empty();
    for row in reservations {
        let limits: JobLimits = serde_json::from_str(row.get("limits"))?;
        let cumulative: JobUsage = serde_json::from_str(row.get("usage"))?;
        let in_window = charged.get(row.get::<&str, _>("id"));
        result.reserved_elapsed_seconds = result.reserved_elapsed_seconds.saturating_add(
            limits
                .elapsed_seconds
                .max(cumulative.elapsed_seconds)
                .saturating_sub(in_window.map_or(0, |v| v.0.elapsed_seconds)),
        );
        result.reserved_spend_micro_usd = result.reserved_spend_micro_usd.saturating_add(
            limits
                .spend_micro_usd
                .max(cumulative.spend_micro_usd.unwrap_or(0))
                .saturating_sub(in_window.and_then(|v| v.0.spend_micro_usd).unwrap_or(0)),
        );
        result.reserved_runs = result
            .reserved_runs
            .saturating_add(1u32.saturating_sub(in_window.map_or(0, |v| v.1)));
        result.accounting_uncertain |= row.get::<bool, _>("uncertain");
        result.usage.estimated |= cumulative.estimated;
    }
    let limits = &worker.detail.mandate.spec.limits;
    let fits = |elapsed: u64, spend: u64, runs: u32| {
        elapsed
            .saturating_add(result.reserved_elapsed_seconds)
            .saturating_add(limits.per_run.elapsed_seconds)
            <= limits.elapsed_seconds_24h
            && spend
                .saturating_add(result.reserved_spend_micro_usd)
                .saturating_add(limits.per_run.spend_micro_usd)
                <= limits.spend_micro_usd_24h
            && runs.saturating_add(result.reserved_runs) < limits.runs_24h
    };
    let mut elapsed = result.usage.elapsed_seconds;
    let mut spend = result.usage.spend_micro_usd.unwrap_or(0);
    let mut runs = result.runs;
    if !result.accounting_uncertain && !fits(elapsed, spend, runs) && !unresolved {
        for (time, usage, count) in expiring {
            elapsed = elapsed.saturating_sub(usage.elapsed_seconds);
            spend = spend.saturating_sub(usage.spend_micro_usd.unwrap_or(0));
            runs = runs.saturating_sub(count);
            if fits(elapsed, spend, runs) {
                result.eligible_at = Some(timestamp_string(
                    time.saturating_add((WORKER_BUDGET_WINDOW_SECONDS * 1000) as i64),
                )?);
                break;
            }
        }
    }
    if result.accounting_uncertain {
        result.usage.spend_micro_usd = None;
    }
    Ok(result)
}

/// Clearing a scheduler pointer is not evidence that a process stopped. The
/// immutable link stays as the exact historical run and host-purge manifest.
pub async fn release_execution(tx: &mut JobTx<'_>, worker: Uuid, job: Uuid) -> Result<()> {
    let stored = jobs::load(tx, job).await?;
    ensure!(
        stored.detail.job.state.is_terminal()
            && stored.detail.runs.iter().all(|run| {
                run.state.is_terminal()
                    && (run.started_at.is_none()
                        || run
                            .stop_evidence
                            .as_ref()
                            .is_some_and(|e| e.descendants_stopped))
            }),
        "worker_execution_stop_pending"
    );
    let changed = sqlx::query(
        "UPDATE autonomous_worker_executions SET active=0 WHERE worker_id=? AND job_id=?",
    )
    .bind(worker.to_string())
    .bind(job.to_string())
    .execute(&mut **tx)
    .await?
    .rows_affected();
    ensure!(changed == 1, "worker_execution_not_found");
    Ok(())
}

#[cfg(test)]
mod tests;
