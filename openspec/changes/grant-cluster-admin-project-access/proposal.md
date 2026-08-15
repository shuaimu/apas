## Why

Cluster administrators currently cannot open projects they do not own or belong to: the 2026-08-08 project-access-control deployment made content access require explicit project membership, which rejects an admin's own CLI daemon when it launches a project directory whose canonical project is owned by another user but whose session runs under the admin's account (first observed with `mako-soumojit`, owned by `soumojit.dalui`, launched from the admin's `zoo-005` daemon). Running the cluster is the admin's job; being a member of every project is not.

## What Changes

- An active cluster administrator MAY open, attach to, and operate projects that are present in their own virtual cluster — projects they own, projects they belong to, and projects with at least one session created under their account — without being the project owner or a project member.
- An administrator does NOT gain access to projects that exist only in other accounts' clusters; those keep their owner/member gating.
- Machine listings and machine-scoped operations are unchanged: they were already scoped to the requester's own daemon registrations.
- The spec requirement "Control-plane authority does not grant project-content access" is replaced by the cluster-scoped requirement above.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `project-access-control`: Cluster administrators gain project content access within their own virtual cluster without project membership; the requirement denying non-member administrators project content access is removed and replaced.

## Impact

- `crates/server/src/db/mod.rs`: `authorize_project_registration`, `check_session_access`, and `get_sessions_for_user` gain a cluster-scoped admin branch keyed on sessions created under the administrator's account; `get_shared_sessions_for_user` returns an empty list for admins to avoid double-listing.
- `crates/server/src/routes/ws_cli.rs`: CLI `SessionStart` rejection path no longer fires for admins operating a project present in their cluster (via the shared DB authorization change).
- Tests: server unit tests pin the cluster-scoped admin access and confirm foreign-cluster projects and ordinary non-member users are still gated.
