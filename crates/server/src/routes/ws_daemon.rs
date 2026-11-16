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
                    Ok(DaemonToServer::LocalProjectRegistered { .. }) => {
                        tracing::warn!("Daemon sent local-registration result before register");
                    }
                    Ok(DaemonToServer::ProjectProvisioningDiscarded { .. }) => {
                        tracing::warn!("Daemon sent provisioning-discard result before register");
                    }
                    Ok(DaemonToServer::ProjectRuntimeStopped { .. }) => {
                        tracing::warn!("Daemon sent project-runtime stop result before register");
                    }
                    Ok(DaemonToServer::ProjectRuntimeStarted { .. }) => {
                        tracing::warn!("Daemon sent project-runtime start result before register");
                    }
                    Ok(DaemonToServer::AutonomousJob { .. }) => {
                        tracing::warn!("Daemon sent job report before register");
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

    // Hydrate persisted preferences before register_daemon's synchronous first
    // broadcast. Raw daemon registration/heartbeats never supply this metadata.
    if let Err(error) = state.refresh_project_machine_preferences(user_id).await {
        tracing::warn!(%user_id, %error, "Could not load project machine preferences");
        let text = serde_json::to_string(&registration_failed(
            "Could not load project machine preferences",
        ))
        .unwrap();
        let _ = sender.send(Message::Text(text.into())).await;
        return;
    }

    // Channel for async server->daemon commands.
    let (tx, mut rx) = mpsc::channel::<ServerToDaemon>(64);
    let connection_sender = tx.clone();
    let job_registration_guard = state.jobs.daemon_guard(machine_id).await;
    // The projects this registration lists: the ones the machine follows.
    let listed_projects: Vec<String> = initial_projects
        .iter()
        .map(|project| project.project_id.clone())
        .collect();
    let registered_msg = register_daemon_session(
        &state.sessions,
        machine_id,
        user_id,
        tx,
        machine_info.clone(),
        initial_projects,
        &daemon_capabilities,
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
        if state
            .sessions
            .is_daemon_connection(&machine_id, &connection_sender)
        {
            state.sessions.unregister_daemon(&machine_id);
        }
        return;
    }
    crate::jobs::daemon_connected(&state, machine_id).await;
    drop(job_registration_guard);

    // The machine was away: bring it in line with each recorded project it
    // lists, now that its capabilities are known and it has been told it is
    // registered. Once, here, and at no other time; a spawned task, so that
    // this connection already drains what the step sends it.
    {
        let state = state.clone();
        let connection = connection_sender.clone();
        tokio::spawn(async move {
            crate::open_state::follow::follow_registered_daemon(
                &state,
                machine_id,
                &connection,
                listed_projects,
            )
            .await;
        });
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
                let _job_delivery_guard = if matches!(&msg, ServerToDaemon::AutonomousJob { .. }) {
                    Some(state.jobs.daemon_guard(machine_id).await)
                } else { None };
                if let ServerToDaemon::AutonomousJob { command } = &msg {
                    if !state.sessions.is_daemon_connection(&machine_id, &connection_sender)
                        || !crate::jobs::outbound_allowed(&state, machine_id, command).await.unwrap_or(false) {
                        continue;
                    }
                }
                let text = serde_json::to_string(&msg).unwrap();
                if sender.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if !state.sessions.is_daemon_connection(&machine_id, &connection_sender) {
                            break;
                        }
                        let parsed: Result<DaemonToServer, _> = serde_json::from_str(&text);
                        match parsed {
                            Ok(message) => {
                                if matches!(&message, DaemonToServer::AutonomousJob { .. }) {
                                    if text.len() > shared::JOB_MAX_FRAME_BYTES {
                                        tracing::warn!(%machine_id, "Rejected oversized job report");
                                        continue;
                                    }
                                    let _guard = state.jobs.daemon_guard(machine_id).await;
                                    if !state.sessions.is_daemon_connection(&machine_id, &connection_sender) {
                                        break;
                                    }
                                    apply_registered_daemon_message(&state, &machine_id, message).await;
                                } else {
                                    apply_registered_daemon_message(&state, &machine_id, message).await;
                                }
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

    let _job_disconnect_guard = state.jobs.daemon_guard(machine_id).await;
    if state
        .sessions
        .is_daemon_connection(&machine_id, &connection_sender)
    {
        state.sessions.unregister_daemon(&machine_id);
    }
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

/// The advertised daemon capabilities this server echoes in `Registered`:
/// only those a daemon reads the echo of. A daemon that advertised
/// `project_instances_v1` and does not see it echoed runs each project on one
/// machine, as a server predating instances requires, so this server, which
/// runs instances, must echo it.
fn negotiated_daemon_capabilities(advertised: &[String]) -> Vec<String> {
    [shared::PROJECT_INSTANCES_CAPABILITY]
        .into_iter()
        .filter(|negotiable| advertised.iter().any(|capability| capability == negotiable))
        .map(str::to_string)
        .collect()
}

fn register_daemon_session(
    sessions: &SessionManager,
    machine_id: Uuid,
    user_id: Uuid,
    sender: mpsc::Sender<ServerToDaemon>,
    machine: MachineInfo,
    projects: Vec<MachineProjectInfo>,
    capabilities: &[String],
) -> ServerToDaemon {
    sessions.register_daemon(machine_id, user_id, sender, machine, projects);
    ServerToDaemon::Registered {
        machine_id,
        cluster_owner_user_id: Some(user_id),
        negotiated_capabilities: negotiated_daemon_capabilities(capabilities),
    }
}

async fn apply_registered_daemon_message(
    state: &AppState,
    machine_id: &Uuid,
    message: DaemonToServer,
) {
    match message {
        DaemonToServer::AutonomousJob { report } => {
            if let Err(error) = crate::jobs::report(state, *machine_id, report).await {
                tracing::warn!(%machine_id, %error, "Rejected autonomous job report");
            }
        }
        DaemonToServer::Heartbeat { projects } => {
            state.sessions.update_daemon_projects(machine_id, projects);
            // A project this machine lists for the first time is read once,
            // so its part is reported. A heartbeat starts and stops nothing.
            crate::open_state::load_missing_bases(state, machine_id).await;
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
        DaemonToServer::LocalProjectRegistered { request_id, result } => {
            super::local_projects::registered(state, machine_id, request_id, result).await;
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
            let request = match state
                .db
                .get_project_provisioning_by_request_id(&request_id_value)
                .await
            {
                Ok(Some(request)) => request,
                Ok(None) => {
                    // Owner requests also carry a browser correlation id, but
                    // only shared-cluster requests have a provisioning row.
                    // Relay ordinary results only to the authenticated owner
                    // of this daemon, without finalizing any shared project.
                    state
                        .sessions
                        .relay_project_instance_created(machine_id, request_id, project_id, error);
                    return;
                }
                Err(error) => {
                    tracing::warn!(%machine_id, request_id = %request_id_value, %error, "could not look up provisioning result");
                    return;
                }
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
                        {
                            // A creation that starts the project records it
                            // open, by its creator and naming this machine.
                            // Held until the start has been sent.
                            let _open_state =
                                state.sessions.project_open_state_guard(&project_id).await;
                            crate::open_state::record_created_open(
                                state,
                                &requester_id,
                                &project_id,
                                Some(*machine_id),
                                "provisioning",
                            )
                            .await;
                            // Kept for the machine's part until it is seen
                            // running; this start carries no id to answer.
                            state.sessions.record_machine_start_asked(
                                &project_id,
                                *machine_id,
                                None,
                            );
                            if !state
                                .sessions
                                .send_to_daemon(
                                    machine_id,
                                    ServerToDaemon::StartProjectCli {
                                        project_id: project_id.clone(),
                                        policy,
                                        request_id: None,
                                    },
                                )
                                .await
                            {
                                state
                                    .sessions
                                    .clear_machine_outcome(&project_id, machine_id);
                            }
                        }
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
        DaemonToServer::ProjectRuntimeStarted {
            request_id,
            project_id,
            success,
            error,
        } => {
            tracing::info!(
                %machine_id,
                %request_id,
                %project_id,
                success,
                error = error.as_deref().unwrap_or(""),
                "daemon project-runtime start completed"
            );
            // The answer is kept for the machine's part in the project: a
            // refusal with the daemon's reason, a success as a machine still
            // starting until it is seen running the project.
            let kept = state.sessions.machine_start_answered(
                machine_id,
                &request_id,
                &project_id,
                success,
                error.clone(),
            );
            // A start sent for new panes waiting on this machine. A refusal
            // fails them; a success leaves them waiting for the instance's
            // `SessionStart`, since only that makes it able to take a pane.
            if !state.sessions.instance_start_result(
                *machine_id,
                request_id,
                &project_id,
                success,
                error,
            ) && !kept
            {
                tracing::debug!(%machine_id, %request_id, "nothing is waiting on this start");
            }
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
            providers: None,
        }
    }

    fn test_project(project_id: &str, path: &str, is_running: bool) -> MachineProjectInfo {
        MachineProjectInfo {
            project_id: project_id.to_string(),
            name: Some(project_id.to_string()),
            path: path.to_string(),
            preferred_machine_id: None,
            is_running,
            pid: is_running.then_some(1234),
            memory_kb: None,
            last_error: None,
            git_remote: None,
            open_state: None,
            part: None,
            part_reason: None,
            other_hosts: Vec::new(),
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
            &[],
        );

        match response {
            ServerToDaemon::Registered {
                machine_id,
                cluster_owner_user_id,
                negotiated_capabilities,
            } => {
                assert_eq!(cluster_owner_user_id, Some(registration.user_id));
                assert_eq!(machine_id, registration.machine_id);
                assert!(negotiated_capabilities.is_empty());
            }
            other => panic!("unexpected registration response: {other:?}"),
        }

        let machine = registered_machine(&sessions, &user_id, &registration.machine_id);
        assert_eq!(machine.machine.hostname, "daemon-host");
        assert_eq!(machine.projects.len(), 1);
        assert_eq!(machine.projects[0].project_id, "alpha");
    }

    /// A daemon learns from `Registered` whether this server runs project
    /// instances, and is told so only when it advertised them. Without the
    /// echo, a capable daemon keeps each project on one machine.
    #[tokio::test]
    async fn ws_daemon_registration_echoes_instance_support_only_when_advertised() {
        use tokio_tungstenite::{connect_async, tungstenite::Message as ClientMessage};

        let state = test_state().await;
        let user_id = Uuid::new_v4();
        add_active_user(&state, user_id, "echo@example.com").await;
        let token = test_token(user_id, &state.config.auth.jwt_secret);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = crate::routes::create_router(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        let mut echoes = Vec::new();
        for capabilities in [
            vec![
                shared::PROJECT_POLICY_CAPABILITY.to_string(),
                shared::PROJECT_INSTANCES_CAPABILITY.to_string(),
            ],
            vec![shared::PROJECT_POLICY_CAPABILITY.to_string()],
        ] {
            let machine_id = Uuid::new_v4();
            let (mut daemon, _) = connect_async(format!("ws://{address}/ws/daemon"))
                .await
                .unwrap();
            let register = DaemonToServer::Register {
                token: token.clone(),
                machine: test_machine(machine_id, "echo-host"),
                projects: Vec::new(),
                capabilities,
            };
            daemon
                .send(ClientMessage::Text(
                    serde_json::to_string(&register).unwrap().into(),
                ))
                .await
                .unwrap();
            let reply = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    match daemon.next().await.unwrap().unwrap() {
                        ClientMessage::Text(text) => {
                            return serde_json::from_str::<ServerToDaemon>(&text).unwrap()
                        }
                        ClientMessage::Ping(data) => {
                            daemon.send(ClientMessage::Pong(data)).await.unwrap()
                        }
                        other => panic!("unexpected websocket frame: {other:?}"),
                    }
                }
            })
            .await
            .expect("timed out waiting for the registration reply");
            match reply {
                ServerToDaemon::Registered {
                    machine_id: registered,
                    negotiated_capabilities,
                    ..
                } => {
                    assert_eq!(registered, machine_id);
                    echoes.push(negotiated_capabilities);
                }
                other => panic!("unexpected registration reply: {other:?}"),
            }
        }
        server.abort();

        assert_eq!(
            echoes,
            vec![
                vec![shared::PROJECT_INSTANCES_CAPABILITY.to_string()],
                Vec::<String>::new(),
            ]
        );
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
            &[],
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
            &[],
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
    async fn owner_creation_results_reach_only_owner_with_or_without_request_id() {
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
            test_machine(machine_id, "owner-host"),
            Vec::new(),
            &[],
        );
        let owner_web_id = Uuid::new_v4();
        let (owner_tx, mut owner_rx) = mpsc::channel(4);
        state.sessions.register_web(owner_web_id, owner_tx);
        state.sessions.set_web_user(owner_web_id, owner);
        let member_web_id = Uuid::new_v4();
        let (member_tx, mut member_rx) = mpsc::channel(4);
        state.sessions.register_web(member_web_id, member_tx);
        state.sessions.set_web_user(member_web_id, member);

        for request_id in [Some(Uuid::new_v4().to_string()), None] {
            for error in [
                None,
                Some("git clone failed: repository does not exist".to_string()),
            ] {
                let project_id = error.is_none().then(|| Uuid::new_v4().to_string());
                apply_registered_daemon_message(
                    &state,
                    &machine_id,
                    DaemonToServer::ProjectInstanceCreated {
                        request_id: request_id.clone(),
                        project_id: project_id.clone(),
                        path: project_id
                            .as_ref()
                            .map(|_| "/projects/rational".to_string()),
                        error: error.clone(),
                    },
                )
                .await;

                let ServerToWeb::ProjectInstanceCreated {
                    machine_id: routed_machine,
                    request_id: routed_request,
                    project_id: routed_project,
                    error: routed_error,
                } = owner_rx
                    .try_recv()
                    .expect("owner must receive the creation result")
                else {
                    panic!("expected a creation result")
                };
                assert_eq!(routed_machine, machine_id);
                assert_eq!(routed_request, request_id);
                assert_eq!(routed_project, project_id);
                assert_eq!(routed_error, error);
                assert!(member_rx.try_recv().is_err());
                assert!(daemon_rx.try_recv().is_err());
            }
        }
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
            &[],
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

        // A shared result must still match its recorded machine and project;
        // neither mismatch may fall through to the owner-result relay.
        for (reported_machine, reported_project) in [
            (Uuid::new_v4(), project_id.clone()),
            (machine_id, Uuid::new_v4().to_string()),
        ] {
            apply_registered_daemon_message(
                &state,
                &reported_machine,
                DaemonToServer::ProjectInstanceCreated {
                    request_id: Some(request_id.clone()),
                    project_id: Some(reported_project),
                    path: Some("/managed/codex".to_string()),
                    error: None,
                },
            )
            .await;
            assert!(web_rx.try_recv().is_err());
            assert!(owner_web_rx.try_recv().is_err());
            assert!(daemon_rx.try_recv().is_err());
            assert!(state.db.get_project(&project_id).await.unwrap().is_none());
        }

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
            request_id: _,
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
        // A creation that starts the project records it open, by its
        // creator and naming the machine it starts on, so the machine is
        // started again when its daemon next registers without running it.
        let record = state
            .db
            .get_project_open_state(&project_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.state, Some(shared::ProjectOpenState::Open));
        assert_eq!(record.set_by_account(), Some(member));
        assert_eq!(record.set_kind, Some(shared::OpenStateSetKind::Request));
        assert_eq!(record.stand_in_machine(), Some(machine_id));
        let audited = state
            .db
            .list_cluster_audit_events(&owner.to_string(), 50, 0)
            .await
            .unwrap();
        assert!(
            audited.iter().any(|event| {
                event.action == "project.opened"
                    && event.actor_user_id == member.to_string()
                    && event.project_id.as_deref() == Some(project_id.as_str())
                    && event
                        .details
                        .as_deref()
                        .is_some_and(|details| details.contains("\"provisioning\""))
            }),
            "the hosting cluster's history shows who opened it"
        );
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
            &[],
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

    /// A daemon that registers is brought in line with the recorded state of
    /// the projects its `Register` frame lists, after it has been told it is
    /// registered: an open project it owns tabs of and does not run is
    /// started with a start it answers, a closed one it still runs is
    /// stopped, and one nobody recorded is left alone. That happens once.
    /// Heartbeats ask nothing, also after the daemon refused the start,
    /// whose reason is kept for the machine's part.
    #[tokio::test]
    async fn a_registering_daemon_follows_the_recorded_state_once() {
        use crate::session::open_state::MachineOutcome;
        use tokio_tungstenite::{connect_async, tungstenite::Message as ClientMessage};

        let state = test_state().await;
        let user_id = Uuid::new_v4();
        add_active_user(&state, user_id, "follow@example.com").await;
        let machine_id = Uuid::new_v4();
        let (open, closed, unrecorded) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        for project in [open, closed, unrecorded] {
            state
                .db
                .authorize_project_registration(&project.to_string(), &user_id.to_string())
                .await
                .unwrap();
            // The project's roster: one tab, recorded for this machine.
            assert!(state
                .sessions
                .attach_web_to_session(&project, Uuid::new_v4(), None));
            let mut pane = shared::PaneConfig::defaults().remove(0);
            pane.machine_id = Some(machine_id);
            pane.hostname = Some("follow-host".to_string());
            state.sessions.set_session_panes(&project, vec![pane]);
        }
        for (project, recorded) in [
            (open, shared::ProjectOpenState::Open),
            (closed, shared::ProjectOpenState::Closed),
        ] {
            assert!(state
                .db
                .set_project_open_state(
                    &project.to_string(),
                    recorded,
                    &user_id.to_string(),
                    shared::OpenStateSetKind::Request,
                    None,
                )
                .await
                .unwrap());
        }
        let inventory = vec![
            test_project(&open.to_string(), "/work/open", false),
            test_project(&closed.to_string(), "/work/closed", true),
            test_project(&unrecorded.to_string(), "/work/unrecorded", false),
        ];

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = crate::routes::create_router(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let (mut daemon, _) = connect_async(format!("ws://{address}/ws/daemon"))
            .await
            .unwrap();
        let send = |message: DaemonToServer| {
            ClientMessage::Text(serde_json::to_string(&message).unwrap().into())
        };
        daemon
            .send(send(DaemonToServer::Register {
                token: test_token(user_id, &state.config.auth.jwt_secret),
                machine: test_machine(machine_id, "follow-host"),
                projects: inventory.clone(),
                capabilities: vec![
                    shared::PROJECT_POLICY_CAPABILITY.to_string(),
                    shared::PROJECT_INSTANCES_CAPABILITY.to_string(),
                ],
            }))
            .await
            .unwrap();

        // Every frame the daemon is sent within `wait`, pings answered.
        async fn frames(
            daemon: &mut tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            wait: std::time::Duration,
            until: usize,
        ) -> Vec<ServerToDaemon> {
            let mut received = Vec::new();
            let _ = tokio::time::timeout(wait, async {
                while received.len() < until {
                    match daemon.next().await.unwrap().unwrap() {
                        ClientMessage::Text(text) => {
                            received.push(serde_json::from_str::<ServerToDaemon>(&text).unwrap())
                        }
                        ClientMessage::Ping(data) => {
                            daemon.send(ClientMessage::Pong(data)).await.unwrap()
                        }
                        other => panic!("unexpected websocket frame: {other:?}"),
                    }
                }
            })
            .await;
            received
        }

        let received = frames(&mut daemon, std::time::Duration::from_secs(5), 3).await;
        assert!(
            matches!(received.first(), Some(ServerToDaemon::Registered { .. })),
            "told it is registered before anything is asked of it: {received:?}"
        );
        let mut request_id = None;
        let mut stopped = Vec::new();
        for frame in &received[1..] {
            match frame {
                ServerToDaemon::StartProjectCli {
                    project_id,
                    policy,
                    request_id: asked,
                } => {
                    assert_eq!(project_id, &open.to_string());
                    assert!(policy.is_some(), "a start carries the project's policy");
                    assert!(request_id.is_none(), "asked once");
                    request_id = Some(asked.expect("a start its daemon answers"));
                }
                ServerToDaemon::StopProjectCli { project_id, .. } => {
                    stopped.push(project_id.clone())
                }
                other => panic!("the follow step sent {other:?}"),
            }
        }
        let request_id = request_id.expect("the open project is started");
        assert_eq!(stopped, vec![closed.to_string()]);

        // The daemon refuses the start. Heartbeats follow; nothing more is
        // asked of it.
        let refusal = "Another host holds this project's claim";
        daemon
            .send(send(DaemonToServer::ProjectRuntimeStarted {
                request_id,
                project_id: open.to_string(),
                success: false,
                error: Some(refusal.to_string()),
            }))
            .await
            .unwrap();
        for _ in 0..3 {
            daemon
                .send(send(DaemonToServer::Heartbeat {
                    projects: inventory.clone(),
                }))
                .await
                .unwrap();
        }
        let more = frames(&mut daemon, std::time::Duration::from_millis(500), usize::MAX).await;
        assert!(more.is_empty(), "nothing is asked on a heartbeat: {more:?}");
        assert_eq!(
            state
                .sessions
                .machine_outcome(&open.to_string(), &machine_id),
            Some(MachineOutcome::NotStarted(refusal.to_string()))
        );
        let part = state
            .sessions
            .project_open_view(&open.to_string())
            .unwrap()
            .machines
            .into_iter()
            .find(|machine| machine.machine_id == Some(machine_id))
            .unwrap();
        assert_eq!(part.part, shared::MachinePart::NotStarted);
        assert_eq!(part.reason.as_deref(), Some(refusal));
        // The project nobody recorded is reported too, and stays so.
        assert!(state
            .sessions
            .project_open_view(&unrecorded.to_string())
            .is_some_and(|view| view.state.is_none()));
        assert_eq!(
            state
                .db
                .get_project_open_state(&unrecorded.to_string())
                .await
                .unwrap()
                .unwrap()
                .state,
            None
        );
        server.abort();
    }

    /// The daemon's answer to a correlated start reaches the new panes
    /// waiting on it. Success only says the runtime started, so they keep
    /// waiting for the instance itself; a refusal fails them with its reason.
    #[tokio::test]
    async fn a_start_result_reaches_the_panes_waiting_on_it() {
        use crate::session::pane_launches::{InstanceStartOutcome, InstanceStartWait};

        let state = test_state().await;
        let machine_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let project_id = session_id.to_string();
        let InstanceStartWait::Waiting {
            waiter,
            start: Some(request_id),
            mut outcome,
        } = state
            .sessions
            .wait_for_instance_start(session_id, machine_id, Some(&project_id))
        else {
            panic!("expected a new start")
        };
        let answer = |success: bool, error: Option<&str>| DaemonToServer::ProjectRuntimeStarted {
            request_id,
            project_id: project_id.clone(),
            success,
            error: error.map(str::to_string),
        };

        apply_registered_daemon_message(&state, &machine_id, answer(true, None)).await;
        assert!(
            matches!(
                outcome.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ),
            "a started runtime is not yet an attached instance"
        );
        // Another machine's daemon cannot answer this start.
        apply_registered_daemon_message(&state, &Uuid::new_v4(), answer(false, Some("not mine")))
            .await;
        apply_registered_daemon_message(
            &state,
            &machine_id,
            answer(false, Some("Another host holds this project's claim")),
        )
        .await;
        assert_eq!(
            state
                .sessions
                .await_instance_start(waiter, outcome, std::time::Duration::from_secs(1))
                .await,
            Some(InstanceStartOutcome::Refused(
                "Another host holds this project's claim".to_string()
            ))
        );
    }
}
