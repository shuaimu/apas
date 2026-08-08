# Cluster administration rollout

The cluster control plane is available to persisted `admin` accounts at
`http://apas.mpaxos.com/admin`. Cluster administration does not grant access
to project messages, files, diffs, or terminals; an administrator needs an
explicit project owner/user relationship for data-plane access.

## Before deployment

Stop the server or use SQLite's online backup command so the database, WAL,
and shared-memory state are captured consistently:

```bash
ssh root@apas.mpaxos.com
systemctl stop apas-server
sqlite3 /opt/apas/data/apas.db ".backup '/opt/apas/backups/apas-before-cluster-admin.db'"
systemctl start apas-server
```

Confirm the production server configuration contains the existing account
that should receive the first durable cluster-admin role:

```toml
[auth]
bootstrap_admin_email = "administrator@example.com"
```

The setting is consulted only while no active administrator exists. Existing
accounts are migrated to active cluster users. Project-level `admin` shares
become ordinary project users and never create cluster administrators.

## Staged deployment

Build and deploy the server first. The server retains read-only compatibility
with older peers but rejects launch-capable requests until both ends advertise
`project_policy_v1`.

```bash
cargo build --release
scp target/release/apas-server root@apas.mpaxos.com:/tmp/apas-server
ssh root@apas.mpaxos.com "mv /tmp/apas-server /opt/apas/apas-server && chmod +x /opt/apas/apas-server && systemctl restart apas-server"
```

Upgrade project-host CLIs and daemons next. A compatible CLI sends its local
`.apas` team/tab settings once; the first accepted snapshot is preserved and
later conflicting snapshots appear in the audit data. Finally deploy the web:

```bash
rsync -av --exclude 'node_modules' --exclude '.next' packages/web/ root@apas.mpaxos.com:/opt/apas/web/
month_start="$(date +%Y-%m-01) 00:00:00"
web_version="$(date +%y.%m).$(git rev-list --count --since="$month_start" HEAD)"
ssh root@apas.mpaxos.com "cd /opt/apas/web && npm install && NEXT_PUBLIC_WEB_UI_VERSION=${web_version} npm run build && systemctl restart apas-web"
```

## Verification

Sign in as the bootstrap account and open `/admin`. Verify the Users,
Projects, and Audit tabs. The following read-only queries provide an
independent database check:

```sql
SELECT id, email, cluster_role, account_status FROM users ORDER BY email;
SELECT COUNT(*) AS active_admins FROM users
 WHERE cluster_role = 'admin' AND account_status = 'active';
SELECT id, owner_user_id, lifecycle_status FROM projects ORDER BY id;
SELECT project_id, user_id FROM project_members ORDER BY project_id, user_id;
SELECT id, team_available, allowed_launch_profiles, version FROM cluster_settings;
SELECT project_id, team_available, allowed_launch_profiles, version,
       legacy_imported, legacy_conflict
  FROM project_policy_overrides ORDER BY project_id;
SELECT actor_user_id, action, target_type, target_id, created_at
  FROM admin_audit_events ORDER BY id DESC LIMIT 50;
```

Exercise one allowed and one denied pane launch, an owner/user attachment on
two instances of the same project, a non-member administrator attachment
denial, and project suspension/reactivation. Suspension should stop connected
runtimes, detach web viewers, preserve project data, and remain enforced when
an offline host reconnects.

## Project ownership, departure, and permanent deletion

Project owners manage access from the project action in the sidebar. They can
transfer ownership only to an active user who already belongs to the project;
the former owner remains as an ordinary user. Ordinary users can leave a
project themselves. Owners must transfer ownership or permanently delete the
project instead of leaving it ownerless.

Permanent deletion requires typing the canonical project ID. The server first
marks the project `deleting`, making registration, attachment, history reads,
invitations, and delayed writes fail closed. Cleanup then stops APAS runtime,
removes the project's session directories and all project-linked SQLite rows,
and deletes the project record last. Interrupted cleanup is resumed before the
server opens its router after restart. Cleanup worker logs contain only
unlabelled attempt/failure counts; they do not include project IDs or project
content.

The deletion boundary covers APAS-managed SQLite data, file-backed session
history, and server runtime caches. It intentionally does not delete a source
checkout, its local `.apas` file, or a daemon's local project registry on a
developer machine. It also cannot erase infrastructure-managed backups,
reverse-proxy access logs, or service journals; apply the cluster's separate
retention process to those systems when required. A later explicit local start
after completed deletion registers a fresh empty APAS project.

## Rollback

Before the later legacy cleanup, rollback remains additive: stop both
services, restore the previous server/web/CLI binaries, restore the SQLite
backup, and restart. Restoring the database is necessary because ownership,
membership, lifecycle, and policy writes made through the new control plane
are not represented completely in the legacy session-share schema.

Do not drop `session_shares`, legacy invitation fields, `.apas` policy fields,
or compatibility message branches during this rollout. Remove them only in a
separate release after production verification and after exporting any audit
or policy data needed for rollback.
