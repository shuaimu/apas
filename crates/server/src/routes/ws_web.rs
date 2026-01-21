use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use shared::{MessageInfo, ServerToCli, ServerToWeb, SessionInfo, SessionStatus, WebToServer};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::routes::auth::verify_token;
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

    // User must authenticate before accessing other features
    let mut user_id: Option<Uuid> = None;
    let mut session_id: Option<Uuid> = None;

    tracing::info!("Web client connected: {}", connection_id);

    // Handle incoming messages
    while let Some(Ok(msg)) = receiver.next().await {
        if let Message::Text(text) = msg {
            let parsed: Result<WebToServer, _> = serde_json::from_str(&text);
            match parsed {
                Ok(WebToServer::Authenticate { token }) => {
                    // Validate JWT token
                    match verify_token(&token, &state.config.auth.jwt_secret) {
                        Ok(claims) => {
                            match Uuid::parse_str(&claims.sub) {
                                Ok(uid) => {
                                    user_id = Some(uid);
                                    tracing::info!("Web client {} authenticated as user {}", connection_id, uid);
                                    state
                                        .sessions
                                        .send_to_web(&connection_id, ServerToWeb::Authenticated { user_id: uid })
                                        .await;
                                }
                                Err(_) => {
                                    state
                                        .sessions
                                        .send_to_web(
                                            &connection_id,
                                            ServerToWeb::AuthenticationFailed {
                                                reason: "Invalid user ID in token".to_string(),
                                            },
                                        )
                                        .await;
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Web client {} auth failed: {}", connection_id, e);
                            state
                                .sessions
                                .send_to_web(
                                    &connection_id,
                                    ServerToWeb::AuthenticationFailed {
                                        reason: e.to_string(),
                                    },
                                )
                                .await;
                        }
                    }
                }
                Ok(WebToServer::ListCliClients) => {
                    // Require authentication
                    let Some(uid) = user_id else {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Not authenticated".to_string(),
                                },
                            )
                            .await;
                        continue;
                    };

                    // Only return CLI clients owned by this user
                    let clients = state.sessions.get_cli_clients_info_for_user(&uid);
                    state
                        .sessions
                        .send_to_web(
                            &connection_id,
                            ServerToWeb::CliClients { clients },
                        )
                        .await;
                }
                Ok(WebToServer::StartSession { cli_client_id }) => {
                    // Require authentication
                    let Some(uid) = user_id else {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Not authenticated".to_string(),
                                },
                            )
                            .await;
                        continue;
                    };

                    let new_session_id = Uuid::new_v4();
                    session_id = Some(new_session_id);

                    // Create session in manager
                    state
                        .sessions
                        .create_session(new_session_id, uid, connection_id);

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
                                pane_type: None,
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
                Ok(WebToServer::Input { text, pane_type }) => {
                    if let Some(sid) = session_id {
                        // Route input to CLI (pane_type will be used for dual-pane routing)
                        let _ = pane_type; // TODO: Use pane_type for routing to correct session
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
                    // Check if user is authenticated and has access to this session
                    let Some(uid) = user_id else {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Not authenticated".to_string(),
                                },
                            )
                            .await;
                        continue;
                    };

                    // Check access (owner or shared)
                    let has_access = match state.db.check_session_access(&sid.to_string(), &uid.to_string()).await {
                        Ok(access) => access,
                        Err(e) => {
                            tracing::error!("Failed to check session access: {}", e);
                            false
                        }
                    };

                    if !has_access {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Access denied".to_string(),
                                },
                            )
                            .await;
                        continue;
                    }

                    // Attach to an existing CLI session to observe output
                    if state.sessions.attach_web_to_session(&sid, connection_id) {
                        session_id = Some(sid);
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::SessionStarted {
                                    session_id: sid,
                                    pane_type: None,
                                },
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

                        // Also load existing messages from file storage (limit to recent 100)
                        if let Ok((stored_messages, has_more)) = state.storage.get_messages_paginated(&sid, Some(100), None).await {
                            let messages: Vec<MessageInfo> = stored_messages
                                .into_iter()
                                .map(|m| MessageInfo {
                                    id: m.id,
                                    role: m.role,
                                    content: m.content,
                                    message_type: m.message_type,
                                    created_at: Some(m.created_at),
                                    pane_type: m.pane_type,
                                })
                                .collect();
                            state
                                .sessions
                                .send_to_web(
                                    &connection_id,
                                    ServerToWeb::SessionMessages { session_id: sid, messages, has_more },
                                )
                                .await;
                        }

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
                Ok(WebToServer::ListSessions) => {
                    // Require authentication
                    let Some(uid) = user_id else {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Not authenticated".to_string(),
                                },
                            )
                            .await;
                        continue;
                    };

                    // Get owned sessions for this user from database
                    let owned_sessions = match state.db.get_sessions_for_user(&uid.to_string()).await {
                        Ok(sessions) => sessions,
                        Err(e) => {
                            tracing::error!("Failed to get owned sessions: {}", e);
                            state
                                .sessions
                                .send_to_web(
                                    &connection_id,
                                    ServerToWeb::Error {
                                        message: "Failed to load sessions".to_string(),
                                    },
                                )
                                .await;
                            continue;
                        }
                    };

                    // Get shared sessions for this user
                    let shared_sessions = match state.db.get_shared_sessions_for_user(&uid.to_string()).await {
                        Ok(sessions) => sessions,
                        Err(e) => {
                            tracing::error!("Failed to get shared sessions: {}", e);
                            vec![] // Continue without shared sessions
                        }
                    };

                    // Combine owned and shared sessions
                    let mut sessions: Vec<SessionInfo> = owned_sessions
                        .into_iter()
                        .map(|s| SessionInfo {
                            id: Uuid::parse_str(&s.id).unwrap_or_default(),
                            cli_client_id: s.cli_client_id.and_then(|id| Uuid::parse_str(&id).ok()),
                            working_dir: s.working_dir,
                            hostname: s.hostname,
                            status: s.status,
                            created_at: s.created_at,
                            is_shared: false,
                            owner_email: None,
                        })
                        .collect();

                    // Add shared sessions with owner email
                    for (s, owner_email) in shared_sessions {
                        sessions.push(SessionInfo {
                            id: Uuid::parse_str(&s.id).unwrap_or_default(),
                            cli_client_id: s.cli_client_id.and_then(|id| Uuid::parse_str(&id).ok()),
                            working_dir: s.working_dir,
                            hostname: s.hostname,
                            status: s.status,
                            created_at: s.created_at,
                            is_shared: true,
                            owner_email: Some(owner_email),
                        });
                    }

                    state
                        .sessions
                        .send_to_web(&connection_id, ServerToWeb::Sessions { sessions })
                        .await;
                }
                Ok(WebToServer::GetSessionMessages { session_id: sid, limit, before_id }) => {
                    // Get messages for a specific session from file storage with pagination
                    let limit = limit.unwrap_or(100);
                    match state.storage.get_messages_paginated(&sid, Some(limit), before_id.as_deref()).await {
                        Ok((stored_messages, has_more)) => {
                            let messages: Vec<MessageInfo> = stored_messages
                                .into_iter()
                                .map(|m| MessageInfo {
                                    id: m.id,
                                    role: m.role,
                                    content: m.content,
                                    message_type: m.message_type,
                                    created_at: Some(m.created_at),
                                    pane_type: m.pane_type,
                                })
                                .collect();
                            state
                                .sessions
                                .send_to_web(
                                    &connection_id,
                                    ServerToWeb::SessionMessages { session_id: sid, messages, has_more },
                                )
                                .await;
                        }
                        Err(e) => {
                            tracing::error!("Failed to get messages from file: {}", e);
                            state
                                .sessions
                                .send_to_web(
                                    &connection_id,
                                    ServerToWeb::Error {
                                        message: "Failed to load messages".to_string(),
                                    },
                                )
                                .await;
                        }
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
