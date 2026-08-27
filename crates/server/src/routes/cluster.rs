//! Virtual-cluster self-service.
//!
//! Every active account owns one virtual cluster and may accept invitations to
//! other accounts' clusters. Projects are hosted according to durable cluster
//! placements; historical session rows are not treated as authorization.
//!
//! Every route resolves the active account, cluster role, project placement,
//! and project role independently as needed. Deployment-wide administration
//! stays on its separate system-credential surface.

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{Duration, NaiveDateTime, Utc};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::admin::{
    AddProjectMemberRequest, AdminProjectDetail, AdminProjectInventory, LifecycleRequest, Page,
    PageQuery, TransferOwnerRequest, UpdatePolicyRequest,
};
use super::authz::{
    require_active_user, require_cluster_access, require_cluster_owner,
    require_cluster_project_runtime_access, ClusterAccess,
};
use crate::{
    db::{
        ClusterDefaultPolicy, ClusterMembership, ClusterReference, ClusterUsageReport,
        ProjectLifecycle, SharedClusterInvitation, User,
    },
    error::AppError,
    state::AppState,
};

pub const SHARED_CLUSTER_TRUST_WARNING: &str = "Projects run on the cluster owner's machines. The cluster owner can access files, processes, terminal output, and credentials exposed to those processes. Only join a cluster whose owner you trust.";

#[derive(Debug, Serialize)]
pub struct InvitationView {
    pub id: String,
    pub cluster_owner_user_id: String,
    pub cluster_owner_email: String,
    pub invitee_user_id: String,
    pub invitee_email: String,
    pub expires_at: String,
    pub status: String,
    pub created_at: Option<String>,
}

impl From<SharedClusterInvitation> for InvitationView {
    fn from(invitation: SharedClusterInvitation) -> Self {
        let expired = NaiveDateTime::parse_from_str(&invitation.expires_at, "%Y-%m-%d %H:%M:%S")
            .map(|expires| expires.and_utc() <= Utc::now())
            .unwrap_or(false);
        let status = if invitation.revoked_at.is_some() {
            "revoked"
        } else if invitation.accepted_at.is_some() {
            "accepted"
        } else if expired {
            "expired"
        } else {
            "pending"
        };
        Self {
            id: invitation.id,
            cluster_owner_user_id: invitation.cluster_owner_user_id,
            cluster_owner_email: invitation.cluster_owner_email,
            invitee_user_id: invitation.invitee_user_id,
            invitee_email: invitation.invitee_email,
            expires_at: invitation.expires_at,
            status: status.to_string(),
            created_at: invitation.created_at,
        }
    }
}

fn invitation_token_hash(token: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes()))
}

#[derive(Debug, Deserialize)]
pub struct CreateSharedClusterInvitationRequest {
    pub email: String,
    #[serde(default)]
    pub trust_confirmed: bool,
    pub expires_in_hours: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CreateSharedClusterInvitationResponse {
    pub invitation: InvitationView,
    /// Returned only at creation time. The database retains only its digest.
    pub token: String,
    pub trust_warning: &'static str,
}

#[derive(Debug, Serialize)]
pub struct InvitationInspectionResponse {
    pub invitation: InvitationView,
    pub trust_warning: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct AcceptSharedClusterInvitationRequest {
    #[serde(default)]
    pub trust_confirmed: bool,
}

fn require_trust_confirmation(confirmed: bool) -> Result<(), AppError> {
    if !confirmed {
        return Err(AppError::BadRequest(
            "Confirm the shared-cluster trust warning before continuing".to_string(),
        ));
    }
    Ok(())
}

pub async fn list_clusters(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ClusterReference>>, AppError> {
    let user = require_active_user(&headers, &state).await?;
    Ok(Json(state.db.list_accessible_clusters(&user.id).await?))
}

pub async fn create_invitation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateSharedClusterInvitationRequest>,
) -> Result<Json<CreateSharedClusterInvitationResponse>, AppError> {
    let owner = require_active_user(&headers, &state).await?;
    require_cluster_owner(&headers, &state, &owner.id).await?;
    require_trust_confirmation(request.trust_confirmed)?;
    let email = request.email.trim().to_ascii_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Err(AppError::BadRequest(
            "A valid account email is required".to_string(),
        ));
    }
    let expires_in_hours = request.expires_in_hours.unwrap_or(168).clamp(1, 720);
    let expires_at = (Utc::now() + Duration::hours(expires_in_hours))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let mut token_bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut token_bytes);
    let token = URL_SAFE_NO_PAD.encode(token_bytes);
    let invitation = state
        .db
        .create_shared_cluster_invitation(
            &uuid::Uuid::new_v4().to_string(),
            &invitation_token_hash(&token),
            &owner.id,
            &email,
            &expires_at,
        )
        .await
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    Ok(Json(CreateSharedClusterInvitationResponse {
        invitation: invitation.into(),
        token,
        trust_warning: SHARED_CLUSTER_TRUST_WARNING,
    }))
}

pub async fn list_invitations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<InvitationView>>, AppError> {
    let owner = require_active_user(&headers, &state).await?;
    require_cluster_owner(&headers, &state, &owner.id).await?;
    Ok(Json(
        state
            .db
            .list_shared_cluster_invitations(&owner.id)
            .await?
            .into_iter()
            .map(InvitationView::from)
            .collect(),
    ))
}

pub async fn revoke_invitation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(invitation_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let owner = require_active_user(&headers, &state).await?;
    require_cluster_owner(&headers, &state, &owner.id).await?;
    let success = state
        .db
        .revoke_shared_cluster_invitation(&owner.id, &invitation_id)
        .await?;
    Ok(Json(serde_json::json!({ "success": success })))
}

pub async fn inspect_invitation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Result<Json<InvitationInspectionResponse>, AppError> {
    let user = require_active_user(&headers, &state).await?;
    let invitation = state
        .db
        .get_shared_cluster_invitation_by_hash(&invitation_token_hash(&token))
        .await?
        .filter(|invitation| invitation.invitee_user_id == user.id)
        .ok_or_else(|| AppError::NotFound("Invitation not found".to_string()))?;
    let owner_active = state
        .db
        .get_user_by_id(&invitation.cluster_owner_user_id)
        .await?
        .is_some_and(|owner| owner.is_active());
    if !owner_active {
        return Err(AppError::NotFound("Invitation not found".to_string()));
    }
    Ok(Json(InvitationInspectionResponse {
        invitation: invitation.into(),
        trust_warning: SHARED_CLUSTER_TRUST_WARNING,
    }))
}

pub async fn accept_invitation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Json(request): Json<AcceptSharedClusterInvitationRequest>,
) -> Result<Json<ClusterMembership>, AppError> {
    require_trust_confirmation(request.trust_confirmed)?;
    let user = require_active_user(&headers, &state).await?;
    let membership = state
        .db
        .accept_shared_cluster_invitation(&invitation_token_hash(&token), &user.id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound("Invitation is invalid or no longer available".to_string())
        })?;
    Ok(Json(membership))
}

pub async fn list_members(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ClusterMembership>>, AppError> {
    let owner = require_active_user(&headers, &state).await?;
    require_cluster_owner(&headers, &state, &owner.id).await?;
    Ok(Json(state.db.list_cluster_memberships(&owner.id).await?))
}

pub async fn revoke_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let owner = require_active_user(&headers, &state).await?;
    require_cluster_owner(&headers, &state, &owner.id).await?;
    let success = state
        .db
        .revoke_cluster_membership(&owner.id, &user_id)
        .await
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    if let Ok(member_id) = uuid::Uuid::parse_str(&user_id) {
        state
            .sessions
            .clear_shared_cluster_machine_access(&member_id);
        state
            .sessions
            .broadcast_machines_update_for_user(&member_id);
    }
    Ok(Json(serde_json::json!({ "success": success })))
}

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

#[derive(Debug, Serialize)]
pub struct ClusterContextOverview {
    pub cluster: ClusterReference,
    pub hosted_project_count: i64,
    pub visible_project_count: i64,
    pub active_session_count: i64,
    pub connected_project_count: i64,
    pub deployment_policy: shared::EffectiveProjectPolicy,
    pub cluster_policy: Option<ClusterDefaultPolicy>,
    pub trust_warning: Option<&'static str>,
}

async fn context_projects(
    state: &AppState,
    cluster_owner_user_id: &str,
    user: &User,
    access: ClusterAccess,
    search: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<crate::db::AdminProjectSummary>, AppError> {
    if access == ClusterAccess::Owner {
        Ok(state
            .db
            .list_cluster_projects(cluster_owner_user_id, search, limit, offset)
            .await?)
    } else {
        Ok(state
            .db
            .list_cluster_projects_for_member(
                cluster_owner_user_id,
                &user.id,
                search,
                limit,
                offset,
            )
            .await?)
    }
}

pub async fn context_overview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cluster_owner_user_id): Path<String>,
) -> Result<Json<ClusterContextOverview>, AppError> {
    let (user, access) = require_cluster_access(&headers, &state, &cluster_owner_user_id).await?;
    let visible =
        context_projects(&state, &cluster_owner_user_id, &user, access, None, 200, 0).await?;
    // Do not reveal the number of unrelated projects in a shared cluster.
    let hosted_project_count = visible.len() as i64;
    let owner = state
        .db
        .get_user_by_id(&cluster_owner_user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Cluster not found".to_string()))?;
    Ok(Json(ClusterContextOverview {
        cluster: ClusterReference {
            owner_user_id: owner.id,
            owner_email: owner.email,
            access: if access == ClusterAccess::Owner {
                "owner".to_string()
            } else {
                "member".to_string()
            },
            accepted_at: if access == ClusterAccess::Member {
                state
                    .db
                    .get_cluster_membership(&cluster_owner_user_id, &user.id)
                    .await?
                    .and_then(|membership| membership.accepted_at)
            } else {
                None
            },
        },
        hosted_project_count,
        visible_project_count: visible.len() as i64,
        active_session_count: visible
            .iter()
            .map(|project| project.active_session_count)
            .sum(),
        connected_project_count: visible
            .iter()
            .filter(|project| state.sessions.is_project_connected(&project.id))
            .count() as i64,
        deployment_policy: state.db.get_deployment_default_policy().await?,
        cluster_policy: state
            .db
            .get_cluster_default_policy(&cluster_owner_user_id)
            .await?,
        trust_warning: (access == ClusterAccess::Member).then_some(SHARED_CLUSTER_TRUST_WARNING),
    }))
}

pub async fn context_list_projects(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cluster_owner_user_id): Path<String>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Page<AdminProjectInventory>>, AppError> {
    let (user, access) = require_cluster_access(&headers, &state, &cluster_owner_user_id).await?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);
    let summaries = context_projects(
        &state,
        &cluster_owner_user_id,
        &user,
        access,
        query.search.as_deref(),
        limit,
        offset,
    )
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

pub async fn context_get_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((cluster_owner_user_id, project_id)): Path<(String, String)>,
) -> Result<Json<AdminProjectDetail>, AppError> {
    let (user, access) = require_cluster_project_runtime_access(
        &headers,
        &state,
        &cluster_owner_user_id,
        &project_id,
    )
    .await?;
    let project = context_projects(
        &state,
        &cluster_owner_user_id,
        &user,
        access,
        Some(&project_id),
        200,
        0,
    )
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

pub async fn context_get_default_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cluster_owner_user_id): Path<String>,
) -> Result<Json<ClusterPolicyResponse>, AppError> {
    require_cluster_access(&headers, &state, &cluster_owner_user_id).await?;
    Ok(Json(ClusterPolicyResponse {
        cluster: state
            .db
            .get_cluster_default_policy(&cluster_owner_user_id)
            .await?,
        deployment: state.db.get_deployment_default_policy().await?,
    }))
}

pub async fn cluster_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cluster_owner_user_id): Path<String>,
    Query(query): Query<PageQuery>,
) -> Result<Json<ClusterUsageReport>, AppError> {
    require_cluster_owner(&headers, &state, &cluster_owner_user_id).await?;
    Ok(Json(
        state
            .db
            .get_cluster_usage_report(
                &cluster_owner_user_id,
                query.limit.unwrap_or(50).clamp(1, 200),
                query.offset.unwrap_or(0).max(0),
            )
            .await?,
    ))
}

pub async fn own_cluster_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
) -> Result<Json<ClusterUsageReport>, AppError> {
    let owner = require_active_user(&headers, &state).await?;
    require_cluster_owner(&headers, &state, &owner.id).await?;
    Ok(Json(
        state
            .db
            .get_cluster_usage_report(
                &owner.id,
                query.limit.unwrap_or(50).clamp(1, 200),
                query.offset.unwrap_or(0).max(0),
            )
            .await?,
    ))
}

pub async fn project_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<shared::ProjectUsageStats>, AppError> {
    let user = require_active_user(&headers, &state).await?;
    if !state
        .db
        .has_project_content_access(&project_id, &user.id)
        .await?
    {
        return Err(AppError::Forbidden("Project access required".to_string()));
    }
    Ok(Json(
        state
            .db
            .get_project_usage_stats_by_project(&project_id)
            .await?,
    ))
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
