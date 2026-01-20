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

use crate::{db::User, error::AppError, state::{AppState, DeviceCodeState, PasswordResetState}};
use lettre::{
    message::header::ContentType,
    transport::smtp::authentication::Credentials,
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};

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

// ============================================================================
// Password Reset Flow
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ForgotPasswordRequest {
    pub email: String,
}

#[derive(Debug, Deserialize)]
pub struct ResetPasswordRequest {
    pub token: String,
    pub password: String,
}

/// Request password reset email
/// POST /auth/forgot-password
pub async fn forgot_password(
    State(state): State<AppState>,
    Json(req): Json<ForgotPasswordRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Check if user exists
    let user = state.db.get_user_by_email(&req.email).await?;

    // Always return success to prevent email enumeration
    if user.is_none() {
        tracing::info!("Password reset requested for non-existent email: {}", req.email);
        return Ok(Json(serde_json::json!({
            "success": true,
            "message": "If your email is registered, you will receive a password reset link."
        })));
    }

    // Generate reset token (32-char hex)
    let token: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();

    let expires_at = Utc::now() + Duration::hours(1);

    // Store the reset token
    state.password_reset_tokens.insert(
        token.clone(),
        PasswordResetState {
            email: req.email.clone(),
            expires_at,
        },
    );

    // Send reset email
    if state.config.smtp.enabled {
        let reset_url = format!("{}/reset-password?token={}", WEB_UI_URL, token);

        if let Err(e) = send_password_reset_email(&state.config.smtp, &req.email, &reset_url).await {
            tracing::error!("Failed to send password reset email: {}", e);
            // Don't expose email errors to user
        } else {
            tracing::info!("Password reset email sent to {}", req.email);
        }
    } else {
        tracing::warn!("SMTP not configured, password reset token: {} for {}", token, req.email);
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "If your email is registered, you will receive a password reset link."
    })))
}

/// Reset password with token
/// POST /auth/reset-password
pub async fn reset_password(
    State(state): State<AppState>,
    Json(req): Json<ResetPasswordRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Clean up expired tokens
    state.password_reset_tokens.retain(|_, v| v.expires_at > Utc::now());

    // Validate token
    let reset_state = state.password_reset_tokens.get(&req.token)
        .ok_or_else(|| AppError::BadRequest("Invalid or expired reset token".to_string()))?;

    if reset_state.expires_at <= Utc::now() {
        state.password_reset_tokens.remove(&req.token);
        return Err(AppError::BadRequest("Reset token has expired".to_string()));
    }

    let email = reset_state.email.clone();
    drop(reset_state); // Release the lock before making DB calls

    // Validate password length
    if req.password.len() < 6 {
        return Err(AppError::BadRequest("Password must be at least 6 characters".to_string()));
    }

    // Hash new password
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(req.password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .to_string();

    // Update password in database
    let updated = state.db.update_user_password(&email, &password_hash).await?;

    if !updated {
        return Err(AppError::Internal("Failed to update password".to_string()));
    }

    // Remove the used token
    state.password_reset_tokens.remove(&req.token);

    tracing::info!("Password reset completed for {}", email);

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Password has been reset successfully."
    })))
}

/// Send password reset email via SMTP
async fn send_password_reset_email(
    smtp_config: &crate::config::SmtpConfig,
    to_email: &str,
    reset_url: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let email = Message::builder()
        .from(format!("{} <{}>", smtp_config.from_name, smtp_config.from_email).parse()?)
        .to(to_email.parse()?)
        .subject("APAS - Password Reset Request")
        .header(ContentType::TEXT_HTML)
        .body(format!(
            r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>Password Reset</title>
</head>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333; max-width: 600px; margin: 0 auto; padding: 20px;">
    <h2 style="color: #0891b2;">APAS Password Reset</h2>
    <p>You requested a password reset for your APAS account.</p>
    <p>Click the button below to reset your password:</p>
    <p style="text-align: center; margin: 30px 0;">
        <a href="{}" style="background-color: #0891b2; color: white; padding: 12px 24px; text-decoration: none; border-radius: 6px; display: inline-block;">Reset Password</a>
    </p>
    <p>Or copy and paste this link into your browser:</p>
    <p style="word-break: break-all; color: #666;">{}</p>
    <p style="margin-top: 30px; color: #666; font-size: 14px;">This link will expire in 1 hour.</p>
    <p style="color: #666; font-size: 14px;">If you didn't request this, you can safely ignore this email.</p>
</body>
</html>"#,
            reset_url, reset_url
        ))?;

    let creds = Credentials::new(
        smtp_config.username.clone(),
        smtp_config.password.clone(),
    );

    let mailer: AsyncSmtpTransport<Tokio1Executor> = AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp_config.host)?
        .credentials(creds)
        .port(smtp_config.port)
        .build();

    mailer.send(email).await?;
    Ok(())
}
