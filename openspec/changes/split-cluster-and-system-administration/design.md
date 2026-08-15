## Context

See proposal.md — Why. The relevant current state:

- **Authority is one flag.** `users.cluster_role ∈ {admin, user}` gates every `/admin/*` route through `authz::require_cluster_admin`, and `db::user_is_cluster_admin` additionally widens project access in `authorize_project_registration`, `check_session_access`, `get_sessions_for_user`, and `get_shared_sessions_for_user` (shipped by `grant-cluster-admin-project-access`, implemented but not yet archived).
- **The virtual cluster is already derived, not stored.** Machines live only in memory: `session::get_machines_for_user` filters `machine_infos` by `daemon_users[machine] == user`. Projects have no machine column; the durable evidence that a project runs in an account's cluster is a `sessions` row with `COALESCE(project_id, id) = project` authored by that account. That is exactly what the predecessor change keyed on.
- **Policy is two levels.** `get_effective_project_policy` CROSS JOINs the single `cluster_settings` row (id = 1) with an optional `project_policy_overrides` row; the override replaces the default field-by-field and may widen it.
- **Audit has an FK.** `admin_audit_events.actor_user_id REFERENCES users(id)`, and the pool sets `PRAGMA foreign_keys=ON`, so no non-account actor can be recorded today. The table already carries `project_id`.
- **nginx routes `/admin/` to the server and `= /admin` to Next.js.** A page route at `/admin/login` would be proxied to `apas-server` and 404. Any new page must stay at exactly `/admin`.
- **The WS layer has one role gate**, `ws_web::can_manage_project_settings`, and it is project-role based (owner), not `cluster_role` based.

## Goals / Non-Goals

**Goals:**

- One derivation of "project P is hosted in account A's cluster", used by every access check, listing, policy resolution, and control-plane authorization.
- Two disjoint authentication domains: account tokens (everything except `/admin/*`) and the system-administrator token (`/admin/*` only). Neither is accepted by the other.
- A migration in which no account keeps deployment-wide reach and no deployment loses the ability to administer itself.
- Policy that can only narrow as it descends, so making the cluster level self-service cannot widen what a deployment allows.

**Non-Goals:**

- A `virtual_clusters` table, named clusters, multiple clusters per account, or cluster membership rows (the cluster stays derived; an explicit table would drift from where sessions actually run).
- More than one system administrator, or any UI path that creates one.
- Changing how machines are discovered, registered, or listed — `get_machines_for_user` is already cluster-scoped and is left alone.
- Reworking share codes, project deletion, or the mobile surfaces.

## Decisions

1. **Cluster hosting is a single DB predicate, and it excludes plain membership.** Add `db::project_in_user_cluster(project_id, user_id) -> bool` (owner row, or a `sessions` row authored by the account) and `db::list_cluster_projects(user_id)`. *Content* access stays owner ∨ member ∨ host; *administration* (lifecycle, stop-runtime, membership, ownership, policy) requires host. Alternative considered: treating membership as cluster inclusion — rejected because it would hand any project user the power to suspend a project they merely joined.

2. **The access grant drops its role test rather than gaining a second branch.** The four DB functions the predecessor change touched keep their shape; `user_is_cluster_admin(...)` is deleted and the surrounding condition becomes the cluster predicate for every active account. This is what keeps the `mako-soumojit` case working after existing admins are demoted — the case that motivated the predecessor is the general rule now, not an admin exception.

3. **Policy resolves as deployment ∧ (⋂ hosting clusters) ∧ project** for the launch-profile allowlist, which is set intersection. `team_available` is *not* folded with AND: it takes the value stated by the lowest level that states one. The seeded deployment value is `false`, and team mode has always been switched on per project against that default, so ANDing it would have stripped team mode from every project that runs it today — the opposite of the migration guarantee. Only the profile allowlist is a genuine ceiling, and only it is enforced as one. New table `cluster_default_policies(user_id PK, team_available INTEGER NULL, allowed_launch_profiles TEXT NULL, version, updated_at)` where NULL means "inherit". `cluster_settings` (id = 1) stays as the deployment level, now writable only by the system administrator.
   - Intersecting over *all* hosting clusters — not just the owner's — is what makes "my cluster default restricts what runs on my machines" true even for projects other accounts own, and set intersection is order-independent, so a project hosted in several clusters still has one deterministic answer. Alternative considered: resolving against the owner's cluster only — simpler and cheaper, but a cluster default that does not apply to the foreign-owned projects running on your hardware is not the feature that was asked for.
   - Monotone narrowing of the allowlist replaces today's field-replacement semantics. Without it, moving the cluster level to self-service would let any account widen its own projects past the deployment policy, silently deleting the governance property the single deployment-wide default provides today.
   - Every `cluster_default_policies` row starts absent (= inherit), so effective policy is unchanged for every existing project on upgrade.

4. **The system administrator is a credential, not an account.** New table `system_admin_credential(id INTEGER PRIMARY KEY CHECK (id = 1), username, password_hash, credential_version, bootstrap_pending, updated_at)`, seeded from `config.system_admin` only when the row is absent. Tokens reuse the existing JWT machinery with `sub = "system-admin"`, `token_kind = "system_admin"`, and the credential version; `authz::require_system_admin` verifies the kind and that the version still matches, so a password change invalidates every outstanding token. `auth::require_active_claims` explicitly rejects `token_kind == "system_admin"` instead of relying on the `users` lookup missing.
   - Alternative considered: a reserved `users` row with a `system` role — rejected, because it would be an account with a password hash that ordinary login and project code would have to be taught to exclude everywhere, which is the failure mode this change exists to remove.
   - Login is throttled per source with a bounded lockout and a constant failure message, since this is a single well-known identity and the surface is unauthenticated by definition.

5. **The admin login lives inline at `/admin`.** nginx proxies `/admin/` to `apas-server`, so a Next.js page at `/admin/login` is unreachable; the page renders its own login form when it holds no system-administration token. The token is kept in `sessionStorage` under a distinct key, never in the zustand store and never in `localStorage`, so it dies with the tab and cannot be picked up by ordinary app code. `/admin/auth/login` and `/admin/auth/password` are new server endpoints under the already-proxied `/admin/` prefix, so nginx needs no change.

6. **`admin_audit_events` is rebuilt once to carry a non-account actor and a cluster.** New columns `actor_kind ∈ {user, system_admin}` and `cluster_user_id`; the FK on the actor is dropped by the standard SQLite create-copy-drop-rename, guarded by a `schema_migrations` row. Historical rows migrate as `actor_kind = 'user'` with `cluster_user_id` backfilled from the project's owner where the project is known, NULL otherwise; NULL-cluster rows are visible only to the system administrator. Alternative considered: a second table for system-administrator events — rejected because the system administrator's audit view must interleave both, and a UNION over two schemas is worse than one rebuild.

7. **API surfaces split by authentication domain, not by role check.** New `routes/cluster.rs` mounted at `/cluster/*` takes an account token and scopes every handler through the cluster predicate. `routes/admin.rs` keeps its paths (so the page URL is unchanged) but every handler swaps `require_cluster_admin` for `require_system_admin`, and gains `/admin/clusters` for the per-account cluster inventory. `require_cluster_admin` is deleted rather than left unused. `ws_web::can_manage_project_settings` additionally accepts the hosting cluster operator.

8. **`cluster_role` stays on the wire, frozen.** The column, the `MeResponse.cluster_role` field, and the `shared` message fields remain and always read `"user"`, so an older web build or mobile client keeps parsing identity responses. Every authorization read of it is removed, the promotion endpoint is removed, and the web stops branching on it. Dropping the field outright would be a needless wire break for a value nothing may act on any more.

## Risks / Trade-offs

- [Any account that ever hosted a session of a project can now administer it — suspend it, change its members, transfer it away] → This is the same trust boundary as the project's files, which are on that account's disk; every such action is audited with its cluster, and the system administrator can reverse ownership and membership changes. It is also strictly the rule the predecessor change already shipped for admins, now stated for everyone.
- [Today's cluster administrators lose deployment-wide reach the moment the server is deployed] → The system-administrator credential must be configured *before* the server rollout, not after; the migration task ordering makes seeding a prerequisite, and the rollback path (previous binary) restores the old role behaviour because the `cluster_role` values themselves are left intact by the migration.
- [The bootstrap credential sits in `apas-server.toml`] → It is a one-time value: the surface warns while `bootstrap_pending` is set, rotation is a first-class endpoint, and rotation bumps the credential version so anything issued against the bootstrap secret stops working.
- [Intersecting policy over all hosting clusters adds a `sessions` scan to a hot path] → `get_effective_project_policy` runs per launch, not per message; the added subquery is a `DISTINCT user_id` over one project's sessions with the existing project index, and the common case is one host. If it ever matters, the result is cacheable per project because it only changes when a session is created or a default is edited.
- [An older web build against the new server sees `/admin/*` start returning 401] → Intended: that build's admin page is exactly what is being withdrawn. Server-then-web ordering keeps the window short, and the machines page in the old build is unaffected because it reads machines over the WebSocket, not `/admin`.
- [Two unarchived changes touch `project-access-control`] → This change's delta removes the requirement `grant-cluster-admin-project-access` adds, so that change must be archived first. Encoded as the first task.

## Migration Plan

1. Configure `[system_admin]` on the server host and confirm the file mode before deploying; without it the server starts with no system administrator and `/admin` cannot be entered at all.
2. Deploy the server. On boot it: seeds the credential row if absent; rebuilds `admin_audit_events`; creates `cluster_default_policies` (empty = inherit); leaves `users.cluster_role` values untouched while removing every read of them; and stops honouring `bootstrap_admin_email`.
3. Verify: `/admin/auth/login` accepts the configured credential and rejects an account token; a plain account can read `/cluster/projects` and sees only its own cluster; an account's effective project policy is byte-identical to what it was before.
4. Rotate the bootstrap credential from the admin surface.
5. Deploy the web build (machines page becomes the cluster surface, `/admin` becomes the self-contained system surface, sidebar link gone).
6. Rollback is binary-level: the previous server honours `cluster_role` again, and the rebuilt audit table and the empty `cluster_default_policies` are both readable by it — the old code selects named columns and never sees the new ones. The only non-restorable state is a rotated credential, which the old build does not use.
