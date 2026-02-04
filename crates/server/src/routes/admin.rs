use axum::{
    extract::State,
    http::HeaderMap,
    Json,
};
use serde::Serialize;

use crate::{error::AppError, state::AppState};
use super::auth::verify_token;

/// Admin email - only this user can access admin endpoints
const ADMIN_EMAIL: &str = "shuai@cs.stonybrook.edu";

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

/// Extract and verify admin user from Authorization header
async fn verify_admin(headers: &HeaderMap, state: &AppState) -> Result<(), AppError> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| AppError::AuthError("Missing Authorization header".to_string()))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::AuthError("Invalid Authorization header format".to_string()))?;

    let claims = verify_token(token, &state.config.auth.jwt_secret)?;

    // Get user email from database
    let user = state
        .db
        .get_user_by_id(&claims.sub)
        .await?
        .ok_or_else(|| AppError::AuthError("User not found".to_string()))?;

    // Check if user is admin
    if user.email != ADMIN_EMAIL {
        return Err(AppError::AuthError("Admin access required".to_string()));
    }

    Ok(())
}

/// GET /admin/stats - Get system statistics (admin only)
pub async fn get_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SystemStats>, AppError> {
    // Verify admin access
    verify_admin(&headers, &state).await?;

    // Gather statistics
    let total_users = state.db.get_user_count().await.unwrap_or(0);
    let recent_users_7d = state.db.get_recent_user_count().await.unwrap_or(0);
    let total_sessions = state.db.get_session_count().await.unwrap_or(0);
    let active_sessions_24h = state.db.get_active_session_count().await.unwrap_or(0);
    let total_cli_clients = state.db.get_cli_client_count().await.unwrap_or(0);
    let total_shares = state.db.get_share_count().await.unwrap_or(0);

    // Get online CLI clients from session manager
    let online_cli_clients = state.sessions.get_online_cli_ids().len();

    // Get recent users
    let recent_users_data = state.db.get_recent_users(10).await.unwrap_or_default();
    let recent_users: Vec<UserSummary> = recent_users_data
        .into_iter()
        .map(|(_, email, created_at)| UserSummary { email, created_at })
        .collect();

    // Get sessions per day
    let sessions_data = state.db.get_sessions_per_day(14).await.unwrap_or_default();
    let sessions_per_day: Vec<DailyStats> = sessions_data
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
