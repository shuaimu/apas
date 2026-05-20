use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use shared::{CliToServer, ServerToCli, ServerToWeb};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::routes::auth::verify_token;
use crate::state::AppState;

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

fn is_minimax_model(model: Option<&str>) -> bool {
    model
        .map(|raw| {
            let normalized = raw.trim().to_ascii_lowercase();
            !normalized.is_empty()
                && (normalized.contains("minimax") || normalized.starts_with("m2"))
        })
        .unwrap_or(false)
}

fn is_glm_model(model: Option<&str>) -> bool {
    model
        .map(|raw| {
            let normalized = raw.trim().to_ascii_lowercase();
            !normalized.is_empty() && (normalized.starts_with("glm") || normalized.contains("glm-"))
        })
        .unwrap_or(false)
}

fn normalize_backend_pane_labels(panes: &mut [shared::PaneConfig]) {
    for pane in panes.iter_mut() {
        if pane.provider != shared::Provider::Claude {
            continue;
        }
        let model = pane.model.as_deref();
        let backend_label = if is_minimax_model(model) {
            Some("MiniMax")
        } else if is_glm_model(model) {
            Some("GLM")
        } else {
            None
        };
        let Some(backend_label) = backend_label else {
            continue;
        };

        let tab_label = format!("Tab {}", pane.pane_id);
        let current_label = pane.label.as_deref().map(str::trim).unwrap_or("");
        if current_label.is_empty() || current_label.eq_ignore_ascii_case(&tab_label) {
            pane.label = Some(format!("{} {}", backend_label, pane.pane_id));
        }
    }
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

    loop {
        match receiver.next().await {
            Some(Ok(Message::Text(text))) => {
                let parsed: Result<CliToServer, _> = serde_json::from_str(&text);
                match parsed {
                    Ok(CliToServer::Register { token, version }) => {
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
                                        user_id = uid;
                                        cli_id = Uuid::new_v4();
                                        cli_version = version;

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
                        let parsed: Result<CliToServer, _> = serde_json::from_str(&text);
                        match parsed {
                            Ok(CliToServer::SessionStart {
                                session_id,
                                project_id,
                                working_dir,
                                hostname,
                                pane_type: _,
                                panes,
                            }) => {
                                // Older CLIs omit project_id; preserve the
                                // historical 1:1 mapping where the .apas id
                                // also served as the session id.
                                let project_id = project_id.unwrap_or(session_id);
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

                                // Cache initial pane list if provided, preserving persisted labels/order
                                if let Some(pane_list) = &panes {
                                    if !pane_list.is_empty() {
                                        let mut normalized_panes = pane_list.clone();
                                        normalize_backend_pane_labels(&mut normalized_panes);
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
                                };
                                if let Err(e) = state.db.create_session(&session).await {
                                    tracing::error!("Failed to persist session to database: {}", e);
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
                                tracing::info!("Received Output for session {} with pane_id {:?}: {}", session_id, pane_id, &data[..data.len().min(50)]);
                                // Route output to web client (if attached)
                                let routed = state
                                    .sessions
                                    .route_to_web(
                                        &session_id,
                                        ServerToWeb::Output {
                                            content: data,
                                            output_type,
                                            pane_type,
                                            pane_id,
                                        },
                                    )
                                    .await;
                                tracing::info!("Output routed to web: {}", routed);
                            }
                            Ok(CliToServer::StreamMessage { session_id, message, pane_type, pane_id }) => {
                                tracing::info!("Received StreamMessage for session {} with pane_id {:?}", session_id, pane_id);

                                // Use pane_id for storage, falling back to pane_type for backward compat
                                let effective_pane_id = pane_id.or_else(|| pane_type.map(|p| shared::PaneConfig::pane_id_from_legacy(&p)));

                                // Save message(s) to file storage
                                for stored_message in stream_message_to_stored(&session_id, &message, effective_pane_id) {
                                    if let Err(e) = state.storage.append_message(&session_id, &stored_message).await {
                                        tracing::error!("Failed to save message to file: {}", e);
                                    }
                                }

                                // Route structured stream message to web client
                                let routed = state
                                    .sessions
                                    .route_to_web(
                                        &session_id,
                                        ServerToWeb::StreamMessage { session_id, message, pane_type, pane_id },
                                    )
                                    .await;
                                tracing::info!("StreamMessage routed to web: {}", routed);
                            }
                            Ok(CliToServer::UserInput { session_id, text, pane_type, pane_id }) => {
                                tracing::info!("Received UserInput for session {}: {}", session_id, text);
                                // Use pane_id for storage, falling back to pane_type
                                let effective_pane_id = pane_id.or_else(|| pane_type.map(|p| shared::PaneConfig::pane_id_from_legacy(&p)));
                                // Save user input to file storage
                                let stored_message = crate::storage::StoredMessage {
                                    id: Uuid::new_v4().to_string(),
                                    role: "user".to_string(),
                                    content: text.clone(),
                                    message_type: "text".to_string(),
                                    created_at: chrono::Utc::now().to_rfc3339(),
                                    pane_type: effective_pane_id.map(|id| id.to_string()),
                                };
                                if let Err(e) = state.storage.append_message(&session_id, &stored_message).await {
                                    tracing::error!("Failed to save user input to file: {}", e);
                                }

                                // Forward user input to web client
                                state
                                    .sessions
                                    .route_to_web(
                                        &session_id,
                                        ServerToWeb::UserInput { session_id, text, pane_type, pane_id },
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
                            Ok(CliToServer::DeadloopStatus { session_id, is_paused }) => {
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
                                            pane_type,
                                            pane_id,
                                            status,
                                        },
                                    )
                                    .await;
                            }
                            Ok(CliToServer::PanePaused { session_id, pane_id, is_paused }) => {
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
                            Ok(CliToServer::TeamRecord { session_id, record }) => {
                                tracing::info!(
                                    "Team scratchpad record for session {} (kind={}, pane={:?})",
                                    session_id, record.kind, record.pane_id,
                                );
                                state
                                    .sessions
                                    .route_to_web(
                                        &session_id,
                                        ServerToWeb::TeamRecord { session_id, record },
                                    )
                                    .await;
                            }
                            Ok(CliToServer::PaneDiff { session_id, pane_id, branch, base, diff, error }) => {
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
                            Ok(CliToServer::PaneList { session_id, mut panes }) => {
                                // Cache pane list and forward to attached web clients
                                tracing::info!("CLI {} sent pane list for session {}: {} panes", cli_id, session_id, panes.len());
                                normalize_backend_pane_labels(&mut panes);
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
