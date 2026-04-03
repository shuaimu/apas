use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use shared::{DaemonToServer, MachineInfo, ServerToDaemon};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::routes::auth::verify_token;
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
                    }) => match verify_token(&token, &state.config.auth.jwt_secret) {
                        Ok(claims) => match Uuid::parse_str(&claims.sub) {
                            Ok(uid) => {
                                user_id = uid;
                                let mut normalized = machine;
                                if normalized.machine_id.is_nil() {
                                    normalized.machine_id = Uuid::new_v4();
                                }
                                machine_id = normalized.machine_id;
                                machine_info = normalized;
                                initial_projects = projects;
                                break;
                            }
                            Err(_) => {
                                let response = ServerToDaemon::RegistrationFailed {
                                    reason: "Invalid user ID in token".to_string(),
                                };
                                let text = serde_json::to_string(&response).unwrap();
                                let _ = sender.send(Message::Text(text.into())).await;
                                return;
                            }
                        },
                        Err(err) => {
                            let response = ServerToDaemon::RegistrationFailed {
                                reason: format!("Authentication failed: {}", err),
                            };
                            let text = serde_json::to_string(&response).unwrap();
                            let _ = sender.send(Message::Text(text.into())).await;
                            return;
                        }
                    },
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
    state.sessions.register_daemon(
        machine_id,
        user_id,
        tx,
        machine_info.clone(),
        initial_projects,
    );

    let registered_msg = ServerToDaemon::Registered { machine_id };
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
                            Ok(DaemonToServer::Heartbeat { projects }) => {
                                state.sessions.update_daemon_projects(&machine_id, projects);
                            }
                            Ok(DaemonToServer::Register {
                                machine, projects, ..
                            }) => {
                                // Ignore token re-auth, but accept machine/project refresh payload.
                                state.sessions.update_daemon_machine_info(&machine_id, machine);
                                state.sessions.update_daemon_projects(&machine_id, projects);
                            }
                            Ok(DaemonToServer::MachineInfoUpdate { machine }) => {
                                state.sessions.update_daemon_machine_info(&machine_id, machine);
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
