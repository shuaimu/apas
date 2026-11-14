use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use base64::Engine as _;
use futures::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use shared::{
    MessageInfo, ServerToCli, ServerToDaemon, ServerToWeb, SessionAttachmentRejectionReason,
    SessionInfo, SessionStatus, TerminalLifecycle, WebToServer,
};
use std::collections::HashSet;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::mobile_metrics::MobileMetric;
use crate::routes::auth::verify_token;
use crate::session::instances::PaneOwner;
use crate::session::pane_launches::{InstanceStartOutcome, InstanceStartWait};
use crate::session::{AutopilotReceipt, CliRouteOutcome, InstanceAttachment};
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
            | WebToServer::TerminalAttach { .. }
            | WebToServer::ListPaneWorkSummaries { .. }
            | WebToServer::MobileTelemetry { .. }
            | WebToServer::SubscribeJob { .. }
            | WebToServer::UnsubscribeJob { .. }
            | WebToServer::SubscribeWorker { .. }
            | WebToServer::UnsubscribeWorker { .. }
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

/// What pane roster to report to a web client on attach.
///
/// `None` means genuinely unknown — a legacy session predating pane
/// persistence — and only then may the caller infer panes from message
/// history. `Some(panes)` means known, and it may legitimately be **empty**:
/// a new project starts with no panes and closing the last pane returns to
/// that state.
///
/// The distinction is the whole point. While "no roster stored" and "a roster
/// of zero panes" both arrived as an empty vec, attaching to a zero-pane
/// project fell through to inference and resurrected panes from old messages,
/// and an empty roster was suppressed rather than sent — so a web client that
/// reloaded kept showing panes the user had already closed, with nothing to
/// correct it.
async fn resolve_pane_roster(
    state: &AppState,
    session_id: &Uuid,
) -> Option<Vec<shared::PaneConfig>> {
    crate::routes::ws_cli::ensure_forgotten_panes_loaded(state, session_id).await;
    let cached = state.sessions.get_session_panes(session_id);
    if state.sessions.pane_roster_seeded(session_id)
        || (!cached.is_empty() && !state.sessions.pane_roster_unreadable(session_id))
    {
        return Some(cached);
    }
    match crate::routes::ws_cli::load_persisted_pane_roster(state, session_id).await {
        // Seeds only a roster nothing has set yet: an instance's report may
        // have merged while this read was in flight, and must not be undone.
        crate::routes::ws_cli::PersistedRoster::Loaded(roster) => {
            Some(attribute_unhosted_panes(state, session_id, roster).await)
        }
        // What instances reported, if anything, is all that is known. While
        // the file is unreadable that is shown but not seeded, and an empty
        // cache leaves the caller to infer panes it will not seed either.
        crate::routes::ws_cli::PersistedRoster::Absent
        | crate::routes::ws_cli::PersistedRoster::Unreadable => {
            (!cached.is_empty()).then_some(cached)
        }
    }
}

/// Give the untagged panes of a roster loaded from `panes.json` the host they
/// lack. Only a roster persisted before panes carried hosts has such panes,
/// and they ran on the host the session last recorded, which an instance's
/// `SessionStart` stamps the same way. Until then they would show no host, a
/// project-wide Start could not count them for their machine, and forgetting
/// that machine would leave them behind. The attribution is a display and
/// routing hint only; it never makes a machine their owner.
async fn attribute_unhosted_panes(
    state: &AppState,
    session_id: &Uuid,
    mut roster: Vec<shared::PaneConfig>,
) -> Vec<shared::PaneConfig> {
    let unhosted = |pane: &shared::PaneConfig| pane.machine_id.is_none() && pane.hostname.is_none();
    if !roster.iter().any(unhosted) {
        return roster;
    }
    let recorded_host = match state.db.get_session(&session_id.to_string()).await {
        Ok(Some(row)) => row
            .hostname
            .map(|host| host.trim().to_string())
            .filter(|host| !host.is_empty()),
        _ => None,
    };
    let Some(host) = recorded_host else {
        return roster;
    };
    if !state.sessions.has_session_state(session_id) {
        for pane in roster.iter_mut().filter(|pane| unhosted(pane)) {
            pane.hostname = Some(host.clone());
        }
        return roster;
    }
    if state.sessions.stamp_untagged_pane_hosts(session_id, &host) {
        if let Err(error) = state
            .storage
            .save_pane_roster_latest(session_id, || {
                state.sessions.pane_roster_snapshot(session_id)
            })
            .await
        {
            tracing::warn!(%session_id, %error, "failed to persist attributed pane hosts");
        }
    }
    state.sessions.get_session_panes(session_id)
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
                machine_id: None,
                hostname: None,
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
    // Machines belong to the account whose client registered them. There is no
    // deployment-wide branch here any more: an account sees its own cluster,
    // plus machines reachable through projects shared with it.
    let mut machines = state.sessions.get_machines_for_user(user_id);
    let mut shared_cluster_access = std::collections::HashMap::new();
    if let Ok(clusters) = state
        .db
        .list_accessible_clusters(&user_id.to_string())
        .await
    {
        for cluster in clusters
            .into_iter()
            .filter(|cluster| cluster.access == "member")
        {
            let Ok(owner_id) = Uuid::parse_str(&cluster.owner_user_id) else {
                continue;
            };
            let Some(membership) = state
                .db
                .get_cluster_membership(&cluster.owner_user_id, &user_id.to_string())
                .await
                .ok()
                .flatten()
                .filter(|membership| membership.status == "active")
            else {
                continue;
            };
            for mut machine in state.sessions.get_machines_for_user(&owner_id) {
                if !membership.allows_machine(&machine.machine.machine_id.to_string()) {
                    continue;
                }
                let mut project_ids = HashSet::new();
                for project in &machine.projects {
                    let placed = state
                        .db
                        .project_is_placed_in_cluster(&project.project_id, &cluster.owner_user_id)
                        .await
                        .unwrap_or(false);
                    let project_access = state
                        .db
                        .get_project_role_for_user(&project.project_id, &user_id.to_string())
                        .await
                        .ok()
                        .flatten()
                        .is_some();
                    if placed && project_access {
                        project_ids.insert(project.project_id.clone());
                    }
                }
                machine
                    .projects
                    .retain(|project| project_ids.contains(&project.project_id));
                machine.machine.deepseek_backend = None;
                machine.cluster_owner_user_id = Some(cluster.owner_user_id.clone());
                machine.cluster_access = shared::MachineClusterAccess::Member;
                machine.local_project_registration_available = false;
                machine.shared_provisioning_available = state.sessions.daemon_supports_capability(
                    &machine.machine.machine_id,
                    shared::SHARED_PROJECT_PROVISIONING_CAPABILITY,
                );
                shared_cluster_access.insert(machine.machine.machine_id, project_ids);
                machines.push(machine);
            }
        }
    }
    state
        .sessions
        .set_shared_cluster_machine_access(*user_id, shared_cluster_access);
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
    let owners: HashSet<Uuid> = machines
        .iter()
        .filter_map(|machine| state.sessions.daemon_owner(&machine.machine.machine_id))
        .collect();
    for owner in owners {
        if let Err(error) = state.refresh_project_machine_preferences(owner).await {
            tracing::warn!(%owner, %error, "Could not refresh project machine preferences");
        }
    }
    for machine in &mut machines {
        state
            .sessions
            .apply_project_machine_preferences(&machine.machine.machine_id, &mut machine.projects);
    }

    machines
}

pub(crate) async fn has_machine_project_runtime_access(
    state: &AppState,
    user_id: &Uuid,
    machine_id: &Uuid,
    project_id: &str,
) -> bool {
    let Some(machine_owner) = state.sessions.daemon_owner(machine_id) else {
        return false;
    };
    let machine_reports_project = state
        .sessions
        .get_machines_for_user(&machine_owner)
        .into_iter()
        .any(|machine| {
            machine.machine.machine_id == *machine_id
                && machine
                    .projects
                    .iter()
                    .any(|project| project.project_id == project_id)
        });
    if !machine_reports_project
        || !state
            .db
            .project_is_placed_in_cluster(project_id, &machine_owner.to_string())
            .await
            .unwrap_or(false)
    {
        return false;
    }
    if machine_owner == *user_id {
        return true;
    }
    state
        .db
        .cluster_member_can_use_machine(
            &machine_owner.to_string(),
            &user_id.to_string(),
            &machine_id.to_string(),
        )
        .await
        .unwrap_or(false)
        && state
            .db
            .get_project_role_for_user(project_id, &user_id.to_string())
            .await
            .ok()
            .flatten()
            .is_some()
}

/// Before an explicit Start on `machine_id`, place a project registered
/// there in its owner's cluster if nothing has yet, exactly as the
/// instance's `SessionStart` would a moment later.
///
/// `apas register`, and a bare `apas` in a new directory, register a project
/// on the machine without telling the server. Until something starts it
/// there is no project record and no placement, and
/// [`has_machine_project_runtime_access`] refuses every Start of it as
/// "Machine not found", so a project registered that way could never be
/// started from the web. This applies only to `user_id`'s own machine that
/// lists the project, never to a cluster member, who needs a project role
/// first. `authorize_project_registration` creates a missing record owned by
/// `user_id`, refuses a suspended or deleting project or one `user_id` has no
/// access to, and adds the direct placement; a refusal leaves everything as
/// it was, so the access check refuses the Start as before.
pub(crate) async fn place_registered_project(
    state: &AppState,
    user_id: &Uuid,
    machine_id: &Uuid,
    project_id: &str,
) {
    if state.sessions.daemon_owner(machine_id) != Some(*user_id)
        || !state
            .sessions
            .machines_listing_project(project_id)
            .contains(machine_id)
    {
        return;
    }
    if state
        .db
        .project_is_placed_in_cluster(project_id, &user_id.to_string())
        .await
        .unwrap_or(true)
    {
        return;
    }
    let _project_guard = state.project_operation_guard(project_id).await;
    match state
        .db
        .authorize_project_registration(project_id, &user_id.to_string())
        .await
    {
        Ok(_) => tracing::info!(
            %user_id,
            %machine_id,
            %project_id,
            "placed a registered project in its owner's cluster for a Start"
        ),
        Err(error) => tracing::info!(
            %user_id,
            %machine_id,
            %project_id,
            %error,
            "a registered project cannot be placed for a Start"
        ),
    }
}

/// Every known pane of a project, across the sessions that belong to it:
/// the roster in memory where one is loaded, else the persisted one.
async fn project_roster(state: &AppState, project_id: &str) -> Vec<shared::PaneConfig> {
    let mut panes = Vec::new();
    for session_id in state.sessions.project_session_ids(project_id) {
        if let Some(roster) = resolve_pane_roster(state, &session_id).await {
            panes.extend(roster);
        }
    }
    panes
}

/// The machines a project-wide Start starts: each online machine that owns
/// tabs of the project, lists it, and can run an instance beside others.
/// Tabs never move between machines, so starting anywhere else would leave
/// them stopped. An untagged tab counts for the machine its attributed host
/// names. Empty when no machine qualifies.
async fn project_start_machines(state: &AppState, project_id: &str) -> Vec<Uuid> {
    let panes = project_roster(state, project_id).await;
    let tagged: HashSet<Uuid> = panes.iter().filter_map(|pane| pane.machine_id).collect();
    let attributed: HashSet<String> = panes
        .iter()
        .filter(|pane| pane.machine_id.is_none())
        .filter_map(|pane| pane.hostname.as_deref())
        .map(normalize_machine_hostname)
        .filter(|host| !host.is_empty())
        .collect();
    let mut machines: Vec<Uuid> = state
        .sessions
        .machines_listing_project(project_id)
        .into_iter()
        .filter(|machine_id| {
            state.sessions.is_daemon_connected(machine_id)
                && state
                    .sessions
                    .daemon_supports_capability(machine_id, shared::PROJECT_INSTANCES_CAPABILITY)
                && (tagged.contains(machine_id)
                    || state
                        .sessions
                        .machine_hostname(machine_id)
                        .is_some_and(|host| {
                            attributed.contains(&normalize_machine_hostname(&host))
                        }))
        })
        .collect();
    machines.sort();
    machines
}

/// Why a project-wide Close stops a machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopRelation {
    /// The machine the request names, which a Close always stopped.
    Named,
    /// An instance of the project is attached there.
    Attached,
    /// Its connected daemon reports running the project, but no instance
    /// of it is attached: one still starting, or reconnecting after its
    /// connection dropped or the server restarted.
    Reporting,
}

/// The machines a project-wide Close stops, each with why: every machine
/// running the project, as `StopMachineProjectCli.all_instances` says. That
/// is `machine_id`, every machine with an instance of the project attached,
/// and every machine whose connected daemon reports running it while its
/// instance is not attached. A client offers Close on that report, so leaving
/// such a machine out would keep its agents running, and the project would
/// come back when its instance reattached.
///
/// A reporting daemon counts only when it belongs to an account the
/// project's sessions run under, the one account every instance of a session
/// runs under. Another account's checkout can carry the same project id, and
/// its daemon reports running it until that instance's `SessionStart` is
/// refused; it is not part of this project.
async fn project_stop_machines(
    state: &AppState,
    project_id: &str,
    machine_id: Uuid,
) -> Vec<(Uuid, StopRelation)> {
    // The machine named keeps its place first; it is the project's own too
    // when it is also found attached or reporting.
    fn relate(machines: &mut Vec<(Uuid, StopRelation)>, machine: Uuid, relation: StopRelation) {
        match machines.iter_mut().find(|(listed, _)| *listed == machine) {
            Some((_, listed)) if *listed == StopRelation::Named => *listed = relation,
            Some(_) => {}
            None => machines.push((machine, relation)),
        }
    }
    let mut machines = vec![(machine_id, StopRelation::Named)];
    let session_ids = state.sessions.project_session_ids(project_id);
    for session_id in &session_ids {
        for instance in state.sessions.session_attachments(session_id) {
            if let Some(attached) = instance.machine_id {
                relate(&mut machines, attached, StopRelation::Attached);
            }
        }
    }
    let reporting: Vec<Uuid> = state
        .sessions
        .machines_running_project(project_id)
        .into_iter()
        .filter(|machine| {
            !machines
                .iter()
                .any(|(listed, relation)| listed == machine && *relation == StopRelation::Attached)
        })
        .collect();
    if reporting.is_empty() {
        return machines;
    }
    let mut accounts = HashSet::new();
    for session_id in &session_ids {
        match state.db.get_session(&session_id.to_string()).await {
            Ok(Some(row)) => accounts.extend(Uuid::parse_str(&row.user_id).ok()),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%session_id, %error, "session lookup failed for a project Close")
            }
        }
    }
    for machine in reporting {
        if state
            .sessions
            .daemon_owner(&machine)
            .is_some_and(|owner| accounts.contains(&owner))
        {
            relate(&mut machines, machine, StopRelation::Reporting);
        }
    }
    machines
}

/// Whether `user_id` may ask for a project-wide Start or Close of
/// `project_id`, which looks at every machine that runs the project or owns
/// its tabs and names the one it refuses. Only someone who can see the
/// project's content, and with it the hosts of its instances and tabs, may:
/// anyone else is answered as a per-machine request is, with a bare
/// "Machine not found", and learns nothing of where the project runs. Runtime
/// access on any machine implies this access, so no permitted Start or Close
/// is lost.
async fn may_act_on_every_instance(state: &AppState, user_id: &Uuid, project_id: &str) -> bool {
    state
        .db
        .has_project_content_access(project_id, &user_id.to_string())
        .await
        .unwrap_or(false)
}

/// A refusal for one machine of a request that may cover several, naming
/// that machine's host when it does. `nameable` false gives the machine's id
/// instead, as for a machine the server does not know; see
/// [`may_name_machine`].
fn machine_refusal(
    state: &AppState,
    machine_id: Uuid,
    several: bool,
    nameable: bool,
    reason: &str,
) -> String {
    if !several {
        return reason.to_string();
    }
    let host = nameable
        .then(|| state.sessions.machine_hostname(&machine_id))
        .flatten()
        .unwrap_or_else(|| machine_id.to_string());
    format!("{host}: {reason}")
}

/// The policy a daemon is sent with `StartProjectCli`, or why the project
/// must not be started: it is suspended, or its policy cannot be read.
async fn start_policy(
    state: &AppState,
    project_id: &str,
) -> Result<shared::EffectiveProjectPolicy, String> {
    match state.db.get_project(project_id).await {
        Ok(Some(project)) if project.lifecycle_status == "suspended" => {
            Err("This project is suspended; reactivate it before starting the CLI".to_string())
        }
        Ok(Some(project)) => match state.db.get_effective_project_policy(&project.id).await {
            Ok(policy) => Ok(policy),
            Err(err) => {
                tracing::warn!(%err, %project_id, "project policy lookup failed");
                Err("Could not verify project policy".to_string())
            }
        },
        Ok(None) => Err("Canonical project metadata is unavailable".to_string()),
        Err(err) => {
            tracing::warn!(%err, %project_id, "project lifecycle lookup failed");
            Err("Could not verify project lifecycle".to_string())
        }
    }
}

/// Why the project must not start on `machine_id`, or `None`: an instance
/// whose CLI cannot share the project runs it on another machine. Such a CLI
/// is only ever the session's sole instance, so an instance started here
/// would be refused when it attached, after its daemon had already started
/// the project and restored its saved tabs, and a Start is not correlated,
/// so its requester would hear nothing. It is refused before any daemon
/// frame instead, with the reason that `SessionStart` would be refused with.
///
/// That instance's own machine is not refused: a Start there goes ahead as
/// it always did, and its daemon answers that the project already runs. It
/// is recognized by the instance's machine when it reported one, else by its
/// host; one with no host could be anywhere, so every machine is refused.
///
/// A native task launch needs no such check: while such a CLI runs the
/// project, the task goes to its running session and no machine is started
/// (`mobile::launch_task`).
fn beside_older_instance_refusal(
    state: &AppState,
    project_id: &str,
    machine_id: &Uuid,
) -> Option<String> {
    let target_host = state
        .sessions
        .machine_hostname(machine_id)
        .map(|host| normalize_machine_hostname(&host))
        .filter(|host| !host.is_empty());
    state
        .sessions
        .project_session_ids(project_id)
        .iter()
        .flat_map(|session_id| state.sessions.session_attachments(session_id))
        .filter(|instance| !instance.capable)
        .find(|older| {
            let same_machine = match older.machine_id {
                Some(older_machine) => older_machine == *machine_id,
                None => {
                    let older_host = older
                        .hostname
                        .as_deref()
                        .map(normalize_machine_hostname)
                        .filter(|host| !host.is_empty());
                    older_host.is_some() && older_host == target_host
                }
            };
            !same_machine
        })
        .map(|older| crate::session::instances::beside_older_cli_refusal(&older))
}

/// Why the project must not start on `machine_id`, or `None`: that machine's
/// connected daemon cannot run instances, and an instance runs the project on
/// another machine. Such a daemon only runs a project as its only runtime. On
/// a shared home it finds the running instance's claim and skips the start
/// without a word; with its own checkout it starts the project, and its CLI
/// is refused when it attaches. A Start is not correlated, so its requester
/// would hear nothing either way. It is refused before any daemon frame
/// instead, with the reason a new tab targeted at that machine gets.
fn older_daemon_beside_instance_refusal(
    state: &AppState,
    project_id: &str,
    machine_id: &Uuid,
) -> Option<String> {
    if !state.sessions.is_daemon_connected(machine_id)
        || state
            .sessions
            .daemon_supports_capability(machine_id, shared::PROJECT_INSTANCES_CAPABILITY)
    {
        return None;
    }
    let runs_elsewhere = state
        .sessions
        .project_session_ids(project_id)
        .iter()
        .flat_map(|session_id| state.sessions.session_attachments(session_id))
        .any(|instance| instance.capable && instance.machine_id != Some(*machine_id));
    runs_elsewhere.then(|| {
        let host = machine_label(state, *machine_id);
        format!(
            "{host} runs an older APAS that cannot run this project beside other machines. Reboot {host} to update it."
        )
    })
}

/// `StartMachineProjectCli`. A per-machine Start on the Machines page, and
/// every older client, starts only `machine_id`. The project view's Start
/// (`all_instances`) starts each machine [`project_start_machines`] names,
/// and `machine_id` only when none qualifies. Each machine is authorized as
/// a per-machine Start would be, and when several were chosen a refusal names
/// the host it concerns. A machine that cannot run the project beside an
/// older CLI running it elsewhere, and a machine whose daemon cannot run
/// instances while an instance runs the project elsewhere, are refused
/// before any daemon frame; see [`beside_older_instance_refusal`] and
/// [`older_daemon_beside_instance_refusal`].
async fn start_machine_project(
    state: &AppState,
    connection_id: &Uuid,
    user_id: &Uuid,
    machine_id: Uuid,
    project_id: &str,
    all_instances: bool,
) {
    // A project only registered on the requester's machine is placed first,
    // or no Start of it would ever pass the access checks below.
    place_registered_project(state, user_id, &machine_id, project_id).await;
    // Checked before the fan-out is even computed: it reads the project's
    // roster, and its refusals name the hosts it found.
    if all_instances && !may_act_on_every_instance(state, user_id, project_id).await {
        send_policy_error(state, connection_id, "Machine not found").await;
        return;
    }
    let fanned_out = if all_instances {
        project_start_machines(state, project_id).await
    } else {
        Vec::new()
    };
    let several = !fanned_out.is_empty();
    let targets = if several {
        fanned_out
    } else {
        vec![machine_id]
    };

    // A refusal names a host only when several machines were chosen, and
    // those all list the project and own its tabs.
    let mut authorized = Vec::new();
    for target in targets {
        if has_machine_project_runtime_access(state, user_id, &target, project_id).await {
            authorized.push(target);
        } else {
            send_policy_error(
                state,
                connection_id,
                machine_refusal(state, target, several, true, "Machine not found"),
            )
            .await;
        }
    }
    if authorized.is_empty() {
        return;
    }
    let incompatible =
        "The web client or machine daemon is incompatible with cluster project policy";
    if !state
        .sessions
        .web_supports_capability(connection_id, shared::PROJECT_POLICY_CAPABILITY)
    {
        send_policy_error(state, connection_id, incompatible).await;
        return;
    }
    let mut compatible = Vec::new();
    for target in authorized {
        let refusal = if !state
            .sessions
            .daemon_supports_capability(&target, shared::PROJECT_POLICY_CAPABILITY)
        {
            Some(incompatible.to_string())
        } else {
            beside_older_instance_refusal(state, project_id, &target)
                .or_else(|| older_daemon_beside_instance_refusal(state, project_id, &target))
        };
        match refusal {
            None => compatible.push(target),
            Some(refusal) => {
                send_policy_error(
                    state,
                    connection_id,
                    machine_refusal(state, target, several, true, &refusal),
                )
                .await
            }
        }
    }
    if compatible.is_empty() {
        return;
    }
    let daemon_policy = match start_policy(state, project_id).await {
        Ok(policy) => policy,
        Err(message) => {
            send_policy_error(state, connection_id, message).await;
            return;
        }
    };

    for target in compatible {
        if !state
            .sessions
            .send_to_daemon(
                &target,
                ServerToDaemon::StartProjectCli {
                    project_id: project_id.to_string(),
                    policy: Some(daemon_policy.clone()),
                    request_id: None,
                },
            )
            .await
        {
            send_policy_error(
                state,
                connection_id,
                machine_refusal(state, target, several, true, "Daemon is offline"),
            )
            .await;
        }
    }
}

/// `StopMachineProjectCli`. A per-machine Stop, and every older client,
/// stops only `machine_id`; the project view's Close (`all_instances`) stops
/// every machine running the project, as [`project_stop_machines`] finds
/// them: the machine named, every attached instance's, and every machine
/// whose connected daemon reports running it while its instance is not
/// attached. Each machine is authorized as a per-machine Stop would be.
///
/// When several machines are stopped a refusal names the host it concerns,
/// except the machine the request named when [`may_name_machine`] does not
/// allow it: that one is refused by its id, as an unknown machine is.
async fn stop_machine_project(
    state: &AppState,
    connection_id: &Uuid,
    user_id: &Uuid,
    machine_id: Uuid,
    project_id: &str,
    all_instances: bool,
) {
    // As for a project-wide Start: its refusals name the hosts it found.
    if all_instances && !may_act_on_every_instance(state, user_id, project_id).await {
        send_policy_error(state, connection_id, "Machine not found").await;
        return;
    }
    let targets = if all_instances {
        project_stop_machines(state, project_id, machine_id).await
    } else {
        vec![(machine_id, StopRelation::Named)]
    };
    let several = targets.len() > 1;
    let session_ids = state.sessions.project_session_ids(project_id);
    for (target, relation) in targets {
        if !has_machine_project_runtime_access(state, user_id, &target, project_id).await {
            // Attached and reporting machines are the project's own; only
            // the machine the request named can be any machine at all.
            let nameable = relation != StopRelation::Named
                || (several && may_name_machine(state, &session_ids, user_id, &target).await);
            send_policy_error(
                state,
                connection_id,
                machine_refusal(state, target, several, nameable, "Machine not found"),
            )
            .await;
            continue;
        }
        if !state
            .sessions
            .send_to_daemon(
                &target,
                ServerToDaemon::StopProjectCli {
                    project_id: project_id.to_string(),
                    request_id: None,
                },
            )
            .await
        {
            // Runtime access there means the requester can see the machine.
            send_policy_error(
                state,
                connection_id,
                machine_refusal(state, target, several, true, "Daemon is offline"),
            )
            .await;
        }
    }
}

async fn resolve_project_creation_target(
    state: &AppState,
    requester: &Uuid,
    machine_id: &Uuid,
    selected_cluster_owner: Option<&str>,
) -> Result<(Uuid, String, bool), &'static str> {
    let machine_owner = state
        .sessions
        .daemon_owner(machine_id)
        .ok_or("Machine not found")?;
    let requested_cluster = selected_cluster_owner
        .map(str::to_string)
        .unwrap_or_else(|| machine_owner.to_string());
    if requested_cluster != machine_owner.to_string() {
        return Err("Machine does not belong to the selected cluster");
    }
    let shared_request = machine_owner != *requester;
    if shared_request
        && !state
            .db
            .cluster_member_can_use_machine(
                &requested_cluster,
                &requester.to_string(),
                &machine_id.to_string(),
            )
            .await
            .unwrap_or(false)
    {
        return Err("This machine is not enabled for your cluster membership");
    }
    if shared_request
        && !state
            .sessions
            .daemon_supports_capability(machine_id, shared::SHARED_PROJECT_PROVISIONING_CAPABILITY)
    {
        return Err("This machine must update APAS before members can create projects");
    }
    Ok((machine_owner, requested_cluster, shared_request))
}

#[cfg(test)]
mod machine_access_tests {
    use super::*;
    use crate::config::Config;
    use crate::db::{Database, Session, User};
    use chrono::{Duration, Utc};

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
            preferred_machine_id: None,
            is_running: false,
            pid: None,
            memory_kb: None,
            last_error: None,
            git_remote: None,
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

    async fn provision_member_project(
        state: &AppState,
        owner: Uuid,
        member: Uuid,
        machine_id: Uuid,
        project_id: &str,
    ) {
        let request_id = Uuid::new_v4().to_string();
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
                project_id,
            )
            .await
            .unwrap();
        state
            .db
            .mark_project_provisioning_cloned(&request_id, project_id, "/managed/codex")
            .await
            .unwrap();
        state
            .db
            .finalize_project_provisioning(&request_id, &member.to_string())
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn shared_creation_rejects_forged_cluster_and_old_daemon() {
        let state = test_state().await;
        let owner = Uuid::new_v4();
        let member = Uuid::new_v4();
        state
            .db
            .create_user(&test_user(owner, "owner@example.test"))
            .await
            .unwrap();
        state
            .db
            .create_user(&test_user(member, "member@example.test"))
            .await
            .unwrap();
        join_cluster(&state, owner, member, "member@example.test").await;
        let machine_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(1);
        state.sessions.register_daemon(
            machine_id,
            owner,
            tx,
            test_machine(machine_id, "shared-host"),
            Vec::new(),
        );

        assert_eq!(
            resolve_project_creation_target(
                &state,
                &member,
                &machine_id,
                Some(&Uuid::new_v4().to_string()),
            )
            .await
            .unwrap_err(),
            "Machine does not belong to the selected cluster"
        );
        assert_eq!(
            resolve_project_creation_target(
                &state,
                &member,
                &machine_id,
                Some(&owner.to_string()),
            )
            .await
            .unwrap_err(),
            "This machine must update APAS before members can create projects"
        );
    }

    #[tokio::test]
    async fn shared_machine_projection_and_creation_honor_member_machine_policy() {
        let state = test_state().await;
        let owner = Uuid::new_v4();
        let member = Uuid::new_v4();
        state
            .db
            .create_user(&test_user(owner, "owner@example.test"))
            .await
            .unwrap();
        state
            .db
            .create_user(&test_user(member, "member@example.test"))
            .await
            .unwrap();
        join_cluster(&state, owner, member, "member@example.test").await;
        let allowed_machine = Uuid::new_v4();
        let blocked_machine = Uuid::new_v4();
        for (machine_id, hostname) in [
            (allowed_machine, "allowed-host"),
            (blocked_machine, "blocked-host"),
        ] {
            let (tx, _rx) = mpsc::channel(1);
            state.sessions.register_daemon(
                machine_id,
                owner,
                tx,
                test_machine(machine_id, hostname),
                Vec::new(),
            );
        }
        state
            .db
            .update_cluster_member_policy(
                &owner.to_string(),
                &member.to_string(),
                Some(vec![allowed_machine.to_string()]),
                None,
            )
            .await
            .unwrap()
            .unwrap();

        let projected = list_accessible_machines_for_user(&state, &member).await;
        assert!(projected
            .iter()
            .any(|entry| entry.machine.machine_id == allowed_machine));
        assert!(!projected
            .iter()
            .any(|entry| entry.machine.machine_id == blocked_machine));
        assert_eq!(
            resolve_project_creation_target(
                &state,
                &member,
                &blocked_machine,
                Some(&owner.to_string()),
            )
            .await
            .unwrap_err(),
            "This machine is not enabled for your cluster membership"
        );
    }

    #[tokio::test]
    async fn shared_cluster_projection_redacts_secrets_and_unrelated_projects_on_refresh_and_push()
    {
        let state = test_state().await;
        let owner = Uuid::new_v4();
        let member = Uuid::new_v4();
        state
            .db
            .create_user(&test_user(owner, "owner@example.test"))
            .await
            .unwrap();
        state
            .db
            .create_user(&test_user(member, "member@example.test"))
            .await
            .unwrap();
        join_cluster(&state, owner, member, "member@example.test").await;
        let machine_id = Uuid::new_v4();
        let project_id = Uuid::new_v4().to_string();
        provision_member_project(&state, owner, member, machine_id, &project_id).await;
        let mut machine = test_machine(machine_id, "shared-host");
        machine.deepseek_backend = Some(shared::DeepseekBackendInfo {
            api_base_url: Some("https://api.deepseek.com/anthropic".to_string()),
            api_key: Some("must-not-leak".to_string()),
            api_key_configured: true,
        });
        let (tx, _rx) = mpsc::channel(1);
        state.sessions.register_daemon(
            machine_id,
            owner,
            tx,
            machine,
            vec![
                test_project(&project_id, "/managed/codex"),
                test_project("unrelated", "/private/owner"),
            ],
        );

        let refreshed = list_accessible_machines_for_user(&state, &member).await;
        let shared = refreshed
            .iter()
            .find(|entry| entry.machine.machine_id == machine_id)
            .unwrap();
        assert!(shared.machine.deepseek_backend.is_none());
        assert_eq!(shared.projects.len(), 1);
        assert_eq!(shared.projects[0].project_id, project_id);
        assert!(!shared.shared_provisioning_available);

        let web_id = Uuid::new_v4();
        let (web_tx, mut web_rx) = mpsc::channel(1);
        state.sessions.register_web(web_id, web_tx);
        state.sessions.set_web_user(web_id, member);
        state.sessions.broadcast_machines_update_for_user(&member);
        let ServerToWeb::Machines { machines } = web_rx.try_recv().unwrap() else {
            panic!("expected cached machine projection")
        };
        let pushed = machines
            .iter()
            .find(|entry| entry.machine.machine_id == machine_id)
            .unwrap();
        assert!(pushed.machine.deepseek_backend.is_none());
        assert_eq!(pushed.projects.len(), 1);
        assert_eq!(pushed.projects[0].project_id, project_id);
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
        Ok(Some(role)) if super::share::parse_share_role(&role).can_manage_access() => true,
        Ok(_) => {
            // The operator of the cluster hosting this project manages its
            // settings too: the project runs on their machines even when
            // another account owns it.
            let Ok(Some(project_id)) = state
                .db
                .get_project_id_for_session(&session_id.to_string())
                .await
            else {
                return false;
            };
            state
                .db
                .project_in_user_cluster(&project_id, &user_id.to_string())
                .await
                .unwrap_or(false)
        }
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

const RETIRED_BOT_MESSAGE: &str = "upgrade_required: Legacy bot history is read-only; create and explicitly approve a new independent worker.";

/// Why typed input cannot go to `pane`, when the tab is history only: a
/// legacy bot, or a tab whose provider is retired (a saved Pi tab). Such a
/// tab has no process on any CLI, so input sent on would be stored and shown
/// as a message no agent ever answers.
#[allow(deprecated)]
fn read_only_pane_message(pane: &shared::PaneConfig) -> Option<String> {
    if pane.mode == shared::PaneMode::Deadloop {
        return Some(RETIRED_BOT_MESSAGE.to_owned());
    }
    if !shared::is_retired_launch(pane.provider, pane.model.as_deref()) {
        return None;
    }
    Some(if pane.provider == shared::Provider::Pi {
        format!("{} This tab is read-only.", shared::PI_RETIRED_MESSAGE)
    } else {
        "This pane uses a retired provider and is read-only.".to_owned()
    })
}

/// Whether raw keystrokes for `pane_id` are sent on to its CLI. A retired
/// provider's tab has no terminal, and its terminal state already says why,
/// so its keystrokes are dropped without an error for every key pressed.
fn terminal_accepts_keystrokes(state: &AppState, session_id: &Uuid, pane_id: u32) -> bool {
    !state
        .sessions
        .get_session_panes(session_id)
        .iter()
        .any(|pane| {
            pane.pane_id == pane_id
                && shared::is_retired_launch(pane.provider, pane.model.as_deref())
        })
}

/// [`read_only_pane_message`] for the tab `pane_id` of a session's roster.
fn read_only_pane_refusal(state: &AppState, session_id: &Uuid, pane_id: u32) -> Option<String> {
    state
        .sessions
        .get_session_panes(session_id)
        .iter()
        .find(|pane| pane.pane_id == pane_id)
        .and_then(read_only_pane_message)
}

/// Why the provider or model of `pane` cannot be switched: its own provider
/// is retired, and the CLI never switches such a tab. Refusing here keeps the
/// roster true. Accepting a switch of a saved Pi tab to OMP used to store and
/// show it as an OMP tab while its CLI kept it as Pi and refused.
#[allow(deprecated)]
fn retired_pane_switch_refusal(pane: &shared::PaneConfig) -> Option<String> {
    if !shared::is_retired_launch(pane.provider, pane.model.as_deref()) {
        return None;
    }
    Some(if pane.provider == shared::Provider::Pi {
        format!(
            "Tab {} uses Pi, which is no longer supported. Close it and open an OMP tab.",
            pane.pane_id
        )
    } else {
        shared::retired_launch_message(pane.provider).to_owned()
    })
}

/// Refuse typed input for `pane_id` with `message`, for a refusal that
/// resending the same input cannot fix. Every client is sent the `Error`, as
/// before. A connection that negotiated project instances and tagged the
/// input with a `client_msg_id` is also sent a
/// [`shared::MutationKind::Input`] refusal under that id, after the `Error`:
/// a client that resends input until it is acknowledged then stops and shows
/// why, instead of resending and giving up with no reason. Older native
/// builds would drop their socket on the new kind, hence the gate.
///
/// Unlike [`send_mutation_ack`] nothing is retained for replay, no decision
/// is registered and no mutation metric moves: a resend is judged afresh,
/// since the tab's machine may run the project by then, and accepted input
/// is deduplicated by its echoed id, which a refused input never records.
async fn refuse_input(
    state: &AppState,
    connection_id: &Uuid,
    session_id: Uuid,
    pane_id: u32,
    client_msg_id: Option<&str>,
    message: String,
) {
    send_policy_error(state, connection_id, message.clone()).await;
    let Some(request_id) = client_msg_id else {
        return;
    };
    if !state
        .sessions
        .web_supports_capability(connection_id, shared::PROJECT_INSTANCES_CAPABILITY)
    {
        return;
    }
    state
        .sessions
        .send_to_web(
            connection_id,
            ServerToWeb::MutationAck {
                request_id: request_id.to_string(),
                session_id,
                pane_id: Some(pane_id),
                mutation: shared::MutationKind::Input,
                accepted: false,
                error: Some(message),
                question_pending: None,
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
        question_pending: None,
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

async fn handle_omp_question_answer(state: &AppState, connection_id: &Uuid, command: WebToServer) {
    let WebToServer::AnswerOmpQuestion {
        session_id,
        pane_id,
        tool_use_id,
        dialog_id,
        request_id,
        answers,
        cancel,
    } = command
    else {
        return;
    };
    let Some(user_id) = state.sessions.get_web_user(connection_id) else {
        return;
    };
    if request_id.is_empty() || request_id.len() > 128 {
        send_policy_error(state, connection_id, "Invalid question request identifier").await;
        return;
    }
    if replay_or_claim_mutation_request(state, connection_id, Some(&request_id)).await {
        return;
    }
    let result: Result<crate::session::OmpAnswerReceipt, String> = async {
        resolve_target_session(state, connection_id, Some(session_id), None)
            .await
            .ok_or("This project runtime is unavailable or access was denied")?;
        if !tool_use_id.starts_with("omp:") || tool_use_id.len() > 1024 {
            return Err("This is not an OMP question".into());
        }
        if !state.sessions.get_session_panes(&session_id).iter().any(|pane| {
            pane.pane_id == pane_id
                && pane.kind.is_terminal()
                && pane.provider == shared::Provider::Omp
        }) {
            return Err("This OMP terminal pane is no longer available".into());
        }
        // The question's dialog is in the instance that runs the pane, so the
        // answer, its capability check and the waiter all name that instance.
        let owner = match state.sessions.pane_owner(&session_id, pane_id) {
            PaneOwner::Instance(owner) => owner,
            PaneOwner::NotRunning { machine_id, hostname } => {
                return Err(not_running_message(state, machine_id, hostname));
            }
            PaneOwner::NoInstance => return Err("The project runtime is unavailable".into()),
        };
        if let Some(refusal) = instance_access_refusal(state, connection_id, &session_id, &owner).await {
            return Err(refusal);
        }
        if !state.sessions.cli_supports_capability(&owner.cli_id, shared::OMP_QUESTION_CAPABILITY) {
            return Err("Update the project CLI before answering OMP questions here; use the terminal meanwhile".into());
        }
        let cli_id = owner.cli_id;
        let decision = state.sessions.claim_pending_decision(
            session_id, &tool_use_id, Some(pane_id), shared::MutationKind::Question,
        ).ok_or("This question was already answered or is no longer current")?;
        let Some(receiver) = state.sessions.begin_omp_answer(
            session_id, pane_id, cli_id, tool_use_id.clone(), request_id.clone(),
        ) else {
            state.sessions.restore_pending_decision(session_id, tool_use_id.clone(), decision);
            return Err("An answer for this pane is already being processed".into());
        };
        let routed = state.sessions.send_to_cli(&cli_id, ServerToCli::AnswerOmpQuestion {
            session_id,
            pane_id,
            tool_use_id: tool_use_id.clone(),
            dialog_id,
            request_id: request_id.clone(),
            answers,
            cancel,
        }).await;
        if !routed {
            state.sessions.cancel_omp_answer_waiter(session_id, pane_id, &request_id);
            state.sessions.restore_pending_decision(session_id, tool_use_id.clone(), decision);
            return Err("The project runtime is unavailable; no answer was delivered".into());
        }
        let response = tokio::time::timeout(Duration::from_secs(5), receiver).await;
        state.sessions.cancel_omp_answer_waiter(session_id, pane_id, &request_id);
        let receipt = response.ok().and_then(Result::ok)
            .ok_or("Answer delivery could not be confirmed; check the terminal before trying again")?;
        if !receipt.accepted && receipt.pending
            && state.sessions.get_pane_statuses(&session_id).iter().any(|(_, id, status)| {
                *id == pane_id && status == shared::PANE_STATUS_PENDING_ANSWER
            })
        {
            state.sessions.restore_pending_decision(session_id, tool_use_id.clone(), decision);
        }
        Ok(receipt)
    }.await;
    let (accepted, question_pending, error) = match result {
        Ok(receipt) => (receipt.accepted, Some(receipt.pending), receipt.error),
        Err(error) => (false, None, Some(error.to_string())),
    };
    let acknowledgement = ServerToWeb::MutationAck {
        request_id: request_id.clone(),
        session_id,
        pane_id: Some(pane_id),
        mutation: shared::MutationKind::Question,
        accepted,
        error,
        question_pending,
    };
    state
        .sessions
        .complete_mutation_request(user_id, request_id, acknowledgement.clone());
    state
        .sessions
        .send_to_web(connection_id, acknowledgement)
        .await;
}

/// How long a requester waits for the tab's instance to answer an autopilot
/// request before it is told the request is not confirmed. The instance's own
/// waits (a second for its lock, two for the tab, and one more for a start
/// the tab took) stay inside it.
const AUTOPILOT_RECEIPT_WAIT: Duration = Duration::from_secs(5);

/// Answer one autopilot request, to the connection that sent it and to no
/// other. Older native builds drop their socket on a frame type they do not
/// know, so nothing is sent to a connection that did not negotiate autopilot.
async fn send_autopilot_receipt(
    state: &AppState,
    connection_id: &Uuid,
    session_id: Uuid,
    pane_id: u32,
    request_id: String,
    receipt: AutopilotReceipt,
) {
    if !state
        .sessions
        .web_supports_capability(connection_id, shared::TAB_AUTOPILOT_CAPABILITY)
    {
        return;
    }
    state
        .sessions
        .send_to_web(
            connection_id,
            ServerToWeb::PaneAutopilotReceipt {
                session_id,
                pane_id,
                request_id,
                outcome: receipt.outcome,
                reason: receipt.reason,
            },
        )
        .await;
}

/// The refusal for an autopilot request whose tab runs on an instance that
/// did not advertise autopilot: an older CLI drops the frame without an
/// answer, so it is never sent one. Clients show this same wording for such
/// a tab.
fn older_instance_autopilot_refusal(state: &AppState, instance: &InstanceAttachment) -> String {
    let host = instance
        .hostname
        .as_deref()
        .map(|host| host.trim().to_string())
        .filter(|host| !host.is_empty())
        .or_else(|| {
            instance
                .machine_id
                .and_then(|machine_id| state.sessions.machine_hostname(&machine_id))
                .map(|host| host.trim().to_string())
                .filter(|host| !host.is_empty())
        });
    match host {
        Some(host) => format!(
            "{host} runs an older APAS that cannot run autopilot. Reboot {host} to update it."
        ),
        None => "This tab's machine runs an older APAS that cannot run autopilot. Reboot it to update it."
            .to_string(),
    }
}

/// Check one autopilot request and forward it to the instance that owns its
/// tab. Returns the wait for that instance's receipt, or why the request is
/// refused, in which case nothing reached the tab.
///
/// Access is the check input to the tab gets, on the machine of the instance
/// that owns it, and reads the same when refused. It is the same for a start
/// as for a stop: whoever may type into a tab may leave it on autopilot, and
/// nobody else may stop it through APAS.
async fn forward_pane_autopilot_control(
    state: &AppState,
    connection_id: &Uuid,
    session_id: Uuid,
    pane_id: u32,
    request_id: &str,
    action: &shared::AutopilotAction,
) -> Result<tokio::sync::oneshot::Receiver<AutopilotReceipt>, String> {
    use crate::session::autopilot::{AutopilotForwardError, AutopilotTarget};
    let session_id = target_session(state, connection_id, Some(session_id), None)
        .await
        .map_err(|refusal| refusal.unwrap_or("Project is unavailable").to_string())?;
    // Held until the request is handed over, as input holds it: a project
    // being suspended or deleted waits for it, and none is forwarded after.
    let Ok((_project_id, _project_guard)) = state
        .active_session_operation(&session_id.to_string())
        .await
    else {
        return Err("Project is unavailable".to_string());
    };
    // The loop runs inside the tab's own OMP process, so the request, its
    // capability check and the waiter all name the instance that runs the tab.
    let owner = match state.sessions.autopilot_target(&session_id, pane_id) {
        AutopilotTarget::Unlisted => return Err("This tab is no longer available.".to_string()),
        AutopilotTarget::LegacyBotHistory => {
            return Err(
                "Legacy bot history is read-only; it cannot be put on autopilot.".to_string(),
            )
        }
        AutopilotTarget::NotOmpTerminal => {
            return Err("Autopilot is available only on OMP terminal tabs.".to_string())
        }
        AutopilotTarget::Tab(PaneOwner::Instance(owner)) => owner,
        AutopilotTarget::Tab(PaneOwner::NotRunning {
            machine_id,
            hostname,
        }) => return Err(not_running_message(state, machine_id, hostname)),
        AutopilotTarget::Tab(PaneOwner::NoInstance) => {
            return Err("CLI client not connected".to_string())
        }
    };
    if let Some(refusal) = instance_access_refusal(state, connection_id, &session_id, &owner).await
    {
        return Err(refusal);
    }
    if !state
        .sessions
        .cli_supports_capability(&owner.cli_id, shared::OMP_TAB_AUTOPILOT_CAPABILITY)
    {
        return Err(older_instance_autopilot_refusal(state, &owner));
    }
    // A frame that fails to parse is dropped without an answer, so a start's
    // bounds arrive as optional fields and are checked here, where a refusal
    // can say why. The instance and the tab check them again.
    let command = action.validate()?;
    // Queued for the instance without waiting, so a stalled instance cannot
    // hold up this connection's other frames, and only if that instance still
    // owns the tab now that the checks above have awaited.
    state
        .sessions
        .forward_autopilot_request(session_id, pane_id, owner.cli_id, request_id, command)
        .map_err(|error| match error {
            AutopilotForwardError::AlreadyWaiting => {
                "A request with this identifier is already being answered for this tab."
            }
            AutopilotForwardError::Undelivered => {
                "The project runtime is unavailable; nothing was sent to the tab."
            }
        })
        .map_err(str::to_string)
}

/// `ControlPaneAutopilot`: start, pause, resume or stop autopilot on one OMP
/// terminal tab.
///
/// The request goes to the instance that owns the tab and to no other, and is
/// refused, never redirected, when that machine is not running the project.
/// Every answer is a `PaneAutopilotReceipt` to the requesting connection: a
/// refusal with its reason, or what the instance answered. That answer is
/// awaited in a task of its own, so a slow tab cannot hold up this
/// connection's other frames, and requests are tracked by id, never one per
/// tab: a stop is not refused because another request for the tab is still
/// being answered. The server remembers request ids and nothing else; which
/// autopilots exist is only what the tabs report.
///
/// `receipt_wait` is [`AUTOPILOT_RECEIPT_WAIT`]; tests shorten it.
async fn handle_pane_autopilot_control(
    state: &AppState,
    connection_id: &Uuid,
    command: WebToServer,
    receipt_wait: Duration,
) {
    let WebToServer::ControlPaneAutopilot {
        session_id,
        pane_id,
        request_id,
        action,
    } = command
    else {
        return;
    };
    let Some(user_id) = state.sessions.get_web_user(connection_id) else {
        return;
    };
    // Checked first: everything below is answered with a frame that only a
    // connection that negotiated autopilot is ever sent.
    if !state
        .sessions
        .web_supports_capability(connection_id, shared::TAB_AUTOPILOT_CAPABILITY)
    {
        send_policy_error(
            state,
            connection_id,
            "This client did not negotiate autopilot; reload it after updating the web deployment",
        )
        .await;
        return;
    }
    if request_id.is_empty() || request_id.len() > shared::AUTOPILOT_REQUEST_ID_MAX_BYTES {
        send_policy_error(state, connection_id, "Invalid autopilot request identifier").await;
        return;
    }
    // A repeated copy of a request has no further effect; its first copy is
    // answered, or was.
    if !state.sessions.claim_autopilot_request(user_id, &request_id) {
        tracing::debug!(%session_id, pane_id, "ignoring a repeated autopilot request");
        return;
    }
    let receiver = match forward_pane_autopilot_control(
        state,
        connection_id,
        session_id,
        pane_id,
        &request_id,
        &action,
    )
    .await
    {
        Ok(receiver) => receiver,
        Err(reason) => {
            send_autopilot_receipt(
                state,
                connection_id,
                session_id,
                pane_id,
                request_id,
                AutopilotReceipt {
                    outcome: shared::AutopilotReceiptOutcome::Refused,
                    reason: Some(reason),
                },
            )
            .await;
            return;
        }
    };
    let state = state.clone();
    let connection_id = *connection_id;
    tokio::spawn(async move {
        let receipt = match tokio::time::timeout(receipt_wait, receiver).await {
            Ok(Ok(receipt)) => receipt,
            // No receipt in time, or the instance's connection closed first.
            // The request was delivered, so a pause or a stop stays in force
            // and a start may have been taken: not confirmed, never refused.
            _ => {
                state
                    .sessions
                    .drop_abandoned_autopilot_request(session_id, pane_id, &request_id);
                AutopilotReceipt {
                    outcome: shared::AutopilotReceiptOutcome::Uncertain,
                    reason: None,
                }
            }
        };
        send_autopilot_receipt(
            &state,
            &connection_id,
            session_id,
            pane_id,
            request_id,
            receipt,
        )
        .await;
    });
}

/// What a viewer that just attached is told about autopilot: the availability
/// of every OMP terminal tab, with the state its instance last reported and
/// when. Only a viewer that negotiated autopilot is sent anything; older
/// native builds drop their socket on the frame. Each frame reads its state
/// as it is queued (`send_to_web_with`), so a change broadcast meanwhile is
/// never overtaken by an older copy.
async fn replay_pane_autopilot_after_attach(
    state: &AppState,
    connection_id: &Uuid,
    session_id: Uuid,
) {
    if !state
        .sessions
        .web_supports_capability(connection_id, shared::TAB_AUTOPILOT_CAPABILITY)
    {
        return;
    }
    let tabs: Vec<u32> = state
        .sessions
        .pane_autopilot_availabilities(&session_id)
        .into_iter()
        .map(|(pane_id, _)| pane_id)
        .collect();
    for pane_id in tabs {
        state
            .sessions
            .send_to_web_with(connection_id, || {
                state.sessions.pane_autopilot_frame(&session_id, pane_id)
            })
            .await;
    }
}

/// Whether `message` may go out on `connection_id`: the autopilot frames only
/// to a connection that negotiated them. Every sender already checks this;
/// the connection's outbound task checks again, because one such frame on an
/// older native build's socket closes it.
fn autopilot_outbound_allowed(
    state: &AppState,
    connection_id: &Uuid,
    message: &ServerToWeb,
) -> bool {
    !matches!(
        message,
        ServerToWeb::PaneAutopilotState { .. } | ServerToWeb::PaneAutopilotReceipt { .. }
    ) || state
        .sessions
        .web_supports_capability(connection_id, shared::TAB_AUTOPILOT_CAPABILITY)
}

/// Load the latest effective policy for a launch-like mutation. Both ends of
/// the data-plane must understand server-owned policy before a launch is
/// routed; otherwise an older peer could silently apply local `.apas` state.
/// The panes in a session and whether each is working.
///
/// Shared by the web and mobile session lists so "this project is working" and
/// "these agents are working" are answered from one place and cannot disagree.
pub(crate) fn pane_summaries(
    state: &AppState,
    session_id: &Uuid,
    is_active: bool,
) -> Vec<shared::MobilePaneSummary> {
    let working = state.sessions.get_pane_statuses(session_id);
    let idle_since = state.sessions.get_pane_idle_since(session_id);
    state
        .sessions
        .get_session_panes(session_id)
        .into_iter()
        .map(|pane| {
            // Usage limits are per CLI, and each machine's instance runs its
            // own; a pane whose machine is not running has none to report.
            let owner_cli = match state.sessions.pane_owner(session_id, pane.pane_id) {
                PaneOwner::Instance(owner) => Some(owner.cli_id),
                PaneOwner::NotRunning { .. } | PaneOwner::NoInstance => None,
            };
            let status = working
                .iter()
                .find(|(_, id, _)| *id == pane.pane_id)
                .map(|(_, _, status)| status.as_str());
            let awaiting_answer = is_active
                && status.is_some_and(|status| status == shared::PANE_STATUS_PENDING_ANSWER);
            let is_working = is_active && status.is_some_and(shared::pane_status_is_working);
            let usage_provider =
                match pane.provider {
                    shared::Provider::Claude
                        if pane.model.as_deref().is_some_and(|model| {
                            model.to_ascii_lowercase().contains("deepseek")
                        }) || pane.label.as_deref().is_some_and(|label| {
                            label.to_ascii_lowercase().contains("deepseek")
                        }) =>
                    {
                        Some(shared::Provider::Deepseek)
                    }
                    shared::Provider::Claude
                    | shared::Provider::Codex
                    | shared::Provider::Deepseek => Some(pane.provider),
                    _ => None,
                };
            let usage_limited = owner_cli
                .zip(usage_provider)
                .and_then(|(cli_id, provider)| state.sessions.get_usage_limits(&cli_id, provider))
                .and_then(|limits| {
                    limits
                        .usage_limited
                        .filter(|limited| limited.applies_to_model(pane.model.as_deref()))
                });

            // Availability and condition only, never the prompt, and only for
            // OMP terminal tabs. An older client ignores the block.
            let autopilot = state
                .sessions
                .pane_autopilot_availability(session_id, pane.pane_id)
                .map(|availability| {
                    shared::PaneAutopilotSummary::from_state(
                        availability.support,
                        availability.state.as_deref(),
                    )
                });

            shared::MobilePaneSummary {
                autopilot,
                pane_id: pane.pane_id,
                label: pane.label,
                kind: pane.kind,
                mode: Some(pane.mode),
                provider: pane.provider,
                model: pane.model,
                is_working,
                awaiting_answer,
                idle_since: (is_active && !is_working && !awaiting_answer)
                    .then(|| idle_since.get(&pane.pane_id).cloned())
                    .flatten(),
                usage_limited,
                machine_id: pane.machine_id,
                hostname: pane.hostname,
            }
        })
        .collect()
}

/// The project policy a launch on `target` runs under, or why there is none
/// to launch under. `target` is the instance that will run what is launched:
/// its CLI must understand server-owned policy, whatever the other instances'
/// CLIs do. `None` leaves that check out, for an instance that is not running
/// yet. `web_policy_capable` is whether the requesting web client understands
/// server-owned policy, read from its connection when it asked.
async fn launch_policy(
    state: &AppState,
    session_id: &Uuid,
    web_policy_capable: bool,
    target: Option<InstanceTarget>,
) -> Result<shared::EffectiveProjectPolicy, String> {
    if !web_policy_capable {
        return Err("This web client is incompatible with cluster project policy; reload after updating the web deployment".to_string());
    }
    if let Some(target) = target {
        if !instance_supports_capability(
            state,
            session_id,
            target,
            shared::PROJECT_POLICY_CAPABILITY,
        ) {
            return Err("The project host CLI is incompatible with cluster project policy; update and reconnect it before launching panes".to_string());
        }
    }
    let project = match state
        .db
        .get_project_for_session(&session_id.to_string())
        .await
    {
        Ok(Some(project)) => project,
        Ok(None) => return Err("Project policy is unavailable".to_string()),
        Err(err) => {
            tracing::warn!(%err, %session_id, "project policy lookup failed");
            return Err("Could not verify project policy".to_string());
        }
    };
    match state.db.get_effective_project_policy(&project.id).await {
        Ok(policy) if !policy.project_suspended => Ok(policy),
        Ok(_) => Err("This project is suspended; reactivate it before launching panes".to_string()),
        Err(err) => {
            tracing::warn!(%err, project_id = %project.id, "effective project policy lookup failed");
            Err("Could not verify project policy".to_string())
        }
    }
}

/// [`launch_policy`] for a request from `connection_id`, reporting a refusal
/// to it.
async fn effective_policy_for_launch(
    state: &AppState,
    connection_id: &Uuid,
    session_id: &Uuid,
    target: InstanceTarget,
) -> Option<shared::EffectiveProjectPolicy> {
    let web_policy_capable = state
        .sessions
        .web_supports_capability(connection_id, shared::PROJECT_POLICY_CAPABILITY);
    match launch_policy(state, session_id, web_policy_capable, Some(target)).await {
        Ok(policy) => Some(policy),
        Err(message) => {
            send_policy_error(state, connection_id, message).await;
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
    target: InstanceTarget,
) -> Option<shared::EffectiveProjectPolicy> {
    if state
        .sessions
        .web_supports_capability(connection_id, shared::PROJECT_POLICY_CAPABILITY)
        && !instance_supports_capability(
            state,
            session_id,
            target,
            shared::PROJECT_POLICY_CAPABILITY,
        )
    {
        send_policy_error(
            state,
            connection_id,
            "This project CLI is too old to reboot from the web. Reboot the CLI manually on the project host to upgrade it, then try again.",
        )
        .await;
        return None;
    }
    effective_policy_for_launch(state, connection_id, session_id, target).await
}

/// Why the project's policy forbids launching this profile on `target`
/// (see [`launch_policy`]), if it does.
async fn profile_launch_refusal(
    state: &AppState,
    session_id: &Uuid,
    kind: shared::PaneKind,
    provider: shared::Provider,
    model: Option<&str>,
    web_policy_capable: bool,
    target: Option<InstanceTarget>,
) -> Result<(), String> {
    if shared::is_retired_launch(provider, model) {
        // Pi included: "Pi is no longer supported. Use OMP instead."
        return Err(shared::retired_launch_message(provider).to_string());
    }
    let policy = launch_policy(state, session_id, web_policy_capable, target).await?;
    if policy.allows(kind, provider, model) {
        return Ok(());
    }
    Err(format!(
        "Launch profile '{}' is disabled by cluster policy (policy version {})",
        shared::launch_profile_key(kind, provider, model),
        policy.version
    ))
}

#[allow(clippy::too_many_arguments)]
async fn authorize_profile_launch(
    state: &AppState,
    connection_id: &Uuid,
    session_id: &Uuid,
    kind: shared::PaneKind,
    provider: shared::Provider,
    model: Option<&str>,
    managed: bool,
    target: InstanceTarget,
) -> bool {
    // `managed` is vestigial: an older `.apas` may still carry it, and it no
    // longer selects a different kind of pane or a different policy.
    let _ = managed;
    let web_policy_capable = state
        .sessions
        .web_supports_capability(connection_id, shared::PROJECT_POLICY_CAPABILITY);
    match profile_launch_refusal(
        state,
        session_id,
        kind,
        provider,
        model,
        web_policy_capable,
        Some(target),
    )
    .await
    {
        Ok(()) => true,
        Err(message) => {
            send_policy_error(state, connection_id, message).await;
            false
        }
    }
}

/// Authorize creation of a brand-new pane. Structured agent panes are a
/// historical resume/reboot path only; all new work must use a supported
/// terminal profile.
/// A provider-specific terminal capability a project CLI must advertise before
/// it can host that provider's interactive TUI, with the error to show when it
/// does not. Kept in one place so pane creation and provider switching cannot
/// drift apart.
fn required_terminal_capability(
    provider: shared::Provider,
) -> Option<(&'static str, &'static str)> {
    match provider {
        shared::Provider::Opencode => Some((
            shared::OPENCODE_TERMINAL_CAPABILITY,
            "The project CLI must be updated and reconnected before creating an OpenCode terminal pane.",
        )),
        shared::Provider::Omp => Some((
            shared::OMP_TERMINAL_CAPABILITY,
            "The project CLI must be updated and reconnected before creating an OMP terminal pane.",
        )),
        _ => None,
    }
}

/// Why `target`'s CLI cannot host `provider`'s terminal, if it cannot.
fn terminal_capability_refusal(
    state: &AppState,
    session_id: &Uuid,
    provider: shared::Provider,
    target: InstanceTarget,
) -> Result<(), String> {
    let Some((capability, message)) = required_terminal_capability(provider) else {
        return Ok(());
    };
    if instance_supports_capability(state, session_id, target, capability) {
        return Ok(());
    }
    Err(message.to_string())
}

async fn requires_capable_terminal_cli(
    state: &AppState,
    connection_id: &Uuid,
    session_id: &Uuid,
    provider: shared::Provider,
    target: InstanceTarget,
) -> bool {
    match terminal_capability_refusal(state, session_id, provider, target) {
        Ok(()) => true,
        Err(message) => {
            send_policy_error(state, connection_id, message).await;
            false
        }
    }
}

/// Why a brand-new pane of this profile must not be created on `target`, if
/// it must not. `target: None` leaves out what depends on the target
/// instance's CLI, for an instance that is not running yet; the full check
/// runs again once it is.
#[allow(clippy::too_many_arguments)]
async fn new_pane_launch_refusal(
    state: &AppState,
    session_id: &Uuid,
    kind: shared::PaneKind,
    provider: shared::Provider,
    model: Option<&str>,
    web_policy_capable: bool,
    target: Option<InstanceTarget>,
) -> Result<(), String> {
    if kind == shared::PaneKind::Agent {
        return Err(
            "Conversation-only panes are retired. Create a supported terminal pane instead."
                .to_string(),
        );
    }
    if let (shared::PaneKind::Terminal, Some(target)) = (kind, target) {
        terminal_capability_refusal(state, session_id, provider, target)?;
    }
    profile_launch_refusal(
        state,
        session_id,
        kind,
        provider,
        model,
        web_policy_capable,
        target,
    )
    .await
}

/// [`new_pane_launch_refusal`] for a request from `connection_id`, reporting
/// a refusal to it as an `Error`. `AddPane` itself answers a correlated
/// request with its own result instead, so only the launch-policy tests use
/// this form. `target` is the instance that will receive the new pane.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
async fn authorize_new_pane_launch(
    state: &AppState,
    connection_id: &Uuid,
    session_id: &Uuid,
    kind: shared::PaneKind,
    provider: shared::Provider,
    model: Option<&str>,
    managed: bool,
    target: InstanceTarget,
) -> bool {
    // `managed` is vestigial; see `authorize_profile_launch`.
    let _ = managed;
    let web_policy_capable = state
        .sessions
        .web_supports_capability(connection_id, shared::PROJECT_POLICY_CAPABILITY);
    match new_pane_launch_refusal(
        state,
        session_id,
        kind,
        provider,
        model,
        web_policy_capable,
        Some(target),
    )
    .await
    {
        Ok(()) => true,
        Err(message) => {
            send_policy_error(state, connection_id, message).await;
            false
        }
    }
}

/// Authorize bringing an **existing** pane back — resume, reboot, start.
///
/// The launch-profile allowlist deliberately does not apply here. It decides
/// which combinations may be brought into existence; a pane that exists is a
/// decision already taken under whatever policy applied then. Applying it to a
/// relaunch never removed the pane it disapproved of — the pane kept running —
/// it only made that pane impossible to bring back, which bites hardest exactly
/// when someone needs it back.
///
/// What still refuses: a legacy bot and a retired backend, which genuinely
/// cannot run. A pane an older `.apas` marked `managed` is an ordinary pane:
/// team mode is removed, and neither that marker nor the policy's retained
/// `team_available` decides anything here. Such a pane used to need a policy
/// lookup an ordinary pane was spared, so it alone could be refused.
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
    if pane.mode == shared::PaneMode::Deadloop {
        send_policy_error(state, connection_id, RETIRED_BOT_MESSAGE).await;
        return false;
    }
    if shared::is_retired_launch(pane.provider, pane.model.as_deref()) {
        send_policy_error(
            state,
            connection_id,
            shared::retired_launch_message(pane.provider),
        )
        .await;
        return false;
    }
    true
}

/// A brand-new pane as the web asked for it.
#[derive(Debug, Clone)]
struct NewPaneSpec {
    provider: shared::Provider,
    mode: shared::PaneMode,
    kind: shared::PaneKind,
    label: Option<String>,
    prompt: Option<String>,
    model: Option<String>,
    isolated_worktree: bool,
    role: Option<String>,
    goal: Option<String>,
    backstory: Option<String>,
    plan_review_mode: shared::PlanReviewMode,
    managed: bool,
}

impl NewPaneSpec {
    fn pane_config(
        &self,
        pane_id: u32,
        machine_id: Option<Uuid>,
        hostname: Option<String>,
    ) -> shared::PaneConfig {
        shared::PaneConfig {
            pane_id,
            provider: self.provider,
            mode: self.mode.clone(),
            kind: self.kind,
            session_id: Uuid::new_v4(),
            is_paused: false,
            stop_requested: false,
            prompt: self.prompt.clone(),
            min_iteration_interval_minutes: None,
            label: self.label.clone(),
            model: self.model.clone(),
            effort: None,
            worktree_path: None,
            role: self.role.clone(),
            goal: self.goal.clone(),
            backstory: self.backstory.clone(),
            plan_review_mode: self.plan_review_mode,
            manual_mode: false,
            managed: self.managed,
            machine_id,
            hostname,
        }
    }
}

/// One `AddPane` and who sent it. Kept whole so that a pane waiting for its
/// machine's instance to start is checked and created when that instance
/// attaches, even if the requesting connection is gone by then.
#[derive(Debug, Clone)]
struct NewPaneRequest {
    connection_id: Uuid,
    user_id: Uuid,
    /// Whether the requesting client understands server-owned policy, read
    /// from its connection when it asked.
    web_policy_capable: bool,
    session_id: Uuid,
    request_id: Option<String>,
    spec: NewPaneSpec,
}

/// Whether `user_id` can see `machine_id` among their machines: they own it,
/// or their cluster membership covers it. Only such a machine is ever named
/// to them for itself; see [`may_name_machine`].
async fn machine_visible_to(state: &AppState, user_id: &Uuid, machine_id: &Uuid) -> bool {
    match state.sessions.daemon_owner(machine_id) {
        Some(owner) if owner == *user_id => true,
        Some(owner) => state
            .db
            .cluster_member_can_use_machine(
                &owner.to_string(),
                &user_id.to_string(),
                &machine_id.to_string(),
            )
            .await
            .unwrap_or(false),
        None => false,
    }
}

/// Whether a refusal may name `machine_id`'s host to `user_id`: the machine
/// runs an instance of one of `session_ids`, a tab of one is tagged for it,
/// or `user_id` can see it among their machines. Any other machine id a
/// request names is answered as one the server does not know. Machine ids
/// derive from `user@host`, so naming the host of whatever id was asked for
/// would confirm to anyone that another account has a daemon on a guessed
/// host.
async fn may_name_machine(
    state: &AppState,
    session_ids: &[Uuid],
    user_id: &Uuid,
    machine_id: &Uuid,
) -> bool {
    let related = session_ids.iter().any(|session_id| {
        state
            .sessions
            .instance_on_machine(session_id, machine_id)
            .is_some()
            || state
                .sessions
                .get_session_panes(session_id)
                .iter()
                .any(|pane| pane.machine_id == Some(*machine_id))
    });
    related || machine_visible_to(state, user_id, machine_id).await
}

/// A machine's host name for messages, or "That machine".
fn machine_label(state: &AppState, machine_id: Uuid) -> String {
    state
        .sessions
        .machine_hostname(&machine_id)
        .map(|host| host.trim().to_string())
        .filter(|host| !host.is_empty())
        .unwrap_or_else(|| "That machine".to_string())
}

/// Tell the requester how its `AddPane` went. A request that carried an id
/// is answered with `AddPaneResult`, on its own connection only, and the
/// answer is dropped if that connection is gone: older native builds drop
/// their socket on a frame they do not know. A request without an id is
/// told only of a failure, as an `Error`, the way every `AddPane` was.
async fn answer_add_pane(
    state: &AppState,
    request: &NewPaneRequest,
    status: shared::AddPaneStatus,
    machine_id: Option<Uuid>,
    pane_id: Option<u32>,
    error: Option<String>,
) {
    let Some(request_id) = &request.request_id else {
        if let Some(error) = error {
            send_policy_error(state, &request.connection_id, error).await;
        }
        return;
    };
    state
        .sessions
        .send_to_web(
            &request.connection_id,
            ServerToWeb::AddPaneResult {
                session_id: request.session_id,
                request_id: request_id.clone(),
                status,
                machine_id,
                pane_id,
                error,
            },
        )
        .await;
}

/// Answer with a launch's final outcome: delivered with its machine and pane
/// id, or failed on `target_machine` with the reason.
async fn answer_add_pane_launch(
    state: &AppState,
    request: &NewPaneRequest,
    target_machine: Option<Uuid>,
    result: Result<(Option<Uuid>, u32), String>,
) {
    match result {
        Ok((machine_id, pane_id)) => {
            answer_add_pane(
                state,
                request,
                shared::AddPaneStatus::Routed,
                machine_id,
                Some(pane_id),
                None,
            )
            .await
        }
        Err(error) => {
            tracing::info!(
                session_id = %request.session_id,
                machine_id = ?target_machine,
                %error,
                "new pane refused"
            );
            answer_add_pane(
                state,
                request,
                shared::AddPaneStatus::Failed,
                target_machine,
                None,
                Some(error),
            )
            .await
        }
    }
}

/// `AddPane`: create the pane on the instance `machine_id` names, else on
/// the primary instance, the only target an older client knows. A machine
/// where the project is registered but not running is started first and
/// the pane waits for its instance; see [`add_pane_on_machine`].
async fn handle_add_pane(
    state: &AppState,
    connection_id: &Uuid,
    msg_session_id: Option<Uuid>,
    fallback_session_id: Option<Uuid>,
    machine_id: Option<Uuid>,
    request_id: Option<String>,
    spec: NewPaneSpec,
) {
    let resolved =
        resolve_target_session(state, connection_id, msg_session_id, fallback_session_id).await;
    let user_id = state.sessions.get_web_user(connection_id);
    let (Some(session_id), Some(user_id)) = (resolved, user_id) else {
        // `resolve_target_session` has said why where it could; a client that
        // correlates its launches is also told that this one failed.
        if let (Some(request_id), Some(session_id)) =
            (request_id, msg_session_id.or(fallback_session_id))
        {
            state
                .sessions
                .send_to_web(
                    connection_id,
                    ServerToWeb::AddPaneResult {
                        session_id,
                        request_id,
                        status: shared::AddPaneStatus::Failed,
                        machine_id,
                        pane_id: None,
                        error: Some("This project is unavailable or access was denied".to_string()),
                    },
                )
                .await;
        }
        return;
    };
    let request = NewPaneRequest {
        connection_id: *connection_id,
        user_id,
        web_policy_capable: state
            .sessions
            .web_supports_capability(connection_id, shared::PROJECT_POLICY_CAPABILITY),
        session_id,
        request_id,
        spec,
    };
    if request.spec.mode == shared::PaneMode::Deadloop {
        answer_add_pane_launch(state, &request, machine_id, Err(RETIRED_BOT_MESSAGE.into())).await;
        return;
    }
    match machine_id {
        Some(machine_id) => add_pane_on_machine(state, request, machine_id).await,
        None => {
            let primary = state.sessions.primary_instance(&session_id);
            let result =
                launch_new_pane(state, &request, primary.as_ref(), InstanceTarget::Primary).await;
            let target_machine = primary.and_then(|primary| primary.machine_id);
            answer_add_pane_launch(state, &request, target_machine, result).await;
        }
    }
}

/// Check a new pane against `instance`, which `target` names for the CLI
/// capability checks, allocate its id, and send it there. The pane is tagged
/// with the instance's machine when its CLI can share the project; an older
/// CLI reports its panes untagged, and they stay untagged. Returns the
/// machine and pane id it was delivered with.
async fn launch_new_pane(
    state: &AppState,
    request: &NewPaneRequest,
    instance: Option<&InstanceAttachment>,
    target: InstanceTarget,
) -> Result<(Option<Uuid>, u32), String> {
    let session_id = request.session_id;
    if let Some(instance) = instance {
        if let Some(refusal) =
            user_instance_access_refusal(state, &session_id, &request.user_id, instance).await
        {
            return Err(refusal);
        }
    }
    let spec = &request.spec;
    if spec.mode == shared::PaneMode::Deadloop {
        return Err(RETIRED_BOT_MESSAGE.into());
    }
    new_pane_launch_refusal(
        state,
        &session_id,
        spec.kind,
        spec.provider,
        spec.model.as_deref(),
        request.web_policy_capable,
        Some(target),
    )
    .await?;
    let tag = instance
        .filter(|instance| instance.capable)
        .and_then(|instance| instance.machine_id);
    let pane_id = state
        .sessions
        .allocate_pane_id(&session_id, tag)
        .ok_or_else(|| {
            "Could not allocate a pane id because the session is full; close a pane and retry."
                .to_string()
        })?;
    let hostname = tag.and(instance.and_then(|instance| instance.hostname.clone()));
    let pane_config = spec.pane_config(pane_id, tag, hostname);
    // To the instance just checked, even if the primary hands over meanwhile.
    let route_machine = instance.and_then(|instance| instance.machine_id);
    tracing::info!(
        %session_id,
        pane_id,
        machine_id = ?route_machine,
        isolated_worktree = spec.isolated_worktree,
        "Adding pane"
    );
    let outcome = state
        .sessions
        .route_to_cli_on_machine(
            &session_id,
            route_machine,
            ServerToCli::AddPane {
                session_id,
                pane_config,
                isolated_worktree: spec.isolated_worktree,
                initial_input: None,
            },
        )
        .await;
    if outcome.delivered() {
        return Ok((route_machine, pane_id));
    }
    state.sessions.release_pane_id(&session_id, pane_id);
    // A creation the host never receives must not fail silently: without
    // this the click produces no tab, no error, and no log on the requesting
    // client.
    Err(match outcome {
        CliRouteOutcome::MachineNotRunning { .. } => undelivered_message(state, &outcome),
        _ => "The project host is unreachable; the pane was not created. Reconnect and retry."
            .to_string(),
    })
}

/// `AddPane` naming `machine_id`. The instance attached there receives the
/// pane. A machine where the project is registered but not running has its
/// instance started, and the pane follows when that instance attaches. Any
/// other machine is refused with the reason: the pane is never created on a
/// machine the user did not pick.
async fn add_pane_on_machine(state: &AppState, request: NewPaneRequest, machine_id: Uuid) {
    let session_id = request.session_id;
    let wait = if state
        .sessions
        .instance_on_machine(&session_id, &machine_id)
        .is_some()
    {
        // Running: wait only for a start of it the server has not finished,
        // since a pane sent sooner could reach the instance before the
        // project's policy, without which its CLI refuses every new pane.
        state
            .sessions
            .wait_for_instance_start(session_id, machine_id, None)
    } else {
        let target = match start_target(state, &request, machine_id).await {
            Ok(target) => target,
            Err(error) => {
                answer_add_pane_launch(state, &request, Some(machine_id), Err(error)).await;
                return;
            }
        };
        let wait = state.sessions.wait_for_instance_start(
            session_id,
            machine_id,
            Some(&target.project_id),
        );
        if let InstanceStartWait::Waiting {
            start: Some(start_request),
            ..
        } = &wait
        {
            let sent = state
                .sessions
                .send_to_daemon(
                    &machine_id,
                    ServerToDaemon::StartProjectCli {
                        project_id: target.project_id.clone(),
                        policy: Some(target.policy),
                        request_id: Some(*start_request),
                    },
                )
                .await;
            if !sent {
                // Every pane that joined this start fails with it.
                state
                    .sessions
                    .fail_instance_start(session_id, machine_id, *start_request);
                let error = format!("{} is offline", machine_label(state, machine_id));
                answer_add_pane_launch(state, &request, Some(machine_id), Err(error)).await;
                return;
            }
            tracing::info!(
                %session_id,
                %machine_id,
                request_id = %start_request,
                "starting the project's instance for a new pane"
            );
        }
        wait
    };
    match wait {
        InstanceStartWait::Ready => {
            let result = match state.sessions.instance_on_machine(&session_id, &machine_id) {
                Some(instance) => {
                    launch_new_pane(
                        state,
                        &request,
                        Some(&instance),
                        InstanceTarget::Machine(machine_id),
                    )
                    .await
                }
                None => Err(not_running_message(state, Some(machine_id), None)),
            };
            answer_add_pane_launch(state, &request, Some(machine_id), result).await;
        }
        InstanceStartWait::Waiting {
            waiter, outcome, ..
        } => {
            // Answered before the wait begins, so `Starting` always reaches the
            // requester ahead of the final answer.
            answer_add_pane(
                state,
                &request,
                shared::AddPaneStatus::Starting,
                Some(machine_id),
                None,
                None,
            )
            .await;
            let state = state.clone();
            tokio::spawn(async move {
                add_pane_after_start(&state, request, machine_id, waiter, outcome).await;
            });
        }
    }
}

/// The rest of an `AddPane` whose machine's instance is starting: create the
/// pane once it has started, or fail with why it did not.
async fn add_pane_after_start(
    state: &AppState,
    request: NewPaneRequest,
    machine_id: Uuid,
    waiter: Uuid,
    outcome: tokio::sync::oneshot::Receiver<InstanceStartOutcome>,
) {
    let limit = state.sessions.instance_start_timeout();
    let result = match state
        .sessions
        .await_instance_start(waiter, outcome, limit)
        .await
    {
        Some(InstanceStartOutcome::Started) => {
            match state
                .sessions
                .instance_on_machine(&request.session_id, &machine_id)
            {
                Some(instance) => {
                    launch_new_pane(
                        state,
                        &request,
                        Some(&instance),
                        InstanceTarget::Machine(machine_id),
                    )
                    .await
                }
                None => Err(not_running_message(state, Some(machine_id), None)),
            }
        }
        Some(InstanceStartOutcome::Refused(reason)) => Err(reason),
        Some(InstanceStartOutcome::DaemonGone) => Err(format!(
            "{} went offline before this project started there",
            machine_label(state, machine_id)
        )),
        None => Err(format!(
            "{} did not start this project within {} seconds",
            machine_label(state, machine_id),
            limit.as_secs().max(1)
        )),
    };
    answer_add_pane_launch(state, &request, Some(machine_id), result).await;
}

/// What starting a machine's instance for a new pane needs.
struct StartTarget {
    project_id: String,
    policy: shared::EffectiveProjectPolicy,
}

/// Whether `request` may start the project on `machine_id`, which runs no
/// instance of it, to create its pane there, and why not otherwise. These
/// are the checks a per-machine Start makes, plus the pane's launch policy
/// as far as it does not depend on the CLI that will run it. The rest is
/// checked once that instance is attached.
async fn start_target(
    state: &AppState,
    request: &NewPaneRequest,
    machine_id: Uuid,
) -> Result<StartTarget, String> {
    let session_id = request.session_id;
    // A CLI without the capability assumes it is the project's only runtime,
    // so an instance started beside it would be refused when it attached.
    if let Some(older) = state
        .sessions
        .session_attachments(&session_id)
        .into_iter()
        .find(|instance| !instance.capable)
    {
        return Err(
            match older
                .hostname
                .as_deref()
                .map(str::trim)
                .filter(|host| !host.is_empty())
            {
                Some(host) => format!(
                    "This project is running on {host} with an APAS CLI that cannot share it with other machines. Reboot {host} from the Machines page to update it, or add the tab there."
                ),
                None => "This project is running elsewhere with an APAS CLI that cannot share it with other machines; update it, or add the tab there.".to_string(),
            },
        );
    }
    let row = match state.db.get_session(&session_id.to_string()).await {
        Ok(Some(row)) => row,
        Ok(None) => return Err("This project is unavailable".to_string()),
        Err(error) => {
            tracing::warn!(%session_id, %error, "session lookup failed");
            return Err("Could not verify this project".to_string());
        }
    };
    let project_id = row.project_id.clone().unwrap_or_else(|| row.id.clone());
    if !machine_visible_to(state, &request.user_id, &machine_id).await {
        return Err("That machine is not available to you".to_string());
    }
    let owner = state.sessions.daemon_owner(&machine_id);
    let host = machine_label(state, machine_id);
    // Every instance runs under the account the session belongs to, and
    // another account's instance is refused when it starts.
    if owner.map(|owner| owner.to_string()).as_deref() != Some(row.user_id.as_str()) {
        return Err(format!(
            "{host} is in another account's cluster, so it cannot run an instance of this project"
        ));
    }
    if !state.sessions.is_daemon_connected(&machine_id) {
        return Err(format!("{host} is offline"));
    }
    if !state
        .sessions
        .daemon_supports_capability(&machine_id, shared::PROJECT_INSTANCES_CAPABILITY)
    {
        return Err(format!(
            "{host} runs an older APAS that cannot run this project beside other machines. Reboot {host} to update it."
        ));
    }
    // Cloning a checkout onto a machine to join the project is a separate
    // request, which the client makes before asking for the pane again.
    if !state
        .sessions
        .machines_listing_project(&project_id)
        .contains(&machine_id)
    {
        return Err(format!("This project is not registered on {host}."));
    }
    if !has_machine_project_runtime_access(state, &request.user_id, &machine_id, &project_id).await
        || !machine_runtime_access(state, &session_id, &request.user_id, Some(machine_id))
            .await
            .unwrap_or(false)
    {
        return Err(format!(
            "Access denied: you cannot use {host} for this project"
        ));
    }
    let spec = &request.spec;
    if spec.mode == shared::PaneMode::Deadloop {
        return Err(RETIRED_BOT_MESSAGE.into());
    }
    new_pane_launch_refusal(
        state,
        &session_id,
        spec.kind,
        spec.provider,
        spec.model.as_deref(),
        request.web_policy_capable,
        None,
    )
    .await?;
    if !state
        .sessions
        .daemon_supports_capability(&machine_id, shared::PROJECT_POLICY_CAPABILITY)
    {
        return Err(
            "The web client or machine daemon is incompatible with cluster project policy"
                .to_string(),
        );
    }
    let policy = start_policy(state, &project_id).await?;
    Ok(StartTarget { project_id, policy })
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

    fn pane(pane_id: u32, provider: shared::Provider, managed: bool) -> shared::PaneConfig {
        shared::PaneConfig {
            pane_id,
            provider,
            mode: shared::PaneMode::Interactive,
            kind: shared::PaneKind::Terminal,
            session_id: Uuid::new_v4(),
            is_paused: false,
            stop_requested: false,
            prompt: None,
            min_iteration_interval_minutes: None,
            label: None,
            model: None,
            effort: None,
            worktree_path: None,
            role: None,
            goal: None,
            backstory: None,
            plan_review_mode: shared::PlanReviewMode::default(),
            manual_mode: false,
            managed,
            machine_id: None,
            hostname: None,
        }
    }

    #[tokio::test]
    async fn historical_bot_cannot_relaunch_or_be_admitted_as_a_new_pane() {
        let (state, connection, session, mut web_rx, mut cli_rx, _) = policy_state().await;
        let mut historical = pane(7, shared::Provider::Claude, false);
        historical.mode = shared::PaneMode::Deadloop;
        historical.prompt = Some("Original private bot instructions".into());
        state
            .sessions
            .set_session_panes(&session, vec![historical.clone()]);
        assert!(!authorize_existing_pane_launch(&state, &connection, &session, 7).await);
        assert!(
            matches!(web_rx.try_recv(), Ok(ServerToWeb::Error { message }) if message.contains("upgrade_required"))
        );
        let user = state.sessions.get_session(&session).unwrap().user_id;
        let request = NewPaneRequest {
            connection_id: connection,
            user_id: user,
            web_policy_capable: true,
            session_id: session,
            request_id: Some("retired-bot-attempt".into()),
            spec: NewPaneSpec {
                provider: shared::Provider::Claude,
                mode: shared::PaneMode::Deadloop,
                kind: shared::PaneKind::Terminal,
                label: None,
                prompt: historical.prompt.clone(),
                model: None,
                isolated_worktree: false,
                role: None,
                goal: None,
                backstory: None,
                plan_review_mode: shared::PlanReviewMode::default(),
                managed: false,
            },
        };
        let failure = launch_new_pane(&state, &request, None, InstanceTarget::Primary)
            .await
            .unwrap_err();
        assert!(failure.contains("upgrade_required"));
        assert!(
            cli_rx.try_recv().is_err(),
            "retired mutations never reach an old CLI"
        );
        let retained = state.sessions.get_session_panes(&session);
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].prompt, historical.prompt);
        assert_eq!(retained[0].mode, shared::PaneMode::Deadloop);
    }

    #[tokio::test]
    async fn historical_bot_typed_input_is_read_only_without_removing_history() {
        let (state, connection, session, mut web_rx, mut cli_rx, _) = policy_state().await;
        let user = state.sessions.get_session(&session).unwrap().user_id;
        state.sessions.set_web_user(connection, user);
        let mut historical = pane(7, shared::Provider::Claude, false);
        historical.mode = shared::PaneMode::Deadloop;
        historical.prompt = Some("Original prompt".into());
        state.sessions.set_session_panes(&session, vec![historical]);
        handle_web_input(
            &state,
            &connection,
            Some(session),
            Some(session),
            "/loop work".into(),
            None,
            Some(7),
            None,
        )
        .await;
        assert!(
            matches!(web_rx.try_recv(), Ok(ServerToWeb::Error { message }) if message.contains("read-only"))
        );
        assert!(cli_rx.try_recv().is_err());
        assert_eq!(
            state.sessions.get_session_panes(&session)[0]
                .prompt
                .as_deref(),
            Some("Original prompt")
        );
    }

    /// The allowlist decides what may be created. Refusing to bring back a pane
    /// that already exists never removed it — it kept running and simply could
    /// not be recovered once stopped.
    /// `team_available` survives on the wire and in stored policy so older web
    /// and mobile builds keep parsing what they are sent, in the same way
    /// `cluster_role` is retained. It must decide nothing: a policy that still
    /// carries it has to behave exactly like one that does not.
    #[tokio::test]
    async fn the_retained_team_field_decides_nothing() {
        let (state, connection_id, session_id, mut web_rx, _cli_rx, _cli_id) = policy_state().await;
        state
            .sessions
            .set_session_panes(&session_id, vec![pane(7, shared::Provider::Claude, true)]);

        // A pane an older `.apas` marked managed, under a policy that says team
        // mode is unavailable — the combination that used to refuse both.
        assert!(authorize_existing_pane_launch(&state, &connection_id, &session_id, 7).await);
        assert!(
            web_rx.try_recv().is_err(),
            "no refusal is raised on account of the retained field"
        );

        // And creating one is permitted where the allowlist permits the
        // profile. `managed` used to refuse here whenever team mode was
        // unavailable, which was always, so this is the behaviour that changed.
        assert!(
            authorize_new_pane_launch(
                &state,
                &connection_id,
                &session_id,
                shared::PaneKind::Terminal,
                shared::Provider::Claude,
                None,
                true,
                InstanceTarget::Primary,
            )
            .await,
            "only the launch profile decides creation now"
        );
    }

    #[tokio::test]
    async fn an_existing_pane_relaunches_outside_the_allowlist() {
        let (state, connection_id, session_id, mut web_rx, _cli_rx, _cli_id) = policy_state().await;
        state
            .sessions
            .set_session_panes(&session_id, vec![pane(7, shared::Provider::Claude, false)]);

        assert!(authorize_existing_pane_launch(&state, &connection_id, &session_id, 7).await);
        assert!(
            web_rx.try_recv().is_err(),
            "relaunching an existing pane raises no policy error"
        );

        // The same combination is still refused for a *new* pane.
        assert!(
            !authorize_new_pane_launch(
                &state,
                &connection_id,
                &session_id,
                shared::PaneKind::Agent,
                shared::Provider::Claude,
                None,
                false,
                InstanceTarget::Primary,
            )
            .await
        );
    }

    #[tokio::test]
    async fn an_existing_pane_with_a_retired_backend_is_still_refused() {
        let (state, connection_id, session_id, mut web_rx, _cli_rx, _cli_id) = policy_state().await;
        let mut retired = pane(8, shared::Provider::Claude, false);
        retired.model = Some("MiniMax-M2.7".to_string());
        state.sessions.set_session_panes(&session_id, vec![retired]);

        assert!(!authorize_existing_pane_launch(&state, &connection_id, &session_id, 8).await);
        let ServerToWeb::Error { message } = web_rx.try_recv().expect("explicit web error") else {
            panic!("expected policy error")
        };
        assert!(message.contains("retired"), "{message}");
    }

    #[tokio::test]
    async fn a_missing_pane_is_still_refused() {
        let (state, connection_id, session_id, mut web_rx, _cli_rx, _cli_id) = policy_state().await;
        state.sessions.set_session_panes(&session_id, vec![]);

        assert!(!authorize_existing_pane_launch(&state, &connection_id, &session_id, 99).await);
        let ServerToWeb::Error { message } = web_rx.try_recv().expect("explicit web error") else {
            panic!("expected policy error")
        };
        assert!(message.contains("Pane not found"), "{message}");
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
                InstanceTarget::Primary,
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
    async fn supported_terminal_profile_remains_authorized() {
        let (state, connection_id, session_id, mut web_rx, _cli_rx, _cli_id) = policy_state().await;

        assert!(
            authorize_profile_launch(
                &state,
                &connection_id,
                &session_id,
                shared::PaneKind::Terminal,
                shared::Provider::Codex,
                None,
                false,
                InstanceTarget::Primary,
            )
            .await
        );
        assert!(web_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn deepseek_variants_are_authorized_exactly_and_unknown_ids_are_rejected() {
        let (state, connection_id, session_id, mut web_rx, _cli_rx, _cli_id) = policy_state().await;

        // The supported model, and the legacy id that canonicalizes to it.
        for model in [
            shared::DEEPSEEK_FLASH_MODEL,
            shared::DEEPSEEK_LEGACY_FLASH_MODEL,
        ] {
            assert!(
                authorize_profile_launch(
                    &state,
                    &connection_id,
                    &session_id,
                    shared::PaneKind::Terminal,
                    shared::Provider::Claude,
                    Some(model),
                    false,
                    InstanceTarget::Primary,
                )
                .await
            );
        }
        assert!(web_rx.try_recv().is_err());

        // The withdrawn model is refused like any other unsupported id, so a
        // stale client cannot launch a pane on it.
        assert!(
            !authorize_profile_launch(
                &state,
                &connection_id,
                &session_id,
                shared::PaneKind::Terminal,
                shared::Provider::Claude,
                Some(shared::DEEPSEEK_RETIRED_PRO_MODEL),
                false,
                InstanceTarget::Primary,
            )
            .await
        );
        assert!(web_rx.try_recv().is_ok(), "a refusal must be reported");

        assert!(
            !authorize_profile_launch(
                &state,
                &connection_id,
                &session_id,
                shared::PaneKind::Terminal,
                shared::Provider::Claude,
                Some("deepseek-chat"),
                false,
                InstanceTarget::Primary,
            )
            .await
        );
        let ServerToWeb::Error { message } = web_rx.try_recv().expect("explicit web error") else {
            panic!("expected policy error")
        };
        assert!(message.contains("deepseek-chat"), "{message}");
    }

    #[tokio::test]
    async fn new_conversation_only_pane_is_rejected_regardless_of_legacy_managed_flag() {
        let (state, connection_id, session_id, mut web_rx, mut cli_rx, _cli_id) =
            policy_state().await;

        for managed in [false, true] {
            assert!(
                !authorize_new_pane_launch(
                    &state,
                    &connection_id,
                    &session_id,
                    shared::PaneKind::Agent,
                    shared::Provider::Claude,
                    None,
                    managed,
                    InstanceTarget::Primary,
                )
                .await
            );

            let ServerToWeb::Error { message } = web_rx.try_recv().expect("explicit web error")
            else {
                panic!("expected retirement error")
            };
            assert!(message.contains("Conversation-only panes are retired"));
        }
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
                InstanceTarget::Primary,
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
                InstanceTarget::Primary,
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
                InstanceTarget::Primary,
            )
            .await
        );
        assert!(web_rx.try_recv().is_err());
    }

    /// Pi was retired. A request to create a Pi tab is refused with the
    /// sentence that says what to use instead, whatever the CLI advertises:
    /// `terminal_pi_v1` from an older CLI no longer routes anything.
    #[tokio::test]
    #[allow(deprecated)]
    async fn a_new_pi_tab_is_refused_whatever_the_cli_advertises() {
        for capabilities in [
            vec![shared::PROJECT_POLICY_CAPABILITY.to_string()],
            vec![
                shared::PROJECT_POLICY_CAPABILITY.to_string(),
                "terminal_pi_v1".to_string(),
                shared::OMP_TERMINAL_CAPABILITY.to_string(),
            ],
        ] {
            let (state, connection_id, session_id, mut web_rx, mut cli_rx, cli_id) =
                policy_state().await;
            state.sessions.set_cli_capabilities(cli_id, capabilities);

            assert!(
                !authorize_new_pane_launch(
                    &state,
                    &connection_id,
                    &session_id,
                    shared::PaneKind::Terminal,
                    shared::Provider::Pi,
                    None,
                    false,
                    InstanceTarget::Primary,
                )
                .await
            );

            let ServerToWeb::Error { message } = web_rx.try_recv().expect("explicit web error")
            else {
                panic!("expected a refusal")
            };
            assert_eq!(message, "Pi is no longer supported. Use OMP instead.");
            assert!(cli_rx.try_recv().is_err(), "nothing reaches the CLI");
        }
    }

    /// Switching an existing tab to Pi is refused the same way, and so is
    /// bringing a saved Pi tab back.
    #[tokio::test]
    #[allow(deprecated)]
    async fn switching_to_pi_and_relaunching_a_saved_pi_tab_are_refused() {
        let (state, connection_id, session_id, mut web_rx, mut cli_rx, _cli_id) =
            policy_state().await;

        assert!(
            !authorize_profile_launch(
                &state,
                &connection_id,
                &session_id,
                shared::PaneKind::Terminal,
                shared::Provider::Pi,
                None,
                false,
                InstanceTarget::Primary,
            )
            .await
        );
        let ServerToWeb::Error { message } = web_rx.try_recv().expect("explicit web error") else {
            panic!("expected a refusal")
        };
        assert_eq!(message, shared::PI_RETIRED_MESSAGE);

        state
            .sessions
            .set_session_panes(&session_id, vec![pane(9, shared::Provider::Pi, false)]);
        assert!(!authorize_existing_pane_launch(&state, &connection_id, &session_id, 9).await);
        let ServerToWeb::Error { message } = web_rx.try_recv().expect("explicit web error") else {
            panic!("expected a refusal")
        };
        assert_eq!(message, shared::PI_RETIRED_MESSAGE);
        assert!(cli_rx.try_recv().is_err(), "nothing reaches the CLI");

        assert_eq!(
            retired_pi_reboot_refusal(&pane(9, shared::Provider::Pi, false)).as_deref(),
            Some(
                "Tab 9 uses Pi, which is no longer supported. Close that tab, then reboot the CLI."
            )
        );
        assert_eq!(
            retired_pi_reboot_refusal(&pane(9, shared::Provider::Omp, false)),
            None
        );
    }

    /// Retiring Pi must not catch OMP: an OMP tab is still authorized for a
    /// CLI that can host it, and still needs that capability.
    /// A saved Pi tab cannot be switched to another provider: its CLI keeps
    /// it as Pi and refuses. Accepting the frame first stored and broadcast
    /// the tab as OMP, so viewers saw a usable OMP tab that did not exist.
    /// An older bundle or the native app can still send the frame.
    #[tokio::test]
    #[allow(deprecated)]
    async fn switching_a_saved_pi_tab_to_omp_changes_nothing_and_reaches_no_cli() {
        let (state, connection, session, mut web_rx, mut cli_rx, cli_id) = policy_state().await;
        let user = state.sessions.get_session(&session).unwrap().user_id;
        state.sessions.set_web_user(connection, user);
        state.sessions.set_cli_capabilities(
            cli_id,
            vec![
                shared::PROJECT_POLICY_CAPABILITY.to_string(),
                shared::OMP_TERMINAL_CAPABILITY.to_string(),
            ],
        );
        state.sessions.set_session_panes(
            &session,
            vec![
                pane(9, shared::Provider::Pi, false),
                pane(10, shared::Provider::Codex, false),
            ],
        );
        let providers = |state: &AppState| {
            state
                .sessions
                .get_session_panes(&session)
                .into_iter()
                .map(|pane| (pane.pane_id, pane.provider))
                .collect::<Vec<_>>()
        };

        handle_update_pane_model(
            &state,
            &connection,
            Some(session),
            Some(session),
            9,
            Some(shared::Provider::Omp),
            None,
        )
        .await;

        let ServerToWeb::Error { message } = web_rx.try_recv().expect("explicit web error") else {
            panic!("expected a refusal")
        };
        assert_eq!(
            message,
            "Tab 9 uses Pi, which is no longer supported. Close it and open an OMP tab."
        );
        assert!(web_rx.try_recv().is_err(), "no roster is broadcast");
        assert!(cli_rx.try_recv().is_err(), "nothing reaches the CLI");
        assert_eq!(
            providers(&state),
            vec![(9, shared::Provider::Pi), (10, shared::Provider::Codex)]
        );

        // The same frame for a tab that can be switched goes through, so the
        // refusal above is the Pi tab's and not the test's setup.
        handle_update_pane_model(
            &state,
            &connection,
            Some(session),
            Some(session),
            10,
            Some(shared::Provider::Omp),
            None,
        )
        .await;
        assert!(matches!(
            cli_rx.try_recv(),
            Ok(ServerToCli::UpdatePaneModel {
                pane_id: 10,
                provider: Some(shared::Provider::Omp),
                ..
            })
        ));
        assert_eq!(
            providers(&state),
            vec![(9, shared::Provider::Pi), (10, shared::Provider::Omp)]
        );
    }

    /// A saved Pi tab has no process on any CLI. A message typed to it, from
    /// the phone layout or an older bundle, used to be sent on, stored in the
    /// tab's history and shown as a message no agent would ever answer.
    #[tokio::test]
    #[allow(deprecated)]
    async fn a_saved_pi_tab_takes_no_typed_input_and_no_keystrokes() {
        let (state, connection, session, mut web_rx, mut cli_rx, _) = policy_state().await;
        let user = state.sessions.get_session(&session).unwrap().user_id;
        state.sessions.set_web_user(connection, user);
        state.sessions.set_session_panes(
            &session,
            vec![
                pane(9, shared::Provider::Pi, false),
                pane(10, shared::Provider::Omp, false),
            ],
        );
        let refusal = "Pi is no longer supported. Use OMP instead. This tab is read-only.";

        handle_terminal_conversation_input(
            &state,
            &connection,
            Some(session),
            session,
            9,
            "hello pi".into(),
            Some("send-1".into()),
        )
        .await;
        assert!(
            matches!(web_rx.try_recv(), Ok(ServerToWeb::Error { message }) if message == refusal)
        );

        handle_web_input(
            &state,
            &connection,
            Some(session),
            Some(session),
            "hello pi".into(),
            None,
            Some(9),
            Some("send-2".into()),
        )
        .await;
        assert!(
            matches!(web_rx.try_recv(), Ok(ServerToWeb::Error { message }) if message == refusal)
        );

        assert!(web_rx.try_recv().is_err(), "nothing is echoed as sent");
        assert!(cli_rx.try_recv().is_err(), "nothing reaches the CLI");
        assert!(
            state
                .storage
                .get_messages(&session)
                .await
                .unwrap_or_default()
                .is_empty(),
            "nothing is added to the tab's history"
        );
        assert_eq!(state.sessions.seen_input_id(&session, "send-1"), None);
        assert_eq!(state.sessions.seen_input_id(&session, "send-2"), None);

        // Raw keystrokes are dropped without an error for every key.
        assert!(!terminal_accepts_keystrokes(&state, &session, 9));
        assert!(terminal_accepts_keystrokes(&state, &session, 10));
        // A tab the roster does not list is left to routing, as before.
        assert!(terminal_accepts_keystrokes(&state, &session, 11));

        assert_eq!(
            read_only_pane_message(&pane(10, shared::Provider::Omp, false)),
            None
        );
        assert_eq!(
            retired_pane_switch_refusal(&pane(10, shared::Provider::Omp, false)),
            None
        );
    }

    #[tokio::test]
    async fn omp_terminal_is_still_authorized_for_a_capable_cli() {
        let (state, connection_id, session_id, mut web_rx, _cli_rx, cli_id) = policy_state().await;

        assert!(
            !authorize_new_pane_launch(
                &state,
                &connection_id,
                &session_id,
                shared::PaneKind::Terminal,
                shared::Provider::Omp,
                None,
                false,
                InstanceTarget::Primary,
            )
            .await
        );
        let ServerToWeb::Error { message } = web_rx.try_recv().expect("explicit web error") else {
            panic!("expected compatibility error")
        };
        assert!(message.contains("OMP terminal"));

        state.sessions.set_cli_capabilities(
            cli_id,
            vec![
                shared::PROJECT_POLICY_CAPABILITY.to_string(),
                shared::OMP_TERMINAL_CAPABILITY.to_string(),
            ],
        );
        assert!(
            authorize_new_pane_launch(
                &state,
                &connection_id,
                &session_id,
                shared::PaneKind::Terminal,
                shared::Provider::Omp,
                None,
                false,
                InstanceTarget::Primary,
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

        assert!(effective_policy_for_cli_reboot(
            &state,
            &connection_id,
            &session_id,
            InstanceTarget::Primary,
        )
        .await
        .is_none());

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

    async fn omp_question_state() -> (
        AppState,
        Uuid,
        Uuid,
        mpsc::Receiver<ServerToWeb>,
        mpsc::Receiver<ServerToCli>,
        Uuid,
    ) {
        let (state, connection, session, web, cli, cli_id) = policy_state().await;
        let user = state.sessions.get_session(&session).unwrap().user_id;
        state.sessions.set_web_user(connection, user);
        state
            .sessions
            .set_cli_capabilities(cli_id, vec![shared::OMP_QUESTION_CAPABILITY.into()]);
        state
            .sessions
            .set_session_panes(&session, vec![pane(7, shared::Provider::Omp, false)]);
        state.sessions.register_pending_decision(
            session,
            "omp:7:native:question".into(),
            Some(7),
            shared::MutationKind::Question,
        );
        state.sessions.set_pane_status(
            &session,
            shared::PaneType::Interactive,
            7,
            Some(shared::PANE_STATUS_PENDING_ANSWER.into()),
        );
        (state, connection, session, web, cli, cli_id)
    }

    fn omp_answer(session_id: Uuid, pane_id: u32, request_id: &str) -> WebToServer {
        WebToServer::AnswerOmpQuestion {
            session_id,
            pane_id,
            tool_use_id: "omp:7:native:question".into(),
            dialog_id: Uuid::new_v4(),
            request_id: request_id.into(),
            answers: vec![shared::OmpQuestionAnswer {
                id: "choice".into(),
                selected_options: vec!["A, B".into()],
                custom_input: None,
            }],
            cancel: false,
        }
    }

    fn omp_ack(web: &mut mpsc::Receiver<ServerToWeb>) -> ServerToWeb {
        while let Ok(message) = web.try_recv() {
            if matches!(message, ServerToWeb::MutationAck { .. }) {
                return message;
            }
        }
        panic!("expected correlated OMP acknowledgement")
    }

    #[tokio::test]
    async fn omp_answer_requires_live_runtime_access_and_the_exact_pane() {
        let (state, connection, session, mut web, mut cli, _) = omp_question_state().await;
        handle_omp_question_answer(&state, &connection, omp_answer(session, 8, "wrong-pane")).await;
        assert!(matches!(
            omp_ack(&mut web),
            ServerToWeb::MutationAck {
                accepted: false,
                ..
            }
        ));
        assert!(cli.try_recv().is_err());

        state.sessions.set_web_user(connection, Uuid::new_v4());
        handle_omp_question_answer(&state, &connection, omp_answer(session, 7, "no-access")).await;
        assert!(matches!(
            omp_ack(&mut web),
            ServerToWeb::MutationAck {
                accepted: false,
                ..
            }
        ));
        assert!(cli.try_recv().is_err());
        assert!(
            state
                .sessions
                .claim_pending_decision(
                    session,
                    "omp:7:native:question",
                    Some(7),
                    shared::MutationKind::Question,
                )
                .is_some(),
            "denied requests must not consume the genuine question"
        );
    }

    #[tokio::test]
    async fn omp_answer_never_reaches_a_cli_without_native_answer_capability() {
        let (state, connection, session, mut web, mut cli, cli_id) = omp_question_state().await;
        state
            .sessions
            .set_cli_capabilities(cli_id, vec![shared::OMP_TERMINAL_CAPABILITY.into()]);
        handle_omp_question_answer(&state, &connection, omp_answer(session, 7, "old-cli")).await;
        assert!(matches!(
            omp_ack(&mut web),
            ServerToWeb::MutationAck {
                accepted: false,
                ..
            }
        ));
        assert!(cli.try_recv().is_err());
        assert!(state
            .sessions
            .claim_pending_decision(
                session,
                "omp:7:native:question",
                Some(7),
                shared::MutationKind::Question,
            )
            .is_some());
    }

    #[tokio::test]
    async fn omp_rejection_waits_for_native_receipt_and_preserves_explicit_retry() {
        let (state, connection, session, mut web, mut cli, cli_id) = omp_question_state().await;
        tokio::join!(
            handle_omp_question_answer(
                &state,
                &connection,
                omp_answer(session, 7, "invalid-option")
            ),
            async {
                let message = cli.recv().await.expect("native answer request");
                assert!(matches!(message, ServerToCli::AnswerOmpQuestion { .. }));
                while let Ok(message) = web.try_recv() {
                    assert!(
                        !matches!(message, ServerToWeb::MutationAck { .. }),
                        "routing a request is not native acceptance"
                    );
                }
                assert!(state.sessions.finish_omp_answer(
                    session,
                    7,
                    cli_id,
                    "omp:7:native:question",
                    "invalid-option",
                    crate::session::OmpAnswerReceipt {
                        accepted: false,
                        pending: true,
                        error: Some("Unknown option".into()),
                    },
                ));
            },
        );
        assert!(matches!(
            omp_ack(&mut web),
            ServerToWeb::MutationAck {
                accepted: false,
                question_pending: Some(true),
                ..
            }
        ));
        handle_omp_question_answer(
            &state,
            &connection,
            omp_answer(session, 7, "invalid-option"),
        )
        .await;
        assert!(matches!(
            omp_ack(&mut web),
            ServerToWeb::MutationAck {
                accepted: false,
                ..
            }
        ));
        assert!(
            cli.try_recv().is_err(),
            "a repeated request must not be redelivered"
        );
        assert!(
            state
                .sessions
                .claim_pending_decision(
                    session,
                    "omp:7:native:question",
                    Some(7),
                    shared::MutationKind::Question,
                )
                .is_some(),
            "a corrected new request remains possible"
        );
    }
}

/// Whether `user_id` may use the session's runtime on `machine_id`. Every
/// instance of a session runs under the account its row names (`SessionStart`
/// refuses any other), so only the machine differs between instances: on a
/// third-party host the viewer's cluster membership may cover some of its
/// machines and not others.
async fn machine_runtime_access(
    state: &AppState,
    session_id: &Uuid,
    user_id: &Uuid,
    machine_id: Option<Uuid>,
) -> anyhow::Result<bool> {
    let machine_id = machine_id.map(|machine_id| machine_id.to_string());
    state
        .db
        .check_session_runtime_access(
            &session_id.to_string(),
            &user_id.to_string(),
            machine_id.as_deref(),
        )
        .await
}

/// The machine an instance's runtime access is evaluated against. An instance
/// that reported no machine is the session's only one, and the legacy
/// inference behind `session_machine_id` names its machine, as it always has.
fn instance_access_machine(
    state: &AppState,
    session_id: &Uuid,
    instance: &InstanceAttachment,
) -> Option<Uuid> {
    instance
        .machine_id
        .or_else(|| state.sessions.session_machine_id(session_id))
}

async fn instance_runtime_access(
    state: &AppState,
    session_id: &Uuid,
    user_id: &Uuid,
    instance: &InstanceAttachment,
) -> bool {
    let machine_id = instance_access_machine(state, session_id, instance);
    machine_runtime_access(state, session_id, user_id, machine_id)
        .await
        .unwrap_or(false)
}

/// Whether `user_id` may use the session's runtime at all: attach to it, and
/// send it what concerns no one instance. With instances attached, at least
/// one must run on a machine that admits the user; a control for one pane is
/// then also checked against that pane's machine (`pane_control_refusal`).
/// With none attached the machine the session last ran on decides, which is
/// the rule a single instance always had.
async fn has_session_runtime_access(state: &AppState, session_id: &Uuid, user_id: &Uuid) -> bool {
    let attachments = state.sessions.session_attachments(session_id);
    if attachments.is_empty() {
        let machine_id = state.sessions.session_machine_id(session_id);
        return machine_runtime_access(state, session_id, user_id, machine_id)
            .await
            .unwrap_or(false);
    }
    for instance in &attachments {
        if instance_runtime_access(state, session_id, user_id, instance).await {
            return true;
        }
    }
    false
}

/// The refusal for a control aimed at a machine that runs no instance of the
/// project. The tab's state lives on that machine, so nothing else can act on
/// it. Clients show this same wording for such tabs.
fn not_running_message(
    state: &AppState,
    machine_id: Option<Uuid>,
    hostname: Option<String>,
) -> String {
    let host = hostname
        .map(|host| host.trim().to_string())
        .filter(|host| !host.is_empty())
        .or_else(|| {
            machine_id
                .and_then(|machine_id| state.sessions.machine_hostname(&machine_id))
                .map(|host| host.trim().to_string())
                .filter(|host| !host.is_empty())
        });
    match host {
        Some(host) => format!("{host} is not running this project. Start it on {host} first."),
        None => "This tab's machine is not running this project.".to_string(),
    }
}

/// What a client is told when a control that passed its checks was still
/// not delivered: its machine stopped in the meantime, or the instance's
/// connection did not take it.
fn undelivered_message(state: &AppState, outcome: &CliRouteOutcome) -> String {
    match outcome {
        CliRouteOutcome::MachineNotRunning {
            machine_id,
            hostname,
        } => not_running_message(state, *machine_id, hostname.clone()),
        _ => "The project runtime is unavailable".to_string(),
    }
}

/// Why this viewer may not use `instance`, or `None` when they may. Always
/// evaluated against this instance's own machine: the session's check passes
/// when any instance admits the viewer, and the one that did may have
/// detached since, leaving only instances that do not.
async fn instance_access_refusal(
    state: &AppState,
    connection_id: &Uuid,
    session_id: &Uuid,
    instance: &InstanceAttachment,
) -> Option<String> {
    let Some(user_id) = state.sessions.get_web_user(connection_id) else {
        return Some("Access denied".to_string());
    };
    user_instance_access_refusal(state, session_id, &user_id, instance).await
}

/// [`instance_access_refusal`] for `user_id`, whose connection may be gone:
/// a pane waiting for its machine's instance is checked again when it
/// attaches.
async fn user_instance_access_refusal(
    state: &AppState,
    session_id: &Uuid,
    user_id: &Uuid,
    instance: &InstanceAttachment,
) -> Option<String> {
    if instance_runtime_access(state, session_id, user_id, instance).await {
        return None;
    }
    let host = instance
        .hostname
        .as_deref()
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .unwrap_or("that machine");
    Some(format!(
        "Access denied: you cannot use {host}, which runs this project instance"
    ))
}

/// Why a control for `pane_id` must not be sent, or `None` when it may go to
/// the pane's owner. Call it after `resolve_target_session`.
///
/// A pane's state lives on the instance that owns it, so a pane whose machine
/// runs no instance is refused by name rather than sent to another machine.
/// And passing the session's check only shows that some instance admits the
/// viewer, so the owner's machine is checked in its own right.
async fn pane_control_refusal(
    state: &AppState,
    connection_id: &Uuid,
    session_id: &Uuid,
    pane_id: u32,
) -> Option<String> {
    match state.sessions.pane_owner(session_id, pane_id) {
        PaneOwner::Instance(owner) => {
            instance_access_refusal(state, connection_id, session_id, &owner).await
        }
        PaneOwner::NotRunning {
            machine_id,
            hostname,
        } => Some(not_running_message(state, machine_id, hostname)),
        // Nothing is attached: routing reports that, as it always has.
        PaneOwner::NoInstance => None,
    }
}

/// [`resolve_target_session`] for a control aimed at one pane. Also refuses,
/// with the reason, a pane whose machine is not running the project or that
/// this viewer may not use; see [`pane_control_refusal`].
async fn resolve_pane_session(
    state: &AppState,
    connection_id: &Uuid,
    msg_session_id: Option<Uuid>,
    fallback: Option<Uuid>,
    pane_id: u32,
) -> Option<Uuid> {
    let session_id = resolve_target_session(state, connection_id, msg_session_id, fallback).await?;
    if let Some(refusal) = pane_control_refusal(state, connection_id, &session_id, pane_id).await {
        send_policy_error(state, connection_id, refusal).await;
        return None;
    }
    Some(session_id)
}

/// Which of a session's instances a control is evaluated against: what its
/// CLI supports decides whether the control can be sent at all, and the
/// other instances' CLIs may be older or newer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstanceTarget {
    /// The primary instance, which receives what names no machine.
    Primary,
    /// The instance that owns this pane.
    Pane(u32),
    /// The instance on this machine.
    Machine(Uuid),
}

fn target_instance(
    state: &AppState,
    session_id: &Uuid,
    target: InstanceTarget,
) -> Option<InstanceAttachment> {
    match target {
        InstanceTarget::Primary => state.sessions.primary_instance(session_id),
        InstanceTarget::Pane(pane_id) => match state.sessions.pane_owner(session_id, pane_id) {
            PaneOwner::Instance(owner) => Some(owner),
            PaneOwner::NotRunning { .. } | PaneOwner::NoInstance => None,
        },
        InstanceTarget::Machine(machine_id) => {
            state.sessions.instance_on_machine(session_id, &machine_id)
        }
    }
}

fn instance_supports_capability(
    state: &AppState,
    session_id: &Uuid,
    target: InstanceTarget,
    capability: &str,
) -> bool {
    target_instance(state, session_id, target).is_some_and(|instance| {
        state
            .sessions
            .cli_supports_capability(&instance.cli_id, capability)
    })
}

/// The panes `instance` runs, by the same ownership rules that route their
/// controls. With no instance, every pane, as for a session with one CLI.
fn instance_panes(
    state: &AppState,
    session_id: &Uuid,
    instance: Option<&InstanceAttachment>,
) -> Vec<shared::PaneConfig> {
    let panes = state.sessions.get_session_panes(session_id);
    let Some(instance) = instance else {
        return panes;
    };
    panes
        .into_iter()
        .filter(|pane| {
            matches!(
                state.sessions.pane_owner(session_id, pane.pane_id),
                PaneOwner::Instance(owner) if owner.cli_id == instance.cli_id
            )
        })
        .collect()
}

/// Fill in, for one viewer, whether they may use each instance's runtime.
/// An instance whose access cannot be evaluated is left `None`, which clients
/// treat as usable and leave to the server to decide.
async fn fill_runtime_access(
    state: &AppState,
    session_id: &Uuid,
    user_id: &Uuid,
    instances: &mut [shared::SessionInstanceInfo],
) {
    for instance in instances
        .iter_mut()
        .filter(|instance| instance.runtime_access.is_none())
    {
        let machine_id = instance
            .machine_id
            .or_else(|| state.sessions.session_machine_id(session_id));
        match machine_runtime_access(state, session_id, user_id, machine_id).await {
            Ok(allowed) => instance.runtime_access = Some(allowed),
            Err(error) => {
                tracing::warn!(%session_id, %error, "could not evaluate instance runtime access")
            }
        }
    }
}

/// The session's attached instances as `user_id` sees them.
pub(crate) async fn viewer_session_instances(
    state: &AppState,
    session_id: &Uuid,
    user_id: &Uuid,
) -> Vec<shared::SessionInstanceInfo> {
    let mut instances = state.sessions.session_instances(session_id);
    fill_runtime_access(state, session_id, user_id, &mut instances).await;
    instances
}

/// Fill `runtime_access` into an instance set on its way to one viewer. The
/// session manager sends the set on every attach, detach and primary
/// handover without it, because whether a viewer may use an instance depends
/// on who is viewing and needs the database. Everything else passes through.
async fn with_viewer_runtime_access(
    state: &AppState,
    connection_id: &Uuid,
    message: ServerToWeb,
) -> ServerToWeb {
    let ServerToWeb::SessionInstances {
        session_id,
        mut instances,
    } = message
    else {
        return message;
    };
    if let Some(user_id) = state.sessions.get_web_user(connection_id) {
        fill_runtime_access(state, &session_id, &user_id, &mut instances).await;
    }
    ServerToWeb::SessionInstances {
        session_id,
        instances,
    }
}

/// What a viewer that just attached is told about the session's instances,
/// right after `SessionAttached`.
///
/// One that advertised project instances gets the complete instance set and
/// one lifecycle inventory per attached instance, each naming its machine.
/// Any other viewer keeps a single inventory per session, the primary's: a
/// second would replace it, and a reboot confirmation would then count
/// another machine's tabs. Older native builds would also drop their socket
/// on the instance-set frame, which is why it is gated at all.
///
/// The viewer already receives every change broadcast, so each frame here
/// reads its state only as it is queued, with nothing awaited in between
/// (`send_to_web_with`). Read earlier, a set or inventory that changed
/// meanwhile would be broadcast first and this older copy would arrive after
/// it, as the viewer's last word: a running machine's tabs shown stopped
/// until the next change. The instance set is queued without `runtime_access`
/// for that reason, as broadcasts are; this connection's outbound task fills
/// it in for the viewer (`with_viewer_runtime_access`).
async fn replay_instances_after_attach(state: &AppState, connection_id: &Uuid, session_id: Uuid) {
    if !state
        .sessions
        .web_supports_capability(connection_id, shared::PROJECT_INSTANCES_CAPABILITY)
    {
        // Only the primary's, and none until it has reported one, as the live
        // relay gives such a viewer: the first instance to report is not
        // always the primary after a restart. With nothing attached, the one
        // last reported, as before.
        if let Some((machine_id, inventory)) =
            state.sessions.primary_lifecycle_inventory(&session_id)
        {
            state
                .sessions
                .send_to_web(
                    connection_id,
                    ServerToWeb::CliLifecycleInventory {
                        session_id,
                        inventory,
                        machine_id,
                    },
                )
                .await;
        }
        return;
    }
    state
        .sessions
        .send_to_web_with(connection_id, || {
            Some(ServerToWeb::SessionInstances {
                session_id,
                instances: state.sessions.session_instances(&session_id),
            })
        })
        .await;
    let machines: Vec<Option<Uuid>> = state
        .sessions
        .attached_lifecycle_inventories(&session_id)
        .into_iter()
        .map(|(machine_id, _)| machine_id)
        .collect();
    for machine_id in machines {
        // That machine's inventory as it stands now: a newer one relayed
        // since the list was read would otherwise be overtaken by this copy.
        // Nothing for one whose instance has detached meanwhile.
        state
            .sessions
            .send_to_web_with(connection_id, || {
                state
                    .sessions
                    .attached_lifecycle_inventories(&session_id)
                    .into_iter()
                    .find(|(machine, _)| *machine == machine_id)
                    .map(
                        |(machine_id, inventory)| ServerToWeb::CliLifecycleInventory {
                            session_id,
                            inventory,
                            machine_id,
                        },
                    )
            })
            .await;
    }
}

/// `ForgetMachinePanes`: drop the tabs of a machine that will not come back.
/// Tabs never move between machines, and a stopped machine's tabs cannot be
/// closed one by one because their state lives there, so this is the only
/// way to clear them. It edits the project's roster for everyone, so it
/// takes the authority project settings do.
///
/// Only the server roster changes: it loses the machine's tagged tabs and the
/// untagged ones attributed to its host, and their ids are remembered for
/// that machine rather than freed. No instance is told. One sharing this
/// project's `.apas` would delete the very entries that machine restores
/// from, so its tabs could never come back as the confirmation promises.
/// Left alone, they come back when that machine runs the project again and
/// reports them, and untagged ones when an instance adopts them.
async fn forget_machine_panes(
    state: &AppState,
    connection_id: &Uuid,
    session_id: Uuid,
    machine_id: Uuid,
) {
    if !can_manage_project_settings(state, connection_id, &session_id).await {
        send_policy_error(
            state,
            connection_id,
            "Only the project owner or the operator of a cluster hosting it can forget a machine's tabs",
        )
        .await;
        return;
    }
    let Ok((_project_id, _project_guard)) = state
        .active_session_operation(&session_id.to_string())
        .await
    else {
        return;
    };
    // After a server restart a stopped machine's tabs may exist only in
    // `panes.json`; the roster must be loaded before anything is removed.
    let roster = resolve_pane_roster(state, &session_id)
        .await
        .unwrap_or_default();
    // A daemon that has not connected since a restart has no hostname here,
    // but the tabs it reported carry it.
    let hostname = state.sessions.machine_hostname(&machine_id).or_else(|| {
        roster
            .iter()
            .find(|pane| pane.machine_id == Some(machine_id))
            .and_then(|pane| pane.hostname.clone())
    });
    let removed =
        match state
            .sessions
            .forget_machine_panes(&session_id, machine_id, hostname.as_deref())
        {
            Ok(removed) => removed,
            Err(refused) => {
                let host = refused
                    .hostname
                    .or(hostname)
                    .map(|host| host.trim().to_string())
                    .filter(|host| !host.is_empty())
                    .unwrap_or_else(|| "That machine".to_string());
                send_policy_error(
                    state,
                    connection_id,
                    format!(
                        "{host} is running this project. Stop it there before forgetting its tabs."
                    ),
                )
                .await;
                return;
            }
        };
    if !removed.is_empty() {
        // The forgotten ids are saved with the roster: they stay unavailable
        // to new panes, across restarts too, until the machine reports them.
        crate::routes::ws_cli::persist_pane_roster(state, &session_id).await;
        state
            .sessions
            .route_to_web(
                &session_id,
                ServerToWeb::PaneList {
                    session_id,
                    panes: state.sessions.get_session_panes(&session_id),
                },
            )
            .await;
    }
    tracing::info!(
        %session_id,
        %machine_id,
        host = hostname.as_deref().unwrap_or("unknown"),
        removed = ?removed,
        "forgot a machine's tabs"
    );
}

async fn reject_session_attachment(
    state: &AppState,
    connection_id: &Uuid,
    session_id: Uuid,
    reason: SessionAttachmentRejectionReason,
    message: &str,
) {
    state
        .sessions
        .send_to_web(
            connection_id,
            ServerToWeb::SessionAttachmentRejected {
                session_id,
                reason,
                message: message.to_string(),
            },
        )
        .await;
}

/// Why a control names no session it can act on: what its sender is told, or
/// `None` where it has always been told nothing.
type SessionRefusal = Option<&'static str>;

/// The session a control from `connection_id` acts on, or why there is none.
/// Nothing is sent here: [`resolve_target_session`] reports a refusal as an
/// `Error`, and an autopilot request reports it in its receipt.
async fn target_session(
    state: &AppState,
    connection_id: &Uuid,
    msg_session_id: Option<Uuid>,
    fallback: Option<Uuid>,
) -> Result<Uuid, SessionRefusal> {
    if let Some(sid) = msg_session_id {
        if state
            .sessions
            .is_web_attached_to_session(&sid, connection_id)
        {
            let still_authorized = match state.sessions.get_web_user(connection_id) {
                Some(user_id) => has_session_runtime_access(state, &sid, &user_id).await,
                None => false,
            };
            if !still_authorized {
                return Err(Some("Access denied"));
            }
            if state
                .active_session_operation(&sid.to_string())
                .await
                .is_ok()
            {
                return Ok(sid);
            }
            return Err(Some("Project is unavailable"));
        }
        // Not registered as attached — but the web client may legitimately
        // own/have access to this session and simply hasn't finished its
        // (re)attach handshake yet. This is the post-reconnect race that
        // dropped `pause_pane` and made "Stop team" fail to stop workers: the
        // user clicks before the current session's attach lands. Verify access
        // with the same gate AttachSession uses, then auto-attach so the
        // control message isn't lost.
        if let Some(uid) = state.sessions.get_web_user(connection_id) {
            let has_access = has_session_runtime_access(state, &sid, &uid).await;
            if has_access {
                if state
                    .active_session_operation(&sid.to_string())
                    .await
                    .is_err()
                {
                    return Err(None);
                }
                // The session's attached instances, not its database row,
                // say which CLIs run it.
                state
                    .sessions
                    .attach_web_to_session(&sid, *connection_id, None);
                tracing::info!(
                    "Auto-attached web connection {} to session {} for control message (access verified)",
                    connection_id,
                    sid
                );
                return Ok(sid);
            }
        }
        tracing::warn!(
            "Web connection {} sent a message for session {} it is not attached to",
            connection_id,
            sid
        );
        return Err(Some("Not attached to that session"));
    }
    if let Some(sid) = fallback {
        let still_authorized = match state.sessions.get_web_user(connection_id) {
            Some(user_id) => has_session_runtime_access(state, &sid, &user_id).await,
            None => false,
        };
        if !still_authorized {
            return Err(Some("Access denied"));
        }
        if state
            .active_session_operation(&sid.to_string())
            .await
            .is_ok()
        {
            return Ok(sid);
        }
        return Err(Some("Project is unavailable"));
    }
    Err(Some("No session attached"))
}

async fn resolve_target_session(
    state: &AppState,
    connection_id: &Uuid,
    msg_session_id: Option<Uuid>,
    fallback: Option<Uuid>,
) -> Option<Uuid> {
    match target_session(state, connection_id, msg_session_id, fallback).await {
        Ok(session_id) => Some(session_id),
        Err(refusal) => {
            if let Some(message) = refusal {
                state
                    .sessions
                    .send_to_web(
                        connection_id,
                        ServerToWeb::Error {
                            message: message.to_string(),
                        },
                    )
                    .await;
            }
            None
        }
    }
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

/// The instance a reboot or lifecycle request is for: the one on `machine_id`,
/// else the primary. `Err` carries the refusal for a named machine that runs
/// no instance; the request must not land on another machine instead.
///
/// The refusal names the machine's host only when [`may_name_machine`]
/// allows it, by the host its tab carries when it has one. Any other machine
/// is refused as one the server does not know: the request only proves
/// access to the session, not to the machine it names.
async fn lifecycle_target(
    state: &AppState,
    connection_id: &Uuid,
    session_id: &Uuid,
    machine_id: Option<Uuid>,
) -> Result<Option<InstanceAttachment>, String> {
    let Some(machine_id) = machine_id else {
        return Ok(state.sessions.primary_instance(session_id));
    };
    if let Some(instance) = state.sessions.instance_on_machine(session_id, &machine_id) {
        return Ok(Some(instance));
    }
    let nameable = match state.sessions.get_web_user(connection_id) {
        Some(user_id) => may_name_machine(state, &[*session_id], &user_id, &machine_id).await,
        None => false,
    };
    // The host a tab of that machine carries survives a restart, which
    // forgets machines until their daemons connect again.
    let tab_host = nameable
        .then(|| {
            state
                .sessions
                .get_session_panes(session_id)
                .into_iter()
                .find(|pane| pane.machine_id == Some(machine_id))
                .and_then(|pane| pane.hostname)
        })
        .flatten();
    Err(not_running_message(
        state,
        nameable.then_some(machine_id),
        tab_host,
    ))
}

/// The first pane of `instance` whose retired backend cannot run at all, and
/// so blocks rebooting that instance. Other machines' panes are not
/// relaunched by it and do not block it.
fn retired_instance_pane(
    state: &AppState,
    session_id: &Uuid,
    instance: Option<&InstanceAttachment>,
) -> Option<shared::PaneConfig> {
    instance_panes(state, session_id, instance)
        .into_iter()
        .find(|pane| shared::is_retired_launch(pane.provider, pane.model.as_deref()))
}

/// The reboot refusal for a saved Pi tab, which says what to do about it.
/// A Pi tab blocks a project's CLI reboot like any other retired pane. The
/// Machines page reboot is not blocked, and neither path leaves a Pi process
/// an older CLI was hosting running for long: the updated CLI keeps no pane
/// host for a retired tab and ends it when it starts the project.
#[allow(deprecated)]
fn retired_pi_reboot_refusal(pane: &shared::PaneConfig) -> Option<String> {
    (pane.provider == shared::Provider::Pi).then(|| {
        format!(
            "Tab {} uses Pi, which is no longer supported. Close that tab, then reboot the CLI.",
            pane.pane_id
        )
    })
}

/// `RebootCli` for a session this connection may use: reboot the instance on
/// `machine_id`, else the primary. Projects run inside their machine's
/// daemon, so this replaces that host's daemon and every project in it and
/// leaves the other machines' instances running.
async fn handle_reboot_cli(
    state: &AppState,
    connection_id: &Uuid,
    session_id: Uuid,
    machine_id: Option<Uuid>,
) {
    let target = match lifecycle_target(state, connection_id, &session_id, machine_id).await {
        Ok(target) => target,
        Err(refusal) => {
            send_policy_error(state, connection_id, refusal).await;
            return;
        }
    };
    if let Some(instance) = &target {
        if let Some(refusal) =
            instance_access_refusal(state, connection_id, &session_id, instance).await
        {
            send_policy_error(state, connection_id, refusal).await;
            return;
        }
    }
    let capability_target = machine_id
        .map(InstanceTarget::Machine)
        .unwrap_or(InstanceTarget::Primary);
    // Called for its own gate — it refuses a reboot from a web client when
    // the project CLI is too old for it — not for the policy itself, which no
    // longer decides anything here.
    if effective_policy_for_cli_reboot(state, connection_id, &session_id, capability_target)
        .await
        .is_none()
    {
        return;
    }
    if let Some(pane) = retired_instance_pane(state, &session_id, target.as_ref()) {
        let message = retired_pi_reboot_refusal(&pane).unwrap_or_else(|| {
            format!(
                "Pane {} uses an unsupported retired provider and blocks CLI reboot",
                pane.pane_id
            )
        });
        send_policy_error(state, connection_id, message).await;
        return;
    }
    tracing::info!(%session_id, ?machine_id, "Rebooting CLI");
    // To the instance just checked, even if the primary hands over meanwhile.
    let route_machine =
        machine_id.or_else(|| target.as_ref().and_then(|instance| instance.machine_id));
    let outcome = state
        .sessions
        .route_to_cli_on_machine(&session_id, route_machine, reboot_cli_message(session_id))
        .await;
    if !outcome.delivered() {
        let message = match outcome {
            CliRouteOutcome::MachineNotRunning { .. } => undelivered_message(state, &outcome),
            _ => "The CLI reboot request could not be delivered. Reboot the CLI manually on the project host, then try again.".to_string(),
        };
        send_policy_error(state, connection_id, message).await;
    }
}

/// `CliLifecycleRequest` for a session this connection may use. The
/// operation is for the instance on `machine_id`, else the primary: that
/// instance's CLI must support it, the request carries that instance's
/// inventory, and it is bound to that instance's machine so progress another
/// machine reports can never complete it.
async fn handle_cli_lifecycle_request(
    state: &AppState,
    connection_id: &Uuid,
    session_id: Uuid,
    request_user: Uuid,
    request_id: Uuid,
    operation: shared::CliLifecycleOperation,
    machine_id: Option<Uuid>,
) {
    let failed = |message: String, inventory, machine_id| ServerToWeb::CliLifecycleStatus {
        session_id,
        request_id,
        operation,
        phase: shared::CliLifecyclePhase::Failed,
        message: Some(message),
        inventory,
        machine_id,
    };
    let target = match lifecycle_target(state, connection_id, &session_id, machine_id).await {
        Ok(target) => target,
        Err(refusal) => {
            state
                .sessions
                .send_to_web(connection_id, failed(refusal, None, machine_id))
                .await;
            return;
        }
    };
    let target_machine =
        machine_id.or_else(|| target.as_ref().and_then(|instance| instance.machine_id));
    if let Some(instance) = &target {
        if let Some(refusal) =
            instance_access_refusal(state, connection_id, &session_id, instance).await
        {
            state
                .sessions
                .send_to_web(connection_id, failed(refusal, None, target_machine))
                .await;
            return;
        }
    }
    // That instance's own inventory: a reboot confirmation must count only
    // the tabs it would restart.
    let inventory = match &target {
        Some(instance) => state
            .sessions
            .attached_lifecycle_inventories(&session_id)
            .into_iter()
            .find(|(machine, _)| *machine == instance.machine_id)
            .map(|(_, inventory)| inventory),
        None => state.sessions.lifecycle_inventory(&session_id),
    };
    let supported = target.as_ref().is_some_and(|instance| {
        state
            .sessions
            .cli_supports_capability(&instance.cli_id, shared::CLI_LIFECYCLE_CAPABILITY)
    });
    let Some(cli_message) = lifecycle_cli_message(supported, session_id, request_id, operation)
    else {
        state
            .sessions
            .send_to_web(
                connection_id,
                failed(
                    "This project CLI is too old for safe lifecycle controls. Upgrade it on the project host."
                        .to_string(),
                    inventory,
                    target_machine,
                ),
            )
            .await;
        return;
    };

    if operation == shared::CliLifecycleOperation::RebootCli {
        // Same here: called for the too-old-CLI gate. Only a retired backend
        // blocks the reboot now, since it cannot run at all; being outside
        // the allowlist no longer does — see `authorize_existing_pane_launch`.
        let capability_target = machine_id
            .map(InstanceTarget::Machine)
            .unwrap_or(InstanceTarget::Primary);
        if effective_policy_for_cli_reboot(state, connection_id, &session_id, capability_target)
            .await
            .is_none()
        {
            return;
        }
        if let Some(pane) = retired_instance_pane(state, &session_id, target.as_ref()) {
            let message = retired_pi_reboot_refusal(&pane).unwrap_or_else(|| {
                format!(
                    "Pane {} uses a retired backend and blocks CLI reboot",
                    pane.pane_id
                )
            });
            send_policy_error(state, connection_id, message).await;
            return;
        }
    }

    match state
        .sessions
        .claim_lifecycle_request(request_id, session_id, request_user, operation)
    {
        crate::session::LifecycleRequestClaim::Conflict => {
            state
                .sessions
                .send_to_web(
                    connection_id,
                    failed(
                        "That request ID is already bound to another lifecycle operation."
                            .to_string(),
                        None,
                        target_machine,
                    ),
                )
                .await;
        }
        crate::session::LifecycleRequestClaim::InFlight(status)
        | crate::session::LifecycleRequestClaim::Replay(status) => {
            state
                .sessions
                .send_to_web(connection_id, status.message())
                .await;
        }
        crate::session::LifecycleRequestClaim::New => {
            state
                .sessions
                .bind_lifecycle_request(request_id, target_machine, inventory.clone());
            let accepted = state
                .sessions
                .update_lifecycle_request(
                    request_id,
                    session_id,
                    operation,
                    shared::CliLifecyclePhase::Accepted,
                    None,
                    inventory,
                )
                .expect("new lifecycle request must be retained");
            broadcast_lifecycle_status(state, session_id, accepted.message()).await;
            let outcome = state
                .sessions
                .route_to_cli_on_machine(&session_id, target_machine, cli_message)
                .await;
            if !outcome.delivered() {
                let message = match outcome {
                    CliRouteOutcome::MachineNotRunning { .. } => undelivered_message(state, &outcome),
                    _ => "The request could not be delivered. Start or restart the CLI manually on the project host.".to_string(),
                };
                if let Some(failed) = state.sessions.update_lifecycle_request(
                    request_id,
                    session_id,
                    operation,
                    shared::CliLifecyclePhase::Failed,
                    Some(message),
                    None,
                ) {
                    broadcast_lifecycle_status(state, session_id, failed.message()).await;
                }
                return;
            }

            let timeout_state = state.clone();
            tokio::spawn(async move {
                tokio::time::sleep(crate::session::LIFECYCLE_REQUEST_TIMEOUT).await;
                if let Some(timed_out) =
                    timeout_state.sessions.timeout_lifecycle_request(request_id)
                {
                    tracing::warn!(%session_id, %request_id, ?operation, "CLI lifecycle operation timed out");
                    broadcast_lifecycle_status(&timeout_state, session_id, timed_out.message())
                        .await;
                }
            });
        }
    }
}

/// Relay a request's status as an instance's own reports are relayed: a
/// viewer that keeps one inventory per session is not handed the inventory
/// of a request bound to another machine than the primary.
async fn broadcast_lifecycle_status(state: &AppState, session_id: Uuid, message: ServerToWeb) {
    crate::routes::ws_cli::route_lifecycle_to_authorized_web(state, session_id, message, false)
        .await;
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

    // The pane that receives the input, and so the instance it goes to. A CLI
    // applies input that names no pane to the legacy pane it means, which is
    // the interactive one unless the client named the type.
    let target_pane = pane_id.unwrap_or_else(|| {
        pane_type
            .map(|pane_type| shared::PaneConfig::pane_id_from_legacy(&pane_type))
            .unwrap_or(shared::PANE_ID_INTERACTIVE)
    });
    if let Some(refusal) = read_only_pane_refusal(state, &sid, target_pane) {
        refuse_input(
            state,
            connection_id,
            sid,
            target_pane,
            client_msg_id.as_deref(),
            refusal,
        )
        .await;
        return;
    }
    if let Some(refusal) = pane_control_refusal(state, connection_id, &sid, target_pane).await {
        refuse_input(
            state,
            connection_id,
            sid,
            target_pane,
            client_msg_id.as_deref(),
            refusal,
        )
        .await;
        return;
    }

    tracing::info!(
        "Routing input to session {}: {:?}",
        sid,
        text.chars().take(50).collect::<String>()
    );

    let outcome = state
        .sessions
        .route_to_cli_for_pane(
            &sid,
            target_pane,
            ServerToCli::Input {
                session_id: sid,
                data: text.clone(),
                pane_id,
            },
        )
        .await;

    if outcome.delivered() {
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
        match outcome {
            // The tab's machine stopped since the check above: as final as
            // that refusal.
            CliRouteOutcome::MachineNotRunning { .. } => {
                refuse_input(
                    state,
                    connection_id,
                    sid,
                    target_pane,
                    client_msg_id.as_deref(),
                    undelivered_message(state, &outcome),
                )
                .await;
            }
            // Nothing reached a CLI, and a resend may once one reconnects, so
            // the send is left unanswered as before.
            _ => send_policy_error(state, connection_id, "CLI client not connected").await,
        }
    }
}

/// `RemovePane`: close a tab on the instance that owns it.
async fn handle_remove_pane(
    state: &AppState,
    connection_id: &Uuid,
    msg_session_id: Option<Uuid>,
    fallback_session_id: Option<Uuid>,
    pane_id: u32,
    cleanup_action: Option<shared::PaneCleanupAction>,
) {
    let Some(sid) = resolve_pane_session(
        state,
        connection_id,
        msg_session_id,
        fallback_session_id,
        pane_id,
    )
    .await
    else {
        return;
    };
    let Ok((_project_id, _project_guard)) = state.active_session_operation(&sid.to_string()).await
    else {
        return;
    };
    if state
        .sessions
        .get_session_panes(&sid)
        .iter()
        .any(|pane| pane.pane_id == pane_id && pane.mode == shared::PaneMode::Deadloop)
    {
        send_policy_error(state, connection_id, RETIRED_BOT_MESSAGE).await;
        return;
    }
    tracing::info!(
        "Removing pane {} from session {} (cleanup_action={:?})",
        pane_id,
        sid,
        cleanup_action,
    );
    state.sessions.cancel_pane_reboot(&sid, pane_id);
    // The instance the close goes to, read before routing it there.
    let receiver = match state.sessions.pane_owner(&sid, pane_id) {
        PaneOwner::Instance(owner) => Some(owner.cli_id),
        PaneOwner::NotRunning { .. } | PaneOwner::NoInstance => None,
    };
    let outcome = state
        .sessions
        .route_to_cli_for_pane(
            &sid,
            pane_id,
            ServerToCli::RemovePane {
                session_id: sid,
                pane_id,
                cleanup_action,
            },
        )
        .await;
    match outcome {
        // Drop any terminal scrollback for this pane so a
        // later pane that reuses the id doesn't inherit the
        // dead pane's frames on attach.
        CliRouteOutcome::Delivered(_) => {
            state.sessions.clear_terminal_scrollback(&sid, pane_id);
            // An untagged tab that the instance it went to does not run
            // would otherwise stay: only that instance's report could remove
            // it, and its report leaves untagged tabs alone.
            if receiver.is_some_and(|cli_id| {
                state
                    .sessions
                    .drop_closed_untagged_pane(&sid, pane_id, &cli_id)
            }) {
                crate::routes::ws_cli::persist_pane_roster(state, &sid).await;
                state
                    .sessions
                    .route_to_web(
                        &sid,
                        ServerToWeb::PaneList {
                            session_id: sid,
                            panes: state.sessions.get_session_panes(&sid),
                        },
                    )
                    .await;
            }
        }
        // Its machine stopped meanwhile: the tab stays,
        // stopped there, and so does what it last showed.
        CliRouteOutcome::MachineNotRunning { .. } => {
            send_policy_error(state, connection_id, undelivered_message(state, &outcome)).await
        }
        CliRouteOutcome::NoInstance | CliRouteOutcome::Undelivered => {
            state.sessions.clear_terminal_scrollback(&sid, pane_id)
        }
    }
}

/// `ReorderPanes`: the project-wide tab order, which only the server keeps.
///
/// The roster is loaded first. A connection attached on its own behalf, as
/// `resolve_target_session` attaches one after a reconnect, has not loaded
/// it, and after a server restart the cache is then empty until something
/// reads `panes.json`. Reordering that empty cache saved an empty roster over
/// the persisted one and marked it loaded, so every stopped machine's tabs
/// vanished, and their ids came free, until those machines ran again.
async fn handle_reorder_panes(
    state: &AppState,
    connection_id: &Uuid,
    msg_session_id: Option<Uuid>,
    fallback_session_id: Option<Uuid>,
    pane_ids: Vec<u32>,
) {
    let Some(sid) =
        resolve_target_session(state, connection_id, msg_session_id, fallback_session_id).await
    else {
        return;
    };
    let Ok((_project_id, _project_guard)) = state.active_session_operation(&sid.to_string()).await
    else {
        return;
    };
    tracing::info!("Reordering panes in session {}", sid);
    // No roster known at all (a legacy session that never saved one) leaves
    // nothing to order.
    if resolve_pane_roster(state, &sid).await.is_none() {
        return;
    }
    let Some(panes) = state.sessions.reorder_session_panes(&sid, &pane_ids) else {
        return;
    };
    crate::routes::ws_cli::persist_pane_roster(state, &sid).await;
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

/// `UpdatePaneModel`: switch one tab's provider or model. The roster is
/// edited and sent to viewers only for a switch the tab's CLI will carry
/// out, so every refusal comes before the edit.
async fn handle_update_pane_model(
    state: &AppState,
    connection_id: &Uuid,
    msg_session_id: Option<Uuid>,
    fallback_session_id: Option<Uuid>,
    pane_id: u32,
    provider: Option<shared::Provider>,
    model: Option<String>,
) {
    let Some(sid) = resolve_pane_session(
        state,
        connection_id,
        msg_session_id,
        fallback_session_id,
        pane_id,
    )
    .await
    else {
        return;
    };
    let Ok((_project_id, _project_guard)) = state.active_session_operation(&sid.to_string()).await
    else {
        return;
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
        send_policy_error(state, connection_id, "Pane not found").await;
        return;
    };
    if existing.mode == shared::PaneMode::Deadloop {
        send_policy_error(state, connection_id, RETIRED_BOT_MESSAGE).await;
        return;
    }
    if let Some(refusal) = retired_pane_switch_refusal(&existing) {
        send_policy_error(state, connection_id, refusal).await;
        return;
    }
    let desired_provider = provider.unwrap_or(existing.provider);
    if provider.is_some()
        && existing.kind == shared::PaneKind::Terminal
        && !requires_capable_terminal_cli(
            state,
            connection_id,
            &sid,
            desired_provider,
            InstanceTarget::Pane(pane_id),
        )
        .await
    {
        return;
    }
    if !authorize_profile_launch(
        state,
        connection_id,
        &sid,
        existing.kind,
        desired_provider,
        trimmed.as_deref(),
        existing.managed,
        InstanceTarget::Pane(pane_id),
    )
    .await
    {
        return;
    }
    tracing::info!(
        "Switching pane {} agent in session {} to provider={:?} model={:?}",
        pane_id,
        sid,
        provider,
        trimmed
    );
    let new_model = trimmed.clone();
    edit_roster_pane(state, sid, pane_id, |pane| {
        if let Some(p) = provider {
            pane.provider = p;
        }
        pane.model = new_model;
    })
    .await;
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

/// Apply a web edit of one tab (its label, effort, model or bot settings),
/// save the roster and send it to the session's viewers. Returns false, and
/// changes nothing, when the roster does not list the tab.
///
/// The tab is edited in place and the roster saved as it is at the moment of
/// writing, as a reorder is: editing a copy and putting the copy back would
/// replace the whole roster, dropping a tab another machine's instance
/// reported in between from memory and from `panes.json`.
async fn edit_roster_pane(
    state: &AppState,
    session_id: Uuid,
    pane_id: u32,
    edit: impl FnOnce(&mut shared::PaneConfig),
) -> bool {
    let autopilot_before = state.sessions.pane_autopilot_availabilities(&session_id);
    let Some(panes) = state
        .sessions
        .update_session_pane(&session_id, pane_id, edit)
    else {
        return false;
    };
    crate::routes::ws_cli::persist_pane_roster(state, &session_id).await;
    state
        .sessions
        .route_to_web(&session_id, ServerToWeb::PaneList { session_id, panes })
        .await;
    // A tab switched to OMP is a new OMP terminal tab to autopilot-capable
    // viewers, who are told whether its instance can run autopilot. The
    // instance's own roster report then changes nothing and says nothing.
    state
        .sessions
        .broadcast_pane_autopilot_changes(&session_id, &autopilot_before);
    true
}

/// Deliver the answer to a pending decision (a question or a plan review),
/// which names no pane on the wire. A claimed decision is no longer there
/// for routing to look up, so the caller passes its pane, or the pane the
/// client named, and the answer goes to the instance that owns it. With
/// neither, a decision still pending names the pane, else it goes to the
/// primary, as an answer whose decision cannot be resolved always has.
/// Whichever instance it reaches, the viewer must be allowed to use that
/// instance's machine. Returns the error to acknowledge with.
async fn route_decision_answer(
    state: &AppState,
    connection_id: &Uuid,
    session_id: &Uuid,
    pane_id: Option<u32>,
    message: ServerToCli,
) -> Result<(), String> {
    let pane_id = pane_id.or_else(|| state.sessions.unscoped_target_pane(session_id, &message));
    let outcome = match pane_id {
        Some(pane_id) => {
            if let Some(refusal) =
                pane_control_refusal(state, connection_id, session_id, pane_id).await
            {
                return Err(refusal);
            }
            state
                .sessions
                .route_to_cli_for_pane(session_id, pane_id, message)
                .await
        }
        None => {
            // Passing the session's check only shows that some instance
            // admits the viewer. An answer whose decision the server no
            // longer holds, as after a restart, goes to the primary, which
            // may be an instance the viewer may not use.
            if let Some(primary) = state.sessions.primary_instance(session_id) {
                if let Some(refusal) =
                    instance_access_refusal(state, connection_id, session_id, &primary).await
                {
                    return Err(refusal);
                }
            }
            state
                .sessions
                .route_to_cli_on_machine(session_id, None, message)
                .await
        }
    };
    if outcome.delivered() {
        Ok(())
    } else {
        Err(undelivered_message(state, &outcome))
    }
}

/// Approve (`"y"`) or reject (`"n"`) a pending tool call. The CLI receives
/// the answer as `Input` for the decision's pane, so it goes to the instance
/// that owns that pane; an answer naming no pane is applied to the legacy
/// interactive pane, whose instance receives it. A refused or undelivered
/// answer gives the decision back so another surface can still resolve it.
#[allow(clippy::too_many_arguments)]
async fn handle_approval_decision(
    state: &AppState,
    connection_id: &Uuid,
    fallback_session_id: Option<Uuid>,
    msg_sid: Option<Uuid>,
    tool_call_id: String,
    pane_id: Option<u32>,
    request_id: Option<String>,
    data: &str,
) {
    let Some(sid) =
        resolve_target_session(state, connection_id, msg_sid, fallback_session_id).await
    else {
        return;
    };
    if replay_or_claim_mutation_request(state, connection_id, request_id.as_deref()).await {
        return;
    }
    let claimed = state.sessions.claim_pending_decision(
        sid,
        &tool_call_id,
        pane_id,
        shared::MutationKind::Approval,
    );
    if claimed.is_none() && request_id.is_some() {
        send_mutation_ack(
            state,
            connection_id,
            request_id.as_deref(),
            sid,
            pane_id,
            shared::MutationKind::Approval,
            Err("This approval was already resolved or is no longer current"),
        )
        .await;
        return;
    }
    let target_pane = claimed
        .as_ref()
        .and_then(|decision| decision.pane_id)
        .or(pane_id);
    let owner_pane = target_pane.unwrap_or(shared::PANE_ID_INTERACTIVE);
    let result = match pane_control_refusal(state, connection_id, &sid, owner_pane).await {
        Some(refusal) => Err(refusal),
        None => {
            let outcome = state
                .sessions
                .route_to_cli_for_pane(
                    &sid,
                    owner_pane,
                    ServerToCli::Input {
                        session_id: sid,
                        data: data.to_string(),
                        pane_id: target_pane,
                    },
                )
                .await;
            if outcome.delivered() {
                Ok(())
            } else {
                Err(undelivered_message(state, &outcome))
            }
        }
    };
    if result.is_err() {
        if let Some(decision) = claimed {
            state
                .sessions
                .restore_pending_decision(sid, tool_call_id, decision);
        }
    }
    send_mutation_ack(
        state,
        connection_id,
        request_id.as_deref(),
        sid,
        target_pane,
        shared::MutationKind::Approval,
        result,
    )
    .await;
}

/// Shared with the CLI's answer-keystroke pacing: both write on a person's
/// behalf into a live TUI, and both are defeated by the same paste-burst
/// classification. One value, so tuning it once tunes it everywhere.
const TERMINAL_CONVERSATION_SUBMIT_DELAY: Duration =
    Duration::from_millis(shared::TERMINAL_SUBMIT_SETTLE_MS);

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

    if let Some(refusal) = read_only_pane_refusal(state, &sid, pane_id) {
        refuse_input(
            state,
            connection_id,
            sid,
            pane_id,
            client_msg_id.as_deref(),
            refusal,
        )
        .await;
        return;
    }
    if let Some(refusal) = pane_control_refusal(state, connection_id, &sid, pane_id).await {
        refuse_input(
            state,
            connection_id,
            sid,
            pane_id,
            client_msg_id.as_deref(),
            refusal,
        )
        .await;
        return;
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
        // Nothing reached the terminal, and a resend may once a CLI
        // reconnects, so the send is left unanswered as before.
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
    state.sessions.expect_terminal_transcript_echo(
        sid,
        pane_id,
        text.clone(),
        client_msg_id.clone(),
    );

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
        // The text is already in the terminal and its id is not recorded, so
        // an unanswered send would be resent and typed there again by a
        // client that resends until acknowledged. The refusal stops that and
        // says where the text is.
        refuse_input(
            state,
            connection_id,
            sid,
            pane_id,
            client_msg_id.as_deref(),
            "The text reached the terminal, but Enter could not be sent".to_string(),
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
    // Forwarding bytes does not prove the provider accepted a prompt: a menu
    // can consume them. Only the transcript observer starts Working and
    // counts a prompt, whether the input came from here or the raw terminal.
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
        assert!(state.sessions.get_pane_statuses(&session_id).is_empty());
        assert_eq!(
            state
                .db
                .get_project_usage_stats(&session_id.to_string())
                .await
                .unwrap()
                .lifetime
                .prompts,
            0
        );
        assert!(
            web_rx.try_recv().is_err(),
            "forwarding is not transcript confirmation"
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
        assert!(
            matches!(web_rx.try_recv().unwrap(), ServerToWeb::TerminalConversationRecorded {
            session_id: got_session, pane_id: 9, client_msg_id: Some(ref id),
        } if got_session == session_id && id == "terminal-client-1")
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
            state.storage.get_messages(&session_id).await.unwrap().len(),
            1,
            "provider transcript echo must not persist a duplicate"
        );
        assert_eq!(
            state
                .db
                .get_project_usage_stats(&session_id.to_string())
                .await
                .unwrap()
                .lifetime
                .prompts,
            1
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

        assert_eq!(
            state
                .db
                .get_project_usage_stats(&session_id.to_string())
                .await
                .unwrap()
                .lifetime
                .prompts,
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
    let outbound_state = state.clone();
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if !crate::jobs::web_outbound_allowed(&outbound_state, connection_id, &msg).await {
                continue;
            }
            if !crate::workers::socket::web_outbound_allowed(&outbound_state, connection_id, &msg)
                .await
            {
                continue;
            }
            if !autopilot_outbound_allowed(&outbound_state, &connection_id, &msg) {
                continue;
            }
            let msg = with_viewer_runtime_access(&outbound_state, &connection_id, msg).await;
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
                    if let Ok(WebToServer::RegisterLocalProject {
                        machine_id,
                        request_id,
                        ..
                    }) = &parsed
                    {
                        super::local_projects::send_failure(
                            &state,
                            &connection_id,
                            *machine_id,
                            request_id.clone(),
                            "Mobile device session is expired or revoked",
                        )
                        .await;
                    }
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
                let error = "This mobile build is read-only because its protocol version is incompatible. Update the app to make changes.";
                if let Ok(WebToServer::RegisterLocalProject {
                    machine_id,
                    request_id,
                    ..
                }) = &parsed
                {
                    super::local_projects::send_failure(
                        &state,
                        &connection_id,
                        *machine_id,
                        request_id.clone(),
                        error,
                    )
                    .await;
                } else {
                    state
                        .sessions
                        .send_to_web(
                            &connection_id,
                            ServerToWeb::Error {
                                message: error.to_string(),
                            },
                        )
                        .await;
                }
                continue;
            }
            match parsed {
                Ok(WebToServer::SubscribeWorker {
                    project_id,
                    worker_id,
                    after_seq,
                }) => {
                    crate::workers::socket::subscribe(
                        &state,
                        connection_id,
                        project_id,
                        worker_id,
                        after_seq,
                    )
                    .await;
                }
                Ok(WebToServer::UnsubscribeWorker { worker_id }) => {
                    crate::workers::socket::unsubscribe(&state, connection_id, worker_id);
                }
                Ok(WebToServer::SubscribeJob {
                    project_id,
                    job_id,
                    after_seq,
                }) => {
                    crate::jobs::subscribe(&state, connection_id, project_id, job_id, after_seq)
                        .await;
                }
                Ok(WebToServer::UnsubscribeJob { job_id }) => {
                    state.jobs.subscriptions.remove(&(connection_id, job_id));
                }
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
                                    state.jobs.connection_auth.insert(
                                        connection_id,
                                        (uid, claims.device_session_id.clone(), claims.exp),
                                    );
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
                                                    | shared::AUTONOMOUS_JOBS_CAPABILITY
                                                    | shared::AUTONOMOUS_WORKERS_CAPABILITY
                                                    | shared::PROJECT_INSTANCES_CAPABILITY
                                                    | shared::TAB_AUTOPILOT_CAPABILITY
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
                    handle_approval_decision(
                        &state,
                        &connection_id,
                        session_id,
                        msg_sid,
                        tool_call_id,
                        pane_id,
                        request_id,
                        "y",
                    )
                    .await;
                }
                Ok(WebToServer::Reject {
                    session_id: msg_sid,
                    tool_call_id,
                    pane_id,
                    request_id,
                }) => {
                    handle_approval_decision(
                        &state,
                        &connection_id,
                        session_id,
                        msg_sid,
                        tool_call_id,
                        pane_id,
                        request_id,
                        "n",
                    )
                    .await;
                }
                Ok(WebToServer::PauseDeadloop {
                    session_id: msg_sid,
                }) => {
                    // The CLI pauses only its deadloop pane, so the instance
                    // that owns that pane receives it.
                    if let Some(sid) = resolve_pane_session(
                        &state,
                        &connection_id,
                        msg_sid,
                        session_id,
                        shared::PANE_ID_DEADLOOP,
                    )
                    .await
                    {
                        tracing::info!("Pausing deadloop for session {}", sid);
                        state
                            .sessions
                            .route_to_cli(&sid, ServerToCli::PauseDeadloop { session_id: sid })
                            .await;
                    }
                }
                Ok(WebToServer::ResumeDeadloop { .. }) => {
                    send_policy_error(&state, &connection_id, RETIRED_BOT_MESSAGE).await;
                }
                Ok(WebToServer::RebootCli {
                    session_id: msg_sid,
                    machine_id,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, msg_sid, session_id).await
                    {
                        handle_reboot_cli(&state, &connection_id, sid, machine_id).await;
                    }
                }
                Ok(WebToServer::CliLifecycleRequest {
                    session_id: sid,
                    request_id,
                    operation,
                    machine_id,
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
                    handle_cli_lifecycle_request(
                        &state,
                        &connection_id,
                        target_sid,
                        request_user,
                        request_id,
                        operation,
                        machine_id,
                    )
                    .await;
                }
                Ok(WebToServer::StartMachineProjectCli {
                    machine_id,
                    project_id,
                    all_instances,
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
                        "StartMachineProjectCli: user={} machine={} project={} all_instances={}",
                        uid,
                        machine_id,
                        project_id,
                        all_instances
                    );
                    start_machine_project(
                        &state,
                        &connection_id,
                        &uid,
                        machine_id,
                        &project_id,
                        all_instances,
                    )
                    .await;
                }
                Ok(WebToServer::StopMachineProjectCli {
                    machine_id,
                    project_id,
                    all_instances,
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

                    stop_machine_project(
                        &state,
                        &connection_id,
                        &uid,
                        machine_id,
                        &project_id,
                        all_instances,
                    )
                    .await;
                }
                Ok(WebToServer::RebootDaemon { machine_id }) => {
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

                    // Authorized by machine ownership alone, not by a project
                    // on it: a daemon is per-machine, and a machine running
                    // nothing still has a daemon worth restarting.
                    let allowed = state
                        .sessions
                        .get_machines_for_user(&uid)
                        .into_iter()
                        .any(|m| m.machine.machine_id == machine_id);
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

                    tracing::info!(%machine_id, "Rebooting daemon");
                    if !state
                        .sessions
                        .send_to_daemon(&machine_id, ServerToDaemon::RebootDaemon)
                        .await
                    {
                        state
                            .sessions
                            .send_to_web(
                                &connection_id,
                                ServerToWeb::Error {
                                    message: "Daemon is offline — the reboot was not delivered"
                                        .to_string(),
                                },
                            )
                            .await;
                    }
                }
                Ok(WebToServer::RegisterLocalProject {
                    machine_id,
                    cluster_owner_user_id,
                    path,
                    request_id,
                }) => {
                    super::local_projects::register(
                        &state,
                        &connection_id,
                        machine_id,
                        cluster_owner_user_id.as_deref(),
                        path,
                        request_id,
                    )
                    .await;
                }
                Ok(WebToServer::CreateProjectInstance {
                    machine_id,
                    cluster_owner_user_id,
                    git_remote,
                    instance_name,
                    branch,
                    clone_url,
                    base_path,
                    request_id,
                    join_project_id,
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

                    // Another checkout of an existing project, onto one of the
                    // requester's own machines: a separate, owner-only path.
                    if let Some(project_id) = join_project_id {
                        super::project_join::create_joined_instance(
                            &state,
                            &connection_id,
                            uid,
                            super::project_join::JoinRequest {
                                machine_id,
                                cluster_owner_user_id,
                                git_remote,
                                instance_name,
                                branch,
                                clone_url,
                                base_path,
                                request_id,
                                project_id,
                            },
                        )
                        .await;
                        continue;
                    }

                    let (_machine_owner, requested_cluster, shared_request) =
                        match resolve_project_creation_target(
                            &state,
                            &uid,
                            &machine_id,
                            cluster_owner_user_id.as_deref(),
                        )
                        .await
                        {
                            Ok(target) => target,
                            Err(message) => {
                                state
                                    .sessions
                                    .send_to_web(
                                        &connection_id,
                                        ServerToWeb::Error {
                                            message: message.to_string(),
                                        },
                                    )
                                    .await;
                                continue;
                            }
                        };

                    tracing::info!(
                        "CreateProjectInstance: user={} machine={} repo={} name={}",
                        uid,
                        machine_id,
                        git_remote,
                        instance_name
                    );

                    let (daemon_message, shared_provisioning) = if shared_request {
                        let Some(submitted_url) = clone_url.as_deref() else {
                            state
                                .sessions
                                .send_to_web(
                                    &connection_id,
                                    ServerToWeb::Error {
                                        message: "Shared machines accept a public GitHub HTTPS URL"
                                            .to_string(),
                                    },
                                )
                                .await;
                            continue;
                        };
                        let repository = match shared::parse_public_github_repository(submitted_url)
                        {
                            Ok(repository) => repository,
                            Err(error) => {
                                state
                                    .sessions
                                    .send_to_web(
                                        &connection_id,
                                        ServerToWeb::Error {
                                            message: error.to_string(),
                                        },
                                    )
                                    .await;
                                continue;
                            }
                        };
                        let request_id = request_id
                            .filter(|value| !value.trim().is_empty() && value.len() <= 128)
                            .unwrap_or_else(|| Uuid::new_v4().to_string());
                        let canonical_remote = repository.git_remote();
                        let canonical_url = repository.clone_url();
                        let fingerprint = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                            Sha256::digest(
                                format!(
                                    "{}\0{}\0{}\0{}\0{}",
                                    requested_cluster,
                                    machine_id,
                                    canonical_remote,
                                    instance_name.trim(),
                                    branch.trim()
                                )
                                .as_bytes(),
                            ),
                        );
                        let reserved_project_id = Uuid::new_v4().to_string();
                        let provisioning = match state
                            .db
                            .claim_project_provisioning(
                                &request_id,
                                &uid.to_string(),
                                &requested_cluster,
                                &machine_id.to_string(),
                                &fingerprint,
                                &canonical_remote,
                                &canonical_url,
                                instance_name.trim(),
                                branch.trim(),
                                &reserved_project_id,
                            )
                            .await
                        {
                            Ok(request) => request,
                            Err(error) => {
                                state
                                    .sessions
                                    .send_to_web(
                                        &connection_id,
                                        ServerToWeb::Error {
                                            message: error.to_string(),
                                        },
                                    )
                                    .await;
                                continue;
                            }
                        };
                        if provisioning.status == "completed" {
                            state.sessions.relay_project_instance_created_to_user(
                                &uid,
                                &machine_id,
                                Some(request_id),
                                Some(provisioning.project_id),
                                None,
                            );
                            continue;
                        }
                        if !matches!(provisioning.status.as_str(), "pending" | "cloned") {
                            state
                                .sessions
                                .send_to_web(
                                    &connection_id,
                                    ServerToWeb::Error {
                                        message: provisioning.error_message.unwrap_or_else(|| {
                                            "Project provisioning is no longer active".to_string()
                                        }),
                                    },
                                )
                                .await;
                            continue;
                        }
                        let provisioning_request = provisioning.request_id.clone();
                        (
                            ServerToDaemon::CreateProjectInstance {
                                git_remote: provisioning.git_remote,
                                instance_name: provisioning.instance_name,
                                branch: provisioning.branch,
                                clone_url: Some(provisioning.clone_url),
                                base_path: None,
                                request_id: Some(provisioning.request_id),
                                project_id: Some(provisioning.project_id),
                                provisioning_mode: shared::ProjectProvisioningMode::PublicGithub,
                                join_existing: false,
                            },
                            Some(provisioning_request),
                        )
                    } else {
                        (
                            ServerToDaemon::CreateProjectInstance {
                                git_remote,
                                instance_name,
                                branch,
                                clone_url,
                                base_path,
                                request_id,
                                project_id: None,
                                provisioning_mode: shared::ProjectProvisioningMode::TrustedOwner,
                                join_existing: false,
                            },
                            None,
                        )
                    };

                    if !state
                        .sessions
                        .send_to_daemon(&machine_id, daemon_message)
                        .await
                    {
                        if let Some(request_id) = shared_provisioning {
                            let _ = state
                                .db
                                .fail_project_provisioning(
                                    &request_id,
                                    &uid.to_string(),
                                    "The selected machine is offline; retry after it reconnects",
                                )
                                .await;
                        }
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
                        resolve_pane_session(&state, &connection_id, msg_sid, session_id, pane_id)
                            .await
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
                        resolve_pane_session(&state, &connection_id, msg_sid, session_id, pane_id)
                            .await
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
                        resolve_pane_session(&state, &connection_id, msg_sid, session_id, pane_id)
                            .await
                    else {
                        continue;
                    };
                    if !authorize_existing_pane_launch(&state, &connection_id, &sid, pane_id).await
                    {
                        continue;
                    }
                    tracing::info!("Rebooting pane {} for session {}", pane_id, sid);
                    let preserve_roster_slot = state.sessions.begin_pane_reboot(&sid, pane_id);
                    let sent = state
                        .sessions
                        .route_to_cli(&sid, reboot_pane_cli_message(sid, pane_id))
                        .await;
                    if preserve_roster_slot && !sent {
                        state.sessions.cancel_pane_reboot(&sid, pane_id);
                    }
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
                    machine_id,
                    request_id,
                }) => {
                    let spec = NewPaneSpec {
                        provider,
                        mode,
                        kind,
                        label,
                        prompt,
                        model,
                        isolated_worktree,
                        role,
                        goal,
                        backstory,
                        plan_review_mode,
                        managed,
                    };
                    handle_add_pane(
                        &state,
                        &connection_id,
                        msg_sid,
                        session_id,
                        machine_id,
                        request_id,
                        spec,
                    )
                    .await;
                }
                Ok(WebToServer::RemovePane {
                    session_id: msg_sid,
                    pane_id,
                    cleanup_action,
                }) => {
                    handle_remove_pane(
                        &state,
                        &connection_id,
                        msg_sid,
                        session_id,
                        pane_id,
                        cleanup_action,
                    )
                    .await;
                }
                Ok(WebToServer::UpdatePaneLabel {
                    session_id: msg_sid,
                    pane_id,
                    label,
                }) => {
                    if let Some(sid) =
                        resolve_pane_session(&state, &connection_id, msg_sid, session_id, pane_id)
                            .await
                    {
                        let Ok((_project_id, _project_guard)) =
                            state.active_session_operation(&sid.to_string()).await
                        else {
                            continue;
                        };
                        tracing::info!("Updating pane {} label in session {}", pane_id, sid);
                        let new_label = label.clone();
                        edit_roster_pane(&state, sid, pane_id, |pane| {
                            pane.label = Some(new_label);
                        })
                        .await;
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
                    let result =
                        match pane_control_refusal(&state, &connection_id, &sid, pane_id).await {
                            Some(refusal) => Err(refusal),
                            None => {
                                tracing::info!("Interrupting pane {} in session {}", pane_id, sid);
                                let outcome = state
                                    .sessions
                                    .route_to_cli_for_pane(
                                        &sid,
                                        pane_id,
                                        ServerToCli::InterruptPane {
                                            session_id: sid,
                                            pane_id,
                                        },
                                    )
                                    .await;
                                if outcome.delivered() {
                                    Ok(())
                                } else {
                                    Err(undelivered_message(&state, &outcome))
                                }
                            }
                        };
                    send_mutation_ack(
                        &state,
                        &connection_id,
                        request_id.as_deref(),
                        sid,
                        Some(pane_id),
                        shared::MutationKind::Interrupt,
                        result,
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
                        let target_pane = claimed
                            .as_ref()
                            .and_then(|decision| decision.pane_id)
                            .or(pane_id);
                        let result = route_decision_answer(
                            &state,
                            &connection_id,
                            &sid,
                            target_pane,
                            ServerToCli::PlanReviewAnswer {
                                session_id: sid,
                                tool_use_id: tool_use_id.clone(),
                                approve,
                            },
                        )
                        .await;
                        if result.is_err() {
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
                            result,
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
                        resolve_pane_session(&state, &connection_id, msg_sid, session_id, pane_id)
                            .await
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
                        resolve_pane_session(&state, &connection_id, msg_sid, session_id, pane_id)
                            .await
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
                Ok(WebToServer::TerminalInput {
                    session_id: msg_sid,
                    pane_id,
                    data_b64,
                }) => {
                    let Some(sid) = resolve_pane_session(
                        &state,
                        &connection_id,
                        Some(msg_sid),
                        session_id,
                        pane_id,
                    )
                    .await
                    else {
                        continue;
                    };
                    if !terminal_accepts_keystrokes(&state, &sid, pane_id) {
                        continue;
                    }
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
                    let Some(sid) = resolve_pane_session(
                        &state,
                        &connection_id,
                        Some(msg_sid),
                        session_id,
                        pane_id,
                    )
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
                Ok(WebToServer::UpdatePaneRole {
                    session_id: msg_sid,
                    pane_id,
                    role,
                    goal,
                    backstory,
                }) => {
                    if let Some(sid) =
                        resolve_pane_session(&state, &connection_id, msg_sid, session_id, pane_id)
                            .await
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
                        resolve_pane_session(&state, &connection_id, msg_sid, session_id, pane_id)
                            .await
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
                        resolve_pane_session(&state, &connection_id, msg_sid, session_id, pane_id)
                            .await
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
                Ok(command @ WebToServer::AnswerOmpQuestion { .. }) => {
                    handle_omp_question_answer(&state, &connection_id, command).await;
                }
                Ok(command @ WebToServer::ControlPaneAutopilot { .. }) => {
                    handle_pane_autopilot_control(
                        &state,
                        &connection_id,
                        command,
                        AUTOPILOT_RECEIPT_WAIT,
                    )
                    .await;
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
                        if tool_use_id.starts_with("omp:") {
                            send_mutation_ack(
                                &state, &connection_id, request_id.as_deref(), sid, pane_id,
                                shared::MutationKind::Question,
                                Err("OMP questions require a structured native answer; refresh this page"),
                            ).await;
                            continue;
                        }
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
                        let target_pane = claimed
                            .as_ref()
                            .and_then(|decision| decision.pane_id)
                            .or(pane_id);
                        let result = route_decision_answer(
                            &state,
                            &connection_id,
                            &sid,
                            target_pane,
                            ServerToCli::AnswerQuestion {
                                session_id: sid,
                                tool_use_id: tool_use_id.clone(),
                                answers,
                            },
                        )
                        .await;
                        if result.is_err() {
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
                            result,
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
                        resolve_pane_session(&state, &connection_id, msg_sid, session_id, pane_id)
                            .await
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
                        let new_effort = normalized.clone();
                        if edit_roster_pane(&state, sid, pane_id, |pane| {
                            pane.effort = new_effort;
                        })
                        .await
                        {
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
                    handle_update_pane_model(
                        &state,
                        &connection_id,
                        msg_sid,
                        session_id,
                        pane_id,
                        provider,
                        model,
                    )
                    .await;
                }
                Ok(WebToServer::ReorderPanes {
                    session_id: msg_sid,
                    pane_ids,
                }) => {
                    handle_reorder_panes(&state, &connection_id, msg_sid, session_id, pane_ids)
                        .await;
                }
                Ok(WebToServer::StartBot { .. }) => {
                    send_policy_error(&state, &connection_id, RETIRED_BOT_MESSAGE).await;
                }
                Ok(WebToServer::StopBot {
                    session_id: msg_sid,
                    pane_id,
                }) => {
                    if let Some(sid) =
                        resolve_pane_session(&state, &connection_id, msg_sid, session_id, pane_id)
                            .await
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
                        edit_roster_pane(&state, sid, pane_id, |pane| {
                            pane.stop_requested = true;
                        })
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

                    // Resolve content access separately from runtime access so
                    // the client can explain whether the missing permission is
                    // the project share or the third-party hosting machine.
                    match state.db.get_session(&sid.to_string()).await {
                        Ok(Some(_)) => {}
                        Ok(None) => {
                            reject_session_attachment(
                                &state,
                                &connection_id,
                                sid,
                                SessionAttachmentRejectionReason::SessionNotFound,
                                "Project session not found",
                            )
                            .await;
                            continue;
                        }
                        Err(err) => {
                            tracing::warn!(%err, session_id = %sid, "failed to resolve web attachment");
                            reject_session_attachment(
                                &state,
                                &connection_id,
                                sid,
                                SessionAttachmentRejectionReason::ProjectUnavailable,
                                "Project is unavailable",
                            )
                            .await;
                            continue;
                        }
                    }

                    let has_project_access = state
                        .db
                        .check_session_access(&sid.to_string(), &uid.to_string())
                        .await
                        .unwrap_or(false);
                    if !has_project_access {
                        reject_session_attachment(
                            &state,
                            &connection_id,
                            sid,
                            SessionAttachmentRejectionReason::ProjectAccessRequired,
                            "You do not have access to this project",
                        )
                        .await;
                        continue;
                    }

                    // Attaching subscribes the browser to a live runtime. An
                    // explicit project share is sufficient for an owner-hosted
                    // session; third-party hosting additionally requires
                    // membership in that host cluster and machine access.
                    if !has_session_runtime_access(&state, &sid, &uid).await {
                        reject_session_attachment(
                            &state,
                            &connection_id,
                            sid,
                            SessionAttachmentRejectionReason::HostMachineAccessRequired,
                            "This project is shared with you, but access to its hosting machine is required",
                        )
                        .await;
                        continue;
                    }

                    let Ok((_project_id, _project_guard)) =
                        state.active_session_operation(&sid.to_string()).await
                    else {
                        reject_session_attachment(
                            &state,
                            &connection_id,
                            sid,
                            SessionAttachmentRejectionReason::ProjectUnavailable,
                            "Project is unavailable",
                        )
                        .await;
                        continue;
                    };

                    // Attach to the session to observe its instances' output. The
                    // instances attached to it, not its database row, say which
                    // CLIs run it.
                    if state
                        .sessions
                        .attach_web_to_session(&sid, connection_id, None)
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

                        replay_instances_after_attach(&state, &connection_id, sid).await;

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
                        // Inference runs only when the roster is unknown; a
                        // known-empty roster is the truth and must be sent.
                        let (panes_to_send, roster_known) =
                            match resolve_pane_roster(&state, &sid).await {
                                Some(panes) => (panes, true),
                                None => {
                                    let inferred = infer_panes_from_messages(sid, &messages);
                                    if !inferred.is_empty() {
                                        state
                                            .sessions
                                            .seed_inferred_pane_roster(&sid, inferred.clone());
                                    }
                                    (inferred, false)
                                }
                            };
                        // Silence still means "nothing is known" for a legacy
                        // session with no roster and nothing to infer, so the
                        // web keeps synthesizing from messages as before.
                        if roster_known || !panes_to_send.is_empty() {
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

                        replay_pane_autopilot_after_attach(&state, &connection_id, sid).await;

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
                        reject_session_attachment(
                            &state,
                            &connection_id,
                            sid,
                            SessionAttachmentRejectionReason::SessionNotFound,
                            "Project session not found",
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
                            let is_working =
                                is_active
                                    && state.sessions.get_pane_statuses(&session_id).iter().any(
                                        |(_, _, status)| shared::pane_status_is_working(status),
                                    );
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
                                panes: pane_summaries(&state, &session_id, is_active),
                                instances: Vec::new(),
                            }
                        })
                        .collect();

                    // Add shared sessions with owner email
                    for (s, owner_email, share_role) in shared_sessions {
                        let session_id = Uuid::parse_str(&s.id).unwrap_or_default();
                        let is_active = state.sessions.is_session_active(&session_id);
                        let is_working = is_active
                            && state
                                .sessions
                                .get_pane_statuses(&session_id)
                                .iter()
                                .any(|(_, _, status)| shared::pane_status_is_working(status));
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
                            panes: pane_summaries(&state, &session_id, is_active),
                            instances: Vec::new(),
                        });
                    }
                    // Every host a project runs on, not only the primary's, each
                    // with whether this viewer may use it, which needs the
                    // database and so is filled in once the list is built.
                    for session in &mut sessions {
                        session.instances =
                            viewer_session_instances(&state, &session.id, &uid).await;
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
                                let (panes_to_send, roster_known) =
                                    match resolve_pane_roster(&state, &sid).await {
                                        Some(panes) => (panes, true),
                                        None => {
                                            let inferred =
                                                infer_panes_from_messages(sid, &messages);
                                            if !inferred.is_empty() {
                                                state.sessions.seed_inferred_pane_roster(
                                                    &sid,
                                                    inferred.clone(),
                                                );
                                            }
                                            (inferred, false)
                                        }
                                    };
                                if roster_known || !panes_to_send.is_empty() {
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
                Ok(WebToServer::ForgetMachinePanes {
                    session_id: msg_sid,
                    machine_id,
                }) => {
                    if let Some(sid) =
                        resolve_target_session(&state, &connection_id, Some(msg_sid), session_id)
                            .await
                    {
                        forget_machine_panes(&state, &connection_id, sid, machine_id).await;
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
    state.jobs.connection_auth.remove(&connection_id);
    state
        .jobs
        .subscriptions
        .retain(|(connection, _), _| *connection != connection_id);
    state
        .jobs
        .worker_subscriptions
        .retain(|(connection, _), _| *connection != connection_id);
    state
        .jobs
        .worker_subscription_cursors
        .retain(|(connection, _), _| *connection != connection_id);
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

/// One project running on several machines, seen from the web: controls go
/// to the instance that owns their pane or names their machine, a stopped
/// machine's tabs are refused by name, runtime access is evaluated per
/// machine, project-wide Start and Close cover every machine concerned, and
/// viewers that asked are told about every instance.
#[cfg(test)]
mod project_instance_tests {
    use super::*;
    use crate::config::Config;
    use crate::db::{Database, Session, User};
    use crate::session::AttachOutcome;
    use shared::{PaneConfig, PROJECT_INSTANCES_CAPABILITY};

    const HOST_A: &str = "zoo-005";
    const HOST_B: &str = "zoo-006";
    const HOST_C: &str = "zoo-007";

    /// What a capable instance's CLI advertises.
    const CAPABLE_CLI: &[&str] = &[
        PROJECT_INSTANCES_CAPABILITY,
        shared::PROJECT_POLICY_CAPABILITY,
        shared::CLI_LIFECYCLE_CAPABILITY,
        shared::OMP_QUESTION_CAPABILITY,
    ];
    const CAPABLE_DAEMON: &[&str] = &[
        PROJECT_INSTANCES_CAPABILITY,
        shared::PROJECT_POLICY_CAPABILITY,
    ];

    struct Endpoint<T> {
        id: Uuid,
        rx: mpsc::Receiver<T>,
    }

    impl<T> Endpoint<T> {
        fn drain(&mut self) -> Vec<T> {
            let mut received = Vec::new();
            while let Ok(message) = self.rx.try_recv() {
                received.push(message);
            }
            received
        }
    }

    impl Endpoint<ServerToWeb> {
        /// What the viewer is sent: every queued frame as the connection's
        /// outbound task delivers it, with each instance set's
        /// `runtime_access` filled in for this viewer.
        async fn delivered(&mut self, state: &AppState) -> Vec<ServerToWeb> {
            let mut delivered = Vec::new();
            for message in self.drain() {
                delivered.push(with_viewer_runtime_access(state, &self.id, message).await);
            }
            delivered
        }
    }

    struct Project {
        state: AppState,
        /// The project's owner.
        owner: Uuid,
        /// The account whose CLIs and daemons run the project; the owner
        /// unless the project is hosted in another account's cluster.
        host: Uuid,
        session: Uuid,
        m5: Uuid,
        m6: Uuid,
        dir: std::path::PathBuf,
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
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

    async fn project_on(owner: Uuid, host: Uuid, users: &[Uuid]) -> Project {
        let dir = std::env::temp_dir().join(format!("apas-project-instances-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("apas.db").to_string_lossy().to_string();
        let db = Database::new(&db_path).await.unwrap();
        db.run_migrations().await.unwrap();
        for id in users {
            db.create_user(&user(*id)).await.unwrap();
        }
        let session = Uuid::new_v4();
        db.authorize_project_registration(&session.to_string(), &owner.to_string())
            .await
            .unwrap();
        db.create_session(&Session {
            id: session.to_string(),
            user_id: host.to_string(),
            cli_client_id: None,
            working_dir: Some(format!("/work/{HOST_A}")),
            hostname: Some(HOST_A.to_string()),
            status: "active".to_string(),
            created_at: None,
            updated_at: None,
            is_paused: false,
            project_id: Some(session.to_string()),
            git_remote: None,
            git_remote_url: None,
        })
        .await
        .unwrap();
        let mut config = Config::default();
        config.database.path = db_path;
        let state = AppState::new(db, config);
        state
            .sessions
            .set_session_project(session, session.to_string());
        Project {
            state,
            owner,
            host,
            session,
            m5: Uuid::new_v4(),
            m6: Uuid::new_v4(),
            dir,
        }
    }

    /// A project its owner runs on their own machines.
    async fn project() -> Project {
        let owner = Uuid::new_v4();
        project_on(owner, owner, &[owner]).await
    }

    impl Project {
        fn project_id(&self) -> String {
            self.session.to_string()
        }

        fn cli(&self, capabilities: &[&str]) -> Endpoint<ServerToCli> {
            let id = Uuid::new_v4();
            let (tx, rx) = mpsc::channel(64);
            self.state.sessions.register_cli(id, self.host, tx, None);
            self.state.sessions.set_cli_capabilities(
                id,
                capabilities
                    .iter()
                    .map(|capability| capability.to_string())
                    .collect(),
            );
            Endpoint { id, rx }
        }

        /// An instance of the project on `machine_id` whose CLI advertises
        /// `capabilities`.
        fn instance_with(
            &self,
            machine_id: Uuid,
            host: &str,
            capabilities: &[&str],
        ) -> Endpoint<ServerToCli> {
            let cli = self.cli(capabilities);
            assert!(matches!(
                self.state.sessions.attach_cli_instance(
                    self.session,
                    cli.id,
                    Some(machine_id),
                    Some(format!("/work/{host}")),
                    Some(host.to_string()),
                ),
                AttachOutcome::Attached { .. }
            ));
            cli
        }

        fn instance(&self, machine_id: Uuid, host: &str) -> Endpoint<ServerToCli> {
            self.instance_with(machine_id, host, CAPABLE_CLI)
        }

        /// A web connection of `user` attached to the project's session.
        fn viewer(&self, user_id: Uuid, capabilities: &[&str]) -> Endpoint<ServerToWeb> {
            let id = Uuid::new_v4();
            let (tx, rx) = mpsc::channel(256);
            self.state.sessions.register_web(id, tx);
            self.state.sessions.set_web_user(id, user_id);
            self.state.sessions.set_web_capabilities(
                id,
                capabilities
                    .iter()
                    .map(|capability| capability.to_string())
                    .collect(),
            );
            assert!(self
                .state
                .sessions
                .attach_web_to_session(&self.session, id, None));
            Endpoint { id, rx }
        }

        /// A connected daemon that lists the project.
        fn daemon(
            &self,
            machine_id: Uuid,
            host: &str,
            capabilities: &[&str],
        ) -> Endpoint<ServerToDaemon> {
            let (tx, rx) = mpsc::channel(16);
            self.state.sessions.register_daemon(
                machine_id,
                self.host,
                tx,
                shared::MachineInfo {
                    machine_id,
                    hostname: host.to_string(),
                    os: "linux".to_string(),
                    arch: "x86_64".to_string(),
                    daemon_version: None,
                    deepseek_backend: None,
                    last_seen: None,
                },
                vec![shared::MachineProjectInfo {
                    project_id: self.project_id(),
                    name: None,
                    path: format!("/work/{host}"),
                    preferred_machine_id: None,
                    is_running: false,
                    pid: None,
                    memory_kb: None,
                    last_error: None,
                    git_remote: None,
                }],
            );
            self.state.sessions.set_daemon_capabilities(
                machine_id,
                capabilities
                    .iter()
                    .map(|capability| capability.to_string())
                    .collect(),
            );
            Endpoint { id: machine_id, rx }
        }

        fn roster(&self, panes: Vec<PaneConfig>) {
            self.state.sessions.set_session_panes(&self.session, panes);
        }

        fn pane_ids(&self) -> Vec<u32> {
            self.state
                .sessions
                .get_session_panes(&self.session)
                .iter()
                .map(|pane| pane.pane_id)
                .collect()
        }
    }

    fn pane(pane_id: u32, machine_id: Option<Uuid>, hostname: Option<&str>) -> PaneConfig {
        let mut pane = PaneConfig::defaults().remove(0);
        pane.pane_id = pane_id;
        pane.kind = shared::PaneKind::Terminal;
        pane.label = None;
        pane.machine_id = machine_id;
        pane.hostname = hostname.map(str::to_string);
        pane
    }

    fn errors(messages: &[ServerToWeb]) -> Vec<String> {
        messages
            .iter()
            .filter_map(|message| match message {
                ServerToWeb::Error { message } => Some(message.clone()),
                _ => None,
            })
            .collect()
    }

    fn not_running(host: &str) -> String {
        format!("{host} is not running this project. Start it on {host} first.")
    }

    fn started(daemon: &mut Endpoint<ServerToDaemon>) -> bool {
        daemon
            .drain()
            .iter()
            .any(|message| matches!(message, ServerToDaemon::StartProjectCli { .. }))
    }

    fn stopped(daemon: &mut Endpoint<ServerToDaemon>) -> bool {
        daemon
            .drain()
            .iter()
            .any(|message| matches!(message, ServerToDaemon::StopProjectCli { .. }))
    }

    /// A control for a tab whose machine runs no instance is refused by
    /// naming that machine, whether the tab is tagged for it or only
    /// attributed to its host, and nothing reaches the instance that runs.
    /// A tab no host is known for still goes to the primary.
    #[tokio::test]
    async fn a_stopped_machines_tab_is_refused_by_name() {
        let p = project().await;
        let mut a = p.instance(p.m5, HOST_A);
        let mut web = p.viewer(p.owner, &[]);
        p.roster(vec![
            pane(3, Some(p.m5), Some(HOST_A)),
            pane(4, Some(p.m6), Some(HOST_B)),
            pane(5, None, Some(HOST_B)),
            pane(6, None, None),
        ]);

        for pane_id in [4, 5] {
            assert_eq!(
                resolve_pane_session(&p.state, &web.id, Some(p.session), None, pane_id).await,
                None
            );
            assert_eq!(
                errors(&web.drain()),
                vec![not_running(HOST_B)],
                "pane {pane_id}"
            );
            handle_web_input(
                &p.state,
                &web.id,
                None,
                Some(p.session),
                "keep going".to_string(),
                None,
                Some(pane_id),
                None,
            )
            .await;
            assert_eq!(
                errors(&web.drain()),
                vec![not_running(HOST_B)],
                "pane {pane_id}"
            );
        }
        assert!(a.drain().is_empty(), "nothing reaches the running instance");

        for pane_id in [3, 6] {
            assert_eq!(
                resolve_pane_session(&p.state, &web.id, Some(p.session), None, pane_id).await,
                Some(p.session),
                "pane {pane_id}"
            );
        }
        handle_web_input(
            &p.state,
            &web.id,
            None,
            Some(p.session),
            "for the unhosted tab".to_string(),
            None,
            Some(6),
            None,
        )
        .await;
        assert!(matches!(
            a.drain().as_slice(),
            [ServerToCli::Input {
                pane_id: Some(6),
                ..
            }]
        ));
        assert!(errors(&web.drain()).is_empty());
    }

    /// `viewer` types into the project, naming `pane_id` and tagging the send
    /// with `id`.
    async fn typed(p: &Project, viewer: &Uuid, pane_id: Option<u32>, id: Option<&str>) {
        handle_web_input(
            &p.state,
            viewer,
            None,
            Some(p.session),
            "keep going".to_string(),
            None,
            pane_id,
            id.map(str::to_string),
        )
        .await;
    }

    /// The refusals a sender is told about under its input's id: request id,
    /// pane and reason of each `Input` mutation acknowledgement.
    fn input_refusals(messages: &[ServerToWeb]) -> Vec<(String, Option<u32>, Option<String>)> {
        messages
            .iter()
            .filter_map(|message| match message {
                ServerToWeb::MutationAck {
                    request_id,
                    pane_id,
                    mutation: shared::MutationKind::Input,
                    accepted,
                    error,
                    question_pending,
                    ..
                } => {
                    assert!(!accepted, "an input acknowledgement is only a refusal");
                    assert!(question_pending.is_none());
                    Some((request_id.clone(), *pane_id, error.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// Typed input for a stopped machine's tab is refused by name, and a
    /// viewer that negotiated project instances is also told under the
    /// input's id, after the error: resending cannot change the answer, and a
    /// client that resends until acknowledged would otherwise give up with no
    /// reason to show. Input naming no pane is reported for the legacy pane
    /// it means. A viewer without the capability, and a send without an id,
    /// get the error alone, as before, and nothing reaches an instance.
    #[tokio::test]
    async fn refused_input_is_acknowledged_to_a_viewer_that_tracks_its_sends() {
        let p = project().await;
        let mut a = p.instance(p.m5, HOST_A);
        p.roster(vec![
            pane(shared::PANE_ID_INTERACTIVE, Some(p.m6), Some(HOST_B)),
            pane(3, Some(p.m5), Some(HOST_A)),
            pane(4, Some(p.m6), Some(HOST_B)),
        ]);
        let mut capable = p.viewer(p.owner, &[PROJECT_INSTANCES_CAPABILITY]);
        let mut older = p.viewer(p.owner, &[]);
        capable.drain();
        older.drain();
        let refusal = not_running(HOST_B);

        typed(&p, &capable.id, Some(4), Some("send-4")).await;
        let seen = capable.drain();
        assert!(
            matches!(seen.first(), Some(ServerToWeb::Error { message }) if *message == refusal),
            "the error comes first: {seen:?}"
        );
        assert_eq!(
            input_refusals(&seen),
            vec![("send-4".to_string(), Some(4), Some(refusal.clone()))]
        );
        assert_eq!(seen.len(), 2, "{seen:?}");

        typed(&p, &capable.id, None, Some("send-legacy")).await;
        let seen = capable.drain();
        assert_eq!(errors(&seen), vec![refusal.clone()]);
        assert_eq!(
            input_refusals(&seen),
            vec![(
                "send-legacy".to_string(),
                Some(shared::PANE_ID_INTERACTIVE),
                Some(refusal.clone())
            )]
        );

        handle_terminal_conversation_input(
            &p.state,
            &capable.id,
            None,
            p.session,
            4,
            "keep going".to_string(),
            Some("conversation-4".to_string()),
        )
        .await;
        let seen = capable.drain();
        assert_eq!(errors(&seen), vec![refusal.clone()]);
        assert_eq!(
            input_refusals(&seen),
            vec![("conversation-4".to_string(), Some(4), Some(refusal.clone()))]
        );

        typed(&p, &capable.id, Some(4), None).await;
        let seen = capable.drain();
        assert_eq!(errors(&seen), vec![refusal.clone()]);
        assert!(input_refusals(&seen).is_empty(), "no id, nothing to settle");

        typed(&p, &older.id, Some(4), Some("older-4")).await;
        let seen = older.drain();
        assert_eq!(errors(&seen), vec![refusal.clone()]);
        assert!(
            input_refusals(&seen).is_empty(),
            "an older viewer gets the error alone"
        );

        assert!(a.drain().is_empty(), "nothing reaches the running instance");
    }

    /// When the text reached the terminal but its Enter could not follow, the
    /// send is refused under its id: resending it would type the text into
    /// the terminal again, and the reason says where it is.
    #[tokio::test]
    async fn input_typed_without_its_enter_is_acknowledged_as_refused() {
        let p = project().await;
        let mut a = p.instance(p.m5, HOST_A);
        p.roster(vec![pane(3, Some(p.m5), Some(HOST_A))]);
        let mut web = p.viewer(p.owner, &[PROJECT_INSTANCES_CAPABILITY]);
        web.drain();
        let (state, connection, session) = (p.state.clone(), web.id, p.session);
        let send = tokio::spawn(async move {
            handle_terminal_conversation_input(
                &state,
                &connection,
                None,
                session,
                3,
                "run the tests".to_string(),
                Some("conversation-3".to_string()),
            )
            .await;
        });
        let typed = a.rx.recv().await.expect("the text reaches the terminal");
        assert!(matches!(
            typed,
            ServerToCli::TerminalInput { pane_id: 3, .. }
        ));
        // The instance's connection closes before Enter is sent.
        drop(a);
        send.await.unwrap();

        let reason = "The text reached the terminal, but Enter could not be sent".to_string();
        let seen = web.drain();
        assert_eq!(errors(&seen), vec![reason.clone()]);
        assert_eq!(
            input_refusals(&seen),
            vec![("conversation-3".to_string(), Some(3), Some(reason))]
        );
        assert!(
            !seen
                .iter()
                .any(|message| matches!(message, ServerToWeb::UserInput { .. })),
            "{seen:?}"
        );
        assert!(p
            .state
            .sessions
            .seen_input_id(&p.session, "conversation-3")
            .is_none());
    }

    /// Typed input, approvals, answers and pane controls for a tab on the
    /// secondary machine reach that machine's instance and no other; input
    /// naming no pane goes to whichever instance runs the interactive pane.
    #[tokio::test]
    async fn controls_reach_only_the_instance_that_owns_the_pane() {
        let p = project().await;
        let mut a = p.instance(p.m5, HOST_A);
        let mut b = p.instance(p.m6, HOST_B);
        let mut web = p.viewer(p.owner, &[]);
        p.roster(vec![
            pane(shared::PANE_ID_INTERACTIVE, Some(p.m6), Some(HOST_B)),
            pane(3, Some(p.m5), Some(HOST_A)),
            pane(4, Some(p.m6), Some(HOST_B)),
        ]);

        handle_web_input(
            &p.state,
            &web.id,
            None,
            Some(p.session),
            "typed".to_string(),
            None,
            Some(4),
            None,
        )
        .await;
        assert!(matches!(
            b.drain().as_slice(),
            [ServerToCli::Input {
                pane_id: Some(4),
                ..
            }]
        ));
        handle_web_input(
            &p.state,
            &web.id,
            None,
            Some(p.session),
            "no pane named".to_string(),
            None,
            None,
            None,
        )
        .await;
        assert!(matches!(
            b.drain().as_slice(),
            [ServerToCli::Input { pane_id: None, .. }]
        ));

        p.state.sessions.register_pending_decision(
            p.session,
            "approval-4".to_string(),
            Some(4),
            shared::MutationKind::Approval,
        );
        handle_approval_decision(
            &p.state,
            &web.id,
            None,
            Some(p.session),
            "approval-4".to_string(),
            None,
            Some("approve-1".to_string()),
            "y",
        )
        .await;
        assert!(matches!(
            b.drain().as_slice(),
            [ServerToCli::Input { pane_id: Some(4), data, .. }] if data == "y"
        ));
        assert!(web.drain().iter().any(|message| matches!(
            message,
            ServerToWeb::MutationAck { request_id, accepted: true, .. } if request_id == "approve-1"
        )));

        // A question's answer names no pane on the wire; its decision does,
        // and the handler has claimed it by the time the answer is routed.
        p.state.sessions.register_pending_decision(
            p.session,
            "toolu_question".to_string(),
            Some(4),
            shared::MutationKind::Question,
        );
        let decision = p
            .state
            .sessions
            .claim_pending_decision(
                p.session,
                "toolu_question",
                None,
                shared::MutationKind::Question,
            )
            .unwrap();
        route_decision_answer(
            &p.state,
            &web.id,
            &p.session,
            decision.pane_id,
            ServerToCli::AnswerQuestion {
                session_id: p.session,
                tool_use_id: "toolu_question".to_string(),
                answers: std::collections::HashMap::new(),
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            b.drain().as_slice(),
            [ServerToCli::AnswerQuestion { .. }]
        ));

        // Pane controls follow the pane, whichever instance is primary.
        let sid = resolve_pane_session(&p.state, &web.id, Some(p.session), None, 3)
            .await
            .unwrap();
        p.state
            .sessions
            .route_to_cli(
                &sid,
                ServerToCli::PausePane {
                    session_id: sid,
                    pane_id: 3,
                },
            )
            .await;
        assert!(matches!(
            a.drain().as_slice(),
            [ServerToCli::PausePane { pane_id: 3, .. }]
        ));
        assert!(b.drain().is_empty());
        assert!(errors(&web.drain()).is_empty());
    }

    /// An approval for a tab whose machine stopped is refused by name, and
    /// the decision stays pending for when that machine runs again.
    #[tokio::test]
    async fn an_answer_for_a_stopped_machines_tab_is_refused_and_kept() {
        let p = project().await;
        let mut a = p.instance(p.m5, HOST_A);
        let mut web = p.viewer(p.owner, &[]);
        p.roster(vec![
            pane(3, Some(p.m5), Some(HOST_A)),
            pane(4, Some(p.m6), Some(HOST_B)),
        ]);
        p.state.sessions.register_pending_decision(
            p.session,
            "approval-4".to_string(),
            Some(4),
            shared::MutationKind::Approval,
        );

        handle_approval_decision(
            &p.state,
            &web.id,
            None,
            Some(p.session),
            "approval-4".to_string(),
            Some(4),
            Some("reject-1".to_string()),
            "n",
        )
        .await;
        assert!(web.drain().iter().any(|message| matches!(
            message,
            ServerToWeb::MutationAck { request_id, accepted: false, error: Some(error), .. }
                if request_id == "reject-1" && *error == not_running(HOST_B)
        )));
        assert!(a.drain().is_empty());
        assert!(p
            .state
            .sessions
            .claim_pending_decision(
                p.session,
                "approval-4",
                Some(4),
                shared::MutationKind::Approval,
            )
            .is_some());
    }

    /// The OMP answer, its capability check and the waiter for its receipt
    /// all name the instance that runs the asking pane, even when the
    /// primary's CLI could not take the answer.
    #[tokio::test]
    async fn an_omp_answer_goes_to_the_instance_that_asked() {
        let p = project().await;
        let mut a = p.instance_with(
            p.m5,
            HOST_A,
            &[
                PROJECT_INSTANCES_CAPABILITY,
                shared::PROJECT_POLICY_CAPABILITY,
            ],
        );
        let mut b = p.instance(p.m6, HOST_B);
        let mut web = p.viewer(p.owner, &[]);
        let mut asking = pane(7, Some(p.m6), Some(HOST_B));
        asking.provider = shared::Provider::Omp;
        p.roster(vec![asking]);
        p.state.sessions.register_pending_decision(
            p.session,
            "omp:7:native:question".to_string(),
            Some(7),
            shared::MutationKind::Question,
        );
        let answer = WebToServer::AnswerOmpQuestion {
            session_id: p.session,
            pane_id: 7,
            tool_use_id: "omp:7:native:question".to_string(),
            dialog_id: Uuid::new_v4(),
            request_id: "omp-answer-1".to_string(),
            answers: Vec::new(),
            cancel: true,
        };

        tokio::join!(
            handle_omp_question_answer(&p.state, &web.id, answer),
            async {
                let message = tokio::time::timeout(Duration::from_secs(5), b.rx.recv())
                    .await
                    .expect("the asking instance receives it")
                    .unwrap();
                assert!(matches!(
                    message,
                    ServerToCli::AnswerOmpQuestion { pane_id: 7, .. }
                ));
                assert!(p.state.sessions.finish_omp_answer(
                    p.session,
                    7,
                    b.id,
                    "omp:7:native:question",
                    "omp-answer-1",
                    crate::session::OmpAnswerReceipt {
                        accepted: true,
                        pending: false,
                        error: None,
                    },
                ));
            },
        );
        assert!(web.drain().iter().any(|message| matches!(
            message,
            ServerToWeb::MutationAck { request_id, accepted: true, .. } if request_id == "omp-answer-1"
        )));
        assert!(a.drain().is_empty());
    }

    /// A project hosted in another account's cluster, viewed by a project
    /// member whose membership of that cluster covers only zoo-005.
    async fn hosted_project() -> (Project, Uuid) {
        let owner = Uuid::new_v4();
        let host = Uuid::new_v4();
        let member = Uuid::new_v4();
        let p = project_on(owner, host, &[owner, host, member]).await;
        p.state
            .db
            .add_project_member(&owner.to_string(), &p.project_id(), &member.to_string())
            .await
            .unwrap();
        let token_hash = Uuid::new_v4().to_string();
        p.state
            .db
            .create_shared_cluster_invitation(
                &Uuid::new_v4().to_string(),
                &token_hash,
                &host.to_string(),
                &format!("{member}@example.test"),
                &(chrono::Utc::now() + chrono::Duration::hours(1))
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string(),
            )
            .await
            .unwrap();
        p.state
            .db
            .accept_shared_cluster_invitation(&token_hash, &member.to_string())
            .await
            .unwrap()
            .unwrap();
        p.state
            .db
            .update_cluster_member_policy(
                &host.to_string(),
                &member.to_string(),
                Some(vec![p.m5.to_string()]),
                None,
            )
            .await
            .unwrap()
            .unwrap();
        (p, member)
    }

    /// Runtime access is evaluated per machine: the member may attach while
    /// zoo-005 runs an instance and use its tabs, is refused zoo-006's tabs
    /// and reboot by name, and is told which instances they may use. With
    /// only zoo-006 running they may not use the project at all.
    #[tokio::test]
    async fn runtime_access_is_evaluated_per_machine() {
        let (p, member) = hosted_project().await;
        let mut a = p.instance(p.m5, HOST_A);
        let mut b = p.instance(p.m6, HOST_B);
        let mut web = p.viewer(member, &[PROJECT_INSTANCES_CAPABILITY]);
        p.roster(vec![
            pane(3, Some(p.m5), Some(HOST_A)),
            pane(4, Some(p.m6), Some(HOST_B)),
        ]);

        assert!(has_session_runtime_access(&p.state, &p.session, &member).await);
        assert_eq!(
            resolve_pane_session(&p.state, &web.id, Some(p.session), None, 3).await,
            Some(p.session)
        );
        assert_eq!(
            resolve_pane_session(&p.state, &web.id, Some(p.session), None, 4).await,
            None
        );
        let refused = errors(&web.drain());
        assert_eq!(refused.len(), 1);
        assert!(
            refused[0].starts_with("Access denied") && refused[0].contains(HOST_B),
            "{refused:?}"
        );
        handle_reboot_cli(&p.state, &web.id, p.session, Some(p.m6)).await;
        assert!(errors(&web.drain())[0].starts_with("Access denied"));
        assert!(b.drain().is_empty());

        // The instance set this viewer is sent says which instances it may use,
        // both when replayed on attach and when broadcast on a change.
        replay_instances_after_attach(&p.state, &web.id, p.session).await;
        let access = |message: &ServerToWeb| match message {
            ServerToWeb::SessionInstances { instances, .. } => Some(
                instances
                    .iter()
                    .map(|instance| (instance.machine_id, instance.runtime_access))
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        };
        let expected = vec![(Some(p.m5), Some(true)), (Some(p.m6), Some(false))];
        assert_eq!(
            web.delivered(&p.state).await.iter().find_map(access),
            Some(expected.clone())
        );
        let broadcast = ServerToWeb::SessionInstances {
            session_id: p.session,
            instances: p.state.sessions.session_instances(&p.session),
        };
        assert_eq!(
            access(&broadcast).unwrap()[1].1,
            None,
            "broadcast unevaluated"
        );
        let delivered = with_viewer_runtime_access(&p.state, &web.id, broadcast).await;
        assert_eq!(access(&delivered), Some(expected));

        // A narrowed membership keeps the viewer while an instance it may
        // use runs, and detaches it once none does.
        let allowed = HashSet::from([p.m5]);
        assert_eq!(
            p.state
                .sessions
                .enforce_cluster_machine_access_for_user(p.host, member, Some(&allowed))
                .await,
            0
        );
        assert!(p
            .state
            .sessions
            .is_web_attached_to_session(&p.session, &web.id));
        p.state
            .sessions
            .detach_cli_instance(&p.session, &a.id)
            .unwrap();
        assert!(!has_session_runtime_access(&p.state, &p.session, &member).await);
        // With zoo-005 gone between the session's check and a pane control,
        // the pane check still evaluates zoo-006 in its own right.
        assert!(pane_control_refusal(&p.state, &web.id, &p.session, 4)
            .await
            .is_some_and(|refusal| refusal.starts_with("Access denied")));
        assert_eq!(
            p.state
                .sessions
                .enforce_cluster_machine_access_for_user(p.host, member, Some(&allowed))
                .await,
            1
        );
        assert!(!p
            .state
            .sessions
            .is_web_attached_to_session(&p.session, &web.id));
        assert!(a.drain().is_empty());
    }

    /// An answer naming no pane whose decision the server no longer holds,
    /// as after a server restart, goes to the primary only for a viewer who
    /// may use the primary's machine. Passing the session's check only shows
    /// that some instance admits them.
    #[tokio::test]
    async fn an_unresolved_answer_reaches_the_primary_only_for_a_viewer_allowed_there() {
        let (p, member) = hosted_project().await;
        let mut b = p.instance(p.m6, HOST_B);
        let mut a = p.instance(p.m5, HOST_A);
        let member_web = p.viewer(member, &[]);
        let answer = |connection_id: Uuid| {
            let state = p.state.clone();
            let session = p.session;
            async move {
                route_decision_answer(
                    &state,
                    &connection_id,
                    &session,
                    None,
                    ServerToCli::AnswerQuestion {
                        session_id: session,
                        tool_use_id: "toolu-restarted".to_string(),
                        answers: std::collections::HashMap::from([(
                            "Which?".to_string(),
                            "This".to_string(),
                        )]),
                    },
                )
                .await
            }
        };

        assert!(has_session_runtime_access(&p.state, &p.session, &member).await);
        let refused = answer(member_web.id).await.unwrap_err();
        assert!(
            refused.starts_with("Access denied") && refused.contains(HOST_B),
            "{refused}"
        );
        assert!(b.drain().is_empty() && a.drain().is_empty());

        // With zoo-005, which the member may use, as the primary, it goes there.
        p.state
            .sessions
            .detach_cli_instance(&p.session, &b.id)
            .unwrap();
        answer(member_web.id).await.unwrap();
        assert!(matches!(
            a.drain().as_slice(),
            [ServerToCli::AnswerQuestion { .. }]
        ));
        assert!(b.drain().is_empty());
    }

    /// Forgetting a stopped machine drops its tagged tabs and the untagged
    /// ones attributed to its host from the roster, persists that and tells
    /// viewers. It tells no instance: one sharing the `.apas` would delete the
    /// entries that machine restores its tabs from when it runs the project
    /// again.
    #[tokio::test]
    async fn forgetting_a_stopped_machine_drops_its_tabs_from_the_roster_only() {
        let p = project().await;
        let mut a = p.instance(p.m5, HOST_A);
        let _d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let mut web = p.viewer(p.owner, &[]);
        p.roster(vec![
            pane(3, Some(p.m5), Some(HOST_A)),
            pane(4, Some(p.m6), Some(HOST_B)),
            pane(5, None, Some(HOST_B)),
            pane(6, None, Some(HOST_A)),
        ]);
        p.state
            .sessions
            .append_terminal_output(&p.session, 4, None, b"last frame", 1);
        p.state.sessions.register_pending_decision(
            p.session,
            "approval-5".to_string(),
            Some(5),
            shared::MutationKind::Approval,
        );

        forget_machine_panes(&p.state, &web.id, p.session, p.m6).await;

        assert_eq!(p.pane_ids(), vec![3, 6]);
        let persisted = p
            .state
            .storage
            .load_pane_roster(&p.session)
            .await
            .unwrap()
            .expect("the roster is persisted");
        assert_eq!(
            persisted
                .iter()
                .map(|pane| pane.pane_id)
                .collect::<Vec<_>>(),
            vec![3, 6]
        );
        let received = web.drain();
        assert!(errors(&received).is_empty(), "{received:?}");
        assert!(received.iter().any(|message| matches!(
            message,
            ServerToWeb::PaneList { panes, .. }
                if panes.iter().map(|pane| pane.pane_id).collect::<Vec<_>>() == vec![3, 6]
        )));
        let to_running_instance = a.drain();
        assert!(
            to_running_instance.is_empty(),
            "the running instance is not told: {to_running_instance:?}"
        );
        assert!(p.state.sessions.terminal_snapshot(&p.session, 4).is_none());
        assert!(p
            .state
            .sessions
            .claim_pending_decision(
                p.session,
                "approval-5",
                Some(5),
                shared::MutationKind::Approval,
            )
            .is_none());
    }

    /// Only someone who may change project settings can forget a machine,
    /// never while that machine runs an instance (which would report its
    /// tabs again), nor while a CLI that reported no machine runs on its host.
    #[tokio::test]
    async fn forgetting_is_refused_without_authority_or_while_the_machine_runs() {
        let owner = Uuid::new_v4();
        let member = Uuid::new_v4();
        let p = project_on(owner, owner, &[owner, member]).await;
        p.state
            .db
            .add_project_member(&owner.to_string(), &p.project_id(), &member.to_string())
            .await
            .unwrap();
        let mut a = p.instance(p.m5, HOST_A);
        let mut b = p.instance(p.m6, HOST_B);
        let mut owner_web = p.viewer(owner, &[]);
        let mut member_web = p.viewer(member, &[]);
        let roster = vec![
            pane(3, Some(p.m5), Some(HOST_A)),
            pane(4, Some(p.m6), Some(HOST_B)),
        ];
        p.roster(roster.clone());

        forget_machine_panes(&p.state, &member_web.id, p.session, p.m6).await;
        assert!(errors(&member_web.drain())[0].starts_with("Only the project owner"));
        forget_machine_panes(&p.state, &owner_web.id, p.session, p.m6).await;
        assert_eq!(
            errors(&owner_web.drain()),
            vec![format!(
                "{HOST_B} is running this project. Stop it there before forgetting its tabs."
            )]
        );
        assert_eq!(p.pane_ids(), vec![3, 4]);
        assert!(a.drain().is_empty() && b.drain().is_empty());

        // A CLI without the capability that reported no machine owns every
        // pane it reports; on zoo-006's host it may be zoo-006 itself.
        p.state
            .sessions
            .detach_cli_instance(&p.session, &a.id)
            .unwrap();
        p.state
            .sessions
            .detach_cli_instance(&p.session, &b.id)
            .unwrap();
        let older = p.cli(&[]);
        assert!(matches!(
            p.state.sessions.attach_cli_instance(
                p.session,
                older.id,
                None,
                None,
                Some(HOST_B.to_string()),
            ),
            AttachOutcome::Attached { .. }
        ));
        p.roster(roster.clone());
        forget_machine_panes(&p.state, &owner_web.id, p.session, p.m6).await;
        assert_eq!(errors(&owner_web.drain()).len(), 1);
        assert_eq!(p.pane_ids(), vec![3, 4]);

        // A capable instance under another machine id on that host, such as
        // the same host after its machine id changed, reports its own tabs
        // tagged and does not keep the old id's tabs from being forgotten.
        p.state
            .sessions
            .detach_cli_instance(&p.session, &older.id)
            .unwrap();
        let mut renamed = p.instance(Uuid::new_v4(), HOST_B);
        p.roster(roster);
        forget_machine_panes(&p.state, &owner_web.id, p.session, p.m6).await;
        assert!(errors(&owner_web.drain()).is_empty());
        assert_eq!(p.pane_ids(), vec![3]);
        // Forgetting changes the roster alone; no instance is told.
        assert!(renamed.drain().is_empty());
    }

    /// After a server restart a stopped machine's tabs are known only from
    /// `panes.json`, with the host they were last reported under; forgetting
    /// that machine loads them first and never saves the roster empty.
    #[tokio::test]
    async fn forgetting_after_a_restart_loads_the_persisted_roster() {
        let p = project().await;
        p.state
            .storage
            .save_pane_list(
                &p.session,
                &[
                    pane(3, Some(p.m5), Some(HOST_A)),
                    pane(4, Some(p.m6), Some(HOST_B)),
                    pane(5, None, Some(HOST_B)),
                ],
            )
            .await
            .unwrap();
        let restarted = AppState::new(p.state.db.clone(), p.state.config.clone());
        let id = Uuid::new_v4();
        let (tx, mut rx) = mpsc::channel(64);
        restarted.sessions.register_web(id, tx);
        restarted.sessions.set_web_user(id, p.owner);
        assert!(restarted
            .sessions
            .attach_web_to_session(&p.session, id, None));
        restarted
            .sessions
            .set_session_project(p.session, p.project_id());

        forget_machine_panes(&restarted, &id, p.session, p.m6).await;

        let ids = |panes: &[PaneConfig]| panes.iter().map(|pane| pane.pane_id).collect::<Vec<_>>();
        assert_eq!(
            ids(&restarted.sessions.get_session_panes(&p.session)),
            vec![3]
        );
        let persisted = restarted
            .storage
            .load_pane_roster(&p.session)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ids(&persisted), vec![3]);
        let mut errors_seen = Vec::new();
        while let Ok(message) = rx.try_recv() {
            if let ServerToWeb::Error { message } = message {
                errors_seen.push(message);
            }
        }
        assert!(errors_seen.is_empty(), "{errors_seen:?}");
    }

    /// Closing an untagged tab that the running instance it goes to does not
    /// have removes it from the roster: that instance's reports leave
    /// untagged tabs alone, and nothing else would ever remove it. Its id is
    /// remembered rather than freed, since an untagged entry left in some
    /// `.apas` comes back when an instance adopts it. A tab the instance
    /// reported, and one an older CLI owns, are left to their reports.
    #[tokio::test]
    async fn closing_a_tab_no_instance_runs_removes_it() {
        let p = project().await;
        let mut a = p.instance(p.m5, HOST_A);
        let mut web = p.viewer(p.owner, &[]);
        p.roster(vec![
            pane(2, None, Some(HOST_A)),
            pane(4, None, None),
            pane(7, Some(p.m5), Some(HOST_A)),
        ]);
        p.state
            .sessions
            .merge_instance_roster(&p.session, &a.id, vec![pane(7, None, None)])
            .unwrap();
        assert_eq!(p.pane_ids(), vec![2, 4, 7], "zoo-005's report keeps them");

        for pane_id in [2, 4] {
            handle_remove_pane(&p.state, &web.id, Some(p.session), None, pane_id, None).await;
            assert!(
                matches!(
                    a.drain().as_slice(),
                    [ServerToCli::RemovePane { pane_id: closed, .. }] if *closed == pane_id
                ),
                "pane {pane_id}"
            );
        }
        assert_eq!(p.pane_ids(), vec![7]);
        let persisted = p
            .state
            .storage
            .load_pane_roster(&p.session)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            persisted
                .iter()
                .map(|pane| pane.pane_id)
                .collect::<Vec<_>>(),
            vec![7]
        );
        let received = web.drain();
        assert!(errors(&received).is_empty(), "{received:?}");
        assert!(received.iter().any(|message| matches!(
            message,
            ServerToWeb::PaneList { panes, .. }
                if panes.iter().map(|pane| pane.pane_id).collect::<Vec<_>>() == vec![7]
        )));
        let forgotten = p
            .state
            .sessions
            .get_session(&p.session)
            .unwrap()
            .forgotten_panes;
        assert_eq!(forgotten.get(&2), Some(&None));
        assert_eq!(forgotten.get(&4), Some(&None));

        // zoo-005's own tab goes when its report omits it.
        handle_remove_pane(&p.state, &web.id, Some(p.session), None, 7, None).await;
        assert_eq!(removed_panes(&a.drain()), vec![7]);
        assert_eq!(p.pane_ids(), vec![7]);

        // An older CLI runs every untagged tab, and its report decides.
        p.state
            .sessions
            .detach_cli_instance(&p.session, &a.id)
            .unwrap();
        let mut older = p.instance_with(p.m6, HOST_B, &[]);
        p.roster(vec![pane(3, None, Some(HOST_B))]);
        handle_remove_pane(&p.state, &web.id, Some(p.session), None, 3, None).await;
        assert_eq!(removed_panes(&older.drain()), vec![3]);
        assert_eq!(p.pane_ids(), vec![3]);
    }

    /// The panes `RemovePane` closed among `messages`.
    fn removed_panes(messages: &[ServerToCli]) -> Vec<u32> {
        messages
            .iter()
            .filter_map(|message| match message {
                ServerToCli::RemovePane { pane_id, .. } => Some(*pane_id),
                _ => None,
            })
            .collect()
    }

    /// Reordering tabs before anything has loaded the roster, as after a
    /// server restart while zoo-005's instance is attaching, loads it first.
    /// The stopped laptop's tab keeps its place in memory and on disk, and
    /// zoo-005's report merges beside it instead of into an empty roster.
    #[tokio::test]
    async fn reordering_before_the_roster_is_loaded_keeps_every_tab() {
        let p = project().await;
        p.state
            .storage
            .save_pane_list(
                &p.session,
                &[
                    pane(517, Some(p.m6), Some(HOST_B)),
                    pane(3, Some(p.m5), Some(HOST_A)),
                ],
            )
            .await
            .unwrap();
        let restarted = AppState::new(p.state.db.clone(), p.state.config.clone());
        restarted
            .sessions
            .set_session_project(p.session, p.project_id());
        let cli = Uuid::new_v4();
        let (cli_tx, _cli_rx) = mpsc::channel(16);
        restarted.sessions.register_cli(cli, p.owner, cli_tx, None);
        restarted.sessions.set_cli_capabilities(
            cli,
            CAPABLE_CLI
                .iter()
                .map(|capability| capability.to_string())
                .collect(),
        );
        assert!(matches!(
            restarted.sessions.attach_cli_instance(
                p.session,
                cli,
                Some(p.m5),
                Some(format!("/work/{HOST_A}")),
                Some(HOST_A.to_string()),
            ),
            AttachOutcome::Attached { .. }
        ));
        let web = Uuid::new_v4();
        let (web_tx, _web_rx) = mpsc::channel(64);
        restarted.sessions.register_web(web, web_tx);
        restarted.sessions.set_web_user(web, p.owner);
        assert!(restarted
            .sessions
            .attach_web_to_session(&p.session, web, None));
        let ids = |panes: &[PaneConfig]| panes.iter().map(|pane| pane.pane_id).collect::<Vec<_>>();

        handle_reorder_panes(&restarted, &web, Some(p.session), None, vec![3, 517]).await;

        assert_eq!(
            ids(&restarted.sessions.get_session_panes(&p.session)),
            vec![3, 517]
        );
        let persisted = restarted
            .storage
            .load_pane_roster(&p.session)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ids(&persisted), vec![3, 517]);
        let merged = restarted
            .sessions
            .merge_instance_roster(&p.session, &cli, vec![pane(3, None, None)])
            .unwrap();
        assert_eq!(ids(&merged), vec![3, 517]);
        assert_eq!(merged[1].machine_id, Some(p.m6));
    }

    /// Forgetting a separate checkout's tabs does not free their ids. New
    /// tabs elsewhere get other ids, before and after a server restart, so
    /// when that checkout runs the project again its tabs come back instead
    /// of being dropped as another machine's. Once back, their ids are in
    /// use rather than remembered.
    #[tokio::test]
    async fn a_forgotten_machines_tabs_come_back_with_their_ids() {
        let p = project().await;
        let mut a = p.running(p.m5, HOST_A);
        let mut web = p.requester();
        let forgotten: Vec<u32> = (40..50).collect();
        let fresh: Vec<u32> = (50..60).collect();
        let free: Vec<u32> = forgotten.iter().chain(&fresh).copied().collect();
        let mut roster = crowded_roster(&p, &free);
        roster.extend(
            forgotten
                .iter()
                .map(|pane_id| pane(*pane_id, Some(p.m6), Some(HOST_B))),
        );
        p.roster(roster);

        forget_machine_panes(&p.state, &web.id, p.session, p.m6).await;
        assert!(errors(&web.drain()).is_empty());
        let mut remembered: Vec<u32> = p
            .state
            .storage
            .load_forgotten_panes(&p.session)
            .await
            .unwrap()
            .iter()
            .map(|forgotten| {
                assert_eq!(forgotten.machine_id, Some(p.m6));
                forgotten.pane_id
            })
            .collect();
        remembered.sort();
        assert_eq!(remembered, forgotten, "saved for after a restart");

        // After a restart the ids are read back with the roster.
        let restarted = AppState::new(p.state.db.clone(), p.state.config.clone());
        restarted
            .sessions
            .attach_web_to_session(&p.session, Uuid::new_v4(), None);
        assert!(resolve_pane_roster(&restarted, &p.session).await.is_some());
        let mut allocated: Vec<u32> = (0..fresh.len())
            .map(|_| {
                restarted
                    .sessions
                    .allocate_pane_id(&p.session, Some(p.m5))
                    .unwrap()
            })
            .collect();
        allocated.sort();
        assert_eq!(allocated, fresh, "the restarted server avoids them too");

        // zoo-005's new tabs take the free ids and it reports them.
        for request in 0..fresh.len() {
            p.add_pane(&web, Some(p.m5), Some(&format!("new-{request}")))
                .await;
        }
        let mut added: Vec<u32> = added_panes(&a.drain())
            .iter()
            .map(|pane| pane.pane_id)
            .collect();
        added.sort();
        assert_eq!(added, fresh);
        let zoo_005: Vec<PaneConfig> = p
            .state
            .sessions
            .get_session_panes(&p.session)
            .into_iter()
            .chain(
                fresh
                    .iter()
                    .map(|pane_id| pane(*pane_id, Some(p.m5), Some(HOST_A))),
            )
            .collect();
        p.state
            .sessions
            .merge_instance_roster(&p.session, &a.id, zoo_005)
            .unwrap();

        // The checkout runs the project again and reports its tabs: each one
        // is back as its own, none dropped as zoo-005's.
        let laptop = p.instance(p.m6, HOST_B);
        let merged = p
            .state
            .sessions
            .merge_instance_roster(
                &p.session,
                &laptop.id,
                forgotten
                    .iter()
                    .map(|pane_id| pane(*pane_id, None, None))
                    .collect(),
            )
            .unwrap();
        for pane_id in &forgotten {
            let back = merged
                .iter()
                .find(|pane| pane.pane_id == *pane_id)
                .expect("the tab is back");
            assert_eq!(back.machine_id, Some(p.m6), "pane {pane_id}");
        }
        crate::routes::ws_cli::persist_pane_roster(&p.state, &p.session).await;
        assert!(p
            .state
            .storage
            .load_forgotten_panes(&p.session)
            .await
            .unwrap()
            .is_empty());
    }

    /// A web edit of one tab changes that tab in place, then saves and shows
    /// the roster as it stands, another machine's tab reported meanwhile
    /// included. An edit of a tab the roster does not list changes nothing.
    #[tokio::test]
    async fn a_web_edit_of_one_tab_keeps_every_other_tab() {
        let p = project().await;
        let a = p.instance(p.m5, HOST_A);
        let b = p.instance(p.m6, HOST_B);
        let mut web = p.viewer(p.owner, &[]);
        p.state
            .sessions
            .merge_instance_roster(&p.session, &a.id, vec![pane(3, None, None)])
            .unwrap();
        // zoo-006 reports a new tab after the rename was asked for.
        p.state
            .sessions
            .merge_instance_roster(&p.session, &b.id, vec![pane(9, None, None)])
            .unwrap();

        assert!(
            edit_roster_pane(&p.state, p.session, 3, |pane| {
                pane.label = Some("renamed".to_string());
            })
            .await
        );
        let summary = |panes: &[PaneConfig]| {
            panes
                .iter()
                .map(|pane| (pane.pane_id, pane.machine_id, pane.label.clone()))
                .collect::<Vec<_>>()
        };
        let expected = vec![
            (3, Some(p.m5), Some("renamed".to_string())),
            (9, Some(p.m6), None),
        ];
        assert_eq!(
            summary(&p.state.sessions.get_session_panes(&p.session)),
            expected
        );
        let persisted = p
            .state
            .storage
            .load_pane_roster(&p.session)
            .await
            .unwrap()
            .expect("the roster is saved");
        assert_eq!(summary(&persisted), expected);
        let lists: Vec<Vec<PaneConfig>> = web
            .drain()
            .into_iter()
            .filter_map(|message| match message {
                ServerToWeb::PaneList { panes, .. } => Some(panes),
                _ => None,
            })
            .collect();
        assert_eq!(lists.len(), 1);
        assert_eq!(summary(&lists[0]), expected);

        assert!(!edit_roster_pane(&p.state, p.session, 7, |pane| pane.label = None).await);
        assert!(web.drain().is_empty());
        assert_eq!(
            summary(&p.state.sessions.get_session_panes(&p.session)),
            expected
        );
    }

    /// A viewer arriving while `panes.json` cannot be read neither treats the
    /// roster as known nor lets panes inferred from history become it, and a
    /// report merged meanwhile is shown but not saved: either would be saved
    /// over the tabs the file holds. Once the file reads, its tabs and the
    /// report are the roster, and every instance is asked to report again.
    #[tokio::test]
    async fn a_roster_that_cannot_be_read_is_not_seeded_from_a_viewer() {
        let p = project().await;
        let file = p
            .dir
            .join("sessions")
            .join(p.session.to_string())
            .join("panes.json");
        // A directory in its place makes every read of the file fail.
        std::fs::create_dir_all(&file).unwrap();
        let _web = p.viewer(p.owner, &[]);
        let ids = |roster: Option<Vec<PaneConfig>>| {
            roster.map(|panes| panes.iter().map(|pane| pane.pane_id).collect::<Vec<_>>())
        };

        assert_eq!(ids(resolve_pane_roster(&p.state, &p.session).await), None);
        assert!(p.state.sessions.pane_roster_unreadable(&p.session));
        assert!(!p
            .state
            .sessions
            .seed_inferred_pane_roster(&p.session, vec![pane(2, None, None)]));
        assert!(
            p.pane_ids().is_empty(),
            "no inferred pane became the roster"
        );

        let mut a = p.instance(p.m5, HOST_A);
        p.state
            .sessions
            .merge_instance_roster(&p.session, &a.id, vec![pane(3, None, None)])
            .unwrap();
        crate::routes::ws_cli::persist_pane_roster(&p.state, &p.session).await;
        assert!(file.is_dir(), "nothing was saved over the file");
        assert_eq!(
            ids(resolve_pane_roster(&p.state, &p.session).await),
            Some(vec![3]),
            "viewers see the report meanwhile"
        );
        assert!(!p.state.sessions.pane_roster_seeded(&p.session));

        std::fs::remove_dir(&file).unwrap();
        p.state
            .storage
            .save_pane_list(&p.session, &[pane(4, Some(p.m6), Some(HOST_B))])
            .await
            .unwrap();
        assert_eq!(
            ids(resolve_pane_roster(&p.state, &p.session).await),
            Some(vec![4, 3])
        );
        assert!(p.state.sessions.pane_roster_seeded(&p.session));
        assert!(a
            .drain()
            .iter()
            .any(|message| matches!(message, ServerToCli::RequestPaneList { .. })));
    }

    /// The project view's Start starts each online, capable machine that owns
    /// tabs, counting an untagged tab for its attributed host, and falls back
    /// to the machine it named only when none qualifies. The Machines page's
    /// per-machine Start starts that machine alone.
    #[tokio::test]
    async fn a_project_wide_start_starts_each_machine_that_owns_tabs() {
        let p = project().await;
        let m7 = Uuid::new_v4();
        let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
        let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let mut d7 = p.daemon(m7, HOST_C, CAPABLE_DAEMON);
        let mut web = p.viewer(p.owner, &[shared::PROJECT_POLICY_CAPABILITY]);
        p.roster(vec![
            pane(3, Some(p.m5), Some(HOST_A)),
            pane(4, None, Some(HOST_B)),
        ]);
        let project_id = p.project_id();
        let start = |machine_id: Uuid, all_instances: bool| {
            start_machine_project(
                &p.state,
                &web.id,
                &p.owner,
                machine_id,
                &project_id,
                all_instances,
            )
        };

        start(m7, true).await;
        assert!(started(&mut d5) && started(&mut d6));
        assert!(!started(&mut d7), "zoo-007 owns no tabs");

        start(m7, false).await;
        assert!(started(&mut d7));
        assert!(!started(&mut d5) && !started(&mut d6));

        // A daemon that cannot run an instance beside others does not qualify.
        p.state
            .sessions
            .set_daemon_capabilities(p.m6, vec![shared::PROJECT_POLICY_CAPABILITY.to_string()]);
        start(m7, true).await;
        assert!(started(&mut d5));
        assert!(!started(&mut d6) && !started(&mut d7));

        // With no tabs anywhere, the named machine starts.
        p.roster(Vec::new());
        start(p.m6, true).await;
        assert!(started(&mut d6));
        assert!(!started(&mut d5) && !started(&mut d7));
        assert!(errors(&web.drain()).is_empty());
    }

    /// The project view's Close stops every machine with an instance
    /// attached, as well as the machine it named; a per-machine Stop stops
    /// that machine alone.
    #[tokio::test]
    async fn a_project_wide_close_stops_every_attached_instance() {
        let p = project().await;
        let m7 = Uuid::new_v4();
        let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
        let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let mut d7 = p.daemon(m7, HOST_C, CAPABLE_DAEMON);
        let _a = p.instance(p.m5, HOST_A);
        let _b = p.instance(p.m6, HOST_B);
        let mut web = p.viewer(p.owner, &[]);

        stop_machine_project(&p.state, &web.id, &p.owner, m7, &p.project_id(), true).await;
        assert!(stopped(&mut d5) && stopped(&mut d6) && stopped(&mut d7));

        stop_machine_project(&p.state, &web.id, &p.owner, p.m5, &p.project_id(), false).await;
        assert!(stopped(&mut d5));
        assert!(!stopped(&mut d6) && !stopped(&mut d7));
        assert!(errors(&web.drain()).is_empty());
    }

    /// A project-wide Start or Close from an account that cannot see the
    /// project is answered as a per-machine one is, with a bare "Machine not
    /// found". It never names the hosts that run the project or own its
    /// tabs, and nothing is started or stopped.
    #[tokio::test]
    async fn a_project_wide_request_from_a_stranger_names_no_host() {
        let p = project().await;
        let stranger = Uuid::new_v4();
        p.state.db.create_user(&user(stranger)).await.unwrap();
        let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
        let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let _a = p.instance(p.m5, HOST_A);
        let _b = p.instance(p.m6, HOST_B);
        p.roster(vec![
            pane(3, Some(p.m5), Some(HOST_A)),
            pane(4, Some(p.m6), Some(HOST_B)),
        ]);
        let mut web = p.viewer(stranger, &[shared::PROJECT_POLICY_CAPABILITY]);
        let elsewhere = Uuid::new_v4();

        for all_instances in [true, false] {
            start_machine_project(
                &p.state,
                &web.id,
                &stranger,
                elsewhere,
                &p.project_id(),
                all_instances,
            )
            .await;
            stop_machine_project(
                &p.state,
                &web.id,
                &stranger,
                elsewhere,
                &p.project_id(),
                all_instances,
            )
            .await;
            assert_eq!(
                errors(&web.drain()),
                vec!["Machine not found".to_string(); 2],
                "all_instances {all_instances}"
            );
        }
        assert!(d5.drain().is_empty() && d6.drain().is_empty());
    }

    /// The project as `host`'s daemon lists it, running or not.
    fn listed(p: &Project, host: &str, running: bool) -> shared::MachineProjectInfo {
        shared::MachineProjectInfo {
            project_id: p.project_id(),
            name: None,
            path: format!("/work/{host}"),
            preferred_machine_id: None,
            is_running: running,
            pid: None,
            memory_kb: None,
            last_error: None,
            git_remote: None,
        }
    }

    /// A connected daemon of `owner`, an account other than the project's
    /// host, on `machine_id`, listing `projects`.
    fn foreign_daemon(
        p: &Project,
        owner: Uuid,
        machine_id: Uuid,
        host: &str,
        projects: Vec<shared::MachineProjectInfo>,
    ) -> Endpoint<ServerToDaemon> {
        let (tx, rx) = mpsc::channel(16);
        p.state.sessions.register_daemon(
            machine_id,
            owner,
            tx,
            shared::MachineInfo {
                machine_id,
                hostname: host.to_string(),
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
                daemon_version: None,
                deepseek_backend: None,
                last_seen: None,
            },
            projects,
        );
        p.state.sessions.set_daemon_capabilities(
            machine_id,
            CAPABLE_DAEMON
                .iter()
                .map(|capability| capability.to_string())
                .collect(),
        );
        Endpoint { id: machine_id, rx }
    }

    /// The project view's Close also stops a machine whose connected daemon
    /// reports running the project while its instance is not attached, one
    /// starting or reconnecting: its machine entry shows the project running,
    /// and stopping only the attached instances would leave its agents
    /// running, to come back when it reattached. A daemon that lists the
    /// project without running it, a disconnected one, and another account's
    /// that runs a checkout carrying the same id are left alone. A
    /// per-machine Stop still stops its machine alone.
    #[tokio::test]
    async fn a_project_wide_close_stops_a_machine_whose_instance_is_not_attached() {
        let p = project().await;
        let m7 = Uuid::new_v4();
        let m8 = Uuid::new_v4();
        let foreign = Uuid::new_v4();
        let stranger = Uuid::new_v4();
        p.state.db.create_user(&user(stranger)).await.unwrap();
        let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
        let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let mut d7 = p.daemon(m7, HOST_C, CAPABLE_DAEMON);
        let mut d8 = p.daemon(m8, "zoo-008", CAPABLE_DAEMON);
        let mut df = foreign_daemon(
            &p,
            stranger,
            foreign,
            "zoo-009",
            vec![listed(&p, "zoo-009", true)],
        );
        p.state
            .sessions
            .update_daemon_projects(&p.m6, vec![listed(&p, HOST_B, true)]);
        // zoo-008 ran it, and its daemon is gone.
        p.state
            .sessions
            .update_daemon_projects(&m8, vec![listed(&p, "zoo-008", true)]);
        p.state.sessions.unregister_daemon(&m8);
        let mut web = p.viewer(p.owner, &[]);
        let viewer = web.id;
        let project_id = p.project_id();
        let close = |machine_id: Uuid, all_instances: bool| {
            stop_machine_project(
                &p.state,
                &viewer,
                &p.owner,
                machine_id,
                &project_id,
                all_instances,
            )
        };

        // Nothing attached: the named machine and the one reporting it.
        close(p.m5, true).await;
        assert!(stopped(&mut d5) && stopped(&mut d6));
        assert!(!stopped(&mut d7), "listed, not running");
        assert!(d8.drain().is_empty(), "disconnected");
        assert!(df.drain().is_empty(), "another account's checkout");

        // The primary attached, zoo-006 reconnecting.
        let _a = p.instance(p.m5, HOST_A);
        close(p.m5, true).await;
        assert!(stopped(&mut d5) && stopped(&mut d6));
        assert!(!stopped(&mut d7) && d8.drain().is_empty() && df.drain().is_empty());

        // A per-machine Stop stops that machine alone.
        close(p.m5, false).await;
        assert!(stopped(&mut d5));
        assert!(!stopped(&mut d6) && !stopped(&mut d7));
        assert!(
            errors(&web.drain()).is_empty(),
            "no refusal for any machine left alone"
        );
    }

    /// A project-wide Close never names the host of the machine it was sent
    /// with unless the requester knows of that machine. Another account's
    /// machine is refused by its id, exactly as an id no daemon ever used.
    #[tokio::test]
    async fn a_project_wide_close_does_not_name_a_machine_it_was_sent_with_for_a_stranger() {
        let p = project().await;
        let stranger = Uuid::new_v4();
        p.state.db.create_user(&user(stranger)).await.unwrap();
        let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
        let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let _a = p.instance(p.m5, HOST_A);
        let _b = p.instance(p.m6, HOST_B);
        let foreign = Uuid::new_v4();
        let mut df = foreign_daemon(&p, stranger, foreign, HOST_C, Vec::new());
        let unknown = Uuid::new_v4();
        let mut web = p.viewer(p.owner, &[]);

        for machine in [foreign, unknown] {
            stop_machine_project(&p.state, &web.id, &p.owner, machine, &p.project_id(), true).await;
            assert_eq!(
                errors(&web.drain()),
                vec![format!("{machine}: Machine not found")]
            );
            assert!(stopped(&mut d5) && stopped(&mut d6));
        }
        p.state.sessions.unregister_daemon(&foreign);
        stop_machine_project(&p.state, &web.id, &p.owner, foreign, &p.project_id(), true).await;
        assert_eq!(
            errors(&web.drain()),
            vec![format!("{foreign}: Machine not found")]
        );
        assert!(df.drain().is_empty());
    }

    /// The text a reboot request naming `machine` is refused with.
    async fn reboot_refusal(p: &Project, web: &mut Endpoint<ServerToWeb>, machine: Uuid) -> String {
        handle_reboot_cli(&p.state, &web.id, p.session, Some(machine)).await;
        let refused = errors(&web.drain());
        assert_eq!(refused.len(), 1, "{refused:?}");
        refused[0].clone()
    }

    /// The text a lifecycle request naming `machine` fails with.
    async fn lifecycle_refusal(
        p: &Project,
        web: &mut Endpoint<ServerToWeb>,
        machine: Uuid,
    ) -> String {
        let request = Uuid::new_v4();
        handle_cli_lifecycle_request(
            &p.state,
            &web.id,
            p.session,
            p.owner,
            request,
            shared::CliLifecycleOperation::RebootCli,
            Some(machine),
        )
        .await;
        web.drain()
            .into_iter()
            .find_map(|message| match message {
                ServerToWeb::CliLifecycleStatus {
                    request_id,
                    phase: shared::CliLifecyclePhase::Failed,
                    message: Some(text),
                    ..
                } if request_id == request => Some(text),
                _ => None,
            })
            .expect("the request failed")
    }

    /// A reboot or lifecycle request naming a machine that runs no instance
    /// of the session names that machine's host only when the requester
    /// knows of it: one of their own machines, or one a tab is tagged for.
    /// Another account's machine is refused exactly as an id no daemon ever
    /// used, before and after its daemon disconnects: machine ids derive from
    /// `user@host`, and naming its host would confirm a guessed one.
    #[tokio::test]
    async fn a_lifecycle_refusal_names_only_a_machine_the_requester_knows() {
        let p = project().await;
        let mut a = p.instance(p.m5, HOST_A);
        let _d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let stranger = Uuid::new_v4();
        p.state.db.create_user(&user(stranger)).await.unwrap();
        let foreign = Uuid::new_v4();
        let _df = foreign_daemon(&p, stranger, foreign, HOST_C, Vec::new());
        let tagged = Uuid::new_v4();
        let _dt = foreign_daemon(&p, stranger, tagged, "zoo-008", Vec::new());
        p.roster(vec![
            pane(3, Some(p.m5), Some(HOST_A)),
            pane(4, Some(tagged), Some("zoo-008")),
        ]);
        let unknown = Uuid::new_v4();
        let mut web = p.viewer(p.owner, &[PROJECT_INSTANCES_CAPABILITY]);
        let anonymous = "This tab's machine is not running this project.".to_string();

        assert_eq!(reboot_refusal(&p, &mut web, unknown).await, anonymous);
        assert_eq!(lifecycle_refusal(&p, &mut web, unknown).await, anonymous);
        assert_eq!(reboot_refusal(&p, &mut web, foreign).await, anonymous);
        assert_eq!(lifecycle_refusal(&p, &mut web, foreign).await, anonymous);
        p.state.sessions.unregister_daemon(&foreign);
        assert_eq!(reboot_refusal(&p, &mut web, foreign).await, anonymous);
        assert_eq!(lifecycle_refusal(&p, &mut web, foreign).await, anonymous);

        // The requester's own machine, and a machine a tab is tagged for.
        assert_eq!(
            reboot_refusal(&p, &mut web, p.m6).await,
            not_running(HOST_B)
        );
        assert_eq!(
            lifecycle_refusal(&p, &mut web, p.m6).await,
            not_running(HOST_B)
        );
        assert_eq!(
            reboot_refusal(&p, &mut web, tagged).await,
            not_running("zoo-008")
        );
        assert_eq!(
            lifecycle_refusal(&p, &mut web, tagged).await,
            not_running("zoo-008")
        );
        assert!(a.drain().is_empty(), "nothing reached the running instance");
    }

    /// A CLI without instance support is the only instance it knows of, so
    /// a Start on any other machine would only bring up an instance the
    /// server refuses when it attaches, after starting that machine's saved
    /// tabs. Such a Start is refused before any daemon frame, with the reason
    /// that refusal would give, per machine and project-wide alike. A Start
    /// of the older CLI's own machine goes ahead as it always did.
    #[tokio::test]
    async fn a_start_beside_an_older_cli_is_refused_before_any_daemon_frame() {
        let p = project().await;
        let m7 = Uuid::new_v4();
        let older_daemon = &[shared::PROJECT_POLICY_CAPABILITY];
        let mut d5 = p.daemon(p.m5, HOST_A, older_daemon);
        let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let mut d7 = p.daemon(m7, HOST_C, CAPABLE_DAEMON);
        // An older CLI runs the project on zoo-005 and reported no machine.
        let older = p.cli(&[shared::PROJECT_POLICY_CAPABILITY]);
        assert!(matches!(
            p.state.sessions.attach_cli_instance(
                p.session,
                older.id,
                None,
                Some(format!("/work/{HOST_A}")),
                Some(HOST_A.to_string()),
            ),
            AttachOutcome::Attached { .. }
        ));
        p.roster(vec![
            pane(3, Some(p.m6), Some(HOST_B)),
            pane(4, Some(m7), Some(HOST_C)),
        ]);
        let mut web = p.viewer(p.owner, &[shared::PROJECT_POLICY_CAPABILITY]);
        let refusal = format!(
            "This project is running on {HOST_A} with an APAS CLI that cannot share it with other machines; stop it there, or reboot {HOST_A} from the Machines page to update it, then start it here"
        );
        let viewer = web.id;
        let project_id = p.project_id();
        let start = |machine_id: Uuid, all_instances: bool| {
            start_machine_project(
                &p.state,
                &viewer,
                &p.owner,
                machine_id,
                &project_id,
                all_instances,
            )
        };

        start(p.m6, false).await;
        assert_eq!(errors(&web.drain()), vec![refusal.clone()]);
        assert!(d6.drain().is_empty());

        start(p.m5, false).await;
        assert!(started(&mut d5), "its own machine starts as before");
        assert!(errors(&web.drain()).is_empty());

        // Project-wide: each machine that owns tabs is refused by name.
        start(p.m5, true).await;
        assert_eq!(
            errors(&web.drain()),
            vec![
                format!("{}: {refusal}", if p.m6 < m7 { HOST_B } else { HOST_C }),
                format!("{}: {refusal}", if p.m6 < m7 { HOST_C } else { HOST_B }),
            ]
        );
        assert!(d5.drain().is_empty() && d6.drain().is_empty() && d7.drain().is_empty());

        // One that reported its machine is recognized by it.
        p.state
            .sessions
            .detach_cli_instance(&p.session, &older.id)
            .unwrap();
        let older = p.instance_with(p.m5, HOST_A, &[shared::PROJECT_POLICY_CAPABILITY]);
        start(p.m5, false).await;
        assert!(started(&mut d5));
        start(m7, false).await;
        assert_eq!(errors(&web.drain()), vec![refusal.clone()]);
        assert!(d7.drain().is_empty());

        // One with no host could be anywhere, so every machine is refused.
        p.state
            .sessions
            .detach_cli_instance(&p.session, &older.id)
            .unwrap();
        let hostless = p.cli(&[shared::PROJECT_POLICY_CAPABILITY]);
        assert!(matches!(
            p.state
                .sessions
                .attach_cli_instance(p.session, hostless.id, None, None, None),
            AttachOutcome::Attached { .. }
        ));
        start(p.m5, false).await;
        assert_eq!(
            errors(&web.drain()),
            vec![refusal.replace(HOST_A, "another machine")]
        );
        assert!(d5.drain().is_empty());
    }

    /// The other way round: an instance runs the project, and the Start
    /// names a machine whose daemon cannot run instances. That daemon would
    /// skip it without a word on a shared home, or start a CLI the server
    /// refuses when it attaches. It is refused before any daemon frame with
    /// the reason a targeted new tab gets, and goes ahead as before once no
    /// instance runs the project.
    #[tokio::test]
    async fn a_start_on_an_older_daemon_beside_an_instance_is_refused_before_any_daemon_frame() {
        let p = project().await;
        let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
        let mut d6 = p.daemon(p.m6, HOST_B, &[shared::PROJECT_POLICY_CAPABILITY]);
        let running = p.instance(p.m5, HOST_A);
        let mut web = p.viewer(p.owner, &[shared::PROJECT_POLICY_CAPABILITY]);
        let viewer = web.id;
        let project_id = p.project_id();
        let start = |machine_id: Uuid| {
            start_machine_project(&p.state, &viewer, &p.owner, machine_id, &project_id, false)
        };

        start(p.m6).await;
        assert_eq!(
            errors(&web.drain()),
            vec![format!(
                "{HOST_B} runs an older APAS that cannot run this project beside other machines. Reboot {HOST_B} to update it."
            )]
        );
        assert!(d6.drain().is_empty(), "nothing reached the older daemon");

        // The running instance's own machine is not affected.
        start(p.m5).await;
        assert!(started(&mut d5));
        assert!(errors(&web.drain()).is_empty());

        // With no instance running, the older daemon runs the project alone.
        p.state
            .sessions
            .detach_cli_instance(&p.session, &running.id)
            .unwrap();
        start(p.m6).await;
        assert!(started(&mut d6));
        assert!(errors(&web.drain()).is_empty());
    }

    /// The refusals clients show word for word for a tab they cannot use,
    /// pinned to what the server sends. The not-running refusal of an input
    /// is the golden fixture `server-mutation-ack-input.json`, which the web
    /// and native tests compare their copy with; the host-less not-running
    /// refusal and the no-access refusal are the copy in
    /// `packages/web/src/lib/tabHosts.ts` and
    /// `packages/mobile/src/state/projectInstances.ts`. Rewording one here
    /// means rewording those too.
    #[tokio::test]
    async fn the_refusals_clients_mirror_are_what_the_server_sends() {
        let p = project().await;
        let _b = p.instance(p.m6, HOST_B);
        let unnamed = Uuid::new_v4();
        p.roster(vec![
            pane(4, Some(p.m5), Some(HOST_A)),
            pane(5, Some(unnamed), None),
        ]);
        let mut web = p.viewer(p.owner, &[PROJECT_INSTANCES_CAPABILITY]);
        web.drain();

        typed(&p, &web.id, Some(4), Some("msg-1")).await;
        let ack = web
            .drain()
            .into_iter()
            .find(|message| matches!(message, ServerToWeb::MutationAck { .. }))
            .expect("the input was refused under its id");
        let mut sent = serde_json::to_value(&ack).unwrap();
        sent["session_id"] = serde_json::json!("20000000-0000-4000-8000-000000000001");
        let fixture: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../packages/protocol/fixtures/server-mutation-ack-input.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(sent, fixture);

        typed(&p, &web.id, Some(5), None).await;
        assert_eq!(
            errors(&web.drain()),
            vec!["This tab's machine is not running this project.".to_string()]
        );

        let (hosted, member) = hosted_project().await;
        let _a = hosted.instance(hosted.m5, HOST_A);
        let _b = hosted.instance(hosted.m6, HOST_B);
        hosted.roster(vec![pane(4, Some(hosted.m6), Some(HOST_B))]);
        let mut member_web = hosted.viewer(member, &[PROJECT_INSTANCES_CAPABILITY]);
        member_web.drain();
        typed(&hosted, &member_web.id, Some(4), None).await;
        assert_eq!(
            errors(&member_web.drain()),
            vec![format!(
                "Access denied: you cannot use {HOST_B}, which runs this project instance"
            )]
        );
    }

    /// A project only registered on its owner's machine, by `apas register`
    /// or a bare `apas`, has no record or placement on the server until it
    /// first starts. A Start from the Machines page places it as that start's
    /// `SessionStart` would, and starts it. Another account's project that
    /// the machine happens to list, and a suspended one, are still refused.
    #[tokio::test]
    async fn a_project_only_registered_on_its_owners_machine_starts() {
        let p = project().await;
        let other = Uuid::new_v4();
        p.state.db.create_user(&user(other)).await.unwrap();
        let registered = Uuid::new_v4().to_string();
        let foreign = Uuid::new_v4().to_string();
        p.state
            .db
            .authorize_project_registration(&foreign, &other.to_string())
            .await
            .unwrap();
        // Shared with the owner, so its access is not what refuses it.
        let suspended = Uuid::new_v4().to_string();
        p.state
            .db
            .authorize_project_registration(&suspended, &other.to_string())
            .await
            .unwrap();
        p.state
            .db
            .add_project_member(&other.to_string(), &suspended, &p.owner.to_string())
            .await
            .unwrap();
        assert!(p
            .state
            .db
            .set_project_lifecycle(
                &other.to_string(),
                &suspended,
                crate::db::ProjectLifecycle::Suspended,
            )
            .await
            .unwrap());
        let m7 = Uuid::new_v4();
        let listed = |project_id: &str| shared::MachineProjectInfo {
            project_id: project_id.to_string(),
            name: None,
            path: format!("/work/{project_id}"),
            preferred_machine_id: None,
            is_running: false,
            pid: None,
            memory_kb: None,
            last_error: None,
            git_remote: None,
        };
        let mut daemon = p.daemon_with(
            p.owner,
            m7,
            HOST_C,
            CAPABLE_DAEMON,
            vec![listed(&registered), listed(&foreign), listed(&suspended)],
        );
        let mut web = p.viewer(p.owner, &[shared::PROJECT_POLICY_CAPABILITY]);
        let placed = |project_id: String| {
            let db = p.state.db.clone();
            let owner = p.owner.to_string();
            async move {
                db.project_is_placed_in_cluster(&project_id, &owner)
                    .await
                    .unwrap()
            }
        };

        start_machine_project(&p.state, &web.id, &p.owner, m7, &registered, false).await;
        assert!(errors(&web.drain()).is_empty());
        assert!(matches!(
            daemon.drain().as_slice(),
            [ServerToDaemon::StartProjectCli { project_id, .. }] if *project_id == registered
        ));
        assert!(placed(registered.clone()).await);
        assert_eq!(
            p.state
                .db
                .get_project(&registered)
                .await
                .unwrap()
                .unwrap()
                .owner_user_id,
            p.owner.to_string()
        );

        for refused in [&foreign, &suspended] {
            start_machine_project(&p.state, &web.id, &p.owner, m7, refused, false).await;
            assert_eq!(errors(&web.drain()), vec!["Machine not found".to_string()]);
            assert!(daemon.drain().is_empty(), "{refused} is not started");
            assert!(!placed(refused.clone()).await, "{refused} is not placed");
        }
    }

    /// A reboot reaches only the instance on the machine it names, else the
    /// primary; a named machine that runs no instance is refused by name,
    /// and only the target's own panes can block it.
    #[tokio::test]
    async fn a_reboot_reaches_only_the_named_machines_instance() {
        let p = project().await;
        let m7 = Uuid::new_v4();
        let _d7 = p.daemon(m7, HOST_C, CAPABLE_DAEMON);
        let mut a = p.instance(p.m5, HOST_A);
        let mut b = p.instance(p.m6, HOST_B);
        let mut web = p.viewer(p.owner, &[shared::PROJECT_POLICY_CAPABILITY]);
        let mut retired = pane(3, Some(p.m5), Some(HOST_A));
        retired.model = Some("MiniMax-M2.7".to_string());
        p.roster(vec![retired, pane(4, Some(p.m6), Some(HOST_B))]);

        handle_reboot_cli(&p.state, &web.id, p.session, Some(p.m6)).await;
        assert!(matches!(
            b.drain().as_slice(),
            [ServerToCli::RebootCli { .. }]
        ));
        assert!(a.drain().is_empty());
        assert!(errors(&web.drain()).is_empty());

        handle_reboot_cli(&p.state, &web.id, p.session, None).await;
        assert!(errors(&web.drain())[0].contains("retired provider"));
        assert!(
            a.drain().is_empty(),
            "the primary's own retired pane blocks it"
        );

        handle_reboot_cli(&p.state, &web.id, p.session, Some(m7)).await;
        assert_eq!(errors(&web.drain()), vec![not_running(HOST_C)]);
        assert!(a.drain().is_empty() && b.drain().is_empty());
    }

    /// A lifecycle request reaches only the named machine's instance, carries
    /// that instance's inventory, names its machine, and ignores progress
    /// reported by another machine.
    #[tokio::test]
    async fn a_lifecycle_request_is_bound_to_its_machine() {
        let p = project().await;
        let mut a = p.instance(p.m5, HOST_A);
        let mut b = p.instance(p.m6, HOST_B);
        // Only a client that keeps an inventory per machine names one.
        let mut web = p.viewer(
            p.owner,
            &[
                shared::PROJECT_POLICY_CAPABILITY,
                PROJECT_INSTANCES_CAPABILITY,
            ],
        );
        let mut older = p.viewer(p.owner, &[shared::PROJECT_POLICY_CAPABILITY]);
        let inventory = |hosting: bool| shared::CliLifecycleInventory {
            persistent_terminal_hosting: hosting,
            ..shared::CliLifecycleInventory::default()
        };
        p.state
            .sessions
            .set_lifecycle_inventory(p.session, Some(p.m5), inventory(true));
        p.state
            .sessions
            .set_lifecycle_inventory(p.session, Some(p.m6), inventory(false));
        let request = Uuid::new_v4();
        let operation = shared::CliLifecycleOperation::RebootCli;

        handle_cli_lifecycle_request(
            &p.state,
            &web.id,
            p.session,
            p.owner,
            request,
            operation,
            Some(p.m6),
        )
        .await;
        assert!(matches!(
            b.drain().as_slice(),
            [ServerToCli::CliLifecycleRequest { request_id, .. }] if *request_id == request
        ));
        assert!(a.drain().is_empty());
        assert!(web.drain().iter().any(|message| matches!(
            message,
            ServerToWeb::CliLifecycleStatus {
                phase: shared::CliLifecyclePhase::Accepted,
                machine_id: Some(machine),
                inventory: Some(inventory),
                ..
            } if *machine == p.m6 && !inventory.persistent_terminal_hosting
        )));
        // A viewer with one inventory per session, the primary's, sees the
        // request's progress without zoo-006's inventory, which would
        // replace the primary's.
        assert!(older.drain().iter().any(|message| matches!(
            message,
            ServerToWeb::CliLifecycleStatus {
                phase: shared::CliLifecyclePhase::Accepted,
                machine_id: Some(machine),
                inventory: None,
                ..
            } if *machine == p.m6
        )));
        assert!(p
            .state
            .sessions
            .update_lifecycle_request_from_instance(
                request,
                p.session,
                Some(p.m5),
                operation,
                shared::CliLifecyclePhase::Failed,
                None,
                None,
            )
            .is_none());
        assert!(p
            .state
            .sessions
            .update_lifecycle_request_from_instance(
                request,
                p.session,
                Some(p.m6),
                operation,
                shared::CliLifecyclePhase::Succeeded,
                None,
                None,
            )
            .is_some());

        let missing = Uuid::new_v4();
        let unknown_machine = Uuid::new_v4();
        handle_cli_lifecycle_request(
            &p.state,
            &web.id,
            p.session,
            p.owner,
            missing,
            operation,
            Some(unknown_machine),
        )
        .await;
        assert!(web.drain().iter().any(|message| matches!(
            message,
            ServerToWeb::CliLifecycleStatus {
                request_id,
                phase: shared::CliLifecyclePhase::Failed,
                machine_id: Some(machine),
                message: Some(text),
                ..
            } if *request_id == missing
                && *machine == unknown_machine
                && text.contains("is not running this project")
        )));
        assert!(a.drain().is_empty() && b.drain().is_empty());
    }

    /// Right after attaching, a viewer that advertised project instances is
    /// sent the instance set and one inventory per instance; any other
    /// viewer gets no instance set and only the primary's inventory.
    #[tokio::test]
    async fn only_a_viewer_that_asked_is_sent_the_instance_set() {
        let p = project().await;
        let _a = p.instance(p.m5, HOST_A);
        let _b = p.instance(p.m6, HOST_B);
        for machine in [p.m5, p.m6] {
            p.state.sessions.set_lifecycle_inventory(
                p.session,
                Some(machine),
                shared::CliLifecycleInventory::default(),
            );
        }
        let mut capable = p.viewer(p.owner, &[PROJECT_INSTANCES_CAPABILITY]);
        let mut older = p.viewer(p.owner, &[]);

        replay_instances_after_attach(&p.state, &capable.id, p.session).await;
        replay_instances_after_attach(&p.state, &older.id, p.session).await;

        let capable = capable.delivered(&p.state).await;
        assert!(matches!(
            capable.first(),
            Some(ServerToWeb::SessionInstances { instances, .. })
                if instances.len() == 2
                    && instances[0].primary
                    && instances.iter().all(|instance| instance.runtime_access == Some(true))
        ));
        let inventories = |messages: &[ServerToWeb]| {
            messages
                .iter()
                .filter_map(|message| match message {
                    ServerToWeb::CliLifecycleInventory { machine_id, .. } => Some(*machine_id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(inventories(&capable), vec![Some(p.m5), Some(p.m6)]);
        let older = older.drain();
        assert!(!older
            .iter()
            .any(|message| matches!(message, ServerToWeb::SessionInstances { .. })));
        assert_eq!(inventories(&older), vec![Some(p.m5)]);
    }

    /// After a restart the primary can attach first and report its inventory
    /// after a secondary has. A viewer keeping one inventory per session is
    /// replayed nothing until the primary's arrives, never the secondary's:
    /// its reboot would restart the primary while counting the secondary's
    /// tabs.
    #[tokio::test]
    async fn an_older_viewer_is_never_replayed_a_secondarys_inventory() {
        let p = project().await;
        let _a = p.instance(p.m5, HOST_A);
        let _b = p.instance(p.m6, HOST_B);
        p.state.sessions.set_lifecycle_inventory(
            p.session,
            Some(p.m6),
            shared::CliLifecycleInventory::default(),
        );
        let inventories = |messages: &[ServerToWeb]| {
            messages
                .iter()
                .filter_map(|message| match message {
                    ServerToWeb::CliLifecycleInventory { machine_id, .. } => Some(*machine_id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let mut capable = p.viewer(p.owner, &[PROJECT_INSTANCES_CAPABILITY]);
        let mut older = p.viewer(p.owner, &[]);

        replay_instances_after_attach(&p.state, &capable.id, p.session).await;
        replay_instances_after_attach(&p.state, &older.id, p.session).await;
        assert_eq!(inventories(&capable.drain()), vec![Some(p.m6)]);
        assert!(inventories(&older.drain()).is_empty());
        assert!(p.state.sessions.lifecycle_inventory(&p.session).is_none());

        p.state.sessions.set_lifecycle_inventory(
            p.session,
            Some(p.m5),
            shared::CliLifecycleInventory::default(),
        );
        replay_instances_after_attach(&p.state, &older.id, p.session).await;
        assert_eq!(inventories(&older.drain()), vec![Some(p.m5)]);
    }

    /// Start the attach replay for `viewer` and let it run up to its first
    /// wait, as a viewer attaching while an instance changes would.
    async fn replay_in_flight(p: &Project, viewer: Uuid) -> tokio::task::JoinHandle<()> {
        let state = p.state.clone();
        let session = p.session;
        let replay = tokio::spawn(async move {
            replay_instances_after_attach(&state, &viewer, session).await;
        });
        tokio::task::yield_now().await;
        replay
    }

    /// The machines of each instance set a viewer was sent, in order.
    fn instance_sets(messages: &[ServerToWeb]) -> Vec<Vec<Option<Uuid>>> {
        messages
            .iter()
            .filter_map(|message| match message {
                ServerToWeb::SessionInstances { instances, .. } => Some(
                    instances
                        .iter()
                        .map(|instance| instance.machine_id)
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .collect()
    }

    /// An instance that detaches or attaches while a viewer's attach replay
    /// is under way is broadcast to that viewer at once. The replay must not
    /// arrive after that broadcast with the set as it was before: the viewer
    /// keeps the last set it is sent, and would show a running machine's tabs
    /// stopped, or a stopped machine's usable, until the next change.
    #[tokio::test]
    async fn an_instance_changing_during_the_attach_replay_is_not_overtaken() {
        // zoo-006 detaches meanwhile.
        let p = project().await;
        let _a = p.instance(p.m5, HOST_A);
        let b = p.instance(p.m6, HOST_B);
        let mut web = p.viewer(p.owner, &[PROJECT_INSTANCES_CAPABILITY]);
        web.drain();
        let replay = replay_in_flight(&p, web.id).await;
        p.state
            .sessions
            .detach_cli_instance(&p.session, &b.id)
            .unwrap();
        replay.await.unwrap();
        let sets = instance_sets(&web.delivered(&p.state).await);
        assert_eq!(sets.last(), Some(&vec![Some(p.m5)]), "{sets:?}");

        // zoo-006 attaches meanwhile.
        let p = project().await;
        let _a = p.instance(p.m5, HOST_A);
        let mut web = p.viewer(p.owner, &[PROJECT_INSTANCES_CAPABILITY]);
        web.drain();
        let replay = replay_in_flight(&p, web.id).await;
        let _b = p.instance(p.m6, HOST_B);
        replay.await.unwrap();
        let delivered = web.delivered(&p.state).await;
        let sets = instance_sets(&delivered);
        assert_eq!(sets.last(), Some(&vec![Some(p.m5), Some(p.m6)]), "{sets:?}");
        // Still filled in for the viewer on its way out, as broadcasts are.
        assert!(delivered.iter().all(|message| match message {
            ServerToWeb::SessionInstances { instances, .. } => instances
                .iter()
                .all(|instance| instance.runtime_access == Some(true)),
            _ => true,
        }));
    }

    /// A newer inventory an instance reports while a viewer's attach replay
    /// is under way reaches that viewer first; the replay must not follow it
    /// with the older one, which a reboot confirmation would then count.
    #[tokio::test]
    async fn an_inventory_reported_during_the_attach_replay_is_not_overtaken() {
        let p = project().await;
        let _a = p.instance(p.m5, HOST_A);
        let _b = p.instance(p.m6, HOST_B);
        let reported = |persistent_terminal_hosting: bool| shared::CliLifecycleInventory {
            persistent_terminal_hosting,
            ..Default::default()
        };
        p.state
            .sessions
            .set_lifecycle_inventory(p.session, Some(p.m5), reported(false));
        let mut web = p.viewer(p.owner, &[PROJECT_INSTANCES_CAPABILITY]);
        web.drain();

        let replay = replay_in_flight(&p, web.id).await;
        // zoo-005 reports again, and the relay delivers it to the viewer.
        p.state
            .sessions
            .set_lifecycle_inventory(p.session, Some(p.m5), reported(true));
        p.state
            .sessions
            .send_to_web(
                &web.id,
                ServerToWeb::CliLifecycleInventory {
                    session_id: p.session,
                    inventory: reported(true),
                    machine_id: Some(p.m5),
                },
            )
            .await;
        replay.await.unwrap();

        let last = web
            .delivered(&p.state)
            .await
            .into_iter()
            .filter_map(|message| match message {
                ServerToWeb::CliLifecycleInventory {
                    machine_id: Some(machine),
                    inventory,
                    ..
                } if machine == p.m5 => Some(inventory.persistent_terminal_hosting),
                _ => None,
            })
            .last();
        assert_eq!(last, Some(true), "the newer inventory is the last word");
    }

    /// Pane summaries name each tab's machine, show an untagged tab under
    /// its attributed host, and read usage limits from the CLI of the
    /// instance that runs the tab.
    #[tokio::test]
    async fn pane_summaries_carry_each_tabs_machine_and_its_instances_limits() {
        let p = project().await;
        let _a = p.instance(p.m5, HOST_A);
        let b = p.instance(p.m6, HOST_B);
        p.roster(vec![
            pane(3, Some(p.m5), Some(HOST_A)),
            pane(4, Some(p.m6), Some(HOST_B)),
            pane(5, None, Some(HOST_B)),
        ]);
        p.state.sessions.update_usage_limits(
            b.id,
            shared::Provider::Claude,
            shared::UsageLimits {
                five_hour: None,
                seven_day: None,
                fetched_at: None,
                usage_limited: Some(shared::UsageLimited {
                    window: "weekly".to_string(),
                    resets_at: None,
                    model: None,
                }),
            },
        );

        let summaries = pane_summaries(&p.state, &p.session, true);
        let summary = |pane_id: u32| {
            summaries
                .iter()
                .find(|summary| summary.pane_id == pane_id)
                .unwrap()
        };
        assert_eq!(summary(3).machine_id, Some(p.m5));
        assert_eq!(summary(3).hostname.as_deref(), Some(HOST_A));
        assert!(summary(3).usage_limited.is_none());
        assert_eq!(summary(4).machine_id, Some(p.m6));
        assert!(summary(4).usage_limited.is_some());
        assert_eq!(summary(5).machine_id, None);
        assert_eq!(summary(5).hostname.as_deref(), Some(HOST_B));
        assert!(summary(5).usage_limited.is_some());
    }

    /// Stopping the runtime rejects every instance on its own connection and
    /// stops every daemon that lists the project.
    #[tokio::test]
    async fn stopping_the_runtime_rejects_every_instance() {
        let p = project().await;
        let mut d5 = p.daemon(p.m5, HOST_A, CAPABLE_DAEMON);
        let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let mut a = p.instance(p.m5, HOST_A);
        let mut b = p.instance(p.m6, HOST_B);

        p.state.sessions.stop_project_runtime(&p.project_id()).await;

        for cli in [&mut a, &mut b] {
            assert!(cli
                .drain()
                .iter()
                .any(|message| matches!(message, ServerToCli::SessionRejected { .. })));
        }
        assert!(stopped(&mut d5) && stopped(&mut d6));
        assert!(p.state.sessions.session_attachments(&p.session).is_empty());
    }

    /// Deletion's runtime cleanup waits for each machine's purge
    /// acknowledgement, not only the first one's.
    #[tokio::test]
    async fn deletion_waits_for_every_machines_purge_acknowledgement() {
        let p = project().await;
        let capabilities = &[
            PROJECT_INSTANCES_CAPABILITY,
            shared::PANE_HOST_CLEANUP_ACK_CAPABILITY,
        ];
        let mut d5 = p.daemon(p.m5, HOST_A, capabilities);
        let mut d6 = p.daemon(p.m6, HOST_B, capabilities);
        let _a = p.instance(p.m5, HOST_A);
        let _b = p.instance(p.m6, HOST_B);
        let sessions = p.state.sessions.clone();
        let project_id = p.project_id();
        let cleanup =
            tokio::spawn(
                async move { sessions.confirm_project_runtime_stopped(&project_id).await },
            );

        let stop_request = |message: ServerToDaemon| match message {
            ServerToDaemon::StopProjectCli {
                request_id: Some(request_id),
                ..
            } => request_id,
            other => panic!("expected a correlated stop, got {other:?}"),
        };
        // zoo-005 acknowledges at once; zoo-006 holds its acknowledgement.
        let held = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::select! {
                Some(message) = d5.rx.recv() => {
                    p.state.sessions.complete_project_runtime_stop(
                        stop_request(message), p.project_id(), true, 0, None,
                    );
                    stop_request(d6.rx.recv().await.unwrap())
                }
                Some(message) = d6.rx.recv() => stop_request(message),
            }
        })
        .await
        .expect("zoo-006 is asked to stop");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!cleanup.is_finished(), "zoo-006 has not acknowledged yet");
        p.state
            .sessions
            .complete_project_runtime_stop(held, p.project_id(), true, 0, None);
        if let Ok(Some(message)) =
            tokio::time::timeout(Duration::from_millis(200), d5.rx.recv()).await
        {
            p.state.sessions.complete_project_runtime_stop(
                stop_request(message),
                p.project_id(),
                true,
                0,
                None,
            );
        }
        assert_eq!(cleanup.await.unwrap().unwrap(), 2);
    }

    // New panes: targeted at a machine, started there when it is stopped,
    // allocated unique ids, and answered only to the requester.

    fn new_pane() -> NewPaneSpec {
        NewPaneSpec {
            provider: shared::Provider::Claude,
            mode: shared::PaneMode::Interactive,
            kind: shared::PaneKind::Terminal,
            label: None,
            prompt: None,
            model: None,
            isolated_worktree: false,
            role: None,
            goal: None,
            backstory: None,
            plan_review_mode: shared::PlanReviewMode::default(),
            managed: false,
        }
    }

    impl Project {
        /// An instance on `machine_id` that has finished starting, as one
        /// does once its `SessionStart` has been handled.
        fn running_with(
            &self,
            machine_id: Uuid,
            host: &str,
            capabilities: &[&str],
        ) -> Endpoint<ServerToCli> {
            let cli = self.instance_with(machine_id, host, capabilities);
            self.state
                .sessions
                .instance_started(self.session, machine_id, cli.id);
            cli
        }

        fn running(&self, machine_id: Uuid, host: &str) -> Endpoint<ServerToCli> {
            self.running_with(machine_id, host, CAPABLE_CLI)
        }

        /// A viewer of the owner's on a client that correlates its launches.
        fn requester(&self) -> Endpoint<ServerToWeb> {
            self.viewer(
                self.owner,
                &[
                    shared::PROJECT_POLICY_CAPABILITY,
                    PROJECT_INSTANCES_CAPABILITY,
                ],
            )
        }

        async fn add_pane(
            &self,
            web: &Endpoint<ServerToWeb>,
            machine_id: Option<Uuid>,
            request_id: Option<&str>,
        ) {
            handle_add_pane(
                &self.state,
                &web.id,
                Some(self.session),
                None,
                machine_id,
                request_id.map(str::to_string),
                new_pane(),
            )
            .await;
        }

        /// A connected daemon of `owner`'s with `projects` in its inventory.
        fn daemon_with(
            &self,
            owner: Uuid,
            machine_id: Uuid,
            host: &str,
            capabilities: &[&str],
            projects: Vec<shared::MachineProjectInfo>,
        ) -> Endpoint<ServerToDaemon> {
            let (tx, rx) = mpsc::channel(16);
            self.state.sessions.register_daemon(
                machine_id,
                owner,
                tx,
                shared::MachineInfo {
                    machine_id,
                    hostname: host.to_string(),
                    os: "linux".to_string(),
                    arch: "x86_64".to_string(),
                    daemon_version: None,
                    deepseek_backend: None,
                    last_seen: None,
                },
                projects,
            );
            self.state.sessions.set_daemon_capabilities(
                machine_id,
                capabilities
                    .iter()
                    .map(|capability| capability.to_string())
                    .collect(),
            );
            Endpoint { id: machine_id, rx }
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    struct AddResult {
        request_id: String,
        status: shared::AddPaneStatus,
        machine_id: Option<Uuid>,
        pane_id: Option<u32>,
        error: Option<String>,
    }

    fn add_result(message: &ServerToWeb) -> Option<AddResult> {
        match message {
            ServerToWeb::AddPaneResult {
                request_id,
                status,
                machine_id,
                pane_id,
                error,
                ..
            } => Some(AddResult {
                request_id: request_id.clone(),
                status: *status,
                machine_id: *machine_id,
                pane_id: *pane_id,
                error: error.clone(),
            }),
            _ => None,
        }
    }

    fn add_results(messages: &[ServerToWeb]) -> Vec<AddResult> {
        messages.iter().filter_map(add_result).collect()
    }

    fn only(mut results: Vec<AddResult>) -> AddResult {
        assert_eq!(results.len(), 1, "{results:?}");
        results.remove(0)
    }

    async fn next_add_result(web: &mut Endpoint<ServerToWeb>) -> AddResult {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let message = web.rx.recv().await.expect("the requester is connected");
                if let Some(result) = add_result(&message) {
                    return result;
                }
            }
        })
        .await
        .expect("an answer to the new pane")
    }

    fn added_panes(messages: &[ServerToCli]) -> Vec<PaneConfig> {
        messages
            .iter()
            .filter_map(|message| match message {
                ServerToCli::AddPane { pane_config, .. } => Some(pane_config.clone()),
                _ => None,
            })
            .collect()
    }

    async fn next_added_pane(cli: &mut Endpoint<ServerToCli>) -> PaneConfig {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let ServerToCli::AddPane { pane_config, .. } =
                    cli.rx.recv().await.expect("the instance is connected")
                {
                    return pane_config;
                }
            }
        })
        .await
        .expect("the new pane reaches the instance")
    }

    /// The request id of the correlated start the daemon was sent.
    async fn start_request(daemon: &mut Endpoint<ServerToDaemon>) -> Uuid {
        let message = tokio::time::timeout(Duration::from_secs(5), daemon.rx.recv())
            .await
            .expect("the daemon is asked to start the project")
            .unwrap();
        match message {
            ServerToDaemon::StartProjectCli {
                request_id: Some(request_id),
                policy: Some(_),
                ..
            } => request_id,
            other => panic!("expected a correlated start with its policy, got {other:?}"),
        }
    }

    /// Every allocatable id except `free`, as zoo-005's panes.
    fn crowded_roster(p: &Project, free: &[u32]) -> Vec<PaneConfig> {
        (3..=1002)
            .filter(|pane_id| !free.contains(pane_id))
            .map(|pane_id| pane(pane_id, Some(p.m5), Some(HOST_A)))
            .collect()
    }

    /// Two adds that no roster report separates draw different ids. With one
    /// id left, one gets it and the other is told the project is full, where
    /// both used to be handed the same id. An id whose pane never appears is
    /// free again once its reservation expires.
    #[tokio::test]
    async fn concurrent_adds_never_share_a_pane_id() {
        let p = project().await;
        let mut a = p.running(p.m5, HOST_A);
        let mut web = p.requester();
        let ids = |panes: Vec<PaneConfig>| {
            let mut ids: Vec<u32> = panes.iter().map(|pane| pane.pane_id).collect();
            ids.sort();
            ids
        };

        p.roster(crowded_roster(&p, &[40, 41]));
        tokio::join!(
            p.add_pane(&web, Some(p.m5), Some("first")),
            p.add_pane(&web, Some(p.m5), Some("second")),
        );
        let results = add_results(&web.drain());
        assert!(results
            .iter()
            .all(|result| result.status == shared::AddPaneStatus::Routed));
        let mut routed: Vec<u32> = results.iter().filter_map(|result| result.pane_id).collect();
        routed.sort();
        assert_eq!(routed, vec![40, 41]);
        assert_eq!(ids(added_panes(&a.drain())), vec![40, 41]);

        p.roster(crowded_roster(&p, &[50]));
        tokio::join!(
            p.add_pane(&web, Some(p.m5), Some("third")),
            p.add_pane(&web, Some(p.m5), Some("fourth")),
        );
        let results = add_results(&web.drain());
        assert_eq!(
            results
                .iter()
                .filter(|result| result.status == shared::AddPaneStatus::Routed)
                .map(|result| result.pane_id)
                .collect::<Vec<_>>(),
            vec![Some(50)]
        );
        assert!(results.iter().any(|result| {
            result.status == shared::AddPaneStatus::Failed
                && result
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("session is full"))
        }));
        assert_eq!(ids(added_panes(&a.drain())), vec![50]);

        p.state.sessions.set_pane_launch_timing(
            Duration::from_millis(30),
            crate::session::pane_launches::INSTANCE_START_TIMEOUT,
        );
        tokio::time::sleep(Duration::from_millis(60)).await;
        p.add_pane(&web, Some(p.m5), Some("fifth")).await;
        assert_eq!(only(add_results(&web.drain())).pane_id, Some(50));
    }

    /// A pane for a machine running the project goes to that machine's
    /// instance, tagged for it, whichever instance is primary; naming no
    /// machine still means the primary. Each answer names its machine and
    /// the pane id.
    #[tokio::test]
    async fn a_pane_goes_to_the_instance_on_the_machine_it_names() {
        let p = project().await;
        let mut a = p.running(p.m5, HOST_A);
        let mut b = p.running(p.m6, HOST_B);
        let mut web = p.requester();
        p.roster(Vec::new());

        p.add_pane(&web, Some(p.m6), Some("on-zoo-006")).await;
        let result = only(add_results(&web.drain()));
        assert_eq!(result.request_id, "on-zoo-006");
        assert_eq!(
            (result.status, result.machine_id),
            (shared::AddPaneStatus::Routed, Some(p.m6))
        );
        let added = added_panes(&b.drain());
        assert_eq!(added.len(), 1);
        assert_eq!(Some(added[0].pane_id), result.pane_id);
        assert_eq!(added[0].machine_id, Some(p.m6));
        assert_eq!(added[0].hostname.as_deref(), Some(HOST_B));
        assert!(a.drain().is_empty());

        p.add_pane(&web, None, Some("on-primary")).await;
        let result = only(add_results(&web.drain()));
        assert_eq!(
            (result.status, result.machine_id),
            (shared::AddPaneStatus::Routed, Some(p.m5))
        );
        assert_eq!(added_panes(&a.drain())[0].machine_id, Some(p.m5));
        assert!(b.drain().is_empty());
    }

    /// An older CLI on the named machine reports its panes untagged, so its new
    /// pane stays untagged.
    #[tokio::test]
    async fn an_older_cli_on_the_named_machine_gets_an_untagged_pane() {
        let p = project().await;
        let mut older = p.running_with(p.m5, HOST_A, &[shared::PROJECT_POLICY_CAPABILITY]);
        let mut web = p.requester();

        p.add_pane(&web, Some(p.m5), Some("older")).await;
        let result = only(add_results(&web.drain()));
        assert_eq!(
            (result.status, result.machine_id),
            (shared::AddPaneStatus::Routed, Some(p.m5))
        );
        let added = added_panes(&older.drain());
        assert_eq!(added.len(), 1);
        assert_eq!(
            (added[0].machine_id, added[0].hostname.as_deref()),
            (None, None)
        );
    }

    /// A pane for a machine that has the project but is not running it
    /// starts that machine's instance and follows it there: `Starting`, then
    /// `Routed` once the server has finished the instance's start. Neither
    /// the daemon's success nor the bare attach routes it.
    #[tokio::test]
    async fn a_pane_for_a_stopped_machine_starts_its_instance_first() {
        let p = project().await;
        let mut a = p.instance(p.m5, HOST_A);
        let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let mut web = p.requester();

        p.add_pane(&web, Some(p.m6), Some("start-zoo-006")).await;
        let starting = only(add_results(&web.drain()));
        assert_eq!(
            (starting.status, starting.machine_id, starting.pane_id),
            (shared::AddPaneStatus::Starting, Some(p.m6), None)
        );
        let start = start_request(&mut d6).await;
        assert!(p
            .state
            .sessions
            .instance_start_result(p.m6, start, &p.project_id(), true, None));
        let mut b = p.instance(p.m6, HOST_B);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            added_panes(&b.drain()).is_empty(),
            "attached is not started"
        );
        assert!(add_results(&web.drain()).is_empty());

        p.state.sessions.instance_started(p.session, p.m6, b.id);
        let routed = next_add_result(&mut web).await;
        assert_eq!(routed.request_id, "start-zoo-006");
        assert_eq!(
            (routed.status, routed.machine_id),
            (shared::AddPaneStatus::Routed, Some(p.m6))
        );
        let added = next_added_pane(&mut b).await;
        assert_eq!(Some(added.pane_id), routed.pane_id);
        assert_eq!(added.machine_id, Some(p.m6));
        assert!(added_panes(&a.drain()).is_empty());
    }

    /// A pane for an instance that has attached but whose start the server
    /// is still finishing waits for it, without a second start: sent sooner,
    /// it could reach the CLI before the project's policy.
    #[tokio::test]
    async fn a_pane_waits_for_an_instance_still_finishing_its_start() {
        let p = project().await;
        let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let mut b = p.instance(p.m6, HOST_B);
        let mut web = p.requester();

        p.add_pane(&web, Some(p.m6), Some("finishing")).await;
        assert_eq!(
            only(add_results(&web.drain())).status,
            shared::AddPaneStatus::Starting
        );
        assert!(added_panes(&b.drain()).is_empty());
        p.state.sessions.instance_started(p.session, p.m6, b.id);
        assert_eq!(
            next_add_result(&mut web).await.status,
            shared::AddPaneStatus::Routed
        );
        assert_eq!(next_added_pane(&mut b).await.machine_id, Some(p.m6));
        assert!(!started(&mut d6), "the instance was already starting");
    }

    /// Two panes for a stopped machine share one start of its instance.
    #[tokio::test]
    async fn two_panes_for_a_stopped_machine_share_its_start() {
        let p = project().await;
        let _a = p.instance(p.m5, HOST_A);
        let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let mut web = p.requester();

        p.add_pane(&web, Some(p.m6), Some("one")).await;
        p.add_pane(&web, Some(p.m6), Some("two")).await;
        assert!(add_results(&web.drain())
            .iter()
            .all(|result| result.status == shared::AddPaneStatus::Starting));
        start_request(&mut d6).await;
        assert!(d6.drain().is_empty(), "one start for both panes");

        let mut b = p.instance(p.m6, HOST_B);
        p.state.sessions.instance_started(p.session, p.m6, b.id);
        let mut answered = vec![
            next_add_result(&mut web).await,
            next_add_result(&mut web).await,
        ];
        answered.sort_by(|left, right| left.request_id.cmp(&right.request_id));
        assert_eq!(
            answered
                .iter()
                .map(|result| (result.request_id.as_str(), result.status))
                .collect::<Vec<_>>(),
            vec![
                ("one", shared::AddPaneStatus::Routed),
                ("two", shared::AddPaneStatus::Routed)
            ]
        );
        assert_ne!(answered[0].pane_id, answered[1].pane_id);
        let first = next_added_pane(&mut b).await;
        let second = next_added_pane(&mut b).await;
        assert_ne!(first.pane_id, second.pane_id);
    }

    /// The daemon refusing the start fails the pane with its reason.
    #[tokio::test]
    async fn a_start_the_daemon_refuses_fails_the_pane() {
        let p = project().await;
        let _a = p.instance(p.m5, HOST_A);
        let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let mut web = p.requester();

        p.add_pane(&web, Some(p.m6), Some("refused")).await;
        assert_eq!(
            only(add_results(&web.drain())).status,
            shared::AddPaneStatus::Starting
        );
        let start = start_request(&mut d6).await;
        assert!(p.state.sessions.instance_start_result(
            p.m6,
            start,
            &p.project_id(),
            false,
            Some("This project's runtime policy is suspended".to_string()),
        ));
        let failed = next_add_result(&mut web).await;
        assert_eq!(
            (failed.status, failed.machine_id, failed.error.as_deref()),
            (
                shared::AddPaneStatus::Failed,
                Some(p.m6),
                Some("This project's runtime policy is suspended")
            )
        );
    }

    /// The daemon leaving fails a pane waiting on its machine at once.
    #[tokio::test]
    async fn a_pane_waiting_on_a_machine_fails_when_its_daemon_leaves() {
        let p = project().await;
        let _a = p.instance(p.m5, HOST_A);
        let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let mut web = p.requester();

        p.add_pane(&web, Some(p.m6), Some("daemon-left")).await;
        assert_eq!(
            only(add_results(&web.drain())).status,
            shared::AddPaneStatus::Starting
        );
        start_request(&mut d6).await;
        p.state.sessions.unregister_daemon(&p.m6);
        let failed = next_add_result(&mut web).await;
        assert_eq!(failed.status, shared::AddPaneStatus::Failed);
        assert_eq!(
            failed.error.as_deref(),
            Some("zoo-006 went offline before this project started there")
        );
    }

    /// A pane gives up when its machine's instance does not start in time,
    /// and an instance that starts afterwards is sent nothing.
    #[tokio::test]
    async fn a_pane_waiting_on_a_machine_gives_up_at_its_time_limit() {
        let p = project().await;
        let _a = p.instance(p.m5, HOST_A);
        let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let mut web = p.requester();
        p.state.sessions.set_pane_launch_timing(
            crate::session::pane_launches::PANE_ID_RESERVATION_TTL,
            Duration::from_millis(100),
        );

        p.add_pane(&web, Some(p.m6), Some("too-slow")).await;
        assert_eq!(
            only(add_results(&web.drain())).status,
            shared::AddPaneStatus::Starting
        );
        start_request(&mut d6).await;
        let failed = next_add_result(&mut web).await;
        assert_eq!(failed.status, shared::AddPaneStatus::Failed);
        assert!(
            failed.error.as_deref().is_some_and(|error| {
                error.starts_with("zoo-006 did not start this project within")
            }),
            "{failed:?}"
        );
        let mut b = p.instance(p.m6, HOST_B);
        p.state.sessions.instance_started(p.session, p.m6, b.id);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(added_panes(&b.drain()).is_empty());
    }

    /// A machine that cannot take the pane is refused with why, and nothing
    /// is started anywhere.
    #[tokio::test]
    async fn a_machine_that_cannot_take_the_pane_is_refused_with_the_reason() {
        let p = project().await;
        let mut a = p.instance(p.m5, HOST_A);
        let mut web = p.requester();
        let listed = || {
            vec![shared::MachineProjectInfo {
                project_id: p.project_id(),
                name: None,
                path: "/work/elsewhere".to_string(),
                preferred_machine_id: None,
                is_running: false,
                pid: None,
                memory_kb: None,
                last_error: None,
                git_remote: None,
            }]
        };
        let refusal = |result: AddResult| {
            assert_eq!(result.status, shared::AddPaneStatus::Failed, "{result:?}");
            result.error.unwrap()
        };

        let mut offline = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        p.state.sessions.unregister_daemon(&p.m6);
        p.add_pane(&web, Some(p.m6), Some("offline")).await;
        assert_eq!(
            refusal(only(add_results(&web.drain()))),
            "zoo-006 is offline"
        );

        let m7 = Uuid::new_v4();
        let mut older = p.daemon(m7, HOST_C, &[shared::PROJECT_POLICY_CAPABILITY]);
        p.add_pane(&web, Some(m7), Some("older")).await;
        let error = refusal(only(add_results(&web.drain())));
        assert!(error.ends_with("Reboot zoo-007 to update it."), "{error}");

        let m8 = Uuid::new_v4();
        let mut unregistered = p.daemon_with(p.host, m8, "zoo-008", CAPABLE_DAEMON, Vec::new());
        p.add_pane(&web, Some(m8), Some("unregistered")).await;
        assert_eq!(
            refusal(only(add_results(&web.drain()))),
            "This project is not registered on zoo-008."
        );

        // Another account's machine, which this user may not use, is not
        // described at all.
        let m9 = Uuid::new_v4();
        let mut foreign = p.daemon_with(Uuid::new_v4(), m9, "zoo-009", CAPABLE_DAEMON, listed());
        p.add_pane(&web, Some(m9), Some("foreign")).await;
        assert_eq!(
            refusal(only(add_results(&web.drain()))),
            "That machine is not available to you"
        );
        p.add_pane(&web, Some(Uuid::new_v4()), Some("unknown"))
            .await;
        assert_eq!(
            refusal(only(add_results(&web.drain()))),
            "That machine is not available to you"
        );

        for daemon in [&mut offline, &mut older, &mut unregistered, &mut foreign] {
            assert!(!started(daemon));
        }
        assert!(added_panes(&a.drain()).is_empty(), "never sent elsewhere");
    }

    /// An older CLI running the project elsewhere would refuse an instance
    /// beside it, so a pane for another machine is refused at once.
    #[tokio::test]
    async fn a_pane_is_refused_beside_an_older_cli_on_another_machine() {
        let p = project().await;
        let mut older = p.instance_with(p.m5, HOST_A, &[shared::PROJECT_POLICY_CAPABILITY]);
        let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let mut web = p.requester();

        p.add_pane(&web, Some(p.m6), Some("beside-older")).await;
        let result = only(add_results(&web.drain()));
        assert_eq!(result.status, shared::AddPaneStatus::Failed);
        let error = result.error.unwrap();
        assert!(
            error.contains(HOST_A) && error.contains("cannot share it"),
            "{error}"
        );
        assert!(!started(&mut d6));
        assert!(older.drain().is_empty());
    }

    /// The answer reaches the connection that asked and no other viewer. A
    /// requester that leaves while its pane waits still gets the pane; the
    /// answer to it is dropped.
    #[tokio::test]
    async fn only_the_requesting_connection_hears_about_its_pane() {
        let p = project().await;
        let _a = p.running(p.m5, HOST_A);
        let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let mut requester = p.requester();
        let mut other = p.requester();

        p.add_pane(&requester, Some(p.m5), Some("mine")).await;
        assert_eq!(add_results(&requester.drain()).len(), 1);
        assert!(add_results(&other.drain()).is_empty());

        p.add_pane(&requester, Some(p.m6), Some("waits")).await;
        assert_eq!(
            only(add_results(&requester.drain())).status,
            shared::AddPaneStatus::Starting
        );
        start_request(&mut d6).await;
        p.state.sessions.unregister_web(&requester.id);
        let mut b = p.instance(p.m6, HOST_B);
        p.state.sessions.instance_started(p.session, p.m6, b.id);
        assert_eq!(next_added_pane(&mut b).await.machine_id, Some(p.m6));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(add_results(&other.drain()).is_empty());
        assert!(add_results(&requester.drain()).is_empty());
    }

    /// A request without an id, from a client that predates correlated
    /// launches, is answered as before: nothing when the pane is sent, an
    /// `Error` when it is refused, and never an `AddPaneResult`.
    #[tokio::test]
    async fn a_request_without_an_id_is_answered_as_before() {
        let p = project().await;
        let mut a = p.instance(p.m5, HOST_A);
        let mut web = p.requester();

        p.add_pane(&web, None, None).await;
        let received = web.drain();
        assert!(add_results(&received).is_empty() && errors(&received).is_empty());
        assert_eq!(added_panes(&a.drain()).len(), 1);

        p.add_pane(&web, Some(Uuid::new_v4()), None).await;
        let received = web.drain();
        assert!(add_results(&received).is_empty());
        assert_eq!(
            errors(&received),
            vec!["That machine is not available to you"]
        );
    }

    /// End to end over the CLI socket: the instance a pane waits for starts
    /// with a real `SessionStart`, and the pane reaches it only after the
    /// project's policy, without which its CLI refuses every new pane. Its
    /// roster report then clears the id's reservation: with only that id
    /// free, it can be handed out again as soon as the pane is closed.
    #[tokio::test]
    async fn a_waiting_pane_reaches_its_instance_after_the_projects_policy() {
        use jsonwebtoken::{encode, EncodingKey, Header};
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let p = project().await;
        let mut d6 = p.daemon(p.m6, HOST_B, CAPABLE_DAEMON);
        let mut web = p.requester();
        p.roster(crowded_roster(&p, &[77]));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = crate::routes::create_router(p.state.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        p.add_pane(&web, Some(p.m6), Some("over-the-socket")).await;
        assert_eq!(
            only(add_results(&web.drain())).status,
            shared::AddPaneStatus::Starting
        );
        start_request(&mut d6).await;

        // zoo-006's daemon starts the project: its instance connects and starts.
        let token = encode(
            &Header::default(),
            &crate::routes::auth::Claims {
                sub: p.host.to_string(),
                exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
                device_session_id: None,
                token_kind: None,
                credential_version: None,
            },
            &EncodingKey::from_secret(p.state.config.auth.jwt_secret.as_bytes()),
        )
        .unwrap();
        let (mut socket, _) = connect_async(format!("ws://{address}/ws/cli"))
            .await
            .unwrap();
        let frame =
            |message: shared::CliToServer| Message::Text(serde_json::to_string(&message).unwrap());
        socket
            .send(frame(shared::CliToServer::Register {
                token,
                version: Some("26.11.0".to_string()),
                capabilities: vec![
                    PROJECT_INSTANCES_CAPABILITY.to_string(),
                    shared::PROJECT_POLICY_CAPABILITY.to_string(),
                ],
            }))
            .await
            .unwrap();
        socket
            .send(frame(shared::CliToServer::SessionStart {
                session_id: p.session,
                project_id: Some(p.session),
                machine_id: Some(p.m6),
                working_dir: Some(format!("/work/{HOST_B}")),
                hostname: Some(HOST_B.to_string()),
                git_remote: None,
                git_remote_url: None,
                pane_type: None,
                panes: Some(Vec::new()),
            }))
            .await
            .unwrap();
        let received = tokio::time::timeout(Duration::from_secs(5), async {
            let mut received = Vec::new();
            loop {
                let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
                    continue;
                };
                let message: ServerToCli = serde_json::from_str(&text).unwrap();
                let done = matches!(message, ServerToCli::AddPane { .. });
                received.push(message);
                if done {
                    return received;
                }
            }
        })
        .await
        .expect("the waiting pane reaches the instance");
        let policy_at = received
            .iter()
            .position(|message| matches!(message, ServerToCli::ProjectPolicy { .. }))
            .expect("the instance is sent the project's policy");
        assert!(policy_at < received.len() - 1, "the policy comes first");
        let Some(ServerToCli::AddPane { pane_config, .. }) = received.last().cloned() else {
            unreachable!()
        };
        assert_eq!(pane_config.machine_id, Some(p.m6));
        assert_eq!(pane_config.pane_id, 77);
        let routed = next_add_result(&mut web).await;
        assert_eq!(
            (routed.status, routed.machine_id, routed.pane_id),
            (shared::AddPaneStatus::Routed, Some(p.m6), Some(77))
        );

        // The instance reports the pane, then closes it.
        for (panes, listed) in [(vec![pane_config.clone()], true), (Vec::new(), false)] {
            socket
                .send(frame(shared::CliToServer::PaneList {
                    session_id: p.session,
                    panes,
                }))
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while p.pane_ids().contains(&77) != listed {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("the report is merged");
        }
        p.add_pane(&web, Some(p.m6), Some("again")).await;
        let again = only(add_results(&web.drain()));
        assert_eq!(
            (again.status, again.pane_id),
            (shared::AddPaneStatus::Routed, Some(77))
        );
        server.abort();
    }

    /// What the CLI of a machine that has autopilot advertises.
    const AUTOPILOT_CLI: &[&str] = &[
        PROJECT_INSTANCES_CAPABILITY,
        shared::PROJECT_POLICY_CAPABILITY,
        shared::OMP_TAB_AUTOPILOT_CAPABILITY,
    ];
    /// What a web client that shows autopilot negotiates.
    const AUTOPILOT_VIEWER: &[&str] = &[
        PROJECT_INSTANCES_CAPABILITY,
        shared::TAB_AUTOPILOT_CAPABILITY,
    ];

    use shared::AutopilotReceiptOutcome::{Accepted, Refused, Uncertain};
    use shared::AutopilotSupport;

    fn omp_tab(pane_id: u32, machine_id: Uuid, host: &str) -> PaneConfig {
        let mut tab = pane(pane_id, Some(machine_id), Some(host));
        tab.provider = shared::Provider::Omp;
        tab
    }

    fn autopilot_spec() -> shared::AutopilotStartRequest {
        shared::AutopilotStartRequest {
            prompt: Some("Check the build and fix what fails.".to_string()),
            repeat: Some(shared::AutopilotRepeat::AfterEachTurn),
            iteration_limit: Some(shared::AutopilotLimit::None),
            time_limit: Some(shared::AutopilotLimit::None),
            end_without_tool_call: Some(true),
        }
    }

    fn autopilot_start() -> shared::AutopilotAction {
        shared::AutopilotAction::Start {
            spec: autopilot_spec(),
        }
    }

    fn autopilot_stop() -> shared::AutopilotAction {
        shared::AutopilotAction::Stop {
            autopilot_id: None,
            now: false,
        }
    }

    /// Every request a viewer can make of a tab's autopilot.
    fn autopilot_actions() -> Vec<(&'static str, shared::AutopilotAction)> {
        vec![
            ("start", autopilot_start()),
            (
                "pause",
                shared::AutopilotAction::Pause { autopilot_id: None },
            ),
            (
                "resume",
                shared::AutopilotAction::Resume {
                    autopilot_id: Uuid::nil(),
                },
            ),
            ("stop", autopilot_stop()),
            (
                "stop-now",
                shared::AutopilotAction::Stop {
                    autopilot_id: Some(Uuid::nil()),
                    now: true,
                },
            ),
        ]
    }

    /// `viewer` asks for `action` on the tab `pane_id`.
    async fn autopilot(
        p: &Project,
        viewer: &Uuid,
        pane_id: u32,
        request_id: &str,
        action: shared::AutopilotAction,
    ) {
        handle_pane_autopilot_control(
            &p.state,
            viewer,
            WebToServer::ControlPaneAutopilot {
                session_id: p.session,
                pane_id,
                request_id: request_id.to_string(),
                action,
            },
            AUTOPILOT_RECEIPT_WAIT,
        )
        .await;
    }

    /// A receipt as a viewer reads it: request id, outcome and reason.
    type Receipt = (String, shared::AutopilotReceiptOutcome, Option<String>);

    fn receipt(
        request_id: &str,
        outcome: shared::AutopilotReceiptOutcome,
        reason: Option<&str>,
    ) -> Receipt {
        (request_id.to_string(), outcome, reason.map(str::to_string))
    }

    fn receipts(messages: &[ServerToWeb]) -> Vec<Receipt> {
        messages
            .iter()
            .filter_map(|message| match message {
                ServerToWeb::PaneAutopilotReceipt {
                    request_id,
                    outcome,
                    reason,
                    ..
                } => Some((request_id.clone(), *outcome, reason.clone())),
                _ => None,
            })
            .collect()
    }

    /// The next receipt a viewer is sent. A forwarded request is answered
    /// from a task of its own, once the tab's instance has answered.
    async fn next_receipt(web: &mut Endpoint<ServerToWeb>) -> Receipt {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let message = web.rx.recv().await.expect("web sender closed");
                if let Some(receipt) = receipts(&[message]).pop() {
                    return receipt;
                }
            }
        })
        .await
        .expect("the request is answered")
    }

    /// The autopilot requests an instance was forwarded: pane, request id
    /// and the strict command.
    fn forwarded(cli: &mut Endpoint<ServerToCli>) -> Vec<(u32, String, shared::AutopilotCommand)> {
        cli.drain()
            .into_iter()
            .filter_map(|message| match message {
                ServerToCli::ControlPaneAutopilot {
                    pane_id,
                    request_id,
                    command,
                    ..
                } => Some((pane_id, request_id, command)),
                _ => None,
            })
            .collect()
    }

    /// The instance behind `cli` answers a request it was forwarded. False
    /// when no such request waits for that instance.
    fn answer(
        p: &Project,
        cli: &Endpoint<ServerToCli>,
        pane_id: u32,
        request_id: &str,
        outcome: shared::AutopilotReceiptOutcome,
        reason: Option<&str>,
    ) -> bool {
        p.state.sessions.finish_autopilot_request(
            p.session,
            pane_id,
            cli.id,
            request_id,
            AutopilotReceipt {
                outcome,
                reason: reason.map(str::to_string),
            },
        )
    }

    /// The autopilot states a viewer was sent: pane, availability and the
    /// time the state was last reported.
    fn autopilot_states(messages: &[ServerToWeb]) -> Vec<(u32, AutopilotSupport, Option<String>)> {
        messages
            .iter()
            .filter_map(|message| match message {
                ServerToWeb::PaneAutopilotState {
                    pane_id,
                    support,
                    reported_at,
                    ..
                } => Some((*pane_id, *support, reported_at.clone())),
                _ => None,
            })
            .collect()
    }

    /// A tab waiting for its next iteration, as its instance reports it.
    fn waiting_autopilot() -> Box<shared::PaneAutopilotState> {
        Box::new(shared::PaneAutopilotState {
            autopilot: Some(shared::AutopilotView {
                autopilot_id: Uuid::nil(),
                spec: autopilot_spec().validate().unwrap(),
                started_at: "2026-11-13T14:00:00Z".to_string(),
                iterations: 7,
                deadline: None,
                next_iteration_at: Some("2026-11-13T14:30:00Z".to_string()),
                condition: shared::AutopilotCondition {
                    kind: shared::autopilot_condition::WAITING.to_string(),
                    detail: Some(shared::autopilot_condition::WAIT_NEXT_ITERATION.to_string()),
                    since: None,
                },
            }),
            ..shared::PaneAutopilotState::default()
        })
    }

    /// Whoever may type into a tab on its machine may start, pause, resume
    /// and stop its autopilot, and nobody else. The hosting account may, and
    /// so may a collaborator its cluster admits to the tab's machine. A
    /// project user it does not admit there may not, nor an account without
    /// access. A refusal reads as input to that tab does, and is the same
    /// for a stop as for a start.
    #[tokio::test]
    async fn whoever_may_type_into_a_tab_may_control_its_autopilot() {
        let (p, member) = hosted_project().await;
        let stranger = Uuid::new_v4();
        p.state.db.create_user(&user(stranger)).await.unwrap();
        // The project is placed in the hosting account's cluster, which is
        // that account's access to it.
        p.state
            .db
            .add_project_cluster_placement(
                &p.project_id(),
                &p.host.to_string(),
                &p.owner.to_string(),
                "test",
            )
            .await
            .unwrap();
        let mut a = p.instance_with(p.m5, HOST_A, AUTOPILOT_CLI);
        let mut b = p.instance_with(p.m6, HOST_B, AUTOPILOT_CLI);
        let mut host_web = p.viewer(p.host, AUTOPILOT_VIEWER);
        let mut member_web = p.viewer(member, AUTOPILOT_VIEWER);
        // The project's owner is only a project user here: the project is
        // hosted in another account's cluster, which admits the owner to none
        // of its machines.
        let mut owner_web = p.viewer(p.owner, AUTOPILOT_VIEWER);
        // An account without access cannot attach at all.
        let mut stranger_web = {
            let id = Uuid::new_v4();
            let (tx, rx) = mpsc::channel(256);
            p.state.sessions.register_web(id, tx);
            p.state.sessions.set_web_user(id, stranger);
            p.state.sessions.set_web_capabilities(
                id,
                AUTOPILOT_VIEWER
                    .iter()
                    .map(|capability| capability.to_string())
                    .collect(),
            );
            Endpoint { id, rx }
        };
        p.roster(vec![omp_tab(3, p.m5, HOST_A), omp_tab(4, p.m6, HOST_B)]);

        // What input to the same tab is refused with.
        typed(&p, &member_web.id, Some(4), None).await;
        let member_refusal = only_error(&mut member_web);
        assert!(
            member_refusal.starts_with("Access denied") && member_refusal.contains(HOST_B),
            "{member_refusal}"
        );
        typed(&p, &owner_web.id, Some(3), None).await;
        let owner_refusal = only_error(&mut owner_web);
        assert_eq!(owner_refusal, "Access denied");
        typed(&p, &stranger_web.id, Some(3), None).await;
        let stranger_refusal = only_error(&mut stranger_web);
        assert_eq!(stranger_refusal, "Not attached to that session");
        assert!(a.drain().is_empty() && b.drain().is_empty());

        for (name, action) in autopilot_actions() {
            let command = action.validate().unwrap();

            // The hosting account, and a collaborator on a machine it may use.
            for (who, web) in [("host", &mut host_web), ("member", &mut member_web)] {
                let id = format!("{who}-{name}");
                autopilot(&p, &web.id, 3, &id, action.clone()).await;
                assert_eq!(
                    forwarded(&mut a),
                    vec![(3, id.clone(), command.clone())],
                    "{id}"
                );
                assert!(answer(&p, &a, 3, &id, Accepted, None));
                assert_eq!(next_receipt(web).await, receipt(&id, Accepted, None));
            }
            let id = format!("host-elsewhere-{name}");
            autopilot(&p, &host_web.id, 4, &id, action.clone()).await;
            assert_eq!(forwarded(&mut b), vec![(4, id.clone(), command.clone())]);
            assert!(answer(&p, &b, 4, &id, Accepted, None));
            assert_eq!(
                next_receipt(&mut host_web).await,
                receipt(&id, Accepted, None)
            );

            // The same collaborator on a machine it may not use, a project
            // user with no machine, and an account with no access.
            for (who, web, pane_id, refusal) in [
                ("member-elsewhere", &mut member_web, 4, &member_refusal),
                ("owner", &mut owner_web, 3, &owner_refusal),
                ("stranger", &mut stranger_web, 3, &stranger_refusal),
            ] {
                let id = format!("{who}-{name}");
                autopilot(&p, &web.id, pane_id, &id, action.clone()).await;
                let delivered = web.drain();
                assert_eq!(
                    receipts(&delivered),
                    vec![receipt(&id, Refused, Some(refusal))],
                    "{id}"
                );
                assert!(
                    errors(&delivered).is_empty(),
                    "the receipt is the whole answer: {delivered:?}"
                );
            }
            assert!(forwarded(&mut a).is_empty() && forwarded(&mut b).is_empty());
        }
    }

    /// The one `Error` a viewer was sent since it was last read.
    fn only_error(web: &mut Endpoint<ServerToWeb>) -> String {
        let mut errors = errors(&web.drain());
        assert_eq!(errors.len(), 1, "{errors:?}");
        errors.remove(0)
    }

    /// In a project on two machines a request reaches only the instance that
    /// runs the tab, and only that instance can answer it. A tab whose
    /// machine is not running the project is refused by naming the machine,
    /// never redirected, for a stop as for a start.
    #[tokio::test]
    async fn an_autopilot_request_reaches_only_the_machine_that_runs_the_tab() {
        let p = project().await;
        let mut a = p.instance_with(p.m5, HOST_A, AUTOPILOT_CLI);
        let mut b = p.instance_with(p.m6, HOST_B, AUTOPILOT_CLI);
        let mut web = p.viewer(p.owner, AUTOPILOT_VIEWER);
        p.roster(vec![
            omp_tab(3, p.m5, HOST_A),
            omp_tab(4, p.m6, HOST_B),
            omp_tab(5, Uuid::new_v4(), HOST_C),
        ]);

        autopilot(&p, &web.id, 4, "start-on-6", autopilot_start()).await;
        assert!(forwarded(&mut a).is_empty());
        assert_eq!(
            forwarded(&mut b),
            vec![(
                4,
                "start-on-6".to_string(),
                shared::AutopilotCommand::Start {
                    spec: autopilot_spec().validate().unwrap()
                }
            )]
        );
        assert!(
            !answer(&p, &a, 4, "start-on-6", Accepted, None),
            "another machine's instance cannot answer for the tab's own"
        );
        assert!(answer(&p, &b, 4, "start-on-6", Accepted, None));
        assert_eq!(
            next_receipt(&mut web).await,
            receipt("start-on-6", Accepted, None)
        );

        for (name, action) in autopilot_actions() {
            let id = format!("stopped-{name}");
            autopilot(&p, &web.id, 5, &id, action).await;
            let delivered = web.drain();
            assert_eq!(
                receipts(&delivered),
                vec![receipt(&id, Refused, Some(&not_running(HOST_C)))]
            );
            assert!(errors(&delivered).is_empty());
        }

        // zoo-006 stops running the project.
        p.state
            .sessions
            .detach_cli_instance(&p.session, &b.id)
            .unwrap();
        autopilot(&p, &web.id, 4, "after-stop", autopilot_stop()).await;
        assert_eq!(
            receipts(&web.drain()),
            vec![receipt(
                "after-stop",
                Refused,
                Some("zoo-006 is not running this project. Start it on zoo-006 first.")
            )]
        );
        assert!(forwarded(&mut a).is_empty() && forwarded(&mut b).is_empty());
    }

    /// Autopilot needs both ends. A tab on a machine whose CLI has no
    /// autopilot is refused with the update text and sent nothing, because an
    /// older CLI drops the frame without an answer. A viewer that did not
    /// negotiate autopilot has its request ignored and is never sent an
    /// autopilot frame, which would close an older native build's socket.
    #[tokio::test]
    async fn autopilot_is_gated_on_the_viewer_and_on_the_tabs_instance() {
        let p = project().await;
        let mut a = p.instance_with(p.m5, HOST_A, AUTOPILOT_CLI);
        // zoo-006 has not been rebooted to update yet.
        let mut b = p.instance(p.m6, HOST_B);
        let mut web = p.viewer(p.owner, AUTOPILOT_VIEWER);
        let mut older_web = p.viewer(p.owner, &[PROJECT_INSTANCES_CAPABILITY]);
        p.roster(vec![omp_tab(3, p.m5, HOST_A), omp_tab(4, p.m6, HOST_B)]);

        for (name, action) in autopilot_actions() {
            let id = format!("older-machine-{name}");
            autopilot(&p, &web.id, 4, &id, action).await;
            assert_eq!(
                receipts(&web.drain()),
                vec![receipt(
                    &id,
                    Refused,
                    Some("zoo-006 runs an older APAS that cannot run autopilot. Reboot zoo-006 to update it.")
                )]
            );
        }
        assert!(b.drain().is_empty());

        autopilot(&p, &older_web.id, 3, "older-viewer", autopilot_start()).await;
        assert!(forwarded(&mut a).is_empty());
        let delivered = older_web.drain();
        assert!(receipts(&delivered).is_empty());
        assert_eq!(errors(&delivered).len(), 1, "{delivered:?}");

        // What the tabs report, and what a viewer is replayed when it
        // attaches, reaches the viewer that negotiated autopilot only.
        let reported_at = Some("2026-11-13T14:07:00Z".to_string());
        p.state.sessions.report_pane_autopilot(
            &p.session,
            a.id,
            3,
            AutopilotSupport::Supported,
            Some(waiting_autopilot()),
            reported_at.clone(),
        );
        // An instance without autopilot cannot report a state either.
        p.state.sessions.report_pane_autopilot(
            &p.session,
            b.id,
            4,
            AutopilotSupport::Supported,
            Some(waiting_autopilot()),
            None,
        );
        for viewer in [&web.id, &older_web.id] {
            replay_pane_autopilot_after_attach(&p.state, viewer, p.session).await;
        }
        let live = (3, AutopilotSupport::Supported, reported_at.clone());
        assert_eq!(
            autopilot_states(&web.drain()),
            vec![
                live.clone(),
                // The replay carries the time the state was last reported.
                live,
                (4, AutopilotSupport::InstanceUpdateRequired, None),
            ]
        );
        let delivered = older_web.drain();
        assert!(autopilot_states(&delivered).is_empty(), "{delivered:?}");

        // The connection's outbound task holds the same line.
        let state_frame = p
            .state
            .sessions
            .pane_autopilot_frame(&p.session, 3)
            .unwrap();
        let receipt_frame = ServerToWeb::PaneAutopilotReceipt {
            session_id: p.session,
            pane_id: 3,
            request_id: "any".to_string(),
            outcome: Uncertain,
            reason: None,
        };
        let other_frame = ServerToWeb::PaneList {
            session_id: p.session,
            panes: Vec::new(),
        };
        for frame in [&state_frame, &receipt_frame] {
            assert!(autopilot_outbound_allowed(&p.state, &web.id, frame));
            assert!(!autopilot_outbound_allowed(&p.state, &older_web.id, frame));
        }
        assert!(autopilot_outbound_allowed(
            &p.state,
            &older_web.id,
            &other_frame
        ));
    }

    /// Requests are tracked by request id, never one per tab: a stop is
    /// forwarded while a start for the same tab is still being answered, each
    /// is answered on its own, and only to the connection that sent it. A
    /// stop for a tab with no autopilot changes nothing, so the tab's
    /// instance accepts it and the server relays that. A repeated copy of a
    /// request has no further effect.
    #[tokio::test]
    async fn a_stop_is_taken_while_another_request_for_its_tab_is_being_answered() {
        let p = project().await;
        let mut a = p.instance_with(p.m5, HOST_A, AUTOPILOT_CLI);
        let mut web = p.viewer(p.owner, AUTOPILOT_VIEWER);
        let mut other_web = p.viewer(p.owner, AUTOPILOT_VIEWER);
        p.roster(vec![omp_tab(3, p.m5, HOST_A)]);

        autopilot(&p, &web.id, 3, "start-1", autopilot_start()).await;
        autopilot(&p, &web.id, 3, "stop-1", autopilot_stop()).await;
        assert_eq!(
            forwarded(&mut a)
                .into_iter()
                .map(|(_, request_id, command)| (request_id, command.starts_work()))
                .collect::<Vec<_>>(),
            vec![("start-1".to_string(), true), ("stop-1".to_string(), false)],
            "both reach the tab, in the order they were sent"
        );
        assert!(receipts(&web.drain()).is_empty(), "neither is answered yet");

        // The stop found no autopilot to stop.
        assert!(answer(&p, &a, 3, "stop-1", Accepted, None));
        assert_eq!(
            next_receipt(&mut web).await,
            receipt("stop-1", Accepted, None)
        );
        let not_taken = "The tab did not take the request in time. Nothing was started.";
        assert!(answer(&p, &a, 3, "start-1", Refused, Some(not_taken)));
        assert_eq!(
            next_receipt(&mut web).await,
            receipt("start-1", Refused, Some(not_taken))
        );

        autopilot(&p, &web.id, 3, "stop-1", autopilot_stop()).await;
        assert!(
            forwarded(&mut a).is_empty(),
            "a repeated copy is not sent on"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(receipts(&web.drain()).is_empty());
        assert!(
            receipts(&other_web.drain()).is_empty(),
            "another connection of the same account hears no receipt"
        );
    }

    /// A request the tab's instance does not answer in time is reported as
    /// not confirmed, never as refused: it was delivered, so a pause or a
    /// stop stays in force. So is one whose instance disconnects first,
    /// without waiting out the time limit.
    #[tokio::test]
    async fn an_unanswered_autopilot_request_is_not_confirmed() {
        let p = project().await;
        let mut a = p.instance_with(p.m5, HOST_A, AUTOPILOT_CLI);
        let mut web = p.viewer(p.owner, AUTOPILOT_VIEWER);
        p.roster(vec![omp_tab(3, p.m5, HOST_A)]);

        handle_pane_autopilot_control(
            &p.state,
            &web.id,
            WebToServer::ControlPaneAutopilot {
                session_id: p.session,
                pane_id: 3,
                request_id: "pause-1".to_string(),
                action: shared::AutopilotAction::Pause { autopilot_id: None },
            },
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(forwarded(&mut a).len(), 1);
        assert_eq!(
            next_receipt(&mut web).await,
            receipt("pause-1", Uncertain, None)
        );
        assert!(
            !answer(&p, &a, 3, "pause-1", Accepted, None),
            "a late receipt finds nothing waiting"
        );

        autopilot(&p, &web.id, 3, "stop-1", autopilot_stop()).await;
        assert_eq!(forwarded(&mut a).len(), 1);
        let sent = std::time::Instant::now();
        p.state.sessions.unregister_cli(&a.id);
        assert_eq!(
            next_receipt(&mut web).await,
            receipt("stop-1", Uncertain, None)
        );
        assert!(sent.elapsed() < AUTOPILOT_RECEIPT_WAIT);
        assert!(
            receipts(&web.drain()).is_empty(),
            "each request is answered once"
        );
    }

    /// A start is checked before anything reaches the tab, and refused with
    /// what is wrong with it: a bound it breaks, or a limit it does not state
    /// as a number or as none. So is a request for a tab that cannot be on
    /// autopilot.
    #[tokio::test]
    async fn an_autopilot_request_that_cannot_be_served_is_refused_with_its_reason() {
        let p = project().await;
        let mut a = p.instance_with(p.m5, HOST_A, AUTOPILOT_CLI);
        let mut web = p.viewer(p.owner, AUTOPILOT_VIEWER);
        let mut structured = omp_tab(5, p.m5, HOST_A);
        structured.kind = shared::PaneKind::Agent;
        let mut history = omp_tab(6, p.m5, HOST_A);
        history.mode = shared::PaneMode::Deadloop;
        p.roster(vec![
            omp_tab(3, p.m5, HOST_A),
            pane(4, Some(p.m5), Some(HOST_A)),
            structured,
            history,
        ]);
        let start = |spec: shared::AutopilotStartRequest| shared::AutopilotAction::Start { spec };
        let not_omp = "Autopilot is available only on OMP terminal tabs.";

        let cases = [
            (
                "no-time-limit",
                3,
                start(shared::AutopilotStartRequest {
                    time_limit: None,
                    ..autopilot_spec()
                }),
                "Say whether autopilot has a time limit: a duration, or none.",
            ),
            (
                "no-iteration-limit",
                3,
                start(shared::AutopilotStartRequest {
                    iteration_limit: None,
                    ..autopilot_spec()
                }),
                "Say whether autopilot has an iteration limit: a number, or none.",
            ),
            (
                "empty-prompt",
                3,
                start(shared::AutopilotStartRequest {
                    prompt: Some(" \n".to_string()),
                    ..autopilot_spec()
                }),
                "Autopilot needs a prompt.",
            ),
            (
                "short-interval",
                3,
                start(shared::AutopilotStartRequest {
                    repeat: Some(shared::AutopilotRepeat::Every { seconds: 10 }),
                    ..autopilot_spec()
                }),
                "The repeat interval must be between one minute and 24 hours.",
            ),
            ("claude-tab", 4, autopilot_start(), not_omp),
            ("structured-pane", 5, autopilot_stop(), not_omp),
            (
                "bot-history",
                6,
                autopilot_start(),
                "Legacy bot history is read-only; it cannot be put on autopilot.",
            ),
            (
                "closed-tab",
                9,
                autopilot_stop(),
                "This tab is no longer available.",
            ),
        ];
        for (id, pane_id, action, reason) in cases {
            if let shared::AutopilotAction::Start { spec } = &action {
                if pane_id == 3 {
                    assert_eq!(spec.validate().unwrap_err(), reason, "{id}");
                }
            }
            autopilot(&p, &web.id, pane_id, id, action).await;
            let delivered = web.drain();
            assert_eq!(
                receipts(&delivered),
                vec![receipt(id, Refused, Some(reason))],
                "{id}"
            );
            assert!(errors(&delivered).is_empty(), "{id}");
        }

        // An id that cannot be tracked is turned away before anything else.
        for id in [
            String::new(),
            "x".repeat(shared::AUTOPILOT_REQUEST_ID_MAX_BYTES + 1),
        ] {
            autopilot(&p, &web.id, 3, &id, autopilot_stop()).await;
            let delivered = web.drain();
            assert!(receipts(&delivered).is_empty());
            assert_eq!(
                errors(&delivered),
                vec!["Invalid autopilot request identifier".to_string()]
            );
        }
        assert!(a.drain().is_empty(), "nothing reached the tab's instance");

        // The longest id a viewer may send is taken.
        let longest = "x".repeat(shared::AUTOPILOT_REQUEST_ID_MAX_BYTES);
        autopilot(&p, &web.id, 3, &longest, autopilot_stop()).await;
        assert_eq!(forwarded(&mut a).len(), 1);
    }

    /// Viewers are not told an instance's capabilities. When a machine that
    /// has not been rebooted to update attaches while a capable viewer is
    /// watching, that viewer is told that each of the machine's OMP tabs
    /// needs an instance update, and session lists say the same. Once the
    /// machine reconnects with autopilot its tabs are not reported, until it
    /// reports them.
    #[tokio::test]
    async fn an_older_instance_attaching_tells_capable_viewers_which_tabs_need_its_update() {
        let p = project().await;
        let mut web = p.viewer(p.owner, AUTOPILOT_VIEWER);
        let mut older_web = p.viewer(p.owner, &[PROJECT_INSTANCES_CAPABILITY]);
        p.roster(vec![
            omp_tab(3, p.m6, HOST_B),
            omp_tab(4, p.m6, HOST_B),
            pane(5, Some(p.m6), Some(HOST_B)),
            omp_tab(6, p.m5, HOST_A),
        ]);
        let support = |pane_id: u32| {
            pane_summaries(&p.state, &p.session, true)
                .into_iter()
                .find(|summary| summary.pane_id == pane_id)
                .unwrap()
                .autopilot
                .map(|autopilot| autopilot.support)
        };
        assert_eq!(support(3), Some(AutopilotSupport::NotReported));

        let _b = p.instance(p.m6, HOST_B);
        assert_eq!(
            autopilot_states(&web.drain()),
            vec![
                (3, AutopilotSupport::InstanceUpdateRequired, None),
                (4, AutopilotSupport::InstanceUpdateRequired, None),
                (6, AutopilotSupport::NotReported, None),
            ]
        );
        assert_eq!(support(3), Some(AutopilotSupport::InstanceUpdateRequired));
        assert_eq!(support(4), Some(AutopilotSupport::InstanceUpdateRequired));
        assert_eq!(support(5), None, "not an OMP terminal tab");
        assert_eq!(support(6), Some(AutopilotSupport::NotReported));

        // The machine is rebooted to update and its instance attaches again.
        let updated = p.instance_with(p.m6, HOST_B, AUTOPILOT_CLI);
        assert_eq!(
            autopilot_states(&web.drain()),
            vec![
                (3, AutopilotSupport::NotReported, None),
                (4, AutopilotSupport::NotReported, None),
                (6, AutopilotSupport::NotReported, None),
            ]
        );
        p.state.sessions.report_pane_autopilot(
            &p.session,
            updated.id,
            3,
            AutopilotSupport::NeedsAgentReboot,
            None,
            None,
        );
        assert_eq!(
            autopilot_states(&web.drain()),
            vec![(3, AutopilotSupport::NeedsAgentReboot, None)]
        );
        assert!(autopilot_states(&older_web.drain()).is_empty());
    }

    /// Session lists carry a tab's autopilot condition and progress beside
    /// its status, for OMP terminal tabs only and never with the prompt.
    #[tokio::test]
    async fn pane_summaries_carry_the_autopilot_condition_without_the_prompt() {
        let p = project().await;
        let a = p.instance_with(p.m5, HOST_A, AUTOPILOT_CLI);
        p.roster(vec![
            omp_tab(3, p.m5, HOST_A),
            pane(4, Some(p.m5), Some(HOST_A)),
        ]);
        p.state.sessions.report_pane_autopilot(
            &p.session,
            a.id,
            3,
            AutopilotSupport::Supported,
            Some(waiting_autopilot()),
            Some("2026-11-13T14:07:00Z".to_string()),
        );

        let summaries = pane_summaries(&p.state, &p.session, true);
        let on_autopilot = summaries[0].autopilot.clone().expect("an OMP terminal tab");
        assert_eq!(on_autopilot.support, AutopilotSupport::Supported);
        assert_eq!(on_autopilot.condition.as_deref(), Some("waiting"));
        assert_eq!(on_autopilot.detail.as_deref(), Some("next_iteration"));
        assert_eq!(
            on_autopilot.next_iteration_at.as_deref(),
            Some("2026-11-13T14:30:00Z")
        );
        assert_eq!(on_autopilot.iterations, Some(7));
        assert_eq!(on_autopilot.iteration_limit, None, "no iteration limit");
        assert!(summaries[1].autopilot.is_none());
        // Autopilot is its own attribute: the tab's status is untouched.
        assert!(!summaries[0].is_working && !summaries[0].awaiting_answer);
        let listed = serde_json::to_string(&summaries).unwrap();
        assert!(!listed.contains("Check the build"), "{listed}");
    }

    /// End to end over the web socket. A client that asks for autopilot is
    /// told the server has it, is replayed each OMP tab's availability when
    /// it attaches, after the roster that lists the tab, and is answered when
    /// it sends a request. A client that does not ask negotiates nothing and
    /// is sent no autopilot frame, whatever it sends.
    #[tokio::test]
    async fn a_web_client_that_asks_for_autopilot_is_replayed_and_answered_over_its_socket() {
        use jsonwebtoken::{encode, EncodingKey, Header};
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        type Socket = tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >;
        async fn send(socket: &mut Socket, message: WebToServer) {
            socket
                .send(Message::Text(serde_json::to_string(&message).unwrap()))
                .await
                .unwrap();
        }
        /// Every frame up to and including the first that `last` accepts.
        async fn receive_until(
            socket: &mut Socket,
            last: impl Fn(&ServerToWeb) -> bool,
        ) -> Vec<ServerToWeb> {
            let mut received = Vec::new();
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
                        continue;
                    };
                    let message: ServerToWeb = serde_json::from_str(&text).unwrap();
                    let done = last(&message);
                    received.push(message);
                    if done {
                        return;
                    }
                }
            })
            .await
            .expect("timed out waiting for the server");
            received
        }
        async fn connect(url: &str, token: &str, capabilities: &[&str]) -> (Socket, Vec<String>) {
            let (mut socket, _) = connect_async(url).await.unwrap();
            send(
                &mut socket,
                WebToServer::Authenticate {
                    token: token.to_string(),
                    capabilities: capabilities
                        .iter()
                        .map(|capability| capability.to_string())
                        .collect(),
                    client_kind: None,
                    app_version: None,
                    protocol_version: None,
                },
            )
            .await;
            let frames = receive_until(&mut socket, |message| {
                matches!(message, ServerToWeb::Authenticated { .. })
            })
            .await;
            let Some(ServerToWeb::Authenticated {
                negotiated_capabilities,
                ..
            }) = frames.last().cloned()
            else {
                unreachable!()
            };
            (socket, negotiated_capabilities)
        }

        let p = project().await;
        let mut a = p.instance_with(p.m5, HOST_A, AUTOPILOT_CLI);
        p.roster(vec![
            omp_tab(3, p.m5, HOST_A),
            pane(4, Some(p.m5), Some(HOST_A)),
        ]);
        let reported_at = Some("2026-11-13T14:07:00Z".to_string());
        p.state.sessions.report_pane_autopilot(
            &p.session,
            a.id,
            3,
            AutopilotSupport::Supported,
            Some(waiting_autopilot()),
            reported_at.clone(),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = crate::routes::create_router(p.state.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let url = format!("ws://{address}/ws/web");
        let token = encode(
            &Header::default(),
            &crate::routes::auth::Claims {
                sub: p.owner.to_string(),
                exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
                device_session_id: None,
                token_kind: None,
                credential_version: None,
            },
            &EncodingKey::from_secret(p.state.config.auth.jwt_secret.as_bytes()),
        )
        .unwrap();
        let attach = WebToServer::AttachSession {
            session_id: p.session,
        };
        let attached =
            |message: &ServerToWeb| matches!(message, ServerToWeb::SessionMessages { .. });
        let autopilot_frames = |frames: &[ServerToWeb]| {
            frames
                .iter()
                .filter(|message| {
                    matches!(
                        message,
                        ServerToWeb::PaneAutopilotState { .. }
                            | ServerToWeb::PaneAutopilotReceipt { .. }
                    )
                })
                .count()
        };

        let (mut capable, negotiated) =
            connect(&url, &token, &[shared::TAB_AUTOPILOT_CAPABILITY]).await;
        assert_eq!(
            negotiated,
            vec![shared::TAB_AUTOPILOT_CAPABILITY.to_string()]
        );
        let (mut older, negotiated) = connect(&url, &token, &[]).await;
        assert!(negotiated.is_empty(), "{negotiated:?}");

        // The replay on attach: the OMP tab only, after the roster.
        send(&mut capable, attach.clone()).await;
        let frames = receive_until(&mut capable, attached).await;
        assert_eq!(
            autopilot_states(&frames),
            vec![(3, AutopilotSupport::Supported, reported_at)]
        );
        let roster_at = frames
            .iter()
            .position(|message| matches!(message, ServerToWeb::PaneList { .. }))
            .expect("the roster is sent");
        let state_at = frames
            .iter()
            .position(|message| matches!(message, ServerToWeb::PaneAutopilotState { .. }))
            .unwrap();
        assert!(roster_at < state_at, "{frames:?}");
        send(&mut older, attach).await;
        let frames = receive_until(&mut older, attached).await;
        assert_eq!(autopilot_frames(&frames), 0, "{frames:?}");

        // A request is forwarded to the tab's instance and answered to its
        // sender when that instance answers.
        let stop = |request_id: &str| WebToServer::ControlPaneAutopilot {
            session_id: p.session,
            pane_id: 3,
            request_id: request_id.to_string(),
            action: autopilot_stop(),
        };
        send(&mut capable, stop("stop-1")).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(ServerToCli::ControlPaneAutopilot { request_id, .. }) =
                    a.rx.recv().await
                {
                    assert_eq!(request_id, "stop-1");
                    return;
                }
            }
        })
        .await
        .expect("the request reaches the tab's instance");
        assert!(answer(&p, &a, 3, "stop-1", Accepted, None));
        let frames = receive_until(&mut capable, |message| {
            matches!(message, ServerToWeb::PaneAutopilotReceipt { .. })
        })
        .await;
        assert_eq!(receipts(&frames), vec![receipt("stop-1", Accepted, None)]);

        // The other client's request is not forwarded, and neither the
        // refusal nor a tab's next report reaches it as an autopilot frame.
        send(&mut older, stop("stop-2")).await;
        send(&mut older, WebToServer::Heartbeat).await;
        let frames = receive_until(&mut older, |message| {
            matches!(message, ServerToWeb::Heartbeat)
        })
        .await;
        assert_eq!(errors(&frames).len(), 1, "{frames:?}");
        p.state.sessions.report_pane_autopilot(
            &p.session,
            a.id,
            3,
            AutopilotSupport::NotReported,
            None,
            None,
        );
        let frames = receive_until(&mut capable, |message| {
            matches!(message, ServerToWeb::PaneAutopilotState { .. })
        })
        .await;
        assert_eq!(
            autopilot_states(&frames),
            vec![(3, AutopilotSupport::NotReported, None)]
        );
        send(&mut older, WebToServer::Heartbeat).await;
        let frames = receive_until(&mut older, |message| {
            matches!(message, ServerToWeb::Heartbeat)
        })
        .await;
        assert_eq!(autopilot_frames(&frames), 0, "{frames:?}");
        assert!(forwarded(&mut a).is_empty(), "stop-2 never reached the tab");
        server.abort();
    }

    /// The request is handled on the requester's read loop, so nothing in it
    /// waits for the tab's instance. When that instance's connection is
    /// stalled the request is refused at once, saying nothing was sent, and
    /// is not kept for later; a request that did reach the instance is still
    /// answered when it answers.
    #[tokio::test]
    async fn a_stalled_instance_does_not_hold_up_the_requesters_connection() {
        let p = project().await;
        // An instance whose connection takes one frame and then stalls.
        let mut a = {
            let id = Uuid::new_v4();
            let (tx, rx) = mpsc::channel(1);
            p.state.sessions.register_cli(id, p.host, tx, None);
            p.state.sessions.set_cli_capabilities(
                id,
                AUTOPILOT_CLI
                    .iter()
                    .map(|capability| capability.to_string())
                    .collect(),
            );
            assert!(matches!(
                p.state.sessions.attach_cli_instance(
                    p.session,
                    id,
                    Some(p.m5),
                    Some(format!("/work/{HOST_A}")),
                    Some(HOST_A.to_string()),
                ),
                AttachOutcome::Attached { .. }
            ));
            Endpoint { id, rx }
        };
        let mut web = p.viewer(p.owner, AUTOPILOT_VIEWER);
        p.roster(vec![omp_tab(3, p.m5, HOST_A)]);
        assert!(a.drain().is_empty());

        let pause = shared::AutopilotAction::Pause { autopilot_id: None };
        autopilot(&p, &web.id, 3, "pause-1", pause).await;
        let sent = std::time::Instant::now();
        for (name, action) in autopilot_actions() {
            let id = format!("stalled-{name}");
            autopilot(&p, &web.id, 3, &id, action).await;
            assert_eq!(
                receipts(&web.drain()),
                vec![receipt(
                    &id,
                    Refused,
                    Some("The project runtime is unavailable; nothing was sent to the tab.")
                )]
            );
            assert!(
                !answer(&p, &a, 3, &id, Accepted, None),
                "nothing waits for a request that was never sent"
            );
        }
        assert!(
            sent.elapsed() < Duration::from_secs(2),
            "no request waited on the stalled connection"
        );

        assert_eq!(
            forwarded(&mut a)
                .into_iter()
                .map(|(_, request_id, _)| request_id)
                .collect::<Vec<_>>(),
            vec!["pause-1".to_string()]
        );
        assert!(answer(&p, &a, 3, "pause-1", Accepted, None));
        assert_eq!(
            next_receipt(&mut web).await,
            receipt("pause-1", Accepted, None)
        );
    }

    /// A tab that names neither a machine nor a host is one no attached
    /// machine runs once several run the project. Routing would hand its
    /// controls to the primary; an autopilot request is refused instead, for
    /// a stop as for a start.
    #[tokio::test]
    async fn an_autopilot_request_for_a_tab_no_machine_runs_is_not_sent_to_the_primary() {
        let p = project().await;
        let mut a = p.instance_with(p.m5, HOST_A, AUTOPILOT_CLI);
        let mut b = p.instance_with(p.m6, HOST_B, AUTOPILOT_CLI);
        let mut web = p.viewer(p.owner, AUTOPILOT_VIEWER);
        let mut unowned = omp_tab(3, p.m5, HOST_A);
        unowned.machine_id = None;
        unowned.hostname = None;
        p.roster(vec![unowned]);

        for (name, action) in autopilot_actions() {
            autopilot(&p, &web.id, 3, name, action).await;
            assert_eq!(
                receipts(&web.drain()),
                vec![receipt(
                    name,
                    Refused,
                    Some("This tab's machine is not running this project.")
                )]
            );
        }
        assert!(forwarded(&mut a).is_empty() && forwarded(&mut b).is_empty());
    }

    /// A tab's provider can be switched from the web. A tab switched away
    /// from OMP has no autopilot left, and what its OMP agent last reported
    /// does not come back if it is switched to OMP again. A tab switched to
    /// OMP is a new OMP terminal tab: viewers that negotiated autopilot are
    /// told whether its instance can run autopilot, which the instance's own
    /// roster report, changing nothing, would never say.
    #[tokio::test]
    async fn a_tab_switched_to_omp_from_the_web_tells_capable_viewers_its_availability() {
        let p = project().await;
        let a = p.instance_with(p.m5, HOST_A, AUTOPILOT_CLI);
        // zoo-006 has not been rebooted to update yet.
        let _b = p.instance(p.m6, HOST_B);
        let mut web = p.viewer(p.owner, AUTOPILOT_VIEWER);
        let mut older_web = p.viewer(p.owner, &[PROJECT_INSTANCES_CAPABILITY]);
        p.roster(vec![
            omp_tab(3, p.m5, HOST_A),
            pane(4, Some(p.m6), Some(HOST_B)),
        ]);
        p.state.sessions.report_pane_autopilot(
            &p.session,
            a.id,
            3,
            AutopilotSupport::Supported,
            Some(waiting_autopilot()),
            Some("2026-11-13T14:07:00Z".to_string()),
        );
        web.drain();
        let switch = |pane_id: u32, provider: shared::Provider| {
            edit_roster_pane(&p.state, p.session, pane_id, move |pane| {
                pane.provider = provider
            })
        };

        assert!(switch(3, shared::Provider::Claude).await);
        assert!(p
            .state
            .sessions
            .cached_pane_autopilot(&p.session, 3)
            .is_none());
        assert!(autopilot_states(&web.drain()).is_empty());

        assert!(switch(3, shared::Provider::Omp).await);
        assert_eq!(
            autopilot_states(&web.drain()),
            vec![(3, AutopilotSupport::NotReported, None)],
            "the old agent's state is gone"
        );
        assert!(switch(4, shared::Provider::Omp).await);
        assert_eq!(
            autopilot_states(&web.drain()),
            vec![(4, AutopilotSupport::InstanceUpdateRequired, None)]
        );

        // An edit that leaves the provider alone says nothing.
        assert!(
            edit_roster_pane(&p.state, p.session, 3, |pane| {
                pane.label = Some("checks".to_string())
            })
            .await
        );
        assert!(autopilot_states(&web.drain()).is_empty());
        assert!(autopilot_states(&older_web.drain()).is_empty());
    }
}
