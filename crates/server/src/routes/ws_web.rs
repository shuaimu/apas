use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use shared::{ServerToCli, ServerToWeb, SessionStatus, WebToServer};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::state::AppState;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let connection_id = Uuid::new_v4();

    // Channel for sending messages to this web client
    let (tx, mut rx) = mpsc::channel::<ServerToWeb>(32);

    // Register this web connection
    state.sessions.register_web(connection_id, tx);

    // Task to forward messages from channel to WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let text = serde_json::to_string(&msg).unwrap();
            if sender.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    // Dev mode: auto-authenticate with a random user ID
    let user_id = Uuid::new_v4();
    let mut session_id: Option<Uuid> = None;

    tracing::info!("Web client connected: {} (dev mode)", connection_id);

    // Send authenticated message immediately
    state
        .sessions
        .send_to_web(&connection_id, ServerToWeb::Authenticated { user_id })
        .await;

    // Handle incoming messages
    while let Some(Ok(msg)) = receiver.next().await {
        if let Message::Text(text) = msg {
            let parsed: Result<WebToServer, _> = serde_json::from_str(&text);
            match parsed {
                Ok(WebToServer::Authenticate { token: _ }) => {
                    // Already authenticated in dev mode, just confirm
                    state
                        .sessions
                        .send_to_web(&connection_id, ServerToWeb::Authenticated { user_id })
                        .await;
                }
                Ok(WebToServer::ListCliClients) => {
                    let clients = state.sessions.get_cli_clients_info();
                    state
                        .sessions
                        .send_to_web(
                            &connection_id,
                            ServerToWeb::CliClients { clients },
                        )
                        .await;
                }
                Ok(WebToServer::StartSession { cli_client_id }) => {
                    let new_session_id = Uuid::new_v4();
                    session_id = Some(new_session_id);

                    // Create session in manager
                    state
                        .sessions
                        .create_session(new_session_id, user_id, connection_id);

                    // Try to assign a CLI client
                    let cli_id = cli_client_id.or_else(|| {
                        state.sessions.get_online_cli_ids().first().copied()
                    });

                    if let Some(cid) = cli_id {
                        state.sessions.assign_cli_to_session(&new_session_id, cid);
                        // Notify CLI about new session
                        state
                            .sessions
                            .send_to_cli(
                                &cid,
                                ServerToCli::SessionAssigned {
                                    session_id: new_session_id,
                                    working_dir: None,
                                },
                            )
                            .await;
                    }

                    // Notify web client
                    state
                        .sessions
                        .send_to_web(
                            &connection_id,
                            ServerToWeb::SessionStarted {
                                session_id: new_session_id,
                            },
                        )
                        .await;

                    let status = if cli_id.is_some() {
                        SessionStatus::Connected
                    } else {
                        SessionStatus::Pending
                    };
                    state
                        .sessions
                        .send_to_web(
                            &connection_id,
                            ServerToWeb::SessionStatus { status },
                        )
                        .await;

                    tracing::info!("Session started: {} (CLI: {:?})", new_session_id, cli_id);
                }
                Ok(WebToServer::Input { text }) => {
                    if let Some(sid) = session_id {
                        // Route input to CLI
                        let sent = state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::Input {
                                    session_id: sid,
                                    data: text,
                                },
                            )
                            .await;
                        if !sent {
                            state
                                .sessions
                                .send_to_web(
                                    &connection_id,
                                    ServerToWeb::Error {
                                        message: "CLI client not connected".to_string(),
                                    },
                                )
                                .await;
                        }
                    }
                }
                Ok(WebToServer::Signal { signal }) => {
                    if let Some(sid) = session_id {
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::Signal {
                                    session_id: sid,
                                    signal,
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::Approve { tool_call_id: _ }) => {
                    if let Some(sid) = session_id {
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::Input {
                                    session_id: sid,
                                    data: "y".to_string(),
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::Reject { tool_call_id: _ }) => {
                    if let Some(sid) = session_id {
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::Input {
                                    session_id: sid,
                                    data: "n".to_string(),
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::ResumeSession { session_id: sid }) => {
                    session_id = Some(sid);
                }
                Ok(WebToServer::AttachSession { session_id: sid }) => {
                    // Attach to an existing CLI session to observe output
                    if state.sessions.attach_web_to_session(&sid, connection_id) {
                        session_id = Some(sid);
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::SessionStarted { session_id: sid },
                            )
                            .await;
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::SessionStatus {
                                    status: shared::SessionStatus::Connected,
                                },
                            )
                            .await;
                        tracing::info!("Web client attached to CLI session {}", sid);
                    } else {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Session not found".to_string(),
                                },
                            )
                            .await;
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to parse message: {}", e);
                }
            }
        }
    }

    // Cleanup
    state.sessions.unregister_web(&connection_id);
    send_task.abort();
    tracing::info!("Web client disconnected: {}", connection_id);
}
