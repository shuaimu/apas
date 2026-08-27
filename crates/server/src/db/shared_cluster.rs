use anyhow::{Context, Result};

#[cfg(test)]
use super::ProjectClusterPlacement;
use super::{
    ClusterMembership, ClusterReference, Database, ProjectProvisioningRequest,
    SharedClusterInvitation,
};

impl Database {
    pub async fn create_shared_cluster_invitation(
        &self,
        id: &str,
        token_hash: &str,
        cluster_owner_user_id: &str,
        invitee_email: &str,
        expires_at: &str,
    ) -> Result<SharedClusterInvitation> {
        let normalized_email = invitee_email.trim().to_ascii_lowercase();
        let mut tx = self.pool.begin().await?;
        let owner_status =
            sqlx::query_scalar::<_, String>("SELECT account_status FROM users WHERE id = ?")
                .bind(cluster_owner_user_id)
                .fetch_optional(&mut *tx)
                .await?
                .context("cluster owner not found")?;
        anyhow::ensure!(owner_status == "active", "cluster owner is suspended");

        let invitee = sqlx::query_as::<_, (String, String)>(
            "SELECT id, account_status FROM users WHERE lower(email) = ?",
        )
        .bind(&normalized_email)
        .fetch_optional(&mut *tx)
        .await?
        .context("invitee must already have an APAS account")?;
        anyhow::ensure!(invitee.1 == "active", "invitee account is suspended");
        anyhow::ensure!(
            invitee.0 != cluster_owner_user_id,
            "cluster owner cannot invite themselves"
        );

        // Replacing a pending invitation makes every previously issued bearer
        // link unusable before the new one is inserted.
        sqlx::query(
            r#"UPDATE shared_cluster_invitations
               SET revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP)
               WHERE cluster_owner_user_id = ? AND invitee_user_id = ?
                 AND accepted_at IS NULL AND revoked_at IS NULL"#,
        )
        .bind(cluster_owner_user_id)
        .bind(&invitee.0)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO shared_cluster_invitations
               (id, token_hash, cluster_owner_user_id, invitee_user_id, email, expires_at)
               VALUES (?, ?, ?, ?, ?, ?)"#,
        )
        .bind(id)
        .bind(token_hash)
        .bind(cluster_owner_user_id)
        .bind(&invitee.0)
        .bind(&normalized_email)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;
        Self::insert_audit_tx(
            &mut tx,
            cluster_owner_user_id,
            "cluster.invitation_created",
            "cluster",
            cluster_owner_user_id,
            Some(serde_json::json!({
                "invitation_id": id,
                "invitee_user_id": invitee.0,
                "invitee_email": normalized_email,
                "expires_at": expires_at,
            })),
        )
        .await?;
        tx.commit().await?;
        self.get_shared_cluster_invitation_by_id(id)
            .await?
            .context("created cluster invitation disappeared")
    }

    pub async fn get_shared_cluster_invitation_by_id(
        &self,
        id: &str,
    ) -> Result<Option<SharedClusterInvitation>> {
        self.get_shared_cluster_invitation("sci.id = ?", id).await
    }

    pub async fn get_shared_cluster_invitation_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<SharedClusterInvitation>> {
        self.get_shared_cluster_invitation("sci.token_hash = ?", token_hash)
            .await
    }

    async fn get_shared_cluster_invitation(
        &self,
        predicate: &str,
        value: &str,
    ) -> Result<Option<SharedClusterInvitation>> {
        // The predicate is selected only by the two private callers above.
        let query = format!(
            r#"SELECT sci.id, sci.cluster_owner_user_id,
                      owner.email AS cluster_owner_email, sci.invitee_user_id,
                      invitee.email AS invitee_email, sci.expires_at,
                      sci.accepted_at, sci.revoked_at, sci.created_at
               FROM shared_cluster_invitations sci
               JOIN users owner ON owner.id = sci.cluster_owner_user_id
               JOIN users invitee ON invitee.id = sci.invitee_user_id
               WHERE {predicate}"#
        );
        Ok(sqlx::query_as::<_, SharedClusterInvitation>(&query)
            .bind(value)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn list_shared_cluster_invitations(
        &self,
        cluster_owner_user_id: &str,
    ) -> Result<Vec<SharedClusterInvitation>> {
        Ok(sqlx::query_as::<_, SharedClusterInvitation>(
            r#"SELECT sci.id, sci.cluster_owner_user_id,
                      owner.email AS cluster_owner_email, sci.invitee_user_id,
                      invitee.email AS invitee_email, sci.expires_at,
                      sci.accepted_at, sci.revoked_at, sci.created_at
               FROM shared_cluster_invitations sci
               JOIN users owner ON owner.id = sci.cluster_owner_user_id
               JOIN users invitee ON invitee.id = sci.invitee_user_id
               WHERE sci.cluster_owner_user_id = ?
               ORDER BY sci.created_at DESC, sci.id DESC"#,
        )
        .bind(cluster_owner_user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn revoke_shared_cluster_invitation(
        &self,
        cluster_owner_user_id: &str,
        invitation_id: &str,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query(
            r#"UPDATE shared_cluster_invitations
               SET revoked_at = CURRENT_TIMESTAMP
               WHERE id = ? AND cluster_owner_user_id = ?
                 AND accepted_at IS NULL AND revoked_at IS NULL"#,
        )
        .bind(invitation_id)
        .bind(cluster_owner_user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        if changed {
            Self::insert_audit_tx(
                &mut tx,
                cluster_owner_user_id,
                "cluster.invitation_revoked",
                "cluster",
                cluster_owner_user_id,
                Some(serde_json::json!({ "invitation_id": invitation_id })),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(changed)
    }

    pub async fn accept_shared_cluster_invitation(
        &self,
        token_hash: &str,
        invitee_user_id: &str,
    ) -> Result<Option<ClusterMembership>> {
        let mut tx = self.pool.begin().await?;
        let invitation = sqlx::query_as::<_, (String, String, String, String)>(
            r#"SELECT sci.id, sci.cluster_owner_user_id, sci.invitee_user_id, sci.created_at
               FROM shared_cluster_invitations sci
               JOIN users owner ON owner.id = sci.cluster_owner_user_id
               JOIN users invitee ON invitee.id = sci.invitee_user_id
               WHERE sci.token_hash = ? AND sci.invitee_user_id = ?
                 AND sci.accepted_at IS NULL AND sci.revoked_at IS NULL
                 AND datetime(sci.expires_at) > CURRENT_TIMESTAMP
                 AND owner.account_status = 'active'
                 AND invitee.account_status = 'active'"#,
        )
        .bind(token_hash)
        .bind(invitee_user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((invitation_id, cluster_owner_user_id, addressed_user_id, created_at)) =
            invitation
        else {
            tx.rollback().await?;
            return Ok(None);
        };
        anyhow::ensure!(
            addressed_user_id == invitee_user_id,
            "invitation belongs to another account"
        );

        let consumed = sqlx::query(
            r#"UPDATE shared_cluster_invitations SET accepted_at = CURRENT_TIMESTAMP
               WHERE id = ? AND accepted_at IS NULL AND revoked_at IS NULL
                 AND datetime(expires_at) > CURRENT_TIMESTAMP"#,
        )
        .bind(&invitation_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        if !consumed {
            tx.rollback().await?;
            return Ok(None);
        }
        sqlx::query(
            r#"INSERT INTO cluster_memberships
               (cluster_owner_user_id, user_id, status, invited_at, accepted_at, revoked_at, updated_at)
               VALUES (?, ?, 'active', ?, CURRENT_TIMESTAMP, NULL, CURRENT_TIMESTAMP)
               ON CONFLICT(cluster_owner_user_id, user_id) DO UPDATE SET
                   status = 'active', invited_at = excluded.invited_at,
                   accepted_at = CURRENT_TIMESTAMP, revoked_at = NULL,
                   updated_at = CURRENT_TIMESTAMP"#,
        )
        .bind(&cluster_owner_user_id)
        .bind(invitee_user_id)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
        Self::insert_audit_tx(
            &mut tx,
            invitee_user_id,
            "cluster.member_joined",
            "cluster",
            &cluster_owner_user_id,
            Some(serde_json::json!({
                "invitation_id": invitation_id,
                "user_id": invitee_user_id,
            })),
        )
        .await?;
        tx.commit().await?;
        self.get_cluster_membership(&cluster_owner_user_id, invitee_user_id)
            .await
    }

    pub async fn get_cluster_membership(
        &self,
        cluster_owner_user_id: &str,
        user_id: &str,
    ) -> Result<Option<ClusterMembership>> {
        Ok(sqlx::query_as::<_, ClusterMembership>(
            r#"SELECT cm.cluster_owner_user_id, owner.email AS cluster_owner_email,
                      cm.user_id, member.email AS user_email, cm.status,
                      cm.invited_at, cm.accepted_at, cm.revoked_at, cm.updated_at
               FROM cluster_memberships cm
               JOIN users owner ON owner.id = cm.cluster_owner_user_id
               JOIN users member ON member.id = cm.user_id
               WHERE cm.cluster_owner_user_id = ? AND cm.user_id = ?"#,
        )
        .bind(cluster_owner_user_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn list_cluster_memberships(
        &self,
        cluster_owner_user_id: &str,
    ) -> Result<Vec<ClusterMembership>> {
        Ok(sqlx::query_as::<_, ClusterMembership>(
            r#"SELECT cm.cluster_owner_user_id, owner.email AS cluster_owner_email,
                      cm.user_id, member.email AS user_email, cm.status,
                      cm.invited_at, cm.accepted_at, cm.revoked_at, cm.updated_at
               FROM cluster_memberships cm
               JOIN users owner ON owner.id = cm.cluster_owner_user_id
               JOIN users member ON member.id = cm.user_id
               WHERE cm.cluster_owner_user_id = ?
               ORDER BY CASE cm.status WHEN 'active' THEN 0 ELSE 1 END,
                        lower(member.email)"#,
        )
        .bind(cluster_owner_user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn revoke_cluster_membership(
        &self,
        cluster_owner_user_id: &str,
        user_id: &str,
    ) -> Result<bool> {
        anyhow::ensure!(
            cluster_owner_user_id != user_id,
            "cluster owner cannot revoke themselves"
        );
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query(
            r#"UPDATE cluster_memberships
               SET status = 'revoked', revoked_at = CURRENT_TIMESTAMP,
                   updated_at = CURRENT_TIMESTAMP
               WHERE cluster_owner_user_id = ? AND user_id = ? AND status = 'active'"#,
        )
        .bind(cluster_owner_user_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        if changed {
            Self::insert_audit_tx(
                &mut tx,
                cluster_owner_user_id,
                "cluster.member_revoked",
                "cluster",
                cluster_owner_user_id,
                Some(serde_json::json!({ "user_id": user_id })),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(changed)
    }

    pub async fn is_active_cluster_member(
        &self,
        cluster_owner_user_id: &str,
        user_id: &str,
    ) -> Result<bool> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"SELECT COUNT(*) FROM cluster_memberships cm
               JOIN users owner ON owner.id = cm.cluster_owner_user_id
               JOIN users member ON member.id = cm.user_id
               WHERE cm.cluster_owner_user_id = ? AND cm.user_id = ?
                 AND cm.status = 'active'
                 AND owner.account_status = 'active'
                 AND member.account_status = 'active'"#,
        )
        .bind(cluster_owner_user_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    #[cfg(test)]
    pub async fn has_active_cluster_access(
        &self,
        cluster_owner_user_id: &str,
        user_id: &str,
    ) -> Result<bool> {
        if cluster_owner_user_id == user_id {
            return Ok(self
                .get_user_by_id(user_id)
                .await?
                .is_some_and(|user| user.is_active()));
        }
        self.is_active_cluster_member(cluster_owner_user_id, user_id)
            .await
    }

    pub async fn list_accessible_clusters(&self, user_id: &str) -> Result<Vec<ClusterReference>> {
        Ok(sqlx::query_as::<_, ClusterReference>(
            r#"SELECT u.id AS owner_user_id, u.email AS owner_email,
                      'owner' AS access, NULL AS accepted_at
               FROM users u
               WHERE u.id = ? AND u.account_status = 'active'
               UNION ALL
               SELECT owner.id, owner.email, 'member', cm.accepted_at
               FROM cluster_memberships cm
               JOIN users owner ON owner.id = cm.cluster_owner_user_id
               JOIN users member ON member.id = cm.user_id
               WHERE cm.user_id = ? AND cm.status = 'active'
                 AND owner.account_status = 'active'
                 AND member.account_status = 'active'
               ORDER BY access, owner_email"#,
        )
        .bind(user_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn add_project_cluster_placement(
        &self,
        project_id: &str,
        cluster_owner_user_id: &str,
        created_by_user_id: &str,
        source: &str,
    ) -> Result<bool> {
        let changed = sqlx::query(
            r#"INSERT OR IGNORE INTO project_cluster_placements
               (project_id, cluster_owner_user_id, created_by_user_id, source)
               SELECT p.id, owner.id, creator.id, ?
               FROM projects p, users owner, users creator
               WHERE p.id = ? AND owner.id = ? AND owner.account_status = 'active'
                 AND creator.id = ? AND creator.account_status = 'active'"#,
        )
        .bind(source)
        .bind(project_id)
        .bind(cluster_owner_user_id)
        .bind(created_by_user_id)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0;
        Ok(changed)
    }

    pub async fn project_is_placed_in_cluster(
        &self,
        project_id: &str,
        cluster_owner_user_id: &str,
    ) -> Result<bool> {
        Ok(sqlx::query_scalar::<_, i64>(
            r#"SELECT COUNT(*) FROM project_cluster_placements pcp
               JOIN users owner ON owner.id = pcp.cluster_owner_user_id
               WHERE pcp.project_id = ? AND pcp.cluster_owner_user_id = ?
                 AND owner.account_status = 'active'"#,
        )
        .bind(project_id)
        .bind(cluster_owner_user_id)
        .fetch_one(&self.pool)
        .await?
            > 0)
    }

    pub async fn has_project_content_access(
        &self,
        project_id: &str,
        user_id: &str,
    ) -> Result<bool> {
        Ok(sqlx::query_scalar::<_, i64>(
            r#"SELECT COUNT(*) FROM projects p
               JOIN users u ON u.id = ? AND u.account_status = 'active'
               WHERE p.id = ? AND p.lifecycle_status != 'deleting'
                 AND (
                     p.owner_user_id = u.id OR EXISTS (
                         SELECT 1 FROM project_members pm
                         WHERE pm.project_id = p.id AND pm.user_id = u.id
                     ) OR EXISTS (
                         SELECT 1 FROM project_cluster_placements pcp
                         WHERE pcp.project_id = p.id
                           AND pcp.cluster_owner_user_id = u.id
                     )
                 )"#,
        )
        .bind(user_id)
        .bind(project_id)
        .fetch_one(&self.pool)
        .await?
            > 0)
    }

    #[cfg(test)]
    pub async fn list_project_cluster_placements(
        &self,
        project_id: &str,
    ) -> Result<Vec<ProjectClusterPlacement>> {
        Ok(sqlx::query_as::<_, ProjectClusterPlacement>(
            r#"SELECT project_id, cluster_owner_user_id, created_by_user_id, source, created_at
               FROM project_cluster_placements WHERE project_id = ?
               ORDER BY created_at, cluster_owner_user_id"#,
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn claim_project_provisioning(
        &self,
        request_id: &str,
        requester_user_id: &str,
        cluster_owner_user_id: &str,
        machine_id: &str,
        request_fingerprint: &str,
        git_remote: &str,
        clone_url: &str,
        instance_name: &str,
        branch: &str,
        project_id: &str,
    ) -> Result<ProjectProvisioningRequest> {
        sqlx::query(
            r#"INSERT OR IGNORE INTO project_provisioning_requests
               (request_id, requester_user_id, cluster_owner_user_id, machine_id,
                request_fingerprint, git_remote, clone_url, instance_name, branch, project_id)
               SELECT ?, requester.id, owner.id, ?, ?, ?, ?, ?, ?, ?
               FROM users requester, users owner
               WHERE requester.id = ? AND requester.account_status = 'active'
                 AND owner.id = ? AND owner.account_status = 'active'
                 AND (
                     requester.id = owner.id OR EXISTS (
                         SELECT 1 FROM cluster_memberships cm
                         WHERE cm.cluster_owner_user_id = owner.id
                           AND cm.user_id = requester.id AND cm.status = 'active'
                     )
                 )"#,
        )
        .bind(request_id)
        .bind(machine_id)
        .bind(request_fingerprint)
        .bind(git_remote)
        .bind(clone_url)
        .bind(instance_name)
        .bind(branch)
        .bind(project_id)
        .bind(requester_user_id)
        .bind(cluster_owner_user_id)
        .execute(&self.pool)
        .await?;

        let record = self
            .get_project_provisioning(request_id, requester_user_id)
            .await?
            .context("provisioning request is unauthorized or unavailable")?;
        anyhow::ensure!(
            record.request_fingerprint == request_fingerprint,
            "request id was already used with different inputs"
        );
        Ok(record)
    }

    pub async fn get_project_provisioning(
        &self,
        request_id: &str,
        requester_user_id: &str,
    ) -> Result<Option<ProjectProvisioningRequest>> {
        Ok(sqlx::query_as::<_, ProjectProvisioningRequest>(
            r#"SELECT request_id, requester_user_id, cluster_owner_user_id, machine_id,
                      request_fingerprint, git_remote, clone_url, instance_name, branch,
                      project_id, status, result_path, error_message, created_at, updated_at
               FROM project_provisioning_requests
               WHERE request_id = ? AND requester_user_id = ?"#,
        )
        .bind(request_id)
        .bind(requester_user_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Internal daemon-ack lookup. Request IDs are unguessable idempotency
    /// keys and this method is never exposed as a user-facing query.
    pub async fn get_project_provisioning_by_request_id(
        &self,
        request_id: &str,
    ) -> Result<Option<ProjectProvisioningRequest>> {
        Ok(sqlx::query_as::<_, ProjectProvisioningRequest>(
            r#"SELECT request_id, requester_user_id, cluster_owner_user_id, machine_id,
                      request_fingerprint, git_remote, clone_url, instance_name, branch,
                      project_id, status, result_path, error_message, created_at, updated_at
               FROM project_provisioning_requests WHERE request_id = ?"#,
        )
        .bind(request_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn mark_project_provisioning_cloned(
        &self,
        request_id: &str,
        project_id: &str,
        result_path: &str,
    ) -> Result<bool> {
        Ok(sqlx::query(
            r#"UPDATE project_provisioning_requests
               SET status = 'cloned', result_path = ?, error_message = NULL,
                   updated_at = CURRENT_TIMESTAMP
               WHERE request_id = ? AND project_id = ? AND status IN ('pending', 'cloned')"#,
        )
        .bind(result_path)
        .bind(request_id)
        .bind(project_id)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }

    /// Finalize requester ownership and hosting placement in one transaction.
    /// Returns `None` after atomically cancelling when access was revoked.
    pub async fn finalize_project_provisioning(
        &self,
        request_id: &str,
        requester_user_id: &str,
    ) -> Result<Option<ProjectProvisioningRequest>> {
        let mut tx = self.pool.begin().await?;
        let request = sqlx::query_as::<_, ProjectProvisioningRequest>(
            r#"SELECT request_id, requester_user_id, cluster_owner_user_id, machine_id,
                      request_fingerprint, git_remote, clone_url, instance_name, branch,
                      project_id, status, result_path, error_message, created_at, updated_at
               FROM project_provisioning_requests
               WHERE request_id = ? AND requester_user_id = ?"#,
        )
        .bind(request_id)
        .bind(requester_user_id)
        .fetch_optional(&mut *tx)
        .await?
        .context("provisioning request not found")?;
        if request.status == "completed" {
            tx.commit().await?;
            return Ok(Some(request));
        }
        anyhow::ensure!(
            matches!(request.status.as_str(), "pending" | "cloned"),
            "provisioning request is {}",
            request.status
        );

        let authorized = sqlx::query_scalar::<_, i64>(
            r#"SELECT COUNT(*) FROM users requester, users owner
               WHERE requester.id = ? AND requester.account_status = 'active'
                 AND owner.id = ? AND owner.account_status = 'active'
                 AND (requester.id = owner.id OR EXISTS (
                     SELECT 1 FROM cluster_memberships cm
                     WHERE cm.cluster_owner_user_id = owner.id
                       AND cm.user_id = requester.id AND cm.status = 'active'
                 ))"#,
        )
        .bind(requester_user_id)
        .bind(&request.cluster_owner_user_id)
        .fetch_one(&mut *tx)
        .await?
            > 0;
        if !authorized {
            sqlx::query(
                r#"UPDATE project_provisioning_requests
                   SET status = 'cancelled', error_message = 'Cluster access was revoked',
                       updated_at = CURRENT_TIMESTAMP
                   WHERE request_id = ? AND status IN ('pending', 'cloned')"#,
            )
            .bind(request_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(None);
        }

        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO projects (id, owner_user_id, lifecycle_status) VALUES (?, ?, 'active')",
        )
        .bind(&request.project_id)
        .bind(requester_user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        if !inserted {
            let owner =
                sqlx::query_scalar::<_, String>("SELECT owner_user_id FROM projects WHERE id = ?")
                    .bind(&request.project_id)
                    .fetch_one(&mut *tx)
                    .await?;
            anyhow::ensure!(
                owner == requester_user_id,
                "generated project id belongs to another account"
            );
        }
        sqlx::query(
            r#"INSERT OR IGNORE INTO project_cluster_placements
               (project_id, cluster_owner_user_id, created_by_user_id, source)
               VALUES (?, ?, ?, 'shared_provisioning')"#,
        )
        .bind(&request.project_id)
        .bind(&request.cluster_owner_user_id)
        .bind(requester_user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"UPDATE project_provisioning_requests
               SET status = 'completed', error_message = NULL, updated_at = CURRENT_TIMESTAMP
               WHERE request_id = ? AND status IN ('pending', 'cloned')"#,
        )
        .bind(request_id)
        .execute(&mut *tx)
        .await?;
        Self::insert_audit_tx(
            &mut tx,
            requester_user_id,
            "project.provisioned",
            "project",
            &request.project_id,
            Some(serde_json::json!({
                "cluster_user_id": request.cluster_owner_user_id,
                "machine_id": request.machine_id,
                "request_id": request.request_id,
            })),
        )
        .await?;
        tx.commit().await?;
        self.get_project_provisioning(request_id, requester_user_id)
            .await
    }

    pub async fn fail_project_provisioning(
        &self,
        request_id: &str,
        requester_user_id: &str,
        error_message: &str,
    ) -> Result<bool> {
        Ok(sqlx::query(
            r#"UPDATE project_provisioning_requests
               SET status = 'failed', error_message = ?, updated_at = CURRENT_TIMESTAMP
               WHERE request_id = ? AND requester_user_id = ?
                 AND status IN ('pending', 'cloned')"#,
        )
        .bind(error_message)
        .bind(request_id)
        .bind(requester_user_id)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::User;

    async fn database(name: &str) -> Database {
        let dir = std::env::temp_dir().join(format!(
            "apas-shared-cluster-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::new(&dir.join("apas.db").to_string_lossy())
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn user(db: &Database, id: &str, email: &str) {
        db.create_user(&User {
            id: id.to_string(),
            email: email.to_string(),
            password_hash: "hash".to_string(),
            created_at: None,
            cluster_role: "user".to_string(),
            account_status: "active".to_string(),
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn invitation_acceptance_and_revocation_are_scoped_and_reversible() {
        let db = database("membership-lifecycle").await;
        user(&db, "owner", "owner@example.test").await;
        user(&db, "member", "member@example.test").await;
        user(&db, "other", "other@example.test").await;

        let invite = db
            .create_shared_cluster_invitation(
                "invite-1",
                "sha256:first-token",
                "owner",
                " MEMBER@example.test ",
                &(chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            )
            .await
            .unwrap();
        assert_eq!(invite.invitee_user_id, "member");
        assert_eq!(invite.invitee_email, "member@example.test");
        let stored_hash = sqlx::query_scalar::<_, String>(
            "SELECT token_hash FROM shared_cluster_invitations WHERE id = ?",
        )
        .bind(&invite.id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(stored_hash, "sha256:first-token");
        assert!(db
            .accept_shared_cluster_invitation("sha256:first-token", "other")
            .await
            .unwrap()
            .is_none());

        let membership = db
            .accept_shared_cluster_invitation("sha256:first-token", "member")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(membership.status, "active");
        assert!(db
            .has_active_cluster_access("owner", "owner")
            .await
            .unwrap());
        assert!(db
            .has_active_cluster_access("owner", "member")
            .await
            .unwrap());
        assert!(!db
            .has_active_cluster_access("owner", "other")
            .await
            .unwrap());
        assert!(db
            .accept_shared_cluster_invitation("sha256:first-token", "member")
            .await
            .unwrap()
            .is_none());

        let clusters = db.list_accessible_clusters("member").await.unwrap();
        assert_eq!(clusters.len(), 2);
        assert!(clusters
            .iter()
            .any(|cluster| cluster.owner_user_id == "owner" && cluster.access == "member"));

        assert!(db
            .revoke_cluster_membership("owner", "member")
            .await
            .unwrap());
        assert!(!db
            .has_active_cluster_access("owner", "member")
            .await
            .unwrap());
        assert_eq!(
            db.get_cluster_membership("owner", "member")
                .await
                .unwrap()
                .unwrap()
                .status,
            "revoked"
        );

        db.create_shared_cluster_invitation(
            "invite-2",
            "sha256:second-token",
            "owner",
            "member@example.test",
            &(chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
        )
        .await
        .unwrap();
        assert!(db
            .accept_shared_cluster_invitation("sha256:second-token", "member")
            .await
            .unwrap()
            .is_some());
        assert!(db
            .has_active_cluster_access("owner", "member")
            .await
            .unwrap());

        let events = db.list_cluster_audit_events("owner", 50, 0).await.unwrap();
        assert!(events
            .iter()
            .any(|event| event.action == "cluster.member_joined"));
        assert!(events
            .iter()
            .any(|event| event.action == "cluster.member_revoked"));
    }

    #[tokio::test]
    async fn unavailable_invitees_and_expired_tokens_fail_closed() {
        let db = database("membership-failures").await;
        user(&db, "owner", "owner@example.test").await;
        user(&db, "member", "member@example.test").await;

        assert!(db
            .create_shared_cluster_invitation(
                "missing",
                "sha256:missing",
                "owner",
                "missing@example.test",
                &(chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("already have"));
        db.create_shared_cluster_invitation(
            "expired",
            "sha256:expired",
            "owner",
            "member@example.test",
            &(chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339(),
        )
        .await
        .unwrap();
        assert!(db
            .accept_shared_cluster_invitation("sha256:expired", "member")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn clean_tree_migration_smoke_preserves_legacy_inventory_and_effective_policy() {
        let db = database("placement-migration").await;
        for (id, email) in [
            ("owner", "owner@example.test"),
            ("host", "host@example.test"),
            ("late", "late@example.test"),
        ] {
            user(&db, id, email).await;
        }
        sqlx::query("DELETE FROM schema_migrations WHERE name = ?")
            .bind(super::super::LEGACY_PROJECT_PLACEMENTS_MIGRATION)
            .execute(&db.pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM project_cluster_placements")
            .execute(&db.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO projects (id, owner_user_id, lifecycle_status) VALUES ('project-a', 'owner', 'active')",
        )
        .execute(&db.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sessions (id, user_id, project_id, status) VALUES ('host-session', 'host', 'project-a', 'active')",
        )
        .execute(&db.pool)
        .await
        .unwrap();
        db.set_deployment_default_policy(
            "owner",
            true,
            vec![
                "agent:claude:official:default".to_string(),
                "agent:codex:official:default".to_string(),
            ],
        )
        .await
        .unwrap();
        db.set_cluster_default_policy(
            "host",
            "host",
            Some(false),
            Some(vec!["agent:codex:official:default".to_string()]),
        )
        .await
        .unwrap();

        // This is the exact pre-upgrade inventory relation that the old
        // application inferred. Capture it before the placement migration so
        // the smoke test compares the two representations rather than merely
        // checking that the new table is non-empty.
        let mut legacy_clusters = sqlx::query_scalar::<_, String>(
            r#"SELECT owner_user_id FROM projects WHERE id = 'project-a'
               UNION
               SELECT user_id FROM sessions WHERE COALESCE(project_id, id) = 'project-a'"#,
        )
        .fetch_all(&db.pool)
        .await
        .unwrap();
        legacy_clusters.sort();
        assert_eq!(
            legacy_clusters,
            vec!["host".to_string(), "owner".to_string()]
        );

        db.run_migrations().await.unwrap();
        let placements = db
            .list_project_cluster_placements("project-a")
            .await
            .unwrap();
        let mut migrated_clusters = placements
            .iter()
            .map(|placement| placement.cluster_owner_user_id.clone())
            .collect::<Vec<_>>();
        migrated_clusters.sort();
        assert_eq!(migrated_clusters, legacy_clusters);
        assert_eq!(
            db.list_cluster_projects("owner", None, 20, 0)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            db.list_cluster_projects("host", None, 20, 0)
                .await
                .unwrap()
                .len(),
            1
        );
        let migrated_policy = db.get_effective_project_policy("project-a").await.unwrap();
        assert!(!migrated_policy.team_available);
        assert_eq!(
            migrated_policy.allowed_launch_profiles,
            vec!["agent:codex:official:default".to_string()]
        );

        sqlx::query(
            "INSERT INTO sessions (id, user_id, project_id, status) VALUES ('late-session', 'late', 'project-a', 'active')",
        )
        .execute(&db.pool)
        .await
        .unwrap();
        db.run_migrations().await.unwrap();
        assert!(!db
            .project_is_placed_in_cluster("project-a", "late")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn placement_drives_inventory_host_access_and_policy_intersection() {
        let db = database("placement-authority").await;
        for (id, email) in [
            ("owner", "owner@example.test"),
            ("host", "host@example.test"),
            ("outsider", "outsider@example.test"),
        ] {
            user(&db, id, email).await;
        }
        db.authorize_project_registration("project-a", "owner")
            .await
            .unwrap();
        assert!(db
            .add_project_cluster_placement("project-a", "host", "owner", "test")
            .await
            .unwrap());
        assert!(db
            .project_in_user_cluster("project-a", "host")
            .await
            .unwrap());
        assert!(!db
            .project_in_user_cluster("project-a", "outsider")
            .await
            .unwrap());
        assert_eq!(
            db.list_cluster_projects("host", None, 20, 0)
                .await
                .unwrap()
                .len(),
            1
        );

        db.set_deployment_default_policy(
            "owner",
            true,
            vec![
                "agent:claude:official:default".to_string(),
                "agent:codex:official:default".to_string(),
            ],
        )
        .await
        .unwrap();
        db.set_cluster_default_policy(
            "host",
            "host",
            Some(false),
            Some(vec!["agent:codex:official:default".to_string()]),
        )
        .await
        .unwrap();
        let policy = db.get_effective_project_policy("project-a").await.unwrap();
        assert!(!policy.team_available);
        assert_eq!(
            policy.allowed_launch_profiles,
            vec!["agent:codex:official:default".to_string()]
        );
    }

    #[tokio::test]
    async fn provisioning_is_idempotent_and_finalization_rechecks_membership() {
        let db = database("provisioning-state").await;
        user(&db, "owner", "owner@example.test").await;
        user(&db, "member", "member@example.test").await;
        user(&db, "other", "other@example.test").await;
        db.create_shared_cluster_invitation(
            "invite",
            "sha256:invite",
            "owner",
            "member@example.test",
            &(chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
        )
        .await
        .unwrap();
        db.accept_shared_cluster_invitation("sha256:invite", "member")
            .await
            .unwrap()
            .unwrap();

        let first = db
            .claim_project_provisioning(
                "request-1",
                "member",
                "owner",
                "machine-1",
                "fingerprint-1",
                "github.com/example/repo",
                "https://github.com/example/repo.git",
                "repo",
                "member-work",
                "project-1",
            )
            .await
            .unwrap();
        assert_eq!(first.status, "pending");
        let retry = db
            .claim_project_provisioning(
                "request-1",
                "member",
                "owner",
                "machine-1",
                "fingerprint-1",
                "github.com/example/repo",
                "https://github.com/example/repo.git",
                "repo",
                "member-work",
                "project-1",
            )
            .await
            .unwrap();
        assert_eq!(retry.project_id, first.project_id);
        assert!(db
            .claim_project_provisioning(
                "request-1",
                "member",
                "owner",
                "machine-1",
                "different",
                "github.com/example/other",
                "https://github.com/example/other.git",
                "other",
                "member-work",
                "project-2",
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("different inputs"));
        assert!(db
            .mark_project_provisioning_cloned("request-1", "project-1", "/managed/repo")
            .await
            .unwrap());
        db.revoke_cluster_membership("owner", "member")
            .await
            .unwrap();
        assert!(db
            .finalize_project_provisioning("request-1", "member")
            .await
            .unwrap()
            .is_none());
        assert!(db.get_project("project-1").await.unwrap().is_none());
        assert_eq!(
            db.get_project_provisioning("request-1", "member")
                .await
                .unwrap()
                .unwrap()
                .status,
            "cancelled"
        );

        db.create_shared_cluster_invitation(
            "invite-2",
            "sha256:invite-2",
            "owner",
            "member@example.test",
            &(chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
        )
        .await
        .unwrap();
        db.accept_shared_cluster_invitation("sha256:invite-2", "member")
            .await
            .unwrap()
            .unwrap();
        db.claim_project_provisioning(
            "request-2",
            "member",
            "owner",
            "machine-1",
            "fingerprint-2",
            "github.com/example/repo",
            "https://github.com/example/repo.git",
            "repo",
            "member-work",
            "project-2",
        )
        .await
        .unwrap();
        db.mark_project_provisioning_cloned("request-2", "project-2", "/managed/repo-2")
            .await
            .unwrap();
        let completed = db
            .finalize_project_provisioning("request-2", "member")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(completed.status, "completed");
        assert_eq!(
            db.get_project("project-2")
                .await
                .unwrap()
                .unwrap()
                .owner_user_id,
            "member"
        );
        assert!(db
            .project_is_placed_in_cluster("project-2", "owner")
            .await
            .unwrap());
        assert_eq!(
            db.finalize_project_provisioning("request-2", "member")
                .await
                .unwrap()
                .unwrap()
                .status,
            "completed"
        );
        assert!(db
            .claim_project_provisioning(
                "unrelated",
                "other",
                "owner",
                "machine-1",
                "fingerprint",
                "github.com/example/repo",
                "https://github.com/example/repo.git",
                "repo",
                "work",
                "project-other",
            )
            .await
            .is_err());
    }

    #[tokio::test]
    async fn revocation_preserves_project_content_but_removes_hosted_runtime_authority() {
        let db = database("runtime-revocation").await;
        user(&db, "owner", "owner@example.test").await;
        user(&db, "member", "member@example.test").await;
        db.create_shared_cluster_invitation(
            "invite-runtime",
            "invite-runtime-hash",
            "owner",
            "member@example.test",
            &(chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
        )
        .await
        .unwrap();
        db.accept_shared_cluster_invitation("invite-runtime-hash", "member")
            .await
            .unwrap()
            .unwrap();
        db.authorize_project_registration("runtime-project", "member")
            .await
            .unwrap();
        db.add_project_cluster_placement("runtime-project", "owner", "owner", "test")
            .await
            .unwrap();
        db.create_session(&crate::db::Session {
            id: "hosted-session".to_string(),
            user_id: "owner".to_string(),
            cli_client_id: None,
            working_dir: Some("/managed/project".to_string()),
            hostname: Some("shared-host".to_string()),
            status: "active".to_string(),
            created_at: None,
            updated_at: None,
            is_paused: false,
            project_id: Some("runtime-project".to_string()),
            git_remote: None,
            git_remote_url: None,
        })
        .await
        .unwrap();
        assert!(db
            .check_session_runtime_access("hosted-session", "member")
            .await
            .unwrap());
        db.revoke_cluster_membership("owner", "member")
            .await
            .unwrap();
        assert!(db
            .check_session_access("hosted-session", "member")
            .await
            .unwrap());
        assert!(!db
            .check_session_runtime_access("hosted-session", "member")
            .await
            .unwrap());
    }
}
