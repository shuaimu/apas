//! Autonomous execution policy is deliberately independent of terminal policy.
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use shared::{EffectiveJobPolicy, JobPolicy, JobPolicyOverride, JobPolicySource};
use sqlx::SqliteConnection;
use uuid::Uuid;

use super::{Database, SYSTEM_ADMIN_ACTOR};
use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobAction {
    Read,
    Admit,
    Answer,
    Cancel,
    Retry,
    Publish,
}

fn normalize_profiles(profiles: &mut Vec<String>) -> Result<()> {
    anyhow::ensure!(
        profiles.iter().all(|p| !p.is_empty()
            && p.len() <= 128
            && p.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))),
        "job_policy_invalid: invalid profile identifier"
    );
    profiles.sort();
    profiles.dedup();
    Ok(())
}

fn validate_policy(policy: &mut JobPolicy) -> Result<()> {
    normalize_profiles(&mut policy.worker_profiles)?;
    normalize_profiles(&mut policy.model_profiles)?;
    normalize_profiles(&mut policy.verification_profiles)?;
    anyhow::ensure!(
        policy.limits.is_finite_positive()
            && policy.max_active_project > 0
            && policy.max_active_machine > 0
            && policy.max_active_cluster > 0
            && policy.max_queued_project > 0,
        "job_policy_invalid: finite positive ceilings required"
    );
    Ok(())
}

fn normalize_override(policy: &mut JobPolicyOverride) -> Result<()> {
    for profiles in [
        &mut policy.worker_profiles,
        &mut policy.model_profiles,
        &mut policy.verification_profiles,
    ]
    .into_iter()
    .flatten()
    {
        normalize_profiles(profiles)?;
    }
    anyhow::ensure!(
        policy
            .limits
            .as_ref()
            .is_none_or(|v| v.is_finite_positive())
            && [
                policy.max_active_project,
                policy.max_active_machine,
                policy.max_active_cluster,
                policy.max_queued_project
            ]
            .iter()
            .all(|v| v.is_none_or(|n| n > 0)),
        "job_policy_invalid: finite positive ceilings required"
    );
    Ok(())
}

fn narrow(policy: &mut JobPolicy, value: &JobPolicyOverride) {
    if let Some(enabled) = value.enabled {
        policy.enabled &= enabled;
    }
    if let Some(enabled) = value.publish_enabled {
        policy.publish_enabled &= enabled;
    }
    for (profiles, allowed) in [
        (&mut policy.worker_profiles, &value.worker_profiles),
        (&mut policy.model_profiles, &value.model_profiles),
        (
            &mut policy.verification_profiles,
            &value.verification_profiles,
        ),
    ] {
        if let Some(allowed) = allowed {
            profiles.retain(|p| allowed.contains(p));
        }
    }
    if let Some(limits) = &value.limits {
        policy.limits = policy.limits.narrowed_by(limits);
    }
    if let Some(n) = value.max_active_project {
        policy.max_active_project = policy.max_active_project.min(n);
    }
    if let Some(n) = value.max_active_machine {
        policy.max_active_machine = policy.max_active_machine.min(n);
    }
    if let Some(n) = value.max_active_cluster {
        policy.max_active_cluster = policy.max_active_cluster.min(n);
    }
    if let Some(n) = value.max_queued_project {
        policy.max_queued_project = policy.max_queued_project.min(n);
    }
}

fn ensure_narrows(value: &JobPolicyOverride, bound: &JobPolicy) -> Result<()> {
    anyhow::ensure!(
        value.enabled != Some(true) || bound.enabled,
        "job policy cannot enable execution denied by inherited policy"
    );
    anyhow::ensure!(
        value.publish_enabled != Some(true) || bound.publish_enabled,
        "job policy cannot enable publication denied by inherited policy"
    );
    for (requested, allowed) in [
        (&value.worker_profiles, &bound.worker_profiles),
        (&value.model_profiles, &bound.model_profiles),
        (&value.verification_profiles, &bound.verification_profiles),
    ] {
        anyhow::ensure!(
            requested
                .as_ref()
                .is_none_or(|profiles| profiles.iter().all(|p| allowed.contains(p))),
            "job policy profiles exceed inherited policy"
        );
    }
    anyhow::ensure!(
        value
            .limits
            .as_ref()
            .is_none_or(|limits| limits.fits_within(&bound.limits)),
        "job policy limits exceed inherited policy"
    );
    for (requested, ceiling) in [
        (value.max_active_project, bound.max_active_project),
        (value.max_active_machine, bound.max_active_machine),
        (value.max_active_cluster, bound.max_active_cluster),
        (value.max_queued_project, bound.max_queued_project),
    ] {
        anyhow::ensure!(
            requested.is_none_or(|n| n <= ceiling),
            "job policy concurrency exceeds inherited policy"
        );
    }
    Ok(())
}

async fn source(
    conn: &mut SqliteConnection,
    scope: &str,
    id: &str,
) -> Result<(Option<String>, JobPolicySource)> {
    let row = sqlx::query_as::<_, (String, i64)>("SELECT policy_json, revision FROM autonomous_job_policies WHERE scope = ? AND scope_id = ?")
        .bind(scope).bind(id).fetch_optional(conn).await?;
    let (raw, revision) = match row {
        Some((raw, revision)) => (
            Some(raw),
            u64::try_from(revision).context("job_policy_invalid: invalid revision")?,
        ),
        None => (None, 0),
    };
    Ok((
        raw,
        JobPolicySource {
            scope: scope.into(),
            id: id.into(),
            revision,
        },
    ))
}

fn snapshot(
    policy: JobPolicy,
    sources: Vec<JobPolicySource>,
    requested: Option<JobPolicyOverride>,
) -> Result<EffectiveJobPolicy> {
    let canonical = serde_json::to_vec(&(&policy, &sources))?;
    let digest = format!("{:x}", Sha256::digest(canonical));
    Ok(EffectiveJobPolicy {
        policy,
        sources,
        requested,
        digest,
    })
}

async fn deployment(conn: &mut SqliteConnection) -> Result<EffectiveJobPolicy> {
    let (raw, revision) = source(conn, "deployment", "deployment").await?;
    let mut policy = match raw {
        Some(raw) => serde_json::from_str::<JobPolicy>(&raw)
            .context("job_policy_invalid: deployment configuration")?,
        None => JobPolicy::default(),
    };
    validate_policy(&mut policy)?;
    snapshot(policy, vec![revision], None)
}

async fn apply_source(
    conn: &mut SqliteConnection,
    effective: &mut EffectiveJobPolicy,
    scope: &str,
    id: &str,
) -> Result<Option<JobPolicyOverride>> {
    let (raw, revision) = source(conn, scope, id).await?;
    effective.sources.push(revision);
    let requested = raw
        .map(|raw| -> Result<JobPolicyOverride> {
            let mut value = serde_json::from_str::<JobPolicyOverride>(&raw)
                .context("job_policy_invalid: inherited configuration")?;
            normalize_override(&mut value)?;
            Ok(value)
        })
        .transpose()?;
    if let Some(value) = &requested {
        narrow(&mut effective.policy, value);
    }
    Ok(requested)
}

async fn resolve(
    conn: &mut SqliteConnection,
    scope: &str,
    id: &str,
    include_requested: bool,
) -> Result<EffectiveJobPolicy> {
    let mut effective = deployment(conn).await?;
    match scope {
        "deployment" => return Ok(effective),
        "cluster" => {
            if include_requested {
                effective.requested = apply_source(conn, &mut effective, "cluster", id).await?;
            }
        }
        "project" => {
            let exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM projects WHERE id = ?")
                .bind(id)
                .fetch_one(&mut *conn)
                .await?;
            anyhow::ensure!(exists > 0, "project not found");
            // All durable placements participate, including a currently suspended/offline host.
            let clusters = sqlx::query_scalar::<_, String>("SELECT cluster_owner_user_id FROM project_cluster_placements WHERE project_id = ? ORDER BY cluster_owner_user_id")
                .bind(id).fetch_all(&mut *conn).await?;
            for cluster in clusters {
                apply_source(conn, &mut effective, "cluster", &cluster).await?;
            }
            if include_requested {
                effective.requested = apply_source(conn, &mut effective, "project", id).await?;
            }
        }
        _ => anyhow::bail!("invalid job policy scope"),
    }
    snapshot(effective.policy, effective.sources, effective.requested)
}

impl Database {
    pub async fn migrate_job_policy(&self) -> Result<()> {
        sqlx::query("CREATE TABLE IF NOT EXISTS autonomous_job_policies (scope TEXT NOT NULL CHECK(scope IN ('deployment','cluster','project')), scope_id TEXT NOT NULL, policy_json TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision > 0), PRIMARY KEY(scope, scope_id))").execute(&self.pool).await?;
        Ok(())
    }

    pub async fn job_policy_requested(
        &self,
        scope: &str,
        id: &str,
    ) -> Result<Option<serde_json::Value>> {
        let mut conn = self.pool.acquire().await?;
        let (raw, _) = source(&mut conn, scope, id).await?;
        raw.map(|raw| {
            serde_json::from_str(&raw).context("job_policy_invalid: stored configuration")
        })
        .transpose()
    }

    pub async fn job_policy_scope(
        &self,
        scope: &str,
        id: &str,
        include_requested: bool,
    ) -> Result<EffectiveJobPolicy> {
        let mut tx = self.pool.begin().await?;
        let effective = resolve(&mut tx, scope, id, include_requested).await?;
        tx.commit().await?;
        Ok(effective)
    }

    pub async fn set_job_policy(
        &self,
        actor: &str,
        scope: &str,
        id: &str,
        value: serde_json::Value,
    ) -> Result<EffectiveJobPolicy> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE autonomous_job_policies SET revision = revision WHERE 0")
            .execute(&mut *tx)
            .await?;
        // Recheck management authority at the write boundary, not from a role sent by the client.
        if actor != SYSTEM_ADMIN_ACTOR {
            let authorized = match scope {
                "cluster" => sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE id = ? AND id = ? AND account_status = 'active'").bind(actor).bind(id).fetch_one(&mut *tx).await? > 0,
                "project" => sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM project_cluster_placements p JOIN users u ON u.id = p.cluster_owner_user_id WHERE p.project_id = ? AND u.id = ? AND u.account_status = 'active'").bind(id).bind(actor).fetch_one(&mut *tx).await? > 0,
                _ => false,
            };
            anyhow::ensure!(authorized, "job policy administration denied");
        }
        let canonical = if scope == "deployment" {
            let mut policy: JobPolicy = serde_json::from_value(value)
                .context("job_policy_invalid: deployment configuration")?;
            validate_policy(&mut policy)?;
            serde_json::to_string(&policy)?
        } else {
            anyhow::ensure!(
                scope == "cluster" || scope == "project",
                "invalid job policy scope"
            );
            let mut policy: JobPolicyOverride = serde_json::from_value(value)
                .context("job_policy_invalid: override configuration")?;
            normalize_override(&mut policy)?;
            let inherited = resolve(&mut tx, scope, id, false).await?;
            ensure_narrows(&policy, &inherited.policy)?;
            serde_json::to_string(&policy)?
        };
        sqlx::query("INSERT INTO autonomous_job_policies(scope, scope_id, policy_json, revision) VALUES (?, ?, ?, 1) ON CONFLICT(scope, scope_id) DO UPDATE SET policy_json = excluded.policy_json, revision = autonomous_job_policies.revision + 1")
            .bind(scope).bind(id).bind(canonical).execute(&mut *tx).await?;
        let effective = resolve(&mut tx, scope, id, true).await?;
        Self::insert_audit_tx(&mut tx, actor, "autonomous_job.policy_updated", if scope == "deployment" { "deployment" } else { scope }, id,
            Some(serde_json::json!({"policy_digest": effective.digest, "sources": effective.sources}))).await?;
        tx.commit().await?;
        Ok(effective)
    }
}

pub async fn effective_job_policy(db: &Database, project_id: &str) -> Result<EffectiveJobPolicy> {
    db.job_policy_scope("project", project_id, true).await
}

/// Account-owned runtime authority: never infer compute permission from a project share,
/// a connected terminal, a mobile device, or a legacy administrator account role.
pub async fn authorize_job_action(
    state: &AppState,
    user_id: Uuid,
    project_id: &str,
    host_owner: Uuid,
    machine_id: Uuid,
    initiator: Uuid,
    action: JobAction,
) -> Result<bool> {
    let db = &state.db;
    let actor = user_id.to_string();
    if !db
        .get_user_by_id(&actor)
        .await?
        .is_some_and(|user| user.is_active())
    {
        return Ok(false);
    }
    if action == JobAction::Read {
        return db.has_project_content_access(project_id, &actor).await;
    }
    let Some(project) = db.get_project(project_id).await? else {
        return Ok(false);
    };
    let owner = project.owner_user_id == actor;
    let operator = user_id == host_owner;
    if !(owner
        || operator
        || (matches!(
            action,
            JobAction::Answer | JobAction::Cancel | JobAction::Publish
        ) && user_id == initiator))
    {
        return Ok(false);
    }
    if !db
        .project_is_placed_in_cluster(project_id, &host_owner.to_string())
        .await?
    {
        return Ok(false);
    }
    // A known conflicting live machine identity is never accepted. Offline durable targets
    // still require DB authority and must remain cancellable/reconcilable.
    if state
        .sessions
        .daemon_owner(&machine_id)
        .is_some_and(|actual| actual != host_owner)
    {
        return Ok(false);
    }
    if !operator {
        if !db
            .is_active_cluster_member(&host_owner.to_string(), &actor)
            .await?
        {
            return Ok(false);
        }
        let Some(membership) = db
            .get_cluster_membership(&host_owner.to_string(), &actor)
            .await?
        else {
            return Ok(false);
        };
        // NULL is the existing explicit all-machines grant. A malformed explicit grant is
        // not equivalent to NULL and cannot authorize an autonomous runtime.
        if let Some(raw) = &membership.allowed_machine_ids {
            let Ok(machines) = serde_json::from_str::<Vec<Uuid>>(raw) else {
                return Ok(false);
            };
            if !machines.contains(&machine_id) {
                return Ok(false);
            }
        }
    }
    if action == JobAction::Cancel {
        return Ok(true);
    }
    if project.lifecycle_status != "active"
        || !db.has_project_content_access(project_id, &actor).await?
    {
        return Ok(false);
    }
    let effective = effective_job_policy(db, project_id).await?;
    Ok(effective.policy.enabled
        && (action != JobAction::Publish || effective.policy.publish_enabled))
}

#[cfg(test)]
#[path = "job_policy_tests.rs"]
mod tests;
