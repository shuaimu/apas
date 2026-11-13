use super::*;
use crate::routes::workers::tests::Fixture;

fn item(worker: Uuid) -> WorkerItem {
    WorkerItem {
        id: Uuid::new_v4(),
        worker_id: worker,
        revision: 1,
        item: WorkerItemDraft {
            title: "Repair maintained test".into(),
            instruction: "Repair the existing deterministic test".into(),
            acceptance_criteria: vec!["The independent check passes".into()],
            scope_rationale: "Maintains approved code".into(),
            evidence: vec![],
            priority: 1,
            dependencies: vec![],
        },
        origin: WorkerItemOrigin::Human,
        state: WorkerItemState::Running,
        attempts: 1,
        job_id: None,
        run_id: None,
        outcome: None,
        created_at: now(),
        updated_at: now(),
    }
}

// Construct admitted records only; these unit tests never start an execution host.
async fn admitted(f: &mut Fixture, purpose: WorkerRunPurpose) -> StoredJob {
    let job_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let reservation_id = Uuid::new_v4();
    let mut selected = (purpose == WorkerRunPurpose::Implementation).then(|| item(f.id()));
    if let Some(item) = selected.as_mut() {
        item.job_id = Some(job_id);
        item.run_id = Some(run_id);
    }
    let context = WorkerExecutionContext {
        worker_id: f.id(),
        mandate_revision: 1,
        backlog_revision: f.worker.detail.worker.backlog_revision,
        schedule_generation: f.worker.schedule_generation,
        reservation_id,
        purpose,
        mandate: f.worker.detail.mandate.spec.clone(),
        items: selected.clone().into_iter().collect(),
        selected_item_id: selected.as_ref().map(|i| i.id),
        checkpoint: None,
    };
    let usage = JobUsage {
        spend_micro_usd: Some(0),
        ..Default::default()
    };
    let time = now();
    let mut job: StoredJob = serde_json::from_value(serde_json::json!({
        "initiator": f.owner(), "dispatched": true, "source_seq": 0, "lease_expires_at": null, "last_renewed_at": null, "review": null,
        "detail": {
            "job": { "id": job_id, "project_id": f.project(), "project_name": "test", "title": "test worker",
                "state": "running", "state_version": 1, "current_run_id": run_id, "cluster_owner_user_id": f.owner(),
                "machine_id": context.mandate.machine_id, "hostname": "test-host", "created_at": time, "updated_at": time,
                "usage": usage, "limits": context.mandate.limits.per_run, "attention_count": 0, "reason": null, "allowed_actions": JobAllowedActions::default() },
            "task": { "instruction": "Inspect", "acceptance_criteria": ["Retain evidence"], "source_url": null,
                "repository": "owner/repo", "base_ref": "main", "base_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "worker_profile": "worker", "worker_profile_digest": "worker-digest", "verification_profile": "verify",
                "verification_profile_digest": "verify-digest", "model_profile": "model", "worker": context },
            "runs": [{ "id": run_id, "job_id": job_id, "attempt": 1, "epoch": 1, "state": "running", "usage": usage,
                "started_at": time, "ended_at": null, "worker_version": null, "candidate_sha": null, "stop_evidence": null, "reason": null }],
            "questions": [], "high_watermark": 0, "policy_digest": "test", "publication": null
        }
    })).unwrap();
    if let Some(item) = selected.as_ref() {
        job.detail.task.instruction = item.item.instruction.clone();
        job.detail.task.acceptance_criteria = item.item.acceptance_criteria.clone();
    }
    let mut tx = f.state.db.job_transaction().await.unwrap();
    if let Some(item) = selected.as_ref() {
        store::save_item(&mut tx, item).await.unwrap();
    }
    jobs_store::save(&mut tx, &job).await.unwrap();
    let limits = job.detail.job.limits.clone();
    store::reserve(
        &mut tx,
        &mut f.worker,
        reservation_id,
        job_id,
        run_id,
        &limits,
        &time,
    )
    .await
    .unwrap();
    f.worker.detail.worker.active_job_id = Some(job_id);
    f.worker.detail.worker.active_run_id = Some(run_id);
    f.worker.detail.worker.current_item_id = selected.map(|i| i.id);
    transition(
        &mut f.worker,
        if purpose == WorkerRunPurpose::Inspection {
            WorkerState::Inspecting
        } else {
            WorkerState::Working
        },
        None,
    );
    store::append(
        &mut tx,
        &mut f.worker,
        WorkerEventKind::RunChanged,
        "Test admission",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    job
}
fn result(job: &StoredJob, disposition: WorkerResultDisposition) -> WorkerResult {
    let c = job.detail.task.worker.as_ref().unwrap();
    WorkerResult {
        version: WORKER_RESULT_VERSION,
        worker_id: c.worker_id,
        mandate_revision: c.mandate_revision,
        backlog_revision: c.backlog_revision,
        schedule_generation: c.schedule_generation,
        reservation_id: c.reservation_id,
        purpose: c.purpose,
        disposition,
        summary: "Bounded observation".into(),
        checklist: vec![],
        upserts: vec![],
        selected_item_id: c.selected_item_id,
    }
}
fn stopped(job: &mut StoredJob) {
    // A typed trusted-host report fixture, not an operational containment claim.
    job.current().stop_evidence = Some(JobStopEvidence {
        descendants_stopped: true,
        boot_id: "test-boot".into(),
        container_id: "test-unit".into(),
        stopped_at: now(),
        reason: "test settled".into(),
    });
    job.current().usage = JobUsage {
        elapsed_seconds: 7,
        spend_micro_usd: Some(11),
        ..Default::default()
    };
    job.detail.job.usage = job.current().usage.clone();
}
async fn accept(f: &Fixture, job: &mut StoredJob, result: &WorkerResult) {
    let mut tx = f.state.db.job_transaction().await.unwrap();
    accept_result(&mut tx, job, result).await.unwrap();
    jobs_store::save(&mut tx, job).await.unwrap();
    reconcile_job(&mut tx, job).await.unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn no_work_is_idle_and_releases_execution_without_verifier() {
    let mut f = Fixture::new().await;
    let mut job = admitted(&mut f, WorkerRunPurpose::Inspection).await;
    stopped(&mut job);
    let output = result(&job, WorkerResultDisposition::NoWork);
    accept(&f, &mut job, &output).await;
    let worker = f.state.db.worker(f.id()).await.unwrap().unwrap();
    assert_eq!(job.detail.job.state, JobState::Completed);
    assert_eq!(worker.detail.worker.state, WorkerState::Idle);
    assert!(worker.detail.worker.active_job_id.is_none());
    assert!(worker.detail.worker.next_check_at.as_ref().unwrap() > &now());
    assert_eq!(
        worker.detail.checkpoint.as_ref().unwrap().run_id,
        job.detail.job.current_run_id
    );
    assert!(f.state.db.job_outbox().await.unwrap().is_empty());
    assert_eq!(
        f.state
            .db
            .worker_runs(f.id(), None)
            .await
            .unwrap()
            .runs
            .len(),
        1
    );
}

#[tokio::test]
async fn pause_generation_preserves_inspection_outcome_and_prevents_next_claim() {
    let mut f = Fixture::new().await;
    let mut job = admitted(&mut f, WorkerRunPurpose::Inspection).await;
    let receipt = mutate(
        &f.state,
        f.owner(),
        &f.project(),
        f.id(),
        WorkerMutationKind::Pause,
        WorkerMutationRequest {
            request_id: Uuid::new_v4(),
            expected_version: f.worker.detail.worker.state_version,
        },
    )
    .await
    .unwrap();
    assert_eq!(receipt.status, JobReceiptStatus::Accepted);
    stopped(&mut job);
    let output = result(&job, WorkerResultDisposition::NoWork);
    accept(&f, &mut job, &output).await;
    let worker = f.state.db.worker(f.id()).await.unwrap().unwrap();
    assert_eq!(worker.detail.worker.state, WorkerState::Paused);
    assert!(worker.detail.checkpoint.is_some());
    assert_eq!(worker.detail.budget.usage.elapsed_seconds, 7);
    assert!(worker.detail.worker.next_check_at.is_none());
    assert!(f
        .state
        .db
        .due_workers(&(Utc::now() + Duration::days(2)).to_rfc3339(), 32)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn pause_does_not_invalidate_candidate_ready_or_trusted_stop() {
    let mut f = Fixture::new().await;
    let mut job = admitted(&mut f, WorkerRunPurpose::Implementation).await;
    let mut tx = f.state.db.job_transaction().await.unwrap();
    f.worker.schedule_generation += 1;
    f.worker.detail.worker.desired_state = WorkerDesiredState::Paused;
    store::append(
        &mut tx,
        &mut f.worker,
        WorkerEventKind::StateChanged,
        "Pause",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    stopped(&mut job);
    let output = result(&job, WorkerResultDisposition::CandidateReady);
    accept(&f, &mut job, &output).await;
    assert_eq!(job.detail.job.state, JobState::Running);
    let mut tx = f.state.db.job_transaction().await.unwrap();
    assert!(candidate_ready(&mut tx, &job).await.unwrap());
    tx.commit().await.unwrap();
    assert!(
        job.current()
            .stop_evidence
            .as_ref()
            .unwrap()
            .descendants_stopped
    );
    assert_eq!(
        f.state
            .db
            .worker(f.id())
            .await
            .unwrap()
            .unwrap()
            .detail
            .worker
            .state,
        WorkerState::Pausing
    );
}

#[tokio::test]
async fn stale_human_backlog_rejects_all_proposals_but_retains_checkpoint_and_usage() {
    let mut f = Fixture::new().await;
    let mut job = admitted(&mut f, WorkerRunPurpose::Inspection).await;
    let mut tx = f.state.db.job_transaction().await.unwrap();
    f.worker.detail.worker.backlog_revision += 1;
    store::append(
        &mut tx,
        &mut f.worker,
        WorkerEventKind::BacklogChanged,
        "Human edited",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let proposed = item(f.id());
    let mut output = result(&job, WorkerResultDisposition::SelectWork);
    output.selected_item_id = Some(proposed.id);
    output.upserts.push(WorkerItemProposal {
        id: proposed.id,
        expected_revision: None,
        item: proposed.item,
    });
    stopped(&mut job);
    accept(&f, &mut job, &output).await;
    let worker = f.state.db.worker(f.id()).await.unwrap().unwrap();
    assert_eq!(worker.detail.worker.backlog_revision, 1);
    assert!(worker.detail.worker.current_item_id.is_none());
    assert_eq!(worker.detail.budget.usage.spend_micro_usd, Some(11));
    assert!(worker.detail.checkpoint.is_some());
    assert!(f
        .state
        .db
        .worker_items(f.id(), None, None)
        .await
        .unwrap()
        .items
        .is_empty());
}

#[tokio::test]
async fn accepted_result_replay_is_idempotent_and_conflicting_document_is_rejected() {
    let mut f = Fixture::new().await;
    let mut job = admitted(&mut f, WorkerRunPurpose::Inspection).await;
    stopped(&mut job);
    let output = result(&job, WorkerResultDisposition::NoWork);
    accept(&f, &mut job, &output).await;
    let before = f.state.db.worker(f.id()).await.unwrap().unwrap();
    accept(&f, &mut job, &output).await;
    let after = f.state.db.worker(f.id()).await.unwrap().unwrap();
    assert_eq!(
        before.detail.worker.high_watermark,
        after.detail.worker.high_watermark
    );
    assert_eq!(
        before.detail.budget.usage.spend_micro_usd,
        after.detail.budget.usage.spend_micro_usd
    );
    let mut conflicting = output;
    conflicting.summary = "Different result".into();
    let mut tx = f.state.db.job_transaction().await.unwrap();
    assert!(accept_result(&mut tx, &mut job, &conflicting)
        .await
        .unwrap_err()
        .to_string()
        .contains("worker_result_conflict"));
}

#[tokio::test]
async fn no_change_implementation_cannot_complete_item_or_enter_verification() {
    let mut f = Fixture::new().await;
    let mut job = admitted(&mut f, WorkerRunPurpose::Implementation).await;
    stopped(&mut job);
    let output = result(&job, WorkerResultDisposition::NoChange);
    accept(&f, &mut job, &output).await;
    assert_eq!(job.detail.job.state, JobState::Failed);
    let worker = f.state.db.worker(f.id()).await.unwrap().unwrap();
    assert_eq!(worker.detail.worker.state, WorkerState::Blocked);
    let items = f
        .state
        .db
        .worker_items(f.id(), None, None)
        .await
        .unwrap()
        .items;
    assert_eq!(items[0].state, WorkerItemState::Blocked);
    assert_eq!(items[0].attempts, 1);
    assert!(f.state.db.job_outbox().await.unwrap().is_empty());
}

#[tokio::test]
async fn invalid_result_identity_retains_trusted_stop_and_blocks_instead_of_restarting() {
    let mut f = Fixture::new().await;
    let mut job = admitted(&mut f, WorkerRunPurpose::Inspection).await;
    stopped(&mut job);
    let mut output = result(&job, WorkerResultDisposition::NoWork);
    output.reservation_id = Uuid::new_v4();
    accept(&f, &mut job, &output).await;
    assert_eq!(job.detail.job.state, JobState::Failed);
    assert!(job.current().stop_evidence.is_some());
    let worker = f.state.db.worker(f.id()).await.unwrap().unwrap();
    assert_eq!(worker.detail.worker.state, WorkerState::Blocked);
    assert_eq!(worker.detail.budget.usage.spend_micro_usd, Some(11));
    assert!(worker.detail.checkpoint.is_none());
}

#[tokio::test]
async fn idle_authority_revocation_remains_durable_after_policy_restoration() {
    let mut f = Fixture::new().await;
    // The fixture intentionally has default-disabled policy and no device session.
    reconcile_authority(&f.state).await.unwrap();
    f.worker = f.state.db.worker(f.id()).await.unwrap().unwrap();
    assert!(!f.worker.grant_active);
    assert_eq!(f.worker.detail.worker.state, WorkerState::Blocked);
    assert_eq!(
        f.worker.detail.worker.reason.as_deref(),
        Some("worker_reapproval_required")
    );
    let mut policy = JobPolicy::default();
    policy.enabled = true;
    policy.worker_profiles = vec!["worker".into()];
    policy.model_profiles = vec!["model".into()];
    policy.verification_profiles = vec!["verify".into()];
    policy.recurring = RecurringWorkerPolicy {
        enabled: true,
        max_elapsed_seconds_24h: 86_400,
        max_spend_micro_usd_24h: 100_000_000,
        max_runs_24h: 24,
        min_interval_seconds: 900,
    };
    f.state
        .db
        .set_job_policy(
            crate::db::SYSTEM_ADMIN_ACTOR,
            "deployment",
            "deployment",
            serde_json::to_value(policy).unwrap(),
        )
        .await
        .unwrap();
    reconcile_authority(&f.state).await.unwrap();
    assert!(
        !f.state
            .db
            .worker(f.id())
            .await
            .unwrap()
            .unwrap()
            .grant_active
    );
    assert!(f
        .state
        .db
        .due_workers(&(Utc::now() + Duration::days(2)).to_rfc3339(), 32)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn no_work_cannot_hide_a_new_eligible_item_or_partially_apply_backlog() {
    let mut f = Fixture::new().await;
    let mut job = admitted(&mut f, WorkerRunPurpose::Inspection).await;
    let proposed = item(f.id());
    let mut output = result(&job, WorkerResultDisposition::NoWork);
    output.upserts.push(WorkerItemProposal {
        id: proposed.id,
        expected_revision: None,
        item: proposed.item,
    });
    stopped(&mut job);
    accept(&f, &mut job, &output).await;
    assert_eq!(job.detail.job.state, JobState::Failed);
    assert_eq!(
        job.detail.job.reason.as_deref(),
        Some("no_work_has_eligible_items")
    );
    assert!(f
        .state
        .db
        .worker_items(f.id(), None, None)
        .await
        .unwrap()
        .items
        .is_empty());
    assert_eq!(
        f.state
            .db
            .worker(f.id())
            .await
            .unwrap()
            .unwrap()
            .detail
            .budget
            .usage
            .spend_micro_usd,
        Some(11)
    );
}

async fn observe_review(f: &Fixture, job: &mut StoredJob, disposition: &str, head: &str) {
    job.current().candidate_sha = Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into());
    job.detail.publication = Some(JobPublication {
        repository: job.detail.task.repository.clone(),
        number: 42,
        url: "https://example.test/pull/42".into(),
        branch: format!("apas/job-{}", job.detail.job.id),
        base_sha: job.detail.task.base_sha.clone(),
        head_sha: head.into(),
        draft: true,
        state: disposition.into(),
        observed_at: now(),
    });
    job.transition(JobState::ReviewReady, None);
    let mut tx = f.state.db.job_transaction().await.unwrap();
    jobs_store::save(&mut tx, job).await.unwrap();
    reconcile_job(&mut tx, job).await.unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn review_wait_holds_responsibility_without_execution_then_merge_respects_pause() {
    let mut f = Fixture::new().await;
    let mut job = admitted(&mut f, WorkerRunPurpose::Implementation).await;
    stopped(&mut job);
    observe_review(
        &f,
        &mut job,
        "open",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .await;
    let mut worker = f.state.db.worker(f.id()).await.unwrap().unwrap();
    assert_eq!(worker.detail.worker.state, WorkerState::WaitingReview);
    assert!(worker.detail.worker.active_job_id.is_some());
    assert_eq!(worker.detail.budget.reserved_runs, 0);
    assert!(f
        .state
        .db
        .due_workers(&(Utc::now() + Duration::days(2)).to_rfc3339(), 32)
        .await
        .unwrap()
        .is_empty());
    let mut tx = f.state.db.job_transaction().await.unwrap();
    worker.detail.worker.desired_state = WorkerDesiredState::Paused;
    worker.schedule_generation += 1;
    store::append(
        &mut tx,
        &mut worker,
        WorkerEventKind::StateChanged,
        "Paused review wait",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    observe_review(
        &f,
        &mut job,
        "merged",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .await;
    let worker = f.state.db.worker(f.id()).await.unwrap().unwrap();
    assert_eq!(worker.detail.worker.state, WorkerState::Paused);
    assert!(worker.detail.worker.active_job_id.is_none());
    assert!(worker.detail.worker.next_check_at.is_none());
    assert_eq!(
        f.state
            .db
            .worker_items(f.id(), None, None)
            .await
            .unwrap()
            .items[0]
            .state,
        WorkerItemState::Done
    );
}

#[tokio::test]
async fn changed_external_head_never_authorizes_merge_continuation() {
    let mut f = Fixture::new().await;
    let mut job = admitted(&mut f, WorkerRunPurpose::Implementation).await;
    stopped(&mut job);
    observe_review(
        &f,
        &mut job,
        "merged",
        "cccccccccccccccccccccccccccccccccccccccc",
    )
    .await;
    let worker = f.state.db.worker(f.id()).await.unwrap().unwrap();
    assert_eq!(worker.detail.worker.state, WorkerState::Blocked);
    assert_eq!(
        worker.detail.worker.reason.as_deref(),
        Some("review_disposition_required")
    );
    assert!(worker.detail.worker.active_job_id.is_some());
    assert!(worker.detail.worker.next_check_at.is_none());
    assert_eq!(
        f.state
            .db
            .worker_items(f.id(), None, None)
            .await
            .unwrap()
            .items[0]
            .state,
        WorkerItemState::Blocked
    );
}

#[tokio::test]
async fn deletion_fence_is_idempotent_and_retains_all_historical_run_links() {
    let mut f = Fixture::new().await;
    let mut job = admitted(&mut f, WorkerRunPurpose::Inspection).await;
    stopped(&mut job);
    let output = result(&job, WorkerResultDisposition::NoWork);
    accept(&f, &mut job, &output).await;
    let mut tx = f.state.db.job_transaction().await.unwrap();
    fence_project(&mut tx, &f.project(), "project_deleting")
        .await
        .unwrap();
    let once = store::load(&mut tx, f.id()).await.unwrap();
    fence_project(&mut tx, &f.project(), "project_deleting")
        .await
        .unwrap();
    let twice = store::load(&mut tx, f.id()).await.unwrap();
    assert_eq!(
        once.detail.worker.state_version,
        twice.detail.worker.state_version
    );
    assert!(!twice.grant_active);
    assert!(twice.detail.worker.active_job_id.is_none());
    tx.commit().await.unwrap();
    assert_eq!(
        f.state
            .db
            .worker_run(f.id(), job.detail.job.current_run_id)
            .await
            .unwrap(),
        Some(job.detail.job.id)
    );
}

#[tokio::test]
async fn attention_keeps_exact_question_identity_during_graceful_pause() {
    let mut f = Fixture::new().await;
    let mut job = admitted(&mut f, WorkerRunPurpose::Inspection).await;
    let question = Uuid::new_v4();
    job.detail.questions.push(JobQuestion {
        id: question,
        run_id: job.identity().run_id,
        epoch: job.identity().epoch,
        version: 1,
        dialog_id: "pending-dialog".into(),
        tool_call_id: "pending-call".into(),
        state: JobQuestionState::Pending,
        questions: vec![],
        answers: None,
        created_at: now(),
        confirmed_at: None,
    });
    let mut confirmed = job.detail.questions[0].clone();
    confirmed.id = Uuid::new_v4();
    confirmed.dialog_id = "confirmed-dialog".into();
    confirmed.state = JobQuestionState::Confirmed;
    job.detail.questions.push(confirmed);
    job.transition(JobState::NeedsInput, None);
    let mut tx = f.state.db.job_transaction().await.unwrap();
    f.worker.detail.worker.desired_state = WorkerDesiredState::Paused;
    f.worker.schedule_generation += 1;
    store::append(
        &mut tx,
        &mut f.worker,
        WorkerEventKind::StateChanged,
        "Pause after current run",
    )
    .await
    .unwrap();
    jobs_store::save(&mut tx, &job).await.unwrap();
    reconcile_job(&mut tx, &job).await.unwrap();
    tx.commit().await.unwrap();
    let detail = f.state.db.worker(f.id()).await.unwrap().unwrap();
    assert_eq!(detail.detail.worker.state, WorkerState::Pausing);
    assert!(detail.detail.worker.attention);
    assert_eq!(
        detail.detail.worker.attention_run_id,
        Some(job.identity().run_id)
    );
    assert_eq!(detail.detail.worker.attention_question_id, Some(question));
    let page = list(&f.state, f.owner(), None, None, Some("attention"))
        .await
        .unwrap();
    assert_eq!(page.workers[0].attention_question_id, Some(question));
}
