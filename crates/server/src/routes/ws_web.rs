use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use base64::Engine as _;
use futures::{SinkExt, StreamExt};
use shared::{
    MessageInfo, ServerToCli, ServerToDaemon, ServerToWeb, SessionInfo, SessionStatus,
    TerminalLifecycle, WebToServer,
};
use std::collections::HashSet;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::mobile_metrics::MobileMetric;
use crate::routes::auth::verify_token;
use crate::state::AppState;

const SERVER_VERSION: &str = env!("APAS_SERVER_VERSION");

fn is_read_only_message(message: &WebToServer) -> bool {
    matches!(
        message,
        WebToServer::Authenticate { .. }
            | WebToServer::ListCliClients
            | WebToServer::ListMachines
            | WebToServer::AttachSession { .. }
            | WebToServer::ListSessions
            | WebToServer::GetSessionMessages { .. }
            | WebToServer::RequestPaneDiff { .. }
            | WebToServer::FetchTeamTodo { .. }
            | WebToServer::FetchSuggestedWorkers { .. }
            | WebToServer::TerminalAttach { .. }
            | WebToServer::ListPaneWorkSummaries { .. }
            | WebToServer::MobileTelemetry { .. }
            | WebToServer::Heartbeat
    )
}

fn protocol_mutations_allowed(
    client_kind: Option<shared::ClientKind>,
    protocol_version: Option<u32>,
) -> bool {
    !matches!(client_kind, Some(shared::ClientKind::Mobile))
        || protocol_version.is_some_and(|version| {
            (shared::MOBILE_PROTOCOL_MIN_VERSION..=shared::MOBILE_PROTOCOL_MAX_VERSION)
                .contains(&version)
        })
}

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
                // Panes reconstructed from stored chat history are agent
                // panes by definition — a terminal pane writes no messages
                // to reconstruct from.
                kind: shared::PaneKind::Agent,
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

pub(crate) async fn list_accessible_machines_for_user(
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
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
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
/// Whether this web connection may change project-level settings.
///
/// Owner (`sessions.user_id`) or admin only. These are project *policy*, not
/// per-seat preference: `team_enabled` decides whether autonomous panes may run
/// for everyone on the project, and `auto_merge_prs` alone lets the Tech Lead
/// `gh pr merge` with no human click. A plain `user` who has been shared into
/// the session must not be able to set either.
///
/// This is the first role gate in the WebSocket layer — every other handler
/// authorizes on *access* (`check_session_access`) and stops there. Reuses
/// `share::can_manage_access` rather than re-deriving the boundary, so the WS
/// and HTTP paths cannot drift apart on who counts as privileged.
///
/// Fails closed: an unknown user, a session the user has no role on, or a
/// failed lookup all deny.
async fn can_manage_project_settings(
    state: &AppState,
    connection_id: &Uuid,
    session_id: &Uuid,
) -> bool {
    let Some(user_id) = state.sessions.get_web_user(connection_id) else {
        return false;
    };
    match state
        .db
        .get_session_role_for_user(&session_id.to_string(), &user_id.to_string())
        .await
    {
        Ok(Some(role)) => super::share::parse_share_role(&role).can_manage_access(),
        Ok(None) => false,
        Err(err) => {
            tracing::warn!(
                %err,
                %session_id,
                "role lookup failed; denying project settings change"
            );
            false
        }
    }
}

async fn send_policy_error(state: &AppState, connection_id: &Uuid, message: impl Into<String>) {
    state
        .sessions
        .send_to_web(
            connection_id,
            ServerToWeb::Error {
                message: message.into(),
            },
        )
        .await;
}

async fn send_mutation_ack(
    state: &AppState,
    connection_id: &Uuid,
    request_id: Option<&str>,
    session_id: Uuid,
    pane_id: Option<u32>,
    mutation: shared::MutationKind,
    result: Result<(), impl Into<String>>,
) {
    let Some(request_id) = request_id else {
        if let Err(error) = result {
            state
                .sessions
                .send_to_web(
                    connection_id,
                    ServerToWeb::Error {
                        message: error.into(),
                    },
                )
                .await;
        }
        return;
    };
    let (accepted, error) = match result {
        Ok(()) => (true, None),
        Err(error) => (false, Some(error.into())),
    };
    state.mobile_metrics.increment(if accepted {
        MobileMetric::MutationAccepted
    } else {
        MobileMetric::MutationRejected
    });
    tracing::info!(
        mutation = ?mutation,
        accepted,
        has_pane = pane_id.is_some(),
        "mobile-capable mutation acknowledged"
    );
    let acknowledgement = ServerToWeb::MutationAck {
        request_id: request_id.to_string(),
        session_id,
        pane_id,
        mutation,
        accepted,
        error,
    };
    if let Some(user_id) = state.sessions.get_web_user(connection_id) {
        state.sessions.complete_mutation_request(
            user_id,
            request_id.to_string(),
            acknowledgement.clone(),
        );
    }
    state
        .sessions
        .send_to_web(connection_id, acknowledgement)
        .await;
}

/// Returns true when the request is already in flight or its retained result
/// was replayed, so the caller must not route the mutation again.
async fn replay_or_claim_mutation_request(
    state: &AppState,
    connection_id: &Uuid,
    request_id: Option<&str>,
) -> bool {
    let Some(request_id) = request_id else {
        return false;
    };
    let Some(user_id) = state.sessions.get_web_user(connection_id) else {
        return true;
    };
    match state.sessions.claim_mutation_request(user_id, request_id) {
        crate::session::MutationRequestClaim::New => false,
        crate::session::MutationRequestClaim::InFlight => true,
        crate::session::MutationRequestClaim::Replay(acknowledgement) => {
            state
                .sessions
                .send_to_web(connection_id, acknowledgement)
                .await;
            true
        }
    }
}

/// Load the latest effective policy for a launch-like mutation. Both ends of
/// the data-plane must understand server-owned policy before a launch is
/// routed; otherwise an older peer could silently apply local `.apas` state.
async fn effective_policy_for_launch(
    state: &AppState,
    connection_id: &Uuid,
    session_id: &Uuid,
) -> Option<shared::EffectiveProjectPolicy> {
    if !state
        .sessions
        .web_supports_capability(connection_id, shared::PROJECT_POLICY_CAPABILITY)
    {
        send_policy_error(
            state,
            connection_id,
            "This web client is incompatible with cluster project policy; reload after updating the web deployment",
        )
        .await;
        return None;
    }
    if !state
        .sessions
        .session_supports_capability(session_id, shared::PROJECT_POLICY_CAPABILITY)
    {
        send_policy_error(
            state,
            connection_id,
            "The project host CLI is incompatible with cluster project policy; update and reconnect it before launching panes",
        )
        .await;
        return None;
    }
    let project = match state
        .db
        .get_project_for_session(&session_id.to_string())
        .await
    {
        Ok(Some(project)) => project,
        Ok(None) => {
            send_policy_error(state, connection_id, "Project policy is unavailable").await;
            return None;
        }
        Err(err) => {
            tracing::warn!(%err, %session_id, "project policy lookup failed");
            send_policy_error(state, connection_id, "Could not verify project policy").await;
            return None;
        }
    };
    match state.db.get_effective_project_policy(&project.id).await {
        Ok(policy) if !policy.project_suspended => Some(policy),
        Ok(_) => {
            send_policy_error(
                state,
                connection_id,
                "This project is suspended; reactivate it before launching panes",
            )
            .await;
            None
        }
        Err(err) => {
            tracing::warn!(%err, project_id = %project.id, "effective project policy lookup failed");
            send_policy_error(state, connection_id, "Could not verify project policy").await;
            None
        }
    }
}

/// Reboot is also the upgrade escape hatch for CLIs that predate server-owned
/// project policy. Keep the normal policy gate, but explain this bootstrap
/// case directly instead of returning the generic "cannot launch panes"
/// error, which left users with no actionable way forward.
async fn effective_policy_for_cli_reboot(
    state: &AppState,
    connection_id: &Uuid,
    session_id: &Uuid,
) -> Option<shared::EffectiveProjectPolicy> {
    if state
        .sessions
        .web_supports_capability(connection_id, shared::PROJECT_POLICY_CAPABILITY)
        && !state
            .sessions
            .session_supports_capability(session_id, shared::PROJECT_POLICY_CAPABILITY)
    {
        send_policy_error(
            state,
            connection_id,
            "This project CLI is too old to reboot from the web. Reboot the CLI manually on the project host to upgrade it, then try again.",
        )
        .await;
        return None;
    }
    effective_policy_for_launch(state, connection_id, session_id).await
}

async fn authorize_profile_launch(
    state: &AppState,
    connection_id: &Uuid,
    session_id: &Uuid,
    kind: shared::PaneKind,
    provider: shared::Provider,
    model: Option<&str>,
    managed: bool,
) -> bool {
    if shared::is_retired_launch(provider, model) {
        send_policy_error(
            state,
            connection_id,
            "Unsupported provider: this backend has been retired",
        )
        .await;
        return false;
    }
    let Some(policy) = effective_policy_for_launch(state, connection_id, session_id).await else {
        return false;
    };
    if managed && !policy.team_available {
        send_policy_error(
            state,
            connection_id,
            format!(
                "Managed pane launch is disabled by cluster policy (policy version {})",
                policy.version
            ),
        )
        .await;
        return false;
    }
    if policy.allows(kind, provider, model) {
        return true;
    }
    send_policy_error(
        state,
        connection_id,
        format!(
            "Launch profile '{}' is disabled by cluster policy (policy version {})",
            shared::launch_profile_key(kind, provider, model),
            policy.version
        ),
    )
    .await;
    false
}

/// Authorize creation of a brand-new pane. Structured agent panes remain
/// available to managed team roles and existing legacy panes can still be
/// resumed/rebooted through `authorize_profile_launch`, but ordinary new work
/// must use the terminal path.
async fn authorize_new_pane_launch(
    state: &AppState,
    connection_id: &Uuid,
    session_id: &Uuid,
    kind: shared::PaneKind,
    provider: shared::Provider,
    model: Option<&str>,
    managed: bool,
) -> bool {
    if !managed && kind == shared::PaneKind::Agent {
        send_policy_error(
            state,
            connection_id,
            "Conversation-only panes are retired. Create a Claude, Codex, or OpenCode terminal pane instead.",
        )
        .await;
        return false;
    }
    if kind == shared::PaneKind::Terminal
        && provider == shared::Provider::Opencode
        && !state
            .sessions
            .session_supports_capability(session_id, shared::OPENCODE_TERMINAL_CAPABILITY)
    {
        send_policy_error(
            state,
            connection_id,
            "The project CLI must be updated and reconnected before creating an OpenCode terminal pane.",
        )
        .await;
        return false;
    }
    authorize_profile_launch(
        state,
        connection_id,
        session_id,
        kind,
        provider,
        model,
        managed,
    )
    .await
}

async fn authorize_existing_pane_launch(
    state: &AppState,
    connection_id: &Uuid,
    session_id: &Uuid,
    pane_id: u32,
) -> bool {
    let pane = state
        .sessions
        .get_session_panes(session_id)
        .into_iter()
        .find(|pane| pane.pane_id == pane_id);
    let Some(pane) = pane else {
        send_policy_error(state, connection_id, "Pane not found").await;
        return false;
    };
    authorize_profile_launch(
        state,
        connection_id,
        session_id,
        pane.kind,
        pane.provider,
        pane.model.as_deref(),
        pane.managed,
    )
    .await
}

async fn authorize_team_launch(
    state: &AppState,
    connection_id: &Uuid,
    session_id: &Uuid,
    roles: &[&shared::TeamRoleSpec],
) -> bool {
    for role in roles {
        let provider = role.provider.unwrap_or(shared::Provider::Claude);
        if shared::is_retired_launch(provider, role.model.as_deref()) {
            send_policy_error(
                state,
                connection_id,
                "Unsupported provider in managed team: this backend has been retired",
            )
            .await;
            return false;
        }
    }
    let Some(policy) = effective_policy_for_launch(state, connection_id, session_id).await else {
        return false;
    };
    if !policy.team_available {
        send_policy_error(
            state,
            connection_id,
            format!(
                "Team launch is disabled by cluster policy (policy version {})",
                policy.version
            ),
        )
        .await;
        return false;
    }
    for role in roles {
        let provider = role.provider.unwrap_or(shared::Provider::Claude);
        if !policy.allows(shared::PaneKind::Agent, provider, role.model.as_deref()) {
            send_policy_error(
                state,
                connection_id,
                format!(
                    "Team launch profile '{}' is disabled by cluster policy (policy version {})",
                    shared::launch_profile_key(
                        shared::PaneKind::Agent,
                        provider,
                        role.model.as_deref(),
                    ),
                    policy.version
                ),
            )
            .await;
            return false;
        }
    }
    true
}

#[cfg(test)]
mod retired_launch_authorization_tests {
    use super::*;
    use crate::config::Config;
    use crate::db::{Database, Session, User};

    async fn policy_state() -> (
        AppState,
        Uuid,
        Uuid,
        mpsc::Receiver<ServerToWeb>,
        mpsc::Receiver<ServerToCli>,
        Uuid,
    ) {
        let dir = std::env::temp_dir().join(format!("apas-retired-auth-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp db dir");
        let db_path = dir.join("apas.db").to_string_lossy().to_string();
        let db = Database::new(&db_path).await.expect("create temp db");
        db.run_migrations().await.expect("run migrations");
        let user_id = Uuid::new_v4();
        db.create_user(&User {
            id: user_id.to_string(),
            email: format!("{user_id}@example.test"),
            password_hash: "hash".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .expect("create user");
        let session_id = Uuid::new_v4();
        db.create_session(&Session {
            id: session_id.to_string(),
            user_id: user_id.to_string(),
            cli_client_id: None,
            working_dir: Some("/project".to_string()),
            hostname: Some("host".to_string()),
            status: "connected".to_string(),
            created_at: None,
            updated_at: None,
            is_paused: false,
            project_id: Some(session_id.to_string()),
            git_remote: None,
            git_remote_url: None,
        })
        .await
        .expect("create session");

        let mut config = Config::default();
        config.database.path = db_path;
        let state = AppState::new(db, config);
        let connection_id = Uuid::new_v4();
        let (web_tx, web_rx) = mpsc::channel(8);
        state.sessions.register_web(connection_id, web_tx);
        state.sessions.set_web_capabilities(
            connection_id,
            vec![shared::PROJECT_POLICY_CAPABILITY.to_string()],
        );
        state
            .sessions
            .create_session(session_id, user_id, connection_id);
        let cli_id = Uuid::new_v4();
        let (cli_tx, cli_rx) = mpsc::channel(8);
        state.sessions.register_cli(cli_id, user_id, cli_tx, None);
        assert!(state.sessions.assign_cli_to_session(&session_id, cli_id));
        state
            .sessions
            .set_cli_capabilities(cli_id, vec![shared::PROJECT_POLICY_CAPABILITY.to_string()]);
        (state, connection_id, session_id, web_rx, cli_rx, cli_id)
    }

    #[tokio::test]
    async fn retired_profile_is_rejected_before_routing() {
        let (state, connection_id, session_id, mut web_rx, mut cli_rx, _cli_id) =
            policy_state().await;

        assert!(
            !authorize_profile_launch(
                &state,
                &connection_id,
                &session_id,
                shared::PaneKind::Agent,
                shared::Provider::Claude,
                Some("MiniMax-M2.7"),
                false,
            )
            .await
        );

        let ServerToWeb::Error { message } = web_rx.try_recv().expect("explicit web error") else {
            panic!("expected policy error")
        };
        assert!(message.contains("Unsupported provider"));
        assert!(
            cli_rx.try_recv().is_err(),
            "retired request must not reach CLI"
        );
    }

    #[tokio::test]
    async fn retired_team_role_is_classified_before_policy_lookup() {
        let (state, connection_id, _session_id, mut web_rx, mut cli_rx, _cli_id) =
            policy_state().await;
        let role = shared::TeamRoleSpec {
            provider: Some(shared::Provider::Claude),
            model: Some("glm-5.1".to_string()),
        };

        assert!(!authorize_team_launch(&state, &connection_id, &Uuid::new_v4(), &[&role],).await);

        let ServerToWeb::Error { message } = web_rx.try_recv().expect("explicit web error") else {
            panic!("expected policy error")
        };
        assert!(message.contains("Unsupported provider"));
        assert!(!message.contains("incompatible"));
        assert!(
            cli_rx.try_recv().is_err(),
            "retired team must not reach CLI"
        );
    }

    #[tokio::test]
    async fn retained_profile_remains_authorized() {
        let (state, connection_id, session_id, mut web_rx, _cli_rx, _cli_id) = policy_state().await;

        assert!(
            authorize_profile_launch(
                &state,
                &connection_id,
                &session_id,
                shared::PaneKind::Agent,
                shared::Provider::Codex,
                None,
                false,
            )
            .await
        );
        assert!(web_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn new_unmanaged_conversation_only_pane_is_rejected() {
        let (state, connection_id, session_id, mut web_rx, mut cli_rx, _cli_id) =
            policy_state().await;

        assert!(
            !authorize_new_pane_launch(
                &state,
                &connection_id,
                &session_id,
                shared::PaneKind::Agent,
                shared::Provider::Claude,
                None,
                false,
            )
            .await
        );

        let ServerToWeb::Error { message } = web_rx.try_recv().expect("explicit web error") else {
            panic!("expected retirement error")
        };
        assert!(message.contains("Conversation-only panes are retired"));
        assert!(cli_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn new_terminal_pane_remains_authorized() {
        let (state, connection_id, session_id, mut web_rx, _cli_rx, _cli_id) = policy_state().await;

        assert!(
            authorize_new_pane_launch(
                &state,
                &connection_id,
                &session_id,
                shared::PaneKind::Terminal,
                shared::Provider::Codex,
                None,
                false,
            )
            .await
        );
        assert!(web_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn opencode_terminal_requires_a_capable_cli() {
        let (state, connection_id, session_id, mut web_rx, mut cli_rx, _cli_id) =
            policy_state().await;

        assert!(
            !authorize_new_pane_launch(
                &state,
                &connection_id,
                &session_id,
                shared::PaneKind::Terminal,
                shared::Provider::Opencode,
                None,
                false,
            )
            .await
        );

        let ServerToWeb::Error { message } = web_rx.try_recv().expect("explicit web error") else {
            panic!("expected compatibility error")
        };
        assert!(message.contains("updated and reconnected"));
        assert!(cli_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn opencode_terminal_is_authorized_for_a_capable_cli() {
        let (state, connection_id, session_id, mut web_rx, _cli_rx, cli_id) = policy_state().await;
        state.sessions.set_cli_capabilities(
            cli_id,
            vec![
                shared::PROJECT_POLICY_CAPABILITY.to_string(),
                shared::OPENCODE_TERMINAL_CAPABILITY.to_string(),
            ],
        );

        assert!(
            authorize_new_pane_launch(
                &state,
                &connection_id,
                &session_id,
                shared::PaneKind::Terminal,
                shared::Provider::Opencode,
                None,
                false,
            )
            .await
        );
        assert!(web_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn legacy_cli_reboot_returns_an_actionable_manual_reboot_error() {
        let (state, connection_id, session_id, mut web_rx, mut cli_rx, cli_id) =
            policy_state().await;
        state.sessions.set_cli_capabilities(cli_id, Vec::new());

        assert!(
            effective_policy_for_cli_reboot(&state, &connection_id, &session_id)
                .await
                .is_none()
        );

        let ServerToWeb::Error { message } = web_rx.try_recv().expect("explicit web error") else {
            panic!("expected manual reboot error")
        };
        assert!(message.contains("too old to reboot from the web"));
        assert!(message.contains("Reboot the CLI manually"));
        assert!(
            cli_rx.try_recv().is_err(),
            "an incompatible CLI must not receive the reboot command"
        );
    }
}

async fn resolve_target_session(
    state: &AppState,
    connection_id: &Uuid,
    msg_session_id: Option<Uuid>,
    fallback: Option<Uuid>,
) -> Option<Uuid> {
    if let Some(sid) = msg_session_id {
        if state
            .sessions
            .is_web_attached_to_session(&sid, connection_id)
        {
            let still_authorized = match state.sessions.get_web_user(connection_id) {
                Some(user_id) => state
                    .db
                    .check_session_access(&sid.to_string(), &user_id.to_string())
                    .await
                    .unwrap_or(false),
                None => false,
            };
            if !still_authorized {
                state
                    .sessions
                    .send_to_web(
                        connection_id,
                        ServerToWeb::Error {
                            message: "Access denied".to_string(),
                        },
                    )
                    .await;
                return None;
            }
            if state
                .active_session_operation(&sid.to_string())
                .await
                .is_ok()
            {
                return Some(sid);
            }
            state
                .sessions
                .send_to_web(
                    connection_id,
                    ServerToWeb::Error {
                        message: "Project is unavailable".to_string(),
                    },
                )
                .await;
            return None;
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
                if state
                    .active_session_operation(&sid.to_string())
                    .await
                    .is_err()
                {
                    return None;
                }
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
        let still_authorized = match state.sessions.get_web_user(connection_id) {
            Some(user_id) => state
                .db
                .check_session_access(&sid.to_string(), &user_id.to_string())
                .await
                .unwrap_or(false),
            None => false,
        };
        if !still_authorized {
            state
                .sessions
                .send_to_web(
                    connection_id,
                    ServerToWeb::Error {
                        message: "Access denied".to_string(),
                    },
                )
                .await;
            return None;
        }
        if state
            .active_session_operation(&sid.to_string())
            .await
            .is_ok()
        {
            return Some(sid);
        }
        state
            .sessions
            .send_to_web(
                connection_id,
                ServerToWeb::Error {
                    message: "Project is unavailable".to_string(),
                },
            )
            .await;
        return None;
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

fn lifecycle_cli_message(
    supported: bool,
    session_id: Uuid,
    request_id: Uuid,
    operation: shared::CliLifecycleOperation,
) -> Option<ServerToCli> {
    supported.then_some(ServerToCli::CliLifecycleRequest {
        session_id,
        request_id,
        operation,
    })
}

async fn broadcast_lifecycle_status(state: &AppState, session_id: Uuid, message: ServerToWeb) {
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
    let Ok((_project_id, _project_guard)) = state.active_session_operation(&sid.to_string()).await
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
        if let Err(error) = state
            .db
            .record_session_user_input(&sid.to_string(), &created_at)
            .await
        {
            tracing::warn!(%error, session_id = %sid, "failed to record session user activity");
        }
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

const TERMINAL_CONVERSATION_SUBMIT_DELAY: Duration = Duration::from_millis(100);

fn terminal_conversation_frame(text: &str) -> String {
    if text.contains('\n') {
        format!("\u{1b}[200~{text}\u{1b}[201~")
    } else {
        text.to_string()
    }
}

async fn handle_terminal_conversation_input(
    state: &AppState,
    connection_id: &Uuid,
    fallback_session_id: Option<Uuid>,
    msg_sid: Uuid,
    pane_id: u32,
    text: String,
    client_msg_id: Option<String>,
) {
    let Some(sid) =
        resolve_target_session(state, connection_id, Some(msg_sid), fallback_session_id).await
    else {
        return;
    };
    let Ok((_project_id, _project_guard)) = state.active_session_operation(&sid.to_string()).await
    else {
        return;
    };

    let text = text.trim().to_string();
    if text.is_empty() {
        state
            .sessions
            .send_to_web(
                connection_id,
                ServerToWeb::Error {
                    message: "Conversation messages cannot be empty".to_string(),
                },
            )
            .await;
        return;
    }

    // A mobile client retries an unacknowledged mutation. Re-ack the
    // original persisted message without typing it into the TUI again.
    if let Some(ref cmid) = client_msg_id {
        if let Some(orig_created_at) = state.sessions.seen_input_id(&sid, cmid) {
            state
                .sessions
                .send_to_web(
                    connection_id,
                    ServerToWeb::UserInput {
                        session_id: sid,
                        text,
                        pane_type: None,
                        pane_id: Some(pane_id),
                        created_at: Some(orig_created_at),
                        client_msg_id: client_msg_id.clone(),
                    },
                )
                .await;
            return;
        }
    }

    let input_frame = terminal_conversation_frame(&text);
    let input_b64 = base64::engine::general_purpose::STANDARD.encode(input_frame.as_bytes());
    let input_sent = state
        .sessions
        .route_to_cli(
            &sid,
            ServerToCli::TerminalInput {
                session_id: sid,
                pane_id,
                data_b64: input_b64,
            },
        )
        .await;
    if !input_sent {
        state
            .sessions
            .send_to_web(
                connection_id,
                ServerToWeb::Error {
                    message: "CLI client not connected".to_string(),
                },
            )
            .await;
        return;
    }

    // The provider transcript will later contain this same user turn. Arm a
    // one-shot correlation before sending Enter so the fast transcript path
    // cannot race ahead and persist/broadcast the message a second time.
    state
        .sessions
        .expect_terminal_transcript_echo(sid, pane_id, text.clone());

    // Full-screen TUIs can classify back-to-back bytes as a paste burst.
    // Deliver Enter separately after the text has landed.
    tokio::time::sleep(TERMINAL_CONVERSATION_SUBMIT_DELAY).await;
    let submit_sent = state
        .sessions
        .route_to_cli(
            &sid,
            ServerToCli::TerminalInput {
                session_id: sid,
                pane_id,
                data_b64: base64::engine::general_purpose::STANDARD.encode(b"\r"),
            },
        )
        .await;
    if !submit_sent {
        state
            .sessions
            .cancel_terminal_transcript_echo(&sid, pane_id, &text);
        state
            .sessions
            .send_to_web(
                connection_id,
                ServerToWeb::Error {
                    message: "The text reached the terminal, but Enter could not be sent"
                        .to_string(),
                },
            )
            .await;
        return;
    }

    let created_at = chrono::Utc::now().to_rfc3339();
    if let Err(error) = state
        .db
        .record_session_user_input(&sid.to_string(), &created_at)
        .await
    {
        tracing::warn!(%error, session_id = %sid, "failed to record terminal conversation activity");
    }
    let stored_message = crate::storage::StoredMessage {
        id: Uuid::new_v4().to_string(),
        role: "user".to_string(),
        content: text.clone(),
        message_type: "text".to_string(),
        created_at: created_at.clone(),
        pane_type: Some(pane_id.to_string()),
    };
    if let Err(error) = state.storage.append_message(&sid, &stored_message).await {
        // Let the transcript observer provide the durable copy when this
        // first persistence attempt fails.
        state
            .sessions
            .cancel_terminal_transcript_echo(&sid, pane_id, &text);
        tracing::error!("Failed to save terminal conversation input: {error}");
    }
    if let Some(cmid) = client_msg_id.clone() {
        state
            .sessions
            .record_input_id(sid, cmid, created_at.clone());
    }
    state
        .sessions
        .route_to_web(
            &sid,
            ServerToWeb::UserInput {
                session_id: sid,
                text,
                pane_type: None,
                pane_id: Some(pane_id),
                created_at: Some(created_at),
                client_msg_id,
            },
        )
        .await;
    // Terminal panes are opaque PTYs, so unlike structured agent panes they
    // cannot announce inference start themselves. Mark the accepted turn as
    // working immediately; the CLI transcript observer clears this when the
    // assistant response is recorded (and terminal exit also clears it).
    crate::routes::ws_cli::set_and_broadcast_pane_status(
        state,
        sid,
        shared::PaneType::Interactive,
        pane_id,
        Some("Working...".to_string()),
    )
    .await;
    crate::routes::ws_cli::record_and_broadcast_usage(
        state,
        sid,
        Some(pane_id),
        crate::db::UsageDelta {
            prompt_count: 1,
            ..Default::default()
        },
    )
    .await;
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
        db.run_migrations().await.expect("run migrations");
        let mut config = Config::default();
        config.database.path = db_path;
        AppState::new(db, config)
    }

    async fn persist_session(state: &AppState, user_id: Uuid, session_id: Uuid) {
        state
            .db
            .create_user(&crate::db::User {
                id: user_id.to_string(),
                email: format!("{user_id}@test"),
                password_hash: "hash".to_string(),
                created_at: None,
                cluster_role: "user".to_string(),
                account_status: "active".to_string(),
            })
            .await
            .unwrap();
        state
            .db
            .authorize_project_registration(&session_id.to_string(), &user_id.to_string())
            .await
            .unwrap();
        state
            .db
            .create_session(&crate::db::Session {
                id: session_id.to_string(),
                user_id: user_id.to_string(),
                cli_client_id: None,
                working_dir: None,
                hostname: None,
                status: "active".to_string(),
                created_at: None,
                updated_at: None,
                is_paused: false,
                project_id: Some(session_id.to_string()),
                git_remote: None,
                git_remote_url: None,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn reboot_pane_falls_back_to_active_session_and_routes_requested_pane_to_cli() {
        let state = test_state().await;
        let user_id = Uuid::new_v4();
        let web_connection_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let cli_id = Uuid::new_v4();
        persist_session(&state, user_id, session_id).await;

        let (web_tx, _web_rx) = mpsc::channel(4);
        state.sessions.register_web(web_connection_id, web_tx);
        state.sessions.set_web_user(web_connection_id, user_id);
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
        persist_session(&state, user_id, session_id).await;

        let (web_tx, _web_rx) = mpsc::channel(4);
        state.sessions.register_web(web_connection_id, web_tx);
        state.sessions.set_web_user(web_connection_id, user_id);
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

    #[test]
    fn an_old_cli_receives_no_lifecycle_request_at_all() {
        // The routing must not invent a substitute for a CLI that cannot handle
        // correlated lifecycle requests — the caller reports an upgrade instead.
        let session_id = Uuid::new_v4();
        let request_id = Uuid::new_v4();
        assert!(lifecycle_cli_message(
            false,
            session_id,
            request_id,
            shared::CliLifecycleOperation::RebootCli,
        )
        .is_none());

        let message = lifecycle_cli_message(
            true,
            session_id,
            request_id,
            shared::CliLifecycleOperation::RebootCli,
        )
        .expect("new CLI receives correlated request");
        assert!(matches!(
            message,
            ServerToCli::CliLifecycleRequest {
                operation: shared::CliLifecycleOperation::RebootCli,
                ..
            }
        ));
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
        db.run_migrations().await.expect("run migrations");
        let mut config = Config::default();
        config.database.path = db_path;
        AppState::new(db, config)
    }

    async fn persist_session(state: &AppState, user_id: Uuid, session_id: Uuid) {
        state
            .db
            .create_user(&crate::db::User {
                id: user_id.to_string(),
                email: format!("{user_id}@test"),
                password_hash: "hash".to_string(),
                created_at: None,
                cluster_role: "user".to_string(),
                account_status: "active".to_string(),
            })
            .await
            .unwrap();
        state
            .db
            .authorize_project_registration(&session_id.to_string(), &user_id.to_string())
            .await
            .unwrap();
        state
            .db
            .create_session(&crate::db::Session {
                id: session_id.to_string(),
                user_id: user_id.to_string(),
                cli_client_id: None,
                working_dir: None,
                hostname: None,
                status: "active".to_string(),
                created_at: None,
                updated_at: None,
                is_paused: false,
                project_id: Some(session_id.to_string()),
                git_remote: None,
                git_remote_url: None,
            })
            .await
            .unwrap();
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

    fn assert_terminal_input(msg: ServerToCli, session_id: Uuid, pane_id: u32, expected: &str) {
        match msg {
            ServerToCli::TerminalInput {
                session_id: got_session_id,
                pane_id: got_pane_id,
                data_b64,
            } => {
                assert_eq!(got_session_id, session_id);
                assert_eq!(got_pane_id, pane_id);
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data_b64)
                    .expect("valid terminal input base64");
                assert_eq!(String::from_utf8(bytes).unwrap(), expected);
            }
            other => panic!("expected TerminalInput message, got {other:?}"),
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

    fn next_user_input(rx: &mut mpsc::Receiver<ServerToWeb>) -> ServerToWeb {
        for _ in 0..8 {
            let message = rx.try_recv().expect("web response");
            if matches!(message, ServerToWeb::UserInput { .. }) {
                return message;
            }
        }
        panic!("no UserInput response found");
    }

    fn next_pane_status(rx: &mut mpsc::Receiver<ServerToWeb>) -> ServerToWeb {
        for _ in 0..8 {
            let message = rx.try_recv().expect("web response");
            if matches!(message, ServerToWeb::PaneStatus { .. }) {
                return message;
            }
        }
        panic!("no PaneStatus response found");
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
        persist_session(&state, user_id, session_id).await;

        let (web_tx, web_rx) = mpsc::channel(8);
        state.sessions.register_web(web_connection_id, web_tx);
        state.sessions.set_web_user(web_connection_id, user_id);
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
            next_user_input(&mut web_rx),
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
        assert_eq!(
            state
                .db
                .get_session_last_user_input_at(&session_id.to_string())
                .await
                .unwrap()
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
            next_user_input(&mut web_rx),
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
        assert_user_input_echo(next_user_input(&mut web_rx), session_id, "legacy one", None);
        assert_user_input_echo(next_user_input(&mut web_rx), session_id, "legacy two", None);

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

    #[tokio::test]
    async fn terminal_conversation_input_types_submits_persists_echoes_and_deduplicates() {
        let (state, web_connection_id, session_id, mut cli_rx, mut web_rx) =
            setup_connected_session().await;

        handle_terminal_conversation_input(
            &state,
            &web_connection_id,
            Some(session_id),
            session_id,
            9,
            "  line one\nline two  ".to_string(),
            Some("terminal-client-1".to_string()),
        )
        .await;

        assert_terminal_input(
            cli_rx.try_recv().expect("conversation text routed"),
            session_id,
            9,
            "\u{1b}[200~line one\nline two\u{1b}[201~",
        );
        assert_terminal_input(
            cli_rx.try_recv().expect("conversation submit routed"),
            session_id,
            9,
            "\r",
        );
        let created_at = assert_user_input_echo(
            next_user_input(&mut web_rx),
            session_id,
            "line one\nline two",
            Some("terminal-client-1"),
        );
        assert!(matches!(
            next_pane_status(&mut web_rx),
            ServerToWeb::PaneStatus {
                session_id: got_session_id,
                pane_id: Some(9),
                status: Some(ref status),
                ..
            } if got_session_id == session_id && status == "Working..."
        ));
        assert_eq!(
            state.sessions.get_pane_statuses(&session_id),
            vec![(shared::PaneType::Interactive, 9, "Working...".to_string())],
        );
        assert_eq!(
            state
                .sessions
                .seen_input_id(&session_id, "terminal-client-1")
                .as_deref(),
            Some(created_at.as_str())
        );

        let stored = state
            .storage
            .get_messages(&session_id)
            .await
            .expect("stored terminal conversation message");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].role, "user");
        assert_eq!(stored[0].content, "line one\nline two");
        assert_eq!(stored[0].pane_type.as_deref(), Some("9"));

        // The terminal transcript watcher observes the same user turn a few
        // seconds later. It must consume the correlation instead of storing
        // and broadcasting a second copy.
        while web_rx.try_recv().is_ok() {}
        crate::routes::ws_cli::handle_cli_user_input(
            &state,
            session_id,
            "line one\nline two".to_string(),
            None,
            Some(9),
        )
        .await;
        assert!(matches!(
            web_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        assert_eq!(
            state.storage.get_messages(&session_id).await.unwrap().len(),
            1,
            "provider transcript echo must not persist a duplicate"
        );

        handle_terminal_conversation_input(
            &state,
            &web_connection_id,
            Some(session_id),
            session_id,
            9,
            "line one\nline two".to_string(),
            Some("terminal-client-1".to_string()),
        )
        .await;
        assert!(matches!(
            cli_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        assert_user_input_echo(
            next_user_input(&mut web_rx),
            session_id,
            "line one\nline two",
            Some("terminal-client-1"),
        );
        assert_eq!(
            state.storage.get_messages(&session_id).await.unwrap().len(),
            1
        );

        // Once the expected transcript echo is consumed, an identical turn
        // typed directly into the raw terminal is genuine and must survive.
        while web_rx.try_recv().is_ok() {}
        crate::routes::ws_cli::handle_cli_user_input(
            &state,
            session_id,
            "line one\nline two".to_string(),
            None,
            Some(9),
        )
        .await;
        assert_user_input_echo(
            next_user_input(&mut web_rx),
            session_id,
            "line one\nline two",
            None,
        );
        assert_eq!(
            state.storage.get_messages(&session_id).await.unwrap().len(),
            2
        );
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
    let mut mutations_allowed = true;
    let mut mobile_device_session_id: Option<String> = None;
    let mut is_mobile_client = false;

    tracing::info!("Web client connected: {}", connection_id);

    // Handle incoming messages
    while let Some(Ok(msg)) = receiver.next().await {
        if let Message::Text(text) = msg {
            if user_id.is_some() && !state.sessions.is_web_connected(&connection_id) {
                break;
            }
            let parsed: Result<WebToServer, _> = serde_json::from_str(&text);
            if let (Some(uid), Some(device_session_id)) =
                (user_id, mobile_device_session_id.as_deref())
            {
                let still_active = state
                    .db
                    .is_mobile_device_session_active(device_session_id, &uid.to_string())
                    .await
                    .unwrap_or(false);
                if !still_active {
                    state
                        .sessions
                        .send_to_web(
                            &connection_id,
                            ServerToWeb::AuthenticationFailed {
                                reason: "Mobile device session is expired or revoked".to_string(),
                            },
                        )
                        .await;
                    break;
                }
            }
            if user_id.is_some()
                && !mutations_allowed
                && parsed
                    .as_ref()
                    .is_ok_and(|message| !is_read_only_message(message))
            {
                state
                    .sessions
                    .send_to_web(
                        &connection_id,
                        ServerToWeb::Error {
                            message: "This mobile build is read-only because its protocol version is incompatible. Update the app to make changes.".to_string(),
                        },
                    )
                    .await;
                continue;
            }
            match parsed {
                Ok(WebToServer::Authenticate {
                    token,
                    capabilities,
                    client_kind,
                    app_version,
                    protocol_version,
                }) => {
                    // Validate JWT token
                    match verify_token(&token, &state.config.auth.jwt_secret) {
                        Ok(claims) => {
                            match Uuid::parse_str(&claims.sub) {
                                Ok(uid) => {
                                    if let Some(device_session_id) =
                                        claims.device_session_id.as_deref()
                                    {
                                        let active = claims.token_kind.as_deref()
                                            == Some("mobile_access")
                                            && state
                                                .db
                                                .is_mobile_device_session_active(
                                                    device_session_id,
                                                    &uid.to_string(),
                                                )
                                                .await
                                                .unwrap_or(false);
                                        if !active {
                                            state
                                                .sessions
                                                .send_to_web(
                                                    &connection_id,
                                                    ServerToWeb::AuthenticationFailed {
                                                        reason: "Mobile device session is expired or revoked".to_string(),
                                                    },
                                                )
                                                .await;
                                            continue;
                                        }
                                    }
                                    let cluster_user =
                                        match state.db.get_user_by_id(&uid.to_string()).await {
                                            Ok(Some(user)) if user.is_active() => user,
                                            Ok(Some(_)) => {
                                                state
                                                    .sessions
                                                    .send_to_web(
                                                        &connection_id,
                                                        ServerToWeb::AuthenticationFailed {
                                                            reason: "Cluster account is suspended"
                                                                .to_string(),
                                                        },
                                                    )
                                                    .await;
                                                continue;
                                            }
                                            Ok(None) => {
                                                state
                                                    .sessions
                                                    .send_to_web(
                                                        &connection_id,
                                                        ServerToWeb::AuthenticationFailed {
                                                            reason: "Cluster account not found"
                                                                .to_string(),
                                                        },
                                                    )
                                                    .await;
                                                continue;
                                            }
                                            Err(err) => {
                                                tracing::warn!(
                                                    "Failed to fetch cluster account {}: {}",
                                                    uid,
                                                    err
                                                );
                                                state
                                                    .sessions
                                                    .send_to_web(
                                                        &connection_id,
                                                        ServerToWeb::AuthenticationFailed {
                                                            reason:
                                                                "Could not load cluster account"
                                                                    .to_string(),
                                                        },
                                                    )
                                                    .await;
                                                continue;
                                            }
                                        };
                                    mobile_device_session_id = claims.device_session_id.clone();
                                    is_mobile_client =
                                        matches!(client_kind, Some(shared::ClientKind::Mobile))
                                            && mobile_device_session_id.is_some();
                                    let mobile_protocol_compatible =
                                        protocol_mutations_allowed(client_kind, protocol_version);
                                    mutations_allowed = mobile_protocol_compatible;
                                    let negotiated_capabilities = capabilities
                                        .iter()
                                        .filter(|capability| {
                                            matches!(
                                                capability.as_str(),
                                                shared::PROJECT_POLICY_CAPABILITY
                                                    | shared::PANE_WORK_SUMMARY_CAPABILITY
                                                    | "mobile_bootstrap_v1"
                                                    | "mobile_coding_mutations_v1"
                                                    | "mobile_terminal_v1"
                                                    | "mobile_notifications_v1"
                                                    | "mobile_deep_links_v1"
                                            )
                                        })
                                        .cloned()
                                        .collect::<Vec<_>>();
                                    user_id = Some(uid);
                                    tracing::info!(
                                        connection_id = %connection_id,
                                        user_id = %uid,
                                        client_kind = ?client_kind,
                                        app_version = app_version.as_deref().unwrap_or("unknown"),
                                        protocol_version = ?protocol_version,
                                        mutations_allowed,
                                        "Web-compatible client authenticated"
                                    );
                                    state.sessions.set_web_user(connection_id, uid);
                                    if is_mobile_client {
                                        state
                                            .mobile_metrics
                                            .increment(MobileMetric::SocketAuthenticated);
                                        if !mobile_protocol_compatible {
                                            state
                                                .mobile_metrics
                                                .increment(MobileMetric::ProtocolIncompatible);
                                        }
                                    }
                                    state
                                        .sessions
                                        .set_web_capabilities(connection_id, capabilities);
                                    state
                                        .sessions
                                        .send_to_web(
                                            &connection_id,
                                            ServerToWeb::Authenticated {
                                                user_id: uid,
                                                user_email: Some(cluster_user.email),
                                                server_version: Some(SERVER_VERSION.to_string()),
                                                cluster_role: cluster_user.cluster_role,
                                                account_status: cluster_user.account_status,
                                                protocol_version: protocol_version.filter(|_| {
                                                    matches!(
                                                        client_kind,
                                                        Some(shared::ClientKind::Mobile)
                                                    )
                                                }),
                                                negotiated_capabilities,
                                                mutations_allowed,
                                            },
                                        )
                                        .await;

                                    if !mobile_protocol_compatible {
                                        state
                                            .sessions
                                            .send_to_web(
                                                &connection_id,
                                                ServerToWeb::ProtocolIncompatible {
                                                    minimum_version:
                                                        shared::MOBILE_PROTOCOL_MIN_VERSION,
                                                    maximum_version:
                                                        shared::MOBILE_PROTOCOL_MAX_VERSION,
                                                    read_only: true,
                                                    message: "Update APAS Mobile to restore coding actions. Read-only session access remains available.".to_string(),
                                                },
                                            )
                                            .await;
                                    }

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
                    tool_call_id,
                    pane_id,
                    request_id,
                }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    else {
                        continue;
                    };
                    if replay_or_claim_mutation_request(
                        &state,
                        &connection_id,
                        request_id.as_deref(),
                    )
                    .await
                    {
                        continue;
                    }
                    let claimed = state.sessions.claim_pending_decision(
                        sid,
                        &tool_call_id,
                        pane_id,
                        shared::MutationKind::Approval,
                    );
                    if claimed.is_none() && request_id.is_some() {
                        send_mutation_ack(
                            &state,
                            &connection_id,
                            request_id.as_deref(),
                            sid,
                            pane_id,
                            shared::MutationKind::Approval,
                            Err("This approval was already resolved or is no longer current"),
                        )
                        .await;
                        continue;
                    }
                    let target_pane = claimed
                        .as_ref()
                        .and_then(|decision| decision.pane_id)
                        .or(pane_id);
                    let routed = state
                        .sessions
                        .route_to_cli(
                            &sid,
                            ServerToCli::Input {
                                session_id: sid,
                                data: "y".to_string(),
                                pane_id: target_pane,
                            },
                        )
                        .await;
                    if !routed {
                        if let Some(decision) = claimed {
                            state
                                .sessions
                                .restore_pending_decision(sid, tool_call_id, decision);
                        }
                    }
                    send_mutation_ack(
                        &state,
                        &connection_id,
                        request_id.as_deref(),
                        sid,
                        target_pane,
                        shared::MutationKind::Approval,
                        routed
                            .then_some(())
                            .ok_or("The project runtime is unavailable"),
                    )
                    .await;
                }
                Ok(WebToServer::Reject {
                    session_id: msg_sid,
                    tool_call_id,
                    pane_id,
                    request_id,
                }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    else {
                        continue;
                    };
                    if replay_or_claim_mutation_request(
                        &state,
                        &connection_id,
                        request_id.as_deref(),
                    )
                    .await
                    {
                        continue;
                    }
                    let claimed = state.sessions.claim_pending_decision(
                        sid,
                        &tool_call_id,
                        pane_id,
                        shared::MutationKind::Approval,
                    );
                    if claimed.is_none() && request_id.is_some() {
                        send_mutation_ack(
                            &state,
                            &connection_id,
                            request_id.as_deref(),
                            sid,
                            pane_id,
                            shared::MutationKind::Approval,
                            Err("This approval was already resolved or is no longer current"),
                        )
                        .await;
                        continue;
                    }
                    let target_pane = claimed
                        .as_ref()
                        .and_then(|decision| decision.pane_id)
                        .or(pane_id);
                    let routed = state
                        .sessions
                        .route_to_cli(
                            &sid,
                            ServerToCli::Input {
                                session_id: sid,
                                data: "n".to_string(),
                                pane_id: target_pane,
                            },
                        )
                        .await;
                    if !routed {
                        if let Some(decision) = claimed {
                            state
                                .sessions
                                .restore_pending_decision(sid, tool_call_id, decision);
                        }
                    }
                    send_mutation_ack(
                        &state,
                        &connection_id,
                        request_id.as_deref(),
                        sid,
                        target_pane,
                        shared::MutationKind::Approval,
                        routed
                            .then_some(())
                            .ok_or("The project runtime is unavailable"),
                    )
                    .await;
                }
                Ok(WebToServer::PauseDeadloop {
                    session_id: msg_sid,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        tracing::info!("Pausing deadloop for session {}", sid);
                        state
                            .sessions
                            .route_to_cli(&sid, ServerToCli::PauseDeadloop { session_id: sid })
                            .await;
                    }
                }
                Ok(WebToServer::ResumeDeadloop {
                    session_id: msg_sid,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        if let Some(pane) = state
                            .sessions
                            .get_session_panes(&sid)
                            .into_iter()
                            .find(|pane| pane.mode == shared::PaneMode::Deadloop)
                        {
                            if !authorize_existing_pane_launch(
                                &state,
                                &connection_id,
                                &sid,
                                pane.pane_id,
                            )
                            .await
                            {
                                continue;
                            }
                        }
                        tracing::info!("Resuming deadloop for session {}", sid);
                        state
                            .sessions
                            .route_to_cli(&sid, ServerToCli::ResumeDeadloop { session_id: sid })
                            .await;
                    }
                }
                Ok(WebToServer::RebootCli {
                    session_id: msg_sid,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        let Some(policy) =
                            effective_policy_for_cli_reboot(&state, &connection_id, &sid).await
                        else {
                            continue;
                        };
                        if let Some(pane) =
                            state
                                .sessions
                                .get_session_panes(&sid)
                                .into_iter()
                                .find(|pane| {
                                    shared::is_retired_launch(pane.provider, pane.model.as_deref())
                                })
                        {
                            send_policy_error(
                                &state,
                                &connection_id,
                                format!(
                                    "Pane {} uses an unsupported retired provider and blocks CLI reboot",
                                    pane.pane_id
                                ),
                            )
                            .await;
                            continue;
                        }
                        if let Some(pane) =
                            state
                                .sessions
                                .get_session_panes(&sid)
                                .into_iter()
                                .find(|pane| {
                                    !policy.allows(pane.kind, pane.provider, pane.model.as_deref())
                                })
                        {
                            send_policy_error(
                                &state,
                                &connection_id,
                                format!(
                                    "Pane {} is noncompliant with cluster policy and blocks CLI reboot",
                                    pane.pane_id
                                ),
                            )
                            .await;
                            continue;
                        }
                        tracing::info!("Rebooting CLI for session {}", sid);
                        let routed = state
                            .sessions
                            .route_to_cli(&sid, reboot_cli_message(sid))
                            .await;
                        if !routed {
                            send_policy_error(
                                &state,
                                &connection_id,
                                "The CLI reboot request could not be delivered. Reboot the CLI manually on the project host, then try again.",
                            )
                            .await;
                        }
                    }
                }
                Ok(WebToServer::CliLifecycleRequest {
                    session_id: sid,
                    request_id,
                    operation,
                }) => {
                    // A client running a bundle from before an operation was
                    // retired decodes to `None`. Drop the request; the socket
                    // stays up and every other message keeps flowing.
                    let Some(operation) = operation else {
                        tracing::debug!(
                            %request_id,
                            "ignoring a lifecycle request for a retired operation"
                        );
                        continue;
                    };
                    let Some(target_sid) =
                        resolve_target_session(&state, &connection_id, Some(sid), session_id).await
                    else {
                        continue;
                    };
                    let Some(request_user) = state.sessions.get_web_user(&connection_id) else {
                        continue;
                    };
                    let cli_message = lifecycle_cli_message(
                        state.sessions.session_supports_capability(
                            &target_sid,
                            shared::CLI_LIFECYCLE_CAPABILITY,
                        ),
                        target_sid,
                        request_id,
                        operation,
                    );
                    if cli_message.is_none() {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::CliLifecycleStatus {
                                    session_id: target_sid,
                                    request_id,
                                    operation,
                                    phase: shared::CliLifecyclePhase::Failed,
                                    message: Some(
                                        "This project CLI is too old for safe lifecycle controls. Upgrade it on the project host."
                                            .to_string(),
                                    ),
                                    inventory: state
                                        .sessions
                                        .lifecycle_inventory(&target_sid),
                                },
                            )
                            .await;
                        continue;
                    }

                    if operation == shared::CliLifecycleOperation::RebootCli {
                        let Some(policy) =
                            effective_policy_for_cli_reboot(&state, &connection_id, &target_sid)
                                .await
                        else {
                            continue;
                        };
                        if let Some(pane) = state
                            .sessions
                            .get_session_panes(&target_sid)
                            .into_iter()
                            .find(|pane| {
                                shared::is_retired_launch(pane.provider, pane.model.as_deref())
                                    || !policy.allows(
                                        pane.kind,
                                        pane.provider,
                                        pane.model.as_deref(),
                                    )
                            })
                        {
                            send_policy_error(
                                &state,
                                &connection_id,
                                format!(
                                    "Pane {} is not allowed by current cluster policy and blocks CLI reboot",
                                    pane.pane_id
                                ),
                            )
                            .await;
                            continue;
                        }
                    }

                    match state.sessions.claim_lifecycle_request(
                        request_id,
                        target_sid,
                        request_user,
                        operation,
                    ) {
                        crate::session::LifecycleRequestClaim::Conflict => {
                            state
                                .sessions
                                .send_to_web(
                                    &connection_id,
                                    ServerToWeb::CliLifecycleStatus {
                                        session_id: target_sid,
                                        request_id,
                                        operation,
                                        phase: shared::CliLifecyclePhase::Failed,
                                        message: Some(
                                            "That request ID is already bound to another lifecycle operation."
                                                .to_string(),
                                        ),
                                        inventory: None,
                                    },
                                )
                                .await;
                        }
                        crate::session::LifecycleRequestClaim::InFlight(status)
                        | crate::session::LifecycleRequestClaim::Replay(status) => {
                            state
                                .sessions
                                .send_to_web(&connection_id, status.message())
                                .await;
                        }
                        crate::session::LifecycleRequestClaim::New => {
                            let accepted = state
                                .sessions
                                .update_lifecycle_request(
                                    request_id,
                                    target_sid,
                                    operation,
                                    shared::CliLifecyclePhase::Accepted,
                                    None,
                                    state.sessions.lifecycle_inventory(&target_sid),
                                )
                                .expect("new lifecycle request must be retained");
                            broadcast_lifecycle_status(&state, target_sid, accepted.message())
                                .await;
                            let routed = state
                                .sessions
                                .route_to_cli(
                                    &target_sid,
                                    cli_message.expect("capability was checked above"),
                                )
                                .await;
                            if !routed {
                                if let Some(failed) = state.sessions.update_lifecycle_request(
                                    request_id,
                                    target_sid,
                                    operation,
                                    shared::CliLifecyclePhase::Failed,
                                    Some(
                                        "The request could not be delivered. Start or restart the CLI manually on the project host."
                                            .to_string(),
                                    ),
                                    None,
                                ) {
                                    broadcast_lifecycle_status(&state, target_sid, failed.message())
                                        .await;
                                }
                                continue;
                            }

                            let timeout_state = state.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(crate::session::LIFECYCLE_REQUEST_TIMEOUT).await;
                                if let Some(timed_out) =
                                    timeout_state.sessions.timeout_lifecycle_request(request_id)
                                {
                                    tracing::warn!(%target_sid, %request_id, ?operation, "CLI lifecycle operation timed out");
                                    broadcast_lifecycle_status(
                                        &timeout_state,
                                        target_sid,
                                        timed_out.message(),
                                    )
                                    .await;
                                }
                            });
                        }
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
                        uid,
                        machine_id,
                        project_id
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
                        .web_supports_capability(&connection_id, shared::PROJECT_POLICY_CAPABILITY)
                        || !state.sessions.daemon_supports_capability(
                            &machine_id,
                            shared::PROJECT_POLICY_CAPABILITY,
                        )
                    {
                        send_policy_error(
                            &state,
                            &connection_id,
                            "The web client or machine daemon is incompatible with cluster project policy",
                        )
                        .await;
                        continue;
                    }
                    let daemon_policy = match state.db.get_project(&project_id).await {
                        Ok(Some(project)) if project.lifecycle_status == "suspended" => {
                            send_policy_error(
                                &state,
                                &connection_id,
                                "This project is suspended; reactivate it before starting the CLI",
                            )
                            .await;
                            continue;
                        }
                        Ok(Some(project)) => {
                            match state.db.get_effective_project_policy(&project.id).await {
                                Ok(policy) => policy,
                                Err(err) => {
                                    tracing::warn!(%err, %project_id, "project policy lookup failed");
                                    send_policy_error(
                                        &state,
                                        &connection_id,
                                        "Could not verify project policy",
                                    )
                                    .await;
                                    continue;
                                }
                            }
                        }
                        Ok(None) => {
                            send_policy_error(
                                &state,
                                &connection_id,
                                "Canonical project metadata is unavailable",
                            )
                            .await;
                            continue;
                        }
                        Err(err) => {
                            tracing::warn!(%err, %project_id, "project lifecycle lookup failed");
                            send_policy_error(
                                &state,
                                &connection_id,
                                "Could not verify project lifecycle",
                            )
                            .await;
                            continue;
                        }
                    };

                    if !state
                        .sessions
                        .send_to_daemon(
                            &machine_id,
                            ServerToDaemon::StartProjectCli {
                                project_id,
                                policy: Some(daemon_policy),
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
                        .send_to_daemon(
                            &machine_id,
                            ServerToDaemon::StopProjectCli {
                                project_id,
                                request_id: None,
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
                        uid,
                        machine_id,
                        git_remote,
                        instance_name
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
                #[allow(deprecated)]
                Ok(WebToServer::SetMachineMiniMaxConfig { .. })
                | Ok(WebToServer::SetMachineGlmConfig { .. }) => {
                    // Decode stale-client messages so the WebSocket remains
                    // usable, but discard credentials without logging or
                    // forwarding them to a daemon.
                    state
                        .sessions
                        .send_to_web(
                            &connection_id,
                            ServerToWeb::Error {
                                message: "Unsupported provider: this backend has been retired"
                                    .to_string(),
                            },
                        )
                        .await;
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
                    if !authorize_existing_pane_launch(&state, &connection_id, &sid, pane_id).await
                    {
                        continue;
                    }
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
                    if !authorize_existing_pane_launch(&state, &connection_id, &sid, pane_id).await
                    {
                        continue;
                    }
                    tracing::info!("Rebooting pane {} for session {}", pane_id, sid);
                    state
                        .sessions
                        .route_to_cli(&sid, reboot_pane_cli_message(sid, pane_id))
                        .await;
                }
                Ok(WebToServer::AddPane {
                    session_id: msg_sid,
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
                    kind,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        if !authorize_new_pane_launch(
                            &state,
                            &connection_id,
                            &sid,
                            kind,
                            provider,
                            model.as_deref(),
                            managed,
                        )
                        .await
                        {
                            continue;
                        }
                        // Generate a unique pane_id starting from 3 (1 and 2 are reserved for legacy deadloop/interactive)
                        let pane_id = 3 + (uuid::Uuid::new_v4().as_u128() % 1000) as u32;
                        let pane_config = shared::PaneConfig {
                            pane_id,
                            provider,
                            mode,
                            kind,
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
                            pane_id,
                            sid,
                            isolated_worktree,
                        );
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::AddPane {
                                    session_id: sid,
                                    pane_config: pane_config.clone(),
                                    isolated_worktree,
                                    initial_input: None,
                                },
                            )
                            .await;
                        // Also broadcast PaneList to web clients
                        // (CLI will send back updated pane config)
                    }
                }
                Ok(WebToServer::RemovePane {
                    session_id: msg_sid,
                    pane_id,
                    cleanup_action,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        let Ok((_project_id, _project_guard)) =
                            state.active_session_operation(&sid.to_string()).await
                        else {
                            continue;
                        };
                        tracing::info!(
                            "Removing pane {} from session {} (cleanup_action={:?})",
                            pane_id,
                            sid,
                            cleanup_action,
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
                        // Drop any terminal scrollback for this pane so a
                        // later pane that reuses the id doesn't inherit the
                        // dead pane's frames on attach.
                        state.sessions.clear_terminal_scrollback(&sid, pane_id);
                    }
                }
                Ok(WebToServer::UpdatePaneLabel {
                    session_id: msg_sid,
                    pane_id,
                    label,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        let Ok((_project_id, _project_guard)) =
                            state.active_session_operation(&sid.to_string()).await
                        else {
                            continue;
                        };
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
                Ok(WebToServer::InterruptPane {
                    session_id: msg_sid,
                    pane_id,
                    request_id,
                }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    else {
                        continue;
                    };
                    if replay_or_claim_mutation_request(
                        &state,
                        &connection_id,
                        request_id.as_deref(),
                    )
                    .await
                    {
                        continue;
                    }
                    if request_id.is_some()
                        && !state
                            .sessions
                            .get_session_panes(&sid)
                            .iter()
                            .any(|pane| pane.pane_id == pane_id)
                    {
                        send_mutation_ack(
                            &state,
                            &connection_id,
                            request_id.as_deref(),
                            sid,
                            Some(pane_id),
                            shared::MutationKind::Interrupt,
                            Err("This pane no longer exists or is not interruptible"),
                        )
                        .await;
                        continue;
                    }
                    tracing::info!("Interrupting pane {} in session {}", pane_id, sid);
                    let routed = state
                        .sessions
                        .route_to_cli(
                            &sid,
                            ServerToCli::InterruptPane {
                                session_id: sid,
                                pane_id,
                            },
                        )
                        .await;
                    send_mutation_ack(
                        &state,
                        &connection_id,
                        request_id.as_deref(),
                        sid,
                        Some(pane_id),
                        shared::MutationKind::Interrupt,
                        routed
                            .then_some(())
                            .ok_or("The project runtime is unavailable"),
                    )
                    .await;
                }
                Ok(WebToServer::PlanReviewAnswer {
                    session_id: msg_sid,
                    tool_use_id,
                    approve,
                    pane_id,
                    request_id,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        if replay_or_claim_mutation_request(
                            &state,
                            &connection_id,
                            request_id.as_deref(),
                        )
                        .await
                        {
                            continue;
                        }
                        let claimed = state.sessions.claim_pending_decision(
                            sid,
                            &tool_use_id,
                            pane_id,
                            shared::MutationKind::PlanReview,
                        );
                        if claimed.is_none() && request_id.is_some() {
                            send_mutation_ack(
                                &state,
                                &connection_id,
                                request_id.as_deref(),
                                sid,
                                pane_id,
                                shared::MutationKind::PlanReview,
                                Err(
                                    "This plan review was already resolved or is no longer current",
                                ),
                            )
                            .await;
                            continue;
                        }
                        tracing::info!(
                            "Plan review answer for session {}: {} → {}",
                            sid,
                            tool_use_id,
                            approve,
                        );
                        let routed = state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::PlanReviewAnswer {
                                    session_id: sid,
                                    tool_use_id: tool_use_id.clone(),
                                    approve,
                                },
                            )
                            .await;
                        if !routed {
                            if let Some(decision) = claimed {
                                state
                                    .sessions
                                    .restore_pending_decision(sid, tool_use_id, decision);
                            }
                        }
                        send_mutation_ack(
                            &state,
                            &connection_id,
                            request_id.as_deref(),
                            sid,
                            pane_id,
                            shared::MutationKind::PlanReview,
                            routed
                                .then_some(())
                                .ok_or("The project runtime is unavailable"),
                        )
                        .await;
                    }
                }
                Ok(WebToServer::UpdatePaneReviewMode {
                    session_id: msg_sid,
                    pane_id,
                    mode,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        tracing::info!(
                            "Update pane {} plan_review_mode for session {} → {:?}",
                            pane_id,
                            sid,
                            mode,
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
                Ok(WebToServer::UpdatePaneManualMode {
                    session_id: msg_sid,
                    pane_id,
                    manual_mode,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        tracing::info!(
                            "Update pane {} manual_mode for session {} → {}",
                            pane_id,
                            sid,
                            manual_mode,
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
                Ok(WebToServer::FetchTeamTodo {
                    session_id: msg_sid,
                }) => {
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
                Ok(WebToServer::TodoApproval {
                    session_id: msg_sid,
                    todo_id,
                    action,
                }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, Some(msg_sid), session_id)
                            .await
                    else {
                        continue;
                    };
                    tracing::info!(
                        "Todo approval for session {}: {} → {}",
                        sid,
                        todo_id,
                        action
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
                Ok(WebToServer::AddTodo {
                    session_id: msg_sid,
                    title,
                    body,
                }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, Some(msg_sid), session_id)
                            .await
                    else {
                        continue;
                    };
                    tracing::info!(
                        "Add TODO for session {}: {} ({} bytes)",
                        sid,
                        title,
                        body.len()
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
                Ok(WebToServer::FetchSuggestedWorkers {
                    session_id: msg_sid,
                }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, Some(msg_sid), session_id)
                            .await
                    else {
                        continue;
                    };
                    state
                        .sessions
                        .route_to_cli(&sid, ServerToCli::FetchSuggestedWorkers { session_id: sid })
                        .await;
                }
                Ok(WebToServer::DismissSuggestion {
                    session_id: msg_sid,
                    suggestion_id,
                }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, Some(msg_sid), session_id)
                            .await
                    else {
                        continue;
                    };
                    tracing::info!("Dismiss suggestion {} for session {}", suggestion_id, sid);
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
                Ok(WebToServer::TerminalInput {
                    session_id: msg_sid,
                    pane_id,
                    data_b64,
                }) => {
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
                            ServerToCli::TerminalInput {
                                session_id: sid,
                                pane_id,
                                data_b64,
                            },
                        )
                        .await;
                }
                Ok(WebToServer::TerminalConversationInput {
                    session_id: msg_sid,
                    pane_id,
                    text,
                    client_msg_id,
                }) => {
                    handle_terminal_conversation_input(
                        &state,
                        &connection_id,
                        session_id,
                        msg_sid,
                        pane_id,
                        text,
                        client_msg_id,
                    )
                    .await;
                }
                Ok(WebToServer::TerminalResize {
                    session_id: msg_sid,
                    pane_id,
                    cols,
                    rows,
                }) => {
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
                            ServerToCli::TerminalResize {
                                session_id: sid,
                                pane_id,
                                cols,
                                rows,
                            },
                        )
                        .await;
                }
                Ok(WebToServer::TerminalAttach {
                    session_id: msg_sid,
                    pane_id,
                }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, Some(msg_sid), session_id)
                            .await
                    else {
                        continue;
                    };
                    if is_mobile_client {
                        state.mobile_metrics.increment(MobileMetric::TerminalAttach);
                    }
                    // Answered straight from the server's ring buffer — no
                    // CLI round-trip, so reattach paints instantly and works
                    // even while the CLI is mid-reconnect.
                    let retained_snapshot = state.sessions.terminal_snapshot(&sid, pane_id);
                    if is_mobile_client && retained_snapshot.is_none() {
                        state
                            .mobile_metrics
                            .increment(MobileMetric::TerminalAttachEmpty);
                    }
                    let (data_b64, seq, truncated, instance_id, lifecycle, status, runtime) =
                        match retained_snapshot {
                            Some(snapshot) => (
                                base64::engine::general_purpose::STANDARD.encode(&snapshot.bytes),
                                snapshot.seq,
                                snapshot.truncated,
                                snapshot.instance_id,
                                snapshot.lifecycle,
                                snapshot.status,
                                snapshot.runtime,
                            ),
                            // No output yet (pane just spawned). Reply with an
                            // empty snapshot rather than staying silent so the
                            // client can leave its "connecting" state. With no
                            // retained report, process lifecycle is unknown.
                            None => (
                                String::new(),
                                0,
                                false,
                                None,
                                TerminalLifecycle::Unknown,
                                None,
                                None,
                            ),
                        };
                    state
                        .sessions
                        .send_to_web(
                            &connection_id,
                            ServerToWeb::TerminalSnapshot {
                                session_id: sid,
                                pane_id,
                                instance_id,
                                data_b64,
                                seq,
                                truncated,
                                lifecycle,
                                status,
                                runtime,
                            },
                        )
                        .await;
                }
                Ok(WebToServer::MobileTelemetry { event }) => {
                    if !is_mobile_client {
                        continue;
                    }
                    let metric = match event {
                        shared::MobileTelemetryEvent::TerminalBridgeReady => {
                            MobileMetric::TerminalBridgeReady
                        }
                        shared::MobileTelemetryEvent::TerminalBridgeRejectedMessage => {
                            MobileMetric::TerminalBridgeRejectedMessage
                        }
                        shared::MobileTelemetryEvent::TerminalBridgeCrash => {
                            MobileMetric::TerminalBridgeCrash
                        }
                    };
                    state.mobile_metrics.increment(metric);
                    tracing::info!(event = ?event, "mobile terminal bridge health event");
                }
                Ok(WebToServer::PromotePaneToManaged {
                    session_id: msg_sid,
                    pane_id,
                }) => {
                    let Some(sid) =
                        resolve_target_session(&state, &connection_id, Some(msg_sid), session_id)
                            .await
                    else {
                        continue;
                    };
                    let pane = state
                        .sessions
                        .get_session_panes(&sid)
                        .into_iter()
                        .find(|pane| pane.pane_id == pane_id);
                    if pane.is_some_and(|pane| pane.kind == shared::PaneKind::Terminal) {
                        send_policy_error(
                            &state,
                            &connection_id,
                            "Terminal panes cannot join a managed team",
                        )
                        .await;
                        continue;
                    }
                    tracing::info!("Promote pane {} → managed for session {}", pane_id, sid);
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
                Ok(WebToServer::UpdatePaneRole {
                    session_id: msg_sid,
                    pane_id,
                    role,
                    goal,
                    backstory,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        tracing::info!(
                            "Updating pane {} role for session {} (role={:?}, goal={:?})",
                            pane_id,
                            sid,
                            role,
                            goal,
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
                Ok(WebToServer::RequestPaneDiff {
                    session_id: msg_sid,
                    pane_id,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        tracing::info!(
                            "Requesting pane diff for pane {} in session {}",
                            pane_id,
                            sid
                        );
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
                Ok(WebToServer::CreatePr {
                    session_id: msg_sid,
                    pane_id,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
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
                Ok(WebToServer::UpdateProjectGoal {
                    session_id: msg_sid,
                    goal,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
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
                    session_id: msg_sid,
                    auto_approve_todos,
                    auto_merge_prs,
                    team_enabled,
                    disallowed_tab_types,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        // `resolve_target_session` only proves this user can
                        // *reach* the session. Project flags are policy for
                        // everyone on it, so they need the stronger check.
                        if !can_manage_project_settings(&state, &connection_id, &sid).await {
                            tracing::warn!(
                                session_id = %sid,
                                "Rejected project flags update — requires project owner or admin"
                            );
                            continue;
                        }
                        tracing::info!(
                            session_id = %sid,
                            auto_approve_todos,
                            auto_merge_prs,
                            team_enabled,
                            "Update project flags"
                        );
                        // Governed team/profile policy is intentionally ignored
                        // here. Legacy clients may still send the combined
                        // payload, but project owners can mutate only workflow
                        // settings; cluster policy has a separate admin API.
                        let _ = (team_enabled, disallowed_tab_types);
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::UpdateProjectOperations {
                                    session_id: sid,
                                    auto_approve_todos,
                                    auto_merge_prs,
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::StartTeam {
                    session_id: msg_sid,
                    manager,
                    tech_lead,
                    reviewer,
                    developer,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        if !authorize_team_launch(
                            &state,
                            &connection_id,
                            &sid,
                            &[&manager, &tech_lead, &reviewer, &developer],
                        )
                        .await
                        {
                            continue;
                        }
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
                Ok(WebToServer::AnswerQuestion {
                    session_id: msg_sid,
                    tool_use_id,
                    pane_id,
                    request_id,
                    answers,
                }) => {
                    // Relay the user's AskUserQuestion answers down to the CLI
                    // streaming worker. The worker matches by tool_use_id
                    // against its pending_questions map and writes the
                    // control_response onto claude's stdin so the SDK's
                    // canUseTool callback completes.
                    //
                    // Route to the session the question belongs to (carried in
                    // the message), NOT the connection's last-attached session:
                    // the web multi-session fan-out overwrites that on every
                    // attach, so a raw fallback misrouted answers to a
                    // different project and left the asking pane stuck.
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        if replay_or_claim_mutation_request(
                            &state,
                            &connection_id,
                            request_id.as_deref(),
                        )
                        .await
                        {
                            continue;
                        }
                        let claimed = state.sessions.claim_pending_decision(
                            sid,
                            &tool_use_id,
                            pane_id,
                            shared::MutationKind::Question,
                        );
                        if claimed.is_none() && request_id.is_some() {
                            send_mutation_ack(
                                &state,
                                &connection_id,
                                request_id.as_deref(),
                                sid,
                                pane_id,
                                shared::MutationKind::Question,
                                Err("This question was already answered or is no longer current"),
                            )
                            .await;
                            continue;
                        }
                        tracing::info!(
                            tool_use_id = tool_use_id.as_str(),
                            answer_count = answers.len(),
                            "Forwarding AskUserQuestion answers to CLI for session {}",
                            sid
                        );
                        let routed = state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::AnswerQuestion {
                                    session_id: sid,
                                    tool_use_id: tool_use_id.clone(),
                                    answers,
                                },
                            )
                            .await;
                        if !routed {
                            if let Some(decision) = claimed {
                                state
                                    .sessions
                                    .restore_pending_decision(sid, tool_use_id, decision);
                            }
                        }
                        send_mutation_ack(
                            &state,
                            &connection_id,
                            request_id.as_deref(),
                            sid,
                            pane_id,
                            shared::MutationKind::Question,
                            routed
                                .then_some(())
                                .ok_or("The project runtime is unavailable"),
                        )
                        .await;
                    }
                }
                Ok(WebToServer::UpdatePaneEffort {
                    session_id: msg_sid,
                    pane_id,
                    effort,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        let Ok((_project_id, _project_guard)) =
                            state.active_session_operation(&sid.to_string()).await
                        else {
                            continue;
                        };
                        let normalized = effort.as_deref().and_then(normalize_start_bot_effort);
                        tracing::info!(
                            "Updating pane {} effort in session {} to {:?}",
                            pane_id,
                            sid,
                            normalized
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
                Ok(WebToServer::UpdatePaneModel {
                    session_id: msg_sid,
                    pane_id,
                    provider,
                    model,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        let Ok((_project_id, _project_guard)) =
                            state.active_session_operation(&sid.to_string()).await
                        else {
                            continue;
                        };
                        let trimmed = model
                            .as_deref()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string);
                        let existing = state
                            .sessions
                            .get_session_panes(&sid)
                            .into_iter()
                            .find(|pane| pane.pane_id == pane_id);
                        let Some(existing) = existing else {
                            send_policy_error(&state, &connection_id, "Pane not found").await;
                            continue;
                        };
                        let desired_provider = provider.unwrap_or(existing.provider);
                        if !authorize_profile_launch(
                            &state,
                            &connection_id,
                            &sid,
                            existing.kind,
                            desired_provider,
                            trimmed.as_deref(),
                            existing.managed,
                        )
                        .await
                        {
                            continue;
                        }
                        tracing::info!(
                            "Switching pane {} agent in session {} to provider={:?} model={:?}",
                            pane_id,
                            sid,
                            provider,
                            trimmed
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
                Ok(WebToServer::ReorderPanes {
                    session_id: msg_sid,
                    pane_ids,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        let Ok((_project_id, _project_guard)) =
                            state.active_session_operation(&sid.to_string()).await
                        else {
                            continue;
                        };
                        tracing::info!("Reordering panes in session {}", sid);
                        let panes = state.sessions.get_session_panes(&sid);
                        let order_map: std::collections::HashMap<u32, usize> = pane_ids
                            .iter()
                            .enumerate()
                            .map(|(i, &id)| (id, i))
                            .collect();
                        let mut ordered: Vec<shared::PaneConfig> = Vec::new();
                        let mut remaining: Vec<shared::PaneConfig> = Vec::new();
                        for pane in panes {
                            if let Some(&pos) = order_map.get(&pane.pane_id) {
                                ordered.push(pane);
                            } else {
                                remaining.push(pane);
                            }
                        }
                        ordered.sort_by_key(|p| {
                            order_map.get(&p.pane_id).copied().unwrap_or(usize::MAX)
                        });
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
                    session_id: msg_sid,
                    pane_id,
                    prompt,
                    min_iteration_interval_minutes,
                    effort,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        let Ok((_project_id, _project_guard)) =
                            state.active_session_operation(&sid.to_string()).await
                        else {
                            continue;
                        };
                        if !authorize_existing_pane_launch(&state, &connection_id, &sid, pane_id)
                            .await
                        {
                            continue;
                        }
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
                Ok(WebToServer::StopBot {
                    session_id: msg_sid,
                    pane_id,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        let Ok((_project_id, _project_guard)) =
                            state.active_session_operation(&sid.to_string()).await
                        else {
                            continue;
                        };
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

                    let Ok((_project_id, _project_guard)) =
                        state.active_session_operation(&sid.to_string()).await
                    else {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Project is unavailable".to_string(),
                                },
                            )
                            .await;
                        continue;
                    };

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

                        if let Some(inventory) = state.sessions.lifecycle_inventory(&sid) {
                            state
                                .sessions
                                .send_to_web(
                                    &connection_id,
                                    ServerToWeb::CliLifecycleInventory {
                                        session_id: sid,
                                        inventory,
                                    },
                                )
                                .await;
                        }

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
                                        panes: panes_to_send.clone(),
                                    },
                                )
                                .await;
                        }

                        if state.sessions.web_supports_capability(
                            &connection_id,
                            shared::PROJECT_POLICY_CAPABILITY,
                        ) {
                            match state.db.get_project_for_session(&sid.to_string()).await {
                                Ok(Some(project)) => {
                                    match state.db.get_effective_project_policy(&project.id).await {
                                        Ok(policy) => {
                                            let noncompliant_pane_ids = panes_to_send
                                                .iter()
                                                .filter(|pane| {
                                                    (!policy.team_available && pane.managed)
                                                        || !policy.allows(
                                                            pane.kind,
                                                            pane.provider,
                                                            pane.model.as_deref(),
                                                        )
                                                })
                                                .map(|pane| pane.pane_id)
                                                .collect();
                                            state
                                                .sessions
                                                .send_to_web(
                                                    &connection_id,
                                                    ServerToWeb::ProjectPolicyChanged {
                                                        session_id: sid,
                                                        policy,
                                                        noncompliant_pane_ids,
                                                    },
                                                )
                                                .await;
                                        }
                                        Err(err) => tracing::warn!(
                                            %err,
                                            session_id = %sid,
                                            "failed to load policy on web attach"
                                        ),
                                    }
                                }
                                Ok(None) => {}
                                Err(err) => tracing::warn!(
                                    %err,
                                    session_id = %sid,
                                    "failed to resolve project on web attach"
                                ),
                            }
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
                            let is_working = is_active
                                && !state.sessions.get_pane_statuses(&session_id).is_empty();
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
                                is_working,
                            }
                        })
                        .collect();

                    // Add shared sessions with owner email
                    for (s, owner_email, share_role) in shared_sessions {
                        let session_id = Uuid::parse_str(&s.id).unwrap_or_default();
                        let is_active = state.sessions.is_session_active(&session_id);
                        let is_working =
                            is_active && !state.sessions.get_pane_statuses(&session_id).is_empty();
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
                            is_working,
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
                    let Some(uid) = user_id else {
                        continue;
                    };
                    if !state
                        .db
                        .check_session_access(&sid.to_string(), &uid.to_string())
                        .await
                        .unwrap_or(false)
                    {
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
                    let Ok((_project_id, _project_guard)) =
                        state.active_session_operation(&sid.to_string()).await
                    else {
                        continue;
                    };
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
                    if is_mobile_client && is_catchup {
                        state.mobile_metrics.increment(MobileMetric::CatchupRequest);
                        tracing::info!(
                            per_pane = pane_watermarks.is_some(),
                            "mobile timeline catch-up requested"
                        );
                    }
                    let is_initial_load =
                        !is_catchup && before_id.is_none() && effective_pane_filter.is_none();
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
                    match fetch_result {
                        Ok((stored_messages, has_more)) => {
                            let messages: Vec<MessageInfo> =
                                stored_messages.into_iter().map(to_message_info).collect();

                            if is_initial_load {
                                let mut panes_to_send = state.sessions.get_session_panes(&sid);
                                if panes_to_send.is_empty() {
                                    if let Ok(stored_panes) =
                                        state.storage.load_pane_list(&sid).await
                                    {
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
                Ok(WebToServer::ListPaneWorkSummaries {
                    session_id: sid,
                    pane_id,
                    include_current,
                }) => {
                    let Some(uid) = user_id else {
                        continue;
                    };
                    if !state.sessions.web_supports_capability(
                        &connection_id,
                        shared::PANE_WORK_SUMMARY_CAPABILITY,
                    ) {
                        continue;
                    }
                    if !state
                        .db
                        .check_session_access(&sid.to_string(), &uid.to_string())
                        .await
                        .unwrap_or(false)
                    {
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
                    let Ok((_project_id, project_guard)) =
                        state.active_session_operation(&sid.to_string()).await
                    else {
                        continue;
                    };
                    match state.pane_work_summaries.list_for_pane(sid, pane_id).await {
                        Ok((summaries, availability)) => {
                            state
                                .sessions
                                .send_to_web(
                                    &connection_id,
                                    ServerToWeb::PaneWorkSummaries {
                                        session_id: sid,
                                        pane_id,
                                        summaries,
                                        availability,
                                    },
                                )
                                .await;
                            if include_current {
                                let summaries = state.pane_work_summaries.clone();
                                tokio::spawn(async move {
                                    // Keep project deletion behind this cache/current-window
                                    // reconciliation just as it was for the synchronous path.
                                    let _project_guard = project_guard;
                                    if let Err(error) =
                                        summaries.reconcile_current_for_pane(sid, pane_id).await
                                    {
                                        tracing::warn!(
                                            %sid,
                                            pane_id,
                                            %error,
                                            "Failed to reconcile current pane summary"
                                        );
                                    }
                                });
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%sid, pane_id, %error, "Failed to list pane summaries");
                            state
                                .sessions
                                .send_to_web(
                                    &connection_id,
                                    ServerToWeb::Error {
                                        message: "Failed to load pane summaries".to_string(),
                                    },
                                )
                                .await;
                        }
                    }
                }
                Ok(WebToServer::RefreshPaneWorkSummary {
                    session_id: sid,
                    pane_id,
                    window_start,
                }) => {
                    let Some(uid) = user_id else {
                        continue;
                    };
                    if !state.sessions.web_supports_capability(
                        &connection_id,
                        shared::PANE_WORK_SUMMARY_CAPABILITY,
                    ) || !state
                        .db
                        .check_session_access(&sid.to_string(), &uid.to_string())
                        .await
                        .unwrap_or(false)
                    {
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
                    let Ok((_project_id, _project_guard)) =
                        state.active_session_operation(&sid.to_string()).await
                    else {
                        continue;
                    };
                    match state
                        .pane_work_summaries
                        .refresh(sid, pane_id, window_start)
                        .await
                    {
                        Ok((summaries, availability)) => {
                            state
                                .sessions
                                .send_to_web(
                                    &connection_id,
                                    ServerToWeb::PaneWorkSummaries {
                                        session_id: sid,
                                        pane_id,
                                        summaries,
                                        availability,
                                    },
                                )
                                .await;
                        }
                        Err(error) => tracing::warn!(
                            %sid,
                            pane_id,
                            %error,
                            "Failed to refresh pane summary"
                        ),
                    }
                }
                Ok(WebToServer::UpdateProjectOperations {
                    session_id: msg_sid,
                    auto_approve_todos,
                    auto_merge_prs,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        if !can_manage_project_settings(&state, &connection_id, &sid).await {
                            tracing::warn!(
                                session_id = %sid,
                                "Rejected project workflow update — requires project owner"
                            );
                            continue;
                        }
                        state
                            .sessions
                            .route_to_cli(
                                &sid,
                                ServerToCli::UpdateProjectOperations {
                                    session_id: sid,
                                    auto_approve_todos,
                                    auto_merge_prs,
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

    // Remove the registry-held sender first so the outbound task can drain any
    // final authentication/access failure already queued for this client. All
    // routing helpers clone and release their DashMap reference before await,
    // so unregistering cannot deadlock with a full channel. Bound the drain in
    // case another transient sender clone or a stalled transport remains.
    state.sessions.unregister_web(&connection_id);
    let mut send_task = send_task;
    if tokio::time::timeout(std::time::Duration::from_millis(250), &mut send_task)
        .await
        .is_err()
    {
        send_task.abort();
    }
    tracing::info!("Web client disconnected: {}", connection_id);
}

#[cfg(test)]
mod mobile_protocol_tests {
    use super::*;

    #[test]
    fn legacy_and_current_web_clients_keep_mutation_access() {
        assert!(protocol_mutations_allowed(None, None));
        assert!(protocol_mutations_allowed(
            Some(shared::ClientKind::Web),
            Some(999)
        ));
    }

    #[test]
    fn incompatible_mobile_clients_are_read_only() {
        assert!(!protocol_mutations_allowed(
            Some(shared::ClientKind::Mobile),
            None
        ));
        assert!(!protocol_mutations_allowed(
            Some(shared::ClientKind::Mobile),
            Some(shared::MOBILE_PROTOCOL_MAX_VERSION + 1)
        ));
        assert!(protocol_mutations_allowed(
            Some(shared::ClientKind::Mobile),
            Some(shared::MOBILE_PROTOCOL_MIN_VERSION)
        ));
    }

    #[test]
    fn read_only_allowlist_excludes_mutations() {
        assert!(is_read_only_message(&WebToServer::ListSessions));
        assert!(is_read_only_message(&WebToServer::TerminalAttach {
            session_id: Uuid::nil(),
            pane_id: 1,
        }));
        assert!(is_read_only_message(&WebToServer::ListPaneWorkSummaries {
            session_id: Uuid::nil(),
            pane_id: 1,
            include_current: true,
        }));
        assert!(is_read_only_message(&WebToServer::MobileTelemetry {
            event: shared::MobileTelemetryEvent::TerminalBridgeReady,
        }));
        assert!(!is_read_only_message(
            &WebToServer::RefreshPaneWorkSummary {
                session_id: Uuid::nil(),
                pane_id: 1,
                window_start: None,
            }
        ));
        assert!(!is_read_only_message(&WebToServer::Input {
            session_id: Some(Uuid::nil()),
            text: "change it".to_string(),
            pane_type: None,
            pane_id: Some(1),
            client_msg_id: Some("request-1".to_string()),
        }));
        assert!(!is_read_only_message(
            &WebToServer::TerminalConversationInput {
                session_id: Uuid::nil(),
                pane_id: 1,
                text: "change it".to_string(),
                client_msg_id: Some("request-2".to_string()),
            }
        ));
    }
}

/// Who may change project-level settings.
///
/// This is the first role gate in the WebSocket layer — everything else here
/// authorizes on *access* and stops there — so it gets its own coverage rather
/// than riding on the flags handler.
#[cfg(test)]
mod project_settings_permission_tests {
    use super::*;
    use crate::config::Config;
    use crate::db::Database;
    use crate::db::{Session, User};

    async fn test_state() -> AppState {
        let dir = std::env::temp_dir().join(format!("apas-project-settings-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp db dir");
        let db_path = dir.join("apas.db").to_string_lossy().to_string();
        let db = Database::new(&db_path).await.expect("create temp db");
        db.run_migrations().await.expect("run migrations");
        let mut config = Config::default();
        config.database.path = db_path;
        AppState::new(db, config)
    }

    fn user(id: Uuid) -> User {
        User {
            id: id.to_string(),
            email: format!("{id}@example.test"),
            password_hash: "hash".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        }
    }

    fn session(session_id: Uuid, owner_id: Uuid) -> Session {
        Session {
            id: session_id.to_string(),
            user_id: owner_id.to_string(),
            cli_client_id: None,
            working_dir: Some("/proj".to_string()),
            hostname: Some("host".to_string()),
            status: "connected".to_string(),
            created_at: None,
            updated_at: None,
            is_paused: false,
            project_id: Some(session_id.to_string()),
            git_remote: None,
            git_remote_url: None,
        }
    }

    /// Register a web connection for `user_id` and return its connection id.
    fn attach_web(state: &AppState, user_id: Uuid) -> Uuid {
        let connection_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(4);
        state.sessions.register_web(connection_id, tx);
        state.sessions.set_web_user(connection_id, user_id);
        connection_id
    }

    /// (owner, session) with the owner's web connection already attached.
    async fn project_with_owner(state: &AppState) -> (Uuid, Uuid, Uuid) {
        let owner_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        state.db.create_user(&user(owner_id)).await.expect("owner");
        state
            .db
            .create_session(&session(session_id, owner_id))
            .await
            .expect("session");
        let connection_id = attach_web(state, owner_id);
        (owner_id, session_id, connection_id)
    }

    async fn share_with(state: &AppState, session_id: Uuid, owner_id: Uuid, role: &str) -> Uuid {
        let member_id = Uuid::new_v4();
        state
            .db
            .create_user(&user(member_id))
            .await
            .expect("member");
        state
            .db
            .create_session_share_with_role(
                &session_id.to_string(),
                &member_id.to_string(),
                &owner_id.to_string(),
                role,
            )
            .await
            .expect("share");
        member_id
    }

    #[tokio::test]
    async fn the_project_owner_may_change_settings() {
        let state = test_state().await;
        let (_owner, session_id, connection_id) = project_with_owner(&state).await;

        assert!(can_manage_project_settings(&state, &connection_id, &session_id).await);
    }

    #[tokio::test]
    async fn a_legacy_project_admin_assignment_is_rejected() {
        let state = test_state().await;
        let (owner_id, session_id, _owner_conn) = project_with_owner(&state).await;
        let member_id = Uuid::new_v4();
        state
            .db
            .create_user(&user(member_id))
            .await
            .expect("member");
        let error = state
            .db
            .create_session_share_with_role(
                &session_id.to_string(),
                &member_id.to_string(),
                &owner_id.to_string(),
                "admin",
            )
            .await
            .expect_err("project admin must no longer be assignable");
        assert!(error.to_string().contains("only 'user' is assignable"));
    }

    #[tokio::test]
    async fn a_plain_user_may_not_change_settings() {
        let state = test_state().await;
        let (owner_id, session_id, _owner_conn) = project_with_owner(&state).await;
        let member_id = share_with(&state, session_id, owner_id, "user").await;
        let member_conn = attach_web(&state, member_id);

        // The whole point of the feature: someone the project was shared with
        // can use it but cannot turn team mode on or off for everyone else.
        assert!(!can_manage_project_settings(&state, &member_conn, &session_id).await);
    }

    #[tokio::test]
    async fn a_user_with_no_role_on_the_project_may_not_change_settings() {
        let state = test_state().await;
        let (_owner, session_id, _owner_conn) = project_with_owner(&state).await;
        let stranger_id = Uuid::new_v4();
        state
            .db
            .create_user(&user(stranger_id))
            .await
            .expect("stranger");
        let stranger_conn = attach_web(&state, stranger_id);

        assert!(!can_manage_project_settings(&state, &stranger_conn, &session_id).await);
    }

    #[tokio::test]
    async fn an_unauthenticated_connection_may_not_change_settings() {
        let state = test_state().await;
        let (_owner, session_id, _owner_conn) = project_with_owner(&state).await;
        // Registered as a web client but never associated with a user.
        let anon_conn = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(4);
        state.sessions.register_web(anon_conn, tx);

        assert!(!can_manage_project_settings(&state, &anon_conn, &session_id).await);
    }
}
