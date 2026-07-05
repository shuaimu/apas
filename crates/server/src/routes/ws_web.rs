use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use shared::{
    MessageInfo, ServerToCli, ServerToDaemon, ServerToWeb, SessionInfo, SessionStatus, WebToServer,
};
use std::collections::HashSet;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::routes::auth::verify_token;
use crate::state::AppState;

const SERVER_VERSION: &str = env!("APAS_SERVER_VERSION");

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

fn parse_stored_pane_id(raw_pane_type: Option<&str>) -> Option<u32> {
    let raw = raw_pane_type?.trim();
    if raw.is_empty() {
        return None;
    }

    if raw.eq_ignore_ascii_case("deadloop") {
        return Some(shared::PANE_ID_DEADLOOP);
    }
    if raw.eq_ignore_ascii_case("interactive") {
        return Some(shared::PANE_ID_INTERACTIVE);
    }
    if let Ok(id) = raw.parse::<u32>() {
        return Some(id);
    }

    let lower = raw.to_ascii_lowercase();
    if lower.contains("deadloop") {
        return Some(shared::PANE_ID_DEADLOOP);
    }
    if lower.contains("interactive") {
        return Some(shared::PANE_ID_INTERACTIVE);
    }

    let trailing_digits_rev: String = lower
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if trailing_digits_rev.is_empty() {
        return None;
    }
    let trailing_digits: String = trailing_digits_rev.chars().rev().collect();
    trailing_digits.parse::<u32>().ok()
}

/// Cap any single message's serialized content to keep batched SessionMessages
/// payloads below tungstenite's default ~16 MiB frame limit. The historical
/// trigger: an Edit tool_result includes the entire before+after of a touched
/// file, which for big source files can easily be multi-MB. 100 such messages
/// for one pane × 3+ active panes blew past the frame limit and the whole
/// load-on-attach payload was being dropped — the symptom was "rusty-lib pane
/// loads no messages."
const MAX_TRANSIT_CONTENT_BYTES: usize = 96 * 1024;

fn truncate_for_transit(content: String, message_type: &str) -> String {
    crate::storage::truncate_message_content(
        content,
        message_type,
        MAX_TRANSIT_CONTENT_BYTES,
        "transit",
    )
}

fn to_message_info(message: crate::storage::StoredMessage) -> MessageInfo {
    let pane_id = parse_stored_pane_id(message.pane_type.as_deref());
    let content = truncate_for_transit(message.content, &message.message_type);
    MessageInfo {
        id: message.id,
        role: message.role,
        content,
        message_type: message.message_type,
        created_at: Some(message.created_at),
        pane_type: message.pane_type,
        pane_id,
    }
}

async fn session_download_response(state: &AppState, sid: Uuid) -> ServerToWeb {
    let (project_id, working_dir, hostname, created_at) =
        match state.db.get_session(&sid.to_string()).await {
            Ok(Some(session)) => {
                let project_id = session
                    .project_id
                    .as_deref()
                    .and_then(|p| Uuid::parse_str(p).ok())
                    .or(Some(sid));
                (
                    project_id,
                    session.working_dir,
                    session.hostname,
                    session.created_at,
                )
            }
            _ => (Some(sid), None, None, None),
        };

    match state.storage.get_messages(&sid).await {
        Ok(stored_messages) => {
            let messages: Vec<MessageInfo> =
                stored_messages.into_iter().map(to_message_info).collect();
            ServerToWeb::SessionDownload {
                session_id: sid,
                project_id,
                messages,
                working_dir,
                hostname,
                created_at,
            }
        }
        Err(e) => {
            tracing::error!("Failed to get messages for download: {}", e);
            ServerToWeb::Error {
                message: "Failed to download session data".to_string(),
            }
        }
    }
}

#[cfg(test)]
mod transit_truncation_tests {
    use super::*;

    #[test]
    fn passes_small_text_through() {
        let s = "short assistant text".to_string();
        assert_eq!(truncate_for_transit(s.clone(), "text"), s);
    }

    #[test]
    fn truncates_non_json_oversize_with_marker() {
        let big = "a".repeat(MAX_TRANSIT_CONTENT_BYTES + 100);
        let out = truncate_for_transit(big.clone(), "text");
        assert!(out.len() < MAX_TRANSIT_CONTENT_BYTES);
        assert!(out.contains("truncated for transit"));
        assert!(out.contains(&format!("{}", big.len())));
    }

    #[test]
    fn keeps_tool_result_envelope_when_truncating() {
        let huge_inner = "x".repeat(MAX_TRANSIT_CONTENT_BYTES);
        let envelope = serde_json::json!({
            "content": huge_inner,
            "is_error": false,
            "tool_use_id": "toolu_abc",
            "tool_use_result": {"oldString": "y".repeat(1024)}
        })
        .to_string();
        let out = truncate_for_transit(envelope, "tool_result");
        let parsed: serde_json::Value =
            serde_json::from_str(&out).expect("envelope must remain valid JSON");
        assert_eq!(parsed["is_error"], false);
        assert_eq!(parsed["tool_use_id"], "toolu_abc");
        // tool_use_result is dropped on truncation
        assert!(parsed.get("tool_use_result").is_none());
        let inner = parsed["content"].as_str().expect("content stays a string");
        assert!(inner.contains("truncated for transit"));
    }

    #[test]
    fn keeps_tool_use_envelope_when_truncating() {
        let envelope = serde_json::json!({
            "id": "toolu_question",
            "name": "AskUserQuestion",
            "input": {
                "questions": [
                    {
                        "id": "confirm",
                        "header": "Confirm",
                        "question": "x".repeat(MAX_TRANSIT_CONTENT_BYTES),
                        "options": [
                            {"label": "Yes", "description": "Continue"},
                            {"label": "No", "description": "Stop"}
                        ]
                    }
                ]
            }
        })
        .to_string();
        let out = truncate_for_transit(envelope, "tool_use");
        let parsed: serde_json::Value =
            serde_json::from_str(&out).expect("tool_use envelope must remain valid JSON");
        assert_eq!(parsed["id"], "toolu_question");
        assert_eq!(parsed["name"], "AskUserQuestion");
        let input = parsed["input"]
            .as_object()
            .expect("structured input is replaced by a marker object");
        assert_eq!(
            input.get("_truncated").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            input.get("_reason").and_then(|v| v.as_str()),
            Some("transit")
        );
    }
}

fn infer_panes_from_messages(
    session_id: Uuid,
    messages: &[MessageInfo],
) -> Vec<shared::PaneConfig> {
    let mut pane_ids: Vec<u32> = messages.iter().filter_map(|m| m.pane_id).collect();
    pane_ids.sort_unstable();
    pane_ids.dedup();

    pane_ids
        .into_iter()
        .map(|pane_id| {
            let (mode, label) = match pane_id {
                shared::PANE_ID_DEADLOOP => (shared::PaneMode::Deadloop, "Deadloop".to_string()),
                shared::PANE_ID_INTERACTIVE => {
                    (shared::PaneMode::Interactive, "Interactive".to_string())
                }
                _ => (shared::PaneMode::Interactive, format!("Tab {}", pane_id)),
            };
            shared::PaneConfig {
                pane_id,
                provider: shared::Provider::Claude,
                mode,
                // We cannot recover provider-specific resume session IDs from historical
                // messages alone, so we fall back to the project session ID.
                session_id,
                is_paused: false,
                stop_requested: false,
                prompt: None,
                min_iteration_interval_minutes: None,
                label: Some(label),
                model: None,
                effort: None,
                worktree_path: None,
                role: None,
                goal: None,
                backstory: None,
                plan_review_mode: shared::PlanReviewMode::default(),
                manual_mode: false,
                managed: false,
            }
        })
        .collect()
}

fn normalize_start_bot_effort(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = trimmed.to_ascii_lowercase();
    // Keep in lock-step with the CLI's `normalize_effort_level` —
    // `ultracode` is an apas-only level (xhigh wire flag + workflow
    // prompt prefix) that must round-trip through the server unchanged.
    match normalized.as_str() {
        "default" | "auto" | "none" | "off" => None,
        "low" => Some("low".to_string()),
        "medium" | "med" => Some("medium".to_string()),
        "high" => Some("high".to_string()),
        "xhigh" | "x-high" => Some("xhigh".to_string()),
        "max" => Some("max".to_string()),
        "ultracode" => Some("ultracode".to_string()),
        _ => None,
    }
}

fn normalize_machine_hostname(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

fn normalize_project_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed == "/" {
        return "/".to_string();
    }
    trimmed.trim_end_matches('/').to_string()
}

async fn get_shared_project_access_refs(
    state: &AppState,
    user_id: &Uuid,
) -> (HashSet<(String, String)>, HashSet<String>) {
    let mut host_path_refs = HashSet::new();
    let mut wildcard_paths = HashSet::new();

    let shared_sessions = match state
        .db
        .get_shared_sessions_for_user(&user_id.to_string())
        .await
    {
        Ok(sessions) => sessions,
        Err(err) => {
            tracing::warn!(
                "Failed to load shared sessions for machine access (user {}): {}",
                user_id,
                err
            );
            Vec::new()
        }
    };

    for (session, _, _) in shared_sessions {
        let Some(path_raw) = session.working_dir else {
            continue;
        };
        let path_key = normalize_project_path(&path_raw);
        if path_key.is_empty() {
            continue;
        }

        if let Some(host_raw) = session.hostname {
            let host_key = normalize_machine_hostname(&host_raw);
            if host_key.is_empty() {
                wildcard_paths.insert(path_key);
            } else {
                host_path_refs.insert((host_key, path_key));
            }
        } else {
            wildcard_paths.insert(path_key);
        }
    }

    (host_path_refs, wildcard_paths)
}

async fn list_accessible_machines_for_user(
    state: &AppState,
    user_id: &Uuid,
) -> Vec<shared::MachineWithProjects> {
    let mut machines = state.sessions.get_machines_for_user(user_id);
    let (host_path_refs, wildcard_paths) = get_shared_project_access_refs(state, user_id).await;
    // Cache the refs so heartbeat-driven `broadcast_machines_update_for_user`
    // can include shared machines too; without this, pushed updates between
    // user-initiated refreshes would drop teammate machines and the UI would
    // appear to flap.
    state.sessions.set_shared_project_refs_for_user(
        *user_id,
        host_path_refs.clone(),
        wildcard_paths.clone(),
    );
    if host_path_refs.is_empty() && wildcard_paths.is_empty() {
        return machines;
    }

    let owner_machine_ids: HashSet<Uuid> = machines.iter().map(|m| m.machine.machine_id).collect();
    for machine in state
        .sessions
        .get_machines_for_project_refs(&host_path_refs, &wildcard_paths)
    {
        if owner_machine_ids.contains(&machine.machine.machine_id) {
            continue;
        }
        machines.push(machine);
    }

    machines
}

#[cfg(test)]
mod session_download_tests {
    use super::*;
    use crate::config::Config;
    use crate::db::{Database, Session, User};
    use crate::storage::StoredMessage;
    use std::path::Path;

    fn test_user(user_id: Uuid) -> User {
        User {
            id: user_id.to_string(),
            email: format!("{user_id}@example.test"),
            password_hash: "hash".to_string(),
            created_at: None,
        }
    }

    fn test_session(
        session_id: Uuid,
        owner_id: Uuid,
        project_id: Uuid,
        working_dir: &str,
        hostname: &str,
    ) -> Session {
        Session {
            id: session_id.to_string(),
            user_id: owner_id.to_string(),
            cli_client_id: None,
            working_dir: Some(working_dir.to_string()),
            hostname: Some(hostname.to_string()),
            status: "connected".to_string(),
            created_at: None,
            updated_at: None,
            is_paused: false,
            project_id: Some(project_id.to_string()),
            git_remote: None,
            git_remote_url: None,
        }
    }

    async fn test_state() -> AppState {
        let dir = std::env::temp_dir().join(format!("apas-session-download-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp db dir");
        let db_path = dir.join("apas.db").to_string_lossy().to_string();
        let db = Database::new(&db_path).await.expect("create temp db");
        db.run_migrations().await.expect("run migrations");
        let mut config = Config::default();
        config.database.path = db_path;
        AppState::new(db, config)
    }

    fn stored_message() -> StoredMessage {
        StoredMessage {
            id: "msg-1".to_string(),
            role: "assistant".to_string(),
            content: "export me".to_string(),
            message_type: "text".to_string(),
            created_at: "2026-06-18T00:31:00Z".to_string(),
            pane_type: Some("42".to_string()),
        }
    }

    #[tokio::test]
    async fn download_session_response_includes_metadata_and_stored_messages() {
        let state = test_state().await;
        let user_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let project_id = Uuid::new_v4();

        state
            .db
            .create_user(&test_user(user_id))
            .await
            .expect("create user");
        state
            .db
            .create_session(&test_session(
                session_id,
                user_id,
                project_id,
                "/workspace/apas",
                "dev-host",
            ))
            .await
            .expect("create session");
        state
            .storage
            .append_message(&session_id, &stored_message())
            .await
            .expect("store message");

        let response = session_download_response(&state, session_id).await;

        match response {
            ServerToWeb::SessionDownload {
                session_id: response_session_id,
                project_id: response_project_id,
                messages,
                working_dir,
                hostname,
                created_at,
            } => {
                assert_eq!(response_session_id, session_id);
                assert_eq!(response_project_id, Some(project_id));
                assert_eq!(working_dir.as_deref(), Some("/workspace/apas"));
                assert_eq!(hostname.as_deref(), Some("dev-host"));
                assert!(created_at.is_some());
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].id, "msg-1");
                assert_eq!(messages[0].role, "assistant");
                assert_eq!(messages[0].content, "export me");
                assert_eq!(messages[0].message_type, "text");
                assert_eq!(
                    messages[0].created_at.as_deref(),
                    Some("2026-06-18T00:31:00Z")
                );
                assert_eq!(messages[0].pane_type.as_deref(), Some("42"));
                assert_eq!(messages[0].pane_id, Some(42));
            }
            other => panic!("expected SessionDownload response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn download_session_response_returns_existing_error_on_storage_failure() {
        let state = test_state().await;
        let session_id = Uuid::new_v4();
        let storage_root = Path::new(&state.config.database.path)
            .parent()
            .expect("db parent");
        let messages_path = storage_root
            .join("sessions")
            .join(session_id.to_string())
            .join("messages.jsonl");
        tokio::fs::create_dir_all(&messages_path)
            .await
            .expect("messages path as directory");

        let response = session_download_response(&state, session_id).await;

        match response {
            ServerToWeb::Error { message } => {
                assert_eq!(message, "Failed to download session data");
            }
            other => panic!("expected download error response, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod machine_access_tests {
    use super::*;
    use crate::config::Config;
    use crate::db::{Database, Session, User};

    fn test_machine(machine_id: Uuid, hostname: &str) -> shared::MachineInfo {
        shared::MachineInfo {
            machine_id,
            hostname: hostname.to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            daemon_version: None,
            minimax_backend: None,
            glm_backend: None,
            deepseek_backend: None,
            last_seen: None,
        }
    }

    fn test_project(project_id: &str, path: &str) -> shared::MachineProjectInfo {
        shared::MachineProjectInfo {
            project_id: project_id.to_string(),
            name: Some(project_id.to_string()),
            path: path.to_string(),
            is_running: false,
            pid: None,
            memory_kb: None,
            last_error: None,
        }
    }

    fn test_user(user_id: Uuid, email: &str) -> User {
        User {
            id: user_id.to_string(),
            email: email.to_string(),
            password_hash: "hash".to_string(),
            created_at: None,
        }
    }

    fn test_session(
        session_id: Uuid,
        owner_id: Uuid,
        working_dir: &str,
        hostname: Option<&str>,
    ) -> Session {
        Session {
            id: session_id.to_string(),
            user_id: owner_id.to_string(),
            cli_client_id: None,
            working_dir: Some(working_dir.to_string()),
            hostname: hostname.map(str::to_string),
            status: "connected".to_string(),
            created_at: None,
            updated_at: None,
            is_paused: false,
            project_id: None,
            git_remote: None,
            git_remote_url: None,
        }
    }

    async fn test_state() -> AppState {
        let dir = std::env::temp_dir().join(format!("apas-machine-access-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp db dir");
        let db_path = dir.join("apas.db").to_string_lossy().to_string();
        let db = Database::new(&db_path).await.expect("create temp db");
        db.run_migrations().await.expect("run migrations");
        let mut config = Config::default();
        config.database.path = db_path;
        AppState::new(db, config)
    }

    #[tokio::test]
    async fn list_accessible_machines_caches_host_and_wildcard_shared_refs() {
        let state = test_state().await;
        let viewer_id = Uuid::new_v4();
        let teammate_id = Uuid::new_v4();
        state
            .db
            .create_user(&test_user(viewer_id, "viewer@example.test"))
            .await
            .expect("viewer user");
        state
            .db
            .create_user(&test_user(teammate_id, "teammate@example.test"))
            .await
            .expect("teammate user");

        let host_shared_session_id = Uuid::new_v4();
        state
            .db
            .create_session(&test_session(
                host_shared_session_id,
                teammate_id,
                "/team/shared/",
                Some("SharedHost"),
            ))
            .await
            .expect("host shared session");
        state
            .db
            .create_session_share(
                &host_shared_session_id.to_string(),
                &viewer_id.to_string(),
                &teammate_id.to_string(),
            )
            .await
            .expect("host shared session share");

        let wildcard_shared_session_id = Uuid::new_v4();
        state
            .db
            .create_session(&test_session(
                wildcard_shared_session_id,
                teammate_id,
                "/team/wildcard",
                None,
            ))
            .await
            .expect("wildcard shared session");
        state
            .db
            .create_session_share(
                &wildcard_shared_session_id.to_string(),
                &viewer_id.to_string(),
                &teammate_id.to_string(),
            )
            .await
            .expect("wildcard shared session share");

        let owner_machine_id = Uuid::new_v4();
        let host_shared_machine_id = Uuid::new_v4();
        let wildcard_shared_machine_id = Uuid::new_v4();
        let (owner_tx, _owner_rx) = mpsc::channel(1);
        state.sessions.register_daemon(
            owner_machine_id,
            viewer_id,
            owner_tx,
            test_machine(owner_machine_id, "ViewerHost"),
            vec![test_project("owned", "/viewer/project")],
        );
        let (host_tx, _host_rx) = mpsc::channel(1);
        state.sessions.register_daemon(
            host_shared_machine_id,
            teammate_id,
            host_tx,
            test_machine(host_shared_machine_id, "sharedhost"),
            vec![
                test_project("host-match", "/team/shared"),
                test_project("host-other", "/team/other"),
            ],
        );
        let (wildcard_tx, _wildcard_rx) = mpsc::channel(1);
        state.sessions.register_daemon(
            wildcard_shared_machine_id,
            teammate_id,
            wildcard_tx,
            test_machine(wildcard_shared_machine_id, "some-other-host"),
            vec![test_project("wildcard-match", "/team/wildcard")],
        );

        let machines = list_accessible_machines_for_user(&state, &viewer_id).await;
        let machine_ids: HashSet<Uuid> = machines
            .iter()
            .map(|machine| machine.machine.machine_id)
            .collect();
        assert!(machine_ids.contains(&owner_machine_id));
        assert!(machine_ids.contains(&host_shared_machine_id));
        assert!(machine_ids.contains(&wildcard_shared_machine_id));
        let host_shared = machines
            .iter()
            .find(|machine| machine.machine.machine_id == host_shared_machine_id)
            .expect("host shared machine");
        assert_eq!(host_shared.projects.len(), 1);
        assert_eq!(host_shared.projects[0].project_id, "host-match");

        let (host_refs, wildcard_refs) = state
            .sessions
            .cached_shared_project_refs_for_user(&viewer_id)
            .expect("shared refs cached");
        assert!(host_refs.contains(&("sharedhost".to_string(), "/team/shared".to_string())));
        assert!(wildcard_refs.contains("/team/wildcard"));
    }
}

/// Pick the session a `WebToServer` message should act on.
///
/// Multi-attach (commit b0c674d) lets one web connection observe several
/// sessions in parallel, but the connection's `session_id` local variable
/// is overwritten on every `AttachSession`, so it points at whichever
/// session was attached last — non-deterministic when the web fan-out
/// races. Messages that carry an explicit `session_id` use that (after
/// verifying this connection has actually attached to it). Messages that
/// don't are legacy — fall back to the connection's last-attached session.
///
/// Returns `None` after pushing an `Error` to the web client.
async fn resolve_target_session(
    state: &AppState,
    connection_id: &Uuid,
    msg_session_id: Option<Uuid>,
    fallback: Option<Uuid>,
) -> Option<Uuid> {
    if let Some(sid) = msg_session_id {
        if state.sessions.is_web_attached_to_session(&sid, connection_id) {
            return Some(sid);
        }
        // Not registered as attached — but the web client may legitimately
        // own/have access to this session and simply hasn't finished its
        // (re)attach handshake yet. This is the post-reconnect race that
        // dropped `pause_pane` and made "Stop team" fail to stop workers: the
        // user clicks before the current session's attach lands. Verify access
        // with the same gate AttachSession uses, then auto-attach so the
        // control message isn't lost.
        if let Some(uid) = state.sessions.get_web_user(connection_id) {
            let has_access = state
                .db
                .check_session_access(&sid.to_string(), &uid.to_string())
                .await
                .unwrap_or(false);
            if has_access {
                let cli_client_id = match state.db.get_session(&sid.to_string()).await {
                    Ok(Some(db_session)) => db_session
                        .cli_client_id
                        .and_then(|id| Uuid::parse_str(&id).ok())
                        .filter(|id| state.sessions.is_cli_connected(id)),
                    _ => None,
                };
                state
                    .sessions
                    .attach_web_to_session(&sid, *connection_id, cli_client_id);
                tracing::info!(
                    "Auto-attached web connection {} to session {} for control message (access verified)",
                    connection_id,
                    sid
                );
                return Some(sid);
            }
        }
        tracing::warn!(
            "Web connection {} sent a message for session {} it is not attached to",
            connection_id,
            sid
        );
        state
            .sessions
            .send_to_web(
                connection_id,
                ServerToWeb::Error {
                    message: "Not attached to that session".to_string(),
                },
            )
            .await;
        return None;
    }
    if let Some(sid) = fallback {
        return Some(sid);
    }
    state
        .sessions
        .send_to_web(
            connection_id,
            ServerToWeb::Error {
                message: "No session attached".to_string(),
            },
        )
        .await;
    None
}

fn reboot_pane_cli_message(session_id: Uuid, pane_id: u32) -> ServerToCli {
    ServerToCli::RebootPane {
        session_id,
        pane_id,
    }
}

fn reboot_cli_message(session_id: Uuid) -> ServerToCli {
    ServerToCli::RebootCli { session_id }
}

async fn handle_web_input(
    state: &AppState,
    connection_id: &Uuid,
    fallback_session_id: Option<Uuid>,
    msg_sid: Option<Uuid>,
    text: String,
    pane_type: Option<shared::PaneType>,
    pane_id: Option<u32>,
    client_msg_id: Option<String>,
) {
    let Some(sid) =
        resolve_target_session(state, connection_id, msg_sid, fallback_session_id).await
    else {
        return;
    };

    // Retransmit of an input we already stored (the web client retries
    // unacked sends): don't route/store it again -- just re-ack the sender
    // so its pending-send queue clears even if the original echo was lost.
    if let Some(ref cmid) = client_msg_id {
        if let Some(orig_created_at) = state.sessions.seen_input_id(&sid, cmid) {
            tracing::info!(
                "Dropping duplicate input retransmit for session {} (client_msg_id {})",
                sid,
                cmid
            );
            state
                .sessions
                .send_to_web(
                    connection_id,
                    ServerToWeb::UserInput {
                        session_id: sid,
                        text,
                        pane_type,
                        pane_id,
                        created_at: Some(orig_created_at),
                        client_msg_id: client_msg_id.clone(),
                    },
                )
                .await;
            return;
        }
    }

    tracing::info!(
        "Routing input to session {}: {:?}",
        sid,
        text.chars().take(50).collect::<String>()
    );

    let sent = state
        .sessions
        .route_to_cli(
            &sid,
            ServerToCli::Input {
                session_id: sid,
                data: text.clone(),
                pane_id,
            },
        )
        .await;

    if sent {
        let effective_pane_id =
            pane_id.or_else(|| pane_type.map(|p| shared::PaneConfig::pane_id_from_legacy(&p)));
        let created_at = chrono::Utc::now().to_rfc3339();
        let stored_message = crate::storage::StoredMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: "user".to_string(),
            content: text.clone(),
            message_type: "text".to_string(),
            created_at: created_at.clone(),
            pane_type: effective_pane_id.map(|id| id.to_string()),
        };
        if let Err(e) = state.storage.append_message(&sid, &stored_message).await {
            tracing::error!("Failed to save user input to file: {}", e);
        }
        if let Some(cmid) = client_msg_id.clone() {
            state
                .sessions
                .record_input_id(sid, cmid, created_at.clone());
        }

        // Echo user input to all web clients for immediate display.
        // The CLI skips CliToServer::UserInput for web-originated input
        // (from_tui=false), so this is the only display path.
        state
            .sessions
            .route_to_web(
                &sid,
                ServerToWeb::UserInput {
                    session_id: sid,
                    text,
                    pane_type,
                    pane_id,
                    created_at: Some(created_at),
                    client_msg_id,
                },
            )
            .await;

        // Count this as a prompt for the pane. The CLI deliberately skips
        // CliToServer::UserInput for web-originated input (it's already echoed
        // above), so this is the only place web prompts get recorded -- without
        // it, web chat would inflate responses/tokens but leave prompts at 0.
        crate::routes::ws_cli::record_and_broadcast_usage(
            state,
            sid,
            effective_pane_id,
            crate::db::UsageDelta {
                prompt_count: 1,
                ..Default::default()
            },
        )
        .await;
    } else {
        tracing::warn!("Failed to route input to CLI for session {}", sid);
        state
            .sessions
            .send_to_web(
                connection_id,
                ServerToWeb::Error {
                    message: "CLI client not connected".to_string(),
                },
            )
            .await;
    }
}

#[cfg(test)]
mod reboot_route_tests {
    use super::*;
    use crate::config::Config;
    use crate::db::Database;

    async fn test_state() -> AppState {
        let dir = std::env::temp_dir().join(format!("apas-reboot-pane-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp db dir");
        let db_path = dir.join("apas.db").to_string_lossy().to_string();
        let db = Database::new(&db_path).await.expect("create temp db");
        let mut config = Config::default();
        config.database.path = db_path;
        AppState::new(db, config)
    }

    #[tokio::test]
    async fn reboot_pane_falls_back_to_active_session_and_routes_requested_pane_to_cli() {
        let state = test_state().await;
        let user_id = Uuid::new_v4();
        let web_connection_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let cli_id = Uuid::new_v4();

        let (web_tx, _web_rx) = mpsc::channel(4);
        state.sessions.register_web(web_connection_id, web_tx);
        state
            .sessions
            .create_session(session_id, user_id, web_connection_id);

        let (cli_tx, mut cli_rx) = mpsc::channel(4);
        state.sessions.register_cli(cli_id, user_id, cli_tx, None);
        assert!(state.sessions.assign_cli_to_session(&session_id, cli_id));

        let sid = resolve_target_session(&state, &web_connection_id, None, Some(session_id))
            .await
            .expect("active session fallback");
        assert_eq!(sid, session_id);

        assert!(
            state
                .sessions
                .route_to_cli(&sid, reboot_pane_cli_message(sid, 42))
                .await
        );

        let msg = cli_rx.try_recv().expect("forwarded CLI message");
        match msg {
            ServerToCli::RebootPane {
                session_id: forwarded_session_id,
                pane_id,
            } => {
                assert_eq!(forwarded_session_id, session_id);
                assert_eq!(pane_id, 42);
            }
            other => panic!("expected RebootPane message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reboot_cli_falls_back_to_active_session_and_routes_full_cli_reboot_to_cli() {
        let state = test_state().await;
        let user_id = Uuid::new_v4();
        let web_connection_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let cli_id = Uuid::new_v4();

        let (web_tx, _web_rx) = mpsc::channel(4);
        state.sessions.register_web(web_connection_id, web_tx);
        state
            .sessions
            .create_session(session_id, user_id, web_connection_id);

        let (cli_tx, mut cli_rx) = mpsc::channel(4);
        state.sessions.register_cli(cli_id, user_id, cli_tx, None);
        assert!(state.sessions.assign_cli_to_session(&session_id, cli_id));

        let sid = resolve_target_session(&state, &web_connection_id, None, Some(session_id))
            .await
            .expect("active session fallback");
        assert_eq!(sid, session_id);

        assert!(
            state
                .sessions
                .route_to_cli(&sid, reboot_cli_message(sid))
                .await
        );

        let msg = cli_rx.try_recv().expect("forwarded CLI message");
        match msg {
            ServerToCli::RebootCli {
                session_id: forwarded_session_id,
            } => {
                assert_eq!(forwarded_session_id, session_id);
            }
            ServerToCli::RebootPane { .. } => {
                panic!("expected RebootCli message, got RebootPane")
            }
            other => panic!("expected RebootCli message, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod web_input_route_tests {
    use super::*;
    use crate::config::Config;
    use crate::db::Database;

    async fn test_state() -> AppState {
        let dir = std::env::temp_dir().join(format!("apas-web-input-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp db dir");
        let db_path = dir.join("apas.db").to_string_lossy().to_string();
        let db = Database::new(&db_path).await.expect("create temp db");
        let mut config = Config::default();
        config.database.path = db_path;
        AppState::new(db, config)
    }

    fn assert_cli_input(msg: ServerToCli, session_id: Uuid, text: &str, pane_id: Option<u32>) {
        match msg {
            ServerToCli::Input {
                session_id: got_session_id,
                data,
                pane_id: got_pane_id,
            } => {
                assert_eq!(got_session_id, session_id);
                assert_eq!(data, text);
                assert_eq!(got_pane_id, pane_id);
            }
            other => panic!("expected Input message, got {other:?}"),
        }
    }

    fn assert_user_input_echo(
        msg: ServerToWeb,
        session_id: Uuid,
        text: &str,
        client_msg_id: Option<&str>,
    ) -> String {
        match msg {
            ServerToWeb::UserInput {
                session_id: got_session_id,
                text: got_text,
                created_at,
                client_msg_id: got_client_msg_id,
                ..
            } => {
                assert_eq!(got_session_id, session_id);
                assert_eq!(got_text, text);
                assert_eq!(got_client_msg_id.as_deref(), client_msg_id);
                created_at.expect("input echo should include storage timestamp")
            }
            other => panic!("expected UserInput echo, got {other:?}"),
        }
    }

    async fn setup_connected_session() -> (
        AppState,
        Uuid,
        Uuid,
        mpsc::Receiver<ServerToCli>,
        mpsc::Receiver<ServerToWeb>,
    ) {
        let state = test_state().await;
        let user_id = Uuid::new_v4();
        let web_connection_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let cli_id = Uuid::new_v4();

        let (web_tx, web_rx) = mpsc::channel(8);
        state.sessions.register_web(web_connection_id, web_tx);
        state
            .sessions
            .create_session(session_id, user_id, web_connection_id);

        let (cli_tx, cli_rx) = mpsc::channel(8);
        state.sessions.register_cli(cli_id, user_id, cli_tx, None);
        assert!(state.sessions.assign_cli_to_session(&session_id, cli_id));

        (state, web_connection_id, session_id, cli_rx, web_rx)
    }

    #[tokio::test]
    async fn duplicate_client_msg_id_is_reacked_without_forwarding_or_storing() {
        let (state, web_connection_id, session_id, mut cli_rx, mut web_rx) =
            setup_connected_session().await;

        handle_web_input(
            &state,
            &web_connection_id,
            Some(session_id),
            None,
            "first send".to_string(),
            None,
            Some(7),
            Some("client-1".to_string()),
        )
        .await;

        assert_cli_input(
            cli_rx.try_recv().expect("first input routed to CLI"),
            session_id,
            "first send",
            Some(7),
        );
        let created_at = assert_user_input_echo(
            web_rx.try_recv().expect("first input echoed to web"),
            session_id,
            "first send",
            Some("client-1"),
        );
        assert_eq!(
            state
                .sessions
                .seen_input_id(&session_id, "client-1")
                .as_deref(),
            Some(created_at.as_str())
        );

        handle_web_input(
            &state,
            &web_connection_id,
            Some(session_id),
            None,
            "first send".to_string(),
            None,
            Some(7),
            Some("client-1".to_string()),
        )
        .await;

        assert!(matches!(
            cli_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        let duplicate_created_at = assert_user_input_echo(
            web_rx
                .try_recv()
                .expect("duplicate input re-acked to sender"),
            session_id,
            "first send",
            Some("client-1"),
        );
        assert_eq!(duplicate_created_at, created_at);
        let messages = state
            .storage
            .get_messages(&session_id)
            .await
            .expect("stored messages");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "first send");
    }

    #[tokio::test]
    async fn missing_client_msg_id_uses_old_client_fallback_and_flows_normally() {
        let (state, web_connection_id, session_id, mut cli_rx, mut web_rx) =
            setup_connected_session().await;

        for text in ["legacy one", "legacy two"] {
            handle_web_input(
                &state,
                &web_connection_id,
                Some(session_id),
                None,
                text.to_string(),
                None,
                None,
                None,
            )
            .await;
        }

        assert_cli_input(
            cli_rx.try_recv().expect("first legacy input routed"),
            session_id,
            "legacy one",
            None,
        );
        assert_cli_input(
            cli_rx.try_recv().expect("second legacy input routed"),
            session_id,
            "legacy two",
            None,
        );
        assert_user_input_echo(
            web_rx.try_recv().expect("first legacy input echoed"),
            session_id,
            "legacy one",
            None,
        );
        assert_user_input_echo(
            web_rx.try_recv().expect("second legacy input echoed"),
            session_id,
            "legacy two",
            None,
        );

        let messages = state
            .storage
            .get_messages(&session_id)
            .await
            .expect("stored messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "legacy one");
        assert_eq!(messages[1].content, "legacy two");
        assert_eq!(state.sessions.seen_input_id(&session_id, "legacy"), None);
    }
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let connection_id = Uuid::new_v4();

    // Channel for sending messages to this web client. Broadcasts use
    // try_send (route_to_web) and drop frames when this is full, so size
    // it to absorb streaming bursts — only a genuinely stalled connection
    // should ever fill it.
    let (tx, mut rx) = mpsc::channel::<ServerToWeb>(256);

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
                                    let user_email =
                                        match state.db.get_user_by_id(&uid.to_string()).await {
                                            Ok(Some(user)) => Some(user.email),
                                            Ok(None) => None,
                                            Err(err) => {
                                                tracing::warn!(
                                                    "Failed to fetch email for user {}: {}",
                                                    uid,
                                                    err
                                                );
                                                None
                                            }
                                        };
                                    tracing::info!(
                                        "Web client {} authenticated as user {}",
                                        connection_id,
                                        uid
                                    );
                                    state.sessions.set_web_user(connection_id, uid);
                                    state
                                        .sessions
                                        .send_to_web(
                                            &connection_id,
                                            ServerToWeb::Authenticated {
                                                user_id: uid,
                                                user_email,
                                                server_version: Some(SERVER_VERSION.to_string()),
                                            },
                                        )
                                        .await;

                                    // Send cached usage limits for all CLI clients
                                    for (cli_id, provider, limits) in
                                        state.sessions.get_all_usage_limits()
                                    {
                                        state
                                            .sessions
                                            .send_to_web(
                                                &connection_id,
                                                ServerToWeb::UsageLimits {
                                                    cli_client_id: cli_id,
                                                    provider,
                                                    limits,
                                                },
                                            )
                                            .await;
                                    }

                                    // Send daemon-reported machines for this user.
                                    let machines =
                                        list_accessible_machines_for_user(&state, &uid).await;
                                    state
                                        .sessions
                                        .send_to_web(
                                            &connection_id,
                                            ServerToWeb::Machines { machines },
                                        )
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
                        .send_to_web(&connection_id, ServerToWeb::CliClients { clients })
                        .await;
                }
                Ok(WebToServer::ListMachines) => {
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

                    let machines = list_accessible_machines_for_user(&state, &uid).await;
                    state
                        .sessions
                        .send_to_web(&connection_id, ServerToWeb::Machines { machines })
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
                    let cli_id = cli_client_id
                        .or_else(|| state.sessions.get_online_cli_ids().first().copied());

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
                                pane_id: None,
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
                        .send_to_web(&connection_id, ServerToWeb::SessionStatus { status })
                        .await;

                    tracing::info!("Session started: {} (CLI: {:?})", new_session_id, cli_id);
                }
                Ok(WebToServer::Input {
                    session_id: msg_sid,
                    text,
                    pane_type,
                    pane_id,
                    client_msg_id,
                }) => {
                    handle_web_input(
                        &state,
                        &connection_id,
                        session_id,
                        msg_sid,
                        text,
                        pane_type,
                        pane_id,
                        client_msg_id,
                    )
                    .await;
                }
                Ok(WebToServer::Heartbeat) => {
                    // Round-trip: client-side liveness detector watches
                    // for any inbound frame, this one being the cheapest
                    // when the daemon happens to be idle. Without a
                    // server-side responder, an idle session looks dead
                    // to the browser's liveness timer and we'd reconnect
                    // unnecessarily.
                    state
                        .sessions
                        .send_to_web(&connection_id, ServerToWeb::Heartbeat)
                        .await;
                }
                Ok(WebToServer::Signal {
                    session_id: msg_sid,
                    signal,
                }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    else {
                        continue;
                    };
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
                Ok(WebToServer::Approve {
                    session_id: msg_sid,
                    tool_call_id: _,
                }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    else {
                        continue;
                    };
                    state
                        .sessions
                        .route_to_cli(
                            &sid,
                            ServerToCli::Input {
                                session_id: sid,
                                data: "y".to_string(),
                                pane_id: None,
                            },
                        )
                        .await;
                }
                Ok(WebToServer::Reject {
                    session_id: msg_sid,
                    tool_call_id: _,
                }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    else {
                        continue;
                    };
                    state
                        .sessions
                        .route_to_cli(
                            &sid,
                            ServerToCli::Input {
                                session_id: sid,
                                data: "n".to_string(),
                                pane_id: None,
                            },
                        )
                        .await;
                }
                Ok(WebToServer::PauseDeadloop) => {
                    if let Some(sid) = session_id {
                        tracing::info!("Pausing deadloop for session {}", sid);
                        state
                            .sessions
                            .route_to_cli(&sid, ServerToCli::PauseDeadloop { session_id: sid })
                            .await;
                    }
                }
                Ok(WebToServer::ResumeDeadloop) => {
                    if let Some(sid) = session_id {
                        tracing::info!("Resuming deadloop for session {}", sid);
                        state
                            .sessions
                            .route_to_cli(&sid, ServerToCli::ResumeDeadloop { session_id: sid })
                            .await;
                    }
                }
                Ok(WebToServer::RebootCli) => {
                    if let Some(sid) = session_id {
                        tracing::info!("Rebooting CLI for session {}", sid);
                        state
                            .sessions
                            .route_to_cli(&sid, reboot_cli_message(sid))
                            .await;
                    }
                }
                Ok(WebToServer::StartMachineProjectCli {
                    machine_id,
                    project_id,
                }) => {
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

                    tracing::info!(
                        "StartMachineProjectCli: user={} machine={} project={}",
                        uid, machine_id, project_id
                    );
                    let allowed = state
                        .sessions
                        .get_machines_for_user(&uid)
                        .into_iter()
                        .any(|m| {
                            m.machine.machine_id == machine_id
                                && m.projects.iter().any(|p| p.project_id == project_id)
                        })
                        || {
                            let (host_path_refs, wildcard_paths) =
                                get_shared_project_access_refs(&state, &uid).await;
                            state.sessions.machine_project_matches_refs(
                                &machine_id,
                                &project_id,
                                &host_path_refs,
                                &wildcard_paths,
                            )
                        };

                    if !allowed {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Machine not found".to_string(),
                                },
                            )
                            .await;
                        continue;
                    }

                    if !state
                        .sessions
                        .send_to_daemon(&machine_id, ServerToDaemon::StartProjectCli { project_id })
                        .await
                    {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Daemon is offline".to_string(),
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::StopMachineProjectCli {
                    machine_id,
                    project_id,
                }) => {
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

                    let allowed = state
                        .sessions
                        .get_machines_for_user(&uid)
                        .into_iter()
                        .any(|m| {
                            m.machine.machine_id == machine_id
                                && m.projects.iter().any(|p| p.project_id == project_id)
                        })
                        || {
                            let (host_path_refs, wildcard_paths) =
                                get_shared_project_access_refs(&state, &uid).await;
                            state.sessions.machine_project_matches_refs(
                                &machine_id,
                                &project_id,
                                &host_path_refs,
                                &wildcard_paths,
                            )
                        };

                    if !allowed {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Machine not found".to_string(),
                                },
                            )
                            .await;
                        continue;
                    }

                    if !state
                        .sessions
                        .send_to_daemon(&machine_id, ServerToDaemon::StopProjectCli { project_id })
                        .await
                    {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Daemon is offline".to_string(),
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::CreateProjectInstance {
                    machine_id,
                    git_remote,
                    instance_name,
                    branch,
                    clone_url,
                    base_path,
                    request_id,
                }) => {
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

                    // A brand-new instance has no project_id yet, so authorize by
                    // machine OWNERSHIP only (the daemon must belong to this user).
                    let owns_machine = state
                        .sessions
                        .get_machines_for_user(&uid)
                        .into_iter()
                        .any(|m| m.machine.machine_id == machine_id);
                    if !owns_machine {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Machine not found".to_string(),
                                },
                            )
                            .await;
                        continue;
                    }

                    tracing::info!(
                        "CreateProjectInstance: user={} machine={} repo={} name={}",
                        uid, machine_id, git_remote, instance_name
                    );

                    if !state
                        .sessions
                        .send_to_daemon(
                            &machine_id,
                            ServerToDaemon::CreateProjectInstance {
                                git_remote,
                                instance_name,
                                branch,
                                clone_url,
                                base_path,
                                request_id,
                            },
                        )
                        .await
                    {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Daemon is offline".to_string(),
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::SetMachineMiniMaxConfig {
                    machine_id,
                    api_base_url,
                    api_key,
                    clear_api_key,
                }) => {
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

                    let owned = state
                        .sessions
                        .get_machines_for_user(&uid)
                        .into_iter()
                        .any(|m| m.machine.machine_id == machine_id);

                    if !owned {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Machine not found".to_string(),
                                },
                            )
                            .await;
                        continue;
                    }

                    let req_api_base_url = api_base_url.clone();
                    let req_api_key = api_key.clone();
                    if !state
                        .sessions
                        .send_to_daemon(
                            &machine_id,
                            ServerToDaemon::SetMiniMaxConfig {
                                api_base_url,
                                api_key,
                                clear_api_key,
                            },
                        )
                        .await
                    {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Daemon is offline".to_string(),
                                },
                            )
                            .await;
                    } else {
                        // Optimistically reflect updated config in machine snapshot for web UI.
                        state.sessions.apply_web_minimax_config(
                            &machine_id,
                            req_api_base_url,
                            req_api_key,
                            clear_api_key,
                        );
                    }
                }
                Ok(WebToServer::SetMachineGlmConfig {
                    machine_id,
                    api_base_url,
                    api_key,
                    clear_api_key,
                }) => {
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

                    let owned = state
                        .sessions
                        .get_machines_for_user(&uid)
                        .into_iter()
                        .any(|m| m.machine.machine_id == machine_id);

                    if !owned {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Machine not found".to_string(),
                                },
                            )
                            .await;
                        continue;
                    }

                    let req_api_base_url = api_base_url.clone();
                    let req_api_key = api_key.clone();
                    if !state
                        .sessions
                        .send_to_daemon(
                            &machine_id,
                            ServerToDaemon::SetGlmConfig {
                                api_base_url,
                                api_key,
                                clear_api_key,
                            },
                        )
                        .await
                    {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Daemon is offline".to_string(),
                                },
                            )
                            .await;
                    } else {
                        state.sessions.apply_web_glm_config(
                            &machine_id,
                            req_api_base_url,
                            req_api_key,
                            clear_api_key,
                        );
                    }
                }
                Ok(WebToServer::SetMachineDeepseekConfig {
                    machine_id,
                    api_base_url,
                    api_key,
                    clear_api_key,
                }) => {
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

                    let owned = state
                        .sessions
                        .get_machines_for_user(&uid)
                        .into_iter()
                        .any(|m| m.machine.machine_id == machine_id);

                    if !owned {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Machine not found".to_string(),
                                },
                            )
                            .await;
                        continue;
                    }

                    let req_api_base_url = api_base_url.clone();
                    let req_api_key = api_key.clone();
                    if !state
                        .sessions
                        .send_to_daemon(
                            &machine_id,
                            ServerToDaemon::SetDeepseekConfig {
                                api_base_url,
                                api_key,
                                clear_api_key,
                            },
                        )
                        .await
                    {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Daemon is offline".to_string(),
                                },
                            )
                            .await;
                    } else {
                        state.sessions.apply_web_deepseek_config(
                            &machine_id,
                            req_api_base_url,
                            req_api_key,
                            clear_api_key,
                        );
                    }
                }
                Ok(WebToServer::PausePane {
                    session_id: msg_sid,
                    pane_id,
                }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    else {
                        continue;
                    };
                    tracing::info!("Pausing pane {} for session {}", pane_id, sid);
                    state
                        .sessions
                        .route_to_cli(
                            &sid,
                            ServerToCli::PausePane {
                                session_id: sid,
                                pane_id,
                            },
                        )
                        .await;
                }
                Ok(WebToServer::ResumePane {
                    session_id: msg_sid,
                    pane_id,
                }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    else {
                        continue;
                    };
                    tracing::info!("Resuming pane {} for session {}", pane_id, sid);
                    state
                        .sessions
                        .route_to_cli(
                            &sid,
                            ServerToCli::ResumePane {
                                session_id: sid,
                                pane_id,
                            },
                        )
                        .await;
                }
                Ok(WebToServer::RebootPane {
                    session_id: msg_sid,
                    pane_id,
                }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    else {
                        continue;
                    };
                    tracing::info!("Rebooting pane {} for session {}", pane_id, sid);
                    state
                        .sessions
                        .route_to_cli(&sid, reboot_pane_cli_message(sid, pane_id))
                        .await;
                }
                Ok(WebToServer::AddPane {
                    provider,
                    mode,
                    label,
                    prompt,
                    model,
                    isolated_worktree,
                    role,
                    goal,
                    backstory,
                    plan_review_mode,
                    managed,
                }) => {
                    if let Some(sid) = session_id {
                        // Generate a unique pane_id starting from 3 (1 and 2 are reserved for legacy deadloop/interactive)
                        let pane_id = 3 + (uuid::Uuid::new_v4().as_u128() % 1000) as u32;
                        let pane_config = shared::PaneConfig {
                            pane_id,
                            provider,
                            mode,
                            session_id: uuid::Uuid::new_v4(),
                            is_paused: false,
                            stop_requested: false,
                            prompt,
                            min_iteration_interval_minutes: None,
                            label,
                            model,
                            effort: None,
                            worktree_path: None,
                            role,
                            goal,
                            backstory,
                            plan_review_mode,
                            manual_mode: false,
                            managed,
                        };
                        tracing::info!(
                            "Adding pane {} to session {} (isolated_worktree={})",
                            pane_id, sid, isolated_worktree,
                        );
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::AddPane {
                                    session_id: sid,
                                    pane_config: pane_config.clone(),
                                    isolated_worktree,
                                },
                            )
                            .await;
                        // Also broadcast PaneList to web clients
                        // (CLI will send back updated pane config)
                    }
                }
                Ok(WebToServer::RemovePane { pane_id, cleanup_action }) => {
                    if let Some(sid) = session_id {
                        tracing::info!(
                            "Removing pane {} from session {} (cleanup_action={:?})",
                            pane_id, sid, cleanup_action,
                        );
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::RemovePane {
                                    session_id: sid,
                                    pane_id,
                                    cleanup_action,
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::UpdatePaneLabel { pane_id, label }) => {
                    if let Some(sid) = session_id {
                        tracing::info!("Updating pane {} label in session {}", pane_id, sid);
                        let mut panes = state.sessions.get_session_panes(&sid);
                        if let Some(pane) = panes.iter_mut().find(|p| p.pane_id == pane_id) {
                            pane.label = Some(label.clone());
                            state.sessions.set_session_panes(&sid, panes.clone());
                            let _ = state.storage.save_pane_list(&sid, &panes).await;
                            state
                                .sessions
                                .route_to_web(
                                    &sid,
                                    ServerToWeb::PaneList {
                                        session_id: sid,
                                        panes,
                                    },
                                )
                                .await;
                        }
                        // Also forward to CLI so it updates meta.label and
                        // persists to .apas — without this, the rename is
                        // cache-only and gets clobbered on next CLI restart
                        // by the CLI's PaneList carrying the on-disk label.
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::UpdatePaneLabel {
                                    session_id: sid,
                                    pane_id,
                                    label,
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::InterruptPane { session_id: msg_sid, pane_id }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    else {
                        continue;
                    };
                    tracing::info!("Interrupting pane {} in session {}", pane_id, sid);
                    state
                        .sessions
                        .route_to_cli(
                            &sid,
                            ServerToCli::InterruptPane {
                                session_id: sid,
                                pane_id,
                            },
                        )
                        .await;
                }
                Ok(WebToServer::PlanReviewAnswer { tool_use_id, approve }) => {
                    if let Some(sid) = session_id {
                        tracing::info!(
                            "Plan review answer for session {}: {} → {}",
                            sid, tool_use_id, approve,
                        );
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::PlanReviewAnswer {
                                    session_id: sid,
                                    tool_use_id,
                                    approve,
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::UpdatePaneReviewMode { pane_id, mode }) => {
                    if let Some(sid) = session_id {
                        tracing::info!(
                            "Update pane {} plan_review_mode for session {} → {:?}",
                            pane_id, sid, mode,
                        );
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::UpdatePaneReviewMode {
                                    session_id: sid,
                                    pane_id,
                                    mode,
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::UpdatePaneManualMode { pane_id, manual_mode }) => {
                    if let Some(sid) = session_id {
                        tracing::info!(
                            "Update pane {} manual_mode for session {} → {}",
                            pane_id, sid, manual_mode,
                        );
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::UpdatePaneManualMode {
                                    session_id: sid,
                                    pane_id,
                                    manual_mode,
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::FetchTeamTodo { session_id: msg_sid }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, Some(msg_sid), session_id)
                            .await
                    else {
                        continue;
                    };
                    state
                        .sessions
                        .route_to_cli(&sid, ServerToCli::FetchTeamTodo { session_id: sid })
                        .await;
                }
                Ok(WebToServer::TodoApproval { session_id: msg_sid, todo_id, action }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, Some(msg_sid), session_id)
                            .await
                    else {
                        continue;
                    };
                    tracing::info!(
                        "Todo approval for session {}: {} → {}",
                        sid, todo_id, action
                    );
                    state
                        .sessions
                        .route_to_cli(
                            &sid,
                            ServerToCli::TodoApproval {
                                session_id: sid,
                                todo_id,
                                action,
                            },
                        )
                        .await;
                }
                Ok(WebToServer::AddTodo { session_id: msg_sid, title, body }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, Some(msg_sid), session_id)
                            .await
                    else {
                        continue;
                    };
                    tracing::info!(
                        "Add TODO for session {}: {} ({} bytes)",
                        sid, title, body.len()
                    );
                    state
                        .sessions
                        .route_to_cli(
                            &sid,
                            ServerToCli::AddTodo {
                                session_id: sid,
                                title,
                                body,
                            },
                        )
                        .await;
                }
                Ok(WebToServer::FetchSuggestedWorkers { session_id: msg_sid }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, Some(msg_sid), session_id)
                            .await
                    else {
                        continue;
                    };
                    state
                        .sessions
                        .route_to_cli(
                            &sid,
                            ServerToCli::FetchSuggestedWorkers { session_id: sid },
                        )
                        .await;
                }
                Ok(WebToServer::DismissSuggestion { session_id: msg_sid, suggestion_id }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, Some(msg_sid), session_id)
                            .await
                    else {
                        continue;
                    };
                    tracing::info!(
                        "Dismiss suggestion {} for session {}",
                        suggestion_id, sid
                    );
                    state
                        .sessions
                        .route_to_cli(
                            &sid,
                            ServerToCli::DismissSuggestion {
                                session_id: sid,
                                suggestion_id,
                            },
                        )
                        .await;
                }
                Ok(WebToServer::PromotePaneToManaged { session_id: msg_sid, pane_id }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, Some(msg_sid), session_id)
                            .await
                    else {
                        continue;
                    };
                    tracing::info!(
                        "Promote pane {} → managed for session {}",
                        pane_id, sid
                    );
                    state
                        .sessions
                        .route_to_cli(
                            &sid,
                            ServerToCli::PromotePaneToManaged {
                                session_id: sid,
                                pane_id,
                            },
                        )
                        .await;
                }
                Ok(WebToServer::UpdatePaneRole { pane_id, role, goal, backstory }) => {
                    if let Some(sid) = session_id {
                        tracing::info!(
                            "Updating pane {} role for session {} (role={:?}, goal={:?})",
                            pane_id, sid, role, goal,
                        );
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::UpdatePaneRole {
                                    session_id: sid,
                                    pane_id,
                                    role,
                                    goal,
                                    backstory,
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::RequestPaneDiff { pane_id }) => {
                    if let Some(sid) = session_id {
                        tracing::info!("Requesting pane diff for pane {} in session {}", pane_id, sid);
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::RequestPaneDiff {
                                    session_id: sid,
                                    pane_id,
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::CreatePr { pane_id }) => {
                    if let Some(sid) = session_id {
                        tracing::info!("Create PR for pane {} in session {}", pane_id, sid);
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::CreatePr {
                                    session_id: sid,
                                    pane_id,
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::UpdateProjectGoal { goal }) => {
                    if let Some(sid) = session_id {
                        tracing::info!(
                            "Update project_goal for session {} ({} bytes)",
                            sid,
                            goal.len()
                        );
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::UpdateProjectGoal {
                                    session_id: sid,
                                    goal,
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::UpdateProjectFlags {
                    auto_approve_todos,
                    auto_merge_prs,
                }) => {
                    if let Some(sid) = session_id {
                        tracing::info!(
                            session_id = %sid,
                            auto_approve_todos,
                            auto_merge_prs,
                            "Update project flags"
                        );
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::UpdateProjectFlags {
                                    session_id: sid,
                                    auto_approve_todos,
                                    auto_merge_prs,
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::StartTeam {
                    manager,
                    tech_lead,
                    reviewer,
                    developer,
                }) => {
                    if let Some(sid) = session_id {
                        tracing::info!(session_id = %sid, "Start team");
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::StartTeam {
                                    session_id: sid,
                                    manager,
                                    tech_lead,
                                    reviewer,
                                    developer,
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::AnswerQuestion { tool_use_id, answers }) => {
                    // Relay the user's AskUserQuestion answers down to the CLI
                    // streaming worker. The worker matches by tool_use_id
                    // against its pending_questions map and writes the
                    // control_response onto claude's stdin so the SDK's
                    // canUseTool callback completes.
                    if let Some(sid) = session_id {
                        tracing::info!(
                            tool_use_id = tool_use_id.as_str(),
                            answer_count = answers.len(),
                            "Forwarding AskUserQuestion answers to CLI for session {}",
                            sid
                        );
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::AnswerQuestion {
                                    session_id: sid,
                                    tool_use_id,
                                    answers,
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::UpdatePaneEffort { pane_id, effort }) => {
                    if let Some(sid) = session_id {
                        let normalized = effort.as_deref().and_then(normalize_start_bot_effort);
                        tracing::info!(
                            "Updating pane {} effort in session {} to {:?}",
                            pane_id, sid, normalized
                        );
                        let mut panes = state.sessions.get_session_panes(&sid);
                        if let Some(pane) = panes.iter_mut().find(|p| p.pane_id == pane_id) {
                            pane.effort = normalized.clone();
                            state.sessions.set_session_panes(&sid, panes.clone());
                            let _ = state.storage.save_pane_list(&sid, &panes).await;
                            state
                                .sessions
                                .route_to_web(
                                    &sid,
                                    ServerToWeb::PaneList {
                                        session_id: sid,
                                        panes,
                                    },
                                )
                                .await;
                            // Forward to CLI so it persists to the .apas file.
                            state
                                .sessions
                                .route_to_cli(
                                    &sid,
                                    ServerToCli::UpdatePaneEffort {
                                        session_id: sid,
                                        pane_id,
                                        effort: normalized,
                                    },
                                )
                                .await;
                        }
                    }
                }
                Ok(WebToServer::UpdatePaneModel { pane_id, provider, model }) => {
                    if let Some(sid) = session_id {
                        let trimmed = model
                            .as_deref()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string);
                        tracing::info!(
                            "Switching pane {} agent in session {} to provider={:?} model={:?}",
                            pane_id, sid, provider, trimmed
                        );
                        let mut panes = state.sessions.get_session_panes(&sid);
                        if let Some(pane) = panes.iter_mut().find(|p| p.pane_id == pane_id) {
                            if let Some(p) = provider {
                                pane.provider = p;
                            }
                            pane.model = trimmed.clone();
                            state.sessions.set_session_panes(&sid, panes.clone());
                            let _ = state.storage.save_pane_list(&sid, &panes).await;
                            state
                                .sessions
                                .route_to_web(
                                    &sid,
                                    ServerToWeb::PaneList {
                                        session_id: sid,
                                        panes,
                                    },
                                )
                                .await;
                        }
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::UpdatePaneModel {
                                    session_id: sid,
                                    pane_id,
                                    provider,
                                    model: trimmed,
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::ReorderPanes { pane_ids }) => {
                    if let Some(sid) = session_id {
                        tracing::info!("Reordering panes in session {}", sid);
                        let panes = state.sessions.get_session_panes(&sid);
                        let order_map: std::collections::HashMap<u32, usize> =
                            pane_ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
                        let mut ordered: Vec<shared::PaneConfig> = Vec::new();
                        let mut remaining: Vec<shared::PaneConfig> = Vec::new();
                        for pane in panes {
                            if let Some(&pos) = order_map.get(&pane.pane_id) {
                                ordered.push(pane);
                            } else {
                                remaining.push(pane);
                            }
                        }
                        ordered.sort_by_key(|p| order_map.get(&p.pane_id).copied().unwrap_or(usize::MAX));
                        let new_panes = [ordered, remaining].concat();
                        state.sessions.set_session_panes(&sid, new_panes.clone());
                        let _ = state.storage.save_pane_list(&sid, &new_panes).await;
                        state
                            .sessions
                            .route_to_web(
                                &sid,
                                ServerToWeb::PaneList {
                                    session_id: sid,
                                    panes: new_panes,
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::StartBot {
                    pane_id,
                    prompt,
                    min_iteration_interval_minutes,
                    effort,
                }) => {
                    if let Some(sid) = session_id {
                        tracing::info!("Starting bot on pane {} for session {}", pane_id, sid);
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::StartBot {
                                    session_id: sid,
                                    pane_id,
                                    prompt: prompt.clone(),
                                    min_iteration_interval_minutes,
                                    effort: effort.clone(),
                                },
                            )
                            .await;

                        // Optimistically update cached pane state so web
                        // reflects the change even if the CLI PaneList is lost.
                        let mut panes = state.sessions.get_session_panes(&sid);
                        if let Some(pane) = panes.iter_mut().find(|p| p.pane_id == pane_id) {
                            pane.mode = shared::PaneMode::Deadloop;
                            pane.prompt = prompt;
                            pane.min_iteration_interval_minutes = min_iteration_interval_minutes
                                .or(pane.min_iteration_interval_minutes);
                            if let Some(requested_effort) = effort.as_deref() {
                                pane.effort = normalize_start_bot_effort(requested_effort);
                            }
                            pane.stop_requested = false;
                            state.sessions.set_session_panes(&sid, panes.clone());
                            let _ = state.storage.save_pane_list(&sid, &panes).await;
                            state
                                .sessions
                                .route_to_web(
                                    &sid,
                                    ServerToWeb::PaneList {
                                        session_id: sid,
                                        panes,
                                    },
                                )
                                .await;
                        }
                    }
                }
                Ok(WebToServer::StopBot { pane_id }) => {
                    if let Some(sid) = session_id {
                        tracing::info!("Stopping bot on pane {} for session {}", pane_id, sid);
                        let routed = state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::StopBot {
                                    session_id: sid,
                                    pane_id,
                                },
                            )
                            .await;
                        tracing::info!("StopBot routed to CLI: {}", routed);

                        // Optimistically set stop_requested so web shows
                        // "Force Stop" without waiting for CLI PaneList.
                        let mut panes = state.sessions.get_session_panes(&sid);
                        if let Some(pane) = panes.iter_mut().find(|p| p.pane_id == pane_id) {
                            pane.stop_requested = true;
                            state.sessions.set_session_panes(&sid, panes.clone());
                            let _ = state.storage.save_pane_list(&sid, &panes).await;
                            state
                                .sessions
                                .route_to_web(
                                    &sid,
                                    ServerToWeb::PaneList {
                                        session_id: sid,
                                        panes,
                                    },
                                )
                                .await;
                        }
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
                    let has_access = match state
                        .db
                        .check_session_access(&sid.to_string(), &uid.to_string())
                        .await
                    {
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

                    // Look up CLI client ID from database, but only trust it if
                    // the CLI is actually connected (prevents stale IDs after server restart)
                    let cli_client_id = match state.db.get_session(&sid.to_string()).await {
                        Ok(Some(db_session)) => db_session
                            .cli_client_id
                            .and_then(|id| Uuid::parse_str(&id).ok())
                            .filter(|id| state.sessions.is_cli_connected(id)),
                        _ => None,
                    };

                    // Attach to an existing CLI session to observe output
                    if state
                        .sessions
                        .attach_web_to_session(&sid, connection_id, cli_client_id)
                    {
                        session_id = Some(sid);
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::SessionStarted {
                                    session_id: sid,
                                    pane_type: None,
                                    pane_id: None,
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

                        // Send attached confirmation with CLI active status
                        // This allows shared users to see pause/reboot buttons
                        let has_active_cli = state.sessions.is_session_active(&sid);
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::SessionAttached {
                                    session_id: sid,
                                    has_active_cli,
                                },
                            )
                            .await;

                        // Replay current usage stats so the Overview panel is
                        // populated immediately on attach / hard refresh.
                        match state.db.get_project_usage_stats(&sid.to_string()).await {
                            Ok(stats) => {
                                state
                                    .sessions
                                    .send_to_web(
                                        &connection_id,
                                        ServerToWeb::ProjectUsageStats {
                                            session_id: sid,
                                            stats,
                                        },
                                    )
                                    .await;
                            }
                            Err(e) => {
                                tracing::error!("Failed to load usage stats on attach: {}", e)
                            }
                        }

                        // Send current pause state for this session
                        // First check in-memory state, then fall back to database (for server restart recovery)
                        let is_paused = if state.sessions.has_session_state(&sid) {
                            state.sessions.is_session_paused(&sid)
                        } else {
                            // Load from database (server may have restarted)
                            match state.db.get_session(&sid.to_string()).await {
                                Ok(Some(db_session)) => {
                                    // Cache it in memory for future lookups
                                    state
                                        .sessions
                                        .set_session_paused(&sid, db_session.is_paused);
                                    db_session.is_paused
                                }
                                _ => false,
                            }
                        };
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::DeadloopStatus {
                                    session_id: sid,
                                    is_paused,
                                },
                            )
                            .await;

                        // Lazy-load mode: don't ship every pane's tail on attach.
                        // The web fetches per-pane history on demand the first
                        // time the user opens a tab, via GetSessionMessages
                        // with a pane_id filter. Attach response carries just
                        // the metadata (pane list, statuses, goal) so the
                        // payload is small even on huge sessions like rusty-cpp
                        // (1+ GB messages.jsonl).
                        let messages: Vec<MessageInfo> = Vec::new();
                        let has_more = false;

                        // Restore pane list for inactive sessions:
                        // 1) in-memory cache, 2) persisted pane metadata, 3) inferred from messages.
                        let mut panes_to_send = state.sessions.get_session_panes(&sid);
                        if panes_to_send.is_empty() {
                            match state.storage.load_pane_list(&sid).await {
                                Ok(stored_panes) if !stored_panes.is_empty() => {
                                    state.sessions.set_session_panes(&sid, stored_panes.clone());
                                    panes_to_send = stored_panes;
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    tracing::warn!(
                                        "Failed to load pane list metadata for session {}: {}",
                                        sid,
                                        e
                                    );
                                }
                            }
                        }
                        if panes_to_send.is_empty() {
                            panes_to_send = infer_panes_from_messages(sid, &messages);
                            if !panes_to_send.is_empty() {
                                state
                                    .sessions
                                    .set_session_panes(&sid, panes_to_send.clone());
                            }
                        }
                        if !panes_to_send.is_empty() {
                            state
                                .sessions
                                .send_to_web(
                                    &connection_id,
                                    ServerToWeb::PaneList {
                                        session_id: sid,
                                        panes: panes_to_send,
                                    },
                                )
                                .await;
                        }

                        // Request fresh pane list from CLI to correct any stale cached data
                        state
                            .sessions
                            .route_to_cli(&sid, ServerToCli::RequestPaneList { session_id: sid })
                            .await;

                        // Replay the last known pane statuses so the "thinking"
                        // indicator survives tab/project switches. Without this,
                        // the web client clears paneStatuses on attach and would
                        // wait for the next CLI status change to repopulate it.
                        for (pane_type, pane_id, status) in state.sessions.get_pane_statuses(&sid) {
                            state
                                .sessions
                                .send_to_web(
                                    &connection_id,
                                    ServerToWeb::PaneStatus {
                                        session_id: sid,
                                        pane_type,
                                        pane_id: Some(pane_id),
                                        status: Some(status),
                                    },
                                )
                                .await;
                        }

                        // Replay the cached project_goal so a hard-refreshed
                        // web client sees the current goal without waiting
                        // for the CLI's mtime poller to fire on an actual
                        // file change (which may not happen for hours).
                        state
                            .sessions
                            .replay_project_goal_to_web(&sid, &connection_id)
                            .await;

                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::SessionMessages {
                                    session_id: sid,
                                    messages,
                                    has_more,
                                    catchup: false,
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
                    let owned_sessions =
                        match state.db.get_sessions_for_user(&uid.to_string()).await {
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
                    let shared_sessions = match state
                        .db
                        .get_shared_sessions_for_user(&uid.to_string())
                        .await
                    {
                        Ok(sessions) => sessions,
                        Err(e) => {
                            tracing::error!("Failed to get shared sessions: {}", e);
                            vec![] // Continue without shared sessions
                        }
                    };

                    // Combine owned and shared sessions
                    let mut sessions: Vec<SessionInfo> = owned_sessions
                        .into_iter()
                        .map(|s| {
                            let session_id = Uuid::parse_str(&s.id).unwrap_or_default();
                            let is_active = state.sessions.is_session_active(&session_id);
                            let project_id = s
                                .project_id
                                .as_deref()
                                .and_then(|p| Uuid::parse_str(p).ok())
                                .or(Some(session_id));
                            SessionInfo {
                                id: session_id,
                                project_id,
                                cli_client_id: s
                                    .cli_client_id
                                    .and_then(|id| Uuid::parse_str(&id).ok()),
                                working_dir: s.working_dir,
                                hostname: s.hostname,
                                git_remote: s.git_remote,
                                git_remote_url: s.git_remote_url,
                                status: s.status,
                                created_at: s.created_at,
                                is_shared: false,
                                owner_email: None,
                                share_role: Some("owner".to_string()),
                                is_active,
                            }
                        })
                        .collect();

                    // Add shared sessions with owner email
                    for (s, owner_email, share_role) in shared_sessions {
                        let session_id = Uuid::parse_str(&s.id).unwrap_or_default();
                        let is_active = state.sessions.is_session_active(&session_id);
                        let project_id = s
                            .project_id
                            .as_deref()
                            .and_then(|p| Uuid::parse_str(p).ok())
                            .or(Some(session_id));
                        sessions.push(SessionInfo {
                            id: session_id,
                            project_id,
                            cli_client_id: s.cli_client_id.and_then(|id| Uuid::parse_str(&id).ok()),
                            working_dir: s.working_dir,
                            hostname: s.hostname,
                            git_remote: s.git_remote,
                            git_remote_url: s.git_remote_url,
                            status: s.status,
                            created_at: s.created_at,
                            is_shared: true,
                            owner_email: Some(owner_email),
                            share_role: Some(share_role),
                            is_active,
                        });
                    }

                    state
                        .sessions
                        .send_to_web(&connection_id, ServerToWeb::Sessions { sessions })
                        .await;
                }
                Ok(WebToServer::GetSessionMessages {
                    session_id: sid,
                    limit,
                    before_id,
                    pane_type,
                    pane_id,
                    after_created_at,
                    pane_watermarks,
                }) => {
                    // Get messages for a specific session from file storage with pagination
                    let limit = limit.unwrap_or(100);
                    // Use pane_id for filtering if provided, otherwise fall back to pane_type
                    let effective_pane_filter = pane_id.or_else(|| {
                        pane_type
                            .as_ref()
                            .map(|p| shared::PaneConfig::pane_id_from_legacy(p))
                    });
                    // Catchup mode: client passed either a per-pane watermark map
                    // (preferred) or a single after_created_at high-water mark.
                    // Per-pane is the right shape — the legacy single-cutoff form
                    // dropped messages for slow panes when fast panes had advanced
                    // the watermark past their tails. Both flag `catchup: true`.
                    let is_catchup = after_created_at.is_some() || pane_watermarks.is_some();
                    let is_initial_load = !is_catchup
                        && before_id.is_none()
                        && effective_pane_filter.is_none();
                    let fetch_result = if let Some(watermarks) = pane_watermarks.as_ref() {
                        // Wire keys are strings (JSON object keys); storage
                        // keys by numeric pane id. Parse back to u32, dropping
                        // any non-numeric key defensively.
                        let numeric: std::collections::HashMap<u32, String> = watermarks
                            .iter()
                            .filter_map(|(k, v)| k.parse::<u32>().ok().map(|id| (id, v.clone())))
                            .collect();
                        state
                            .storage
                            .get_messages_per_pane_after(&sid, &numeric)
                            .await
                            .map(|msgs| (msgs, false))
                    } else if let Some(after) = after_created_at.as_deref() {
                        state
                            .storage
                            .get_messages_after(&sid, after)
                            .await
                            .map(|msgs| (msgs, false))
                    } else if is_initial_load {
                        state.storage.get_messages_per_pane(&sid, limit).await
                    } else {
                        state
                            .storage
                            .get_messages_paginated_by_pane_id(
                                &sid,
                                Some(limit),
                                before_id.as_deref(),
                                effective_pane_filter,
                            )
                            .await
                    };
                    match fetch_result
                    {
                        Ok((stored_messages, has_more)) => {
                            let messages: Vec<MessageInfo> =
                                stored_messages.into_iter().map(to_message_info).collect();

                            if is_initial_load {
                                let mut panes_to_send = state.sessions.get_session_panes(&sid);
                                if panes_to_send.is_empty() {
                                    if let Ok(stored_panes) = state.storage.load_pane_list(&sid).await {
                                        if !stored_panes.is_empty() {
                                            state
                                                .sessions
                                                .set_session_panes(&sid, stored_panes.clone());
                                            panes_to_send = stored_panes;
                                        }
                                    }
                                }
                                if panes_to_send.is_empty() {
                                    panes_to_send = infer_panes_from_messages(sid, &messages);
                                    if !panes_to_send.is_empty() {
                                        state
                                            .sessions
                                            .set_session_panes(&sid, panes_to_send.clone());
                                    }
                                }
                                if !panes_to_send.is_empty() {
                                    state
                                        .sessions
                                        .send_to_web(
                                            &connection_id,
                                            ServerToWeb::PaneList {
                                                session_id: sid,
                                                panes: panes_to_send,
                                            },
                                        )
                                        .await;
                                }
                            }

                            state
                                .sessions
                                .send_to_web(
                                    &connection_id,
                                    ServerToWeb::SessionMessages {
                                        session_id: sid,
                                        messages,
                                        has_more,
                                        catchup: is_catchup,
                                    },
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
                Ok(WebToServer::DownloadSession { session_id: sid }) => {
                    tracing::info!("Downloading session data for {}", sid);
                    let response = session_download_response(&state, sid).await;
                    state.sessions.send_to_web(&connection_id, response).await;
                }
                Err(e) => {
                    tracing::warn!("Failed to parse message: {}", e);
                }
            }
        }
    }

    // Cleanup: abort send_task first so the channel receiver is dropped,
    // preventing deadlock if a sender is awaiting on a full channel
    // while unregister_web tries to acquire a write lock on the same DashMap shard.
    send_task.abort();
    state.sessions.unregister_web(&connection_id);
    tracing::info!("Web client disconnected: {}", connection_id);
}
