use super::*;
use crate::db::{jobs::StoredJob, User};
use sqlx::sqlite::SqlitePoolOptions;

const START: &str = "2026-01-01T00:00:00.000Z";
const MINUTE: &str = "2026-01-01T00:01:00.000Z";
const TOMORROW: &str = "2026-01-02T00:02:00.000Z";

async fn fixture() -> (Database, StoredWorker) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys=ON")
        .execute(&pool)
        .await
        .unwrap();
    let db = Database { pool };
    db.run_migrations().await.unwrap();
    let actor = Uuid::new_v4();
    db.create_user(&User {
        id: actor.to_string(),
        email: format!("{actor}@workers.test"),
        password_hash: "unused".into(),
        created_at: None,
        cluster_role: "user".into(),
        account_status: "active".into(),
    })
    .await
    .unwrap();
    let project = Uuid::new_v4().to_string();
    db.authorize_project_registration(&project, &actor.to_string())
        .await
        .unwrap();
    let machine = Uuid::new_v4();
    let spec = WorkerMandateSpec {
        title: "Repository maintenance".into(),
        description: "Inspect and maintain the approved repository".into(),
        checklist: vec!["Look for actionable defects".into()],
        machine_id: machine,
        worker_profile: "worker".into(),
        verification_profile: "verify".into(),
        model_profile: "model".into(),
        base_ref: "main".into(),
        interval_seconds: 900,
        limits: WorkerLimits {
            per_run: JobLimits {
                elapsed_seconds: 100,
                spend_micro_usd: 1000,
                ..JobLimits::default()
            },
            elapsed_seconds_24h: 200,
            spend_micro_usd_24h: 2000,
            runs_24h: 2,
            attempts_per_item: 2,
        },
    };
    let worker = StoredWorker {
        detail: WorkerDetail {
            worker: WorkerSummary {
                id: Uuid::new_v4(),
                project_id: project,
                title: spec.title.clone(),
                description: spec.description.clone(),
                state: WorkerState::Idle,
                reason: None,
                desired_state: WorkerDesiredState::Active,
                mandate_revision: 1,
                backlog_revision: 0,
                state_version: 1,
                high_watermark: 0,
                next_check_at: Some(START.into()),
                active_job_id: None,
                active_run_id: None,
                current_item_id: None,
                machine_id: machine,
                cluster_owner_user_id: actor,
                hostname: "test-host".into(),
                attention: false,
                attention_run_id: None,
                attention_question_id: None,
                created_at: START.into(),
                updated_at: START.into(),
            },
            mandate: WorkerMandate {
                revision: 1,
                spec,
                repository: "https://example.test/repo.git".into(),
                worker_profile_digest: "worker-digest".into(),
                verification_profile_digest: "verify-digest".into(),
                policy_digest: "policy".into(),
                approved_by: actor,
                approved_at: START.into(),
            },
            checkpoint: None,
            budget: WorkerBudget::default(),
            allowed_actions: WorkerAllowedActions::default(),
            current_job: None,
        },
        schedule_generation: 1,
        grant_active: true,
        stop_requested: false,
        accounting_at: START.into(),
    };
    let mut tx = db.job_transaction().await.unwrap();
    save(&mut tx, &worker).await.unwrap();
    tx.commit().await.unwrap();
    (db, worker)
}

fn item(worker: &StoredWorker) -> WorkerItem {
    WorkerItem {
        id: Uuid::new_v4(),
        worker_id: worker.detail.worker.id,
        revision: 1,
        item: WorkerItemDraft {
            title: "Fix a defect".into(),
            instruction: "Reproduce and repair the defect".into(),
            acceptance_criteria: vec!["The regression no longer occurs".into()],
            scope_rationale: "Maintains the repository".into(),
            evidence: vec![],
            priority: 0,
            dependencies: vec![],
        },
        origin: WorkerItemOrigin::Human,
        state: WorkerItemState::Ready,
        attempts: 0,
        job_id: None,
        run_id: None,
        outcome: None,
        created_at: START.into(),
        updated_at: START.into(),
    }
}

fn job(worker: &StoredWorker, selected: Option<&WorkerItem>) -> (StoredJob, Uuid) {
    let id = Uuid::new_v4();
    let run = Uuid::new_v4();
    let reservation = Uuid::new_v4();
    let mandate = &worker.detail.mandate;
    let w = &worker.detail.worker;
    let usage = JobUsage {
        spend_micro_usd: Some(0),
        ..JobUsage::default()
    };
    (
        StoredJob {
            detail: JobDetail {
                job: JobSummary {
                    id,
                    project_id: w.project_id.clone(),
                    project_name: "test".into(),
                    title: "Inspection".into(),
                    state: JobState::Queued,
                    state_version: 1,
                    current_run_id: run,
                    cluster_owner_user_id: w.cluster_owner_user_id,
                    machine_id: w.machine_id,
                    hostname: w.hostname.clone(),
                    created_at: START.into(),
                    updated_at: START.into(),
                    usage: usage.clone(),
                    limits: mandate.spec.limits.per_run.clone(),
                    attention_count: 0,
                    reason: None,
                    allowed_actions: JobAllowedActions::default(),
                },
                task: JobTaskSpec {
                    instruction: "Inspect approved responsibility".into(),
                    acceptance_criteria: vec!["Return a checkpoint".into()],
                    source_url: None,
                    repository: mandate.repository.clone(),
                    base_ref: mandate.spec.base_ref.clone(),
                    base_sha: "a".repeat(40),
                    worker_profile: mandate.spec.worker_profile.clone(),
                    worker_profile_digest: mandate.worker_profile_digest.clone(),
                    verification_profile: mandate.spec.verification_profile.clone(),
                    verification_profile_digest: mandate.verification_profile_digest.clone(),
                    model_profile: mandate.spec.model_profile.clone(),
                    worker: Some(WorkerExecutionContext {
                        worker_id: w.id,
                        mandate_revision: mandate.revision,
                        backlog_revision: w.backlog_revision,
                        schedule_generation: worker.schedule_generation,
                        reservation_id: reservation,
                        purpose: if selected.is_some() {
                            WorkerRunPurpose::Implementation
                        } else {
                            WorkerRunPurpose::Inspection
                        },
                        mandate: mandate.spec.clone(),
                        items: selected.into_iter().cloned().collect(),
                        selected_item_id: selected.map(|i| i.id),
                        checkpoint: worker.detail.checkpoint.clone(),
                    }),
                },
                runs: vec![JobRun {
                    id: run,
                    job_id: id,
                    attempt: 1,
                    epoch: 1,
                    state: JobState::Queued,
                    usage,
                    started_at: None,
                    ended_at: None,
                    worker_version: None,
                    candidate_sha: None,
                    stop_evidence: None,
                    reason: None,
                }],
                questions: vec![],
                high_watermark: 0,
                policy_digest: mandate.policy_digest.clone(),
                publication: None,
            },
            initiator: mandate.approved_by,
            review: None,
            dispatched: false,
            source_seq: 0,
            lease_expires_at: None,
            last_renewed_at: None,
        },
        reservation,
    )
}

async fn admit(
    tx: &mut JobTx<'_>,
    worker: &mut StoredWorker,
    value: &StoredJob,
    reservation: Uuid,
    now: &str,
) -> Result<()> {
    jobs::save(tx, value).await?;
    reserve(
        tx,
        worker,
        reservation,
        value.detail.job.id,
        value.detail.job.current_run_id,
        &value.detail.job.limits,
        now,
    )
    .await
}

async fn finish(
    tx: &mut JobTx<'_>,
    worker: &mut StoredWorker,
    value: &mut StoredJob,
    reservation: Uuid,
    usage: &JobUsage,
    now: &str,
) {
    value.detail.job.state = JobState::Completed;
    value.detail.job.usage = usage.clone();
    value.current().state = JobState::Completed;
    value.current().usage = usage.clone();
    value.current().started_at = Some(START.into());
    value.current().ended_at = Some(now.into());
    value.current().stop_evidence = Some(JobStopEvidence {
        boot_id: "boot".into(),
        container_id: "container".into(),
        stopped_at: now.into(),
        reason: "settled".into(),
        descendants_stopped: true,
    });
    jobs::save(tx, value).await.unwrap();
    reconcile_usage(tx, worker, reservation, usage, true, now)
        .await
        .unwrap();
    release_execution(tx, worker.detail.worker.id, value.detail.job.id)
        .await
        .unwrap();
}

#[tokio::test]
async fn migration_is_idempotent_and_event_receipts_are_atomic_actor_scoped() {
    let (db, mut worker) = fixture().await;
    db.migrate_workers().await.unwrap();
    let actor = worker.detail.mandate.approved_by;
    let request = Uuid::new_v4();
    let mut tx = db.job_transaction().await.unwrap();
    let event = append(&mut tx, &mut worker, WorkerEventKind::Approved, "approved")
        .await
        .unwrap();
    let value = WorkerMutationReceipt {
        request_id: request,
        operation: WorkerMutationKind::Approve,
        status: JobReceiptStatus::Completed,
        worker_id: Some(worker.detail.worker.id),
        project_id: worker.detail.worker.project_id.clone(),
        state_version: Some(event.state_version),
        error_code: None,
    };
    save_receipt(&mut tx, actor, "payload", &value)
        .await
        .unwrap();
    save_receipt(&mut tx, actor, "payload", &value)
        .await
        .unwrap();
    assert_eq!(
        receipt(&mut tx, actor, request, "payload")
            .await
            .unwrap()
            .unwrap()
            .state_version,
        Some(event.state_version)
    );
    assert!(receipt(&mut tx, actor, request, "changed").await.is_err());
    assert!(receipt(&mut tx, Uuid::new_v4(), request, "payload")
        .await
        .unwrap()
        .is_none());
    tx.commit().await.unwrap();
    let replay = db
        .worker_events(worker.detail.worker.id, 0, 1)
        .await
        .unwrap();
    assert_eq!(replay.events.len(), 1);
    assert_eq!(replay.high_watermark, event.seq);
    assert!(!replay.resync_required);
    let persisted = db.worker(worker.detail.worker.id).await.unwrap().unwrap();
    assert_eq!(persisted.detail.worker.state_version, event.state_version);
    assert_eq!(
        db.worker_receipt(actor, request)
            .await
            .unwrap()
            .unwrap()
            .state_version,
        Some(event.state_version)
    );

    let mut tx = db.job_transaction().await.unwrap();
    append(
        &mut tx,
        &mut worker,
        WorkerEventKind::StateChanged,
        "rolled back",
    )
    .await
    .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        db.worker_events(worker.detail.worker.id, 0, 10)
            .await
            .unwrap()
            .events
            .len(),
        1
    );
}

#[tokio::test]
async fn stale_worker_and_mandate_updates_conflict_without_rewriting_history() {
    let (db, mut worker) = fixture().await;
    let mut stale = worker.clone();
    let mut tx = db.job_transaction().await.unwrap();
    worker.detail.worker.desired_state = WorkerDesiredState::Paused;
    worker.schedule_generation += 1;
    append(
        &mut tx,
        &mut worker,
        WorkerEventKind::StateChanged,
        "paused",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let mut tx = db.job_transaction().await.unwrap();
    stale.detail.worker.title = "stale overwrite".into();
    assert!(
        append(&mut tx, &mut stale, WorkerEventKind::StateChanged, "stale")
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    let mut tx = db.job_transaction().await.unwrap();
    worker.detail.mandate.spec.description = "unreviewed change".into();
    assert!(save(&mut tx, &worker).await.is_err());
    tx.rollback().await.unwrap();
    assert_eq!(
        db.worker(worker.detail.worker.id)
            .await
            .unwrap()
            .unwrap()
            .detail
            .worker
            .desired_state,
        WorkerDesiredState::Paused
    );
}

#[tokio::test]
async fn simultaneous_claims_cannot_create_overlapping_execution() {
    let (db, worker) = fixture().await;
    let observed_a = db.due_workers(TOMORROW, 100).await.unwrap();
    let observed_b = db.due_workers(TOMORROW, 100).await.unwrap();
    assert_eq!(observed_a.len(), 1);
    assert_eq!(observed_b.len(), 1);
    async fn claim(db: &Database, id: Uuid) -> Result<Uuid> {
        let mut tx = db.job_transaction().await?;
        let mut worker = load(&mut tx, id).await?;
        let (value, reservation) = job(&worker, None);
        admit(&mut tx, &mut worker, &value, reservation, START).await?;
        let id = value.detail.job.id;
        worker.detail.worker.active_job_id = Some(id);
        worker.detail.worker.active_run_id = Some(value.detail.job.current_run_id);
        worker.detail.worker.state = WorkerState::Inspecting;
        append(&mut tx, &mut worker, WorkerEventKind::RunChanged, "claimed").await?;
        tx.commit().await?;
        Ok(id)
    }
    let (a, b) = tokio::join!(
        claim(&db, worker.detail.worker.id),
        claim(&db, worker.detail.worker.id)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert!(db.due_workers(TOMORROW, 100).await.unwrap().is_empty());
    assert_eq!(
        db.worker_runs(worker.detail.worker.id, None)
            .await
            .unwrap()
            .runs
            .len(),
        1
    );
    let jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM autonomous_jobs")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        jobs, 1,
        "losing claim rolls back its job as well as its reservation"
    );
}

#[tokio::test]
async fn reservation_replay_and_release_require_exact_identity_and_positive_stop() {
    let (db, mut worker) = fixture().await;
    let (mut value, reservation) = job(&worker, None);
    let mut tx = db.job_transaction().await.unwrap();
    admit(&mut tx, &mut worker, &value, reservation, START)
        .await
        .unwrap();
    admit(&mut tx, &mut worker, &value, reservation, START)
        .await
        .unwrap();
    assert_eq!(budget(&mut tx, &worker, START).await.unwrap().runs, 1);
    value.detail.job.state = JobState::Completed;
    value.current().state = JobState::Completed;
    value.current().started_at = Some(START.into());
    jobs::save(&mut tx, &value).await.unwrap();
    assert!(
        release_execution(&mut tx, worker.detail.worker.id, value.detail.job.id)
            .await
            .is_err()
    );
    let usage = JobUsage {
        elapsed_seconds: 20,
        spend_micro_usd: Some(200),
        ..JobUsage::default()
    };
    finish(
        &mut tx,
        &mut worker,
        &mut value,
        reservation,
        &usage,
        MINUTE,
    )
    .await;
    tx.commit().await.unwrap();
    assert_eq!(
        db.worker_run(worker.detail.worker.id, value.detail.job.current_run_id)
            .await
            .unwrap(),
        Some(value.detail.job.id)
    );
    assert!(db
        .worker_run(Uuid::new_v4(), value.detail.job.current_run_id)
        .await
        .unwrap()
        .is_none());
    let mut tx = db.job_transaction().await.unwrap();
    assert!(
        sqlx::query("UPDATE autonomous_worker_executions SET worker_id=? WHERE job_id=?")
            .bind(Uuid::new_v4().to_string())
            .bind(value.detail.job.id.to_string())
            .execute(&mut *tx)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn rolling_allowance_survives_reload_and_duplicate_or_late_reports() {
    let (db, mut worker) = fixture().await;
    let (mut first, reservation) = job(&worker, None);
    let usage = JobUsage {
        elapsed_seconds: 60,
        spend_micro_usd: Some(600),
        estimated: true,
        ..JobUsage::default()
    };
    let mut tx = db.job_transaction().await.unwrap();
    admit(&mut tx, &mut worker, &first, reservation, START)
        .await
        .unwrap();
    finish(
        &mut tx,
        &mut worker,
        &mut first,
        reservation,
        &usage,
        MINUTE,
    )
    .await;
    reconcile_usage(&mut tx, &mut worker, reservation, &usage, true, MINUTE)
        .await
        .unwrap();
    save(&mut tx, &worker).await.unwrap();
    tx.commit().await.unwrap();
    let mut tx = db.job_transaction().await.unwrap();
    let mut restored = load(&mut tx, worker.detail.worker.id).await.unwrap();
    let initial = budget(&mut tx, &restored, MINUTE).await.unwrap();
    assert_eq!(initial.usage.elapsed_seconds, 60);
    assert_eq!(initial.usage.spend_micro_usd, Some(600));
    assert!(initial.usage.estimated);
    let late = JobUsage {
        elapsed_seconds: 80,
        spend_micro_usd: Some(900),
        ..usage.clone()
    };
    reconcile_usage(&mut tx, &mut restored, reservation, &late, true, TOMORROW)
        .await
        .unwrap();
    reconcile_usage(&mut tx, &mut restored, reservation, &late, true, TOMORROW)
        .await
        .unwrap();
    let current = budget(&mut tx, &restored, TOMORROW).await.unwrap();
    assert_eq!(current.usage.elapsed_seconds, 20);
    assert_eq!(current.usage.spend_micro_usd, Some(300));
    assert_eq!(current.runs, 0);
    let backwards = budget(&mut tx, &restored, START).await.unwrap();
    assert_eq!(backwards.window_started_at, current.window_started_at);
    assert_eq!(backwards.usage, current.usage);
}

#[tokio::test]
async fn unresolved_and_unknown_allowance_does_not_age_out_or_double_charge() {
    let (db, mut worker) = fixture().await;
    let (value, reservation) = job(&worker, None);
    let mut tx = db.job_transaction().await.unwrap();
    admit(&mut tx, &mut worker, &value, reservation, START)
        .await
        .unwrap();
    let known = JobUsage {
        elapsed_seconds: 30,
        spend_micro_usd: Some(300),
        ..JobUsage::default()
    };
    reconcile_usage(&mut tx, &mut worker, reservation, &known, false, MINUTE)
        .await
        .unwrap();
    let current = budget(&mut tx, &worker, MINUTE).await.unwrap();
    assert_eq!(current.reserved_elapsed_seconds, 70);
    assert_eq!(current.reserved_spend_micro_usd, 700);
    assert_eq!(current.usage.spend_micro_usd, Some(300));
    let aged = budget(&mut tx, &worker, TOMORROW).await.unwrap();
    assert_eq!(aged.reserved_elapsed_seconds, 100);
    assert_eq!(aged.reserved_spend_micro_usd, 1000);
    assert_eq!(aged.reserved_runs, 1);
    let unknown = JobUsage {
        elapsed_seconds: 40,
        spend_micro_usd: None,
        ..known.clone()
    };
    reconcile_usage(&mut tx, &mut worker, reservation, &unknown, true, TOMORROW)
        .await
        .unwrap();
    assert!(
        budget(&mut tx, &worker, TOMORROW)
            .await
            .unwrap()
            .accounting_uncertain
    );
    // An older known report must not make newer missing metering disappear.
    reconcile_usage(&mut tx, &mut worker, reservation, &known, true, TOMORROW)
        .await
        .unwrap();
    assert!(
        budget(&mut tx, &worker, TOMORROW)
            .await
            .unwrap()
            .accounting_uncertain
    );
    let final_usage = JobUsage {
        elapsed_seconds: 40,
        spend_micro_usd: Some(400),
        ..known
    };
    reconcile_usage(
        &mut tx,
        &mut worker,
        reservation,
        &final_usage,
        true,
        TOMORROW,
    )
    .await
    .unwrap();
    reconcile_usage(
        &mut tx,
        &mut worker,
        reservation,
        &final_usage,
        true,
        TOMORROW,
    )
    .await
    .unwrap();
    let accounted = budget(&mut tx, &worker, TOMORROW).await.unwrap();
    assert!(!accounted.accounting_uncertain);
    assert_eq!(accounted.reserved_elapsed_seconds, 0);
    assert_eq!(accounted.usage.elapsed_seconds, 40);
    assert_eq!(accounted.usage.spend_micro_usd, Some(400));
    assert_eq!(accounted.runs, 1);
}

#[tokio::test]
async fn all_runs_consume_budget_overruns_are_visible_and_eligibility_is_exact() {
    let (db, mut worker) = fixture().await;
    let mut tx = db.job_transaction().await.unwrap();
    let (mut first, first_reservation) = job(&worker, None);
    admit(&mut tx, &mut worker, &first, first_reservation, START)
        .await
        .unwrap();
    let usage = JobUsage {
        elapsed_seconds: 20,
        spend_micro_usd: Some(200),
        ..JobUsage::default()
    };
    finish(
        &mut tx,
        &mut worker,
        &mut first,
        first_reservation,
        &usage,
        MINUTE,
    )
    .await;
    let (mut second, second_reservation) = job(&worker, None);
    admit(&mut tx, &mut worker, &second, second_reservation, MINUTE)
        .await
        .unwrap();
    finish(
        &mut tx,
        &mut worker,
        &mut second,
        second_reservation,
        &usage,
        MINUTE,
    )
    .await;
    let exhausted = budget(&mut tx, &worker, MINUTE).await.unwrap();
    assert_eq!(exhausted.runs, 2);
    assert_eq!(
        exhausted.eligible_at.as_deref(),
        Some("2026-01-02T00:00:00.000Z")
    );
    let (third, third_reservation) = job(&worker, None);
    assert!(
        admit(&mut tx, &mut worker, &third, third_reservation, MINUTE)
            .await
            .is_err()
    );
    let overrun = JobUsage {
        elapsed_seconds: 500,
        spend_micro_usd: Some(5000),
        ..usage
    };
    reconcile_usage(
        &mut tx,
        &mut worker,
        second_reservation,
        &overrun,
        true,
        MINUTE,
    )
    .await
    .unwrap();
    let exhausted = budget(&mut tx, &worker, MINUTE).await.unwrap();
    assert_eq!(exhausted.usage.elapsed_seconds, 520);
    assert_eq!(exhausted.usage.spend_micro_usd, Some(5200));
    assert_eq!(
        exhausted.eligible_at.as_deref(),
        Some("2026-01-02T00:01:00.000Z")
    );
}

#[tokio::test]
async fn backlog_revisions_preserve_attempts_and_lifetime_item_usage() {
    let (db, mut worker) = fixture().await;
    let mut selected = item(&worker);
    selected.attempts = 1;
    let mut tx = db.job_transaction().await.unwrap();
    save_item(&mut tx, &selected).await.unwrap();
    let (mut execution, reservation) = job(&worker, Some(&selected));
    admit(&mut tx, &mut worker, &execution, reservation, START)
        .await
        .unwrap();
    let usage = JobUsage {
        elapsed_seconds: 60,
        spend_micro_usd: Some(600),
        ..JobUsage::default()
    };
    finish(
        &mut tx,
        &mut worker,
        &mut execution,
        reservation,
        &usage,
        MINUTE,
    )
    .await;
    selected.revision += 1;
    selected.item.title = "Refined defect".into();
    save_item(&mut tx, &selected).await.unwrap();
    assert_eq!(
        item_usage(&mut tx, worker.detail.worker.id, selected.id)
            .await
            .unwrap(),
        usage
    );
    selected.attempts = 0;
    assert!(save_item(&mut tx, &selected).await.is_err());
    selected.attempts = 1;
    selected.item.title = "Stale same-revision edit".into();
    assert!(save_item(&mut tx, &selected).await.is_err());
    tx.commit().await.unwrap();
    let restored = db
        .worker_items(worker.detail.worker.id, None, Some("active"))
        .await
        .unwrap();
    assert_eq!(restored.items[0].attempts, 1);
    assert_eq!(restored.items[0].revision, 2);
}

#[tokio::test]
async fn active_backlog_is_bounded_by_count_and_encoded_bytes_without_truncation() {
    let (db, worker) = fixture().await;
    let mut tx = db.job_transaction().await.unwrap();
    for _ in 0..WORKER_MAX_BACKLOG_ITEMS {
        save_item(&mut tx, &item(&worker)).await.unwrap();
    }
    assert_eq!(
        items(&mut tx, worker.detail.worker.id).await.unwrap().len(),
        WORKER_MAX_BACKLOG_ITEMS
    );
    assert!(save_item(&mut tx, &item(&worker)).await.is_err());
    let mut dismissed = items(&mut tx, worker.detail.worker.id)
        .await
        .unwrap()
        .remove(0);
    dismissed.state = WorkerItemState::Dismissed;
    save_item(&mut tx, &dismissed).await.unwrap();
    save_item(&mut tx, &item(&worker)).await.unwrap();
    tx.rollback().await.unwrap();
    let mut tx = db.job_transaction().await.unwrap();
    for _ in 0..20 {
        let mut large = item(&worker);
        large.item.instruction = "i".repeat(8192);
        large.item.acceptance_criteria = vec!["a".repeat(1024); 16];
        if save_item(&mut tx, &large).await.is_err() {
            let active = items(&mut tx, worker.detail.worker.id).await.unwrap();
            assert!(active.len() < WORKER_MAX_BACKLOG_ITEMS);
            assert!(
                active
                    .iter()
                    .map(|i| serde_json::to_vec(i).unwrap().len())
                    .sum::<usize>()
                    <= WORKER_MAX_CONTEXT_BYTES
            );
            return;
        }
    }
    panic!("encoded byte limit was not enforced");
}

#[tokio::test]
async fn pause_keeps_checkpoint_and_accounting_but_prevents_coalesced_due_dispatch() {
    let (db, mut worker) = fixture().await;
    assert_eq!(db.due_workers(TOMORROW, 100).await.unwrap().len(), 1);
    assert_eq!(
        db.due_workers("2026-02-01T00:00:00Z", 100)
            .await
            .unwrap()
            .len(),
        1
    );
    let (mut execution, reservation) = job(&worker, None);
    let mut tx = db.job_transaction().await.unwrap();
    admit(&mut tx, &mut worker, &execution, reservation, START)
        .await
        .unwrap();
    worker.detail.worker.desired_state = WorkerDesiredState::Paused;
    worker.schedule_generation += 1;
    append(
        &mut tx,
        &mut worker,
        WorkerEventKind::StateChanged,
        "pause after this run",
    )
    .await
    .unwrap();
    let usage = JobUsage {
        elapsed_seconds: 10,
        spend_micro_usd: Some(100),
        ..JobUsage::default()
    };
    finish(
        &mut tx,
        &mut worker,
        &mut execution,
        reservation,
        &usage,
        MINUTE,
    )
    .await;
    worker.detail.checkpoint = Some(WorkerCheckpoint {
        id: Uuid::new_v4(),
        worker_id: worker.detail.worker.id,
        run_id: execution.detail.job.current_run_id,
        mandate_revision: 1,
        backlog_revision: 0,
        base_sha: "a".repeat(40),
        summary: "No actionable work".into(),
        checklist: vec![WorkerChecklistObservation {
            index: 0,
            observation: "No defects found".into(),
        }],
        recorded_at: MINUTE.into(),
    });
    worker.detail.worker.state = WorkerState::Paused;
    append(
        &mut tx,
        &mut worker,
        WorkerEventKind::Checkpoint,
        "inspection retained while paused",
    )
    .await
    .unwrap();
    assert!(items(&mut tx, worker.detail.worker.id)
        .await
        .unwrap()
        .is_empty());
    tx.commit().await.unwrap();
    assert!(db.due_workers(TOMORROW, 100).await.unwrap().is_empty());
    let restored = db.worker(worker.detail.worker.id).await.unwrap().unwrap();
    assert_eq!(
        restored.detail.checkpoint.unwrap().summary,
        "No actionable work"
    );
    assert_eq!(restored.detail.budget.usage.elapsed_seconds, 10);
    assert_eq!(
        db.worker_runs(worker.detail.worker.id, None)
            .await
            .unwrap()
            .runs
            .len(),
        1
    );
}

#[tokio::test]
async fn replay_gaps_require_resync_without_losing_current_state() {
    let (db, mut worker) = fixture().await;
    let mut tx = db.job_transaction().await.unwrap();
    for _ in 0..3 {
        append(&mut tx, &mut worker, WorkerEventKind::StateChanged, "state")
            .await
            .unwrap();
    }
    sqlx::query("DELETE FROM autonomous_worker_events WHERE worker_id=? AND seq=1")
        .bind(worker.detail.worker.id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(
        db.worker_events(worker.detail.worker.id, 0, 1)
            .await
            .unwrap()
            .resync_required
    );
    let retained = db
        .worker_events(worker.detail.worker.id, 1, 1)
        .await
        .unwrap();
    assert!(!retained.resync_required);
    assert!(retained.has_more);
    assert_eq!(retained.events[0].seq, 2);
    assert_eq!(retained.high_watermark, 3);
    assert!(
        db.worker_events(worker.detail.worker.id, 4, 1)
            .await
            .unwrap()
            .resync_required
    );
    sqlx::query("DELETE FROM autonomous_worker_events WHERE worker_id=? AND seq=3")
        .bind(worker.detail.worker.id.to_string())
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        db.worker_events(worker.detail.worker.id, 2, 10)
            .await
            .unwrap()
            .resync_required
    );
}

#[tokio::test]
async fn mandate_narrowing_and_pause_resume_do_not_reset_consumption() {
    let (db, mut worker) = fixture().await;
    let (mut execution, reservation) = job(&worker, None);
    let mut tx = db.job_transaction().await.unwrap();
    admit(&mut tx, &mut worker, &execution, reservation, START)
        .await
        .unwrap();
    let usage = JobUsage {
        elapsed_seconds: 60,
        spend_micro_usd: Some(600),
        ..JobUsage::default()
    };
    finish(
        &mut tx,
        &mut worker,
        &mut execution,
        reservation,
        &usage,
        MINUTE,
    )
    .await;
    worker.detail.worker.desired_state = WorkerDesiredState::Paused;
    worker.schedule_generation += 1;
    append(
        &mut tx,
        &mut worker,
        WorkerEventKind::StateChanged,
        "paused",
    )
    .await
    .unwrap();
    worker.detail.mandate.revision += 1;
    worker.detail.worker.mandate_revision += 1;
    worker.detail.mandate.spec.limits.elapsed_seconds_24h = 120;
    worker.detail.mandate.spec.limits.spend_micro_usd_24h = 1200;
    worker.detail.worker.desired_state = WorkerDesiredState::Active;
    worker.schedule_generation += 1;
    append(
        &mut tx,
        &mut worker,
        WorkerEventKind::MandateChanged,
        "narrower reviewed mandate",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let mut tx = db.job_transaction().await.unwrap();
    let mut restored = load(&mut tx, worker.detail.worker.id).await.unwrap();
    let retained = budget(&mut tx, &restored, MINUTE).await.unwrap();
    assert_eq!(retained.usage, usage);
    assert!(retained.eligible_at.is_some());
    let revisions: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM autonomous_worker_mandates WHERE worker_id=?")
            .bind(worker.detail.worker.id.to_string())
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(revisions, 2);
    let (next, next_reservation) = job(&restored, None);
    assert!(
        admit(&mut tx, &mut restored, &next, next_reservation, MINUTE)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn stale_due_observation_cannot_cross_a_persisted_pause_fence() {
    let (db, mut worker) = fixture().await;
    let mut observed = worker.clone();
    let (execution, reservation) = job(&observed, None);
    let mut tx = db.job_transaction().await.unwrap();
    worker.detail.worker.desired_state = WorkerDesiredState::Paused;
    worker.schedule_generation += 1;
    append(
        &mut tx,
        &mut worker,
        WorkerEventKind::StateChanged,
        "pause wins",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let mut tx = db.job_transaction().await.unwrap();
    assert!(
        admit(&mut tx, &mut observed, &execution, reservation, START)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    assert!(db
        .worker_runs(worker.detail.worker.id, None)
        .await
        .unwrap()
        .runs
        .is_empty());
}

#[tokio::test]
async fn result_document_is_immutable_while_application_acknowledgment_can_advance() {
    let (db, mut worker) = fixture().await;
    let (execution, reservation) = job(&worker, None);
    let mut tx = db.job_transaction().await.unwrap();
    admit(&mut tx, &mut worker, &execution, reservation, START)
        .await
        .unwrap();
    let result = WorkerResult {
        version: WORKER_RESULT_VERSION,
        worker_id: worker.detail.worker.id,
        mandate_revision: 1,
        backlog_revision: 0,
        schedule_generation: 1,
        reservation_id: reservation,
        purpose: WorkerRunPurpose::Inspection,
        disposition: WorkerResultDisposition::NoWork,
        summary: "Nothing actionable".into(),
        checklist: vec![],
        upserts: vec![],
        selected_item_id: None,
    };
    let document = serde_json::to_string(&result).unwrap();
    sqlx::query(
        "INSERT INTO autonomous_worker_results(run_id,worker_id,document,applied) VALUES(?,?,?,0)",
    )
    .bind(execution.detail.job.current_run_id.to_string())
    .bind(worker.detail.worker.id.to_string())
    .bind(&document)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("UPDATE autonomous_worker_results SET applied=1 WHERE run_id=?")
        .bind(execution.detail.job.current_run_id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let mut conflicting = result;
    conflicting.summary = "Different result".into();
    assert!(
        sqlx::query("UPDATE autonomous_worker_results SET document=? WHERE run_id=?")
            .bind(serde_json::to_string(&conflicting).unwrap())
            .bind(execution.detail.job.current_run_id.to_string())
            .execute(&mut *tx)
            .await
            .is_err()
    );
    let retained: String =
        sqlx::query_scalar("SELECT document FROM autonomous_worker_results WHERE run_id=?")
            .bind(execution.detail.job.current_run_id.to_string())
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(retained, document);
}

#[tokio::test]
async fn complete_inventory_and_historical_purge_barrier_survive_idle_workers() {
    let (db, mut worker) = fixture().await;
    let (mut execution, reservation) = job(&worker, None);
    let mut tx = db.job_transaction().await.unwrap();
    admit(&mut tx, &mut worker, &execution, reservation, START)
        .await
        .unwrap();
    let usage = JobUsage {
        elapsed_seconds: 10,
        spend_micro_usd: Some(100),
        ..JobUsage::default()
    };
    finish(
        &mut tx,
        &mut worker,
        &mut execution,
        reservation,
        &usage,
        MINUTE,
    )
    .await;
    for _ in 0..55 {
        let mut another = worker.clone();
        another.detail.worker.id = Uuid::new_v4();
        save(&mut tx, &another).await.unwrap();
    }
    tx.commit().await.unwrap();
    let inventory = db.all_workers().await.unwrap();
    assert_eq!(inventory.len(), 56);
    sqlx::query("UPDATE projects SET lifecycle_status='deleting' WHERE id=?")
        .bind(&worker.detail.worker.project_id)
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(db
        .delete_project_records(&worker.detail.worker.project_id)
        .await
        .unwrap_err()
        .to_string()
        .contains("worker stop"));

    let mut tx = db.job_transaction().await.unwrap();
    for mut worker in inventory {
        worker.grant_active = false;
        worker.detail.worker.desired_state = WorkerDesiredState::Paused;
        worker.detail.worker.state = WorkerState::Paused;
        worker.schedule_generation += 1;
        append(
            &mut tx,
            &mut worker,
            WorkerEventKind::StateChanged,
            "project deletion",
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    assert!(db
        .delete_project_records(&worker.detail.worker.project_id)
        .await
        .unwrap_err()
        .to_string()
        .contains("purge pending"));
    let mut tx = db.job_transaction().await.unwrap();
    let command = Uuid::new_v4();
    jobs::command(
        &mut tx,
        &execution,
        command,
        JobWorkerCommand::PurgeRun {
            command_id: command,
            identity: execution.identity(),
        },
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO autonomous_job_purges(run_id,command_id,confirmed) VALUES(?,?,1)")
        .bind(execution.detail.job.current_run_id.to_string())
        .bind(command.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(db
        .delete_project_records(&worker.detail.worker.project_id)
        .await
        .unwrap());
    assert!(db.all_workers().await.unwrap().is_empty());
}

#[tokio::test]
async fn worker_only_failure_notifications_are_durable_and_not_routine_spam() {
    let (db, mut worker) = fixture().await;
    let mut tx = db.job_transaction().await.unwrap();
    append(
        &mut tx,
        &mut worker,
        WorkerEventKind::Checkpoint,
        "routine idle inspection",
    )
    .await
    .unwrap();
    worker.detail.worker.state = WorkerState::Blocked;
    worker.detail.worker.reason = Some("grant revoked".into());
    let failure = append(
        &mut tx,
        &mut worker,
        WorkerEventKind::StateChanged,
        "grant revoked",
    )
    .await
    .unwrap();
    append(
        &mut tx,
        &mut worker,
        WorkerEventKind::StateChanged,
        "same failure replay",
    )
    .await
    .unwrap();
    worker.detail.worker.state = WorkerState::RecoveryRequired;
    append(
        &mut tx,
        &mut worker,
        WorkerEventKind::RunChanged,
        "job-backed failure",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let pending = db.pending_worker_notifications(100).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].seq, failure.seq);
    db.acknowledge_worker_notification(worker.detail.worker.id, failure.seq)
        .await
        .unwrap();
    assert!(db
        .pending_worker_notifications(100)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn worker_pages_filter_projects_and_status_before_keyset_limits() {
    let (db, worker) = fixture().await;
    let other_project = Uuid::new_v4().to_string();
    db.authorize_project_registration(
        &other_project,
        &worker.detail.mandate.approved_by.to_string(),
    )
    .await
    .unwrap();
    let mut tx = db.job_transaction().await.unwrap();
    for _ in 0..70 {
        let mut other = worker.clone();
        other.detail.worker.id = Uuid::new_v4();
        other.detail.worker.project_id = other_project.clone();
        other.detail.worker.created_at = MINUTE.into();
        save(&mut tx, &other).await.unwrap();
    }
    let mut expected = vec![worker.detail.worker.id];
    let mut attention = None;
    for index in 0..3 {
        let mut same_project = worker.clone();
        same_project.detail.worker.id = Uuid::new_v4();
        if index == 0 {
            same_project.detail.worker.state = WorkerState::Paused;
            same_project.detail.worker.attention = true;
            attention = Some(same_project.detail.worker.id);
        }
        expected.push(same_project.detail.worker.id);
        save(&mut tx, &same_project).await.unwrap();
    }
    tx.commit().await.unwrap();
    let global = db.worker_page(None, None, None, 50).await.unwrap();
    assert_eq!(global.len(), 50);
    assert!(global
        .iter()
        .all(|w| w.detail.worker.project_id == other_project));

    let project = &worker.detail.worker.project_id;
    let first = db.worker_page(Some(project), None, None, 2).await.unwrap();
    assert_eq!(first.len(), 2);
    let last = &first.last().unwrap().detail.worker;
    let cursor = format!("{}|{}", last.created_at, last.id);
    let second = db
        .worker_page(Some(project), Some(&cursor), None, 2)
        .await
        .unwrap();
    let actual: Vec<Uuid> = first
        .iter()
        .chain(&second)
        .map(|w| w.detail.worker.id)
        .collect();
    expected.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(actual, expected);
    let paused = db
        .worker_page(Some(project), None, Some("paused"), 1)
        .await
        .unwrap();
    assert_eq!(paused[0].detail.worker.id, attention.unwrap());
    let attentive = db
        .worker_page(Some(project), None, Some("attention"), 1)
        .await
        .unwrap();
    assert_eq!(attentive[0].detail.worker.id, attention.unwrap());
    assert!(db
        .worker_page(None, Some("invalid"), None, 1)
        .await
        .is_err());
    assert!(db
        .worker_page(None, None, Some("invalid"), 1)
        .await
        .is_err());
}
