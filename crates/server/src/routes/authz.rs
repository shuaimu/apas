use axum::http::{header, HeaderMap};

use crate::{
    db::{SystemAdminCredential, User},
    error::AppError,
    routes::auth::{require_active_claims, verify_token},
    routes::system_admin::{claims_are_system_admin, parse_bearer, SYSTEM_ADMIN_TOKEN_KIND},
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
    // A system-administration token authorizes nothing outside /admin. Reject
    // it explicitly rather than letting the account lookup fail with a
    // misleading "account not found".
    if claims_are_system_admin(&claims) {
        return Err(AppError::AuthError(
            "System administrator tokens do not grant account access".to_string(),
        ));
    }
    require_active_claims(state, &claims).await
}

/// The only gate on the system-administration surface. It authenticates a
/// credential, never an account: no `users` row is consulted, so no account
/// attribute can grant this. The credential version in the token must still
/// match the stored one, which is what makes a password rotation invalidate
/// every outstanding token.
pub(crate) async fn require_system_admin(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<SystemAdminCredential, AppError> {
    let claims = parse_bearer(headers, &state.config.auth.jwt_secret)?;
    if claims.token_kind.as_deref() != Some(SYSTEM_ADMIN_TOKEN_KIND) {
        return Err(AppError::AuthError(
            "System administrator sign-in required".to_string(),
        ));
    }
    let credential = state
        .db
        .get_system_admin_credential()
        .await?
        .ok_or_else(|| AppError::AuthError("No system administrator is configured".to_string()))?;
    if claims.credential_version != Some(credential.credential_version) {
        return Err(AppError::AuthError(
            "System administrator session has expired".to_string(),
        ));
    }
    Ok(credential)
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
