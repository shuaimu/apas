//! Transactional job journal. Ownership rows outlive replay retention and socket lifetimes.
use super::Database;
use anyhow::{ensure, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use shared::*;
use sqlx::{Row, Sqlite, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredJob {
    pub detail: JobDetail,
    pub initiator: Uuid,
    pub review: Option<JobReview>,
    pub dispatched: bool,
    pub source_seq: u64,
    pub lease_expires_at: Option<String>,
    pub last_renewed_at: Option<String>,
}
impl StoredJob {
    pub fn identity(&self) -> JobRunIdentity {
        let j = &self.detail.job;
        JobRunIdentity {
            project_id: j.project_id.clone(),
            job_id: j.id,
            run_id: j.current_run_id,
            cluster_owner_user_id: j.cluster_owner_user_id,
            machine_id: j.machine_id,
            epoch: self
                .detail
                .runs
                .last()
                .expect("persisted job has run")
                .epoch,
        }
    }
    pub fn current(&mut self) -> &mut JobRun {
        self.detail.runs.last_mut().expect("persisted job has run")
    }
    pub fn transition(&mut self, state: JobState, reason: Option<String>) {
        self.detail.job.state = state;
        self.detail.job.reason = reason.clone();
        self.detail.job.state_version += 1;
        self.detail.job.updated_at = Utc::now().to_rfc3339();
        self.current().state = state;
        self.current().reason = reason;
    }
}
pub type JobTx<'a> = Transaction<'a, Sqlite>;
impl Database {
    pub async fn migrate_jobs(&self) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for statement in [
            "CREATE TABLE IF NOT EXISTS autonomous_job_lock (id INTEGER PRIMARY KEY CHECK(id=1), generation INTEGER NOT NULL)",
            "INSERT OR IGNORE INTO autonomous_job_lock VALUES(1,0)",
            "CREATE TABLE IF NOT EXISTS autonomous_jobs (id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, initiator TEXT NOT NULL, owner TEXT NOT NULL, machine TEXT NOT NULL, state TEXT NOT NULL, created_at TEXT NOT NULL, document TEXT NOT NULL)",
            "CREATE TABLE IF NOT EXISTS autonomous_job_project_fences(project_id TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,reason TEXT NOT NULL)",
            "CREATE TRIGGER IF NOT EXISTS autonomous_job_immutable_task BEFORE UPDATE OF document ON autonomous_jobs WHEN json_extract(OLD.document,'$.detail.task') IS NOT json_extract(NEW.document,'$.detail.task') OR json_extract(OLD.document,'$.detail.job.limits') IS NOT json_extract(NEW.document,'$.detail.job.limits') BEGIN SELECT RAISE(ABORT,'immutable_job_inputs'); END",
            "CREATE INDEX IF NOT EXISTS autonomous_jobs_target ON autonomous_jobs(owner,machine,state,created_at,id)",
            "CREATE INDEX IF NOT EXISTS autonomous_jobs_project ON autonomous_jobs(project_id,created_at,id)",
            "CREATE TABLE IF NOT EXISTS autonomous_job_runs (id TEXT PRIMARY KEY, job_id TEXT NOT NULL REFERENCES autonomous_jobs(id) ON DELETE CASCADE, attempt INTEGER NOT NULL, epoch INTEGER NOT NULL, unresolved INTEGER NOT NULL, document TEXT NOT NULL, UNIQUE(job_id,attempt))",
            "CREATE UNIQUE INDEX IF NOT EXISTS autonomous_job_one_unresolved ON autonomous_job_runs(job_id) WHERE unresolved=1",
            "CREATE TABLE IF NOT EXISTS autonomous_job_previews (id TEXT PRIMARY KEY, actor TEXT NOT NULL, project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, document TEXT NOT NULL)",
            "CREATE TABLE IF NOT EXISTS autonomous_job_mutations (actor TEXT NOT NULL, request_id TEXT NOT NULL, project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, fingerprint TEXT NOT NULL, document TEXT NOT NULL, PRIMARY KEY(actor,request_id))",
            "CREATE TABLE IF NOT EXISTS autonomous_job_events (job_id TEXT NOT NULL REFERENCES autonomous_jobs(id) ON DELETE CASCADE, seq INTEGER NOT NULL, id TEXT UNIQUE NOT NULL, run_id TEXT, epoch INTEGER, source_seq INTEGER, created_at TEXT NOT NULL, document TEXT NOT NULL, PRIMARY KEY(job_id,seq), UNIQUE(run_id,epoch,source_seq))",
            "CREATE TABLE IF NOT EXISTS autonomous_job_questions (id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES autonomous_job_runs(id) ON DELETE CASCADE, dialog_id TEXT NOT NULL, document TEXT NOT NULL, UNIQUE(run_id,dialog_id))",
            "CREATE TABLE IF NOT EXISTS autonomous_job_answer_intents(question_id TEXT PRIMARY KEY REFERENCES autonomous_job_questions(id) ON DELETE CASCADE,actor TEXT NOT NULL,request_id TEXT NOT NULL)",
            "CREATE TABLE IF NOT EXISTS autonomous_job_outbox (id TEXT PRIMARY KEY, job_id TEXT NOT NULL REFERENCES autonomous_jobs(id) ON DELETE CASCADE, owner TEXT NOT NULL, machine TEXT NOT NULL, document TEXT NOT NULL, acknowledged INTEGER NOT NULL DEFAULT 0, sent_at TEXT)",
            "CREATE INDEX IF NOT EXISTS autonomous_job_outbox_pending ON autonomous_job_outbox(acknowledged,machine)",
            "CREATE TABLE IF NOT EXISTS autonomous_job_artifacts (id TEXT PRIMARY KEY, job_id TEXT NOT NULL REFERENCES autonomous_jobs(id) ON DELETE CASCADE, run_id TEXT NOT NULL, manifest TEXT NOT NULL, content BLOB NOT NULL DEFAULT X'', complete INTEGER NOT NULL DEFAULT 0)",
            "CREATE TABLE IF NOT EXISTS autonomous_job_reviews(run_id TEXT PRIMARY KEY REFERENCES autonomous_job_runs(id) ON DELETE CASCADE,job_id TEXT NOT NULL REFERENCES autonomous_jobs(id) ON DELETE CASCADE,document TEXT NOT NULL)",
            "CREATE TABLE IF NOT EXISTS autonomous_job_review_refreshes(command_id TEXT PRIMARY KEY REFERENCES autonomous_job_outbox(id) ON DELETE CASCADE,actor TEXT NOT NULL,requested_at TEXT NOT NULL)",
            "CREATE TABLE IF NOT EXISTS autonomous_job_notification_pending(event_id TEXT PRIMARY KEY,job_id TEXT NOT NULL REFERENCES autonomous_jobs(id) ON DELETE CASCADE,project_id TEXT NOT NULL,document TEXT NOT NULL)",
            "CREATE TABLE IF NOT EXISTS autonomous_job_purges(run_id TEXT PRIMARY KEY REFERENCES autonomous_job_runs(id) ON DELETE CASCADE,command_id TEXT NOT NULL REFERENCES autonomous_job_outbox(id) ON DELETE CASCADE,confirmed INTEGER NOT NULL DEFAULT 0)",
            "INSERT OR IGNORE INTO schema_migrations(name) VALUES('autonomous_jobs_v1')",
        ] { sqlx::query(statement).execute(&mut *tx).await?; }
        tx.commit().await?;
        Ok(())
    }
    /// Acquire SQLite's write reservation before reading state: no deferred-upgrade race.
    pub async fn job_transaction(&self) -> Result<JobTx<'_>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE autonomous_job_lock SET generation=generation+1 WHERE id=1")
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }
    pub async fn job_project_fenced(&self, project: &str) -> Result<bool> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM autonomous_job_project_fences WHERE project_id=?",
        )
        .bind(project)
        .fetch_one(&self.pool)
        .await?
            > 0)
    }
    pub async fn clear_stopped_job_fences(&self) -> Result<()> {
        sqlx::query("DELETE FROM autonomous_job_project_fences WHERE project_id IN (SELECT id FROM projects WHERE lifecycle_status='active') AND NOT EXISTS (SELECT 1 FROM autonomous_jobs j WHERE j.project_id=autonomous_job_project_fences.project_id AND j.state NOT IN ('review_ready','cancelled','failed','completed'))").execute(&self.pool).await?;
        Ok(())
    }
    pub async fn job(&self, id: Uuid) -> Result<Option<StoredJob>> {
        let row = sqlx::query("SELECT document FROM autonomous_jobs WHERE id=?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| serde_json::from_str(r.get::<&str, _>("document")).map_err(Into::into))
            .transpose()
    }
    pub async fn all_jobs(&self) -> Result<Vec<StoredJob>> {
        sqlx::query("SELECT document FROM autonomous_jobs ORDER BY created_at,id")
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|r| serde_json::from_str(r.get::<&str, _>("document")).map_err(Into::into))
            .collect()
    }
    pub async fn project_jobs(
        &self,
        project: &str,
        cursor: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<StoredJob>> {
        let mut query = sqlx::QueryBuilder::<Sqlite>::new(
            "SELECT document FROM autonomous_jobs WHERE project_id=",
        );
        query.push_bind(project);
        query.push(" AND json_extract(document,'$.detail.task.worker') IS NULL");
        if let Some(cursor) = cursor {
            if let Some((created_at, id)) = cursor.rsplit_once('|') {
                query
                    .push(" AND (created_at,id)<(")
                    .push_bind(created_at)
                    .push(",")
                    .push_bind(id)
                    .push(")");
            } else {
                // Preserve the global list's lexical behavior for opaque cursors.
                query
                    .push(" AND (created_at || '|' || id)<")
                    .push_bind(cursor);
            }
        }
        if let Some(status) = status {
            if status == "attention" {
                query.push(" AND json_extract(document,'$.detail.job.attention_count')>0");
            } else {
                query.push(" AND state=").push_bind(status);
            }
        }
        query.push(" ORDER BY created_at DESC,id DESC LIMIT 50");
        query
            .build()
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|r| serde_json::from_str(r.get::<&str, _>("document")).map_err(Into::into))
            .collect()
    }
    pub async fn job_receipt(
        &self,
        actor: Uuid,
        request: Uuid,
    ) -> Result<Option<(String, JobMutationReceipt)>> {
        let row = sqlx::query("SELECT project_id,document FROM autonomous_job_mutations WHERE actor=? AND request_id=?")
            .bind(actor.to_string()).bind(request.to_string()).fetch_optional(&self.pool).await?;
        row.map(|r| {
            Ok((
                r.get("project_id"),
                serde_json::from_str(r.get::<&str, _>("document"))?,
            ))
        })
        .transpose()
    }
    pub async fn job_events(&self, id: Uuid, after: u64) -> Result<JobEventsResponse> {
        let mut tx = self.pool.begin().await?;
        let stored = load(&mut tx, id).await?;
        let first: Option<i64> =
            sqlx::query_scalar("SELECT MIN(seq) FROM autonomous_job_events WHERE job_id=?")
                .bind(id.to_string())
                .fetch_one(&mut *tx)
                .await?;
        let rows = sqlx::query("SELECT document FROM autonomous_job_events WHERE job_id=? AND seq>? ORDER BY seq LIMIT 201")
            .bind(id.to_string()).bind(after as i64).fetch_all(&mut *tx).await?;
        let mut events: Vec<JobEvent> = rows
            .into_iter()
            .map(|r| serde_json::from_str(r.get::<&str, _>("document")))
            .collect::<Result<_, _>>()?;
        let has_more = events.len() > 200;
        events.truncate(200);
        let high = stored.detail.high_watermark;
        let resync = after > high
            || first
                .map(|n| after.saturating_add(1) < n as u64)
                .unwrap_or(after < high)
            || events
                .first()
                .is_some_and(|e| e.seq != after.saturating_add(1))
            || events.windows(2).any(|pair| pair[1].seq != pair[0].seq + 1);
        let next_seq = events.last().map(|e| e.seq).unwrap_or(after.min(high));
        tx.commit().await?;
        Ok(JobEventsResponse {
            events,
            next_seq,
            high_watermark: high,
            has_more,
            resync_required: resync,
        })
    }
    pub async fn job_outbox(&self) -> Result<Vec<(Uuid, Uuid, Uuid, JobWorkerCommand)>> {
        sqlx::query("SELECT id,owner,machine,document FROM autonomous_job_outbox WHERE acknowledged=0 ORDER BY rowid LIMIT 256").fetch_all(&self.pool).await?.into_iter().map(|r| {
            Ok((Uuid::parse_str(r.get("id"))?,Uuid::parse_str(r.get("owner"))?,Uuid::parse_str(r.get("machine"))?,serde_json::from_str(r.get("document"))?))
        }).collect()
    }
    pub async fn pending_job_notifications(&self) -> Result<Vec<(String, JobEvent)>> {
        sqlx::query("SELECT project_id,document FROM autonomous_job_notification_pending ORDER BY rowid LIMIT 100").fetch_all(&self.pool).await?.into_iter().map(|r|Ok((r.get("project_id"),serde_json::from_str(r.get("document"))?))).collect()
    }
    pub async fn complete_job_notification(&self, event: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM autonomous_job_notification_pending WHERE event_id=?")
            .bind(event.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn job_review(&self, job: Uuid, run: Uuid) -> Result<Option<JobReview>> {
        let row: Option<String> = sqlx::query_scalar(
            "SELECT document FROM autonomous_job_reviews WHERE job_id=? AND run_id=?",
        )
        .bind(job.to_string())
        .bind(run.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(|s| serde_json::from_str(&s).map_err(Into::into))
            .transpose()
    }
    pub async fn job_artifact(
        &self,
        job: Uuid,
        artifact: Uuid,
        offset: usize,
        limit: usize,
    ) -> Result<JobArtifactContent> {
        let row = sqlx::query("SELECT manifest,content,complete FROM autonomous_job_artifacts WHERE id=? AND job_id=?")
            .bind(artifact.to_string()).bind(job.to_string()).fetch_optional(&self.pool).await?.ok_or_else(||anyhow::anyhow!("artifact_not_found"))?;
        let manifest: JobArtifact = serde_json::from_str(row.get("manifest"))?;
        let expired = manifest
            .expires_at
            .as_deref()
            .is_some_and(|s| s < Utc::now().to_rfc3339().as_str());
        ensure!(
            row.get::<i64, _>("complete") == 1 || expired,
            "artifact_pending"
        );
        let bytes: Vec<u8> = row.get("content");
        ensure!(offset <= bytes.len() || expired, "invalid_cursor");
        let end = offset
            .saturating_add(limit.clamp(4, 64 * 1024))
            .min(bytes.len());
        let (text, next_cursor) = if expired {
            (String::new(), None)
        } else {
            // Artifact bytes are UTF-8 text for this endpoint; refuse binary instead of corrupting it.
            let all = std::str::from_utf8(&bytes)
                .map_err(|_| anyhow::anyhow!("binary_artifact_not_text"))?;
            ensure!(all.is_char_boundary(offset), "invalid_cursor");
            let mut end = end;
            while !all.is_char_boundary(end) {
                end -= 1;
            }
            (
                all[offset..end].to_owned(),
                (end < bytes.len()).then(|| end.to_string()),
            )
        };
        Ok(JobArtifactContent {
            artifact: manifest,
            text,
            next_cursor,
            expired,
        })
    }
    pub async fn retain_job_content(&self) -> Result<()> {
        let cutoff = (Utc::now() - chrono::Duration::days(30)).to_rfc3339();
        let mut tx = self.job_transaction().await?;
        sqlx::query("DELETE FROM autonomous_job_events WHERE job_id IN (SELECT id FROM autonomous_jobs WHERE state IN ('review_ready','cancelled','failed','completed') AND json_extract(document,'$.detail.runs[#-1].ended_at')<?)").bind(&cutoff).execute(&mut *tx).await?;
        sqlx::query("UPDATE autonomous_job_artifacts SET content=X'' WHERE json_extract(manifest,'$.expires_at') IS NOT NULL AND json_extract(manifest,'$.expires_at')<?").bind(Utc::now().to_rfc3339()).execute(&mut *tx).await?;
        sqlx::query(
            "DELETE FROM autonomous_job_previews WHERE json_extract(document,'$.expires_at')<?",
        )
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
pub async fn load(tx: &mut JobTx<'_>, id: Uuid) -> Result<StoredJob> {
    let value: String = sqlx::query_scalar("SELECT document FROM autonomous_jobs WHERE id=?")
        .bind(id.to_string())
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("job_not_found"))?;
    Ok(serde_json::from_str(&value)?)
}
pub async fn save(tx: &mut JobTx<'_>, value: &StoredJob) -> Result<()> {
    let j = &value.detail.job;
    let state = serde_json::to_value(j.state)?.as_str().unwrap().to_owned();
    sqlx::query("INSERT INTO autonomous_jobs(id,project_id,initiator,owner,machine,state,created_at,document) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET state=excluded.state,document=excluded.document")
        .bind(j.id.to_string()).bind(&j.project_id).bind(value.initiator.to_string()).bind(j.cluster_owner_user_id.to_string()).bind(j.machine_id.to_string()).bind(state).bind(&j.created_at).bind(serde_json::to_string(value)?).execute(&mut **tx).await?;
    for run in &value.detail.runs {
        let unresolved = !run.state.is_terminal()
            || (run.started_at.is_some()
                && !run
                    .stop_evidence
                    .as_ref()
                    .is_some_and(|e| e.descendants_stopped));
        sqlx::query("INSERT INTO autonomous_job_runs(id,job_id,attempt,epoch,unresolved,document) VALUES(?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET unresolved=excluded.unresolved,document=excluded.document")
            .bind(run.id.to_string()).bind(j.id.to_string()).bind(run.attempt as i64).bind(run.epoch as i64).bind(unresolved).bind(serde_json::to_string(run)?).execute(&mut **tx).await?;
    }
    if let Some(review) = &value.review {
        sqlx::query("INSERT INTO autonomous_job_reviews(run_id,job_id,document) VALUES(?,?,?) ON CONFLICT(run_id) DO UPDATE SET document=excluded.document")
            .bind(review.run_id.to_string()).bind(j.id.to_string()).bind(serde_json::to_string(review)?).execute(&mut **tx).await?;
    }
    if j.state.is_terminal() {
        sqlx::query("UPDATE autonomous_job_artifacts SET manifest=json_set(manifest,'$.expires_at',?) WHERE job_id=? AND json_extract(manifest,'$.expires_at') IS NULL")
            .bind((Utc::now()+chrono::Duration::days(30)).to_rfc3339()).bind(j.id.to_string()).execute(&mut **tx).await?;
    }
    for q in &value.detail.questions {
        sqlx::query("INSERT INTO autonomous_job_questions(id,run_id,dialog_id,document) VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET document=excluded.document")
            .bind(q.id.to_string()).bind(q.run_id.to_string()).bind(&q.dialog_id).bind(serde_json::to_string(q)?).execute(&mut **tx).await?;
    }
    Ok(())
}
pub async fn receipt(
    tx: &mut JobTx<'_>,
    actor: Uuid,
    request: Uuid,
    fingerprint: &str,
) -> Result<Option<JobMutationReceipt>> {
    let row = sqlx::query(
        "SELECT fingerprint,document FROM autonomous_job_mutations WHERE actor=? AND request_id=?",
    )
    .bind(actor.to_string())
    .bind(request.to_string())
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|r| {
        ensure!(
            r.get::<&str, _>("fingerprint") == fingerprint,
            "request_conflict"
        );
        Ok(serde_json::from_str(r.get("document"))?)
    })
    .transpose()
}
pub async fn save_receipt(
    tx: &mut JobTx<'_>,
    actor: Uuid,
    project: &str,
    fingerprint: &str,
    receipt: &JobMutationReceipt,
) -> Result<()> {
    sqlx::query("INSERT INTO autonomous_job_mutations(actor,request_id,project_id,fingerprint,document) VALUES(?,?,?,?,?)")
        .bind(actor.to_string()).bind(receipt.request_id.to_string()).bind(project).bind(fingerprint).bind(serde_json::to_string(receipt)?).execute(&mut **tx).await?;
    Ok(())
}
pub async fn append(
    tx: &mut JobTx<'_>,
    job: &mut StoredJob,
    kind: JobEventKind,
    summary: &str,
    detail: Option<serde_json::Value>,
    source: Option<u64>,
) -> Result<JobEvent> {
    ensure!(summary.len() <= 8192, "event_too_large");
    ensure!(
        detail.as_ref().map(|d| d.to_string().len()).unwrap_or(0) <= 128 * 1024,
        "event_too_large"
    );
    let identity = job.identity();
    job.detail.high_watermark += 1;
    job.detail.job.state_version += 1;
    job.detail.job.updated_at = Utc::now().to_rfc3339();
    let event = JobEvent {
        id: Uuid::new_v4(),
        job_id: identity.job_id,
        run_id: Some(identity.run_id),
        seq: job.detail.high_watermark,
        kind,
        summary: summary.into(),
        created_at: Utc::now().to_rfc3339(),
        detail,
    };
    sqlx::query("INSERT INTO autonomous_job_events(job_id,seq,id,run_id,epoch,source_seq,created_at,document) VALUES(?,?,?,?,?,?,?,?)")
        .bind(identity.job_id.to_string()).bind(event.seq as i64).bind(event.id.to_string()).bind(identity.run_id.to_string()).bind(identity.epoch as i64).bind(source.map(|s|s as i64)).bind(&event.created_at).bind(serde_json::to_string(&event)?).execute(&mut **tx).await?;
    if matches!(
        kind,
        JobEventKind::Question
            | JobEventKind::Failed
            | JobEventKind::Stopped
            | JobEventKind::RecoveryRequired
            | JobEventKind::Publication
            | JobEventKind::Verification
    ) {
        sqlx::query("INSERT INTO autonomous_job_notification_pending(event_id,job_id,project_id,document) VALUES(?,?,?,?)").bind(event.id.to_string()).bind(identity.job_id.to_string()).bind(&identity.project_id).bind(serde_json::to_string(&event)?).execute(&mut **tx).await?;
    }
    // A bounded replay tail is independent of retained ownership/evidence.
    sqlx::query("DELETE FROM autonomous_job_events WHERE job_id=? AND seq<?")
        .bind(identity.job_id.to_string())
        .bind(event.seq.saturating_sub(2000) as i64)
        .execute(&mut **tx)
        .await?;
    let artifact_bytes: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(length(content)),0) FROM autonomous_job_artifacts WHERE run_id=?",
    )
    .bind(identity.run_id.to_string())
    .fetch_one(&mut **tx)
    .await?;
    let budget = job
        .detail
        .job
        .limits
        .output_bytes
        .min(100 * 1024 * 1024)
        .saturating_sub(artifact_bytes as u64);
    sqlx::query("DELETE FROM autonomous_job_events WHERE job_id=? AND seq IN (SELECT seq FROM (SELECT seq,SUM(length(document)) OVER(ORDER BY seq DESC) AS retained FROM autonomous_job_events WHERE job_id=?) WHERE retained>?)").bind(identity.job_id.to_string()).bind(identity.job_id.to_string()).bind(budget as i64).execute(&mut **tx).await?;
    save(tx, job).await?;
    Ok(event)
}
pub async fn command(
    tx: &mut JobTx<'_>,
    job: &StoredJob,
    id: Uuid,
    command: JobWorkerCommand,
) -> Result<()> {
    let document = serde_json::to_string(&command)?;
    let existing: Option<String> =
        sqlx::query_scalar("SELECT document FROM autonomous_job_outbox WHERE id=?")
            .bind(id.to_string())
            .fetch_optional(&mut **tx)
            .await?;
    if let Some(existing) = existing {
        ensure!(existing == document, "command_payload_conflict");
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO autonomous_job_outbox(id,job_id,owner,machine,document) VALUES(?,?,?,?,?)",
    )
    .bind(id.to_string())
    .bind(job.detail.job.id.to_string())
    .bind(job.detail.job.cluster_owner_user_id.to_string())
    .bind(job.detail.job.machine_id.to_string())
    .bind(document)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
pub async fn acknowledge(tx: &mut JobTx<'_>, id: Uuid) -> Result<()> {
    sqlx::query("UPDATE autonomous_job_outbox SET acknowledged=1 WHERE id=?")
        .bind(id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}
