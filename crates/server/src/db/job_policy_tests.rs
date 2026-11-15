use super::*;
use crate::db::{
    jobs::{self, StoredJob},
    User,
};
use shared::*;

async fn fixture() -> (AppState, Uuid, Uuid, Uuid, String, Uuid) {
    let root = std::env::temp_dir().join(format!("apas-job-policy-{}", Uuid::new_v4()));
    let db = Database::new(&root.join("db.sqlite").to_string_lossy())
        .await
        .unwrap();
    db.run_migrations().await.unwrap();
    let state = AppState::new(db, crate::config::Config::default());
    let owner = Uuid::new_v4();
    let host = Uuid::new_v4();
    let member = Uuid::new_v4();
    for id in [owner, host, member] {
        state
            .db
            .create_user(&User {
                id: id.to_string(),
                email: format!("{id}@test"),
                password_hash: "unused".into(),
                created_at: None,
                cluster_role: "user".into(),
                account_status: "active".into(),
            })
            .await
            .unwrap();
    }
    let project = Uuid::new_v4().to_string();
    state
        .db
        .authorize_project_registration(&project, &owner.to_string())
        .await
        .unwrap();
    state
        .db
        .add_project_cluster_placement(&project, &host.to_string(), &owner.to_string(), "test")
        .await
        .unwrap();
    state
        .db
        .add_project_member(&owner.to_string(), &project, &member.to_string())
        .await
        .unwrap();
    let machine = Uuid::new_v4();
    state
        .db
        .add_cluster_member(
            &host.to_string(),
            &format!("{owner}@test"),
            Some(vec![machine.to_string()]),
            None,
        )
        .await
        .unwrap();
    (state, owner, host, member, project, machine)
}

async fn enable(state: &AppState) {
    let policy = JobPolicy {
        enabled: true,
        publish_enabled: true,
        worker_profiles: vec!["worker".into(), "other".into()],
        model_profiles: vec!["model".into()],
        verification_profiles: vec!["verify".into()],
        ..JobPolicy::default()
    };
    state
        .db
        .set_job_policy(
            SYSTEM_ADMIN_ACTOR,
            "deployment",
            "deployment",
            serde_json::to_value(policy).unwrap(),
        )
        .await
        .unwrap();
}

async fn permits(
    state: &AppState,
    actor: Uuid,
    host: Uuid,
    project: &str,
    machine: Uuid,
    initiator: Uuid,
    action: JobAction,
) -> bool {
    authorize_job_action(state, actor, project, host, machine, initiator, action)
        .await
        .unwrap()
}

#[tokio::test]
async fn project_share_is_content_not_unattended_authority_and_machine_grants_are_exact() {
    let (state, owner, host, member, project, machine) = fixture().await;
    assert!(
        !permits(
            &state,
            owner,
            host,
            &project,
            machine,
            owner,
            JobAction::Admit
        )
        .await
    );
    enable(&state).await;
    assert!(
        permits(
            &state,
            member,
            host,
            &project,
            machine,
            owner,
            JobAction::Read
        )
        .await
    );
    for action in [
        JobAction::Admit,
        JobAction::Answer,
        JobAction::Retry,
        JobAction::Publish,
    ] {
        assert!(!permits(&state, member, host, &project, machine, owner, action).await);
    }
    assert!(
        permits(
            &state,
            owner,
            host,
            &project,
            machine,
            owner,
            JobAction::Admit
        )
        .await
    );
    assert!(
        !permits(
            &state,
            owner,
            host,
            &project,
            Uuid::new_v4(),
            owner,
            JobAction::Admit
        )
        .await
    );
    for raw in ["[]", "{broken", "null", "[123]", "[\"not-a-uuid\"]"] {
        sqlx::query("UPDATE cluster_memberships SET allowed_machine_ids = ? WHERE cluster_owner_user_id = ? AND user_id = ?").bind(raw).bind(host.to_string()).bind(owner.to_string()).execute(&state.db.pool).await.unwrap();
        assert!(
            !permits(
                &state,
                owner,
                host,
                &project,
                machine,
                owner,
                JobAction::Admit
            )
            .await
        );
    }
    assert!(
        permits(
            &state,
            host,
            host,
            &project,
            machine,
            owner,
            JobAction::Admit
        )
        .await
    );
}

#[tokio::test]
async fn all_host_policies_narrow_and_source_revisions_pin_previews() {
    let (state, owner, host, _, project, _) = fixture().await;
    enable(&state).await;
    let initial = effective_job_policy(&state.db, &project).await.unwrap();
    let mut limits = JobLimits::default();
    limits.elapsed_seconds = 900;
    state
        .db
        .set_job_policy(
            &host.to_string(),
            "cluster",
            &host.to_string(),
            serde_json::json!({"limits": limits, "worker_profiles": ["worker"]}),
        )
        .await
        .unwrap();
    state
        .db
        .set_job_policy(
            &owner.to_string(),
            "cluster",
            &owner.to_string(),
            serde_json::json!({"publish_enabled": false}),
        )
        .await
        .unwrap();
    let policy = effective_job_policy(&state.db, &project).await.unwrap();
    assert_eq!(policy.policy.limits.elapsed_seconds, 900);
    assert_eq!(policy.policy.worker_profiles, vec!["worker"]);
    assert!(!policy.policy.publish_enabled);
    assert_ne!(initial.digest, policy.digest);
    assert!(state
        .db
        .set_job_policy(
            &host.to_string(),
            "project",
            &project,
            serde_json::json!({"publish_enabled": true})
        )
        .await
        .is_err());
    assert!(state
        .db
        .set_job_policy(
            &host.to_string(),
            "project",
            &project,
            serde_json::json!({"worker_profiles": ["other"]})
        )
        .await
        .is_err());
    state
        .db
        .set_job_policy(
            &host.to_string(),
            "project",
            &project,
            serde_json::json!({}),
        )
        .await
        .unwrap();
    let revised = effective_job_policy(&state.db, &project).await.unwrap();
    assert_eq!(policy.policy, revised.policy);
    assert_ne!(policy.digest, revised.digest);
    let first = snapshot(policy.policy.clone(), policy.sources.clone(), None).unwrap();
    assert_eq!(first.digest, policy.digest);
}

#[tokio::test]
async fn recurring_authority_is_not_inherited_from_finite_job_enablement() {
    let (state, _, host, _, project, _) = fixture().await;
    enable(&state).await;
    let initial = effective_job_policy(&state.db, &project).await.unwrap();
    assert!(initial.policy.enabled);
    assert!(!initial.policy.recurring.enabled);
    let requested = RecurringWorkerPolicy {
        enabled: true,
        ..RecurringWorkerPolicy::default()
    };
    assert!(state
        .db
        .set_job_policy(
            &host.to_string(),
            "cluster",
            &host.to_string(),
            serde_json::json!({ "recurring": requested }),
        )
        .await
        .is_err());
    let unchanged = effective_job_policy(&state.db, &project).await.unwrap();
    assert_eq!(initial.digest, unchanged.digest);
    assert!(!unchanged.policy.recurring.enabled);
}

#[tokio::test]
async fn recurring_host_intersection_rejects_faster_cadence_and_larger_allowances() {
    let (state, owner, host, _, project, _) = fixture().await;
    let deployment = JobPolicy {
        enabled: true,
        recurring: RecurringWorkerPolicy {
            enabled: true,
            ..RecurringWorkerPolicy::default()
        },
        ..JobPolicy::default()
    };
    state
        .db
        .set_job_policy(
            SYSTEM_ADMIN_ACTOR,
            "deployment",
            "deployment",
            serde_json::to_value(deployment).unwrap(),
        )
        .await
        .unwrap();
    let host_bound = RecurringWorkerPolicy {
        enabled: true,
        max_spend_micro_usd_24h: 6_000_000,
        max_runs_24h: 8,
        min_interval_seconds: 1800,
        ..RecurringWorkerPolicy::default()
    };
    let owner_bound = RecurringWorkerPolicy {
        enabled: true,
        max_elapsed_seconds_24h: 3600,
        min_interval_seconds: 1200,
        ..RecurringWorkerPolicy::default()
    };
    for (actor, bound) in [(host, &host_bound), (owner, &owner_bound)] {
        state
            .db
            .set_job_policy(
                &actor.to_string(),
                "cluster",
                &actor.to_string(),
                serde_json::json!({ "recurring": bound }),
            )
            .await
            .unwrap();
    }
    let effective = effective_job_policy(&state.db, &project).await.unwrap();
    assert_eq!(
        effective.policy.recurring,
        RecurringWorkerPolicy {
            enabled: true,
            max_elapsed_seconds_24h: 3600,
            max_spend_micro_usd_24h: 6_000_000,
            max_runs_24h: 8,
            min_interval_seconds: 1800,
        }
    );
    for invalid in [
        RecurringWorkerPolicy {
            max_runs_24h: 9,
            ..effective.policy.recurring.clone()
        },
        RecurringWorkerPolicy {
            min_interval_seconds: 1799,
            ..effective.policy.recurring.clone()
        },
        RecurringWorkerPolicy {
            max_elapsed_seconds_24h: 0,
            ..effective.policy.recurring.clone()
        },
    ] {
        assert!(state
            .db
            .set_job_policy(
                &host.to_string(),
                "project",
                &project,
                serde_json::json!({ "recurring": invalid }),
            )
            .await
            .is_err());
    }
    let unchanged = effective_job_policy(&state.db, &project).await.unwrap();
    assert_eq!(effective.digest, unchanged.digest);
    state.db.set_job_policy(
        &owner.to_string(), "cluster", &owner.to_string(),
        serde_json::json!({ "recurring": RecurringWorkerPolicy { enabled: false, ..owner_bound } }),
    ).await.unwrap();
    let denied = effective_job_policy(&state.db, &project).await.unwrap();
    assert!(!denied.policy.recurring.enabled);
    assert!(denied.policy.enabled);
    assert_ne!(effective.digest, denied.digest);
}

#[tokio::test]
async fn malformed_policy_denies_and_interactive_policy_never_enables_jobs() {
    let (state, owner, host, member, project, machine) = fixture().await;
    let interactive = state
        .db
        .get_effective_project_policy(&project)
        .await
        .unwrap();
    assert!(
        !effective_job_policy(&state.db, &project)
            .await
            .unwrap()
            .policy
            .enabled
    );
    assert!(state
        .db
        .set_job_policy(
            &member.to_string(),
            "project",
            &project,
            serde_json::json!({})
        )
        .await
        .is_err());
    assert!(state
        .db
        .set_job_policy(
            &owner.to_string(),
            "deployment",
            "deployment",
            serde_json::to_value(JobPolicy::default()).unwrap()
        )
        .await
        .is_err());
    enable(&state).await;
    for raw in [
        "null",
        "{broken",
        "{\"enabled\":\"yes\"}",
        "{\"unrecognized\":true}",
    ] {
        sqlx::query("INSERT INTO autonomous_job_policies(scope,scope_id,policy_json,revision) VALUES ('project',?,?,1) ON CONFLICT(scope,scope_id) DO UPDATE SET policy_json=excluded.policy_json,revision=revision+1").bind(&project).bind(raw).execute(&state.db.pool).await.unwrap();
        assert!(effective_job_policy(&state.db, &project)
            .await
            .unwrap_err()
            .to_string()
            .starts_with("job_policy_invalid:"));
        assert!(authorize_job_action(
            &state,
            owner,
            &project,
            host,
            machine,
            owner,
            JobAction::Admit
        )
        .await
        .is_err());
        assert!(
            permits(
                &state,
                owner,
                host,
                &project,
                machine,
                owner,
                JobAction::Cancel
            )
            .await
        );
    }
    state
        .db
        .set_job_policy(
            &host.to_string(),
            "project",
            &project,
            serde_json::json!({}),
        )
        .await
        .unwrap();
    assert!(
        effective_job_policy(&state.db, &project)
            .await
            .unwrap()
            .policy
            .enabled
    );
    assert_eq!(
        serde_json::to_value(interactive).unwrap(),
        serde_json::to_value(
            state
                .db
                .get_effective_project_policy(&project)
                .await
                .unwrap()
        )
        .unwrap()
    );
}

async fn running_job(
    state: &AppState,
    owner: Uuid,
    host: Uuid,
    project: &str,
    machine: Uuid,
) -> Uuid {
    let id = Uuid::new_v4();
    let run = Uuid::new_v4();
    let now = chrono::Utc::now().to_rfc3339();
    let record: StoredJob = serde_json::from_value(serde_json::json!({
        "initiator": owner, "review": null, "dispatched": true, "source_seq": 0,
        "lease_expires_at": null, "last_renewed_at": null,
        "detail": {
            "job": {"id": id,"project_id":project,"project_name":"test","title":"task","state":"running","state_version":1,"current_run_id":run,"cluster_owner_user_id":host,"machine_id":machine,"hostname":"offline-host","created_at":now,"updated_at":now,"usage":JobUsage::default(),"limits":JobLimits::default(),"attention_count":0,"reason":null,"allowed_actions":JobAllowedActions::default()},
            "task":{"instruction":"test","acceptance_criteria":["test"],"source_url":null,"repository":"test/repo","base_ref":"main","base_sha":"base","worker_profile":"worker","worker_profile_digest":"worker-digest","verification_profile":"verify","verification_profile_digest":"verify-digest","model_profile":"model"},
            "runs":[{"id":run,"job_id":id,"attempt":1,"epoch":1,"state":"running","usage":JobUsage::default(),"started_at":now,"ended_at":null,"worker_version":"test","candidate_sha":null,"stop_evidence":null,"reason":null}],
            "questions":[],"high_watermark":0,"policy_digest":effective_job_policy(&state.db, project).await.unwrap().digest,"publication":null
        }
    })).unwrap();
    let mut tx = state.db.job_transaction().await.unwrap();
    jobs::save(&mut tx, &record).await.unwrap();
    tx.commit().await.unwrap();
    id
}

#[tokio::test]
async fn reconciliation_covers_more_than_one_project_page_and_offline_stop_is_not_evidence() {
    let (state, owner, host, _, first, machine) = fixture().await;
    enable(&state).await;
    let mut ids = vec![running_job(&state, owner, host, &first, machine).await];
    for _ in 0..204 {
        let project = Uuid::new_v4().to_string();
        state
            .db
            .authorize_project_registration(&project, &owner.to_string())
            .await
            .unwrap();
        state
            .db
            .add_project_cluster_placement(&project, &host.to_string(), &owner.to_string(), "test")
            .await
            .unwrap();
        ids.push(running_job(&state, owner, host, &project, machine).await);
    }
    state
        .db
        .revoke_cluster_membership(&host.to_string(), &owner.to_string())
        .await
        .unwrap();
    crate::jobs::reconcile_authority(&state).await.unwrap();
    for id in ids {
        let record = state.db.job(id).await.unwrap().unwrap();
        assert_eq!(record.detail.job.state, JobState::Cancelling);
        assert!(
            !crate::jobs::project_jobs_stopped(&state, &record.detail.job.project_id)
                .await
                .unwrap()
        );
    }
    state
        .db
        .begin_project_deletion(&owner.to_string(), &first, &first)
        .await
        .unwrap();
    assert!(crate::project_lifecycle::cleanup_project(&state, &first)
        .await
        .is_err());
    assert!(state.db.get_project(&first).await.unwrap().is_some());
}

#[tokio::test]
async fn suspension_preserves_read_and_stop_but_revokes_execution() {
    let (state, owner, host, member, project, machine) = fixture().await;
    enable(&state).await;
    state
        .db
        .set_project_lifecycle(
            SYSTEM_ADMIN_ACTOR,
            &project,
            crate::db::ProjectLifecycle::Suspended,
        )
        .await
        .unwrap();
    assert!(
        permits(
            &state,
            member,
            host,
            &project,
            machine,
            owner,
            JobAction::Read
        )
        .await
    );
    assert!(
        permits(
            &state,
            owner,
            host,
            &project,
            machine,
            owner,
            JobAction::Cancel
        )
        .await
    );
    assert!(
        !permits(
            &state,
            owner,
            host,
            &project,
            machine,
            owner,
            JobAction::Admit
        )
        .await
    );
    assert!(
        !permits(
            &state,
            owner,
            host,
            &project,
            machine,
            owner,
            JobAction::Answer
        )
        .await
    );
}

fn headers(state: &AppState, user: Uuid, admin: bool) -> axum::http::HeaderMap {
    let claims = crate::routes::auth::Claims {
        sub: if admin {
            crate::routes::system_admin::SYSTEM_ADMIN_SUBJECT.into()
        } else {
            user.to_string()
        },
        exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
        device_session_id: None,
        token_kind: admin.then(|| crate::routes::system_admin::SYSTEM_ADMIN_TOKEN_KIND.to_string()),
        credential_version: admin.then_some(1),
    };
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(state.config.auth.jwt_secret.as_bytes()),
    )
    .unwrap();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        format!("Bearer {token}").parse().unwrap(),
    );
    headers
}

#[tokio::test]
async fn every_authority_mutation_route_fences_durable_execution() {
    use crate::routes::{admin, cluster, job_policy, projects, share};
    use axum::{
        extract::{Path, State},
        Json,
    };
    for mutation in [
        "owner_transfer",
        "cluster_transfer",
        "admin_transfer",
        "leave",
        "cluster_remove",
        "admin_remove",
        "share_remove",
        "machine_grant",
        "cluster_revoke",
        "account_suspend",
        "cluster_suspend",
        "admin_suspend",
        "cluster_stop",
        "admin_stop",
        "project_policy",
        "cluster_policy",
        "deployment_policy",
        "delete",
    ] {
        let (state, owner, host, member, project, machine) = fixture().await;
        enable(&state).await;
        state
            .db
            .seed_system_admin_credential("test-admin", "unused")
            .await
            .unwrap();
        let id = running_job(&state, owner, host, &project, machine).await;
        let ordinary = headers(&state, owner, false);
        let operator = headers(&state, host, false);
        let system = headers(&state, host, true);
        if matches!(
            mutation,
            "leave" | "cluster_remove" | "admin_remove" | "share_remove"
        ) {
            // The prior approver is now an ordinary member. The following route must
            // reconcile that durable authority loss even with no daemon connected.
            state
                .db
                .transfer_project_ownership(SYSTEM_ADMIN_ACTOR, &project, &member.to_string())
                .await
                .unwrap();
        }
        match mutation {
            "owner_transfer" => {
                projects::transfer_owner(
                    State(state.clone()),
                    ordinary,
                    Path(project.clone()),
                    Json(projects::TransferOwnerRequest {
                        user_id: member.to_string(),
                    }),
                )
                .await
                .unwrap();
            }
            "cluster_transfer" => {
                cluster::transfer_owner(
                    State(state.clone()),
                    operator,
                    Path(project.clone()),
                    Json(admin::TransferOwnerRequest {
                        user_id: member.to_string(),
                    }),
                )
                .await
                .unwrap();
            }
            "admin_transfer" => {
                admin::transfer_owner(
                    State(state.clone()),
                    system,
                    Path(project.clone()),
                    Json(admin::TransferOwnerRequest {
                        user_id: member.to_string(),
                    }),
                )
                .await
                .unwrap();
            }
            "leave" => {
                projects::leave_project(State(state.clone()), ordinary, Path(project.clone()))
                    .await
                    .unwrap();
            }
            "cluster_remove" => {
                cluster::remove_project_member(
                    State(state.clone()),
                    operator,
                    Path((project.clone(), owner.to_string())),
                )
                .await
                .unwrap();
            }
            "admin_remove" => {
                admin::remove_project_member(
                    State(state.clone()),
                    system,
                    Path((project.clone(), owner.to_string())),
                )
                .await
                .unwrap();
            }
            "share_remove" => {
                let session = Uuid::new_v4().to_string();
                state
                    .db
                    .create_session(&crate::db::Session {
                        id: session.clone(),
                        user_id: member.to_string(),
                        cli_client_id: None,
                        working_dir: None,
                        hostname: None,
                        status: "active".into(),
                        created_at: None,
                        updated_at: None,
                        is_paused: false,
                        project_id: Some(project.clone()),
                        git_remote: None,
                        git_remote_url: None,
                    })
                    .await
                    .unwrap();
                state
                    .db
                    .create_session_share(&session, &owner.to_string(), &member.to_string())
                    .await
                    .unwrap();
                share::revoke_access(
                    State(state.clone()),
                    headers(&state, member, false),
                    Path((session, owner.to_string())),
                )
                .await
                .unwrap();
            }
            "machine_grant" => {
                cluster::update_member(
                    State(state.clone()),
                    operator,
                    Path(owner.to_string()),
                    Json(cluster::UpdateClusterMemberRequest {
                        allowed_machine_ids: Some(vec![]),
                        default_launch_profile: None,
                    }),
                )
                .await
                .unwrap();
            }
            "cluster_revoke" => {
                cluster::revoke_member(State(state.clone()), operator, Path(owner.to_string()))
                    .await
                    .unwrap();
            }
            "account_suspend" => {
                admin::update_user(
                    State(state.clone()),
                    system,
                    Path(owner.to_string()),
                    Json(admin::UpdateUserRequest {
                        account_status: Some("suspended".into()),
                    }),
                )
                .await
                .unwrap();
            }
            "cluster_suspend" => {
                cluster::update_lifecycle(
                    State(state.clone()),
                    operator,
                    Path(project.clone()),
                    Json(admin::LifecycleRequest {
                        status: "suspended".into(),
                    }),
                )
                .await
                .unwrap();
            }
            "admin_suspend" => {
                admin::update_lifecycle(
                    State(state.clone()),
                    system,
                    Path(project.clone()),
                    Json(admin::LifecycleRequest {
                        status: "suspended".into(),
                    }),
                )
                .await
                .unwrap();
            }
            "cluster_stop" => {
                cluster::stop_runtime(State(state.clone()), operator, Path(project.clone()))
                    .await
                    .unwrap();
            }
            "admin_stop" => {
                admin::stop_runtime(State(state.clone()), system, Path(project.clone()))
                    .await
                    .unwrap();
            }
            "project_policy" => {
                job_policy::put_operator_project(
                    State(state.clone()),
                    operator,
                    Path(project.clone()),
                    Json(JobPolicyOverride {
                        enabled: Some(false),
                        ..Default::default()
                    }),
                )
                .await
                .unwrap();
            }
            "cluster_policy" => {
                job_policy::put_cluster(
                    State(state.clone()),
                    operator,
                    Json(JobPolicyOverride {
                        enabled: Some(false),
                        ..Default::default()
                    }),
                )
                .await
                .unwrap();
            }
            "deployment_policy" => {
                job_policy::put_deployment(
                    State(state.clone()),
                    system,
                    Json(JobPolicy::default()),
                )
                .await
                .unwrap();
            }
            "delete" => {
                projects::delete_project(
                    State(state.clone()),
                    ordinary,
                    Path(project.clone()),
                    Json(projects::DeleteProjectRequest {
                        confirmation: project.clone(),
                    }),
                )
                .await
                .unwrap();
            }
            _ => unreachable!(),
        }
        let record = state.db.job(id).await.unwrap().unwrap();
        assert_eq!(record.detail.job.state, JobState::Cancelling, "{mutation}");
        assert!(
            !crate::jobs::project_jobs_stopped(&state, &project)
                .await
                .unwrap(),
            "{mutation}"
        );
    }
}

#[tokio::test]
async fn device_revocation_keeps_account_job_but_account_suspension_cancels_it() {
    use axum::extract::{Path, State};
    let (state, owner, host, _, project, machine) = fixture().await;
    enable(&state).await;
    let id = running_job(&state, owner, host, &project, machine).await;
    let device = Uuid::new_v4().to_string();
    state
        .db
        .create_mobile_device_session(
            &device,
            &owner.to_string(),
            &Uuid::new_v4().to_string(),
            "ios",
            None,
            "test",
            "refresh-hash",
            &(chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339(),
        )
        .await
        .unwrap();
    let connection = Uuid::new_v4();
    let (sender, _receiver) = tokio::sync::mpsc::channel(4);
    state.sessions.register_web(connection, sender);
    state.sessions.set_web_user(connection, owner);
    state.jobs.connection_auth.insert(
        connection,
        (
            owner,
            Some(device.clone()),
            (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
        ),
    );
    state
        .jobs
        .subscriptions
        .insert((connection, id), project.clone());
    crate::routes::mobile_auth::revoke_device(
        State(state.clone()),
        headers(&state, owner, false),
        Path(device.clone()),
    )
    .await
    .unwrap();
    assert!(!state
        .db
        .is_mobile_device_session_active(&device, &owner.to_string())
        .await
        .unwrap());
    assert!(!state.jobs.subscriptions.contains_key(&(connection, id)));
    assert_eq!(
        state.db.job(id).await.unwrap().unwrap().detail.job.state,
        JobState::Running
    );
    state
        .db
        .update_cluster_user_status(
            SYSTEM_ADMIN_ACTOR,
            &owner.to_string(),
            crate::db::AccountStatus::Suspended,
        )
        .await
        .unwrap();
    crate::jobs::reconcile_authority(&state).await.unwrap();
    assert_eq!(
        state.db.job(id).await.unwrap().unwrap().detail.job.state,
        JobState::Cancelling
    );
}

#[tokio::test]
async fn ownership_without_hosting_does_not_grant_policy_administration() {
    use axum::{
        extract::{Path, State},
        Json,
    };
    let (state, owner, host, _, project, _) = fixture().await;
    sqlx::query(
        "DELETE FROM project_cluster_placements WHERE project_id = ? AND cluster_owner_user_id = ?",
    )
    .bind(&project)
    .bind(owner.to_string())
    .execute(&state.db.pool)
    .await
    .unwrap();
    assert!(matches!(
        crate::routes::job_policy::put_operator_project(
            State(state.clone()),
            headers(&state, owner, false),
            Path(project.clone()),
            Json(JobPolicyOverride::default())
        )
        .await,
        Err(crate::error::AppError::Forbidden(_))
    ));
    let Json(view) = crate::routes::job_policy::get_project(
        State(state.clone()),
        headers(&state, owner, false),
        Path(project.clone()),
    )
    .await
    .unwrap();
    assert!(!view.can_manage);
    assert!(!view.effective.unwrap().policy.enabled);
    crate::routes::job_policy::put_operator_project(
        State(state.clone()),
        headers(&state, host, false),
        Path(project),
        Json(JobPolicyOverride::default()),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn malformed_scope_cancels_affected_job_without_skipping_other_projects() {
    let (state, owner, host, _, project, machine) = fixture().await;
    enable(&state).await;
    let affected = running_job(&state, owner, host, &project, machine).await;
    let other = Uuid::new_v4().to_string();
    state
        .db
        .authorize_project_registration(&other, &owner.to_string())
        .await
        .unwrap();
    state
        .db
        .add_project_cluster_placement(&other, &host.to_string(), &owner.to_string(), "test")
        .await
        .unwrap();
    let unaffected = running_job(&state, owner, host, &other, machine).await;
    sqlx::query("INSERT INTO autonomous_job_policies(scope,scope_id,policy_json,revision) VALUES ('project',?,'{\"enabled\":\"invalid\"}',1)").bind(&project).execute(&state.db.pool).await.unwrap();
    crate::jobs::reconcile_authority(&state).await.unwrap();
    assert_eq!(
        state
            .db
            .job(affected)
            .await
            .unwrap()
            .unwrap()
            .detail
            .job
            .state,
        JobState::Cancelling
    );
    assert_eq!(
        state
            .db
            .job(unaffected)
            .await
            .unwrap()
            .unwrap()
            .detail
            .job
            .state,
        JobState::Running
    );
}

#[tokio::test]
async fn management_readiness_is_report_based_scoped_and_sanitized() {
    use axum::{
        extract::{Path, State},
        Json,
    };
    let (state, owner, host, _, project, machine) = fixture().await;
    enable(&state).await;
    let now = chrono::Utc::now();
    state.jobs.readiness.insert(
        machine,
        JobWorkerReadiness {
            machine_id: machine,
            cluster_owner_user_id: host,
            protocol_version: JOB_PROTOCOL_VERSION,
            checked_at: now.to_rfc3339(),
            expires_at: (now + chrono::Duration::seconds(JOB_READINESS_SECONDS as i64))
                .to_rfc3339(),
            profiles: vec![JobWorkerProfile {
                key: "worker".into(),
                label: "Private worker".into(),
                digest: "digest".into(),
                image: "image".into(),
                omp_version: "version".into(),
                model_profiles: vec![],
                verification_profiles: vec![],
                limits: JobLimits::default(),
                ready: false,
                unavailable_reasons: vec![
                    "Cannot read dedicated credential at /private/operator/secret-token".into(),
                ],
            }],
        },
    );
    let Json(own) =
        crate::routes::job_policy::get_cluster(State(state.clone()), headers(&state, owner, false))
            .await
            .unwrap();
    assert!(own
        .unavailable_reasons
        .contains(&"job_readiness_not_reported".into()));
    assert!(!own
        .unavailable_reasons
        .contains(&"job_credential_unavailable".into()));
    let Json(hosted) = crate::routes::job_policy::get_operator_project(
        State(state.clone()),
        headers(&state, host, false),
        Path(project),
    )
    .await
    .unwrap();
    assert!(hosted
        .unavailable_reasons
        .contains(&"job_credential_unavailable".into()));
    assert!(hosted
        .unavailable_reasons
        .contains(&"job_target_offline".into()));
    assert!(!serde_json::to_string(&hosted)
        .unwrap()
        .contains("/private/operator"));
    assert!(!serde_json::to_string(&hosted)
        .unwrap()
        .contains("secret-token"));
}

#[tokio::test]
async fn membership_upsert_revocation_survives_grant_restoration() {
    use axum::{extract::State, Json};
    let (state, owner, host, _, project, machine) = fixture().await;
    enable(&state).await;
    let id = running_job(&state, owner, host, &project, machine).await;
    crate::routes::cluster::add_member(
        State(state.clone()),
        headers(&state, host, false),
        Json(crate::routes::cluster::AddClusterMemberRequest {
            email: format!("{owner}@test"),
            allowed_machine_ids: Some(vec![]),
            default_launch_profile: None,
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        state.db.job(id).await.unwrap().unwrap().detail.job.state,
        JobState::Cancelling
    );
    state
        .db
        .add_cluster_member(
            &host.to_string(),
            &format!("{owner}@test"),
            Some(vec![machine.to_string()]),
            None,
        )
        .await
        .unwrap();
    crate::jobs::reconcile_authority(&state).await.unwrap();
    assert_eq!(
        state.db.job(id).await.unwrap().unwrap().detail.job.state,
        JobState::Cancelling
    );
}

#[tokio::test]
async fn unknown_refresh_cannot_trigger_job_or_subscription_reconciliation() {
    use axum::{extract::State, Json};
    let (state, owner, host, _, project, machine) = fixture().await;
    enable(&state).await;
    let id = running_job(&state, owner, host, &project, machine).await;
    // A pending authority change is deliberately not reconciled here. An unrelated
    // unknown credential must not gain the ability to trigger the global scan.
    state
        .db
        .revoke_cluster_membership(&host.to_string(), &owner.to_string())
        .await
        .unwrap();
    let stale_connection = Uuid::new_v4();
    state
        .jobs
        .subscriptions
        .insert((stale_connection, id), project);
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("x-forwarded-proto", "https".parse().unwrap());
    let result = crate::routes::mobile_auth::refresh(
        State(state.clone()),
        headers,
        Json(MobileRefreshRequest {
            refresh_token: "unrecognized-random-token".into(),
            installation_id: "unknown-installation".into(),
        }),
    )
    .await;
    assert!(
        matches!(result, Err(crate::error::AppError::AuthError(message)) if message == "Invalid refresh token")
    );
    assert_eq!(
        state.db.job(id).await.unwrap().unwrap().detail.job.state,
        JobState::Running
    );
    assert!(state
        .jobs
        .subscriptions
        .contains_key(&(stale_connection, id)));
}

#[tokio::test]
async fn deletion_stops_reachable_interactive_host_while_job_host_is_offline() {
    let (state, owner, host, _, project, machine) = fixture().await;
    enable(&state).await;
    let id = running_job(&state, owner, host, &project, machine).await;
    let interactive = Uuid::new_v4();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
    state.sessions.register_daemon(
        interactive,
        owner,
        sender,
        MachineInfo {
            machine_id: interactive,
            hostname: "interactive".into(),
            os: "linux".into(),
            arch: "x64".into(),
            daemon_version: None,
            deepseek_backend: None,
            last_seen: None,
            providers: None,
        },
        vec![MachineProjectInfo {
            project_id: project.clone(),
            name: None,
            path: "/workspace".into(),
            preferred_machine_id: None,
            is_running: true,
            pid: None,
            memory_kb: None,
            last_error: None,
            git_remote: None,
        }],
    );
    state
        .sessions
        .set_daemon_capabilities(interactive, vec![PANE_HOST_CLEANUP_ACK_CAPABILITY.into()]);
    state
        .db
        .begin_project_deletion(&owner.to_string(), &project, &project)
        .await
        .unwrap();
    let cleanup_state = state.clone();
    let cleanup_project = project.clone();
    let cleanup = tokio::spawn(async move {
        crate::project_lifecycle::cleanup_project(&cleanup_state, &cleanup_project).await
    });
    let command = tokio::time::timeout(std::time::Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let ServerToDaemon::StopProjectCli {
        project_id,
        request_id: Some(request_id),
    } = command
    else {
        panic!("expected interactive stop command");
    };
    assert_eq!(project_id, project);
    state
        .sessions
        .complete_project_runtime_stop(request_id, project_id, true, 0, None);
    assert!(cleanup.await.unwrap().is_err());
    assert!(state.db.get_project(&project).await.unwrap().is_some());
    assert_eq!(
        state.db.job(id).await.unwrap().unwrap().detail.job.state,
        JobState::Cancelling
    );
}

#[tokio::test]
async fn scheduled_deletion_finishes_after_late_stop_and_purge_evidence_without_resubmission() {
    let (state, owner, host, _, project, machine) = fixture().await;
    enable(&state).await;
    let id = running_job(&state, owner, host, &project, machine).await;
    state
        .db
        .begin_project_deletion(&owner.to_string(), &project, &project)
        .await
        .unwrap();
    crate::project_lifecycle::schedule_project_cleanup(state.clone(), project.clone());
    crate::project_lifecycle::schedule_project_cleanup(state.clone(), project.clone());
    assert_eq!(state.jobs.cleanup_scheduled.len(), 1);
    // Evidence arrives after the former 0/1/3-second retry window.
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    assert!(state.db.get_project(&project).await.unwrap().is_some());
    let mut tx = state.db.job_transaction().await.unwrap();
    let mut record = jobs::load(&mut tx, id).await.unwrap();
    record.transition(JobState::Cancelled, None);
    record.current().stop_evidence = Some(JobStopEvidence {
        boot_id: "recorded-boot".into(),
        container_id: "recorded-container".into(),
        stopped_at: chrono::Utc::now().to_rfc3339(),
        reason: "cancelled".into(),
        descendants_stopped: true,
    });
    let run_id = record.current().id;
    jobs::save(&mut tx, &record).await.unwrap();
    tx.commit().await.unwrap();
    assert!(!crate::jobs::project_jobs_stopped(&state, &project)
        .await
        .unwrap());
    // Model committed exact-host purge confirmation independently of scheduler timing.
    sqlx::query("UPDATE autonomous_job_purges SET confirmed = 1 WHERE run_id = ?")
        .bind(run_id.to_string())
        .execute(&state.db.pool)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(12), async {
        loop {
            if state.db.get_project(&project).await.unwrap().is_none()
                && !state.jobs.cleanup_scheduled.contains_key(&project)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("durable scheduled deletion should finish without another request");
}
