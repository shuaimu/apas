use std::collections::HashSet;

use serde::Serialize;

use super::SessionManager;

#[derive(Default, Serialize)]
pub(crate) struct Activity {
    online_users: usize,
    viewer_connections: usize,
    connected_sessions: usize,
    working_sessions: usize,
    connected_daemons: usize,
}

impl SessionManager {
    pub(crate) fn activity(&self) -> Activity {
        let mut users = HashSet::new();
        let mut activity = Activity::default();
        for connection in self.web_users.iter() {
            users.insert(*connection.value());
            activity.viewer_connections += 1;
        }
        activity.online_users = users.len();
        activity.connected_daemons = self.daemon_senders.len();

        // Several machine instances can run the same project session.
        let session_ids: HashSet<_> = self
            .get_online_cli_ids()
            .iter()
            .filter_map(|id| self.get_cli_active_session(id))
            .collect();
        for id in session_ids {
            if self.is_session_active(&id) {
                activity.connected_sessions += 1;
                if self
                    .get_pane_statuses(&id)
                    .iter()
                    .any(|(_, _, status)| shared::pane_status_is_working(status))
                {
                    activity.working_sessions += 1;
                }
            }
        }
        activity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::PaneType;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    #[tokio::test]
    async fn server_load_counts_accounts_and_working_sessions_once() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let viewers = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        for viewer in viewers {
            manager.register_web(viewer, mpsc::channel(16).0);
        }
        manager.set_web_user(viewers[0], user);
        manager.set_web_user(viewers[1], user);
        let cli = Uuid::new_v4();
        let session = Uuid::new_v4();
        manager.register_cli(cli, user, mpsc::channel(16).0, None);
        manager.create_cli_session(session, cli, None, None);
        manager.set_pane_status(
            &session,
            PaneType::Interactive,
            1,
            Some(shared::PANE_STATUS_PENDING_ANSWER.into()),
        );
        assert_eq!(manager.activity().working_sessions, 0);
        for pane in [1, 2] {
            manager.set_pane_status(
                &session,
                PaneType::Interactive,
                pane,
                Some("Thinking".into()),
            );
        }
        let activity = manager.activity();
        assert_eq!(activity.online_users, 1);
        assert_eq!(activity.viewer_connections, 2);
        assert_eq!(activity.connected_sessions, 1);
        assert_eq!(activity.working_sessions, 1);
        manager.unregister_web(&viewers[0]);
        assert_eq!(manager.activity().online_users, 1);
        manager.unregister_web(&viewers[1]);
        assert_eq!(manager.activity().online_users, 0);
        manager.unregister_cli(&cli);
        assert_eq!(manager.activity().connected_sessions, 0);
        assert_eq!(manager.activity().working_sessions, 0);
    }
}
