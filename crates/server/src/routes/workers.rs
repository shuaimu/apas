use crate::{jobs, state::AppState, workers};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use shared::*;
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/projects/workers", get(list))
        .route("/projects/workers/:worker", get(resolve))
        .route("/projects/worker-requests/:request", get(receipt))
        .route("/projects/:project/worker-targets", get(targets))
        .route("/projects/:project/worker-previews", post(preview))
        .route(
            "/projects/:project/workers",
            get(project_list).post(approve),
        )
        .route("/projects/:project/workers/:worker", get(detail))
        .route("/projects/:project/workers/:worker/mandate", post(mandate))
        .route("/projects/:project/workers/:worker/pause", post(pause))
        .route("/projects/:project/workers/:worker/stop", post(stop))
        .route("/projects/:project/workers/:worker/resume", post(resume))
        .route("/projects/:project/workers/:worker/wake", post(wake))
        .route(
            "/projects/:project/workers/:worker/items",
            get(items).post(upsert_item),
        )
        .route(
            "/projects/:project/workers/:worker/items/:item/dismiss",
            post(dismiss_item),
        )
        .route(
            "/projects/:project/workers/:worker/items/:item/retry",
            post(retry_item),
        )
        .route("/projects/:project/workers/:worker/runs", get(runs))
        .route("/projects/:project/workers/:worker/runs/:run", get(run))
        .route(
            "/projects/:project/workers/:worker/runs/:run/review",
            get(review),
        )
        .route(
            "/projects/:project/workers/:worker/runs/:run/artifacts/:artifact",
            get(artifact),
        )
        .route(
            "/projects/:project/workers/:worker/runs/:run/questions/:question/answers",
            post(answer),
        )
        .route(
            "/projects/:project/workers/:worker/runs/:run/retry-publication",
            post(retry_publication),
        )
        .route("/projects/:project/workers/:worker/events", get(events))
}
struct WorkerError {
    error: anyhow::Error,
    request: Option<Uuid>,
}
impl From<anyhow::Error> for WorkerError {
    fn from(error: anyhow::Error) -> Self {
        Self {
            error,
            request: None,
        }
    }
}
impl IntoResponse for WorkerError {
    fn into_response(self) -> Response {
        let raw = self.error.to_string();
        let code = if raw.len() <= 80 && raw.bytes().all(|c| c.is_ascii_lowercase() || c == b'_') {
            raw.as_str()
        } else {
            "worker_operation_failed"
        };
        let status = if code.contains("not_found") {
            StatusCode::NOT_FOUND
        } else if code.contains("denied") || code == "authentication_required" {
            StatusCode::FORBIDDEN
        } else if code.contains("unavailable") || code == "jobs_disabled" {
            StatusCode::SERVICE_UNAVAILABLE
        } else if code == "worker_operation_failed" {
            StatusCode::INTERNAL_SERVER_ERROR
        } else {
            StatusCode::CONFLICT
        };
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::warn!(error=%self.error,"Worker API request failed");
        }
        (
            status,
            Json(JobApiError {
                error: code.replace('_', " "),
                code: code.into(),
                request_id: self.request,
                retryable: status == StatusCode::SERVICE_UNAVAILABLE,
            }),
        )
            .into_response()
    }
}
type Api<T> = Result<Json<T>, WorkerError>;
async fn actor(state: &AppState, headers: &HeaderMap, mutation: bool) -> Result<Uuid, WorkerError> {
    let user = super::authz::require_active_user(headers, state)
        .await
        .map_err(|_| anyhow::anyhow!("authentication_required"))?;
    let claims = super::mobile_auth::claims_from_headers(headers, state)
        .map_err(|_| anyhow::anyhow!("authentication_required"))?;
    if mutation
        && claims.device_session_id.is_some()
        && !state.config.mobile.features.coding_mutations
    {
        return Err(anyhow::anyhow!("mobile_coding_denied").into());
    }
    Ok(Uuid::parse_str(&user.id).map_err(anyhow::Error::from)?)
}
#[derive(Default, Deserialize)]
struct Page {
    cursor: Option<String>,
    status: Option<String>,
    after_seq: Option<u64>,
    offset: Option<usize>,
    limit: Option<usize>,
}
async fn list(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(page): Query<Page>,
) -> Api<WorkerListResponse> {
    let a = actor(&s, &h, false).await?;
    Ok(Json(
        workers::list(&s, a, None, page.cursor.as_deref(), page.status.as_deref()).await?,
    ))
}
async fn project_list(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(p): Path<String>,
    Query(page): Query<Page>,
) -> Api<WorkerListResponse> {
    let a = actor(&s, &h, false).await?;
    Ok(Json(
        workers::list(
            &s,
            a,
            Some(&p),
            page.cursor.as_deref(),
            page.status.as_deref(),
        )
        .await?,
    ))
}
async fn resolve(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Api<WorkerDetail> {
    let a = actor(&s, &h, false).await?;
    Ok(Json(workers::detail_by_id(&s, a, id).await?))
}
async fn receipt(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Api<WorkerMutationReceipt> {
    let a = actor(&s, &h, false).await?;
    Ok(Json(workers::receipt(&s, a, id).await?))
}
async fn targets(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(p): Path<String>,
) -> Api<WorkerTargetsResponse> {
    let a = actor(&s, &h, false).await?;
    Ok(Json(workers::targets(&s, a, &p).await?))
}
async fn preview(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(p): Path<String>,
    Json(r): Json<WorkerPreviewRequest>,
) -> Api<WorkerPreview> {
    let a = actor(&s, &h, true).await?;
    let _guard = s.project_operation_guard(&p).await;
    Ok(Json(workers::preview(&s, a, &p, r).await?))
}
async fn approve(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(p): Path<String>,
    Json(r): Json<WorkerApprovalRequest>,
) -> Api<WorkerMutationReceipt> {
    let a = actor(&s, &h, true).await?;
    let _guard = s.project_operation_guard(&p).await;
    let request = r.request_id;
    Ok(Json(workers::approve(&s, a, &p, None, r).await.map_err(
        |error| WorkerError {
            error,
            request: Some(request),
        },
    )?))
}
async fn mandate(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id)): Path<(String, Uuid)>,
    Json(r): Json<WorkerApprovalRequest>,
) -> Api<WorkerMutationReceipt> {
    let a = actor(&s, &h, true).await?;
    let _guard = s.project_operation_guard(&p).await;
    let request = r.request_id;
    Ok(Json(
        workers::approve(&s, a, &p, Some(id), r)
            .await
            .map_err(|error| WorkerError {
                error,
                request: Some(request),
            })?,
    ))
}
async fn detail(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id)): Path<(String, Uuid)>,
) -> Api<WorkerDetail> {
    let a = actor(&s, &h, false).await?;
    Ok(Json(workers::detail(&s, a, &p, id).await?))
}
async fn items(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id)): Path<(String, Uuid)>,
    Query(page): Query<Page>,
) -> Api<WorkerItemsResponse> {
    let a = actor(&s, &h, false).await?;
    Ok(Json(
        workers::items(
            &s,
            a,
            &p,
            id,
            page.cursor.as_deref(),
            page.status.as_deref(),
        )
        .await?,
    ))
}
async fn runs(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id)): Path<(String, Uuid)>,
    Query(page): Query<Page>,
) -> Api<WorkerRunsResponse> {
    let a = actor(&s, &h, false).await?;
    Ok(Json(
        workers::runs(&s, a, &p, id, page.cursor.as_deref()).await?,
    ))
}
async fn events(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id)): Path<(String, Uuid)>,
    Query(page): Query<Page>,
) -> Api<WorkerEventsResponse> {
    let a = actor(&s, &h, false).await?;
    Ok(Json(
        workers::events(
            &s,
            a,
            &p,
            id,
            page.after_seq.unwrap_or(0),
            page.limit.unwrap_or(100).clamp(1, 1000),
        )
        .await?,
    ))
}
async fn pause(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id)): Path<(String, Uuid)>,
    Json(r): Json<WorkerMutationRequest>,
) -> Api<WorkerMutationReceipt> {
    let a = actor(&s, &h, true).await?;
    let _guard = s.project_operation_guard(&p).await;
    let request = r.request_id;
    Ok(Json(
        workers::mutate(&s, a, &p, id, WorkerMutationKind::Pause, r)
            .await
            .map_err(|error| WorkerError {
                error,
                request: Some(request),
            })?,
    ))
}
async fn stop(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id)): Path<(String, Uuid)>,
    Json(r): Json<WorkerMutationRequest>,
) -> Api<WorkerMutationReceipt> {
    let a = actor(&s, &h, true).await?;
    let _guard = s.project_operation_guard(&p).await;
    let request = r.request_id;
    Ok(Json(
        workers::mutate(&s, a, &p, id, WorkerMutationKind::Stop, r)
            .await
            .map_err(|error| WorkerError {
                error,
                request: Some(request),
            })?,
    ))
}
async fn resume(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id)): Path<(String, Uuid)>,
    Json(r): Json<WorkerMutationRequest>,
) -> Api<WorkerMutationReceipt> {
    let a = actor(&s, &h, true).await?;
    let _guard = s.project_operation_guard(&p).await;
    let request = r.request_id;
    Ok(Json(
        workers::mutate(&s, a, &p, id, WorkerMutationKind::Resume, r)
            .await
            .map_err(|error| WorkerError {
                error,
                request: Some(request),
            })?,
    ))
}
async fn wake(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id)): Path<(String, Uuid)>,
    Json(r): Json<WorkerMutationRequest>,
) -> Api<WorkerMutationReceipt> {
    let a = actor(&s, &h, true).await?;
    let _guard = s.project_operation_guard(&p).await;
    let request = r.request_id;
    Ok(Json(
        workers::mutate(&s, a, &p, id, WorkerMutationKind::Wake, r)
            .await
            .map_err(|error| WorkerError {
                error,
                request: Some(request),
            })?,
    ))
}
async fn upsert_item(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id)): Path<(String, Uuid)>,
    Json(r): Json<WorkerItemRequest>,
) -> Api<WorkerMutationReceipt> {
    let a = actor(&s, &h, true).await?;
    let _guard = s.project_operation_guard(&p).await;
    let request = r.request_id;
    Ok(Json(workers::upsert_item(&s, a, &p, id, r).await.map_err(
        |error| WorkerError {
            error,
            request: Some(request),
        },
    )?))
}
async fn dismiss_item(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id, item)): Path<(String, Uuid, Uuid)>,
    Json(r): Json<WorkerMutationRequest>,
) -> Api<WorkerMutationReceipt> {
    let a = actor(&s, &h, true).await?;
    let _guard = s.project_operation_guard(&p).await;
    let request = r.request_id;
    Ok(Json(
        workers::mutate_item(&s, a, &p, id, item, WorkerMutationKind::DismissItem, r)
            .await
            .map_err(|error| WorkerError {
                error,
                request: Some(request),
            })?,
    ))
}
async fn retry_item(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id, item)): Path<(String, Uuid, Uuid)>,
    Json(r): Json<WorkerMutationRequest>,
) -> Api<WorkerMutationReceipt> {
    let a = actor(&s, &h, true).await?;
    let _guard = s.project_operation_guard(&p).await;
    let request = r.request_id;
    Ok(Json(
        workers::mutate_item(&s, a, &p, id, item, WorkerMutationKind::RetryItem, r)
            .await
            .map_err(|error| WorkerError {
                error,
                request: Some(request),
            })?,
    ))
}
async fn run(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id, run)): Path<(String, Uuid, Uuid)>,
) -> Api<JobDetail> {
    let a = actor(&s, &h, false).await?;
    Ok(Json(workers::exact_run(&s, a, &p, id, run).await?))
}
async fn review(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id, run)): Path<(String, Uuid, Uuid)>,
    Query(page): Query<Page>,
) -> Api<JobReview> {
    let a = actor(&s, &h, false).await?;
    let d = workers::exact_run(&s, a, &p, id, run).await?;
    jobs::refresh_review(&s, a, d.job.id, run).await?;
    let mut review = s.db.job_review(d.job.id, run).await?.unwrap_or(JobReview {
        job_id: d.job.id,
        run_id: run,
        base_sha: d.task.base_sha,
        candidate_sha: None,
        files: vec![],
        verification: None,
        publication: None,
        reason: Some("review_not_available".into()),
        next_cursor: None,
    });
    let offset = page
        .cursor
        .as_deref()
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| anyhow::anyhow!("invalid_cursor"))?;
    if offset > review.files.len() {
        return Err(anyhow::anyhow!("invalid_cursor").into());
    }
    let end = offset
        .saturating_add(page.limit.unwrap_or(100).clamp(1, 100))
        .min(review.files.len());
    review.next_cursor = (end < review.files.len()).then(|| end.to_string());
    review.files = review.files[offset..end].to_vec();
    Ok(Json(review))
}
async fn artifact(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id, run, artifact)): Path<(String, Uuid, Uuid, Uuid)>,
    Query(page): Query<Page>,
) -> Api<JobArtifactContent> {
    let a = actor(&s, &h, false).await?;
    let d = workers::exact_run(&s, a, &p, id, run).await?;
    let content =
        s.db.job_artifact(
            d.job.id,
            artifact,
            page.offset.unwrap_or(0),
            page.limit.unwrap_or(64 * 1024),
        )
        .await?;
    if content.artifact.run_id != run {
        return Err(anyhow::anyhow!("artifact_not_found").into());
    }
    Ok(Json(content))
}
async fn answer(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id, run, question)): Path<(String, Uuid, Uuid, Uuid)>,
    Json(r): Json<JobAnswerRequest>,
) -> Api<JobMutationReceipt> {
    let a = actor(&s, &h, true).await?;
    let _guard = s.project_operation_guard(&p).await;
    let d = workers::exact_run(&s, a, &p, id, run).await?;
    if r.run_id != run
        || !d
            .questions
            .iter()
            .any(|q| q.id == question && q.run_id == run && q.epoch == r.epoch)
    {
        return Err(anyhow::anyhow!("question_not_found").into());
    }
    let request = r.request_id;
    Ok(Json(
        jobs::answer(&s, a, &p, d.job.id, question, r)
            .await
            .map_err(|error| WorkerError {
                error,
                request: Some(request),
            })?,
    ))
}
async fn retry_publication(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id, run)): Path<(String, Uuid, Uuid)>,
    Json(r): Json<JobMutationRequest>,
) -> Api<JobMutationReceipt> {
    let a = actor(&s, &h, true).await?;
    let _guard = s.project_operation_guard(&p).await;
    let d = workers::exact_run(&s, a, &p, id, run).await?;
    if d.job.current_run_id != run {
        return Err(anyhow::anyhow!("run_not_current").into());
    }
    let request = r.request_id;
    Ok(Json(
        jobs::mutate(&s, a, &p, d.job.id, JobMutationKind::RetryPublication, r)
            .await
            .map_err(|error| WorkerError {
                error,
                request: Some(request),
            })?,
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::{jobs as job_store, workers as worker_store, Database, User};
    use chrono::{Duration, Utc};
    use std::path::PathBuf;

    // Real SQLite state, using the same account/project setup as the job and
    // project-join harnesses. No daemon or provider is started by these tests.
    pub(crate) struct Fixture {
        pub state: AppState,
        pub worker: worker_store::StoredWorker,
        dir: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    impl Fixture {
        pub async fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("apas-worker-api-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("db.sqlite").to_string_lossy().into_owned();
            let db = Database::new(&path).await.unwrap();
            db.run_migrations().await.unwrap();
            let mut config = crate::config::Config::default();
            config.database.path = path;
            config.auth.jwt_secret = "worker-api-test-secret".into();
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
                .add_project_cluster_placement(
                    &project,
                    &owner.to_string(),
                    &owner.to_string(),
                    "test",
                )
                .await
                .unwrap();
            let time = Utc::now().to_rfc3339();
            let spec = WorkerMandateSpec {
                title: "Inspect repository health".into(),
                description: "Inspect the approved repository and report concrete findings".into(),
                checklist: vec!["Review maintained code".into()],
                machine_id: Uuid::new_v4(),
                worker_profile: "worker".into(),
                verification_profile: "verify".into(),
                model_profile: "model".into(),
                base_ref: "main".into(),
                interval_seconds: 900,
                limits: WorkerLimits {
                    per_run: JobLimits {
                        attempts: 1,
                        ..Default::default()
                    },
                    elapsed_seconds_24h: 86_400,
                    spend_micro_usd_24h: 100_000_000,
                    runs_24h: 24,
                    attempts_per_item: 1,
                },
            };
            let mut worker = worker_store::StoredWorker {
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
                        state_version: 0,
                        high_watermark: 0,
                        next_check_at: None,
                        active_job_id: None,
                        active_run_id: None,
                        current_item_id: None,
                        machine_id: spec.machine_id,
                        cluster_owner_user_id: owner,
                        hostname: "test-host".into(),
                        attention: false,
                        attention_run_id: None,
                        attention_question_id: None,
                        created_at: time.clone(),
                        updated_at: time.clone(),
                    },
                    mandate: WorkerMandate {
                        revision: 1,
                        spec,
                        repository: "owner/repo".into(),
                        worker_profile_digest: "worker-digest".into(),
                        verification_profile_digest: "verify-digest".into(),
                        policy_digest: "test".into(),
                        approved_by: owner,
                        approved_at: time.clone(),
                    },
                    checkpoint: None,
                    budget: WorkerBudget::default(),
                    allowed_actions: Default::default(),
                    current_job: None,
                },
                schedule_generation: 1,
                grant_active: true,
                stop_requested: false,
                accounting_at: time,
            };
            let mut tx = state.db.job_transaction().await.unwrap();
            worker_store::save(&mut tx, &worker).await.unwrap();
            worker_store::append(&mut tx, &mut worker, WorkerEventKind::Approved, "Approved")
                .await
                .unwrap();
            tx.commit().await.unwrap();
            Self { state, worker, dir }
        }

        pub fn owner(&self) -> Uuid {
            self.worker.detail.mandate.approved_by
        }
        pub fn id(&self) -> Uuid {
            self.worker.detail.worker.id
        }
        pub fn project(&self) -> String {
            self.worker.detail.worker.project_id.clone()
        }

        pub fn headers(&self, device: Option<&str>) -> HeaderMap {
            let token = jsonwebtoken::encode(
                &jsonwebtoken::Header::default(),
                &crate::routes::auth::Claims {
                    sub: self.owner().to_string(),
                    exp: (Utc::now() + Duration::hours(1)).timestamp() as usize,
                    device_session_id: device.map(str::to_owned),
                    token_kind: device.map(|_| "mobile_access".into()),
                    credential_version: None,
                },
                &jsonwebtoken::EncodingKey::from_secret(
                    self.state.config.auth.jwt_secret.as_bytes(),
                ),
            )
            .unwrap();
            let mut headers = HeaderMap::new();
            headers.insert(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {token}").parse().unwrap(),
            );
            headers
        }

        pub async fn device(&self) -> String {
            let id = Uuid::new_v4().to_string();
            self.state
                .db
                .create_mobile_device_session(
                    &id,
                    &self.owner().to_string(),
                    &Uuid::new_v4().to_string(),
                    "ios",
                    Some("test-phone"),
                    "test",
                    &format!("hash-{id}"),
                    &(Utc::now() + Duration::days(1)).to_rfc3339(),
                )
                .await
                .unwrap();
            id
        }

        pub async fn append(&mut self) {
            let mut tx = self.state.db.job_transaction().await.unwrap();
            worker_store::append(
                &mut tx,
                &mut self.worker,
                WorkerEventKind::StateChanged,
                "Updated",
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
        }

        async fn history(&mut self, label: &str) -> job_store::StoredJob {
            let job = Uuid::new_v4();
            let run = Uuid::new_v4();
            let reservation = Uuid::new_v4();
            let time = Utc::now().to_rfc3339();
            let limits = self.worker.detail.mandate.spec.limits.per_run.clone();
            let context = WorkerExecutionContext {
                worker_id: self.id(),
                mandate_revision: 1,
                backlog_revision: 0,
                schedule_generation: 1,
                reservation_id: reservation,
                purpose: WorkerRunPurpose::Inspection,
                mandate: self.worker.detail.mandate.spec.clone(),
                items: vec![],
                selected_item_id: None,
                checkpoint: None,
            };
            // Cancelled before launch: retained history does not assert a provider
            // completed work, or fabricate a successful result/stop report.
            let stored: job_store::StoredJob = serde_json::from_value(serde_json::json!({
                "initiator": self.owner(), "dispatched": false, "source_seq": 0,
                "lease_expires_at": null, "last_renewed_at": null,
                "review": {
                    "job_id": job, "run_id": run, "base_sha": label, "candidate_sha": null,
                    "files": [], "verification": null, "publication": null,
                    "reason": "cancelled_before_launch", "next_cursor": null
                },
                "detail": {
                    "job": {
                        "id": job, "project_id": self.project(), "project_name": "test", "title": label,
                        "state": "cancelled", "state_version": 1, "current_run_id": run,
                        "cluster_owner_user_id": self.owner(), "machine_id": context.mandate.machine_id,
                        "hostname": "test-host", "created_at": time, "updated_at": time,
                        "usage": JobUsage::default(), "limits": limits, "attention_count": 0,
                        "reason": "cancelled_before_launch", "allowed_actions": JobAllowedActions::default()
                    },
                    "task": {
                        "instruction": label, "acceptance_criteria": ["Inspect"], "source_url": null,
                        "repository": "owner/repo", "base_ref": "main", "base_sha": label,
                        "worker_profile": "worker", "worker_profile_digest": "worker-digest",
                        "verification_profile": "verify", "verification_profile_digest": "verify-digest",
                        "model_profile": "model", "worker": context
                    },
                    "runs": [{
                        "id": run, "job_id": job, "attempt": 1, "epoch": 1, "state": "cancelled",
                        "usage": JobUsage::default(), "started_at": null, "ended_at": time,
                        "worker_version": null, "candidate_sha": null, "stop_evidence": null,
                        "reason": "cancelled_before_launch"
                    }],
                    "questions": [], "high_watermark": 0, "policy_digest": "test", "publication": null
                }
            })).unwrap();
            let mut tx = self.state.db.job_transaction().await.unwrap();
            job_store::save(&mut tx, &stored).await.unwrap();
            worker_store::reserve(
                &mut tx,
                &mut self.worker,
                reservation,
                job,
                run,
                &limits,
                &time,
            )
            .await
            .unwrap();
            worker_store::reconcile_usage(
                &mut tx,
                &mut self.worker,
                reservation,
                &JobUsage {
                    spend_micro_usd: Some(0),
                    ..Default::default()
                },
                true,
                &time,
            )
            .await
            .unwrap();
            worker_store::release_execution(&mut tx, self.id(), job)
                .await
                .unwrap();
            tx.commit().await.unwrap();
            stored
        }
    }

    fn value<T>(result: Api<T>) -> T {
        match result {
            Ok(Json(value)) => value,
            Err(error) => panic!("unexpected API failure: {}", error.error),
        }
    }

    fn error<T>(result: Api<T>, code: &str) {
        match result {
            Err(error) => assert_eq!(error.error.to_string(), code),
            Ok(_) => panic!("expected {code}"),
        }
    }

    #[tokio::test]
    async fn historical_run_and_review_never_resolve_to_a_newer_execution() {
        let mut f = Fixture::new().await;
        let old = f.history("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").await;
        let newer = f.history("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").await;
        let path = (f.project(), f.id(), old.identity().run_id);
        let detail = value(run(State(f.state.clone()), f.headers(None), Path(path.clone())).await);
        assert_eq!(detail.job.id, old.detail.job.id);
        assert_ne!(detail.job.id, newer.detail.job.id);
        let review = value(
            super::review(
                State(f.state.clone()),
                f.headers(None),
                Path(path),
                Query(Page::default()),
            )
            .await,
        );
        assert_eq!(review.run_id, old.identity().run_id);
        assert_eq!(review.job_id, old.detail.job.id);
        assert_eq!(review.base_sha, old.detail.task.base_sha);
        for path in [
            (f.project(), f.id(), Uuid::new_v4()),
            (f.project(), Uuid::new_v4(), old.identity().run_id),
            (Uuid::new_v4().to_string(), f.id(), old.identity().run_id),
        ] {
            let result = run(State(f.state.clone()), f.headers(None), Path(path)).await;
            assert!(result.is_err());
        }
    }

    #[tokio::test]
    async fn artifact_and_question_mismatches_cannot_cross_execution_identity() {
        let mut f = Fixture::new().await;
        let mut old = f.history("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").await;
        let newer = f.history("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").await;
        let old_run = old.identity().run_id;
        let new_run = newer.identity().run_id;
        let question = Uuid::new_v4();
        old.detail.questions.push(JobQuestion {
            id: question,
            run_id: old_run,
            epoch: 1,
            version: 1,
            dialog_id: "historical-dialog".into(),
            tool_call_id: "historical-call".into(),
            state: JobQuestionState::Cancelled,
            questions: vec![],
            answers: None,
            created_at: old.detail.job.created_at.clone(),
            confirmed_at: None,
        });
        let artifact = JobArtifact {
            id: Uuid::new_v4(),
            run_id: old_run,
            kind: JobArtifactKind::WorkerLog,
            sha256: "test".into(),
            size_bytes: 8,
            content_type: "text/plain".into(),
            expires_at: None,
            truncated: false,
        };
        let mut tx = f.state.db.job_transaction().await.unwrap();
        job_store::save(&mut tx, &old).await.unwrap();
        sqlx::query("INSERT INTO autonomous_job_artifacts(id,job_id,run_id,manifest,content,complete) VALUES(?,?,?,?,?,1)")
            .bind(artifact.id.to_string()).bind(old.detail.job.id.to_string()).bind(old_run.to_string())
            .bind(serde_json::to_string(&artifact).unwrap()).bind(b"old logs".as_slice())
            .execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        let content = value(
            super::artifact(
                State(f.state.clone()),
                f.headers(None),
                Path((f.project(), f.id(), old_run, artifact.id)),
                Query(Page::default()),
            )
            .await,
        );
        assert_eq!(content.artifact.run_id, old_run);
        assert_eq!(content.text, "old logs");
        error(
            super::artifact(
                State(f.state.clone()),
                f.headers(None),
                Path((f.project(), f.id(), new_run, artifact.id)),
                Query(Page::default()),
            )
            .await,
            "artifact_not_found",
        );
        for (path_run, body_run, epoch, question_id) in [
            (old_run, new_run, 1, question),
            (old_run, old_run, 2, question),
            (new_run, new_run, 1, question),
            (old_run, old_run, 1, Uuid::new_v4()),
        ] {
            error(
                answer(
                    State(f.state.clone()),
                    f.headers(None),
                    Path((f.project(), f.id(), path_run, question_id)),
                    Json(JobAnswerRequest {
                        request_id: Uuid::new_v4(),
                        run_id: body_run,
                        epoch,
                        expected_version: 1,
                        answers: vec![],
                    }),
                )
                .await,
                "question_not_found",
            );
        }
        let retained = f.state.db.job(old.detail.job.id).await.unwrap().unwrap();
        assert_eq!(
            retained.detail.questions[0].state,
            JobQuestionState::Cancelled
        );
        assert!(retained.detail.questions[0].answers.is_none());
        let mut tx = f.state.db.job_transaction().await.unwrap();
        let intents: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM autonomous_job_answer_intents")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert_eq!(intents, 0);
        tx.commit().await.unwrap();
    }

    #[tokio::test]
    async fn event_pages_expose_retention_resync_and_preserve_the_snapshot_watermark() {
        let mut f = Fixture::new().await;
        f.append().await;
        f.append().await;
        let first = value(
            events(
                State(f.state.clone()),
                f.headers(None),
                Path((f.project(), f.id())),
                Query(Page {
                    after_seq: Some(0),
                    limit: Some(1),
                    ..Default::default()
                }),
            )
            .await,
        );
        assert_eq!(first.events.len(), 1);
        assert_eq!(first.events[0].seq, 1);
        assert!(first.has_more);
        assert!(!first.resync_required);
        assert_eq!(first.high_watermark, 3);
        let mut tx = f.state.db.job_transaction().await.unwrap();
        sqlx::query("DELETE FROM autonomous_worker_events WHERE worker_id=? AND seq<=1")
            .bind(f.id().to_string())
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let expired = value(
            events(
                State(f.state.clone()),
                f.headers(None),
                Path((f.project(), f.id())),
                Query(Page::default()),
            )
            .await,
        );
        assert!(expired.resync_required);
        assert_eq!(expired.high_watermark, 3);
        let retained = value(
            events(
                State(f.state.clone()),
                f.headers(None),
                Path((f.project(), f.id())),
                Query(Page {
                    after_seq: Some(1),
                    ..Default::default()
                }),
            )
            .await,
        );
        assert!(!retained.resync_required);
        assert!(!retained.has_more);
        assert_eq!(
            retained
                .events
                .iter()
                .map(|event| event.seq)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        let snapshot = value(
            detail(
                State(f.state.clone()),
                f.headers(None),
                Path((f.project(), f.id())),
            )
            .await,
        );
        assert_eq!(snapshot.worker.high_watermark, 3);
        assert_eq!(snapshot.mandate.revision, f.worker.detail.mandate.revision);
    }

    #[tokio::test]
    async fn http_reauthorizes_device_and_account_without_mutating_the_worker() {
        let f = Fixture::new().await;
        let device = f.device().await;
        let headers = f.headers(Some(&device));
        value(
            detail(
                State(f.state.clone()),
                headers.clone(),
                Path((f.project(), f.id())),
            )
            .await,
        );
        error(
            pause(
                State(f.state.clone()),
                headers.clone(),
                Path((f.project(), f.id())),
                Json(WorkerMutationRequest {
                    request_id: Uuid::new_v4(),
                    expected_version: 1,
                }),
            )
            .await,
            "mobile_coding_denied",
        );
        f.state
            .db
            .revoke_mobile_device_session(&f.owner().to_string(), &device, false, "test")
            .await
            .unwrap();
        error(
            detail(State(f.state.clone()), headers, Path((f.project(), f.id()))).await,
            "authentication_required",
        );
        value(
            detail(
                State(f.state.clone()),
                f.headers(None),
                Path((f.project(), f.id())),
            )
            .await,
        );
        f.state
            .db
            .update_cluster_user_status(
                crate::db::SYSTEM_ADMIN_ACTOR,
                &f.owner().to_string(),
                crate::db::AccountStatus::Suspended,
            )
            .await
            .unwrap();
        error(
            detail(
                State(f.state.clone()),
                f.headers(None),
                Path((f.project(), f.id())),
            )
            .await,
            "authentication_required",
        );
        let retained = f.state.db.worker(f.id()).await.unwrap().unwrap();
        assert!(retained.grant_active);
        assert_eq!(
            retained.detail.worker.state_version,
            f.worker.detail.worker.state_version
        );
        assert_eq!(
            retained.detail.worker.desired_state,
            WorkerDesiredState::Active
        );
    }
}
