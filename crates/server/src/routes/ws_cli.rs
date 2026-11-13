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
use std::collections::HashSet;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::routes::auth::verify_token;
use crate::session::{AttachOutcome, InboundPaneVerdict, InstanceDetach, SessionLeave};
use crate::state::AppState;

/// Which attached instance may send an inbound CLI message, by the session
/// and pane it is about. A session can have one instance per machine, each
/// owning the panes tagged with its machine, so "a CLI of this session" no
/// longer vouches for every pane. Every variant is classified by
/// [`inbound_scope`], which has no wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InboundScope {
    /// About the connection, not a session: registration, heartbeats,
    /// per-CLI usage limits, and summary results, which the summary service
    /// matches to the exact CLI it asked.
    Connection,
    /// `SessionStart`: the attach itself, which the admission rules decide.
    Attach,
    /// Session-level reports any attached instance may send: its own roster
    /// (merged per machine), lifecycle reports, the legacy policy snapshot,
    /// `SessionEnd`, and output that names no pane.
    Session(Uuid),
    /// Legacy single-valued project state that every instance rebroadcasts.
    /// Only the primary's copy is used, so two instances cannot make it flap.
    Primary(Uuid),
    /// One pane's output, state or status: accepted only from the instance
    /// that owns the pane.
    Pane(Uuid, u32),
}

fn inbound_scope(message: &CliToServer) -> InboundScope {
    use InboundScope::{Attach, Connection, Pane, Primary, Session};
    // Older CLIs name the legacy panes by type rather than id.
    let pane_or_session =
        |session_id: &Uuid, pane_id: &Option<u32>, pane_type: &Option<PaneType>| {
            let legacy = pane_type.map(|pane_type| PaneConfig::pane_id_from_legacy(&pane_type));
            match pane_id.or(legacy) {
                Some(pane_id) => Pane(*session_id, pane_id),
                None => Session(*session_id),
            }
        };
    match message {
        CliToServer::Register { .. }
        | CliToServer::Heartbeat
        | CliToServer::UsageLimits { .. }
        | CliToServer::PaneWorkSummaryResult { .. } => Connection,
        CliToServer::SessionStart { .. } => Attach,
        CliToServer::Output {
            session_id,
            pane_type,
            pane_id,
            ..
        }
        | CliToServer::StreamMessage {
            session_id,
            pane_type,
            pane_id,
            ..
        }
        | CliToServer::UserInput {
            session_id,
            pane_type,
            pane_id,
            ..
        } => pane_or_session(session_id, pane_id, pane_type),
        CliToServer::PaneStatus {
            session_id,
            pane_type,
            pane_id,
            ..
        } => Pane(
            *session_id,
            pane_id.unwrap_or_else(|| PaneConfig::pane_id_from_legacy(pane_type)),
        ),
        // The legacy deadloop report names no pane, but it is the deadloop
        // pane's, as `ServerToCli::PauseDeadloop` is.
        CliToServer::DeadloopStatus { session_id, .. } => {
            Pane(*session_id, shared::PANE_ID_DEADLOOP)
        }
        CliToServer::PanePaused {
            session_id,
            pane_id,
            ..
        }
        | CliToServer::PlanReviewRequest {
            session_id,
            pane_id,
            ..
        }
        | CliToServer::PaneDiff {
            session_id,
            pane_id,
            ..
        }
        | CliToServer::PrCreated {
            session_id,
            pane_id,
            ..
        }
        | CliToServer::TerminalOutput {
            session_id,
            pane_id,
            ..
        }
        | CliToServer::TerminalExited {
            session_id,
            pane_id,
            ..
        }
        | CliToServer::TerminalState {
            session_id,
            pane_id,
            ..
        }
        | CliToServer::OmpQuestionAnswerResult {
            session_id,
            pane_id,
            ..
        }
        | CliToServer::PaneAutopilotReceipt {
            session_id,
            pane_id,
            ..
        }
        | CliToServer::PaneAutopilotState {
            session_id,
            pane_id,
            ..
        } => Pane(*session_id, *pane_id),
        CliToServer::SessionEnd { session_id, .. }
        | CliToServer::PaneList { session_id, .. }
        | CliToServer::ProjectPolicySnapshot { session_id, .. }
        | CliToServer::CliLifecycleInventory { session_id, .. }
        | CliToServer::CliLifecycleStatus { session_id, .. } => Session(*session_id),
        CliToServer::ProjectFlagsChanged { session_id, .. } => Primary(*session_id),
    }
}

/// Whether to handle an inbound message from `cli_id`, by its
/// [`InboundScope`]. A refused message is dropped with a warning, once per
/// session and pane for each connection: a replaced CLI can keep streaming
/// terminal frames, and one line per frame would drown the log.
fn admit_inbound(
    state: &AppState,
    cli_id: Uuid,
    message: &CliToServer,
    warned: &mut HashSet<(Uuid, Option<u32>)>,
) -> bool {
    let (session_id, pane_id, refusal) = match inbound_scope(message) {
        InboundScope::Connection | InboundScope::Attach => return true,
        InboundScope::Primary(session_id) => {
            // Every instance rebroadcasts these; only the primary's copy is
            // the session's, so the rest are expected and dropped quietly.
            return state
                .sessions
                .primary_instance(&session_id)
                .is_some_and(|primary| primary.cli_id == cli_id);
        }
        InboundScope::Session(session_id) => {
            if state.sessions.instance_for_cli(&session_id, &cli_id).is_some() {
                return true;
            }
            (session_id, None, "it runs no instance of the session".to_string())
        }
        InboundScope::Pane(session_id, pane_id) => {
            match state.sessions.inbound_pane_verdict(&session_id, &cli_id, pane_id) {
                InboundPaneVerdict::Accept => return true,
                InboundPaneVerdict::NotAttached => (
                    session_id,
                    Some(pane_id),
                    "it runs no instance of the session".to_string(),
                ),
                InboundPaneVerdict::Foreign { owner_hostname } => (
                    session_id,
                    Some(pane_id),
                    format!(
                        "the pane belongs to the instance on {}",
                        owner_hostname.as_deref().unwrap_or("another machine")
                    ),
                ),
            }
        }
    };
    if warned.insert((session_id, pane_id)) {
        // Serialized only for this first report, never per streamed frame.
        let kind = serde_json::to_value(message)
            .ok()
            .and_then(|value| value.get("type").and_then(|kind| kind.as_str()).map(str::to_string))
            .unwrap_or_default();
        tracing::warn!(
            %cli_id,
            %session_id,
            ?pane_id,
            kind,
            "dropping a CLI message: {refusal}"
        );
    } else {
        tracing::debug!(%cli_id, %session_id, ?pane_id, "dropping a CLI message: {refusal}");
    }
    false
}

/// Load `panes.json` into the session's roster before an instance's report
/// merges into it. After a server restart the cached roster is empty, and a
/// capable instance's report merged into nothing would drop every stopped
/// machine's tabs. A failed read is tried again at the next report; see
/// [`load_persisted_pane_roster`].
async fn ensure_pane_roster_loaded(state: &AppState, session_id: &Uuid) {
    ensure_forgotten_panes_loaded(state, session_id).await;
    if state.sessions.pane_roster_seeded(session_id) {
        return;
    }
    load_persisted_pane_roster(state, session_id).await;
}

/// What reading a session's `panes.json` found.
pub(crate) enum PersistedRoster {
    /// The saved roster seeded the session's, unless something seeded it
    /// first; the roster now held.
    Loaded(Vec<PaneConfig>),
    /// Nothing is saved: nothing ever was, or a file that does not parse was
    /// moved aside.
    Absent,
    /// The file is there but could not be read. Until a read succeeds the
    /// roster is not seeded, so nothing saves over the file.
    Unreadable,
}

/// Read `panes.json` into the session's unseeded roster.
///
/// A file that cannot be read now, for example while descriptors run out,
/// may hold tabs nothing else knows, such as a stopped machine's. Reports
/// still merge for viewers to see, but the roster stays unseeded and unsaved
/// until a later load reads the file; merging into nothing and saving that
/// would have erased those tabs for good. When a read then succeeds, the
/// reports merged meanwhile are laid over the saved roster and every instance
/// is asked to report again, so what they dropped meanwhile goes too. A file
/// that does not parse never will, and would leave the roster unsaved for
/// good, so it is moved aside for an operator and treated as never saved.
pub(crate) async fn load_persisted_pane_roster(
    state: &AppState,
    session_id: &Uuid,
) -> PersistedRoster {
    match state.storage.load_pane_roster(session_id).await {
        Ok(Some(stored)) => {
            let recovering = state.sessions.pane_roster_unreadable(session_id);
            let roster = state.sessions.hydrate_pane_roster(session_id, stored);
            if recovering {
                tracing::info!(
                    %session_id,
                    "read the persisted pane roster after a failed read; asking every instance to report again"
                );
                state
                    .sessions
                    .route_to_cli(
                        session_id,
                        ServerToCli::RequestPaneList {
                            session_id: *session_id,
                        },
                    )
                    .await;
            }
            PersistedRoster::Loaded(roster)
        }
        // Never saved: there are no stopped tabs to keep, and the roster
        // stays unknown until an instance reports one.
        Ok(None) => {
            state.sessions.clear_pane_roster_unreadable(session_id);
            PersistedRoster::Absent
        }
        Err(error) if error.downcast_ref::<serde_json::Error>().is_some() => {
            match state.storage.set_aside_pane_roster(session_id).await {
                Ok(aside) => tracing::error!(
                    %session_id,
                    %error,
                    aside = %aside.display(),
                    "the persisted pane roster does not parse; moved it aside and treating the roster as never saved"
                ),
                // Another load moved it first.
                Err(set_aside)
                    if set_aside
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound) => {}
                Err(set_aside) => {
                    tracing::error!(
                        %session_id,
                        %error,
                        %set_aside,
                        "the persisted pane roster does not parse and could not be moved aside; it will not be saved over"
                    );
                    state.sessions.mark_pane_roster_unreadable(session_id);
                    return PersistedRoster::Unreadable;
                }
            }
            state.sessions.clear_pane_roster_unreadable(session_id);
            PersistedRoster::Absent
        }
        Err(error) => {
            tracing::warn!(
                %session_id,
                %error,
                "failed to read the persisted pane roster; it will not be saved over until a read succeeds"
            );
            state.sessions.mark_pane_roster_unreadable(session_id);
            PersistedRoster::Unreadable
        }
    }
}

/// Read the session's forgotten pane ids once, before anything allocates an
/// id against them or saves them. Every path that hands out a pane id runs
/// behind an instance's `SessionStart`, which loads the roster and with it
/// these. A failed read leaves them unloaded, so nothing overwrites the file,
/// and the next roster load tries again.
pub(crate) async fn ensure_forgotten_panes_loaded(state: &AppState, session_id: &Uuid) {
    if state.sessions.forgotten_panes_loaded(session_id) {
        return;
    }
    match state.storage.load_forgotten_panes(session_id).await {
        Ok(stored) => state.sessions.hydrate_forgotten_panes(session_id, stored),
        Err(error) => tracing::warn!(
            %session_id,
            %error,
            "failed to load the forgotten pane ids; new panes may reuse them"
        ),
    }
}

/// Persist the session's current roster, tags and attributed hosts
/// included, so stopped instances' tabs survive a server restart, and the
/// ids of forgotten tabs, which stay unavailable to new panes across one.
pub(crate) async fn persist_pane_roster(state: &AppState, session_id: &Uuid) {
    if let Err(error) = state
        .storage
        .save_pane_roster_latest(session_id, || state.sessions.pane_roster_snapshot(session_id))
        .await
    {
        tracing::warn!("Failed to persist pane list for session {}: {}", session_id, error);
    }
    if let Err(error) = state
        .storage
        .save_forgotten_panes_latest(session_id, || {
            state.sessions.forgotten_panes_snapshot(session_id)
        })
        .await
    {
        tracing::warn!(%session_id, %error, "failed to persist the forgotten pane ids");
    }
}

/// A refused `SessionStart` also fails the new panes waiting for that
/// machine's instance, saying why. Its daemon has already answered that the
/// runtime started, so without this they would wait out their time limit and
/// be told the machine "did not start this project".
fn refuse_waiting_panes(
    state: &AppState,
    session_id: Uuid,
    machine_id: Option<Uuid>,
    hostname: Option<&str>,
    why: &str,
) {
    let Some(machine_id) = machine_id else {
        return;
    };
    let host = state
        .sessions
        .machine_hostname(&machine_id)
        .or_else(|| hostname.map(str::to_string))
        .map(|host| host.trim().to_string())
        .filter(|host| !host.is_empty())
        .unwrap_or_else(|| "The machine".to_string());
    state.sessions.refuse_instance_start(
        session_id,
        machine_id,
        &format!("{host} started this project, but the server refused its instance: {why}"),
    );
}

/// Move the session's database row after an instance left: inactive when it
/// was the last, else to the new primary when the primary left. Both writes
/// apply only while the row still names the departing CLI, so a reconnect
/// that already rewrote the row is never undone.
async fn persist_instance_detach(state: &AppState, detach: &InstanceDetach) {
    let session_id = detach.session_id.to_string();
    let departing = detach.instance.cli_id.to_string();
    let result = match &detach.primary {
        None => state.db.clear_session_cli(&session_id, &departing).await,
        Some(primary) if detach.was_primary => {
            state
                .db
                .hand_over_session_primary(
                    &session_id,
                    &departing,
                    &primary.cli_id.to_string(),
                    primary.working_dir.as_deref(),
                    primary.hostname.as_deref(),
                )
                .await
        }
        Some(_) => return,
    };
    if let Err(error) = result {
        tracing::error!(%session_id, %error, "Failed to update the session after an instance left");
    }
}

/// Relay a lifecycle inventory or status to the session's viewers that may
/// still see the session.
///
/// A viewer that did not negotiate [`shared::PROJECT_INSTANCES_CAPABILITY`]
/// keeps one inventory per session, which any inventory it receives
/// replaces, and its reboot and lifecycle requests name no machine and so
/// reach the primary. It is sent only the primary's inventory, and another
/// machine's status without the inventory that status carries; otherwise its
/// reboot confirmation would count another machine's tabs while the reboot
/// restarted the primary's. `replay_instances_after_attach` gives such a
/// viewer the primary's inventory alone for the same reason.
/// `only_older_viewers` leaves out the viewers that keep one inventory per
/// machine.
pub(crate) async fn route_lifecycle_to_authorized_web(
    state: &AppState,
    session_id: Uuid,
    message: ServerToWeb,
    only_older_viewers: bool,
) {
    let web_ids = state
        .sessions
        .get_session(&session_id)
        .map(|session| session.web_connection_ids)
        .unwrap_or_default();
    let primary_machine = state
        .sessions
        .primary_instance(&session_id)
        .map(|primary| primary.machine_id);
    for web_id in web_ids {
        let per_machine = state
            .sessions
            .web_supports_capability(&web_id, shared::PROJECT_INSTANCES_CAPABILITY);
        if per_machine && only_older_viewers {
            continue;
        }
        let authorized = match state.sessions.get_web_user(&web_id) {
            Some(user_id) => state
                .db
                .check_session_access(&session_id.to_string(), &user_id.to_string())
                .await
                .unwrap_or(false),
            None => false,
        };
        if !authorized {
            continue;
        }
        let frame = if per_machine {
            Some(message.clone())
        } else {
            single_inventory_frame(&message, primary_machine)
        };
        if let Some(frame) = frame {
            state.sessions.send_to_web(&web_id, frame).await;
        }
    }
}

/// `message` as a viewer that keeps one lifecycle inventory per session, the
/// primary's, receives it, or `None` for nothing. `primary_machine` is the
/// primary's machine (itself `None` for an instance that reported none), or
/// `None` with no instance attached, when every frame is the session's, as
/// with one instance it always was.
fn single_inventory_frame(
    message: &ServerToWeb,
    primary_machine: Option<Option<Uuid>>,
) -> Option<ServerToWeb> {
    let from_primary =
        |machine_id: &Option<Uuid>| primary_machine.is_none_or(|primary| primary == *machine_id);
    match message {
        ServerToWeb::CliLifecycleInventory { machine_id, .. } => {
            from_primary(machine_id).then(|| message.clone())
        }
        ServerToWeb::CliLifecycleStatus {
            session_id,
            request_id,
            operation,
            phase,
            message: text,
            inventory: Some(_),
            machine_id,
        } if !from_primary(machine_id) => Some(ServerToWeb::CliLifecycleStatus {
            session_id: *session_id,
            request_id: *request_id,
            operation: *operation,
            phase: *phase,
            message: text.clone(),
            inventory: None,
            machine_id: *machine_id,
        }),
        _ => Some(message.clone()),
    }
}

/// After the primary hands over, a viewer that keeps one lifecycle inventory
/// per session holds the departed primary's, while its next reboot request
/// reaches the new primary. Send it the new primary's, once that instance
/// has reported one.
async fn relay_primary_inventory_after_handover(state: &AppState, detach: &InstanceDetach) {
    let (true, Some(primary)) = (detach.was_primary, &detach.primary) else {
        return;
    };
    let Some((machine_id, inventory)) = state
        .sessions
        .attached_lifecycle_inventories(&detach.session_id)
        .into_iter()
        .find(|(machine, _)| *machine == primary.machine_id)
    else {
        return;
    };
    route_lifecycle_to_authorized_web(
        state,
        detach.session_id,
        ServerToWeb::CliLifecycleInventory {
            session_id: detach.session_id,
            inventory,
            machine_id,
        },
        true,
    )
    .await;
}

/// The advertised CLI capabilities this server echoes in `Registered`: only
/// those a CLI reads the echo of. A CLI that advertised
/// `omp_tab_autopilot_v1` and does not see it echoed tells its OMP tabs that
/// the server cannot control autopilot, and they hold. This server relays
/// autopilot state and controls, so it must echo it.
fn negotiated_cli_capabilities(advertised: &[String]) -> Vec<String> {
    [shared::OMP_TAB_AUTOPILOT_CAPABILITY]
        .into_iter()
        .filter(|negotiable| advertised.iter().any(|capability| capability == negotiable))
        .map(str::to_string)
        .collect()
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

fn ask_user_question_id(message: &shared::ClaudeStreamMessage) -> Option<String> {
    match message {
        shared::ClaudeStreamMessage::Assistant { message, .. } => {
            message.content.iter().find_map(|block| match block {
                shared::ClaudeContentBlock::ToolUse { id, name, .. }
                    if name.eq_ignore_ascii_case("AskUserQuestion") =>
                {
                    Some(id.clone())
                }
                _ => None,
            })
        }
        _ => None,
    }
}

fn recorded_tool_result_id(message: &shared::ClaudeStreamMessage) -> Option<String> {
    match message {
        shared::ClaudeStreamMessage::User { message, .. } => {
            message.content.iter().find_map(|block| match block {
                shared::ClaudeContentBlock::ToolResult { tool_use_id, .. } => {
                    Some(tool_use_id.clone())
                }
                _ => None,
            })
        }
        _ => None,
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
    // one-shot correlation to avoid a duplicate message, but tell the sender
    // the provider actually recorded it. A PTY write alone cannot prove that.
    let effective_pane_id =
        pane_id.or_else(|| pane_type.map(|pane| PaneConfig::pane_id_from_legacy(&pane)));
    if let Some((pane_id, echo)) = effective_pane_id.and_then(|pane_id| {
        state
            .sessions
            .consume_terminal_transcript_echo(&session_id, pane_id, &text)
            .map(|echo| (pane_id, echo))
    }) {
        state
            .sessions
            .route_to_web(
                &session_id,
                ServerToWeb::TerminalConversationRecorded {
                    session_id,
                    pane_id,
                    client_msg_id: echo.client_msg_id,
                },
            )
            .await;
        set_and_broadcast_pane_status(
            state,
            session_id,
            PaneType::Interactive,
            pane_id,
            Some("Working...".to_string()),
        )
        .await;
        record_and_broadcast_usage(
            state,
            session_id,
            Some(pane_id),
            crate::db::UsageDelta {
                prompt_count: 1,
                ..Default::default()
            },
        )
        .await;
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
        // Start the coarse working state only when the provider records input.
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
                                        let response = ServerToCli::Registered {
                                            cli_id,
                                            negotiated_capabilities: negotiated_cli_capabilities(
                                                &cli_capabilities,
                                            ),
                                        };
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

    // (session, pane) pairs whose refused messages were already logged.
    let mut refused_inbound: HashSet<(Uuid, Option<u32>)> = HashSet::new();

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
                        // A CLI speaks only for the sessions it runs an instance of, and
                        // for a pane only when its instance owns it. A replaced or
                        // rejected connection, or another machine's instance, is dropped.
                        if let Ok(message) = &parsed {
                            if !admit_inbound(&state, cli_id, message, &mut refused_inbound) {
                                continue;
                            }
                        }
                        match parsed {
                            Ok(CliToServer::SessionStart {
                                session_id,
                                project_id,
                                machine_id,
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
                                    refuse_waiting_panes(
                                        &state,
                                        session_id,
                                        machine_id,
                                        hostname.as_deref(),
                                        &reason,
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
                                // The host the session last recorded, read before this start can
                                // overwrite it: panes persisted before panes carried hosts ran there.
                                let mut recorded_host = None;
                                // Reject if this session_id is already owned by a different user
                                // (typically caused by .apas files copied/shared between users).
                                if let Ok(Some(existing)) =
                                    state.db.get_session(&session_id.to_string()).await
                                {
                                    recorded_host = existing.hostname.clone();
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
                                        // The CLI's reason names the session's owner; the
                                        // panes' requester is told only why.
                                        refuse_waiting_panes(
                                            &state,
                                            session_id,
                                            machine_id,
                                            hostname.as_deref(),
                                            "this project's session belongs to another account",
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

                                // Attach this machine's instance. A CLI that cannot run beside the
                                // instance already attached is refused with the host that runs it.
                                let (primary, replaced) = match state.sessions.attach_cli_instance(
                                    session_id,
                                    cli_id,
                                    machine_id,
                                    working_dir.clone(),
                                    hostname.clone(),
                                ) {
                                    AttachOutcome::Attached { primary, replaced } => {
                                        (primary, replaced)
                                    }
                                    AttachOutcome::Rejected { reason } => {
                                        tracing::warn!(
                                            "Rejecting SessionStart from CLI {} (user {}, session {}): {}",
                                            cli_id,
                                            user_id,
                                            session_id,
                                            reason
                                        );
                                        refuse_waiting_panes(
                                            &state,
                                            session_id,
                                            machine_id,
                                            hostname.as_deref(),
                                            &reason,
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
                                };
                                // The project behind a session is single-valued: the primary sets it,
                                // and a later instance only fills it in if it is somehow missing.
                                if primary
                                    || state.sessions.project_for_session(&session_id).is_none()
                                {
                                    state
                                        .sessions
                                        .set_session_project(session_id, project_id.to_string());
                                }

                                // What autopilot-capable viewers hold for each OMP tab, read
                                // before the roster is loaded and merged: a tab that either
                                // adds, or gives to this instance, is told to them below.
                                let autopilot_before =
                                    state.sessions.pane_autopilot_availabilities(&session_id);

                                // Merge this instance's roster into the project's, after loading the
                                // persisted one so other machines' stopped tabs survive a server
                                // restart. `None` means the CLI did not supply one and changes nothing;
                                // `Some([])` is an instance that genuinely has no panes, and discarding
                                // it left the server serving the roster from before its last pane was
                                // closed.
                                ensure_pane_roster_loaded(&state, &session_id).await;
                                let stamped = recorded_host.as_deref().is_some_and(|host| {
                                    state.sessions.stamp_untagged_pane_hosts(&session_id, host)
                                });
                                let reported: Option<HashSet<u32>> = panes
                                    .as_ref()
                                    .map(|panes| panes.iter().map(|pane| pane.pane_id).collect());
                                let merged = panes.and_then(|panes| {
                                    state.sessions.merge_instance_roster(&session_id, &cli_id, panes)
                                });
                                // Reserved ids this instance now lists are in the roster.
                                if let (Some(_), Some(reported)) = (&merged, &reported) {
                                    state.sessions.clear_reported_pane_reservations(
                                        &session_id,
                                        &cli_id,
                                        reported,
                                    );
                                }
                                if merged.is_some() || stamped {
                                    persist_pane_roster(&state, &session_id).await;
                                    // Forward to any already-attached web clients
                                    let panes = merged.unwrap_or_else(|| {
                                        state.sessions.get_session_panes(&session_id)
                                    });
                                    state
                                        .sessions
                                        .route_to_web(
                                            &session_id,
                                            ServerToWeb::PaneList { session_id, panes },
                                        )
                                        .await;
                                }
                                state
                                    .sessions
                                    .broadcast_pane_autopilot_changes(&session_id, &autopilot_before);

                                // The session's database row describes its primary instance. Another
                                // machine's instance must not rewrite its host or directory, or move
                                // shares keyed by them.
                                if primary {
                                    let session = crate::db::Session {
                                        id: session_id.to_string(),
                                        user_id: user_id.to_string(),
                                        cli_client_id: Some(cli_id.to_string()),
                                        working_dir: working_dir.clone(),
                                        hostname: hostname.clone(),
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
                                }

                                if state.sessions.cli_supports_capability(
                                    &cli_id,
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
                                // Keyed by the primary's host and directory like the row above, so a
                                // joining checkout never seeds the session from its own old project.
                                let previous_session = if primary {
                                    state
                                        .db
                                        .get_latest_project_session_id(
                                            &user_id.to_string(),
                                            working_dir.as_deref(),
                                            hostname.as_deref(),
                                            &session_id.to_string(),
                                        )
                                        .await
                                } else {
                                    Ok(None)
                                };
                                match previous_session {
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

                                // New panes waiting for this machine's instance can go to it now.
                                // Not sooner: its CLI refuses a new pane until it holds the
                                // project's policy, which was sent above.
                                if let Some(machine_id) = machine_id {
                                    state.sessions.instance_started(session_id, machine_id, cli_id);
                                }

                                tracing::info!(
                                    "CLI {} started local session {} (machine {:?}, primary {}, replaced {:?})",
                                    cli_id,
                                    session_id,
                                    machine_id,
                                    primary,
                                    replaced
                                );
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

                                let pending_question = ask_user_question_id(&message);
                                let recorded_answer = recorded_tool_result_id(&message);
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
                                if let Some(tool_use_id) = pending_question.as_ref() {
                                    state.sessions.register_pending_decision(
                                        session_id,
                                        tool_use_id.clone(),
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
                                        cost_reported: true,
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

                                if pending_question.is_some() {
                                    if let Some(pane_id) = effective_pane_id {
                                        set_and_broadcast_pane_status(
                                            &state,
                                            session_id,
                                            PaneType::Interactive,
                                            pane_id,
                                            Some(shared::PANE_STATUS_PENDING_ANSWER.to_string()),
                                        )
                                        .await;
                                    }
                                } else if let Some(tool_use_id) = recorded_answer {
                                    // The transcript is the acknowledgement for a
                                    // terminal answer. Keep Pending answer while
                                    // bytes are merely queued; only the recorded
                                    // tool result proves the picker settled and
                                    // the agent can resume work.
                                    let _ = state.sessions.claim_pending_decision(
                                        session_id,
                                        &tool_use_id,
                                        effective_pane_id,
                                        shared::MutationKind::Question,
                                    );
                                    if let Some(pane_id) = effective_pane_id {
                                        set_and_broadcast_pane_status(
                                            &state,
                                            session_id,
                                            PaneType::Interactive,
                                            pane_id,
                                            Some("Working...".to_string()),
                                        )
                                        .await;
                                    }
                                } else if terminal_assistant_reply {
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
                                // While another machine still runs the project, only this
                                // instance leaves; the session ends with its last instance.
                                match state.sessions.leave_session(&session_id, &cli_id) {
                                    SessionLeave::Detached(detach) => {
                                        persist_instance_detach(&state, &detach).await;
                                        relay_primary_inventory_after_handover(&state, &detach)
                                            .await;
                                        tracing::info!(
                                            "Instance {} left session {}: {}",
                                            cli_id,
                                            session_id,
                                            reason
                                        );
                                        continue;
                                    }
                                    SessionLeave::NotAttached => continue,
                                    SessionLeave::Last => {}
                                }
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
                                if let Some(current) = state.sessions.reconcile_terminal_state_from(
                                    Some(cli_id),
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
                                // Each instance has its own hosting capability and panes,
                                // so the inventory is kept and relayed per machine.
                                let machine_id = state
                                    .sessions
                                    .instance_for_cli(&session_id, &cli_id)
                                    .and_then(|instance| instance.machine_id);
                                state.sessions.set_lifecycle_inventory(
                                    session_id,
                                    machine_id,
                                    inventory.clone(),
                                );
                                route_lifecycle_to_authorized_web(
                                    &state,
                                    session_id,
                                    ServerToWeb::CliLifecycleInventory {
                                        session_id,
                                        inventory,
                                        machine_id,
                                    },
                                    false,
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
                                let machine_id = state
                                    .sessions
                                    .instance_for_cli(&session_id, &cli_id)
                                    .and_then(|instance| instance.machine_id);
                                let Some(status) =
                                    state.sessions.update_lifecycle_request_from_instance(
                                        request_id,
                                        session_id,
                                        machine_id,
                                        operation,
                                        phase,
                                        message,
                                        inventory,
                                    )
                                else {
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
                                route_lifecycle_to_authorized_web(
                                    &state,
                                    session_id,
                                    status.message(),
                                    false,
                                )
                                .await;
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
                            Ok(CliToServer::PaneList { session_id, panes }) => {
                                let Ok((_project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                tracing::info!("CLI {} sent pane list for session {}: {} panes", cli_id, session_id, panes.len());
                                // Merge this instance's panes into the project roster: it
                                // replaces only its own, keeps every other machine's, and
                                // keeps web-side labels and order. A pane mid-reboot (a close
                                // followed by an add on the CLI) keeps its place.
                                // Read before the roster is loaded, as at `SessionStart`: a
                                // tab that comes with a late load is new to viewers too.
                                let autopilot_before =
                                    state.sessions.pane_autopilot_availabilities(&session_id);
                                ensure_pane_roster_loaded(&state, &session_id).await;
                                let reported: HashSet<u32> =
                                    panes.iter().map(|pane| pane.pane_id).collect();
                                let Some(panes) =
                                    state.sessions.merge_instance_roster(&session_id, &cli_id, panes)
                                else {
                                    continue;
                                };
                                // Reserved ids this instance now lists are in the roster.
                                state.sessions.clear_reported_pane_reservations(
                                    &session_id,
                                    &cli_id,
                                    &reported,
                                );
                                persist_pane_roster(&state, &session_id).await;
                                let web_msg = ServerToWeb::PaneList {
                                    session_id,
                                    panes,
                                };
                                state.sessions.route_to_web(&session_id, web_msg).await;
                                // An OMP terminal tab this report added, or gave to this
                                // instance, is told to autopilot-capable viewers after the
                                // roster that lists it.
                                state
                                    .sessions
                                    .broadcast_pane_autopilot_changes(&session_id, &autopilot_before);
                            }
                            Ok(CliToServer::Register { .. }) => {
                                // Already registered, ignore
                            }
                            Ok(CliToServer::PaneAutopilotReceipt {
                                session_id,
                                pane_id,
                                request_id,
                                outcome,
                                reason,
                            }) => {
                                // Only the tab's instance gets this far, and the waiter
                                // also names the connection the request was sent to. The
                                // receipt says the tab took the request; what viewers are
                                // shown changes only with the tab's reported state.
                                state.sessions.finish_autopilot_request(
                                    session_id,
                                    pane_id,
                                    cli_id,
                                    &request_id,
                                    crate::session::AutopilotReceipt { outcome, reason },
                                );
                            }
                            Ok(CliToServer::PaneAutopilotState {
                                session_id,
                                pane_id,
                                support,
                                state: reported,
                                reported_at,
                            }) => {
                                let Ok((_project_id, _operation_guard)) = state
                                    .active_session_operation(&session_id.to_string())
                                    .await
                                else {
                                    continue;
                                };
                                // Cached for viewers that attach later, and relayed only to
                                // viewers that negotiated autopilot.
                                state.sessions.report_pane_autopilot(
                                    &session_id,
                                    cli_id,
                                    pane_id,
                                    support,
                                    reported,
                                    reported_at,
                                );
                            }
                            Ok(CliToServer::PaneWorkSummaryResult { result }) => {
                                let _ = state
                                    .pane_work_summaries
                                    .accept_result(cli_id, result)
                                    .await;
                            }
                            Ok(CliToServer::OmpQuestionAnswerResult {
                                session_id,
                                pane_id,
                                tool_use_id,
                                request_id,
                                accepted,
                                pending,
                                error,
                            }) => {
                                // Only the pane's instance gets this far, and the waiter
                                // also names the connection the answer was sent to.
                                state.sessions.finish_omp_answer(
                                    session_id, pane_id, cli_id, &tool_use_id, &request_id,
                                    crate::session::OmpAnswerReceipt { accepted, pending, error },
                                );
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
                                        // Policy is the project's, so every instance holds it.
                                        state
                                            .sessions
                                            .route_to_cli(
                                                &session_id,
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

    // Cleanup - detach this CLI's instances. Only their own terminals become
    // disconnected and only their statuses clear; a session goes inactive
    // only when its last instance leaves, and otherwise its row moves to the
    // next primary. Transport loss does not prove that this APAS process or
    // its PTYs ended, so their presentation is retained until a reconnect
    // reconciles each process instance.
    let detached = state.sessions.detach_cli_instances(&cli_id);
    for detach in &detached {
        persist_instance_detach(&state, detach).await;
        relay_primary_inventory_after_handover(&state, detach).await;
    }

    state.sessions.unregister_cli(&cli_id);
    let _ = state
        .db
        .update_cli_client_status(&cli_id.to_string(), "offline")
        .await;
    tracing::info!(
        "CLI client disconnected: {} (left {} sessions, {} now inactive)",
        cli_id,
        detached.len(),
        detached.iter().filter(|detach| detach.primary.is_none()).count()
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
    use super::{
        ask_user_question_id, is_terminal_pane, recorded_tool_result_id,
        terminal_assistant_completes_work,
    };
    use shared::{
        ClaudeAssistantMessage, ClaudeContentBlock, ClaudeStreamMessage, ClaudeUserMessage,
        PaneConfig, PaneKind,
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

    #[test]
    fn question_and_recorded_answer_events_are_identified_by_tool_id() {
        let question = ClaudeStreamMessage::Assistant {
            message: ClaudeAssistantMessage {
                content: vec![ClaudeContentBlock::ToolUse {
                    id: "question-7".to_string(),
                    name: "AskUserQuestion".to_string(),
                    input: serde_json::json!({"questions": []}),
                }],
                model: String::new(),
                extra: serde_json::Value::Null,
            },
            session_id: "provider-session".to_string(),
            extra: serde_json::json!({"terminal_turn_complete": false}),
        };
        let answer = ClaudeStreamMessage::User {
            message: ClaudeUserMessage {
                content: vec![ClaudeContentBlock::ToolResult {
                    tool_use_id: "question-7".to_string(),
                    content: "recorded answer".to_string(),
                    is_error: false,
                }],
                role: "user".to_string(),
            },
            session_id: "provider-session".to_string(),
            tool_use_result: None,
            extra: serde_json::Value::Null,
        };

        assert_eq!(
            ask_user_question_id(&question).as_deref(),
            Some("question-7")
        );
        assert_eq!(
            recorded_tool_result_id(&answer).as_deref(),
            Some("question-7")
        );
        assert!(recorded_tool_result_id(&question).is_none());
        assert!(ask_user_question_id(&answer).is_none());
    }
}

#[cfg(test)]
mod instance_tests {
    use super::{inbound_scope, InboundScope};
    use crate::state::AppState;
    use futures::{SinkExt, StreamExt};
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::json;
    use shared::{
        CliLifecycleInventory, CliLifecycleOperation, CliLifecyclePhase, CliToServer,
        PaneConfig, PaneType, Provider, ServerToCli, ServerToWeb, TerminalLifecycle,
        UsageLimits, PANE_ID_DEADLOOP, PANE_ID_INTERACTIVE, PROJECT_INSTANCES_CAPABILITY,
    };
    use std::collections::BTreeSet;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio_tungstenite::{connect_async, tungstenite::Message};
    use uuid::Uuid;

    const HOST_A: &str = "zoo-005";
    const HOST_B: &str = "zoo-006";
    const SECRET: &str = "project-instances-test-secret";

    type CliSocket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    /// Every `CliToServer` wire tag, as serde lists them when it meets an
    /// unknown one, so the list cannot drift from the enum.
    fn every_cli_to_server_tag() -> BTreeSet<String> {
        let error = serde_json::from_str::<CliToServer>(r#"{"type":"__no_such_message__"}"#)
            .unwrap_err()
            .to_string();
        let expected = error
            .split("expected one of")
            .nth(1)
            .expect("serde lists the variants it expected");
        expected
            .split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .collect()
    }

    fn wire_tag(message: &CliToServer) -> String {
        serde_json::to_value(message).unwrap()["type"]
            .as_str()
            .expect("internally tagged message")
            .to_string()
    }

    /// One example of every inbound message with the instance it must come
    /// from. The table is the reviewed classification.
    fn classification(s: Uuid) -> Vec<(CliToServer, InboundScope)> {
        use InboundScope::{Attach, Connection, Pane, Primary, Session};
        let assistant = shared::ClaudeStreamMessage::Assistant {
            message: shared::ClaudeAssistantMessage {
                content: Vec::new(),
                model: String::new(),
                extra: serde_json::Value::Null,
            },
            session_id: "provider-session".to_string(),
            extra: serde_json::Value::Null,
        };
        vec![
            (
                CliToServer::Register {
                    token: "token".into(),
                    version: None,
                    capabilities: Vec::new(),
                },
                Connection,
            ),
            (CliToServer::Heartbeat, Connection),
            (
                CliToServer::UsageLimits {
                    provider: Provider::Claude,
                    limits: UsageLimits {
                        five_hour: None,
                        seven_day: None,
                        fetched_at: None,
                        usage_limited: None,
                    },
                },
                Connection,
            ),
            (
                CliToServer::PaneWorkSummaryResult {
                    result: serde_json::from_value(json!({
                        "job_id": Uuid::new_v4(),
                        "session_id": s,
                        "pane_id": 9,
                        "window_start": "2026-11-11T00:00:00Z",
                        "source_digest": "digest",
                    }))
                    .unwrap(),
                },
                Connection,
            ),
            (
                CliToServer::SessionStart {
                    session_id: s,
                    project_id: None,
                    machine_id: None,
                    working_dir: None,
                    hostname: None,
                    git_remote: None,
                    git_remote_url: None,
                    pane_type: None,
                    panes: None,
                },
                Attach,
            ),
            (
                CliToServer::Output {
                    session_id: s,
                    data: "out".into(),
                    output_type: Default::default(),
                    pane_type: None,
                    pane_id: Some(7),
                },
                Pane(s, 7),
            ),
            (
                CliToServer::StreamMessage {
                    session_id: s,
                    message: assistant,
                    pane_type: Some(PaneType::Deadloop),
                    pane_id: None,
                },
                Pane(s, PANE_ID_DEADLOOP),
            ),
            (
                CliToServer::UserInput {
                    session_id: s,
                    text: "hi".into(),
                    pane_type: None,
                    pane_id: None,
                },
                Session(s),
            ),
            (
                CliToServer::SessionEnd {
                    session_id: s,
                    reason: "done".into(),
                },
                Session(s),
            ),
            (
                CliToServer::DeadloopStatus {
                    session_id: s,
                    is_paused: true,
                },
                Pane(s, PANE_ID_DEADLOOP),
            ),
            (
                CliToServer::PanePaused {
                    session_id: s,
                    pane_id: 4,
                    is_paused: true,
                },
                Pane(s, 4),
            ),
            (
                CliToServer::PaneStatus {
                    session_id: s,
                    pane_type: PaneType::Interactive,
                    pane_id: None,
                    status: None,
                },
                Pane(s, PANE_ID_INTERACTIVE),
            ),
            (
                CliToServer::PaneList {
                    session_id: s,
                    panes: Vec::new(),
                },
                Session(s),
            ),
            (
                CliToServer::PlanReviewRequest {
                    session_id: s,
                    pane_id: 4,
                    tool_use_id: "toolu_1".into(),
                    tool_name: "Write".into(),
                    input: json!({}),
                },
                Pane(s, 4),
            ),
            (
                CliToServer::PaneDiff {
                    session_id: s,
                    pane_id: 4,
                    branch: None,
                    base: None,
                    diff: None,
                    error: None,
                },
                Pane(s, 4),
            ),
            (
                CliToServer::PrCreated {
                    session_id: s,
                    pane_id: 4,
                    url: None,
                    error: None,
                },
                Pane(s, 4),
            ),
            (
                CliToServer::ProjectFlagsChanged {
                    session_id: s,
                    auto_approve_todos: false,
                    auto_merge_prs: false,
                    team_enabled: false,
                    disallowed_tab_types: Vec::new(),
                },
                Primary(s),
            ),
            (
                CliToServer::ProjectPolicySnapshot {
                    session_id: s,
                    team_enabled: false,
                    disallowed_tab_types: Vec::new(),
                },
                Session(s),
            ),
            (
                CliToServer::TerminalOutput {
                    session_id: s,
                    pane_id: 4,
                    instance_id: None,
                    data_b64: "eA==".into(),
                    seq: 0,
                },
                Pane(s, 4),
            ),
            (
                CliToServer::TerminalExited {
                    session_id: s,
                    pane_id: 4,
                    instance_id: None,
                    status: None,
                },
                Pane(s, 4),
            ),
            (
                CliToServer::TerminalState {
                    session_id: s,
                    pane_id: 4,
                    instance_id: None,
                    lifecycle: TerminalLifecycle::Running,
                    status: None,
                    runtime: None,
                },
                Pane(s, 4),
            ),
            (
                CliToServer::CliLifecycleInventory {
                    session_id: s,
                    inventory: CliLifecycleInventory::default(),
                },
                Session(s),
            ),
            (
                CliToServer::CliLifecycleStatus {
                    session_id: s,
                    request_id: Uuid::new_v4(),
                    operation: CliLifecycleOperation::RebootCli,
                    phase: CliLifecyclePhase::Accepted,
                    message: None,
                    inventory: None,
                },
                Session(s),
            ),
            (
                CliToServer::OmpQuestionAnswerResult {
                    session_id: s,
                    pane_id: 4,
                    tool_use_id: "omp:1".into(),
                    request_id: "answer-1".into(),
                    accepted: true,
                    pending: false,
                    error: None,
                },
                Pane(s, 4),
            ),
            (
                CliToServer::PaneAutopilotReceipt {
                    session_id: s,
                    pane_id: 4,
                    request_id: "autopilot-1".into(),
                    outcome: shared::AutopilotReceiptOutcome::Accepted,
                    reason: None,
                },
                Pane(s, 4),
            ),
            (
                CliToServer::PaneAutopilotState {
                    session_id: s,
                    pane_id: 4,
                    support: shared::AutopilotSupport::Supported,
                    state: None,
                    reported_at: None,
                },
                Pane(s, 4),
            ),
        ]
    }

    /// `inbound_scope` has no wildcard arm, so a new message does not
    /// compile unclassified; this pins each classification and fails until
    /// a new message also has a row here.
    #[test]
    fn every_inbound_message_has_a_pinned_scope() {
        let session = Uuid::new_v4();
        let table = classification(session);
        let covered: BTreeSet<String> = table.iter().map(|(message, _)| wire_tag(message)).collect();
        assert_eq!(covered, every_cli_to_server_tag());
        for (message, expected) in &table {
            assert_eq!(inbound_scope(message), *expected, "{}", wire_tag(message));
        }
        // A pane named by id wins over the legacy type; output that names
        // neither belongs to the session.
        let named = CliToServer::Output {
            session_id: session,
            data: String::new(),
            output_type: Default::default(),
            pane_type: Some(PaneType::Deadloop),
            pane_id: Some(9),
        };
        assert_eq!(inbound_scope(&named), InboundScope::Pane(session, 9));
        let unnamed = CliToServer::StreamMessage {
            session_id: session,
            message: shared::ClaudeStreamMessage::Assistant {
                message: shared::ClaudeAssistantMessage {
                    content: Vec::new(),
                    model: String::new(),
                    extra: serde_json::Value::Null,
                },
                session_id: String::new(),
                extra: serde_json::Value::Null,
            },
            pane_type: None,
            pane_id: None,
        };
        assert_eq!(inbound_scope(&unnamed), InboundScope::Session(session));
    }

    struct Harness {
        state: AppState,
        url: String,
        user_id: Uuid,
        token: String,
        server: tokio::task::JoinHandle<()>,
        dir: std::path::PathBuf,
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            self.server.abort();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    async fn harness() -> Harness {
        let dir = std::env::temp_dir().join(format!("apas-ws-cli-instances-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("apas.db");
        let db = crate::db::Database::new(&db_path.to_string_lossy())
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let user_id = Uuid::new_v4();
        db.create_user(&crate::db::User {
            id: user_id.to_string(),
            email: format!("{user_id}@example.test"),
            password_hash: "unused".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .unwrap();
        let mut config = crate::config::Config::default();
        config.database.path = db_path.to_string_lossy().into_owned();
        config.auth.jwt_secret = SECRET.to_string();
        let state = AppState::new(db, config);
        let token = encode(
            &Header::default(),
            &crate::routes::auth::Claims {
                sub: user_id.to_string(),
                exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
                device_session_id: None,
                token_kind: None,
                credential_version: None,
            },
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = crate::routes::create_router(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Harness {
            state,
            url: format!("ws://{address}/ws/cli"),
            user_id,
            token,
            server,
            dir,
        }
    }

    async fn send(socket: &mut CliSocket, message: CliToServer) {
        socket
            .send(Message::Text(serde_json::to_string(&message).unwrap()))
            .await
            .unwrap();
    }

    async fn receive(socket: &mut CliSocket, matches: impl Fn(&ServerToCli) -> bool) -> ServerToCli {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let frame = socket.next().await.unwrap().unwrap();
                let Message::Text(text) = frame else { continue };
                let message = serde_json::from_str::<ServerToCli>(&text).unwrap();
                if matches(&message) {
                    return message;
                }
            }
        })
        .await
        .expect("timed out waiting for the server")
    }

    /// A registered CLI connection; `capable` advertises project instances.
    async fn connect_cli(harness: &Harness, capable: bool) -> (CliSocket, Uuid) {
        let mut capabilities = vec![shared::PROJECT_POLICY_CAPABILITY];
        if capable {
            capabilities.push(PROJECT_INSTANCES_CAPABILITY);
        }
        let (socket, cli_id, _) = register_cli(harness, &capabilities).await;
        (socket, cli_id)
    }

    /// A CLI connection registered with `capabilities`, and the capabilities
    /// the server echoed in its reply.
    async fn register_cli(
        harness: &Harness,
        capabilities: &[&str],
    ) -> (CliSocket, Uuid, Vec<String>) {
        let (mut socket, _) = connect_async(&harness.url).await.unwrap();
        send(
            &mut socket,
            CliToServer::Register {
                token: harness.token.clone(),
                version: Some("26.11.0".to_string()),
                capabilities: capabilities
                    .iter()
                    .map(|capability| capability.to_string())
                    .collect(),
            },
        )
        .await;
        match receive(&mut socket, |message| matches!(message, ServerToCli::Registered { .. })).await
        {
            ServerToCli::Registered {
                cli_id,
                negotiated_capabilities,
            } => (socket, cli_id, negotiated_capabilities),
            other => panic!("expected registration, got {other:?}"),
        }
    }

    fn pane(pane_id: u32) -> PaneConfig {
        let mut pane = PaneConfig::defaults().remove(0);
        pane.pane_id = pane_id;
        pane.kind = shared::PaneKind::Terminal;
        pane
    }

    fn session_start(
        session_id: Uuid,
        machine_id: Uuid,
        host: &str,
        panes: Option<Vec<PaneConfig>>,
    ) -> CliToServer {
        CliToServer::SessionStart {
            session_id,
            project_id: Some(session_id),
            machine_id: Some(machine_id),
            working_dir: Some(format!("/work/{host}")),
            hostname: Some(host.to_string()),
            git_remote: None,
            git_remote_url: None,
            pane_type: None,
            panes,
        }
    }

    /// Start an instance and wait for the server to finish its start, which
    /// ends by sending the project's policy.
    async fn start(socket: &mut CliSocket, message: CliToServer) {
        send(socket, message).await;
        receive(socket, |message| matches!(message, ServerToCli::ProjectPolicy { .. })).await;
    }

    async fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting until {what}"));
    }

    async fn db_row(harness: &Harness, session_id: Uuid) -> crate::db::Session {
        harness
            .state
            .db
            .get_session(&session_id.to_string())
            .await
            .unwrap()
            .expect("session row")
    }

    async fn eventually_row(
        harness: &Harness,
        session_id: Uuid,
        what: &str,
        condition: impl Fn(&crate::db::Session) -> bool,
    ) -> crate::db::Session {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let row = db_row(harness, session_id).await;
            if condition(&row) {
                return row;
            }
            assert!(tokio::time::Instant::now() < deadline, "timed out waiting until {what}: {row:?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// An in-memory web viewer of `session_id`, signed in as the owner.
    fn viewer(harness: &Harness, session_id: Uuid) -> mpsc::Receiver<ServerToWeb> {
        viewer_with(harness, session_id, &[])
    }

    /// [`viewer`] on a client that negotiated `capabilities`.
    fn viewer_with(
        harness: &Harness,
        session_id: Uuid,
        capabilities: &[&str],
    ) -> mpsc::Receiver<ServerToWeb> {
        let connection_id = Uuid::new_v4();
        let (tx, rx) = mpsc::channel(256);
        harness.state.sessions.register_web(connection_id, tx);
        harness.state.sessions.set_web_user(connection_id, harness.user_id);
        harness.state.sessions.set_web_capabilities(
            connection_id,
            capabilities
                .iter()
                .map(|capability| capability.to_string())
                .collect(),
        );
        assert!(harness
            .state
            .sessions
            .attach_web_to_session(&session_id, connection_id, None));
        rx
    }

    async fn receive_web(
        rx: &mut mpsc::Receiver<ServerToWeb>,
        matches: impl Fn(&ServerToWeb) -> bool,
    ) -> (ServerToWeb, Vec<ServerToWeb>) {
        let mut before = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let message = rx.recv().await.expect("web sender closed");
                if matches(&message) {
                    return (message, before);
                }
                before.push(message);
            }
        })
        .await
        .expect("timed out waiting for a web message")
    }

    fn terminal_output(session_id: Uuid, pane_id: u32) -> CliToServer {
        CliToServer::TerminalOutput {
            session_id,
            pane_id,
            instance_id: None,
            data_b64: "eA==".into(),
            seq: 0,
        }
    }

    /// Two machines' instances share the project's session. The session row
    /// stays the primary's, another machine's pane traffic is refused, and a
    /// departing primary hands the row to the remaining instance until the
    /// last one leaves.
    #[tokio::test]
    async fn two_machines_share_one_session_and_only_the_primary_writes_its_row() {
        let harness = harness().await;
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let (mut a, a_id) = connect_cli(&harness, true).await;
        let (mut b, b_id) = connect_cli(&harness, true).await;
        start(&mut a, session_start(session, m5, HOST_A, Some(vec![pane(3)]))).await;
        start(&mut b, session_start(session, m6, HOST_B, Some(vec![pane(4)]))).await;

        let sessions = &harness.state.sessions;
        assert_eq!(
            sessions
                .session_attachments(&session)
                .iter()
                .map(|instance| instance.cli_id)
                .collect::<Vec<_>>(),
            vec![a_id, b_id]
        );
        let row = db_row(&harness, session).await;
        assert_eq!(row.hostname.as_deref(), Some(HOST_A), "a second machine must not rewrite the row");
        assert_eq!(row.working_dir.as_deref(), Some("/work/zoo-005"));
        assert_eq!(row.cli_client_id, Some(a_id.to_string()));
        let roster = sessions.get_session_panes(&session);
        assert_eq!(
            roster
                .iter()
                .map(|pane| (pane.pane_id, pane.machine_id))
                .collect::<Vec<_>>(),
            vec![(3, Some(m5)), (4, Some(m6))]
        );
        let persisted = harness
            .state
            .storage
            .load_pane_roster(&session)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(persisted[1].hostname.as_deref(), Some(HOST_B), "tags are persisted");

        // Terminal output for zoo-005's pane from zoo-006 is dropped; its own
        // pane's output, sent after it on the same socket, gets through.
        let mut web = viewer(&harness, session);
        send(&mut b, terminal_output(session, 3)).await;
        send(&mut b, terminal_output(session, 4)).await;
        let (_, before) = receive_web(&mut web, |message| {
            matches!(message, ServerToWeb::TerminalOutput { pane_id: 4, .. })
        })
        .await;
        assert!(
            !before
                .iter()
                .any(|message| matches!(message, ServerToWeb::TerminalOutput { pane_id: 3, .. })),
            "{before:?}"
        );
        assert!(sessions.terminal_snapshot(&session, 3).is_none());

        // The primary leaves: the row moves to the remaining instance.
        a.close(None).await.unwrap();
        let row = eventually_row(&harness, session, "the row moves to zoo-006", |row| {
            row.cli_client_id.as_deref() == Some(b_id.to_string().as_str())
        })
        .await;
        assert_eq!(row.hostname.as_deref(), Some(HOST_B));
        assert_eq!(row.working_dir.as_deref(), Some("/work/zoo-006"));
        assert_eq!(row.status, "active");
        assert!(sessions.is_session_active(&session));
        assert_eq!(
            sessions.get_session_panes(&session).len(),
            2,
            "the departed machine's tabs stay listed"
        );

        b.close(None).await.unwrap();
        let row = eventually_row(&harness, session, "the session goes inactive", |row| {
            row.status == "inactive"
        })
        .await;
        assert_eq!(row.cli_client_id, None);
        eventually("no instance is attached", || !sessions.is_session_active(&session)).await;
    }

    /// An older CLI is turned away while another machine runs the project,
    /// and nothing it sends afterwards reaches the session.
    #[tokio::test]
    async fn an_older_cli_is_turned_away_and_ignored() {
        let harness = harness().await;
        let session = Uuid::new_v4();
        let (mut a, a_id) = connect_cli(&harness, true).await;
        let (mut older, _) = connect_cli(&harness, false).await;
        start(&mut a, session_start(session, Uuid::new_v4(), HOST_A, Some(vec![pane(3)]))).await;

        send(&mut older, session_start(session, Uuid::new_v4(), HOST_B, Some(vec![pane(9)]))).await;
        match receive(&mut older, |message| matches!(message, ServerToCli::SessionRejected { .. }))
            .await
        {
            ServerToCli::SessionRejected { session_id, reason } => {
                assert_eq!(session_id, session);
                assert!(reason.contains(HOST_A), "{reason}");
            }
            other => panic!("expected a rejection, got {other:?}"),
        }
        let mut web = viewer(&harness, session);
        send(&mut older, CliToServer::PaneList {
            session_id: session,
            panes: vec![pane(9)],
        })
        .await;
        send(&mut older, terminal_output(session, 3)).await;
        send(&mut a, terminal_output(session, 3)).await;
        let (_, before) = receive_web(&mut web, |message| {
            matches!(message, ServerToWeb::TerminalOutput { pane_id: 3, .. })
        })
        .await;
        assert!(before.iter().all(|message| !matches!(message, ServerToWeb::PaneList { .. })));
        let row = db_row(&harness, session).await;
        assert_eq!(row.cli_client_id, Some(a_id.to_string()));
        assert_eq!(row.hostname.as_deref(), Some(HOST_A));
        assert_eq!(
            harness
                .state
                .sessions
                .get_session_panes(&session)
                .iter()
                .map(|pane| pane.pane_id)
                .collect::<Vec<_>>(),
            vec![3]
        );
    }

    /// A new pane waiting for the instance the server asked a machine's
    /// daemon to start fails as soon as that instance's `SessionStart` is
    /// refused, with the reason, rather than at its time limit as a start
    /// that never happened.
    #[tokio::test]
    async fn a_refused_instance_fails_the_panes_waiting_for_it() {
        let harness = harness().await;
        let session = Uuid::new_v4();
        let m6 = Uuid::new_v4();
        let sessions = &harness.state.sessions;
        let (mut older, _) = connect_cli(&harness, false).await;
        start(&mut older, session_start(session, Uuid::new_v4(), HOST_A, Some(vec![pane(3)]))).await;
        let crate::session::pane_launches::InstanceStartWait::Waiting {
            waiter,
            start: Some(_),
            outcome,
        } = sessions.wait_for_instance_start(session, m6, Some(&session.to_string()))
        else {
            panic!("the pane begins a start of zoo-006's instance");
        };

        // Its daemon started the runtime, whose CLI is refused beside the
        // older one.
        let (mut b, _) = connect_cli(&harness, true).await;
        send(&mut b, session_start(session, m6, HOST_B, Some(Vec::new()))).await;
        receive(&mut b, |message| matches!(message, ServerToCli::SessionRejected { .. })).await;

        match sessions
            .await_instance_start(waiter, outcome, Duration::from_secs(5))
            .await
        {
            Some(crate::session::pane_launches::InstanceStartOutcome::Refused(reason)) => {
                assert!(
                    reason.starts_with(&format!(
                        "{HOST_B} started this project, but the server refused its instance:"
                    )) && reason.contains(HOST_A),
                    "{reason}"
                );
            }
            other => panic!("expected the refusal, got {other:?}"),
        }
    }

    /// A start without a roster changes nothing, an empty one removes only
    /// that machine's tabs, inventories name their machine, and only the
    /// primary's copy of the legacy flags is relayed. A viewer that keeps
    /// one inventory per session gets only the primary's, also after the
    /// primary hands over.
    #[tokio::test]
    async fn instance_reports_stay_scoped_to_their_machine() {
        let harness = harness().await;
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let (mut a, _) = connect_cli(&harness, true).await;
        let (mut b, _) = connect_cli(&harness, true).await;
        start(&mut a, session_start(session, m5, HOST_A, Some(vec![pane(3)]))).await;
        start(&mut b, session_start(session, m6, HOST_B, None)).await;
        let sessions = &harness.state.sessions;
        assert_eq!(sessions.get_session_panes(&session).len(), 1, "None changes nothing");

        let mut web = viewer(&harness, session);
        let mut per_machine = viewer_with(&harness, session, &[PROJECT_INSTANCES_CAPABILITY]);
        send(&mut b, CliToServer::PaneList {
            session_id: session,
            panes: vec![pane(4)],
        })
        .await;
        let (list, _) =
            receive_web(&mut web, |message| matches!(message, ServerToWeb::PaneList { .. })).await;
        let ServerToWeb::PaneList { panes, .. } = list else { unreachable!() };
        assert_eq!(panes.iter().map(|pane| pane.pane_id).collect::<Vec<_>>(), vec![3, 4]);
        send(&mut b, CliToServer::PaneList {
            session_id: session,
            panes: Vec::new(),
        })
        .await;
        let (list, _) =
            receive_web(&mut web, |message| matches!(message, ServerToWeb::PaneList { .. })).await;
        let ServerToWeb::PaneList { panes, .. } = list else { unreachable!() };
        assert_eq!(
            panes.iter().map(|pane| pane.pane_id).collect::<Vec<_>>(),
            vec![3],
            "Some([]) removes only zoo-006's tabs"
        );

        // zoo-006's copy of the legacy flags is not the session's; its
        // inventory, sent after it on the same socket, is relayed by machine
        // to a viewer that keeps one per machine.
        send(&mut b, CliToServer::ProjectFlagsChanged {
            session_id: session,
            auto_approve_todos: false,
            auto_merge_prs: false,
            team_enabled: false,
            disallowed_tab_types: Vec::new(),
        })
        .await;
        send(&mut b, CliToServer::CliLifecycleInventory {
            session_id: session,
            inventory: CliLifecycleInventory {
                persistent_terminal_hosting: true,
                panes: Vec::new(),
            },
        })
        .await;
        let (inventory, before) = receive_web(&mut per_machine, |message| {
            matches!(message, ServerToWeb::CliLifecycleInventory { .. })
        })
        .await;
        assert!(matches!(
            inventory,
            ServerToWeb::CliLifecycleInventory { machine_id: Some(machine), .. } if machine == m6
        ));
        assert!(
            !before
                .iter()
                .any(|message| matches!(message, ServerToWeb::ProjectFlagsChanged { .. })),
            "{before:?}"
        );
        assert_eq!(
            sessions
                .attached_lifecycle_inventories(&session)
                .iter()
                .map(|(machine, _)| *machine)
                .collect::<Vec<_>>(),
            vec![Some(m6)]
        );
        send(&mut a, CliToServer::ProjectFlagsChanged {
            session_id: session,
            auto_approve_todos: true,
            auto_merge_prs: false,
            team_enabled: false,
            disallowed_tab_types: Vec::new(),
        })
        .await;
        let (flags, before) = receive_web(&mut web, |message| {
            matches!(message, ServerToWeb::ProjectFlagsChanged { .. })
        })
        .await;
        assert!(matches!(
            flags,
            ServerToWeb::ProjectFlagsChanged {
                auto_approve_todos: true,
                ..
            }
        ));
        // A viewer with one inventory per session would let zoo-006's replace
        // the primary's, though its reboot request reaches the primary.
        assert!(
            !before
                .iter()
                .any(|message| matches!(message, ServerToWeb::CliLifecycleInventory { .. })),
            "{before:?}"
        );

        // The primary's own inventory reaches it.
        let primary_inventory = CliLifecycleInventory {
            persistent_terminal_hosting: false,
            panes: Vec::new(),
        };
        send(&mut a, CliToServer::CliLifecycleInventory {
            session_id: session,
            inventory: primary_inventory.clone(),
        })
        .await;
        let (inventory, _) = receive_web(&mut web, |message| {
            matches!(message, ServerToWeb::CliLifecycleInventory { .. })
        })
        .await;
        assert!(matches!(
            inventory,
            ServerToWeb::CliLifecycleInventory { machine_id: Some(machine), .. } if machine == m5
        ));

        // zoo-005 leaves and zoo-006 becomes the primary: that viewer is sent
        // zoo-006's inventory now, which its next reboot request concerns.
        a.close(None).await.unwrap();
        let (inventory, _) = receive_web(&mut web, |message| {
            matches!(message, ServerToWeb::CliLifecycleInventory { .. })
        })
        .await;
        assert!(matches!(
            inventory,
            ServerToWeb::CliLifecycleInventory {
                machine_id: Some(machine),
                inventory: CliLifecycleInventory { persistent_terminal_hosting: true, .. },
                ..
            } if machine == m6
        ));
    }

    /// `SessionEnd` from one machine's instance detaches only it; the
    /// session ends with its last instance.
    #[tokio::test]
    async fn ending_one_instance_leaves_the_session_running_elsewhere() {
        let harness = harness().await;
        let session = Uuid::new_v4();
        let (mut a, _) = connect_cli(&harness, true).await;
        let (mut b, b_id) = connect_cli(&harness, true).await;
        start(&mut a, session_start(session, Uuid::new_v4(), HOST_A, Some(Vec::new()))).await;
        start(&mut b, session_start(session, Uuid::new_v4(), HOST_B, Some(Vec::new()))).await;

        send(&mut a, CliToServer::SessionEnd {
            session_id: session,
            reason: "stopped on zoo-005".into(),
        })
        .await;
        let row = eventually_row(&harness, session, "the row moves to zoo-006", |row| {
            row.cli_client_id.as_deref() == Some(b_id.to_string().as_str())
        })
        .await;
        assert_eq!(row.status, "active", "the project still runs on zoo-006");

        send(&mut b, CliToServer::SessionEnd {
            session_id: session,
            reason: "stopped on zoo-006".into(),
        })
        .await;
        eventually_row(&harness, session, "the session ends", |row| row.status == "ended").await;
    }

    fn tagged(pane_id: u32, machine_id: Uuid, host: &str) -> PaneConfig {
        let mut pane = pane(pane_id);
        pane.machine_id = Some(machine_id);
        pane.hostname = Some(host.to_string());
        pane
    }

    fn tags(panes: &[PaneConfig]) -> Vec<(u32, Option<Uuid>)> {
        panes
            .iter()
            .map(|pane| (pane.pane_id, pane.machine_id))
            .collect()
    }

    fn panes_file(harness: &Harness, session_id: Uuid) -> std::path::PathBuf {
        harness
            .dir
            .join("sessions")
            .join(session_id.to_string())
            .join("panes.json")
    }

    /// After a restart the saved roster holds a stopped machine's tab. While
    /// `panes.json` cannot be read, an instance's report is shown but never
    /// saved over the file, which would erase that tab for good. Once the
    /// file reads again the tab is back beside the report, every instance
    /// is asked to report again, and the whole roster is saved.
    #[tokio::test]
    async fn a_roster_that_cannot_be_read_is_never_saved_over() {
        let harness = harness().await;
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        harness
            .state
            .storage
            .save_pane_list(&session, &[tagged(3, m5, HOST_A), tagged(4, m6, HOST_B)])
            .await
            .unwrap();
        // A directory in its place makes every read of the file fail.
        let file = panes_file(&harness, session);
        let saved = file.with_file_name("panes.json.saved");
        std::fs::rename(&file, &saved).unwrap();
        std::fs::create_dir(&file).unwrap();

        let (mut a, _) = connect_cli(&harness, true).await;
        start(&mut a, session_start(session, m5, HOST_A, Some(vec![pane(3)]))).await;
        let sessions = &harness.state.sessions;
        assert_eq!(tags(&sessions.get_session_panes(&session)), vec![(3, Some(m5))]);
        assert!(sessions.pane_roster_unreadable(&session));
        assert!(!sessions.pane_roster_seeded(&session));
        assert!(sessions.pane_roster_snapshot(&session).is_none());
        assert!(file.is_dir(), "nothing was saved over the unreadable roster");

        // The file reads again; the instance's next report loads it first.
        std::fs::remove_dir(&file).unwrap();
        std::fs::rename(&saved, &file).unwrap();
        send(&mut a, CliToServer::PaneList {
            session_id: session,
            panes: vec![pane(3), pane(5)],
        })
        .await;
        receive(&mut a, |message| matches!(message, ServerToCli::RequestPaneList { .. })).await;
        let expected = vec![(3, Some(m5)), (4, Some(m6)), (5, Some(m5))];
        eventually("the report merges into the saved roster", || {
            tags(&sessions.get_session_panes(&session)) == expected
        })
        .await;
        assert!(sessions.pane_roster_seeded(&session));
        assert!(!sessions.pane_roster_unreadable(&session));
        let persisted = harness
            .state
            .storage
            .load_pane_roster(&session)
            .await
            .unwrap()
            .expect("the roster is saved again");
        assert_eq!(tags(&persisted), expected);
    }

    /// A `panes.json` that does not parse never will. It is moved aside with
    /// its bytes for an operator, and the project goes on as if no roster
    /// had been saved, rather than never saving one again.
    #[tokio::test]
    async fn a_roster_that_does_not_parse_is_moved_aside() {
        let harness = harness().await;
        let session = Uuid::new_v4();
        let m5 = Uuid::new_v4();
        let file = panes_file(&harness, session);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"[{\"pane_id\": 3,").unwrap();

        let (mut a, _) = connect_cli(&harness, true).await;
        start(&mut a, session_start(session, m5, HOST_A, Some(vec![pane(3)]))).await;
        let sessions = &harness.state.sessions;
        assert!(sessions.pane_roster_seeded(&session));
        assert!(!sessions.pane_roster_unreadable(&session));
        let persisted = harness
            .state
            .storage
            .load_pane_roster(&session)
            .await
            .unwrap()
            .expect("the reported roster is saved");
        assert_eq!(tags(&persisted), vec![(3, Some(m5))]);
        let aside: Vec<std::path::PathBuf> = std::fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("panes.json.unreadable-"))
            })
            .collect();
        assert_eq!(aside.len(), 1, "{aside:?}");
        assert_eq!(std::fs::read(&aside[0]).unwrap(), b"[{\"pane_id\": 3,");
    }

    /// What the CLI of a machine that has autopilot advertises.
    const AUTOPILOT_CLI: &[&str] = &[
        shared::PROJECT_POLICY_CAPABILITY,
        PROJECT_INSTANCES_CAPABILITY,
        shared::OMP_TAB_AUTOPILOT_CAPABILITY,
    ];

    fn omp_tab(pane_id: u32) -> PaneConfig {
        let mut tab = pane(pane_id);
        tab.provider = Provider::Omp;
        tab
    }

    fn autopilot_state(
        session_id: Uuid,
        pane_id: u32,
        support: shared::AutopilotSupport,
        reported_at: &str,
    ) -> CliToServer {
        CliToServer::PaneAutopilotState {
            session_id,
            pane_id,
            support,
            state: Some(Box::new(shared::PaneAutopilotState::default())),
            reported_at: Some(reported_at.to_string()),
        }
    }

    /// The autopilot states among `frames`: pane, availability and the time
    /// the state was last reported.
    fn autopilot_states(
        frames: &[ServerToWeb],
    ) -> Vec<(u32, shared::AutopilotSupport, Option<String>)> {
        frames
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

    /// A CLI that advertised autopilot is told that this server controls it,
    /// which is what lets its tabs go on submitting iterations. A CLI that
    /// did not advertise it is echoed nothing, as from an older server.
    #[tokio::test]
    async fn registration_echoes_autopilot_to_a_cli_that_advertised_it() {
        let harness = harness().await;
        let (_with, _, echoed) = register_cli(&harness, AUTOPILOT_CLI).await;
        assert_eq!(echoed, vec![shared::OMP_TAB_AUTOPILOT_CAPABILITY.to_string()]);
        let (_without, _, echoed) = register_cli(
            &harness,
            &[shared::PROJECT_POLICY_CAPABILITY, PROJECT_INSTANCES_CAPABILITY],
        )
        .await;
        assert!(echoed.is_empty(), "{echoed:?}");
        assert!(super::negotiated_cli_capabilities(&[]).is_empty());
    }

    /// A tab's autopilot state and the receipt for a request come only from
    /// the instance that runs the tab. The state is relayed to viewers that
    /// negotiated autopilot and to no other, a tab a roster report adds is
    /// told after the roster that lists it, and a tab whose instance leaves
    /// is not reported.
    #[tokio::test]
    async fn a_tabs_autopilot_is_reported_by_its_own_instance_to_capable_viewers() {
        use shared::AutopilotSupport::{NotReported, Supported};
        let harness = harness().await;
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let (mut a, a_id, _) = register_cli(&harness, AUTOPILOT_CLI).await;
        let (mut b, _, _) = register_cli(&harness, AUTOPILOT_CLI).await;
        start(&mut a, session_start(session, m5, HOST_A, Some(vec![omp_tab(3)]))).await;
        start(&mut b, session_start(session, m6, HOST_B, Some(vec![omp_tab(4)]))).await;
        let mut web = viewer_with(&harness, session, &[shared::TAB_AUTOPILOT_CAPABILITY]);
        let mut older_web = viewer_with(&harness, session, &[PROJECT_INSTANCES_CAPABILITY]);
        let sessions = &harness.state.sessions;

        // zoo-006 cannot speak for zoo-005's tab: its report for that tab is
        // dropped, and the one for its own tab, sent after it, is relayed.
        send(&mut b, autopilot_state(session, 3, Supported, "2026-11-13T13:59:00Z")).await;
        send(&mut b, autopilot_state(session, 4, Supported, "2026-11-13T14:00:00Z")).await;
        let (own, before) = receive_web(&mut web, |message| {
            matches!(message, ServerToWeb::PaneAutopilotState { pane_id: 4, .. })
        })
        .await;
        let on_zoo_006 = (4, Supported, Some("2026-11-13T14:00:00Z".to_string()));
        assert_eq!(autopilot_states(&[own]), vec![on_zoo_006.clone()]);
        assert!(autopilot_states(&before).is_empty(), "{before:?}");

        // zoo-005's own report is relayed. A CLI cannot say that an instance
        // needs an update: only the server does.
        send(&mut a, autopilot_state(session, 3, Supported, "2026-11-13T14:01:00Z")).await;
        send(
            &mut a,
            autopilot_state(
                session,
                3,
                shared::AutopilotSupport::InstanceUpdateRequired,
                "2026-11-13T14:02:00Z",
            ),
        )
        .await;
        let (last, before) = receive_web(&mut web, |message| {
            matches!(message, ServerToWeb::PaneAutopilotState { reported_at: Some(at), .. } if at == "2026-11-13T14:02:00Z")
        })
        .await;
        assert_eq!(
            autopilot_states(&[before, vec![last]].concat()),
            vec![
                (3, Supported, Some("2026-11-13T14:01:00Z".to_string())),
                (3, NotReported, Some("2026-11-13T14:02:00Z".to_string())),
            ]
        );

        // The same for a receipt: only the tab's instance answers a request.
        let receipt = sessions
            .begin_autopilot_request(session, 3, a_id, "stop-1")
            .unwrap();
        let answered = |reason: &str| CliToServer::PaneAutopilotReceipt {
            session_id: session,
            pane_id: 3,
            request_id: "stop-1".to_string(),
            outcome: shared::AutopilotReceiptOutcome::Accepted,
            reason: Some(reason.to_string()),
        };
        send(&mut b, answered("from another machine")).await;
        send(&mut a, answered("from the tab's machine")).await;
        let receipt = tokio::time::timeout(Duration::from_secs(5), receipt)
            .await
            .expect("the receipt arrives")
            .unwrap();
        assert_eq!(receipt.reason.as_deref(), Some("from the tab's machine"));

        // A new OMP tab is told after the roster that lists it, and a tab of
        // another provider is not mentioned.
        send(&mut a, CliToServer::PaneList {
            session_id: session,
            panes: vec![omp_tab(3), omp_tab(8), pane(9)],
        })
        .await;
        let (added, before) = receive_web(&mut web, |message| {
            matches!(message, ServerToWeb::PaneAutopilotState { .. })
        })
        .await;
        assert_eq!(autopilot_states(&[added]), vec![(8, NotReported, None)]);
        assert!(
            before
                .iter()
                .any(|message| matches!(message, ServerToWeb::PaneList { .. })),
            "{before:?}"
        );

        // zoo-005 drops off: its tabs are not reported, zoo-006's is as it was.
        a.close(None).await.unwrap();
        let (last, before) = receive_web(&mut web, |message| {
            matches!(message, ServerToWeb::PaneAutopilotState { pane_id: 8, .. })
        })
        .await;
        assert_eq!(
            autopilot_states(&[before, vec![last]].concat()),
            vec![(3, NotReported, None), on_zoo_006, (8, NotReported, None)]
        );
        assert!(sessions.cached_pane_autopilot(&session, 3).is_none());

        // The viewer that did not negotiate autopilot saw none of it. Its own
        // tab's output, sent last, is the first thing it is asked to match.
        send(&mut b, terminal_output(session, 4)).await;
        let (_, before) = receive_web(&mut older_web, |message| {
            matches!(message, ServerToWeb::TerminalOutput { pane_id: 4, .. })
        })
        .await;
        assert!(
            !before.iter().any(|message| matches!(
                message,
                ServerToWeb::PaneAutopilotState { .. } | ServerToWeb::PaneAutopilotReceipt { .. }
            )),
            "{before:?}"
        );
    }

    /// An iteration is marked in the conversation by an assistant tool-use
    /// block with a reserved name, sent as not completing the turn. The
    /// server stores and relays it like any tool-use block and acts on
    /// nothing in it: the tab's status stays what it was, Working or idle,
    /// and no question is registered.
    #[tokio::test]
    async fn an_iteration_marker_leaves_the_tabs_status_unchanged() {
        let harness = harness().await;
        let session = Uuid::new_v4();
        let (mut a, _, _) = register_cli(&harness, AUTOPILOT_CLI).await;
        start(
            &mut a,
            session_start(session, Uuid::new_v4(), HOST_A, Some(vec![omp_tab(3), omp_tab(4)])),
        )
        .await;
        let mut web = viewer(&harness, session);
        let sessions = &harness.state.sessions;
        let marker = |pane_id: u32| CliToServer::StreamMessage {
            session_id: session,
            message: shared::ClaudeStreamMessage::Assistant {
                message: shared::ClaudeAssistantMessage {
                    content: vec![shared::ClaudeContentBlock::ToolUse {
                        id: format!("autopilot-iteration-{pane_id}"),
                        name: shared::AUTOPILOT_ITERATION_TOOL_NAME.to_string(),
                        input: serde_json::to_value(shared::AutopilotIterationMarker {
                            autopilot_id: Uuid::nil(),
                            iteration: 3,
                            limit: Some(20),
                            prompt: None,
                        })
                        .unwrap(),
                    }],
                    model: String::new(),
                    extra: serde_json::Value::Null,
                },
                session_id: "omp-session".to_string(),
                extra: json!({ "terminal_turn_complete": false }),
            },
            pane_type: None,
            pane_id: Some(pane_id),
        };

        // Tab 3 is Working, tab 4 is idle.
        send(&mut a, CliToServer::PaneStatus {
            session_id: session,
            pane_type: PaneType::Interactive,
            pane_id: Some(3),
            status: Some("Working...".to_string()),
        })
        .await;
        send(&mut a, marker(3)).await;
        send(&mut a, marker(4)).await;
        send(&mut a, terminal_output(session, 3)).await;
        let (_, before) = receive_web(&mut web, |message| {
            matches!(message, ServerToWeb::TerminalOutput { pane_id: 3, .. })
        })
        .await;

        let relayed: Vec<u32> = before
            .iter()
            .filter_map(|message| match message {
                ServerToWeb::StreamMessage { pane_id, .. } => *pane_id,
                _ => None,
            })
            .collect();
        assert_eq!(relayed, vec![3, 4], "the markers reach the conversation");
        let statuses: Vec<(Option<u32>, Option<String>)> = before
            .iter()
            .filter_map(|message| match message {
                ServerToWeb::PaneStatus {
                    pane_id, status, ..
                } => Some((*pane_id, status.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            statuses,
            vec![(Some(3), Some("Working...".to_string()))],
            "only the status the CLI sent"
        );
        assert_eq!(
            sessions.get_pane_statuses(&session),
            vec![(PaneType::Interactive, 3, "Working...".to_string())]
        );
        for pane_id in [3, 4] {
            assert!(
                sessions
                    .claim_pending_decision(
                        session,
                        &format!("autopilot-iteration-{pane_id}"),
                        Some(pane_id),
                        shared::MutationKind::Question,
                    )
                    .is_none(),
                "a marker is not a question"
            );
        }
    }
}
