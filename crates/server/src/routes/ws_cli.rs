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

/// Minimum supported client version (YY.MM.COMMIT format)
/// Update this when making breaking API changes
const MIN_CLIENT_VERSION: &str = "26.01.0";

/// Parse version string (YY.MM.COMMIT) into comparable number
fn parse_version(v: &str) -> Option<u64> {
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let yy: u64 = parts[0].parse().ok()?;
    let mm: u64 = parts[1].parse().ok()?;
    let commit: u64 = parts[2].parse().ok()?;
    Some(yy * 1_000_000 + mm * 10_000 + commit)
}

/// Check if client version is supported
fn is_version_supported(client_version: &str) -> bool {
    let min = parse_version(MIN_CLIENT_VERSION);
    let client = parse_version(client_version);
    match (min, client) {
        (Some(m), Some(c)) => c >= m,
        _ => true, // Allow if we can't parse (be permissive)
    }
}

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
                    Ok(CliToServer::Register { token: _, version }) => {
                        // Check client version
                        let client_version = version.as_deref().unwrap_or("unknown");
                        if !is_version_supported(client_version) {
                            tracing::warn!(
                                "Client version {} is unsupported (min: {})",
                                client_version,
                                MIN_CLIENT_VERSION
                            );
                            let response = ServerToCli::VersionUnsupported {
                                client_version: client_version.to_string(),
                                min_version: MIN_CLIENT_VERSION.to_string(),
                            };
                            let text = serde_json::to_string(&response).unwrap();
                            let _ = sender.send(Message::Text(text.into())).await;
                            return;
                        }

                        // Dev mode: skip authentication, accept all connections
                        user_id = Uuid::new_v4();
                        cli_id = Uuid::new_v4();

                        // Send registration success
                        let response = ServerToCli::Registered { cli_id };
                        let text = serde_json::to_string(&response).unwrap();
                        if sender.send(Message::Text(text.into())).await.is_err() {
                            return;
                        }
                        tracing::info!("CLI client registered: {} (version: {}, dev mode)", cli_id, client_version);
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

    // Update database - first ensure user exists (dev mode creates random users)
    let dev_user = crate::db::User {
        id: user_id.to_string(),
        email: format!("dev-{}@local", user_id),
        password_hash: "dev".to_string(),
        created_at: None,
    };
    if let Err(e) = state.db.create_user(&dev_user).await {
        // Ignore duplicate user errors
        if !e.to_string().contains("UNIQUE constraint") {
            tracing::warn!("Failed to create dev user: {}", e);
        }
    }

    let cli_client = crate::db::CliClient {
        id: cli_id.to_string(),
        user_id: user_id.to_string(),
        name: None,
        last_seen: Some(chrono::Utc::now().to_rfc3339()),
        status: "online".to_string(),
        created_at: None,
    };
    if let Err(e) = state.db.upsert_cli_client(&cli_client).await {
        tracing::error!("Failed to upsert cli_client: {}", e);
    }

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
                        working_dir,
                        hostname,
                        pane_type: _,
                    }) => {
                        // CLI is starting a local session (hybrid mode)
                        state.sessions.create_cli_session(session_id, cli_id);

                        // Persist session to database
                        let session = crate::db::Session {
                            id: session_id.to_string(),
                            user_id: user_id.to_string(),
                            cli_client_id: Some(cli_id.to_string()),
                            working_dir,
                            hostname,
                            status: "active".to_string(),
                            created_at: None,
                            updated_at: None,
                        };
                        if let Err(e) = state.db.create_session(&session).await {
                            tracing::error!("Failed to persist session to database: {}", e);
                        }

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
                                    pane_type: None,
                                },
                            )
                            .await;
                    }
                    Ok(CliToServer::StreamMessage { session_id, message, pane_type }) => {
                        // Save message to file storage
                        if let Some(stored_message) = stream_message_to_stored(&session_id, &message) {
                            if let Err(e) = state.storage.append_message(&session_id, &stored_message).await {
                                tracing::error!("Failed to save message to file: {}", e);
                            }
                        }

                        // Route structured stream message to web client
                        state
                            .sessions
                            .route_to_web(
                                &session_id,
                                ServerToWeb::StreamMessage { session_id, message, pane_type },
                            )
                            .await;
                    }
                    Ok(CliToServer::UserInput { session_id, text, pane_type }) => {
                        tracing::info!("Received UserInput for session {}: {}", session_id, text);
                        // Save user input to file storage
                        let stored_message = crate::storage::StoredMessage {
                            id: Uuid::new_v4().to_string(),
                            role: "user".to_string(),
                            content: text.clone(),
                            message_type: "text".to_string(),
                            created_at: chrono::Utc::now().to_rfc3339(),
                        };
                        if let Err(e) = state.storage.append_message(&session_id, &stored_message).await {
                            tracing::error!("Failed to save user input to file: {}", e);
                        }

                        // Forward user input to web client
                        state
                            .sessions
                            .route_to_web(
                                &session_id,
                                ServerToWeb::UserInput { session_id, text, pane_type },
                            )
                            .await;
                    }
                    Ok(CliToServer::SessionEnd { session_id, reason }) => {
                        // Update session status in database
                        let _ = state.db.update_session_status(&session_id.to_string(), "ended").await;

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

    // Cleanup - mark all sessions for this CLI as inactive
    let session_ids = state.sessions.get_cli_session_ids(&cli_id);
    for session_id in &session_ids {
        if let Err(e) = state.db.update_session_status(&session_id.to_string(), "inactive").await {
            tracing::error!("Failed to update session {} status: {}", session_id, e);
        }
    }

    state.sessions.unregister_cli(&cli_id);
    let _ = state.db.update_cli_client_status(&cli_id.to_string(), "offline").await;
    send_task.abort();
    tracing::info!("CLI client disconnected: {} (marked {} sessions as inactive)", cli_id, session_ids.len());
}

/// Convert a ClaudeStreamMessage to a StoredMessage for file storage
fn stream_message_to_stored(session_id: &Uuid, message: &shared::ClaudeStreamMessage) -> Option<crate::storage::StoredMessage> {
    use shared::{ClaudeStreamMessage, ClaudeContentBlock};

    match message {
        ClaudeStreamMessage::Assistant { message: msg, .. } => {
            // Extract text content from assistant message
            let text_content: Vec<String> = msg.content.iter()
                .filter_map(|block| {
                    match block {
                        ClaudeContentBlock::Text { text } => Some(text.clone()),
                        ClaudeContentBlock::ToolUse { name, input, .. } => {
                            Some(format!("[Tool: {}] {}", name, serde_json::to_string(input).unwrap_or_default()))
                        }
                        _ => None,
                    }
                })
                .collect();

            if text_content.is_empty() {
                return None;
            }

            Some(crate::storage::StoredMessage {
                id: Uuid::new_v4().to_string(),
                role: "assistant".to_string(),
                content: text_content.join("\n"),
                message_type: "text".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
            })
        }
        ClaudeStreamMessage::Result { subtype, total_cost_usd, duration_ms, .. } => {
            Some(crate::storage::StoredMessage {
                id: Uuid::new_v4().to_string(),
                role: "system".to_string(),
                content: format!("{} - Cost: ${:.4}, Duration: {}ms", subtype, total_cost_usd, duration_ms),
                message_type: "result".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
            })
        }
        _ => None, // Skip system and user messages for now
    }
}
