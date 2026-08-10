use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{Duration, Utc};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use shared::{
    MobileAuthResponse, MobileDeviceSession, MobileLoginRequest, MobileLogoutRequest,
    MobilePlatform, MobileRefreshRequest,
};
use uuid::Uuid;

use crate::{
    db::{ClusterRole, MobileDeviceSessionRecord, MobileRefreshFailure},
    error::AppError,
    mobile_metrics::{MobileMetric, MobileMetrics},
    routes::{
        auth::{generate_mobile_access_token, require_active_claims, verify_token, Claims},
        authz::require_active_user,
    },
    state::AppState,
};
use std::sync::Arc;

type HmacSha256 = Hmac<Sha256>;
const MOBILE_AUTH_ATTEMPTS_PER_MINUTE: usize = 10;

struct AuthAttemptMetric {
    metrics: Arc<MobileMetrics>,
    completed: bool,
}

impl AuthAttemptMetric {
    fn new(metrics: Arc<MobileMetrics>) -> Self {
        Self {
            metrics,
            completed: false,
        }
    }

    fn success(&mut self, metric: MobileMetric) {
        self.metrics.increment(metric);
        self.completed = true;
    }
}

impl Drop for AuthAttemptMetric {
    fn drop(&mut self) {
        if !self.completed {
            self.metrics.increment(MobileMetric::AuthFailure);
        }
    }
}

fn require_secure_request(headers: &HeaderMap, state: &AppState) -> Result<(), AppError> {
    let forwarded_https = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .next()
                .is_some_and(|part| part.trim() == "https")
        });
    if forwarded_https {
        return Ok(());
    }
    let local_host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|host| {
            let host = host.split(':').next().unwrap_or(host);
            matches!(host, "localhost" | "127.0.0.1" | "::1")
        });
    if state.config.mobile.allow_insecure_localhost && local_host {
        return Ok(());
    }
    Err(AppError::BadRequest(
        "Mobile authentication requires HTTPS".to_string(),
    ))
}

fn rate_limit_key(headers: &HeaderMap, installation_id: &str) -> String {
    let address = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");
    format!("{address}:{installation_id}")
}

fn check_rate_limit(state: &AppState, key: String) -> Result<(), AppError> {
    let now = Utc::now();
    let cutoff = now - Duration::minutes(1);
    let mut attempts = state.mobile_auth_attempts.entry(key).or_default();
    attempts.retain(|attempt| *attempt > cutoff);
    if attempts.len() >= MOBILE_AUTH_ATTEMPTS_PER_MINUTE {
        return Err(AppError::Forbidden(
            "Too many mobile authentication attempts; retry later".to_string(),
        ));
    }
    attempts.push(now);
    Ok(())
}

fn validate_installation_id(value: &str) -> Result<(), AppError> {
    if value.is_empty()
        || value.len() > 200
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(AppError::BadRequest(
            "Invalid installation identifier".to_string(),
        ));
    }
    Ok(())
}

fn new_refresh_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn hash_refresh_token(token: &str, secret: &str) -> Result<String, AppError> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| AppError::Internal("Invalid refresh-token key".to_string()))?;
    mac.update(token.as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

pub(crate) fn claims_from_headers(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<Claims, AppError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| {
            AppError::AuthError("Missing or invalid Authorization header".to_string())
        })?;
    verify_token(token, &state.config.auth.jwt_secret)
}

fn platform_name(platform: MobilePlatform) -> &'static str {
    match platform {
        MobilePlatform::Ios => "ios",
        MobilePlatform::Android => "android",
    }
}

fn public_device(record: MobileDeviceSessionRecord) -> MobileDeviceSession {
    MobileDeviceSession {
        id: Uuid::parse_str(&record.id).unwrap_or_else(|_| Uuid::nil()),
        installation_id: record.installation_id,
        platform: if record.platform == "ios" {
            MobilePlatform::Ios
        } else {
            MobilePlatform::Android
        },
        device_name: record.device_name,
        app_version: record.app_version,
        created_at: record.created_at,
        last_used_at: record.last_used_at,
        expires_at: record.expires_at,
        revoked_at: record.revoked_at,
    }
}

fn auth_response(
    state: &AppState,
    record: &MobileDeviceSessionRecord,
    user: crate::db::User,
    refresh_token: String,
) -> Result<MobileAuthResponse, AppError> {
    let (access_token, access_expires_at) =
        generate_mobile_access_token(&user.id, &record.id, &state.config.auth)?;
    Ok(MobileAuthResponse {
        access_token,
        access_expires_at: access_expires_at.to_rfc3339(),
        refresh_token,
        refresh_expires_at: record.expires_at.clone(),
        device_session_id: Uuid::parse_str(&record.id)
            .map_err(|_| AppError::Internal("Invalid device session identifier".to_string()))?,
        user_id: Uuid::parse_str(&user.id)
            .map_err(|_| AppError::Internal("Invalid user identifier".to_string()))?,
        user_email: user.email,
        cluster_role: user.cluster_role,
    })
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MobileLoginRequest>,
) -> Result<Json<MobileAuthResponse>, AppError> {
    let mut metric = AuthAttemptMetric::new(state.mobile_metrics.clone());
    require_secure_request(&headers, &state)?;
    validate_installation_id(&request.installation_id)?;
    check_rate_limit(&state, rate_limit_key(&headers, &request.installation_id))?;
    if request.app_version.trim().is_empty() || request.app_version.len() > 100 {
        return Err(AppError::BadRequest(
            "Invalid application version".to_string(),
        ));
    }
    let user = state
        .db
        .get_user_by_email(request.email.trim())
        .await?
        .ok_or_else(|| AppError::AuthError("Invalid email or password".to_string()))?;
    let parsed_hash = PasswordHash::new(&user.password_hash)
        .map_err(|error| AppError::Internal(error.to_string()))?;
    Argon2::default()
        .verify_password(request.password.as_bytes(), &parsed_hash)
        .map_err(|_| AppError::AuthError("Invalid email or password".to_string()))?;
    if !user.is_active() {
        return Err(AppError::AuthError(
            "Cluster account is suspended".to_string(),
        ));
    }

    let refresh_token = new_refresh_token();
    let refresh_hash = hash_refresh_token(&refresh_token, &state.config.auth.jwt_secret)?;
    let session_id = Uuid::new_v4();
    let refresh_expires_at =
        Utc::now() + Duration::days(state.config.auth.mobile_refresh_expiry_days as i64);
    state
        .db
        .create_mobile_device_session(
            &session_id.to_string(),
            &user.id,
            &request.installation_id,
            platform_name(request.platform),
            request.device_name.as_deref(),
            request.app_version.trim(),
            &refresh_hash,
            &refresh_expires_at.to_rfc3339(),
        )
        .await
        .map_err(|error| AppError::Conflict(error.to_string()))?;
    state
        .db
        .record_audit(
            &user.id,
            "mobile.device_session_created",
            "mobile_device_session",
            &session_id.to_string(),
            Some(serde_json::json!({
                "platform": platform_name(request.platform),
                "app_version": request.app_version,
            })),
        )
        .await?;
    let record = state
        .db
        .get_mobile_device_session(&session_id.to_string())
        .await?
        .ok_or_else(|| AppError::Internal("Device session was not persisted".to_string()))?;
    let response = auth_response(&state, &record, user, refresh_token)?;
    metric.success(MobileMetric::AuthLoginSuccess);
    tracing::info!(
        platform = platform_name(request.platform),
        app_version = request.app_version.trim(),
        "mobile authentication succeeded"
    );
    Ok(Json(response))
}

pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MobileRefreshRequest>,
) -> Result<Json<MobileAuthResponse>, AppError> {
    let mut metric = AuthAttemptMetric::new(state.mobile_metrics.clone());
    require_secure_request(&headers, &state)?;
    validate_installation_id(&request.installation_id)?;
    check_rate_limit(&state, rate_limit_key(&headers, &request.installation_id))?;
    let current_hash = hash_refresh_token(&request.refresh_token, &state.config.auth.jwt_secret)?;
    let next_refresh_token = new_refresh_token();
    let next_hash = hash_refresh_token(&next_refresh_token, &state.config.auth.jwt_secret)?;
    let record = match state
        .db
        .rotate_mobile_refresh_token(&current_hash, &request.installation_id, &next_hash)
        .await?
    {
        Ok(record) => record,
        Err(failure) => {
            let reason = match failure {
                MobileRefreshFailure::Invalid => "Invalid refresh token",
                MobileRefreshFailure::Expired => "Mobile device session expired",
                MobileRefreshFailure::Revoked => "Mobile device session revoked",
                MobileRefreshFailure::Reused => {
                    "Refresh token reuse detected; device session revoked"
                }
                MobileRefreshFailure::InstallationMismatch => {
                    "Refresh token belongs to another installation; device session revoked"
                }
            };
            return Err(AppError::AuthError(reason.to_string()));
        }
    };
    let user = state
        .db
        .get_user_by_id(&record.user_id)
        .await?
        .ok_or_else(|| AppError::AuthError("Cluster account not found".to_string()))?;
    if !user.is_active() {
        state
            .db
            .revoke_mobile_device_sessions_for_user(&user.id, "account_suspended")
            .await?;
        return Err(AppError::AuthError(
            "Cluster account is suspended".to_string(),
        ));
    }
    let response = auth_response(&state, &record, user, next_refresh_token)?;
    metric.success(MobileMetric::AuthRefreshSuccess);
    tracing::info!(
        app_version = record.app_version,
        "mobile access token refreshed"
    );
    Ok(Json(response))
}

pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MobileLogoutRequest>,
) -> Result<StatusCode, AppError> {
    require_secure_request(&headers, &state)?;
    let claims = claims_from_headers(&headers, &state)?;
    require_active_claims(&state, &claims).await?;
    let session_id = claims
        .device_session_id
        .ok_or_else(|| AppError::Forbidden("Mobile access token required".to_string()))?;
    let refresh_hash = hash_refresh_token(&request.refresh_token, &state.config.auth.jwt_secret)?;
    if !state
        .db
        .mobile_refresh_token_matches(&session_id, &refresh_hash)
        .await?
    {
        return Err(AppError::AuthError("Invalid refresh token".to_string()));
    }
    state
        .db
        .revoke_mobile_device_session(&claims.sub, &session_id, false, "logout")
        .await?;
    state
        .mobile_metrics
        .increment(MobileMetric::DeviceRevocation);
    tracing::info!(reason = "logout", "mobile device session revoked");
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_devices(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<MobileDeviceSession>>, AppError> {
    let user = require_active_user(&headers, &state).await?;
    let devices = state
        .db
        .list_mobile_device_sessions(&user.id)
        .await?
        .into_iter()
        .map(public_device)
        .collect();
    Ok(Json(devices))
}

pub async fn revoke_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_session_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let user = require_active_user(&headers, &state).await?;
    let revoked = state
        .db
        .revoke_mobile_device_session(
            &user.id,
            &device_session_id,
            user.role() == ClusterRole::Admin,
            "explicit_revocation",
        )
        .await
        .map_err(|error| AppError::Forbidden(error.to_string()))?;
    if !revoked {
        return Err(AppError::NotFound(
            "Mobile device session not found".to_string(),
        ));
    }
    state
        .mobile_metrics
        .increment(MobileMetric::DeviceRevocation);
    tracing::info!(reason = "explicit", "mobile device session revoked");
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};
    use axum::http::HeaderValue;

    async fn state_with_user() -> AppState {
        let dir = std::env::temp_dir().join(format!("apas-mobile-auth-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::Database::new(&dir.join("apas.db").to_string_lossy())
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let salt = SaltString::generate(&mut OsRng);
        let password_hash = Argon2::default()
            .hash_password(b"correct horse battery staple", &salt)
            .unwrap()
            .to_string();
        db.create_user(&crate::db::User {
            id: Uuid::new_v4().to_string(),
            email: "mobile@example.test".to_string(),
            password_hash,
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .unwrap();
        let mut config = crate::config::Config::default();
        config.database.path = dir.join("apas.db").to_string_lossy().to_string();
        AppState::new(db, config)
    }

    fn secure_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("apas.mpaxos.com"));
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        headers.insert("x-forwarded-for", HeaderValue::from_static("192.0.2.1"));
        headers
    }

    fn login_request() -> MobileLoginRequest {
        MobileLoginRequest {
            email: "mobile@example.test".to_string(),
            password: "correct horse battery staple".to_string(),
            installation_id: "install-1".to_string(),
            platform: MobilePlatform::Ios,
            device_name: Some("Test phone".to_string()),
            app_version: "0.1.0".to_string(),
        }
    }

    #[test]
    fn refresh_hash_is_keyed_and_stable() {
        let first = hash_refresh_token("token", "secret").unwrap();
        assert_eq!(first, hash_refresh_token("token", "secret").unwrap());
        assert_ne!(first, hash_refresh_token("token", "other").unwrap());
        assert!(!first.contains("token"));
    }

    #[tokio::test]
    async fn production_mobile_auth_rejects_cleartext() {
        let dir = std::env::temp_dir().join(format!("apas-mobile-auth-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::Database::new(&dir.join("apas.db").to_string_lossy())
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let state = AppState::new(db, crate::config::Config::default());
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("apas.mpaxos.com"));
        assert!(require_secure_request(&headers, &state).is_err());
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        assert!(require_secure_request(&headers, &state).is_ok());
    }

    #[tokio::test]
    async fn refresh_rotates_and_reuse_revokes_the_device_session() {
        let state = state_with_user().await;
        let Json(initial) = login(
            State(state.clone()),
            secure_headers(),
            Json(login_request()),
        )
        .await
        .unwrap();
        assert!(state
            .db
            .is_mobile_device_session_active(
                &initial.device_session_id.to_string(),
                &initial.user_id.to_string()
            )
            .await
            .unwrap());

        let Json(rotated) = refresh(
            State(state.clone()),
            secure_headers(),
            Json(MobileRefreshRequest {
                refresh_token: initial.refresh_token.clone(),
                installation_id: "install-1".to_string(),
            }),
        )
        .await
        .unwrap();
        assert_ne!(rotated.refresh_token, initial.refresh_token);

        let reused = refresh(
            State(state.clone()),
            secure_headers(),
            Json(MobileRefreshRequest {
                refresh_token: initial.refresh_token,
                installation_id: "install-1".to_string(),
            }),
        )
        .await;
        assert!(matches!(reused, Err(AppError::AuthError(message)) if message.contains("reuse")));
        assert!(!state
            .db
            .is_mobile_device_session_active(
                &rotated.device_session_id.to_string(),
                &rotated.user_id.to_string()
            )
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn concurrent_refresh_fails_closed_and_password_reset_revokes_sessions() {
        let state = state_with_user().await;
        let Json(initial) = login(
            State(state.clone()),
            secure_headers(),
            Json(login_request()),
        )
        .await
        .unwrap();
        let current_hash =
            hash_refresh_token(&initial.refresh_token, &state.config.auth.jwt_secret).unwrap();
        let db_a = state.db.clone();
        let db_b = state.db.clone();
        let hash_a = current_hash.clone();
        let hash_b = current_hash;
        let (first, second) = tokio::join!(
            db_a.rotate_mobile_refresh_token(&hash_a, "install-1", "next-a"),
            db_b.rotate_mobile_refresh_token(&hash_b, "install-1", "next-b")
        );
        let outcomes = [first.unwrap(), second.unwrap()];
        assert_eq!(outcomes.iter().filter(|value| value.is_ok()).count(), 1);
        assert_eq!(outcomes.iter().filter(|value| value.is_err()).count(), 1);
        assert!(!state
            .db
            .is_mobile_device_session_active(
                &initial.device_session_id.to_string(),
                &initial.user_id.to_string()
            )
            .await
            .unwrap());

        let Json(second_login) = login(
            State(state.clone()),
            secure_headers(),
            Json(login_request()),
        )
        .await
        .unwrap();
        state
            .db
            .update_user_password("mobile@example.test", "replacement-hash")
            .await
            .unwrap();
        assert!(!state
            .db
            .is_mobile_device_session_active(
                &second_login.device_session_id.to_string(),
                &second_login.user_id.to_string()
            )
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn logout_explicit_revocation_and_suspension_invalidate_access() {
        let state = state_with_user().await;
        let Json(initial) = login(
            State(state.clone()),
            secure_headers(),
            Json(login_request()),
        )
        .await
        .unwrap();
        let mut authenticated = secure_headers();
        authenticated.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", initial.access_token)).unwrap(),
        );
        assert_eq!(
            list_devices(State(state.clone()), authenticated.clone())
                .await
                .unwrap()
                .0
                .len(),
            1
        );
        assert_eq!(
            logout(
                State(state.clone()),
                authenticated,
                Json(MobileLogoutRequest {
                    refresh_token: initial.refresh_token,
                }),
            )
            .await
            .unwrap(),
            StatusCode::NO_CONTENT
        );
        assert!(!state
            .db
            .is_mobile_device_session_active(
                &initial.device_session_id.to_string(),
                &initial.user_id.to_string()
            )
            .await
            .unwrap());

        let Json(second) = login(
            State(state.clone()),
            secure_headers(),
            Json(login_request()),
        )
        .await
        .unwrap();
        state
            .db
            .update_cluster_user_status(
                &second.user_id.to_string(),
                &second.user_id.to_string(),
                crate::db::AccountStatus::Suspended,
            )
            .await
            .unwrap();
        assert!(!state
            .db
            .is_mobile_device_session_active(
                &second.device_session_id.to_string(),
                &second.user_id.to_string()
            )
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn expired_refresh_is_rejected_and_cluster_admin_can_revoke_any_device() {
        let state = state_with_user().await;
        let user = state
            .db
            .get_user_by_email("mobile@example.test")
            .await
            .unwrap()
            .unwrap();
        let expired_id = Uuid::new_v4();
        state
            .db
            .create_mobile_device_session(
                &expired_id.to_string(),
                &user.id,
                "expired-install",
                "android",
                None,
                "0.1.0",
                "expired-hash",
                &(Utc::now() - Duration::minutes(1)).to_rfc3339(),
            )
            .await
            .unwrap();
        let expired = state
            .db
            .rotate_mobile_refresh_token("expired-hash", "expired-install", "unused")
            .await
            .unwrap();
        assert!(matches!(expired, Err(MobileRefreshFailure::Expired)));

        let Json(active) = login(
            State(state.clone()),
            secure_headers(),
            Json(login_request()),
        )
        .await
        .unwrap();
        let admin_id = Uuid::new_v4();
        state
            .db
            .create_user(&crate::db::User {
                id: admin_id.to_string(),
                email: "admin@example.test".to_string(),
                password_hash: "hash".to_string(),
                created_at: None,
                cluster_role: "admin".to_string(),
                account_status: "active".to_string(),
            })
            .await
            .unwrap();
        let admin_token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &Claims {
                sub: admin_id.to_string(),
                exp: (Utc::now() + Duration::hours(1)).timestamp() as usize,
                device_session_id: None,
                token_kind: None,
            },
            &jsonwebtoken::EncodingKey::from_secret(state.config.auth.jwt_secret.as_bytes()),
        )
        .unwrap();
        let mut admin_headers = secure_headers();
        admin_headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {admin_token}")).unwrap(),
        );
        assert_eq!(
            revoke_device(
                State(state.clone()),
                admin_headers,
                Path(active.device_session_id.to_string()),
            )
            .await
            .unwrap(),
            StatusCode::NO_CONTENT
        );
        assert!(!state
            .db
            .is_mobile_device_session_active(
                &active.device_session_id.to_string(),
                &active.user_id.to_string()
            )
            .await
            .unwrap());
    }
}
