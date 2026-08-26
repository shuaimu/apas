//! Virtual-cluster self-service.
//!
//! Every active account operates exactly one virtual cluster: the machines its
//! clients registered, plus the projects hosted in it. A project is hosted in
//! an account's cluster when the account owns it or when at least one of its
//! sessions was created under that account — so a project owned by someone
//! else can still be administered here, and belonging to a project is not
//! enough to administer it.
//!
//! These routes carry no role check. The gate is `project_in_user_cluster`,
//! evaluated per request, and the same DB operations back the
//! system-administration surface so the two cannot drift.

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use serde::{Deserialize, Serialize};

use super::admin::{
    AddProjectMemberRequest, AdminProjectDetail, AdminProjectInventory, LifecycleRequest, Page,
    PageQuery, TransferOwnerRequest, UpdatePolicyRequest,
};
use super::authz::require_active_user;
use crate::{
    db::{ClusterDefaultPolicy, ProjectLifecycle, User},
    error::AppError,
    state::AppState,
};

#[derive(Debug, Serialize)]
pub struct ClusterOverview {
    pub user_id: String,
    pub email: String,
    pub hosted_project_count: i64,
    pub owned_project_count: i64,
    pub active_session_count: i64,
    pub connected_project_count: i64,
    /// The deployment default: the bound this cluster's own default must stay
    /// inside. Shown so an operator can see why a profile is unavailable.
    pub deployment_policy: shared::EffectiveProjectPolicy,
    pub cluster_policy: Option<ClusterDefaultPolicy>,
}

/// Authorize the caller as the operator of the cluster hosting `project_id`.
/// Denies with the same shape whether the project is absent or simply outside
/// the caller's cluster: a project id is not something to confirm to an
/// account that has no relationship with it.
async fn require_cluster_operator(
    headers: &HeaderMap,
    state: &AppState,
    project_id: &str,
) -> Result<User, AppError> {
    let user = require_active_user(headers, state).await?;
    if !state
        .db
        .project_in_user_cluster(project_id, &user.id)
        .await?
    {
        return Err(AppError::Forbidden(
            "That project is not in your cluster".to_string(),
        ));
    }
    Ok(user)
}

pub async fn overview(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ClusterOverview>, AppError> {
    let user = require_active_user(&headers, &state).await?;
    let projects = state
        .db
        .list_cluster_projects(&user.id, None, 200, 0)
        .await?;
    let connected_project_count = projects
        .iter()
        .filter(|project| state.sessions.is_project_connected(&project.id))
        .count() as i64;
    Ok(Json(ClusterOverview {
        owned_project_count: projects
            .iter()
            .filter(|project| project.owner_user_id == user.id)
            .count() as i64,
        hosted_project_count: projects.len() as i64,
        active_session_count: projects
            .iter()
            .map(|project| project.active_session_count)
            .sum(),
        connected_project_count,
        deployment_policy: state.db.get_deployment_default_policy().await?,
        cluster_policy: state.db.get_cluster_default_policy(&user.id).await?,
        user_id: user.id,
        email: user.email,
    }))
}

pub async fn list_projects(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
) -> Result<Json<Page<AdminProjectInventory>>, AppError> {
    let user = require_active_user(&headers, &state).await?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);
    let summaries = state
        .db
        .list_cluster_projects(&user.id, query.search.as_deref(), limit, offset)
        .await?;
    let mut items = Vec::with_capacity(summaries.len());
    for project in summaries {
        let effective_policy = state.db.get_effective_project_policy(&project.id).await?;
        let connected = state.sessions.is_project_connected(&project.id);
        items.push(AdminProjectInventory {
            project,
            effective_policy,
            connected,
        });
    }
    Ok(Json(Page {
        items,
        limit,
        offset,
    }))
}

pub async fn get_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<AdminProjectDetail>, AppError> {
    let user = require_cluster_operator(&headers, &state, &project_id).await?;
    let project = state
        .db
        .list_cluster_projects(&user.id, Some(&project_id), 200, 0)
        .await?
        .into_iter()
        .find(|project| project.id == project_id)
        .ok_or_else(|| AppError::NotFound("Project not found".to_string()))?;
    Ok(Json(AdminProjectDetail {
        members: state.db.list_project_members(&project_id).await?,
        policy: state.db.get_effective_project_policy(&project_id).await?,
        policy_override: state.db.get_project_policy_override(&project_id).await?,
        project,
    }))
}

pub async fn add_project_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(request): Json<AddProjectMemberRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user = require_cluster_operator(&headers, &state, &project_id).await?;
    state
        .db
        .add_project_member(&user.id, &project_id, &request.user_id)
        .await
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    Ok(Json(serde_json::json!({ "success": true })))
}

pub async fn remove_project_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_id, user_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let actor = require_cluster_operator(&headers, &state, &project_id).await?;
    let removed = state
        .db
        .remove_project_member(&actor.id, &project_id, &user_id)
        .await?;
    Ok(Json(serde_json::json!({ "success": removed })))
}

pub async fn transfer_owner(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(request): Json<TransferOwnerRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let actor = require_cluster_operator(&headers, &state, &project_id).await?;
    let changed = state
        .db
        .transfer_project_ownership(&actor.id, &project_id, &request.user_id)
        .await
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    Ok(Json(serde_json::json!({ "success": changed })))
}

pub async fn update_lifecycle(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(request): Json<LifecycleRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let actor = require_cluster_operator(&headers, &state, &project_id).await?;
    let lifecycle = match request.status.trim().to_ascii_lowercase().as_str() {
        "active" => ProjectLifecycle::Active,
        "suspended" => ProjectLifecycle::Suspended,
        _ => return Err(AppError::BadRequest("Invalid project status".to_string())),
    };
    let changed = state
        .db
        .set_project_lifecycle(&actor.id, &project_id, lifecycle)
        .await?;
    if lifecycle == ProjectLifecycle::Suspended {
        state.sessions.stop_project_runtime(&project_id).await;
    }
    Ok(Json(
        serde_json::json!({ "success": changed, "status": lifecycle.as_str() }),
    ))
}

pub async fn stop_runtime(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let actor = require_cluster_operator(&headers, &state, &project_id).await?;
    let stopped = state.sessions.stop_project_runtime(&project_id).await;
    state
        .db
        .record_audit(
            &actor.id,
            "project.runtime_stopped",
            "project",
            &project_id,
            Some(serde_json::json!({ "commands_sent": stopped })),
        )
        .await?;
    Ok(Json(
        serde_json::json!({ "success": true, "commands_sent": stopped }),
    ))
}

pub async fn update_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(request): Json<UpdatePolicyRequest>,
) -> Result<Json<shared::EffectiveProjectPolicy>, AppError> {
    let actor = require_cluster_operator(&headers, &state, &project_id).await?;
    let policy = state
        .db
        .set_project_policy_override(
            &actor.id,
            &project_id,
            request.team_available,
            request.allowed_launch_profiles,
        )
        .await
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    state
        .sessions
        .broadcast_project_policy(&project_id, policy.clone())
        .await;
    Ok(Json(policy))
}

pub async fn list_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<shared::LaunchProfile>>, AppError> {
    require_active_user(&headers, &state).await?;
    Ok(Json(shared::supported_launch_profiles()))
}

#[derive(Debug, Serialize)]
pub struct ClusterPolicyResponse {
    pub cluster: Option<ClusterDefaultPolicy>,
    /// The bound the cluster default must stay inside.
    pub deployment: shared::EffectiveProjectPolicy,
}

pub async fn get_default_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ClusterPolicyResponse>, AppError> {
    let user = require_active_user(&headers, &state).await?;
    Ok(Json(ClusterPolicyResponse {
        cluster: state.db.get_cluster_default_policy(&user.id).await?,
        deployment: state.db.get_deployment_default_policy().await?,
    }))
}

#[derive(Debug, Deserialize)]
pub struct UpdateClusterPolicyRequest {
    /// `None` means inherit the deployment default for that field.
    pub team_available: Option<bool>,
    pub allowed_launch_profiles: Option<Vec<String>>,
}

pub async fn update_default_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<UpdateClusterPolicyRequest>,
) -> Result<Json<ClusterPolicyResponse>, AppError> {
    let user = require_active_user(&headers, &state).await?;
    let cluster = state
        .db
        .set_cluster_default_policy(
            &user.id,
            &user.id,
            request.team_available,
            request.allowed_launch_profiles,
        )
        .await
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    // A cluster default changes the effective policy of every project hosted
    // here, including projects other accounts own, so recalculate before
    // fanning out to live peers.
    for project in state
        .db
        .list_cluster_projects(&user.id, None, 200, 0)
        .await?
    {
        let effective = state.db.get_effective_project_policy(&project.id).await?;
        state
            .sessions
            .broadcast_project_policy(&project.id, effective)
            .await;
    }
    Ok(Json(ClusterPolicyResponse {
        cluster,
        deployment: state.db.get_deployment_default_policy().await?,
    }))
}

pub async fn list_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
) -> Result<Json<Page<crate::db::AdminAuditEvent>>, AppError> {
    let user = require_active_user(&headers, &state).await?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);
    let items = state
        .db
        .list_cluster_audit_events(&user.id, limit, offset)
        .await?;
    Ok(Json(Page {
        items,
        limit,
        offset,
    }))
}
