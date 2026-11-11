use axum::{
    extract::{Path, State},
    http::HeaderMap,
    routing::get,
    Json, Router,
};
use shared::{JobPolicy, JobPolicyManagement, JobPolicyOverride};

use super::authz::{require_active_user, require_system_admin};
use crate::{db::SYSTEM_ADMIN_ACTOR, error::AppError, state::AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/job-policy", get(get_deployment).put(put_deployment))
        .route("/cluster/job-policy", get(get_cluster).put(put_cluster))
        .route(
            "/admin/projects/:project_id/job-policy",
            get(get_admin_project).put(put_admin_project),
        )
        .route(
            "/cluster/projects/:project_id/job-policy",
            get(get_operator_project).put(put_operator_project),
        )
        .route("/projects/:project_id/job-policy", get(get_project))
}

fn safe_readiness_reason(reason: &str) -> &'static str {
    // Never forward host-controlled diagnostics, credentials or private paths to management.
    // These categories describe a reported failure; they are not readiness attestations.
    let reason = reason.to_ascii_lowercase();
    if reason.contains("credential") {
        "job_credential_unavailable"
    } else if reason.contains("metering") {
        "job_metering_unavailable"
    } else if reason.contains("publisher") || reason.contains("protected base") {
        "job_publisher_unavailable"
    } else if reason.contains("verification") || reason.contains("verifier") {
        "job_verifier_unavailable"
    } else if reason.contains("image") {
        "job_image_unavailable"
    } else if reason.contains("cgroup") || reason.contains("quota") || reason.contains("resource") {
        "job_resource_isolation_unavailable"
    } else if reason.contains("network") || reason.contains("egress") {
        "job_network_isolation_unavailable"
    } else if reason.contains("storage")
        || reason.contains("directory")
        || reason.contains("filesystem")
    {
        "job_private_storage_unavailable"
    } else if reason.contains("podman") || reason.contains("rootless") || reason.contains("runroot")
    {
        "job_container_runtime_unavailable"
    } else if reason.contains("qualif") {
        "job_profile_qualification_required"
    } else {
        "job_worker_readiness_failed"
    }
}

async fn readiness_reasons(
    state: &AppState,
    actor: &str,
    scope: &str,
    id: &str,
) -> Result<Vec<String>, AppError> {
    let placements = if scope == "project" {
        Some(state.db.project_cluster_user_ids(id).await?)
    } else {
        None
    };
    let reports: Vec<_> = state
        .jobs
        .readiness
        .iter()
        .filter(|entry| {
            let owner = entry.cluster_owner_user_id.to_string();
            (actor == SYSTEM_ADMIN_ACTOR || owner == actor)
                && (scope != "cluster" || owner == id)
                && placements
                    .as_ref()
                    .is_none_or(|owners| owners.contains(&owner))
        })
        .map(|entry| entry.value().clone())
        .collect();
    if reports.is_empty() {
        return Ok(vec!["job_readiness_not_reported".into()]);
    }
    let now = chrono::Utc::now();
    let mut reasons = Vec::new();
    for report in reports {
        if report.protocol_version != shared::JOB_PROTOCOL_VERSION {
            reasons.push("job_worker_protocol_unsupported".into());
        }
        if !chrono::DateTime::parse_from_rfc3339(&report.checked_at).is_ok_and(|time| {
            time <= now
                && time > now - chrono::Duration::seconds(shared::JOB_READINESS_SECONDS as i64)
        }) || !chrono::DateTime::parse_from_rfc3339(&report.expires_at)
            .is_ok_and(|time| time > now)
        {
            reasons.push("job_readiness_expired".into());
        }
        if state.sessions.daemon_owner(&report.machine_id) != Some(report.cluster_owner_user_id)
            || !state.sessions.is_daemon_connected(&report.machine_id)
        {
            reasons.push("job_target_offline".into());
        }
        if !state
            .sessions
            .daemon_supports_capability(&report.machine_id, shared::OMP_JOB_WORKER_CAPABILITY)
        {
            reasons.push("job_worker_capability_missing".into());
        }
        if report.profiles.is_empty() {
            reasons.push("job_worker_profiles_unavailable".into());
        }
        for profile in report.profiles {
            if !profile.ready && profile.unavailable_reasons.is_empty() {
                reasons.push("job_worker_readiness_failed".into());
            }
            reasons.extend(
                profile
                    .unavailable_reasons
                    .iter()
                    .map(|reason| safe_readiness_reason(reason).to_string()),
            );
        }
    }
    Ok(reasons)
}

async fn view(
    state: &AppState,
    actor: &str,
    scope: &str,
    id: &str,
    can_manage: bool,
) -> Result<Json<JobPolicyManagement>, AppError> {
    let mut unavailable_reasons = Vec::new();
    let requested = match state.db.job_policy_requested(scope, id).await {
        Ok(value) => match value.map(serde_json::from_value).transpose() {
            Ok(value) => value,
            Err(_) => {
                unavailable_reasons.push("job_policy_invalid".into());
                None
            }
        },
        Err(error) if error.to_string().starts_with("job_policy_invalid:") => {
            unavailable_reasons.push("job_policy_invalid".into());
            None
        }
        Err(error) => return Err(error.into()),
    };
    let effective = match state.db.job_policy_scope(scope, id, true).await {
        Ok(value) => Some(value),
        Err(error) if error.to_string().starts_with("job_policy_invalid:") => {
            unavailable_reasons.push("job_policy_invalid".into());
            None
        }
        Err(error) => return Err(error.into()),
    };
    let inherited = if scope == "deployment" {
        None
    } else {
        match state.db.job_policy_scope(scope, id, false).await {
            Ok(value) => Some(value),
            Err(error) if error.to_string().starts_with("job_policy_invalid:") => {
                unavailable_reasons.push("job_policy_inherited_invalid".into());
                None
            }
            Err(error) => return Err(error.into()),
        }
    };
    if effective
        .as_ref()
        .is_some_and(|value| !value.policy.enabled)
    {
        unavailable_reasons.push("job_policy_disabled".into());
    }
    unavailable_reasons.extend(readiness_reasons(state, actor, scope, id).await?);
    unavailable_reasons.sort();
    unavailable_reasons.dedup();
    Ok(Json(JobPolicyManagement {
        requested,
        effective,
        inherited,
        unavailable_reasons,
        can_manage,
    }))
}

async fn update(
    state: &AppState,
    actor: &str,
    scope: &str,
    id: &str,
    value: serde_json::Value,
) -> Result<Json<JobPolicyManagement>, AppError> {
    state
        .db
        .set_job_policy(actor, scope, id, value)
        .await
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    // This scans durable jobs, not the first page of live projects or machines.
    crate::jobs::reconcile_authority(state).await?;
    view(state, actor, scope, id, true).await
}

async fn operator(
    headers: &HeaderMap,
    state: &AppState,
    project: &str,
) -> Result<String, AppError> {
    let user = require_active_user(headers, state).await?;
    if !state.db.project_in_user_cluster(project, &user.id).await? {
        return Err(AppError::Forbidden(
            "Hosting operator access required".into(),
        ));
    }
    Ok(user.id)
}

pub async fn get_deployment(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<JobPolicyManagement>, AppError> {
    require_system_admin(&headers, &state).await?;
    view(&state, SYSTEM_ADMIN_ACTOR, "deployment", "deployment", true).await
}

pub async fn put_deployment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(policy): Json<JobPolicy>,
) -> Result<Json<JobPolicyManagement>, AppError> {
    require_system_admin(&headers, &state).await?;
    update(
        &state,
        SYSTEM_ADMIN_ACTOR,
        "deployment",
        "deployment",
        serde_json::to_value(policy).map_err(anyhow::Error::from)?,
    )
    .await
}

pub async fn get_cluster(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<JobPolicyManagement>, AppError> {
    let user = require_active_user(&headers, &state).await?;
    view(&state, &user.id, "cluster", &user.id, true).await
}

pub async fn put_cluster(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(policy): Json<JobPolicyOverride>,
) -> Result<Json<JobPolicyManagement>, AppError> {
    let user = require_active_user(&headers, &state).await?;
    update(
        &state,
        &user.id,
        "cluster",
        &user.id,
        serde_json::to_value(policy).map_err(anyhow::Error::from)?,
    )
    .await
}

pub async fn get_admin_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project): Path<String>,
) -> Result<Json<JobPolicyManagement>, AppError> {
    require_system_admin(&headers, &state).await?;
    view(&state, SYSTEM_ADMIN_ACTOR, "project", &project, true).await
}

pub async fn put_admin_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Json(policy): Json<JobPolicyOverride>,
) -> Result<Json<JobPolicyManagement>, AppError> {
    require_system_admin(&headers, &state).await?;
    update(
        &state,
        SYSTEM_ADMIN_ACTOR,
        "project",
        &project,
        serde_json::to_value(policy).map_err(anyhow::Error::from)?,
    )
    .await
}

pub async fn get_operator_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project): Path<String>,
) -> Result<Json<JobPolicyManagement>, AppError> {
    let actor = operator(&headers, &state, &project).await?;
    view(&state, &actor, "project", &project, true).await
}

pub async fn put_operator_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Json(policy): Json<JobPolicyOverride>,
) -> Result<Json<JobPolicyManagement>, AppError> {
    let actor = operator(&headers, &state, &project).await?;
    update(
        &state,
        &actor,
        "project",
        &project,
        serde_json::to_value(policy).map_err(anyhow::Error::from)?,
    )
    .await
}

pub async fn get_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project): Path<String>,
) -> Result<Json<JobPolicyManagement>, AppError> {
    let user = require_active_user(&headers, &state).await?;
    if !state
        .db
        .has_project_content_access(&project, &user.id)
        .await?
    {
        return Err(AppError::Forbidden("Project access required".into()));
    }
    let can_manage = state.db.project_in_user_cluster(&project, &user.id).await?;
    view(&state, &user.id, "project", &project, can_manage).await
}
