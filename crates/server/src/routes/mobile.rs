use axum::{extract::State, http::HeaderMap, Json};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use shared::{
    MobileBootstrapResponse, MobileLaunchProfile, MobileLaunchTarget, MobileSessionSummary,
    MobileTaskLaunchRequest, MobileTaskLaunchResponse, PaneConfig, PaneKind, PlanReviewMode,
    ServerToCli, ServerToDaemon, SessionInfo, MOBILE_PROTOCOL_MAX_VERSION,
    MOBILE_PROTOCOL_MIN_VERSION,
};
use std::time::Duration;
use uuid::Uuid;

use crate::{
    db::Session,
    error::AppError,
    routes::{
        auth::require_active_claims, authz::require_active_user, mobile_auth::claims_from_headers,
    },
    session::pane_launches::{native_task_pane_id, InstanceStartOutcome, InstanceStartWait},
    state::AppState,
};

type HmacSha256 = Hmac<Sha256>;

fn session_info(
    state: &AppState,
    session: Session,
    shared: bool,
    owner_email: Option<String>,
) -> SessionInfo {
    let session_id = Uuid::parse_str(&session.id).unwrap_or_default();
    let is_active = state.sessions.is_session_active(&session_id);
    let working = state.sessions.get_pane_statuses(&session_id);
    SessionInfo {
        id: session_id,
        project_id: session
            .project_id
            .as_deref()
            .and_then(|id| Uuid::parse_str(id).ok())
            .or(Some(session_id)),
        cli_client_id: session
            .cli_client_id
            .and_then(|id| Uuid::parse_str(&id).ok()),
        working_dir: session.working_dir,
        hostname: session.hostname,
        git_remote: session.git_remote,
        git_remote_url: session.git_remote_url,
        status: session.status,
        created_at: session.created_at,
        is_shared: shared,
        owner_email,
        share_role: Some(if shared { "user" } else { "owner" }.to_string()),
        is_active,
        is_working: is_active
            && working
                .iter()
                .any(|(_, _, status)| shared::pane_status_is_working(status)),
        // Same map the flag above is derived from, so "this session is working"
        // and "these panes are working" cannot disagree — and the same helper
        // the web list uses, so the two surfaces cannot either.
        panes: super::ws_web::pane_summaries(state, &session_id, is_active),
        instances: Vec::new(),
        open_state: None,
    }
}

fn project_name(session: &SessionInfo) -> Option<String> {
    session
        .git_remote
        .as_deref()
        .and_then(|remote| remote.rsplit('/').next())
        .or_else(|| {
            session
                .working_dir
                .as_deref()
                .and_then(|path| path.trim_end_matches('/').rsplit('/').next())
        })
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
}

/// `info` with every attached instance of its project, each with whether
/// `viewer` may use it. Older native builds never receive the instance-set
/// frame, so this list is how they learn every host a project runs on.
async fn with_instances(
    state: &AppState,
    mut info: SessionInfo,
    viewer: Option<Uuid>,
) -> SessionInfo {
    info.instances = match viewer {
        Some(viewer) => super::ws_web::viewer_session_instances(state, &info.id, &viewer).await,
        None => state.sessions.session_instances(&info.id),
    };
    info
}

async fn accessible_sessions(
    state: &AppState,
    user_id: &str,
) -> Result<Vec<MobileSessionSummary>, AppError> {
    let viewer = Uuid::parse_str(user_id).ok();
    let mut summaries = Vec::new();
    for session in state.db.get_sessions_for_user(user_id).await? {
        let last_user_input_at = state.db.get_session_last_user_input_at(&session.id).await?;
        let latest_update_at = session
            .updated_at
            .clone()
            .or_else(|| session.created_at.clone());
        let info = with_instances(state, session_info(state, session, false, None), viewer).await;
        let attention_count = info
            .panes
            .iter()
            .filter(|pane| pane.awaiting_answer)
            .count() as u32;
        summaries.push(MobileSessionSummary {
            project_name: project_name(&info),
            latest_update_at,
            last_user_input_at,
            latest_summary: None,
            attention_count,
            session: info,
        });
    }
    for (session, owner_email, _) in state.db.get_shared_sessions_for_user(user_id).await? {
        let last_user_input_at = state.db.get_session_last_user_input_at(&session.id).await?;
        let latest_update_at = session
            .updated_at
            .clone()
            .or_else(|| session.created_at.clone());
        let info = with_instances(
            state,
            session_info(state, session, true, Some(owner_email)),
            viewer,
        )
        .await;
        let attention_count = info
            .panes
            .iter()
            .filter(|pane| pane.awaiting_answer)
            .count() as u32;
        summaries.push(MobileSessionSummary {
            project_name: project_name(&info),
            latest_update_at,
            last_user_input_at,
            latest_summary: None,
            attention_count,
            session: info,
        });
    }
    crate::open_state::fill_session_open_states(
        state,
        summaries.iter_mut().map(|summary| &mut summary.session),
    )
    .await;
    summaries.sort_by(|left, right| {
        right
            .last_user_input_at
            .cmp(&left.last_user_input_at)
            .then_with(|| right.latest_update_at.cmp(&left.latest_update_at))
    });
    Ok(summaries)
}

fn mobile_launch_profiles(policy: &shared::EffectiveProjectPolicy) -> Vec<MobileLaunchProfile> {
    shared::supported_launch_profiles()
        .into_iter()
        .filter(|profile| {
            profile.kind == PaneKind::Terminal
                && policy.allowed_launch_profiles.contains(&profile.key)
        })
        .map(|profile| MobileLaunchProfile {
            key: profile.key,
            label: profile.label,
            kind: profile.kind,
            provider: profile.provider,
            mode: shared::PaneMode::Interactive,
            model: profile.model,
        })
        .collect()
}

fn mobile_launch_capability_error(
    provider: shared::Provider,
    supports_mobile_launch: bool,
    supports_opencode_terminal: bool,
    supports_omp_terminal: bool,
) -> Option<&'static str> {
    if !supports_mobile_launch {
        return Some("The project CLI must be updated and reconnected before mobile task launch");
    }
    if provider == shared::Provider::Opencode && !supports_opencode_terminal {
        return Some(
            "The project CLI must be updated and reconnected before launching an OpenCode task",
        );
    }
    if shared::is_retired_provider(provider) {
        // Unreachable through the catalog, which no longer lists a retired
        // provider; kept so this function alone cannot let one through.
        return Some(shared::retired_launch_message(provider));
    }
    if provider == shared::Provider::Omp && !supports_omp_terminal {
        return Some(
            "The project CLI must be updated and reconnected before launching an OMP task",
        );
    }
    None
}

async fn launch_targets(
    state: &AppState,
    machines: &[shared::MachineWithProjects],
) -> Vec<MobileLaunchTarget> {
    let mut targets = Vec::new();
    for machine in machines {
        for project in &machine.projects {
            let Ok(policy) = state
                .db
                .get_effective_project_policy(&project.project_id)
                .await
            else {
                continue;
            };
            if policy.project_suspended {
                continue;
            }
            let profiles = mobile_launch_profiles(&policy);
            if profiles.is_empty() {
                continue;
            }
            targets.push(MobileLaunchTarget {
                machine_id: machine.machine.machine_id,
                hostname: machine.machine.hostname.clone(),
                project_id: project.project_id.clone(),
                project_name: project
                    .name
                    .clone()
                    .unwrap_or_else(|| project.project_id.clone()),
                instance_path: project.path.clone(),
                online: true,
                profiles,
            });
        }
    }
    targets
}

fn launch_fingerprint(
    request: &MobileTaskLaunchRequest,
    instruction: &str,
    secret: &str,
) -> Result<String, AppError> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| AppError::Internal("Invalid task-launch key".to_string()))?;
    mac.update(request.machine_id.as_bytes());
    mac.update(&[0]);
    mac.update(request.project_id.as_bytes());
    mac.update(&[0]);
    mac.update(request.profile_key.as_bytes());
    mac.update(&[0]);
    mac.update(instruction.as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

fn completed_launch(
    record: &crate::db::MobileTaskLaunchRecord,
) -> Result<MobileTaskLaunchResponse, AppError> {
    let session_id = record
        .session_id
        .as_deref()
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| AppError::Internal("Stored task launch has no session".to_string()))?;
    Ok(MobileTaskLaunchResponse {
        request_id: Uuid::parse_str(&record.request_id)
            .map_err(|_| AppError::Internal("Stored task launch is invalid".to_string()))?,
        session_id,
        pane_id: record.pane_id.and_then(|value| u32::try_from(value).ok()),
        status: "acknowledged".to_string(),
    })
}

/// The answer to a launch that is still starting. The request stays pending,
/// and a retry with the same request id carries on where this one stopped.
const STILL_STARTING: &str =
    "The project runtime is still starting; retry this same submission shortly";
const MACHINE_WENT_OFFLINE: &str = "The selected machine went offline; reconnect it and retry";

/// Record that the launch failed, so every retry of it gets the same
/// answer, and return that answer.
async fn fail_task_launch(
    state: &AppState,
    request: &MobileTaskLaunchRequest,
    user_id: &str,
    message: impl Into<String>,
) -> AppError {
    let message = message.into();
    match state
        .db
        .fail_mobile_task_launch(&request.request_id.to_string(), user_id, &message)
        .await
    {
        Ok(()) => AppError::Conflict(message),
        Err(error) => error.into(),
    }
}

/// Admit a native task launch that needs its chosen machine started. A
/// native build shows nothing about a project's open state, so the launch
/// never starts a machine other than the one it names (design Decision 3):
/// a project that is not recorded or is open is left as it is, and a closed
/// one is opened only when no other machine owns tabs of it, else the launch
/// is refused with the reason and nothing starts. The caller holds the
/// project's open-state lock until it has sent the start. `caller_starts`
/// says the caller sends the chosen machine's start itself.
async fn admit_task_start(
    state: &AppState,
    user_id: &str,
    request: &MobileTaskLaunchRequest,
    caller_starts: bool,
) -> Result<(), String> {
    let actor = Uuid::parse_str(user_id).map_err(|_| "Invalid user identifier".to_string())?;
    crate::open_state::admit_start_for_new_work(
        state,
        &actor,
        &request.project_id,
        request.machine_id,
        false,
        "task",
        caller_starts,
    )
    .await
}

/// Admit a native task launch that needs no machine started: the project
/// already runs where the task will go. An instance can be attached in a
/// project recorded closed (a close left the machine running, could not
/// reach its daemon, or its stop is still taking effect), and the closed row
/// of design Decision 3 applies then too: the launch is refused when other
/// machines own tabs, and otherwise opens the project. A project that is
/// open or not recorded is left exactly as it is.
///
/// Opening the project this way takes the authority to set its state, as
/// the project-wide Open does: the chosen machine's own check still passes
/// while its daemon's channel is away, and an account whose Open would be
/// refused then must not record the project open through a task. It is
/// refused as that Open is.
async fn admit_task_where_it_runs(
    state: &AppState,
    user_id: &str,
    request: &MobileTaskLaunchRequest,
) -> Result<(), String> {
    let _open_state = state
        .sessions
        .project_open_state_guard(&request.project_id)
        .await;
    if !crate::open_state::is_recorded_closed(state, &request.project_id).await {
        return Ok(());
    }
    let actor = Uuid::parse_str(user_id).map_err(|_| "Invalid user identifier".to_string())?;
    if !crate::open_state::may_set_open_state(state, &actor, &request.project_id).await {
        return Err("Machine not found".to_string());
    }
    admit_task_start(state, user_id, request, false).await
}

/// Why a native launch does not start the chosen machine, whose daemon lacks
/// multi-instance support, in a project recorded open: another machine owns
/// tabs of it. `None` for a project that is closed or not recorded, whose
/// rule [`admit_task_start`] applies. The caller holds the project's
/// open-state lock.
async fn older_daemon_beside_owners_refusal(
    state: &AppState,
    request: &MobileTaskLaunchRequest,
) -> Option<String> {
    let record = crate::open_state::read_open_state(state, &request.project_id)
        .await
        .ok()
        .flatten()?;
    if record.state != Some(shared::ProjectOpenState::Open) {
        return None;
    }
    let owners = crate::open_state::project_tab_owners(
        state,
        &request.project_id,
        record.stand_in_machine(),
    )
    .await;
    crate::open_state::beside_other_owners_refusal(state, &owners, &request.machine_id)
}

/// The chosen machine's instance of the project, for a launch on a project
/// an instance that can share it runs, or will run. One registered there but
/// not running is started, as a web `AddPane` naming that machine starts it.
/// Machines that cannot run an instance beside the others fail the launch.
/// `Err` is the HTTP answer: a failure recorded on the request, or a
/// still-starting conflict that a retry of the same request continues.
async fn task_launch_instance(
    state: &AppState,
    user_id: &str,
    request: &MobileTaskLaunchRequest,
    session_id: Uuid,
) -> Result<crate::session::InstanceAttachment, AppError> {
    let machine_id = request.machine_id;
    let (wait, start_policy, open_state_guard) = if state
        .sessions
        .instance_on_machine(&session_id, &machine_id)
        .is_some()
    {
        if let Err(reason) = admit_task_where_it_runs(state, user_id, request).await {
            return Err(fail_task_launch(state, request, user_id, reason).await);
        }
        // Wait only for a start of it the server has not finished: before
        // then its CLI holds no policy and would refuse the task.
        let wait = state
            .sessions
            .wait_for_instance_start(session_id, machine_id, None);
        (wait, None, None)
    } else {
        if !state.sessions.is_daemon_connected(&machine_id) {
            return Err(fail_task_launch(state, request, user_id, MACHINE_WENT_OFFLINE).await);
        }
        let host = state
            .sessions
            .machine_hostname(&machine_id)
            .unwrap_or_else(|| "the selected machine".to_string());
        // Every instance runs under the account the session belongs to, and
        // another account's instance is refused when it starts.
        if let (Some(row), Some(owner)) = (
            state.db.get_session(&session_id.to_string()).await?,
            state.sessions.daemon_owner(&machine_id),
        ) {
            if row.user_id != owner.to_string() {
                return Err(fail_task_launch(
                    state,
                    request,
                    user_id,
                    format!(
                        "{host} is in another account's cluster, so it cannot run an instance of this project"
                    ),
                )
                .await);
            }
        }
        if !state
            .sessions
            .daemon_supports_capability(&machine_id, shared::PROJECT_INSTANCES_CAPABILITY)
        {
            return Err(fail_task_launch(
                state,
                request,
                user_id,
                format!(
                    "{host} runs an older APAS that cannot run this project beside other machines. Reboot {host} to update it."
                ),
            )
            .await);
        }
        let policy = state
            .db
            .get_effective_project_policy(&request.project_id)
            .await?;
        // The machine may be started for this task. A native launch names
        // one machine and reaches no other; see `admit_task_start`. Held
        // until that start has been sent.
        let open_state_guard = state
            .sessions
            .project_open_state_guard(&request.project_id)
            .await;
        if let Err(reason) = admit_task_start(state, user_id, request, true).await {
            return Err(fail_task_launch(state, request, user_id, reason).await);
        }
        let wait = state.sessions.wait_for_instance_start(
            session_id,
            machine_id,
            Some(&request.project_id),
        );
        (wait, Some(policy), Some(open_state_guard))
    };
    // A request dropped from here on, because its client went away, must not
    // leave its wait behind.
    let _withdraw = match &wait {
        InstanceStartWait::Waiting { waiter, .. } => Some(state.sessions.withdraw_on_drop(*waiter)),
        InstanceStartWait::Ready => None,
    };
    if let (
        InstanceStartWait::Waiting {
            start: Some(start_request),
            ..
        },
        Some(policy),
    ) = (&wait, start_policy)
    {
        // Kept before the frame goes out, so the daemon's answer finds it.
        state.sessions.record_machine_start_asked(
            &request.project_id,
            machine_id,
            Some(*start_request),
        );
        if !state
            .sessions
            .send_to_daemon(
                &machine_id,
                ServerToDaemon::StartProjectCli {
                    project_id: request.project_id.clone(),
                    policy: Some(policy),
                    request_id: Some(*start_request),
                },
            )
            .await
        {
            state
                .sessions
                .clear_machine_outcome(&request.project_id, &machine_id);
            state
                .sessions
                .fail_instance_start(session_id, machine_id, *start_request);
            return Err(fail_task_launch(state, request, user_id, MACHINE_WENT_OFFLINE).await);
        }
    }
    drop(open_state_guard);
    if let InstanceStartWait::Waiting {
        waiter, outcome, ..
    } = wait
    {
        // The window a launch always had to find its project running.
        let window = state
            .sessions
            .instance_start_timeout()
            .min(Duration::from_secs(15));
        match state
            .sessions
            .await_instance_start(waiter, outcome, window)
            .await
        {
            Some(InstanceStartOutcome::Started) => {}
            Some(InstanceStartOutcome::Refused(reason)) => {
                return Err(fail_task_launch(state, request, user_id, reason).await)
            }
            Some(InstanceStartOutcome::DaemonGone) => {
                return Err(fail_task_launch(state, request, user_id, MACHINE_WENT_OFFLINE).await)
            }
            None => return Err(AppError::Conflict(STILL_STARTING.to_string())),
        }
    }
    state
        .sessions
        .instance_on_machine(&session_id, &machine_id)
        .ok_or_else(|| AppError::Conflict(STILL_STARTING.to_string()))
}

pub async fn launch_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MobileTaskLaunchRequest>,
) -> Result<Json<MobileTaskLaunchResponse>, AppError> {
    if !state.config.mobile.features.coding_mutations {
        return Err(AppError::Forbidden(
            "Mobile coding actions are disabled".to_string(),
        ));
    }
    let claims = claims_from_headers(&headers, &state)?;
    let user = require_active_claims(&state, &claims).await?;
    let device_session_id = claims
        .device_session_id
        .as_deref()
        .ok_or_else(|| AppError::Forbidden("Mobile access token required".to_string()))?;
    let instruction = request.instruction.trim();
    if instruction.is_empty() || instruction.len() > 16_000 {
        return Err(AppError::BadRequest(
            "Instruction must contain between 1 and 16000 characters".to_string(),
        ));
    }

    // Recompute the eligible catalog for every submission; bootstrap values
    // are hints and can be stale by the time the user confirms the draft.
    let user_id = Uuid::parse_str(&user.id)
        .map_err(|_| AppError::Internal("Invalid user identifier".to_string()))?;
    let machines = super::ws_web::list_accessible_machines_for_user(&state, &user_id).await;
    let targets = launch_targets(&state, &machines).await;
    let target = targets
        .iter()
        .find(|target| {
            target.machine_id == request.machine_id && target.project_id == request.project_id
        })
        .ok_or_else(|| {
            AppError::Conflict(
                "The selected project target is no longer available; refresh and choose another target"
                    .to_string(),
            )
        })?;
    let profile = target
        .profiles
        .iter()
        .find(|profile| profile.key == request.profile_key)
        .cloned()
        .ok_or_else(|| {
            // A build that still holds a profile list from before a
            // retirement sends the retired key: say what to use instead.
            AppError::Conflict(
                shared::retired_launch_profile_message(&request.profile_key)
                    .unwrap_or(
                        "The selected coding profile is no longer allowed; choose another profile",
                    )
                    .to_string(),
            )
        })?;
    let _launch_guard = state.mobile_task_launch_guard(request.request_id).await;
    // A launch starts the project when nothing runs it, so a project the
    // requester's own machine lists but that has no placement in the
    // requester's cluster yet is placed first, as a web Start does. This
    // needs a project record: one that `apas register` created and that has
    // never run has none, so it has no policy, `launch_targets` never offers
    // it, and it is refused above. A web Start of it creates the record.
    super::ws_web::place_registered_project(
        &state,
        &user_id,
        &request.machine_id,
        &request.project_id,
    )
    .await;
    if !super::ws_web::has_machine_project_runtime_access(
        &state,
        &user_id,
        &request.machine_id,
        &request.project_id,
    )
    .await
    {
        return Err(AppError::Forbidden(
            "The selected project is no longer runnable on that machine".to_string(),
        ));
    }

    let fingerprint = launch_fingerprint(&request, instruction, &state.config.auth.jwt_secret)?;
    let record = state
        .db
        .claim_mobile_task_launch(
            &request.request_id.to_string(),
            &user.id,
            device_session_id,
            &fingerprint,
            &request.machine_id.to_string(),
            &request.project_id,
        )
        .await
        .map_err(|error| AppError::Conflict(error.to_string()))?;
    if record.user_id != user.id
        || record.device_session_id != device_session_id
        || record.request_fingerprint != fingerprint
        || record.machine_id != request.machine_id.to_string()
        || record.project_id != request.project_id
    {
        return Err(AppError::Conflict(
            "Task request identifier was already used for a different submission".to_string(),
        ));
    }
    if record.status == "completed" {
        return Ok(Json(completed_launch(&record)?));
    }
    if record.status == "failed" {
        return Err(AppError::Conflict(record.error_message.unwrap_or_else(
            || "The original task launch failed; edit the draft and submit again".to_string(),
        )));
    }

    // Until an instance that can share the project with other machines runs
    // it, the chosen machine keeps its original meaning: start the project
    // there when nothing runs it, otherwise add the task to the running
    // session. Once one does, the task runs on the chosen machine's instance,
    // started first when the project is registered there but not running, and
    // the launch fails rather than land on another machine. A project nothing
    // runs is started on a chosen machine whose daemon can share it the same
    // way, since the instance that attaches there is one that can.
    let running = state
        .sessions
        .active_session_for_project(&request.project_id);
    let shared_start = match running {
        None if state.sessions.daemon_supports_capability(
            &request.machine_id,
            shared::PROJECT_INSTANCES_CAPABILITY,
        ) =>
        {
            Uuid::parse_str(&request.project_id).ok()
        }
        _ => None,
    };
    let session_id = match running.or(shared_start) {
        Some(session_id) => session_id,
        None => {
            let policy = state
                .db
                .get_effective_project_policy(&request.project_id)
                .await?;
            {
                // Held until the start has been sent; see `admit_task_start`.
                let _open_state = state
                    .sessions
                    .project_open_state_guard(&request.project_id)
                    .await;
                // This daemon cannot run the project beside other machines.
                // In a project recorded open it is started only while no
                // other machine owns tabs, as an open and a web Start of it
                // decide: from the roster, since nothing is attached to ask.
                if let Some(reason) = older_daemon_beside_owners_refusal(&state, &request).await {
                    return Err(fail_task_launch(&state, &request, &user.id, reason).await);
                }
                // A machine whose daemon is away stays a launch target and
                // keeps passing its own access check. Nothing can be started
                // there now, so nothing is recorded either: an open recorded
                // here would start the machine when its daemon registers, on
                // the word of an account whose Open is refused meanwhile.
                if !state.sessions.is_daemon_connected(&request.machine_id) {
                    return Err(
                        fail_task_launch(&state, &request, &user.id, MACHINE_WENT_OFFLINE).await,
                    );
                }
                if let Err(reason) = admit_task_start(&state, &user.id, &request, true).await {
                    return Err(fail_task_launch(&state, &request, &user.id, reason).await);
                }
                // This daemon answers no start; kept for the machine's part.
                state.sessions.record_machine_start_asked(
                    &request.project_id,
                    request.machine_id,
                    None,
                );
                if !state
                    .sessions
                    .send_to_daemon(
                        &request.machine_id,
                        ServerToDaemon::StartProjectCli {
                            project_id: request.project_id.clone(),
                            policy: Some(policy),
                            request_id: None,
                        },
                    )
                    .await
                {
                    state
                        .sessions
                        .clear_machine_outcome(&request.project_id, &request.machine_id);
                    return Err(
                        fail_task_launch(&state, &request, &user.id, MACHINE_WENT_OFFLINE).await,
                    );
                }
            }
            let mut session_id = None;
            for _ in 0..60 {
                tokio::time::sleep(Duration::from_millis(250)).await;
                session_id = state
                    .sessions
                    .active_session_for_project(&request.project_id);
                if session_id.is_some() {
                    break;
                }
            }
            // Keep the retained operation pending. A retry with the same
            // request id continues waiting for the idempotent daemon start.
            session_id.ok_or_else(|| AppError::Conflict(STILL_STARTING.to_string()))?
        }
    };
    let instance = if shared_start.is_some()
        || state
            .sessions
            .session_attachments(&session_id)
            .iter()
            .any(|instance| instance.capable)
    {
        Some(task_launch_instance(&state, &user.id, &request, session_id).await?)
    } else {
        // The task joins the session a CLI that cannot share the project
        // runs. Nothing is started for it, and a project recorded closed
        // still follows the closed row.
        if running.is_some() {
            if let Err(reason) = admit_task_where_it_runs(&state, &user.id, &request).await {
                return Err(fail_task_launch(&state, &request, &user.id, reason).await);
            }
        }
        None
    };
    // What the CLI that receives the task supports: the chosen machine's
    // instance's, or the running session's primary's.
    let supports = |capability: &str| match &instance {
        Some(instance) => state
            .sessions
            .cli_supports_capability(&instance.cli_id, capability),
        None => state
            .sessions
            .session_supports_capability(&session_id, capability),
    };
    if let Some(message) = mobile_launch_capability_error(
        profile.provider,
        supports(shared::MOBILE_TASK_LAUNCH_CAPABILITY),
        supports(shared::OPENCODE_TERMINAL_CAPABILITY),
        supports(shared::OMP_TERMINAL_CAPABILITY),
    ) {
        return Err(fail_task_launch(&state, &request, &user.id, message).await);
    }

    let pane_id = native_task_pane_id(request.request_id);
    let existing = state.sessions.get_session_panes(&session_id);
    if !existing.iter().any(|pane| pane.pane_id == pane_id) {
        // An instance that can share the project owns the panes tagged with
        // its machine; an older CLI's stay untagged.
        let tag = instance
            .as_ref()
            .filter(|instance| instance.capable)
            .and_then(|instance| instance.machine_id);
        let pane_config = PaneConfig {
            pane_id,
            provider: profile.provider,
            mode: profile.mode,
            kind: profile.kind,
            session_id: Uuid::new_v5(&request.request_id, b"apas-mobile-task-pane"),
            is_paused: false,
            stop_requested: false,
            prompt: None,
            min_iteration_interval_minutes: None,
            label: Some("Mobile task".to_string()),
            model: profile.model,
            effort: None,
            worktree_path: None,
            role: None,
            goal: None,
            backstory: None,
            plan_review_mode: PlanReviewMode::default(),
            manual_mode: false,
            managed: false,
            machine_id: tag,
            hostname: tag.and(
                instance
                    .as_ref()
                    .and_then(|instance| instance.hostname.clone()),
            ),
        };
        state.sessions.reserve_pane_id(&session_id, pane_id, tag);
        let add = ServerToCli::AddPane {
            session_id,
            pane_config,
            isolated_worktree: false,
            initial_input: Some(instruction.to_string()),
        };
        let delivered = match &instance {
            Some(instance) => state
                .sessions
                .route_to_cli_on_machine(&session_id, instance.machine_id, add)
                .await
                .delivered(),
            None => state.sessions.route_to_cli(&session_id, add).await,
        };
        if !delivered {
            state.sessions.release_pane_id(&session_id, pane_id);
            return Err(AppError::Conflict(
                "The project runtime disconnected before task creation; retry this same submission"
                    .to_string(),
            ));
        }
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if state
                .sessions
                .get_session_panes(&session_id)
                .iter()
                .any(|pane| pane.pane_id == pane_id)
            {
                break;
            }
        }
        if !state
            .sessions
            .get_session_panes(&session_id)
            .iter()
            .any(|pane| pane.pane_id == pane_id)
        {
            return Err(AppError::Conflict(
                "The task pane is still starting; retry this same submission shortly".to_string(),
            ));
        }
    }
    state
        .db
        .complete_mobile_task_launch(
            &request.request_id.to_string(),
            &user.id,
            &session_id.to_string(),
            pane_id,
        )
        .await?;
    state
        .db
        .record_audit(
            &user.id,
            "mobile.task_launched",
            "project",
            &request.project_id,
            Some(serde_json::json!({
                "request_id": request.request_id,
                "machine_id": request.machine_id,
                "session_id": session_id,
                "pane_id": pane_id,
                "profile_key": request.profile_key,
            })),
        )
        .await?;
    if let Err(error) = state
        .db
        .record_session_user_input(&session_id.to_string(), &chrono::Utc::now().to_rfc3339())
        .await
    {
        tracing::warn!(%error, %session_id, "failed to record mobile task activity");
    }
    Ok(Json(MobileTaskLaunchResponse {
        request_id: request.request_id,
        session_id,
        pane_id: Some(pane_id),
        status: "acknowledged".to_string(),
    }))
}

pub async fn bootstrap(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MobileBootstrapResponse>, AppError> {
    let user = require_active_user(&headers, &state).await?;
    let user_id = Uuid::parse_str(&user.id)
        .map_err(|_| AppError::Internal("Invalid user identifier".to_string()))?;
    let sessions = accessible_sessions(&state, &user.id).await?;
    let machines = super::ws_web::list_accessible_machines_for_user(&state, &user_id).await;
    let launch_targets = launch_targets(&state, &machines).await;
    let jobs = crate::jobs::list_jobs(&state, user_id, None, None)
        .await
        .map_err(|_| AppError::Internal("Unable to load autonomous jobs".to_string()))?;
    let workers = crate::workers::list(&state, user_id, None, None, None)
        .await
        .map_err(|_| AppError::Internal("Unable to load autonomous workers".to_string()))?;
    Ok(Json(MobileBootstrapResponse {
        user_id,
        user_email: user.email,
        cluster_role: user.cluster_role,
        account_status: user.account_status,
        protocol_min_version: MOBILE_PROTOCOL_MIN_VERSION,
        protocol_max_version: MOBILE_PROTOCOL_MAX_VERSION,
        features: state.config.mobile.features.clone(),
        sessions,
        machines,
        launch_targets,
        autonomous_jobs: shared::MobileJobsBootstrap {
            available: true,
            jobs: jobs.jobs,
            next_cursor: jobs.next_cursor,
            unavailable_reasons: Vec::new(),
        },
        autonomous_workers: shared::MobileWorkersBootstrap {
            enabled: true,
            workers: workers.workers,
            next_cursor: workers.next_cursor,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_pane(pane_id: u32, label: &str) -> shared::PaneConfig {
        shared::PaneConfig {
            pane_id,
            provider: shared::Provider::Claude,
            mode: shared::PaneMode::Interactive,
            kind: shared::PaneKind::Terminal,
            session_id: Uuid::new_v4(),
            is_paused: false,
            stop_requested: false,
            prompt: None,
            min_iteration_interval_minutes: None,
            label: Some(label.to_string()),
            model: None,
            effort: None,
            worktree_path: None,
            role: None,
            goal: None,
            backstory: None,
            plan_review_mode: shared::PlanReviewMode::default(),
            manual_mode: false,
            managed: false,
            machine_id: None,
            hostname: None,
        }
    }
    use crate::{
        config::Config,
        db::{Database, User},
        routes::auth::Claims,
    };
    use axum::http::{header, HeaderValue};
    use jsonwebtoken::{encode, EncodingKey, Header};

    fn headers(state: &AppState, user_id: &str) -> HeaderMap {
        let token = encode(
            &Header::default(),
            &Claims {
                sub: user_id.to_string(),
                exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
                device_session_id: None,
                token_kind: None,
                credential_version: None,
            },
            &EncodingKey::from_secret(state.config.auth.jwt_secret.as_bytes()),
        )
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        headers
    }

    #[test]
    fn mobile_launch_catalog_exposes_only_terminal_profiles() {
        let policy = shared::EffectiveProjectPolicy::default();
        let profiles = mobile_launch_profiles(&policy);

        assert_eq!(
            profiles
                .iter()
                .map(|profile| profile.key.as_str())
                .collect::<Vec<_>>(),
            vec![
                "terminal:claude:official:default",
                "terminal:codex:official:default",
                "terminal:opencode:official:default",
                "terminal:omp:official:default",
                "terminal:claude:deepseek:deepseek-flash",
            ]
        );
        assert!(profiles
            .iter()
            .all(|profile| profile.kind == PaneKind::Terminal));
    }

    #[test]
    fn mobile_opencode_launch_requires_its_provider_capability() {
        let error = mobile_launch_capability_error(shared::Provider::Opencode, true, false, true)
            .expect("older CLI must be rejected");
        assert!(error.contains("OpenCode"));
        assert!(error.contains("updated and reconnected"));

        assert!(
            mobile_launch_capability_error(shared::Provider::Opencode, true, true, true).is_none()
        );
        assert!(
            mobile_launch_capability_error(shared::Provider::Claude, true, false, true).is_none()
        );
        assert!(
            mobile_launch_capability_error(shared::Provider::Codex, true, false, true).is_none()
        );
    }

    /// Pi was retired. No profile offers it, a stored policy that still lists
    /// its profile does not bring it back, and a launch that names it is
    /// refused whatever the CLI can host.
    #[test]
    #[allow(deprecated)]
    fn mobile_launch_never_offers_or_accepts_pi() {
        let mut policy = shared::EffectiveProjectPolicy::default();
        policy
            .allowed_launch_profiles
            .push("terminal:pi:official:default".to_string());
        assert!(mobile_launch_profiles(&policy)
            .iter()
            .all(|profile| profile.provider != shared::Provider::Pi));

        assert_eq!(
            mobile_launch_capability_error(shared::Provider::Pi, true, true, true),
            Some("Pi is no longer supported. Use OMP instead.")
        );
        assert_eq!(
            shared::retired_launch_profile_message("terminal:pi:official:default"),
            Some(shared::PI_RETIRED_MESSAGE)
        );
        // OMP is untouched: it needs only its own capability.
        assert!(mobile_launch_capability_error(shared::Provider::Omp, true, false, true).is_none());
        assert!(
            mobile_launch_capability_error(shared::Provider::Omp, true, true, false)
                .expect("an older CLI cannot host OMP")
                .contains("OMP")
        );
    }

    /// A native build that never receives the instance-set frame still learns
    /// from bootstrap every host a project runs on, whether it may use each,
    /// and the machine of each tab.
    #[tokio::test]
    async fn bootstrap_lists_every_instance_and_each_panes_machine() {
        let dir = std::env::temp_dir().join(format!("apas-mobile-instances-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::new(&dir.join("apas.db").to_string_lossy())
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let owner = Uuid::new_v4();
        db.create_user(&User {
            id: owner.to_string(),
            email: "instances@example.test".to_string(),
            password_hash: "hash".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .unwrap();
        let session_id = Uuid::new_v4();
        db.authorize_project_registration(&session_id.to_string(), &owner.to_string())
            .await
            .unwrap();
        db.create_session(&Session {
            id: session_id.to_string(),
            user_id: owner.to_string(),
            cli_client_id: None,
            working_dir: Some("/work/zoo-005".to_string()),
            hostname: Some("zoo-005".to_string()),
            status: "active".to_string(),
            created_at: None,
            updated_at: None,
            is_paused: false,
            project_id: Some(session_id.to_string()),
            git_remote: None,
            git_remote_url: None,
        })
        .await
        .unwrap();
        let mut config = Config::default();
        config.database.path = dir.join("apas.db").to_string_lossy().to_string();
        let state = AppState::new(db, config);
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let mut clis = Vec::new();
        for (machine_id, host) in [(m5, "zoo-005"), (m6, "zoo-006")] {
            let cli_id = Uuid::new_v4();
            let (cli_tx, cli_rx) = tokio::sync::mpsc::channel(4);
            state.sessions.register_cli(cli_id, owner, cli_tx, None);
            state
                .sessions
                .set_cli_capabilities(cli_id, vec![shared::PROJECT_INSTANCES_CAPABILITY.into()]);
            assert!(matches!(
                state.sessions.attach_cli_instance(
                    session_id,
                    cli_id,
                    Some(machine_id),
                    Some(format!("/work/{host}")),
                    Some(host.to_string()),
                ),
                crate::session::AttachOutcome::Attached { .. }
            ));
            clis.push(cli_rx);
        }
        let tagged = |pane_id: u32, machine_id: Option<Uuid>, host: &str| {
            let mut pane = test_pane(pane_id, "tab");
            pane.machine_id = machine_id;
            pane.hostname = Some(host.to_string());
            pane
        };
        state.sessions.set_session_panes(
            &session_id,
            vec![
                tagged(3, Some(m5), "zoo-005"),
                tagged(4, Some(m6), "zoo-006"),
                tagged(5, None, "zoo-006"),
            ],
        );

        let Json(response) = bootstrap(State(state.clone()), headers(&state, &owner.to_string()))
            .await
            .unwrap();
        let session = &response
            .sessions
            .iter()
            .find(|summary| summary.session.id == session_id)
            .expect("the project is listed")
            .session;
        assert_eq!(
            session
                .instances
                .iter()
                .map(|instance| (
                    instance.machine_id,
                    instance.hostname.as_str(),
                    instance.primary,
                    instance.runtime_access
                ))
                .collect::<Vec<_>>(),
            vec![
                (Some(m5), "zoo-005", true, Some(true)),
                (Some(m6), "zoo-006", false, Some(true)),
            ]
        );
        assert_eq!(
            session
                .panes
                .iter()
                .map(|pane| (pane.pane_id, pane.machine_id, pane.hostname.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                (3, Some(m5), Some("zoo-005")),
                (4, Some(m6), Some("zoo-006")),
                (5, None, Some("zoo-006")),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn bootstrap_excludes_inaccessible_sessions_and_advertises_flags() {
        let dir = std::env::temp_dir().join(format!("apas-mobile-bootstrap-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::new(&dir.join("apas.db").to_string_lossy())
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let owner_id = Uuid::new_v4().to_string();
        let stranger_id = Uuid::new_v4().to_string();
        let owner_session_id = Uuid::new_v4().to_string();
        let stranger_session_id = Uuid::new_v4().to_string();
        for (id, label, session_id) in [
            (&owner_id, "owner", &owner_session_id),
            (&stranger_id, "stranger", &stranger_session_id),
        ] {
            db.create_user(&User {
                id: id.clone(),
                email: format!("{label}@example.test"),
                password_hash: "hash".to_string(),
                created_at: None,
                cluster_role: "user".to_string(),
                account_status: "active".to_string(),
            })
            .await
            .unwrap();
            db.create_session(&Session {
                id: session_id.clone(),
                user_id: id.clone(),
                cli_client_id: None,
                working_dir: Some(format!("/workspace/{label}")),
                hostname: Some("host".to_string()),
                status: "active".to_string(),
                created_at: None,
                updated_at: None,
                is_paused: false,
                project_id: Some(Uuid::new_v4().to_string()),
                git_remote: None,
                git_remote_url: None,
            })
            .await
            .unwrap();
        }
        let recent_owner_session_id = Uuid::new_v4().to_string();
        db.create_session(&Session {
            id: recent_owner_session_id.clone(),
            user_id: owner_id.clone(),
            cli_client_id: None,
            working_dir: Some("/workspace/owner-recent".to_string()),
            hostname: Some("host".to_string()),
            status: "active".to_string(),
            created_at: None,
            updated_at: None,
            is_paused: false,
            project_id: Some(Uuid::new_v4().to_string()),
            git_remote: None,
            git_remote_url: None,
        })
        .await
        .unwrap();
        db.record_session_user_input(&owner_session_id, "2026-08-08T12:00:00Z")
            .await
            .unwrap();
        db.record_session_user_input(&recent_owner_session_id, "2026-08-09T12:00:00Z")
            .await
            .unwrap();
        let mut config = Config::default();
        config.database.path = dir.join("apas.db").to_string_lossy().to_string();
        config.mobile.features.bootstrap = true;
        let state = AppState::new(db, config);
        let cli_id = Uuid::new_v4();
        let session_id = Uuid::parse_str(&recent_owner_session_id).unwrap();
        let (cli_tx, _cli_rx) = tokio::sync::mpsc::channel(1);
        state
            .sessions
            .register_cli(cli_id, Uuid::parse_str(&owner_id).unwrap(), cli_tx, None);
        state
            .sessions
            .create_cli_session(session_id, cli_id, None, None);
        // A project with working, idle, and human-blocked panes: the case the
        // old binary flag could not express without calling the pending pane
        // working too.
        let mut fable_pane = test_pane(6, "Fable one");
        fable_pane.model = Some("claude-fable-5".to_string());
        state.sessions.set_session_panes(
            &session_id,
            vec![
                test_pane(3, "Busy one"),
                test_pane(4, "Idle one"),
                test_pane(5, "Needs answer"),
                fable_pane,
            ],
        );
        state.sessions.set_pane_status(
            &session_id,
            shared::PaneType::Interactive,
            3,
            Some("Working…".to_string()),
        );
        state.sessions.set_pane_status(
            &session_id,
            shared::PaneType::Interactive,
            5,
            Some(shared::PANE_STATUS_PENDING_ANSWER.to_string()),
        );
        state.sessions.update_usage_limits(
            cli_id,
            shared::Provider::Claude,
            shared::UsageLimits {
                five_hour: None,
                seven_day: Some(shared::UsageLimitWindow {
                    utilization: 1.0,
                    resets_at: Some("2026-08-23T13:00:00Z".to_string()),
                }),
                fetched_at: Some("2026-08-20T20:00:00Z".to_string()),
                usage_limited: Some(shared::UsageLimited {
                    window: "weekly".to_string(),
                    resets_at: Some("2026-08-23T13:00:00Z".to_string()),
                    model: None,
                }),
            },
        );
        let Json(response) = bootstrap(State(state.clone()), headers(&state, &owner_id))
            .await
            .unwrap();
        assert!(response.features.bootstrap);
        assert_eq!(response.sessions.len(), 2);
        assert_eq!(
            response.sessions[0].session.working_dir.as_deref(),
            Some("/workspace/owner-recent")
        );
        assert_eq!(
            response.sessions[0].last_user_input_at.as_deref(),
            Some("2026-08-09T12:00:00Z")
        );
        assert!(response.sessions[0].session.is_active);
        assert!(response.sessions[0].session.is_working);
        // Per pane, so an idle agent inside a working project is findable.
        let panes = &response.sessions[0].session.panes;
        assert_eq!(panes.len(), 4, "every pane state is reported");
        let busy = panes.iter().find(|pane| pane.pane_id == 3).unwrap();
        let idle = panes.iter().find(|pane| pane.pane_id == 4).unwrap();
        let pending = panes.iter().find(|pane| pane.pane_id == 5).unwrap();
        assert!(busy.is_working);
        assert!(!idle.is_working);
        assert!(!pending.is_working);
        assert!(pending.awaiting_answer);
        assert!(
            pending.idle_since.is_none(),
            "waiting for a human is not an idle transition"
        );
        assert_eq!(response.sessions[0].attention_count, 1);
        assert_eq!(idle.label.as_deref(), Some("Idle one"));
        assert!(
            idle.idle_since.is_some(),
            "idle panes carry ranking recency"
        );
        assert_eq!(
            idle.usage_limited
                .as_ref()
                .map(|limited| limited.window.as_str()),
            Some("weekly"),
            "provider availability is reported separately from pane work"
        );
        state.sessions.update_usage_limits(
            cli_id,
            shared::Provider::Claude,
            shared::UsageLimits {
                five_hour: None,
                seven_day: Some(shared::UsageLimitWindow {
                    utilization: 0.86,
                    resets_at: Some("2026-08-30T13:00:00Z".to_string()),
                }),
                fetched_at: Some("2026-08-29T20:00:00Z".to_string()),
                usage_limited: Some(shared::UsageLimited {
                    window: "weekly".to_string(),
                    resets_at: Some("2026-08-30T13:00:00Z".to_string()),
                    model: Some("Fable".to_string()),
                }),
            },
        );
        let scoped = accessible_sessions(&state, &owner_id).await.unwrap();
        let scoped_panes = &scoped
            .iter()
            .find(|summary| summary.session.id == session_id)
            .unwrap()
            .session
            .panes;
        assert!(scoped_panes
            .iter()
            .find(|pane| pane.pane_id == 4)
            .unwrap()
            .usage_limited
            .is_none());
        assert_eq!(
            scoped_panes
                .iter()
                .find(|pane| pane.pane_id == 6)
                .unwrap()
                .usage_limited
                .as_ref()
                .and_then(|limited| limited.model.as_deref()),
            Some("Fable")
        );
        // The two views agree, because they read the same statuses.
        assert!(panes.iter().any(|pane| pane.is_working));

        state
            .sessions
            .set_pane_status(&session_id, shared::PaneType::Interactive, 3, None);
        let pending_only = accessible_sessions(&state, &owner_id).await.unwrap();
        let recent = pending_only
            .iter()
            .find(|summary| summary.session.id == session_id)
            .unwrap();
        assert!(
            !recent.session.is_working,
            "a session with only an unanswered question is not working"
        );
        assert_eq!(recent.attention_count, 1);
        assert!(!response.sessions[1].session.is_active);
        assert!(!response.sessions[1].session.is_working);
        assert!(
            response.sessions[1].session.panes.is_empty(),
            "a session with no pane roster reports none, not phantom panes"
        );
    }

    // Native task launches aimed at a machine: today's meaning until an
    // instance that can share the project runs it, the chosen machine's
    // instance afterwards.

    const HOST_A: &str = "zoo-005";
    const HOST_B: &str = "zoo-006";
    const CAPABLE_CLI: &[&str] = &[
        shared::PROJECT_INSTANCES_CAPABILITY,
        shared::PROJECT_POLICY_CAPABILITY,
        shared::MOBILE_TASK_LAUNCH_CAPABILITY,
    ];
    const OLDER_CLI: &[&str] = &[
        shared::PROJECT_POLICY_CAPABILITY,
        shared::MOBILE_TASK_LAUNCH_CAPABILITY,
    ];
    const CAPABLE_DAEMON: &[&str] = &[
        shared::PROJECT_INSTANCES_CAPABILITY,
        shared::PROJECT_POLICY_CAPABILITY,
    ];

    /// A project its owner launches tasks in from a phone.
    struct NativeProject {
        state: AppState,
        owner: Uuid,
        session: Uuid,
        headers: HeaderMap,
        dir: std::path::PathBuf,
    }

    impl Drop for NativeProject {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// What one instance's CLI was sent as new panes, with their first
    /// instruction.
    type AddedPanes = std::sync::Arc<std::sync::Mutex<Vec<(PaneConfig, Option<String>)>>>;

    async fn native_project() -> NativeProject {
        let dir = std::env::temp_dir().join(format!("apas-native-launch-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("apas.db").to_string_lossy().to_string();
        let db = Database::new(&db_path).await.unwrap();
        db.run_migrations().await.unwrap();
        let owner = Uuid::new_v4();
        db.create_user(&User {
            id: owner.to_string(),
            email: format!("{owner}@example.test"),
            password_hash: "hash".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .unwrap();
        let session = Uuid::new_v4();
        db.authorize_project_registration(&session.to_string(), &owner.to_string())
            .await
            .unwrap();
        db.create_session(&Session {
            id: session.to_string(),
            user_id: owner.to_string(),
            cli_client_id: None,
            working_dir: Some(format!("/work/{HOST_A}")),
            hostname: Some(HOST_A.to_string()),
            status: "active".to_string(),
            created_at: None,
            updated_at: None,
            is_paused: false,
            project_id: Some(session.to_string()),
            git_remote: None,
            git_remote_url: None,
        })
        .await
        .unwrap();
        let device = Uuid::new_v4().to_string();
        db.create_mobile_device_session(
            &device,
            &owner.to_string(),
            &format!("installation-{device}"),
            "ios",
            Some("Phone"),
            "0.1.0",
            "refresh-hash",
            &(chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339(),
        )
        .await
        .unwrap();
        let mut config = Config::default();
        config.database.path = db_path;
        config.mobile.features.coding_mutations = true;
        let state = AppState::new(db, config);
        state
            .sessions
            .set_session_project(session, session.to_string());
        let (token, _) = crate::routes::auth::generate_mobile_access_token(
            &owner.to_string(),
            &device,
            &state.config.auth,
        )
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        NativeProject {
            state,
            owner,
            session,
            headers,
            dir,
        }
    }

    impl NativeProject {
        /// A connected daemon whose inventory lists the project.
        fn daemon(
            &self,
            machine_id: Uuid,
            host: &str,
            capabilities: &[&str],
        ) -> tokio::sync::mpsc::Receiver<ServerToDaemon> {
            let (tx, rx) = tokio::sync::mpsc::channel(16);
            self.state.sessions.register_daemon(
                machine_id,
                self.owner,
                tx,
                shared::MachineInfo {
                    machine_id,
                    hostname: host.to_string(),
                    os: "linux".to_string(),
                    arch: "x86_64".to_string(),
                    daemon_version: None,
                    deepseek_backend: None,
                    last_seen: None,
                    providers: None,
                },
                vec![shared::MachineProjectInfo {
                    project_id: self.session.to_string(),
                    name: None,
                    path: format!("/work/{host}"),
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
            self.state.sessions.set_daemon_capabilities(
                machine_id,
                capabilities
                    .iter()
                    .map(|capability| capability.to_string())
                    .collect(),
            );
            rx
        }

        /// A running instance of the project on `machine_id`, whose start the
        /// server has finished and whose CLI reports each new pane it is
        /// sent, as a real one does.
        fn instance(&self, machine_id: Uuid, host: &str, capabilities: &[&str]) -> AddedPanes {
            let (cli_id, added) = instance_reporting_its_panes(
                &self.state,
                self.session,
                machine_id,
                host,
                capabilities,
            );
            self.state
                .sessions
                .instance_started(self.session, machine_id, cli_id);
            added
        }

        fn launch(&self, machine_id: Uuid, request_id: Uuid) -> MobileTaskLaunchRequest {
            MobileTaskLaunchRequest {
                request_id,
                machine_id,
                project_id: self.session.to_string(),
                instruction: "Make the flaky upload test deterministic".to_string(),
                profile_key: "terminal:claude:official:default".to_string(),
            }
        }

        async fn submit(
            &self,
            request: MobileTaskLaunchRequest,
        ) -> Result<MobileTaskLaunchResponse, AppError> {
            launch_task(
                State(self.state.clone()),
                self.headers.clone(),
                Json(request),
            )
            .await
            .map(|Json(response)| response)
        }
    }

    fn instance_reporting_its_panes(
        state: &AppState,
        session: Uuid,
        machine_id: Uuid,
        host: &str,
        capabilities: &[&str],
    ) -> (Uuid, AddedPanes) {
        let cli_id = Uuid::new_v4();
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        state
            .sessions
            .register_cli(cli_id, Uuid::new_v4(), tx, None);
        state.sessions.set_cli_capabilities(
            cli_id,
            capabilities
                .iter()
                .map(|capability| capability.to_string())
                .collect(),
        );
        assert!(matches!(
            state.sessions.attach_cli_instance(
                session,
                cli_id,
                Some(machine_id),
                Some(format!("/work/{host}")),
                Some(host.to_string()),
            ),
            crate::session::AttachOutcome::Attached { .. }
        ));
        let added: AddedPanes = Default::default();
        let sessions = state.sessions.clone();
        let record = added.clone();
        tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                if let ServerToCli::AddPane {
                    pane_config,
                    initial_input,
                    ..
                } = message
                {
                    let roster = {
                        let mut added = record.lock().unwrap();
                        added.push((pane_config, initial_input));
                        added.iter().map(|(pane, _)| pane.clone()).collect()
                    };
                    sessions.merge_instance_roster(&session, &cli_id, roster);
                }
            }
        });
        (cli_id, added)
    }

    fn conflict(result: Result<MobileTaskLaunchResponse, AppError>) -> String {
        match result {
            Err(AppError::Conflict(message)) => message,
            other => panic!("expected a conflict, got {other:?}"),
        }
    }

    /// Until an instance that can share the project runs it, a task picked
    /// by machine is added to the running session, as it always was.
    #[tokio::test]
    async fn before_a_sharing_instance_runs_the_project_a_task_joins_the_running_session() {
        let p = native_project().await;
        let m5 = Uuid::new_v4();
        let m6 = Uuid::new_v4();
        let on_zoo_005 = p.instance(m5, HOST_A, OLDER_CLI);
        let mut zoo_006 = p.daemon(m6, HOST_B, CAPABLE_DAEMON);
        let request_id = Uuid::new_v4();

        let response = p.submit(p.launch(m6, request_id)).await.unwrap();

        assert_eq!(response.pane_id, Some(native_task_pane_id(request_id)));
        assert_eq!(response.session_id, p.session);
        let added = on_zoo_005.lock().unwrap().clone();
        assert_eq!(added.len(), 1);
        assert_eq!(
            added[0].0.machine_id, None,
            "an older CLI's panes stay untagged"
        );
        assert_eq!(
            added[0].1.as_deref(),
            Some("Make the flaky upload test deterministic")
        );
        assert!(zoo_006.try_recv().is_err(), "nothing is started on zoo-006");
    }

    /// Once one does, a task for a machine where the project is registered
    /// but stopped starts that machine's instance and runs there, tagged for
    /// it. A retry of the same launch replays its result.
    #[tokio::test]
    async fn a_task_for_a_stopped_machine_starts_its_instance_and_runs_there() {
        let p = native_project().await;
        let m5 = Uuid::new_v4();
        let m6 = Uuid::new_v4();
        let on_zoo_005 = p.instance(m5, HOST_A, CAPABLE_CLI);
        let mut zoo_006 = p.daemon(m6, HOST_B, CAPABLE_DAEMON);
        // zoo-006's daemon starts the project when asked, and its instance
        // finishes its start.
        let state = p.state.clone();
        let session = p.session;
        let started = tokio::spawn(async move {
            let Some(ServerToDaemon::StartProjectCli {
                project_id,
                request_id: Some(request_id),
                ..
            }) = zoo_006.recv().await
            else {
                panic!("expected a correlated start")
            };
            state
                .sessions
                .instance_start_result(m6, request_id, &project_id, true, None);
            let (cli_id, added) =
                instance_reporting_its_panes(&state, session, m6, HOST_B, CAPABLE_CLI);
            state.sessions.instance_started(session, m6, cli_id);
            added
        });
        let request_id = Uuid::new_v4();

        let response = p.submit(p.launch(m6, request_id)).await.unwrap();

        let on_zoo_006 = started.await.unwrap();
        let added = on_zoo_006.lock().unwrap().clone();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].0.pane_id, native_task_pane_id(request_id));
        assert_eq!(added[0].0.machine_id, Some(m6));
        assert!(added[0].1.is_some());
        assert!(on_zoo_005.lock().unwrap().is_empty(), "never on zoo-005");
        assert_eq!(response.pane_id, Some(native_task_pane_id(request_id)));
        let replayed = p.submit(p.launch(m6, request_id)).await.unwrap();
        assert_eq!(replayed.pane_id, response.pane_id);
        assert_eq!(
            on_zoo_006.lock().unwrap().len(),
            1,
            "a retry creates nothing"
        );
    }

    /// A machine whose daemon cannot run an instance beside the others fails
    /// the launch with the reason, for every retry, and the task runs nowhere.
    #[tokio::test]
    async fn a_task_fails_rather_than_run_on_another_machine() {
        let p = native_project().await;
        let m5 = Uuid::new_v4();
        let m6 = Uuid::new_v4();
        let on_zoo_005 = p.instance(m5, HOST_A, CAPABLE_CLI);
        let mut zoo_006 = p.daemon(m6, HOST_B, &[shared::PROJECT_POLICY_CAPABILITY]);
        let request_id = Uuid::new_v4();

        let refusal = conflict(p.submit(p.launch(m6, request_id)).await);
        assert!(
            refusal.ends_with("Reboot zoo-006 to update it."),
            "{refusal}"
        );
        assert_eq!(conflict(p.submit(p.launch(m6, request_id)).await), refusal);
        assert!(on_zoo_005.lock().unwrap().is_empty());
        assert!(zoo_006.try_recv().is_err());
    }

    /// A start that has not finished within the launch's window leaves the
    /// launch pending; retrying it once the instance runs creates the task
    /// there.
    #[tokio::test]
    async fn a_task_still_starting_is_finished_by_its_retry() {
        let p = native_project().await;
        p.state.sessions.set_pane_launch_timing(
            crate::session::pane_launches::PANE_ID_RESERVATION_TTL,
            Duration::from_millis(200),
        );
        let m5 = Uuid::new_v4();
        let m6 = Uuid::new_v4();
        let _zoo_005 = p.instance(m5, HOST_A, CAPABLE_CLI);
        let mut zoo_006 = p.daemon(m6, HOST_B, CAPABLE_DAEMON);
        let request_id = Uuid::new_v4();

        assert_eq!(
            conflict(p.submit(p.launch(m6, request_id)).await),
            STILL_STARTING
        );
        assert!(matches!(
            zoo_006.try_recv(),
            Ok(ServerToDaemon::StartProjectCli {
                request_id: Some(_),
                ..
            })
        ));
        let on_zoo_006 = p.instance(m6, HOST_B, CAPABLE_CLI);

        let response = p.submit(p.launch(m6, request_id)).await.unwrap();
        assert_eq!(response.pane_id, Some(native_task_pane_id(request_id)));
        assert_eq!(on_zoo_006.lock().unwrap()[0].0.machine_id, Some(m6));
    }

    // A native build shows nothing about a project's open state, so a task
    // launch never starts a machine other than the one it names.

    async fn open_state_of(p: &NativeProject) -> crate::db::ProjectOpenStateRecord {
        p.state
            .db
            .get_project_open_state(&p.session.to_string())
            .await
            .unwrap()
            .unwrap()
    }

    async fn record_state(
        p: &NativeProject,
        state: shared::ProjectOpenState,
        machine: Option<Uuid>,
    ) {
        assert!(p
            .state
            .db
            .set_project_open_state(
                &p.session.to_string(),
                state,
                &p.owner.to_string(),
                shared::OpenStateSetKind::Request,
                machine.as_ref(),
            )
            .await
            .unwrap());
    }

    /// The project's persisted roster: one tab on each of `machines`.
    async fn tabs_on(p: &NativeProject, machines: &[(Uuid, &str)]) {
        let panes: Vec<PaneConfig> = machines
            .iter()
            .enumerate()
            .map(|(index, (machine_id, host))| {
                let mut pane = test_pane(3 + index as u32, "tab");
                pane.machine_id = Some(*machine_id);
                pane.hostname = Some(host.to_string());
                pane
            })
            .collect();
        p.state
            .storage
            .save_pane_list(&p.session, &panes)
            .await
            .unwrap();
    }

    /// Scenario "A native task in a closed project with tabs elsewhere": the
    /// launch fails with the reason, for every retry, and nothing starts.
    #[tokio::test]
    async fn a_task_in_a_closed_project_with_tabs_on_another_machine_starts_nothing() {
        let p = native_project().await;
        let m5 = Uuid::new_v4();
        let m6 = Uuid::new_v4();
        let mut zoo_005 = p.daemon(m5, HOST_A, CAPABLE_DAEMON);
        let mut zoo_006 = p.daemon(m6, HOST_B, CAPABLE_DAEMON);
        tabs_on(&p, &[(m5, HOST_A), (m6, HOST_B)]).await;
        record_state(&p, shared::ProjectOpenState::Closed, None).await;
        let closed = open_state_of(&p).await;
        let request_id = Uuid::new_v4();

        let refusal = conflict(p.submit(p.launch(m5, request_id)).await);
        assert_eq!(
            refusal,
            "This project is closed and has tabs on other machines. Open it from the project."
        );
        assert_eq!(conflict(p.submit(p.launch(m5, request_id)).await), refusal);
        assert!(zoo_005.try_recv().is_err() && zoo_006.try_recv().is_err());
        assert_eq!(open_state_of(&p).await, closed);
        assert!(p
            .state
            .sessions
            .machines_with_pending_instance_start(&[p.session])
            .is_empty());
    }

    /// Scenario "A native task in a closed single-machine project": the
    /// project is recorded open, by the account that launched the task and
    /// naming its machine, and the task runs there.
    #[tokio::test]
    async fn a_task_in_a_closed_single_machine_project_opens_it_and_runs_there() {
        let p = native_project().await;
        let m5 = Uuid::new_v4();
        let m6 = Uuid::new_v4();
        let mut zoo_005 = p.daemon(m5, HOST_A, CAPABLE_DAEMON);
        let mut zoo_006 = p.daemon(m6, HOST_B, CAPABLE_DAEMON);
        tabs_on(&p, &[(m5, HOST_A)]).await;
        record_state(&p, shared::ProjectOpenState::Closed, None).await;
        let state = p.state.clone();
        let session = p.session;
        let started = tokio::spawn(async move {
            let Some(ServerToDaemon::StartProjectCli {
                project_id,
                request_id: Some(request_id),
                ..
            }) = zoo_005.recv().await
            else {
                panic!("expected a correlated start")
            };
            state
                .sessions
                .instance_start_result(m5, request_id, &project_id, true, None);
            let (cli_id, added) =
                instance_reporting_its_panes(&state, session, m5, HOST_A, CAPABLE_CLI);
            state.sessions.instance_started(session, m5, cli_id);
            added
        });
        let request_id = Uuid::new_v4();

        let response = p.submit(p.launch(m5, request_id)).await.unwrap();

        let on_zoo_005 = started.await.unwrap();
        assert_eq!(on_zoo_005.lock().unwrap().len(), 1);
        assert_eq!(response.pane_id, Some(native_task_pane_id(request_id)));
        let opened = open_state_of(&p).await;
        assert_eq!(opened.state, Some(shared::ProjectOpenState::Open));
        assert_eq!(opened.set_by_account(), Some(p.owner));
        assert_eq!(opened.stand_in_machine(), Some(m5));
        assert!(zoo_006.try_recv().is_err(), "no other machine is started");
    }

    /// On a project nobody recorded, and on an open one, a task that needs
    /// its machine started starts that machine and no other, and changes
    /// nothing that is recorded.
    #[tokio::test]
    async fn a_task_starts_only_its_machine_and_records_nothing() {
        let p = native_project().await;
        p.state.sessions.set_pane_launch_timing(
            crate::session::pane_launches::PANE_ID_RESERVATION_TTL,
            Duration::from_millis(200),
        );
        let m5 = Uuid::new_v4();
        let m6 = Uuid::new_v4();
        let mut zoo_005 = p.daemon(m5, HOST_A, CAPABLE_DAEMON);
        let mut zoo_006 = p.daemon(m6, HOST_B, CAPABLE_DAEMON);
        tabs_on(&p, &[(m5, HOST_A), (m6, HOST_B)]).await;

        // Not recorded.
        assert_eq!(
            conflict(p.submit(p.launch(m6, Uuid::new_v4())).await),
            STILL_STARTING
        );
        assert!(matches!(
            zoo_006.try_recv(),
            Ok(ServerToDaemon::StartProjectCli {
                request_id: Some(_),
                ..
            })
        ));
        assert!(zoo_005.try_recv().is_err());
        assert_eq!(open_state_of(&p).await.state, None);

        // Open: zoo-005 owns tabs and does not run them, and stays so.
        record_state(&p, shared::ProjectOpenState::Open, Some(m5)).await;
        let open = open_state_of(&p).await;
        assert_eq!(
            conflict(p.submit(p.launch(m6, Uuid::new_v4())).await),
            STILL_STARTING
        );
        assert!(matches!(
            zoo_006.try_recv(),
            Ok(ServerToDaemon::StartProjectCli { .. })
        ));
        assert!(zoo_005.try_recv().is_err());
        assert_eq!(open_state_of(&p).await, open);
    }

    /// The other launch path, for a machine whose daemon cannot share the
    /// project while nothing runs it, follows the same rule: refused in a
    /// closed project with tabs elsewhere, and the project's open in a closed
    /// one whose tabs are all on that machine.
    #[tokio::test]
    async fn a_task_on_an_older_daemon_follows_the_same_rule() {
        let older_daemon: &[&str] = &[shared::PROJECT_POLICY_CAPABILITY];
        let p = native_project().await;
        let m5 = Uuid::new_v4();
        let m6 = Uuid::new_v4();
        let mut zoo_005 = p.daemon(m5, HOST_A, older_daemon);
        let mut zoo_006 = p.daemon(m6, HOST_B, CAPABLE_DAEMON);
        tabs_on(&p, &[(m5, HOST_A), (m6, HOST_B)]).await;

        // Open, with tabs on zoo-006 too: a daemon that cannot share the
        // project is started only while no other machine owns tabs. Nothing
        // is attached, so only the roster says so. It is sent nothing.
        record_state(&p, shared::ProjectOpenState::Open, Some(m6)).await;
        let open = open_state_of(&p).await;
        let request_id = Uuid::new_v4();
        let refusal = conflict(p.submit(p.launch(m5, request_id)).await);
        assert_eq!(
            refusal,
            "zoo-005 runs an older APAS that cannot run this project beside other machines. Reboot zoo-005 to update it."
        );
        assert_eq!(conflict(p.submit(p.launch(m5, request_id)).await), refusal);
        assert!(zoo_005.try_recv().is_err() && zoo_006.try_recv().is_err());
        assert_eq!(open_state_of(&p).await, open);
        assert_eq!(
            p.state
                .sessions
                .machine_outcome(&p.session.to_string(), &m5),
            None
        );

        record_state(&p, shared::ProjectOpenState::Closed, None).await;
        let closed = open_state_of(&p).await;

        assert_eq!(
            conflict(p.submit(p.launch(m5, Uuid::new_v4())).await),
            "This project is closed and has tabs on other machines. Open it from the project."
        );
        assert!(zoo_005.try_recv().is_err() && zoo_006.try_recv().is_err());
        assert_eq!(open_state_of(&p).await, closed);

        tabs_on(&p, &[(m5, HOST_A)]).await;
        let state = p.state.clone();
        let session = p.session;
        let started = tokio::spawn(async move {
            let Some(ServerToDaemon::StartProjectCli {
                request_id: None, ..
            }) = zoo_005.recv().await
            else {
                panic!("expected the start an older daemon is sent")
            };
            // Its CLI attaches as the project's only runtime.
            instance_reporting_its_panes(&state, session, m5, HOST_A, OLDER_CLI).1
        });
        let request_id = Uuid::new_v4();
        let response = p.submit(p.launch(m5, request_id)).await.unwrap();
        assert_eq!(response.pane_id, Some(native_task_pane_id(request_id)));
        assert_eq!(started.await.unwrap().lock().unwrap().len(), 1);
        let opened = open_state_of(&p).await;
        assert_eq!(opened.state, Some(shared::ProjectOpenState::Open));
        assert_eq!(opened.set_by_account(), Some(p.owner));
        assert!(zoo_006.try_recv().is_err(), "no other machine is started");
    }

    /// The project's roster as its instances reported it: one tab on each of
    /// `machines`, tagged, or attributed to the host by name when untagged.
    fn tabs_reported(p: &NativeProject, machines: &[(Option<Uuid>, &str)]) {
        let panes: Vec<PaneConfig> = machines
            .iter()
            .enumerate()
            .map(|(index, (machine_id, host))| {
                let mut pane = test_pane(3 + index as u32, "tab");
                pane.machine_id = *machine_id;
                pane.hostname = Some(host.to_string());
                pane
            })
            .collect();
        p.state.sessions.set_session_panes(&p.session, panes);
    }

    /// A task on a machine that still has an instance attached in a project
    /// recorded closed, as after a close that could not reach its daemon:
    /// the launch needs no machine started, and the closed row applies all
    /// the same. With tabs on another machine it fails with the reason, for
    /// every retry, and no task is created; with none it opens the project
    /// and runs there.
    #[tokio::test]
    async fn a_task_where_a_closed_project_still_runs_follows_the_closed_row() {
        let p = native_project().await;
        let m5 = Uuid::new_v4();
        let m6 = Uuid::new_v4();
        let mut zoo_005 = p.daemon(m5, HOST_A, CAPABLE_DAEMON);
        let mut zoo_006 = p.daemon(m6, HOST_B, CAPABLE_DAEMON);
        let on_zoo_006 = p.instance(m6, HOST_B, CAPABLE_CLI);
        tabs_reported(&p, &[(Some(m5), HOST_A), (Some(m6), HOST_B)]);
        record_state(&p, shared::ProjectOpenState::Closed, None).await;
        let closed = open_state_of(&p).await;
        let request_id = Uuid::new_v4();

        let refusal = conflict(p.submit(p.launch(m6, request_id)).await);
        assert_eq!(
            refusal,
            "This project is closed and has tabs on other machines. Open it from the project."
        );
        assert_eq!(conflict(p.submit(p.launch(m6, request_id)).await), refusal);
        assert!(on_zoo_006.lock().unwrap().is_empty(), "no task is created");
        assert!(zoo_005.try_recv().is_err() && zoo_006.try_recv().is_err());
        assert_eq!(open_state_of(&p).await, closed);

        // Every tab on zoo-006: the task opens the project and runs there.
        tabs_reported(&p, &[(Some(m6), HOST_B)]);
        let request_id = Uuid::new_v4();
        let response = p.submit(p.launch(m6, request_id)).await.unwrap();
        assert_eq!(response.pane_id, Some(native_task_pane_id(request_id)));
        assert_eq!(on_zoo_006.lock().unwrap().len(), 1);
        let opened = open_state_of(&p).await;
        assert_eq!(opened.state, Some(shared::ProjectOpenState::Open));
        assert_eq!(opened.set_by_account(), Some(p.owner));
        assert_eq!(opened.stand_in_machine(), Some(m6));
        assert!(zoo_005.try_recv().is_err(), "no other machine is started");
        // The machine that still runs it is sent the start its daemon
        // answers by changing nothing.
        assert!(matches!(
            zoo_006.try_recv(),
            Ok(ServerToDaemon::StartProjectCli {
                request_id: None,
                ..
            })
        ));

        // An open project and one nobody recorded are left as they are.
        let before = open_state_of(&p).await;
        p.submit(p.launch(m6, Uuid::new_v4())).await.unwrap();
        assert_eq!(open_state_of(&p).await, before);
        assert!(zoo_006.try_recv().is_err() && zoo_005.try_recv().is_err());

        let q = native_project().await;
        let mut q5 = q.daemon(m5, HOST_A, CAPABLE_DAEMON);
        let on_q5 = q.instance(m5, HOST_A, CAPABLE_CLI);
        tabs_reported(&q, &[(Some(m5), HOST_A), (Some(m6), HOST_B)]);
        q.submit(q.launch(m5, Uuid::new_v4())).await.unwrap();
        assert_eq!(on_q5.lock().unwrap().len(), 1);
        assert_eq!(open_state_of(&q).await.state, None);
        assert!(q5.try_recv().is_err());
    }

    /// A project member who is also a member of the owner's cluster, allowed
    /// on `machines` only, with a phone signed in: the account and the
    /// headers its launches carry.
    async fn cluster_member_phone(p: &NativeProject, machines: &[Uuid]) -> (Uuid, HeaderMap) {
        let member = Uuid::new_v4();
        let db = &p.state.db;
        db.create_user(&User {
            id: member.to_string(),
            email: format!("{member}@example.test"),
            password_hash: "hash".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .unwrap();
        db.add_project_member(
            &p.owner.to_string(),
            &p.session.to_string(),
            &member.to_string(),
        )
        .await
        .unwrap();
        let token_hash = Uuid::new_v4().to_string();
        db.create_shared_cluster_invitation(
            &Uuid::new_v4().to_string(),
            &token_hash,
            &p.owner.to_string(),
            &format!("{member}@example.test"),
            &(chrono::Utc::now() + chrono::Duration::hours(1))
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        )
        .await
        .unwrap();
        db.accept_shared_cluster_invitation(&token_hash, &member.to_string())
            .await
            .unwrap()
            .unwrap();
        db.update_cluster_member_policy(
            &p.owner.to_string(),
            &member.to_string(),
            Some(machines.iter().map(Uuid::to_string).collect()),
            None,
        )
        .await
        .unwrap()
        .unwrap();
        let device = Uuid::new_v4().to_string();
        db.create_mobile_device_session(
            &device,
            &member.to_string(),
            &format!("installation-{device}"),
            "ios",
            Some("Phone"),
            "0.1.0",
            &format!("refresh-hash-{device}"),
            &(chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339(),
        )
        .await
        .unwrap();
        let (token, _) = crate::routes::auth::generate_mobile_access_token(
            &member.to_string(),
            &device,
            &p.state.config.auth,
        )
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        (member, headers)
    }

    /// Opening a closed project through a task on a machine that still has
    /// an instance attached takes the authority to set the project's state,
    /// as the project-wide Open does. The machine stays a launch target, and
    /// its own check still passes, while its daemon's channel is away; a
    /// cluster member who may use only that machine may not open the project
    /// then, so their launch fails with the answer that Open gets, for every
    /// retry, and records nothing. The owner's launch there still opens the
    /// project, and so does the member's while the machine is connected.
    #[tokio::test]
    async fn a_task_where_a_closed_project_still_runs_opens_nothing_for_an_account_that_may_not_set_the_state(
    ) {
        let p = native_project().await;
        let m5 = Uuid::new_v4();
        let m6 = Uuid::new_v4();
        let mut zoo_005 = p.daemon(m5, HOST_A, CAPABLE_DAEMON);
        let _zoo_006 = p.daemon(m6, HOST_B, CAPABLE_DAEMON);
        let on_zoo_006 = p.instance(m6, HOST_B, CAPABLE_CLI);
        tabs_reported(&p, &[(Some(m6), HOST_B)]);
        record_state(&p, shared::ProjectOpenState::Closed, None).await;
        let closed = open_state_of(&p).await;
        let (member, member_headers) = cluster_member_phone(&p, &[m6]).await;
        let submit_as_member = |request: MobileTaskLaunchRequest| {
            let state = p.state.clone();
            let headers = member_headers.clone();
            async move {
                launch_task(State(state), headers, Json(request))
                    .await
                    .map(|Json(response)| response)
            }
        };

        p.state.sessions.unregister_daemon(&m6);
        assert!(
            super::super::ws_web::has_machine_project_runtime_access(
                &p.state,
                &member,
                &m6,
                &p.session.to_string()
            )
            .await,
            "the machine's own check still passes while it is away"
        );
        assert!(
            !crate::open_state::may_set_open_state(&p.state, &member, &p.session.to_string()).await,
            "the member's Open is refused meanwhile"
        );
        let request_id = Uuid::new_v4();
        let refusal = conflict(submit_as_member(p.launch(m6, request_id)).await);
        assert_eq!(refusal, "Machine not found");
        assert_eq!(
            conflict(submit_as_member(p.launch(m6, request_id)).await),
            refusal
        );
        assert!(on_zoo_006.lock().unwrap().is_empty(), "no task is created");
        assert_eq!(open_state_of(&p).await, closed, "nothing is recorded");
        assert!(zoo_005.try_recv().is_err());

        // The owner may set the state while zoo-005 is connected.
        let request_id = Uuid::new_v4();
        let response = p.submit(p.launch(m6, request_id)).await.unwrap();
        assert_eq!(response.pane_id, Some(native_task_pane_id(request_id)));
        assert_eq!(on_zoo_006.lock().unwrap().len(), 1);
        let opened = open_state_of(&p).await;
        assert_eq!(opened.state, Some(shared::ProjectOpenState::Open));
        assert_eq!(opened.set_by_account(), Some(p.owner));

        // While the machine is connected the member's launch opens it.
        let q = native_project().await;
        let mut q6 = q.daemon(m6, HOST_B, CAPABLE_DAEMON);
        let on_q6 = q.instance(m6, HOST_B, CAPABLE_CLI);
        tabs_reported(&q, &[(Some(m6), HOST_B)]);
        record_state(&q, shared::ProjectOpenState::Closed, None).await;
        let (member, member_headers) = cluster_member_phone(&q, &[m6]).await;
        let request_id = Uuid::new_v4();
        let response = launch_task(
            State(q.state.clone()),
            member_headers,
            Json(q.launch(m6, request_id)),
        )
        .await
        .map(|Json(response)| response)
        .unwrap();
        assert_eq!(response.pane_id, Some(native_task_pane_id(request_id)));
        assert_eq!(on_q6.lock().unwrap().len(), 1);
        let opened = open_state_of(&q).await;
        assert_eq!(opened.state, Some(shared::ProjectOpenState::Open));
        assert_eq!(opened.set_by_account(), Some(member));
        assert!(matches!(
            q6.try_recv(),
            Ok(ServerToDaemon::StartProjectCli {
                request_id: None,
                ..
            })
        ));
    }

    /// A task chosen for a machine whose daemon is away, in a closed project
    /// nothing runs: the launch fails as offline and records nothing, for a
    /// member whose Open is refused at that moment and for the owner alike.
    /// Recorded open, the project would start on that machine when its
    /// daemon registers, for a task that was never created.
    #[tokio::test]
    async fn a_task_for_a_machine_that_is_away_leaves_a_closed_project_closed() {
        let p = native_project().await;
        let m5 = Uuid::new_v4();
        let m6 = Uuid::new_v4();
        let mut zoo_005 = p.daemon(m5, HOST_A, CAPABLE_DAEMON);
        let _zoo_006 = p.daemon(m6, HOST_B, CAPABLE_DAEMON);
        tabs_reported(&p, &[(Some(m6), HOST_B)]);
        record_state(&p, shared::ProjectOpenState::Closed, None).await;
        let closed = open_state_of(&p).await;
        let (member, member_headers) = cluster_member_phone(&p, &[m6]).await;
        p.state.sessions.unregister_daemon(&m6);
        assert!(
            !crate::open_state::may_set_open_state(&p.state, &member, &p.session.to_string()).await,
            "the member's Open is refused meanwhile"
        );

        let refusal = conflict(
            launch_task(
                State(p.state.clone()),
                member_headers,
                Json(p.launch(m6, Uuid::new_v4())),
            )
            .await
            .map(|Json(response)| response),
        );
        assert_eq!(refusal, MACHINE_WENT_OFFLINE);
        assert_eq!(open_state_of(&p).await, closed, "nothing is recorded");

        assert_eq!(
            conflict(p.submit(p.launch(m6, Uuid::new_v4())).await),
            MACHINE_WENT_OFFLINE
        );
        assert_eq!(open_state_of(&p).await, closed, "nor for the owner");
        assert!(zoo_005.try_recv().is_err(), "no other machine is sent a frame");
    }

    /// The same for a task that joins the session of a CLI that cannot share
    /// the project: nothing is started for it either. Chosen for another
    /// machine while that CLI's host holds the tabs, it fails with the
    /// reason; chosen for the machine that runs it, it opens the project.
    #[tokio::test]
    async fn a_task_joining_an_older_clis_session_in_a_closed_project_follows_the_closed_row() {
        let older_daemon: &[&str] = &[shared::PROJECT_POLICY_CAPABILITY];
        let p = native_project().await;
        let m5 = Uuid::new_v4();
        let m6 = Uuid::new_v4();
        let mut zoo_005 = p.daemon(m5, HOST_A, older_daemon);
        let mut zoo_006 = p.daemon(m6, HOST_B, CAPABLE_DAEMON);
        let on_zoo_005 = p.instance(m5, HOST_A, OLDER_CLI);
        // An older CLI's tabs are untagged and attributed to its host.
        tabs_reported(&p, &[(None, HOST_A)]);
        record_state(&p, shared::ProjectOpenState::Closed, None).await;
        let closed = open_state_of(&p).await;

        assert_eq!(
            conflict(p.submit(p.launch(m6, Uuid::new_v4())).await),
            "This project is closed and has tabs on other machines. Open it from the project."
        );
        assert!(on_zoo_005.lock().unwrap().is_empty(), "no task is created");
        assert!(zoo_005.try_recv().is_err() && zoo_006.try_recv().is_err());
        assert_eq!(open_state_of(&p).await, closed);

        let request_id = Uuid::new_v4();
        let response = p.submit(p.launch(m5, request_id)).await.unwrap();
        assert_eq!(response.pane_id, Some(native_task_pane_id(request_id)));
        assert_eq!(on_zoo_005.lock().unwrap().len(), 1);
        let opened = open_state_of(&p).await;
        assert_eq!(opened.state, Some(shared::ProjectOpenState::Open));
        assert_eq!(opened.set_by_account(), Some(p.owner));
        assert!(zoo_006.try_recv().is_err(), "no other machine is started");
    }
}
