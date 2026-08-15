## 1. Prerequisites

- [x] 1.1 Archive the completed `grant-cluster-admin-project-access` change so `openspec/specs/project-access-control/spec.md` contains "Cluster administrators access projects within their own cluster", which this change's delta removes

## 2. Cluster derivation in the DB layer

- [x] 2.1 Add `project_in_user_cluster(project_id, user_id) -> Result<bool>` in `crates/server/src/db/mod.rs`: true when the account owns the project or a `sessions` row with `COALESCE(project_id, id) = project_id` was created under it; false for suspended accounts
- [x] 2.2 Add `list_cluster_projects(user_id) -> Result<Vec<AdminProjectSummary>>` returning the same summary shape as the deployment inventory, restricted to the account's hosted projects, with owner email, member count, active-session count, last activity, and lifecycle status
- [x] 2.3 Add `cluster_summaries()` returning per-account cluster rows (account, hosted project count, active session count, last activity) for the system-administration inventory
- [x] 2.4 Delete `user_is_cluster_admin` and replace its use in `authorize_project_registration`, `check_session_access`, and `get_sessions_for_user` with the cluster predicate applied to every active account; restore `get_shared_sessions_for_user` to its unconditional member query
- [x] 2.5 Extend the db unit tests: hosting account passes without membership, member-only account is not an operator, foreign account is denied, suspended account is denied, listings stay inside the account's own cluster

## 3. Policy narrowing

- [x] 3.1 Create `cluster_default_policies(user_id PK, team_available INTEGER NULL, allowed_launch_profiles TEXT NULL, version, updated_at)`; absent row and NULL fields both mean inherit
- [x] 3.2 Rewrite `get_effective_project_policy` to resolve deployment ∧ (intersection over the project's hosting clusters) ∧ project override, with boolean AND for `team_available` and set intersection for `allowed_launch_profiles`
- [x] 3.3 Reject (or clamp, consistently and with an explicit error) a cluster default or project override that would widen the level above it, in the DB writer rather than per route
- [x] 3.4 Add `get_cluster_default_policy(user_id)` / `set_cluster_default_policy(user_id, ...)` with version bump and audit write
- [x] 3.5 Tests: existing projects keep byte-identical effective policy with no cluster rows present; a narrower cluster default takes effect; a widening cluster default or project override does not; a project hosted in two clusters gets the intersection

## 4. System-administrator credential

- [x] 4.1 Add the `[system_admin]` block to `crates/server/src/config.rs` (username, bootstrap password, token expiry minutes) and to `apas-server.toml`
- [x] 4.2 Create `system_admin_credential(id CHECK (id = 1), username, password_hash, credential_version, bootstrap_pending, updated_at)` and seed it from config only when the row is absent
- [x] 4.3 Add `routes/system_admin.rs` with `POST /admin/auth/login`, `GET /admin/auth/me`, `POST /admin/auth/password`; issue JWTs with `sub = "system-admin"`, `token_kind = "system_admin"`, and the credential version; password change bumps the version
- [x] 4.4 Add per-source failed-login throttling with a bounded lockout and a constant failure message
- [x] 4.5 Add `authz::require_system_admin` (kind + version check) and make `auth::require_active_claims` explicitly reject `token_kind == "system_admin"`
- [x] 4.6 Remove `bootstrap_cluster_admin` and its call in `crates/server/src/main.rs`, and remove `db::set_cluster_role` and the last-administrator guard
- [x] 4.7 Tests: account token rejected on `/admin/*`, system-admin token rejected on account routes and on the web WebSocket, rotation invalidates prior tokens, throttle engages, second credential row refused

## 5. Audit rebuild

- [x] 5.1 Rebuild `admin_audit_events` with `actor_kind` and `cluster_user_id` and no actor FK, via create-copy-drop-rename guarded by a `schema_migrations` row; recreate the existing indexes plus one on `cluster_user_id`
- [x] 5.2 Backfill `actor_kind = 'user'` and `cluster_user_id` from the project's owner where the project is known; leave it NULL otherwise
- [x] 5.3 Stamp `cluster_user_id` and `actor_kind` on every audit write, including system-administrator writes
- [x] 5.4 Add `list_cluster_audit(user_id, limit, offset)` (own cluster only) and keep the deployment-wide listing for the system administrator
- [x] 5.5 Tests: an operator sees only their cluster's records; NULL-cluster historical records are system-administrator only

## 6. Cluster API for every account

- [x] 6.1 Add `routes/cluster.rs` mounted at `/cluster/*`, authenticated with `require_active_user`, with `GET /cluster/overview`, `GET /cluster/projects`, `GET /cluster/projects/:id`
- [x] 6.2 Add the mutating routes: `PATCH /cluster/projects/:id/lifecycle`, `POST /cluster/projects/:id/stop-runtime`, `PATCH /cluster/projects/:id/owner`, `POST` and `DELETE /cluster/projects/:id/members/...`, `PATCH /cluster/projects/:id/policy`
- [x] 6.3 Add `GET`/`PATCH /cluster/policy/default` and `GET /cluster/audit`
- [x] 6.4 Gate every handler on `project_in_user_cluster` (project routes) or the caller's own account (cluster routes); return the same denial shape as the existing routes
- [x] 6.5 Reuse the existing lifecycle/stop-runtime/membership/transfer implementations from `routes/admin.rs` rather than duplicating them, so both surfaces cannot drift
- [x] 6.6 Extend `ws_web::can_manage_project_settings` to accept the hosting cluster operator in addition to the project owner
- [x] 6.7 Tests: each mutation succeeds for the hosting account and is denied for a member-only account, a foreign account, and a suspended account

## 7. System-administration API

- [x] 7.1 Swap `require_cluster_admin` for `require_system_admin` in every `routes/admin.rs` handler and delete `require_cluster_admin`
- [x] 7.2 Add `GET /admin/clusters` (every virtual cluster with its summary) and give `GET /admin/projects` a cluster column
- [x] 7.3 Keep `/admin/users*` as the only account-lifecycle path and drop its cluster-role field from request and response shapes
- [x] 7.4 Point `/admin/policy/default` at the deployment level and document that it is the outer bound for every cluster
- [x] 7.5 Tests: full deployment inventory is returned, cross-cluster lifecycle works, and every route rejects an account token

## 8. Web: cluster surface on the machines page

- [x] 8.1 Extend `packages/web/src/app/machines/page.tsx` with a cluster project list (owner, members, lifecycle, connection, effective policy) fed by `/cluster/projects`
- [x] 8.2 Add the per-project controls: suspend/reactivate, stop runtime, member add/remove, owner transfer, policy override
- [x] 8.3 Add the cluster default policy editor and the cluster audit table, reusing the `PolicyEditor` component extracted from the admin page
- [x] 8.4 Retitle the page as the account's cluster and keep the `/machines` route so existing links and bookmarks work
- [x] 8.5 Update `packages/web/src/app/machines/page.test.tsx` to cover the new sections, including the denial path for a project outside the cluster

## 9. Web: system-administration page

- [x] 9.1 Rewrite `packages/web/src/app/admin/page.tsx` as a self-contained page with an inline login form (no `/admin/login` route — nginx proxies `/admin/` to the server), its own fetch wrapper, and its token in `sessionStorage`
- [x] 9.2 Add the Clusters tab and keep Overview, Users, Projects, and Audit; show the credential-rotation warning while the bootstrap credential is unrotated, with a change-password form
- [x] 9.3 Remove the admin link and the `clusterRole` branch from `packages/web/src/components/Sidebar.tsx`
- [x] 9.4 Remove `clusterRole` from `packages/web/src/lib/store.ts` and its `localStorage` key, leaving the server field parsed and ignored
- [x] 9.5 Update `packages/web/src/app/admin/page.test.tsx` and `Sidebar.test.tsx`: unauthenticated visit shows the login form, an ordinary session does not authorize the page, and no navigation entry exists

## 10. Documentation and verification

- [x] 10.1 Update `CLAUDE.md` (cluster vs system administration, the `[system_admin]` block, the `/admin` inline-login constraint) and `README.md`
- [x] 10.2 Add the deployment-runbook step: configure and then rotate the system-administrator credential before the web deploy
- [x] 10.3 `cargo test` for the workspace and `cargo clippy` clean
- [x] 10.4 `npm run lint` and `npm test` clean in `packages/web`
- [x] 10.5 Manual verification against a scratch database: an ordinary account administers a foreign-owned project hosted in its cluster, cannot touch one outside it, and `/admin` refuses an ordinary account token while accepting the seeded credential
