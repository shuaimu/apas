//! Session sharing endpoints

use axum::{
    extract::{Path, State},
    http::header,
    Json,
};
use chrono::{Duration, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::{
    db::InvitationCode,
    error::AppError,
    routes::auth::verify_token,
    state::AppState,
};

const WEB_UI_URL: &str = "http://apas.mpaxos.com";

// Helper to extract and verify JWT from Authorization header
async fn extract_user_id(
    state: &AppState,
    auth_header: Option<&str>,
) -> Result<String, AppError> {
    let token = auth_header
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::AuthError("Missing or invalid Authorization header".to_string()))?;

    let claims = verify_token(token, &state.config.auth.jwt_secret)?;
    Ok(claims.sub)
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

    // Verify user owns the session
    let owner = state
        .db
        .get_session_owner(&req.session_id)
        .await?
        .ok_or_else(|| AppError::BadRequest("Session not found".to_string()))?;

    if owner != user_id {
        return Err(AppError::AuthError(
            "You can only share sessions you own".to_string(),
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
        created_by: user_id,
        expires_at: expires_at_str.clone(),
        redeemed_by: None,
        redeemed_at: None,
        created_at: None,
    };
    state.db.create_invitation_code(&invitation).await?;

    tracing::info!("Generated share code {} for session {}", code, req.session_id);

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

    // Create the share entry
    state
        .db
        .create_session_share(&invitation.session_id, &user_id, &invitation.created_by)
        .await?;

    // Delete the used invitation code (no longer needed)
    state
        .db
        .delete_invitation_code(&req.code)
        .await?;

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
    pub created_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ShareListResponse {
    pub owner: Option<ShareInfo>,
    pub shares: Vec<ShareInfo>,
}

/// List users who have access to a session (owner only)
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

    // Verify user owns the session
    let owner_id = state
        .db
        .get_session_owner(&session_id)
        .await?
        .ok_or_else(|| AppError::BadRequest("Session not found".to_string()))?;

    if owner_id != user_id {
        return Err(AppError::AuthError(
            "Only the session owner can view shares".to_string(),
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
            created_at: None,
        });

    // Get shares with user emails
    let share_rows = state
        .db
        .get_session_shares_with_emails(&session_id)
        .await?;

    let shares: Vec<ShareInfo> = share_rows
        .into_iter()
        .map(|(id, email, created_at)| ShareInfo {
            user_id: id,
            user_email: email,
            is_owner: false,
            created_at,
        })
        .collect();

    Ok(Json(ShareListResponse { owner: owner_info, shares }))
}

/// Revoke a user's access to a session (owner only)
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

    // Verify user owns the session
    let owner = state
        .db
        .get_session_owner(&session_id)
        .await?
        .ok_or_else(|| AppError::BadRequest("Session not found".to_string()))?;

    if owner != user_id {
        return Err(AppError::AuthError(
            "Only the session owner can revoke access".to_string(),
        ));
    }

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
