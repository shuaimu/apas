//! Why a configured terminal tab has no process, so the server is told.
//!
//! A terminal launch that failed used to be logged and written to the local
//! status channel, which never reaches the server. The tab then appeared in
//! the pane list with no terminal state at all: an OMP tab on a Mac without
//! OMP showed an empty terminal, "state unknown", forever. Every failure now
//! goes through [`report_failure`], which records the reason and reports the
//! tab as an exited terminal with it, the same frames an ended process sends.
//! A reconnect reports the recorded reason again (`terminal_state_reports`)
//! instead of a generic "terminal process unavailable".
//!
//! Process-wide and keyed by session: projects run inside one process, and
//! this avoids threading another table through every launch path.

use shared::{CliToServer, TerminalLifecycle};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

/// What is known about a configured terminal tab that has no process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchStatus {
    /// Its provider is being installed. The installer's progress is shown as
    /// a terminal of its own, `instance_id`, which the real process replaces.
    Installing { instance_id: Uuid, status: String },
    /// The launch failed, for this reason.
    Failed(String),
}

fn table() -> &'static Mutex<HashMap<(Uuid, u32), LaunchStatus>> {
    static TABLE: OnceLock<Mutex<HashMap<(Uuid, u32), LaunchStatus>>> = OnceLock::new();
    TABLE.get_or_init(Default::default)
}

pub fn set(session_id: Uuid, pane_id: u32, status: LaunchStatus) {
    if let Ok(mut table) = table().lock() {
        table.insert((session_id, pane_id), status);
    }
}

pub fn get(session_id: Uuid, pane_id: u32) -> Option<LaunchStatus> {
    table().lock().ok()?.get(&(session_id, pane_id)).cloned()
}

/// A process now exists (or the tab is gone): nothing is owed any more.
pub fn clear(session_id: Uuid, pane_id: u32) {
    if let Ok(mut table) = table().lock() {
        table.remove(&(session_id, pane_id));
    }
}

/// The launch errors are written for the local status line, in brackets.
/// The web shows the status inside "Process ended (…)", so drop them.
pub fn status_text(error: &str) -> String {
    let trimmed = error.trim();
    trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(trimmed)
        .trim()
        .to_string()
}

/// The state a reconnect reports for a tab with no process, when one is known.
pub fn state_report(session_id: Uuid, pane_id: u32) -> Option<CliToServer> {
    Some(match get(session_id, pane_id)? {
        LaunchStatus::Installing {
            instance_id,
            status,
        } => CliToServer::TerminalState {
            session_id,
            pane_id,
            instance_id: Some(instance_id),
            lifecycle: TerminalLifecycle::Running,
            status: Some(status),
            runtime: None,
        },
        LaunchStatus::Failed(status) => CliToServer::TerminalState {
            session_id,
            pane_id,
            instance_id: None,
            lifecycle: TerminalLifecycle::Exited,
            status: Some(status),
            runtime: None,
        },
    })
}

/// The frames that report a tab's terminal as exited for `status`. Instance
/// `None` matches whatever the server holds, as an ended reboot does, unless
/// the installer's own terminal is what ends.
pub fn exited_frames(
    session_id: Uuid,
    pane_id: u32,
    instance_id: Option<Uuid>,
    status: &str,
) -> [CliToServer; 2] {
    [
        CliToServer::TerminalState {
            session_id,
            pane_id,
            instance_id,
            lifecycle: TerminalLifecycle::Exited,
            status: Some(status.to_string()),
            runtime: None,
        },
        CliToServer::TerminalExited {
            session_id,
            pane_id,
            instance_id,
            status: Some(status.to_string()),
        },
    ]
}

/// Record a failed launch and tell the server, from a thread that may block
/// (not an async task). `error` is the bracketed launch error.
pub fn report_failure(
    server_tx: &tokio::sync::mpsc::Sender<CliToServer>,
    session_id: Uuid,
    pane_id: u32,
    instance_id: Option<Uuid>,
    error: &str,
) -> String {
    let status = status_text(error);
    set(session_id, pane_id, LaunchStatus::Failed(status.clone()));
    for frame in exited_frames(session_id, pane_id, instance_id, &status) {
        let _ = server_tx.blocking_send(frame);
    }
    // The conversation view says it too, as other launch refusals do.
    let _ = server_tx.blocking_send(CliToServer::Output {
        session_id,
        data: format!("[{status}]"),
        output_type: shared::OutputType::System,
        pane_type: None,
        pane_id: Some(pane_id),
    });
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_launch_is_reported_as_an_exited_terminal_with_its_reason() {
        let session_id = Uuid::new_v4();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let status = report_failure(
            &tx,
            session_id,
            655,
            None,
            "[Error spawning OMP binary 'omp': executable not found. Configure with: apas config set omp_path <path>]",
        );
        assert_eq!(
            status,
            "Error spawning OMP binary 'omp': executable not found. Configure with: apas config set omp_path <path>"
        );
        match rx.try_recv().unwrap() {
            CliToServer::TerminalState {
                pane_id,
                lifecycle,
                status: Some(sent),
                instance_id: None,
                ..
            } => {
                assert_eq!(pane_id, 655);
                assert_eq!(lifecycle, TerminalLifecycle::Exited);
                assert_eq!(sent, status);
            }
            other => panic!("unexpected frame {other:?}"),
        }
        assert!(matches!(
            rx.try_recv().unwrap(),
            CliToServer::TerminalExited {
                pane_id: 655,
                status: Some(_),
                ..
            }
        ));
        assert!(matches!(
            rx.try_recv().unwrap(),
            CliToServer::Output {
                pane_id: Some(655),
                ..
            }
        ));
        // A reconnect says the same.
        assert!(matches!(
            state_report(session_id, 655),
            Some(CliToServer::TerminalState {
                lifecycle: TerminalLifecycle::Exited,
                status: Some(ref sent),
                ..
            }) if *sent == status
        ));
        clear(session_id, 655);
        assert!(state_report(session_id, 655).is_none());
    }

    #[test]
    fn an_install_in_progress_is_a_running_terminal_of_the_installer() {
        let session_id = Uuid::new_v4();
        let instance_id = Uuid::new_v4();
        set(
            session_id,
            3,
            LaunchStatus::Installing {
                instance_id,
                status: "Installing OMP".into(),
            },
        );
        assert!(matches!(
            state_report(session_id, 3),
            Some(CliToServer::TerminalState {
                lifecycle: TerminalLifecycle::Running,
                instance_id: Some(id),
                ..
            }) if id == instance_id
        ));
        assert!(
            state_report(Uuid::new_v4(), 3).is_none(),
            "keyed by session"
        );
    }

    #[test]
    fn status_text_drops_only_the_outer_brackets() {
        assert_eq!(status_text("[a [b] c]"), "a [b] c");
        assert_eq!(status_text("plain"), "plain");
    }
}
