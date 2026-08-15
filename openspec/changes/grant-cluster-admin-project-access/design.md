## Context

See proposal.md - Why. Enforcement today funnels through a few DB/route choke points:

- `db::authorize_project_registration(project_id, user_id)` (crates/server/src/db/mod.rs:2392) is the shared gate for CLI `SessionStart` (routes/ws_cli.rs:482) and mobile bootstrap (routes/mobile.rs:278). It creates-or-ignores the project row and then requires the caller to be owner or member.
- Web bootstrap and session lists call `db::get_sessions_for_user` (owner-scoped, db/mod.rs:3498) and `db::get_shared_sessions_for_user` (member-scoped, db/mod.rs:3729).
- Web machine operations gate on `sessions::get_machines_for_user` (in-memory, session/mod.rs:1511) plus `get_shared_project_access_refs` in routes/ws_web.rs (e.g. the `StartMachineProjectCli` allowed check at ws_web.rs:3293).
- `authz::require_cluster_admin` already exists and the user's cluster role is already loaded (`User::role() -> ClusterRole`), so no new auth primitives are needed.

## Goals / Non-Goals

**Goals:**

- Active cluster admins pass project content authorization everywhere without owner/member rows.
- Ordinary users keep exactly today's gating.
- Suspended admin accounts stay denied.
- Keep the change readable and testable: the admin branch lives in the DB authorization helpers, not scattered per route.

**Non-Goals:**

- Changing owner/member management semantics (transfer, add/remove, policy, lifecycle stay as-is).
- Changing web UI rendering beyond what the session/machine lists already support.
- Any data migration: no schema or row changes; cluster role is read live from `users`.

## Decisions

1. **Admin bypass inside `authorize_project_registration`.** After the existing active-account and project-lifecycle checks, short-circuit the owner/member test when `user.role() == ClusterRole::Admin`. Rationale: it is the single shared gate for CLI and mobile content access; one branch fixes both paths. Alternative considered: bypass only in ws_cli.rs — rejected because it leaves mobile and future callers inconsistent with the spec.

2. **Session listing via role-aware DB queries.** Add a private helper `user_is_cluster_admin(user_id)` and branch inside `get_sessions_for_user` (admins: all sessions ordered by recency, same LIMIT) and `get_shared_sessions_for_user` (admins: return empty so bootstrap does not double-list). Rationale: callers keep their shapes; the admin distinction stays in one layer. Alternative considered: fetch role in each route and pick a different function — rejected as it would spread the branch across ws_web and mobile call sites.

3. **Machine access via explicit admin check at the web control plane.** `get_machines_for_user` is in-memory and has no DB access; add an admin path at its call sites in ws_web.rs (bootstrap machine list and `StartMachineProjectCli` allowed check) using `state.db.get_user_by_id(&uid)` role lookup. Alternative considered: pushing cluster roles into the in-memory session manager at register time — rejected because role changes would require invalidation plumbing.

4. **Project creation semantics unchanged for admins.** `authorize_project_registration` still INSERT-OR-IGNOREs the project row with the caller as owner when the project does not exist yet; the admin bypass only skips the membership assertion afterwards. This keeps first-registration-owns behavior for every account type.

## Risks / Trade-offs

- [Wider blast radius than the one reported rejection] Admin content access now applies to web/mobile too, not just the CLI. → This is exactly the spec the owner asked for; mitigation is comprehensive tests pinning ordinary-user behavior.
- [Admin bypass could mask a copied `.apas` collision] An admin launching a directory that shares another user's project id will now attach to that project's history instead of being rejected. → Acceptable: that is the intended semantics of canonical project identity plus admin access.
- [Suspended-admin edge] The bypass must not fire for suspended accounts. → Keep the existing `user.is_active()` ensure at the top of `authorize_project_registration` and mirror it in the web role check.
- [No rollback data] If this is later reversed, no migration is needed; behavior-only change. → Standard binary rollback suffices.
