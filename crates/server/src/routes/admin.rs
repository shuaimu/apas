use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use chrono::{Duration, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};

use super::authz::require_cluster_admin;
use crate::{
    db::{
        AccountStatus, AdminProjectSummary, ClusterInvitation, ClusterRole, ClusterUserSummary,
        ProjectLifecycle, ProjectMemberInfo,
    },
    error::AppError,
    state::AppState,
};

const WEB_UI_URL: &str = "http://apas.mpaxos.com";

#[derive(Debug, Serialize)]
pub struct SystemStats {
    pub total_users: i64,
    pub recent_users_7d: i64,
    pub total_sessions: i64,
    pub active_sessions_24h: i64,
    pub total_cli_clients: i64,
    pub online_cli_clients: usize,
    pub total_shares: i64,
    pub recent_users: Vec<UserSummary>,
    pub sessions_per_day: Vec<DailyStats>,
}

#[derive(Debug, Serialize)]
pub struct UserSummary {
    pub email: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DailyStats {
    pub date: String,
    pub count: i64,
}

#[derive(Debug, Deserialize, Default)]
pub struct PageQuery {
    pub search: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub limit: i64,
    pub offset: i64,
}

pub async fn get_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SystemStats>, AppError> {
    require_cluster_admin(&headers, &state).await?;
    let total_users = state.db.get_user_count().await.unwrap_or(0);
    let recent_users_7d = state.db.get_recent_user_count().await.unwrap_or(0);
    let total_sessions = state.db.get_session_count().await.unwrap_or(0);
    let active_sessions_24h = state.db.get_active_session_count().await.unwrap_or(0);
    let total_cli_clients = state.db.get_cli_client_count().await.unwrap_or(0);
    let total_shares = state.db.get_share_count().await.unwrap_or(0);
    let online_cli_clients = state.sessions.get_online_cli_ids().len();
    let recent_users = state
        .db
        .get_recent_users(10)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(_, email, created_at)| UserSummary { email, created_at })
        .collect();
    let sessions_per_day = state
        .db
        .get_sessions_per_day(14)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(date, count)| DailyStats { date, count })
        .collect();
    Ok(Json(SystemStats {
        total_users,
        recent_users_7d,
        total_sessions,
        active_sessions_24h,
        total_cli_clients,
        online_cli_clients,
        total_shares,
        recent_users,
        sessions_per_day,
    }))
}

pub async fn list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
) -> Result<Json<Page<ClusterUserSummary>>, AppError> {
    require_cluster_admin(&headers, &state).await?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);
    let items = state
        .db
        .list_cluster_users(query.search.as_deref(), limit, offset)
        .await?;
    Ok(Json(Page {
        items,
        limit,
        offset,
    }))
}

#[derive(Debug, Deserialize)]
pub struct CreateInvitationRequest {
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct CreateInvitationResponse {
    pub code: String,
    pub email: String,
    pub expires_at: String,
    pub registration_url: String,
}

pub async fn invite_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateInvitationRequest>,
) -> Result<Json<CreateInvitationResponse>, AppError> {
    let actor = require_cluster_admin(&headers, &state).await?;
    let email = request.email.trim().to_ascii_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Err(AppError::BadRequest(
            "A valid email is required".to_string(),
        ));
    }
    if state.db.get_user_by_email(&email).await?.is_some() {
        return Err(AppError::BadRequest(
            "That email already belongs to a cluster account".to_string(),
        ));
    }
    let code = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(32)
        .map(char::from)
        .collect::<String>();
    let expires_at = (Utc::now() + Duration::hours(48)).to_rfc3339();
    state
        .db
        .create_cluster_invitation(&ClusterInvitation {
            code: code.clone(),
            email: email.clone(),
            created_by: actor.id.clone(),
            expires_at: expires_at.clone(),
            redeemed_at: None,
            created_at: None,
        })
        .await?;
    state
        .db
        .record_audit(
            &actor.id,
            "cluster_user.invited",
            "cluster_invitation",
            &email,
            None,
        )
        .await?;
    Ok(Json(CreateInvitationResponse {
        registration_url: format!("{WEB_UI_URL}/register?invitation={code}"),
        code,
        email,
        expires_at,
    }))
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub cluster_role: Option<String>,
    pub account_status: Option<String>,
}

pub async fn update_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(target_user_id): Path<String>,
    Json(request): Json<UpdateUserRequest>,
) -> Result<Json<ClusterUserSummary>, AppError> {
    let actor = require_cluster_admin(&headers, &state).await?;
    if let Some(raw) = request.cluster_role {
        let role = match raw.trim().to_ascii_lowercase().as_str() {
            "admin" => ClusterRole::Admin,
            "user" => ClusterRole::User,
            _ => return Err(AppError::BadRequest("Invalid cluster role".to_string())),
        };
        state
            .db
            .update_cluster_user_role(&actor.id, &target_user_id, role)
            .await
            .map_err(|error| AppError::BadRequest(error.to_string()))?;
    }
    if let Some(raw) = request.account_status {
        let status = match raw.trim().to_ascii_lowercase().as_str() {
            "active" => AccountStatus::Active,
            "suspended" => AccountStatus::Suspended,
            _ => return Err(AppError::BadRequest("Invalid account status".to_string())),
        };
        state
            .db
            .update_cluster_user_status(&actor.id, &target_user_id, status)
            .await
            .map_err(|error| AppError::BadRequest(error.to_string()))?;
        if status == AccountStatus::Suspended {
            state.sessions.disconnect_user(&target_user_id).await;
        }
    }
    let user = state
        .db
        .get_user_by_id(&target_user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Cluster account not found".to_string()))?;
    Ok(Json(ClusterUserSummary {
        id: user.id,
        email: user.email,
        cluster_role: user.cluster_role,
        account_status: user.account_status,
        created_at: user.created_at,
    }))
}

pub async fn list_projects(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
) -> Result<Json<Page<AdminProjectInventory>>, AppError> {
    require_cluster_admin(&headers, &state).await?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);
    let summaries = state
        .db
        .list_admin_projects(query.search.as_deref(), limit, offset)
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

#[derive(Debug, Serialize)]
pub struct AdminProjectInventory {
    #[serde(flatten)]
    pub project: AdminProjectSummary,
    pub effective_policy: shared::EffectiveProjectPolicy,
    pub connected: bool,
}

#[derive(Debug, Serialize)]
pub struct AdminProjectDetail {
    pub project: AdminProjectSummary,
    pub members: Vec<ProjectMemberInfo>,
    pub policy: shared::EffectiveProjectPolicy,
    pub policy_override: Option<crate::db::ProjectPolicyOverride>,
}

pub async fn get_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<AdminProjectDetail>, AppError> {
    require_cluster_admin(&headers, &state).await?;
    let project = state
        .db
        .list_admin_projects(Some(&project_id), 200, 0)
        .await?
        .into_iter()
        .find(|project| project.id == project_id)
        .ok_or_else(|| AppError::NotFound("Project not found".to_string()))?;
    let members = state.db.list_project_members(&project_id).await?;
    let policy = state.db.get_effective_project_policy(&project_id).await?;
    let policy_override = state.db.get_project_policy_override(&project_id).await?;
    Ok(Json(AdminProjectDetail {
        project,
        members,
        policy,
        policy_override,
    }))
}

#[derive(Debug, Deserialize)]
pub struct AddProjectMemberRequest {
    pub user_id: String,
}

pub async fn add_project_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(request): Json<AddProjectMemberRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let actor = require_cluster_admin(&headers, &state).await?;
    state
        .db
        .add_project_member(&actor.id, &project_id, &request.user_id)
        .await
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    Ok(Json(serde_json::json!({ "success": true })))
}

pub async fn remove_project_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((project_id, user_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let actor = require_cluster_admin(&headers, &state).await?;
    let removed = state
        .db
        .remove_project_member(&actor.id, &project_id, &user_id)
        .await?;
    Ok(Json(serde_json::json!({ "success": removed })))
}

#[derive(Debug, Deserialize)]
pub struct TransferOwnerRequest {
    pub user_id: String,
}

pub async fn transfer_owner(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(request): Json<TransferOwnerRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let actor = require_cluster_admin(&headers, &state).await?;
    let changed = state
        .db
        .transfer_project_ownership(&actor.id, &project_id, &request.user_id)
        .await
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    Ok(Json(serde_json::json!({ "success": changed })))
}

#[derive(Debug, Deserialize)]
pub struct LifecycleRequest {
    pub status: String,
}

pub async fn update_lifecycle(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(request): Json<LifecycleRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let actor = require_cluster_admin(&headers, &state).await?;
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
    let actor = require_cluster_admin(&headers, &state).await?;
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

#[derive(Debug, Deserialize)]
pub struct UpdatePolicyRequest {
    pub team_available: Option<bool>,
    pub allowed_launch_profiles: Option<Vec<String>>,
}

pub async fn update_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(request): Json<UpdatePolicyRequest>,
) -> Result<Json<shared::EffectiveProjectPolicy>, AppError> {
    let actor = require_cluster_admin(&headers, &state).await?;
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

pub async fn get_default_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<shared::EffectiveProjectPolicy>, AppError> {
    require_cluster_admin(&headers, &state).await?;
    Ok(Json(state.db.get_cluster_default_policy().await?))
}

pub async fn update_default_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<UpdatePolicyRequest>,
) -> Result<Json<shared::EffectiveProjectPolicy>, AppError> {
    let actor = require_cluster_admin(&headers, &state).await?;
    let current = state.db.get_cluster_default_policy().await?;
    let policy = state
        .db
        .set_cluster_default_policy(
            &actor.id,
            request.team_available.unwrap_or(current.team_available),
            request
                .allowed_launch_profiles
                .unwrap_or(current.allowed_launch_profiles),
        )
        .await
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    // Default changes can alter effective policy for every project without an
    // override, so recalculate before fanning out to live web/CLI peers.
    for project_id in state.db.list_project_ids().await? {
        let effective = state.db.get_effective_project_policy(&project_id).await?;
        state
            .sessions
            .broadcast_project_policy(&project_id, effective)
            .await;
    }
    Ok(Json(policy))
}

pub async fn list_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<shared::LaunchProfile>>, AppError> {
    require_cluster_admin(&headers, &state).await?;
    Ok(Json(shared::supported_launch_profiles()))
}

pub async fn list_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
) -> Result<Json<Page<crate::db::AdminAuditEvent>>, AppError> {
    require_cluster_admin(&headers, &state).await?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);
    let items = state.db.list_audit_events(limit, offset).await?;
    Ok(Json(Page {
        items,
        limit,
        offset,
    }))
}
