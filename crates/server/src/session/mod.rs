use dashmap::DashMap;
use shared::{
    CliClientInfo, CliClientStatus, DeepseekBackendInfo, GlmBackendInfo, MachineInfo,
    MachineProjectInfo, MachineWithProjects, MiniMaxBackendInfo, PaneConfig, PaneType, Provider,
    ServerToCli, ServerToDaemon, ServerToWeb, TerminalLifecycle, UsageLimits,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

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

fn normalize_optional_string(raw: Option<String>) -> Option<String> {
    raw.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn merge_minimax_backend(
    existing: Option<MiniMaxBackendInfo>,
    incoming: Option<MiniMaxBackendInfo>,
) -> Option<MiniMaxBackendInfo> {
    match (existing, incoming) {
        (None, None) => None,
        (Some(existing), None) => Some(existing),
        (None, Some(mut incoming)) => {
            incoming.api_base_url = normalize_optional_string(incoming.api_base_url);
            incoming.api_key = normalize_optional_string(incoming.api_key);
            incoming.api_key_configured = incoming.api_key.is_some() || incoming.api_key_configured;
            Some(incoming)
        }
        (Some(existing), Some(mut incoming)) => {
            incoming.api_base_url = normalize_optional_string(incoming.api_base_url)
                .or_else(|| normalize_optional_string(existing.api_base_url));

            let incoming_key = normalize_optional_string(incoming.api_key);
            incoming.api_key = incoming_key.or_else(|| {
                if incoming.api_key_configured {
                    normalize_optional_string(existing.api_key)
                } else {
                    None
                }
            });
            incoming.api_key_configured = incoming.api_key.is_some() || incoming.api_key_configured;
            Some(incoming)
        }
    }
}

fn merge_glm_backend(
    existing: Option<GlmBackendInfo>,
    incoming: Option<GlmBackendInfo>,
) -> Option<GlmBackendInfo> {
    match (existing, incoming) {
        (None, None) => None,
        (Some(existing), None) => Some(existing),
        (None, Some(mut incoming)) => {
            incoming.api_base_url = normalize_optional_string(incoming.api_base_url);
            incoming.api_key = normalize_optional_string(incoming.api_key);
            incoming.api_key_configured = incoming.api_key.is_some() || incoming.api_key_configured;
            Some(incoming)
        }
        (Some(existing), Some(mut incoming)) => {
            incoming.api_base_url = normalize_optional_string(incoming.api_base_url)
                .or_else(|| normalize_optional_string(existing.api_base_url));

            let incoming_key = normalize_optional_string(incoming.api_key);
            incoming.api_key = incoming_key.or_else(|| {
                if incoming.api_key_configured {
                    normalize_optional_string(existing.api_key)
                } else {
                    None
                }
            });
            incoming.api_key_configured = incoming.api_key.is_some() || incoming.api_key_configured;
            Some(incoming)
        }
    }
}

fn merge_deepseek_backend(
    existing: Option<DeepseekBackendInfo>,
    incoming: Option<DeepseekBackendInfo>,
) -> Option<DeepseekBackendInfo> {
    match (existing, incoming) {
        (None, None) => None,
        (Some(existing), None) => Some(existing),
        (None, Some(mut incoming)) => {
            incoming.api_base_url = normalize_optional_string(incoming.api_base_url);
            incoming.api_key = normalize_optional_string(incoming.api_key);
            incoming.api_key_configured = incoming.api_key.is_some() || incoming.api_key_configured;
            Some(incoming)
        }
        (Some(existing), Some(mut incoming)) => {
            incoming.api_base_url = normalize_optional_string(incoming.api_base_url)
                .or_else(|| normalize_optional_string(existing.api_base_url));

            let incoming_key = normalize_optional_string(incoming.api_key);
            incoming.api_key = incoming_key.or_else(|| {
                if incoming.api_key_configured {
                    normalize_optional_string(existing.api_key)
                } else {
                    None
                }
            });
            incoming.api_key_configured = incoming.api_key.is_some() || incoming.api_key_configured;
            Some(incoming)
        }
    }
}

/// Manages active sessions and routes messages between web and CLI clients
pub struct SessionManager {
    /// Map of session ID -> session state
    sessions: DashMap<Uuid, SessionState>,
    /// Map of CLI client ID -> sender to CLI
    cli_senders: DashMap<Uuid, mpsc::Sender<ServerToCli>>,
    /// Map of web connection ID -> sender to web
    web_senders: DashMap<Uuid, mpsc::Sender<ServerToWeb>>,
    /// Map of web connection ID -> authenticated user ID
    web_users: DashMap<Uuid, Uuid>,
    /// Map of CLI client ID -> list of session IDs
    cli_sessions: DashMap<Uuid, Vec<Uuid>>,
    /// Map of CLI client ID -> user ID (owner)
    cli_users: DashMap<Uuid, Uuid>,
    /// Map of CLI client ID -> reported CLI version
    cli_versions: DashMap<Uuid, String>,
    /// Map of (CLI client ID, provider) -> latest usage limits
    cli_usage_limits: DashMap<(Uuid, Provider), UsageLimits>,
    /// Map of machine ID -> sender to daemon
    daemon_senders: DashMap<Uuid, mpsc::Sender<ServerToDaemon>>,
    /// Map of machine ID -> user ID (owner)
    daemon_users: DashMap<Uuid, Uuid>,
    /// Map of machine ID -> machine metadata
    machine_infos: DashMap<Uuid, MachineInfo>,
    /// Map of machine ID -> project list
    machine_projects: DashMap<Uuid, Vec<MachineProjectInfo>>,
    /// Cached shared-project access refs per user. Populated when the web
    /// layer computes accessible machines (e.g., on `ListMachines`). Used by
    /// the heartbeat-driven `broadcast_machines_update_for_user` so pushed
    /// updates include shared machines too — without it, the broadcast would
    /// only return owner machines and shared entries (like a teammate's
    /// daemon) would visibly disappear between user-initiated refreshes.
    shared_project_refs: DashMap<Uuid, (HashSet<(String, String)>, HashSet<String>)>,
    /// Recently stored web-input ids per session: (client_msg_id, created_at).
    /// Idempotency guard — the web client retransmits unacked inputs (3s
    /// retry + reconnect replay), and without this each retransmit was
    /// stored and displayed as a fresh message. Bounded ring per session.
    recent_input_ids: DashMap<Uuid, VecDeque<(String, String)>>,
    /// Raw pty presentation and lifecycle per (session, pane) for
    /// `PaneKind::Terminal` panes. In memory only and deliberately never
    /// persisted: these are ANSI byte streams, not chat records, and writing
    /// them to `messages.jsonl` would corrupt the message store.
    terminal_states: DashMap<(Uuid, u32), TerminalStateEntry>,
}

/// Bounded rolling window and last authoritative lifecycle for a terminal.
///
/// Replayed verbatim when a web client attaches, which is why it stores
/// raw bytes rather than decoded text — re-encoding would have to
/// interpret escape sequences, and interpreting them correctly is the
/// emulator's job, not the broker's.
#[derive(Debug, Default)]
pub struct TerminalStateEntry {
    buf: VecDeque<u8>,
    /// Sequence of the newest chunk in `buf`.
    seq: u64,
    has_output: bool,
    /// True once the cap has forced us to drop bytes off the front, which
    /// means a replay can start mid-escape-sequence.
    truncated: bool,
    instance_id: Option<Uuid>,
    lifecycle: TerminalLifecycle,
    status: Option<String>,
}

/// Immutable terminal state returned to attach handlers and lifecycle fans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSnapshotState {
    pub bytes: Vec<u8>,
    pub seq: u64,
    pub truncated: bool,
    pub instance_id: Option<Uuid>,
    pub lifecycle: TerminalLifecycle,
    pub status: Option<String>,
}

impl TerminalStateEntry {
    fn snapshot(&self) -> TerminalSnapshotState {
        TerminalSnapshotState {
            bytes: self.buf.iter().copied().collect(),
            seq: self.seq,
            truncated: self.truncated,
            instance_id: self.instance_id,
            lifecycle: self.lifecycle,
            status: self.status.clone(),
        }
    }

    fn replace_instance(&mut self, instance_id: Uuid) {
        self.buf.clear();
        self.seq = 0;
        self.has_output = false;
        self.truncated = false;
        self.instance_id = Some(instance_id);
        self.lifecycle = TerminalLifecycle::Unknown;
        self.status = None;
    }
}

/// Scrollback retained per terminal pane. A full-screen TUI repaint is a
/// few KB, so this holds a healthy number of frames while capping the
/// worst case at a few MB across a large fleet of panes.
const TERMINAL_SCROLLBACK_MAX_BYTES: usize = 256 * 1024;

#[derive(Debug)]
pub struct SessionState {
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub cli_client_id: Option<Uuid>,
    /// All web clients currently viewing this session
    pub web_connection_ids: Vec<Uuid>,
    pub is_paused: bool,
    /// Cached pane configurations (last PaneList from CLI)
    pub panes: Vec<PaneConfig>,
    /// Latest pane status per pane_id (e.g., "thinking") so we can replay to
    /// re-attaching web clients — otherwise the indicator vanishes on tab
    /// switch until the CLI next reports status.
    pub pane_statuses: HashMap<u32, (PaneType, String)>,
    /// Cached project_goal.md content (last `ProjectGoalChanged` from CLI).
    /// Replayed to newly-attaching web clients so a hard-refresh doesn't
    /// leave the Project goal textbox empty until the next file change.
    pub project_goal: Option<String>,
    /// Working directory of the CLI session
    pub working_dir: Option<String>,
    /// Hostname of the CLI session
    pub hostname: Option<String>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            cli_senders: DashMap::new(),
            web_senders: DashMap::new(),
            web_users: DashMap::new(),
            cli_sessions: DashMap::new(),
            cli_users: DashMap::new(),
            cli_versions: DashMap::new(),
            cli_usage_limits: DashMap::new(),
            daemon_senders: DashMap::new(),
            daemon_users: DashMap::new(),
            machine_infos: DashMap::new(),
            machine_projects: DashMap::new(),
            shared_project_refs: DashMap::new(),
            recent_input_ids: DashMap::new(),
            terminal_states: DashMap::new(),
        }
    }

    /// Append a terminal chunk to the pane's scrollback ring, evicting
    /// from the front once the cap is reached. Returns false for an event
    /// belonging to a replaced terminal generation.
    pub fn append_terminal_output(
        &self,
        session_id: &Uuid,
        pane_id: u32,
        instance_id: Option<Uuid>,
        bytes: &[u8],
        seq: u64,
    ) -> bool {
        let mut entry = self
            .terminal_states
            .entry((*session_id, pane_id))
            .or_default();

        match (entry.instance_id, instance_id) {
            (Some(current), Some(incoming)) if current != incoming => return false,
            (None, Some(incoming)) => entry.instance_id = Some(incoming),
            _ => {}
        }

        // Identified streams have a per-instance monotonic counter. Rejecting
        // duplicates here prevents delayed frames from being appended twice
        // after a reconnect. Legacy streams are accepted even if their
        // counters restart because they cannot prove process identity.
        if instance_id.is_some() && entry.has_output && seq <= entry.seq {
            return false;
        }

        entry.buf.extend(bytes.iter().copied());
        entry.seq = seq;
        entry.has_output = true;
        if entry.buf.len() > TERMINAL_SCROLLBACK_MAX_BYTES {
            let overflow = entry.buf.len() - TERMINAL_SCROLLBACK_MAX_BYTES;
            entry.buf.drain(..overflow);
            entry.truncated = true;
        }
        true
    }

    /// Apply an authoritative CLI state report. A running report for a new
    /// instance replaces the old presentation. Non-running reports for a
    /// different instance are stale by definition and are ignored.
    pub fn reconcile_terminal_state(
        &self,
        session_id: &Uuid,
        pane_id: u32,
        instance_id: Option<Uuid>,
        lifecycle: TerminalLifecycle,
        status: Option<String>,
    ) -> Option<TerminalSnapshotState> {
        let mut entry = self
            .terminal_states
            .entry((*session_id, pane_id))
            .or_default();

        match (entry.instance_id, instance_id) {
            (Some(current), Some(incoming)) if current != incoming => {
                if lifecycle != TerminalLifecycle::Running {
                    return None;
                }
                entry.replace_instance(incoming);
            }
            (None, Some(incoming)) => entry.replace_instance(incoming),
            _ => {}
        }

        // Exited is terminal for one process instance. An idempotent exit is
        // accepted, but a delayed running/disconnected report cannot revive it.
        if entry.lifecycle == TerminalLifecycle::Exited
            && lifecycle != TerminalLifecycle::Exited
            && instance_id.is_some()
        {
            return None;
        }

        entry.lifecycle = lifecycle;
        entry.status = status;
        Some(entry.snapshot())
    }

    /// Record the legacy exit event as retained lifecycle state. Matching or
    /// metadata-less exits are accepted; exits from replaced instances are not.
    pub fn record_terminal_exit(
        &self,
        session_id: &Uuid,
        pane_id: u32,
        instance_id: Option<Uuid>,
        status: Option<String>,
    ) -> Option<TerminalSnapshotState> {
        let mut entry = self
            .terminal_states
            .entry((*session_id, pane_id))
            .or_default();
        match (entry.instance_id, instance_id) {
            (Some(current), Some(incoming)) if current != incoming => return None,
            (None, Some(incoming)) => entry.instance_id = Some(incoming),
            _ => {}
        }
        entry.lifecycle = TerminalLifecycle::Exited;
        entry.status = status;
        Some(entry.snapshot())
    }

    /// Current retained presentation and lifecycle. State-only entries are
    /// returned even when a process has not produced any output.
    pub fn terminal_snapshot(
        &self,
        session_id: &Uuid,
        pane_id: u32,
    ) -> Option<TerminalSnapshotState> {
        self.terminal_states
            .get(&(*session_id, pane_id))
            .map(|entry| entry.snapshot())
    }

    /// Drop a terminal pane's scrollback. Called when the pane is removed
    /// so a later pane reusing the id can't inherit stale frames.
    pub fn clear_terminal_scrollback(&self, session_id: &Uuid, pane_id: u32) {
        self.terminal_states.remove(&(*session_id, pane_id));
    }

    /// Mark currently-running terminals as transport-disconnected while
    /// retaining presentation. Unknown and exited entries are not rewritten.
    pub fn mark_session_terminals_disconnected(
        &self,
        session_id: &Uuid,
    ) -> Vec<(u32, TerminalSnapshotState)> {
        let mut changed = Vec::new();
        for mut item in self.terminal_states.iter_mut() {
            let ((sid, pane_id), entry) = item.pair_mut();
            if sid == session_id && entry.lifecycle == TerminalLifecycle::Running {
                entry.lifecycle = TerminalLifecycle::Disconnected;
                entry.status = None;
                changed.push((*pane_id, entry.snapshot()));
            }
        }
        changed
    }

    /// Returns the original `created_at` if this client_msg_id was already
    /// stored for the session (i.e., this is a retransmit), else None.
    pub fn seen_input_id(&self, session_id: &Uuid, client_msg_id: &str) -> Option<String> {
        self.recent_input_ids.get(session_id).and_then(|ids| {
            ids.iter()
                .find(|(id, _)| id == client_msg_id)
                .map(|(_, created_at)| created_at.clone())
        })
    }

    /// Record a stored web input's client_msg_id for retransmit dedup.
    pub fn record_input_id(&self, session_id: Uuid, client_msg_id: String, created_at: String) {
        const MAX_TRACKED: usize = 64;
        let mut ids = self.recent_input_ids.entry(session_id).or_default();
        ids.push_back((client_msg_id, created_at));
        while ids.len() > MAX_TRACKED {
            ids.pop_front();
        }
    }

    pub fn set_shared_project_refs_for_user(
        &self,
        user_id: Uuid,
        host_path_refs: HashSet<(String, String)>,
        wildcard_paths: HashSet<String>,
    ) {
        if host_path_refs.is_empty() && wildcard_paths.is_empty() {
            self.shared_project_refs.remove(&user_id);
        } else {
            self.shared_project_refs
                .insert(user_id, (host_path_refs, wildcard_paths));
        }
    }

    #[cfg(test)]
    pub fn cached_shared_project_refs_for_user(
        &self,
        user_id: &Uuid,
    ) -> Option<(HashSet<(String, String)>, HashSet<String>)> {
        self.shared_project_refs
            .get(user_id)
            .map(|entry| entry.value().clone())
    }

    // CLI client management
    pub fn register_cli(
        &self,
        cli_id: Uuid,
        user_id: Uuid,
        sender: mpsc::Sender<ServerToCli>,
        version: Option<String>,
    ) {
        self.cli_senders.insert(cli_id, sender);
        self.cli_sessions.insert(cli_id, Vec::new());
        self.cli_users.insert(cli_id, user_id);
        if let Some(version) = version.filter(|v| !v.trim().is_empty()) {
            self.cli_versions.insert(cli_id, version);
        } else {
            self.cli_versions.remove(&cli_id);
        }
        tracing::info!("CLI client registered: {} (user: {})", cli_id, user_id);
        // Broadcast updated client list to all web clients
        self.broadcast_cli_clients_update();
    }

    pub fn unregister_cli(&self, cli_id: &Uuid) {
        self.cli_senders.remove(cli_id);
        let owner = self.cli_users.remove(cli_id).map(|(_, uid)| uid);
        self.cli_versions.remove(cli_id);
        let keys_to_remove: Vec<(Uuid, Provider)> = self
            .cli_usage_limits
            .iter()
            .filter(|entry| entry.key().0 == *cli_id)
            .map(|entry| *entry.key())
            .collect();
        for key in keys_to_remove {
            self.cli_usage_limits.remove(&key);
        }
        if let Some((_, session_ids)) = self.cli_sessions.remove(cli_id) {
            for session_id in session_ids {
                if let Some(mut session) = self.sessions.get_mut(&session_id) {
                    // Only clear if this CLI is still the active one for this session.
                    // A new CLI may have already taken over (reconnect scenario).
                    if session.cli_client_id == Some(*cli_id) {
                        session.cli_client_id = None;
                        // Drop any cached "thinking"/status — the producer is gone,
                        // otherwise the next web attach would replay a stale indicator.
                        session.pane_statuses.clear();
                    }
                }
            }
        }
        tracing::info!("CLI client unregistered: {}", cli_id);
        // Broadcast updated client list to all web clients
        self.broadcast_cli_clients_update();
        // Broadcast updated machines (is_running depends on active CLI sessions)
        if let Some(user_id) = owner {
            self.broadcast_machines_update_for_user(&user_id);
        }
    }

    /// Check if a CLI client is currently connected (has an active sender)
    pub fn is_cli_connected(&self, cli_id: &Uuid) -> bool {
        self.cli_senders.contains_key(cli_id)
    }

    // Daemon management
    pub fn register_daemon(
        &self,
        machine_id: Uuid,
        user_id: Uuid,
        sender: mpsc::Sender<ServerToDaemon>,
        mut machine: MachineInfo,
        projects: Vec<MachineProjectInfo>,
    ) {
        let existing_minimax = self
            .machine_infos
            .get(&machine_id)
            .and_then(|m| m.minimax_backend.clone());
        let existing_glm = self
            .machine_infos
            .get(&machine_id)
            .and_then(|m| m.glm_backend.clone());
        let existing_deepseek = self
            .machine_infos
            .get(&machine_id)
            .and_then(|m| m.deepseek_backend.clone());
        machine.machine_id = machine_id;
        machine.last_seen = Some(chrono::Utc::now().to_rfc3339());
        machine.minimax_backend = merge_minimax_backend(existing_minimax, machine.minimax_backend);
        machine.glm_backend = merge_glm_backend(existing_glm, machine.glm_backend);
        machine.deepseek_backend =
            merge_deepseek_backend(existing_deepseek, machine.deepseek_backend);
        self.daemon_senders.insert(machine_id, sender);
        self.daemon_users.insert(machine_id, user_id);
        self.machine_infos.insert(machine_id, machine);
        let running_count = projects.iter().filter(|p| p.is_running).count();
        tracing::info!(
            "Daemon {} register: {} projects ({} running)",
            machine_id,
            projects.len(),
            running_count,
        );
        self.machine_projects.insert(machine_id, projects);
        self.broadcast_machines_update_for_user(&user_id);
        tracing::info!("Daemon registered: {} (user: {})", machine_id, user_id);
    }

    pub fn unregister_daemon(&self, machine_id: &Uuid) {
        let owner = self.daemon_users.get(machine_id).map(|entry| *entry);
        self.daemon_senders.remove(machine_id);
        // Keep machine metadata/project snapshot to avoid UI flicker during transient daemon reconnects.
        if let Some(mut machine) = self.machine_infos.get_mut(machine_id) {
            machine.last_seen = Some(chrono::Utc::now().to_rfc3339());
        }
        // Without the daemon we can't trust the `is_running` flags — the
        // processes may still be running, but we have no way to interact with
        // them. Mark all projects as not running so the UI doesn't advertise
        // them as bootable/attachable via this machine.
        if let Some(mut projects) = self.machine_projects.get_mut(machine_id) {
            for p in projects.iter_mut() {
                p.is_running = false;
                p.pid = None;
            }
        }

        if let Some(user_id) = owner {
            self.broadcast_machines_update_for_user(&user_id);
        }

        tracing::info!("Daemon unregistered: {}", machine_id);
    }

    pub fn update_daemon_projects(&self, machine_id: &Uuid, projects: Vec<MachineProjectInfo>) {
        let running_count = projects.iter().filter(|p| p.is_running).count();
        // Log when project count or running set changes (not every heartbeat)
        let prev = self.machine_projects.get(machine_id).map(|p| {
            (
                p.len(),
                p.iter().filter(|pp| pp.is_running).count(),
            )
        });
        let current = (projects.len(), running_count);
        if prev != Some(current) {
            tracing::info!(
                "Daemon {} project state: {} projects ({} running)",
                machine_id,
                projects.len(),
                running_count,
            );
        }
        self.machine_projects.insert(*machine_id, projects);
        if let Some(mut machine) = self.machine_infos.get_mut(machine_id) {
            machine.last_seen = Some(chrono::Utc::now().to_rfc3339());
        }
        if let Some(owner) = self.daemon_users.get(machine_id).map(|entry| *entry) {
            self.broadcast_machines_update_for_user(&owner);
        }
    }

    pub fn update_daemon_machine_info(&self, machine_id: &Uuid, mut machine: MachineInfo) {
        let existing_minimax = self
            .machine_infos
            .get(machine_id)
            .and_then(|m| m.minimax_backend.clone());
        let existing_glm = self
            .machine_infos
            .get(machine_id)
            .and_then(|m| m.glm_backend.clone());
        let existing_deepseek = self
            .machine_infos
            .get(machine_id)
            .and_then(|m| m.deepseek_backend.clone());
        machine.machine_id = *machine_id;
        machine.last_seen = Some(chrono::Utc::now().to_rfc3339());
        machine.minimax_backend = merge_minimax_backend(existing_minimax, machine.minimax_backend);
        machine.glm_backend = merge_glm_backend(existing_glm, machine.glm_backend);
        machine.deepseek_backend =
            merge_deepseek_backend(existing_deepseek, machine.deepseek_backend);
        self.machine_infos.insert(*machine_id, machine);
        if let Some(owner) = self.daemon_users.get(machine_id).map(|entry| *entry) {
            self.broadcast_machines_update_for_user(&owner);
        }
    }

    pub fn apply_web_minimax_config(
        &self,
        machine_id: &Uuid,
        api_base_url: Option<String>,
        api_key: Option<String>,
        clear_api_key: bool,
    ) {
        let owner = self.daemon_users.get(machine_id).map(|entry| *entry);
        if let Some(mut machine) = self.machine_infos.get_mut(machine_id) {
            let mut backend = machine
                .minimax_backend
                .clone()
                .unwrap_or(MiniMaxBackendInfo {
                    api_base_url: None,
                    api_key: None,
                    api_key_configured: false,
                });

            if let Some(url) = normalize_optional_string(api_base_url) {
                backend.api_base_url = Some(url);
            }

            if clear_api_key {
                backend.api_key = None;
                backend.api_key_configured = false;
            } else if let Some(key) = normalize_optional_string(api_key) {
                backend.api_key = Some(key);
                backend.api_key_configured = true;
            } else {
                backend.api_key_configured = backend.api_key.is_some();
            }

            machine.minimax_backend = Some(backend);
            machine.last_seen = Some(chrono::Utc::now().to_rfc3339());
        }

        if let Some(user_id) = owner {
            self.broadcast_machines_update_for_user(&user_id);
        }
    }

    pub fn apply_web_glm_config(
        &self,
        machine_id: &Uuid,
        api_base_url: Option<String>,
        api_key: Option<String>,
        clear_api_key: bool,
    ) {
        let owner = self.daemon_users.get(machine_id).map(|entry| *entry);
        if let Some(mut machine) = self.machine_infos.get_mut(machine_id) {
            let mut backend = machine.glm_backend.clone().unwrap_or(GlmBackendInfo {
                api_base_url: None,
                api_key: None,
                api_key_configured: false,
            });

            if let Some(url) = normalize_optional_string(api_base_url) {
                backend.api_base_url = Some(url);
            }

            if clear_api_key {
                backend.api_key = None;
                backend.api_key_configured = false;
            } else if let Some(key) = normalize_optional_string(api_key) {
                backend.api_key = Some(key);
                backend.api_key_configured = true;
            } else {
                backend.api_key_configured = backend.api_key.is_some();
            }

            machine.glm_backend = Some(backend);
            machine.last_seen = Some(chrono::Utc::now().to_rfc3339());
        }

        if let Some(user_id) = owner {
            self.broadcast_machines_update_for_user(&user_id);
        }
    }

    pub fn apply_web_deepseek_config(
        &self,
        machine_id: &Uuid,
        api_base_url: Option<String>,
        api_key: Option<String>,
        clear_api_key: bool,
    ) {
        let owner = self.daemon_users.get(machine_id).map(|entry| *entry);
        if let Some(mut machine) = self.machine_infos.get_mut(machine_id) {
            let mut backend = machine
                .deepseek_backend
                .clone()
                .unwrap_or(DeepseekBackendInfo {
                    api_base_url: None,
                    api_key: None,
                    api_key_configured: false,
                });

            if let Some(url) = normalize_optional_string(api_base_url) {
                backend.api_base_url = Some(url);
            }

            if clear_api_key {
                backend.api_key = None;
                backend.api_key_configured = false;
            } else if let Some(key) = normalize_optional_string(api_key) {
                backend.api_key = Some(key);
                backend.api_key_configured = true;
            } else {
                backend.api_key_configured = backend.api_key.is_some();
            }

            machine.deepseek_backend = Some(backend);
            machine.last_seen = Some(chrono::Utc::now().to_rfc3339());
        }

        if let Some(user_id) = owner {
            self.broadcast_machines_update_for_user(&user_id);
        }
    }

    pub async fn send_to_daemon(&self, machine_id: &Uuid, msg: ServerToDaemon) -> bool {
        let sender = self.daemon_senders.get(machine_id).map(|s| s.clone());
        if let Some(sender) = sender {
            matches!(
                tokio::time::timeout(Duration::from_secs(5), sender.send(msg)).await,
                Ok(Ok(()))
            )
        } else {
            false
        }
    }

    /// Relay a daemon's create-instance result to the owning user's web clients
    /// as a `ServerToWeb::ProjectInstanceCreated` toast (a create can succeed or
    /// fail before any project_id exists, so the generic Machines refresh can't
    /// convey it). Best-effort via try_send; dropped messages are acceptable.
    pub fn relay_project_instance_created(
        &self,
        machine_id: &Uuid,
        request_id: Option<String>,
        project_id: Option<String>,
        error: Option<String>,
    ) {
        let Some(owner) = self.daemon_users.get(machine_id).map(|e| *e) else {
            return;
        };
        let msg = ServerToWeb::ProjectInstanceCreated {
            machine_id: *machine_id,
            request_id,
            project_id,
            error,
        };
        for web_entry in self.web_users.iter() {
            if *web_entry.value() != owner {
                continue;
            }
            if let Some(sender) = self.web_senders.get(web_entry.key()) {
                let _ = sender.try_send(msg.clone());
            }
        }
    }

    pub fn get_machines_for_user(&self, user_id: &Uuid) -> Vec<MachineWithProjects> {
        // Collect working dirs of active CLI sessions grouped by hostname
        let mut active_dirs_by_host: HashMap<String, HashSet<String>> = HashMap::new();
        for session_entry in self.sessions.iter() {
            let session = session_entry.value();
            if session.cli_client_id.is_some() {
                if let (Some(hostname), Some(working_dir)) =
                    (&session.hostname, &session.working_dir)
                {
                    active_dirs_by_host
                        .entry(normalize_machine_hostname(hostname))
                        .or_default()
                        .insert(normalize_project_path(working_dir));
                }
            }
        }

        self.machine_infos
            .iter()
            .filter_map(|entry| {
                let machine_id = *entry.key();
                let owner_matches = self
                    .daemon_users
                    .get(&machine_id)
                    .map(|owner| *owner == *user_id)
                    .unwrap_or(false);
                if !owner_matches {
                    return None;
                }

                let machine = entry.value().clone();
                let mut projects = self
                    .machine_projects
                    .get(&machine_id)
                    .map(|p| p.clone())
                    .unwrap_or_default();

                // Enrich is_running from active CLI sessions on the same host
                if let Some(active_dirs) =
                    active_dirs_by_host.get(&normalize_machine_hostname(&machine.hostname))
                {
                    for project in &mut projects {
                        if !project.is_running
                            && active_dirs.contains(&normalize_project_path(&project.path))
                        {
                            project.is_running = true;
                        }
                    }
                }

                Some(MachineWithProjects { machine, projects })
            })
            .collect()
    }

    pub fn get_machines_for_project_refs(
        &self,
        host_path_refs: &HashSet<(String, String)>,
        wildcard_paths: &HashSet<String>,
    ) -> Vec<MachineWithProjects> {
        if host_path_refs.is_empty() && wildcard_paths.is_empty() {
            return Vec::new();
        }

        // Collect working dirs of active CLI sessions grouped by hostname.
        let mut active_dirs_by_host: HashMap<String, HashSet<String>> = HashMap::new();
        for session_entry in self.sessions.iter() {
            let session = session_entry.value();
            if session.cli_client_id.is_some() {
                if let (Some(hostname), Some(working_dir)) =
                    (&session.hostname, &session.working_dir)
                {
                    active_dirs_by_host
                        .entry(normalize_machine_hostname(hostname))
                        .or_default()
                        .insert(normalize_project_path(working_dir));
                }
            }
        }

        self.machine_infos
            .iter()
            .filter_map(|entry| {
                let machine_id = *entry.key();
                let machine = entry.value().clone();
                let host_key = normalize_machine_hostname(&machine.hostname);

                let mut projects = self
                    .machine_projects
                    .get(&machine_id)
                    .map(|p| p.clone())
                    .unwrap_or_default();

                // Enrich running status from active sessions.
                if let Some(active_dirs) = active_dirs_by_host.get(&host_key) {
                    for project in &mut projects {
                        if !project.is_running
                            && active_dirs.contains(&normalize_project_path(&project.path))
                        {
                            project.is_running = true;
                        }
                    }
                }

                projects.retain(|project| {
                    let path_key = normalize_project_path(&project.path);
                    wildcard_paths.contains(&path_key)
                        || host_path_refs.contains(&(host_key.clone(), path_key))
                });

                if projects.is_empty() {
                    None
                } else {
                    Some(MachineWithProjects { machine, projects })
                }
            })
            .collect()
    }

    pub fn machine_project_matches_refs(
        &self,
        machine_id: &Uuid,
        project_id: &str,
        host_path_refs: &HashSet<(String, String)>,
        wildcard_paths: &HashSet<String>,
    ) -> bool {
        let Some(machine) = self.machine_infos.get(machine_id) else {
            return false;
        };
        let host_key = normalize_machine_hostname(&machine.hostname);

        let Some(projects) = self.machine_projects.get(machine_id) else {
            return false;
        };
        let Some(project) = projects.iter().find(|p| p.project_id == project_id) else {
            return false;
        };

        let path_key = normalize_project_path(&project.path);
        wildcard_paths.contains(&path_key) || host_path_refs.contains(&(host_key, path_key))
    }

    fn broadcast_machines_update_for_user(&self, user_id: &Uuid) {
        let mut machines = self.get_machines_for_user(user_id);
        // Union with shared-project-access machines so pushed broadcasts match
        // what `list_accessible_machines_for_user` returns on explicit refresh;
        // otherwise the heartbeat-driven push would drop teammate machines and
        // the UI would visibly flap between refresh and heartbeat.
        if let Some(refs_entry) = self.shared_project_refs.get(user_id) {
            let (host_path_refs, wildcard_paths) = refs_entry.value();
            if !host_path_refs.is_empty() || !wildcard_paths.is_empty() {
                let owner_ids: HashSet<Uuid> =
                    machines.iter().map(|m| m.machine.machine_id).collect();
                for machine in self.get_machines_for_project_refs(host_path_refs, wildcard_paths) {
                    if !owner_ids.contains(&machine.machine.machine_id) {
                        machines.push(machine);
                    }
                }
            }
        }
        let msg = ServerToWeb::Machines { machines };

        for web_entry in self.web_users.iter() {
            if *web_entry.value() != *user_id {
                continue;
            }
            let connection_id = *web_entry.key();
            if let Some(sender) = self.web_senders.get(&connection_id) {
                // Use try_send to avoid blocking and unbounded task spawning.
                // Dropping a periodic broadcast message is acceptable.
                let _ = sender.try_send(msg.clone());
            }
        }
    }

    // Web client management
    pub fn register_web(&self, connection_id: Uuid, sender: mpsc::Sender<ServerToWeb>) {
        self.web_senders.insert(connection_id, sender);
        tracing::info!("Web client registered: {}", connection_id);
    }

    pub fn set_web_user(&self, connection_id: Uuid, user_id: Uuid) {
        self.web_users.insert(connection_id, user_id);
    }

    /// The authenticated user behind a web connection, if any. Lets control
    /// handlers re-check session access (and auto-attach) without threading
    /// `user_id` through every call site.
    pub fn get_web_user(&self, connection_id: &Uuid) -> Option<Uuid> {
        self.web_users.get(connection_id).map(|e| *e.value())
    }

    pub fn unregister_web(&self, connection_id: &Uuid) {
        self.web_senders.remove(connection_id);
        self.web_users.remove(connection_id);
        // Remove this connection from any sessions it was viewing
        for mut session in self.sessions.iter_mut() {
            session.web_connection_ids.retain(|id| id != connection_id);
        }
        tracing::info!("Web client unregistered: {}", connection_id);
    }

    // Session management
    pub fn create_session(&self, session_id: Uuid, user_id: Uuid, web_connection_id: Uuid) {
        let state = SessionState {
            session_id,
            user_id,
            cli_client_id: None,
            web_connection_ids: vec![web_connection_id],
            is_paused: false,
            panes: Vec::new(),
            pane_statuses: HashMap::new(),
            project_goal: None,
            working_dir: None,
            hostname: None,
        };
        self.sessions.insert(session_id, state);
        tracing::info!("Session created: {}", session_id);
    }

    pub fn assign_cli_to_session(&self, session_id: &Uuid, cli_id: Uuid) -> bool {
        if let Some(mut session) = self.sessions.get_mut(session_id) {
            session.cli_client_id = Some(cli_id);
            // Track this session for the CLI
            if let Some(mut sessions) = self.cli_sessions.get_mut(&cli_id) {
                sessions.push(*session_id);
            }
            tracing::info!("CLI {} assigned to session {}", cli_id, session_id);
            return true;
        }
        false
    }

    /// Create or update a CLI-initiated session (hybrid mode)
    /// Preserves web connections if session already exists (for reconnection)
    pub fn create_cli_session(
        &self,
        session_id: Uuid,
        cli_id: Uuid,
        working_dir: Option<String>,
        hostname: Option<String>,
    ) {
        // Check if session already exists (preserve web connections)
        if let Some(mut existing) = self.sessions.get_mut(&session_id) {
            let old_cli_id = existing.cli_client_id;
            existing.cli_client_id = Some(cli_id);
            existing.working_dir = working_dir;
            existing.hostname = hostname;
            // Drop the RefMut before accessing cli_sessions to avoid potential deadlock
            let web_viewers = existing.web_connection_ids.len();
            drop(existing);

            // Remove session from old CLI's tracking to prevent stale unregister_cli
            // from clearing the new CLI's association
            if let Some(old_id) = old_cli_id {
                if old_id != cli_id {
                    if let Some(mut old_sessions) = self.cli_sessions.get_mut(&old_id) {
                        old_sessions.retain(|s| *s != session_id);
                    }
                }
            }

            tracing::info!(
                "CLI session {} updated: cli {:?} -> {} (web viewers: {})",
                session_id,
                old_cli_id,
                cli_id,
                web_viewers,
            );
        } else {
            let state = SessionState {
                session_id,
                user_id: Uuid::nil(), // No user for CLI-initiated sessions
                cli_client_id: Some(cli_id),
                web_connection_ids: Vec::new(),
                is_paused: false,
                panes: Vec::new(),
                pane_statuses: HashMap::new(),
                project_goal: None,
                working_dir,
                hostname,
            };
            self.sessions.insert(session_id, state);
            tracing::info!("CLI session created: {} (cli: {})", session_id, cli_id);
        }

        // Track this session for the CLI
        if let Some(mut sessions) = self.cli_sessions.get_mut(&cli_id) {
            if !sessions.contains(&session_id) {
                sessions.push(session_id);
            }
        }

        // Broadcast machines update (is_running depends on active sessions)
        if let Some(user_id) = self.cli_users.get(&cli_id).map(|e| *e) {
            self.broadcast_machines_update_for_user(&user_id);
        }
        // Broadcast updated client list to all web clients (shows active session)
        self.broadcast_cli_clients_update();
    }

    /// Attach a web client to an existing session (to observe CLI output)
    /// If the session doesn't exist in memory, creates it (for reconnection scenarios)
    pub fn attach_web_to_session(
        &self,
        session_id: &Uuid,
        web_connection_id: Uuid,
        cli_client_id: Option<Uuid>,
    ) -> bool {
        // Multi-attach: leave any previously-attached sessions alone so the
        // web client keeps receiving stream_messages for all of them in
        // parallel. The client routes per-session into its sessionCache so
        // background tabs stay live. Disconnect (`unregister_web`) handles
        // the cleanup wholesale, so we don't leak attachments across page
        // reloads.

        if let Some(mut session) = self.sessions.get_mut(session_id) {
            // Add this web client if not already attached
            if !session.web_connection_ids.contains(&web_connection_id) {
                session.web_connection_ids.push(web_connection_id);
            }
            // Update CLI client ID if provided (for reconnection)
            if let Some(cli_id) = cli_client_id {
                session.cli_client_id = Some(cli_id);
            }
            tracing::info!(
                "Web client {} attached to session {} (total viewers: {})",
                web_connection_id,
                session_id,
                session.web_connection_ids.len()
            );
            return true;
        }

        // Session not in memory - create it (happens after server restart or reconnection)
        tracing::info!(
            "Creating session {} in memory for web attach (cli: {:?})",
            session_id,
            cli_client_id
        );
        let state = SessionState {
            session_id: *session_id,
            user_id: Uuid::nil(), // Will be updated when needed
            cli_client_id,
            web_connection_ids: vec![web_connection_id],
            is_paused: false,
            panes: Vec::new(),
            pane_statuses: HashMap::new(),
            project_goal: None,
            working_dir: None,
            hostname: None,
        };
        self.sessions.insert(*session_id, state);

        // If we have a CLI ID, track this session for the CLI
        if let Some(cli_id) = cli_client_id {
            if let Some(mut sessions) = self.cli_sessions.get_mut(&cli_id) {
                if !sessions.contains(session_id) {
                    sessions.push(*session_id);
                }
            }
        }

        true
    }

    /// Is this web connection currently attached to the given session?
    /// Used to validate `WebToServer` messages that carry an explicit
    /// session_id — without this gate, a connection could route input to
    /// sessions it never asked to observe (and that the user-access check
    /// at attach time vouched for).
    pub fn is_web_attached_to_session(
        &self,
        session_id: &Uuid,
        web_connection_id: &Uuid,
    ) -> bool {
        self.sessions
            .get(session_id)
            .map(|s| s.web_connection_ids.contains(web_connection_id))
            .unwrap_or(false)
    }

    /// Get the active session for a CLI client
    pub fn get_cli_active_session(&self, cli_id: &Uuid) -> Option<Uuid> {
        self.cli_sessions
            .get(cli_id)
            .and_then(|sessions| sessions.last().copied())
    }

    /// Get all session IDs for a CLI client
    pub fn get_cli_session_ids(&self, cli_id: &Uuid) -> Vec<Uuid> {
        self.cli_sessions
            .get(cli_id)
            .map(|sessions| sessions.clone())
            .unwrap_or_default()
    }

    pub fn get_session(&self, session_id: &Uuid) -> Option<SessionState> {
        self.sessions.get(session_id).map(|s| SessionState {
            session_id: s.session_id,
            user_id: s.user_id,
            cli_client_id: s.cli_client_id,
            web_connection_ids: s.web_connection_ids.clone(),
            is_paused: s.is_paused,
            panes: s.panes.clone(),
            pane_statuses: s.pane_statuses.clone(),
            project_goal: s.project_goal.clone(),
            working_dir: s.working_dir.clone(),
            hostname: s.hostname.clone(),
        })
    }

    /// Cache the latest `project_goal.md` content for this session so we can
    /// replay it to newly-attaching web clients. Called from the CLI's
    /// `ProjectGoalChanged` forwarder.
    pub fn set_project_goal(&self, session_id: &Uuid, content: String) {
        if let Some(mut session) = self.sessions.get_mut(session_id) {
            session.project_goal = Some(content);
        }
    }

    /// Read the cached project goal for replay on web-client attach.
    pub fn get_project_goal(&self, session_id: &Uuid) -> Option<String> {
        self.sessions
            .get(session_id)
            .and_then(|s| s.project_goal.clone())
    }

    /// Replay the cached project goal to one newly-attached web client.
    pub async fn replay_project_goal_to_web(
        &self,
        session_id: &Uuid,
        web_connection_id: &Uuid,
    ) -> bool {
        if let Some(content) = self.get_project_goal(session_id) {
            self.send_to_web(
                web_connection_id,
                ServerToWeb::ProjectGoalChanged {
                    session_id: *session_id,
                    content,
                },
            )
            .await
        } else {
            false
        }
    }

    /// Check if a session has an active CLI client connected
    pub fn is_session_active(&self, session_id: &Uuid) -> bool {
        // Check if any connected CLI client has this session as their active session
        for entry in self.cli_sessions.iter() {
            let cli_id = entry.key();
            let sessions = entry.value();
            let is_connected = self.cli_senders.contains_key(cli_id);
            // Check if this CLI has the session and is still connected
            if sessions.last() == Some(session_id) && is_connected {
                return true;
            }
        }
        false
    }

    /// Update the pause state for a session
    pub fn set_session_paused(&self, session_id: &Uuid, is_paused: bool) {
        if let Some(mut session) = self.sessions.get_mut(session_id) {
            session.is_paused = is_paused;
        }
    }

    /// Get the pause state for a session
    pub fn is_session_paused(&self, session_id: &Uuid) -> bool {
        self.sessions
            .get(session_id)
            .map(|s| s.is_paused)
            .unwrap_or(false)
    }

    /// Check if a session exists in memory (for determining if DB fallback is needed)
    pub fn has_session_state(&self, session_id: &Uuid) -> bool {
        self.sessions.contains_key(session_id)
    }

    /// Cache pane configurations for a session (from CLI PaneList)
    pub fn set_session_panes(&self, session_id: &Uuid, panes: Vec<PaneConfig>) {
        if let Some(mut session) = self.sessions.get_mut(session_id) {
            session.panes = panes;
        }
    }

    /// Get cached pane configurations for a session
    pub fn get_session_panes(&self, session_id: &Uuid) -> Vec<PaneConfig> {
        self.sessions
            .get(session_id)
            .map(|s| s.panes.clone())
            .unwrap_or_default()
    }

    /// Cache the latest pane status so it can be replayed when a web client
    /// re-attaches. `None` status clears the cache entry (pane is idle).
    pub fn set_pane_status(
        &self,
        session_id: &Uuid,
        pane_type: PaneType,
        pane_id: u32,
        status: Option<String>,
    ) {
        if let Some(mut session) = self.sessions.get_mut(session_id) {
            match status {
                Some(s) => {
                    session.pane_statuses.insert(pane_id, (pane_type, s));
                }
                None => {
                    session.pane_statuses.remove(&pane_id);
                }
            }
        }
    }

    /// Get cached pane statuses for replay on web re-attach.
    pub fn get_pane_statuses(&self, session_id: &Uuid) -> Vec<(PaneType, u32, String)> {
        self.sessions
            .get(session_id)
            .map(|s| {
                s.pane_statuses
                    .iter()
                    .map(|(pane_id, (pane_type, status))| (*pane_type, *pane_id, status.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }


    // Message routing
    pub async fn send_to_cli(&self, cli_id: &Uuid, msg: ServerToCli) -> bool {
        // Clone sender and drop DashMap Ref before awaiting to prevent deadlock.
        // Holding the Ref across .await would block DashMap write operations
        // (e.g., unregister_cli) on the same shard, freezing the server.
        let sender = self.cli_senders.get(cli_id).map(|s| s.clone());
        if let Some(sender) = sender {
            matches!(
                tokio::time::timeout(Duration::from_secs(5), sender.send(msg)).await,
                Ok(Ok(()))
            )
        } else {
            false
        }
    }

    pub async fn send_to_web(&self, connection_id: &Uuid, msg: ServerToWeb) -> bool {
        // Clone sender and drop DashMap Ref before awaiting to prevent deadlock.
        let sender = self.web_senders.get(connection_id).map(|s| s.clone());
        if let Some(sender) = sender {
            matches!(
                tokio::time::timeout(Duration::from_secs(5), sender.send(msg)).await,
                Ok(Ok(()))
            )
        } else {
            false
        }
    }

    pub async fn route_to_cli(&self, session_id: &Uuid, msg: ServerToCli) -> bool {
        if let Some(session) = self.sessions.get(session_id) {
            if let Some(cli_id) = session.cli_client_id {
                let cli_exists = self.cli_senders.contains_key(&cli_id);
                tracing::debug!(
                    "route_to_cli: session {} -> cli {} (cli exists in senders: {})",
                    session_id,
                    cli_id,
                    cli_exists
                );
                return self.send_to_cli(&cli_id, msg).await;
            } else {
                tracing::warn!("route_to_cli: session {} has no cli_client_id", session_id);
            }
        } else {
            tracing::warn!("route_to_cli: session {} not found in memory", session_id);
        }
        false
    }

    pub async fn route_to_web(&self, session_id: &Uuid, msg: ServerToWeb) -> bool {
        if let Some(session) = self.sessions.get(session_id) {
            if session.web_connection_ids.is_empty() {
                tracing::debug!("No web clients attached to session {}", session_id);
                return false;
            }
            let web_ids = session.web_connection_ids.clone();
            drop(session); // Release lock before sending
            let mut any_sent = false;
            for web_id in &web_ids {
                tracing::debug!(
                    "Routing message to web client {} for session {}",
                    web_id,
                    session_id
                );
                // try_send, never await: one stale connection with a full
                // queue (e.g. backgrounded phone with a dead TCP) used to
                // block every broadcast for the 5s send timeout, delaying
                // user_input echoes past the web client's 3s retransmit
                // deadline — the cause of duplicate stored inputs. Dropping
                // a frame for a backlogged client is safe: it repairs via
                // the watermark catchup on reconnect/visibility.
                let sender = self.web_senders.get(web_id).map(|s| s.clone());
                let Some(sender) = sender else { continue };
                match sender.try_send(msg.clone()) {
                    Ok(()) => any_sent = true,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        tracing::warn!(
                            "Web client {} send queue full — dropping broadcast for session {}",
                            web_id,
                            session_id
                        );
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {}
                }
            }
            return any_sent;
        } else {
            tracing::debug!("Session {} not found for routing", session_id);
        }
        false
    }

    // Get available CLI clients for a user
    pub fn get_online_cli_ids(&self) -> Vec<Uuid> {
        self.cli_senders.iter().map(|r| *r.key()).collect()
    }

    /// Get CLI clients info for the web UI (all clients)
    pub fn get_cli_clients_info(&self) -> Vec<CliClientInfo> {
        self.cli_senders
            .iter()
            .map(|entry| {
                let cli_id = *entry.key();
                // Get active session for this CLI
                let active_session = self.get_cli_active_session(&cli_id);
                let is_busy = active_session.is_some();

                CliClientInfo {
                    id: cli_id,
                    name: None, // CLI name not tracked yet
                    status: if is_busy {
                        CliClientStatus::Busy
                    } else {
                        CliClientStatus::Online
                    },
                    last_seen: Some(chrono::Utc::now()),
                    version: self.cli_versions.get(&cli_id).map(|v| v.clone()),
                    active_session,
                }
            })
            .collect()
    }

    /// Get CLI clients info for a specific user
    pub fn get_cli_clients_info_for_user(&self, user_id: &Uuid) -> Vec<CliClientInfo> {
        self.cli_senders
            .iter()
            .filter(|entry| {
                // Only include CLIs owned by this user
                self.cli_users
                    .get(entry.key())
                    .map(|u| *u == *user_id)
                    .unwrap_or(false)
            })
            .map(|entry| {
                let cli_id = *entry.key();
                // Get active session for this CLI
                let active_session = self.get_cli_active_session(&cli_id);
                let is_busy = active_session.is_some();

                CliClientInfo {
                    id: cli_id,
                    name: None, // CLI name not tracked yet
                    status: if is_busy {
                        CliClientStatus::Busy
                    } else {
                        CliClientStatus::Online
                    },
                    last_seen: Some(chrono::Utc::now()),
                    version: self.cli_versions.get(&cli_id).map(|v| v.clone()),
                    active_session,
                }
            })
            .collect()
    }

    /// Broadcast CLI clients list to all connected web clients (filtered per user)
    fn broadcast_cli_clients_update(&self) {
        // Build a per-user cache to avoid recomputing for each web connection
        let mut user_msgs: std::collections::HashMap<Uuid, ServerToWeb> =
            std::collections::HashMap::new();

        for web_entry in self.web_users.iter() {
            let connection_id = *web_entry.key();
            let user_id = *web_entry.value();
            let msg = user_msgs.entry(user_id).or_insert_with(|| {
                let clients = self.get_cli_clients_info_for_user(&user_id);
                ServerToWeb::CliClients { clients }
            });
            if let Some(sender) = self.web_senders.get(&connection_id) {
                let _ = sender.try_send(msg.clone());
            }
        }
    }

    /// Update usage limits for a CLI client and broadcast to the owning user's web clients
    pub fn update_usage_limits(&self, cli_id: Uuid, provider: Provider, limits: UsageLimits) {
        self.cli_usage_limits
            .insert((cli_id, provider.clone()), limits.clone());

        // Only send to web clients belonging to the CLI's owner
        let owner = self.cli_users.get(&cli_id).map(|e| *e);
        let msg = ServerToWeb::UsageLimits {
            cli_client_id: cli_id,
            provider,
            limits,
        };

        if let Some(user_id) = owner {
            for web_entry in self.web_users.iter() {
                if *web_entry.value() != user_id {
                    continue;
                }
                if let Some(sender) = self.web_senders.get(web_entry.key()) {
                    let _ = sender.try_send(msg.clone());
                }
            }
        }
    }

    /// Get usage limits for a CLI client
    pub fn get_usage_limits(&self, cli_id: &Uuid, provider: Provider) -> Option<UsageLimits> {
        self.cli_usage_limits
            .get(&(*cli_id, provider))
            .map(|r| r.clone())
    }

    /// Get all usage limits for all CLI clients
    pub fn get_all_usage_limits(&self) -> Vec<(Uuid, Provider, UsageLimits)> {
        self.cli_usage_limits
            .iter()
            .map(|entry| (entry.key().0, entry.key().1.clone(), entry.value().clone()))
            .collect()
    }
}

#[cfg(test)]
mod project_goal_tests {
    use super::*;

    #[tokio::test]
    async fn cached_project_goal_replays_to_new_web_attachment() {
        let sessions = SessionManager::new();
        let session_id = Uuid::new_v4();
        let cli_id = Uuid::new_v4();
        let web_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let (cli_tx, _cli_rx) = mpsc::channel(1);
        let (web_tx, mut web_rx) = mpsc::channel(1);

        sessions.register_cli(cli_id, user_id, cli_tx, None);
        sessions.create_cli_session(
            session_id,
            cli_id,
            Some("/work/project".to_string()),
            Some("host".to_string()),
        );
        sessions.set_project_goal(&session_id, "line one\n\nline two\n".to_string());
        sessions.register_web(web_id, web_tx);

        assert!(sessions.attach_web_to_session(&session_id, web_id, Some(cli_id)));
        assert!(sessions.replay_project_goal_to_web(&session_id, &web_id).await);

        match web_rx.recv().await.expect("project goal replay") {
            ServerToWeb::ProjectGoalChanged {
                session_id: got_session,
                content,
            } => {
                assert_eq!(got_session, session_id);
                assert_eq!(content, "line one\n\nline two\n");
            }
            other => panic!("unexpected replay message: {other:?}"),
        }
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_machine(machine_id: Uuid, hostname: &str) -> MachineInfo {
        MachineInfo {
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

    fn test_project(project_id: &str, path: &str) -> MachineProjectInfo {
        MachineProjectInfo {
            project_id: project_id.to_string(),
            name: Some(project_id.to_string()),
            path: path.to_string(),
            is_running: false,
            pid: None,
            memory_kb: None,
            last_error: None,
        }
    }

    fn sorted_pane_statuses(
        mgr: &SessionManager,
        session_id: &Uuid,
    ) -> Vec<(PaneType, u32, String)> {
        let mut statuses = mgr.get_pane_statuses(session_id);
        statuses.sort_by_key(|(_, pane_id, _)| *pane_id);
        statuses
    }

    #[test]
    fn input_id_dedup_remembers_and_caps() {
        let mgr = SessionManager::new();
        let sid = Uuid::new_v4();

        assert_eq!(mgr.seen_input_id(&sid, "a"), None);
        mgr.record_input_id(sid, "a".to_string(), "t1".to_string());
        assert_eq!(mgr.seen_input_id(&sid, "a").as_deref(), Some("t1"));
        // Different session is independent.
        assert_eq!(mgr.seen_input_id(&Uuid::new_v4(), "a"), None);

        // Ring is bounded: after 64 more inserts, "a" has been evicted.
        for i in 0..64 {
            mgr.record_input_id(sid, format!("id-{i}"), format!("t-{i}"));
        }
        assert_eq!(mgr.seen_input_id(&sid, "a"), None);
        assert_eq!(mgr.seen_input_id(&sid, "id-63").as_deref(), Some("t-63"));
    }

    #[test]
    fn pane_status_cache_replays_latest_statuses_and_clears_idle_panes() {
        let mgr = SessionManager::new();
        let sid = Uuid::new_v4();
        mgr.create_session(sid, Uuid::new_v4(), Uuid::new_v4());

        assert!(mgr.get_pane_statuses(&sid).is_empty());

        mgr.set_pane_status(
            &sid,
            PaneType::Deadloop,
            11,
            Some("Thinking...".to_string()),
        );
        mgr.set_pane_status(
            &sid,
            PaneType::Interactive,
            22,
            Some("Waiting for input".to_string()),
        );

        assert_eq!(
            sorted_pane_statuses(&mgr, &sid),
            vec![
                (PaneType::Deadloop, 11, "Thinking...".to_string()),
                (PaneType::Interactive, 22, "Waiting for input".to_string()),
            ]
        );

        mgr.set_pane_status(
            &sid,
            PaneType::Interactive,
            11,
            Some("Running tool".to_string()),
        );

        assert_eq!(
            sorted_pane_statuses(&mgr, &sid),
            vec![
                (PaneType::Interactive, 11, "Running tool".to_string()),
                (PaneType::Interactive, 22, "Waiting for input".to_string()),
            ],
            "latest status and PaneType should replace the previous entry for that pane_id"
        );

        mgr.set_pane_status(&sid, PaneType::Interactive, 11, None);

        assert_eq!(
            sorted_pane_statuses(&mgr, &sid),
            vec![(PaneType::Interactive, 22, "Waiting for input".to_string())],
            "None status clears only the matching pane entry"
        );
    }

    #[test]
    fn broadcast_machines_update_includes_cached_shared_refs_without_duplicates() {
        let mgr = SessionManager::new();
        let viewer_id = Uuid::new_v4();
        let teammate_id = Uuid::new_v4();
        let owned_machine_id = Uuid::new_v4();
        let shared_machine_id = Uuid::new_v4();

        let (owned_tx, _owned_rx) = mpsc::channel(1);
        mgr.register_daemon(
            owned_machine_id,
            viewer_id,
            owned_tx,
            test_machine(owned_machine_id, "ViewerHost"),
            vec![test_project("owned", "/work/owned")],
        );
        let (shared_tx, _shared_rx) = mpsc::channel(1);
        mgr.register_daemon(
            shared_machine_id,
            teammate_id,
            shared_tx,
            test_machine(shared_machine_id, "SharedHost"),
            vec![
                test_project("shared-match", "/team/shared"),
                test_project("shared-other", "/team/other"),
            ],
        );

        mgr.set_shared_project_refs_for_user(
            viewer_id,
            HashSet::from([
                ("sharedhost".to_string(), "/team/shared".to_string()),
                ("viewerhost".to_string(), "/work/owned".to_string()),
            ]),
            HashSet::new(),
        );

        let web_connection_id = Uuid::new_v4();
        let (web_tx, mut web_rx) = mpsc::channel(1);
        mgr.register_web(web_connection_id, web_tx);
        mgr.set_web_user(web_connection_id, viewer_id);

        mgr.broadcast_machines_update_for_user(&viewer_id);

        let ServerToWeb::Machines { machines } = web_rx.try_recv().expect("machine broadcast")
        else {
            panic!("expected machines broadcast");
        };
        assert_eq!(
            machines
                .iter()
                .filter(|machine| machine.machine.machine_id == owned_machine_id)
                .count(),
            1,
            "owned machine should not be duplicated when it also matches shared refs"
        );
        let shared = machines
            .iter()
            .find(|machine| machine.machine.machine_id == shared_machine_id)
            .expect("broadcast should include cached shared machine");
        assert_eq!(shared.projects.len(), 1);
        assert_eq!(shared.projects[0].project_id, "shared-match");
    }

    #[tokio::test]
    async fn route_to_web_skips_full_sender_and_delivers_to_available_clients() {
        let mgr = SessionManager::new();
        let session_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let full_web_id = Uuid::new_v4();
        let available_web_id = Uuid::new_v4();

        let (full_tx, mut full_rx) = mpsc::channel(1);
        full_tx
            .try_send(ServerToWeb::SessionStatus {
                status: shared::SessionStatus::Pending,
            })
            .expect("pre-fill stale client queue");
        mgr.register_web(full_web_id, full_tx);
        mgr.create_session(session_id, user_id, full_web_id);

        let (available_tx, mut available_rx) = mpsc::channel(1);
        mgr.register_web(available_web_id, available_tx);
        assert!(mgr.attach_web_to_session(&session_id, available_web_id, None));

        let sent = mgr
            .route_to_web(
                &session_id,
                ServerToWeb::SessionStatus {
                    status: shared::SessionStatus::Connected,
                },
            )
            .await;

        assert!(sent, "available client should receive the broadcast");
        match available_rx
            .try_recv()
            .expect("available client receives broadcast")
        {
            ServerToWeb::SessionStatus { status } => {
                assert_eq!(status, shared::SessionStatus::Connected);
            }
            other => panic!("expected session status broadcast, got {other:?}"),
        }
        match full_rx
            .try_recv()
            .expect("full client keeps original queued message")
        {
            ServerToWeb::SessionStatus { status } => {
                assert_eq!(status, shared::SessionStatus::Pending);
            }
            other => panic!("expected original session status message, got {other:?}"),
        }
    }

    #[test]
    fn terminal_scrollback_accumulates_and_reports_latest_seq() {
        let sessions = SessionManager::new();
        let sid = Uuid::new_v4();
        let instance = Uuid::new_v4();

        assert!(sessions.terminal_snapshot(&sid, 7).is_none());

        assert!(sessions.append_terminal_output(&sid, 7, Some(instance), b"hello ", 0));
        assert!(sessions.append_terminal_output(&sid, 7, Some(instance), b"world", 1));

        let snapshot = sessions.terminal_snapshot(&sid, 7).expect("snapshot");
        assert_eq!(snapshot.bytes, b"hello world");
        assert_eq!(snapshot.seq, 1);
        assert!(!snapshot.truncated);
        assert_eq!(snapshot.instance_id, Some(instance));
        assert_eq!(snapshot.lifecycle, TerminalLifecycle::Unknown);
    }

    #[test]
    fn terminal_scrollback_evicts_oldest_and_flags_truncation() {
        let sessions = SessionManager::new();
        let sid = Uuid::new_v4();

        // Overflow the cap, then verify we kept the *newest* bytes: a
        // terminal's useful state is its latest frame, so dropping from
        // the front is the only correct eviction direction.
        let filler = vec![b'a'; TERMINAL_SCROLLBACK_MAX_BYTES];
        sessions.append_terminal_output(&sid, 1, None, &filler, 0);
        sessions.append_terminal_output(&sid, 1, None, b"TAIL", 1);

        let snapshot = sessions.terminal_snapshot(&sid, 1).expect("snapshot");
        assert_eq!(snapshot.bytes.len(), TERMINAL_SCROLLBACK_MAX_BYTES);
        assert!(snapshot.bytes.ends_with(b"TAIL"));
        assert_eq!(snapshot.seq, 1);
        assert!(
            snapshot.truncated,
            "client must be told the replay may start mid-escape-sequence"
        );
    }

    #[test]
    fn terminal_scrollback_is_isolated_per_pane_and_session() {
        let sessions = SessionManager::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();

        sessions.append_terminal_output(&a, 1, None, b"pane-a1", 0);
        sessions.append_terminal_output(&a, 2, None, b"pane-a2", 0);
        sessions.append_terminal_output(&b, 1, None, b"pane-b1", 0);

        assert_eq!(sessions.terminal_snapshot(&a, 1).unwrap().bytes, b"pane-a1");
        assert_eq!(sessions.terminal_snapshot(&a, 2).unwrap().bytes, b"pane-a2");
        assert_eq!(sessions.terminal_snapshot(&b, 1).unwrap().bytes, b"pane-b1");

        // Removing one pane must not disturb the other panes or sessions —
        // a pane_id is only unique within a session.
        sessions.clear_terminal_scrollback(&a, 1);
        assert!(sessions.terminal_snapshot(&a, 1).is_none());
        assert_eq!(sessions.terminal_snapshot(&a, 2).unwrap().bytes, b"pane-a2");
        assert_eq!(sessions.terminal_snapshot(&b, 1).unwrap().bytes, b"pane-b1");
    }

    #[test]
    fn cli_disconnect_retains_bytes_and_preserves_exited_state() {
        let sessions = SessionManager::new();
        let dead = Uuid::new_v4();
        let alive = Uuid::new_v4();
        let running_instance = Uuid::new_v4();
        let exited_instance = Uuid::new_v4();

        sessions.reconcile_terminal_state(
            &dead,
            1,
            Some(running_instance),
            TerminalLifecycle::Running,
            None,
        );
        sessions.append_terminal_output(&dead, 1, Some(running_instance), b"x", 0);
        sessions.reconcile_terminal_state(
            &dead,
            2,
            Some(exited_instance),
            TerminalLifecycle::Exited,
            Some("status 7".into()),
        );
        sessions.append_terminal_output(&dead, 2, Some(exited_instance), b"y", 0);
        sessions.append_terminal_output(&alive, 1, None, b"z", 0);

        let changed = sessions.mark_session_terminals_disconnected(&dead);
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].0, 1);

        let running = sessions.terminal_snapshot(&dead, 1).unwrap();
        assert_eq!(running.bytes, b"x");
        assert_eq!(running.lifecycle, TerminalLifecycle::Disconnected);
        let exited = sessions.terminal_snapshot(&dead, 2).unwrap();
        assert_eq!(exited.bytes, b"y");
        assert_eq!(exited.lifecycle, TerminalLifecycle::Exited);
        assert_eq!(exited.status.as_deref(), Some("status 7"));
        assert_eq!(sessions.terminal_snapshot(&alive, 1).unwrap().bytes, b"z");
    }

    #[test]
    fn same_instance_reconnect_preserves_presentation() {
        let sessions = SessionManager::new();
        let sid = Uuid::new_v4();
        let instance = Uuid::new_v4();
        sessions.reconcile_terminal_state(
            &sid,
            2,
            Some(instance),
            TerminalLifecycle::Running,
            None,
        );
        sessions.append_terminal_output(&sid, 2, Some(instance), b"before", 0);
        sessions.mark_session_terminals_disconnected(&sid);

        let reconciled = sessions
            .reconcile_terminal_state(&sid, 2, Some(instance), TerminalLifecycle::Running, None)
            .expect("same instance accepted");
        assert_eq!(reconciled.bytes, b"before");
        assert_eq!(reconciled.lifecycle, TerminalLifecycle::Running);
        assert!(sessions.append_terminal_output(&sid, 2, Some(instance), b" after", 1));
        assert_eq!(
            sessions.terminal_snapshot(&sid, 2).unwrap().bytes,
            b"before after"
        );
    }

    #[test]
    fn replacement_instance_resets_and_stale_events_are_ignored() {
        let sessions = SessionManager::new();
        let sid = Uuid::new_v4();
        let old = Uuid::new_v4();
        let new = Uuid::new_v4();
        sessions.reconcile_terminal_state(&sid, 3, Some(old), TerminalLifecycle::Running, None);
        sessions.append_terminal_output(&sid, 3, Some(old), b"old", 4);

        let replacement = sessions
            .reconcile_terminal_state(&sid, 3, Some(new), TerminalLifecycle::Running, None)
            .expect("new running instance replaces old state");
        assert!(replacement.bytes.is_empty());
        assert_eq!(replacement.seq, 0);
        assert_eq!(replacement.instance_id, Some(new));

        assert!(sessions.append_terminal_output(&sid, 3, Some(new), b"new", 0));
        assert!(!sessions.append_terminal_output(&sid, 3, Some(old), b" stale", 5));
        assert!(sessions
            .record_terminal_exit(&sid, 3, Some(old), Some("old exit".into()))
            .is_none());
        assert!(sessions
            .reconcile_terminal_state(
                &sid,
                3,
                Some(old),
                TerminalLifecycle::Exited,
                Some("old state".into()),
            )
            .is_none());
        let current = sessions.terminal_snapshot(&sid, 3).unwrap();
        assert_eq!(current.bytes, b"new");
        assert_eq!(current.lifecycle, TerminalLifecycle::Running);
        assert_eq!(current.instance_id, Some(new));
    }

    #[test]
    fn exit_without_output_is_retained_for_later_attach() {
        let sessions = SessionManager::new();
        let sid = Uuid::new_v4();
        let instance = Uuid::new_v4();
        let snapshot = sessions
            .record_terminal_exit(&sid, 9, Some(instance), Some("status 1".into()))
            .expect("exit accepted");
        assert!(snapshot.bytes.is_empty());
        assert_eq!(snapshot.lifecycle, TerminalLifecycle::Exited);
        assert_eq!(snapshot.status.as_deref(), Some("status 1"));
        assert_eq!(sessions.mark_session_terminals_disconnected(&sid).len(), 0);
        assert_eq!(
            sessions.terminal_snapshot(&sid, 9).unwrap().lifecycle,
            TerminalLifecycle::Exited
        );
    }

    #[test]
    fn legacy_output_is_accepted_but_does_not_confirm_running() {
        let sessions = SessionManager::new();
        let sid = Uuid::new_v4();
        assert!(sessions.append_terminal_output(&sid, 4, None, b"legacy", 0));
        let snapshot = sessions.terminal_snapshot(&sid, 4).unwrap();
        assert_eq!(snapshot.bytes, b"legacy");
        assert_eq!(snapshot.instance_id, None);
        assert_eq!(snapshot.lifecycle, TerminalLifecycle::Unknown);
    }
}
