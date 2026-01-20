use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{extract::State, Json};
use chrono::{Duration, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use rand::Rng;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{db::User, error::AppError, state::{AppState, DeviceCodeState}};

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // user_id
    pub exp: usize,
}

pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    // Check if user already exists
    if state.db.get_user_by_email(&req.email).await?.is_some() {
        return Err(AppError::BadRequest("Email already registered".to_string()));
    }

    // Hash password
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(req.password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .to_string();

    // Create user
    let user_id = Uuid::new_v4().to_string();
    let user = User {
        id: user_id.clone(),
        email: req.email,
        password_hash,
        created_at: None,
    };
    state.db.create_user(&user).await?;

    // Generate token
    let token = generate_token(&user_id, &state.config.auth)?;

    Ok(Json(AuthResponse { token, user_id }))
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    // Find user
    let user = state
        .db
        .get_user_by_email(&req.email)
        .await?
        .ok_or_else(|| AppError::AuthError("Invalid email or password".to_string()))?;

    // Verify password
    let parsed_hash = PasswordHash::new(&user.password_hash)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Argon2::default()
        .verify_password(req.password.as_bytes(), &parsed_hash)
        .map_err(|_| AppError::AuthError("Invalid email or password".to_string()))?;

    // Generate token
    let token = generate_token(&user.id, &state.config.auth)?;

    Ok(Json(AuthResponse {
        token,
        user_id: user.id,
    }))
}

fn generate_token(user_id: &str, auth_config: &crate::config::AuthConfig) -> Result<String, AppError> {
    let expiration = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::hours(auth_config.token_expiry_hours as i64))
        .ok_or_else(|| AppError::Internal("Failed to calculate expiration".to_string()))?
        .timestamp() as usize;

    let claims = Claims {
        sub: user_id.to_string(),
        exp: expiration,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(auth_config.jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(e.to_string()))
}

pub fn verify_token(token: &str, secret: &str) -> Result<Claims, AppError> {
    jsonwebtoken::decode::<Claims>(
        token,
        &jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()),
        &jsonwebtoken::Validation::default(),
    )
    .map(|data| data.claims)
    .map_err(|e| AppError::AuthError(e.to_string()))
}

// ============================================================================
// Device Code Flow (for CLI login)
// ============================================================================

const WEB_UI_URL: &str = "http://apas.mpaxos.com";

#[derive(Debug, Serialize)]
pub struct DeviceCodeResponse {
    pub code: String,
    pub url: String,
    pub expires_in: u64,
}

#[derive(Debug, Deserialize)]
pub struct DevicePollRequest {
    pub code: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status")]
pub enum DevicePollResponse {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "success")]
    Success { token: String, user_id: String },
    #[serde(rename = "expired")]
    Expired,
}

#[derive(Debug, Deserialize)]
pub struct DeviceCompleteRequest {
    pub code: String,
    pub user_id: String,
}

/// Generate a device code for CLI login
/// POST /auth/device-code
pub async fn device_code(State(state): State<AppState>) -> Json<DeviceCodeResponse> {
    // Generate random 8-character code
    let code: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(8)
        .map(char::from)
        .collect::<String>()
        .to_uppercase();

    let expires_at = Utc::now() + Duration::minutes(10);

    // Store the device code
    state.device_codes.insert(
        code.clone(),
        DeviceCodeState {
            expires_at,
            user_id: None,
        },
    );

    tracing::info!("Generated device code: {}", code);

    Json(DeviceCodeResponse {
        url: format!("{}/login?code={}", WEB_UI_URL, code),
        code,
        expires_in: 600,
    })
}

/// Poll for device code completion
/// POST /auth/device-poll
pub async fn device_poll(
    State(state): State<AppState>,
    Json(req): Json<DevicePollRequest>,
) -> Result<Json<DevicePollResponse>, AppError> {
    // Clean up expired codes first
    state.device_codes.retain(|_, v| v.expires_at > Utc::now());

    match state.device_codes.get(&req.code) {
        Some(code_state) => {
            if code_state.expires_at <= Utc::now() {
                state.device_codes.remove(&req.code);
                Ok(Json(DevicePollResponse::Expired))
            } else if let Some(user_id) = code_state.user_id {
                // User has completed login - generate token
                let token = generate_token(&user_id.to_string(), &state.config.auth)?;
                state.device_codes.remove(&req.code);
                tracing::info!("Device code {} completed for user {}", req.code, user_id);
                Ok(Json(DevicePollResponse::Success {
                    token,
                    user_id: user_id.to_string(),
                }))
            } else {
                // Still waiting for user to complete login
                Ok(Json(DevicePollResponse::Pending))
            }
        }
        None => Ok(Json(DevicePollResponse::Expired)),
    }
}

/// Complete device code authentication (called after user logs in via web)
/// POST /auth/device-complete
pub async fn device_complete(
    State(state): State<AppState>,
    Json(req): Json<DeviceCompleteRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = Uuid::parse_str(&req.user_id)
        .map_err(|_| AppError::BadRequest("Invalid user_id".to_string()))?;

    match state.device_codes.get_mut(&req.code) {
        Some(mut code_state) => {
            if code_state.expires_at <= Utc::now() {
                state.device_codes.remove(&req.code);
                return Err(AppError::BadRequest("Device code expired".to_string()));
            }
            code_state.user_id = Some(user_id);
            tracing::info!("Device code {} linked to user {}", req.code, user_id);
            Ok(Json(serde_json::json!({ "success": true })))
        }
        None => Err(AppError::BadRequest("Invalid device code".to_string())),
    }
}
