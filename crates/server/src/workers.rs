//! Durable standing grants above the immutable bounded-job engine.
use crate::{
    db::{
        job_policy::{authorize_job_action, effective_job_policy, JobAction},
        jobs::{self as jobs_store, JobTx, StoredJob},
        workers::{self as store, StoredWorker},
    },
    jobs,
    state::AppState,
};
use anyhow::{ensure, Result};
use chrono::{Duration, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use shared::*;
use sqlx::Row;
use uuid::Uuid;

pub mod socket;

fn digest(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn now() -> String {
    Utc::now().to_rfc3339()
}
fn scheduling_now(worker: &StoredWorker) -> Result<chrono::DateTime<Utc>> {
    Ok(chrono::DateTime::parse_from_rfc3339(&worker.accounting_at)?
        .with_timezone(&Utc)
        .max(Utc::now()))
}
fn transition(worker: &mut StoredWorker, state: WorkerState, reason: Option<&str>) {
    worker.detail.worker.state = state;
    worker.detail.worker.reason = reason.map(str::to_owned);
    worker.detail.worker.attention = matches!(
        state,
        WorkerState::NeedsInput
            | WorkerState::WaitingReview
            | WorkerState::Blocked
            | WorkerState::RecoveryRequired
    );
    worker.detail.worker.attention_run_id = if worker.detail.worker.attention {
        worker.detail.worker.active_run_id
    } else {
        None
    };
    worker.detail.worker.attention_question_id = None;
    worker.detail.worker.updated_at = now();
}
async fn authorized(
    state: &AppState,
    actor: Uuid,
    worker: &StoredWorker,
    action: JobAction,
) -> Result<bool> {
    let w = &worker.detail.worker;
    authorize_job_action(
        state,
        actor,
        &w.project_id,
        w.cluster_owner_user_id,
        w.machine_id,
        worker.detail.mandate.approved_by,
        action,
    )
    .await
}
async fn require(
    state: &AppState,
    actor: Uuid,
    worker: &StoredWorker,
    action: JobAction,
) -> Result<()> {
    ensure!(
        authorized(state, actor, worker, action).await?,
        "worker_access_denied"
    );
    Ok(())
}
async fn read_worker(
    state: &AppState,
    actor: Uuid,
    project: Option<&str>,
    id: Uuid,
) -> Result<StoredWorker> {
    let worker = state
        .db
        .worker(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("worker_not_found"))?;
    ensure!(
        project.is_none_or(|p| p == worker.detail.worker.project_id),
        "worker_not_found"
    );
    require(state, actor, &worker, JobAction::Read).await?;
    Ok(worker)
}
fn policy_allows(spec: &WorkerMandateSpec, policy: &JobPolicy) -> bool {
    policy.enabled
        && policy.recurring.enabled
        && spec.limits.fits_within(policy)
        && spec.interval_seconds >= policy.recurring.min_interval_seconds
        && i64::try_from(spec.interval_seconds)
            .ok()
            .and_then(Duration::try_seconds)
            .and_then(|d| Utc::now().checked_add_signed(d))
            .is_some()
        && policy.worker_profiles.contains(&spec.worker_profile)
        && policy
            .verification_profiles
            .contains(&spec.verification_profile)
        && policy.model_profiles.contains(&spec.model_profile)
}

pub async fn targets(
    state: &AppState,
    actor: Uuid,
    project: &str,
) -> Result<WorkerTargetsResponse> {
    let mut old = jobs::targets(state, actor, project).await?;
    let mut tx = state.db.job_transaction().await?;
    for target in &mut old.targets {
        if !target.policy.policy.recurring.enabled {
            target
                .unavailable_reasons
                .push("recurring_workers_disabled".into());
        }
        if !state
            .sessions
            .daemon_supports_capability(&target.machine_id, OMP_PERSISTENT_WORKER_CAPABILITY)
            || !state
                .jobs
                .readiness
                .get(&target.machine_id)
                .is_some_and(|r| {
                    r.protocol_version == WORKER_EXECUTION_PROTOCOL_VERSION && jobs::fresh(&r)
                })
        {
            target
                .unavailable_reasons
                .push("persistent_worker_readiness_required".into());
        }
        let legacy: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM autonomous_jobs j WHERE owner=? AND machine=? AND json_extract(document,'$.detail.task.worker') IS NULL AND (state NOT IN ('completed','review_ready','cancelled','failed') OR EXISTS(SELECT 1 FROM json_each(j.document,'$.detail.runs') r WHERE json_extract(r.value,'$.started_at') IS NOT NULL AND COALESCE(json_extract(r.value,'$.stop_evidence.descendants_stopped'),0)=0))")
            .bind(target.cluster_owner_user_id.to_string()).bind(target.machine_id.to_string()).fetch_one(&mut *tx).await?;
        if legacy > 0 {
            target
                .unavailable_reasons
                .push("legacy_jobs_drain_required".into());
        }
        target.available = target.unavailable_reasons.is_empty();
        target.allowed_actions.admit &= target.available;
    }
    tx.commit().await?;
    Ok(WorkerTargetsResponse {
        project_id: project.into(),
        targets: old.targets,
        unavailable_reasons: old.unavailable_reasons,
    })
}

async fn conversion_allowed(
    state: &AppState,
    actor: Uuid,
    project: &str,
    source: &Option<WorkerConversionSource>,
) -> Result<()> {
    match source {
        Some(WorkerConversionSource::Job { job_id }) => {
            let old = jobs::detail(state, actor, project, *job_id).await?;
            ensure!(
                old.task.worker.is_none()
                    && old.job.state.is_terminal()
                    && old.runs.iter().all(|r| r.started_at.is_none()
                        || r.stop_evidence
                            .as_ref()
                            .is_some_and(|e| e.descendants_stopped)),
                "legacy_execution_stop_unproven"
            );
        }
        None => {}
    }
    Ok(())
}

pub async fn preview(
    state: &AppState,
    actor: Uuid,
    project: &str,
    request: WorkerPreviewRequest,
) -> Result<WorkerPreview> {
    ensure!(request.spec.valid_text(), "invalid_mandate");
    let mut expected_version = request.expected_version;
    let revision = if let Some(id) = request.worker_id {
        let worker = read_worker(state, actor, Some(project), id).await?;
        require(state, actor, &worker, JobAction::Admit).await?;
        ensure!(
            request.expected_version == Some(worker.detail.worker.state_version),
            "worker_version_conflict"
        );
        let mut tx = state.db.job_transaction().await?;
        let mut worker = store::load(&mut tx, id).await?;
        ensure!(
            request.expected_version == Some(worker.detail.worker.state_version),
            "worker_version_conflict"
        );
        ensure!(
            worker.detail.worker.active_job_id.is_none(),
            "worker_execution_unresolved"
        );
        worker.schedule_generation += 1;
        worker.detail.worker.desired_state = WorkerDesiredState::Paused;
        worker.detail.worker.next_check_at = None;
        transition(
            &mut worker,
            WorkerState::Paused,
            Some("mandate_review_pending"),
        );
        store::append(
            &mut tx,
            &mut worker,
            WorkerEventKind::StateChanged,
            "Future dispatch fenced for mandate review",
        )
        .await?;
        expected_version = Some(worker.detail.worker.state_version);
        let revision = worker.detail.mandate.revision + 1;
        tx.commit().await?;
        socket::fanout(state, id).await;
        revision
    } else {
        ensure!(
            request.expected_version.is_none(),
            "worker_version_conflict"
        );
        1
    };
    conversion_allowed(state, actor, project, &request.source).await?;
    let target = targets(state, actor, project)
        .await?
        .targets
        .into_iter()
        .find(|t| t.machine_id == request.spec.machine_id && t.available)
        .ok_or_else(|| anyhow::anyhow!("worker_target_unavailable"))?;
    ensure!(
        policy_allows(&request.spec, &target.policy.policy),
        "worker_policy_denied"
    );
    let prepared = jobs::prepare(
        state,
        actor,
        project,
        &target,
        &request.spec.base_ref,
        &request.spec.worker_profile,
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
        base_ref == request.spec.base_ref
            && profile.key == request.spec.worker_profile
            && profile.ready
            && !repository.is_empty()
            && base_sha.len() == 40
            && base_sha.bytes().all(|b| b.is_ascii_hexdigit()),
        "prepared_inputs_invalid"
    );
    ensure!(
        request.spec.limits.per_run.fits_within(&profile.limits)
            && profile
                .model_profiles
                .iter()
                .any(|p| p.key == request.spec.model_profile),
        "worker_profile_denied"
    );
    let verifier = profile
        .verification_profiles
        .iter()
        .find(|p| p.key == request.spec.verification_profile)
        .ok_or_else(|| anyhow::anyhow!("verification_unavailable"))?;
    let mut result = WorkerPreview {
        id: Uuid::new_v4(),
        project_id: project.into(),
        worker_id: request.worker_id,
        expected_version,
        fingerprint: String::new(),
        expires_at: (Utc::now() + Duration::minutes(5)).to_rfc3339(),
        mandate: WorkerMandate {
            revision,
            spec: request.spec,
            repository,
            worker_profile_digest: profile.digest,
            verification_profile_digest: verifier.digest.clone(),
            policy_digest: target.policy.digest.clone(),
            approved_by: actor,
            approved_at: now(),
        },
        cluster_owner_user_id: target.cluster_owner_user_id,
        hostname: target.hostname,
        base_sha,
        policy: target.policy,
        source: request.source,
    };
    result.fingerprint = digest(&result)?;
    state.db.store_worker_preview(actor, &result).await?;
    Ok(result)
}
async fn existing(
    state: &AppState,
    actor: Uuid,
    request: Uuid,
    fingerprint: &str,
) -> Result<Option<WorkerMutationReceipt>> {
    let mut tx = state.db.job_transaction().await?;
    let result = store::receipt(&mut tx, actor, request, fingerprint).await?;
    tx.commit().await?;
    if let Some(receipt) = &result {
        receipt_access(state, actor, receipt).await?;
    }
    Ok(result)
}
async fn receipt_access(
    state: &AppState,
    actor: Uuid,
    receipt: &WorkerMutationReceipt,
) -> Result<()> {
    if let Some(id) = receipt.worker_id {
        read_worker(state, actor, Some(&receipt.project_id), id).await?;
    } else {
        ensure!(
            authorize_job_action(
                state,
                actor,
                &receipt.project_id,
                Uuid::nil(),
                Uuid::nil(),
                actor,
                JobAction::Read
            )
            .await?,
            "worker_access_denied"
        );
    }
    Ok(())
}
pub async fn receipt(
    state: &AppState,
    actor: Uuid,
    request: Uuid,
) -> Result<WorkerMutationReceipt> {
    let receipt = state
        .db
        .worker_receipt(actor, request)
        .await?
        .ok_or_else(|| anyhow::anyhow!("worker_receipt_not_found"))?;
    receipt_access(state, actor, &receipt).await?;
    Ok(receipt)
}
fn accepted(
    request: Uuid,
    kind: WorkerMutationKind,
    worker: &StoredWorker,
) -> WorkerMutationReceipt {
    WorkerMutationReceipt {
        request_id: request,
        operation: kind,
        status: JobReceiptStatus::Accepted,
        worker_id: Some(worker.detail.worker.id),
        project_id: worker.detail.worker.project_id.clone(),
        state_version: Some(worker.detail.worker.state_version),
        error_code: None,
    }
}
async fn outcome(
    state: &AppState,
    actor: Uuid,
    request: Uuid,
    project: &str,
    worker: Option<Uuid>,
    kind: WorkerMutationKind,
    fingerprint: &str,
    result: Result<WorkerMutationReceipt>,
) -> Result<WorkerMutationReceipt> {
    match result {
        Ok(receipt) => {
            if let Some(id) = receipt.worker_id {
                socket::fanout(state, id).await;
            }
            Ok(receipt)
        }
        Err(error) => {
            let code = error.to_string();
            if code.contains("request_payload_conflict") {
                return Err(error);
            }
            let receipt = WorkerMutationReceipt {
                request_id: request,
                operation: kind,
                status: JobReceiptStatus::Rejected,
                worker_id: worker,
                project_id: project.into(),
                state_version: None,
                error_code: Some(code),
            };
            receipt_access(state, actor, &receipt).await?;
            let mut tx = state.db.job_transaction().await?;
            if let Some(prior) = store::receipt(&mut tx, actor, request, fingerprint).await? {
                return Ok(prior);
            }
            store::save_receipt(&mut tx, actor, fingerprint, &receipt).await?;
            tx.commit().await?;
            Ok(receipt)
        }
    }
}

pub async fn approve(
    state: &AppState,
    actor: Uuid,
    project: &str,
    id: Option<Uuid>,
    request: WorkerApprovalRequest,
) -> Result<WorkerMutationReceipt> {
    let kind = if id.is_some() {
        WorkerMutationKind::EditMandate
    } else {
        WorkerMutationKind::Approve
    };
    let fingerprint = digest(&(kind, project, id, &request))?;
    if let Some(receipt) = existing(state, actor, request.request_id, &fingerprint).await? {
        return Ok(receipt);
    }
    let result = approve_inner(state, actor, project, id, &request, kind, &fingerprint).await;
    outcome(
        state,
        actor,
        request.request_id,
        project,
        id,
        kind,
        &fingerprint,
        result,
    )
    .await
}
async fn approve_inner(
    state: &AppState,
    actor: Uuid,
    project: &str,
    id: Option<Uuid>,
    request: &WorkerApprovalRequest,
    kind: WorkerMutationKind,
    fingerprint: &str,
) -> Result<WorkerMutationReceipt> {
    let preview = state
        .db
        .worker_preview(actor, request.preview_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("worker_preview_not_found"))?;
    ensure!(
        preview.project_id == project
            && preview.worker_id == id
            && preview.fingerprint == request.fingerprint
            && preview.expires_at > now(),
        "worker_preview_changed"
    );
    conversion_allowed(state, actor, project, &preview.source).await?;
    let target = targets(state, actor, project)
        .await?
        .targets
        .into_iter()
        .find(|t| {
            t.machine_id == preview.mandate.spec.machine_id
                && t.cluster_owner_user_id == preview.cluster_owner_user_id
                && t.available
        })
        .ok_or_else(|| anyhow::anyhow!("worker_target_unavailable"))?;
    ensure!(
        target.policy.digest == preview.policy.digest
            && policy_allows(&preview.mandate.spec, &target.policy.policy),
        "worker_preview_changed"
    );
    let prepared = jobs::prepare(
        state,
        actor,
        project,
        &target,
        &preview.mandate.spec.base_ref,
        &preview.mandate.spec.worker_profile,
    )
    .await?;
    ensure!(
        matches!(prepared, JobWorkerReport::Prepared { repository, base_sha, profile, .. }
        if repository == preview.mandate.repository && base_sha == preview.base_sha && profile.ready
            && profile.digest == preview.mandate.worker_profile_digest
            && profile.verification_profiles.iter().any(|p| p.key == preview.mandate.spec.verification_profile && p.digest == preview.mandate.verification_profile_digest)),
        "worker_preview_changed"
    );
    let mut tx = state.db.job_transaction().await?;
    if let Some(receipt) = store::receipt(&mut tx, actor, request.request_id, fingerprint).await? {
        return Ok(receipt);
    }
    ensure!(
        effective_job_policy(&state.db, project).await?.digest == preview.policy.digest,
        "worker_preview_changed"
    );
    ensure!(
        authorize_job_action(
            state,
            actor,
            project,
            preview.cluster_owner_user_id,
            preview.mandate.spec.machine_id,
            actor,
            JobAction::Admit
        )
        .await?,
        "worker_access_denied"
    );
    let fenced: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM autonomous_job_project_fences WHERE project_id=?")
            .bind(project)
            .fetch_one(&mut *tx)
            .await?;
    ensure!(fenced == 0, "project_stop_pending");
    let time = now();
    let mut worker = if let Some(id) = id {
        let mut worker = store::load(&mut tx, id).await?;
        ensure!(
            worker.detail.worker.project_id == project
                && preview.expected_version == Some(worker.detail.worker.state_version),
            "worker_version_conflict"
        );
        ensure!(
            worker.detail.worker.active_job_id.is_none(),
            "worker_execution_unresolved"
        );
        ensure!(
            preview.mandate.revision == worker.detail.mandate.revision + 1,
            "mandate_revision_conflict"
        );
        worker.schedule_generation += 1;
        // Retained work must be reassessed by an inspection under the new responsibility.
        for mut item in store::items(&mut tx, id).await? {
            if item.state == WorkerItemState::Selected {
                item.state = WorkerItemState::Ready;
                store::save_item(&mut tx, &item).await?;
            }
        }
        worker.detail.worker.current_item_id = None;
        worker.detail.worker.backlog_revision += 1;
        worker
    } else {
        StoredWorker {
            detail: WorkerDetail {
                worker: WorkerSummary {
                    id: Uuid::new_v4(),
                    project_id: project.into(),
                    title: preview.mandate.spec.title.clone(),
                    description: preview.mandate.spec.description.clone(),
                    state: WorkerState::Queued,
                    reason: None,
                    desired_state: WorkerDesiredState::Active,
                    mandate_revision: 1,
                    backlog_revision: 0,
                    state_version: 0,
                    high_watermark: 0,
                    next_check_at: Some(time.clone()),
                    active_job_id: None,
                    active_run_id: None,
                    current_item_id: None,
                    machine_id: preview.mandate.spec.machine_id,
                    cluster_owner_user_id: preview.cluster_owner_user_id,
                    hostname: preview.hostname.clone(),
                    attention: false,
                    attention_run_id: None,
                    attention_question_id: None,
                    created_at: time.clone(),
                    updated_at: time.clone(),
                },
                mandate: preview.mandate.clone(),
                checkpoint: None,
                budget: WorkerBudget::default(),
                allowed_actions: Default::default(),
                current_job: None,
            },
            schedule_generation: 1,
            grant_active: true,
            stop_requested: false,
            accounting_at: time.clone(),
        }
    };
    worker.detail.mandate = preview.mandate;
    worker.detail.mandate.approved_at = time.clone();
    worker.detail.worker.title = worker.detail.mandate.spec.title.clone();
    worker.detail.worker.description = worker.detail.mandate.spec.description.clone();
    worker.detail.worker.mandate_revision = worker.detail.mandate.revision;
    worker.detail.worker.machine_id = worker.detail.mandate.spec.machine_id;
    worker.detail.worker.cluster_owner_user_id = preview.cluster_owner_user_id;
    worker.detail.worker.hostname = preview.hostname;
    worker.detail.worker.desired_state = WorkerDesiredState::Active;
    worker.detail.worker.next_check_at = Some(time);
    worker.grant_active = true;
    worker.stop_requested = false;
    transition(&mut worker, WorkerState::Queued, None);
    store::append(
        &mut tx,
        &mut worker,
        if id.is_some() {
            WorkerEventKind::MandateChanged
        } else {
            WorkerEventKind::Approved
        },
        "Recurring responsibility approved",
    )
    .await?;
    let receipt = accepted(request.request_id, kind, &worker);
    store::save_receipt(&mut tx, actor, fingerprint, &receipt).await?;
    tx.commit().await?;
    Ok(receipt)
}

pub async fn detail_by_id(state: &AppState, actor: Uuid, id: Uuid) -> Result<WorkerDetail> {
    let mut worker = read_worker(state, actor, None, id).await?;
    let admit = authorized(state, actor, &worker, JobAction::Admit).await?;
    let cancel = authorized(state, actor, &worker, JobAction::Cancel).await?;
    let grant = worker.grant_active
        && policy_allows(
            &worker.detail.mandate.spec,
            &effective_job_policy(&state.db, &worker.detail.worker.project_id)
                .await?
                .policy,
        );
    let active = worker.detail.worker.active_job_id.is_some();
    let blocked = matches!(
        worker.detail.worker.state,
        WorkerState::Blocked
            | WorkerState::RecoveryRequired
            | WorkerState::WaitingReview
            | WorkerState::Stopping
    );
    let retry_inspection = worker.detail.worker.state == WorkerState::Blocked
        && worker.detail.worker.current_item_id.is_none();
    worker.detail.allowed_actions = WorkerAllowedActions {
        edit_mandate: admit && !active,
        edit_backlog: admit,
        pause: cancel && worker.detail.worker.desired_state == WorkerDesiredState::Active,
        stop: cancel
            && (active || worker.detail.worker.desired_state == WorkerDesiredState::Active),
        resume: admit
            && grant
            && !active
            && ((!blocked && worker.detail.worker.desired_state == WorkerDesiredState::Paused)
                || retry_inspection),
        wake: admit
            && grant
            && !active
            && !blocked
            && worker.detail.worker.desired_state == WorkerDesiredState::Active,
        retry: false,
        answer: false,
        retry_publication: false,
    };
    if let Some(job) = worker.detail.worker.active_job_id {
        let current = jobs::detail(state, actor, &worker.detail.worker.project_id, job).await?;
        worker.detail.allowed_actions.answer =
            current.job.allowed_actions.answer && !worker.stop_requested;
        worker.detail.allowed_actions.retry_publication =
            current.job.allowed_actions.retry_publication && !worker.stop_requested;
        worker.detail.allowed_actions.resume |= admit
            && grant
            && worker.detail.worker.desired_state == WorkerDesiredState::Paused
            && worker.detail.worker.state == WorkerState::WaitingReview
            && current.job.state.is_terminal();
        worker.detail.current_job = Some(current);
    }
    let mut tx = state.db.job_transaction().await?;
    worker.detail.budget = store::budget(&mut tx, &worker, &now()).await?;
    if admit && grant && !worker.detail.budget.accounting_uncertain {
        if let Some(item) = store::items(&mut tx, id).await?.into_iter().find(|item| {
            Some(item.id) == worker.detail.worker.current_item_id
                && item.state == WorkerItemState::Blocked
        }) {
            let usage = store::item_usage(&mut tx, id, item.id).await?;
            let review_closed = worker.detail.current_job.as_ref().is_some_and(|j| {
                j.job.state.is_terminal()
                    && j.publication.as_ref().is_some_and(|p| p.state == "closed")
            });
            worker.detail.allowed_actions.retry = (!active || review_closed)
                && item.attempts < worker.detail.mandate.spec.limits.attempts_per_item
                && usage.elapsed_seconds
                    < worker.detail.mandate.spec.limits.per_run.elapsed_seconds
                && usage.spend_micro_usd.is_some_and(|spent| {
                    spent < worker.detail.mandate.spec.limits.per_run.spend_micro_usd
                });
        }
    }
    tx.commit().await?;
    Ok(worker.detail)
}
pub async fn detail(
    state: &AppState,
    actor: Uuid,
    project: &str,
    id: Uuid,
) -> Result<WorkerDetail> {
    let result = detail_by_id(state, actor, id).await?;
    ensure!(result.worker.project_id == project, "worker_not_found");
    Ok(result)
}
pub async fn list(
    state: &AppState,
    actor: Uuid,
    project: Option<&str>,
    cursor: Option<&str>,
    status: Option<&str>,
) -> Result<WorkerListResponse> {
    if let Some(project) = project {
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
            "worker_access_denied"
        );
    }
    let mut workers = Vec::new();
    let mut scan = cursor.map(str::to_owned);
    loop {
        let rows = state
            .db
            .worker_page(project, scan.as_deref(), status, 100)
            .await?;
        let last_page = rows.len() < 100;
        scan = rows
            .last()
            .map(|w| format!("{}|{}", w.detail.worker.created_at, w.detail.worker.id));
        for worker in rows {
            if authorized(state, actor, &worker, JobAction::Read).await? {
                workers.push(worker.detail.worker);
            }
            if workers.len() == 51 {
                break;
            }
        }
        if workers.len() == 51 || last_page {
            break;
        }
    }
    let has_more = workers.len() > 50;
    workers.truncate(50);
    let next_cursor = if has_more {
        workers.last().map(|w| format!("{}|{}", w.created_at, w.id))
    } else {
        None
    };
    Ok(WorkerListResponse {
        workers,
        next_cursor,
    })
}
pub async fn events(
    state: &AppState,
    actor: Uuid,
    project: &str,
    id: Uuid,
    after: u64,
    limit: usize,
) -> Result<WorkerEventsResponse> {
    read_worker(state, actor, Some(project), id).await?;
    state.db.worker_events(id, after, limit.clamp(1, 200)).await
}
pub async fn runs(
    state: &AppState,
    actor: Uuid,
    project: &str,
    id: Uuid,
    cursor: Option<&str>,
) -> Result<WorkerRunsResponse> {
    read_worker(state, actor, Some(project), id).await?;
    state.db.worker_runs(id, cursor).await
}
pub async fn exact_run(
    state: &AppState,
    actor: Uuid,
    project: &str,
    id: Uuid,
    run: Uuid,
) -> Result<JobDetail> {
    read_worker(state, actor, Some(project), id).await?;
    let job = state
        .db
        .worker_run(id, run)
        .await?
        .ok_or_else(|| anyhow::anyhow!("worker_run_not_found"))?;
    let result = jobs::detail(state, actor, project, job).await?;
    ensure!(
        result
            .task
            .worker
            .as_ref()
            .is_some_and(|w| w.worker_id == id)
            && result.runs.iter().any(|r| r.id == run),
        "worker_run_not_found"
    );
    Ok(result)
}
pub async fn items(
    state: &AppState,
    actor: Uuid,
    project: &str,
    id: Uuid,
    cursor: Option<&str>,
    status: Option<&str>,
) -> Result<WorkerItemsResponse> {
    read_worker(state, actor, Some(project), id).await?;
    state.db.worker_items(id, cursor, status).await
}

pub async fn mutate(
    state: &AppState,
    actor: Uuid,
    project: &str,
    id: Uuid,
    kind: WorkerMutationKind,
    request: WorkerMutationRequest,
) -> Result<WorkerMutationReceipt> {
    let fingerprint = digest(&(kind, project, id, &request))?;
    if let Some(receipt) = existing(state, actor, request.request_id, &fingerprint).await? {
        return Ok(receipt);
    }
    let result = mutate_inner(state, actor, project, id, kind, &request, &fingerprint).await;
    outcome(
        state,
        actor,
        request.request_id,
        project,
        Some(id),
        kind,
        &fingerprint,
        result,
    )
    .await
}
async fn mutate_inner(
    state: &AppState,
    actor: Uuid,
    project: &str,
    id: Uuid,
    kind: WorkerMutationKind,
    request: &WorkerMutationRequest,
    fingerprint: &str,
) -> Result<WorkerMutationReceipt> {
    let old = read_worker(state, actor, Some(project), id).await?;
    require(
        state,
        actor,
        &old,
        if matches!(kind, WorkerMutationKind::Pause | WorkerMutationKind::Stop) {
            JobAction::Cancel
        } else {
            JobAction::Admit
        },
    )
    .await?;
    let policy = effective_job_policy(&state.db, project).await?;
    let mut tx = state.db.job_transaction().await?;
    if let Some(receipt) = store::receipt(&mut tx, actor, request.request_id, fingerprint).await? {
        return Ok(receipt);
    }
    let mut worker = store::load(&mut tx, id).await?;
    ensure!(
        worker.detail.worker.state_version == request.expected_version,
        "worker_version_conflict"
    );
    match kind {
        WorkerMutationKind::Pause | WorkerMutationKind::Stop => {
            worker.detail.worker.desired_state = WorkerDesiredState::Paused;
            worker.schedule_generation += 1;
            worker.detail.worker.next_check_at = None;
            if kind == WorkerMutationKind::Stop {
                worker.stop_requested = true;
            }
            if let Some(job_id) = worker.detail.worker.active_job_id {
                let mut job = jobs_store::load(&mut tx, job_id).await?;
                if job.detail.job.state.is_terminal() {
                    transition(&mut worker, WorkerState::WaitingReview, None);
                } else if kind == WorkerMutationKind::Stop {
                    jobs::cancel_locked(&mut tx, &mut job, "worker_stop_requested").await?;
                    transition(
                        &mut worker,
                        WorkerState::Stopping,
                        Some("awaiting_positive_stop"),
                    );
                } else {
                    transition(&mut worker, WorkerState::Pausing, None);
                }
            } else {
                transition(&mut worker, WorkerState::Paused, None);
            }
        }
        WorkerMutationKind::Resume if worker.detail.worker.state == WorkerState::WaitingReview => {
            ensure!(
                worker.grant_active && policy_allows(&worker.detail.mandate.spec, &policy.policy),
                "worker_reapproval_required"
            );
            let id = worker
                .detail
                .worker
                .active_job_id
                .ok_or_else(|| anyhow::anyhow!("worker_execution_missing"))?;
            let job = jobs_store::load(&mut tx, id).await?;
            ensure!(
                job.detail.job.state.is_terminal()
                    && job.detail.runs.iter().all(|r| r.started_at.is_none()
                        || r.stop_evidence
                            .as_ref()
                            .is_some_and(|e| e.descendants_stopped)),
                "worker_execution_unresolved"
            );
            worker.detail.worker.desired_state = WorkerDesiredState::Active;
            worker.stop_requested = false;
            worker.schedule_generation += 1;
            worker.detail.worker.next_check_at = None;
        }
        WorkerMutationKind::Resume | WorkerMutationKind::Wake => {
            ensure!(
                worker.grant_active && policy_allows(&worker.detail.mandate.spec, &policy.policy),
                "worker_reapproval_required"
            );
            ensure!(
                worker.detail.worker.active_job_id.is_none(),
                "worker_execution_unresolved"
            );
            let retry_inspection = kind == WorkerMutationKind::Resume
                && worker.detail.worker.state == WorkerState::Blocked
                && worker.detail.worker.current_item_id.is_none();
            ensure!(
                retry_inspection
                    || !matches!(
                        worker.detail.worker.state,
                        WorkerState::Blocked
                            | WorkerState::RecoveryRequired
                            | WorkerState::WaitingReview
                            | WorkerState::Stopping
                    ),
                "worker_disposition_required"
            );
            if kind == WorkerMutationKind::Wake {
                ensure!(
                    worker.detail.worker.desired_state == WorkerDesiredState::Active,
                    "worker_paused"
                );
            } else {
                ensure!(
                    worker.detail.worker.desired_state == WorkerDesiredState::Paused
                        || retry_inspection,
                    "worker_not_paused"
                );
                worker.detail.worker.desired_state = WorkerDesiredState::Active;
                worker.stop_requested = false;
                worker.schedule_generation += 1;
            }
            // Check now coalesces, but cannot shorten the currently approved minimum cadence.
            let earliest = worker
                .detail
                .checkpoint
                .as_ref()
                .and_then(|c| chrono::DateTime::parse_from_rfc3339(&c.recorded_at).ok())
                .map(|d| {
                    d + Duration::seconds(policy.policy.recurring.min_interval_seconds as i64)
                });
            let due = earliest
                .map(|d| d.with_timezone(&Utc).max(Utc::now()))
                .unwrap_or_else(Utc::now)
                .to_rfc3339();
            worker.detail.worker.next_check_at = Some(due);
            transition(&mut worker, WorkerState::Queued, None);
        }
        _ => anyhow::bail!("worker_operation_invalid"),
    }
    store::append(
        &mut tx,
        &mut worker,
        WorkerEventKind::StateChanged,
        "Worker lifecycle updated",
    )
    .await?;
    let receipt = accepted(request.request_id, kind, &worker);
    store::save_receipt(&mut tx, actor, fingerprint, &receipt).await?;
    tx.commit().await?;
    Ok(receipt)
}

pub async fn upsert_item(
    state: &AppState,
    actor: Uuid,
    project: &str,
    id: Uuid,
    request: WorkerItemRequest,
) -> Result<WorkerMutationReceipt> {
    let fingerprint = digest(&(WorkerMutationKind::UpsertItem, project, id, &request))?;
    if let Some(receipt) = existing(state, actor, request.request_id, &fingerprint).await? {
        return Ok(receipt);
    }
    let result = upsert_item_inner(state, actor, project, id, &request, &fingerprint).await;
    outcome(
        state,
        actor,
        request.request_id,
        project,
        Some(id),
        WorkerMutationKind::UpsertItem,
        &fingerprint,
        result,
    )
    .await
}
async fn upsert_item_inner(
    state: &AppState,
    actor: Uuid,
    project: &str,
    id: Uuid,
    request: &WorkerItemRequest,
    fingerprint: &str,
) -> Result<WorkerMutationReceipt> {
    let old = read_worker(state, actor, Some(project), id).await?;
    require(state, actor, &old, JobAction::Admit).await?;
    ensure!(
        !request.item_id.is_nil() && request.item.is_valid(),
        "invalid_worker_item"
    );
    let mut tx = state.db.job_transaction().await?;
    if let Some(receipt) = store::receipt(&mut tx, actor, request.request_id, fingerprint).await? {
        return Ok(receipt);
    }
    let mut worker = store::load(&mut tx, id).await?;
    ensure!(
        worker.detail.worker.state_version == request.expected_version,
        "worker_version_conflict"
    );
    let mut backlog = store::items(&mut tx, id).await?;
    let prior = backlog.iter().find(|i| i.id == request.item_id);
    ensure!(
        prior.map(|i| i.revision) == request.expected_item_revision,
        "item_revision_conflict"
    );
    ensure!(
        prior.is_none_or(|i| matches!(i.state, WorkerItemState::Ready | WorkerItemState::Blocked)),
        "item_claimed"
    );
    if prior.is_none() {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM autonomous_worker_items WHERE id=?")
                .bind(request.item_id.to_string())
                .fetch_one(&mut *tx)
                .await?;
        ensure!(count == 0, "item_revision_conflict");
    }
    let time = now();
    let mut item = if let Some(prior) = prior {
        prior.clone()
    } else {
        WorkerItem {
            id: request.item_id,
            worker_id: id,
            revision: 0,
            item: request.item.clone(),
            origin: WorkerItemOrigin::Human,
            state: WorkerItemState::Ready,
            attempts: 0,
            job_id: None,
            run_id: None,
            outcome: None,
            created_at: time.clone(),
            updated_at: time.clone(),
        }
    };
    item.revision += 1;
    item.item = request.item.clone();
    item.updated_at = time;
    backlog.retain(|i| i.id != item.id);
    backlog.push(item.clone());
    validate_backlog(&backlog)?;
    validate_dependencies(&mut tx, id, &backlog).await?;
    store::save_item(&mut tx, &item).await?;
    worker.detail.worker.backlog_revision += 1;
    store::append(
        &mut tx,
        &mut worker,
        WorkerEventKind::BacklogChanged,
        "Human backlog edit accepted",
    )
    .await?;
    let receipt = accepted(request.request_id, WorkerMutationKind::UpsertItem, &worker);
    store::save_receipt(&mut tx, actor, fingerprint, &receipt).await?;
    tx.commit().await?;
    Ok(receipt)
}
fn validate_backlog(items: &[WorkerItem]) -> Result<()> {
    ensure!(
        items.len() <= WORKER_MAX_BACKLOG_ITEMS
            && serde_json::to_vec(items)?.len() <= WORKER_MAX_CONTEXT_BYTES / 2,
        "worker_backlog_full"
    );
    for item in items {
        ensure!(
            item.item.is_valid() && !item.item.dependencies.contains(&item.id),
            "invalid_worker_item"
        );
        ensure!(
            item.item
                .dependencies
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                == item.item.dependencies.len(),
            "item_dependency_duplicate"
        );
        let mut visiting = vec![item.id];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = visiting.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let Some(current) = items.iter().find(|i| i.id == id) {
                for dependency in &current.item.dependencies {
                    ensure!(*dependency != item.id, "item_dependency_cycle");
                    visiting.push(*dependency);
                }
            }
        }
    }
    Ok(())
}
async fn validate_dependencies(
    tx: &mut JobTx<'_>,
    worker: Uuid,
    items: &[WorkerItem],
) -> Result<()> {
    for item in items {
        for dependency in &item.item.dependencies {
            if items.iter().any(|i| i.id == *dependency) {
                continue;
            }
            let state: Option<String> = sqlx::query_scalar(
                "SELECT state FROM autonomous_worker_items WHERE worker_id=? AND id=?",
            )
            .bind(worker.to_string())
            .bind(dependency.to_string())
            .fetch_optional(&mut **tx)
            .await?;
            ensure!(
                state.as_deref() == Some("done"),
                "item_dependency_not_found"
            );
        }
    }
    Ok(())
}
pub async fn mutate_item(
    state: &AppState,
    actor: Uuid,
    project: &str,
    id: Uuid,
    item: Uuid,
    kind: WorkerMutationKind,
    request: WorkerMutationRequest,
) -> Result<WorkerMutationReceipt> {
    let fingerprint = digest(&(kind, project, id, item, &request))?;
    if let Some(receipt) = existing(state, actor, request.request_id, &fingerprint).await? {
        return Ok(receipt);
    }
    let result = mutate_item_inner(
        state,
        actor,
        project,
        id,
        item,
        kind,
        &request,
        &fingerprint,
    )
    .await;
    outcome(
        state,
        actor,
        request.request_id,
        project,
        Some(id),
        kind,
        &fingerprint,
        result,
    )
    .await
}
async fn mutate_item_inner(
    state: &AppState,
    actor: Uuid,
    project: &str,
    id: Uuid,
    item_id: Uuid,
    kind: WorkerMutationKind,
    request: &WorkerMutationRequest,
    fingerprint: &str,
) -> Result<WorkerMutationReceipt> {
    let old = read_worker(state, actor, Some(project), id).await?;
    require(state, actor, &old, JobAction::Admit).await?;
    let policy = effective_job_policy(&state.db, project).await?;
    let mut tx = state.db.job_transaction().await?;
    if let Some(receipt) = store::receipt(&mut tx, actor, request.request_id, fingerprint).await? {
        return Ok(receipt);
    }
    let mut worker = store::load(&mut tx, id).await?;
    ensure!(
        worker.detail.worker.state_version == request.expected_version,
        "worker_version_conflict"
    );
    let mut item = store::items(&mut tx, id)
        .await?
        .into_iter()
        .find(|i| i.id == item_id)
        .ok_or_else(|| anyhow::anyhow!("worker_item_not_found"))?;
    ensure!(
        matches!(
            item.state,
            WorkerItemState::Ready | WorkerItemState::Blocked
        ),
        "item_claimed"
    );
    if worker.detail.worker.current_item_id == Some(item_id) {
        if let Some(job_id) = worker.detail.worker.active_job_id {
            let job = jobs_store::load(&mut tx, job_id).await?;
            ensure!(
                job.detail.job.state.is_terminal()
                    && job.detail.runs.iter().all(|r| r.started_at.is_none()
                        || r.stop_evidence
                            .as_ref()
                            .is_some_and(|e| e.descendants_stopped)),
                "worker_execution_unresolved"
            );
            // A delivered draft requires explicit dismissal; retry may not stack
            // another candidate while its external disposition is unresolved.
            ensure!(
                kind == WorkerMutationKind::DismissItem
                    || job
                        .detail
                        .publication
                        .as_ref()
                        .is_some_and(|p| p.state == "closed"),
                "review_disposition_required"
            );
            store::release_execution(&mut tx, id, job_id).await?;
            worker.detail.worker.active_job_id = None;
            worker.detail.worker.active_run_id = None;
        }
    }
    match kind {
        WorkerMutationKind::DismissItem => {
            item.state = WorkerItemState::Dismissed;
            item.outcome = Some("dismissed_by_human".into());
            if worker.detail.worker.current_item_id == Some(item_id) {
                worker.detail.worker.current_item_id = None;
            }
            if worker.detail.worker.active_job_id.is_none()
                && worker.detail.worker.state == WorkerState::Blocked
            {
                // Dismissal resolves the failed item, but requires an explicit Resume.
                worker.detail.worker.desired_state = WorkerDesiredState::Paused;
                transition(&mut worker, WorkerState::Paused, None);
            }
        }
        WorkerMutationKind::RetryItem => {
            ensure!(
                worker.grant_active && policy_allows(&worker.detail.mandate.spec, &policy.policy),
                "worker_reapproval_required"
            );
            ensure!(
                worker.detail.worker.active_job_id.is_none()
                    && item.state == WorkerItemState::Blocked,
                "worker_execution_unresolved"
            );
            ensure!(
                item.attempts < worker.detail.mandate.spec.limits.attempts_per_item,
                "item_attempts_exhausted"
            );
            let usage = store::item_usage(&mut tx, id, item_id).await?;
            ensure!(
                usage.spend_micro_usd.is_some_and(|s| s < worker
                    .detail
                    .mandate
                    .spec
                    .limits
                    .per_run
                    .spend_micro_usd)
                    && usage.elapsed_seconds
                        < worker.detail.mandate.spec.limits.per_run.elapsed_seconds,
                "item_allowance_exhausted"
            );
            item.state = WorkerItemState::Selected;
            worker.detail.worker.current_item_id = Some(item_id);
            worker.detail.worker.desired_state = WorkerDesiredState::Active;
            worker.stop_requested = false;
            worker.schedule_generation += 1;
            worker.detail.worker.next_check_at = Some(now());
            transition(&mut worker, WorkerState::Queued, None);
        }
        _ => anyhow::bail!("worker_operation_invalid"),
    }
    item.revision += 1;
    item.updated_at = now();
    store::save_item(&mut tx, &item).await?;
    worker.detail.worker.backlog_revision += 1;
    store::append(
        &mut tx,
        &mut worker,
        WorkerEventKind::BacklogChanged,
        "Explicit item disposition accepted",
    )
    .await?;
    let receipt = accepted(request.request_id, kind, &worker);
    store::save_receipt(&mut tx, actor, fingerprint, &receipt).await?;
    tx.commit().await?;
    Ok(receipt)
}

/// The authenticated envelope, not the model document, establishes usage and stop.
/// A human edit can reject proposals without rolling back those trusted facts.
pub(crate) async fn accept_result(
    tx: &mut JobTx<'_>,
    job: &mut StoredJob,
    result: &WorkerResult,
) -> Result<()> {
    let context = job
        .detail
        .task
        .worker
        .clone()
        .ok_or_else(|| anyhow::anyhow!("worker_context_missing"))?;
    let mut worker = store::load(tx, context.worker_id).await?;
    let run = job.detail.job.current_run_id;
    let document = serde_json::to_string(result)?;
    if let Some(old) = sqlx::query_scalar::<_, String>(
        "SELECT document FROM autonomous_worker_results WHERE run_id=?",
    )
    .bind(run.to_string())
    .fetch_optional(&mut **tx)
    .await?
    {
        ensure!(old == document, "worker_result_conflict");
        return Ok(());
    }
    let valid = result
        .validate(&context)
        .map_err(anyhow::Error::msg)
        .and_then(|_| {
            ensure!(
                document.len() <= WORKER_MAX_RESULT_BYTES,
                "worker_result_too_large"
            );
            ensure!(
                job.detail
                    .questions
                    .iter()
                    .filter(|q| q.run_id == run)
                    .all(|q| matches!(
                        q.state,
                        JobQuestionState::Confirmed | JobQuestionState::Cancelled
                    )),
                "worker_questions_unresolved"
            );
            Ok(())
        });
    let mut applied = false;
    let mut reason = valid.as_ref().err().map(ToString::to_string);
    if valid.is_ok() {
        worker.detail.checkpoint = Some(WorkerCheckpoint {
            id: Uuid::new_v4(),
            worker_id: context.worker_id,
            run_id: run,
            mandate_revision: context.mandate_revision,
            backlog_revision: context.backlog_revision,
            base_sha: job.detail.task.base_sha.clone(),
            summary: result.summary.clone(),
            checklist: result.checklist.clone(),
            recorded_at: scheduling_now(&worker)?.to_rfc3339(),
        });
        if worker.detail.mandate.revision == context.mandate_revision
            && worker.detail.worker.backlog_revision == context.backlog_revision
        {
            let proposals = prepare_proposals(tx, &worker, &context, result).await;
            match proposals {
                Ok(items) => {
                    for item in items {
                        store::save_item(tx, &item).await?;
                    }
                    if !result.upserts.is_empty() || result.selected_item_id.is_some() {
                        worker.detail.worker.backlog_revision += 1;
                    }
                    if result.purpose == WorkerRunPurpose::Inspection {
                        worker.detail.worker.current_item_id = result.selected_item_id;
                    }
                    applied = true;
                }
                Err(error) => reason = Some(error.to_string()),
            }
        } else {
            reason = Some("worker_result_stale_human_revision".into());
        }
    }
    sqlx::query(
        "INSERT INTO autonomous_worker_results(run_id,worker_id,document,applied) VALUES(?,?,?,?)",
    )
    .bind(run.to_string())
    .bind(context.worker_id.to_string())
    .bind(document)
    .bind(applied)
    .execute(&mut **tx)
    .await?;
    if valid.is_err()
        || (!applied && reason.as_deref() != Some("worker_result_stale_human_revision"))
    {
        job.transition(JobState::Failed, reason.clone());
        job.current().ended_at = Some(now());
        job.detail.job.attention_count = 1;
    } else {
        match result.disposition {
            WorkerResultDisposition::SelectWork | WorkerResultDisposition::NoWork => {
                job.transition(JobState::Completed, reason.clone());
                job.current().ended_at = Some(now());
                let at = scheduling_now(&worker)?;
                let next = if applied && result.disposition == WorkerResultDisposition::SelectWork {
                    Some(at)
                } else {
                    i64::try_from(worker.detail.mandate.spec.interval_seconds)
                        .ok()
                        .and_then(Duration::try_seconds)
                        .and_then(|duration| at.checked_add_signed(duration))
                };
                worker.detail.worker.next_check_at = next.map(|at| at.to_rfc3339());
                if next.is_none() {
                    job.transition(
                        JobState::Failed,
                        Some("worker_schedule_out_of_range".into()),
                    );
                    job.detail.job.attention_count = 1;
                }
            }
            WorkerResultDisposition::Blocked | WorkerResultDisposition::NoChange => {
                job.transition(
                    JobState::Failed,
                    Some(
                        if result.disposition == WorkerResultDisposition::NoChange {
                            "implementation_no_change"
                        } else {
                            "worker_reported_blocked"
                        }
                        .into(),
                    ),
                );
                job.current().ended_at = Some(now());
                job.detail.job.attention_count = 1;
            }
            WorkerResultDisposition::CandidateReady => {
                // Only the following trusted Settled report may start independent verification.
                // A graceful pause or stale human backlog edit does not discard the candidate.
            }
        }
    }
    if job.detail.job.state.is_terminal() {
        for question in job.detail.questions.iter_mut().filter(|q| q.run_id == run) {
            if question.state != JobQuestionState::Confirmed
                && question.state != JobQuestionState::Cancelled
            {
                question.state = JobQuestionState::Cancelled;
                question.version += 1;
            }
        }
    }
    store::append(
        tx,
        &mut worker,
        WorkerEventKind::Checkpoint,
        if applied {
            "Execution checkpoint accepted"
        } else {
            "Execution evidence retained; backlog proposals rejected"
        },
    )
    .await?;
    Ok(())
}
async fn prepare_proposals(
    tx: &mut JobTx<'_>,
    worker: &StoredWorker,
    context: &WorkerExecutionContext,
    result: &WorkerResult,
) -> Result<Vec<WorkerItem>> {
    let mut items = store::items(tx, context.worker_id).await?;
    for proposal in &result.upserts {
        let existing = items.iter_mut().find(|i| i.id == proposal.id);
        if let Some(item) = existing {
            ensure!(
                Some(item.revision) == proposal.expected_revision,
                "item_revision_conflict"
            );
            ensure!(item.state == WorkerItemState::Ready, "item_claimed");
            item.revision += 1;
            item.item = proposal.item.clone();
            item.updated_at = now();
        } else {
            ensure!(
                proposal.expected_revision.is_none(),
                "item_revision_conflict"
            );
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM autonomous_worker_items WHERE worker_id=? AND id=?",
            )
            .bind(context.worker_id.to_string())
            .bind(proposal.id.to_string())
            .fetch_one(&mut **tx)
            .await?;
            ensure!(count == 0, "item_revision_conflict");
            items.push(WorkerItem {
                id: proposal.id,
                worker_id: context.worker_id,
                revision: 1,
                item: proposal.item.clone(),
                origin: if context.purpose == WorkerRunPurpose::Inspection {
                    WorkerItemOrigin::Inspection
                } else {
                    WorkerItemOrigin::Implementation
                },
                state: WorkerItemState::Ready,
                attempts: 0,
                job_id: None,
                run_id: None,
                outcome: None,
                created_at: now(),
                updated_at: now(),
            });
        }
    }
    validate_backlog(&items)?;
    validate_dependencies(tx, context.worker_id, &items).await?;
    if context.purpose == WorkerRunPurpose::Inspection {
        if let Some(id) = result.selected_item_id {
            let item = items
                .iter_mut()
                .find(|i| i.id == id)
                .ok_or_else(|| anyhow::anyhow!("selected_item_not_found"))?;
            ensure!(
                item.state == WorkerItemState::Ready
                    && item.attempts < worker.detail.mandate.spec.limits.attempts_per_item,
                "selected_item_ineligible"
            );
            ensure!(
                dependencies_done(tx, context.worker_id, &item.item.dependencies).await?,
                "selected_item_dependencies"
            );
            item.state = WorkerItemState::Selected;
            item.updated_at = now();
        }
        if result.disposition == WorkerResultDisposition::NoWork {
            for item in &items {
                ensure!(
                    item.state != WorkerItemState::Ready
                        || !dependencies_done(tx, context.worker_id, &item.item.dependencies)
                            .await?,
                    "no_work_has_eligible_items"
                );
            }
        }
    }
    Ok(items)
}
async fn dependencies_done(
    tx: &mut JobTx<'_>,
    worker: Uuid,
    dependencies: &[Uuid],
) -> Result<bool> {
    for id in dependencies {
        let state: Option<String> = sqlx::query_scalar(
            "SELECT state FROM autonomous_worker_items WHERE worker_id=? AND id=?",
        )
        .bind(worker.to_string())
        .bind(id.to_string())
        .fetch_optional(&mut **tx)
        .await?;
        if state.as_deref() != Some("done") {
            return Ok(false);
        }
    }
    Ok(true)
}
pub(crate) async fn candidate_ready(tx: &mut JobTx<'_>, job: &StoredJob) -> Result<bool> {
    let Some(context) = &job.detail.task.worker else {
        return Ok(false);
    };
    let raw: Option<String> = sqlx::query_scalar(
        "SELECT document FROM autonomous_worker_results WHERE worker_id=? AND run_id=?",
    )
    .bind(context.worker_id.to_string())
    .bind(job.detail.job.current_run_id.to_string())
    .fetch_optional(&mut **tx)
    .await?;
    Ok(raw
        .and_then(|s| serde_json::from_str::<WorkerResult>(&s).ok())
        .is_some_and(|r| {
            r.validate(context).is_ok() && r.disposition == WorkerResultDisposition::CandidateReady
        }))
}
pub(crate) async fn publication_unknown(tx: &mut JobTx<'_>, job: &StoredJob) -> Result<()> {
    let Some(context) = &job.detail.task.worker else {
        return Ok(());
    };
    let mut worker = store::load(tx, context.worker_id).await?;
    transition(
        &mut worker,
        WorkerState::Blocked,
        Some("publication_disposition_unknown"),
    );
    worker.detail.worker.next_check_at = None;
    store::append(
        tx,
        &mut worker,
        WorkerEventKind::Review,
        "Publication disposition requires human review",
    )
    .await?;
    Ok(())
}

pub(crate) async fn reconcile_job(
    tx: &mut JobTx<'_>,
    job: &StoredJob,
) -> Result<Option<WorkerEvent>> {
    let Some(context) = &job.detail.task.worker else {
        return Ok(None);
    };
    let mut worker = store::load(tx, context.worker_id).await?;
    let prior = serde_json::to_string(&worker.detail.worker)?;
    let budget_before = budget_counters(&worker.detail.budget);
    let run = job
        .detail
        .runs
        .last()
        .ok_or_else(|| anyhow::anyhow!("worker_run_missing"))?;
    let stopped = run.started_at.is_none()
        || run.stop_evidence.as_ref().is_some_and(|e| {
            e.descendants_stopped && !e.container_id.is_empty() && !e.boot_id.is_empty()
        });
    store::reconcile_usage(
        tx,
        &mut worker,
        context.reservation_id,
        &run.usage,
        stopped,
        &now(),
    )
    .await?;
    if worker.detail.worker.active_job_id != Some(job.detail.job.id) {
        if budget_before != budget_counters(&worker.detail.budget) {
            let mut event = store::append(
                tx,
                &mut worker,
                WorkerEventKind::Budget,
                "Historical execution accounting reconciled",
            )
            .await?;
            event.job_id = Some(job.detail.job.id);
            event.run_id = Some(run.id);
            sqlx::query(
                "UPDATE autonomous_worker_events SET document=? WHERE worker_id=? AND seq=?",
            )
            .bind(serde_json::to_string(&event)?)
            .bind(event.worker_id.to_string())
            .bind(i64::try_from(event.seq)?)
            .execute(&mut **tx)
            .await?;
            return Ok(Some(event));
        }
        store::save(tx, &worker).await?;
        return Ok(None);
    }
    let mut item = match context.selected_item_id {
        Some(id) => store::items(tx, context.worker_id)
            .await?
            .into_iter()
            .find(|i| i.id == id),
        None => None,
    };
    let terminal = job.detail.job.state.is_terminal() && stopped;
    if terminal {
        store::release_execution(tx, context.worker_id, job.detail.job.id).await?;
        match job.detail.job.state {
            JobState::ReviewReady => {
                if let Some(item) = item.as_mut() {
                    item.state = WorkerItemState::AwaitingReview;
                    item.outcome = Some("verified_draft_delivered".into());
                }
                match job.detail.publication.as_ref() {
                    Some(publication)
                        if Some(&publication.head_sha) == run.candidate_sha.as_ref()
                            && publication.base_sha == job.detail.task.base_sha
                            && publication.state == "merged" =>
                    {
                        if let Some(item) = item.as_mut() {
                            item.state = WorkerItemState::Done;
                            item.outcome = Some("confirmed_merged".into());
                        }
                        worker.detail.worker.active_job_id = None;
                        worker.detail.worker.active_run_id = None;
                        worker.detail.worker.current_item_id = None;
                        worker.detail.worker.next_check_at = Some(now());
                        let next = if worker.detail.worker.desired_state
                            == WorkerDesiredState::Active
                            && worker.grant_active
                        {
                            WorkerState::Queued
                        } else {
                            WorkerState::Paused
                        };
                        transition(&mut worker, next, None);
                        if next == WorkerState::Paused {
                            worker.detail.worker.next_check_at = None;
                        }
                    }
                    Some(publication)
                        if Some(&publication.head_sha) == run.candidate_sha.as_ref()
                            && publication.base_sha == job.detail.task.base_sha
                            && publication.state == "open"
                            && !matches!(
                                worker.detail.worker.reason.as_deref(),
                                Some(
                                    "publication_disposition_unknown"
                                        | "review_disposition_required"
                                )
                            ) =>
                    {
                        transition(&mut worker, WorkerState::WaitingReview, None);
                        worker.detail.worker.next_check_at = None;
                    }
                    _ => {
                        if let Some(item) = item.as_mut() {
                            item.state = WorkerItemState::Blocked;
                            item.outcome = Some("review_disposition_required".into());
                        }
                        transition(
                            &mut worker,
                            WorkerState::Blocked,
                            Some("review_disposition_required"),
                        );
                        worker.detail.worker.next_check_at = None;
                    }
                }
            }
            JobState::Completed => {
                worker.detail.worker.active_job_id = None;
                worker.detail.worker.active_run_id = None;
                if worker.detail.worker.desired_state == WorkerDesiredState::Paused
                    || !worker.grant_active
                {
                    transition(&mut worker, WorkerState::Paused, None);
                    worker.detail.worker.next_check_at = None;
                } else {
                    let next = if worker.detail.worker.current_item_id.is_some() {
                        WorkerState::Queued
                    } else {
                        WorkerState::Idle
                    };
                    transition(&mut worker, next, None);
                }
            }
            _ => {
                worker.detail.worker.active_job_id = None;
                worker.detail.worker.active_run_id = None;
                if let Some(item) = item.as_mut() {
                    item.state = WorkerItemState::Blocked;
                    item.outcome = job.detail.job.reason.clone();
                }
                transition(
                    &mut worker,
                    WorkerState::Blocked,
                    job.detail
                        .job
                        .reason
                        .as_deref()
                        .or(Some("execution_failed")),
                );
                worker.detail.worker.next_check_at = None;
            }
        }
    } else if worker.stop_requested {
        transition(
            &mut worker,
            WorkerState::Stopping,
            Some("awaiting_positive_stop"),
        );
    } else if worker.detail.worker.desired_state == WorkerDesiredState::Paused {
        transition(&mut worker, WorkerState::Pausing, None);
    } else {
        let state = match job.detail.job.state {
            JobState::Queued => WorkerState::Queued,
            JobState::Preparing => WorkerState::Preparing,
            JobState::Running => {
                if context.purpose == WorkerRunPurpose::Inspection {
                    WorkerState::Inspecting
                } else {
                    WorkerState::Working
                }
            }
            JobState::NeedsInput => WorkerState::NeedsInput,
            JobState::Verifying => WorkerState::Verifying,
            JobState::Publishing => WorkerState::Publishing,
            JobState::RecoveryRequired => WorkerState::RecoveryRequired,
            JobState::Cancelling => WorkerState::Stopping,
            _ => WorkerState::Blocked,
        };
        transition(&mut worker, state, job.detail.job.reason.as_deref());
    }
    if job.detail.job.state == JobState::NeedsInput && !worker.stop_requested {
        worker.detail.worker.attention = true;
        worker.detail.worker.attention_question_id = job
            .detail
            .questions
            .iter()
            .rev()
            .find(|q| {
                q.run_id == run.id
                    && matches!(
                        q.state,
                        JobQuestionState::Pending
                            | JobQuestionState::AnswerAccepted
                            | JobQuestionState::AnswerDelivered
                    )
            })
            .map(|q| q.id);
    }
    worker.detail.worker.attention_run_id = worker.detail.worker.attention.then_some(run.id);
    if let Some(item) = item.as_mut() {
        item.updated_at = now();
        store::save_item(tx, item).await?;
    }
    if serde_json::to_string(&worker.detail.worker)? != prior {
        let kind = if worker.detail.worker.state == WorkerState::NeedsInput {
            WorkerEventKind::Question
        } else if matches!(worker.detail.worker.state, WorkerState::WaitingReview) {
            WorkerEventKind::Review
        } else {
            WorkerEventKind::RunChanged
        };
        let mut event =
            store::append(tx, &mut worker, kind, "Bounded execution state reconciled").await?;
        event.job_id = Some(job.detail.job.id);
        event.run_id = Some(run.id);
        event.question_id = job
            .detail
            .questions
            .iter()
            .rev()
            .find(|q| {
                q.run_id == run.id
                    && matches!(
                        q.state,
                        JobQuestionState::Pending
                            | JobQuestionState::AnswerAccepted
                            | JobQuestionState::AnswerDelivered
                    )
            })
            .map(|q| q.id);
        sqlx::query("UPDATE autonomous_worker_events SET document=? WHERE worker_id=? AND seq=?")
            .bind(serde_json::to_string(&event)?)
            .bind(event.worker_id.to_string())
            .bind(i64::try_from(event.seq)?)
            .execute(&mut **tx)
            .await?;
        Ok(Some(event))
    } else {
        store::save(tx, &worker).await?;
        Ok(None)
    }
}
fn budget_counters(budget: &WorkerBudget) -> (JobUsage, u32, u64, u64, u32, bool) {
    (
        budget.usage.clone(),
        budget.runs,
        budget.reserved_elapsed_seconds,
        budget.reserved_spend_micro_usd,
        budget.reserved_runs,
        budget.accounting_uncertain,
    )
}

pub(crate) async fn runtime_allowed(state: &AppState, job: &StoredJob) -> Result<bool> {
    let Some(context) = &job.detail.task.worker else {
        return Ok(true);
    };
    let Some(worker) = state.db.worker(context.worker_id).await? else {
        return Ok(false);
    };
    Ok(worker.grant_active
        && !worker.stop_requested
        && worker.detail.mandate.revision == context.mandate_revision
        && worker.detail.worker.active_job_id == Some(job.detail.job.id)
        && policy_allows(
            &worker.detail.mandate.spec,
            &effective_job_policy(&state.db, &worker.detail.worker.project_id)
                .await?
                .policy,
        ))
}
pub(crate) async fn fence_project(tx: &mut JobTx<'_>, project: &str, reason: &str) -> Result<()> {
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM autonomous_workers WHERE project_id=?")
            .bind(project)
            .fetch_all(&mut **tx)
            .await?;
    for id in ids {
        let mut worker = store::load(tx, Uuid::parse_str(&id)?).await?;
        if worker.stop_requested
            && worker.detail.worker.desired_state == WorkerDesiredState::Paused
            && (reason != "project_deleting"
                || (!worker.grant_active && worker.detail.worker.active_job_id.is_none()))
        {
            continue;
        }
        worker.schedule_generation += 1;
        worker.stop_requested = true;
        worker.detail.worker.desired_state = WorkerDesiredState::Paused;
        worker.detail.worker.next_check_at = None;
        if reason == "project_deleting" {
            worker.grant_active = false;
        }
        let mut state = WorkerState::Paused;
        if let Some(id) = worker.detail.worker.active_job_id {
            let job = jobs_store::load(tx, id).await?;
            let stopped = job.detail.job.state.is_terminal()
                && job.detail.runs.iter().all(|r| {
                    r.started_at.is_none()
                        || r.stop_evidence
                            .as_ref()
                            .is_some_and(|e| e.descendants_stopped)
                });
            if stopped && reason == "project_deleting" {
                store::release_execution(tx, worker.detail.worker.id, id).await?;
                worker.detail.worker.active_job_id = None;
                worker.detail.worker.active_run_id = None;
            } else {
                state = if stopped {
                    WorkerState::WaitingReview
                } else {
                    WorkerState::Stopping
                };
            }
        }
        transition(&mut worker, state, Some(reason));
        store::append(
            tx,
            &mut worker,
            WorkerEventKind::StateChanged,
            "Project lifecycle fenced recurrence",
        )
        .await?;
    }
    Ok(())
}
pub(crate) async fn reconcile_authority(state: &AppState) -> Result<()> {
    for old in state.db.all_workers().await? {
        if !old.grant_active {
            continue;
        }
        let permitted = authorized(
            state,
            old.detail.mandate.approved_by,
            &old,
            JobAction::Admit,
        )
        .await
        .unwrap_or(false)
            && effective_job_policy(&state.db, &old.detail.worker.project_id)
                .await
                .is_ok_and(|p| policy_allows(&old.detail.mandate.spec, &p.policy));
        if permitted {
            continue;
        }
        let mut tx = state.db.job_transaction().await?;
        let mut worker = store::load(&mut tx, old.detail.worker.id).await?;
        if !worker.grant_active {
            continue;
        }
        worker.grant_active = false;
        worker.stop_requested = true;
        worker.schedule_generation += 1;
        worker.detail.worker.desired_state = WorkerDesiredState::Paused;
        worker.detail.worker.next_check_at = None;
        let mut next = WorkerState::Blocked;
        if let Some(id) = worker.detail.worker.active_job_id {
            let mut job = jobs_store::load(&mut tx, id).await?;
            next = if job.detail.job.state.is_terminal() {
                WorkerState::WaitingReview
            } else {
                WorkerState::Stopping
            };
            jobs::cancel_locked(&mut tx, &mut job, "worker_authority_revoked").await?;
        }
        transition(&mut worker, next, Some("worker_reapproval_required"));
        store::append(
            &mut tx,
            &mut worker,
            WorkerEventKind::StateChanged,
            "Standing grant revoked; renewed approval required",
        )
        .await?;
        tx.commit().await?;
        socket::fanout(state, old.detail.worker.id).await;
    }
    socket::reconcile_subscriptions(state).await
}

/// Runs on the existing dispatcher, never on a browser connection or daemon timer.
pub(crate) async fn tick(state: &AppState) -> Result<()> {
    crate::notifications::drain_worker_notifications(state).await?;
    let time = now();
    let mut refreshes = 0;
    for worker in state.db.all_workers().await? {
        if refreshes >= 16 {
            break;
        }
        if worker.detail.worker.state != WorkerState::WaitingReview || !worker.grant_active {
            continue;
        }
        let (Some(job), Some(run)) = (
            worker.detail.worker.active_job_id,
            worker.detail.worker.active_run_id,
        ) else {
            continue;
        };
        if !state
            .sessions
            .is_daemon_connected(&worker.detail.worker.machine_id)
        {
            continue;
        }
        let Some(stored) = state.db.job(job).await? else {
            continue;
        };
        if stored
            .detail
            .publication
            .as_ref()
            .and_then(|p| chrono::DateTime::parse_from_rfc3339(&p.observed_at).ok())
            .is_some_and(|at| at > Utc::now() - Duration::seconds(60))
        {
            continue;
        }
        let mut tx = state.db.job_transaction().await?;
        let recent: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM autonomous_job_review_refreshes r JOIN autonomous_job_outbox o ON o.id=r.command_id WHERE o.job_id=? AND r.requested_at>?")
            .bind(job.to_string()).bind((Utc::now() - Duration::seconds(60)).to_rfc3339()).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        if recent > 0 {
            continue;
        }
        refreshes += 1;
        if let Err(error) =
            jobs::refresh_review(state, worker.detail.mandate.approved_by, job, run).await
        {
            tracing::debug!(%error,worker_id=%worker.detail.worker.id,"Worker review refresh deferred");
        }
    }
    for old in state.db.due_workers(&time, 32).await? {
        if state.jobs.worker_claims.len() >= 8 {
            break;
        }
        let id = old.detail.worker.id;
        if state.jobs.worker_claims.insert(id, ()).is_some() {
            continue;
        }
        let state = state.clone();
        tokio::spawn(async move {
            let _claim = PendingClaim { state: &state, id };
            if let Err(error) = claim_with_outcome(&state, &old).await {
                tracing::warn!(%error,worker_id=%id,"Worker eligibility reconciliation failed");
            }
        });
    }
    Ok(())
}
struct PendingClaim<'a> {
    state: &'a AppState,
    id: Uuid,
}
impl Drop for PendingClaim<'_> {
    fn drop(&mut self) {
        self.state.jobs.worker_claims.remove(&self.id);
    }
}
async fn claim_with_outcome(state: &AppState, old: &StoredWorker) -> Result<()> {
    if let Err(error) = claim(state, old).await {
        let reason = error.to_string();
        let mut tx = state.db.job_transaction().await?;
        let mut worker = store::load(&mut tx, old.detail.worker.id).await?;
        if worker.detail.worker.state_version != old.detail.worker.state_version
            || worker.detail.worker.active_job_id.is_some()
        {
            return Ok(());
        }
        let transient = matches!(
            reason.as_str(),
            "worker_target_unavailable" | "capacity_wait" | "prepare_unavailable"
        );
        if reason == "worker_budget_exhausted" || reason == "worker_accounting_uncertain" {
            worker.detail.budget = store::budget(&mut tx, &worker, &now()).await?;
            worker.detail.worker.next_check_at = worker.detail.budget.eligible_at.clone();
            transition(
                &mut worker,
                if reason == "worker_accounting_uncertain" {
                    WorkerState::RecoveryRequired
                } else {
                    WorkerState::WaitingBudget
                },
                Some(&reason),
            );
        } else {
            worker.detail.worker.next_check_at =
                transient.then(|| (Utc::now() + Duration::seconds(30)).to_rfc3339());
            transition(
                &mut worker,
                if transient {
                    WorkerState::Queued
                } else {
                    WorkerState::Blocked
                },
                Some(&reason),
            );
        }
        store::append(
            &mut tx,
            &mut worker,
            WorkerEventKind::StateChanged,
            "Worker dispatch eligibility changed",
        )
        .await?;
        tx.commit().await?;
        socket::fanout(state, old.detail.worker.id).await;
    }
    Ok(())
}
async fn claim(state: &AppState, old: &StoredWorker) -> Result<()> {
    let id = old.detail.worker.id;
    let project = &old.detail.worker.project_id;
    let actor = old.detail.mandate.approved_by;
    let target = targets(state, actor, project)
        .await?
        .targets
        .into_iter()
        .find(|t| {
            t.available
                && t.machine_id == old.detail.worker.machine_id
                && t.cluster_owner_user_id == old.detail.worker.cluster_owner_user_id
        })
        .ok_or_else(|| anyhow::anyhow!("worker_target_unavailable"))?;
    ensure!(
        policy_allows(&old.detail.mandate.spec, &target.policy.policy),
        "worker_policy_denied"
    );
    let prepared = jobs::prepare(
        state,
        actor,
        project,
        &target,
        &old.detail.mandate.spec.base_ref,
        &old.detail.mandate.spec.worker_profile,
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
        anyhow::bail!("approved_base_unavailable")
    };
    ensure!(
        repository == old.detail.mandate.repository
            && base_ref == old.detail.mandate.spec.base_ref
            && base_sha.len() == 40
            && base_sha.bytes().all(|b| b.is_ascii_hexdigit())
            && profile.ready
            && profile.digest == old.detail.mandate.worker_profile_digest
            && profile
                .verification_profiles
                .iter()
                .any(|p| p.key == old.detail.mandate.spec.verification_profile
                    && p.digest == old.detail.mandate.verification_profile_digest),
        "approved_repository_or_profile_changed"
    );
    let mut tx = state.db.job_transaction().await?;
    let mut worker = store::load(&mut tx, id).await?;
    if worker.detail.worker.state_version != old.detail.worker.state_version
        || worker.schedule_generation != old.schedule_generation
        || worker.detail.worker.backlog_revision != old.detail.worker.backlog_revision
        || worker.detail.worker.active_job_id.is_some()
        || !worker.grant_active
        || worker.stop_requested
        || worker.detail.worker.desired_state != WorkerDesiredState::Active
    {
        return Ok(());
    }
    let time = now();
    ensure!(
        worker
            .detail
            .worker
            .next_check_at
            .as_ref()
            .is_some_and(|at| at <= &time),
        "worker_not_due"
    );
    ensure!(
        authorized(state, actor, &worker, JobAction::Admit).await?,
        "worker_access_denied"
    );
    let policy = effective_job_policy(&state.db, project).await?;
    ensure!(
        policy_allows(&worker.detail.mandate.spec, &policy.policy),
        "worker_policy_denied"
    );
    let fenced: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM autonomous_job_project_fences WHERE project_id=?")
            .bind(project)
            .fetch_one(&mut *tx)
            .await?;
    ensure!(fenced == 0, "project_stop_pending");
    let counts = sqlx::query("SELECT COALESCE(SUM(project_id=?),0) AS project_count,COALESCE(SUM(owner=? AND machine=?),0) AS machine_count,COALESCE(SUM(owner=?),0) AS cluster_count FROM autonomous_jobs WHERE json_extract(document,'$.dispatched')=1 AND state NOT IN ('review_ready','completed','cancelled','failed')")
        .bind(project).bind(worker.detail.worker.cluster_owner_user_id.to_string()).bind(worker.detail.worker.machine_id.to_string())
        .bind(worker.detail.worker.cluster_owner_user_id.to_string()).fetch_one(&mut *tx).await?;
    ensure!(
        counts.get::<i64, _>("project_count") < policy.policy.max_active_project as i64
            && counts.get::<i64, _>("machine_count") < policy.policy.max_active_machine as i64
            && counts.get::<i64, _>("cluster_count") < policy.policy.max_active_cluster as i64,
        "capacity_wait"
    );
    let mut backlog = store::items(&mut tx, id).await?;
    let mut limits = worker.detail.mandate.spec.limits.per_run.clone();
    let job_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let reservation_id = Uuid::new_v4();
    let (purpose, instruction, acceptance_criteria) =
        if let Some(selected) = worker.detail.worker.current_item_id {
            let item = backlog
                .iter_mut()
                .find(|i| i.id == selected)
                .ok_or_else(|| anyhow::anyhow!("selected_item_not_found"))?;
            ensure!(
                item.state == WorkerItemState::Selected
                    && item.attempts < worker.detail.mandate.spec.limits.attempts_per_item,
                "selected_item_ineligible"
            );
            ensure!(
                dependencies_done(&mut tx, id, &item.item.dependencies).await?,
                "selected_item_dependencies"
            );
            let usage = store::item_usage(&mut tx, id, selected).await?;
            limits.elapsed_seconds = limits.elapsed_seconds.saturating_sub(usage.elapsed_seconds);
            limits.spend_micro_usd = limits.spend_micro_usd.saturating_sub(
                usage
                    .spend_micro_usd
                    .ok_or_else(|| anyhow::anyhow!("worker_accounting_uncertain"))?,
            );
            ensure!(
                limits.elapsed_seconds > 0 && limits.spend_micro_usd > 0,
                "item_allowance_exhausted"
            );
            item.attempts += 1;
            item.state = WorkerItemState::Running;
            item.job_id = Some(job_id);
            item.run_id = Some(run_id);
            item.updated_at = time.clone();
            store::save_item(&mut tx, item).await?;
            (
                WorkerRunPurpose::Implementation,
                item.item.instruction.clone(),
                item.item.acceptance_criteria.clone(),
            )
        } else {
            (
                WorkerRunPurpose::Inspection,
                worker.detail.mandate.spec.description.clone(),
                worker.detail.mandate.spec.checklist.clone(),
            )
        };
    let context = WorkerExecutionContext {
        worker_id: id,
        mandate_revision: worker.detail.mandate.revision,
        backlog_revision: worker.detail.worker.backlog_revision,
        schedule_generation: worker.schedule_generation,
        reservation_id,
        purpose,
        mandate: worker.detail.mandate.spec.clone(),
        items: backlog,
        selected_item_id: worker.detail.worker.current_item_id,
        checkpoint: worker.detail.checkpoint.clone(),
    };
    ensure!(
        serde_json::to_vec(&context)?.len() <= WORKER_MAX_CONTEXT_BYTES,
        "worker_context_too_large"
    );
    let task = JobTaskSpec {
        instruction,
        acceptance_criteria,
        source_url: None,
        repository,
        base_ref,
        base_sha,
        worker_profile: worker.detail.mandate.spec.worker_profile.clone(),
        worker_profile_digest: worker.detail.mandate.worker_profile_digest.clone(),
        verification_profile: worker.detail.mandate.spec.verification_profile.clone(),
        verification_profile_digest: worker.detail.mandate.verification_profile_digest.clone(),
        model_profile: worker.detail.mandate.spec.model_profile.clone(),
        worker: Some(context),
    };
    let usage = JobUsage {
        spend_micro_usd: Some(0),
        ..Default::default()
    };
    let expires = (Utc::now() + Duration::seconds(JOB_LEASE_SECONDS as i64)).to_rfc3339();
    let mut job = StoredJob {
        detail: JobDetail {
            job: JobSummary {
                id: job_id,
                project_id: project.clone(),
                project_name: project.clone(),
                title: worker.detail.worker.title.clone(),
                state: JobState::Preparing,
                state_version: 1,
                current_run_id: run_id,
                cluster_owner_user_id: worker.detail.worker.cluster_owner_user_id,
                machine_id: worker.detail.worker.machine_id,
                hostname: worker.detail.worker.hostname.clone(),
                created_at: time.clone(),
                updated_at: time.clone(),
                usage: usage.clone(),
                limits: limits.clone(),
                attention_count: 0,
                reason: None,
                allowed_actions: Default::default(),
            },
            task,
            runs: vec![JobRun {
                id: run_id,
                job_id,
                attempt: 1,
                epoch: 1,
                state: JobState::Preparing,
                usage,
                started_at: Some(time.clone()),
                ended_at: None,
                worker_version: None,
                candidate_sha: None,
                stop_evidence: None,
                reason: None,
            }],
            questions: vec![],
            high_watermark: 0,
            policy_digest: policy.digest.clone(),
            publication: None,
        },
        initiator: actor,
        review: None,
        dispatched: true,
        source_seq: 0,
        lease_expires_at: Some(expires.clone()),
        last_renewed_at: Some(time),
    };
    jobs_store::save(&mut tx, &job).await?;
    store::reserve(
        &mut tx,
        &mut worker,
        reservation_id,
        job_id,
        run_id,
        &limits,
        &now(),
    )
    .await?;
    let command_id = Uuid::new_v4();
    jobs_store::command(
        &mut tx,
        &job,
        command_id,
        JobWorkerCommand::Start {
            command_id,
            identity: job.identity(),
            task: job.detail.task.clone(),
            limits,
            policy_digest: policy.digest,
            lease_expires_at: expires,
        },
    )
    .await?;
    jobs_store::append(
        &mut tx,
        &mut job,
        JobEventKind::Admitted,
        "Worker allowance and execution atomically reserved",
        None,
        None,
    )
    .await?;
    worker.detail.worker.active_job_id = Some(job_id);
    worker.detail.worker.active_run_id = Some(run_id);
    worker.detail.worker.next_check_at = None;
    transition(&mut worker, WorkerState::Preparing, None);
    store::append(
        &mut tx,
        &mut worker,
        WorkerEventKind::RunChanged,
        "Bounded worker execution claimed",
    )
    .await?;
    tx.commit().await?;
    socket::fanout(state, id).await;
    Ok(())
}

#[cfg(test)]
mod tests;
