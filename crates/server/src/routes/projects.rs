//! Project-scoped owner and member lifecycle endpoints.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use shared::ProjectAccessChange;
use uuid::Uuid;

use crate::{
    db::{Project, ProjectLifecycle},
    error::AppError,
    project_lifecycle::schedule_project_cleanup,
    routes::authz::require_active_user,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct TransferOwnerRequest {
    pub user_id: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteProjectRequest {
    pub confirmation: String,
}

#[derive(Debug, Serialize)]
pub struct ProjectMutationResponse {
    pub success: bool,
    pub status: &'static str,
}

fn ensure_owner(project: &Project, user_id: &str) -> Result<(), AppError> {
    if project.owner_user_id != user_id {
        return Err(AppError::Forbidden(
            "Project owner access required".to_string(),
        ));
    }
    Ok(())
}

fn ensure_not_deleting(project: &Project) -> Result<(), AppError> {
    if project.lifecycle() == ProjectLifecycle::Deleting {
        return Err(AppError::Conflict(
            "Project deletion is already in progress".to_string(),
        ));
    }
    Ok(())
}

async fn load_project(state: &AppState, project_id: &str) -> Result<Project, AppError> {
    state
        .db
        .get_project(project_id)
        .await?
        .ok_or_else(|| AppError::NotFound("Project not found".to_string()))
}

pub async fn transfer_owner(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(request): Json<TransferOwnerRequest>,
) -> Result<Json<ProjectMutationResponse>, AppError> {
    let actor = require_active_user(&headers, &state).await?;
    let _guard = state.project_operation_guard(&project_id).await;
    let project = load_project(&state, &project_id).await?;
    ensure_owner(&project, &actor.id)?;
    ensure_not_deleting(&project)?;
    if project.lifecycle() != ProjectLifecycle::Active {
        return Err(AppError::Conflict(
            "Ownership can only be transferred for an active project".to_string(),
        ));
    }
    if request.user_id == actor.id {
        return Err(AppError::Conflict(
            "The selected user already owns the project".to_string(),
        ));
    }
    let target = state
        .db
        .get_user_by_id(&request.user_id)
        .await?
        .ok_or_else(|| AppError::BadRequest("Target cluster account not found".to_string()))?;
    if !target.is_active() {
        return Err(AppError::Conflict(
            "Target cluster account is suspended".to_string(),
        ));
    }
    if state
        .db
        .get_project_role_for_user(&project_id, &request.user_id)
        .await?
        .as_deref()
        != Some("user")
    {
        return Err(AppError::Conflict(
            "Ownership can only be transferred to an existing project user".to_string(),
        ));
    }

    let changed = state
        .db
        .transfer_project_ownership_by_owner(&actor.id, &project_id, &request.user_id)
        .await
        .map_err(|error| AppError::Conflict(error.to_string()))?;
    if !changed {
        return Err(AppError::Conflict(
            "Project ownership changed concurrently".to_string(),
        ));
    }
    crate::jobs::reconcile_authority(&state).await?;

    if let Ok(old_owner_id) = Uuid::parse_str(&actor.id) {
        state.sessions.notify_project_access_changed(
            &old_owner_id,
            &project_id,
            ProjectAccessChange::Transferred,
            Some("user"),
        );
    }
    if let Ok(new_owner_id) = Uuid::parse_str(&request.user_id) {
        state.sessions.notify_project_access_changed(
            &new_owner_id,
            &project_id,
            ProjectAccessChange::Transferred,
            Some("owner"),
        );
    }

    Ok(Json(ProjectMutationResponse {
        success: true,
        status: "transferred",
    }))
}

pub async fn leave_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<ProjectMutationResponse>, AppError> {
    let actor = require_active_user(&headers, &state).await?;
    let _guard = state.project_operation_guard(&project_id).await;
    let project = load_project(&state, &project_id).await?;
    ensure_not_deleting(&project)?;
    if project.owner_user_id == actor.id {
        return Err(AppError::Conflict(
            "The project owner must transfer ownership or delete the project".to_string(),
        ));
    }
    if state
        .db
        .get_project_role_for_user(&project_id, &actor.id)
        .await?
        .as_deref()
        != Some("user")
    {
        return Err(AppError::Forbidden(
            "Project membership required".to_string(),
        ));
    }
    let changed = state
        .db
        .leave_project(&actor.id, &project_id)
        .await
        .map_err(|error| AppError::Conflict(error.to_string()))?;
    if !changed {
        return Err(AppError::Conflict(
            "Project membership changed concurrently".to_string(),
        ));
    }
    crate::jobs::reconcile_authority(&state).await?;
    if let Ok(user_id) = Uuid::parse_str(&actor.id) {
        state
            .sessions
            .revoke_project_access_for_user(&project_id, &user_id)
            .await;
    }

    Ok(Json(ProjectMutationResponse {
        success: true,
        status: "left",
    }))
}

pub async fn delete_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(request): Json<DeleteProjectRequest>,
) -> Result<(StatusCode, Json<ProjectMutationResponse>), AppError> {
    let actor = require_active_user(&headers, &state).await?;
    let project = load_project(&state, &project_id).await?;
    ensure_owner(&project, &actor.id)?;
    if request.confirmation != project_id {
        return Err(AppError::BadRequest(
            "Project deletion confirmation does not match".to_string(),
        ));
    }
    {
        // Beginning a deletion records the project closed in its own
        // transaction, so a host that returns meanwhile is not started.
        let _open_state = state.sessions.project_open_state_guard(&project_id).await;
        let began = state
            .db
            .begin_project_deletion(&actor.id, &project_id, &request.confirmation)
            .await
            .map_err(|error| AppError::Conflict(error.to_string()))?;
        if began {
            crate::open_state::note_administrative_close(
                &state,
                &actor.id,
                &project_id,
                "deletion",
            )
            .await;
        }
    }
    crate::jobs::cancel_project_jobs(&state, &project_id, "project_deleting").await?;
    crate::jobs::reconcile_authority(&state).await?;
    schedule_project_cleanup(state, project_id);

    Ok((
        StatusCode::ACCEPTED,
        Json(ProjectMutationResponse {
            success: true,
            status: "deleting",
        }),
    ))
}

pub async fn deletion_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<ProjectMutationResponse>, AppError> {
    let actor = require_active_user(&headers, &state).await?;
    let project = load_project(&state, &project_id).await?;
    ensure_owner(&project, &actor.id)?;
    if project.lifecycle() != ProjectLifecycle::Deleting {
        return Err(AppError::Conflict(
            "Project deletion has not started".to_string(),
        ));
    }
    Ok(Json(ProjectMutationResponse {
        success: true,
        status: "deleting",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{header, HeaderValue};
    use jsonwebtoken::{encode, EncodingKey, Header};

    async fn test_state() -> AppState {
        let dir = std::env::temp_dir().join(format!("apas-project-routes-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("apas.db");
        let db = crate::db::Database::new(&db_path.to_string_lossy())
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let mut config = crate::config::Config::default();
        config.database.path = db_path.to_string_lossy().into_owned();
        config.auth.jwt_secret = "project-route-test-secret".to_string();
        AppState::new(db, config)
    }

    async fn add_user(state: &AppState, id: Uuid, status: &str) {
        state
            .db
            .create_user(&crate::db::User {
                id: id.to_string(),
                email: format!("{id}@test"),
                password_hash: "hash".to_string(),
                created_at: None,
                cluster_role: "user".to_string(),
                account_status: status.to_string(),
            })
            .await
            .unwrap();
    }

    fn headers(state: &AppState, user_id: Uuid) -> HeaderMap {
        let token = encode(
            &Header::default(),
            &crate::routes::auth::Claims {
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

    #[tokio::test]
    async fn owner_transfer_and_self_departure_enforce_current_roles() {
        let state = test_state().await;
        let owner = Uuid::new_v4();
        let member = Uuid::new_v4();
        let outsider = Uuid::new_v4();
        let suspended = Uuid::new_v4();
        for user in [owner, member, outsider] {
            add_user(&state, user, "active").await;
        }
        add_user(&state, suspended, "suspended").await;
        let project_id = Uuid::new_v4().to_string();
        state
            .db
            .authorize_project_registration(&project_id, &owner.to_string())
            .await
            .unwrap();
        state
            .db
            .add_project_member(&owner.to_string(), &project_id, &member.to_string())
            .await
            .unwrap();

        let outsider_transfer = transfer_owner(
            State(state.clone()),
            headers(&state, outsider),
            Path(project_id.clone()),
            Json(TransferOwnerRequest {
                user_id: member.to_string(),
            }),
        )
        .await;
        assert!(matches!(outsider_transfer, Err(AppError::Forbidden(_))));
        let ineligible = transfer_owner(
            State(state.clone()),
            headers(&state, owner),
            Path(project_id.clone()),
            Json(TransferOwnerRequest {
                user_id: outsider.to_string(),
            }),
        )
        .await;
        assert!(matches!(ineligible, Err(AppError::Conflict(_))));

        let _ = transfer_owner(
            State(state.clone()),
            headers(&state, owner),
            Path(project_id.clone()),
            Json(TransferOwnerRequest {
                user_id: member.to_string(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            state
                .db
                .get_project_role_for_user(&project_id, &owner.to_string())
                .await
                .unwrap()
                .as_deref(),
            Some("user")
        );
        assert!(matches!(
            transfer_owner(
                State(state.clone()),
                headers(&state, owner),
                Path(project_id.clone()),
                Json(TransferOwnerRequest {
                    user_id: outsider.to_string(),
                }),
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
        assert!(matches!(
            leave_project(
                State(state.clone()),
                headers(&state, member),
                Path(project_id.clone()),
            )
            .await,
            Err(AppError::Conflict(_))
        ));

        state
            .db
            .set_project_lifecycle(
                &member.to_string(),
                &project_id,
                ProjectLifecycle::Suspended,
            )
            .await
            .unwrap();
        let _ = leave_project(
            State(state.clone()),
            headers(&state, owner),
            Path(project_id.clone()),
        )
        .await
        .unwrap();
        assert!(state
            .db
            .get_project_role_for_user(&project_id, &owner.to_string())
            .await
            .unwrap()
            .is_none());
        assert!(matches!(
            leave_project(
                State(state.clone()),
                headers(&state, suspended),
                Path(project_id),
            )
            .await,
            Err(AppError::AuthError(_))
        ));
    }

    /// Beginning a deletion records the project closed by its owner, as an
    /// administrative stop, so a host that returns while the project is
    /// being deleted is not started.
    #[tokio::test]
    async fn beginning_a_deletion_records_the_project_closed() {
        use shared::{OpenStateSetKind, ProjectOpenState};
        let state = test_state().await;
        let owner = Uuid::new_v4();
        add_user(&state, owner, "active").await;
        let project_id = Uuid::new_v4().to_string();
        state
            .db
            .authorize_project_registration(&project_id, &owner.to_string())
            .await
            .unwrap();
        assert!(state
            .db
            .set_project_open_state(
                &project_id,
                ProjectOpenState::Open,
                &owner.to_string(),
                OpenStateSetKind::Request,
                None,
            )
            .await
            .unwrap());

        // Hold the cleanup back so the record can still be read.
        let cleanup_blocker = state.project_deletion_guard(&project_id).await;
        let (status, _) = delete_project(
            State(state.clone()),
            headers(&state, owner),
            Path(project_id.clone()),
            Json(DeleteProjectRequest {
                confirmation: project_id.clone(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::ACCEPTED);
        let record = state
            .db
            .get_project_open_state(&project_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.lifecycle, ProjectLifecycle::Deleting);
        assert_eq!(record.state, Some(ProjectOpenState::Closed));
        assert_eq!(record.set_by_account(), Some(owner));
        assert_eq!(record.set_kind, Some(OpenStateSetKind::Administrative));
        let audited = state
            .db
            .list_cluster_audit_events(&owner.to_string(), 50, 0)
            .await
            .unwrap();
        assert!(audited.iter().any(|event| {
            event.action == "project.closed"
                && event
                    .details
                    .as_deref()
                    .is_some_and(|details| details.contains("\"deletion\""))
        }));
        // It cannot be opened while it is being deleted.
        assert!(state
            .db
            .set_project_open_state(
                &project_id,
                ProjectOpenState::Open,
                &owner.to_string(),
                OpenStateSetKind::Request,
                None,
            )
            .await
            .is_err());
        drop(cleanup_blocker);
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if state.db.get_project(&project_id).await.unwrap().is_none() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn deletion_confirmation_is_owner_bound_repeatable_and_recoverable() {
        let state = test_state().await;
        let owner = Uuid::new_v4();
        let outsider = Uuid::new_v4();
        add_user(&state, owner, "active").await;
        add_user(&state, outsider, "active").await;
        let project_id = Uuid::new_v4().to_string();
        state
            .db
            .authorize_project_registration(&project_id, &owner.to_string())
            .await
            .unwrap();

        assert!(matches!(
            delete_project(
                State(state.clone()),
                headers(&state, outsider),
                Path(project_id.clone()),
                Json(DeleteProjectRequest {
                    confirmation: project_id.clone(),
                }),
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
        assert!(matches!(
            delete_project(
                State(state.clone()),
                headers(&state, owner),
                Path(project_id.clone()),
                Json(DeleteProjectRequest {
                    confirmation: "another-project".to_string(),
                }),
            )
            .await,
            Err(AppError::BadRequest(_))
        ));

        let cleanup_blocker = state.project_deletion_guard(&project_id).await;
        for _ in 0..2 {
            let (status, response) = delete_project(
                State(state.clone()),
                headers(&state, owner),
                Path(project_id.clone()),
                Json(DeleteProjectRequest {
                    confirmation: project_id.clone(),
                }),
            )
            .await
            .unwrap();
            assert_eq!(status, StatusCode::ACCEPTED);
            assert_eq!(response.status, "deleting");
        }
        let _ = deletion_status(
            State(state.clone()),
            headers(&state, owner),
            Path(project_id.clone()),
        )
        .await
        .unwrap();
        drop(cleanup_blocker);

        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if state.db.get_project(&project_id).await.unwrap().is_none() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(matches!(
            deletion_status(
                State(state.clone()),
                headers(&state, owner),
                Path(project_id),
            )
            .await,
            Err(AppError::NotFound(_))
        ));
    }
}
