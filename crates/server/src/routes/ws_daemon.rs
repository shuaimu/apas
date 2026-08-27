use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
#[cfg(test)]
use shared::ServerToWeb;
use shared::{DaemonToServer, MachineInfo, MachineProjectInfo, ServerToDaemon};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::routes::auth::verify_token;
use crate::session::SessionManager;
use crate::state::AppState;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    let machine_id: Uuid;
    let user_id: Uuid;
    let machine_info: MachineInfo;
    let initial_projects: Vec<shared::MachineProjectInfo>;
    let daemon_capabilities: Vec<String>;

    // Registration handshake.
    loop {
        match receiver.next().await {
            Some(Ok(Message::Text(text))) => {
                let parsed: Result<DaemonToServer, _> = serde_json::from_str(&text);
                match parsed {
                    Ok(DaemonToServer::Register {
                        token,
                        machine,
                        projects,
                        capabilities,
                    }) => {
                        match validate_daemon_registration(
                            &token,
                            machine,
                            projects,
                            &state.config.auth.jwt_secret,
                        ) {
                            Ok(registration) => {
                                match state
                                    .db
                                    .get_user_by_id(&registration.user_id.to_string())
                                    .await
                                {
                                    Ok(Some(user)) if user.is_active() => {}
                                    Ok(Some(_)) => {
                                        let text = serde_json::to_string(&registration_failed(
                                            "Cluster account is suspended",
                                        ))
                                        .unwrap();
                                        let _ = sender.send(Message::Text(text.into())).await;
                                        return;
                                    }
                                    Ok(None) => {
                                        let text = serde_json::to_string(&registration_failed(
                                            "Cluster account not found",
                                        ))
                                        .unwrap();
                                        let _ = sender.send(Message::Text(text.into())).await;
                                        return;
                                    }
                                    Err(err) => {
                                        tracing::warn!("Daemon account lookup failed: {}", err);
                                        let text = serde_json::to_string(&registration_failed(
                                            "Could not load cluster account",
                                        ))
                                        .unwrap();
                                        let _ = sender.send(Message::Text(text.into())).await;
                                        return;
                                    }
                                }
                                machine_id = registration.machine_id;
                                user_id = registration.user_id;
                                machine_info = registration.machine;
                                initial_projects = registration.projects;
                                daemon_capabilities = capabilities;
                                break;
                            }
                            Err(response) => {
                                let text = serde_json::to_string(&response).unwrap();
                                let _ = sender.send(Message::Text(text.into())).await;
                                return;
                            }
                        }
                    }
                    Ok(DaemonToServer::Heartbeat { .. }) => {
                        tracing::warn!("Daemon sent heartbeat before register");
                    }
                    Ok(DaemonToServer::MachineInfoUpdate { .. }) => {
                        tracing::warn!("Daemon sent machine info update before register");
                    }
                    Ok(DaemonToServer::ProjectInstanceCreated { .. }) => {
                        tracing::warn!("Daemon sent project-instance result before register");
                    }
                    Ok(DaemonToServer::ProjectProvisioningDiscarded { .. }) => {
                        tracing::warn!("Daemon sent provisioning-discard result before register");
                    }
                    Ok(DaemonToServer::ProjectRuntimeStopped { .. }) => {
                        tracing::warn!("Daemon sent project-runtime stop result before register");
                    }
                    Err(err) => {
                        tracing::warn!("Failed to parse daemon registration message: {}", err);
                    }
                }
            }
            Some(Ok(Message::Ping(data))) => {
                let _ = sender.send(Message::Pong(data)).await;
            }
            Some(Err(err)) => {
                tracing::warn!("Daemon websocket error during registration: {}", err);
                return;
            }
            None => return,
            _ => {}
        }
    }

    // Channel for async server->daemon commands.
    let (tx, mut rx) = mpsc::channel::<ServerToDaemon>(64);
    let registered_msg = register_daemon_session(
        &state.sessions,
        machine_id,
        user_id,
        tx,
        machine_info.clone(),
        initial_projects,
    );
    state
        .sessions
        .set_daemon_capabilities(machine_id, daemon_capabilities);
    let registered_text = serde_json::to_string(&registered_msg).unwrap();
    if sender
        .send(Message::Text(registered_text.into()))
        .await
        .is_err()
    {
        state.sessions.unregister_daemon(&machine_id);
        return;
    }

    tracing::info!(
        "Daemon connected: machine={} user={} hostname={}",
        machine_id,
        user_id,
        machine_info.hostname
    );

    loop {
        tokio::select! {
            Some(msg) = rx.recv() => {
                let text = serde_json::to_string(&msg).unwrap();
                if sender.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if !state.sessions.is_daemon_connected(&machine_id) {
                            break;
                        }
                        let parsed: Result<DaemonToServer, _> = serde_json::from_str(&text);
                        match parsed {
                            Ok(message) => {
                                apply_registered_daemon_message(&state, &machine_id, message).await
                            }
                            Err(err) => {
                                tracing::warn!("Failed to parse daemon message: {}", err);
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if sender.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        tracing::warn!("Daemon websocket error: {}", err);
                        break;
                    }
                    None => break,
                }
            }
        }
    }

    state.sessions.unregister_daemon(&machine_id);
    tracing::info!("Daemon disconnected: {}", machine_id);
}

#[derive(Debug)]
struct ValidatedDaemonRegistration {
    machine_id: Uuid,
    user_id: Uuid,
    machine: MachineInfo,
    projects: Vec<MachineProjectInfo>,
}

fn registration_failed(reason: impl Into<String>) -> ServerToDaemon {
    ServerToDaemon::RegistrationFailed {
        reason: reason.into(),
    }
}

fn validate_daemon_registration(
    token: &str,
    machine: MachineInfo,
    projects: Vec<MachineProjectInfo>,
    jwt_secret: &str,
) -> Result<ValidatedDaemonRegistration, ServerToDaemon> {
    let claims = verify_token(token, jwt_secret)
        .map_err(|err| registration_failed(format!("Authentication failed: {}", err)))?;
    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| registration_failed("Invalid user ID in token"))?;

    let mut machine = machine;
    if machine.machine_id.is_nil() {
        machine.machine_id = Uuid::new_v4();
    }

    Ok(ValidatedDaemonRegistration {
        machine_id: machine.machine_id,
        user_id,
        machine,
        projects,
    })
}

fn register_daemon_session(
    sessions: &SessionManager,
    machine_id: Uuid,
    user_id: Uuid,
    sender: mpsc::Sender<ServerToDaemon>,
    machine: MachineInfo,
    projects: Vec<MachineProjectInfo>,
) -> ServerToDaemon {
    sessions.register_daemon(machine_id, user_id, sender, machine, projects);
    ServerToDaemon::Registered { machine_id }
}

async fn apply_registered_daemon_message(
    state: &AppState,
    machine_id: &Uuid,
    message: DaemonToServer,
) {
    match message {
        DaemonToServer::Heartbeat { projects } => {
            state.sessions.update_daemon_projects(machine_id, projects);
        }
        DaemonToServer::Register {
            machine, projects, ..
        } => {
            // Ignore token re-auth, but accept machine/project refresh payload.
            state
                .sessions
                .update_daemon_machine_info(machine_id, machine);
            state.sessions.update_daemon_projects(machine_id, projects);
        }
        DaemonToServer::MachineInfoUpdate { machine } => {
            state
                .sessions
                .update_daemon_machine_info(machine_id, machine);
        }
        DaemonToServer::ProjectInstanceCreated {
            request_id,
            project_id,
            error,
            path,
        } => {
            let Some(request_id_value) = request_id.clone() else {
                state
                    .sessions
                    .relay_project_instance_created(machine_id, request_id, project_id, error);
                return;
            };
            let Ok(Some(request)) = state
                .db
                .get_project_provisioning_by_request_id(&request_id_value)
                .await
            else {
                tracing::warn!(%machine_id, request_id = %request_id_value, "ignored unknown provisioning result");
                return;
            };
            let Ok(requester_id) = Uuid::parse_str(&request.requester_user_id) else {
                tracing::warn!(request_id = %request_id_value, "provisioning requester id is invalid");
                return;
            };
            if request.machine_id != machine_id.to_string()
                || project_id
                    .as_deref()
                    .is_some_and(|id| id != request.project_id)
            {
                tracing::warn!(%machine_id, request_id = %request_id_value, "ignored forged provisioning result");
                return;
            }
            if let Some(error) = error {
                let safe_error = shared::scrub_shared_clone_error(&error).to_string();
                let _ = state
                    .db
                    .fail_project_provisioning(
                        &request_id_value,
                        &request.requester_user_id,
                        &safe_error,
                    )
                    .await;
                state.sessions.relay_project_instance_created_to_user(
                    &requester_id,
                    machine_id,
                    Some(request_id_value),
                    None,
                    Some(safe_error),
                );
                return;
            }
            let (Some(project_id), Some(path)) = (project_id, path) else {
                return;
            };
            if state
                .db
                .mark_project_provisioning_cloned(&request_id_value, &project_id, &path)
                .await
                .unwrap_or(false)
            {
                match state
                    .db
                    .finalize_project_provisioning(&request_id_value, &request.requester_user_id)
                    .await
                {
                    Ok(Some(_)) => {
                        let policy = state
                            .db
                            .get_effective_project_policy(&project_id)
                            .await
                            .ok();
                        let _ = state
                            .sessions
                            .send_to_daemon(
                                machine_id,
                                ServerToDaemon::StartProjectCli {
                                    project_id: project_id.clone(),
                                    policy,
                                },
                            )
                            .await;
                        state.sessions.relay_project_instance_created_to_user(
                            &requester_id,
                            machine_id,
                            Some(request_id_value),
                            Some(project_id),
                            None,
                        );
                    }
                    Ok(None) => {
                        let _ = state
                            .sessions
                            .send_to_daemon(
                                machine_id,
                                ServerToDaemon::DiscardProjectProvisioning {
                                    request_id: request_id_value.clone(),
                                    project_id,
                                },
                            )
                            .await;
                        state.sessions.relay_project_instance_created_to_user(
                            &requester_id,
                            machine_id,
                            Some(request_id_value),
                            None,
                            Some("Cluster access was revoked".to_string()),
                        );
                    }
                    Err(error) => tracing::warn!(%error, "could not finalize provisioning"),
                }
            }
        }
        DaemonToServer::ProjectProvisioningDiscarded {
            request_id,
            project_id,
            discarded,
            error,
        } => {
            tracing::info!(
                %machine_id,
                %request_id,
                %project_id,
                discarded,
                error = error.as_deref().unwrap_or(""),
                "daemon provisioning cleanup completed"
            );
        }
        DaemonToServer::ProjectRuntimeStopped {
            request_id,
            project_id,
            success,
            remaining_pane_hosts,
            error,
        } => {
            tracing::info!(
                %machine_id,
                %request_id,
                %project_id,
                success,
                remaining_pane_hosts,
                "daemon project-runtime cleanup completed"
            );
            state.sessions.complete_project_runtime_stop(
                request_id,
                project_id,
                success,
                remaining_pane_hosts,
                error,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::auth::Claims;
    use chrono::{Duration, Utc};
    use jsonwebtoken::{encode, EncodingKey, Header};

    async fn test_state() -> AppState {
        let dir = std::env::temp_dir().join(format!("apas-ws-daemon-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("apas.db");
        let db = crate::db::Database::new(&db_path.to_string_lossy())
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let mut config = crate::config::Config::default();
        config.database.path = db_path.to_string_lossy().into_owned();
        AppState::new(db, config)
    }

    async fn add_active_user(state: &AppState, id: Uuid, email: &str) {
        state
            .db
            .create_user(&crate::db::User {
                id: id.to_string(),
                email: email.to_string(),
                password_hash: "hash".to_string(),
                created_at: None,
                cluster_role: "user".to_string(),
                account_status: "active".to_string(),
            })
            .await
            .unwrap();
    }

    async fn join_cluster(state: &AppState, owner: Uuid, member: Uuid, email: &str) {
        let token_hash = Uuid::new_v4().to_string();
        state
            .db
            .create_shared_cluster_invitation(
                &Uuid::new_v4().to_string(),
                &token_hash,
                &owner.to_string(),
                email,
                &(Utc::now() + Duration::hours(1))
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string(),
            )
            .await
            .unwrap();
        state
            .db
            .accept_shared_cluster_invitation(&token_hash, &member.to_string())
            .await
            .unwrap()
            .unwrap();
    }

    fn test_token(user_id: Uuid, secret: &str) -> String {
        let claims = Claims {
            sub: user_id.to_string(),
            exp: (Utc::now() + Duration::hours(1)).timestamp() as usize,
            device_session_id: None,
            token_kind: None,
            credential_version: None,
        };
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("encode token")
    }

    fn test_machine(machine_id: Uuid, hostname: &str) -> MachineInfo {
        MachineInfo {
            machine_id,
            hostname: hostname.to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            daemon_version: Some("test-daemon".to_string()),
            deepseek_backend: None,
            last_seen: None,
        }
    }

    fn test_project(project_id: &str, path: &str, is_running: bool) -> MachineProjectInfo {
        MachineProjectInfo {
            project_id: project_id.to_string(),
            name: Some(project_id.to_string()),
            path: path.to_string(),
            is_running,
            pid: is_running.then_some(1234),
            memory_kb: None,
            last_error: None,
        }
    }

    fn registered_machine(
        sessions: &SessionManager,
        user_id: &Uuid,
        machine_id: &Uuid,
    ) -> shared::MachineWithProjects {
        sessions
            .get_machines_for_user(user_id)
            .into_iter()
            .find(|machine| machine.machine.machine_id == *machine_id)
            .expect("registered machine")
    }

    #[test]
    fn ws_daemon_register_normalizes_nil_machine_id_and_returns_registered() {
        let sessions = SessionManager::new();
        let user_id = Uuid::new_v4();
        let secret = "daemon-test-secret";
        let token = test_token(user_id, secret);

        let registration = validate_daemon_registration(
            &token,
            test_machine(Uuid::nil(), "daemon-host"),
            vec![test_project("alpha", "/work/alpha", true)],
            secret,
        )
        .expect("valid registration");

        assert_eq!(registration.user_id, user_id);
        assert!(!registration.machine_id.is_nil());
        assert_eq!(registration.machine.machine_id, registration.machine_id);

        let (tx, _rx) = mpsc::channel(1);
        let response = register_daemon_session(
            &sessions,
            registration.machine_id,
            registration.user_id,
            tx,
            registration.machine,
            registration.projects,
        );

        match response {
            ServerToDaemon::Registered { machine_id } => {
                assert_eq!(machine_id, registration.machine_id);
            }
            other => panic!("unexpected registration response: {other:?}"),
        }

        let machine = registered_machine(&sessions, &user_id, &registration.machine_id);
        assert_eq!(machine.machine.hostname, "daemon-host");
        assert_eq!(machine.projects.len(), 1);
        assert_eq!(machine.projects[0].project_id, "alpha");
    }

    #[test]
    fn ws_daemon_invalid_auth_returns_registration_failed_without_registration() {
        let sessions = SessionManager::new();
        let user_id = Uuid::new_v4();

        let result = validate_daemon_registration(
            "not-a-valid-token",
            test_machine(Uuid::new_v4(), "daemon-host"),
            vec![test_project("alpha", "/work/alpha", true)],
            "daemon-test-secret",
        );

        match result {
            Err(ServerToDaemon::RegistrationFailed { reason }) => {
                assert!(reason.contains("Authentication failed"));
            }
            other => panic!("unexpected registration result: {other:?}"),
        }
        assert!(sessions.get_machines_for_user(&user_id).is_empty());
    }

    #[tokio::test]
    async fn ws_daemon_heartbeat_and_machine_info_update_refresh_registered_state() {
        let state = test_state().await;
        let sessions = &state.sessions;
        let user_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(1);
        register_daemon_session(
            &sessions,
            machine_id,
            user_id,
            tx,
            test_machine(machine_id, "initial-host"),
            vec![test_project("old", "/work/old", true)],
        );

        apply_registered_daemon_message(
            &state,
            &machine_id,
            DaemonToServer::Heartbeat {
                projects: vec![test_project("new", "/work/new", false)],
            },
        )
        .await;

        let machine = registered_machine(&sessions, &user_id, &machine_id);
        assert_eq!(machine.projects.len(), 1);
        assert_eq!(machine.projects[0].project_id, "new");
        assert!(!machine.projects[0].is_running);

        apply_registered_daemon_message(
            &state,
            &machine_id,
            DaemonToServer::MachineInfoUpdate {
                machine: test_machine(Uuid::new_v4(), "updated-host"),
            },
        )
        .await;

        let machine = registered_machine(&sessions, &user_id, &machine_id);
        assert_eq!(machine.machine.hostname, "updated-host");
        assert_eq!(machine.machine.machine_id, machine_id);
    }

    #[tokio::test]
    async fn ws_daemon_unregister_clears_sender_and_marks_projects_not_running() {
        let sessions = SessionManager::new();
        let user_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();
        let (tx, mut rx) = mpsc::channel(2);
        register_daemon_session(
            &sessions,
            machine_id,
            user_id,
            tx,
            test_machine(machine_id, "daemon-host"),
            vec![
                test_project("running-a", "/work/a", true),
                test_project("running-b", "/work/b", true),
            ],
        );

        assert!(
            sessions
                .send_to_daemon(&machine_id, ServerToDaemon::Heartbeat)
                .await
        );
        assert!(matches!(rx.recv().await, Some(ServerToDaemon::Heartbeat)));

        sessions.unregister_daemon(&machine_id);

        assert!(
            !sessions
                .send_to_daemon(&machine_id, ServerToDaemon::Heartbeat)
                .await
        );
        let machine = registered_machine(&sessions, &user_id, &machine_id);
        assert!(machine
            .projects
            .iter()
            .all(|project| !project.is_running && project.pid.is_none()));
    }

    #[tokio::test]
    async fn shared_provisioning_finalizes_before_start_and_routes_to_requester() {
        let state = test_state().await;
        let owner = Uuid::new_v4();
        let member = Uuid::new_v4();
        add_active_user(&state, owner, "owner@example.test").await;
        add_active_user(&state, member, "member@example.test").await;
        join_cluster(&state, owner, member, "member@example.test").await;

        let machine_id = Uuid::new_v4();
        let (daemon_tx, mut daemon_rx) = mpsc::channel(4);
        register_daemon_session(
            &state.sessions,
            machine_id,
            owner,
            daemon_tx,
            test_machine(machine_id, "shared-host"),
            Vec::new(),
        );
        state.sessions.set_daemon_capabilities(
            machine_id,
            vec![shared::SHARED_PROJECT_PROVISIONING_CAPABILITY.to_string()],
        );
        let web_id = Uuid::new_v4();
        let (web_tx, mut web_rx) = mpsc::channel(4);
        state.sessions.register_web(web_id, web_tx);
        state.sessions.set_web_user(web_id, member);
        let owner_web_id = Uuid::new_v4();
        let (owner_web_tx, mut owner_web_rx) = mpsc::channel(4);
        state.sessions.register_web(owner_web_id, owner_web_tx);
        state.sessions.set_web_user(owner_web_id, owner);

        let request_id = Uuid::new_v4().to_string();
        let project_id = Uuid::new_v4().to_string();
        state
            .db
            .claim_project_provisioning(
                &request_id,
                &member.to_string(),
                &owner.to_string(),
                &machine_id.to_string(),
                "fingerprint",
                "github.com/openai/codex",
                "https://github.com/openai/codex.git",
                "codex",
                "apas/codex",
                &project_id,
            )
            .await
            .unwrap();

        apply_registered_daemon_message(
            &state,
            &machine_id,
            DaemonToServer::ProjectInstanceCreated {
                request_id: Some(request_id.clone()),
                project_id: Some(project_id.clone()),
                path: Some("/managed/codex".to_string()),
                error: None,
            },
        )
        .await;

        let ServerToDaemon::StartProjectCli {
            project_id: started,
            policy: Some(_),
        } = daemon_rx.recv().await.unwrap()
        else {
            panic!("finalization must be followed by a policy-bound start")
        };
        assert_eq!(started, project_id);
        let ServerToWeb::ProjectInstanceCreated {
            request_id: Some(routed_request),
            project_id: Some(routed_project),
            error: None,
            ..
        } = web_rx.recv().await.unwrap()
        else {
            panic!("requester must receive the provisioning result")
        };
        assert_eq!(routed_request, request_id);
        assert_eq!(routed_project, project_id);
        assert!(
            owner_web_rx.try_recv().is_err(),
            "the cluster owner must not receive a member's provisioning acknowledgement"
        );
        assert_eq!(
            state
                .db
                .get_project(&project_id)
                .await
                .unwrap()
                .unwrap()
                .owner_user_id,
            member.to_string()
        );
        assert!(state
            .db
            .project_is_placed_in_cluster(&project_id, &owner.to_string())
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn revocation_before_clone_ack_cancels_and_requests_marker_bound_discard() {
        let state = test_state().await;
        let owner = Uuid::new_v4();
        let member = Uuid::new_v4();
        add_active_user(&state, owner, "owner@example.test").await;
        add_active_user(&state, member, "member@example.test").await;
        join_cluster(&state, owner, member, "member@example.test").await;
        let machine_id = Uuid::new_v4();
        let (daemon_tx, mut daemon_rx) = mpsc::channel(4);
        register_daemon_session(
            &state.sessions,
            machine_id,
            owner,
            daemon_tx,
            test_machine(machine_id, "shared-host"),
            Vec::new(),
        );
        let request_id = Uuid::new_v4().to_string();
        let project_id = Uuid::new_v4().to_string();
        state
            .db
            .claim_project_provisioning(
                &request_id,
                &member.to_string(),
                &owner.to_string(),
                &machine_id.to_string(),
                "fingerprint",
                "github.com/openai/codex",
                "https://github.com/openai/codex.git",
                "codex",
                "apas/codex",
                &project_id,
            )
            .await
            .unwrap();
        state
            .db
            .revoke_cluster_membership(&owner.to_string(), &member.to_string())
            .await
            .unwrap();

        apply_registered_daemon_message(
            &state,
            &machine_id,
            DaemonToServer::ProjectInstanceCreated {
                request_id: Some(request_id.clone()),
                project_id: Some(project_id.clone()),
                path: Some("/managed/codex".to_string()),
                error: None,
            },
        )
        .await;
        assert!(matches!(
            daemon_rx.recv().await.unwrap(),
            ServerToDaemon::DiscardProjectProvisioning {
                request_id: discarded_request,
                project_id: discarded_project,
            } if discarded_request == request_id && discarded_project == project_id
        ));
        assert!(state.db.get_project(&project_id).await.unwrap().is_none());
    }
}
