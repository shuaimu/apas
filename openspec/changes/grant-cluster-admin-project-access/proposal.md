## Why

Cluster administrators currently cannot open projects they do not own or belong to: the 2026-08-08 project-access-control deployment made content access require explicit project membership, which rejects an admin's own CLI daemon when it launches a project directory whose canonical project is owned by another user (first observed with `mako-soumojit`, owned by `soumojit.dalui`, launched from the admin's `zoo-005` daemon). Running the cluster is the admin's job; being a member of every project is not.

## What Changes

- An active cluster administrator MAY open, attach to, and operate any project in the cluster (CLI sessions, web attach, machine project launches) without being the project owner or a project member.
- Project ownership and membership rules are unchanged for ordinary users.
- Project ownership/membership management operations (transfer owner, add/remove member, policy, lifecycle) are unchanged and remain admin-only control-plane operations.
- The spec requirement "Control-plane authority does not grant project-content access" is replaced: cluster administration now explicitly grants project content access across the cluster.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `project-access-control`: Cluster administrators gain project content access across the cluster without project membership; the requirement denying non-member administrators project content access is removed and replaced.

## Impact

- `crates/server/src/db/mod.rs`: `authorize_project_registration` and session-listing queries (`get_sessions_for_user`, `get_shared_sessions_for_user`) gain a cluster-admin branch.
- `crates/server/src/routes/ws_cli.rs`: CLI `SessionStart` rejection path no longer fires for admins (via the shared DB authorization change).
- `crates/server/src/routes/ws_web.rs`: web bootstrap/session lists and the `StartMachineProjectCli` allowed check honor admin access.
- `crates/server/src/session/mod.rs`: machine listing (`get_machines_for_user`) gains an admin-visible variant used by the web control plane.
- `crates/server/src/routes/mobile.rs`: mobile session bootstrap inherits the admin bypass through `authorize_project_registration`.
- Tests: server unit tests updated/extended to pin the admin bypass and confirm ordinary users are still gated.
