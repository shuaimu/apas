use axum::{
    routing::{delete, get, patch, post},
    Router,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::state::AppState;

mod admin;
pub mod auth;
mod authz;
mod health;
mod projects;
mod share;
mod ws_cli;
mod ws_daemon;
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
        .route("/auth/me", get(auth::me))
        // Device code flow (CLI login)
        .route("/auth/device-code", post(auth::device_code))
        .route("/auth/device-poll", post(auth::device_poll))
        .route("/auth/device-complete", post(auth::device_complete))
        // Password reset
        .route("/auth/forgot-password", post(auth::forgot_password))
        .route("/auth/reset-password", post(auth::reset_password))
        // Cluster administration control plane
        .route("/admin/stats", get(admin::get_stats))
        .route("/admin/users", get(admin::list_users))
        .route("/admin/users/invitations", post(admin::invite_user))
        .route("/admin/users/:user_id", patch(admin::update_user))
        .route("/admin/projects", get(admin::list_projects))
        .route("/admin/projects/:project_id", get(admin::get_project))
        .route(
            "/admin/projects/:project_id/members",
            post(admin::add_project_member),
        )
        .route(
            "/admin/projects/:project_id/members/:user_id",
            delete(admin::remove_project_member),
        )
        .route(
            "/admin/projects/:project_id/owner",
            patch(admin::transfer_owner),
        )
        .route(
            "/admin/projects/:project_id/lifecycle",
            patch(admin::update_lifecycle),
        )
        .route(
            "/admin/projects/:project_id/stop-runtime",
            post(admin::stop_runtime),
        )
        .route(
            "/admin/projects/:project_id/policy",
            patch(admin::update_policy),
        )
        .route("/admin/launch-profiles", get(admin::list_profiles))
        .route(
            "/admin/policy/default",
            get(admin::get_default_policy).patch(admin::update_default_policy),
        )
        .route("/admin/audit", get(admin::list_audit))
        // Project lifecycle self-service
        .route(
            "/projects/:project_id/owner",
            patch(projects::transfer_owner),
        )
        .route(
            "/projects/:project_id/members/me",
            delete(projects::leave_project),
        )
        .route(
            "/projects/:project_id/delete",
            post(projects::delete_project),
        )
        .route(
            "/projects/:project_id/deletion",
            get(projects::deletion_status),
        )
        // Session sharing routes
        .route("/share/generate", post(share::generate_code))
        .route("/share/redeem", post(share::redeem_code))
        .route("/share/list/:session_id", get(share::list_shares))
        .route("/share/:session_id/:user_id", delete(share::revoke_access))
        .route(
            "/share/:session_id/:user_id/role",
            patch(share::update_share_role),
        )
        // WebSocket routes
        .route("/ws/web", get(ws_web::ws_handler))
        .route("/ws/cli", get(ws_cli::ws_handler))
        .route("/ws/daemon", get(ws_daemon::ws_handler))
        // Middleware
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

#[cfg(test)]
mod cluster_authorization_tests {
    use super::*;
    use axum::extract::{Path, Query, State};
    use axum::http::{header, HeaderMap, HeaderValue};
    use axum::Json;
    use jsonwebtoken::{encode, EncodingKey, Header};

    async fn state() -> AppState {
        let dir = std::env::temp_dir().join(format!("apas-route-authz-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("apas.db");
        let db = crate::db::Database::new(&db_path.to_string_lossy())
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let mut config = crate::config::Config::default();
        config.database.path = db_path.to_string_lossy().into_owned();
        config.auth.jwt_secret = "route-test-secret".to_string();
        AppState::new(db, config)
    }

    async fn add_user(state: &AppState, id: &str, role: &str, status: &str) {
        state
            .db
            .create_user(&crate::db::User {
                id: id.to_string(),
                email: format!("{id}@test"),
                password_hash: "hash".to_string(),
                created_at: None,
                cluster_role: role.to_string(),
                account_status: status.to_string(),
            })
            .await
            .unwrap();
    }

    fn headers(state: &AppState, user_id: &str) -> HeaderMap {
        let token = encode(
            &Header::default(),
            &auth::Claims {
                sub: user_id.to_string(),
                exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
            },
            &EncodingKey::from_secret(state.config.auth.jwt_secret.as_bytes()),
        )
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        headers
    }

    #[tokio::test]
    async fn cluster_inventory_requires_admin_but_does_not_grant_content_access() {
        let state = state().await;
        add_user(&state, "owner", "user", "active").await;
        add_user(&state, "admin", "admin", "active").await;
        state
            .db
            .authorize_project_registration("project-a", "owner")
            .await
            .unwrap();

        assert!(admin::list_projects(
            State(state.clone()),
            headers(&state, "owner"),
            Query(admin::PageQuery::default()),
        )
        .await
        .is_err());
        let inventory = admin::list_projects(
            State(state.clone()),
            headers(&state, "admin"),
            Query(admin::PageQuery::default()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(inventory.items.len(), 1);
        let json = serde_json::to_value(&inventory.items[0]).unwrap();
        for content_field in ["messages", "files", "diff", "terminal", "conversation"] {
            assert!(json.get(content_field).is_none());
        }
        assert!(
            authz::require_project_member(&headers(&state, "admin"), &state, "project-a",)
                .await
                .is_err()
        );

        let audit_before = state.db.list_audit_events(100, 0).await.unwrap().len();
        assert!(admin::update_policy(
            State(state.clone()),
            headers(&state, "owner"),
            Path("project-a".to_string()),
            Json(admin::UpdatePolicyRequest {
                team_available: Some(true),
                allowed_launch_profiles: Some(vec!["agent:codex:official:default".to_string(),]),
            }),
        )
        .await
        .is_err());
        assert_eq!(
            state.db.list_audit_events(100, 0).await.unwrap().len(),
            audit_before,
            "a rejected control-plane mutation is not a successful audit event",
        );
        let _ = admin::update_policy(
            State(state.clone()),
            headers(&state, "admin"),
            Path("project-a".to_string()),
            Json(admin::UpdatePolicyRequest {
                team_available: Some(true),
                allowed_launch_profiles: Some(vec!["agent:codex:official:default".to_string()]),
            }),
        )
        .await
        .unwrap();
        assert!(state
            .db
            .list_audit_events(100, 0)
            .await
            .unwrap()
            .iter()
            .any(|event| event.action == "project.policy_changed"));
    }

    #[tokio::test]
    async fn mutable_role_and_status_are_reloaded_for_every_http_guard() {
        let state = state().await;
        add_user(&state, "admin", "admin", "active").await;
        let bearer = headers(&state, "admin");
        assert!(authz::require_cluster_admin(&bearer, &state).await.is_ok());
        state
            .db
            .create_user(&crate::db::User {
                id: "other-admin".to_string(),
                email: "other-admin@test".to_string(),
                password_hash: "hash".to_string(),
                created_at: None,
                cluster_role: "admin".to_string(),
                account_status: "active".to_string(),
            })
            .await
            .unwrap();
        state
            .db
            .update_cluster_user_status("other-admin", "admin", crate::db::AccountStatus::Suspended)
            .await
            .unwrap();
        assert!(authz::require_cluster_admin(&bearer, &state).await.is_err());
    }
}
