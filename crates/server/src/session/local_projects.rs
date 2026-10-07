use super::SessionManager;
use shared::{LocalProjectRegistrationResult, MachineProjectInfo, ServerToDaemon, ServerToWeb};
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct PendingLocalProjectRegistration {
    pub connection_id: Uuid,
    pub user_id: Uuid,
    pub machine_id: Uuid,
    pub request_id: String,
    pub daemon_sender: mpsc::Sender<ServerToDaemon>,
}

impl SessionManager {
    pub(crate) fn daemon_connection(
        &self,
        machine_id: &Uuid,
    ) -> Option<mpsc::Sender<ServerToDaemon>> {
        self.daemon_senders
            .get(machine_id)
            .map(|sender| sender.clone())
    }

    pub(crate) fn is_daemon_connection(
        &self,
        machine_id: &Uuid,
        sender: &mpsc::Sender<ServerToDaemon>,
    ) -> bool {
        self.daemon_senders
            .get(machine_id)
            .is_some_and(|current| current.same_channel(sender))
    }

    pub(crate) fn begin_local_project_registration(
        &self,
        connection_id: Uuid,
        user_id: Uuid,
        machine_id: Uuid,
        request_id: String,
    ) -> Result<(String, PendingLocalProjectRegistration), &'static str> {
        if self.get_web_user(&connection_id) != Some(user_id)
            || self.daemon_owner(&machine_id) != Some(user_id)
        {
            return Err("Only the authenticated machine owner can register a local folder");
        }
        let daemon_sender = self
            .daemon_connection(&machine_id)
            .filter(|sender| !sender.is_closed())
            .ok_or("The selected machine is offline; reconnect it and retry manually")?;
        if self.local_project_requests.iter().any(|pending| {
            pending.connection_id == connection_id && pending.request_id == request_id
        }) {
            return Err("This registration request is already pending");
        }
        // Never forward a browser-supplied ID: equal IDs on different sockets,
        // or a manual retry after reconnect, must not share a daemon result.
        let wire_id = Uuid::new_v4().to_string();
        let pending = PendingLocalProjectRegistration {
            connection_id,
            user_id,
            machine_id,
            request_id,
            daemon_sender,
        };
        self.local_project_requests
            .insert(wire_id.clone(), pending.clone());
        Ok((wire_id, pending))
    }

    pub(crate) fn pending_local_project_registration(
        &self,
        wire_id: &str,
        machine_id: &Uuid,
    ) -> Option<PendingLocalProjectRegistration> {
        let pending = self.local_project_requests.get(wire_id)?;
        (pending.machine_id == *machine_id && self.local_project_registration_is_current(&pending))
            .then(|| pending.clone())
    }

    pub(crate) fn local_project_registration_is_current(
        &self,
        pending: &PendingLocalProjectRegistration,
    ) -> bool {
        self.get_web_user(&pending.connection_id) == Some(pending.user_id)
            && self.daemon_owner(&pending.machine_id) == Some(pending.user_id)
            && self.is_daemon_connection(&pending.machine_id, &pending.daemon_sender)
    }

    /// Return the daemon's actual inventory, not the UI's session-enriched view.
    pub(crate) fn registered_local_project(
        &self,
        machine_id: &Uuid,
        project_id: &str,
        path: &str,
    ) -> Option<MachineProjectInfo> {
        self.machine_projects
            .get(machine_id)?
            .iter()
            .find(|project| project.project_id == project_id && project.path == path)
            .cloned()
    }

    pub(crate) async fn complete_local_project_registration(
        &self,
        wire_id: &str,
        result: LocalProjectRegistrationResult,
    ) {
        let pending = self.local_project_requests.remove(wire_id);
        if let Some((_, pending)) = pending {
            if self.get_web_user(&pending.connection_id) == Some(pending.user_id) {
                self.send_to_web(
                    &pending.connection_id,
                    ServerToWeb::LocalProjectRegistered {
                        machine_id: pending.machine_id,
                        request_id: pending.request_id,
                        result,
                    },
                )
                .await;
            }
        }
    }

    pub(super) fn clear_local_project_requests_for_web(&self, connection_id: &Uuid) {
        self.local_project_requests
            .retain(|_, pending| pending.connection_id != *connection_id);
    }

    pub(super) fn fail_local_project_requests_for_machine(&self, machine_id: &Uuid) {
        let ids: Vec<_> = self
            .local_project_requests
            .iter()
            .filter(|pending| pending.machine_id == *machine_id)
            .map(|pending| pending.key().clone())
            .collect();
        for id in ids {
            let Some((_, pending)) = self.local_project_requests.remove(&id) else {
                continue;
            };
            if self.get_web_user(&pending.connection_id) != Some(pending.user_id) {
                continue;
            }
            let Some(sender) = self
                .web_senders
                .get(&pending.connection_id)
                .map(|s| s.clone())
            else {
                continue;
            };
            let message = ServerToWeb::LocalProjectRegistered {
                machine_id: pending.machine_id,
                request_id: pending.request_id,
                result: LocalProjectRegistrationResult::Failed {
                    error: "The machine disconnected before registration was confirmed. Files were retained; reconnect and retry manually".to_string(),
                },
            };
            // Disconnect cleanup is synchronous. Do not silently drop the
            // terminal result merely because an inventory update filled the queue.
            if let Err(mpsc::error::TrySendError::Full(message)) = sender.try_send(message) {
                tokio::spawn(async move {
                    let _ = sender.send(message).await;
                });
            }
        }
    }
}
