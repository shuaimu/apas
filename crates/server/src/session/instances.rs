//! Project instances: one project runtime per machine, every one of them
//! attached to the project's single session.
//!
//! Each attached instance owns the panes tagged with its machine. This module
//! decides which CLI may attach beside which, merges each instance's roster
//! report into the project roster, answers which instance owns a pane, scopes
//! a departing instance's disconnect to its own panes, and routes CLI-bound
//! messages by [`ServerToCli::scope`].

use super::{
    normalize_machine_hostname, normalize_project_path, PendingPaneReboot, SessionManager,
    SessionState, PENDING_PANE_REBOOT_TTL,
};
use shared::{
    PaneConfig, PaneType, ServerToCli, ServerToCliScope, ServerToWeb, SessionInstanceInfo,
    PANE_ID_INTERACTIVE, PROJECT_INSTANCES_CAPABILITY,
};
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use uuid::Uuid;

/// One project instance attached to a session: the project's runtime on one
/// machine, reached through one CLI connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceAttachment {
    pub cli_id: Uuid,
    /// The machine the CLI reported. Standalone and older CLIs report none;
    /// such a CLI is never capable, so it can only be the session's only
    /// instance.
    pub machine_id: Option<Uuid>,
    pub hostname: Option<String>,
    pub working_dir: Option<String>,
    /// The CLI advertised [`PROJECT_INSTANCES_CAPABILITY`] and reported a
    /// machine: it reports only its own panes and can run beside other
    /// machines' instances. Without it a CLI assumes it is the project's only
    /// runtime and reports its whole roster untagged.
    pub capable: bool,
    /// When this machine's instance attached. A same-machine reconnect keeps
    /// it, and with it the instance's place in the primary order.
    pub attached_at: chrono::DateTime<chrono::Utc>,
}

/// Which attached instance receives a control for one pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneOwner {
    /// The instance that owns the pane. For a pane no machine owns yet (an
    /// untagged pane with no attributed host, or one the roster does not
    /// list) this is the primary, as a fallback.
    Instance(InstanceAttachment),
    /// The pane belongs to a machine that runs no instance of the project, so
    /// its state lives where nothing can act on it. Refuse with
    /// "<host> is not running this project. Open the project again to start
    /// it there." while that machine's daemon is connected, and "<host> is
    /// not running this project because it is offline." while it is not.
    NotRunning {
        machine_id: Option<Uuid>,
        hostname: Option<String>,
    },
    /// No instance of the project is attached.
    NoInstance,
}

/// What became of a message `route_to_cli` was asked to deliver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliRouteOutcome {
    /// Accepted by this many instances: one, or every attached instance for a
    /// project-wide message.
    Delivered(usize),
    /// The message is for a machine that runs no instance of the project now:
    /// the owner of the pane it names, or the machine the caller named. This
    /// is not a transport failure, and the refusal should name the host.
    MachineNotRunning {
        machine_id: Option<Uuid>,
        hostname: Option<String>,
    },
    /// No instance is attached to the session, or it is not in memory.
    NoInstance,
    /// An instance was chosen but its connection did not take the message.
    Undelivered,
}

impl CliRouteOutcome {
    pub fn delivered(&self) -> bool {
        matches!(self, Self::Delivered(_))
    }
}

/// Result of a CLI's `SessionStart`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachOutcome {
    Attached {
        /// This instance is the session's primary: it supplies the session's
        /// database row and single-valued fields.
        primary: bool,
        /// Connections this one replaced: earlier attachments of the same
        /// machine, or of this connection.
        replaced: Vec<Uuid>,
    },
    /// The CLI cannot run beside the instance already attached. The reason
    /// names that instance's host.
    Rejected { reason: String },
}

/// One instance leaving a session.
#[derive(Debug, Clone)]
pub struct InstanceDetach {
    pub session_id: Uuid,
    pub instance: InstanceAttachment,
    /// The departing instance was the primary, so the session's database row
    /// must move to `primary`.
    pub was_primary: bool,
    /// The session's primary afterwards; `None` when no instance remains and
    /// the session becomes inactive.
    pub primary: Option<InstanceAttachment>,
}

/// Result of an instance's `SessionEnd`.
#[derive(Debug, Clone)]
pub enum SessionLeave {
    /// The CLI has no instance in the session.
    NotAttached,
    /// It is the only instance, so its end is the session's end. It stays
    /// attached until its connection closes, as before.
    Last,
    /// Other instances still run the project; this one left.
    Detached(InstanceDetach),
}

/// Whether a CLI may report output, state or status for one pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundPaneVerdict {
    Accept,
    /// The CLI has no instance in the session: it was replaced, rejected, or
    /// never started it.
    NotAttached,
    /// The roster lists the pane for another machine.
    Foreign { owner_hostname: Option<String> },
}

/// How a newly starting CLI relates to one instance already attached.
enum Admission {
    /// The same machine (or the same connection): take its place.
    Replace,
    /// Another machine's capable instance: attach beside it.
    Beside,
    Reject(String),
}

fn host_label(instance: &InstanceAttachment) -> String {
    instance
        .hostname
        .as_deref()
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .unwrap_or("another machine")
        .to_string()
}

/// Why a project cannot start on another machine while `older`, an instance
/// whose CLI cannot share the project, runs it: what such a start's
/// `SessionStart` is refused with, and what a Start is refused with before
/// it reaches a daemon.
pub(crate) fn beside_older_cli_refusal(older: &InstanceAttachment) -> String {
    let host = host_label(older);
    format!(
        "This project is running on {host} with an APAS CLI that cannot share it with other machines; stop it there, or reboot {host} from the Machines page to update it, then start it here"
    )
}

fn admission(existing: &InstanceAttachment, incoming: &InstanceAttachment) -> Admission {
    if existing.cli_id == incoming.cli_id {
        return Admission::Replace;
    }
    // Refusal for a CLI that assumes it is the project's only runtime.
    let update_this_cli = || {
        Admission::Reject(format!(
            "This project is running on {}; update this APAS CLI to run it on more than one machine at once",
            host_label(existing)
        ))
    };
    match (existing.capable, incoming.capable) {
        (true, true) if existing.machine_id == incoming.machine_id => Admission::Replace,
        (true, true) => Admission::Beside,
        // An older CLI is the only instance it knows of: beside it, its
        // untagged full roster would overwrite this instance's tabs, and
        // replacing it would orphan a runtime that keeps writing into the
        // session. Only its own machine may take its place, and one that
        // reported no machine could be anywhere.
        (false, true) => {
            if existing.machine_id.is_some() && existing.machine_id == incoming.machine_id {
                Admission::Replace
            } else {
                Admission::Reject(beside_older_cli_refusal(existing))
            }
        }
        (true, false) => {
            if incoming.machine_id.is_some() && incoming.machine_id == existing.machine_id {
                Admission::Replace
            } else {
                update_this_cli()
            }
        }
        // Two CLIs that each assume they are the only runtime keep the old
        // replacement unless they are visibly on different machines. Hostnames
        // decide when both are known: a `daemon.machine_id` set in a shared
        // `config.toml` gives every host on that home the same id.
        (false, false) => {
            let different = match (existing.hostname.as_deref(), incoming.hostname.as_deref()) {
                (Some(a), Some(b)) => {
                    normalize_machine_hostname(a) != normalize_machine_hostname(b)
                }
                _ => matches!(
                    (existing.machine_id, incoming.machine_id),
                    (Some(a), Some(b)) if a != b
                ),
            };
            if different {
                update_this_cli()
            } else {
                Admission::Replace
            }
        }
    }
}

/// Which instance owns `pane_id`, by the pane's tag or attributed host.
pub(super) fn resolve_pane_owner(session: &SessionState, pane_id: u32) -> PaneOwner {
    let Some(primary) = session.instances.first() else {
        return PaneOwner::NoInstance;
    };
    let Some(pane) = session.panes.iter().find(|pane| pane.pane_id == pane_id) else {
        return PaneOwner::Instance(primary.clone());
    };
    // A tagged pane is its machine's, whichever instance is primary, and
    // even while a CLI without the capability is the sole instance: such a
    // CLI reports every pane it runs untagged, so a tab still tagged for
    // another machine is one it does not run, such as a separate checkout's.
    if let Some(machine_id) = pane.machine_id {
        return match session
            .instances
            .iter()
            .find(|instance| instance.machine_id == Some(machine_id))
        {
            Some(owner) => PaneOwner::Instance(owner.clone()),
            None => PaneOwner::NotRunning {
                machine_id: Some(machine_id),
                hostname: pane.hostname.clone(),
            },
        };
    }
    // An untagged pane goes to the instance on its attributed host, else to a
    // sole CLI without the capability, which runs every untagged pane as
    // before instances. With no attributed host there is nothing to refuse
    // in the name of, so the primary takes it.
    let host = pane
        .hostname
        .as_deref()
        .map(normalize_machine_hostname)
        .filter(|host| !host.is_empty());
    if let Some(host) = &host {
        if let Some(owner) = session.instances.iter().find(|instance| {
            instance
                .hostname
                .as_deref()
                .is_some_and(|candidate| normalize_machine_hostname(candidate) == *host)
        }) {
            return PaneOwner::Instance(owner.clone());
        }
    }
    if session.instances.len() == 1 && !primary.capable {
        return PaneOwner::Instance(primary.clone());
    }
    match host {
        None => PaneOwner::Instance(primary.clone()),
        Some(_) => PaneOwner::NotRunning {
            machine_id: None,
            hostname: pane.hostname.clone(),
        },
    }
}

/// The panes a departing instance's disconnect touches.
enum DepartingPanes {
    /// A CLI without the capability was the only instance and ran every pane
    /// but these, which are tagged for other machines: a tab still tagged
    /// for another machine is one such a CLI does not run, since it reports
    /// every pane it runs untagged.
    AllBut(HashSet<u32>),
    /// A capable instance owns the panes tagged with its machine.
    Tagged(HashSet<u32>),
}

impl DepartingPanes {
    fn of(session: &SessionState, instance: &InstanceAttachment) -> Self {
        let tagged = |mine: bool| -> HashSet<u32> {
            session
                .panes
                .iter()
                .filter(|pane| {
                    pane.machine_id
                        .is_some_and(|owner| (Some(owner) == instance.machine_id) == mine)
                })
                .map(|pane| pane.pane_id)
                .collect()
        };
        if instance.capable {
            Self::Tagged(tagged(true))
        } else {
            Self::AllBut(tagged(false))
        }
    }

    fn touches(&self, pane_id: u32) -> bool {
        match self {
            Self::AllBut(others) => !others.contains(&pane_id),
            Self::Tagged(owned) => owned.contains(&pane_id),
        }
    }
}

/// The departing instance and what is needed to finish its detach once the
/// session's lock is released.
struct LockedDetach {
    instance: InstanceAttachment,
    was_primary: bool,
    primary: Option<InstanceAttachment>,
    departing: DepartingPanes,
    /// The panes whose cached status the detach dropped, which viewers are
    /// told are no longer busy.
    cleared_statuses: Vec<(u32, PaneType)>,
    /// The saved prompts viewers are shown changed to the next primary's.
    prompts_changed: bool,
}

impl SessionState {
    /// Copy the primary instance onto the single-valued fields older readers
    /// use (`cli_client_id`, `working_dir`, `hostname`, `machine_id`). With no
    /// instance left only the CLI is cleared: the last host and directory
    /// remain the session's description, as they always have.
    pub(super) fn sync_primary(&mut self) {
        match self.instances.first() {
            Some(primary) => {
                self.cli_client_id = Some(primary.cli_id);
                self.working_dir = primary.working_dir.clone();
                self.hostname = primary.hostname.clone();
                self.machine_id = primary.machine_id;
            }
            None => self.cli_client_id = None,
        }
    }

    /// Remove the instance at `index`. Its pane statuses go with it, because
    /// their producer is gone and a viewer attaching later would otherwise
    /// be shown a stale indicator; every pane stays in the roster.
    fn detach_at(&mut self, index: usize) -> LockedDetach {
        let instance = self.instances.remove(index);
        let departing = DepartingPanes::of(self, &instance);
        // A status for a pane the roster does not list has no owner left to
        // clear it, so it goes too.
        let listed: HashSet<u32> = self.panes.iter().map(|pane| pane.pane_id).collect();
        let mut cleared_statuses = Vec::new();
        self.pane_statuses.retain(|pane_id, (pane_type, _)| {
            let keep = listed.contains(pane_id) && !departing.touches(*pane_id);
            if !keep {
                cleared_statuses.push((*pane_id, *pane_type));
            }
            keep
        });
        // The instance that reported these is gone, and viewers are told
        // their state is not reported; its provider may well still be running
        // under its pane host.
        self.pane_autopilot.retain(|pane_id, report| {
            report.reporter != instance.cli_id && !departing.touches(*pane_id)
        });
        self.sync_primary();
        // The saved prompts shown are the primary's. When it leaves, the next
        // primary's last report takes their place at once: on separate
        // checkouts it is a different list, and its CLI, having reported it
        // already, sends it again only when it changes. A next primary that
        // has reported nothing keeps the departed one's list on show, as
        // `autopilot_prompts` explains.
        self.autopilot_prompts_reports.remove(&instance.cli_id);
        let mut prompts_changed = false;
        if index == 0 {
            let promoted = self
                .instances
                .first()
                .and_then(|primary| self.autopilot_prompts_reports.get(&primary.cli_id))
                .cloned();
            if let Some(view) = promoted {
                prompts_changed = self.autopilot_prompts.as_ref() != Some(&view);
                self.autopilot_prompts = Some(view);
            }
        }
        LockedDetach {
            instance,
            was_primary: index == 0,
            primary: self.instances.first().cloned(),
            departing,
            cleared_statuses,
            prompts_changed,
        }
    }
}

/// Keep the first report of each pane id.
fn dedupe_by_pane_id(panes: Vec<PaneConfig>) -> Vec<PaneConfig> {
    let mut seen = HashSet::new();
    panes
        .into_iter()
        .filter(|pane| seen.insert(pane.pane_id))
        .collect()
}

/// A capable instance's report merged into `existing`: its own panes are
/// replaced, the untagged ones it reports adopted, everything else kept in
/// place, and new panes appended. Returns the roster, the reporter's panes it
/// no longer reports, and reported panes the roster lists for another machine.
fn merge_capable_report(
    existing: &[PaneConfig],
    reported: Vec<PaneConfig>,
    machine_id: Option<Uuid>,
) -> (Vec<PaneConfig>, Vec<u32>, Vec<u32>) {
    let order: Vec<u32> = reported.iter().map(|pane| pane.pane_id).collect();
    let mut incoming: HashMap<u32, PaneConfig> =
        reported.into_iter().map(|pane| (pane.pane_id, pane)).collect();
    let mut merged = Vec::with_capacity(existing.len() + incoming.len());
    let mut removed = Vec::new();
    let mut foreign = Vec::new();
    for pane in existing {
        match pane.machine_id {
            Some(owner) if Some(owner) == machine_id => match incoming.remove(&pane.pane_id) {
                Some(update) => merged.push(update),
                None => removed.push(pane.pane_id),
            },
            Some(_) => {
                if incoming.remove(&pane.pane_id).is_some() {
                    foreign.push(pane.pane_id);
                }
                merged.push(pane.clone());
            }
            None => merged.push(incoming.remove(&pane.pane_id).unwrap_or_else(|| pane.clone())),
        }
    }
    merged.extend(order.into_iter().filter_map(|pane_id| incoming.remove(&pane_id)));
    (merged, removed, foreign)
}

/// A report from a CLI without the capability, the session's sole instance,
/// merged into `existing`; `machine_id` is the machine its `SessionStart`
/// named, if any. Each pane it reports takes that id's place, whatever the
/// entry's tag: on a shared home such a CLI restores and runs every pane in
/// the `.apas`, as before instances. Untagged panes it did not report go,
/// and so do panes tagged with its own machine. A pane tagged for another
/// machine that it did not report stays, stopped there with its id still
/// allocated: a separate checkout's tabs are not ones it can see, and
/// dropping them would leave nothing to start or forget until that machine
/// ran the project again. Existing panes keep their places and new ones are
/// appended. See [`PaneConfig::machine_id`].
fn merge_sole_report(
    existing: &[PaneConfig],
    reported: Vec<PaneConfig>,
    machine_id: Option<Uuid>,
) -> Vec<PaneConfig> {
    let order: Vec<u32> = reported.iter().map(|pane| pane.pane_id).collect();
    let mut incoming: HashMap<u32, PaneConfig> =
        reported.into_iter().map(|pane| (pane.pane_id, pane)).collect();
    let mut merged: Vec<PaneConfig> = existing
        .iter()
        .filter_map(|pane| match incoming.remove(&pane.pane_id) {
            Some(update) => Some(update),
            None => pane
                .machine_id
                .filter(|owner| Some(*owner) != machine_id)
                .map(|_| pane.clone()),
        })
        .collect();
    merged.extend(order.into_iter().filter_map(|pane_id| incoming.remove(&pane_id)));
    merged
}

/// One attached instance as the machine lists see it.
pub(super) struct RunningInstance {
    pub machine_id: Option<Uuid>,
    pub hostname: Option<String>,
    pub working_dir: Option<String>,
    pub project_id: Option<String>,
}

impl RunningInstance {
    /// Whether this instance runs `project` on the machine `machine_id`
    /// named `hostname`. The machine matches by id or by hostname, as hosts
    /// always did; the project by path, or by id on an identified machine.
    pub(super) fn runs(
        &self,
        machine_id: &Uuid,
        hostname: &str,
        project: &shared::MachineProjectInfo,
    ) -> bool {
        let on_machine = self.machine_id == Some(*machine_id);
        let same_host = on_machine
            || self.hostname.as_deref().is_some_and(|host| {
                normalize_machine_hostname(host) == normalize_machine_hostname(hostname)
            });
        if !same_host {
            return false;
        }
        let same_path = self.working_dir.as_deref().is_some_and(|dir| {
            normalize_project_path(dir) == normalize_project_path(&project.path)
        });
        same_path || (on_machine && self.project_id.as_deref() == Some(project.project_id.as_str()))
    }
}

impl SessionManager {
    pub fn cli_supports_capability(&self, cli_id: &Uuid, capability: &str) -> bool {
        self.cli_capabilities
            .get(cli_id)
            .is_some_and(|capabilities| capabilities.contains(capability))
    }

    /// Attach a CLI's project instance to `session_id`, creating the session
    /// in memory when needed. A capable CLI adds its machine's instance, or
    /// replaces that machine's earlier one in place, keeping its place in the
    /// primary order. A CLI without the capability is only ever the session's
    /// sole instance: it replaces the instance on its own machine and is
    /// refused while another machine's instance is attached; a capable CLI is
    /// likewise refused while such a CLI runs the project elsewhere. Either
    /// way the refusal names the host that runs it.
    pub fn attach_cli_instance(
        &self,
        session_id: Uuid,
        cli_id: Uuid,
        machine_id: Option<Uuid>,
        working_dir: Option<String>,
        hostname: Option<String>,
    ) -> AttachOutcome {
        let outcome =
            self.attach_instance_state(session_id, cli_id, machine_id, working_dir, hostname);
        if matches!(outcome, AttachOutcome::Attached { .. }) {
            // Machine lists mark the project running from its instances.
            if let Some(user_id) = self.cli_users.get(&cli_id).map(|owner| *owner) {
                self.broadcast_machines_update_for_user(&user_id);
            }
            self.broadcast_session_presence(&session_id, true);
            self.broadcast_cli_clients_update();
            self.broadcast_session_instances(&session_id);
            // Viewers are not told an instance's capabilities, so they hear
            // here whether each OMP tab's instance can run autopilot.
            self.broadcast_pane_autopilot(&session_id, None);
            // The machine runs the project: a start pending there is over,
            // and its part changed.
            let project_id = self
                .project_for_session(&session_id)
                .unwrap_or_else(|| session_id.to_string());
            if let Some(machine_id) = machine_id {
                self.note_machine_runs(&project_id, &machine_id);
            }
            self.report_project_open_state(&project_id);
        }
        outcome
    }

    /// The state change behind [`Self::attach_cli_instance`], without telling
    /// viewers: the legacy web-started session announces its own CLI.
    pub(super) fn attach_instance_state(
        &self,
        session_id: Uuid,
        cli_id: Uuid,
        machine_id: Option<Uuid>,
        working_dir: Option<String>,
        hostname: Option<String>,
    ) -> AttachOutcome {
        let capable = machine_id.is_some()
            && self.cli_supports_capability(&cli_id, PROJECT_INSTANCES_CAPABILITY);
        let mut incoming = InstanceAttachment {
            cli_id,
            machine_id,
            hostname,
            working_dir,
            capable,
            attached_at: chrono::Utc::now(),
        };
        let (primary, replaced) = {
            let mut session = self
                .sessions
                .entry(session_id)
                .or_insert_with(|| SessionState::empty(session_id, Uuid::nil()));
            let mut replace = Vec::new();
            for (index, existing) in session.instances.iter().enumerate() {
                match admission(existing, &incoming) {
                    Admission::Replace => replace.push(index),
                    Admission::Beside => {}
                    Admission::Reject(reason) => {
                        tracing::warn!(
                            %session_id,
                            %cli_id,
                            ?machine_id,
                            capable,
                            running = %existing.cli_id,
                            "refusing a project instance: {reason}"
                        );
                        return AttachOutcome::Rejected { reason };
                    }
                }
            }
            let replaced: Vec<Uuid> = replace
                .iter()
                .map(|&index| session.instances[index].cli_id)
                .collect();
            // A replaced connection's autopilot reports are not the new one's:
            // it reports every OMP tab again after its own registration.
            session.pane_autopilot.retain(|_, report| {
                report.reporter == cli_id || !replaced.contains(&report.reporter)
            });
            // Nor are its saved prompts; the list on show stays until the new
            // connection reports its own.
            for old_cli in replaced.iter().filter(|old_cli| **old_cli != cli_id) {
                session.autopilot_prompts_reports.remove(old_cli);
            }
            match replace.split_first() {
                Some((&first, rest)) => {
                    incoming.attached_at = session.instances[first].attached_at;
                    session.instances[first] = incoming;
                    for &index in rest.iter().rev() {
                        session.instances.remove(index);
                    }
                }
                None => session.instances.push(incoming),
            }
            session.sync_primary();
            (session.cli_client_id == Some(cli_id), replaced)
        };

        // A replaced connection no longer speaks for this session, so its
        // later disconnect must not detach the instance that took its place.
        for old_cli in replaced.iter().filter(|old_cli| **old_cli != cli_id) {
            if let Some(mut tracked) = self.cli_sessions.get_mut(old_cli) {
                tracked.retain(|tracked_session| *tracked_session != session_id);
            }
        }
        if let Some(mut tracked) = self.cli_sessions.get_mut(&cli_id) {
            if !tracked.contains(&session_id) {
                tracked.push(session_id);
            }
        }
        tracing::info!(
            %session_id,
            %cli_id,
            ?machine_id,
            capable,
            primary,
            ?replaced,
            "project instance attached"
        );
        AttachOutcome::Attached { primary, replaced }
    }

    /// Detach `cli_id`'s instance from `session_id`. Only that instance's
    /// running terminals become disconnected and only its pane statuses
    /// clear; every pane stays in the roster. The session becomes inactive
    /// only when no instance remains, and otherwise the next earliest
    /// instance becomes primary. The caller moves the database row.
    pub fn detach_cli_instance(&self, session_id: &Uuid, cli_id: &Uuid) -> Option<InstanceDetach> {
        let locked = {
            let mut session = self.sessions.get_mut(session_id)?;
            let index = session
                .instances
                .iter()
                .position(|instance| instance.cli_id == *cli_id)?;
            session.detach_at(index)
        };
        Some(self.finish_detach(session_id, locked))
    }

    /// Detach `cli_id` from every session it runs an instance of, as its
    /// connection closes.
    pub fn detach_cli_instances(&self, cli_id: &Uuid) -> Vec<InstanceDetach> {
        self.get_cli_session_ids(cli_id)
            .iter()
            .filter_map(|session_id| self.detach_cli_instance(session_id, cli_id))
            .collect()
    }

    /// An instance ended its session. While another machine still runs the
    /// project only this instance leaves; the last one ends the session and
    /// stays attached until its connection closes, as before. Decided under
    /// the session's lock, so two instances ending at once cannot both
    /// believe the other remains.
    pub fn leave_session(&self, session_id: &Uuid, cli_id: &Uuid) -> SessionLeave {
        let locked = {
            let Some(mut session) = self.sessions.get_mut(session_id) else {
                return SessionLeave::NotAttached;
            };
            let Some(index) = session
                .instances
                .iter()
                .position(|instance| instance.cli_id == *cli_id)
            else {
                return SessionLeave::NotAttached;
            };
            if session.instances.len() == 1 {
                return SessionLeave::Last;
            }
            session.detach_at(index)
        };
        SessionLeave::Detached(self.finish_detach(session_id, locked))
    }

    fn finish_detach(&self, session_id: &Uuid, locked: LockedDetach) -> InstanceDetach {
        let LockedDetach {
            instance,
            was_primary,
            primary,
            departing,
            cleared_statuses,
            prompts_changed,
        } = locked;
        if let Some(mut tracked) = self.cli_sessions.get_mut(&instance.cli_id) {
            tracked.retain(|tracked_session| tracked_session != session_id);
        }
        // Transport loss does not prove that the instance or its PTYs ended.
        // Keep their presentation and show the uncertainty until a reconnect
        // reconciles each process, but only for this instance's terminals:
        // another machine's keep running, and so does one an instance still
        // attached has reported since, such as this machine's reconnect.
        let attached: HashSet<Uuid> = self
            .session_attachments(session_id)
            .iter()
            .map(|attachment| attachment.cli_id)
            .collect();
        let terminals = self.mark_terminals_disconnected(session_id, |pane_id, reporter| {
            if reporter.is_some_and(|reporter| attached.contains(&reporter)) {
                return false;
            }
            departing.touches(pane_id) || reporter == Some(instance.cli_id)
        });
        for (pane_id, terminal) in terminals {
            self.route_to_web_now(
                session_id,
                ServerToWeb::TerminalState {
                    session_id: *session_id,
                    pane_id,
                    instance_id: terminal.instance_id,
                    lifecycle: terminal.lifecycle,
                    status: terminal.status,
                    runtime: terminal.runtime,
                },
            );
        }
        // Viewers keep each pane's last status until told otherwise. While
        // another machine keeps the session active nothing else clears the
        // departing machine's: a native client, which never re-reads
        // statuses, would show its tabs Working or Pending answer for good.
        for (pane_id, pane_type) in cleared_statuses {
            self.route_to_web_now(
                session_id,
                ServerToWeb::PaneStatus {
                    session_id: *session_id,
                    pane_type,
                    pane_id: Some(pane_id),
                    status: None,
                },
            );
        }
        if primary.is_none() {
            self.broadcast_session_presence(session_id, false);
        }
        self.broadcast_session_instances(session_id);
        self.broadcast_pane_autopilot(session_id, None);
        // The departing machine's part in the project changed.
        self.report_session_open_state(session_id);
        if prompts_changed {
            self.broadcast_autopilot_prompts(session_id);
        }
        if let Some(user_id) = self.cli_users.get(&instance.cli_id).map(|owner| *owner) {
            self.broadcast_machines_update_for_user(&user_id);
        }
        tracing::info!(
            %session_id,
            cli_id = %instance.cli_id,
            machine_id = ?instance.machine_id,
            was_primary,
            remaining_primary = ?primary.as_ref().map(|primary| primary.cli_id),
            "project instance detached"
        );
        InstanceDetach {
            session_id: *session_id,
            instance,
            was_primary,
            primary,
        }
    }

    /// The session's attached instances, primary first.
    pub fn session_attachments(&self, session_id: &Uuid) -> Vec<InstanceAttachment> {
        self.sessions
            .get(session_id)
            .map(|session| session.instances.clone())
            .unwrap_or_default()
    }

    /// The earliest attached instance, which receives every control that
    /// names no machine.
    pub fn primary_instance(&self, session_id: &Uuid) -> Option<InstanceAttachment> {
        self.sessions
            .get(session_id)
            .and_then(|session| session.instances.first().cloned())
    }

    /// The instance `cli_id` runs in `session_id`, if it is attached.
    pub fn instance_for_cli(&self, session_id: &Uuid, cli_id: &Uuid) -> Option<InstanceAttachment> {
        self.sessions.get(session_id).and_then(|session| {
            session
                .instances
                .iter()
                .find(|instance| instance.cli_id == *cli_id)
                .cloned()
        })
    }

    /// The instance running on `machine_id`, if one is attached.
    pub fn instance_on_machine(
        &self,
        session_id: &Uuid,
        machine_id: &Uuid,
    ) -> Option<InstanceAttachment> {
        self.sessions.get(session_id).and_then(|session| {
            session
                .instances
                .iter()
                .find(|instance| instance.machine_id == Some(*machine_id))
                .cloned()
        })
    }

    /// Which instance owns `pane_id`. A tagged pane belongs to its machine's
    /// instance, whether or not that one has the capability. An untagged pane
    /// goes to the instance on its attributed host, else to a sole CLI
    /// without the capability, which runs every untagged pane, else to the
    /// primary when it has no attributed host. A pane the roster does not
    /// list also goes to the primary. Anything else is
    /// [`PaneOwner::NotRunning`] on the pane's machine.
    pub fn pane_owner(&self, session_id: &Uuid, pane_id: u32) -> PaneOwner {
        match self.sessions.get(session_id) {
            Some(session) => resolve_pane_owner(&session, pane_id),
            None => PaneOwner::NoInstance,
        }
    }

    /// Whether `cli_id` may report output, state or status for `pane_id`.
    /// Only the instance on a tagged pane's machine may, whether or not it
    /// has the capability. An untagged pane belongs to no machine until an
    /// instance reports it, so any attached instance may; and a pane the
    /// roster does not list yet is accepted, because a new tab's first frames
    /// can arrive before the roster report that lists it, unless its id is
    /// reserved for a new pane on another machine.
    pub fn inbound_pane_verdict(
        &self,
        session_id: &Uuid,
        cli_id: &Uuid,
        pane_id: u32,
    ) -> InboundPaneVerdict {
        let Some(session) = self.sessions.get(session_id) else {
            return InboundPaneVerdict::NotAttached;
        };
        let Some(reporter) = session
            .instances
            .iter()
            .find(|instance| instance.cli_id == *cli_id)
        else {
            return InboundPaneVerdict::NotAttached;
        };
        // A pane tagged for another machine is never the reporter's, even a
        // sole CLI without the capability's: that CLI reports every pane it
        // runs untagged.
        match session.panes.iter().find(|pane| pane.pane_id == pane_id) {
            Some(pane) if pane.machine_id.is_some() && pane.machine_id != reporter.machine_id => {
                InboundPaneVerdict::Foreign {
                    owner_hostname: pane.hostname.clone(),
                }
            }
            Some(_) => InboundPaneVerdict::Accept,
            // Not listed yet. While its id is reserved for a new pane sent to
            // another machine, that machine is the only one whose instance
            // can be running it.
            None => match self.live_pane_reservation(&session, pane_id) {
                Some(Some(machine_id)) if Some(machine_id) != reporter.machine_id => {
                    InboundPaneVerdict::Foreign {
                        owner_hostname: self.machine_hostname(&machine_id),
                    }
                }
                _ => InboundPaneVerdict::Accept,
            },
        }
    }

    pub fn machine_hostname(&self, machine_id: &Uuid) -> Option<String> {
        self.machine_infos
            .get(machine_id)
            .map(|machine| machine.hostname.clone())
    }

    /// Deliver to the owner of `pane_id` (see [`Self::pane_owner`]); for a
    /// message the caller has already resolved to a pane, such as an answer
    /// to that pane's pending decision.
    pub async fn route_to_cli_for_pane(
        &self,
        session_id: &Uuid,
        pane_id: u32,
        msg: ServerToCli,
    ) -> CliRouteOutcome {
        match self.pane_owner(session_id, pane_id) {
            PaneOwner::Instance(owner) => {
                self.deliver_to_clis(session_id, &[owner.cli_id], msg).await
            }
            PaneOwner::NotRunning {
                machine_id,
                hostname,
            } => CliRouteOutcome::MachineNotRunning {
                machine_id,
                hostname,
            },
            PaneOwner::NoInstance => CliRouteOutcome::NoInstance,
        }
    }

    /// Deliver to the instance on `machine_id`, or to the primary for
    /// `None`. A named machine with no attached instance is refused, never
    /// redirected: the message would act on a machine nobody picked.
    pub async fn route_to_cli_on_machine(
        &self,
        session_id: &Uuid,
        machine_id: Option<Uuid>,
        msg: ServerToCli,
    ) -> CliRouteOutcome {
        let target = match machine_id {
            None => self.primary_instance(session_id),
            Some(machine_id) => match self.instance_on_machine(session_id, &machine_id) {
                Some(instance) => Some(instance),
                None => {
                    return CliRouteOutcome::MachineNotRunning {
                        machine_id: Some(machine_id),
                        hostname: self.machine_hostname(&machine_id),
                    }
                }
            },
        };
        match target {
            Some(instance) => self.deliver_to_clis(session_id, &[instance.cli_id], msg).await,
            None => CliRouteOutcome::NoInstance,
        }
    }

    /// Route by [`ServerToCli::scope`]: a pane's controls to its owner, a new
    /// pane to the machine it was tagged for, project-wide state to every
    /// instance, process controls to the primary, and an answer or untargeted
    /// input to the pane it concerns. Callers that know a machine or a pane
    /// use [`Self::route_to_cli_on_machine`] or
    /// [`Self::route_to_cli_for_pane`].
    pub async fn route_to_cli_by_scope(
        &self,
        session_id: &Uuid,
        msg: ServerToCli,
    ) -> CliRouteOutcome {
        match msg.scope() {
            ServerToCliScope::Pane(pane_id) => {
                self.route_to_cli_for_pane(session_id, pane_id, msg).await
            }
            ServerToCliScope::NewPane { machine_id } => {
                self.route_to_cli_on_machine(session_id, machine_id, msg)
                    .await
            }
            ServerToCliScope::Project => {
                let targets: Vec<Uuid> = self
                    .session_attachments(session_id)
                    .iter()
                    .map(|instance| instance.cli_id)
                    .collect();
                if targets.is_empty() {
                    return CliRouteOutcome::NoInstance;
                }
                self.deliver_to_clis(session_id, &targets, msg).await
            }
            // Connection messages are normally sent straight to one CLI; one
            // routed by session reaches the primary, as it always did.
            ServerToCliScope::Process | ServerToCliScope::Connection => {
                self.route_to_cli_on_machine(session_id, None, msg).await
            }
            ServerToCliScope::Unscoped => match self.unscoped_target_pane(session_id, &msg) {
                Some(pane_id) => self.route_to_cli_for_pane(session_id, pane_id, msg).await,
                None => self.route_to_cli_on_machine(session_id, None, msg).await,
            },
        }
    }

    /// The pane an [`ServerToCliScope::Unscoped`] message concerns, whose
    /// owner receives it; `None` sends it to the primary. Input naming no
    /// pane is applied by a CLI to the legacy interactive pane. An answer
    /// goes with the decision it answers while the server holds that
    /// decision, and to the primary when it does not.
    pub fn unscoped_target_pane(&self, session_id: &Uuid, msg: &ServerToCli) -> Option<u32> {
        match msg {
            ServerToCli::Input { .. } => Some(PANE_ID_INTERACTIVE),
            ServerToCli::AnswerQuestion { tool_use_id, .. }
            | ServerToCli::PlanReviewAnswer { tool_use_id, .. } => self
                .pending_decisions
                .get(&(*session_id, tool_use_id.clone()))
                .and_then(|decision| decision.pane_id),
            _ => None,
        }
    }

    async fn deliver_to_clis(
        &self,
        session_id: &Uuid,
        cli_ids: &[Uuid],
        msg: ServerToCli,
    ) -> CliRouteOutcome {
        let mut delivered = 0;
        for cli_id in cli_ids {
            tracing::debug!(
                %session_id,
                %cli_id,
                connected = self.cli_senders.contains_key(cli_id),
                "route_to_cli"
            );
            if self.send_to_cli(cli_id, msg.clone()).await {
                delivered += 1;
            }
        }
        if delivered > 0 {
            CliRouteOutcome::Delivered(delivered)
        } else {
            CliRouteOutcome::Undelivered
        }
    }

    /// Merge one instance's roster report into the project roster and return
    /// the result, or `None` when `cli_id` has no instance in the session.
    ///
    /// A capable instance M's report replaces every pane tagged M, adopts the
    /// untagged entries it reports, and leaves every other pane alone; a
    /// reported pane the roster lists for another machine is dropped and
    /// logged. A sole CLI without the capability replaces every pane but
    /// those tagged for another machine that it did not report; see
    /// [`merge_sole_report`].
    /// The server tags what it accepts from the reporting instance, whatever
    /// the pane data says: a capable instance's machine and hostname, or for
    /// an older CLI no machine and its hostname as the attributed host. See
    /// [`PaneConfig::machine_id`]. Existing panes keep their places and
    /// web-set labels, new ones are appended, and only the reporter's removed
    /// panes lose their status. The result seeds the roster unless
    /// `panes.json` could not be read; see [`SessionState::roster_unreadable`].
    pub fn merge_instance_roster(
        &self,
        session_id: &Uuid,
        cli_id: &Uuid,
        reported: Vec<PaneConfig>,
    ) -> Option<Vec<PaneConfig>> {
        let mut reboots: Vec<((Uuid, u32), PendingPaneReboot)> = self
            .pending_pane_reboots
            .iter()
            .filter(|entry| entry.key().0 == *session_id)
            .map(|entry| (*entry.key(), entry.value().clone()))
            .collect();
        reboots.sort_by_key(|(_, reboot)| reboot.original_index);
        let now = Instant::now();

        let (merged, finished_reboots, foreign) = {
            let mut entry = self.sessions.get_mut(session_id)?;
            let session = &mut *entry;
            let reporter = session
                .instances
                .iter()
                .find(|instance| instance.cli_id == *cli_id)?
                .clone();
            let mut reported = dedupe_by_pane_id(reported);

            // A web reboot is a close and then an add on the CLI. While one is
            // in flight, keep the logical pane in its owner's report so the
            // tab neither flashes out nor loses its place. Only the
            // reporter's own reboots: another machine's pane is not this
            // report's to keep.
            let mut finished_reboots = Vec::new();
            for (key, reboot) in reboots {
                if now.duration_since(reboot.registered_at) > PENDING_PANE_REBOOT_TTL {
                    finished_reboots.push(key);
                    continue;
                }
                let owned = matches!(
                    resolve_pane_owner(session, reboot.pane.pane_id),
                    PaneOwner::Instance(owner) if owner.cli_id == *cli_id
                );
                if !owned {
                    continue;
                }
                if reported
                    .iter()
                    .any(|pane| pane.pane_id == reboot.pane.pane_id)
                {
                    finished_reboots.push(key);
                } else {
                    reported.push(reboot.pane);
                }
            }

            {
                let previous: HashMap<u32, &PaneConfig> = session
                    .panes
                    .iter()
                    .map(|pane| (pane.pane_id, pane))
                    .collect();
                for pane in &mut reported {
                    let previous = previous.get(&pane.pane_id);
                    pane.machine_id = if reporter.capable {
                        reporter.machine_id
                    } else {
                        None
                    };
                    pane.hostname = reporter
                        .hostname
                        .clone()
                        .or_else(|| previous.and_then(|pane| pane.hostname.clone()));
                    // A label set from the web wins over the CLI's copy.
                    if let Some(label) = previous.and_then(|pane| pane.label.clone()) {
                        pane.label = Some(label);
                    }
                }
            }

            let reported_ids: Vec<u32> = reported.iter().map(|pane| pane.pane_id).collect();
            let (merged, foreign) = if reporter.capable {
                let (merged, removed, foreign) =
                    merge_capable_report(&session.panes, reported, reporter.machine_id);
                for pane_id in &removed {
                    session.pane_statuses.remove(pane_id);
                    session.pane_idle_since.remove(pane_id);
                    session.pane_autopilot.remove(pane_id);
                }
                (merged, foreign)
            } else {
                let merged = merge_sole_report(&session.panes, reported, reporter.machine_id);
                let listed: HashSet<u32> = merged.iter().map(|pane| pane.pane_id).collect();
                session
                    .pane_statuses
                    .retain(|pane_id, _| listed.contains(pane_id));
                session
                    .pane_idle_since
                    .retain(|pane_id, _| listed.contains(pane_id));
                session
                    .pane_autopilot
                    .retain(|pane_id, _| listed.contains(pane_id));
                (merged, Vec::new())
            };
            // A tab that stopped being an OMP terminal tab, as after a change
            // of provider, has no autopilot left to show.
            session.pane_autopilot.retain(|pane_id, _| {
                merged.iter().all(|pane| {
                    pane.pane_id != *pane_id || super::autopilot::is_autopilot_tab(pane)
                })
            });
            let observed_at = chrono::Utc::now().to_rfc3339();
            for pane in &merged {
                if !session.pane_statuses.contains_key(&pane.pane_id) {
                    session
                        .pane_idle_since
                        .entry(pane.pane_id)
                        .or_insert_with(|| observed_at.clone());
                }
            }
            // A forgotten tab that its machine reports again is back in the
            // roster, and its id is in use rather than remembered. An untagged
            // one comes back with whichever instance reports it. Another
            // machine reporting a forgotten id is kept, since dropping a
            // running tab would hide it, but the id stays remembered for the
            // machine it was forgotten for.
            for pane_id in reported_ids.iter().filter(|pane_id| !foreign.contains(pane_id)) {
                let Some(forgotten_for) = session.forgotten_panes.get(pane_id).copied() else {
                    continue;
                };
                if forgotten_for.is_none() || forgotten_for == reporter.machine_id {
                    session.forgotten_panes.remove(pane_id);
                } else {
                    tracing::warn!(
                        %session_id,
                        %cli_id,
                        pane_id,
                        ?forgotten_for,
                        "an instance reported a pane id forgotten for another machine"
                    );
                }
            }
            session.panes = merged.clone();
            // Merged into nothing because `panes.json` could not be read, the
            // roster lacks whatever the file holds: shown, but not seeded, so
            // it is not saved over the file and the next load reads it again.
            if !session.roster_unreadable {
                session.roster_seeded = true;
            }
            (merged, finished_reboots, foreign)
        };

        for key in finished_reboots {
            self.pending_pane_reboots.remove(&key);
        }
        if !foreign.is_empty() {
            tracing::warn!(
                %session_id,
                %cli_id,
                pane_ids = ?foreign,
                "dropped reported panes the roster lists for another machine"
            );
        }
        Some(merged)
    }

    /// After a close of `pane_id` was delivered to `cli_id`, drop the pane
    /// from the roster if no instance runs it: the roster lists it untagged
    /// and `cli_id`'s instance, which received the close, can share the
    /// project. Returns whether it was dropped.
    ///
    /// Such an instance reports every pane it runs, and its report tags each
    /// one, so an untagged entry routed to it is one it does not have: a
    /// stale entry persisted before instances, panes inferred from history,
    /// or a pane closed before the instance's first report. Its report leaves
    /// untagged entries alone, as it must for other hosts' tabs, and the close
    /// changes nothing on its side, so without this the tab could never be
    /// closed. If the instance did run it after all, the close it received
    /// ends it there too. The id is remembered, not freed: an untagged entry
    /// left in some `.apas` comes back when an instance adopts it.
    pub fn drop_closed_untagged_pane(
        &self,
        session_id: &Uuid,
        pane_id: u32,
        cli_id: &Uuid,
    ) -> bool {
        {
            let Some(mut entry) = self.sessions.get_mut(session_id) else {
                return false;
            };
            let session = &mut *entry;
            let received_by_capable = session
                .instances
                .iter()
                .any(|instance| instance.cli_id == *cli_id && instance.capable);
            let Some(index) = session
                .panes
                .iter()
                .position(|pane| pane.pane_id == pane_id && pane.machine_id.is_none())
            else {
                return false;
            };
            if !received_by_capable {
                return false;
            }
            session.panes.remove(index);
            session.pane_statuses.remove(&pane_id);
            session.pane_idle_since.remove(&pane_id);
            session.pane_autopilot.remove(&pane_id);
            session.forgotten_panes.insert(pane_id, None);
        }
        self.terminal_states.remove(&(*session_id, pane_id));
        self.pending_pane_reboots.remove(&(*session_id, pane_id));
        self.pending_decisions
            .retain(|(decision_session, _), decision| {
                decision_session != session_id || decision.pane_id != Some(pane_id)
            });
        tracing::info!(
            %session_id,
            pane_id,
            %cli_id,
            "dropped a closed tab that no instance runs"
        );
        true
    }

    /// Whether the in-memory roster holds the project's roster yet: seeded
    /// from `panes.json`, reported by an instance, or set by a web change. A
    /// report merged while `panes.json` could not be read does not seed it.
    pub fn pane_roster_seeded(&self, session_id: &Uuid) -> bool {
        self.sessions
            .get(session_id)
            .is_some_and(|session| session.roster_seeded)
    }

    /// Seed the roster from the persisted one unless something already set
    /// it, and return the roster now held. After a server restart a capable
    /// instance's report must merge into the persisted roster rather than an
    /// empty one, or every stopped machine's tabs would vanish; and a late
    /// seed must not overwrite a report that merged first.
    ///
    /// Reports merged while `panes.json` could not be read are cached but
    /// never seeded the roster. They are the latest word on the panes they
    /// list, so those replace the stored entries with their ids and the rest
    /// are appended; stored entries they do not list, a stopped machine's
    /// among them, stay. A pane such a report had dropped is back until its
    /// instance reports again, which the caller asks every instance to do.
    pub fn hydrate_pane_roster(
        &self,
        session_id: &Uuid,
        stored: Vec<PaneConfig>,
    ) -> Vec<PaneConfig> {
        let Some(mut session) = self.sessions.get_mut(session_id) else {
            return stored;
        };
        if !session.roster_seeded {
            let mut roster = stored;
            for reported in std::mem::take(&mut session.panes) {
                match roster
                    .iter_mut()
                    .find(|pane| pane.pane_id == reported.pane_id)
                {
                    Some(pane) => *pane = reported,
                    None => roster.push(reported),
                }
            }
            session.replace_roster(roster);
        }
        session.panes.clone()
    }

    /// Whether the last read of the session's `panes.json` failed, with none
    /// succeeding since; see [`SessionState::roster_unreadable`].
    pub fn pane_roster_unreadable(&self, session_id: &Uuid) -> bool {
        self.sessions
            .get(session_id)
            .is_some_and(|session| session.roster_unreadable)
    }

    /// Record that `panes.json` could not be read, so reports merge without
    /// seeding the roster and nothing saves over the file. A roster seeded
    /// meanwhile, by a read that succeeded, no longer depends on the file and
    /// is left alone.
    pub fn mark_pane_roster_unreadable(&self, session_id: &Uuid) {
        if let Some(mut session) = self.sessions.get_mut(session_id) {
            if !session.roster_seeded {
                session.roster_unreadable = true;
            }
        }
    }

    /// `panes.json` is gone, or was moved aside: nothing is left to protect,
    /// and the next report seeds the roster.
    pub fn clear_pane_roster_unreadable(&self, session_id: &Uuid) {
        if let Some(mut session) = self.sessions.get_mut(session_id) {
            session.roster_unreadable = false;
        }
    }

    /// Seed the roster with panes inferred from message history, for a session
    /// that never saved one, and only while nothing else has set it: a report
    /// merged since the roster was found missing is the truth, and while
    /// `panes.json` is unreadable a guess must not become the roster saved
    /// over it. Returns whether it seeded.
    pub fn seed_inferred_pane_roster(&self, session_id: &Uuid, inferred: Vec<PaneConfig>) -> bool {
        let Some(mut session) = self.sessions.get_mut(session_id) else {
            return false;
        };
        if session.roster_seeded || session.roster_unreadable || !session.panes.is_empty() {
            return false;
        }
        session.replace_roster(inferred);
        true
    }

    /// Attribute every untagged pane that has no host to `host`, the session's
    /// recorded host. Only a roster persisted before panes carried hosts has
    /// such panes, and they ran on the host the session last recorded. Without
    /// it a capable instance joining from elsewhere would be their fallback
    /// owner and receive their controls. Returns whether anything changed.
    pub fn stamp_untagged_pane_hosts(&self, session_id: &Uuid, host: &str) -> bool {
        let host = host.trim();
        if host.is_empty() {
            return false;
        }
        let Some(mut session) = self.sessions.get_mut(session_id) else {
            return false;
        };
        let mut changed = false;
        for pane in session
            .panes
            .iter_mut()
            .filter(|pane| pane.machine_id.is_none() && pane.hostname.is_none())
        {
            pane.hostname = Some(host.to_string());
            changed = true;
        }
        changed
    }

    /// The session's attached instances as viewers see them, primary first.
    /// `runtime_access` is left unevaluated: it depends on the viewer.
    pub fn session_instances(&self, session_id: &Uuid) -> Vec<SessionInstanceInfo> {
        self.session_attachments(session_id)
            .into_iter()
            .enumerate()
            .map(|(index, instance)| SessionInstanceInfo {
                hostname: instance
                    .hostname
                    .clone()
                    .filter(|host| !host.trim().is_empty())
                    .or_else(|| {
                        instance
                            .machine_id
                            .and_then(|machine_id| self.machine_hostname(&machine_id))
                    })
                    .unwrap_or_else(|| "unknown host".to_string()),
                machine_id: instance.machine_id,
                working_dir: instance.working_dir,
                primary: index == 0,
                runtime_access: None,
            })
            .collect()
    }

    /// Send the complete instance set to the session's viewers that asked for
    /// it with [`PROJECT_INSTANCES_CAPABILITY`]. Older native builds drop
    /// their socket on a frame their schema does not list.
    pub fn broadcast_session_instances(&self, session_id: &Uuid) {
        let viewers: Vec<Uuid> = self
            .sessions
            .get(session_id)
            .map(|session| session.web_connection_ids.clone())
            .unwrap_or_default();
        let viewers: Vec<Uuid> = viewers
            .into_iter()
            .filter(|connection_id| {
                self.web_supports_capability(connection_id, PROJECT_INSTANCES_CAPABILITY)
            })
            .collect();
        if viewers.is_empty() {
            return;
        }
        let message = ServerToWeb::SessionInstances {
            session_id: *session_id,
            instances: self.session_instances(session_id),
        };
        for connection_id in viewers {
            let sender = self
                .web_senders
                .get(&connection_id)
                .map(|sender| sender.clone());
            if let Some(sender) = sender {
                let _ = sender.try_send(message.clone());
            }
        }
    }

    /// Every attached instance with its project, for marking machine
    /// projects running.
    pub(super) fn running_instances(&self) -> Vec<RunningInstance> {
        let attached: Vec<(Uuid, Vec<InstanceAttachment>)> = self
            .sessions
            .iter()
            .filter(|session| !session.instances.is_empty())
            .map(|session| (session.session_id, session.instances.clone()))
            .collect();
        attached
            .into_iter()
            .flat_map(|(session_id, instances)| {
                let project_id = self.project_for_session(&session_id);
                instances.into_iter().map(move |instance| RunningInstance {
                    machine_id: instance.machine_id,
                    hostname: instance.hostname,
                    working_dir: instance.working_dir,
                    project_id: project_id.clone(),
                })
            })
            .collect()
    }

    /// `(daemon_connected, project_instances_available)` for a machine
    /// entry: the daemon holds a live connection, and it also advertises
    /// [`PROJECT_INSTANCES_CAPABILITY`]. Connected but not available is an
    /// older daemon, which a reboot updates.
    pub(super) fn machine_instance_support(&self, machine_id: &Uuid) -> (bool, bool) {
        let connected = self.is_daemon_connected(machine_id);
        (
            connected,
            connected && self.daemon_supports_capability(machine_id, PROJECT_INSTANCES_CAPABILITY),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::TerminalSnapshotState;
    use shared::{
        CliLifecycleInventory, CliLifecycleOperation, CliLifecyclePhase, MachineInfo,
        MachineProjectInfo, PaneType, TerminalLifecycle,
    };
    use tokio::sync::mpsc;

    const HOST_A: &str = "zoo-005";
    const HOST_B: &str = "zoo-006";

    struct Cli {
        id: Uuid,
        rx: mpsc::Receiver<ServerToCli>,
    }

    fn cli(manager: &SessionManager, user_id: Uuid, capable: bool) -> Cli {
        let id = Uuid::new_v4();
        let (tx, rx) = mpsc::channel(64);
        manager.register_cli(id, user_id, tx, None);
        if capable {
            manager.set_cli_capabilities(id, vec![PROJECT_INSTANCES_CAPABILITY.to_string()]);
        }
        Cli { id, rx }
    }

    fn attach(
        manager: &SessionManager,
        session_id: Uuid,
        cli: &Cli,
        machine_id: Option<Uuid>,
        host: Option<&str>,
    ) -> AttachOutcome {
        manager.attach_cli_instance(
            session_id,
            cli.id,
            machine_id,
            host.map(|host| format!("/work/{host}")),
            host.map(str::to_string),
        )
    }

    fn attached(outcome: AttachOutcome) -> (bool, Vec<Uuid>) {
        match outcome {
            AttachOutcome::Attached { primary, replaced } => (primary, replaced),
            AttachOutcome::Rejected { reason } => panic!("expected an attach, got {reason}"),
        }
    }

    fn rejection(outcome: AttachOutcome) -> String {
        match outcome {
            AttachOutcome::Rejected { reason } => reason,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn pane(pane_id: u32, machine_id: Option<Uuid>, hostname: Option<&str>) -> PaneConfig {
        let mut pane = PaneConfig::defaults().remove(0);
        pane.pane_id = pane_id;
        pane.kind = shared::PaneKind::Terminal;
        pane.label = None;
        pane.machine_id = machine_id;
        pane.hostname = hostname.map(str::to_string);
        pane
    }

    fn ids(panes: &[PaneConfig]) -> Vec<u32> {
        panes.iter().map(|pane| pane.pane_id).collect()
    }

    fn drain<T>(rx: &mut mpsc::Receiver<T>) -> Vec<T> {
        let mut received = Vec::new();
        while let Ok(message) = rx.try_recv() {
            received.push(message);
        }
        received
    }

    fn attached_clis(manager: &SessionManager, session_id: &Uuid) -> Vec<Uuid> {
        manager
            .session_attachments(session_id)
            .iter()
            .map(|instance| instance.cli_id)
            .collect()
    }

    fn running_terminal(manager: &SessionManager, session_id: &Uuid, pane_id: u32, reporter: Uuid) {
        manager
            .reconcile_terminal_state_from(
                Some(reporter),
                session_id,
                pane_id,
                Some(Uuid::new_v4()),
                TerminalLifecycle::Running,
                None,
                None,
            )
            .expect("running report accepted");
    }

    fn lifecycle(manager: &SessionManager, session_id: &Uuid, pane_id: u32) -> TerminalLifecycle {
        manager
            .terminal_snapshot(session_id, pane_id)
            .map(|snapshot: TerminalSnapshotState| snapshot.lifecycle)
            .expect("terminal state")
    }

    /// Two machines' instances of one project share its session; the
    /// earliest supplies the session's host and receives what names no
    /// machine.
    #[test]
    fn capable_instances_on_two_machines_attach_side_by_side() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let a = cli(&manager, user, true);
        let b = cli(&manager, user, true);

        assert_eq!(attached(attach(&manager, session, &a, Some(m5), Some(HOST_A))), (true, vec![]));
        assert_eq!(attached(attach(&manager, session, &b, Some(m6), Some(HOST_B))), (false, vec![]));

        assert_eq!(attached_clis(&manager, &session), vec![a.id, b.id]);
        let state = manager.get_session(&session).unwrap();
        assert_eq!(state.cli_client_id, Some(a.id), "the primary keeps the legacy field");
        assert_eq!(state.hostname.as_deref(), Some(HOST_A));
        assert_eq!(state.machine_id, Some(m5));
        assert!(manager.is_session_active(&session));

        let instances = manager.session_instances(&session);
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].machine_id, Some(m5));
        assert_eq!(instances[0].hostname, HOST_A);
        assert!(instances[0].primary);
        assert_eq!(instances[1].machine_id, Some(m6));
        assert_eq!(instances[1].working_dir.as_deref(), Some("/work/zoo-006"));
        assert!(!instances[1].primary);
        assert!(instances.iter().all(|instance| instance.runtime_access.is_none()));
    }

    /// After a restart a project's session is not in memory, and its first
    /// instance and its first viewer can arrive at the same moment. Whichever
    /// creates the session, the other joins it: the viewer never replaces the
    /// session the instance attached to, which nothing would attach again.
    /// Both run on their own threads from a shared barrier, many times over,
    /// since the two only collide when they interleave.
    #[test]
    fn a_viewer_attaching_alongside_the_first_instance_keeps_it_attached() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let a = cli(&manager, user, true);
        let cli_id = a.id;
        let machine = Uuid::new_v4();
        let web = Uuid::new_v4();
        let sessions: Vec<Uuid> = (0..10_000).map(|_| Uuid::new_v4()).collect();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for session in &sessions {
                    barrier.wait();
                    attached(manager.attach_cli_instance(
                        *session,
                        cli_id,
                        Some(machine),
                        Some(format!("/work/{HOST_A}")),
                        Some(HOST_A.to_string()),
                    ));
                }
            });
            scope.spawn(|| {
                for session in &sessions {
                    barrier.wait();
                    assert!(manager.attach_web_to_session(session, web, None));
                }
            });
        });
        let lost: Vec<&Uuid> = sessions
            .iter()
            .filter(|session| manager.instance_for_cli(session, &a.id).is_none())
            .collect();
        assert!(lost.is_empty(), "{} sessions lost their instance", lost.len());
        assert!(sessions
            .iter()
            .all(|session| manager.is_web_attached_to_session(session, &web)));
    }

    /// A reconnect replaces only its own machine's attachment, in place, so
    /// a primary that reconnects stays primary, and the replaced socket's
    /// late disconnect detaches nothing.
    #[test]
    fn a_same_machine_reconnect_replaces_only_its_own_attachment() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let a = cli(&manager, user, true);
        let b = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        attached(attach(&manager, session, &b, Some(m6), Some(HOST_B)));
        let first_attached_at = manager.session_attachments(&session)[0].attached_at;

        let a2 = cli(&manager, user, true);
        assert_eq!(
            attached(attach(&manager, session, &a2, Some(m5), Some(HOST_A))),
            (true, vec![a.id])
        );
        assert_eq!(attached_clis(&manager, &session), vec![a2.id, b.id]);
        assert_eq!(manager.session_attachments(&session)[0].attached_at, first_attached_at);
        assert!(!manager.get_cli_session_ids(&a.id).contains(&session));

        // The replaced connection closing later is no longer this session's.
        assert!(manager.detach_cli_instances(&a.id).is_empty());
        manager.unregister_cli(&a.id);
        assert_eq!(attached_clis(&manager, &session), vec![a2.id, b.id]);

        let b2 = cli(&manager, user, true);
        assert_eq!(
            attached(attach(&manager, session, &b2, Some(m6), Some(HOST_B))),
            (false, vec![b.id])
        );
        assert_eq!(attached_clis(&manager, &session), vec![a2.id, b2.id]);
    }

    /// A CLI without the capability assumes it is the project's only
    /// runtime, so it never attaches beside another machine's instance and
    /// no capable instance attaches beside it. On its own machine it keeps
    /// the old replacement.
    #[test]
    fn an_older_cli_never_runs_beside_another_machines_instance() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let (m5, m6, shared_id) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());

        // An older CLI meets a running capable instance.
        let session = Uuid::new_v4();
        let a = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        let elsewhere = cli(&manager, user, false);
        let reason = rejection(attach(&manager, session, &elsewhere, Some(m6), Some(HOST_B)));
        assert!(reason.contains(HOST_A) && reason.contains("update this APAS CLI"), "{reason}");
        // Nothing shows a CLI that reported no machine is on the same one.
        let machineless = cli(&manager, user, false);
        rejection(attach(&manager, session, &machineless, None, Some(HOST_A)));
        assert_eq!(attached_clis(&manager, &session), vec![a.id]);
        // The same machine's older CLI takes its place as the sole instance.
        let downgraded = cli(&manager, user, false);
        assert_eq!(
            attached(attach(&manager, session, &downgraded, Some(m5), Some(HOST_A))),
            (true, vec![a.id])
        );

        // A capable instance meets a running older CLI.
        let session = Uuid::new_v4();
        let older = cli(&manager, user, false);
        attached(attach(&manager, session, &older, Some(shared_id), Some(HOST_A)));
        let joining = cli(&manager, user, true);
        let reason = rejection(attach(&manager, session, &joining, Some(m6), Some(HOST_B)));
        assert!(reason.contains(HOST_A) && reason.contains("Machines page"), "{reason}");
        let upgraded = cli(&manager, user, true);
        assert_eq!(
            attached(attach(&manager, session, &upgraded, Some(shared_id), Some(HOST_A))),
            (true, vec![older.id])
        );

        let session = Uuid::new_v4();
        let standalone = cli(&manager, user, false);
        attached(attach(&manager, session, &standalone, None, Some(HOST_A)));
        let same_host = cli(&manager, user, true);
        rejection(attach(&manager, session, &same_host, Some(m5), Some(HOST_A)));
        assert_eq!(attached_clis(&manager, &session), vec![standalone.id]);

        // Two older CLIs: replaced on one host, refused across hosts.
        let session = Uuid::new_v4();
        let first = cli(&manager, user, false);
        attached(attach(&manager, session, &first, None, Some(HOST_A)));
        let other_host = cli(&manager, user, false);
        rejection(attach(&manager, session, &other_host, None, Some(HOST_B)));
        let same = cli(&manager, user, false);
        assert_eq!(
            attached(attach(&manager, session, &same, None, Some("ZOO-005"))),
            (true, vec![first.id])
        );
        let unknown = cli(&manager, user, false);
        assert_eq!(
            attached(attach(&manager, session, &unknown, None, None)),
            (true, vec![same.id])
        );

        // A `daemon.machine_id` shared through the home gives two hosts one
        // id; their hostnames still tell them apart.
        let session = Uuid::new_v4();
        let zoo5 = cli(&manager, user, false);
        attached(attach(&manager, session, &zoo5, Some(shared_id), Some(HOST_A)));
        let zoo6 = cli(&manager, user, false);
        rejection(attach(&manager, session, &zoo6, Some(shared_id), Some(HOST_B)));

        // Advertising the capability without a machine is not capable.
        let session = Uuid::new_v4();
        let capable = cli(&manager, user, true);
        attached(attach(&manager, session, &capable, Some(m5), Some(HOST_A)));
        let no_machine = cli(&manager, user, true);
        rejection(attach(&manager, session, &no_machine, None, Some(HOST_B)));
    }

    /// Pane traffic is accepted only from the instance that owns the pane.
    #[test]
    fn pane_reports_are_accepted_only_from_the_owning_instance() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let a = cli(&manager, user, true);
        let b = cli(&manager, user, true);
        let stranger = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        attached(attach(&manager, session, &b, Some(m6), Some(HOST_B)));
        manager.set_session_panes(
            &session,
            vec![
                pane(3, Some(m5), Some(HOST_A)),
                pane(4, Some(m6), Some(HOST_B)),
                pane(7, None, Some(HOST_A)),
            ],
        );

        assert_eq!(manager.inbound_pane_verdict(&session, &a.id, 3), InboundPaneVerdict::Accept);
        assert_eq!(
            manager.inbound_pane_verdict(&session, &b.id, 3),
            InboundPaneVerdict::Foreign {
                owner_hostname: Some(HOST_A.to_string())
            }
        );
        assert_eq!(manager.inbound_pane_verdict(&session, &b.id, 4), InboundPaneVerdict::Accept);
        assert!(matches!(
            manager.inbound_pane_verdict(&session, &a.id, 4),
            InboundPaneVerdict::Foreign { .. }
        ));
        // Untagged panes belong to no machine until one reports them.
        assert_eq!(manager.inbound_pane_verdict(&session, &b.id, 7), InboundPaneVerdict::Accept);
        // A new tab's first frames can precede the roster that lists it.
        assert_eq!(manager.inbound_pane_verdict(&session, &a.id, 99), InboundPaneVerdict::Accept);
        assert_eq!(
            manager.inbound_pane_verdict(&session, &stranger.id, 99),
            InboundPaneVerdict::NotAttached
        );
        // While its id is reserved for a new tab on zoo-006, an unlisted pane
        // is zoo-006's alone.
        let reserved = manager.allocate_pane_id(&session, Some(m6)).unwrap();
        assert_eq!(
            manager.inbound_pane_verdict(&session, &b.id, reserved),
            InboundPaneVerdict::Accept
        );
        assert_eq!(
            manager.inbound_pane_verdict(&session, &a.id, reserved),
            InboundPaneVerdict::Foreign {
                owner_hostname: None
            }
        );

        // A sole older CLI reports every pane it runs untagged: one still
        // tagged for another machine is not one it runs.
        let legacy_session = Uuid::new_v4();
        let older = cli(&manager, user, false);
        attached(attach(&manager, legacy_session, &older, Some(m6), Some(HOST_B)));
        manager.set_session_panes(
            &legacy_session,
            vec![
                pane(3, Some(m5), Some(HOST_A)),
                pane(4, Some(m6), Some(HOST_B)),
                pane(5, None, Some(HOST_A)),
            ],
        );
        assert_eq!(
            manager.inbound_pane_verdict(&legacy_session, &older.id, 3),
            InboundPaneVerdict::Foreign {
                owner_hostname: Some(HOST_A.to_string())
            }
        );
        for pane_id in [4, 5] {
            assert_eq!(
                manager.inbound_pane_verdict(&legacy_session, &older.id, pane_id),
                InboundPaneVerdict::Accept
            );
        }
    }

    /// One machine's instance leaving disconnects only its own terminals and
    /// clears only its own statuses, telling viewers so; the session stays
    /// active under the next primary, and only the last instance leaving
    /// makes it inactive.
    #[test]
    fn a_departing_instance_touches_only_its_own_tabs_and_hands_over_the_primary() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let a = cli(&manager, user, true);
        let b = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        attached(attach(&manager, session, &b, Some(m6), Some(HOST_B)));
        manager.merge_instance_roster(&session, &a.id, vec![pane(3, None, None)]);
        manager.merge_instance_roster(&session, &b.id, vec![pane(4, None, None)]);
        running_terminal(&manager, &session, 3, a.id);
        running_terminal(&manager, &session, 4, b.id);
        // Not in the roster yet; its reporter still identifies its machine.
        running_terminal(&manager, &session, 9, b.id);
        manager.set_pane_status(&session, PaneType::Interactive, 3, Some("Working...".into()));
        manager.set_pane_status(&session, PaneType::Interactive, 4, Some("Working...".into()));
        let web = Uuid::new_v4();
        let (web_tx, mut web_rx) = mpsc::channel(64);
        manager.register_web(web, web_tx);
        assert!(manager.attach_web_to_session(&session, web, None));

        let detached = manager.detach_cli_instances(&a.id);
        assert_eq!(detached.len(), 1);
        assert!(detached[0].was_primary);
        assert_eq!(detached[0].instance.cli_id, a.id);
        assert_eq!(detached[0].primary.as_ref().map(|primary| primary.cli_id), Some(b.id));
        assert_eq!(lifecycle(&manager, &session, 3), TerminalLifecycle::Disconnected);
        assert_eq!(lifecycle(&manager, &session, 4), TerminalLifecycle::Running);
        assert_eq!(lifecycle(&manager, &session, 9), TerminalLifecycle::Running);
        let statuses = manager.get_pane_statuses(&session);
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].1, 4, "the remaining instance keeps its status");
        assert_eq!(ids(&manager.get_session_panes(&session)), vec![3, 4], "tabs stay listed");
        let state = manager.get_session(&session).unwrap();
        assert_eq!(state.cli_client_id, Some(b.id), "the next earliest becomes primary");
        assert_eq!(state.hostname.as_deref(), Some(HOST_B));
        assert_eq!(state.machine_id, Some(m6));
        assert!(manager.is_session_active(&session));
        let seen = drain(&mut web_rx);
        assert!(seen.iter().any(|message| matches!(
            message,
            ServerToWeb::TerminalState { pane_id: 3, lifecycle: TerminalLifecycle::Disconnected, .. }
        )));
        assert!(
            !seen.iter().any(|message| matches!(
                message,
                ServerToWeb::TerminalState { pane_id: 4 | 9, .. }
                    | ServerToWeb::SessionAttached { has_active_cli: false, .. }
            )),
            "{seen:?}"
        );
        // The session stays active, so nothing else would clear the departed
        // machine's Working indicator in a viewer that keeps statuses.
        let status_updates = |seen: &[ServerToWeb]| {
            seen.iter()
                .filter_map(|message| match message {
                    ServerToWeb::PaneStatus { pane_id, status, .. } => {
                        Some((*pane_id, status.clone()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(status_updates(&seen), vec![(Some(3), None)]);

        let detached = manager.detach_cli_instance(&session, &b.id).unwrap();
        assert!(detached.was_primary);
        assert!(detached.primary.is_none());
        assert_eq!(lifecycle(&manager, &session, 4), TerminalLifecycle::Disconnected);
        assert_eq!(lifecycle(&manager, &session, 9), TerminalLifecycle::Disconnected);
        assert!(manager.get_pane_statuses(&session).is_empty());
        assert!(!manager.is_session_active(&session));
        let state = manager.get_session(&session).unwrap();
        assert_eq!(state.cli_client_id, None);
        assert_eq!(state.hostname.as_deref(), Some(HOST_B), "the last host stays described");
        let seen = drain(&mut web_rx);
        assert!(seen.iter().any(|message| matches!(
            message,
            ServerToWeb::SessionAttached { has_active_cli: false, .. }
        )));
        assert_eq!(status_updates(&seen), vec![(Some(4), None)]);
    }

    /// The same machine can reconnect between a departing instance leaving
    /// the session and its terminals being marked. A terminal the reconnected
    /// instance has already reported running must stay running.
    #[test]
    fn a_reconnect_racing_the_disconnect_keeps_its_terminals_running() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let m5 = Uuid::new_v4();
        let a = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        manager.merge_instance_roster(&session, &a.id, vec![pane(3, None, None), pane(4, None, None)]);
        running_terminal(&manager, &session, 3, a.id);
        running_terminal(&manager, &session, 4, a.id);

        // The departing instance leaves the session...
        let locked = {
            let mut state = manager.sessions.get_mut(&session).unwrap();
            state.detach_at(0)
        };
        // ...the machine reconnects and reports one terminal before the
        // disconnect is finished.
        let a2 = cli(&manager, user, true);
        attached(attach(&manager, session, &a2, Some(m5), Some(HOST_A)));
        running_terminal(&manager, &session, 3, a2.id);
        manager.finish_detach(&session, locked);

        assert_eq!(lifecycle(&manager, &session, 3), TerminalLifecycle::Running);
        assert_eq!(lifecycle(&manager, &session, 4), TerminalLifecycle::Disconnected);
    }

    /// `SessionEnd` from one instance detaches it; only the last one ends the
    /// session, and it stays attached until its connection closes.
    #[test]
    fn ending_the_session_from_one_instance_detaches_only_it() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let a = cli(&manager, user, true);
        let b = cli(&manager, user, true);
        let stranger = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(Uuid::new_v4()), Some(HOST_A)));
        attached(attach(&manager, session, &b, Some(Uuid::new_v4()), Some(HOST_B)));

        match manager.leave_session(&session, &b.id) {
            SessionLeave::Detached(detach) => {
                assert!(!detach.was_primary);
                assert_eq!(detach.primary.map(|primary| primary.cli_id), Some(a.id));
            }
            other => panic!("expected the instance to leave, got {other:?}"),
        }
        assert_eq!(attached_clis(&manager, &session), vec![a.id]);
        assert!(matches!(manager.leave_session(&session, &a.id), SessionLeave::Last));
        assert_eq!(attached_clis(&manager, &session), vec![a.id]);
        assert!(matches!(
            manager.leave_session(&session, &stranger.id),
            SessionLeave::NotAttached
        ));
    }

    /// Pane controls reach only the instance owning the pane.
    #[tokio::test]
    async fn pane_controls_reach_only_the_owning_instance() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let mut a = cli(&manager, user, true);
        let mut b = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        attached(attach(&manager, session, &b, Some(m6), Some(HOST_B)));
        manager.merge_instance_roster(&session, &a.id, vec![pane(3, None, None)]);
        manager.merge_instance_roster(&session, &b.id, vec![pane(4, None, None)]);

        let typed = ServerToCli::TerminalInput {
            session_id: session,
            pane_id: 4,
            data_b64: "eA==".into(),
        };
        assert!(manager.route_to_cli(&session, typed).await);
        assert!(matches!(
            drain(&mut b.rx).as_slice(),
            [ServerToCli::TerminalInput { pane_id: 4, .. }]
        ));
        assert!(drain(&mut a.rx).is_empty(), "the primary never sees another machine's input");

        let reboot = ServerToCli::RebootPane {
            session_id: session,
            pane_id: 3,
        };
        assert_eq!(
            manager.route_to_cli_by_scope(&session, reboot).await,
            CliRouteOutcome::Delivered(1)
        );
        assert!(matches!(drain(&mut a.rx).as_slice(), [ServerToCli::RebootPane { pane_id: 3, .. }]));
        assert!(drain(&mut b.rx).is_empty());
    }

    /// Project-wide state reaches every instance; a process control reaches
    /// the primary, or exactly the machine the caller named.
    #[tokio::test]
    async fn project_state_fans_out_and_process_controls_reach_one_instance() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6, m9) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let mut a = cli(&manager, user, true);
        let mut b = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        attached(attach(&manager, session, &b, Some(m6), Some(HOST_B)));
        let (daemon_tx, _daemon_rx) = mpsc::channel(4);
        manager.register_daemon(
            m9,
            user,
            daemon_tx,
            MachineInfo {
                machine_id: m9,
                hostname: "zoo-009".to_string(),
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
                daemon_version: None,
                deepseek_backend: None,
                last_seen: None,
                providers: None,
            },
            Vec::new(),
        );

        let policy = ServerToCli::ProjectPolicy {
            session_id: session,
            policy: shared::EffectiveProjectPolicy::default(),
        };
        assert_eq!(
            manager.route_to_cli_by_scope(&session, policy).await,
            CliRouteOutcome::Delivered(2)
        );
        assert!(matches!(drain(&mut a.rx).as_slice(), [ServerToCli::ProjectPolicy { .. }]));
        assert!(matches!(drain(&mut b.rx).as_slice(), [ServerToCli::ProjectPolicy { .. }]));

        let reboot = || ServerToCli::RebootCli {
            session_id: session,
        };
        assert!(manager.route_to_cli(&session, reboot()).await);
        assert_eq!(drain(&mut a.rx).len(), 1, "a reboot naming no machine is the primary's");
        assert!(drain(&mut b.rx).is_empty());
        assert_eq!(
            manager.route_to_cli_on_machine(&session, Some(m6), reboot()).await,
            CliRouteOutcome::Delivered(1)
        );
        assert!(drain(&mut a.rx).is_empty());
        assert_eq!(drain(&mut b.rx).len(), 1);
        assert_eq!(
            manager.route_to_cli_on_machine(&session, Some(m9), reboot()).await,
            CliRouteOutcome::MachineNotRunning {
                machine_id: Some(m9),
                hostname: Some("zoo-009".to_string()),
            },
            "a named machine without an instance is refused, never redirected"
        );
        assert!(drain(&mut a.rx).is_empty() && drain(&mut b.rx).is_empty());
    }

    /// An untagged pane goes to the instance on its attributed host, else to
    /// a sole older CLI, else to the primary only when it has no attributed
    /// host; otherwise it is stopped on that host. A tagged pane is always
    /// its machine's.
    #[tokio::test]
    async fn untagged_panes_follow_their_attributed_host() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6, m9) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let a = cli(&manager, user, true);
        let b = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        attached(attach(&manager, session, &b, Some(m6), Some(HOST_B)));
        manager.set_session_panes(
            &session,
            vec![
                pane(7, None, Some("ZOO-006")),
                pane(8, None, None),
                pane(9, None, Some("zoo-009")),
                pane(10, Some(m9), Some("zoo-009")),
            ],
        );
        let owner = |pane_id| match manager.pane_owner(&session, pane_id) {
            PaneOwner::Instance(instance) => Ok(instance.cli_id),
            PaneOwner::NotRunning {
                machine_id,
                hostname,
            } => Err((machine_id, hostname)),
            PaneOwner::NoInstance => panic!("instances are attached"),
        };
        assert_eq!(owner(7), Ok(b.id));
        assert_eq!(owner(8), Ok(a.id), "no attributed host: the primary");
        assert_eq!(owner(9), Err((None, Some("zoo-009".to_string()))));
        assert_eq!(owner(10), Err((Some(m9), Some("zoo-009".to_string()))));
        assert_eq!(owner(11), Ok(a.id), "a pane the roster does not list: the primary");
        assert_eq!(
            manager
                .route_to_cli_by_scope(
                    &session,
                    ServerToCli::InterruptPane {
                        session_id: session,
                        pane_id: 9,
                    },
                )
                .await,
            CliRouteOutcome::MachineNotRunning {
                machine_id: None,
                hostname: Some("zoo-009".to_string()),
            }
        );

        // A sole older CLI runs every untagged pane, whatever its attributed
        // host, but a tagged pane stays its machine's, stopped there.
        let legacy_session = Uuid::new_v4();
        let older = cli(&manager, user, false);
        attached(attach(&manager, legacy_session, &older, None, Some(HOST_A)));
        manager.set_session_panes(
            &legacy_session,
            vec![pane(9, None, Some("zoo-009")), pane(10, Some(m9), Some("zoo-009"))],
        );
        for pane_id in [9, 11] {
            assert!(matches!(
                manager.pane_owner(&legacy_session, pane_id),
                PaneOwner::Instance(instance) if instance.cli_id == older.id
            ));
        }
        assert!(matches!(
            manager.pane_owner(&legacy_session, 10),
            PaneOwner::NotRunning { machine_id: Some(machine), .. } if machine == m9
        ));
    }

    /// A new pane goes to the machine it was tagged for; input that names no
    /// pane follows the interactive pane, and an answer its decision's pane.
    #[tokio::test]
    async fn new_panes_and_unscoped_messages_find_their_instance() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let mut a = cli(&manager, user, true);
        let mut b = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        attached(attach(&manager, session, &b, Some(m6), Some(HOST_B)));
        manager.merge_instance_roster(
            &session,
            &b.id,
            vec![pane(shared::PANE_ID_INTERACTIVE, None, None), pane(4, None, None)],
        );
        let add = |machine_id| ServerToCli::AddPane {
            session_id: session,
            pane_config: pane(12, machine_id, None),
            isolated_worktree: false,
            initial_input: None,
        };

        assert!(manager.route_to_cli(&session, add(Some(m6))).await);
        assert!(matches!(drain(&mut b.rx).as_slice(), [ServerToCli::AddPane { .. }]));
        assert!(drain(&mut a.rx).is_empty());
        assert!(manager.route_to_cli(&session, add(None)).await);
        assert!(matches!(drain(&mut a.rx).as_slice(), [ServerToCli::AddPane { .. }]));
        assert!(matches!(
            manager.route_to_cli_by_scope(&session, add(Some(Uuid::new_v4()))).await,
            CliRouteOutcome::MachineNotRunning { .. }
        ));

        let input = ServerToCli::Input {
            session_id: session,
            data: "y".into(),
            pane_id: None,
        };
        assert!(manager.route_to_cli(&session, input).await);
        assert!(matches!(drain(&mut b.rx).as_slice(), [ServerToCli::Input { .. }]));
        assert!(drain(&mut a.rx).is_empty());

        manager.register_pending_decision(
            session,
            "toolu_4".to_string(),
            Some(4),
            shared::MutationKind::Question,
        );
        let answer = |tool_use_id: &str| ServerToCli::AnswerQuestion {
            session_id: session,
            tool_use_id: tool_use_id.to_string(),
            answers: HashMap::new(),
        };
        assert!(manager.route_to_cli(&session, answer("toolu_4")).await);
        assert!(matches!(drain(&mut b.rx).as_slice(), [ServerToCli::AnswerQuestion { .. }]));
        assert!(manager.route_to_cli(&session, answer("toolu_unknown")).await);
        assert!(
            matches!(drain(&mut a.rx).as_slice(), [ServerToCli::AnswerQuestion { .. }]),
            "an answer whose decision cannot be resolved goes to the primary"
        );
    }

    #[tokio::test]
    async fn routing_without_an_instance_delivers_nothing() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let a = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(Uuid::new_v4()), Some(HOST_A)));
        manager.detach_cli_instance(&session, &a.id).unwrap();
        let input = || ServerToCli::TerminalInput {
            session_id: session,
            pane_id: 3,
            data_b64: "eA==".into(),
        };
        assert_eq!(
            manager.route_to_cli_by_scope(&session, input()).await,
            CliRouteOutcome::NoInstance
        );
        assert!(!manager.route_to_cli(&session, input()).await);
        assert_eq!(
            manager
                .route_to_cli_by_scope(&Uuid::new_v4(), ServerToCli::RequestPaneList {
                    session_id: session,
                })
                .await,
            CliRouteOutcome::NoInstance
        );
    }

    /// `Some(list)` replaces exactly the reporter's panes and `Some([])`
    /// means it has none; every other machine's panes stay.
    #[test]
    fn an_instance_report_replaces_only_its_own_panes() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let a = cli(&manager, user, true);
        let b = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        attached(attach(&manager, session, &b, Some(m6), Some(HOST_B)));

        manager.merge_instance_roster(&session, &a.id, vec![pane(3, None, None), pane(5, None, None)]);
        // Whatever the data claims, the server tags from the reporter.
        let merged = manager
            .merge_instance_roster(&session, &b.id, vec![pane(4, Some(m5), Some(HOST_A))])
            .unwrap();
        assert_eq!(ids(&merged), vec![3, 5, 4]);
        assert_eq!(merged[2].machine_id, Some(m6));
        assert_eq!(merged[2].hostname.as_deref(), Some(HOST_B));
        assert!(manager.pane_roster_seeded(&session));

        assert_eq!(ids(&manager.merge_instance_roster(&session, &b.id, vec![]).unwrap()), vec![3, 5]);
        assert_eq!(
            ids(&manager.merge_instance_roster(&session, &a.id, vec![pane(5, None, None)]).unwrap()),
            vec![5]
        );
        let stranger = cli(&manager, user, true);
        assert!(manager
            .merge_instance_roster(&session, &stranger.id, vec![pane(6, None, None)])
            .is_none());
        assert_eq!(ids(&manager.get_session_panes(&session)), vec![5]);
    }

    /// A reported pane the roster lists for another machine is dropped.
    #[test]
    fn a_report_cannot_claim_another_machines_pane() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let a = cli(&manager, user, true);
        let b = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        attached(attach(&manager, session, &b, Some(m6), Some(HOST_B)));
        let original = pane(3, None, None);
        let original_runtime = original.session_id;
        manager.merge_instance_roster(&session, &a.id, vec![original]);

        let merged = manager
            .merge_instance_roster(&session, &b.id, vec![pane(3, None, None), pane(4, None, None)])
            .unwrap();
        assert_eq!(ids(&merged), vec![3, 4]);
        assert_eq!(merged[0].machine_id, Some(m5));
        assert_eq!(merged[0].session_id, original_runtime, "the owner's pane is untouched");
        assert_eq!(merged[1].machine_id, Some(m6));
    }

    /// Ownership of an untagged pane is established only by an instance
    /// reporting it; one the reporter does not report is left alone.
    #[test]
    fn untagged_panes_are_adopted_only_by_reporting_them() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let m6 = Uuid::new_v4();
        let b = cli(&manager, user, true);
        attached(attach(&manager, session, &b, Some(m6), Some(HOST_B)));
        manager.set_session_panes(&session, vec![pane(3, None, Some(HOST_A)), pane(4, None, None)]);

        let merged = manager
            .merge_instance_roster(&session, &b.id, vec![pane(3, None, None)])
            .unwrap();
        assert_eq!(ids(&merged), vec![3, 4]);
        assert_eq!(merged[0].machine_id, Some(m6), "adopted by reporting it");
        assert_eq!(merged[0].hostname.as_deref(), Some(HOST_B));
        assert_eq!(merged[1].machine_id, None, "an unreported untagged pane is left alone");

        let merged = manager.merge_instance_roster(&session, &b.id, vec![]).unwrap();
        assert_eq!(ids(&merged), vec![4], "once adopted, the pane is the reporter's to remove");
    }

    /// A CLI without the capability is the only instance, and nothing it
    /// reports is tagged. Each pane it reports takes its id's place under its
    /// own host, whatever the entry's tag, as a shared home's older CLI runs
    /// every pane in the `.apas`. Untagged panes and its own machine's that it
    /// did not report go. Another machine's tagged tab that it did not report,
    /// such as a separate checkout's, stays, stopped there, so it can still be
    /// started or forgotten and its id is not handed out again.
    #[test]
    fn an_older_clis_report_keeps_only_other_machines_unreported_tabs() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let older = cli(&manager, user, false);
        attached(attach(&manager, session, &older, Some(m5), Some(HOST_A)));
        manager.set_session_panes(
            &session,
            vec![
                pane(3, Some(m6), Some(HOST_B)),
                pane(5, Some(m6), Some(HOST_B)),
                pane(6, Some(m5), Some(HOST_A)),
                pane(7, None, Some("elsewhere")),
            ],
        );
        manager.set_pane_status(&session, PaneType::Interactive, 6, Some("Working...".into()));
        manager.set_pane_status(&session, PaneType::Interactive, 7, Some("Working...".into()));

        let merged = manager
            .merge_instance_roster(
                &session,
                &older.id,
                vec![pane(5, Some(m6), Some("elsewhere")), pane(4, Some(m6), Some("elsewhere"))],
            )
            .unwrap();
        assert_eq!(ids(&merged), vec![3, 5, 4]);
        assert_eq!(
            (merged[0].machine_id, merged[0].hostname.as_deref()),
            (Some(m6), Some(HOST_B)),
            "zoo-006's unreported tab stays, stopped there"
        );
        for reported in &merged[1..] {
            assert_eq!(reported.machine_id, None, "an older CLI's panes are never tagged");
            assert_eq!(reported.hostname.as_deref(), Some(HOST_A));
        }
        assert!(manager.get_pane_statuses(&session).is_empty());
        assert!(matches!(
            manager.pane_owner(&session, 3),
            PaneOwner::NotRunning { machine_id: Some(machine), .. } if machine == m6
        ));
        for pane_id in [4, 5] {
            assert!(matches!(
                manager.pane_owner(&session, pane_id),
                PaneOwner::Instance(instance) if instance.cli_id == older.id
            ));
        }
    }

    /// The server owns the order: existing panes keep their places and
    /// web-set labels, and new ones are appended.
    #[test]
    fn existing_panes_keep_their_places_and_labels() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let a = cli(&manager, user, true);
        let b = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        attached(attach(&manager, session, &b, Some(m6), Some(HOST_B)));
        let mut renamed = pane(10, Some(m6), Some(HOST_B));
        renamed.label = Some("web label".into());
        manager.set_session_panes(
            &session,
            vec![pane(30, Some(m5), Some(HOST_A)), renamed, pane(20, Some(m5), Some(HOST_A))],
        );

        let mut reported = pane(10, None, None);
        reported.label = Some("cli label".into());
        let merged = manager
            .merge_instance_roster(&session, &b.id, vec![pane(11, None, None), reported])
            .unwrap();
        assert_eq!(ids(&merged), vec![30, 10, 20, 11]);
        assert_eq!(merged[1].label.as_deref(), Some("web label"));
    }

    /// Only the reporter's removed panes lose their status.
    #[test]
    fn only_the_reporters_removed_panes_lose_their_status() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let a = cli(&manager, user, true);
        let b = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        attached(attach(&manager, session, &b, Some(m6), Some(HOST_B)));
        manager.merge_instance_roster(&session, &a.id, vec![pane(3, None, None)]);
        manager.merge_instance_roster(&session, &b.id, vec![pane(4, None, None)]);
        manager.set_pane_status(&session, PaneType::Interactive, 3, Some("Working...".into()));
        manager.set_pane_status(&session, PaneType::Interactive, 4, Some("Working...".into()));
        manager.set_pane_status(&session, PaneType::Interactive, 4, None);
        assert!(manager.get_pane_idle_since(&session).contains_key(&4));

        manager.merge_instance_roster(&session, &b.id, vec![]).unwrap();
        let statuses = manager.get_pane_statuses(&session);
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].1, 3);
        assert!(!manager.get_pane_idle_since(&session).contains_key(&4));
    }

    /// A reboot in flight keeps its pane only in its owner's report; another
    /// machine's report neither keeps nor completes it.
    #[test]
    fn a_reboot_is_kept_only_in_its_owners_report() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let a = cli(&manager, user, true);
        let b = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        attached(attach(&manager, session, &b, Some(m6), Some(HOST_B)));
        manager.merge_instance_roster(&session, &a.id, vec![pane(3, None, None)]);
        manager.merge_instance_roster(&session, &b.id, vec![pane(4, None, None)]);
        assert!(manager.begin_pane_reboot(&session, 3));

        manager.merge_instance_roster(&session, &b.id, vec![pane(4, None, None)]).unwrap();
        assert!(manager.pending_pane_reboots.contains_key(&(session, 3)));
        assert_eq!(ids(&manager.get_session_panes(&session)), vec![3, 4]);

        let during = manager.merge_instance_roster(&session, &a.id, vec![]).unwrap();
        assert_eq!(ids(&during), vec![3, 4], "the transient close keeps the tab");
        let replacement = pane(3, None, None);
        let replacement_runtime = replacement.session_id;
        let after = manager
            .merge_instance_roster(&session, &a.id, vec![replacement])
            .unwrap();
        assert_eq!(ids(&after), vec![3, 4]);
        assert_eq!(after[0].session_id, replacement_runtime);
        assert!(!manager.pending_pane_reboots.contains_key(&(session, 3)));
    }

    /// The persisted roster keeps tags and attributed hosts, so after a
    /// server restart a stopped machine's tabs are still listed under their
    /// host and an instance's report merges into them.
    #[tokio::test]
    async fn tags_survive_a_storage_reload_and_a_restart() {
        let base = std::env::temp_dir().join(format!("apas-instances-{}", Uuid::new_v4()));
        let storage = crate::storage::FileStorage::new(&base);
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let a = cli(&manager, user, true);
        let b = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        attached(attach(&manager, session, &b, Some(m6), Some(HOST_B)));
        manager.merge_instance_roster(&session, &a.id, vec![pane(3, None, None)]);
        manager.merge_instance_roster(&session, &b.id, vec![pane(4, None, None)]);
        assert!(storage
            .save_pane_roster_latest(&session, || manager.pane_roster_snapshot(&session))
            .await
            .unwrap());
        assert!(!storage
            .save_pane_roster_latest(&Uuid::new_v4(), || None)
            .await
            .unwrap());

        // A new server process: fresh storage handle and session state.
        let reloaded = crate::storage::FileStorage::new(&base)
            .load_pane_roster(&session)
            .await
            .unwrap()
            .expect("the roster was saved");
        assert_eq!(
            serde_json::to_value(&reloaded).unwrap(),
            serde_json::to_value(manager.get_session_panes(&session)).unwrap()
        );
        assert_eq!(reloaded[1].machine_id, Some(m6));
        assert_eq!(reloaded[1].hostname.as_deref(), Some(HOST_B));

        let restarted = SessionManager::new();
        let a2 = cli(&restarted, user, true);
        attached(attach(&restarted, session, &a2, Some(m5), Some(HOST_A)));
        assert!(!restarted.pane_roster_seeded(&session));
        assert_eq!(ids(&restarted.hydrate_pane_roster(&session, reloaded.clone())), vec![3, 4]);
        // A late hydrate cannot undo a report that merged first.
        restarted.merge_instance_roster(
            &session,
            &a2.id,
            vec![pane(3, None, None), pane(5, None, None)],
        );
        assert_eq!(ids(&restarted.hydrate_pane_roster(&session, reloaded)), vec![3, 4, 5]);
        assert_eq!(
            restarted.pane_owner(&session, 4),
            PaneOwner::NotRunning {
                machine_id: Some(m6),
                hostname: Some(HOST_B.to_string()),
            },
            "the stopped machine's tab stays listed under its host"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A roster persisted before panes carried hosts is attributed to the
    /// session's recorded host, so a capable instance joining from elsewhere
    /// is not handed its controls.
    #[tokio::test]
    async fn a_pre_upgrade_roster_is_attributed_to_the_recorded_host() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let laptop = cli(&manager, user, true);
        attached(attach(&manager, session, &laptop, Some(Uuid::new_v4()), Some("laptop")));
        manager.hydrate_pane_roster(
            &session,
            vec![
                pane(3, None, None),
                pane(4, None, Some("zoo-007")),
                pane(5, Some(Uuid::new_v4()), None),
            ],
        );
        assert!(matches!(
            manager.pane_owner(&session, 3),
            PaneOwner::Instance(instance) if instance.cli_id == laptop.id
        ));

        assert!(manager.stamp_untagged_pane_hosts(&session, HOST_A));
        let panes = manager.get_session_panes(&session);
        assert_eq!(panes[0].hostname.as_deref(), Some(HOST_A));
        assert_eq!(panes[1].hostname.as_deref(), Some("zoo-007"), "an attributed host stays");
        assert_eq!(panes[2].hostname, None, "a tagged pane is not attributed");
        assert!(!manager.stamp_untagged_pane_hosts(&session, HOST_A), "nothing left to stamp");
        assert!(matches!(
            manager.pane_owner(&session, 3),
            PaneOwner::NotRunning { machine_id: None, hostname: Some(host) } if host == HOST_A
        ));
    }

    /// Inventories are kept per instance machine and replayed one per
    /// attached instance, primary first.
    #[test]
    fn lifecycle_inventories_are_kept_per_machine() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let a = cli(&manager, user, true);
        let b = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        attached(attach(&manager, session, &b, Some(m6), Some(HOST_B)));
        let inventory = |hosting: bool| CliLifecycleInventory {
            persistent_terminal_hosting: hosting,
            ..CliLifecycleInventory::default()
        };
        manager.set_lifecycle_inventory(session, Some(m6), inventory(false));
        manager.set_lifecycle_inventory(session, Some(m5), inventory(true));

        let replay = manager.attached_lifecycle_inventories(&session);
        assert_eq!(
            replay
                .iter()
                .map(|(machine, inventory)| (*machine, inventory.persistent_terminal_hosting))
                .collect::<Vec<_>>(),
            vec![(Some(m5), true), (Some(m6), false)]
        );
        assert!(manager.lifecycle_inventory(&session).unwrap().persistent_terminal_hosting);

        manager.detach_cli_instance(&session, &a.id).unwrap();
        assert_eq!(manager.attached_lifecycle_inventories(&session).len(), 1);
        assert!(!manager.lifecycle_inventory(&session).unwrap().persistent_terminal_hosting);
        manager.detach_cli_instance(&session, &b.id).unwrap();
        assert!(manager.attached_lifecycle_inventories(&session).is_empty());
        assert!(
            manager.lifecycle_inventory(&session).unwrap().persistent_terminal_hosting,
            "with nothing attached, the latest reported inventory remains"
        );
    }

    /// Lifecycle progress names the instance that reported it, and a request
    /// bound to one machine ignores another machine's progress.
    #[test]
    fn lifecycle_progress_names_its_instance() {
        let manager = SessionManager::new();
        let session = Uuid::new_v4();
        let request = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let operation = CliLifecycleOperation::RebootCli;
        manager.claim_lifecycle_request(request, session, Uuid::new_v4(), operation);

        let status = manager
            .update_lifecycle_request_from_instance(
                request,
                session,
                Some(m6),
                operation,
                CliLifecyclePhase::Preparing,
                None,
                None,
            )
            .expect("the first report binds the request");
        assert!(matches!(
            status.message(),
            ServerToWeb::CliLifecycleStatus { machine_id: Some(machine), .. } if machine == m6
        ));
        assert!(manager
            .update_lifecycle_request_from_instance(
                request,
                session,
                Some(m5),
                operation,
                CliLifecyclePhase::Failed,
                None,
                None,
            )
            .is_none());
        assert!(manager
            .update_lifecycle_request_from_instance(
                request,
                Uuid::new_v4(),
                Some(m6),
                operation,
                CliLifecyclePhase::Failed,
                None,
                None,
            )
            .is_none());
    }

    /// The instance set goes only to viewers that asked for it, on every
    /// attach, detach and primary handover.
    #[test]
    fn instance_sets_reach_only_viewers_that_asked_for_them() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let capable_web = Uuid::new_v4();
        let older_web = Uuid::new_v4();
        let (capable_tx, mut capable_rx) = mpsc::channel(64);
        let (older_tx, mut older_rx) = mpsc::channel(64);
        manager.register_web(capable_web, capable_tx);
        manager.register_web(older_web, older_tx);
        manager.set_web_capabilities(capable_web, vec![PROJECT_INSTANCES_CAPABILITY.to_string()]);
        let a = cli(&manager, user, true);
        let b = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        assert!(manager.attach_web_to_session(&session, capable_web, None));
        assert!(manager.attach_web_to_session(&session, older_web, None));
        let instance_sets = |rx: &mut mpsc::Receiver<ServerToWeb>| {
            drain(rx)
                .into_iter()
                .filter_map(|message| match message {
                    ServerToWeb::SessionInstances {
                        session_id,
                        instances,
                    } => {
                        assert_eq!(session_id, session);
                        Some(
                            instances
                                .into_iter()
                                .map(|instance| (instance.machine_id, instance.primary))
                                .collect::<Vec<_>>(),
                        )
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };

        attached(attach(&manager, session, &b, Some(m6), Some(HOST_B)));
        assert_eq!(
            instance_sets(&mut capable_rx),
            vec![vec![(Some(m5), true), (Some(m6), false)]]
        );
        manager.detach_cli_instance(&session, &a.id).unwrap();
        assert_eq!(instance_sets(&mut capable_rx), vec![vec![(Some(m6), true)]]);
        manager.detach_cli_instance(&session, &b.id).unwrap();
        assert_eq!(instance_sets(&mut capable_rx), vec![Vec::new()]);
        assert!(instance_sets(&mut older_rx).is_empty(), "older builds never see the frame");
    }

    /// Machine entries say whether the daemon is connected and can run an
    /// instance beside others, and mark a project running on every machine
    /// with an attached instance, not only the primary's.
    #[test]
    fn machine_entries_report_instance_support_and_every_running_instance() {
        let manager = SessionManager::new();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        for (machine_id, host) in [(m5, HOST_A), (m6, HOST_B)] {
            let (daemon_tx, _daemon_rx) = mpsc::channel(4);
            manager.register_daemon(
                machine_id,
                user,
                daemon_tx,
                MachineInfo {
                    machine_id,
                    hostname: host.to_string(),
                    os: "linux".to_string(),
                    arch: "x86_64".to_string(),
                    daemon_version: None,
                    deepseek_backend: None,
                    last_seen: None,
                    providers: None,
                },
                vec![MachineProjectInfo {
                    project_id: "project".to_string(),
                    name: None,
                    path: format!("/checkouts/{host}"),
                    preferred_machine_id: None,
                    is_running: false,
                    pid: None,
                    memory_kb: None,
                    last_error: None,
                    git_remote: None,
                    open_state: None,
                    part: None,
                    part_reason: None,
                    other_hosts: Vec::new(),
                }],
            );
        }
        manager.set_daemon_capabilities(m5, vec![PROJECT_INSTANCES_CAPABILITY.to_string()]);
        manager.set_session_project(session, "project".to_string());
        let a = cli(&manager, user, true);
        let b = cli(&manager, user, true);
        attached(attach(&manager, session, &a, Some(m5), Some(HOST_A)));
        // A different path on its machine: the project id identifies it.
        attached(attach(&manager, session, &b, Some(m6), Some(HOST_B)));

        let machines = manager.get_machines_for_user(&user);
        let entry = |machine_id: Uuid| {
            machines
                .iter()
                .find(|machine| machine.machine.machine_id == machine_id)
                .expect("machine listed")
                .clone()
        };
        let (zoo5, zoo6) = (entry(m5), entry(m6));
        assert!(zoo5.daemon_connected && zoo5.project_instances_available);
        assert!(zoo6.daemon_connected && !zoo6.project_instances_available, "an older daemon");
        assert!(zoo5.projects[0].is_running);
        assert!(zoo6.projects[0].is_running, "the secondary instance runs it too");

        manager.unregister_daemon(&m5);
        let zoo5 = manager
            .get_machines_for_user(&user)
            .into_iter()
            .find(|machine| machine.machine.machine_id == m5)
            .unwrap();
        assert!(!zoo5.daemon_connected && !zoo5.project_instances_available);
    }
}
