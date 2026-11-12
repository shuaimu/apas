//! Joining a checkout to an existing project, so that it becomes another
//! machine's instance of that project instead of a project of its own.
//!
//! `GET /projects/by-remote` answers `apas register`: which of the caller's
//! projects use the checkout's repository, and what the server knows of the
//! identity the checkout has now. A web `CreateProjectInstance` naming
//! `join_project_id` clones a project onto one of the requester's own
//! machines as another checkout of it. Both are limited to projects the
//! caller owns or hosts.

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    Json,
};
use serde::Deserialize;
use shared::{
    MachineWithProjects, ProjectRemoteInstance, ProjectRemoteMatch, ProjectsByRemoteResponse,
    ServerToDaemon, ServerToWeb,
};
use std::collections::BTreeSet;
use uuid::Uuid;

use crate::{
    db::ProjectLifecycle,
    error::AppError,
    routes::authz::require_active_user,
    session::{normalize_machine_hostname, normalize_project_path},
    state::AppState,
};

/// The canonical lowercase `host/owner/repo` key of a git remote URL, made
/// exactly as the CLI's `worktree::normalized_git_remote` makes it from a
/// checkout's `origin`. Every remote the server records arrives in that form
/// (`SessionStart.git_remote`, a daemon's `MachineProjectInfo.git_remote`),
/// so lookups compare in it too. The CLI's copy is private to the CLI; the
/// two must stay identical.
///
///   `git@github.com:Owner/Repo.git`     -> `github.com/owner/repo`
///   `https://github.com/owner/repo.git` -> `github.com/owner/repo`
///   `ssh://git@github.com/owner/repo`   -> `github.com/owner/repo`
pub(crate) fn canonical_git_remote(raw: &str) -> String {
    // Trim trailing slashes, strip a single `.git`, then trim slashes again so
    // `repo`, `repo/`, `repo.git`, and `repo.git/` all collapse to one key.
    let s = raw.trim().trim_end_matches('/');
    let s = s.strip_suffix(".git").unwrap_or(s).trim_end_matches('/');

    let (host, path) = if let Some(idx) = s.find("://") {
        // scheme://[user@]authority[:port]/owner/repo...
        let rest = &s[idx + 3..];
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        // An IPv6 literal authority is `[addr]:port`; the address itself
        // contains colons, so pull it out of the brackets before the port split.
        let host = if let Some(rest) = authority.strip_prefix('[') {
            rest.split_once(']').map_or(authority, |(addr, _)| addr)
        } else {
            authority.split_once(':').map_or(authority, |(h, _)| h)
        };
        (host.to_string(), path.to_string())
    } else if let Some((before, after)) = s.split_once(':') {
        // scp-like `[user@]host:owner/repo`, only when the host part has no `/`.
        if before.contains('/') {
            (String::new(), s.to_string())
        } else {
            let host = before.rsplit_once('@').map_or(before, |(_, h)| h);
            (host.to_string(), after.to_string())
        }
    } else {
        // Bare `owner/repo`, a local path, or anything unparseable.
        (String::new(), s.to_string())
    };

    let mut combined = if host.is_empty() {
        path
    } else {
        format!("{host}/{path}")
    };
    while combined.contains("//") {
        combined = combined.replace("//", "/");
    }
    combined.trim_matches('/').to_lowercase()
}

#[derive(Debug, Default, Deserialize)]
pub struct ProjectsByRemoteQuery {
    #[serde(default)]
    git_remote: Option<String>,
    #[serde(default)]
    project_id: Option<String>,
}

/// `GET /projects/by-remote?git_remote=<host/owner/repo>&project_id=<uuid>`:
/// the caller's projects for one repository, so `apas register` can offer to
/// join a checkout to one of them; see [`ProjectsByRemoteResponse`].
///
/// A project matches when the caller owns or hosts it and either one of its
/// sessions recorded the remote or one of the caller's own machines lists it
/// with that remote. The machine inventory is what lets a registered project
/// that never started match. Such a project may have no project record yet:
/// only the caller's machine lists it, and its first start makes it the
/// caller's. Another account's machines never count.
///
/// No match is an empty answer, never a 404, which the CLI takes to mean a
/// server that predates this route.
pub async fn projects_by_remote(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ProjectsByRemoteQuery>,
) -> Result<Json<ProjectsByRemoteResponse>, AppError> {
    let user = require_active_user(&headers, &state).await?;
    let requested = query.git_remote.as_deref().unwrap_or_default().trim();
    let git_remote = canonical_git_remote(requested);
    if git_remote.is_empty() {
        return Err(AppError::BadRequest(
            "git_remote must name a repository, as host/owner/repo".to_string(),
        ));
    }
    let local_id =
        match query
            .project_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            Some(id) => Some(Uuid::parse_str(id).map_err(|_| {
                AppError::BadRequest("project_id must be a project UUID".to_string())
            })?),
            None => None,
        };
    // The CLI sends the canonical form already. Canonicalizing that again
    // does not always give it back (an IPv6 host keeps its colons), so the
    // remote as sent counts as well as its canonical spelling.
    let mut spellings = vec![git_remote.as_str()];
    if requested != git_remote {
        spellings.push(requested);
    }
    let inventory = Uuid::parse_str(&user.id)
        .map(|caller| state.sessions.get_machines_for_user(&caller))
        .unwrap_or_default();

    let mut candidates: BTreeSet<String> = state
        .db
        .owned_or_hosted_projects_with_session_remote(&user.id, &spellings)
        .await?
        .into_iter()
        .collect();
    candidates.extend(
        inventory
            .iter()
            .flat_map(|machine| machine.projects.iter())
            .filter(|project| {
                project
                    .git_remote
                    .as_deref()
                    .is_some_and(|remote| spellings.contains(&remote))
            })
            .map(|project| project.project_id.clone()),
    );

    let mut projects = Vec::new();
    for project_id in candidates {
        let Ok(project_uuid) = Uuid::parse_str(&project_id) else {
            continue;
        };
        let standing = state
            .db
            .project_join_standing(&project_id, &user.id)
            .await?;
        // A project being deleted is not offered; one with no record yet is
        // the caller's only through their own machine's registration.
        let joinable = match standing {
            Some(standing) => {
                standing.owned_or_hosted && standing.lifecycle != ProjectLifecycle::Deleting
            }
            None => listed_on(&inventory, &project_id),
        };
        if joinable {
            projects.push(remote_match(
                &state,
                &inventory,
                project_uuid,
                &project_id,
                standing.is_some_and(|standing| standing.has_session),
            ));
        }
    }
    projects.sort_by(|a, b| {
        let key = |project: &ProjectRemoteMatch| {
            (
                project.name.is_none(),
                project.name.as_deref().map(str::to_lowercase),
            )
        };
        key(a)
            .cmp(&key(b))
            .then_with(|| a.project_id.cmp(&b.project_id))
    });

    // The checkout's own identity, whatever its sessions recorded, so the
    // CLI never re-identifies a project with history. Its lifecycle does not
    // matter here: history is history.
    let mut local = None;
    if let Some(local_uuid) = local_id {
        let project_id = local_uuid.to_string();
        let standing = state
            .db
            .project_join_standing(&project_id, &user.id)
            .await?;
        let known = match standing {
            Some(standing) => standing.owned_or_hosted,
            None => listed_on(&inventory, &project_id),
        };
        if known {
            local = Some(remote_match(
                &state,
                &inventory,
                local_uuid,
                &project_id,
                standing.is_some_and(|standing| standing.has_session),
            ));
        }
    }

    Ok(Json(ProjectsByRemoteResponse {
        git_remote,
        projects,
        local,
    }))
}

/// Whether one of the caller's own machines lists `project_id`.
fn listed_on(inventory: &[MachineWithProjects], project_id: &str) -> bool {
    inventory.iter().any(|machine| {
        machine
            .projects
            .iter()
            .any(|project| project.project_id == project_id)
    })
}

fn remote_match(
    state: &AppState,
    inventory: &[MachineWithProjects],
    project_uuid: Uuid,
    project_id: &str,
    has_session: bool,
) -> ProjectRemoteMatch {
    ProjectRemoteMatch {
        project_id: project_uuid,
        name: registered_name(inventory, project_id),
        has_session,
        instances: checkouts(state, inventory, project_id),
    }
}

/// The project's `.apas` name, which only a machine's registration carries;
/// the alphabetically first host's when the caller's machines disagree.
fn registered_name(inventory: &[MachineWithProjects], project_id: &str) -> Option<String> {
    inventory
        .iter()
        .flat_map(|machine| {
            machine
                .projects
                .iter()
                .filter(|project| project.project_id == project_id)
                .filter_map(move |project| {
                    let name = project.name.as_deref()?.trim();
                    (!name.is_empty()).then(|| (machine.machine.hostname.as_str(), name))
                })
        })
        .min()
        .map(|(_, name)| name.to_string())
}

/// Where `project_id` is checked out, as far as the caller may know: the
/// caller's own machines that list it, and every instance attached to the
/// project now, which anyone with access to the project sees in its session
/// summary anyway. An attached instance on a listed machine marks that
/// checkout running instead of adding another; one that reported no machine
/// is matched by host and path. Running checkouts come first.
fn checkouts(
    state: &AppState,
    inventory: &[MachineWithProjects],
    project_id: &str,
) -> Vec<ProjectRemoteInstance> {
    let mut checkouts: Vec<ProjectRemoteInstance> = inventory
        .iter()
        .flat_map(|machine| {
            machine
                .projects
                .iter()
                .filter(|project| project.project_id == project_id)
                .map(move |project| ProjectRemoteInstance {
                    machine_id: Some(machine.machine.machine_id),
                    hostname: machine.machine.hostname.clone(),
                    path: project.path.clone(),
                    running: project.is_running,
                })
        })
        .collect();
    for session_id in state.sessions.project_session_ids(project_id) {
        for instance in state.sessions.session_attachments(&session_id) {
            let hostname = instance
                .hostname
                .clone()
                .filter(|host| !host.trim().is_empty())
                .or_else(|| {
                    instance
                        .machine_id
                        .and_then(|machine_id| state.sessions.machine_hostname(&machine_id))
                });
            let known = checkouts
                .iter_mut()
                .find(|checkout| match instance.machine_id {
                    Some(machine_id) => checkout.machine_id == Some(machine_id),
                    None => {
                        hostname.as_deref().is_some_and(|host| {
                            normalize_machine_hostname(host)
                                == normalize_machine_hostname(&checkout.hostname)
                        }) && instance.working_dir.as_deref().is_some_and(|dir| {
                            normalize_project_path(dir) == normalize_project_path(&checkout.path)
                        })
                    }
                });
            if let Some(known) = known {
                known.running = true;
                continue;
            }
            let (Some(hostname), Some(path)) = (hostname, instance.working_dir) else {
                continue;
            };
            checkouts.push(ProjectRemoteInstance {
                machine_id: instance.machine_id,
                hostname,
                path,
                running: true,
            });
        }
    }
    checkouts.sort_by(|a, b| {
        b.running
            .cmp(&a.running)
            .then_with(|| a.hostname.cmp(&b.hostname))
            .then_with(|| a.path.cmp(&b.path))
    });
    checkouts
}

/// A web `CreateProjectInstance` that names `join_project_id`: clone a
/// project's repository onto one of the requester's own machines as another
/// checkout of that project.
pub(super) struct JoinRequest {
    pub machine_id: Uuid,
    pub cluster_owner_user_id: Option<String>,
    pub git_remote: String,
    pub instance_name: String,
    pub branch: String,
    pub clone_url: Option<String>,
    pub base_path: Option<String>,
    pub request_id: Option<String>,
    pub project_id: Uuid,
}

/// Forward a clone-to-join to the machine's daemon, or refuse it.
///
/// Only the owner path joins: the requester owns the machine and owns or
/// hosts the project, and the machine's daemon is connected and advertises
/// project instances. An older daemon would ignore `join_existing` and, on
/// this path, branch, mint an unrelated project and start it.
///
/// A refusal goes to the requesting connection as the `ProjectInstanceCreated`
/// error the client waits on for this request; the daemon's own answer is
/// relayed when it comes (`ws_daemon`). The server chains nothing after it:
/// the client asks for the checkout's first tab with an ordinary `AddPane`.
pub(super) async fn create_joined_instance(
    state: &AppState,
    connection_id: &Uuid,
    requester: Uuid,
    request: JoinRequest,
) {
    let request_id = request
        .request_id
        .clone()
        .filter(|value| !value.trim().is_empty() && value.len() <= 128)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let project_id = request.project_id.to_string();
    // Deletion marks the project and then waits for these permits, so a join
    // either sees the project deleting or reaches the daemon first.
    let _project_guard = state.project_operation_guard(&project_id).await;
    let refusal = match join_refusal(state, requester, &request).await {
        Err(reason) => reason,
        Ok(()) => {
            tracing::info!(
                user = %requester,
                machine = %request.machine_id,
                project = %project_id,
                repo = %request.git_remote,
                "CreateProjectInstance: joining an existing project"
            );
            let command = ServerToDaemon::CreateProjectInstance {
                git_remote: request.git_remote,
                instance_name: request.instance_name,
                // The daemon creates no branch for a join.
                branch: request.branch,
                clone_url: request.clone_url,
                base_path: request.base_path,
                request_id: Some(request_id.clone()),
                project_id: Some(project_id),
                provisioning_mode: shared::ProjectProvisioningMode::TrustedOwner,
                join_existing: true,
            };
            if state
                .sessions
                .send_to_daemon(&request.machine_id, command)
                .await
            {
                return;
            }
            format!("{} is offline", host_label(state, request.machine_id))
        }
    };
    state
        .sessions
        .send_to_web(
            connection_id,
            ServerToWeb::ProjectInstanceCreated {
                machine_id: request.machine_id,
                request_id: Some(request_id),
                project_id: None,
                error: Some(refusal),
            },
        )
        .await;
}

/// Why `requester` may not clone `request.project_id` onto
/// `request.machine_id`, if anything stops it. A machine the requester has no
/// relation to is not named.
async fn join_refusal(
    state: &AppState,
    requester: Uuid,
    request: &JoinRequest,
) -> Result<(), String> {
    let machine_id = request.machine_id;
    let host = host_label(state, machine_id);
    match state.sessions.daemon_owner(&machine_id) {
        Some(owner) if owner == requester => {}
        Some(owner) => {
            let visible = state
                .db
                .cluster_member_can_use_machine(
                    &owner.to_string(),
                    &requester.to_string(),
                    &machine_id.to_string(),
                )
                .await
                .unwrap_or(false);
            return Err(if visible {
                format!(
                    "Only your own machines can clone a project to join it, and {host} is in another account's cluster"
                )
            } else {
                "That machine is not available to you".to_string()
            });
        }
        None => return Err("That machine is not available to you".to_string()),
    }
    if request
        .cluster_owner_user_id
        .as_deref()
        .is_some_and(|cluster| cluster != requester.to_string())
    {
        return Err("Machine does not belong to the selected cluster".to_string());
    }

    let project_id = request.project_id.to_string();
    let standing = match state
        .db
        .project_join_standing(&project_id, &requester.to_string())
        .await
    {
        Ok(standing) => standing,
        Err(error) => {
            tracing::warn!(%project_id, %error, "could not look up a project to join");
            return Err("Could not verify this project".to_string());
        }
    };
    // The same answer whether the project is missing or someone else's, so
    // the refusal does not say which.
    let Some(standing) = standing.filter(|standing| standing.owned_or_hosted) else {
        return Err("Only a project you own or host can be cloned onto your machines".to_string());
    };
    match standing.lifecycle {
        ProjectLifecycle::Active => {}
        ProjectLifecycle::Suspended => {
            return Err("This project is suspended, so no instance of it can start".to_string())
        }
        ProjectLifecycle::Deleting => return Err("This project is being deleted".to_string()),
    }
    // Every instance runs under the account its session belongs to, and a
    // `SessionStart` from any other account is refused. A checkout on the
    // requester's machine runs under the requester, so it could never start.
    match state.db.get_session(&project_id).await {
        Ok(Some(session)) if session.user_id != requester.to_string() => {
            return Err(format!(
                "This project runs under another account, so an instance on {host} could not join it"
            ));
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(%project_id, %error, "could not look up the session of a project to join");
            return Err("Could not verify this project".to_string());
        }
    }

    if !state.sessions.is_daemon_connected(&machine_id) {
        return Err(format!("{host} is offline"));
    }
    // The checkout's first tab starts it through the start checks, which
    // need project policy as well.
    if !state
        .sessions
        .daemon_supports_capability(&machine_id, shared::PROJECT_INSTANCES_CAPABILITY)
        || !state
            .sessions
            .daemon_supports_capability(&machine_id, shared::PROJECT_POLICY_CAPABILITY)
    {
        return Err(format!(
            "{host} runs an older APAS that cannot run this project beside other machines. Reboot {host} to update it."
        ));
    }
    // One instance per machine: a second checkout there would be a second
    // copy of the project on one host.
    if state
        .sessions
        .machines_listing_project(&project_id)
        .contains(&machine_id)
    {
        return Err(format!(
            "{host} already has this project. Add the tab there instead."
        ));
    }
    Ok(())
}

fn host_label(state: &AppState, machine_id: Uuid) -> String {
    state
        .sessions
        .machine_hostname(&machine_id)
        .filter(|host| !host.trim().is_empty())
        .unwrap_or_else(|| "That machine".to_string())
}

#[cfg(test)]
mod tests;
