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
mod mobile;
mod mobile_auth;
mod mobile_notifications;
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
        .route("/auth/mobile/login", post(mobile_auth::login))
        .route("/auth/mobile/refresh", post(mobile_auth::refresh))
        .route("/auth/mobile/logout", post(mobile_auth::logout))
        .route("/mobile/v1/devices", get(mobile_auth::list_devices))
        .route("/mobile/v1/bootstrap", get(mobile::bootstrap))
        .route("/mobile/v1/task-launches", post(mobile::launch_task))
        .route(
            "/mobile/v1/push-token",
            post(mobile_notifications::register_push_token),
        )
        .route(
            "/mobile/v1/notification-preferences",
            get(mobile_notifications::get_preferences)
                .put(mobile_notifications::update_preferences),
        )
        .route(
            "/mobile/v1/devices/:id/revoke",
            post(mobile_auth::revoke_device),
        )
        // Device code flow (CLI login)
        .route("/auth/device-code", post(auth::device_code))
        .route("/auth/device-poll", post(auth::device_poll))
        .route("/auth/device-complete", post(auth::device_complete))
        // Password reset
        .route("/auth/forgot-password", post(auth::forgot_password))
        .route("/auth/reset-password", post(auth::reset_password))
        // Cluster administration control plane
        .route("/admin/stats", get(admin::get_stats))
        .route("/admin/mobile/metrics", get(admin::get_mobile_metrics))
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
                device_session_id: None,
                token_kind: None,
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

#[cfg(test)]
mod mobile_websocket_load_tests {
    use super::*;
    use futures::{SinkExt, StreamExt};
    use std::sync::Arc;
    use tokio::sync::{mpsc, Notify};
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    type ClientSocket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn send_client(socket: &mut ClientSocket, message: shared::WebToServer) {
        socket
            .send(Message::Text(serde_json::to_string(&message).unwrap()))
            .await
            .unwrap();
    }

    async fn receive_server(
        socket: &mut ClientSocket,
        matches: impl Fn(&shared::ServerToWeb) -> bool,
    ) -> shared::ServerToWeb {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let frame = socket.next().await.unwrap().unwrap();
                let Message::Text(text) = frame else { continue };
                let message = serde_json::from_str::<shared::ServerToWeb>(&text).unwrap();
                if matches(&message) {
                    return message;
                }
            }
        })
        .await
        .expect("timed out waiting for server message")
    }

    async fn connect_client(
        url: &str,
        token: String,
        client_kind: Option<shared::ClientKind>,
    ) -> ClientSocket {
        let (mut socket, _) = connect_async(url).await.unwrap();
        send_client(
            &mut socket,
            shared::WebToServer::Authenticate {
                token,
                capabilities: vec!["mobile_coding_mutations_v1".to_string()],
                client_kind,
                app_version: Some("e2e-test".to_string()),
                protocol_version: Some(shared::MOBILE_PROTOCOL_MIN_VERSION),
            },
        )
        .await;
        receive_server(&mut socket, |message| {
            matches!(message, shared::ServerToWeb::Authenticated { .. })
        })
        .await;
        socket
    }

    async fn receive_cli(
        receiver: &mut mpsc::Receiver<shared::ServerToCli>,
        matches: impl Fn(&shared::ServerToCli) -> bool,
    ) -> shared::ServerToCli {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let message = receiver.recv().await.expect("CLI sender closed");
                if matches(&message) {
                    return message;
                }
            }
        })
        .await
        .expect("timed out waiting for CLI message")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn bounded_mobile_websocket_connections_authenticate_concurrently() {
        const CONNECTIONS: usize = 128;
        let dir =
            std::env::temp_dir().join(format!("apas-mobile-ws-load-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("apas.db");
        let db = crate::db::Database::new(&db_path.to_string_lossy())
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let user_id = uuid::Uuid::new_v4();
        db.create_user(&crate::db::User {
            id: user_id.to_string(),
            email: "mobile-load@example.test".to_string(),
            password_hash: "unused".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .unwrap();
        db.create_mobile_device_session(
            "load-device",
            &user_id.to_string(),
            "load-installation",
            "ios",
            None,
            "load-test",
            "unused-refresh-hash",
            &(chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
        )
        .await
        .unwrap();
        let mut config = crate::config::Config::default();
        config.database.path = db_path.to_string_lossy().into_owned();
        config.auth.jwt_secret = "mobile-load-secret".to_string();
        let state = AppState::new(db, config);
        let (access_token, _) = auth::generate_mobile_access_token(
            &user_id.to_string(),
            "load-device",
            &state.config.auth,
        )
        .unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = create_router(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let release = Arc::new(Notify::new());
        let (ready_tx, mut ready_rx) = mpsc::channel(CONNECTIONS);
        let started = std::time::Instant::now();
        let mut clients = Vec::with_capacity(CONNECTIONS);
        for _ in 0..CONNECTIONS {
            let url = format!("ws://{address}/ws/web");
            let token = access_token.clone();
            let release = release.clone();
            let ready_tx = ready_tx.clone();
            clients.push(tokio::spawn(async move {
                let (mut socket, _) = connect_async(url).await.unwrap();
                let authenticate = shared::WebToServer::Authenticate {
                    token,
                    capabilities: vec!["mobile_bootstrap_v1".to_string()],
                    client_kind: Some(shared::ClientKind::Mobile),
                    app_version: Some("load-test".to_string()),
                    protocol_version: Some(shared::MOBILE_PROTOCOL_MIN_VERSION),
                };
                socket
                    .send(Message::Text(serde_json::to_string(&authenticate).unwrap()))
                    .await
                    .unwrap();
                loop {
                    let frame = socket.next().await.unwrap().unwrap();
                    let Message::Text(text) = frame else { continue };
                    if matches!(
                        serde_json::from_str::<shared::ServerToWeb>(&text).unwrap(),
                        shared::ServerToWeb::Authenticated { .. }
                    ) {
                        break;
                    }
                }
                ready_tx.send(()).await.unwrap();
                release.notified().await;
                socket.close(None).await.unwrap();
            }));
        }
        drop(ready_tx);
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            for _ in 0..CONNECTIONS {
                ready_rx
                    .recv()
                    .await
                    .expect("client task did not authenticate");
            }
        })
        .await
        .expect("bounded mobile WebSocket load timed out");
        assert_eq!(
            state.mobile_metrics.snapshot().socket_authenticated,
            CONNECTIONS as u64
        );
        eprintln!(
            "authenticated {CONNECTIONS} concurrent mobile sockets in {:?}",
            started.elapsed()
        );
        release.notify_waiters();
        for client in clients {
            client.await.unwrap();
        }
        server.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn revoking_a_device_session_terminates_its_authenticated_mobile_socket() {
        let dir = std::env::temp_dir().join(format!(
            "apas-mobile-revocation-e2e-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("apas.db");
        let db = crate::db::Database::new(&db_path.to_string_lossy())
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let user_id = uuid::Uuid::new_v4();
        db.create_user(&crate::db::User {
            id: user_id.to_string(),
            email: "mobile-revocation@example.test".to_string(),
            password_hash: "unused".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .unwrap();
        db.create_mobile_device_session(
            "revoked-device",
            &user_id.to_string(),
            "revoked-installation",
            "android",
            None,
            "e2e-test",
            "unused-refresh-hash",
            &(chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
        )
        .await
        .unwrap();
        let mut config = crate::config::Config::default();
        config.database.path = db_path.to_string_lossy().into_owned();
        config.auth.jwt_secret = "mobile-revocation-secret".to_string();
        let state = AppState::new(db, config);
        let (access_token, _) = auth::generate_mobile_access_token(
            &user_id.to_string(),
            "revoked-device",
            &state.config.auth,
        )
        .unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = create_router(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let url = format!("ws://{address}/ws/web");
        let mut mobile = connect_client(&url, access_token, Some(shared::ClientKind::Mobile)).await;

        state
            .db
            .revoke_mobile_device_session(
                &user_id.to_string(),
                "revoked-device",
                false,
                "e2e_revocation",
            )
            .await
            .unwrap();
        send_client(&mut mobile, shared::WebToServer::Heartbeat).await;
        receive_server(&mut mobile, |message| {
            matches!(
                message,
                shared::ServerToWeb::AuthenticationFailed { reason }
                    if reason.contains("expired or revoked")
            )
        })
        .await;

        server.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn mobile_mutations_converge_once_and_fail_closed_after_access_loss() {
        let dir =
            std::env::temp_dir().join(format!("apas-mobile-mutation-e2e-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("apas.db");
        let db = crate::db::Database::new(&db_path.to_string_lossy())
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let owner_id = uuid::Uuid::new_v4();
        let member_id = uuid::Uuid::new_v4();
        for (id, email) in [
            (owner_id, "mutation-owner@example.test"),
            (member_id, "mutation-mobile@example.test"),
        ] {
            db.create_user(&crate::db::User {
                id: id.to_string(),
                email: email.to_string(),
                password_hash: "unused".to_string(),
                created_at: None,
                cluster_role: "user".to_string(),
                account_status: "active".to_string(),
            })
            .await
            .unwrap();
        }
        let project_id = "mobile-mutation-project";
        db.authorize_project_registration(project_id, &owner_id.to_string())
            .await
            .unwrap();
        db.add_project_member(&owner_id.to_string(), project_id, &member_id.to_string())
            .await
            .unwrap();
        let cli_id = uuid::Uuid::new_v4();
        let session_id = uuid::Uuid::new_v4();
        db.create_session(&crate::db::Session {
            id: session_id.to_string(),
            user_id: owner_id.to_string(),
            cli_client_id: Some(cli_id.to_string()),
            working_dir: Some("/workspace/project".to_string()),
            hostname: Some("builder".to_string()),
            status: "active".to_string(),
            created_at: None,
            updated_at: None,
            is_paused: false,
            project_id: Some(project_id.to_string()),
            git_remote: None,
            git_remote_url: None,
        })
        .await
        .unwrap();
        db.create_mobile_device_session(
            "mutation-device",
            &member_id.to_string(),
            "mutation-installation",
            "ios",
            None,
            "e2e-test",
            "unused-refresh-hash",
            &(chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
        )
        .await
        .unwrap();
        let mut config = crate::config::Config::default();
        config.database.path = db_path.to_string_lossy().into_owned();
        config.auth.jwt_secret = "mobile-mutation-secret".to_string();
        let state = AppState::new(db, config);
        let (cli_tx, mut cli_rx) = mpsc::channel(128);
        state
            .sessions
            .register_cli(cli_id, owner_id, cli_tx, Some("e2e-test".to_string()));
        state.sessions.create_cli_session(
            session_id,
            cli_id,
            Some("/workspace/project".to_string()),
            Some("builder".to_string()),
        );
        state
            .sessions
            .set_session_project(session_id, project_id.to_string());
        state
            .sessions
            .set_session_panes(&session_id, shared::PaneConfig::defaults());
        let pane_id = shared::PANE_ID_INTERACTIVE;
        let (access_token, _) = auth::generate_mobile_access_token(
            &member_id.to_string(),
            "mutation-device",
            &state.config.auth,
        )
        .unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = create_router(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let url = format!("ws://{address}/ws/web");
        let mut mobile =
            connect_client(&url, access_token.clone(), Some(shared::ClientKind::Mobile)).await;
        let mut browser = connect_client(&url, access_token.clone(), None).await;
        for socket in [&mut mobile, &mut browser] {
            send_client(socket, shared::WebToServer::AttachSession { session_id }).await;
            receive_server(socket, |message| {
                matches!(
                    message,
                    shared::ServerToWeb::SessionAttached {
                        session_id: attached,
                        ..
                    } if *attached == session_id
                )
            })
            .await;
        }
        while tokio::time::timeout(std::time::Duration::from_millis(20), cli_rx.recv())
            .await
            .is_ok()
        {}

        send_client(
            &mut mobile,
            shared::WebToServer::Input {
                session_id: Some(session_id),
                text: "run focused tests".to_string(),
                pane_type: None,
                pane_id: Some(pane_id),
                client_msg_id: Some("steer-1".to_string()),
            },
        )
        .await;
        let steered = receive_cli(&mut cli_rx, |message| {
            matches!(
                message,
                shared::ServerToCli::Input {
                    session_id: target,
                    pane_id: Some(target_pane),
                    data,
                } if *target == session_id && *target_pane == pane_id && data == "run focused tests"
            )
        })
        .await;
        assert!(matches!(steered, shared::ServerToCli::Input { .. }));
        for socket in [&mut mobile, &mut browser] {
            receive_server(socket, |message| {
                matches!(
                    message,
                    shared::ServerToWeb::UserInput {
                        client_msg_id: Some(id),
                        ..
                    } if id == "steer-1"
                )
            })
            .await;
        }

        // Model normal mobile OS suspension by dropping the transport, opening
        // a fresh authenticated socket, and reattaching before retrying the
        // same acknowledged input identifier.
        mobile.close(None).await.unwrap();
        let mut mobile =
            connect_client(&url, access_token.clone(), Some(shared::ClientKind::Mobile)).await;
        send_client(
            &mut mobile,
            shared::WebToServer::AttachSession { session_id },
        )
        .await;
        receive_server(&mut mobile, |message| {
            matches!(
                message,
                shared::ServerToWeb::SessionAttached {
                    session_id: attached,
                    ..
                } if *attached == session_id
            )
        })
        .await;
        while tokio::time::timeout(std::time::Duration::from_millis(20), cli_rx.recv())
            .await
            .is_ok()
        {}
        send_client(
            &mut mobile,
            shared::WebToServer::Input {
                session_id: Some(session_id),
                text: "run focused tests".to_string(),
                pane_type: None,
                pane_id: Some(pane_id),
                client_msg_id: Some("steer-1".to_string()),
            },
        )
        .await;
        receive_server(&mut mobile, |message| {
            matches!(
                message,
                shared::ServerToWeb::UserInput {
                    client_msg_id: Some(id),
                    ..
                } if id == "steer-1"
            )
        })
        .await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), cli_rx.recv())
                .await
                .is_err()
        );

        state.sessions.register_pending_decision(
            session_id,
            "approval-1".to_string(),
            Some(pane_id),
            shared::MutationKind::Approval,
        );
        send_client(
            &mut browser,
            shared::WebToServer::Reject {
                session_id: Some(session_id),
                tool_call_id: "approval-1".to_string(),
                pane_id: Some(pane_id),
                request_id: Some("decision-browser".to_string()),
            },
        )
        .await;
        let browser_ack = receive_server(&mut browser, |message| {
            matches!(
                message,
                shared::ServerToWeb::MutationAck {
                    request_id,
                    accepted: true,
                    ..
                } if request_id == "decision-browser"
            )
        })
        .await;
        assert!(matches!(
            browser_ack,
            shared::ServerToWeb::MutationAck { accepted: true, .. }
        ));
        receive_cli(&mut cli_rx, |message| {
            matches!(
                message,
                shared::ServerToCli::Input {
                    session_id: target,
                    pane_id: Some(target_pane),
                    data,
                } if *target == session_id && *target_pane == pane_id && data == "n"
            )
        })
        .await;

        // The mobile view was stale while another surface resolved the
        // decision. Its later answer must be rejected and never reach the CLI.
        send_client(
            &mut mobile,
            shared::WebToServer::Approve {
                session_id: Some(session_id),
                tool_call_id: "approval-1".to_string(),
                pane_id: Some(pane_id),
                request_id: Some("decision-mobile".to_string()),
            },
        )
        .await;
        let mobile_ack = receive_server(&mut mobile, |message| {
            matches!(
                message,
                shared::ServerToWeb::MutationAck { request_id, .. }
                    if request_id == "decision-mobile"
            )
        })
        .await;
        assert!(matches!(
            mobile_ack,
            shared::ServerToWeb::MutationAck {
                accepted: false,
                ..
            }
        ));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), cli_rx.recv())
                .await
                .is_err()
        );

        let interrupt = shared::WebToServer::InterruptPane {
            session_id: Some(session_id),
            pane_id,
            request_id: Some("interrupt-1".to_string()),
        };
        send_client(&mut mobile, interrupt.clone()).await;
        receive_cli(&mut cli_rx, |message| {
            matches!(
                message,
                shared::ServerToCli::InterruptPane {
                    session_id: target,
                    pane_id: target_pane,
                } if *target == session_id && *target_pane == pane_id
            )
        })
        .await;

        // Lose the acknowledgement with the suspended transport, then replay
        // on a newly authenticated connection. The original result is replayed
        // without emitting a second signal to the CLI.
        mobile.close(None).await.unwrap();
        let mut mobile = connect_client(&url, access_token, Some(shared::ClientKind::Mobile)).await;
        send_client(
            &mut mobile,
            shared::WebToServer::AttachSession { session_id },
        )
        .await;
        receive_server(&mut mobile, |message| {
            matches!(
                message,
                shared::ServerToWeb::SessionAttached {
                    session_id: attached,
                    ..
                } if *attached == session_id
            )
        })
        .await;
        while tokio::time::timeout(std::time::Duration::from_millis(20), cli_rx.recv())
            .await
            .is_ok()
        {}
        send_client(&mut mobile, interrupt).await;
        receive_server(&mut mobile, |message| {
            matches!(
                message,
                shared::ServerToWeb::MutationAck {
                    request_id,
                    accepted: true,
                    ..
                } if request_id == "interrupt-1"
            )
        })
        .await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), cli_rx.recv())
                .await
                .is_err()
        );

        state
            .db
            .remove_project_member(&owner_id.to_string(), project_id, &member_id.to_string())
            .await
            .unwrap();
        send_client(
            &mut mobile,
            shared::WebToServer::Input {
                session_id: Some(session_id),
                text: "stale mutation".to_string(),
                pane_type: None,
                pane_id: Some(pane_id),
                client_msg_id: Some("stale-1".to_string()),
            },
        )
        .await;
        receive_server(&mut mobile, |message| {
            matches!(message, shared::ServerToWeb::Error { message } if message.contains("Access denied"))
        })
        .await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), cli_rx.recv())
                .await
                .is_err()
        );

        mobile.close(None).await.unwrap();
        browser.close(None).await.unwrap();
        server.abort();
    }
}
