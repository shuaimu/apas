use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use shared::{CliToServer, ServerToCli, ServerToWeb};
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

    // Wait for registration message first
    let cli_id: Uuid;
    let user_id: Uuid;

    loop {
        match receiver.next().await {
            Some(Ok(Message::Text(text))) => {
                let parsed: Result<CliToServer, _> = serde_json::from_str(&text);
                match parsed {
                    Ok(CliToServer::Register { token: _ }) => {
                        // Dev mode: skip authentication, accept all connections
                        user_id = Uuid::new_v4();
                        cli_id = Uuid::new_v4();

                        // Send registration success
                        let response = ServerToCli::Registered { cli_id };
                        let text = serde_json::to_string(&response).unwrap();
                        if sender.send(Message::Text(text.into())).await.is_err() {
                            return;
                        }
                        tracing::info!("CLI client registered: {} (dev mode)", cli_id);
                        break;
                    }
                    _ => {
                        tracing::warn!("Expected Register message, got something else");
                        continue;
                    }
                }
            }
            Some(Ok(Message::Ping(data))) => {
                let _ = sender.send(Message::Pong(data)).await;
            }
            Some(Err(e)) => {
                tracing::error!("WebSocket error: {}", e);
                return;
            }
            None => return,
            _ => continue,
        }
    }

    // Channel for sending messages to this CLI client
    let (tx, mut rx) = mpsc::channel::<ServerToCli>(32);

    // Register this CLI connection
    state.sessions.register_cli(cli_id, tx);

    // Update database
    let cli_client = crate::db::CliClient {
        id: cli_id.to_string(),
        user_id: user_id.to_string(),
        name: None,
        last_seen: Some(chrono::Utc::now().to_rfc3339()),
        status: "online".to_string(),
        created_at: None,
    };
    let _ = state.db.upsert_cli_client(&cli_client).await;

    // Task to forward messages from channel to WebSocket
    let mut send_sender = sender;
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let text = serde_json::to_string(&msg).unwrap();
            if send_sender.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    // Handle incoming messages
    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(text) => {
                let parsed: Result<CliToServer, _> = serde_json::from_str(&text);
                match parsed {
                    Ok(CliToServer::SessionStart {
                        session_id,
                        working_dir: _,
                    }) => {
                        // CLI is starting a local session (hybrid mode)
                        state.sessions.create_cli_session(session_id, cli_id);
                        tracing::info!("CLI {} started local session {}", cli_id, session_id);
                    }
                    Ok(CliToServer::Output {
                        session_id,
                        data,
                        output_type,
                    }) => {
                        // Route output to web client (if attached)
                        state
                            .sessions
                            .route_to_web(
                                &session_id,
                                ServerToWeb::Output {
                                    content: data,
                                    output_type,
                                },
                            )
                            .await;
                    }
                    Ok(CliToServer::SessionEnd { session_id, reason }) => {
                        state
                            .sessions
                            .route_to_web(
                                &session_id,
                                ServerToWeb::SessionStatus {
                                    status: shared::SessionStatus::Ended,
                                },
                            )
                            .await;
                        tracing::info!("Session {} ended: {}", session_id, reason);
                    }
                    Ok(CliToServer::Heartbeat) => {
                        state
                            .sessions
                            .send_to_cli(&cli_id, ServerToCli::Heartbeat)
                            .await;
                    }
                    Ok(CliToServer::Register { .. }) => {
                        // Already registered, ignore
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse CLI message: {}", e);
                    }
                }
            }
            Message::Ping(_) => {
                // Pong is handled automatically
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    // Cleanup
    state.sessions.unregister_cli(&cli_id);
    let _ = state.db.update_cli_client_status(&cli_id.to_string(), "offline").await;
    send_task.abort();
    tracing::info!("CLI client disconnected: {}", cli_id);
}
