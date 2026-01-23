use dashmap::DashMap;
use shared::{CliClientInfo, CliClientStatus, ServerToCli, ServerToWeb};
use tokio::sync::mpsc;
use uuid::Uuid;

/// Manages active sessions and routes messages between web and CLI clients
pub struct SessionManager {
    /// Map of session ID -> session state
    sessions: DashMap<Uuid, SessionState>,
    /// Map of CLI client ID -> sender to CLI
    cli_senders: DashMap<Uuid, mpsc::Sender<ServerToCli>>,
    /// Map of web connection ID -> sender to web
    web_senders: DashMap<Uuid, mpsc::Sender<ServerToWeb>>,
    /// Map of CLI client ID -> list of session IDs
    cli_sessions: DashMap<Uuid, Vec<Uuid>>,
    /// Map of CLI client ID -> user ID (owner)
    cli_users: DashMap<Uuid, Uuid>,
}

#[derive(Debug)]
pub struct SessionState {
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub cli_client_id: Option<Uuid>,
    pub web_connection_id: Option<Uuid>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            cli_senders: DashMap::new(),
            web_senders: DashMap::new(),
            cli_sessions: DashMap::new(),
            cli_users: DashMap::new(),
        }
    }

    // CLI client management
    pub fn register_cli(&self, cli_id: Uuid, user_id: Uuid, sender: mpsc::Sender<ServerToCli>) {
        self.cli_senders.insert(cli_id, sender);
        self.cli_sessions.insert(cli_id, Vec::new());
        self.cli_users.insert(cli_id, user_id);
        tracing::info!("CLI client registered: {} (user: {})", cli_id, user_id);
        // Broadcast updated client list to all web clients
        self.broadcast_cli_clients_update();
    }

    pub fn unregister_cli(&self, cli_id: &Uuid) {
        self.cli_senders.remove(cli_id);
        self.cli_users.remove(cli_id);
        if let Some((_, session_ids)) = self.cli_sessions.remove(cli_id) {
            for session_id in session_ids {
                if let Some(mut session) = self.sessions.get_mut(&session_id) {
                    session.cli_client_id = None;
                }
            }
        }
        tracing::info!("CLI client unregistered: {}", cli_id);
        // Broadcast updated client list to all web clients
        self.broadcast_cli_clients_update();
    }

    // Web client management
    pub fn register_web(&self, connection_id: Uuid, sender: mpsc::Sender<ServerToWeb>) {
        self.web_senders.insert(connection_id, sender);
        tracing::info!("Web client registered: {}", connection_id);
    }

    pub fn unregister_web(&self, connection_id: &Uuid) {
        self.web_senders.remove(connection_id);
        // Find and update any sessions using this web connection
        for mut session in self.sessions.iter_mut() {
            if session.web_connection_id == Some(*connection_id) {
                session.web_connection_id = None;
            }
        }
        tracing::info!("Web client unregistered: {}", connection_id);
    }

    // Session management
    pub fn create_session(&self, session_id: Uuid, user_id: Uuid, web_connection_id: Uuid) {
        let state = SessionState {
            session_id,
            user_id,
            cli_client_id: None,
            web_connection_id: Some(web_connection_id),
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
    /// Preserves web_connection_id if session already exists (for reconnection)
    pub fn create_cli_session(&self, session_id: Uuid, cli_id: Uuid) {
        // Check if session already exists (preserve web connection)
        if let Some(mut existing) = self.sessions.get_mut(&session_id) {
            let old_cli_id = existing.cli_client_id;
            existing.cli_client_id = Some(cli_id);
            tracing::info!(
                "CLI session {} updated: cli {:?} -> {} (web: {:?})",
                session_id, old_cli_id, cli_id, existing.web_connection_id
            );
        } else {
            let state = SessionState {
                session_id,
                user_id: Uuid::nil(), // No user for CLI-initiated sessions
                cli_client_id: Some(cli_id),
                web_connection_id: None,
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
        // Broadcast updated client list to all web clients (shows active session)
        self.broadcast_cli_clients_update();
    }

    /// Attach a web client to an existing session (to observe CLI output)
    /// If the session doesn't exist in memory, creates it (for reconnection scenarios)
    pub fn attach_web_to_session(&self, session_id: &Uuid, web_connection_id: Uuid, cli_client_id: Option<Uuid>) -> bool {
        if let Some(mut session) = self.sessions.get_mut(session_id) {
            session.web_connection_id = Some(web_connection_id);
            // Update CLI client ID if provided (for reconnection)
            if let Some(cli_id) = cli_client_id {
                session.cli_client_id = Some(cli_id);
            }
            tracing::info!("Web client {} attached to session {}", web_connection_id, session_id);
            return true;
        }

        // Session not in memory - create it (happens after server restart or reconnection)
        tracing::info!("Creating session {} in memory for web attach (cli: {:?})", session_id, cli_client_id);
        let state = SessionState {
            session_id: *session_id,
            user_id: Uuid::nil(), // Will be updated when needed
            cli_client_id,
            web_connection_id: Some(web_connection_id),
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
            web_connection_id: s.web_connection_id,
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

    // Message routing
    pub async fn send_to_cli(&self, cli_id: &Uuid, msg: ServerToCli) -> bool {
        if let Some(sender) = self.cli_senders.get(cli_id) {
            if sender.send(msg).await.is_ok() {
                return true;
            }
        }
        false
    }

    pub async fn send_to_web(&self, connection_id: &Uuid, msg: ServerToWeb) -> bool {
        if let Some(sender) = self.web_senders.get(connection_id) {
            if sender.send(msg).await.is_ok() {
                return true;
            }
        }
        false
    }

    pub async fn route_to_cli(&self, session_id: &Uuid, msg: ServerToCli) -> bool {
        if let Some(session) = self.sessions.get(session_id) {
            if let Some(cli_id) = session.cli_client_id {
                let cli_exists = self.cli_senders.contains_key(&cli_id);
                tracing::debug!(
                    "route_to_cli: session {} -> cli {} (cli exists in senders: {})",
                    session_id, cli_id, cli_exists
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
            if let Some(web_id) = session.web_connection_id {
                tracing::debug!("Routing message to web client {} for session {}", web_id, session_id);
                return self.send_to_web(&web_id, msg).await;
            } else {
                tracing::debug!("No web client attached to session {}", session_id);
            }
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
                self.cli_users.get(entry.key()).map(|u| *u == *user_id).unwrap_or(false)
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
            let sender = entry.value().clone();
            let msg_clone = msg.clone();
            tokio::spawn(async move {
                let _ = sender.send(msg_clone).await;
            });
        }
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}
