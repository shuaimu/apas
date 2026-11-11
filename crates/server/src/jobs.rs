//! Durable unattended-job control plane, independent of terminal session ownership.
use crate::{
    db::{
        job_policy::{authorize_job_action, effective_job_policy, JobAction},
        jobs::{self as store, StoredJob},
    },
    state::AppState,
};
use anyhow::{ensure, Result};
use chrono::{Duration, Utc};
use dashmap::DashMap;
use serde::Serialize;
use sha2::{Digest, Sha256};
use shared::*;
use sqlx::Row;
use tokio::sync::oneshot;
use uuid::Uuid;

#[derive(Default)]
pub struct JobService {
    pub readiness: DashMap<Uuid, JobWorkerReadiness>,
    preparations: DashMap<Uuid, (Uuid, Uuid, String, Uuid, oneshot::Sender<JobWorkerReport>)>,
    pub subscriptions: DashMap<(Uuid, Uuid), String>,
    pub connection_auth: DashMap<Uuid, (Uuid, Option<String>, usize)>,
    pub cleanup_scheduled: DashMap<String, ()>,
    daemon_gates: DashMap<Uuid, std::sync::Arc<tokio::sync::Mutex<()>>>,
}
impl JobService {
    pub async fn daemon_guard(&self, machine: Uuid) -> tokio::sync::OwnedMutexGuard<()> {
        self.daemon_gates
            .entry(machine)
            .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
            .clone()
            .lock_owned()
            .await
    }
}
struct PendingPreparation<'a> {
    service: &'a JobService,
    id: Uuid,
}
impl Drop for PendingPreparation<'_> {
    fn drop(&mut self) {
        self.service.preparations.remove(&self.id);
    }
}
fn digest(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn fresh(readiness: &JobWorkerReadiness) -> bool {
    readiness.protocol_version == JOB_PROTOCOL_VERSION
        && chrono::DateTime::parse_from_rfc3339(&readiness.expires_at).is_ok_and(|d| d > Utc::now())
        && chrono::DateTime::parse_from_rfc3339(&readiness.checked_at).is_ok_and(|d| {
            d <= Utc::now() && d > Utc::now() - Duration::seconds(JOB_READINESS_SECONDS as i64)
        })
}
async fn authorized(
    state: &AppState,
    actor: Uuid,
    job: &StoredJob,
    action: JobAction,
) -> Result<bool> {
    authorize_job_action(
        state,
        actor,
        &job.detail.job.project_id,
        job.detail.job.cluster_owner_user_id,
        job.detail.job.machine_id,
        job.initiator,
        action,
    )
    .await
}
async fn require(state: &AppState, actor: Uuid, job: &StoredJob, action: JobAction) -> Result<()> {
    ensure!(
        authorized(state, actor, job, action).await?,
        "job_access_denied"
    );
    Ok(())
}
pub async fn targets(state: &AppState, actor: Uuid, project: &str) -> Result<JobTargetsResponse> {
    ensure!(
        authorize_job_action(
            state,
            actor,
            project,
            Uuid::nil(),
            Uuid::nil(),
            actor,
            JobAction::Read
        )
        .await?,
        "job_access_denied"
    );
    let policy = effective_job_policy(&state.db, project).await?;
    let fenced = state.db.job_project_fenced(project).await?;
    let readiness: Vec<_> = state.jobs.readiness.iter().map(|r| r.clone()).collect();
    let mut targets = Vec::new();
    for ready in readiness {
        let owner = ready.cluster_owner_user_id;
        let machine = ready.machine_id;
        if !state
            .db
            .project_is_placed_in_cluster(project, &owner.to_string())
            .await?
        {
            continue;
        }
        let machines = state.sessions.get_machines_for_user(&owner);
        let Some(info) = machines.iter().find(|m| {
            m.machine.machine_id == machine && m.projects.iter().any(|p| p.project_id == project)
        }) else {
            continue;
        };
        let admit = authorize_job_action(
            state,
            actor,
            project,
            owner,
            machine,
            actor,
            JobAction::Admit,
        )
        .await?;
        let mut reasons = Vec::new();
        if fenced {
            reasons.push("project_stop_pending".into());
        }
        if !policy.policy.enabled {
            reasons.push("jobs_disabled".into());
        }
        if !admit {
            reasons.push("admission_denied".into());
        }
        if !fresh(&ready) {
            reasons.push("readiness_expired".into());
        }
        if !state.sessions.is_daemon_connected(&machine)
            || state.sessions.daemon_owner(&machine) != Some(owner)
        {
            reasons.push("target_offline".into());
        }
        if !state
            .sessions
            .daemon_supports_capability(&machine, OMP_JOB_WORKER_CAPABILITY)
        {
            reasons.push("worker_capability_missing".into());
        }
        let profiles: Vec<_> = ready
            .profiles
            .into_iter()
            .filter(|p| policy.policy.worker_profiles.contains(&p.key))
            .collect();
        if !profiles.iter().any(|p| p.ready) {
            reasons.push("worker_profile_unavailable".into());
        }
        targets.push(JobTarget {
            project_id: project.into(),
            machine_id: machine,
            cluster_owner_user_id: owner,
            hostname: info.machine.hostname.clone(),
            profiles,
            available: reasons.is_empty(),
            unavailable_reasons: reasons,
            policy: policy.clone(),
            allowed_actions: JobAllowedActions {
                admit,
                ..Default::default()
            },
        });
    }
    let unavailable_reasons = if targets.is_empty() {
        vec!["no_qualified_job_targets".into()]
    } else {
        vec![]
    };
    Ok(JobTargetsResponse {
        project_id: project.into(),
        targets,
        unavailable_reasons,
    })
}
async fn prepare(
    state: &AppState,
    actor: Uuid,
    project: &str,
    target: &JobTarget,
    base: &str,
    profile: &str,
) -> Result<JobWorkerReport> {
    let command_id = Uuid::new_v4();
    let (tx, rx) = oneshot::channel();
    state.jobs.preparations.insert(
        command_id,
        (
            target.cluster_owner_user_id,
            target.machine_id,
            project.into(),
            actor,
            tx,
        ),
    );
    let _pending = PendingPreparation {
        service: &state.jobs,
        id: command_id,
    };
    let sent = state
        .sessions
        .send_to_daemon(
            &target.machine_id,
            ServerToDaemon::AutonomousJob {
                command: JobWorkerCommand::Prepare {
                    command_id,
                    project_id: project.into(),
                    machine_id: target.machine_id,
                    cluster_owner_user_id: target.cluster_owner_user_id,
                    base_ref: base.into(),
                    worker_profile: profile.into(),
                },
            },
        )
        .await;
    let response = if sent {
        tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .ok()
            .and_then(Result::ok)
    } else {
        None
    };
    response.ok_or_else(|| anyhow::anyhow!("prepare_unavailable"))
}
pub async fn preview(
    state: &AppState,
    actor: Uuid,
    project: &str,
    request: JobPreviewRequest,
) -> Result<JobPreview> {
    ensure!(
        !request.instruction.trim().is_empty()
            && request.instruction.len() <= 64 * 1024
            && request.acceptance_criteria.len() <= 64
            && request.acceptance_criteria.iter().all(|s| s.len() <= 8192),
        "invalid_task"
    );
    ensure!(
        !request.base_ref.is_empty() && request.base_ref.len() <= 512,
        "invalid_base"
    );
    let target = targets(state, actor, project)
        .await?
        .targets
        .into_iter()
        .find(|t| t.machine_id == request.machine_id && t.available)
        .ok_or_else(|| anyhow::anyhow!("target_unavailable"))?;
    let prepared = prepare(
        state,
        actor,
        project,
        &target,
        &request.base_ref,
        &request.worker_profile,
    )
    .await?;
    let JobWorkerReport::Prepared {
        repository,
        base_sha,
        base_ref,
        profile,
        ..
    } = prepared
    else {
        anyhow::bail!("prepare_rejected")
    };
    ensure!(
        base_ref == request.base_ref
            && profile.key == request.worker_profile
            && profile.ready
            && !repository.is_empty()
            && base_sha.len() == 40
            && base_sha.bytes().all(|b| b.is_ascii_hexdigit()),
        "prepared_inputs_invalid"
    );
    ensure!(
        target.policy.policy.worker_profiles.contains(&profile.key)
            && target
                .policy
                .policy
                .model_profiles
                .contains(&request.model_profile)
            && target
                .policy
                .policy
                .verification_profiles
                .contains(&request.verification_profile),
        "profile_denied"
    );
    ensure!(
        profile
            .model_profiles
            .iter()
            .any(|p| p.key == request.model_profile),
        "model_unavailable"
    );
    let verifier = profile
        .verification_profiles
        .iter()
        .find(|p| p.key == request.verification_profile)
        .ok_or_else(|| anyhow::anyhow!("verification_unavailable"))?;
    ensure!(
        request.limits.fits_within(&target.policy.policy.limits)
            && request.limits.fits_within(&profile.limits),
        "limits_denied"
    );
    let task = JobTaskSpec {
        instruction: request.instruction,
        acceptance_criteria: request.acceptance_criteria,
        source_url: request.source_url,
        repository,
        base_ref,
        base_sha,
        worker_profile: profile.key,
        worker_profile_digest: profile.digest,
        verification_profile: verifier.key.clone(),
        verification_profile_digest: verifier.digest.clone(),
        model_profile: request.model_profile,
    };
    let mut preview = JobPreview {
        id: Uuid::new_v4(),
        project_id: project.into(),
        fingerprint: String::new(),
        expires_at: (Utc::now() + Duration::minutes(5)).to_rfc3339(),
        task,
        machine_id: target.machine_id,
        cluster_owner_user_id: target.cluster_owner_user_id,
        hostname: target.hostname,
        limits: request.limits,
        policy_digest: target.policy.digest,
    };
    preview.fingerprint = digest(&preview)?;
    state.db.store_job_preview(actor, &preview).await?;
    Ok(preview)
}
async fn admit_inner(
    state: &AppState,
    actor: Uuid,
    project: &str,
    request: JobAdmissionRequest,
) -> Result<JobMutationReceipt> {
    let fingerprint = digest(&("admit", project, &request))?;
    {
        let mut tx = state.db.job_transaction().await?;
        if let Some(receipt) =
            store::receipt(&mut tx, actor, request.request_id, &fingerprint).await?
        {
            tx.commit().await?;
            receipt_access(state, actor, &receipt).await?;
            return Ok(receipt);
        }
    }
    let preview = state
        .db
        .job_preview(actor, request.preview_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("preview_not_found"))?;
    ensure!(
        preview.project_id == project
            && preview.fingerprint == request.fingerprint
            && preview.expires_at > Utc::now().to_rfc3339(),
        "preview_changed"
    );
    let current = preview_for_confirmation(state, actor, &preview).await?;
    ensure!(current, "preview_changed");
    let policy = effective_job_policy(&state.db, project).await?;
    let mut tx = state.db.job_transaction().await?;
    if let Some(receipt) = store::receipt(&mut tx, actor, request.request_id, &fingerprint).await? {
        tx.commit().await?;
        return Ok(receipt);
    }
    ensure!(
        effective_job_policy(&state.db, project).await?.digest == preview.policy_digest,
        "preview_changed"
    );
    ensure!(
        authorize_job_action(
            state,
            actor,
            project,
            preview.cluster_owner_user_id,
            preview.machine_id,
            actor,
            JobAction::Admit
        )
        .await?,
        "job_access_denied"
    );
    let fenced: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM autonomous_job_project_fences WHERE project_id=?")
            .bind(project)
            .fetch_one(&mut *tx)
            .await?;
    ensure!(fenced == 0, "project_stop_pending");
    let queued: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM autonomous_jobs WHERE project_id=? AND state IN ('queued','blocked')",
    )
    .bind(project)
    .fetch_one(&mut *tx)
    .await?;
    ensure!(
        queued < policy.policy.max_queued_project as i64,
        "queue_capacity"
    );
    let id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();
    let zero_usage = JobUsage {
        spend_micro_usd: Some(0),
        ..Default::default()
    };
    let run = JobRun {
        id: run_id,
        job_id: id,
        attempt: 1,
        epoch: 1,
        state: JobState::Queued,
        usage: zero_usage.clone(),
        started_at: None,
        ended_at: None,
        worker_version: None,
        candidate_sha: None,
        stop_evidence: None,
        reason: None,
    };
    let summary = JobSummary {
        id,
        project_id: project.into(),
        project_name: project.into(),
        title: preview.task.instruction.chars().take(100).collect(),
        state: JobState::Queued,
        state_version: 1,
        current_run_id: run_id,
        cluster_owner_user_id: preview.cluster_owner_user_id,
        machine_id: preview.machine_id,
        hostname: preview.hostname,
        created_at: now.clone(),
        updated_at: now,
        usage: zero_usage,
        limits: preview.limits,
        attention_count: 0,
        reason: None,
        allowed_actions: Default::default(),
    };
    let mut job = StoredJob {
        detail: JobDetail {
            job: summary,
            task: preview.task,
            runs: vec![run],
            questions: vec![],
            high_watermark: 0,
            policy_digest: preview.policy_digest,
            publication: None,
        },
        initiator: actor,
        review: None,
        dispatched: false,
        source_seq: 0,
        lease_expires_at: None,
        last_renewed_at: None,
    };
    store::save(&mut tx, &job).await?;
    let event = store::append(
        &mut tx,
        &mut job,
        JobEventKind::Admitted,
        "Job admitted",
        None,
        None,
    )
    .await?;
    let receipt = JobMutationReceipt {
        request_id: request.request_id,
        operation: JobMutationKind::Admit,
        job_id: Some(id),
        run_id: Some(run_id),
        status: JobReceiptStatus::Accepted,
        error_code: None,
        message: None,
    };
    store::save_receipt(&mut tx, actor, project, &fingerprint, &receipt).await?;
    tx.commit().await?;
    fanout(state, project, &event).await;
    Ok(receipt)
}
async fn preview_for_confirmation(
    state: &AppState,
    actor: Uuid,
    preview: &JobPreview,
) -> Result<bool> {
    let target = targets(state, actor, &preview.project_id)
        .await?
        .targets
        .into_iter()
        .find(|t| {
            t.machine_id == preview.machine_id
                && t.cluster_owner_user_id == preview.cluster_owner_user_id
                && t.available
        });
    let Some(target) = target else {
        return Ok(false);
    };
    if target.policy.digest != preview.policy_digest {
        return Ok(false);
    }
    let report = prepare(
        state,
        actor,
        &preview.project_id,
        &target,
        &preview.task.base_ref,
        &preview.task.worker_profile,
    )
    .await?;
    Ok(
        matches!(report,JobWorkerReport::Prepared{repository,base_sha,profile,..} if repository==preview.task.repository && base_sha==preview.task.base_sha && profile.digest==preview.task.worker_profile_digest && profile.ready && profile.verification_profiles.iter().any(|v|v.key==preview.task.verification_profile && v.digest==preview.task.verification_profile_digest)),
    )
}
async fn receipt_access(state: &AppState, actor: Uuid, receipt: &JobMutationReceipt) -> Result<()> {
    if let Some(id) = receipt.job_id {
        let job = state
            .db
            .job(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("job_not_found"))?;
        require(state, actor, &job, JobAction::Read).await?;
    }
    Ok(())
}
pub async fn receipt(state: &AppState, actor: Uuid, request: Uuid) -> Result<JobMutationReceipt> {
    let (project, receipt) = state
        .db
        .job_receipt(actor, request)
        .await?
        .ok_or_else(|| anyhow::anyhow!("receipt_not_found"))?;
    ensure!(
        authorize_job_action(
            state,
            actor,
            &project,
            Uuid::nil(),
            Uuid::nil(),
            actor,
            JobAction::Read
        )
        .await?,
        "job_access_denied"
    );
    receipt_access(state, actor, &receipt).await?;
    Ok(receipt)
}
async fn actions(state: &AppState, actor: Uuid, job: &StoredJob) -> Result<JobAllowedActions> {
    let phase = job.detail.job.state;
    let stopped = job.detail.runs.last().is_some_and(|r| {
        r.stop_evidence
            .as_ref()
            .is_some_and(|e| e.descendants_stopped)
            || r.started_at.is_none()
    });
    let fenced = state
        .db
        .job_project_fenced(&job.detail.job.project_id)
        .await
        .unwrap_or(true);
    Ok(JobAllowedActions {
        admit: false,
        answer: !fenced
            && phase == JobState::NeedsInput
            && authorized(state, actor, job, JobAction::Answer)
                .await
                .unwrap_or(false),
        cancel: !phase.is_terminal()
            && authorized(state, actor, job, JobAction::Cancel)
                .await
                .unwrap_or(false),
        retry: !fenced
            && matches!(phase, JobState::Failed | JobState::Cancelled)
            && job.detail.publication.is_none()
            && stopped
            && remaining(job).is_ok()
            && authorized(state, actor, job, JobAction::Retry)
                .await
                .unwrap_or(false),
        retry_publication: !fenced
            && matches!(phase, JobState::Blocked | JobState::RecoveryRequired)
            && valid_review(job)
            && runtime_authorized(state, job).await.unwrap_or(false)
            && authorized(state, actor, job, JobAction::Publish)
                .await
                .unwrap_or(false),
        manage_policy: false,
    })
}
pub async fn detail_by_id(state: &AppState, actor: Uuid, id: Uuid) -> Result<JobDetail> {
    let job = state
        .db
        .job(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("job_not_found"))?;
    require(state, actor, &job, JobAction::Read).await?;
    let allowed = actions(state, actor, &job).await?;
    let mut detail = job.detail;
    detail.job.allowed_actions = allowed;
    Ok(detail)
}
pub async fn detail(state: &AppState, actor: Uuid, project: &str, id: Uuid) -> Result<JobDetail> {
    let d = detail_by_id(state, actor, id).await?;
    ensure!(d.job.project_id == project, "job_not_found");
    Ok(d)
}
pub async fn list_jobs(
    state: &AppState,
    actor: Uuid,
    cursor: Option<&str>,
    status: Option<&str>,
) -> Result<JobListResponse> {
    let mut jobs = Vec::new();
    let mut next_cursor = None;
    let mut rows = state.db.all_jobs().await?;
    rows.reverse();
    for job in rows {
        let key = format!("{}|{}", job.detail.job.created_at, job.detail.job.id);
        if cursor.is_some_and(|c| key.as_str() >= c) {
            continue;
        }
        if status.is_some_and(|s| {
            s != "attention"
                && serde_json::to_value(job.detail.job.state)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .as_deref()
                    != Some(s)
        }) {
            continue;
        }
        if status == Some("attention") && job.detail.job.attention_count == 0 {
            continue;
        }
        if !authorized(state, actor, &job, JobAction::Read).await? {
            continue;
        }
        if jobs.len() == 50 {
            break;
        }
        let allowed = actions(state, actor, &job).await?;
        let mut summary = job.detail.job;
        summary.allowed_actions = allowed;
        next_cursor = Some(key);
        jobs.push(summary);
    }
    if jobs.len() < 50 {
        next_cursor = None
    }
    Ok(JobListResponse { jobs, next_cursor })
}
pub async fn list_project_jobs(
    state: &AppState,
    actor: Uuid,
    project: &str,
    cursor: Option<&str>,
    status: Option<&str>,
) -> Result<ProjectJobListResponse> {
    ensure!(
        authorize_job_action(
            state,
            actor,
            project,
            Uuid::nil(),
            Uuid::nil(),
            actor,
            JobAction::Read,
        )
        .await?,
        "job_access_denied"
    );
    let rows = state.db.project_jobs(project, cursor, status).await?;
    let next_cursor = if rows.len() == 50 {
        rows.last().map(|job| {
            format!("{}|{}", job.detail.job.created_at, job.detail.job.id)
        })
    } else {
        None
    };
    let mut jobs = Vec::with_capacity(rows.len());
    for job in rows {
        let allowed = actions(state, actor, &job).await?;
        let mut summary = job.detail.job;
        summary.allowed_actions = allowed;
        jobs.push(ProjectJobSummary {
            summary,
            description: job.detail.task.instruction,
        });
    }
    Ok(ProjectJobListResponse { jobs, next_cursor })
}
fn remaining(job: &StoredJob) -> Result<JobLimits> {
    let mut limits = job.detail.job.limits.clone();
    let usage = &job.detail.job.usage;
    ensure!(
        job.detail.runs.len() < limits.attempts as usize,
        "attempts_exhausted"
    );
    limits.elapsed_seconds = limits.elapsed_seconds.saturating_sub(usage.elapsed_seconds);
    limits.spend_micro_usd = limits.spend_micro_usd.saturating_sub(
        usage
            .spend_micro_usd
            .ok_or_else(|| anyhow::anyhow!("usage_unknown"))?,
    );
    ensure!(
        limits.elapsed_seconds > 0 && limits.spend_micro_usd > 0,
        "budget_exhausted"
    );
    Ok(limits)
}
fn valid_review(job: &StoredJob) -> bool {
    job.review
        .as_ref()
        .and_then(|r| r.verification.as_ref())
        .is_some_and(|v| {
            !v.checks.is_empty()
                && !v.completed_at.is_empty()
                && v.image_digest.contains("@sha256:")
                && v.checks.iter().all(|c| {
                    c.status == JobCheckStatus::Passed
                        && c.exit_code == Some(0)
                        && c.log_artifact_id.is_some()
                })
                && v.base_sha == job.detail.task.base_sha
                && v.profile_digest == job.detail.task.verification_profile_digest
                && job.review.as_ref().and_then(|r| r.candidate_sha.as_ref())
                    == Some(&v.candidate_sha)
        })
}
async fn cancel_locked(
    tx: &mut store::JobTx<'_>,
    job: &mut StoredJob,
    reason: &str,
) -> Result<Option<JobEvent>> {
    if job.detail.job.state.is_terminal() || job.detail.job.state == JobState::Cancelling {
        return Ok(None);
    }
    sqlx::query("UPDATE autonomous_job_outbox SET acknowledged=1 WHERE job_id=?")
        .bind(job.detail.job.id.to_string())
        .execute(&mut **tx)
        .await?;
    if !job.dispatched {
        job.transition(JobState::Cancelled, Some(reason.into()));
        job.current().ended_at = Some(Utc::now().to_rfc3339());
    } else {
        job.transition(JobState::Cancelling, Some(reason.into()));
        let id = Uuid::new_v4();
        store::command(
            tx,
            job,
            id,
            JobWorkerCommand::Cancel {
                command_id: id,
                identity: job.identity(),
                reason: reason.into(),
            },
        )
        .await?;
    }
    for q in &mut job.detail.questions {
        if q.state != JobQuestionState::Confirmed {
            q.state = JobQuestionState::Cancelled;
        }
    }
    Ok(Some(
        store::append(
            tx,
            job,
            JobEventKind::CancellationRequested,
            "Cancellation requested",
            None,
            None,
        )
        .await?,
    ))
}
async fn mutate_inner(
    state: &AppState,
    actor: Uuid,
    project: &str,
    id: Uuid,
    kind: JobMutationKind,
    request: JobMutationRequest,
) -> Result<JobMutationReceipt> {
    let old = state
        .db
        .job(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("job_not_found"))?;
    ensure!(old.detail.job.project_id == project, "job_not_found");
    require(
        state,
        actor,
        &old,
        match kind {
            JobMutationKind::Cancel => JobAction::Cancel,
            JobMutationKind::Retry => JobAction::Retry,
            JobMutationKind::RetryPublication => JobAction::Publish,
            _ => anyhow::bail!("invalid_operation"),
        },
    )
    .await?;
    let fp = digest(&(project, id, kind, &request))?;
    let mut tx = state.db.job_transaction().await?;
    if let Some(r) = store::receipt(&mut tx, actor, request.request_id, &fp).await? {
        tx.commit().await?;
        return Ok(r);
    }
    let mut job = store::load(&mut tx, id).await?;
    ensure!(
        job.detail.job.state_version == request.expected_version,
        "state_changed"
    );
    require(
        state,
        actor,
        &job,
        match kind {
            JobMutationKind::Cancel => JobAction::Cancel,
            JobMutationKind::Retry => JobAction::Retry,
            JobMutationKind::RetryPublication => JobAction::Publish,
            _ => anyhow::bail!("invalid_operation"),
        },
    )
    .await?;
    if kind != JobMutationKind::Cancel {
        let fenced: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM autonomous_job_project_fences WHERE project_id=?",
        )
        .bind(project)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(fenced == 0, "project_stop_pending");
        ensure!(
            runtime_authorized(state, &job).await?,
            "runtime_allowance_denied"
        );
        if kind == JobMutationKind::Retry {
            let policy = effective_job_policy(&state.db, project).await?;
            ensure!(
                job.detail.runs.len() < policy.policy.limits.attempts as usize,
                "attempts_exhausted"
            );
        }
    }
    let event = match kind {
        JobMutationKind::Cancel => cancel_locked(&mut tx, &mut job, "user_cancelled").await?,
        JobMutationKind::Retry => {
            ensure!(
                matches!(job.detail.job.state, JobState::Failed | JobState::Cancelled)
                    && job.detail.publication.is_none(),
                "retry_not_allowed"
            );
            ensure!(
                !job.dispatched
                    || job
                        .current()
                        .stop_evidence
                        .as_ref()
                        .is_some_and(|e| e.descendants_stopped),
                "stop_unconfirmed"
            );
            remaining(&job)?;
            let run_id = Uuid::new_v4();
            let attempt = job.detail.runs.len() as u32 + 1;
            let epoch = job.current().epoch + 1;
            job.detail.runs.push(JobRun {
                id: run_id,
                job_id: id,
                attempt,
                epoch,
                state: JobState::Queued,
                usage: JobUsage::default(),
                started_at: None,
                ended_at: None,
                worker_version: None,
                candidate_sha: None,
                stop_evidence: None,
                reason: None,
            });
            job.detail.job.current_run_id = run_id;
            job.dispatched = false;
            job.source_seq = 0;
            job.review = None;
            job.transition(JobState::Queued, None);
            Some(
                store::append(
                    &mut tx,
                    &mut job,
                    JobEventKind::PhaseChanged,
                    "Explicit coding retry queued",
                    None,
                    None,
                )
                .await?,
            )
        }
        JobMutationKind::RetryPublication => {
            ensure!(
                matches!(
                    job.detail.job.state,
                    JobState::Blocked | JobState::RecoveryRequired
                ) && valid_review(&job),
                "publication_retry_denied"
            );
            job.transition(JobState::Publishing, None);
            let cmd = Uuid::new_v4();
            let candidate_sha = job
                .review
                .as_ref()
                .and_then(|r| r.candidate_sha.clone())
                .unwrap();
            store::command(
                &mut tx,
                &job,
                cmd,
                JobWorkerCommand::Publish {
                    command_id: cmd,
                    identity: job.identity(),
                    candidate_sha,
                },
            )
            .await?;
            Some(
                store::append(
                    &mut tx,
                    &mut job,
                    JobEventKind::Publication,
                    "Publication reconciliation requested",
                    None,
                    None,
                )
                .await?,
            )
        }
        _ => unreachable!(),
    };
    let r = JobMutationReceipt {
        request_id: request.request_id,
        operation: kind,
        job_id: Some(id),
        run_id: Some(job.detail.job.current_run_id),
        status: JobReceiptStatus::Accepted,
        error_code: None,
        message: None,
    };
    store::save_receipt(&mut tx, actor, project, &fp, &r).await?;
    tx.commit().await?;
    if let Some(event) = event {
        fanout(state, project, &event).await
    }
    Ok(r)
}
async fn answer_inner(
    state: &AppState,
    actor: Uuid,
    project: &str,
    id: Uuid,
    question_id: Uuid,
    request: JobAnswerRequest,
) -> Result<JobMutationReceipt> {
    let old = state
        .db
        .job(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("job_not_found"))?;
    ensure!(old.detail.job.project_id == project, "job_not_found");
    require(state, actor, &old, JobAction::Answer).await?;
    let fp = digest(&("answer", project, id, question_id, &request))?;
    let mut tx = state.db.job_transaction().await?;
    if let Some(r) = store::receipt(&mut tx, actor, request.request_id, &fp).await? {
        tx.commit().await?;
        return Ok(r);
    }
    let mut job = store::load(&mut tx, id).await?;
    ensure!(
        job.detail.job.state == JobState::NeedsInput
            && job.identity().run_id == request.run_id
            && job.identity().epoch == request.epoch,
        "stale_run"
    );
    require(state, actor, &job, JobAction::Answer).await?;
    let q = job
        .detail
        .questions
        .iter_mut()
        .find(|q| q.id == question_id)
        .ok_or_else(|| anyhow::anyhow!("question_not_found"))?;
    ensure!(
        q.version == request.expected_version && q.state == JobQuestionState::Pending,
        "answer_already_reserved"
    );
    validate_answers(q, &request.answers)?;
    q.answers = Some(request.answers.clone());
    q.state = JobQuestionState::AnswerAccepted;
    q.version += 1;
    let dialog_id = q.dialog_id.clone();
    let tool_call_id = q.tool_call_id.clone();
    let cmd = Uuid::new_v4();
    store::command(
        &mut tx,
        &job,
        cmd,
        JobWorkerCommand::Answer {
            command_id: cmd,
            identity: job.identity(),
            question_id,
            dialog_id,
            tool_call_id,
            answers: request.answers,
        },
    )
    .await?;
    let event = store::append(
        &mut tx,
        &mut job,
        JobEventKind::Progress,
        "Answer reserved; awaiting provider confirmation",
        Some(serde_json::json!({"question_id":question_id})),
        None,
    )
    .await?;
    sqlx::query(
        "INSERT INTO autonomous_job_answer_intents(question_id,actor,request_id) VALUES(?,?,?)",
    )
    .bind(question_id.to_string())
    .bind(actor.to_string())
    .bind(request.request_id.to_string())
    .execute(&mut *tx)
    .await?;
    let r = JobMutationReceipt {
        request_id: request.request_id,
        operation: JobMutationKind::Answer,
        job_id: Some(id),
        run_id: Some(request.run_id),
        status: JobReceiptStatus::Accepted,
        error_code: None,
        message: None,
    };
    store::save_receipt(&mut tx, actor, project, &fp, &r).await?;
    tx.commit().await?;
    fanout(state, project, &event).await;
    Ok(r)
}
fn validate_answers(q: &JobQuestion, answers: &[JobQuestionAnswer]) -> Result<()> {
    ensure!(
        answers.len() == q.questions.len() && !answers.is_empty(),
        "incomplete_answer"
    );
    let mut ids = std::collections::HashSet::new();
    for answer in answers {
        ensure!(ids.insert(&answer.id), "duplicate_answer");
        let item = q
            .questions
            .iter()
            .find(|i| i.id == answer.id)
            .ok_or_else(|| anyhow::anyhow!("invalid_question"))?;
        ensure!(
            item.multi || answer.selected_option_ids.len() <= 1,
            "invalid_selection"
        );
        let mut selected = std::collections::HashSet::new();
        for id in &answer.selected_option_ids {
            ensure!(
                selected.insert(id) && item.options.iter().any(|o| &o.id == id),
                "invalid_option"
            );
        }
        ensure!(
            !answer.selected_option_ids.is_empty()
                || answer
                    .custom_input
                    .as_ref()
                    .is_some_and(|v| !v.trim().is_empty()),
            "empty_answer"
        );
        ensure!(
            answer.custom_input.as_ref().map(|s| s.len()).unwrap_or(0) <= 32768,
            "answer_too_large"
        );
    }
    Ok(())
}

fn merge_usage(job: &mut StoredJob, usage: JobUsage) -> Result<()> {
    let old = &job.current().usage;
    ensure!(
        usage.elapsed_seconds >= old.elapsed_seconds
            && usage.input_tokens >= old.input_tokens
            && usage.output_tokens >= old.output_tokens,
        "usage_regressed"
    );
    if let (Some(before), Some(after)) = (old.spend_micro_usd, usage.spend_micro_usd) {
        ensure!(after >= before, "usage_regressed");
    }
    job.current().usage = usage;
    let mut total = JobUsage {
        spend_micro_usd: Some(0),
        ..Default::default()
    };
    for run in &job.detail.runs {
        total.elapsed_seconds = total
            .elapsed_seconds
            .saturating_add(run.usage.elapsed_seconds);
        total.input_tokens = total.input_tokens.saturating_add(run.usage.input_tokens);
        total.output_tokens = total.output_tokens.saturating_add(run.usage.output_tokens);
        total.estimated |= run.usage.estimated;
        total.spend_micro_usd = total
            .spend_micro_usd
            .zip(run.usage.spend_micro_usd)
            .map(|(a, b)| a.saturating_add(b));
    }
    job.detail.job.usage = total;
    Ok(())
}
fn report_identity(report: &JobWorkerReport) -> Option<&JobRunIdentity> {
    match report {
        JobWorkerReport::Accepted { identity, .. }
        | JobWorkerReport::Event { identity, .. }
        | JobWorkerReport::Question { identity, .. }
        | JobWorkerReport::AnswerConfirmed { identity, .. }
        | JobWorkerReport::Settled { identity, .. }
        | JobWorkerReport::Stopped { identity, .. }
        | JobWorkerReport::ArtifactChunk { identity, .. }
        | JobWorkerReport::Verified { identity, .. }
        | JobWorkerReport::Published { identity, .. }
        | JobWorkerReport::PublicationObserved { identity, .. } => Some(identity),
        JobWorkerReport::Rejected { identity, .. } => identity.as_ref(),
        _ => None,
    }
}
fn report_source(report: &JobWorkerReport) -> Option<u64> {
    match report {
        JobWorkerReport::Event { source_seq, .. }
        | JobWorkerReport::Question { source_seq, .. }
        | JobWorkerReport::AnswerConfirmed { source_seq, .. }
        | JobWorkerReport::Settled { source_seq, .. }
        | JobWorkerReport::Stopped { source_seq, .. } => Some(*source_seq),
        _ => None,
    }
}
async fn matched_command(
    tx: &mut store::JobTx<'_>,
    id: Uuid,
    job: &StoredJob,
) -> Result<JobWorkerCommand> {
    let raw: String=sqlx::query_scalar("SELECT document FROM autonomous_job_outbox WHERE id=? AND job_id=? AND owner=? AND machine=?").bind(id.to_string()).bind(job.detail.job.id.to_string()).bind(job.detail.job.cluster_owner_user_id.to_string()).bind(job.detail.job.machine_id.to_string()).fetch_optional(&mut **tx).await?.ok_or_else(||anyhow::anyhow!("unknown_command"))?;
    let command: JobWorkerCommand = serde_json::from_str(&raw)?;
    ensure!(
        command_identity(&command) == Some(&job.identity()),
        "stale_command"
    );
    Ok(command)
}
fn command_identity(command: &JobWorkerCommand) -> Option<&JobRunIdentity> {
    match command {
        JobWorkerCommand::Start { identity, .. }
        | JobWorkerCommand::RenewLease { identity, .. }
        | JobWorkerCommand::Answer { identity, .. }
        | JobWorkerCommand::Cancel { identity, .. }
        | JobWorkerCommand::Verify { identity, .. }
        | JobWorkerCommand::Publish { identity, .. }
        | JobWorkerCommand::RefreshPublication { identity, .. }
        | JobWorkerCommand::PurgeRun { identity, .. } => Some(identity),
        _ => None,
    }
}
pub async fn report(state: &AppState, machine: Uuid, report: JobWorkerReport) -> Result<()> {
    let owner = state
        .sessions
        .daemon_owner(&machine)
        .ok_or_else(|| anyhow::anyhow!("daemon_unregistered"))?;
    ensure!(
        state
            .sessions
            .daemon_supports_capability(&machine, OMP_JOB_WORKER_CAPABILITY),
        "worker_capability_missing"
    );
    match &report {
        JobWorkerReport::Purged {
            command_id,
            identity,
        } => {
            confirm_purge(state, machine, owner, *command_id, identity).await?;
            return Ok(());
        }
        JobWorkerReport::Readiness { readiness } => {
            ensure!(
                readiness.machine_id == machine && readiness.cluster_owner_user_id == owner,
                "wrong_worker"
            );
            ensure!(readiness.profiles.len() <= 64, "readiness_too_large");
            state.jobs.readiness.insert(machine, readiness.clone());
            return Ok(());
        }
        JobWorkerReport::Prepared {
            command_id,
            project_id,
            machine_id,
            cluster_owner_user_id,
            ..
        } => {
            ensure!(
                *machine_id == machine && *cluster_owner_user_id == owner,
                "wrong_worker"
            );
            if let Some(p) = state.jobs.preparations.get(command_id) {
                ensure!(
                    p.0 == owner && p.1 == machine && p.2 == *project_id,
                    "wrong_preparation"
                );
            } else {
                anyhow::bail!("unknown_preparation")
            }
            if let Some((_, (_, _, _, _, sender))) = state.jobs.preparations.remove(command_id) {
                let _ = sender.send(report);
            }
            return Ok(());
        }
        JobWorkerReport::Inventory {
            machine_id,
            cluster_owner_user_id,
            runs,
            ..
        } => {
            ensure!(
                *machine_id == machine && *cluster_owner_user_id == owner && runs.len() <= 1024,
                "wrong_inventory"
            );
            for inventory in runs {
                if let Err(error) = reconcile_inventory(state, machine, owner, inventory).await {
                    tracing::warn!(%machine,%error,"Rejected stale or unknown inventory entry");
                }
            }
            return Ok(());
        }
        JobWorkerReport::Rejected {
            command_id,
            identity: None,
            ..
        } => {
            if let Some(p) = state.jobs.preparations.get(command_id) {
                ensure!(p.0 == owner && p.1 == machine, "wrong_worker");
            }
            if let Some((_, (_, _, _, _, sender))) = state.jobs.preparations.remove(command_id) {
                let _ = sender.send(report);
            }
            return Ok(());
        }
        _ => {}
    }
    let identity = report_identity(&report)
        .ok_or_else(|| anyhow::anyhow!("missing_identity"))?
        .clone();
    ensure!(
        identity.machine_id == machine && identity.cluster_owner_user_id == owner,
        "wrong_worker"
    );
    let mut tx = state.db.job_transaction().await?;
    let mut job = store::load(&mut tx, identity.job_id).await?;
    ensure!(job.identity() == identity, "stale_epoch");
    let source = report_source(&report);
    if let Some(seq) = source {
        if seq <= job.source_seq {
            return Ok(());
        }
        ensure!(seq == job.source_seq + 1, "source_sequence_gap");
        job.source_seq = seq;
    }
    let cancelled = matches!(
        job.detail.job.state,
        JobState::Cancelling | JobState::Cancelled
    );
    let kind: JobEventKind;
    let summary: String;
    let mut event_detail = None;
    match report {
        JobWorkerReport::Accepted { command_id, .. } => {
            let command = matched_command(&mut tx, command_id, &job).await?;
            if let JobWorkerCommand::Start {
                lease_expires_at, ..
            }
            | JobWorkerCommand::RenewLease {
                lease_expires_at, ..
            } = &command
            {
                if job
                    .lease_expires_at
                    .as_ref()
                    .is_none_or(|old| old < lease_expires_at)
                {
                    job.lease_expires_at = Some(lease_expires_at.clone());
                }
            }
            if matches!(command, JobWorkerCommand::Start { .. })
                && !cancelled
                && job.detail.job.state == JobState::Preparing
            {
                job.transition(JobState::Running, None);
            }
            if let JobWorkerCommand::Answer { question_id, .. } = command {
                if let Some(q) =
                    job.detail.questions.iter_mut().find(|q| {
                        q.id == question_id && q.state == JobQuestionState::AnswerAccepted
                    })
                {
                    q.state = JobQuestionState::AnswerDelivered;
                    q.version += 1;
                }
            }
            // Accepted means the host durably owns the operation, not provider confirmation.
            store::acknowledge(&mut tx, command_id).await?;
            kind = JobEventKind::PhaseChanged;
            summary = "Host accepted durable command".into();
        }
        JobWorkerReport::Rejected {
            command_id,
            code,
            message,
            ..
        } => {
            let command = matched_command(&mut tx, command_id, &job).await?;
            store::acknowledge(&mut tx, command_id).await?;
            let refresh = matches!(&command, JobWorkerCommand::RefreshPublication { .. });
            if !cancelled
                && !job.detail.job.state.is_terminal()
                && !matches!(command, JobWorkerCommand::RefreshPublication { .. })
            {
                job.transition(JobState::RecoveryRequired, Some(code));
            }
            if let JobWorkerCommand::Answer { question_id, .. } = command {
                if let Some(q) = job
                    .detail
                    .questions
                    .iter_mut()
                    .find(|q| q.id == question_id)
                {
                    q.state = JobQuestionState::Uncertain;
                    q.version += 1;
                }
            }
            kind = if refresh {
                JobEventKind::Progress
            } else {
                JobEventKind::RecoveryRequired
            };
            summary = message.chars().take(2048).collect();
            if !refresh {
                job.detail.job.attention_count = 1;
            }
        }
        JobWorkerReport::Event {
            event_kind,
            summary: worker_summary,
            detail,
            usage,
            ..
        } => {
            if let Some(usage) = usage {
                merge_usage(&mut job, usage)?;
            }
            if let Some(evidence) = detail
                .as_ref()
                .and_then(|d| d.get("stop_evidence"))
                .and_then(|v| serde_json::from_value::<JobStopEvidence>(v.clone()).ok())
                .filter(|e| {
                    e.descendants_stopped && !e.boot_id.is_empty() && !e.container_id.is_empty()
                })
            {
                job.current().stop_evidence = Some(evidence);
            }
            // Worker-authored phase/success prose cannot create verified state.
            kind = event_kind;
            summary = worker_summary;
            event_detail = detail;
            if event_kind == JobEventKind::RecoveryRequired
                && !cancelled
                && !job.detail.job.state.is_terminal()
            {
                job.transition(
                    JobState::RecoveryRequired,
                    Some("worker_ownership_uncertain".into()),
                );
                job.detail.job.attention_count = 1;
            }
        }
        JobWorkerReport::Question { mut question, .. } => {
            ensure!(
                question.run_id == identity.run_id
                    && question.epoch == identity.epoch
                    && question.state == JobQuestionState::Pending
                    && !question.questions.is_empty()
                    && question.questions.len() <= 64,
                "invalid_question"
            );
            ensure!(
                !job.detail.questions.iter().any(|q| q.id == question.id
                    || (q.run_id == question.run_id && q.dialog_id == question.dialog_id)),
                "duplicate_dialog"
            );
            let mut ids = std::collections::HashSet::new();
            for item in &question.questions {
                ensure!(ids.insert(&item.id), "ambiguous_question");
                let mut options = std::collections::HashSet::new();
                let mut labels = std::collections::HashSet::new();
                for option in &item.options {
                    ensure!(
                        options.insert(&option.id) && labels.insert(&option.label),
                        "ambiguous_option"
                    );
                }
            }
            event_detail = Some(serde_json::json!({"question_id":question.id}));
            if cancelled || job.detail.job.state.is_terminal() {
                question.state = JobQuestionState::Cancelled;
                job.detail.questions.push(question);
                kind = JobEventKind::Progress;
                summary = "Fenced dialog retained without requesting an answer".into();
            } else {
                ensure!(
                    matches!(
                        job.detail.job.state,
                        JobState::Running | JobState::NeedsInput | JobState::RecoveryRequired
                    ),
                    "question_phase_invalid"
                );
                job.detail.questions.push(question);
                job.detail.job.attention_count += 1;
                job.transition(JobState::NeedsInput, None);
                kind = JobEventKind::Question;
                summary = "Decision required".into();
            }
        }
        JobWorkerReport::AnswerConfirmed { question, .. } => {
            let stored = job
                .detail
                .questions
                .iter_mut()
                .find(|q| q.id == question.id)
                .ok_or_else(|| anyhow::anyhow!("unknown_question"))?;
            ensure!(
                stored.run_id == question.run_id
                    && stored.epoch == question.epoch
                    && stored.dialog_id == question.dialog_id
                    && stored.tool_call_id == question.tool_call_id
                    && stored.answers.is_some()
                    && stored.answers == question.answers
                    && question.state == JobQuestionState::Confirmed,
                "answer_confirmation_mismatch"
            );
            stored.state = JobQuestionState::Confirmed;
            stored.confirmed_at = Some(Utc::now().to_rfc3339());
            stored.version += 1;
            sqlx::query("UPDATE autonomous_job_mutations SET document=json_set(document,'$.status','completed') WHERE (actor,request_id) IN (SELECT actor,request_id FROM autonomous_job_answer_intents WHERE question_id=?)").bind(question.id.to_string()).execute(&mut *tx).await?;
            job.detail.job.attention_count = job.detail.job.attention_count.saturating_sub(1);
            if !cancelled
                && !job.detail.questions.iter().any(|q| {
                    q.run_id == identity.run_id
                        && q.state != JobQuestionState::Confirmed
                        && q.state != JobQuestionState::Cancelled
                })
            {
                job.transition(JobState::Running, None);
            }
            event_detail = Some(serde_json::json!({"question_id":question.id}));
            kind = JobEventKind::AnswerConfirmed;
            summary = "Provider confirmed answer".into();
        }
        JobWorkerReport::Settled { usage, .. } => {
            merge_usage(&mut job, usage)?;
            ensure!(
                job.current()
                    .stop_evidence
                    .as_ref()
                    .is_some_and(|e| e.descendants_stopped),
                "worker_freeze_unconfirmed"
            );
            if !cancelled
                && matches!(
                    job.detail.job.state,
                    JobState::Running | JobState::RecoveryRequired
                )
            {
                job.transition(JobState::Verifying, None);
                let id = Uuid::new_v4();
                store::command(
                    &mut tx,
                    &job,
                    id,
                    JobWorkerCommand::Verify {
                        command_id: id,
                        identity: identity.clone(),
                    },
                )
                .await?;
            }
            kind = JobEventKind::PhaseChanged;
            summary = "Worker settled; independent verification required".into();
        }
        JobWorkerReport::Stopped {
            evidence, usage, ..
        } => {
            ensure!(
                evidence.descendants_stopped
                    && !evidence.boot_id.is_empty()
                    && !evidence.container_id.is_empty(),
                "stop_unconfirmed"
            );
            merge_usage(&mut job, usage)?;
            job.current().stop_evidence = Some(evidence);
            job.current().ended_at = Some(Utc::now().to_rfc3339());
            if !job.detail.job.state.is_terminal() {
                job.transition(
                    if cancelled {
                        JobState::Cancelled
                    } else {
                        JobState::Failed
                    },
                    Some(
                        if cancelled {
                            "cancelled"
                        } else {
                            "runtime_stopped"
                        }
                        .into(),
                    ),
                );
            }
            sqlx::query("UPDATE autonomous_job_outbox SET acknowledged=1 WHERE job_id=?")
                .bind(identity.job_id.to_string())
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE autonomous_job_mutations SET document=json_set(document,'$.status','completed') WHERE json_extract(document,'$.job_id')=? AND json_extract(document,'$.operation')='cancel' AND json_extract(document,'$.status')='accepted'").bind(identity.job_id.to_string()).execute(&mut *tx).await?;
            kind = if job.detail.job.state == JobState::Failed {
                JobEventKind::Failed
            } else {
                JobEventKind::Stopped
            };
            summary = "Owned execution positively stopped".into();
        }
        JobWorkerReport::ArtifactChunk {
            artifact,
            offset,
            data_base64,
            final_chunk,
            ..
        } => {
            persist_artifact(&mut tx, &job, artifact, offset, &data_base64, final_chunk).await?;
            tx.commit().await?;
            return Ok(());
        }
        JobWorkerReport::Verified {
            command_id, review, ..
        } => {
            ensure!(
                matches!(
                    matched_command(&mut tx, command_id, &job).await?,
                    JobWorkerCommand::Verify { .. }
                ),
                "wrong_command"
            );
            store::acknowledge(&mut tx, command_id).await?;
            ensure!(
                review.job_id == identity.job_id
                    && review.run_id == identity.run_id
                    && review.base_sha == job.detail.task.base_sha
                    && review.files.len() <= 10000,
                "verification_identity_mismatch"
            );
            if let Some(old) = &job.review {
                ensure!(
                    old.candidate_sha == review.candidate_sha
                        && serde_json::to_value(&old.verification)?
                            == serde_json::to_value(&review.verification)?,
                    "candidate_changed"
                );
                return Ok(());
            }
            job.current().candidate_sha = review.candidate_sha.clone();
            job.review = Some(review);
            if !cancelled && !job.detail.job.state.is_terminal() {
                if valid_review(&job) {
                    let verification = job.review.as_ref().unwrap().verification.as_ref().unwrap();
                    for check in &verification.checks {
                        let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM autonomous_job_artifacts WHERE id=? AND job_id=? AND run_id=? AND complete=1").bind(check.log_artifact_id.unwrap().to_string()).bind(identity.job_id.to_string()).bind(identity.run_id.to_string()).fetch_one(&mut *tx).await?;
                        ensure!(count == 1, "verification_artifact_missing");
                    }
                    if let Some(readiness) = state.jobs.readiness.get(&machine) {
                        ensure!(
                            readiness
                                .profiles
                                .iter()
                                .filter(|p| p.key == job.detail.task.worker_profile)
                                .flat_map(|p| &p.verification_profiles)
                                .any(|v| v.key == job.detail.task.verification_profile
                                    && v.digest == verification.profile_digest
                                    && v.image == verification.image_digest),
                            "verification_image_mismatch"
                        );
                    }
                }
                if valid_review(&job) {
                    let policy = effective_job_policy(&state.db, &identity.project_id).await?;
                    if policy.policy.publish_enabled {
                        job.transition(JobState::Publishing, None);
                        let id = Uuid::new_v4();
                        let candidate_sha = job.current().candidate_sha.clone().unwrap();
                        store::command(
                            &mut tx,
                            &job,
                            id,
                            JobWorkerCommand::Publish {
                                command_id: id,
                                identity: identity.clone(),
                                candidate_sha,
                            },
                        )
                        .await?;
                    } else {
                        job.transition(JobState::Blocked, Some("publication_disabled".into()));
                        job.detail.job.attention_count = 1;
                    }
                } else {
                    job.transition(JobState::Blocked, Some("verification_failed".into()));
                    job.detail.job.attention_count = 1;
                    let id = Uuid::new_v4();
                    store::command(
                        &mut tx,
                        &job,
                        id,
                        JobWorkerCommand::Cancel {
                            command_id: id,
                            identity: identity.clone(),
                            reason: "verification_failed".into(),
                        },
                    )
                    .await?;
                }
            }
            kind = JobEventKind::Verification;
            summary = "Independent verification evidence retained".into();
        }
        JobWorkerReport::PublicationObserved {
            command_id,
            publication,
            ..
        } => {
            ensure!(
                matches!(
                    matched_command(&mut tx, command_id, &job).await?,
                    JobWorkerCommand::RefreshPublication { .. }
                ),
                "wrong_command"
            );
            store::acknowledge(&mut tx, command_id).await?;
            if let Some(publication) = publication {
                let old = job
                    .detail
                    .publication
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("publication_not_found"))?;
                ensure!(
                    old.repository == publication.repository
                        && old.number == publication.number
                        && old.branch == publication.branch
                        && old.head_sha == publication.head_sha
                        && old.base_sha == publication.base_sha
                        && publication.url.starts_with("https://"),
                    "publication_conflict"
                );
                if old.observed_at > publication.observed_at {
                    return Ok(());
                }
                job.detail.publication = Some(publication.clone());
                if let Some(review) = job.review.as_mut() {
                    review.publication = Some(publication);
                }
                summary = "External pull request disposition refreshed".into();
            } else {
                summary = "External pull request could not be located; retained evidence unchanged"
                    .into();
            }
            kind = JobEventKind::Progress;
        }
        JobWorkerReport::Published {
            command_id,
            publication,
            ..
        } => {
            ensure!(
                matches!(
                    matched_command(&mut tx, command_id, &job).await?,
                    JobWorkerCommand::Publish { .. }
                ),
                "wrong_command"
            );
            store::acknowledge(&mut tx, command_id).await?;
            ensure!(
                valid_review(&job)
                    && publication.repository == job.detail.task.repository
                    && publication.base_sha == job.detail.task.base_sha
                    && Some(&publication.head_sha) == job.current().candidate_sha.as_ref()
                    && publication.branch == format!("apas/job-{}", identity.job_id)
                    && publication.number > 0
                    && publication.draft
                    && publication.url.starts_with("https://"),
                "publication_identity_mismatch"
            );
            if let Some(old) = &job.detail.publication {
                ensure!(
                    old.repository == publication.repository && old.number == publication.number,
                    "publication_conflict"
                );
                return Ok(());
            }
            job.detail.publication = Some(publication.clone());
            if let Some(review) = job.review.as_mut() {
                review.publication = Some(publication);
            }
            if !cancelled {
                ensure!(
                    job.current()
                        .stop_evidence
                        .as_ref()
                        .is_some_and(|e| e.descendants_stopped),
                    "stop_unconfirmed"
                );
                job.transition(JobState::ReviewReady, None);
                job.current().ended_at = Some(Utc::now().to_rfc3339());
                job.detail.job.attention_count = 1;
            }
            kind = JobEventKind::Publication;
            summary = "Verified draft publication retained".into();
        }
        _ => anyhow::bail!("unexpected_report"),
    }
    let project = job.detail.job.project_id.clone();
    let event = store::append(&mut tx, &mut job, kind, &summary, event_detail, source).await?;
    tx.commit().await?;
    fanout(state, &project, &event).await;
    Ok(())
}
async fn persist_artifact(
    tx: &mut store::JobTx<'_>,
    job: &StoredJob,
    mut artifact: JobArtifact,
    offset: u64,
    encoded: &str,
    final_chunk: bool,
) -> Result<()> {
    use base64::Engine;
    ensure!(
        artifact.run_id == job.identity().run_id
            && encoded.len() <= JOB_MAX_FRAME_BYTES
            && artifact.size_bytes <= job.detail.job.limits.output_bytes.min(100 * 1024 * 1024),
        "artifact_limit"
    );
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
    ensure!(
        offset.saturating_add(bytes.len() as u64) <= artifact.size_bytes,
        "artifact_size_mismatch"
    );
    let row =
        sqlx::query("SELECT manifest,content,complete FROM autonomous_job_artifacts WHERE id=?")
            .bind(artifact.id.to_string())
            .fetch_optional(&mut **tx)
            .await?;
    let mut content = Vec::new();
    if let Some(row) = row {
        let old: JobArtifact = serde_json::from_str(row.get("manifest"))?;
        ensure!(
            old.run_id == artifact.run_id
                && old.sha256 == artifact.sha256
                && old.size_bytes == artifact.size_bytes
                && old.kind == artifact.kind
                && old.content_type == artifact.content_type
                && old.truncated == artifact.truncated,
            "artifact_conflict"
        );
        content = row.get("content");
    }
    if offset < content.len() as u64 {
        let end = offset as usize + bytes.len();
        ensure!(
            end <= content.len() && content[offset as usize..end] == bytes,
            "artifact_conflict"
        );
        if final_chunk {
            ensure!(
                content.len() as u64 == artifact.size_bytes
                    && format!("{:x}", Sha256::digest(&content)) == artifact.sha256,
                "artifact_digest_mismatch"
            );
            sqlx::query("UPDATE autonomous_job_artifacts SET complete=1 WHERE id=?")
                .bind(artifact.id.to_string())
                .execute(&mut **tx)
                .await?;
        }
        return Ok(());
    }
    ensure!(offset == content.len() as u64, "artifact_offset_gap");
    let total: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(length(content)),0) FROM autonomous_job_artifacts WHERE run_id=?",
    )
    .bind(artifact.run_id.to_string())
    .fetch_one(&mut **tx)
    .await?;
    ensure!(
        (total as u64).saturating_add(bytes.len() as u64)
            <= job.detail.job.limits.output_bytes.min(100 * 1024 * 1024),
        "artifact_budget_exhausted"
    );
    let event_bytes: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(length(document)),0) FROM autonomous_job_events WHERE run_id=?",
    )
    .bind(artifact.run_id.to_string())
    .fetch_one(&mut **tx)
    .await?;
    if (total as u64)
        .saturating_add(bytes.len() as u64)
        .saturating_add(event_bytes as u64)
        > job.detail.job.limits.output_bytes.min(100 * 1024 * 1024)
    {
        let budget = job
            .detail
            .job
            .limits
            .output_bytes
            .min(100 * 1024 * 1024)
            .saturating_sub(total as u64 + bytes.len() as u64);
        sqlx::query("DELETE FROM autonomous_job_events WHERE run_id=? AND seq IN (SELECT seq FROM (SELECT seq,SUM(length(document)) OVER(ORDER BY seq DESC) AS retained FROM autonomous_job_events WHERE run_id=?) WHERE retained>?)").bind(artifact.run_id.to_string()).bind(artifact.run_id.to_string()).bind(budget as i64).execute(&mut **tx).await?;
    }
    content.extend_from_slice(&bytes);
    if final_chunk {
        ensure!(
            content.len() as u64 == artifact.size_bytes
                && format!("{:x}", Sha256::digest(&content)) == artifact.sha256,
            "artifact_digest_mismatch"
        );
    }
    artifact.expires_at = job
        .detail
        .job
        .state
        .is_terminal()
        .then(|| (Utc::now() + Duration::days(30)).to_rfc3339());
    sqlx::query("INSERT INTO autonomous_job_artifacts(id,job_id,run_id,manifest,content,complete) VALUES(?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET content=excluded.content,complete=excluded.complete")
        .bind(artifact.id.to_string()).bind(job.detail.job.id.to_string()).bind(artifact.run_id.to_string()).bind(serde_json::to_string(&artifact)?).bind(content).bind(final_chunk).execute(&mut **tx).await?;
    Ok(())
}
async fn reconcile_inventory(
    state: &AppState,
    machine: Uuid,
    owner: Uuid,
    inventory: &JobRunInventory,
) -> Result<()> {
    ensure!(
        inventory.identity.machine_id == machine
            && inventory.identity.cluster_owner_user_id == owner,
        "wrong_inventory_owner"
    );
    let mut tx = state.db.job_transaction().await?;
    let mut job = store::load(&mut tx, inventory.identity.job_id).await?;
    ensure!(job.identity() == inventory.identity, "stale_inventory");
    if job.detail.job.state.is_terminal() {
        return Ok(());
    }
    merge_usage(&mut job, inventory.usage.clone())?;
    if let Some(evidence) = inventory
        .stop_evidence
        .as_ref()
        .filter(|e| e.descendants_stopped && !e.boot_id.is_empty() && !e.container_id.is_empty())
    {
        job.current().stop_evidence = Some(evidence.clone());
        if matches!(inventory.state, JobState::Cancelled | JobState::Failed) {
            job.current().ended_at = Some(evidence.stopped_at.clone());
            job.transition(
                if job.detail.job.state == JobState::Cancelling {
                    JobState::Cancelled
                } else {
                    JobState::Failed
                },
                Some(evidence.reason.clone()),
            );
        }
    } else if !inventory.prompt_confirmed && job.detail.job.state != JobState::Cancelling {
        job.transition(
            JobState::RecoveryRequired,
            Some("prompt_admission_uncertain".into()),
        );
    } else if job.detail.job.state == JobState::RecoveryRequired
        && inventory.prompt_confirmed
        && matches!(
            inventory.state,
            JobState::Running | JobState::NeedsInput | JobState::Verifying | JobState::Publishing
        )
    {
        job.transition(inventory.state, None);
    }
    let project = job.detail.job.project_id.clone();
    let event = store::append(
        &mut tx,
        &mut job,
        JobEventKind::PhaseChanged,
        "Recorded host inventory reconciled",
        None,
        None,
    )
    .await?;
    tx.commit().await?;
    fanout(state, &project, &event).await;
    Ok(())
}

pub async fn cancel_project_jobs(state: &AppState, project: &str, reason: &str) -> Result<()> {
    let mut tx = state.db.job_transaction().await?;
    sqlx::query("INSERT INTO autonomous_job_project_fences(project_id,reason) VALUES(?,?) ON CONFLICT(project_id) DO UPDATE SET reason=excluded.reason").bind(project).bind(reason).execute(&mut *tx).await?;
    let rows=sqlx::query("SELECT document FROM autonomous_jobs WHERE project_id=? AND state NOT IN ('review_ready','cancelled','failed')").bind(project).fetch_all(&mut *tx).await?;
    let mut events = Vec::new();
    for row in rows {
        let mut job: StoredJob = serde_json::from_str(row.get("document"))?;
        if let Some(event) = cancel_locked(&mut tx, &mut job, reason).await? {
            events.push(event);
        }
    }
    tx.commit().await?;
    for event in events {
        fanout(state, project, &event).await;
    }
    Ok(())
}
pub async fn project_jobs_stopped(state: &AppState, project: &str) -> Result<bool> {
    let deleting = state
        .db
        .get_project(project)
        .await?
        .is_some_and(|p| p.lifecycle() == crate::db::ProjectLifecycle::Deleting);
    let mut tx = state.db.job_transaction().await?;
    let rows = sqlx::query("SELECT document FROM autonomous_jobs WHERE project_id=?")
        .bind(project)
        .fetch_all(&mut *tx)
        .await?;
    let jobs: Vec<StoredJob> = rows
        .into_iter()
        .map(|r| serde_json::from_str(r.get("document")))
        .collect::<Result<_, _>>()?;
    if !jobs.iter().all(|j| {
        j.detail.job.state.is_terminal()
            && j.detail.runs.iter().all(|r| {
                r.started_at.is_none()
                    || r.stop_evidence
                        .as_ref()
                        .is_some_and(|e| e.descendants_stopped)
            })
    }) {
        return Ok(false);
    }
    let mut complete = true;
    if deleting {
        for job in &jobs {
            for run in job.detail.runs.iter().filter(|r| r.started_at.is_some()) {
                let status: Option<i64> = sqlx::query_scalar(
                    "SELECT confirmed FROM autonomous_job_purges WHERE run_id=?",
                )
                .bind(run.id.to_string())
                .fetch_optional(&mut *tx)
                .await?;
                if status != Some(1) {
                    complete = false;
                }
                if status.is_none() {
                    let command_id = Uuid::new_v4();
                    let mut identity = job.identity();
                    identity.run_id = run.id;
                    identity.epoch = run.epoch;
                    store::command(
                        &mut tx,
                        job,
                        command_id,
                        JobWorkerCommand::PurgeRun {
                            command_id,
                            identity,
                        },
                    )
                    .await?;
                    sqlx::query("INSERT INTO autonomous_job_purges(run_id,command_id) VALUES(?,?)")
                        .bind(run.id.to_string())
                        .bind(command_id.to_string())
                        .execute(&mut *tx)
                        .await?;
                }
            }
        }
    }
    tx.commit().await?;
    Ok(complete)
}
async fn runtime_authorized(state: &AppState, job: &StoredJob) -> Result<bool> {
    if !authorized(state, job.initiator, job, JobAction::Admit)
        .await
        .unwrap_or(false)
    {
        return Ok(false);
    }
    let p = match effective_job_policy(&state.db, &job.detail.job.project_id).await {
        Ok(p) => p.policy,
        Err(_) => return Ok(false),
    };
    let task = &job.detail.task;
    let usage = &job.detail.job.usage;
    let approved = &job.detail.job.limits;
    Ok(p.enabled
        && p.worker_profiles.contains(&task.worker_profile)
        && p.model_profiles.contains(&task.model_profile)
        && p.verification_profiles.contains(&task.verification_profile)
        && approved.cpu_millis <= p.limits.cpu_millis
        && approved.memory_bytes <= p.limits.memory_bytes
        && approved.workspace_bytes <= p.limits.workspace_bytes
        && approved.output_bytes <= p.limits.output_bytes
        && approved.pids <= p.limits.pids
        && approved.model_concurrency <= p.limits.model_concurrency
        && approved.model_output_tokens <= p.limits.model_output_tokens
        && usage.elapsed_seconds
            < job
                .detail
                .job
                .limits
                .elapsed_seconds
                .min(p.limits.elapsed_seconds)
        && usage
            .spend_micro_usd
            .map(|s| {
                s < job
                    .detail
                    .job
                    .limits
                    .spend_micro_usd
                    .min(p.limits.spend_micro_usd)
            })
            .unwrap_or(!job.dispatched || job.current_read().usage.input_tokens == 0)
        && job.detail.runs.len() <= p.limits.attempts as usize)
}
impl StoredJob {
    fn current_read(&self) -> &JobRun {
        self.detail.runs.last().expect("persisted job has run")
    }
}
pub async fn reconcile_authority(state: &AppState) -> Result<()> {
    for old in state.db.all_jobs().await? {
        if old.detail.job.state.is_terminal() || old.detail.job.state == JobState::Cancelling {
            continue;
        }
        if !runtime_authorized(state, &old).await? {
            let mut tx = state.db.job_transaction().await?;
            let mut job = store::load(&mut tx, old.detail.job.id).await?;
            let event = cancel_locked(&mut tx, &mut job, "runtime_authority_revoked").await?;
            tx.commit().await?;
            if let Some(event) = event {
                fanout(state, &old.detail.job.project_id, &event).await;
            }
        }
    }
    reconcile_subscriptions(state).await
}
pub async fn reconcile_subscriptions(state: &AppState) -> Result<()> {
    let subscriptions: Vec<_> = state
        .jobs
        .subscriptions
        .iter()
        .map(|s| (*s.key(), s.value().clone()))
        .collect();
    for ((connection, job), project) in subscriptions {
        let permitted = connection_actor(state, connection).await.is_some()
            && if let Some(actor) = state.sessions.get_web_user(&connection) {
                detail(state, actor, &project, job).await.is_ok()
            } else {
                false
            };
        if !permitted {
            state.jobs.subscriptions.remove(&(connection, job));
            reject_subscription(state, connection, &project, job, "job_access_denied", false).await;
        }
    }
    Ok(())
}
pub async fn recover(state: &AppState) -> Result<()> {
    for old in state.db.all_jobs().await? {
        if old.dispatched
            && !old.detail.job.state.is_terminal()
            && old.detail.job.state != JobState::Cancelling
        {
            let mut tx = state.db.job_transaction().await?;
            let mut job = store::load(&mut tx, old.detail.job.id).await?;
            job.transition(
                JobState::RecoveryRequired,
                Some("awaiting_recorded_host_inventory".into()),
            );
            store::append(
                &mut tx,
                &mut job,
                JobEventKind::RecoveryRequired,
                "Server restarted; reconciling exact recorded host",
                None,
                None,
            )
            .await?;
            tx.commit().await?;
        }
    }
    Ok(())
}
pub async fn daemon_connected(state: &AppState, machine: Uuid) {
    state.jobs.readiness.remove(&machine);
    if state
        .sessions
        .daemon_supports_capability(&machine, OMP_JOB_WORKER_CAPABILITY)
    {
        state
            .sessions
            .send_to_daemon(
                &machine,
                ServerToDaemon::AutonomousJob {
                    command: JobWorkerCommand::Inventory {
                        command_id: Uuid::new_v4(),
                    },
                },
            )
            .await;
    }
}
fn ready_for_job(state: &AppState, job: &StoredJob) -> bool {
    state
        .sessions
        .is_daemon_connected(&job.detail.job.machine_id)
        && state.sessions.daemon_owner(&job.detail.job.machine_id)
            == Some(job.detail.job.cluster_owner_user_id)
        && state
            .jobs
            .readiness
            .get(&job.detail.job.machine_id)
            .is_some_and(|r| {
                fresh(&r)
                    && r.cluster_owner_user_id == job.detail.job.cluster_owner_user_id
                    && r.profiles.iter().any(|p| {
                        p.ready
                            && p.key == job.detail.task.worker_profile
                            && p.digest == job.detail.task.worker_profile_digest
                            && p.verification_profiles.iter().any(|v| {
                                v.key == job.detail.task.verification_profile
                                    && v.digest == job.detail.task.verification_profile_digest
                            })
                    })
            })
}
fn run_limits(job: &StoredJob, policy: &JobPolicy) -> JobLimits {
    let mut limits = job.detail.job.limits.narrowed_by(&policy.limits);
    let past_time: u64 = job
        .detail
        .runs
        .iter()
        .filter(|r| r.id != job.detail.job.current_run_id)
        .map(|r| r.usage.elapsed_seconds)
        .sum();
    let past_spend: u64 = job
        .detail
        .runs
        .iter()
        .filter(|r| r.id != job.detail.job.current_run_id)
        .filter_map(|r| r.usage.spend_micro_usd)
        .sum();
    limits.elapsed_seconds = limits.elapsed_seconds.saturating_sub(past_time);
    limits.spend_micro_usd = limits.spend_micro_usd.saturating_sub(past_spend);
    limits
}
async fn dispatch_tick(state: &AppState) -> Result<()> {
    reconcile_authority(state).await?;
    state.db.clear_stopped_job_fences().await?;
    for (project, event) in state.db.pending_job_notifications().await? {
        if let Err(error) = deliver_notification(state, &project, &event).await {
            tracing::warn!(%error,"Durable job notification deferred");
        }
    }
    for old in state.db.all_jobs().await? {
        if old.detail.job.state.is_terminal() || old.detail.job.state == JobState::Cancelling {
            continue;
        }
        if !runtime_authorized(state, &old).await? {
            continue;
        }
        let policy = effective_job_policy(&state.db, &old.detail.job.project_id).await?;
        if !old.dispatched && matches!(old.detail.job.state, JobState::Queued | JobState::Blocked) {
            let ready = ready_for_job(state, &old);
            let mut tx = state.db.job_transaction().await?;
            let mut job = store::load(&mut tx, old.detail.job.id).await?;
            if job.dispatched
                || !matches!(job.detail.job.state, JobState::Queued | JobState::Blocked)
            {
                continue;
            }
            let fenced: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM autonomous_job_project_fences WHERE project_id=?",
            )
            .bind(&job.detail.job.project_id)
            .fetch_one(&mut *tx)
            .await?;
            if fenced > 0 {
                continue;
            }
            let counts=sqlx::query("SELECT COALESCE(SUM(project_id=?),0) AS project_count,COALESCE(SUM(owner=? AND machine=?),0) AS machine_count,COALESCE(SUM(owner=?),0) AS cluster_count FROM autonomous_jobs WHERE json_extract(document,'$.dispatched')=1 AND state NOT IN ('review_ready','cancelled','failed')")
                .bind(&job.detail.job.project_id).bind(job.detail.job.cluster_owner_user_id.to_string()).bind(job.detail.job.machine_id.to_string()).bind(job.detail.job.cluster_owner_user_id.to_string()).fetch_one(&mut *tx).await?;
            let earlier:i64=sqlx::query_scalar("SELECT COUNT(*) FROM autonomous_jobs WHERE owner=? AND machine=? AND json_extract(document,'$.dispatched')=0 AND state IN ('queued','blocked') AND (created_at<? OR (created_at=? AND id<?))")
                .bind(job.detail.job.cluster_owner_user_id.to_string()).bind(job.detail.job.machine_id.to_string()).bind(&job.detail.job.created_at).bind(&job.detail.job.created_at).bind(job.detail.job.id.to_string()).fetch_one(&mut *tx).await?;
            let capacity = earlier == 0
                && counts.get::<i64, _>("project_count") < policy.policy.max_active_project as i64
                && counts.get::<i64, _>("machine_count") < policy.policy.max_active_machine as i64
                && counts.get::<i64, _>("cluster_count") < policy.policy.max_active_cluster as i64;
            if !ready || !capacity {
                let reason = if !ready {
                    "target_readiness_unavailable"
                } else {
                    "capacity_wait"
                };
                if job.detail.job.reason.as_deref() != Some(reason) {
                    job.transition(
                        if ready {
                            JobState::Queued
                        } else {
                            JobState::Blocked
                        },
                        Some(reason.into()),
                    );
                    let event = store::append(
                        &mut tx,
                        &mut job,
                        JobEventKind::PhaseChanged,
                        "Waiting for approved target readiness or capacity",
                        None,
                        None,
                    )
                    .await?;
                    tx.commit().await?;
                    fanout(state, &job.detail.job.project_id, &event).await;
                }
                continue;
            }
            job.dispatched = true;
            job.transition(JobState::Preparing, None);
            job.current().started_at = Some(Utc::now().to_rfc3339());
            let expires = (Utc::now() + Duration::seconds(JOB_LEASE_SECONDS as i64)).to_rfc3339();
            job.lease_expires_at = Some(expires.clone());
            job.last_renewed_at = Some(Utc::now().to_rfc3339());
            let cmd = Uuid::new_v4();
            store::command(
                &mut tx,
                &job,
                cmd,
                JobWorkerCommand::Start {
                    command_id: cmd,
                    identity: job.identity(),
                    task: job.detail.task.clone(),
                    limits: run_limits(&job, &policy.policy),
                    policy_digest: policy.digest,
                    lease_expires_at: expires,
                },
            )
            .await?;
            let event = store::append(
                &mut tx,
                &mut job,
                JobEventKind::PhaseChanged,
                "Capacity reserved; dispatch recorded",
                None,
                None,
            )
            .await?;
            tx.commit().await?;
            fanout(state, &job.detail.job.project_id, &event).await;
        } else if old.dispatched && old.detail.job.state != JobState::RecoveryRequired {
            let lease_expired = old
                .lease_expires_at
                .as_deref()
                .is_some_and(|s| s < Utc::now().to_rfc3339().as_str());
            if lease_expired {
                let mut tx = state.db.job_transaction().await?;
                let mut job = store::load(&mut tx, old.detail.job.id).await?;
                if !job.detail.job.state.is_terminal()
                    && job.detail.job.state != JobState::Cancelling
                {
                    job.transition(
                        JobState::RecoveryRequired,
                        Some("lease_expired_stop_unconfirmed".into()),
                    );
                    let event = store::append(
                        &mut tx,
                        &mut job,
                        JobEventKind::RecoveryRequired,
                        "Execution lease expired; positive stop evidence still required",
                        None,
                        None,
                    )
                    .await?;
                    tx.commit().await?;
                    fanout(state, &job.detail.job.project_id, &event).await;
                    state
                        .sessions
                        .send_to_daemon(
                            &old.detail.job.machine_id,
                            ServerToDaemon::AutonomousJob {
                                command: JobWorkerCommand::Inventory {
                                    command_id: Uuid::new_v4(),
                                },
                            },
                        )
                        .await;
                }
                continue;
            }
            if !state
                .sessions
                .is_daemon_connected(&old.detail.job.machine_id)
            {
                continue;
            }
            let due = old
                .last_renewed_at
                .as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .is_none_or(|d| {
                    d <= Utc::now() - Duration::seconds(JOB_LEASE_RENEW_SECONDS as i64)
                });
            if due {
                let mut tx = state.db.job_transaction().await?;
                let mut job = store::load(&mut tx, old.detail.job.id).await?;
                if job.detail.job.state.is_terminal()
                    || job.detail.job.state == JobState::Cancelling
                {
                    continue;
                }
                sqlx::query("UPDATE autonomous_job_outbox SET acknowledged=1 WHERE job_id=? AND json_extract(document,'$.operation')='renew_lease'").bind(job.detail.job.id.to_string()).execute(&mut *tx).await?;
                let expires =
                    (Utc::now() + Duration::seconds(JOB_LEASE_SECONDS as i64)).to_rfc3339();
                job.last_renewed_at = Some(Utc::now().to_rfc3339());
                let cmd = Uuid::new_v4();
                store::command(
                    &mut tx,
                    &job,
                    cmd,
                    JobWorkerCommand::RenewLease {
                        command_id: cmd,
                        identity: job.identity(),
                        lease_expires_at: expires,
                        limits: run_limits(&job, &policy.policy),
                    },
                )
                .await?;
                store::save(&mut tx, &job).await?;
                tx.commit().await?;
            }
        }
    }
    for project in state.db.list_deleting_project_ids().await? {
        project_jobs_stopped(state, &project).await?;
    }
    for (_, owner, machine, command) in state.db.job_outbox().await? {
        if state.sessions.daemon_owner(&machine) != Some(owner)
            || !state
                .sessions
                .daemon_supports_capability(&machine, OMP_JOB_WORKER_CAPABILITY)
        {
            continue;
        }
        if outbound_allowed(state, machine, &command).await? {
            state
                .sessions
                .send_to_daemon(&machine, ServerToDaemon::AutonomousJob { command })
                .await;
        }
    }
    Ok(())
}
/// Checked both before queueing and immediately before the socket write.
pub async fn outbound_allowed(
    state: &AppState,
    machine: Uuid,
    command: &JobWorkerCommand,
) -> Result<bool> {
    if let JobWorkerCommand::Prepare {
        command_id,
        project_id,
        machine_id,
        cluster_owner_user_id,
        ..
    } = command
    {
        let actor = state.jobs.preparations.get(command_id).map(|p| p.3);
        let Some(actor) = actor else { return Ok(false) };
        return authorize_job_action(
            state,
            actor,
            project_id,
            *cluster_owner_user_id,
            *machine_id,
            actor,
            JobAction::Admit,
        )
        .await;
    }
    let Some(identity) = command_identity(command) else {
        return Ok(true);
    };
    let Some(job) = state.db.job(identity.job_id).await? else {
        return Ok(false);
    };
    if matches!(command, JobWorkerCommand::PurgeRun { .. }) {
        return Ok(historical_identity(&job, identity)
            && machine == identity.machine_id
            && state.sessions.daemon_owner(&machine) == Some(identity.cluster_owner_user_id)
            && state
                .db
                .get_project(&identity.project_id)
                .await?
                .is_some_and(|p| p.lifecycle() == crate::db::ProjectLifecycle::Deleting)
            && job
                .detail
                .runs
                .iter()
                .find(|r| r.id == identity.run_id)
                .is_some_and(|r| {
                    r.state.is_terminal()
                        && r.stop_evidence
                            .as_ref()
                            .is_some_and(|e| e.descendants_stopped)
                }));
    }
    if job.identity() != *identity
        || machine != identity.machine_id
        || state.sessions.daemon_owner(&machine) != Some(identity.cluster_owner_user_id)
    {
        return Ok(false);
    }
    if let JobWorkerCommand::RefreshPublication { command_id, .. } = command {
        let mut tx = state.db.job_transaction().await?;
        let actor: Option<String> = sqlx::query_scalar(
            "SELECT actor FROM autonomous_job_review_refreshes WHERE command_id=?",
        )
        .bind(command_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        let Some(actor) = actor.and_then(|a| Uuid::parse_str(&a).ok()) else {
            return Ok(false);
        };
        return authorized(state, actor, &job, JobAction::Read).await;
    }
    if matches!(command, JobWorkerCommand::Cancel { .. }) {
        return Ok(!job.detail.job.state.is_terminal());
    }
    if job.detail.job.state.is_terminal()
        || job.detail.job.state == JobState::Cancelling
        || !runtime_authorized(state, &job).await?
    {
        return Ok(false);
    }
    if let JobWorkerCommand::Start {
        lease_expires_at, ..
    }
    | JobWorkerCommand::RenewLease {
        lease_expires_at, ..
    } = command
    {
        if !chrono::DateTime::parse_from_rfc3339(lease_expires_at).is_ok_and(|d| d > Utc::now()) {
            return Ok(false);
        }
    }
    if matches!(command, JobWorkerCommand::Start { .. }) && !ready_for_job(state, &job) {
        return Ok(false);
    }
    if matches!(command, JobWorkerCommand::Publish { .. }) {
        let policy = effective_job_policy(&state.db, &identity.project_id).await?;
        return Ok(policy.policy.publish_enabled
            && valid_review(&job)
            && authorized(state, job.initiator, &job, JobAction::Publish).await?);
    }
    Ok(true)
}
pub async fn run_dispatcher(state: AppState) {
    let mut ticks = 0u64;
    loop {
        if let Err(error) = dispatch_tick(&state).await {
            tracing::warn!(%error,"Job dispatcher reconciliation failed");
        }
        ticks += 1;
        if ticks % 720 == 0 {
            if let Err(error) = state.db.retain_job_content().await {
                tracing::warn!(%error,"Job retention failed");
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}
async fn reject_subscription(
    state: &AppState,
    connection: Uuid,
    project: &str,
    job: Uuid,
    code: &str,
    resync: bool,
) {
    state
        .sessions
        .send_to_web(
            &connection,
            ServerToWeb::JobSubscriptionRejected {
                project_id: project.into(),
                job_id: job,
                code: code.into(),
                message: code.into(),
                resync_required: resync,
            },
        )
        .await;
}
pub async fn subscribe(state: &AppState, connection: Uuid, project: String, job: Uuid, after: u64) {
    if !state
        .sessions
        .web_supports_capability(&connection, AUTONOMOUS_JOBS_CAPABILITY)
    {
        return;
    }
    let Some(actor) = connection_actor(state, connection).await else {
        return;
    };
    let snapshot = match detail(state, actor, &project, job).await {
        Ok(d) => d,
        Err(_) => {
            reject_subscription(state, connection, &project, job, "job_access_denied", false).await;
            return;
        }
    };
    state
        .jobs
        .subscriptions
        .insert((connection, job), project.clone());
    state
        .sessions
        .send_to_web(&connection, ServerToWeb::JobSnapshot { detail: snapshot })
        .await;
    match state.db.job_events(job, after).await {
        Ok(replay) if !replay.resync_required => {
            for event in replay.events {
                if detail(state, actor, &project, job).await.is_err() {
                    state.jobs.subscriptions.remove(&(connection, job));
                    reject_subscription(
                        state,
                        connection,
                        &project,
                        job,
                        "job_access_denied",
                        false,
                    )
                    .await;
                    return;
                }
                state
                    .sessions
                    .send_to_web(&connection, ServerToWeb::JobEvent { event })
                    .await;
            }
            if replay.has_more {
                reject_subscription(
                    state,
                    connection,
                    &project,
                    job,
                    "replay_page_required",
                    true,
                )
                .await;
            }
        }
        _ => {
            reject_subscription(
                state,
                connection,
                &project,
                job,
                "snapshot_resync_required",
                true,
            )
            .await;
        }
    }
}
async fn fanout(state: &AppState, project: &str, event: &JobEvent) {
    let connections: Vec<_> = state
        .jobs
        .subscriptions
        .iter()
        .filter(|s| s.key().1 == event.job_id)
        .map(|s| s.key().0)
        .collect();
    for connection in connections {
        let permitted = if let Some(actor) = connection_actor(state, connection).await {
            detail(state, actor, project, event.job_id).await.is_ok()
        } else {
            false
        };
        if !permitted {
            state.jobs.subscriptions.remove(&(connection, event.job_id));
            reject_subscription(
                state,
                connection,
                project,
                event.job_id,
                "job_access_denied",
                false,
            )
            .await;
            continue;
        }
        state
            .sessions
            .send_to_web(
                &connection,
                ServerToWeb::JobEvent {
                    event: event.clone(),
                },
            )
            .await;
        if let Ok(Some(job)) = state.db.job(event.job_id).await {
            state
                .sessions
                .send_to_web(
                    &connection,
                    ServerToWeb::JobChanged {
                        project_id: project.into(),
                        job_id: event.job_id,
                        state_version: job.detail.job.state_version,
                        last_event_seq: job.detail.high_watermark,
                    },
                )
                .await;
        }
    }
    let inbox: Vec<_> = state
        .jobs
        .connection_auth
        .iter()
        .map(|c| *c.key())
        .collect();
    if let Ok(Some(job)) = state.db.job(event.job_id).await {
        for connection in inbox {
            if !state
                .sessions
                .web_supports_capability(&connection, AUTONOMOUS_JOBS_CAPABILITY)
            {
                continue;
            }
            if let Some(actor) = connection_actor(state, connection).await {
                if authorized(state, actor, &job, JobAction::Read)
                    .await
                    .unwrap_or(false)
                {
                    state
                        .sessions
                        .send_to_web(
                            &connection,
                            ServerToWeb::JobChanged {
                                project_id: project.into(),
                                job_id: event.job_id,
                                state_version: job.detail.job.state_version,
                                last_event_seq: job.detail.high_watermark,
                            },
                        )
                        .await;
                }
            }
        }
    }
    if let Err(error) = deliver_notification(state, project, event).await {
        tracing::warn!(%error,"Job notification enqueue deferred");
    }
}

async fn connection_actor(state: &AppState, connection: Uuid) -> Option<Uuid> {
    let binding = state
        .jobs
        .connection_auth
        .get(&connection)
        .map(|v| v.clone())?;
    if binding.2 <= Utc::now().timestamp() as usize
        || state.sessions.get_web_user(&connection) != Some(binding.0)
    {
        return None;
    }
    if let Some(device) = binding.1 {
        if !state
            .db
            .is_mobile_device_session_active(&device, &binding.0.to_string())
            .await
            .unwrap_or(false)
        {
            return None;
        }
    }
    Some(binding.0)
}
pub async fn web_outbound_allowed(
    state: &AppState,
    connection: Uuid,
    message: &ServerToWeb,
) -> bool {
    let job = match message {
        ServerToWeb::JobSnapshot { detail } => detail.job.id,
        ServerToWeb::JobEvent { event } => event.job_id,
        ServerToWeb::JobChanged { job_id, .. } => *job_id,
        ServerToWeb::JobSubscriptionRejected { .. } => {
            return state
                .sessions
                .web_supports_capability(&connection, AUTONOMOUS_JOBS_CAPABILITY)
        }
        _ => return true,
    };
    if !state
        .sessions
        .web_supports_capability(&connection, AUTONOMOUS_JOBS_CAPABILITY)
    {
        return false;
    }
    let Some(actor) = connection_actor(state, connection).await else {
        return false;
    };
    detail_by_id(state, actor, job).await.is_ok()
}

async fn existing_request(
    state: &AppState,
    actor: Uuid,
    request: Uuid,
    fp: &str,
) -> Result<Option<JobMutationReceipt>> {
    let mut tx = state.db.job_transaction().await?;
    let existing = store::receipt(&mut tx, actor, request, fp).await?;
    tx.commit().await?;
    if existing.is_some() {
        return Ok(Some(receipt(state, actor, request).await?));
    }
    Ok(None)
}
async fn mutation_result(
    state: &AppState,
    actor: Uuid,
    project: &str,
    request: Uuid,
    kind: JobMutationKind,
    job: Option<Uuid>,
    fp: &str,
    result: Result<JobMutationReceipt>,
) -> Result<JobMutationReceipt> {
    let error = match result {
        Ok(receipt) => return Ok(receipt),
        Err(error) => error,
    };
    let code = error.to_string();
    if code == "request_conflict"
        || code.len() > 80
        || !code.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')
    {
        return Err(error);
    }
    // Only known project readers may obtain a durable rejection in this scope.
    ensure!(
        authorize_job_action(
            state,
            actor,
            project,
            Uuid::nil(),
            Uuid::nil(),
            actor,
            JobAction::Read
        )
        .await?,
        "job_access_denied"
    );
    let job = match job {
        Some(id) => state
            .db
            .job(id)
            .await?
            .filter(|j| j.detail.job.project_id == project)
            .map(|j| j.detail.job.id),
        None => None,
    };
    let mut tx = state.db.job_transaction().await?;
    if let Some(receipt) = store::receipt(&mut tx, actor, request, fp).await? {
        tx.commit().await?;
        return Ok(receipt);
    }
    let rejected = JobMutationReceipt {
        request_id: request,
        operation: kind,
        job_id: job,
        run_id: None,
        status: JobReceiptStatus::Rejected,
        error_code: Some(code.clone()),
        message: Some(code.replace('_', " ")),
    };
    store::save_receipt(&mut tx, actor, project, fp, &rejected).await?;
    tx.commit().await?;
    Ok(rejected)
}
pub async fn admit(
    state: &AppState,
    actor: Uuid,
    project: &str,
    request: JobAdmissionRequest,
) -> Result<JobMutationReceipt> {
    let fp = digest(&("admit", project, &request))?;
    let id = request.request_id;
    if let Some(receipt) = existing_request(state, actor, id, &fp).await? {
        return Ok(receipt);
    }
    let result = admit_inner(state, actor, project, request).await;
    mutation_result(
        state,
        actor,
        project,
        id,
        JobMutationKind::Admit,
        None,
        &fp,
        result,
    )
    .await
}
pub async fn mutate(
    state: &AppState,
    actor: Uuid,
    project: &str,
    job: Uuid,
    kind: JobMutationKind,
    request: JobMutationRequest,
) -> Result<JobMutationReceipt> {
    let fp = digest(&(project, job, kind, &request))?;
    let id = request.request_id;
    if let Some(receipt) = existing_request(state, actor, id, &fp).await? {
        return Ok(receipt);
    }
    let result = mutate_inner(state, actor, project, job, kind, request).await;
    mutation_result(state, actor, project, id, kind, Some(job), &fp, result).await
}
pub async fn answer(
    state: &AppState,
    actor: Uuid,
    project: &str,
    job: Uuid,
    question: Uuid,
    request: JobAnswerRequest,
) -> Result<JobMutationReceipt> {
    let fp = digest(&("answer", project, job, question, &request))?;
    let id = request.request_id;
    if let Some(receipt) = existing_request(state, actor, id, &fp).await? {
        return Ok(receipt);
    }
    let result = answer_inner(state, actor, project, job, question, request).await;
    mutation_result(
        state,
        actor,
        project,
        id,
        JobMutationKind::Answer,
        Some(job),
        &fp,
        result,
    )
    .await
}

#[cfg(test)]
mod tests;

/// Refresh only external disposition, never launch or republish on a read request.
pub async fn refresh_review(state: &AppState, actor: Uuid, id: Uuid, run: Uuid) -> Result<()> {
    let old = state
        .db
        .job(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("job_not_found"))?;
    require(state, actor, &old, JobAction::Read).await?;
    if old.identity().run_id != run || old.detail.publication.is_none() {
        return Ok(());
    }
    let mut tx = state.db.job_transaction().await?;
    let job = store::load(&mut tx, id).await?;
    let recent:i64=sqlx::query_scalar("SELECT COUNT(*) FROM autonomous_job_review_refreshes r JOIN autonomous_job_outbox o ON o.id=r.command_id WHERE o.job_id=? AND r.requested_at>?").bind(id.to_string()).bind((Utc::now()-Duration::minutes(1)).to_rfc3339()).fetch_one(&mut *tx).await?;
    if recent == 0 {
        sqlx::query("UPDATE autonomous_job_outbox SET acknowledged=1 WHERE job_id=? AND json_extract(document,'$.operation')='refresh_publication'").bind(id.to_string()).execute(&mut *tx).await?;
        let command_id = Uuid::new_v4();
        store::command(
            &mut tx,
            &job,
            command_id,
            JobWorkerCommand::RefreshPublication {
                command_id,
                identity: job.identity(),
            },
        )
        .await?;
        sqlx::query("INSERT INTO autonomous_job_review_refreshes(command_id,actor,requested_at) VALUES(?,?,?)").bind(command_id.to_string()).bind(actor.to_string()).bind(Utc::now().to_rfc3339()).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn deliver_notification(state: &AppState, project: &str, event: &JobEvent) -> Result<()> {
    crate::notifications::enqueue_job_event(state, project, event).await?;
    state.db.complete_job_notification(event.id).await
}

fn historical_identity(job: &StoredJob, identity: &JobRunIdentity) -> bool {
    job.detail.job.id == identity.job_id
        && job.detail.job.project_id == identity.project_id
        && job.detail.job.machine_id == identity.machine_id
        && job.detail.job.cluster_owner_user_id == identity.cluster_owner_user_id
        && job
            .detail
            .runs
            .iter()
            .any(|r| r.id == identity.run_id && r.epoch == identity.epoch)
}
async fn confirm_purge(
    state: &AppState,
    machine: Uuid,
    owner: Uuid,
    command: Uuid,
    identity: &JobRunIdentity,
) -> Result<()> {
    ensure!(
        identity.machine_id == machine && identity.cluster_owner_user_id == owner,
        "wrong_purge_worker"
    );
    let mut tx = state.db.job_transaction().await?;
    let job = store::load(&mut tx, identity.job_id).await?;
    ensure!(historical_identity(&job, identity), "stale_purge_identity");
    let matched:Option<String>=sqlx::query_scalar("SELECT o.document FROM autonomous_job_purges p JOIN autonomous_job_outbox o ON o.id=p.command_id WHERE p.run_id=? AND p.command_id=?").bind(identity.run_id.to_string()).bind(command.to_string()).fetch_optional(&mut *tx).await?;
    let command_payload: JobWorkerCommand =
        serde_json::from_str(&matched.ok_or_else(|| anyhow::anyhow!("unknown_purge_command"))?)?;
    ensure!(
        matches!(command_payload,JobWorkerCommand::PurgeRun{identity:ref expected,..} if expected==identity),
        "purge_command_mismatch"
    );
    ensure!(
        job.detail
            .runs
            .iter()
            .find(|r| r.id == identity.run_id)
            .is_some_and(|r| r.state.is_terminal()
                && r.stop_evidence
                    .as_ref()
                    .is_some_and(|e| e.descendants_stopped)),
        "purge_stop_unconfirmed"
    );
    sqlx::query("UPDATE autonomous_job_purges SET confirmed=1 WHERE run_id=? AND command_id=?")
        .bind(identity.run_id.to_string())
        .bind(command.to_string())
        .execute(&mut *tx)
        .await?;
    store::acknowledge(&mut tx, command).await?;
    tx.commit().await?;
    Ok(())
}
