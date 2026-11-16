//! What the server holds in memory for a project's open state: the lock
//! that applies an open, a close and a follow step one at a time, what
//! became of the last start or stop it asked of each machine, and each
//! machine's part in the project as viewers are told it.
//!
//! The recorded state itself is durable and lives in the database
//! (`db/open_state.rs`). Nothing here survives a server restart: a lock is
//! only needed while a request runs, and an outcome is unknown again until
//! that machine's daemon registers or someone opens the project.
//!
//! A machine's part is derived, never stored: [`SessionManager::project_open_view`]
//! reads the roster, the machines' inventories, the attached instances, the
//! starts in flight and the outcomes at the moment it is asked. What it
//! cannot read without the database (the recorded state, who set it, and the
//! roster of a session that is not in memory) it takes from the project's
//! [`OpenStateBasis`], which `crate::open_state::refresh_basis` keeps.

use super::{normalize_machine_hostname, SessionManager};
use dashmap::DashMap;
use shared::{
    MachinePart, MachineProjectInfo, MachineWithProjects, OpenStateSetKind, ProjectHostSummary,
    ProjectMachinePart, ProjectOpenState, ServerToWeb,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, OwnedMutexGuard};
use uuid::Uuid;

/// What became of the last start or stop the server asked of one machine
/// for one project, or of the check that kept it from asking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineOutcome {
    /// `StartProjectCli` was sent and nothing has answered it yet.
    /// `request_id` is the id it carried; a daemon without multi-instance
    /// support is sent none and never answers. `asked_at` is for the time
    /// limit after which an unanswered start reads as not started.
    StartAsked {
        request_id: Option<Uuid>,
        asked_at: Instant,
    },
    /// The machine's daemon answered that it started the project, and the
    /// machine has not been seen running it yet: its inventory does not
    /// report it and no instance of it is attached. `asked_at` is when the
    /// start was sent, for the same time limit.
    StartAccepted { asked_at: Instant },
    /// The machine was seen running the project: its daemon reported it, or
    /// its instance attached.
    Started,
    /// The machine was not started, or its start failed, for this reason:
    /// the sentence the refusing check or the daemon gave.
    NotStarted(String),
    /// A close left the machine running, for this reason.
    NotStopped(String),
}

impl MachineOutcome {
    /// A start was sent and the machine has not been seen running the
    /// project since, however long ago that was. A close stops such a
    /// machine too.
    pub fn is_start_pending(&self) -> bool {
        matches!(self, Self::StartAsked { .. } | Self::StartAccepted { .. })
    }

    /// [`Self::is_start_pending`], and the start was sent less than `limit`
    /// ago: the machine still reads as starting.
    pub fn is_start_pending_within(&self, limit: Duration) -> bool {
        self.pending_since()
            .is_some_and(|asked_at| asked_at.elapsed() < limit)
    }

    /// When the start still pending was sent.
    fn pending_since(&self) -> Option<Instant> {
        match self {
            Self::StartAsked { asked_at, .. } | Self::StartAccepted { asked_at } => Some(*asked_at),
            _ => None,
        }
    }
}

/// One machine that owns tabs of a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TabOwner {
    /// `None` for a host the server knows only by name: untagged tabs are
    /// attributed to a hostname, and no machine that lists the project is
    /// known under it. Nothing can be sent to such a host.
    pub machine_id: Option<Uuid>,
    /// For display only.
    pub hostname: String,
    /// Not in the roster: the machine an open named, which counts as owning
    /// the project's tabs for as long as no machine owns one.
    pub stand_in: bool,
}

/// The machine and host a roster records for one tab: all that deciding who
/// owns the project's tabs reads of it.
pub(crate) type TabPlace = (Option<Uuid>, Option<String>);

/// What deriving a project's parts needs and cannot read without the
/// database or the disk. Kept per project by `crate::open_state::refresh_basis`,
/// which every path that records the state calls, as do a daemon's
/// registration, a viewer's attach and an instance's `SessionStart`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct OpenStateBasis {
    /// `None` is "not recorded".
    pub state: Option<ProjectOpenState>,
    /// The email of the account that set the state, or the words for the
    /// system administrator.
    pub set_by: Option<String>,
    pub set_kind: Option<OpenStateSetKind>,
    pub set_at: Option<String>,
    /// The machine an open named, which owns the project's tabs while no
    /// machine owns one.
    pub stand_in: Option<Uuid>,
    /// The accounts whose machines are this project's: its owner and every
    /// account whose cluster hosts it. Another account's checkout can carry
    /// the same project id; its machine is not one of the project's, is not
    /// listed among its machines and is told nothing of its state.
    pub hosts: HashSet<Uuid>,
    /// The roster of each of the project's sessions as last read for it, for
    /// a session whose roster is not in memory. A session absent here and
    /// not in memory has no known roster.
    pub rosters: HashMap<Uuid, Vec<TabPlace>>,
}

/// A project's open state and each machine's part in it, as derived at one
/// moment: the content of [`ServerToWeb::ProjectOpenState`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectOpenView {
    pub state: Option<ProjectOpenState>,
    pub set_by: Option<String>,
    pub set_kind: Option<OpenStateSetKind>,
    pub set_at: Option<String>,
    /// Sorted by host name.
    pub machines: Vec<ProjectMachinePart>,
}

impl ProjectOpenView {
    pub(crate) fn frame(&self, project_id: &str) -> ServerToWeb {
        ServerToWeb::ProjectOpenState {
            project_id: project_id.to_string(),
            state: self.state,
            set_by: self.set_by.clone(),
            set_kind: self.set_kind,
            set_at: self.set_at.clone(),
            machines: self.machines.clone(),
        }
    }
}

/// Why a machine asked to start a project reads as not started once the
/// time limit has passed without an answer from its daemon.
pub(crate) fn start_unanswered_reason(host: &str) -> String {
    format!("{host} did not answer the request to start this project.")
}

/// Why a tab waiting for `host`'s instance fails when the project is closed
/// or stopped before that instance attached.
pub(crate) fn closed_before_start_reason(host: &str) -> String {
    format!("The project was closed before it started on {host}.")
}

/// Why a machine reads as not started when its daemon answered that it
/// started the project and the machine was never seen running it.
pub(crate) fn start_not_seen_reason(host: &str) -> String {
    format!("{host} answered that it started this project, but is not running it.")
}

#[derive(Default)]
pub(super) struct OpenStateRuntime {
    /// One exclusive lock per project id.
    locks: DashMap<String, Arc<tokio::sync::Mutex<()>>>,
    /// By (project id, machine id).
    outcomes: Mutex<HashMap<(String, Uuid), MachineOutcome>>,
    /// By project id.
    bases: DashMap<String, OpenStateBasis>,
    /// One lock per project id for refreshing its basis, so two refreshes
    /// never store their reads out of order.
    basis_locks: DashMap<String, Arc<tokio::sync::Mutex<()>>>,
    /// What viewers were last told of each project, to tell them only of a
    /// change.
    reported: Mutex<HashMap<String, ProjectOpenView>>,
    /// By account: the projects it holds a role in, among those listed on
    /// machines it does not own, as its last machine list found them.
    readers: DashMap<Uuid, HashSet<String>>,
    /// Run once by the next report, between deriving its view and queuing
    /// it: where a test lets a second report of the project begin.
    #[cfg(test)]
    after_view_derived: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl SessionManager {
    /// The exclusive open-state lock of `project_id`.
    ///
    /// An open, a close, a tab or task request that opens the project, every
    /// other path that records the state, and the follow step at a daemon's
    /// registration hold it from reading the recorded state to sending their
    /// last frame. So a step that read "closed" cannot send its stop after a
    /// later open. The lock orders what the server sends, not what a daemon
    /// has done with it: a machine still reads as running while a stop sent
    /// to it takes effect, which is why an open that changes the record
    /// sends its start to such a machine too.
    /// `AppState::project_operation_guard` is a shared permit and serializes
    /// none of this.
    ///
    /// Take it before `project_operation_guard`, never while holding that
    /// permit: a deletion waiting for the permits would otherwise deadlock
    /// the two.
    pub async fn project_open_state_guard(&self, project_id: &str) -> OwnedMutexGuard<()> {
        let lock = self
            .open_state
            .locks
            .entry(project_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        lock.lock_owned().await
    }

    fn set_machine_outcome(&self, project_id: &str, machine_id: Uuid, outcome: MachineOutcome) {
        self.open_state
            .outcomes
            .lock()
            .expect("open-state outcomes poisoned")
            .insert((project_id.to_string(), machine_id), outcome);
        // A machine's part reads its outcome.
        self.report_project_open_state(project_id);
    }

    /// `StartProjectCli` for `project_id` was sent to `machine_id`, carrying
    /// `request_id` when its daemon answers one.
    pub fn record_machine_start_asked(
        &self,
        project_id: &str,
        machine_id: Uuid,
        request_id: Option<Uuid>,
    ) {
        self.set_machine_outcome(
            project_id,
            machine_id,
            MachineOutcome::StartAsked {
                request_id,
                asked_at: Instant::now(),
            },
        );
    }

    /// `machine_id` runs `project_id`: its daemon reports it, or its instance
    /// is attached.
    pub fn record_machine_started(&self, project_id: &str, machine_id: Uuid) {
        self.set_machine_outcome(project_id, machine_id, MachineOutcome::Started);
    }

    /// `machine_id`'s daemon answered the start that carried `request_id`.
    /// A refusal is kept as the reason the machine is not started. A success
    /// only says the daemon started the project: the machine reads as
    /// starting until it is seen running it, within the same time limit.
    /// False when no start of this machine's still waits for that answer.
    pub fn machine_start_answered(
        &self,
        machine_id: &Uuid,
        request_id: &Uuid,
        project_id: &str,
        success: bool,
        error: Option<String>,
    ) -> bool {
        let key = (project_id.to_string(), *machine_id);
        {
            let mut outcomes = self
                .open_state
                .outcomes
                .lock()
                .expect("open-state outcomes poisoned");
            let Some(MachineOutcome::StartAsked {
                request_id: Some(asked),
                asked_at,
            }) = outcomes.get(&key).cloned()
            else {
                return false;
            };
            if asked != *request_id {
                return false;
            }
            let outcome = if success {
                MachineOutcome::StartAccepted { asked_at }
            } else {
                let host = self
                    .machine_hostname(machine_id)
                    .unwrap_or_else(|| "The machine".to_string());
                MachineOutcome::NotStarted(
                    error
                        .map(|error| error.trim().to_string())
                        .filter(|error| !error.is_empty())
                        .unwrap_or_else(|| format!("{host} did not start this project")),
                )
            };
            outcomes.insert(key, outcome);
        }
        self.report_project_open_state(project_id);
        true
    }

    /// `machine_id` was seen running `project_id`: a start still pending
    /// there is over. Nothing else is touched, so a refusal kept for a
    /// machine that runs the project anyway stays until it is replaced.
    pub(super) fn note_machine_runs(&self, project_id: &str, machine_id: &Uuid) {
        let mut outcomes = self
            .open_state
            .outcomes
            .lock()
            .expect("open-state outcomes poisoned");
        let key = (project_id.to_string(), *machine_id);
        if outcomes
            .get(&key)
            .is_some_and(MachineOutcome::is_start_pending)
        {
            outcomes.insert(key, MachineOutcome::Started);
        }
    }

    /// `machine_id` was not started for `project_id`, or its start failed,
    /// for `reason`.
    pub fn record_machine_not_started(
        &self,
        project_id: &str,
        machine_id: Uuid,
        reason: impl Into<String>,
    ) {
        self.set_machine_outcome(
            project_id,
            machine_id,
            MachineOutcome::NotStarted(reason.into()),
        );
    }

    /// A close of `project_id` left `machine_id` running, for `reason`.
    pub fn record_machine_not_stopped(
        &self,
        project_id: &str,
        machine_id: Uuid,
        reason: impl Into<String>,
    ) {
        self.set_machine_outcome(
            project_id,
            machine_id,
            MachineOutcome::NotStopped(reason.into()),
        );
    }

    /// The project and machine of the unanswered start that carried
    /// `request_id`, for a daemon's `ProjectRuntimeStarted`.
    #[cfg(test)]
    pub fn machine_start_request(&self, request_id: &Uuid) -> Option<(String, Uuid)> {
        self.open_state
            .outcomes
            .lock()
            .expect("open-state outcomes poisoned")
            .iter()
            .find(|(_, outcome)| {
                matches!(
                    outcome,
                    MachineOutcome::StartAsked {
                        request_id: Some(asked),
                        ..
                    } if asked == request_id
                )
            })
            .map(|((project_id, machine_id), _)| (project_id.clone(), *machine_id))
    }

    /// Forget what is held for `machine_id` in `project_id`, as when it was
    /// asked to stop or went offline.
    pub fn clear_machine_outcome(&self, project_id: &str, machine_id: &Uuid) {
        let removed = self
            .open_state
            .outcomes
            .lock()
            .expect("open-state outcomes poisoned")
            .remove(&(project_id.to_string(), *machine_id))
            .is_some();
        if removed {
            self.report_project_open_state(project_id);
        }
    }

    /// Forget every machine's outcome for `project_id`, as when it is closed.
    pub fn clear_project_outcomes(&self, project_id: &str) {
        self.drop_project_outcomes(project_id);
        self.report_project_open_state(project_id);
    }

    fn drop_project_outcomes(&self, project_id: &str) {
        self.open_state
            .outcomes
            .lock()
            .expect("open-state outcomes poisoned")
            .retain(|(project, _), _| project != project_id);
    }

    /// Forget what is held for `machine_id` in every project, and return
    /// those projects: its daemon's connection closed or was replaced, so a
    /// start sent on it will not be answered, and what it refused or failed
    /// is decided again when it registers.
    pub(super) fn drop_machine_outcomes(&self, machine_id: &Uuid) -> Vec<String> {
        let mut projects = Vec::new();
        self.open_state
            .outcomes
            .lock()
            .expect("open-state outcomes poisoned")
            .retain(|(project, machine), _| {
                if machine == machine_id {
                    projects.push(project.clone());
                    false
                } else {
                    true
                }
            });
        projects
    }

    /// Drop everything held for a project that has been deleted: its lock,
    /// its machines' outcomes and what viewers were told of it.
    ///
    /// The lock is dropped only while nobody holds it or waits for it. A
    /// task that took it before this call keeps the same mutex as whoever
    /// asks next; removing it under that task would hand the next one a new
    /// mutex, and the two would run at once.
    pub fn forget_project_open_state(&self, project_id: &str) {
        self.drop_project_outcomes(project_id);
        self.open_state
            .locks
            .remove_if(project_id, |_, lock| Arc::strong_count(lock) == 1);
        self.open_state.bases.remove(project_id);
        self.open_state.basis_locks.remove(project_id);
        self.open_state
            .reported
            .lock()
            .expect("open-state reports poisoned")
            .remove(project_id);
    }

    /// `StopProjectCli` for `project_id` was sent to `machine_id`: a tab
    /// waiting for a start of its instance that the server sent fails now,
    /// with [`closed_before_start_reason`]. Its daemon may already have
    /// answered that the runtime started, so nothing else would end the wait
    /// before its time limit, and a tab request made meanwhile would join
    /// that wait instead of asking the machine to start again.
    pub fn end_tab_starts_for_stop(&self, project_id: &str, machine_id: &Uuid) {
        let host = self
            .machine_hostname(machine_id)
            .map(|host| host.trim().to_string())
            .filter(|host| !host.is_empty())
            .unwrap_or_else(|| "that machine".to_string());
        let reason = closed_before_start_reason(&host);
        for session_id in self.project_session_ids(project_id) {
            self.refuse_instance_start(session_id, *machine_id, &reason);
        }
    }

    pub fn machine_outcome(&self, project_id: &str, machine_id: &Uuid) -> Option<MachineOutcome> {
        self.open_state
            .outcomes
            .lock()
            .expect("open-state outcomes poisoned")
            .get(&(project_id.to_string(), *machine_id))
            .cloned()
    }

    /// Every machine an outcome is held for in `project_id`, sorted by id.
    pub fn project_machine_outcomes(&self, project_id: &str) -> Vec<(Uuid, MachineOutcome)> {
        let mut outcomes: Vec<(Uuid, MachineOutcome)> = self
            .open_state
            .outcomes
            .lock()
            .expect("open-state outcomes poisoned")
            .iter()
            .filter(|((project, _), _)| project == project_id)
            .map(|((_, machine_id), outcome)| (*machine_id, outcome.clone()))
            .collect();
        outcomes.sort_by_key(|(machine_id, _)| *machine_id);
        outcomes
    }
}

impl SessionManager {
    /// The machines that own the tabs at `places`, a project's roster: each
    /// machine a tab is recorded for, and for an untagged tab every machine
    /// listing the project under the host it is attributed to, or that host
    /// by name when none does. While no machine owns a tab, `stand_in` is
    /// the one owner. Sorted, machines the server knows by id first.
    ///
    /// `hosts` are the accounts whose machines are the project's (its owner
    /// and every account whose cluster hosts it). When given, a host name
    /// resolves only to their machines: another account's checkout can carry
    /// the same project id on a machine of the same name, and that machine
    /// owns none of these tabs. A name only such a machine carries stays a
    /// name.
    pub(crate) fn tab_owners_at(
        &self,
        project_id: &str,
        places: &[TabPlace],
        stand_in: Option<Uuid>,
        hosts: Option<&HashSet<Uuid>>,
    ) -> Vec<TabOwner> {
        let mut listing = self.machines_listing_project(project_id);
        if let Some(hosts) = hosts {
            listing.retain(|machine_id| {
                self.daemon_owner(machine_id)
                    .is_some_and(|account| hosts.contains(&account))
            });
        }
        // The name its daemon registered under, else the one the roster holds.
        let host_of = |machine_id: &Uuid, recorded: Option<&str>| {
            self.machine_hostname(machine_id)
                .map(|host| host.trim().to_string())
                .filter(|host| !host.is_empty())
                .or_else(|| {
                    recorded
                        .map(str::trim)
                        .filter(|host| !host.is_empty())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| machine_id.to_string())
        };
        let mut owners: Vec<TabOwner> = Vec::new();
        for (machine_id, hostname) in places {
            let Some(machine_id) = machine_id else {
                continue;
            };
            if !owners
                .iter()
                .any(|owner| owner.machine_id == Some(*machine_id))
            {
                owners.push(TabOwner {
                    machine_id: Some(*machine_id),
                    hostname: host_of(machine_id, hostname.as_deref()),
                    stand_in: false,
                });
            }
        }
        for (_, hostname) in places.iter().filter(|(machine_id, _)| machine_id.is_none()) {
            let Some(host) = hostname
                .as_deref()
                .map(str::trim)
                .filter(|host| !host.is_empty())
            else {
                // A tab no host is known for belongs to no machine.
                continue;
            };
            let normalized = normalize_machine_hostname(host);
            let mut machines: Vec<Uuid> = listing
                .iter()
                .filter(|machine_id| {
                    self.machine_hostname(machine_id)
                        .is_some_and(|known| normalize_machine_hostname(&known) == normalized)
                })
                .copied()
                .collect();
            machines.sort();
            if machines.is_empty() {
                // Known by name only, unless a tagged tab already names a
                // machine on that host.
                if !owners
                    .iter()
                    .any(|owner| normalize_machine_hostname(&owner.hostname) == normalized)
                {
                    owners.push(TabOwner {
                        machine_id: None,
                        hostname: host.to_string(),
                        stand_in: false,
                    });
                }
                continue;
            }
            for machine_id in machines {
                if !owners
                    .iter()
                    .any(|owner| owner.machine_id == Some(machine_id))
                {
                    owners.push(TabOwner {
                        machine_id: Some(machine_id),
                        hostname: host_of(&machine_id, Some(host)),
                        stand_in: false,
                    });
                }
            }
        }
        if owners.is_empty() {
            if let Some(machine_id) = stand_in {
                owners.push(TabOwner {
                    machine_id: Some(machine_id),
                    hostname: host_of(&machine_id, None),
                    stand_in: true,
                });
            }
        }
        owners.sort_by(|a, b| match (a.machine_id, b.machine_id) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.hostname.cmp(&b.hostname),
        });
        owners
    }

    /// The lock under which one project's basis is read and stored; see
    /// `crate::open_state::refresh_basis`. Nothing else is taken while it is
    /// held.
    pub(crate) async fn open_state_basis_guard(&self, project_id: &str) -> OwnedMutexGuard<()> {
        let lock = self
            .open_state
            .basis_locks
            .entry(project_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        lock.lock_owned().await
    }

    /// Keep `basis` for `project_id` and tell viewers what changed with it.
    pub(crate) fn set_open_state_basis(&self, project_id: &str, basis: OpenStateBasis) {
        self.open_state.bases.insert(project_id.to_string(), basis);
        self.report_project_open_state(project_id);
    }

    /// Whether a basis is held for `project_id`.
    pub(crate) fn has_open_state_basis(&self, project_id: &str) -> bool {
        self.open_state.bases.contains_key(project_id)
    }

    /// The projects `machine_id`'s inventory lists.
    pub(crate) fn machine_project_ids(&self, machine_id: &Uuid) -> Vec<String> {
        self.machine_projects
            .get(machine_id)
            .map(|projects| {
                projects
                    .iter()
                    .map(|project| project.project_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The roster of the project's sessions as far as it is known: in memory
    /// where a session holds one, else as last read for the basis. `None`
    /// when no session's roster is known, which is not an empty roster.
    fn project_tab_places(
        &self,
        session_ids: &[Uuid],
        basis: &OpenStateBasis,
    ) -> Option<Vec<TabPlace>> {
        let mut known = false;
        let mut places = Vec::new();
        for session_id in session_ids {
            let cached = self.get_session_panes(session_id);
            if self.pane_roster_seeded(session_id)
                || (!cached.is_empty() && !self.pane_roster_unreadable(session_id))
            {
                known = true;
                places.extend(
                    cached
                        .into_iter()
                        .map(|pane| (pane.machine_id, pane.hostname)),
                );
            } else if let Some(stored) = basis.rosters.get(session_id) {
                known = true;
                places.extend(stored.iter().cloned());
            }
        }
        known.then_some(places)
    }

    /// The project's open state and each machine's part in it, now. `None`
    /// until a basis is held for the project.
    ///
    /// Listed are the machines that own tabs, the machines that run the
    /// project (an instance attached, or a connected daemon of one of the
    /// project's accounts reporting it), the machines being started, and
    /// for an open project the machines of the project's accounts whose
    /// start was refused, failed or was never answered, whether or not they
    /// own a tab and also while the tab owners are unknown.
    /// Each one's part, by the recorded state:
    ///
    /// - **open**: running; else waiting while its daemon is not connected;
    ///   else starting while a start sent to it is within the time limit;
    ///   else not started, with the reason the refusing check or its daemon
    ///   gave, else its daemon's `last_error`, else that it never answered;
    /// - **closed**: still running, with the reason a close left it so when
    ///   one is known; else, for a tab owner, not started with no reason;
    /// - **not recorded**: running; else starting; else, for a tab owner,
    ///   not started, connected or not, since nothing starts it by itself.
    pub(crate) fn project_open_view(&self, project_id: &str) -> Option<ProjectOpenView> {
        let basis = self
            .open_state
            .bases
            .get(project_id)
            .map(|basis| basis.clone())?;
        let session_ids = self.project_session_ids(project_id);
        let mut owners = match self.project_tab_places(&session_ids, &basis) {
            Some(places) => {
                self.tab_owners_at(project_id, &places, basis.stand_in, Some(&basis.hosts))
            }
            None => Vec::new(),
        };
        // An untagged tab is attributed to a host by name, and another
        // account's checkout with the same project id can sit on a machine of
        // that name. That machine is not the project's: the tab's host stays
        // a name here, and nothing of that machine is told.
        for owner in &mut owners {
            let foreign = owner
                .machine_id
                .and_then(|machine_id| self.daemon_owner(&machine_id))
                .is_some_and(|account| !basis.hosts.contains(&account));
            if foreign {
                owner.machine_id = None;
            }
        }
        let mut named = HashSet::new();
        owners.retain(|owner| {
            owner.machine_id.is_some() || named.insert(normalize_machine_hostname(&owner.hostname))
        });
        let attached: HashSet<Uuid> = session_ids
            .iter()
            .flat_map(|session_id| self.session_attachments(session_id))
            .filter_map(|instance| instance.machine_id)
            .collect();
        let reporting: HashSet<Uuid> = self
            .machines_running_project(project_id)
            .into_iter()
            .filter(|machine_id| {
                self.daemon_owner(machine_id)
                    .is_some_and(|owner| basis.hosts.contains(&owner))
            })
            .collect();
        let tab_starts: HashSet<Uuid> = self
            .machines_with_pending_instance_start(&session_ids)
            .into_iter()
            .collect();
        let outcomes: HashMap<Uuid, MachineOutcome> = self
            .project_machine_outcomes(project_id)
            .into_iter()
            .collect();
        let limit = self.instance_start_timeout();
        let starting = |machine_id: &Uuid| {
            tab_starts.contains(machine_id)
                || outcomes
                    .get(machine_id)
                    .and_then(MachineOutcome::pending_since)
                    .is_some_and(|asked_at| asked_at.elapsed() < limit)
        };

        // A machine of the project that an open project's start did not
        // bring up: it was refused or failed, or its start was never
        // answered within the time limit. It is listed with that reason
        // whether or not it owns a tab, which is also how the machine an
        // open named is reported while the project has no roster yet.
        let open = basis.state == Some(ProjectOpenState::Open);
        let failed_start = |machine_id: &Uuid| {
            open && outcomes.get(machine_id).is_some_and(|outcome| {
                matches!(outcome, MachineOutcome::NotStarted(_)) || outcome.is_start_pending()
            }) && self
                .daemon_owner(machine_id)
                .is_some_and(|owner| basis.hosts.contains(&owner))
        };

        // Tab owners first, then every other machine that runs the project,
        // is being started for it, or was not started for an open one.
        let mut listed: Vec<(Option<Uuid>, String, bool)> = owners
            .iter()
            .map(|owner| (owner.machine_id, owner.hostname.clone(), true))
            .collect();
        let mut others: Vec<Uuid> = attached
            .iter()
            .chain(reporting.iter())
            .chain(tab_starts.iter())
            .copied()
            .chain(
                outcomes
                    .keys()
                    .filter(|machine_id| starting(machine_id) || failed_start(machine_id))
                    .copied(),
            )
            .filter(|machine_id| {
                !owners
                    .iter()
                    .any(|owner| owner.machine_id == Some(*machine_id))
            })
            .collect();
        others.sort();
        others.dedup();
        for machine_id in others {
            let host = self
                .machine_hostname(&machine_id)
                .map(|host| host.trim().to_string())
                .filter(|host| !host.is_empty())
                .unwrap_or_else(|| machine_id.to_string());
            listed.push((Some(machine_id), host, false));
        }

        let mut machines = Vec::new();
        for (machine_id, hostname, owns_tabs) in listed {
            let connected = machine_id.is_some_and(|id| self.is_daemon_connected(&id));
            let runs =
                machine_id.is_some_and(|id| attached.contains(&id) || reporting.contains(&id));
            let outcome = machine_id.and_then(|id| outcomes.get(&id));
            let is_starting = connected && machine_id.is_some_and(|id| starting(&id));
            // Why a connected machine that owns tabs is not running them.
            let not_started_reason = || {
                if !connected {
                    return None;
                }
                if let Some(MachineOutcome::NotStarted(reason)) = outcome {
                    return Some(reason.clone());
                }
                let last_error = machine_id.and_then(|id| {
                    self.machine_projects.get(&id).and_then(|projects| {
                        projects
                            .iter()
                            .find(|project| project.project_id == project_id)
                            .and_then(|project| project.last_error.clone())
                    })
                });
                last_error
                    .map(|error| error.trim().to_string())
                    .filter(|error| !error.is_empty())
                    .or_else(|| match outcome {
                        Some(MachineOutcome::StartAsked { .. }) => {
                            Some(start_unanswered_reason(&hostname))
                        }
                        Some(MachineOutcome::StartAccepted { .. }) => {
                            Some(start_not_seen_reason(&hostname))
                        }
                        _ => None,
                    })
            };
            let (part, reason) = match basis.state {
                Some(ProjectOpenState::Closed) => {
                    if runs {
                        let reason = match outcome {
                            Some(MachineOutcome::NotStopped(reason)) => Some(reason.clone()),
                            _ => None,
                        };
                        (MachinePart::StillRunning, reason)
                    } else if owns_tabs {
                        (MachinePart::NotStarted, None)
                    } else {
                        continue;
                    }
                }
                Some(ProjectOpenState::Open) => {
                    if runs {
                        (MachinePart::Running, None)
                    } else if !connected && owns_tabs {
                        (MachinePart::WaitingOffline, None)
                    } else if is_starting {
                        (MachinePart::Starting, None)
                    } else if owns_tabs || machine_id.is_some_and(|id| failed_start(&id)) {
                        (MachinePart::NotStarted, not_started_reason())
                    } else {
                        continue;
                    }
                }
                None => {
                    if runs {
                        (MachinePart::Running, None)
                    } else if is_starting {
                        (MachinePart::Starting, None)
                    } else if owns_tabs {
                        (MachinePart::NotStarted, not_started_reason())
                    } else {
                        continue;
                    }
                }
            };
            machines.push(ProjectMachinePart {
                machine_id,
                hostname,
                part,
                reason,
                owns_tabs,
                connected,
            });
        }
        machines.sort_by(|a, b| {
            normalize_machine_hostname(&a.hostname)
                .cmp(&normalize_machine_hostname(&b.hostname))
                .then_with(|| a.machine_id.cmp(&b.machine_id))
        });
        Some(ProjectOpenView {
            state: basis.state,
            set_by: basis.set_by,
            set_kind: basis.set_kind,
            set_at: basis.set_at,
            machines,
        })
    }

    /// Tell viewers of `project_id` its open state and machines' parts if
    /// they differ from what they were last told: one
    /// [`ServerToWeb::ProjectOpenState`] to every connection attached to one
    /// of the project's sessions that negotiated `project_open_state_v1`,
    /// and a fresh machine list to the owners of the machines that list the
    /// project. Attached connections passed the project's access check when
    /// they attached, and lose their attachment with their access.
    ///
    /// Called after every change a part reads: the recorded state, a
    /// machine's outcome, a daemon's registration, inventory or departure,
    /// an instance attaching or leaving, and the roster.
    pub(crate) fn report_project_open_state(&self, project_id: &str) {
        self.report_open_state_change(project_id, true);
    }

    /// [`Self::report_project_open_state`]; `machine_lists` false leaves the
    /// machine lists to a caller that is about to send them itself.
    ///
    /// Reports are made one at a time, from deriving the view to queuing
    /// its frames: two of one project that ran side by side could queue the
    /// older view last, and nothing would correct it, since the next report
    /// would find the newer view already told. A frame a viewer's full queue
    /// did not take is not counted as told either: the next report sends the
    /// complete frame again.
    pub(super) fn report_open_state_change(&self, project_id: &str, machine_lists: bool) {
        {
            // Held from deriving the view to queuing the last frame. Deriving
            // takes only short synchronous locks and never this one, and no
            // caller holds one of those when it reports.
            let mut reported = self
                .open_state
                .reported
                .lock()
                .expect("open-state reports poisoned");
            let Some(view) = self.project_open_view(project_id) else {
                return;
            };
            #[cfg(test)]
            self.run_after_view_derived();
            if reported.get(project_id) == Some(&view) {
                return;
            }
            reported.insert(project_id.to_string(), view.clone());
            let mut viewers: Vec<Uuid> = Vec::new();
            for session_id in self.project_session_ids(project_id) {
                if let Some(session) = self.sessions.get(&session_id) {
                    for connection_id in &session.web_connection_ids {
                        if !viewers.contains(connection_id) {
                            viewers.push(*connection_id);
                        }
                    }
                }
            }
            let frame = view.frame(project_id);
            let mut dropped = false;
            for connection_id in viewers {
                if !self
                    .web_supports_capability(&connection_id, shared::PROJECT_OPEN_STATE_CAPABILITY)
                {
                    continue;
                }
                let sender = self
                    .web_senders
                    .get(&connection_id)
                    .map(|sender| sender.clone());
                if let Some(sender) = sender {
                    if let Err(mpsc::error::TrySendError::Full(_)) = sender.try_send(frame.clone())
                    {
                        dropped = true;
                    }
                }
            }
            if dropped {
                // That viewer was not told. Forgetting what was told makes
                // the next report send the whole frame again, to everyone.
                reported.remove(project_id);
            }
        }
        if machine_lists {
            let mut owners: Vec<Uuid> = self
                .machines_listing_project(project_id)
                .iter()
                .filter_map(|machine_id| self.daemon_owner(machine_id))
                .collect();
            owners.sort();
            owners.dedup();
            for owner in owners {
                self.broadcast_machines_update_for_user(&owner);
            }
        }
    }

    /// Have the next report run `hook` once it has derived its view and
    /// before it queues it.
    #[cfg(test)]
    pub(crate) fn after_next_view_derived(&self, hook: impl FnOnce() + Send + 'static) {
        *self
            .open_state
            .after_view_derived
            .lock()
            .expect("report hook poisoned") = Some(Box::new(hook));
    }

    #[cfg(test)]
    fn run_after_view_derived(&self) {
        let hook = self
            .open_state
            .after_view_derived
            .lock()
            .expect("report hook poisoned")
            .take();
        if let Some(hook) = hook {
            hook();
        }
    }

    /// [`Self::report_project_open_state`] for the project of `session_id`.
    pub(crate) fn report_session_open_state(&self, session_id: &Uuid) {
        let project_id = self
            .project_for_session(session_id)
            .unwrap_or_else(|| session_id.to_string());
        self.report_project_open_state(&project_id);
    }

    /// The projects `viewer` holds a role in among those listed on machines
    /// it does not own, as its machine list just found them. Entries of
    /// other projects on those machines carry nothing of the open state.
    pub(crate) fn set_open_state_readable_projects(&self, viewer: Uuid, projects: HashSet<String>) {
        self.open_state.readers.insert(viewer, projects);
    }

    /// Remove from a project entry everything the server fills in about the
    /// open state. A daemon reports none of it, and whatever one sent is
    /// discarded here.
    pub(crate) fn clear_open_state_fields(project: &mut MachineProjectInfo) {
        project.open_state = None;
        project.part = None;
        project.part_reason = None;
        project.other_hosts.clear();
    }

    /// Fill the server's open-state fields into one project entry of
    /// `machine_id` as `viewer` may see them: the recorded state, this
    /// machine's part with its reason, and the other machines that own tabs
    /// of the project or run it. Whatever the entry held is overwritten, and
    /// cleared when nothing may be shown: the machine is not one of the
    /// project's (another account's checkout with the same id), or `viewer`
    /// does not own the machine and holds no role in the project.
    pub(crate) fn fill_project_open_state(
        &self,
        viewer: &Uuid,
        machine_id: &Uuid,
        project: &mut MachineProjectInfo,
    ) {
        self.fill_project_open_state_with(viewer, machine_id, project, &mut HashMap::new());
    }

    fn fill_project_open_state_with(
        &self,
        viewer: &Uuid,
        machine_id: &Uuid,
        project: &mut MachineProjectInfo,
        views: &mut HashMap<String, Option<ProjectOpenView>>,
    ) {
        Self::clear_open_state_fields(project);
        let Some(owner) = self.daemon_owner(machine_id) else {
            return;
        };
        let of_project = self
            .open_state
            .bases
            .get(&project.project_id)
            .is_some_and(|basis| basis.hosts.contains(&owner));
        if !of_project {
            return;
        }
        if owner != *viewer
            && !self
                .open_state
                .readers
                .get(viewer)
                .is_some_and(|readable| readable.contains(&project.project_id))
        {
            return;
        }
        let Some(view) = views
            .entry(project.project_id.clone())
            .or_insert_with(|| self.project_open_view(&project.project_id))
            .as_ref()
        else {
            return;
        };
        project.open_state = view.state;
        for machine in &view.machines {
            if machine.machine_id == Some(*machine_id) {
                project.part = Some(machine.part);
                project.part_reason = machine.reason.clone();
            } else {
                project.other_hosts.push(ProjectHostSummary {
                    hostname: machine.hostname.clone(),
                    connected: machine.connected,
                    running: matches!(
                        machine.part,
                        MachinePart::Running | MachinePart::StillRunning
                    ),
                    owns_tabs: machine.owns_tabs,
                });
            }
        }
    }

    /// [`Self::fill_project_open_state`] for every project entry of a machine
    /// list on its way to `viewer`.
    pub(crate) fn fill_machines_open_state(
        &self,
        viewer: &Uuid,
        machines: &mut [MachineWithProjects],
    ) {
        let mut views = HashMap::new();
        for machine in machines {
            let machine_id = machine.machine.machine_id;
            for project in &mut machine.projects {
                self.fill_project_open_state_with(viewer, &machine_id, project, &mut views);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The lock is exclusive per project: a second holder waits for the
    /// first, and another project's lock is not involved.
    #[tokio::test]
    async fn the_open_state_lock_is_exclusive_per_project() {
        let manager = Arc::new(SessionManager::new());
        let held = manager.project_open_state_guard("p").await;
        let entered = Arc::new(AtomicUsize::new(0));
        let waiting = {
            let manager = manager.clone();
            let entered = entered.clone();
            tokio::spawn(async move {
                let _guard = manager.project_open_state_guard("p").await;
                entered.fetch_add(1, Ordering::SeqCst);
            })
        };
        // Another project is free while this one is held.
        tokio::time::timeout(
            Duration::from_secs(5),
            manager.project_open_state_guard("other"),
        )
        .await
        .expect("another project's lock is free");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(entered.load(Ordering::SeqCst), 0, "still held");
        drop(held);
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("the waiter got the lock")
            .unwrap();
        assert_eq!(entered.load(Ordering::SeqCst), 1);
    }

    /// Forgetting a deleted project does not take its lock from under a
    /// task that holds it: whoever asks next waits for that same lock, and
    /// never runs beside its holder. The lock is dropped once nobody holds
    /// it or waits for it.
    #[tokio::test]
    async fn forgetting_a_project_keeps_a_lock_that_is_held() {
        let manager = Arc::new(SessionManager::new());
        let held = manager.project_open_state_guard("p").await;
        manager.forget_project_open_state("p");
        let entered = Arc::new(AtomicUsize::new(0));
        let waiting = {
            let manager = manager.clone();
            let entered = entered.clone();
            tokio::spawn(async move {
                let _guard = manager.project_open_state_guard("p").await;
                entered.fetch_add(1, Ordering::SeqCst);
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            entered.load(Ordering::SeqCst),
            0,
            "the next holder waits for the lock that was held"
        );
        // Forgetting again while one holds it and one waits changes nothing.
        manager.forget_project_open_state("p");
        assert!(manager.open_state.locks.contains_key("p"));
        drop(held);
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("the waiter got the lock")
            .unwrap();
        assert_eq!(entered.load(Ordering::SeqCst), 1);

        manager.forget_project_open_state("p");
        assert!(
            !manager.open_state.locks.contains_key("p"),
            "nobody holds it: it is dropped"
        );
    }

    #[test]
    fn outcomes_are_kept_per_project_and_machine() {
        let manager = SessionManager::new();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let request = Uuid::new_v4();
        assert_eq!(manager.machine_outcome("p", &m5), None);

        manager.record_machine_start_asked("p", m5, Some(request));
        manager.record_machine_start_asked("p", m6, None);
        manager.record_machine_not_started("q", m5, "zoo-005 refused");
        assert!(matches!(
            manager.machine_outcome("p", &m5),
            Some(MachineOutcome::StartAsked { request_id, .. }) if request_id == Some(request)
        ));
        assert_eq!(
            manager.machine_start_request(&request),
            Some(("p".to_string(), m5))
        );
        assert_eq!(manager.machine_start_request(&Uuid::new_v4()), None);
        assert_eq!(manager.project_machine_outcomes("p").len(), 2);

        // An answer replaces the pending start, so its id matches nothing.
        manager.record_machine_started("p", m5);
        assert_eq!(
            manager.machine_outcome("p", &m5),
            Some(MachineOutcome::Started)
        );
        assert_eq!(manager.machine_start_request(&request), None);

        manager.record_machine_not_stopped("p", m6, "not allowed");
        assert_eq!(
            manager.machine_outcome("p", &m6),
            Some(MachineOutcome::NotStopped("not allowed".to_string()))
        );
        manager.clear_machine_outcome("p", &m6);
        assert_eq!(manager.machine_outcome("p", &m6), None);

        manager.clear_project_outcomes("p");
        assert!(manager.project_machine_outcomes("p").is_empty());
        manager.forget_project_open_state("q");
        assert_eq!(manager.machine_outcome("q", &m5), None);
        manager.record_machine_not_started("q", m5, "zoo-005 refused");
        assert_eq!(
            manager.machine_outcome("q", &m5),
            Some(MachineOutcome::NotStarted("zoo-005 refused".to_string())),
            "another project's outcomes are kept"
        );
    }
}
