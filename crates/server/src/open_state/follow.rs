//! Bringing a machine in line with the recorded open state when its daemon
//! registers (design Decision 5).
//!
//! Registration is the one event that means "this machine was away": it was
//! offline or rebooting when the project was opened or closed, or its daemon
//! replaced itself and started again what it had been running. So this runs
//! once per registration, for the projects that registration lists, and at
//! no other time. Nothing here runs on a heartbeat or in a loop: a machine
//! that refused or failed stays as it is until its daemon registers again or
//! someone opens the project again.

use super::{
    beside_other_owners_refusal, machine_runs_project, project_tab_owners, read_open_state,
    recorded_actor_may_use_machine, refresh_basis, send_recorded_start, POLICY_INCOMPATIBLE,
};
use crate::db::{ProjectLifecycle, ProjectOpenStateRecord};
use crate::routes::ws_web::{
    beside_older_instance_refusal, machine_label, older_daemon_beside_instance_refusal,
    start_policy,
};
use crate::state::AppState;
use shared::{OpenStateSetKind, ProjectOpenState, ServerToDaemon};
use tokio::sync::mpsc;
use uuid::Uuid;

/// What the follow step did for one project on the machine that registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Followed {
    /// The machine is in line with the project's state, the project is not
    /// recorded, or the machine is not one of the project's.
    Nothing,
    /// The project is open and the machine was asked to start it.
    Started,
    /// The project is closed and the machine was asked to stop it.
    Stopped,
    /// The machine is out of line and was left as it is, for this reason,
    /// which its part now carries.
    Left(String),
}

/// Whether `machine_id` is one of the project's machines: its owner owns the
/// project or hosts it in their cluster. Another account's checkout can
/// carry the same project id, and its machine is not.
async fn machine_of_project(
    state: &AppState,
    record: &ProjectOpenStateRecord,
    machine_id: &Uuid,
) -> bool {
    let Some(owner) = state.sessions.daemon_owner(machine_id) else {
        return false;
    };
    owner.to_string() == record.owner_user_id
        || state
            .db
            .project_is_placed_in_cluster(&record.project_id, &owner.to_string())
            .await
            .unwrap_or(false)
}

/// Bring `machine_id` in line with one recorded project. The caller holds
/// the project's open-state lock and read `record` under it.
async fn follow_project_locked(
    state: &AppState,
    machine_id: &Uuid,
    record: &ProjectOpenStateRecord,
) -> Followed {
    let project_id = record.project_id.as_str();
    let administrative = record.set_kind == Some(OpenStateSetKind::Administrative);
    match record.state {
        // Nobody has recorded what this project should do, so the server
        // neither starts nor stops it.
        None => Followed::Nothing,
        Some(ProjectOpenState::Open) => {
            // Only an active project is open; one that is suspended or being
            // deleted is never started.
            if record.lifecycle != ProjectLifecycle::Active {
                return Followed::Nothing;
            }
            // Unknown owners are no owners here: with no roster held, no
            // follow step starts anything. A machine that owns no tab, and
            // is not the one the open named for a tabless project, is not
            // started either.
            let owners = project_tab_owners(state, project_id, record.stand_in_machine()).await;
            if !owners.owns(machine_id) {
                return Followed::Nothing;
            }
            if machine_runs_project(state, project_id, machine_id) {
                return Followed::Nothing;
            }
            if !machine_of_project(state, record, machine_id).await {
                return Followed::Nothing;
            }
            let refusal = match recorded_actor_may_use_machine(state, record, machine_id).await {
                Err(reason) => Some(reason),
                Ok(()) => beside_other_owners_refusal(state, &owners, machine_id)
                    .or_else(|| {
                        (!state.sessions.daemon_supports_capability(
                            machine_id,
                            shared::PROJECT_POLICY_CAPABILITY,
                        ))
                        .then(|| POLICY_INCOMPATIBLE.to_string())
                    })
                    .or_else(|| beside_older_instance_refusal(state, project_id, machine_id))
                    .or_else(|| {
                        older_daemon_beside_instance_refusal(state, project_id, machine_id)
                    }),
            };
            let policy = match refusal {
                Some(reason) => Err(reason),
                None => start_policy(state, project_id).await,
            };
            match policy {
                Err(reason) => {
                    state.sessions.record_machine_not_started(
                        project_id,
                        *machine_id,
                        reason.clone(),
                    );
                    Followed::Left(reason)
                }
                Ok(policy) => {
                    if send_recorded_start(state, project_id, machine_id, &policy).await {
                        Followed::Started
                    } else {
                        // Its connection closed meanwhile; it follows when it
                        // registers again.
                        Followed::Nothing
                    }
                }
            }
        }
        Some(ProjectOpenState::Closed) => {
            if !machine_runs_project(state, project_id, machine_id) {
                return Followed::Nothing;
            }
            // An administrative stop reaches every machine that lists the
            // project, and so does following it. A person's close reaches
            // the project's own machines, where that person may use them.
            if !administrative {
                if !machine_of_project(state, record, machine_id).await {
                    return Followed::Nothing;
                }
                if let Err(reason) = recorded_actor_may_use_machine(state, record, machine_id).await
                {
                    state.sessions.record_machine_not_stopped(
                        project_id,
                        *machine_id,
                        reason.clone(),
                    );
                    return Followed::Left(reason);
                }
            }
            if state
                .sessions
                .send_to_daemon(
                    machine_id,
                    ServerToDaemon::StopProjectCli {
                        project_id: project_id.to_string(),
                        request_id: None,
                    },
                )
                .await
            {
                state.sessions.clear_machine_outcome(project_id, machine_id);
                Followed::Stopped
            } else {
                Followed::Nothing
            }
        }
    }
}

/// The follow step for one project `machine_id`'s registration listed, under
/// the project's open-state lock and on a fresh read of its state.
/// `connection` is the daemon connection that registered: a later
/// registration replaces it and follows for itself.
pub(crate) async fn follow_project(
    state: &AppState,
    machine_id: &Uuid,
    connection: &mpsc::Sender<ServerToDaemon>,
    project_id: &str,
) -> Followed {
    let followed = {
        // Held from reading the state to the last frame sent, as an open and
        // a close hold it: a stop decided here for a closed project is never
        // sent after a later open.
        let _open_state = state.sessions.project_open_state_guard(project_id).await;
        if !state.sessions.is_daemon_connection(machine_id, connection) {
            return Followed::Nothing;
        }
        match read_open_state(state, project_id).await {
            Ok(Some(record)) => follow_project_locked(state, machine_id, &record).await,
            // No record of the project, or its state cannot be read: nothing
            // is started and nothing stopped.
            Ok(None) | Err(_) => Followed::Nothing,
        }
    };
    if followed != Followed::Nothing {
        tracing::info!(
            %project_id,
            %machine_id,
            host = %machine_label(state, *machine_id),
            ?followed,
            "machine followed the project's open state at registration"
        );
    }
    // Also what first reads the state of a project this machine brought
    // into view, so its part is reported whether or not it is recorded.
    refresh_basis(state, project_id).await;
    followed
}

/// Bring a machine whose daemon just registered in line with every recorded
/// project its `Register` frame listed: the projects in `listed`, in order,
/// one at a time. Stops as soon as `connection` is no longer that machine's
/// daemon connection.
pub(crate) async fn follow_registered_daemon(
    state: &AppState,
    machine_id: Uuid,
    connection: &mpsc::Sender<ServerToDaemon>,
    listed: Vec<String>,
) {
    let mut seen = std::collections::HashSet::new();
    for project_id in listed {
        if !seen.insert(project_id.clone()) {
            continue;
        }
        if !state.sessions.is_daemon_connection(&machine_id, connection) {
            return;
        }
        follow_project(state, &machine_id, connection, &project_id).await;
    }
}
