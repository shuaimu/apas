use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use shared::{MobileNotificationPreferences, MobilePlatform, MobilePushTokenRequest};
use uuid::Uuid;

use crate::{
    error::AppError,
    routes::{auth::require_active_claims, mobile_auth::claims_from_headers},
    state::AppState,
};

type HmacSha256 = Hmac<Sha256>;

fn mobile_claims(headers: &HeaderMap, state: &AppState) -> Result<(String, String), AppError> {
    let claims = claims_from_headers(headers, state)?;
    let device_session_id = claims
        .device_session_id
        .clone()
        .ok_or_else(|| AppError::Forbidden("Mobile access token required".to_string()))?;
    Ok((claims.sub, device_session_id))
}

fn hash_push_token(token: &str, secret: &str) -> Result<String, AppError> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| AppError::Internal("Invalid push-token key".to_string()))?;
    mac.update(token.as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

fn validate_push_token(token: &str) -> Result<(), AppError> {
    let token = token.trim();
    if token.len() < 16 || token.len() > 4096 || token.chars().any(char::is_whitespace) {
        return Err(AppError::BadRequest("Invalid push token".to_string()));
    }
    Ok(())
}

fn platform(platform: MobilePlatform) -> &'static str {
    match platform {
        MobilePlatform::Ios => "ios",
        MobilePlatform::Android => "android",
    }
}

pub async fn register_push_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MobilePushTokenRequest>,
) -> Result<StatusCode, AppError> {
    if !state.config.mobile.features.notifications {
        return Err(AppError::Forbidden(
            "Mobile notifications are disabled".to_string(),
        ));
    }
    let (user_id, device_session_id) = mobile_claims(&headers, &state)?;
    let claims = claims_from_headers(&headers, &state)?;
    require_active_claims(&state, &claims).await?;
    validate_push_token(&request.token)?;
    let token_hash = hash_push_token(&request.token, &state.config.auth.jwt_secret)?;
    let token_id = Uuid::new_v4().to_string();
    state
        .db
        .register_mobile_push_token(
            &token_id,
            &user_id,
            &device_session_id,
            &request.installation_id,
            platform(request.platform),
            request.token.trim(),
            &token_hash,
        )
        .await
        .map_err(|error| AppError::Forbidden(error.to_string()))?;
    state
        .db
        .record_audit(
            &user_id,
            "mobile.push_token_registered",
            "mobile_device_session",
            &device_session_id,
            Some(serde_json::json!({ "platform": platform(request.platform) })),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MobileNotificationPreferences>, AppError> {
    let (user_id, device_session_id) = mobile_claims(&headers, &state)?;
    let claims = claims_from_headers(&headers, &state)?;
    require_active_claims(&state, &claims).await?;
    let preferences = state
        .db
        .mobile_notification_preferences(&user_id, &device_session_id)
        .await?
        .unwrap_or(MobileNotificationPreferences {
            decisions: true,
            failures: true,
            pull_requests: true,
            completions: false,
        });
    Ok(Json(preferences))
}

pub async fn update_preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(preferences): Json<MobileNotificationPreferences>,
) -> Result<Json<MobileNotificationPreferences>, AppError> {
    let (user_id, device_session_id) = mobile_claims(&headers, &state)?;
    let claims = claims_from_headers(&headers, &state)?;
    require_active_claims(&state, &claims).await?;
    if !state
        .db
        .update_mobile_notification_preferences(&user_id, &device_session_id, &preferences)
        .await?
    {
        return Err(AppError::Forbidden(
            "Mobile device session is unavailable".to_string(),
        ));
    }
    Ok(Json(preferences))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_tokens_are_bounded_and_keyed_before_persistence() {
        assert!(validate_push_token("ExponentPushToken[abcdefghijklmnop]").is_ok());
        assert!(validate_push_token("short").is_err());
        let left = hash_push_token("token-a-abcdefghijkl", "secret").unwrap();
        let right = hash_push_token("token-a-abcdefghijkl", "other-secret").unwrap();
        assert_ne!(left, right);
        assert!(!left.contains("token-a"));
    }
}
