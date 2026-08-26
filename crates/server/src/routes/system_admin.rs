//! The deployment's single system administrator.
//!
//! This is a credential, not an account: it lives outside the `users` table,
//! is seeded from server configuration, cannot be granted through any UI, and
//! its token authorizes nothing but `/admin/*`. Ordinary account tokens are
//! rejected here, and this token is rejected everywhere else.
//!
//! The login lives at `/admin/auth/login` rather than behind a page route
//! because nginx proxies the whole `/admin/` prefix to this server; a Next.js
//! page at `/admin/login` would never be reached. The web surface therefore
//! renders its login form inline at exactly `/admin`.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{extract::State, http::HeaderMap, Json};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    config::SystemAdminConfig,
    db::Database,
    error::AppError,
    routes::auth::{verify_token, Claims},
    state::AppState,
};

/// `sub` of every system-administration token. Not a user id, and not a valid
/// one: `require_active_claims` looks accounts up by id and would find nothing.
pub const SYSTEM_ADMIN_SUBJECT: &str = "system-admin";
pub const SYSTEM_ADMIN_TOKEN_KIND: &str = "system_admin";

/// Deliberately identical for an unknown username and a wrong password: this
/// is one well-known identity on an unauthenticated surface, so the response
/// must not confirm which half was wrong.
const INVALID_CREDENTIAL: &str = "Invalid system administrator credentials";

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub username: String,
    pub bootstrap_pending: bool,
    pub expires_in_seconds: i64,
}

#[derive(Debug, Serialize)]
pub struct IdentityResponse {
    pub username: String,
    pub bootstrap_pending: bool,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|err| AppError::Internal(err.to_string()))?
        .to_string())
}

/// Seed the credential at boot when the deployment has none. An existing
/// credential is never overwritten, so a rotated password cannot be reverted
/// by editing the config file.
pub async fn seed_system_admin(
    db: &Database,
    config: &SystemAdminConfig,
) -> Result<(), anyhow::Error> {
    if db.get_system_admin_credential().await?.is_some() {
        return Ok(());
    }
    if config.bootstrap_password.trim().is_empty() {
        tracing::warn!(
            "No system administrator is configured: set [system_admin] bootstrap_password \
             before starting the server, or /admin cannot be entered"
        );
        return Ok(());
    }
    let hash = hash_password(&config.bootstrap_password)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    if db
        .seed_system_admin_credential(config.username.trim(), &hash)
        .await?
    {
        tracing::info!(
            username = %config.username.trim(),
            "Seeded the system administrator credential; rotate it after the first sign-in"
        );
    }
    Ok(())
}

/// Bounded per-source lockout. Held in memory only — a restart forgives
/// attempts, which is acceptable for a throttle whose job is to make online
/// guessing slow rather than to be a durable ledger.
fn throttle_key(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn check_throttle(state: &AppState, key: &str) -> Result<(), AppError> {
    let config = &state.config.system_admin;
    if let Some(entry) = state.system_admin_auth_attempts.get(key) {
        let (failures, last) = *entry;
        if failures >= config.max_failed_attempts {
            let elapsed = Utc::now().signed_duration_since(last).num_seconds();
            if elapsed < config.lockout_seconds as i64 {
                return Err(AppError::AuthError(INVALID_CREDENTIAL.to_string()));
            }
        }
    }
    Ok(())
}

fn record_failure(state: &AppState, key: &str) {
    let config = &state.config.system_admin;
    let mut entry = state
        .system_admin_auth_attempts
        .entry(key.to_string())
        .or_insert((0, Utc::now()));
    let expired = Utc::now().signed_duration_since(entry.1).num_seconds()
        >= config.lockout_seconds as i64
        && entry.0 >= config.max_failed_attempts;
    entry.0 = if expired {
        1
    } else {
        entry.0.saturating_add(1)
    };
    entry.1 = Utc::now();
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let key = throttle_key(&headers);
    check_throttle(&state, &key)?;

    let Some(credential) = state.db.get_system_admin_credential().await? else {
        record_failure(&state, &key);
        return Err(AppError::AuthError(INVALID_CREDENTIAL.to_string()));
    };
    let username_matches = credential
        .username
        .eq_ignore_ascii_case(request.username.trim());
    let parsed = PasswordHash::new(&credential.password_hash)
        .map_err(|err| AppError::Internal(err.to_string()))?;
    let password_matches = Argon2::default()
        .verify_password(request.password.as_bytes(), &parsed)
        .is_ok();
    if !username_matches || !password_matches {
        record_failure(&state, &key);
        return Err(AppError::AuthError(INVALID_CREDENTIAL.to_string()));
    }
    state.system_admin_auth_attempts.remove(&key);

    let expiry_minutes = state.config.system_admin.token_expiry_minutes.max(1) as i64;
    let token = issue_token(&state, credential.credential_version, expiry_minutes)?;
    let bootstrap_pending = credential.bootstrap_pending();
    tracing::info!("System administrator signed in");
    Ok(Json(LoginResponse {
        token,
        username: credential.username,
        bootstrap_pending,
        expires_in_seconds: expiry_minutes * 60,
    }))
}

fn issue_token(
    state: &AppState,
    credential_version: i64,
    expiry_minutes: i64,
) -> Result<String, AppError> {
    let claims = Claims {
        sub: SYSTEM_ADMIN_SUBJECT.to_string(),
        exp: (Utc::now() + Duration::minutes(expiry_minutes)).timestamp() as usize,
        device_session_id: None,
        token_kind: Some(SYSTEM_ADMIN_TOKEN_KIND.to_string()),
        credential_version: Some(credential_version),
    };
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(state.config.auth.jwt_secret.as_bytes()),
    )
    .map_err(|err| AppError::Internal(err.to_string()))
}

pub async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<IdentityResponse>, AppError> {
    let credential = super::authz::require_system_admin(&headers, &state).await?;
    let bootstrap_pending = credential.bootstrap_pending();
    Ok(Json(IdentityResponse {
        username: credential.username,
        bootstrap_pending,
    }))
}

pub async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ChangePasswordRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let credential = super::authz::require_system_admin(&headers, &state).await?;
    let parsed = PasswordHash::new(&credential.password_hash)
        .map_err(|err| AppError::Internal(err.to_string()))?;
    Argon2::default()
        .verify_password(request.current_password.as_bytes(), &parsed)
        .map_err(|_| AppError::AuthError(INVALID_CREDENTIAL.to_string()))?;
    if request.new_password.chars().count() < 12 {
        return Err(AppError::BadRequest(
            "The system administrator password must be at least 12 characters".to_string(),
        ));
    }

    let hash = hash_password(&request.new_password)?;
    let next_version = state.db.rotate_system_admin_password(&hash).await?;
    state
        .db
        .record_system_admin_audit(
            "system.credential_rotated",
            "system_admin",
            "credential",
            None,
        )
        .await?;

    // Every token issued against the previous secret is now invalid, this
    // request's included, so hand back a fresh one rather than signing the
    // caller out of the page they are standing on.
    let expiry_minutes = state.config.system_admin.token_expiry_minutes.max(1) as i64;
    let token = issue_token(&state, next_version, expiry_minutes)?;
    tracing::info!("System administrator credential rotated");
    Ok(Json(LoginResponse {
        token,
        username: credential.username,
        bootstrap_pending: false,
        expires_in_seconds: expiry_minutes * 60,
    }))
}

/// Whether a bearer token is a system-administration token, without checking
/// whether it is still valid. Used by account-authenticated paths to reject it
/// with an accurate message instead of "account not found".
pub fn claims_are_system_admin(claims: &Claims) -> bool {
    claims.token_kind.as_deref() == Some(SYSTEM_ADMIN_TOKEN_KIND)
        || claims.sub == SYSTEM_ADMIN_SUBJECT
}

pub(crate) fn parse_bearer(headers: &HeaderMap, secret: &str) -> Result<Claims, AppError> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| {
            AppError::AuthError("Missing or invalid Authorization header".to_string())
        })?;
    verify_token(token, secret)
}
