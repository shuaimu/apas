use super::*;
use crate::db::{Database, User};
use base64::Engine;

async fn fixture() -> (
    AppState,
    StoredJob,
    tokio::sync::mpsc::Receiver<ServerToDaemon>,
) {
    let root = std::env::temp_dir().join(format!("apas-jobs-{}", Uuid::new_v4()));
    let db = Database::new(&root.join("db.sqlite").to_string_lossy())
        .await
        .unwrap();
    db.run_migrations().await.unwrap();
    let mut config = crate::config::Config::default();
    config.database.path = root.join("db.sqlite").to_string_lossy().into_owned();
    let state = AppState::new(db, config);
    let owner = Uuid::new_v4();
    state
        .db
        .create_user(&User {
            id: owner.to_string(),
            email: format!("{owner}@test"),
            password_hash: "unused".into(),
            created_at: None,
            cluster_role: "user".into(),
            account_status: "active".into(),
        })
        .await
        .unwrap();
    let project = Uuid::new_v4().to_string();
    state
        .db
        .authorize_project_registration(&project, &owner.to_string())
        .await
        .unwrap();
    state
        .db
        .add_project_cluster_placement(&project, &owner.to_string(), &owner.to_string(), "test")
        .await
        .unwrap();
    let policy = JobPolicy {
        enabled: true,
        publish_enabled: true,
        worker_profiles: vec!["worker".into()],
        model_profiles: vec!["model".into()],
        verification_profiles: vec!["verify".into()],
        ..Default::default()
    };
    state
        .db
        .set_job_policy(
            crate::db::SYSTEM_ADMIN_ACTOR,
            "deployment",
            "deployment",
            serde_json::to_value(policy).unwrap(),
        )
        .await
        .unwrap();
    let machine = Uuid::new_v4();
    let (tx, rx) = tokio::sync::mpsc::channel(128);
    state.sessions.register_daemon(
        machine,
        owner,
        tx,
        MachineInfo {
            machine_id: machine,
            hostname: "test-host".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            daemon_version: Some("test".into()),
            deepseek_backend: None,
            last_seen: None,
        },
        vec![MachineProjectInfo {
            project_id: project.clone(),
            name: None,
            path: "/project".into(),
            preferred_machine_id: None,
            is_running: false,
            pid: None,
            memory_kb: None,
            last_error: None,
        }],
    );
    state
        .sessions
        .set_daemon_capabilities(machine, vec![OMP_JOB_WORKER_CAPABILITY.into()]);
    state.jobs.readiness.insert(
        machine,
        JobWorkerReadiness {
            machine_id: machine,
            cluster_owner_user_id: owner,
            protocol_version: JOB_PROTOCOL_VERSION,
            checked_at: Utc::now().to_rfc3339(),
            expires_at: (Utc::now() + Duration::minutes(5)).to_rfc3339(),
            profiles: vec![JobWorkerProfile {
                key: "worker".into(),
                label: "worker".into(),
                digest: "worker-digest".into(),
                image: "worker@sha256:test".into(),
                omp_version: "test".into(),
                model_profiles: vec![JobModelProfile {
                    key: "model".into(),
                    label: "model".into(),
                    model: "model".into(),
                    api: JobModelApi::OpenaiResponses,
                }],
                verification_profiles: vec![JobVerificationProfile {
                    key: "verify".into(),
                    label: "verify".into(),
                    digest: "verify-digest".into(),
                    image: "verify@sha256:test".into(),
                }],
                limits: JobLimits::default(),
                ready: true,
                unavailable_reasons: vec![],
            }],
        },
    );
    let id = Uuid::new_v4();
    let run = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();
    let instruction = inventory_instruction(id);
    let title: String = instruction.chars().take(100).collect();
    let job:StoredJob=serde_json::from_value(serde_json::json!({"initiator":owner,"review":null,"dispatched":false,"source_seq":0,"lease_expires_at":null,"last_renewed_at":null,"detail":{
        "job":{"id":id,"project_id":project,"project_name":"test","title":title,"state":"queued","state_version":1,"current_run_id":run,"cluster_owner_user_id":owner,"machine_id":machine,"hostname":"test-host","created_at":now,"updated_at":now,"usage":JobUsage::default(),"limits":JobLimits::default(),"attention_count":0,"reason":null,"allowed_actions":JobAllowedActions::default()},
        "task":{"instruction":instruction,"acceptance_criteria":["test"],"source_url":null,"repository":"owner/repo","base_ref":"main","base_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","worker_profile":"worker","worker_profile_digest":"worker-digest","verification_profile":"verify","verification_profile_digest":"verify-digest","model_profile":"model"},
        "runs":[{"id":run,"job_id":id,"attempt":1,"epoch":1,"state":"queued","usage":JobUsage::default(),"started_at":null,"ended_at":null,"worker_version":null,"candidate_sha":null,"stop_evidence":null,"reason":null}],"questions":[],"high_watermark":0,"policy_digest":effective_job_policy(&state.db,&project).await.unwrap().digest,"publication":null}})).unwrap();
    let mut tx = state.db.job_transaction().await.unwrap();
    store::save(&mut tx, &job).await.unwrap();
    tx.commit().await.unwrap();
    (state, job, rx)
}

fn inventory_instruction(id: Uuid) -> String {
    format!("Agent {id}: {}\nVerify the complete approved task.", "检查完整说明。".repeat(30))
}

async fn interleaved_inventory() -> (AppState, StoredJob, Vec<JobSummary>) {
    let (state, seed, _) = fixture().await;
    let other_project = Uuid::new_v4().to_string();
    state
        .db
        .authorize_project_registration(&other_project, &seed.initiator.to_string())
        .await
        .unwrap();
    let mut expected = vec![seed.detail.job.clone()];
    let mut tx = state.db.job_transaction().await.unwrap();
    for index in 1..=120 {
        let mut job = seed.clone();
        job.detail.job.id = Uuid::from_u128(index);
        job.detail.task.instruction = inventory_instruction(job.detail.job.id);
        job.detail.job.title = job.detail.task.instruction.chars().take(100).collect();
        job.detail.job.current_run_id = Uuid::from_u128(index + 1_000);
        job.detail.runs[0].id = job.detail.job.current_run_id;
        job.detail.runs[0].job_id = job.detail.job.id;
        if index % 2 == 0 {
            job.detail.job.project_id = other_project.clone();
        }
        if index % 4 < 2 {
            job.detail.job.state = JobState::NeedsInput;
            job.detail.job.attention_count = 1;
        }
        store::save(&mut tx, &job).await.unwrap();
        expected.push(job.detail.job);
    }
    tx.commit().await.unwrap();
    // Equal creation timestamps force pagination to use the stable ID tie-breaker.
    expected.sort_by(|a, b| b.id.cmp(&a.id));
    (state, seed, expected)
}

#[tokio::test]
async fn project_job_pages_filter_interleaved_projects_before_the_page_boundary() {
    let (state, seed, expected) = interleaved_inventory().await;
    let project = &seed.detail.job.project_id;
    let expected: Vec<_> = expected
        .iter()
        .filter(|job| &job.project_id == project)
        .map(|job| job.id)
        .collect();
    let first = list_project_jobs(&state, seed.initiator, project, None, None)
        .await
        .unwrap();
    assert_eq!(first.jobs.len(), 50);
    assert_eq!(
        first.jobs.iter().map(|job| job.summary.id).collect::<Vec<_>>(),
        expected[..50]
    );
    assert!(first.jobs.iter().all(|job| job.summary.allowed_actions.cancel));
    let second = list_project_jobs(
        &state,
        seed.initiator,
        project,
        first.next_cursor.as_deref(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        second.jobs.iter().map(|job| job.summary.id).collect::<Vec<_>>(),
        expected[50..]
    );
    assert!(second.next_cursor.is_none());
    for job in first.jobs.iter().chain(&second.jobs) {
        assert_eq!(&job.summary.project_id, project);
        assert_eq!(job.description, inventory_instruction(job.summary.id));
        let expected_state = if job.summary.id == seed.detail.job.id {
            seed.detail.job.state
        } else if job.summary.id.as_u128() % 4 < 2 {
            JobState::NeedsInput
        } else {
            JobState::Queued
        };
        assert_eq!(job.summary.state, expected_state);
    }
    assert_eq!(state.db.all_jobs().await.unwrap().len(), 121);
}

#[tokio::test]
async fn project_job_status_filters_match_the_unchanged_global_listing() {
    let (state, seed, expected) = interleaved_inventory().await;
    for status in [None, Some("attention"), Some("needs_input"), Some("unknown")] {
        let matching: Vec<_> = expected
            .iter()
            .filter(|job| match status {
                None => true,
                Some("attention") => job.attention_count > 0,
                Some("needs_input") => job.state == JobState::NeedsInput,
                _ => false,
            })
            .collect();
        let global = list_jobs(&state, seed.initiator, None, status)
            .await
            .unwrap();
        assert_eq!(
            global.jobs.iter().map(|job| job.id).collect::<Vec<_>>(),
            matching.iter().take(50).map(|job| job.id).collect::<Vec<_>>()
        );
        assert_eq!(global.next_cursor.is_some(), matching.len() >= 50);
        if let Some(cursor) = global.next_cursor.as_deref() {
            let next = list_jobs(&state, seed.initiator, Some(cursor), status)
                .await
                .unwrap();
            assert_eq!(
                next.jobs.iter().map(|job| job.id).collect::<Vec<_>>(),
                matching.iter().skip(50).take(50).map(|job| job.id).collect::<Vec<_>>()
            );
        }
        let project = &seed.detail.job.project_id;
        let scoped = list_project_jobs(&state, seed.initiator, project, None, status)
            .await
            .unwrap();
        assert_eq!(
            scoped.jobs.iter().map(|job| job.summary.id).collect::<Vec<_>>(),
            matching
                .iter()
                .filter(|job| &job.project_id == project)
                .take(50)
                .map(|job| job.id)
                .collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn project_job_listing_requires_current_content_access_without_admission_authority() {
    let (state, seed, _) = fixture().await;
    let reader = Uuid::new_v4();
    state
        .db
        .create_user(&User {
            id: reader.to_string(),
            email: format!("{reader}@test"),
            password_hash: "unused".into(),
            created_at: None,
            cluster_role: "user".into(),
            account_status: "active".into(),
        })
        .await
        .unwrap();
    let project = &seed.detail.job.project_id;
    for denied in [project.as_str(), "unknown-project"] {
        let error = list_project_jobs(&state, reader, denied, None, None)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "job_access_denied");
    }
    state
        .db
        .add_project_member(&seed.initiator.to_string(), project, &reader.to_string())
        .await
        .unwrap();
    let page = list_project_jobs(&state, reader, project, None, None)
        .await
        .unwrap();
    assert_eq!(page.jobs.len(), 1);
    assert_eq!(page.jobs[0].summary.id, seed.detail.job.id);
    assert_eq!(page.jobs[0].description, seed.detail.task.instruction);
    let allowed = &page.jobs[0].summary.allowed_actions;
    assert!(!allowed.admit && !allowed.answer && !allowed.cancel && !allowed.retry);
    state
        .db
        .remove_project_member(&seed.initiator.to_string(), project, &reader.to_string())
        .await
        .unwrap();
    assert_eq!(
        list_project_jobs(&state, reader, project, None, None)
            .await
            .unwrap_err()
            .to_string(),
        "job_access_denied"
    );
    assert!(list_jobs(&state, reader, None, None)
        .await
        .unwrap()
        .jobs
        .is_empty());
}

#[tokio::test]
async fn actor_receipts_survive_restart_and_conflicting_payload_cannot_replace_result() {
    let (state, job, _) = fixture().await;
    let actor = job.initiator;
    let request = Uuid::new_v4();
    let result = JobMutationReceipt {
        request_id: request,
        operation: JobMutationKind::Cancel,
        job_id: Some(job.detail.job.id),
        run_id: Some(job.detail.job.current_run_id),
        status: JobReceiptStatus::Accepted,
        error_code: None,
        message: None,
    };
    let mut tx = state.db.job_transaction().await.unwrap();
    store::save_receipt(
        &mut tx,
        actor,
        &job.detail.job.project_id,
        "original",
        &result,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let db = Database::new(&state.config.database.path).await.unwrap();
    db.run_migrations().await.unwrap();
    let mut tx = db.job_transaction().await.unwrap();
    assert_eq!(
        store::receipt(&mut tx, actor, request, "original")
            .await
            .unwrap()
            .unwrap()
            .job_id,
        result.job_id
    );
    assert!(store::receipt(&mut tx, actor, request, "different")
        .await
        .is_err());
    assert!(store::receipt(&mut tx, Uuid::new_v4(), request, "original")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn concurrent_dispatch_reserves_one_slot_and_preserves_fifo() {
    let (state, first, _) = fixture().await;
    let mut second = first.clone();
    second.detail.job.id = Uuid::new_v4();
    second.detail.job.current_run_id = Uuid::new_v4();
    second.detail.job.created_at = (Utc::now() + Duration::seconds(1)).to_rfc3339();
    second.detail.runs[0].id = second.detail.job.current_run_id;
    second.detail.runs[0].job_id = second.detail.job.id;
    let mut tx = state.db.job_transaction().await.unwrap();
    store::save(&mut tx, &second).await.unwrap();
    tx.commit().await.unwrap();
    let (a, b) = tokio::join!(dispatch_tick(&state), dispatch_tick(&state));
    a.unwrap();
    b.unwrap();
    let rows = state.db.all_jobs().await.unwrap();
    assert_eq!(rows.iter().filter(|j| j.dispatched).count(), 1);
    assert!(
        state
            .db
            .job(first.detail.job.id)
            .await
            .unwrap()
            .unwrap()
            .dispatched
    );
    assert_eq!(
        state
            .db
            .job(second.detail.job.id)
            .await
            .unwrap()
            .unwrap()
            .detail
            .job
            .reason
            .as_deref(),
        Some("capacity_wait")
    );
}

#[tokio::test]
async fn immutable_task_and_one_unresolved_attempt_are_database_invariants() {
    let (state, mut job, _) = fixture().await;
    let mut tx = state.db.job_transaction().await.unwrap();
    job.detail.task.instruction = "changed".into();
    assert!(store::save(&mut tx, &job).await.is_err());
    tx.rollback().await.unwrap();
    let mut job = state.db.job(job.detail.job.id).await.unwrap().unwrap();
    let mut run = job.detail.runs[0].clone();
    run.id = Uuid::new_v4();
    run.attempt = 2;
    job.detail.runs.push(run);
    let mut tx = state.db.job_transaction().await.unwrap();
    assert!(store::save(&mut tx, &job).await.is_err());
    tx.rollback().await.unwrap();
    assert_eq!(
        state
            .db
            .job(job.detail.job.id)
            .await
            .unwrap()
            .unwrap()
            .detail
            .runs
            .len(),
        1
    );
}

#[tokio::test]
async fn replay_gap_and_snapshot_high_watermark_survive_event_pruning() {
    let (state, mut job, _) = fixture().await;
    let mut tx = state.db.job_transaction().await.unwrap();
    for _ in 0..3 {
        store::append(
            &mut tx,
            &mut job,
            JobEventKind::Progress,
            "progress",
            None,
            None,
        )
        .await
        .unwrap();
    }
    sqlx::query("DELETE FROM autonomous_job_events WHERE job_id=? AND seq<3")
        .bind(job.detail.job.id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let replay = state.db.job_events(job.detail.job.id, 0).await.unwrap();
    assert!(replay.resync_required);
    assert_eq!(replay.high_watermark, 3);
    let replay = state.db.job_events(job.detail.job.id, 2).await.unwrap();
    assert!(!replay.resync_required);
    assert_eq!(replay.events[0].seq, 3);
    assert_eq!(
        state
            .db
            .job(job.detail.job.id)
            .await
            .unwrap()
            .unwrap()
            .detail
            .high_watermark,
        3
    );
}

#[tokio::test]
async fn stale_epoch_and_source_duplicates_cannot_advance_state() {
    let (state, job, _) = fixture().await;
    let mut wrong = job.identity();
    wrong.epoch += 1;
    assert!(report(
        &state,
        wrong.machine_id,
        JobWorkerReport::Event {
            identity: wrong,
            source_seq: 1,
            event_kind: JobEventKind::Progress,
            summary: "wrong".into(),
            detail: None,
            usage: None
        }
    )
    .await
    .is_err());
    let message = JobWorkerReport::Event {
        identity: job.identity(),
        source_seq: 1,
        event_kind: JobEventKind::Progress,
        summary: "once".into(),
        detail: None,
        usage: None,
    };
    report(&state, job.identity().machine_id, message.clone())
        .await
        .unwrap();
    report(&state, job.identity().machine_id, message)
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .job_events(job.detail.job.id, 0)
            .await
            .unwrap()
            .events
            .len(),
        1
    );
}

#[tokio::test]
async fn restart_and_offline_cancel_never_establish_positive_stop() {
    let (state, job, _) = fixture().await;
    dispatch_tick(&state).await.unwrap();
    recover(&state).await.unwrap();
    let current = state.db.job(job.detail.job.id).await.unwrap().unwrap();
    assert_eq!(current.detail.job.state, JobState::RecoveryRequired);
    state.sessions.unregister_daemon(&job.detail.job.machine_id);
    cancel_project_jobs(&state, &job.detail.job.project_id, "test")
        .await
        .unwrap();
    assert!(!project_jobs_stopped(&state, &job.detail.job.project_id)
        .await
        .unwrap());
    assert_eq!(
        state
            .db
            .job(job.detail.job.id)
            .await
            .unwrap()
            .unwrap()
            .detail
            .job
            .state,
        JobState::Cancelling
    );
    assert!(state
        .db
        .job_project_fenced(&job.detail.job.project_id)
        .await
        .unwrap());
}

#[tokio::test]
async fn artifact_chunks_require_contiguous_exact_bytes_and_digest() {
    let (state, job, _) = fixture().await;
    let bytes = b"retained diff";
    let artifact = JobArtifact {
        id: Uuid::new_v4(),
        run_id: job.identity().run_id,
        kind: JobArtifactKind::Diff,
        sha256: format!("{:x}", Sha256::digest(bytes)),
        size_bytes: bytes.len() as u64,
        content_type: "text/plain".into(),
        expires_at: None,
        truncated: false,
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let mut tx = state.db.job_transaction().await.unwrap();
    assert!(
        persist_artifact(&mut tx, &job, artifact.clone(), 1, &encoded, true)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    let mut tx = state.db.job_transaction().await.unwrap();
    persist_artifact(&mut tx, &job, artifact.clone(), 0, &encoded, true)
        .await
        .unwrap();
    persist_artifact(&mut tx, &job, artifact.clone(), 0, &encoded, true)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        state
            .db
            .job_artifact(job.detail.job.id, artifact.id, 0, 65536)
            .await
            .unwrap()
            .text,
        "retained diff"
    );
}

#[tokio::test]
async fn expired_readiness_never_substitutes_another_worker() {
    let (state, job, _) = fixture().await;
    state
        .jobs
        .readiness
        .get_mut(&job.detail.job.machine_id)
        .unwrap()
        .expires_at = (Utc::now() - Duration::seconds(1)).to_rfc3339();
    dispatch_tick(&state).await.unwrap();
    let current = state.db.job(job.detail.job.id).await.unwrap().unwrap();
    assert!(!current.dispatched);
    assert_eq!(current.detail.job.state, JobState::Blocked);
    assert!(state.db.job_outbox().await.unwrap().is_empty());
}

#[tokio::test]
async fn full_multi_question_answer_is_reserved_once_and_never_provider_confirmed_by_receipt() {
    let (state, mut job, _) = fixture().await;
    job.detail.job.state = JobState::NeedsInput;
    job.detail.runs[0].state = JobState::NeedsInput;
    let q = JobQuestion {
        id: Uuid::new_v4(),
        run_id: job.identity().run_id,
        epoch: 1,
        version: 1,
        dialog_id: "dialog".into(),
        tool_call_id: "call".into(),
        state: JobQuestionState::Pending,
        questions: vec![
            JobQuestionItem {
                id: "one".into(),
                question: "Pick".into(),
                header: None,
                options: vec![JobQuestionOption {
                    id: "a".into(),
                    label: "A".into(),
                    description: None,
                    preview: None,
                }],
                multi: false,
                recommended: None,
            },
            JobQuestionItem {
                id: "two".into(),
                question: "Explain".into(),
                header: None,
                options: vec![],
                multi: false,
                recommended: None,
            },
        ],
        answers: None,
        created_at: Utc::now().to_rfc3339(),
        confirmed_at: None,
    };
    job.detail.questions.push(q.clone());
    let mut tx = state.db.job_transaction().await.unwrap();
    store::save(&mut tx, &job).await.unwrap();
    tx.commit().await.unwrap();
    let request = JobAnswerRequest {
        request_id: Uuid::new_v4(),
        run_id: job.identity().run_id,
        epoch: 1,
        expected_version: 1,
        answers: vec![
            JobQuestionAnswer {
                id: "one".into(),
                selected_option_ids: vec!["a".into()],
                custom_input: None,
            },
            JobQuestionAnswer {
                id: "two".into(),
                selected_option_ids: vec![],
                custom_input: Some("yes".into()),
            },
        ],
    };
    let result = answer(
        &state,
        job.initiator,
        &job.detail.job.project_id,
        job.detail.job.id,
        q.id,
        request.clone(),
    )
    .await
    .unwrap();
    let again = answer(
        &state,
        job.initiator,
        &job.detail.job.project_id,
        job.detail.job.id,
        q.id,
        request,
    )
    .await
    .unwrap();
    assert_eq!(result.request_id, again.request_id);
    assert_eq!(state.db.job_outbox().await.unwrap().len(), 1);
    assert_eq!(
        state
            .db
            .job(job.detail.job.id)
            .await
            .unwrap()
            .unwrap()
            .detail
            .questions[0]
            .state,
        JobQuestionState::AnswerAccepted
    );
}

#[tokio::test]
async fn changed_preview_returns_a_durable_rejection_not_an_uncertain_receipt_gap() {
    let (state, job, _) = fixture().await;
    let preview = JobPreview {
        id: Uuid::new_v4(),
        project_id: job.detail.job.project_id.clone(),
        fingerprint: "fingerprint".into(),
        expires_at: (Utc::now() - Duration::seconds(1)).to_rfc3339(),
        task: job.detail.task.clone(),
        machine_id: job.detail.job.machine_id,
        cluster_owner_user_id: job.detail.job.cluster_owner_user_id,
        hostname: "test-host".into(),
        limits: job.detail.job.limits.clone(),
        policy_digest: job.detail.policy_digest.clone(),
    };
    state
        .db
        .store_job_preview(job.initiator, &preview)
        .await
        .unwrap();
    let request = JobAdmissionRequest {
        request_id: Uuid::new_v4(),
        preview_id: preview.id,
        fingerprint: preview.fingerprint.clone(),
    };
    let result = admit(&state, job.initiator, &preview.project_id, request.clone())
        .await
        .unwrap();
    assert_eq!(result.status, JobReceiptStatus::Rejected);
    assert_eq!(result.error_code.as_deref(), Some("preview_changed"));
    assert_eq!(
        receipt(&state, job.initiator, request.request_id)
            .await
            .unwrap()
            .status,
        JobReceiptStatus::Rejected
    );
    assert_eq!(
        admit(&state, job.initiator, &preview.project_id, request.clone())
            .await
            .unwrap()
            .status,
        JobReceiptStatus::Rejected
    );
    let mut changed = request;
    changed.fingerprint = "different".into();
    assert!(admit(&state, job.initiator, &preview.project_id, changed)
        .await
        .is_err());
}

#[tokio::test]
async fn unauthorized_socket_fanout_drops_queued_private_content() {
    let (state, job, _) = fixture().await;
    let connection = Uuid::new_v4();
    let (tx, _) = tokio::sync::mpsc::channel(8);
    state.sessions.register_web(connection, tx);
    state.sessions.set_web_user(connection, job.initiator);
    state
        .sessions
        .set_web_capabilities(connection, vec![AUTONOMOUS_JOBS_CAPABILITY.into()]);
    state.jobs.connection_auth.insert(
        connection,
        (
            job.initiator,
            None,
            (Utc::now() + Duration::hours(1)).timestamp() as usize,
        ),
    );
    let message = ServerToWeb::JobSnapshot {
        detail: job.detail.clone(),
    };
    assert!(web_outbound_allowed(&state, connection, &message).await);
    state.jobs.connection_auth.insert(
        connection,
        (
            job.initiator,
            Some("revoked-device".into()),
            (Utc::now() + Duration::hours(1)).timestamp() as usize,
        ),
    );
    assert!(!web_outbound_allowed(&state, connection, &message).await);
    assert_eq!(
        state
            .db
            .job(job.detail.job.id)
            .await
            .unwrap()
            .unwrap()
            .detail
            .job
            .state,
        JobState::Queued
    );
}

#[tokio::test]
async fn concurrent_confirmations_create_one_job_and_one_actor_receipt() {
    let (state, job, mut daemon) = fixture().await;
    let preview = JobPreview {
        id: Uuid::new_v4(),
        project_id: job.detail.job.project_id.clone(),
        fingerprint: "reviewed".into(),
        expires_at: (Utc::now() + Duration::minutes(5)).to_rfc3339(),
        task: job.detail.task.clone(),
        machine_id: job.detail.job.machine_id,
        cluster_owner_user_id: job.detail.job.cluster_owner_user_id,
        hostname: "test-host".into(),
        limits: job.detail.job.limits.clone(),
        policy_digest: job.detail.policy_digest.clone(),
    };
    state
        .db
        .store_job_preview(job.initiator, &preview)
        .await
        .unwrap();
    let responder_state = state.clone();
    let approved = preview.clone();
    let responder = tokio::spawn(async move {
        while let Some(message) = daemon.recv().await {
            if let ServerToDaemon::AutonomousJob {
                command:
                    JobWorkerCommand::Prepare {
                        command_id,
                        project_id,
                        machine_id,
                        cluster_owner_user_id,
                        base_ref,
                        ..
                    },
            } = message
            {
                let profile = responder_state
                    .jobs
                    .readiness
                    .get(&machine_id)
                    .unwrap()
                    .profiles[0]
                    .clone();
                report(
                    &responder_state,
                    machine_id,
                    JobWorkerReport::Prepared {
                        command_id,
                        project_id,
                        machine_id,
                        cluster_owner_user_id,
                        repository: approved.task.repository.clone(),
                        base_ref,
                        base_sha: approved.task.base_sha.clone(),
                        profile,
                    },
                )
                .await
                .unwrap();
            }
        }
    });
    let request = JobAdmissionRequest {
        request_id: Uuid::new_v4(),
        preview_id: preview.id,
        fingerprint: preview.fingerprint.clone(),
    };
    let (one, two) = tokio::join!(
        admit(&state, job.initiator, &preview.project_id, request.clone()),
        admit(&state, job.initiator, &preview.project_id, request.clone())
    );
    responder.abort();
    let one = one.unwrap();
    let two = two.unwrap();
    assert_eq!(one.status, JobReceiptStatus::Accepted);
    assert_eq!(one.job_id, two.job_id);
    assert_eq!(state.db.all_jobs().await.unwrap().len(), 2);
    let recovered = receipt(&state, job.initiator, request.request_id)
        .await
        .unwrap();
    assert_eq!(recovered.job_id, one.job_id);
    assert!(
        state.db.job_outbox().await.unwrap().is_empty(),
        "confirmation itself must not launch a prompt"
    );
}

#[tokio::test]
async fn external_publication_refresh_is_read_only_and_preserves_terminal_execution() {
    let (state, mut job, _) = fixture().await;
    let publication = JobPublication {
        repository: job.detail.task.repository.clone(),
        number: 12,
        url: "https://github.com/owner/repo/pull/12".into(),
        branch: format!("apas/job-{}", job.detail.job.id),
        base_sha: job.detail.task.base_sha.clone(),
        head_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        draft: true,
        state: "open".into(),
        observed_at: Utc::now().to_rfc3339(),
    };
    job.detail.publication = Some(publication.clone());
    job.transition(JobState::ReviewReady, None);
    job.current().ended_at = Some(Utc::now().to_rfc3339());
    let mut tx = state.db.job_transaction().await.unwrap();
    store::save(&mut tx, &job).await.unwrap();
    tx.commit().await.unwrap();
    refresh_review(
        &state,
        job.initiator,
        job.detail.job.id,
        job.identity().run_id,
    )
    .await
    .unwrap();
    let commands = state.db.job_outbox().await.unwrap();
    assert_eq!(commands.len(), 1);
    let JobWorkerCommand::RefreshPublication {
        command_id,
        identity,
    } = commands[0].3.clone()
    else {
        panic!("review read must not publish or restart")
    };
    assert!(
        outbound_allowed(&state, identity.machine_id, &commands[0].3)
            .await
            .unwrap()
    );
    let mut merged = publication;
    merged.draft = false;
    merged.state = "merged".into();
    report(
        &state,
        identity.machine_id,
        JobWorkerReport::PublicationObserved {
            command_id,
            identity,
            publication: Some(merged),
        },
    )
    .await
    .unwrap();
    let retained = state.db.job(job.detail.job.id).await.unwrap().unwrap();
    assert_eq!(retained.detail.job.state, JobState::ReviewReady);
    assert_eq!(retained.detail.runs.len(), 1);
    assert_eq!(retained.detail.publication.unwrap().state, "merged");
    assert!(state.db.job_outbox().await.unwrap().is_empty());
}

#[tokio::test]
async fn deletion_requires_purge_confirmation_for_every_started_attempt() {
    let (state, mut job, _) = fixture().await;
    let now = Utc::now().to_rfc3339();
    job.dispatched = true;
    job.transition(JobState::Cancelled, Some("test".into()));
    job.current().started_at = Some(now.clone());
    job.current().ended_at = Some(now.clone());
    job.current().stop_evidence = Some(JobStopEvidence {
        boot_id: "boot".into(),
        container_id: "first".into(),
        stopped_at: now.clone(),
        reason: "test".into(),
        descendants_stopped: true,
    });
    let mut second = job.current().clone();
    second.id = Uuid::new_v4();
    second.attempt = 2;
    second.epoch = 2;
    second.stop_evidence.as_mut().unwrap().container_id = "second".into();
    job.detail.job.current_run_id = second.id;
    job.detail.runs.push(second);
    let mut tx = state.db.job_transaction().await.unwrap();
    store::save(&mut tx, &job).await.unwrap();
    tx.commit().await.unwrap();
    state
        .db
        .begin_project_deletion(
            &job.initiator.to_string(),
            &job.detail.job.project_id,
            &job.detail.job.project_id,
        )
        .await
        .unwrap();
    assert!(!project_jobs_stopped(&state, &job.detail.job.project_id)
        .await
        .unwrap());
    assert!(state
        .db
        .delete_project_records(&job.detail.job.project_id)
        .await
        .is_err());
    let commands = state.db.job_outbox().await.unwrap();
    assert_eq!(commands.len(), 2);
    for (_, _, machine, command) in commands {
        let JobWorkerCommand::PurgeRun {
            command_id,
            identity,
        } = command
        else {
            panic!("expected purge only")
        };
        assert!(outbound_allowed(
            &state,
            machine,
            &JobWorkerCommand::PurgeRun {
                command_id,
                identity: identity.clone()
            }
        )
        .await
        .unwrap());
        let mut wrong = identity.clone();
        wrong.epoch += 1;
        assert!(report(
            &state,
            machine,
            JobWorkerReport::Purged {
                command_id,
                identity: wrong
            }
        )
        .await
        .is_err());
        report(
            &state,
            machine,
            JobWorkerReport::Purged {
                command_id,
                identity,
            },
        )
        .await
        .unwrap();
    }
    assert!(project_jobs_stopped(&state, &job.detail.job.project_id)
        .await
        .unwrap());
    assert!(state
        .db
        .delete_project_records(&job.detail.job.project_id)
        .await
        .unwrap());
    assert!(state.db.job(job.detail.job.id).await.unwrap().is_none());
}

#[tokio::test]
async fn notification_intent_commits_with_event_and_survives_restart_before_enqueue() {
    let (state, mut job, _) = fixture().await;
    let mut tx = state.db.job_transaction().await.unwrap();
    let event = store::append(
        &mut tx,
        &mut job,
        JobEventKind::RecoveryRequired,
        "Recovery required",
        None,
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let reopened = Database::new(&state.config.database.path).await.unwrap();
    reopened.run_migrations().await.unwrap();
    let pending = reopened.pending_job_notifications().await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].1.id, event.id);
    assert_eq!(
        reopened
            .job_events(job.detail.job.id, 0)
            .await
            .unwrap()
            .events[0]
            .id,
        event.id
    );
    reopened.complete_job_notification(event.id).await.unwrap();
    assert!(reopened
        .pending_job_notifications()
        .await
        .unwrap()
        .is_empty());
}
