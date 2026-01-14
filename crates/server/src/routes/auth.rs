use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{extract::State, Json};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{db::User, error::AppError, state::AppState};

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
