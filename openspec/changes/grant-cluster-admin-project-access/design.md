## Context

See proposal.md - Why. Enforcement today funnels through a few DB choke points:

- `db::authorize_project_registration(project_id, user_id)` (crates/server/src/db/mod.rs) is the shared gate for CLI `SessionStart` (routes/ws_cli.rs:482) and mobile bootstrap (routes/mobile.rs:278). It creates-or-ignores the project row and then requires the caller to be owner or member.
- Web attach and control paths check `db::check_session_access(session_id, user_id)`; web bootstrap and session lists call `db::get_sessions_for_user` (owner-scoped) and `db::get_shared_sessions_for_user` (member-scoped).
- Machine listings already follow the same model as the rest of the change: `get_machines_for_user` returns only machines whose daemon registered under the requester's account, which is exactly "the requester's own virtual cluster". No machine-path change is needed.
- `User::role() -> ClusterRole` already exists, so no new auth primitives are needed.

## Goals / Non-Goals

**Goals:**

- Active cluster admins pass project content authorization for projects present in their own virtual cluster: projects they own, projects they belong to, or projects with at least one session created under their account.
- Admin listings reflect the same boundary: no other users' clusters leak into the admin's sidebar.
- Ordinary users keep exactly today's gating. Suspended admin accounts stay denied.

**Non-Goals:**

- Granting admins blanket access to every project in the deployment (rejected: the "cluster" is the admin's virtual cluster, not the physical cluster; other users' projects stay out).
- Changing owner/member management semantics (transfer, add/remove, policy, lifecycle stay as-is).
- Any data migration: no schema or row changes; cluster role and session authorship are read live.

## Decisions

1. **Admin bypass inside `authorize_project_registration`, gated on session authorship.** After the existing active-account and project-lifecycle checks, an admin who is neither owner nor member passes only when a session row exists with `COALESCE(project_id, id) = project` and `user_id = admin` — i.e. the project is present in the admin's virtual cluster. Alternative considered: blanket admin return (first implementation) — rejected because it leaked every other user's projects into the admin's listings and access.

2. **Session listing via cluster-scoped admin query.** For admins, `get_sessions_for_user` returns sessions where the admin owns the project, belongs to it, or authored the session (`s.user_id = ?`); `get_shared_sessions_for_user` returns empty for admins so the union does not double-list. Callers keep their shapes. Alternative considered: route-level role branches — rejected as spreading the distinction across call sites.

3. **`check_session_access` evaluates admin session authorship as an additional grant, not an early return.** The admin branch returns true only when the target session's row was created under the admin's account and the project is active; otherwise evaluation falls through to the regular owner/member check, so an admin who owns a project still reaches member-run sessions through the owner branch.

4. **No changes to machine paths.** `get_machines_for_user`, `list_accessible_machines_for_user`, and the `StartMachineProjectCli`/`StopMachineProjectCli` allowed checks are already scoped to the requester's own daemon registrations (their virtual cluster), which matches the corrected model. The earlier `get_all_machines` addition is withdrawn.

5. **Project creation semantics unchanged.** `authorize_project_registration` still INSERT-OR-IGNOREs the project row with the caller as owner when the project does not exist yet; first-registration-owns behavior is kept for every account type.

## Risks / Trade-offs

- [Admin copying a `.apas` from another account's project cannot adopt it] First launch is rejected until the admin regenerates the `.apas` or gains membership, because no session of that project exists under the admin yet. → Accepted: this is the intended boundary between virtual clusters; the existing "already owned by another user" hint and the membership invite flow cover the remedy.
- [Session authorship is historical evidence, not a current grant] A project stays in the admin's cluster even after the admin's last session for it ends. → Accepted and intended: the virtual cluster is defined by the sessions created under the account; removal is an explicit membership/cleanup operation.
- [Suspended-admin edge] The bypass must not fire for suspended accounts. → The existing `user.is_active()` ensure at the top of `authorize_project_registration` and the active-account read in `user_is_cluster_admin` cover this.
- [No rollback data] Behavior-only change. → Standard binary rollback suffices.
