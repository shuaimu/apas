use axum::{
    routing::{delete, get, post},
    Router,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::state::AppState;

pub mod auth;
mod health;
mod share;
mod ws_cli;
mod ws_web;

pub fn create_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Health check
        .route("/health", get(health::health_check))
        // Auth routes
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        // Device code flow (CLI login)
        .route("/auth/device-code", post(auth::device_code))
        .route("/auth/device-poll", post(auth::device_poll))
        .route("/auth/device-complete", post(auth::device_complete))
        // Password reset
        .route("/auth/forgot-password", post(auth::forgot_password))
        .route("/auth/reset-password", post(auth::reset_password))
        // Admin routes (for debugging)
        .route("/admin/impersonate", post(auth::admin_impersonate))
        .route("/admin/users", post(auth::admin_list_users))
        // Session sharing routes
        .route("/share/generate", post(share::generate_code))
        .route("/share/redeem", post(share::redeem_code))
        .route("/share/list/:session_id", get(share::list_shares))
        .route("/share/:session_id/:user_id", delete(share::revoke_access))
        // WebSocket routes
        .route("/ws/web", get(ws_web::ws_handler))
        .route("/ws/cli", get(ws_cli::ws_handler))
        // Middleware
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}
