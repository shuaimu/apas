use shared::{LocalProjectRegistrationResult, ServerToDaemon, ServerToWeb};
use std::time::Duration;
use uuid::Uuid;

use crate::session::PendingLocalProjectRegistration;
use crate::state::AppState;

pub(super) async fn send_failure(
    state: &AppState,
    connection_id: &Uuid,
    machine_id: Uuid,
    request_id: String,
    error: impl Into<String>,
) {
    state
        .sessions
        .send_to_web(
            connection_id,
            ServerToWeb::LocalProjectRegistered {
                machine_id,
                request_id,
                result: LocalProjectRegistrationResult::Failed {
                    error: error.into(),
                },
            },
        )
        .await;
}

async fn authorize_target(
    state: &AppState,
    connection_id: &Uuid,
    machine_id: &Uuid,
    selected_cluster: Option<&str>,
) -> anyhow::Result<Uuid> {
    let user_id = state
        .sessions
        .get_web_user(connection_id)
        .ok_or_else(|| anyhow::anyhow!("Not authenticated"))?;
    let user = state
        .db
        .get_user_by_id(&user_id.to_string())
        .await?
        .ok_or_else(|| anyhow::anyhow!("Account not found"))?;
    anyhow::ensure!(user.is_active(), "The account is suspended");
    let owner = state
        .sessions
        .daemon_owner(machine_id)
        .ok_or_else(|| anyhow::anyhow!("Machine not found"))?;
    anyhow::ensure!(
        owner == user_id,
        "Only the machine owner can register an existing local folder"
    );
    if let Some(selected_cluster) = selected_cluster {
        anyhow::ensure!(
            selected_cluster == owner.to_string(),
            "Machine does not belong to the selected cluster"
        );
    }
    anyhow::ensure!(
        state.sessions.is_daemon_connected(machine_id),
        "The selected machine is offline; reconnect it and retry manually"
    );
    anyhow::ensure!(
        state
            .sessions
            .daemon_supports_capability(machine_id, shared::LOCAL_PROJECT_REGISTRATION_CAPABILITY,),
        "Update the machine daemon before registering a local folder"
    );
    // Registration must leave the project usable by the existing explicit
    // Start gate, including its policy-capability checks.
    anyhow::ensure!(
        state
            .sessions
            .daemon_supports_capability(machine_id, shared::PROJECT_POLICY_CAPABILITY)
            && state
                .sessions
                .web_supports_capability(connection_id, shared::PROJECT_POLICY_CAPABILITY),
        "Update the web client and machine daemon to support project policy"
    );
    Ok(user_id)
}

pub(super) async fn register(
    state: &AppState,
    connection_id: &Uuid,
    machine_id: Uuid,
    selected_cluster: Option<&str>,
    path: String,
    request_id: String,
) {
    let authorization = async {
        let user_id = authorize_target(state, connection_id, &machine_id, selected_cluster).await?;
        anyhow::ensure!(
            !request_id.trim().is_empty() && request_id.len() <= 128,
            "A nonempty registration request ID of at most 128 bytes is required"
        );
        anyhow::ensure!(
            !path.contains('\0') && (path.starts_with('/') || path.starts_with("~/")),
            "Enter an absolute directory path or a ~/ path on the selected machine"
        );
        Ok::<_, anyhow::Error>(user_id)
    }
    .await;
    let user_id = match authorization {
        Ok(user_id) => user_id,
        Err(error) => {
            send_failure(
                state,
                connection_id,
                machine_id,
                request_id,
                error.to_string(),
            )
            .await;
            return;
        }
    };
    let (wire_id, pending) = match state.sessions.begin_local_project_registration(
        *connection_id,
        user_id,
        machine_id,
        request_id.clone(),
    ) {
        Ok(pending) => pending,
        Err(error) => {
            send_failure(state, connection_id, machine_id, request_id, error).await;
            return;
        }
    };
    let command = ServerToDaemon::RegisterLocalProject {
        path,
        request_id: wire_id.clone(),
    };
    // Use the captured channel, never a replacement daemon on the same machine.
    if !state
        .sessions
        .local_project_registration_is_current(&pending)
        || !matches!(
            tokio::time::timeout(Duration::from_secs(5), pending.daemon_sender.send(command)).await,
            Ok(Ok(()))
        )
    {
        state
            .sessions
            .complete_local_project_registration(
                &wire_id,
                LocalProjectRegistrationResult::Failed {
                    error: "The machine disconnected before registration could be dispatched; reconnect and retry manually".to_string(),
                },
            )
            .await;
    }
}

async fn finalize(
    state: &AppState,
    pending: &PendingLocalProjectRegistration,
    project: shared::MachineProjectInfo,
) -> anyhow::Result<shared::MachineProjectInfo> {
    anyhow::ensure!(
        !project.project_id.trim().is_empty() && project.path.starts_with('/'),
        "The daemon did not return a valid project identity and canonical path"
    );
    let _guard = state.project_operation_guard(&project.project_id).await;
    authorize_target(
        state,
        &pending.connection_id,
        &pending.machine_id,
        Some(&pending.user_id.to_string()),
    )
    .await?;
    anyhow::ensure!(
        state
            .sessions
            .local_project_registration_is_current(pending),
        "The registration connection changed; retry manually"
    );
    anyhow::ensure!(
        state
            .sessions
            .registered_local_project(&pending.machine_id, &project.project_id, &project.path)
            .is_some(),
        "The daemon has not confirmed this project and path in its inventory; retry manually"
    );
    // This gate preserves an existing owner, memberships and policy override;
    // it also denies suspended/deleting/unauthorized foreign identities.
    state
        .db
        .authorize_project_registration(&project.project_id, &pending.user_id.to_string())
        .await?;
    let policy = state
        .db
        .get_effective_project_policy(&project.project_id)
        .await?;
    anyhow::ensure!(
        !policy.project_suspended,
        "This project is suspended or deleting"
    );
    anyhow::ensure!(
        super::ws_web::has_machine_project_runtime_access(
            state,
            &pending.user_id,
            &pending.machine_id,
            &project.project_id,
        )
        .await,
        "The registered project is not authorized for an explicit Start"
    );
    // Recheck after the asynchronous DB work, and acknowledge the current raw
    // inventory rather than trusting stale runtime fields in the result.
    authorize_target(state, &pending.connection_id, &pending.machine_id, None).await?;
    let canonical = state
        .db
        .get_project(&project.project_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Canonical project metadata is unavailable"))?;
    anyhow::ensure!(
        canonical.lifecycle() == crate::db::ProjectLifecycle::Active,
        "The project is suspended or deletion is in progress"
    );
    anyhow::ensure!(
        state
            .sessions
            .local_project_registration_is_current(pending),
        "The registration connection changed; retry manually"
    );
    state
        .sessions
        .registered_local_project(&pending.machine_id, &project.project_id, &project.path)
        .ok_or_else(|| {
            anyhow::anyhow!("The registered project left the machine inventory; retry manually")
        })
}

pub(super) async fn registered(
    state: &AppState,
    machine_id: &Uuid,
    wire_id: String,
    result: LocalProjectRegistrationResult,
) {
    let Some(pending) = state
        .sessions
        .pending_local_project_registration(&wire_id, machine_id)
    else {
        tracing::warn!(%machine_id, "Ignored unsolicited or stale local registration result");
        return;
    };
    let result = match result {
        LocalProjectRegistrationResult::Registered { project } => {
            match finalize(state, &pending, project).await {
                Ok(project) => LocalProjectRegistrationResult::Registered { project },
                Err(error) => LocalProjectRegistrationResult::Failed {
                    error: error.to_string(),
                },
            }
        }
        failure @ LocalProjectRegistrationResult::Failed { .. } => failure,
    };
    state
        .sessions
        .complete_local_project_registration(&wire_id, result)
        .await;
}

#[cfg(test)]
mod tests;
