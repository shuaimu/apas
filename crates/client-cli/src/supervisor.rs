// The runtime identity and protocol land before the socket server that uses
// them, so every item here is unused until the next increment wires it. The
// allow is scoped to this module and should be removed with that wiring.
#![allow(dead_code)]

//! The resident per-host supervisor's runtime identity and control protocol.
//!
//! A host has one supervisor — the daemon — and it is the authority on which
//! projects are running there. Before this, the daemon and its project CLIs
//! could not see each other: it located them by matching a `-d <path>`
//! substring in another process's `/proc` command line, which is why running
//! state was inferred rather than known, and why nothing stopped a person
//! typing `apas` in a directory the daemon was already running.
//!
//! The runtime discipline is deliberately the same as `pane_host`: an
//! owner-only directory under the host-local runtime root, a descriptor
//! carrying identity but never a secret, and a random credential in its own
//! `0600` file. That pattern is already proven here, and a second convention
//! would be one more thing to get subtly wrong.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::pane_host::{ensure_private_dir, random_credential, write_private};

/// Where the supervisor's socket, descriptor, and credential live.
///
/// A sibling of the pane hosts' `ph` root rather than a child: pane hosts are
/// per project, this is per host, and nesting it under one project's directory
/// would make its lifetime accidentally depend on that project's.
#[derive(Debug, Clone)]
pub struct SupervisorPaths {
    pub dir: PathBuf,
    pub descriptor: PathBuf,
    pub credential: PathBuf,
    pub socket: PathBuf,
}

pub fn supervisor_paths() -> Result<SupervisorPaths> {
    let dir = crate::config::Config::runtime_dir()?.join("sup");
    ensure_private_dir(&dir)?;
    Ok(SupervisorPaths {
        descriptor: dir.join("runtime.json"),
        credential: dir.join("credential"),
        socket: dir.join("s.sock"),
        dir,
    })
}

/// Where a worker's attach socket and credential live, keyed by project.
///
/// The worker creates these itself rather than receiving them from the
/// supervisor, so a project started by any means — the supervisor, a person,
/// or a leftover script — is attachable on the same terms. A worker from
/// before this existed simply has no such directory, which is exactly what
/// `worker_socket: None` reports.
pub fn worker_paths(project_id: Uuid) -> Result<(PathBuf, PathBuf)> {
    let dir = crate::config::Config::runtime_dir()?
        .join("sup")
        .join("w")
        .join(&project_id.simple().to_string()[..12]);
    ensure_private_dir(&dir)?;
    Ok((dir.join("w.sock"), dir.join("credential")))
}

/// Non-secret identity of the running supervisor. Never carries the
/// credential — that is a separate owner-only file, the same split pane hosts
/// use.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SupervisorDescriptor {
    pub pid: u32,
    pub version: String,
    pub socket: PathBuf,
    pub started_at: String,
}

/// Requests a controller makes of the supervisor.
///
/// `EnsureProject` is the one that removes the duplicate-CLI foot-gun: asking
/// for a project is how you get it, so there is no path that starts a second
/// worker for one that is already running.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControllerToSupervisor {
    /// Authenticate this connection. Sent first; anything else before it is
    /// refused.
    Hello { credential: String },
    /// Start the project at this path if it is not running here, then report
    /// how to reach it. Idempotent by design.
    EnsureProject { path: PathBuf },
    /// Every project this host is running.
    ListProjects,
    /// Stop a project's worker.
    StopProject { project_id: Uuid },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SupervisorToController {
    Welcome {
        version: String,
    },
    /// The project is running here and can be attached to at `worker_socket`.
    ProjectRunning {
        project_id: Uuid,
        path: PathBuf,
        worker_socket: Option<PathBuf>,
        /// True when this request is what started it, false when it was
        /// already running. The distinction is the whole point of the call.
        started: bool,
    },
    Projects {
        projects: Vec<SupervisedProject>,
    },
    Stopped {
        project_id: Uuid,
    },
    Error {
        message: String,
    },
}

/// What an attaching controller sends its worker.
///
/// One message, because the TUI is read-only: `App` consumes `output_rx` and
/// `command_rx` and nothing else, so there is no input to carry back.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControllerToWorker {
    Attach { credential: String },
}

/// What a worker streams to each attached controller.
///
/// `Snapshot` first so a controller attaching to a project that has been
/// running for hours starts with the panes that exist, not with whatever
/// happens to be said next.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerToController {
    Snapshot {
        tabs: Vec<AttachedTab>,
    },
    Output(crate::tui::PaneOutput),
    Command(crate::tui::TuiCommand),
    /// The worker is going away. A reboot replaces the worker, and an
    /// attachment to the old one cannot silently keep rendering a dead
    /// process — it says so and ends.
    Ending {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttachedTab {
    pub pane_id: u32,
    pub label: String,
    pub mode: shared::PaneMode,
}

/// One project as the supervisor knows it. This is the answer to "is it
/// running here" — not a `/proc` match, which could disagree with it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SupervisedProject {
    pub project_id: Uuid,
    pub path: PathBuf,
    /// Where an attaching controller reaches this worker. `None` until the
    /// worker listens — an adopted worker from before this existed has no
    /// socket, and claiming one it does not have would be worse than saying
    /// so.
    pub worker_socket: Option<PathBuf>,
    /// Set when the supervisor started this worker itself. An adopted worker
    /// has none, which is the honest representation of "it was here before
    /// me".
    pub pid: Option<u32>,
    pub adopted: bool,
}

pub fn write_descriptor(paths: &SupervisorPaths, descriptor: &SupervisorDescriptor) -> Result<()> {
    let encoded = serde_json::to_vec_pretty(descriptor).context("encode supervisor descriptor")?;
    write_private(&paths.descriptor, &encoded)
}

pub fn read_descriptor(paths: &SupervisorPaths) -> Option<SupervisorDescriptor> {
    let raw = std::fs::read(&paths.descriptor).ok()?;
    serde_json::from_slice(&raw).ok()
}

pub fn write_credential(paths: &SupervisorPaths) -> Result<String> {
    let credential = random_credential()?;
    write_private(&paths.credential, credential.as_bytes())?;
    Ok(credential)
}

pub fn read_credential(paths: &SupervisorPaths) -> Result<String> {
    let raw = std::fs::read_to_string(&paths.credential)
        .with_context(|| format!("read {}", paths.credential.display()))?;
    Ok(raw.trim().to_string())
}

/// Whether a descriptor describes a supervisor that is still alive.
///
/// A stale descriptor is the normal case after a crash or a kill: the file
/// outlives the process. Checking the pid is what keeps a new supervisor from
/// refusing to start because of a dead one's leftovers.
pub fn descriptor_is_live(descriptor: &SupervisorDescriptor) -> bool {
    Path::new(&format!("/proc/{}", descriptor.pid)).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_descriptor_never_carries_the_credential() {
        // The split is the point: identity is readable state, the secret is a
        // separate owner-only file.
        let descriptor = SupervisorDescriptor {
            pid: 42,
            version: "26.08.1".into(),
            socket: PathBuf::from("/run/apas/sup/s.sock"),
            started_at: "2026-08-15T00:00:00Z".into(),
        };
        let encoded = serde_json::to_string(&descriptor).unwrap();
        assert!(!encoded.contains("credential"));
    }

    #[test]
    fn control_messages_round_trip() {
        let ensure = ControllerToSupervisor::EnsureProject {
            path: PathBuf::from("/work/project"),
        };
        let raw = serde_json::to_string(&ensure).unwrap();
        assert_eq!(
            serde_json::from_str::<ControllerToSupervisor>(&raw).unwrap(),
            ensure
        );

        let project_id = Uuid::new_v4();
        let running = SupervisorToController::ProjectRunning {
            project_id,
            path: PathBuf::from("/work/project"),
            worker_socket: Some(PathBuf::from("/run/apas/sup/w.sock")),
            started: false,
        };
        let raw = serde_json::to_string(&running).unwrap();
        assert_eq!(
            serde_json::from_str::<SupervisorToController>(&raw).unwrap(),
            running
        );
    }

    #[test]
    fn a_dead_supervisors_descriptor_is_not_live() {
        // Pid 0 never names a live process, so a leftover descriptor cannot
        // stop a new supervisor from starting.
        let descriptor = SupervisorDescriptor {
            pid: 0,
            version: "26.08.1".into(),
            socket: PathBuf::from("/run/apas/sup/s.sock"),
            started_at: "2026-08-15T00:00:00Z".into(),
        };
        assert!(!descriptor_is_live(&descriptor));

        let live = SupervisorDescriptor {
            pid: std::process::id(),
            ..descriptor
        };
        assert!(descriptor_is_live(&live));
    }
}
