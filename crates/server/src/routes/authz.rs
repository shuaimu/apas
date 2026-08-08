use axum::http::{header, HeaderMap};

use crate::{
    db::{ClusterRole, User},
    error::AppError,
    routes::auth::verify_token,
    state::AppState,
};

pub(crate) async fn require_active_user(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<User, AppError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
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
    Ok(user)
}

pub(crate) async fn require_cluster_admin(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<User, AppError> {
    let user = require_active_user(headers, state).await?;
    if user.role() != ClusterRole::Admin {
        return Err(AppError::AuthError(
            "Cluster administrator access required".to_string(),
        ));
    }
    Ok(user)
}

pub(crate) async fn require_project_owner(
    headers: &HeaderMap,
    state: &AppState,
    project_id: &str,
) -> Result<User, AppError> {
    let user = require_active_user(headers, state).await?;
    let role = state
        .db
        .get_project_role_for_user(project_id, &user.id)
        .await?;
    if role.as_deref() != Some("owner") {
        return Err(AppError::Forbidden(
            "Project owner access required".to_string(),
        ));
    }
    Ok(user)
}

pub(crate) async fn require_project_member(
    headers: &HeaderMap,
    state: &AppState,
    project_id: &str,
) -> Result<User, AppError> {
    let user = require_active_user(headers, state).await?;
    if state
        .db
        .get_project_role_for_user(project_id, &user.id)
        .await?
        .is_none()
    {
        return Err(AppError::Forbidden(
            "Project membership required".to_string(),
        ));
    }
    Ok(user)
}
