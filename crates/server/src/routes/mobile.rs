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
        is_working: is_active && !state.sessions.get_pane_statuses(&session_id).is_empty(),
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

async fn accessible_sessions(
    state: &AppState,
    user_id: &str,
) -> Result<Vec<MobileSessionSummary>, AppError> {
    let mut summaries = Vec::new();
    for session in state.db.get_sessions_for_user(user_id).await? {
        let last_user_input_at = state.db.get_session_last_user_input_at(&session.id).await?;
        let latest_update_at = session
            .updated_at
            .clone()
            .or_else(|| session.created_at.clone());
        let info = session_info(state, session, false, None);
        summaries.push(MobileSessionSummary {
            project_name: project_name(&info),
            latest_update_at,
            last_user_input_at,
            latest_summary: None,
            attention_count: 0,
            session: info,
        });
    }
    for (session, owner_email, _) in state.db.get_shared_sessions_for_user(user_id).await? {
        let last_user_input_at = state.db.get_session_last_user_input_at(&session.id).await?;
        let latest_update_at = session
            .updated_at
            .clone()
            .or_else(|| session.created_at.clone());
        let info = session_info(state, session, true, Some(owner_email));
        summaries.push(MobileSessionSummary {
            project_name: project_name(&info),
            latest_update_at,
            last_user_input_at,
            latest_summary: None,
            attention_count: 0,
            session: info,
        });
    }
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
) -> Option<&'static str> {
    if !supports_mobile_launch {
        return Some("The project CLI must be updated and reconnected before mobile task launch");
    }
    if provider == shared::Provider::Opencode && !supports_opencode_terminal {
        return Some(
            "The project CLI must be updated and reconnected before launching an OpenCode task",
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
            AppError::Conflict(
                "The selected coding profile is no longer allowed; choose another profile"
                    .to_string(),
            )
        })?;
    let _launch_guard = state.mobile_task_launch_guard(request.request_id).await;
    state
        .db
        .authorize_project_registration(&request.project_id, &user.id)
        .await
        .map_err(|error| AppError::Forbidden(error.to_string()))?;

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

    let mut session_id = state
        .sessions
        .active_session_for_project(&request.project_id);
    if session_id.is_none() {
        let policy = state
            .db
            .get_effective_project_policy(&request.project_id)
            .await?;
        if !state
            .sessions
            .send_to_daemon(
                &request.machine_id,
                ServerToDaemon::StartProjectCli {
                    project_id: request.project_id.clone(),
                    policy: Some(policy),
                },
            )
            .await
        {
            let message = "The selected machine went offline; reconnect it and retry";
            state
                .db
                .fail_mobile_task_launch(&request.request_id.to_string(), &user.id, message)
                .await?;
            return Err(AppError::Conflict(message.to_string()));
        }
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            session_id = state
                .sessions
                .active_session_for_project(&request.project_id);
            if session_id.is_some() {
                break;
            }
        }
    }
    let Some(session_id) = session_id else {
        // Keep the retained operation pending. A retry with the same request
        // id continues waiting for the idempotent daemon start.
        return Err(AppError::Conflict(
            "The project runtime is still starting; retry this same submission shortly".to_string(),
        ));
    };
    let supports_mobile_launch = state
        .sessions
        .session_supports_capability(&session_id, shared::MOBILE_TASK_LAUNCH_CAPABILITY);
    let supports_opencode_terminal = state
        .sessions
        .session_supports_capability(&session_id, shared::OPENCODE_TERMINAL_CAPABILITY);
    if let Some(message) = mobile_launch_capability_error(
        profile.provider,
        supports_mobile_launch,
        supports_opencode_terminal,
    ) {
        state
            .db
            .fail_mobile_task_launch(&request.request_id.to_string(), &user.id, message)
            .await?;
        return Err(AppError::Conflict(message.to_string()));
    }

    let pane_id = 10_000 + (request.request_id.as_u128() % 1_000_000) as u32;
    let existing = state.sessions.get_session_panes(&session_id);
    if !existing.iter().any(|pane| pane.pane_id == pane_id) {
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
        };
        if !state
            .sessions
            .route_to_cli(
                &session_id,
                ServerToCli::AddPane {
                    session_id,
                    pane_config: pane_config.clone(),
                    isolated_worktree: false,
                    initial_input: Some(instruction.to_string()),
                },
            )
            .await
        {
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
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
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
            ]
        );
        assert!(profiles
            .iter()
            .all(|profile| profile.kind == PaneKind::Terminal));
    }

    #[test]
    fn mobile_opencode_launch_requires_its_provider_capability() {
        let error = mobile_launch_capability_error(shared::Provider::Opencode, true, false)
            .expect("older CLI must be rejected");
        assert!(error.contains("OpenCode"));
        assert!(error.contains("updated and reconnected"));

        assert!(mobile_launch_capability_error(shared::Provider::Opencode, true, true).is_none());
        assert!(mobile_launch_capability_error(shared::Provider::Claude, true, false).is_none());
        assert!(mobile_launch_capability_error(shared::Provider::Codex, true, false).is_none());
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
        state.sessions.set_pane_status(
            &session_id,
            shared::PaneType::Interactive,
            3,
            Some("Working…".to_string()),
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
        assert!(!response.sessions[1].session.is_active);
        assert!(!response.sessions[1].session.is_working);
    }
}
