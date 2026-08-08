//! Session sharing endpoints

use axum::{
    extract::{Path, State},
    http::header,
    Json,
};
use chrono::{Duration, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::{db::InvitationCode, error::AppError, routes::auth::verify_token, state::AppState};

const WEB_UI_URL: &str = "http://apas.mpaxos.com";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectRole {
    Owner,
    User,
}

impl ProjectRole {
    fn as_str(self) -> &'static str {
        match self {
            ProjectRole::Owner => "owner",
            ProjectRole::User => "user",
        }
    }

    pub(crate) fn can_manage_access(self) -> bool {
        matches!(self, ProjectRole::Owner)
    }
}

pub(crate) fn parse_share_role(raw: &str) -> ProjectRole {
    match raw.trim().to_ascii_lowercase().as_str() {
        "owner" => ProjectRole::Owner,
        _ => ProjectRole::User,
    }
}

fn parse_assignable_share_role(raw: &str) -> Result<ProjectRole, AppError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "user" => Ok(ProjectRole::User),
        "owner" => Err(AppError::BadRequest(
            "Role 'owner' is reserved for the project owner".to_string(),
        )),
        _ => Err(AppError::BadRequest(
            "Invalid project role. Expected 'user'".to_string(),
        )),
    }
}

// Helper to extract and verify JWT from Authorization header
async fn extract_user_id(state: &AppState, auth_header: Option<&str>) -> Result<String, AppError> {
    let token = auth_header
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(|| {
            AppError::AuthError("Missing or invalid Authorization header".to_string())
        })?;

    let claims = verify_token(token, &state.config.auth.jwt_secret)?;
    let user = state
        .db
        .get_user_by_id(&claims.sub)
        .await?
        .ok_or_else(|| AppError::AuthError("Cluster account not found".to_string()))?;
    if !user.is_active() {
        return Err(AppError::AuthError(
            "Cluster account is suspended".to_string(),
        ));
    }
    Ok(user.id)
}

async fn get_project_role_for_user(
    state: &AppState,
    session_id: &str,
    user_id: &str,
) -> Result<ProjectRole, AppError> {
    let role = state
        .db
        .get_session_role_for_user(session_id, user_id)
        .await?;
    role.map(|raw| parse_share_role(&raw))
        .ok_or_else(|| AppError::AuthError("You do not have access to this session".to_string()))
}

#[derive(Debug, Deserialize)]
pub struct GenerateCodeRequest {
    pub session_id: String,
}

#[derive(Debug, Serialize)]
pub struct GenerateCodeResponse {
    pub code: String,
    pub expires_at: String,
    pub share_url: String,
}

/// Generate an invitation code for sharing a session
/// POST /share/generate
pub async fn generate_code(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<GenerateCodeRequest>,
) -> Result<Json<GenerateCodeResponse>, AppError> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let user_id = extract_user_id(&state, auth_header).await?;
    let (_project_id, _project_guard) = state
        .active_session_operation(&req.session_id)
        .await
        .map_err(|_| AppError::Conflict("Project is unavailable".to_string()))?;

    let actor_role = get_project_role_for_user(&state, &req.session_id, &user_id).await?;
    if !actor_role.can_manage_access() {
        return Err(AppError::AuthError(
            "You do not have permission to share this session".to_string(),
        ));
    }

    // Generate 8-character alphanumeric code
    let code: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(8)
        .map(char::from)
        .collect::<String>()
        .to_uppercase();

    let expires_at = Utc::now() + Duration::hours(24);
    let expires_at_str = expires_at.to_rfc3339();

    // Store the invitation code
    let invitation = InvitationCode {
        code: code.clone(),
        session_id: req.session_id.clone(),
        project_id: None,
        created_by: user_id,
        expires_at: expires_at_str.clone(),
        redeemed_by: None,
        redeemed_at: None,
        created_at: None,
    };
    state.db.create_invitation_code(&invitation).await?;

    tracing::info!(
        "Generated share code {} for session {}",
        code,
        req.session_id
    );

    Ok(Json(GenerateCodeResponse {
        share_url: format!("{}/share?code={}", WEB_UI_URL, code),
        code,
        expires_at: expires_at_str,
    }))
}

#[derive(Debug, Deserialize)]
pub struct RedeemCodeRequest {
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct RedeemCodeResponse {
    pub success: bool,
    pub session_id: Option<String>,
    pub message: String,
}

/// Redeem an invitation code to get access to a session
/// POST /share/redeem
pub async fn redeem_code(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<RedeemCodeRequest>,
) -> Result<Json<RedeemCodeResponse>, AppError> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let user_id = extract_user_id(&state, auth_header).await?;

    // Look up the invitation code
    let invitation = state
        .db
        .get_invitation_code(&req.code)
        .await?
        .ok_or_else(|| AppError::BadRequest("Invalid invitation code".to_string()))?;
    let (_project_id, _project_guard) = state
        .active_session_operation(&invitation.session_id)
        .await
        .map_err(|_| AppError::Conflict("Project is unavailable".to_string()))?;

    // Check if already redeemed
    if invitation.redeemed_by.is_some() {
        return Ok(Json(RedeemCodeResponse {
            success: false,
            session_id: None,
            message: "This invitation code has already been used".to_string(),
        }));
    }

    // Check if expired
    let expires_at = chrono::DateTime::parse_from_rfc3339(&invitation.expires_at)
        .map_err(|_| AppError::Internal("Invalid expiration date".to_string()))?;
    if Utc::now() > expires_at {
        return Ok(Json(RedeemCodeResponse {
            success: false,
            session_id: None,
            message: "This invitation code has expired".to_string(),
        }));
    }

    // Check if user already owns or has access to this session
    let has_access = state
        .db
        .check_session_access(&invitation.session_id, &user_id)
        .await?;
    if has_access {
        return Ok(Json(RedeemCodeResponse {
            success: false,
            session_id: Some(invitation.session_id),
            message: "You already have access to this session".to_string(),
        }));
    }

    if !state
        .db
        .redeem_project_invitation(&req.code, &user_id)
        .await?
    {
        return Ok(Json(RedeemCodeResponse {
            success: false,
            session_id: None,
            message: "This invitation is no longer valid for an active project".to_string(),
        }));
    }

    tracing::info!(
        "User {} redeemed share code {} for session {}",
        user_id,
        req.code,
        invitation.session_id
    );

    Ok(Json(RedeemCodeResponse {
        success: true,
        session_id: Some(invitation.session_id),
        message: "Session shared with you successfully".to_string(),
    }))
}

#[derive(Debug, Serialize)]
pub struct ShareInfo {
    pub user_id: String,
    pub user_email: String,
    pub is_owner: bool,
    pub role: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ShareListResponse {
    pub owner: Option<ShareInfo>,
    pub shares: Vec<ShareInfo>,
    pub viewer_role: String,
    pub can_manage: bool,
}

/// List users who have access to a session (owner/admin)
/// GET /share/list/:session_id
pub async fn list_shares(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<ShareListResponse>, AppError> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let user_id = extract_user_id(&state, auth_header).await?;
    let (_project_id, _project_guard) = state
        .active_session_operation(&session_id)
        .await
        .map_err(|_| AppError::Conflict("Project is unavailable".to_string()))?;

    let actor_role = get_project_role_for_user(&state, &session_id, &user_id).await?;
    if !actor_role.can_manage_access() {
        return Err(AppError::AuthError(
            "You do not have permission to view shares".to_string(),
        ));
    }

    // Get owner info
    let owner_info = state
        .db
        .get_session_owner_info(&session_id)
        .await?
        .map(|(id, email)| ShareInfo {
            user_id: id,
            user_email: email,
            is_owner: true,
            role: "owner".to_string(),
            created_at: None,
        });

    // Get shares with user emails
    let share_rows = state.db.get_session_shares_with_emails(&session_id).await?;

    let shares: Vec<ShareInfo> = share_rows
        .into_iter()
        .map(|(id, email, created_at, role)| ShareInfo {
            user_id: id,
            user_email: email,
            is_owner: false,
            role,
            created_at,
        })
        .collect();

    Ok(Json(ShareListResponse {
        owner: owner_info,
        shares,
        viewer_role: actor_role.as_str().to_string(),
        can_manage: actor_role.can_manage_access(),
    }))
}

/// Revoke a user's access to a session (owner/admin)
/// DELETE /share/:session_id/:user_id
pub async fn revoke_access(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((session_id, target_user_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let user_id = extract_user_id(&state, auth_header).await?;
    let (_project_id, _project_guard) = state
        .active_session_operation(&session_id)
        .await
        .map_err(|_| AppError::Conflict("Project is unavailable".to_string()))?;

    let actor_role = get_project_role_for_user(&state, &session_id, &user_id).await?;
    if !actor_role.can_manage_access() {
        return Err(AppError::AuthError(
            "You do not have permission to revoke access".to_string(),
        ));
    }

    let owner = state
        .db
        .get_session_owner(&session_id)
        .await?
        .ok_or_else(|| AppError::BadRequest("Session not found".to_string()))?;
    if owner == target_user_id {
        return Err(AppError::BadRequest(
            "Cannot revoke access for the project owner".to_string(),
        ));
    }

    let target_role = state
        .db
        .get_session_share_role(&session_id, &target_user_id)
        .await?;
    let Some(_target_role_raw) = target_role else {
        return Ok(Json(serde_json::json!({
            "success": false,
            "message": "Share not found"
        })));
    };
    // Delete the share
    let deleted = state
        .db
        .delete_session_share(&session_id, &target_user_id)
        .await?;

    if deleted {
        tracing::info!(
            "User {} revoked access for {} to session {}",
            user_id,
            target_user_id,
            session_id
        );
        Ok(Json(serde_json::json!({ "success": true })))
    } else {
        Ok(Json(serde_json::json!({
            "success": false,
            "message": "Share not found"
        })))
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateShareRoleRequest {
    pub role: String,
}

/// Update a user's access role for a session (owner/admin)
/// PATCH /share/:session_id/:user_id/role
pub async fn update_share_role(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((session_id, target_user_id)): Path<(String, String)>,
    Json(req): Json<UpdateShareRoleRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let user_id = extract_user_id(&state, auth_header).await?;
    let (_project_id, _project_guard) = state
        .active_session_operation(&session_id)
        .await
        .map_err(|_| AppError::Conflict("Project is unavailable".to_string()))?;

    let actor_role = get_project_role_for_user(&state, &session_id, &user_id).await?;
    if !actor_role.can_manage_access() {
        return Err(AppError::AuthError(
            "You do not have permission to update roles".to_string(),
        ));
    }

    let desired_role = parse_assignable_share_role(&req.role)?;
    let owner = state
        .db
        .get_session_owner(&session_id)
        .await?
        .ok_or_else(|| AppError::BadRequest("Session not found".to_string()))?;
    if owner == target_user_id {
        return Err(AppError::BadRequest(
            "Cannot change role for the project owner".to_string(),
        ));
    }

    let target_role = state
        .db
        .get_session_share_role(&session_id, &target_user_id)
        .await?;
    let Some(_target_role_raw) = target_role else {
        return Ok(Json(serde_json::json!({
            "success": false,
            "message": "Share not found"
        })));
    };
    let _ = state
        .db
        .update_session_share_role(&session_id, &target_user_id, desired_role.as_str())
        .await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "role": desired_role.as_str(),
    })))
}
