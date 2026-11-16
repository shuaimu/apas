//! Opening and closing a project as one recorded state.
//!
//! The state is recorded per project (`db/open_state.rs`). An open starts
//! the project on every machine that owns its tabs; a close stops it on
//! every machine that runs it. Which machines own tabs is read from the
//! persisted roster, and whether a machine can run the project beside the
//! others from that roster and its daemon's support, never from which
//! instances happen to be attached.
//!
//! Every function here that reads the recorded state and then sends a frame
//! expects its caller to hold
//! [`crate::session::SessionManager::project_open_state_guard`] from before
//! the read until after the last send. Functions named `*_locked` say so in
//! their name; the entry points in `routes/` take the lock.

use crate::db::{ProjectLifecycle, ProjectOpenStateRecord};
use crate::routes::ws_web::{
    beside_older_instance_refusal, has_machine_project_runtime_access, machine_label,
    older_daemon_beside_instance_refusal, project_stop_machines, resolve_pane_roster, start_policy,
    StopRelation,
};
use crate::session::open_state::OpenStateBasis;
pub(crate) use crate::session::open_state::TabOwner;
use crate::state::AppState;
use shared::{OpenStateSetKind, ProjectOpenState, ServerToDaemon};
use uuid::Uuid;

pub(crate) mod follow;

/// A stop naming one machine of an open project that other machines own
/// tabs of or run.
pub(crate) const RUNS_ON_OTHER_MACHINES: &str =
    "This project runs on other machines too. Close it from the project.";
/// A start, tab or task naming one machine of a closed project that other
/// machines own tabs of.
pub(crate) const CLOSED_WITH_TABS_ELSEWHERE: &str =
    "This project is closed and has tabs on other machines. Open it from the project.";
pub(crate) const PROJECT_SUSPENDED: &str =
    "This project is suspended; reactivate it before starting the CLI";
pub(crate) const PROJECT_DELETING: &str = "This project is being deleted and cannot be opened";
pub(crate) const POLICY_INCOMPATIBLE: &str =
    "The web client or machine daemon is incompatible with cluster project policy";
const STATE_UNREADABLE: &str = "Could not verify this project's open state";

/// Why a machine whose daemon lacks multi-instance support is not started
/// while another machine owns tabs of the project.
pub(crate) fn older_daemon_refusal(host: &str) -> String {
    format!(
        "{host} runs an older APAS that cannot run this project beside other machines. Reboot {host} to update it."
    )
}

/// The machines that own tabs of a project, as the roster says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TabOwners {
    /// The server holds no roster for the project, as for a session from
    /// before rosters were kept. This is not an empty roster: an open then
    /// starts the machine it names only, and no follow step starts anything.
    Unknown,
    /// The owners, sorted, machines the server knows by id first. Empty
    /// when the roster has no hosted tab and nothing stands in.
    Known(Vec<TabOwner>),
}

impl TabOwners {
    #[cfg(test)]
    pub(crate) fn is_known(&self) -> bool {
        matches!(self, Self::Known(_))
    }

    /// The owners; none when they are unknown.
    pub(crate) fn list(&self) -> &[TabOwner] {
        match self {
            Self::Known(owners) => owners,
            Self::Unknown => &[],
        }
    }

    pub(crate) fn owns(&self, machine_id: &Uuid) -> bool {
        self.list()
            .iter()
            .any(|owner| owner.machine_id == Some(*machine_id))
    }

    /// The owners other than `machine_id`. A host known only by name is
    /// always another machine: were it `machine_id`, listing the project
    /// there would have resolved it to that id.
    pub(crate) fn others_than(&self, machine_id: &Uuid) -> Vec<&TabOwner> {
        self.list()
            .iter()
            .filter(|owner| owner.machine_id != Some(*machine_id))
            .collect()
    }
}

/// The machines that own tabs of `project_id`.
///
/// Read from the roster of the project's sessions, in memory where one is
/// loaded and else as persisted; the session id is the project id, so this
/// works right after a server restart with nothing attached. A machine owns
/// a tab the roster records for it, and an untagged tab attributed to its
/// host. While the roster is known and no machine owns a tab, `stand_in`
/// (the machine an open named; see
/// [`ProjectOpenStateRecord::stand_in_machine`]) is the one owner.
pub(crate) async fn project_tab_owners(
    state: &AppState,
    project_id: &str,
    stand_in: Option<Uuid>,
) -> TabOwners {
    let mut known = false;
    let mut panes = Vec::new();
    for session_id in state.sessions.project_session_ids(project_id) {
        if let Some(roster) = resolve_pane_roster(state, &session_id).await {
            known = true;
            panes.extend(roster);
        }
    }
    if !known {
        return TabOwners::Unknown;
    }
    let places: Vec<_> = panes
        .into_iter()
        .map(|pane| (pane.machine_id, pane.hostname))
        .collect();
    let owners = state.sessions.tab_owners_at(project_id, &places, stand_in);
    TabOwners::Known(owners)
}

/// Why `machine_id` cannot run the project beside the other tab owners, or
/// `None`. Decided from the roster and the daemon's support alone: a daemon
/// without multi-instance support runs a project only as its sole runtime,
/// so it is started only when no other machine owns tabs, and is otherwise
/// sent nothing. Deciding this from the attached instances instead would let
/// such a daemon start after a server restart, before the other machines'
/// instances have attached again, and get their sessions rejected.
pub(crate) fn beside_other_owners_refusal(
    state: &AppState,
    owners: &TabOwners,
    machine_id: &Uuid,
) -> Option<String> {
    if state
        .sessions
        .daemon_supports_capability(machine_id, shared::PROJECT_INSTANCES_CAPABILITY)
        || owners.others_than(machine_id).is_empty()
    {
        return None;
    }
    Some(older_daemon_refusal(&machine_label(state, *machine_id)))
}

/// Whether an instance of the project is attached on `machine_id`.
fn instance_attached_on(state: &AppState, project_id: &str, machine_id: &Uuid) -> bool {
    state
        .sessions
        .project_session_ids(project_id)
        .iter()
        .any(|session_id| {
            state
                .sessions
                .instance_on_machine(session_id, machine_id)
                .is_some()
        })
}

/// Whether `machine_id` runs the project now: an instance of it is attached
/// there, or its connected daemon reports running it.
pub(crate) fn machine_runs_project(state: &AppState, project_id: &str, machine_id: &Uuid) -> bool {
    instance_attached_on(state, project_id, machine_id)
        || state
            .sessions
            .machines_running_project(project_id)
            .contains(machine_id)
}

/// The project's recorded open state, `Ok(None)` when the server holds no
/// record of the project, or the text to refuse with when it cannot be read.
pub(crate) async fn read_open_state(
    state: &AppState,
    project_id: &str,
) -> Result<Option<ProjectOpenStateRecord>, String> {
    state
        .db
        .get_project_open_state(project_id)
        .await
        .map_err(|error| {
            tracing::warn!(%project_id, %error, "open state lookup failed");
            STATE_UNREADABLE.to_string()
        })
}

/// Read again what deriving the project's parts takes from the database and
/// the disk (the recorded state, who set it, the accounts that host the
/// project, and the roster of each session that is not in memory), keep it,
/// and tell viewers what changed with it.
///
/// Called by every path that records the state, after it did, and where a
/// project comes into view: a daemon's registration or first listing of it,
/// a viewer's attach, an instance's `SessionStart`. Refreshes of one project
/// run one at a time, each on what is committed when it reads, so the last
/// one to finish holds the newest state. It takes neither the open-state
/// lock nor a project permit, and may be called with either held.
pub(crate) async fn refresh_basis(state: &AppState, project_id: &str) {
    let _refresh = state.sessions.open_state_basis_guard(project_id).await;
    let record = match state.db.get_project_open_state(project_id).await {
        Ok(record) => record,
        Err(error) => {
            tracing::warn!(%project_id, %error, "open state lookup failed; its report is not refreshed");
            return;
        }
    };
    let mut basis = OpenStateBasis::default();
    if let Some(record) = &record {
        basis.state = record.state;
        basis.set_kind = record.set_kind;
        basis.set_at = record.set_at.clone();
        basis.stand_in = record.stand_in_machine();
        // The system administrator is not an account and is not named.
        if let Some(actor) = record.set_by_account() {
            basis.set_by = match state.db.get_user_by_id(&actor.to_string()).await {
                Ok(user) => user.map(|user| user.email),
                Err(error) => {
                    tracing::warn!(%project_id, %error, "could not name who set the open state");
                    None
                }
            };
        }
        basis
            .hosts
            .extend(Uuid::parse_str(&record.owner_user_id).ok());
        match state.db.project_cluster_user_ids(project_id).await {
            Ok(hosts) => basis
                .hosts
                .extend(hosts.iter().filter_map(|host| Uuid::parse_str(host).ok())),
            Err(error) => {
                tracing::warn!(%project_id, %error, "could not read the clusters hosting the project")
            }
        }
    }
    for session_id in state.sessions.project_session_ids(project_id) {
        if let Some(roster) = resolve_pane_roster(state, &session_id).await {
            basis.rosters.insert(
                session_id,
                roster
                    .into_iter()
                    .map(|pane| (pane.machine_id, pane.hostname))
                    .collect(),
            );
        }
    }
    state.sessions.set_open_state_basis(project_id, basis);
}

/// Fill the recorded open state into session entries on their way to a
/// viewer: the session lists and the mobile bootstrap. An entry of a project
/// nobody has recorded carries none, which is how "not recorded" is said.
pub(crate) async fn fill_session_open_states(
    state: &AppState,
    sessions: impl Iterator<Item = &mut shared::SessionInfo>,
) {
    let recorded = match state.db.list_recorded_project_open_states().await {
        Ok(recorded) => recorded,
        Err(error) => {
            tracing::warn!(%error, "could not read the projects' open states for a session list");
            return;
        }
    };
    for session in sessions {
        let project_id = session.project_id.unwrap_or(session.id).to_string();
        session.open_state = recorded.get(&project_id).copied();
    }
}

/// [`refresh_basis`] for each project `machine_id` lists that nothing has
/// been read for yet, as when a heartbeat first lists a new project.
pub(crate) async fn load_missing_bases(state: &AppState, machine_id: &Uuid) {
    for project_id in state.sessions.machine_project_ids(machine_id) {
        if !state.sessions.has_open_state_basis(&project_id) {
            refresh_basis(state, &project_id).await;
        }
    }
}

/// Whether `user_id` may open or close `project_id` (design Decision 6).
///
/// Runtime authority is per machine and can be evaluated only for a machine
/// whose daemon is connected. So: while some machine of the project is
/// connected, the account must be able to use the project's runtime on at
/// least one of them; while none is, it must own the project or hold a
/// placement for it. A reader can therefore not record a close while the
/// machines are away and have each one stopped as it returns.
///
/// A machine of the project is one that lists it and whose owner's cluster
/// hosts it. Another account's checkout that carries the same project id is
/// not one, connected or not.
pub(crate) async fn may_set_open_state(state: &AppState, user_id: &Uuid, project_id: &str) -> bool {
    let mut any_connected = false;
    for machine_id in state.sessions.machines_listing_project(project_id) {
        if !state.sessions.is_daemon_connected(&machine_id) {
            continue;
        }
        let Some(owner) = state.sessions.daemon_owner(&machine_id) else {
            continue;
        };
        if !state
            .db
            .project_is_placed_in_cluster(project_id, &owner.to_string())
            .await
            .unwrap_or(false)
        {
            continue;
        }
        any_connected = true;
        if has_machine_project_runtime_access(state, user_id, &machine_id, project_id).await {
            return true;
        }
    }
    if any_connected {
        return false;
    }
    match state.db.get_project_open_state(project_id).await {
        Ok(Some(record)) if record.owner_user_id == user_id.to_string() => true,
        Ok(Some(_)) => state
            .db
            .project_is_placed_in_cluster(project_id, &user_id.to_string())
            .await
            .unwrap_or(false),
        _ => false,
    }
}

/// The reason a machine is left as it is because `actor` may not use the
/// project's runtime on it.
pub(crate) async fn may_not_use_reason(
    state: &AppState,
    actor: &Uuid,
    machine_id: &Uuid,
) -> String {
    let host = machine_label(state, *machine_id);
    let who = match state.db.get_user_by_id(&actor.to_string()).await {
        Ok(Some(user)) => user.email,
        _ => "The account that asked".to_string(),
    };
    format!("{who} may not use {host} for this project.")
}

/// Whether the account or administrator that set `record`'s state may have
/// `machine_id` follow it now (design Decision 6): the opener for a start,
/// the closer for a stop, checked as a direct request of theirs would be at
/// this moment. A close made by an administrative stop is followed on every
/// machine. `Err` is the reason the machine is left as it is.
pub(crate) async fn recorded_actor_may_use_machine(
    state: &AppState,
    record: &ProjectOpenStateRecord,
    machine_id: &Uuid,
) -> Result<(), String> {
    if record.state == Some(ProjectOpenState::Closed)
        && record.set_kind == Some(OpenStateSetKind::Administrative)
    {
        return Ok(());
    }
    let host = machine_label(state, *machine_id);
    let Some(actor) = record.set_by_account() else {
        return Err(format!(
            "No account that may use {host} for this project set its state."
        ));
    };
    if has_machine_project_runtime_access(state, &actor, machine_id, &record.project_id).await {
        Ok(())
    } else {
        Err(may_not_use_reason(state, &actor, machine_id).await)
    }
}

/// A machine an open or a close left as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MachineRefusal {
    pub machine_id: Uuid,
    /// Kept for the machine's part.
    pub reason: String,
    /// What a requester whose client shows no parts is told instead: the
    /// text the request was refused with before the open state existed.
    pub legacy: String,
}

/// What an open did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct OpenOutcome {
    /// The project was not recorded open before, and now is.
    pub recorded: bool,
    /// How many machines the open looked at.
    pub targets: usize,
    /// The machines sent `StartProjectCli`, and the machine the caller
    /// starts itself.
    pub asked: Vec<Uuid>,
    pub refused: Vec<MachineRefusal>,
}

/// Why the open of `project_id` by `actor` does not start `machine_id`, a
/// connected machine that owns tabs and does not run the project.
async fn open_refusal(
    state: &AppState,
    actor: &Uuid,
    project_id: &str,
    owners: &TabOwners,
    machine_id: &Uuid,
) -> Option<MachineRefusal> {
    let refusal = |reason: String| {
        Some(MachineRefusal {
            machine_id: *machine_id,
            legacy: reason.clone(),
            reason,
        })
    };
    let host = machine_label(state, *machine_id);
    if !state
        .sessions
        .machines_listing_project(project_id)
        .contains(machine_id)
    {
        return Some(MachineRefusal {
            machine_id: *machine_id,
            reason: format!("This project is not registered on {host}."),
            legacy: "Machine not found".to_string(),
        });
    }
    if !has_machine_project_runtime_access(state, actor, machine_id, project_id).await {
        return Some(MachineRefusal {
            machine_id: *machine_id,
            reason: may_not_use_reason(state, actor, machine_id).await,
            legacy: "Machine not found".to_string(),
        });
    }
    if let Some(reason) = beside_other_owners_refusal(state, owners, machine_id) {
        return refusal(reason);
    }
    if !state
        .sessions
        .daemon_supports_capability(machine_id, shared::PROJECT_POLICY_CAPABILITY)
    {
        return refusal(POLICY_INCOMPATIBLE.to_string());
    }
    // The second line, for what the roster cannot show: an older CLI that
    // runs the project and has taken every tab untagged.
    beside_older_instance_refusal(state, project_id, machine_id)
        .or_else(|| older_daemon_beside_instance_refusal(state, project_id, machine_id))
        .and_then(refusal)
}

/// Send `machine_id` a start of `project_id` and keep that it was asked.
/// The start carries an id only for a daemon that answers one. False when
/// the frame could not be handed to the daemon.
pub(crate) async fn send_recorded_start(
    state: &AppState,
    project_id: &str,
    machine_id: &Uuid,
    policy: &shared::EffectiveProjectPolicy,
) -> bool {
    let request_id = state
        .sessions
        .daemon_supports_capability(machine_id, shared::PROJECT_INSTANCES_CAPABILITY)
        .then(Uuid::new_v4);
    // Kept before the frame goes out, so the daemon's answer finds it.
    state
        .sessions
        .record_machine_start_asked(project_id, *machine_id, request_id);
    let sent = state
        .sessions
        .send_to_daemon(
            machine_id,
            ServerToDaemon::StartProjectCli {
                project_id: project_id.to_string(),
                policy: Some(policy.clone()),
                request_id,
            },
        )
        .await;
    if !sent {
        state.sessions.clear_machine_outcome(project_id, machine_id);
        return false;
    }
    // A daemon that never answers reads as not started once the time limit
    // has passed; viewers are told then, not at its next heartbeat.
    let sessions = state.sessions.clone();
    let project_id = project_id.to_string();
    tokio::spawn(async move {
        let limit = sessions.instance_start_timeout();
        tokio::time::sleep(limit + std::time::Duration::from_millis(50)).await;
        sessions.report_project_open_state(&project_id);
    });
    true
}

fn machines_for_audit(state: &AppState, machines: &[Uuid]) -> serde_json::Value {
    serde_json::Value::Array(
        machines
            .iter()
            .map(|machine_id| {
                serde_json::json!({
                    "machine_id": machine_id,
                    "hostname": state.sessions.machine_hostname(machine_id),
                })
            })
            .collect(),
    )
}

/// Write the audit record of an open or a close: who, by which path, and
/// the machines asked to start or stop. It is filed under the project, so
/// every cluster that hosts the project finds it in its history.
pub(crate) async fn audit_open_state(
    state: &AppState,
    actor_user_id: &str,
    project_id: &str,
    open_state: ProjectOpenState,
    kind: OpenStateSetKind,
    source: &str,
    machines: &[Uuid],
) {
    let action = match open_state {
        ProjectOpenState::Open => "project.opened",
        ProjectOpenState::Closed => "project.closed",
    };
    let details = serde_json::json!({
        "kind": kind,
        "source": source,
        "machines": machines_for_audit(state, machines),
    });
    if let Err(error) = state
        .db
        .record_audit(actor_user_id, action, "project", project_id, Some(details))
        .await
    {
        tracing::warn!(%project_id, %error, action, "failed to write the open-state audit record");
    }
}

/// Open `project_id` as `actor`. The caller holds the project's open-state
/// lock and has checked that `actor` may set the state.
///
/// Refused, recording nothing, for a project the server holds no record of,
/// one that is suspended or being deleted, and one whose policy cannot be
/// read. Otherwise the project is recorded open, by `actor` and naming
/// `named_machine`, unless it already was: opening an open project leaves
/// the recorded opener and machine as they were.
///
/// Then every machine that owns tabs, is connected and does not run the
/// project is started where `actor` may use it and it can run beside the
/// other owners; `named_machine` (the recorded machine, for a project
/// already open) when no machine owns a tab; and `named_machine` alone when
/// the owners are unknown. A machine left out keeps its reason for its
/// part, and never fails the request. `caller_starts` is a machine the
/// caller checks and starts itself, for a tab or task waiting on it: it is
/// sent nothing here.
pub(crate) async fn open_project_locked(
    state: &AppState,
    actor: &Uuid,
    project_id: &str,
    named_machine: Option<Uuid>,
    source: &str,
    caller_starts: Option<Uuid>,
) -> Result<OpenOutcome, String> {
    let record = read_open_state(state, project_id)
        .await?
        .ok_or_else(|| "Canonical project metadata is unavailable".to_string())?;
    match record.lifecycle {
        ProjectLifecycle::Active => {}
        ProjectLifecycle::Suspended => return Err(PROJECT_SUSPENDED.to_string()),
        ProjectLifecycle::Deleting => return Err(PROJECT_DELETING.to_string()),
    }
    let policy = start_policy(state, project_id).await?;
    let transition = record.state != Some(ProjectOpenState::Open);
    let stand_in = if transition {
        named_machine
    } else {
        record.machine_id.or(named_machine)
    };
    let owners = project_tab_owners(state, project_id, stand_in).await;
    let mut targets: Vec<Uuid> = match &owners {
        TabOwners::Unknown => named_machine.into_iter().collect(),
        TabOwners::Known(owners) => owners.iter().filter_map(|owner| owner.machine_id).collect(),
    };
    targets.sort();
    targets.dedup();

    if transition {
        state
            .db
            .set_project_open_state(
                project_id,
                ProjectOpenState::Open,
                &actor.to_string(),
                OpenStateSetKind::Request,
                named_machine.as_ref(),
            )
            .await
            .map_err(|error| {
                tracing::warn!(%project_id, %error, "failed to record the project open");
                "Could not record this project as open".to_string()
            })?;
        // What the last close left behind says nothing about this open.
        state.sessions.clear_project_outcomes(project_id);
    }

    let mut outcome = OpenOutcome {
        recorded: transition,
        targets: targets.len(),
        ..OpenOutcome::default()
    };
    outcome.asked.extend(caller_starts);
    for target in targets {
        if Some(target) == caller_starts {
            continue;
        }
        if !state.sessions.is_daemon_connected(&target) {
            // It is started when its daemon registers; until then nothing
            // the server last heard from it applies.
            state.sessions.clear_machine_outcome(project_id, &target);
            continue;
        }
        if machine_runs_project(state, project_id, &target) {
            // Nothing changes on a machine that runs it.
            state.sessions.record_machine_started(project_id, target);
            continue;
        }
        if let Some(refusal) = open_refusal(state, actor, project_id, &owners, &target).await {
            state
                .sessions
                .record_machine_not_started(project_id, target, refusal.reason.clone());
            outcome.refused.push(refusal);
            continue;
        }
        if send_recorded_start(state, project_id, &target, &policy).await {
            outcome.asked.push(target);
        } else {
            let host = machine_label(state, target);
            let reason = format!("{host} did not take the request to start this project.");
            if state.sessions.is_daemon_connected(&target) {
                state
                    .sessions
                    .record_machine_not_started(project_id, target, reason.clone());
            }
            outcome.refused.push(MachineRefusal {
                machine_id: target,
                reason,
                legacy: "Daemon is offline".to_string(),
            });
        }
    }
    if transition || !outcome.asked.is_empty() {
        audit_open_state(
            state,
            &actor.to_string(),
            project_id,
            ProjectOpenState::Open,
            OpenStateSetKind::Request,
            source,
            &outcome.asked,
        )
        .await;
    }
    tracing::info!(
        %project_id,
        %actor,
        source,
        recorded = transition,
        asked = outcome.asked.len(),
        refused = outcome.refused.len(),
        "project opened"
    );
    refresh_basis(state, project_id).await;
    Ok(outcome)
}

/// What a close did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CloseOutcome {
    /// The project was not recorded closed before, and now is.
    pub recorded: bool,
    /// How many machines the close looked at.
    pub targets: usize,
    /// The machines sent `StopProjectCli`.
    pub asked: Vec<Uuid>,
    /// The machines left running, with why.
    pub refused: Vec<MachineRefusal>,
}

/// The machines a close stops: every machine with an instance of the
/// project attached, every machine of the project's accounts whose connected
/// daemon reports running it, and every machine the server asked to start
/// it and has not heard from. `named_machine` is in the list only when it is
/// one of those, or is connected and may be stopped by `actor`: a close sent
/// from a machine's entry right after a start reaches that machine before
/// its daemon reports the project running.
async fn close_targets(
    state: &AppState,
    actor: &Uuid,
    project_id: &str,
    named_machine: Uuid,
) -> Vec<Uuid> {
    let mut targets = Vec::new();
    for (machine_id, relation) in project_stop_machines(state, project_id, named_machine).await {
        if relation != StopRelation::Named
            || (state.sessions.is_daemon_connected(&machine_id)
                && has_machine_project_runtime_access(state, actor, &machine_id, project_id).await)
        {
            targets.push(machine_id);
        }
    }
    let session_ids = state.sessions.project_session_ids(project_id);
    let starting = state
        .sessions
        .machines_with_pending_instance_start(&session_ids)
        .into_iter()
        .chain(
            state
                .sessions
                .project_machine_outcomes(project_id)
                .into_iter()
                .filter(|(_, outcome)| outcome.is_start_pending())
                .map(|(machine_id, _)| machine_id),
        );
    for machine_id in starting {
        if !targets.contains(&machine_id) {
            targets.push(machine_id);
        }
    }
    targets
}

/// Close `project_id` as `actor`. The caller holds the project's open-state
/// lock and has checked that `actor` may set the state.
///
/// The project is recorded closed by `actor`, unless it already is: a
/// repeated close leaves the recorded closer as it was. Then every
/// connected machine that runs the project, has an instance of it attached
/// or was asked to start it is sent the stop a project stop always sent,
/// where `actor` may use it; a machine `actor` may not use keeps running
/// and keeps the reason for its part.
///
/// A close sends nothing but `StopProjectCli`. It forgets no machine's tabs,
/// closes no tab and deletes nothing, on the server or on any machine.
pub(crate) async fn close_project_locked(
    state: &AppState,
    actor: &Uuid,
    project_id: &str,
    named_machine: Uuid,
    source: &str,
) -> Result<CloseOutcome, String> {
    let record = read_open_state(state, project_id)
        .await?
        .ok_or_else(|| "Machine not found".to_string())?;
    let transition = record.state != Some(ProjectOpenState::Closed);
    // Found before anything is cleared: a start still pending is stopped.
    let targets = close_targets(state, actor, project_id, named_machine).await;
    if transition {
        match state
            .db
            .set_project_open_state(
                project_id,
                ProjectOpenState::Closed,
                &actor.to_string(),
                OpenStateSetKind::Request,
                None,
            )
            .await
        {
            Ok(true) => {}
            Ok(false) => return Err("Machine not found".to_string()),
            Err(error) => {
                tracing::warn!(%project_id, %error, "failed to record the project closed");
                return Err("Could not record this project as closed".to_string());
            }
        }
    }
    state.sessions.clear_project_outcomes(project_id);

    let mut outcome = CloseOutcome {
        recorded: transition,
        targets: targets.len(),
        ..CloseOutcome::default()
    };
    for target in targets {
        if !state.sessions.is_daemon_connected(&target) {
            // It is stopped when its daemon registers still running it.
            continue;
        }
        if !has_machine_project_runtime_access(state, actor, &target, project_id).await {
            let reason = may_not_use_reason(state, actor, &target).await;
            state
                .sessions
                .record_machine_not_stopped(project_id, target, reason.clone());
            outcome.refused.push(MachineRefusal {
                machine_id: target,
                reason,
                legacy: "Machine not found".to_string(),
            });
            continue;
        }
        if state
            .sessions
            .send_to_daemon(
                &target,
                ServerToDaemon::StopProjectCli {
                    project_id: project_id.to_string(),
                    request_id: None,
                },
            )
            .await
        {
            outcome.asked.push(target);
        } else {
            let host = machine_label(state, target);
            outcome.refused.push(MachineRefusal {
                machine_id: target,
                reason: format!("{host} did not take the request to stop this project."),
                legacy: "Daemon is offline".to_string(),
            });
        }
    }
    if transition || !outcome.asked.is_empty() {
        audit_open_state(
            state,
            &actor.to_string(),
            project_id,
            ProjectOpenState::Closed,
            OpenStateSetKind::Request,
            source,
            &outcome.asked,
        )
        .await;
    }
    tracing::info!(
        %project_id,
        %actor,
        source,
        recorded = transition,
        asked = outcome.asked.len(),
        refused = outcome.refused.len(),
        "project closed"
    );
    refresh_basis(state, project_id).await;
    Ok(outcome)
}

/// Whether a machine other than `machine_id` owns tabs of the project, runs
/// it, has an instance of it attached or is being started for it: what
/// keeps a stop naming `machine_id` alone from closing an open project.
pub(crate) async fn other_machines_own_or_run(
    state: &AppState,
    record: &ProjectOpenStateRecord,
    machine_id: &Uuid,
) -> bool {
    let project_id = record.project_id.as_str();
    let owners = project_tab_owners(state, project_id, record.stand_in_machine()).await;
    if !owners.others_than(machine_id).is_empty() {
        return true;
    }
    if project_stop_machines(state, project_id, *machine_id)
        .await
        .iter()
        .any(|(other, relation)| other != machine_id && *relation != StopRelation::Named)
    {
        return true;
    }
    let session_ids = state.sessions.project_session_ids(project_id);
    state
        .sessions
        .machines_with_pending_instance_start(&session_ids)
        .iter()
        .any(|other| other != machine_id)
        || state
            .sessions
            .project_machine_outcomes(project_id)
            .iter()
            .any(|(other, outcome)| other != machine_id && outcome.is_start_pending())
}

/// Whether a machine other than `machine_id` owns tabs of the project: what
/// keeps a start, tab or task naming `machine_id` alone from opening a
/// closed project. Unknown owners are no owners here: opening then starts
/// the named machine only, which is all such a request may do.
pub(crate) async fn other_machines_own_tabs(
    state: &AppState,
    project_id: &str,
    machine_id: &Uuid,
) -> bool {
    !project_tab_owners(state, project_id, None)
        .await
        .others_than(machine_id)
        .is_empty()
}

/// Admit a tab or task that needs `machine_id` started for it, after the
/// caller's own checks for that machine have passed and before it sends the
/// start. The caller holds the project's open-state lock and keeps it until
/// that start is sent.
///
/// From a viewer that negotiated the open state (`knows_open_state`), the
/// project is opened as `actor`: recorded open, and the other machines that
/// own tabs started as an open starts them. From any other client the
/// request reaches no machine but the one it names (design Decision 3): a
/// project that is not recorded or is open is left as it is, and a closed
/// one is opened only when no other machine owns tabs of it, else refused.
/// `Err` is the refusal; nothing was recorded and no machine was started.
pub(crate) async fn admit_start_for_new_work(
    state: &AppState,
    actor: &Uuid,
    project_id: &str,
    machine_id: Uuid,
    knows_open_state: bool,
    source: &str,
) -> Result<(), String> {
    let Some(record) = read_open_state(state, project_id).await? else {
        // No record of the project: as before the open state.
        return Ok(());
    };
    if !knows_open_state {
        match record.state {
            None | Some(ProjectOpenState::Open) => return Ok(()),
            Some(ProjectOpenState::Closed) => {
                if other_machines_own_tabs(state, project_id, &machine_id).await {
                    return Err(CLOSED_WITH_TABS_ELSEWHERE.to_string());
                }
            }
        }
    }
    open_project_locked(
        state,
        actor,
        project_id,
        Some(machine_id),
        source,
        Some(machine_id),
    )
    .await
    .map(|_| ())
}

/// A creation that starts the project records it open, by its creator and
/// naming the machine it starts on, so a new project is followed from its
/// first day. The caller holds the project's open-state lock. A project
/// already recorded open keeps its opener.
pub(crate) async fn record_created_open(
    state: &AppState,
    creator: &Uuid,
    project_id: &str,
    machine_id: Option<Uuid>,
    source: &str,
) {
    match state.db.get_project_open_state(project_id).await {
        Ok(Some(record)) if record.state == Some(ProjectOpenState::Open) => return,
        Ok(Some(record)) if record.lifecycle != ProjectLifecycle::Active => return,
        Ok(Some(_)) => {}
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(%project_id, %error, "open state lookup failed for a new project");
            return;
        }
    }
    match state
        .db
        .set_project_open_state(
            project_id,
            ProjectOpenState::Open,
            &creator.to_string(),
            OpenStateSetKind::Request,
            machine_id.as_ref(),
        )
        .await
    {
        Ok(true) => {
            state.sessions.clear_project_outcomes(project_id);
            let machines: Vec<Uuid> = machine_id.into_iter().collect();
            audit_open_state(
                state,
                &creator.to_string(),
                project_id,
                ProjectOpenState::Open,
                OpenStateSetKind::Request,
                source,
                &machines,
            )
            .await;
            tracing::info!(%project_id, %creator, source, "new project recorded open");
            refresh_basis(state, project_id).await;
        }
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(%project_id, %error, "failed to record a new project open");
        }
    }
}

/// The machines an administrative stop reaches: every connected machine
/// whose inventory lists the project. Sorted.
pub(crate) fn administrative_stop_machines(state: &AppState, project_id: &str) -> Vec<Uuid> {
    let mut machines: Vec<Uuid> = state
        .sessions
        .machines_listing_project(project_id)
        .into_iter()
        .filter(|machine_id| state.sessions.is_daemon_connected(machine_id))
        .collect();
    machines.sort();
    machines
}

/// An operator's or the system administrator's "Stop runtime": record the
/// project closed by `actor_user_id` (an account id, or
/// [`crate::db::SYSTEM_ADMIN_ACTOR`]) as an administrative stop, so the next
/// registration does not start what was just stopped. Recorded again even
/// when the project was closed already: an administrative stop is followed
/// on every machine, a member's close only where that member may. The
/// caller holds the project's open-state lock and then stops the runtime.
/// `Ok(false)` when the server holds no record of the project.
pub(crate) async fn record_administrative_close(
    state: &AppState,
    actor_user_id: &str,
    project_id: &str,
    source: &str,
) -> anyhow::Result<bool> {
    let recorded = state
        .db
        .set_project_open_state(
            project_id,
            ProjectOpenState::Closed,
            actor_user_id,
            OpenStateSetKind::Administrative,
            None,
        )
        .await?;
    if recorded {
        note_administrative_close(state, actor_user_id, project_id, source).await;
    }
    Ok(recorded)
}

/// The rest of an administrative close whose state the database already
/// recorded in its own transaction (a suspension, the start of a deletion):
/// drop what was held for the project's machines and write the audit record.
pub(crate) async fn note_administrative_close(
    state: &AppState,
    actor_user_id: &str,
    project_id: &str,
    source: &str,
) {
    state.sessions.clear_project_outcomes(project_id);
    let machines = administrative_stop_machines(state, project_id);
    audit_open_state(
        state,
        actor_user_id,
        project_id,
        ProjectOpenState::Closed,
        OpenStateSetKind::Administrative,
        source,
        &machines,
    )
    .await;
    refresh_basis(state, project_id).await;
}
