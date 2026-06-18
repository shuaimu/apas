use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
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
                    }) => {
                        match validate_daemon_registration(
                            &token,
                            machine,
                            projects,
                            &state.config.auth.jwt_secret,
                        ) {
                            Ok(registration) => {
                                machine_id = registration.machine_id;
                                user_id = registration.user_id;
                                machine_info = registration.machine;
                                initial_projects = registration.projects;
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
                        let parsed: Result<DaemonToServer, _> = serde_json::from_str(&text);
                        match parsed {
                            Ok(message) => apply_registered_daemon_message(
                                &state.sessions,
                                &machine_id,
                                message,
                            ),
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

fn apply_registered_daemon_message(
    sessions: &SessionManager,
    machine_id: &Uuid,
    message: DaemonToServer,
) {
    match message {
        DaemonToServer::Heartbeat { projects } => {
            sessions.update_daemon_projects(machine_id, projects);
        }
        DaemonToServer::Register {
            machine, projects, ..
        } => {
            // Ignore token re-auth, but accept machine/project refresh payload.
            sessions.update_daemon_machine_info(machine_id, machine);
            sessions.update_daemon_projects(machine_id, projects);
        }
        DaemonToServer::MachineInfoUpdate { machine } => {
            sessions.update_daemon_machine_info(machine_id, machine);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::auth::Claims;
    use chrono::{Duration, Utc};
    use jsonwebtoken::{encode, EncodingKey, Header};

    fn test_token(user_id: Uuid, secret: &str) -> String {
        let claims = Claims {
            sub: user_id.to_string(),
            exp: (Utc::now() + Duration::hours(1)).timestamp() as usize,
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
            minimax_backend: None,
            glm_backend: None,
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

    #[test]
    fn ws_daemon_heartbeat_and_machine_info_update_refresh_registered_state() {
        let sessions = SessionManager::new();
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
            &sessions,
            &machine_id,
            DaemonToServer::Heartbeat {
                projects: vec![test_project("new", "/work/new", false)],
            },
        );

        let machine = registered_machine(&sessions, &user_id, &machine_id);
        assert_eq!(machine.projects.len(), 1);
        assert_eq!(machine.projects[0].project_id, "new");
        assert!(!machine.projects[0].is_running);

        sessions.apply_web_minimax_config(
            &machine_id,
            Some("https://minimax.example".to_string()),
            Some("secret-key".to_string()),
            false,
        );
        apply_registered_daemon_message(
            &sessions,
            &machine_id,
            DaemonToServer::MachineInfoUpdate {
                machine: test_machine(Uuid::new_v4(), "updated-host"),
            },
        );

        let machine = registered_machine(&sessions, &user_id, &machine_id);
        assert_eq!(machine.machine.hostname, "updated-host");
        assert_eq!(machine.machine.machine_id, machine_id);
        let backend = machine
            .machine
            .minimax_backend
            .expect("minimax backend should be preserved");
        assert_eq!(
            backend.api_base_url.as_deref(),
            Some("https://minimax.example")
        );
        assert_eq!(backend.api_key.as_deref(), Some("secret-key"));
        assert!(backend.api_key_configured);
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
}
