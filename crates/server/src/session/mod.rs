use dashmap::DashMap;
use shared::{
    CliClientInfo, CliClientStatus, MachineInfo, MachineProjectInfo, MachineWithProjects,
    PaneConfig, Provider, ServerToCli, ServerToDaemon, ServerToWeb, UsageLimits,
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
        machine.machine_id = machine_id;
        machine.last_seen = Some(chrono::Utc::now().to_rfc3339());
        self.daemon_senders.insert(machine_id, sender);
        self.daemon_users.insert(machine_id, user_id);
        self.machine_infos.insert(machine_id, machine);
        self.machine_projects.insert(machine_id, projects);
        self.broadcast_machines_update_for_user(&user_id);
        tracing::info!("Daemon registered: {} (user: {})", machine_id, user_id);
    }

    pub fn unregister_daemon(&self, machine_id: &Uuid) {
        let owner = self.daemon_users.get(machine_id).map(|entry| *entry);
        self.daemon_senders.remove(machine_id);
        self.daemon_users.remove(machine_id);
        self.machine_infos.remove(machine_id);
        self.machine_projects.remove(machine_id);

        if let Some(user_id) = owner {
            self.broadcast_machines_update_for_user(&user_id);
        }

        tracing::info!("Daemon unregistered: {}", machine_id);
    }

    pub fn update_daemon_projects(&self, machine_id: &Uuid, projects: Vec<MachineProjectInfo>) {
        let running_count = projects.iter().filter(|p| p.is_running).count();
        if running_count > 0 {
            tracing::debug!(
                "Daemon {} heartbeat: {}/{} projects running",
                machine_id,
                running_count,
                projects.len()
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
        let machines = self.get_machines_for_user(user_id);
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
            working_dir: s.working_dir.clone(),
            hostname: s.hostname.clone(),
        })
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

    /// Broadcast CLI clients list to all connected web clients
    fn broadcast_cli_clients_update(&self) {
        let clients = self.get_cli_clients_info();
        let msg = ServerToWeb::CliClients { clients };

        for entry in self.web_senders.iter() {
            // Use try_send to avoid blocking and unbounded task spawning.
            let _ = entry.value().try_send(msg.clone());
        }
    }

    /// Update usage limits for a CLI client and broadcast to web clients
    pub fn update_usage_limits(&self, cli_id: Uuid, provider: Provider, limits: UsageLimits) {
        self.cli_usage_limits
            .insert((cli_id, provider.clone()), limits.clone());

        // Broadcast to all web clients
        let msg = ServerToWeb::UsageLimits {
            cli_client_id: cli_id,
            provider,
            limits,
        };

        for entry in self.web_senders.iter() {
            // Use try_send to avoid blocking and unbounded task spawning.
            let _ = entry.value().try_send(msg.clone());
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
