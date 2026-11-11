use crate::{jobs, state::AppState};
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
        .route("/projects/jobs", get(list))
        .route("/projects/jobs/:job", get(resolve))
        .route("/projects/job-requests/:request", get(receipt))
        .route("/projects/:project/job-targets", get(targets))
        .route("/projects/:project/job-previews", post(preview))
        .route("/projects/:project/jobs", get(project_list).post(admit))
        .route("/projects/:project/jobs/:job", get(detail))
        .route("/projects/:project/jobs/:job/runs", get(runs))
        .route("/projects/:project/jobs/:job/events", get(events))
        .route("/projects/:project/jobs/:job/review", get(review))
        .route(
            "/projects/:project/jobs/:job/artifacts/:artifact",
            get(artifact),
        )
        .route(
            "/projects/:project/jobs/:job/questions/:question/answers",
            post(answer),
        )
        .route("/projects/:project/jobs/:job/cancel", post(cancel))
        .route("/projects/:project/jobs/:job/retry", post(retry))
        .route(
            "/projects/:project/jobs/:job/retry-publication",
            post(retry_publication),
        )
}
struct JobError {
    error: anyhow::Error,
    request: Option<Uuid>,
}
impl From<anyhow::Error> for JobError {
    fn from(error: anyhow::Error) -> Self {
        Self {
            error,
            request: None,
        }
    }
}
impl IntoResponse for JobError {
    fn into_response(self) -> Response {
        let raw = self.error.to_string();
        let code = if raw.len() <= 80 && raw.bytes().all(|c| c.is_ascii_lowercase() || c == b'_') {
            raw.as_str()
        } else {
            "job_operation_failed"
        };
        let status = if code.contains("not_found") {
            StatusCode::NOT_FOUND
        } else if code.contains("denied") || code == "authentication_required" {
            StatusCode::FORBIDDEN
        } else if code.contains("unavailable") || code == "jobs_disabled" {
            StatusCode::SERVICE_UNAVAILABLE
        } else if code == "job_operation_failed" {
            StatusCode::INTERNAL_SERVER_ERROR
        } else {
            StatusCode::CONFLICT
        };
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::warn!(error=%self.error,"Job API request failed");
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
type Api<T> = Result<Json<T>, JobError>;
async fn actor(state: &AppState, headers: &HeaderMap, mutation: bool) -> Result<Uuid, JobError> {
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
    run_id: Option<Uuid>,
    limit: Option<usize>,
}
async fn list(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(p): Query<Page>,
) -> Api<JobListResponse> {
    let a = actor(&s, &h, false).await?;
    Ok(Json(
        jobs::list_jobs(&s, a, p.cursor.as_deref(), p.status.as_deref()).await?,
    ))
}
async fn project_list(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(project): Path<String>,
    Query(p): Query<Page>,
) -> Api<ProjectJobListResponse> {
    let a = actor(&s, &h, false).await?;
    Ok(Json(
        jobs::list_project_jobs(&s, a, &project, p.cursor.as_deref(), p.status.as_deref())
            .await?,
    ))
}
async fn resolve(State(s): State<AppState>, h: HeaderMap, Path(id): Path<Uuid>) -> Api<JobDetail> {
    let a = actor(&s, &h, false).await?;
    Ok(Json(jobs::detail_by_id(&s, a, id).await?))
}
async fn detail(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id)): Path<(String, Uuid)>,
) -> Api<JobDetail> {
    let a = actor(&s, &h, false).await?;
    Ok(Json(jobs::detail(&s, a, &p, id).await?))
}
async fn targets(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(p): Path<String>,
) -> Api<JobTargetsResponse> {
    let a = actor(&s, &h, false).await?;
    Ok(Json(jobs::targets(&s, a, &p).await?))
}
async fn preview(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(p): Path<String>,
    Json(r): Json<JobPreviewRequest>,
) -> Api<JobPreview> {
    let a = actor(&s, &h, true).await?;
    let _guard = s.project_operation_guard(&p).await;
    Ok(Json(jobs::preview(&s, a, &p, r).await?))
}
async fn admit(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(p): Path<String>,
    Json(r): Json<JobAdmissionRequest>,
) -> Result<(StatusCode, Json<JobMutationReceipt>), JobError> {
    let a = actor(&s, &h, true).await?;
    let _guard = s.project_operation_guard(&p).await;
    let request = r.request_id;
    let result = jobs::admit(&s, a, &p, r).await.map_err(|error| JobError {
        error,
        request: Some(request),
    })?;
    Ok((StatusCode::ACCEPTED, Json(result)))
}
async fn receipt(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Api<JobMutationReceipt> {
    let a = actor(&s, &h, false).await?;
    Ok(Json(jobs::receipt(&s, a, id).await?))
}
async fn runs(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id)): Path<(String, Uuid)>,
) -> Api<Vec<JobRun>> {
    let a = actor(&s, &h, false).await?;
    Ok(Json(jobs::detail(&s, a, &p, id).await?.runs))
}
async fn events(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id)): Path<(String, Uuid)>,
    Query(page): Query<Page>,
) -> Api<JobEventsResponse> {
    let a = actor(&s, &h, false).await?;
    jobs::detail(&s, a, &p, id).await?;
    Ok(Json(
        s.db.job_events(id, page.after_seq.unwrap_or(0)).await?,
    ))
}
async fn review(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id)): Path<(String, Uuid)>,
    Query(page): Query<Page>,
) -> Api<JobReview> {
    let a = actor(&s, &h, false).await?;
    let d = jobs::detail(&s, a, &p, id).await?;
    let run = page.run_id.unwrap_or(d.job.current_run_id);
    if !d.runs.iter().any(|r| r.id == run) {
        return Err(anyhow::anyhow!("run_not_found").into());
    }
    jobs::refresh_review(&s, a, id, run).await?;
    let mut review = s.db.job_review(id, run).await?.unwrap_or(JobReview {
        job_id: id,
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
    Path((p, id, artifact)): Path<(String, Uuid, Uuid)>,
    Query(page): Query<Page>,
) -> Api<JobArtifactContent> {
    let a = actor(&s, &h, false).await?;
    jobs::detail(&s, a, &p, id).await?;
    let offset = page
        .cursor
        .as_deref()
        .unwrap_or("0")
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid_cursor"))?;
    Ok(Json(
        s.db.job_artifact(id, artifact, offset, page.limit.unwrap_or(64 * 1024))
            .await?,
    ))
}
async fn answer(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id, q)): Path<(String, Uuid, Uuid)>,
    Json(r): Json<JobAnswerRequest>,
) -> Api<JobMutationReceipt> {
    let a = actor(&s, &h, true).await?;
    let request = r.request_id;
    let _guard = s.project_operation_guard(&p).await;
    Ok(Json(jobs::answer(&s, a, &p, id, q, r).await.map_err(
        |error| JobError {
            error,
            request: Some(request),
        },
    )?))
}
async fn mutation(
    s: AppState,
    h: HeaderMap,
    p: String,
    id: Uuid,
    r: JobMutationRequest,
    kind: JobMutationKind,
) -> Api<JobMutationReceipt> {
    let a = actor(&s, &h, true).await?;
    let request = r.request_id;
    let _guard = s.project_operation_guard(&p).await;
    Ok(Json(jobs::mutate(&s, a, &p, id, kind, r).await.map_err(
        |error| JobError {
            error,
            request: Some(request),
        },
    )?))
}
async fn cancel(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id)): Path<(String, Uuid)>,
    Json(r): Json<JobMutationRequest>,
) -> Api<JobMutationReceipt> {
    mutation(s, h, p, id, r, JobMutationKind::Cancel).await
}
async fn retry(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id)): Path<(String, Uuid)>,
    Json(r): Json<JobMutationRequest>,
) -> Api<JobMutationReceipt> {
    mutation(s, h, p, id, r, JobMutationKind::Retry).await
}
async fn retry_publication(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((p, id)): Path<(String, Uuid)>,
    Json(r): Json<JobMutationRequest>,
) -> Api<JobMutationReceipt> {
    mutation(s, h, p, id, r, JobMutationKind::RetryPublication).await
}
