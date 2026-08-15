## Why

`/admin` currently fuses two unrelated jobs behind one `cluster_role = "admin"` flag: running *your own* corner of APAS (the machines you registered and the projects that run on them) and running *the whole deployment* (every account, every project, the default policy). The first is ordinary self-service that every user needs and today cannot reach at all — a user who owns the hardware cannot suspend a project on it, stop its runtime, or see what happened to it without being granted deployment-wide authority over everyone else's projects too. The second is a superuser surface that should not be one promotion away from any account, nor reachable with an ordinary web session.

## What Changes

- **Every account operates exactly one virtual cluster**, defined as it already is implicitly: the machines whose daemon registered under that account, plus the projects hosted in it — projects the account owns, projects it belongs to, and projects with at least one session created under it. Others may own a project in my cluster and I need not be a member of it.
- **The `/machines` page becomes the virtual-cluster management surface**, visible to every user. Alongside machines and usage it gains: the cluster's project inventory, project suspend/reactivate, stop runtime, member add/remove, owner transfer, a per-cluster default launch policy, and the cluster's own audit log.
- **The cluster-scoped project grant generalizes from cluster admins to every account.** The access branch shipped by `grant-cluster-admin-project-access` (an active admin may open projects present in their own cluster) becomes the rule for all active accounts over their own cluster. Without this, demoting today's admins would re-break the case that change fixed.
- **Launch policy narrows monotonically down three levels**: deployment default (system administrator) → cluster default (cluster operator) → project override. A lower level may only restrict what the level above allows, never widen it. This keeps the governance property today's single deployment-wide default provides while making the cluster level self-service.
- **`/admin` becomes the APAS system-administration surface for the whole deployment**, removed from every normal user's UI: no sidebar entry, no role-conditional rendering, reachable only by navigating to the URL.
- **`/admin` requires a separate login as a single system administrator.** One credential, stored outside the `users` table, seeded from server configuration, never promotable through any UI, issuing a short-lived token scoped only to `/admin/*`. An ordinary user token is rejected there, and a system-administrator token grants nothing else.
- The system administrator manages every account, every virtual cluster, and every project in the deployment: account invitation/activation/suspension, the deployment default policy, cross-cluster project inventory and lifecycle, and the full audit log.
- **BREAKING**: `cluster_role = "admin"` stops conferring any authority. On upgrade every account migrates to a plain user administering only its own virtual cluster, and deployment-wide reach exists solely behind the new system-administrator credential. Accounts that relied on the admin role for cross-cluster visibility lose it.
- **BREAKING**: `config.bootstrap_admin_email` no longer promotes an account; system-administrator bootstrap moves to the new credential configuration.

## Capabilities

### New Capabilities

- `system-administration`: The single deployment-wide system administrator — its separate credential and login, its token scope, its exclusive `/admin` surface, and its authority over every account, cluster, and project.

### Modified Capabilities

- `cluster-user-administration`: Cluster identity stops carrying an `admin`/`user` authority split; account lifecycle and the deployment project inventory move to `system-administration`; every account gains self-service administration of its own virtual cluster (inventory, project lifecycle, membership, owner transfer, audit) with cluster-scoped visibility.
- `project-access-control`: The requirement granting cluster administrators project access inside their own cluster is replaced by one granting every active account access inside its own cluster; control-plane operations on membership and ownership become operations of the project's cluster operator and of the system administrator rather than of a cluster-role admin.
- `project-policy-governance`: Governed policy is no longer cluster-admin-only. The system administrator sets the deployment default, the cluster operator sets a cluster default and per-project overrides within it, and the effective policy is the monotone narrowing of the three levels.

## Impact

- **Server**: new `routes/system_admin.rs` (credential login, token scope, deployment inventory) and `routes/cluster.rs` (per-cluster self-service API); `routes/authz.rs` gains `require_system_admin` and loses `require_cluster_admin`; `routes/admin.rs` re-gated and re-scoped; `db/mod.rs` gains the system-administrator credential table, per-cluster default policy rows, an audit `cluster_user_id`/`actor_kind` dimension, cluster-membership queries for projects, and the migration demoting every `cluster_role`.
- **Access core**: `authorize_project_registration`, `check_session_access`, `get_sessions_for_user`, `get_shared_sessions_for_user` drop their `user_is_cluster_admin` condition in favour of a cluster-membership condition that holds for every account.
- **Web**: `/machines` becomes the cluster surface; `/admin` becomes a self-contained page with its own login form and its own token storage key; `Sidebar` loses the admin link and the `clusterRole` branch; `store.ts` stops carrying `clusterRole` as an authority signal.
- **Config/ops**: `apas-server.toml` gains the system-administrator credential block; the deployment runbook gains a first-login password-change step. Server-first rollout, then web.
- **Docs**: `CLAUDE.md` and `README.md` sections describing cluster administration and the `/admin` page.
