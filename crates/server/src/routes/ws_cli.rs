use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use base64::Engine as _;
use futures::{SinkExt, StreamExt};
use shared::{CliToServer, PaneConfig, PaneType, ServerToCli, ServerToWeb, TerminalLifecycle};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::routes::auth::verify_token;
use crate::state::AppState;

async fn route_lifecycle_to_authorized_web(
    state: &AppState,
    session_id: Uuid,
    message: ServerToWeb,
) {
    let web_ids = state
        .sessions
        .get_session(&session_id)
        .map(|session| session.web_connection_ids)
        .unwrap_or_default();
    for web_id in web_ids {
        let authorized = match state.sessions.get_web_user(&web_id) {
            Some(user_id) => state
                .db
                .check_session_access(&session_id.to_string(), &user_id.to_string())
                .await
                .unwrap_or(false),
            None => false,
        };
        if authorized {
            state.sessions.send_to_web(&web_id, message.clone()).await;
        }
    }
}

/// Minimum supported client version (YY.MM.COMMIT format)
/// Update this when making breaking API changes
const MIN_CLIENT_VERSION: &str = "26.01.0";

/// How often to send ping frames to CLI clients
const PING_INTERVAL: Duration = Duration::from_secs(30);

/// How long to wait for any activity before considering connection dead
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(90);

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

fn is_terminal_pane(panes: &[PaneConfig], pane_id: u32) -> bool {
    panes
        .iter()
        .any(|pane| pane.pane_id == pane_id && pane.kind.is_terminal())
}

/// Whether this assistant message is an actual terminal-pane completion.
///
/// Updated CLIs include an explicit false marker on intermediate assistant
/// text (for example, before a tool call). Messages from older CLIs have no
/// marker, so retain the legacy behavior for those clients during rollout.
fn terminal_assistant_completes_work(
    message: &shared::ClaudeStreamMessage,
    panes: &[PaneConfig],
    pane_id: Option<u32>,
) -> bool {
    let Some(pane_id) = pane_id else {
        return false;
    };
    match message {
        shared::ClaudeStreamMessage::Assistant { extra, .. } => {
            // The explicit marker is emitted only by the terminal transcript
            // watcher, so it remains authoritative while pane metadata is
            // being repopulated after a server/CLI reconnect. Requiring the
            // pane list first stranded "Working..." when completion won that
            // race. Unmarked messages retain the pane-kind guard for rolling
            // compatibility with older clients.
            extra
                .get("terminal_turn_complete")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or_else(|| is_terminal_pane(panes, pane_id))
        }
        _ => false,
    }
}

pub(crate) async fn set_and_broadcast_pane_status(
    state: &AppState,
    session_id: Uuid,
    pane_type: PaneType,
    pane_id: u32,
    status: Option<String>,
) {
    state
        .sessions
        .set_pane_status(&session_id, pane_type, pane_id, status.clone());
    state
        .sessions
        .route_to_web(
            &session_id,
            ServerToWeb::PaneStatus {
                session_id,
                pane_type,
                pane_id: Some(pane_id),
                status,
            },
        )
        .await;
}

pub(crate) async fn handle_cli_user_input(
    state: &AppState,
    session_id: Uuid,
    text: String,
    pane_type: Option<PaneType>,
    pane_id: Option<u32>,
) {
    let Ok((_project_id, _operation_guard)) = state
        .active_session_operation(&session_id.to_string())
        .await
    else {
        return;
    };

    // Terminal conversation input is stored and echoed by the web route as
    // soon as the text and Enter reach the pty. The transcript watcher later
    // reports that same user turn through CliToServer::UserInput. Consume its
    // one-shot correlation before storage, broadcast, status, and accounting;
    // raw terminal/TUI input has no expectation and continues normally.
    let effective_pane_id =
        pane_id.or_else(|| pane_type.map(|pane| PaneConfig::pane_id_from_legacy(&pane)));
    if effective_pane_id.is_some_and(|pane_id| {
        state
            .sessions
            .consume_terminal_transcript_echo(&session_id, pane_id, &text)
    }) {
        tracing::debug!(
            %session_id,
            pane_id = effective_pane_id,
            "Consumed duplicate terminal transcript user turn"
        );
        return;
    }

    tracing::info!("Received UserInput for session {}: {}", session_id, text);
    let created_at = chrono::Utc::now().to_rfc3339();
    if let Err(error) = state
        .db
        .record_session_user_input(&session_id.to_string(), &created_at)
        .await
    {
        tracing::warn!(%error, %session_id, "failed to record session user activity");
    }
    let stored_message = crate::storage::StoredMessage {
        id: Uuid::new_v4().to_string(),
        role: "user".to_string(),
        content: text.clone(),
        message_type: "text".to_string(),
        created_at: created_at.clone(),
        pane_type: effective_pane_id.map(|id| id.to_string()),
    };
    if let Err(error) = state
        .storage
        .append_message(&session_id, &stored_message)
        .await
    {
        tracing::error!("Failed to save user input to file: {error}");
    }

    state
        .sessions
        .route_to_web(
            &session_id,
            ServerToWeb::UserInput {
                session_id,
                text,
                pane_type,
                pane_id,
                created_at: Some(created_at),
                client_msg_id: None,
            },
        )
        .await;

    if let Some(pane_id) = effective_pane_id
        .filter(|id| is_terminal_pane(&state.sessions.get_session_panes(&session_id), *id))
    {
        // User turns harvested from a raw terminal still start the coarse
        // working state. Web-originated turns already did this in ws_web.
        set_and_broadcast_pane_status(
            state,
            session_id,
            PaneType::Interactive,
            pane_id,
            Some("Working...".to_string()),
        )
        .await;
    }

    record_and_broadcast_usage(
        state,
        session_id,
        effective_pane_id,
        crate::db::UsageDelta {
            prompt_count: 1,
            ..Default::default()
        },
    )
    .await;
}

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Wait for registration message first
    let cli_id: Uuid;
    let user_id: Uuid;
    let cli_version: Option<String>;
    let cli_capabilities: Vec<String>;

    loop {
        match receiver.next().await {
            Some(Ok(Message::Text(text))) => {
                let parsed: Result<CliToServer, _> = serde_json::from_str(&text);
                match parsed {
                    Ok(CliToServer::Register {
                        token,
                        version,
                        capabilities,
                    }) => {
                        // Check client version
                        let client_version =
                            version.clone().unwrap_or_else(|| "unknown".to_string());
                        if !is_version_supported(&client_version) {
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

                        // Validate JWT token
                        match verify_token(&token, &state.config.auth.jwt_secret) {
                            Ok(claims) => {
                                match Uuid::parse_str(&claims.sub) {
                                    Ok(uid) => {
                                        match state.db.get_user_by_id(&uid.to_string()).await {
                                            Ok(Some(user)) if user.is_active() => {}
                                            Ok(Some(_)) => {
                                                let response = ServerToCli::RegistrationFailed {
                                                    reason: "Cluster account is suspended"
                                                        .to_string(),
                                                };
                                                let text =
                                                    serde_json::to_string(&response).unwrap();
                                                let _ =
                                                    sender.send(Message::Text(text.into())).await;
                                                return;
                                            }
                                            Ok(None) => {
                                                let response = ServerToCli::RegistrationFailed {
                                                    reason: "Cluster account not found".to_string(),
                                                };
                                                let text =
                                                    serde_json::to_string(&response).unwrap();
                                                let _ =
                                                    sender.send(Message::Text(text.into())).await;
                                                return;
                                            }
                                            Err(err) => {
                                                tracing::warn!(
                                                    "CLI account lookup failed: {}",
                                                    err
                                                );
                                                let response = ServerToCli::RegistrationFailed {
                                                    reason: "Could not load cluster account"
                                                        .to_string(),
                                                };
                                                let text =
                                                    serde_json::to_string(&response).unwrap();
                                                let _ =
                                                    sender.send(Message::Text(text.into())).await;
                                                return;
                                            }
                                        }
                                        user_id = uid;
                                        cli_id = Uuid::new_v4();
                                        cli_version = version;
                                        cli_capabilities = capabilities;

                                        // Send registration success
                                        let response = ServerToCli::Registered { cli_id };
                                        let text = serde_json::to_string(&response).unwrap();
                                        if sender.send(Message::Text(text.into())).await.is_err() {
                                            return;
                                        }
                                        tracing::info!(
                                            "CLI client registered: {} (version: {}, user: {})",
                                            cli_id,
                                            client_version,
                                            user_id
                                        );
                                        break;
                                    }
                                    Err(_) => {
                                        let response = ServerToCli::RegistrationFailed {
                                            reason: "Invalid user ID in token".to_string(),
                                        };
                                        let text = serde_json::to_string(&response).unwrap();
                                        let _ = sender.send(Message::Text(text.into())).await;
                                        return;
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("CLI registration failed: {}", e);
                                let response = ServerToCli::RegistrationFailed {
                                    reason: format!("Authentication failed: {}", e),
                                };
                                let text = serde_json::to_string(&response).unwrap();
                                let _ = sender.send(Message::Text(text.into())).await;
                                return;
                            }
                        }
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

    // Register this CLI connection with user association
    state
        .sessions
        .register_cli(cli_id, user_id, tx, cli_version);
    state
        .sessions
        .set_cli_capabilities(cli_id, cli_capabilities);

    // Update database - first ensure user exists (dev mode creates random users)
    let dev_user = crate::db::User {
        id: user_id.to_string(),
        email: format!("dev-{}@local", user_id),
        password_hash: "dev".to_string(),
        created_at: None,
        cluster_role: "user".to_string(),
        account_status: "active".to_string(),
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

    // Track last activity for timeout detection
    let mut last_activity = Instant::now();
    let mut ping_interval = tokio::time::interval(PING_INTERVAL);
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Main message handling loop with ping/timeout
    loop {
        tokio::select! {
            // Handle outgoing messages from channel
            Some(msg) = rx.recv() => {
                let text = serde_json::to_string(&msg).unwrap();
                if sender.send(Message::Text(text.into())).await.is_err() {
                    tracing::warn!("CLI {} send failed, closing connection", cli_id);
                    break;
                }
            }

            // Periodic ping to detect dead connections
            _ = ping_interval.tick() => {
                // Check if connection has timed out
                if last_activity.elapsed() > CONNECTION_TIMEOUT {
                    tracing::warn!("CLI {} connection timed out (no activity for {:?})", cli_id, last_activity.elapsed());
                    break;
                }

                // Send ping frame
                if sender.send(Message::Ping(vec![].into())).await.is_err() {
                    tracing::warn!("CLI {} ping failed, closing connection", cli_id);
                    break;
                }
            }

            // Handle incoming messages
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        last_activity = Instant::now();
                        // Account/project suspension removes the sender from
                        // the manager. Do not let a peer that ignores the
                        // rejection continue mutating through this socket.
                        if !state.sessions.is_cli_connected(&cli_id) {
                            break;
                        }
                        let parsed: Result<CliToServer, _> = serde_json::from_str(&text);
                        match parsed {
                            Ok(CliToServer::SessionStart {
                                session_id,
                                project_id,
                                working_dir,
                                hostname,
                                git_remote,
                                git_remote_url,
                                pane_type: _,
                                panes,
                            }) => {
                                // Older CLIs omit project_id; preserve the
                                // historical 1:1 mapping where the .apas id
                                // also served as the session id.
                                let project_id = project_id.unwrap_or(session_id);
                                let _project_guard = state
                                    .project_operation_guard(&project_id.to_string())
                                    .await;
                                if let Err(err) = state
                                    .db
                                    .authorize_project_registration(
                                        &project_id.to_string(),
                                        &user_id.to_string(),
                                    )
                                    .await
                                {
                                    let reason = err.to_string();
                                    tracing::warn!(
                                        "Rejecting SessionStart from CLI {} (user {}, project {}): {}",
                                        cli_id,
                                        user_id,
                                        project_id,
                                        reason
                                    );
                                    state
                                        .sessions
                                        .send_to_cli(
                                            &cli_id,
                                            ServerToCli::SessionRejected {
                                                session_id,
                                                reason,
                                            },
                                        )
                                        .await;
                                    continue;
                                }
                                // Reject if this session_id is already owned by a different user
                                // (typically caused by .apas files copied/shared between users).
                                if let Ok(Some(existing)) =
                                    state.db.get_session(&session_id.to_string()).await
                                {
                                    if existing.user_id != user_id.to_string() {
                                        let owner_email = state
                                            .db
                                            .get_session_owner_info(&session_id.to_string())
                                            .await
                                            .ok()
                                            .flatten()
                                            .map(|(_, email)| email)
                                            .unwrap_or_else(|| existing.user_id.clone());
                                        let reason = format!(
                                            "Session {} is already owned by another user ({}). \
                                             This usually means the .apas file was copied from \
                                             another user. Delete the .apas file in your project \
                                             directory so a fresh one is generated.",
                                            session_id, owner_email
                                        );
                                        tracing::warn!(
                                            "Rejecting SessionStart from CLI {} (user {}): {}",
                                            cli_id, user_id, reason
                                        );
                                        state
                                            .sessions
                                            .send_to_cli(
                                                &cli_id,
                                                ServerToCli::SessionRejected {
                                                    session_id,
                                                    reason,
                                                },
                                            )
                                            .await;
                                        continue;
                                    }
                                }

                                // CLI is starting a local session (hybrid mode)
                                state.sessions.create_cli_session(
                                    session_id,
                                    cli_id,
                                    working_dir.clone(),
                                    hostname.clone(),
                                );
                                state
                                    .sessions
                                    .set_session_project(session_id, project_id.to_string());

                                // Cache initial pane list if provided, preserving persisted labels/order
                                if let Some(pane_list) = &panes {
                                    if !pane_list.is_empty() {
                                        let mut normalized_panes = pane_list.clone();
                                        // Recover custom labels/order from persisted file
                                        if let Ok(stored) = state.storage.load_pane_list(&session_id).await {
                                            if !stored.is_empty() {
                                                let label_map: std::collections::HashMap<u32, String> = stored
                                                    .iter()
                                                    .filter_map(|p| p.label.as_ref().map(|l| (p.pane_id, l.clone())))
                                                    .collect();
                                                for pane in &mut normalized_panes {
                                                    if let Some(label) = label_map.get(&pane.pane_id) {
                                                        pane.label = Some(label.clone());
                                                    }
                                                }
                                                // Reorder to match persisted order; new panes appended
                                                let existing_order: Vec<u32> = stored.iter().map(|p| p.pane_id).collect();
                                                let mut reordered: Vec<shared::PaneConfig> = Vec::new();
                                                for &id in &existing_order {
                                                    if let Some(p) = normalized_panes.iter().find(|p| p.pane_id == id) {
                                                        reordered.push(p.clone());
                                                    }
                                                }
                                                for p in &normalized_panes {
                                                    if !existing_order.contains(&p.pane_id) {
                                                        reordered.push(p.clone());
                                                    }
                                                }
                                                if reordered.len() == normalized_panes.len() {
                                                    normalized_panes = reordered;
                                                }
                                            }
                                        }
                                        state
                                            .sessions
                                            .set_session_panes(&session_id, normalized_panes.clone());
                                        if let Err(e) = state
                                            .storage
                                            .save_pane_list(&session_id, &normalized_panes)
                                            .await
                                        {
                                            tracing::warn!(
                                                "Failed to persist initial pane list for session {}: {}",
                                                session_id,
                                                e
                                            );
                                        }
                                        // Forward to any already-attached web clients
                                        state
                                            .sessions
                                            .route_to_web(
                                                &session_id,
                                                ServerToWeb::PaneList {
                                                    session_id,
                                                    panes: normalized_panes,
                                                },
                                            )
                                            .await;
                                    }
                                }

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
                                    is_paused: false,
                                    project_id: Some(project_id.to_string()),
                                    git_remote,
                                    git_remote_url,
                                };
                                if let Err(e) = state.db.create_session(&session).await {
                                    tracing::error!("Failed to persist session to database: {}", e);
                                }

                                if state.sessions.session_supports_capability(
                                    &session_id,
                                    shared::PROJECT_POLICY_CAPABILITY,
                                ) {
                                    match state
                                        .db
                                        .get_effective_project_policy(&project_id.to_string())
                                        .await
                                    {
                                        Ok(policy) => {
                                            state
                                                .sessions
                                                .send_to_cli(
                                                    &cli_id,
                                                    ServerToCli::ProjectPolicy {
                                                        session_id,
                                                        policy,
                                                    },
                                                )
                                                .await;
                                        }
                                        Err(err) => tracing::warn!(
                                            "Failed to load policy for project {}: {}",
                                            project_id,
                                            err
                                        ),
                                    }
                                }

                                // If this project got a regenerated session ID, carry over history
                                // from the latest prior project session into this new session ID.
                                match state
                                    .db
                                    .get_latest_project_session_id(
                                        &user_id.to_string(),
                                        session.working_dir.as_deref(),
                                        session.hostname.as_deref(),
                                        &session_id.to_string(),
                                    )
                                    .await
                                {
                                    Ok(Some(previous_sid)) => {
                                        match Uuid::parse_str(&previous_sid) {
                                            Ok(previous_uuid) => {
                                                match state
                                                    .storage
                                                    .seed_history_if_missing(&previous_uuid, &session_id)
                                                    .await
                                                {
                                                    Ok(true) => {
                                                        tracing::info!(
                                                            "Seeded history for regenerated project session {} from {}",
                                                            session_id,
                                                            previous_sid
                                                        );
                                                    }
                                                    Ok(false) => {}
                                                    Err(err) => {
                                                        tracing::warn!(
                                                            "Failed to seed history for session {} from {}: {}",
                                                            session_id,
                                                            previous_sid,
                                                            err
                                                        );
                                                    }
                                                }
                                            }
                                            Err(err) => {
                                                tracing::warn!(
                                                    "Invalid previous session id {} while seeding history for {}: {}",
                                                    previous_sid,
                                                    session_id,
                                                    err
                                                );
                                            }
                                        }
                                    }
                                    Ok(None) => {}
                                    Err(err) => {
                                        tracing::warn!(
                                            "Failed to look up previous project session for {}: {}",
                                            session_id,
                                            err
                                        );
                                    }
                                }

                                // Notify any already-attached web clients that CLI is (re)connected
                                state.sessions.route_to_web(
                                    &session_id,
                                    ServerToWeb::SessionAttached {
                                        session_id,
                                        has_active_cli: true,
                                    },
                                ).await;

                                tracing::info!("CLI {} started local session {}", cli_id, session_id);
                            }
                            Ok(CliToServer::Output {
                                session_id,
                                data,
                                output_type,
                                pane_type,
                                pane_id,
                            }) => {
                                let Ok((project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                let approval = match &output_type {
                                    shared::OutputType::ApprovalRequest { tool_call_id, .. } => {
                                        Some((
                                            tool_call_id.clone(),
                                            format!("approval:{session_id}:{tool_call_id}"),
                                        ))
                                    }
                                    _ => None,
                                };
                                if let Some((tool_call_id, _)) = &approval {
                                    state.sessions.register_pending_decision(
                                        session_id,
                                        tool_call_id.clone(),
                                        pane_id,
                                        shared::MutationKind::Approval,
                                    );
                                }
                                // Char-boundary-safe slice: plain byte index can panic
                                // mid-codepoint on multibyte content (e.g. `…`).
                                let preview_end = {
                                    let mut end = data.len().min(50);
                                    while end > 0 && !data.is_char_boundary(end) { end -= 1; }
                                    end
                                };
                                tracing::info!("Received Output for session {} with pane_id {:?}: {}", session_id, pane_id, &data[..preview_end]);
                                // Route output to web client (if attached)
                                let routed = state
                                    .sessions
                                    .route_to_web(
                                        &session_id,
                                        ServerToWeb::Output {
                                            session_id: Some(session_id),
                                            content: data,
                                            output_type,
                                            pane_type,
                                            pane_id,
                                        },
                                    )
                                    .await;
                                tracing::info!("Output routed to web: {}", routed);
                                if let Some((_, dedupe_key)) = approval {
                                    let routing_id = Uuid::new_v4().to_string();
                                    if let Err(error) = crate::notifications::enqueue_project_event(
                                        &state,
                                        &project_id,
                                        Some(&session_id.to_string()),
                                        pane_id,
                                        "decision",
                                        &routing_id,
                                        &dedupe_key,
                                    )
                                    .await
                                    {
                                        tracing::warn!(%error, %session_id, "failed to enqueue approval notification");
                                    }
                                }
                            }
                            Ok(CliToServer::StreamMessage { session_id, message, pane_type, pane_id }) => {
                                let Ok((project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                tracing::info!("Received StreamMessage for session {} with pane_id {:?}", session_id, pane_id);

                                let pending_question = match &message {
                                    shared::ClaudeStreamMessage::Assistant { message, .. } => message
                                        .content
                                        .iter()
                                        .find_map(|block| match block {
                                            shared::ClaudeContentBlock::ToolUse { id, name, .. }
                                                if name.eq_ignore_ascii_case("AskUserQuestion") =>
                                            {
                                                Some(id.clone())
                                            }
                                            _ => None,
                                        }),
                                    _ => None,
                                };
                                let notification = match &message {
                                    shared::ClaudeStreamMessage::Result {
                                        subtype,
                                        session_id: provider_session_id,
                                        duration_ms,
                                        is_error,
                                        ..
                                    } => Some((
                                        if *is_error || subtype != "success" { "failure" } else { "completion" },
                                        format!("result:{session_id}:{provider_session_id}:{subtype}:{duration_ms}"),
                                    )),
                                    shared::ClaudeStreamMessage::Assistant { message, .. } => message
                                        .content
                                        .iter()
                                        .find_map(|block| match block {
                                            shared::ClaudeContentBlock::ToolUse { id, name, .. }
                                                if name.eq_ignore_ascii_case("AskUserQuestion") =>
                                            {
                                                Some(("decision", format!("question:{session_id}:{id}")))
                                            }
                                            _ => None,
                                        }),
                                    _ => None,
                                };

                                // Use pane_id for storage, falling back to pane_type for backward compat
                                let effective_pane_id = pane_id.or_else(|| pane_type.map(|p| shared::PaneConfig::pane_id_from_legacy(&p)));
                                let terminal_assistant_reply = terminal_assistant_completes_work(
                                    &message,
                                    &state.sessions.get_session_panes(&session_id),
                                    effective_pane_id,
                                );
                                if let Some(tool_use_id) = pending_question {
                                    state.sessions.register_pending_decision(
                                        session_id,
                                        tool_use_id,
                                        effective_pane_id,
                                        shared::MutationKind::Question,
                                    );
                                }

                                // A `result` stream event ends a turn and carries the turn's
                                // token usage (in `extra.usage`) + cost. Capture it as a usage
                                // delta before `message` is moved into the web broadcast below.
                                let usage_delta = if let shared::ClaudeStreamMessage::Result {
                                    extra,
                                    total_cost_usd,
                                    subtype,
                                    ..
                                } = &message
                                {
                                    let success = subtype.as_str() == "success";
                                    let usage = extra.get("usage");
                                    let token = |key: &str| {
                                        usage
                                            .and_then(|u| u.get(key))
                                            .and_then(serde_json::Value::as_u64)
                                            .unwrap_or(0) as i64
                                    };
                                    Some(crate::db::UsageDelta {
                                        prompt_count: 0,
                                        input_tokens: token("input_tokens"),
                                        output_tokens: token("output_tokens"),
                                        cache_read_tokens: token("cache_read_input_tokens"),
                                        cache_creation_tokens: token("cache_creation_input_tokens"),
                                        // Cost is billed even for error_max_turns /
                                        // error_during_execution turns, so record it
                                        // regardless of subtype; only count a *completed*
                                        // response on success.
                                        total_cost_usd: *total_cost_usd,
                                        num_responses: if success { 1 } else { 0 },
                                    })
                                } else {
                                    None
                                };

                                // Save message(s) to file storage. Remember the max created_at
                                // of the stored fragments so we can hand it to the web client as
                                // a reconnect high-water mark.
                                let mut max_created_at: Option<String> = None;
                                for stored_message in stream_message_to_stored(&session_id, &message, effective_pane_id) {
                                    match &max_created_at {
                                        Some(prev) if prev.as_str() >= stored_message.created_at.as_str() => {}
                                        _ => max_created_at = Some(stored_message.created_at.clone()),
                                    }
                                    if let Err(e) = state.storage.append_message(&session_id, &stored_message).await {
                                        tracing::error!("Failed to save message to file: {}", e);
                                    }
                                }

                                // Route structured stream message to web client
                                let routed = state
                                    .sessions
                                    .route_to_web(
                                        &session_id,
                                        ServerToWeb::StreamMessage { session_id, message, pane_type, pane_id, created_at: max_created_at },
                                    )
                                    .await;
                                tracing::info!("StreamMessage routed to web: {}", routed);

                                if terminal_assistant_reply {
                                    if let Some(pane_id) = effective_pane_id {
                                        // Updated terminal clients only mark the
                                        // provider's real completion boundary. Older
                                        // clients retain their previous behavior
                                        // during the rolling upgrade.
                                        set_and_broadcast_pane_status(
                                            &state,
                                            session_id,
                                            PaneType::Interactive,
                                            pane_id,
                                            None,
                                        )
                                        .await;
                                    }
                                }

                                if let Some((category, dedupe_key)) = notification {
                                    let routing_id = Uuid::new_v4().to_string();
                                    if let Err(error) = crate::notifications::enqueue_project_event(
                                        &state,
                                        &project_id,
                                        Some(&session_id.to_string()),
                                        effective_pane_id,
                                        category,
                                        &routing_id,
                                        &dedupe_key,
                                    )
                                    .await
                                    {
                                        tracing::warn!(%error, %session_id, "failed to enqueue mobile coding notification");
                                    }
                                }

                                // Record the turn's usage and push refreshed project
                                // stats to the Overview. `result` arrives once per turn,
                                // so this is not chatty.
                                if let Some(delta) = usage_delta {
                                    record_and_broadcast_usage(&state, session_id, effective_pane_id, delta).await;
                                }
                            }
                            Ok(CliToServer::UserInput { session_id, text, pane_type, pane_id }) => {
                                handle_cli_user_input(&state, session_id, text, pane_type, pane_id).await;
                            }
                            Ok(CliToServer::SessionEnd { session_id, reason }) => {
                                let Ok((_project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
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
                            Ok(CliToServer::DeadloopStatus { session_id, is_paused }) => {
                                let Ok((_project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                // Save pause state for the session (in-memory)
                                state.sessions.set_session_paused(&session_id, is_paused);
                                // Persist to database for server restart recovery
                                if let Err(e) = state.db.update_session_paused(&session_id.to_string(), is_paused).await {
                                    tracing::error!("Failed to persist pause status to database: {}", e);
                                }
                                // Forward deadloop status to web clients
                                tracing::info!("Deadloop status for session {}: paused={}", session_id, is_paused);
                                state
                                    .sessions
                                    .route_to_web(
                                        &session_id,
                                        ServerToWeb::DeadloopStatus {
                                            session_id,
                                            is_paused,
                                        },
                                    )
                                    .await;
                            }
                            Ok(CliToServer::PaneStatus { session_id, pane_type, pane_id, status }) => {
                                let Ok((_project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                // Forward pane status to web clients
                                tracing::info!("Pane status for session {}: pane_id={:?} = {:?}", session_id, pane_id, status);
                                // Cache so we can replay to web clients that attach
                                // later (tab switch, sidebar switch) — the CLI won't
                                // re-send the current status until it changes.
                                let resolved_pane_id = pane_id
                                    .unwrap_or_else(|| shared::PaneConfig::pane_id_from_legacy(&pane_type));
                                state.sessions.set_pane_status(
                                    &session_id,
                                    pane_type,
                                    resolved_pane_id,
                                    status.clone(),
                                );
                                state
                                    .sessions
                                    .route_to_web(
                                        &session_id,
                                        ServerToWeb::PaneStatus {
                                            session_id,
                                            pane_type,
                                            pane_id,
                                            status,
                                        },
                                    )
                                    .await;
                            }
                            Ok(CliToServer::PanePaused { session_id, pane_id, is_paused }) => {
                                let Ok((_project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                // Save pause state
                                state.sessions.set_session_paused(&session_id, is_paused);
                                if let Err(e) = state.db.update_session_paused(&session_id.to_string(), is_paused).await {
                                    tracing::error!("Failed to persist pause status to database: {}", e);
                                }
                                tracing::info!("Pane {} paused={} for session {}", pane_id, is_paused, session_id);
                                state
                                    .sessions
                                    .route_to_web(
                                        &session_id,
                                        ServerToWeb::PanePaused {
                                            session_id,
                                            pane_id,
                                            is_paused,
                                        },
                                    )
                                    .await;
                            }
                            Ok(CliToServer::UsageLimits { provider, limits }) => {
                                if shared::is_retired_provider(provider) {
                                    tracing::warn!(
                                        cli_id = %cli_id,
                                        "ignored usage limits for a retired provider"
                                    );
                                    continue;
                                }
                                // Update and broadcast usage limits
                                tracing::info!(
                                    "Usage limits from CLI {} ({}): 5h={:.1}%, 7d={:.1}%",
                                    cli_id,
                                    format!("{:?}", provider).to_lowercase(),
                                    limits.five_hour.as_ref().map(|w| w.utilization * 100.0).unwrap_or(0.0),
                                    limits.seven_day.as_ref().map(|w| w.utilization * 100.0).unwrap_or(0.0)
                                );
                                state.sessions.update_usage_limits(cli_id, provider, limits);
                            }
                            Ok(CliToServer::PlanReviewRequest { session_id, pane_id, tool_use_id, tool_name, input }) => {
                                let Ok((project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                tracing::info!(
                                    "Plan review requested for session {} pane {} tool {} (id={})",
                                    session_id, pane_id, tool_name, tool_use_id,
                                );
                                let notification_dedupe = format!("plan:{session_id}:{tool_use_id}");
                                state.sessions.register_pending_decision(
                                    session_id,
                                    tool_use_id.clone(),
                                    Some(pane_id),
                                    shared::MutationKind::PlanReview,
                                );
                                state
                                    .sessions
                                    .route_to_web(
                                        &session_id,
                                        ServerToWeb::PlanReviewRequest {
                                            session_id,
                                            pane_id,
                                            tool_use_id,
                                            tool_name,
                                            input,
                                        },
                                    )
                                    .await;
                                let routing_id = Uuid::new_v4().to_string();
                                if let Err(error) = crate::notifications::enqueue_project_event(
                                    &state,
                                    &project_id,
                                    Some(&session_id.to_string()),
                                    Some(pane_id),
                                    "decision",
                                    &routing_id,
                                    &notification_dedupe,
                                )
                                .await
                                {
                                    tracing::warn!(%error, %session_id, pane_id, "failed to enqueue plan notification");
                                }
                            }
                            Ok(CliToServer::TerminalOutput { session_id, pane_id, instance_id, data_b64, seq }) => {
                                let Ok((_project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                // Buffer for reattach, then fan out live.
                                // Decoded only to measure/store bytes — the
                                // server never interprets the stream.
                                let accepted = match base64::engine::general_purpose::STANDARD.decode(data_b64.as_bytes()) {
                                    Ok(bytes) => {
                                        state.sessions.append_terminal_output(
                                            &session_id,
                                            pane_id,
                                            instance_id,
                                            &bytes,
                                            seq,
                                        )
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            pane_id,
                                            error = %e,
                                            "terminal output was not valid base64; dropping frame"
                                        );
                                        false
                                    }
                                };
                                if !accepted {
                                    continue;
                                }
                                state
                                    .sessions
                                    .route_to_web(
                                        &session_id,
                                        ServerToWeb::TerminalOutput {
                                            session_id,
                                            pane_id,
                                            instance_id,
                                            data_b64,
                                            seq,
                                        },
                                    )
                                    .await;
                            }
                            Ok(CliToServer::TerminalState { session_id, pane_id, instance_id, lifecycle, status, runtime }) => {
                                let Ok((_project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                if let Some(current) = state.sessions.reconcile_terminal_state(
                                    &session_id,
                                    pane_id,
                                    instance_id,
                                    lifecycle,
                                    status,
                                    runtime,
                                ) {
                                    state
                                        .sessions
                                        .route_to_web(
                                            &session_id,
                                            ServerToWeb::TerminalState {
                                                session_id,
                                                pane_id,
                                                instance_id: current.instance_id,
                                                lifecycle: current.lifecycle,
                                                status: current.status,
                                                runtime: current.runtime,
                                            },
                                        )
                                        .await;
                                    if current.lifecycle == TerminalLifecycle::Exited {
                                        set_and_broadcast_pane_status(
                                            &state,
                                            session_id,
                                            PaneType::Interactive,
                                            pane_id,
                                            None,
                                        )
                                        .await;
                                    }
                                } else {
                                    tracing::debug!(
                                        pane_id,
                                        ?instance_id,
                                        ?lifecycle,
                                        "ignored stale terminal state"
                                    );
                                }
                            }
                            Ok(CliToServer::CliLifecycleInventory { session_id, inventory }) => {
                                let Ok((_project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                state.sessions.set_lifecycle_inventory(session_id, inventory.clone());
                                route_lifecycle_to_authorized_web(
                                    &state,
                                    session_id,
                                    ServerToWeb::CliLifecycleInventory { session_id, inventory },
                                )
                                .await;
                            }
                            Ok(CliToServer::CliLifecycleStatus {
                                session_id,
                                request_id,
                                operation,
                                phase,
                                message,
                                inventory,
                            }) => {
                                let Ok((_project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                let Some(status) = state.sessions.update_lifecycle_request(
                                    request_id,
                                    session_id,
                                    operation,
                                    phase,
                                    message,
                                    inventory,
                                ) else {
                                    tracing::warn!(%session_id, %request_id, ?operation, ?phase, "ignored unknown or mismatched CLI lifecycle status");
                                    continue;
                                };
                                tracing::info!(
                                    %session_id,
                                    %request_id,
                                    ?operation,
                                    ?phase,
                                    duration_ms = status.created_at.elapsed().as_millis(),
                                    "CLI lifecycle progress"
                                );
                                route_lifecycle_to_authorized_web(&state, session_id, status.message()).await;
                            }
                            Ok(CliToServer::TerminalExited { session_id, pane_id, instance_id, status }) => {
                                let Ok((_project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                tracing::info!(
                                    pane_id,
                                    ?status,
                                    "terminal pane exited for session {}",
                                    session_id
                                );
                                let Some(current) = state.sessions.record_terminal_exit(
                                    &session_id,
                                    pane_id,
                                    instance_id,
                                    status,
                                ) else {
                                    tracing::debug!(pane_id, ?instance_id, "ignored stale terminal exit");
                                    continue;
                                };
                                state
                                    .sessions
                                    .route_to_web(
                                        &session_id,
                                        ServerToWeb::TerminalState {
                                            session_id,
                                            pane_id,
                                            instance_id: current.instance_id,
                                            lifecycle: TerminalLifecycle::Exited,
                                            status: current.status.clone(),
                                            runtime: current.runtime.clone(),
                                        },
                                    )
                                    .await;
                                set_and_broadcast_pane_status(
                                    &state,
                                    session_id,
                                    PaneType::Interactive,
                                    pane_id,
                                    None,
                                )
                                .await;
                                state
                                    .sessions
                                    .route_to_web(
                                        &session_id,
                                        ServerToWeb::TerminalExited {
                                            session_id,
                                            pane_id,
                                            instance_id: current.instance_id,
                                            status: current.status,
                                        },
                                    )
                                    .await;
                            }
                            Ok(CliToServer::PaneDiff { session_id, pane_id, branch, base, diff, error }) => {
                                let Ok((_project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                tracing::info!(
                                    "Pane diff for pane {} in session {} ({} bytes)",
                                    pane_id, session_id,
                                    diff.as_ref().map(|s| s.len()).unwrap_or(0),
                                );
                                state
                                    .sessions
                                    .route_to_web(
                                        &session_id,
                                        ServerToWeb::PaneDiff {
                                            session_id,
                                            pane_id,
                                            branch,
                                            base,
                                            diff,
                                            error,
                                        },
                                    )
                                    .await;
                            }
                            Ok(CliToServer::ProjectFlagsChanged {
                                session_id,
                                auto_approve_todos,
                                auto_merge_prs,
                                team_enabled,
                                disallowed_tab_types,
                            }) => {
                                let Ok((_project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                tracing::debug!(
                                    %session_id,
                                    auto_approve_todos,
                                    auto_merge_prs,
                                    team_enabled,
                                    "Project flags changed",
                                );
                                state
                                    .sessions
                                    .route_to_web(
                                        &session_id,
                                        ServerToWeb::ProjectFlagsChanged {
                                            session_id,
                                            auto_approve_todos,
                                            auto_merge_prs,
                                            team_enabled,
                                            disallowed_tab_types,
                                        },
                                    )
                                    .await;
                            }
                            Ok(CliToServer::PrCreated { session_id, pane_id, url, error }) => {
                                let Ok((project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                tracing::info!(
                                    "PR created for pane {} in session {}: url={:?} error={:?}",
                                    pane_id, session_id, url, error,
                                );
                                let category = if error.is_some() { "failure" } else { "pull_request" };
                                let notification_dedupe = format!(
                                    "pull_request:{session_id}:{pane_id}:{}",
                                    url.as_deref().unwrap_or("error")
                                );
                                state
                                    .sessions
                                    .route_to_web(
                                        &session_id,
                                        ServerToWeb::PrCreated {
                                            session_id,
                                            pane_id,
                                            url,
                                            error,
                                        },
                                    )
                                    .await;
                                let routing_id = Uuid::new_v4().to_string();
                                if let Err(notification_error) = crate::notifications::enqueue_project_event(
                                    &state,
                                    &project_id,
                                    Some(&session_id.to_string()),
                                    Some(pane_id),
                                    category,
                                    &routing_id,
                                    &notification_dedupe,
                                )
                                .await
                                {
                                    tracing::warn!(%notification_error, %session_id, pane_id, "failed to enqueue pull-request notification");
                                }
                            }
                            Ok(CliToServer::PaneList { session_id, mut panes }) => {
                                let Ok((_project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                // Cache pane list and forward to attached web clients
                                tracing::info!("CLI {} sent pane list for session {}: {} panes", cli_id, session_id, panes.len());
                                // Preserve web-side custom labels and order from cache or persisted file
                                let mut existing = state.sessions.get_session_panes(&session_id);
                                if existing.is_empty() {
                                    // After server restart the in-memory cache is empty;
                                    // fall back to the persisted panes.json to recover labels/order.
                                    if let Ok(stored) = state.storage.load_pane_list(&session_id).await {
                                        if !stored.is_empty() {
                                            existing = stored;
                                        }
                                    }
                                }
                                if !existing.is_empty() {
                                    let label_map: std::collections::HashMap<u32, String> = existing
                                        .iter()
                                        .filter_map(|p| p.label.as_ref().map(|l| (p.pane_id, l.clone())))
                                        .collect();
                                    for pane in &mut panes {
                                        if let Some(label) = label_map.get(&pane.pane_id) {
                                            pane.label = Some(label.clone());
                                        }
                                    }
                                    // Reorder to match existing cached order; new panes appended at end
                                    let existing_order: Vec<u32> = existing.iter().map(|p| p.pane_id).collect();
                                    let new_ids: std::collections::HashSet<u32> = panes.iter().map(|p| p.pane_id).collect();
                                    let mut reordered: Vec<shared::PaneConfig> = Vec::new();
                                    for &id in &existing_order {
                                        if let Some(p) = panes.iter().find(|p| p.pane_id == id) {
                                            reordered.push(p.clone());
                                        }
                                    }
                                    for p in &panes {
                                        if !existing_order.contains(&p.pane_id) {
                                            reordered.push(p.clone());
                                        }
                                    }
                                    // Only apply reorder if all incoming panes are accounted for
                                    if reordered.len() == panes.len() {
                                        panes = reordered;
                                    }
                                }
                                state.sessions.set_session_panes(&session_id, panes.clone());
                                if let Err(e) = state.storage.save_pane_list(&session_id, &panes).await {
                                    tracing::warn!(
                                        "Failed to persist pane list for session {}: {}",
                                        session_id,
                                        e
                                    );
                                }
                                let web_msg = ServerToWeb::PaneList {
                                    session_id,
                                    panes,
                                };
                                state.sessions.route_to_web(&session_id, web_msg).await;
                            }
                            Ok(CliToServer::Register { .. }) => {
                                // Already registered, ignore
                            }
                            Ok(CliToServer::PaneWorkSummaryResult { result }) => {
                                let _ = state
                                    .pane_work_summaries
                                    .accept_result(cli_id, result)
                                    .await;
                            }
                            Ok(CliToServer::ProjectPolicySnapshot {
                                session_id,
                                team_enabled,
                                disallowed_tab_types,
                            }) => {
                                let Ok((_project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                if !state
                                    .sessions
                                    .get_cli_session_ids(&cli_id)
                                    .contains(&session_id)
                                {
                                    tracing::warn!(
                                        "Ignoring policy snapshot for unowned session {} from CLI {}",
                                        session_id,
                                        cli_id
                                    );
                                    continue;
                                }
                                let Some(project_id) =
                                    state.sessions.project_for_session(&session_id)
                                else {
                                    tracing::warn!(
                                        "Ignoring policy snapshot before project registration for session {}",
                                        session_id
                                    );
                                    continue;
                                };
                                match state
                                    .db
                                    .import_legacy_project_policy(
                                        &project_id,
                                        team_enabled,
                                        &disallowed_tab_types,
                                    )
                                    .await
                                {
                                    Ok(policy) => {
                                        let noncompliant_pane_ids = state
                                            .sessions
                                            .get_session_panes(&session_id)
                                            .into_iter()
                                            .filter(|pane| {
                                                !policy.allows(
                                                    pane.kind,
                                                    pane.provider,
                                                    pane.model.as_deref(),
                                                )
                                            })
                                            .map(|pane| pane.pane_id)
                                            .collect();
                                        state
                                            .sessions
                                            .send_to_cli(
                                                &cli_id,
                                                ServerToCli::ProjectPolicy {
                                                    session_id,
                                                    policy: policy.clone(),
                                                },
                                            )
                                            .await;
                                        state
                                            .sessions
                                            .route_to_web(
                                                &session_id,
                                                ServerToWeb::ProjectPolicyChanged {
                                                    session_id,
                                                    policy,
                                                    noncompliant_pane_ids,
                                                },
                                            )
                                            .await;
                                    }
                                    Err(err) => tracing::warn!(
                                        "Failed to import legacy policy for project {}: {}",
                                        project_id,
                                        err
                                    ),
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to parse CLI message: {}", e);
                            }
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {
                        // Pong received - connection is alive
                        last_activity = Instant::now();
                    }
                    Some(Ok(Message::Ping(data))) => {
                        // Respond to ping from client
                        last_activity = Instant::now();
                        let _ = sender.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) => {
                        tracing::info!("CLI {} sent close frame", cli_id);
                        break;
                    }
                    Some(Ok(_)) => {
                        // Binary or other message types - still counts as activity
                        last_activity = Instant::now();
                    }
                    Some(Err(e)) => {
                        tracing::warn!("CLI {} WebSocket error: {}", cli_id, e);
                        break;
                    }
                    None => {
                        // Connection closed
                        tracing::info!("CLI {} connection closed", cli_id);
                        break;
                    }
                }
            }
        }
    }

    // Cleanup - mark all sessions for this CLI as inactive and clear cli_client_id
    let session_ids = state.sessions.get_cli_session_ids(&cli_id);
    for session_id in &session_ids {
        if let Err(e) = state.db.clear_session_cli(&session_id.to_string()).await {
            tracing::error!(
                "Failed to clear session {} cli_client_id: {}",
                session_id,
                e
            );
        }
        // Transport loss does not prove that this APAS process or its PTYs
        // ended. Retain their bounded presentation and expose the uncertainty
        // until the reconnect state report reconciles each process instance.
        for (pane_id, terminal) in state
            .sessions
            .mark_session_terminals_disconnected(session_id)
        {
            state
                .sessions
                .route_to_web(
                    session_id,
                    ServerToWeb::TerminalState {
                        session_id: *session_id,
                        pane_id,
                        instance_id: terminal.instance_id,
                        lifecycle: terminal.lifecycle,
                        status: terminal.status,
                        runtime: terminal.runtime,
                    },
                )
                .await;
        }
    }

    state.sessions.unregister_cli(&cli_id);
    let _ = state
        .db
        .update_cli_client_status(&cli_id.to_string(), "offline")
        .await;
    tracing::info!(
        "CLI client disconnected: {} (marked {} sessions as inactive)",
        cli_id,
        session_ids.len()
    );
}

/// Record a usage delta for (session, pane) into today's UTC bucket and push
/// the refreshed project-level usage stats to any attached web clients. Shared
/// with the web-input path (`ws_web::handle_web_input`) so prompts typed in the
/// web UI are counted too.
pub(crate) async fn record_and_broadcast_usage(
    state: &AppState,
    session_id: Uuid,
    pane_id: Option<u32>,
    delta: crate::db::UsageDelta,
) {
    let Ok((_project_id, _operation_guard)) = state
        .active_session_operation(&session_id.to_string())
        .await
    else {
        return;
    };
    let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
    // Unattributed turns (no pane_id, e.g. legacy hybrid mode) land in pane 0.
    let pid = pane_id.unwrap_or(0) as i64;
    if let Err(e) = state
        .db
        .add_pane_usage(&session_id.to_string(), pid, &day, &delta)
        .await
    {
        tracing::error!("Failed to record pane usage: {}", e);
        return;
    }
    match state
        .db
        .get_project_usage_stats(&session_id.to_string())
        .await
    {
        Ok(stats) => {
            state
                .sessions
                .route_to_web(
                    &session_id,
                    ServerToWeb::ProjectUsageStats { session_id, stats },
                )
                .await;
        }
        Err(e) => tracing::error!("Failed to read project usage stats: {}", e),
    }
}

/// Convert a ClaudeStreamMessage to StoredMessages for file storage
/// Returns a Vec because assistant messages may have multiple content blocks
fn stream_message_to_stored(
    _session_id: &Uuid,
    message: &shared::ClaudeStreamMessage,
    pane_id: Option<u32>,
) -> Vec<crate::storage::StoredMessage> {
    use shared::{ClaudeContentBlock, ClaudeStreamMessage};

    let pane_type_str = pane_id.map(|id| id.to_string());
    let mut messages = Vec::new();

    match message {
        ClaudeStreamMessage::Assistant { message: msg, .. } => {
            // Store each content block separately to preserve structure
            for block in &msg.content {
                match block {
                    ClaudeContentBlock::Text { text } => {
                        messages.push(crate::storage::StoredMessage {
                            id: Uuid::new_v4().to_string(),
                            role: "assistant".to_string(),
                            content: text.clone(),
                            message_type: "text".to_string(),
                            created_at: chrono::Utc::now().to_rfc3339(),
                            pane_type: pane_type_str.clone(),
                        });
                    }
                    ClaudeContentBlock::ToolUse { id, name, input } => {
                        // Store tool_use with structured JSON content
                        let tool_data = serde_json::json!({
                            "id": id,
                            "name": name,
                            "input": input
                        });
                        messages.push(crate::storage::StoredMessage {
                            id: Uuid::new_v4().to_string(),
                            role: "assistant".to_string(),
                            content: tool_data.to_string(),
                            message_type: "tool_use".to_string(),
                            created_at: chrono::Utc::now().to_rfc3339(),
                            pane_type: pane_type_str.clone(),
                        });
                    }
                    ClaudeContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => {
                        // Store tool_result with structured JSON content
                        let result_data = serde_json::json!({
                            "tool_use_id": tool_use_id,
                            "content": content,
                            "is_error": is_error
                        });
                        messages.push(crate::storage::StoredMessage {
                            id: Uuid::new_v4().to_string(),
                            role: "assistant".to_string(),
                            content: result_data.to_string(),
                            message_type: "tool_result".to_string(),
                            created_at: chrono::Utc::now().to_rfc3339(),
                            pane_type: pane_type_str.clone(),
                        });
                    }
                }
            }
        }
        ClaudeStreamMessage::Result {
            subtype,
            total_cost_usd,
            duration_ms,
            ..
        } => {
            messages.push(crate::storage::StoredMessage {
                id: Uuid::new_v4().to_string(),
                role: "system".to_string(),
                content: format!(
                    "{} - Cost: ${:.4}, Duration: {}ms",
                    subtype, total_cost_usd, duration_ms
                ),
                message_type: "result".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
                pane_type: pane_type_str,
            });
        }
        ClaudeStreamMessage::User {
            message: msg,
            tool_use_result,
            ..
        } => {
            // Store tool results from user messages. We tuck the top-level
            // `tool_use_result` payload (e.g. AskUserQuestion's
            // `{questions, answers}` echo) inside the stored JSON so the
            // web UI can recover the structured answer after a reload.
            for block in &msg.content {
                if let ClaudeContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } = block
                {
                    let mut result_data = serde_json::json!({
                        "tool_use_id": tool_use_id,
                        "content": content,
                        "is_error": is_error,
                    });
                    if let Some(tur) = tool_use_result {
                        if let serde_json::Value::Object(ref mut map) = result_data {
                            map.insert("tool_use_result".to_string(), tur.clone());
                        }
                    }
                    messages.push(crate::storage::StoredMessage {
                        id: Uuid::new_v4().to_string(),
                        role: "tool".to_string(),
                        content: result_data.to_string(),
                        message_type: "tool_result".to_string(),
                        created_at: chrono::Utc::now().to_rfc3339(),
                        pane_type: pane_type_str.clone(),
                    });
                }
            }
        }
        _ => {} // Skip system init messages
    }

    messages
}

#[cfg(test)]
mod pane_status_tests {
    use super::{is_terminal_pane, terminal_assistant_completes_work};
    use shared::{
        ClaudeAssistantMessage, ClaudeContentBlock, ClaudeStreamMessage, PaneConfig, PaneKind,
    };

    fn assistant(extra: serde_json::Value) -> ClaudeStreamMessage {
        ClaudeStreamMessage::Assistant {
            message: ClaudeAssistantMessage {
                content: vec![ClaudeContentBlock::Text {
                    text: "still working".to_string(),
                }],
                model: String::new(),
                extra: serde_json::Value::Null,
            },
            session_id: "provider-session".to_string(),
            extra,
        }
    }

    #[test]
    fn working_status_inference_is_limited_to_terminal_panes() {
        let mut panes = PaneConfig::defaults();
        panes[0].pane_id = 7;
        assert!(!is_terminal_pane(&panes, 7));

        panes[0].kind = PaneKind::Terminal;
        assert!(is_terminal_pane(&panes, 7));
        assert!(!is_terminal_pane(&panes, 8));
    }

    #[test]
    fn terminal_working_status_only_clears_on_confirmed_completion() {
        let mut panes = PaneConfig::defaults();
        panes[0].pane_id = 7;
        panes[0].kind = PaneKind::Terminal;

        assert!(!terminal_assistant_completes_work(
            &assistant(serde_json::json!({"terminal_turn_complete": false})),
            &panes,
            Some(7),
        ));
        assert!(terminal_assistant_completes_work(
            &assistant(serde_json::json!({"terminal_turn_complete": true})),
            &panes,
            Some(7),
        ));
        assert!(
            terminal_assistant_completes_work(
                &assistant(serde_json::json!({"terminal_turn_complete": true})),
                &[],
                Some(7),
            ),
            "an explicit terminal completion must win the reconnect pane-list race"
        );
        assert!(!terminal_assistant_completes_work(
            &assistant(serde_json::json!({"terminal_turn_complete": false})),
            &[],
            Some(7),
        ));
        assert!(
            terminal_assistant_completes_work(&assistant(serde_json::Value::Null), &panes, Some(7),),
            "unmarked messages keep the legacy behavior during CLI rollout"
        );
        assert!(
            !terminal_assistant_completes_work(&assistant(serde_json::Value::Null), &[], Some(7),),
            "unmarked structured assistant messages cannot clear arbitrary panes"
        );
        assert!(!terminal_assistant_completes_work(
            &assistant(serde_json::Value::Null),
            &panes,
            Some(8),
        ));
        assert!(!terminal_assistant_completes_work(
            &assistant(serde_json::json!({"terminal_turn_complete": true})),
            &panes,
            None,
        ));
    }
}
