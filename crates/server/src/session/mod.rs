use dashmap::DashMap;
use shared::{
    CliClientInfo, CliClientStatus, DeepseekBackendInfo, GlmBackendInfo, MachineInfo,
    MachineProjectInfo, MachineWithProjects, MiniMaxBackendInfo, PaneConfig, PaneType, Provider,
    ServerToCli, ServerToDaemon, ServerToWeb, UsageLimits,
};
use std::collections::{HashMap, HashSet};
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
}

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
                if self.send_to_web(web_id, msg.clone()).await {
                    any_sent = true;
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

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}
