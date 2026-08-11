## Why

APAS currently mixes project collaboration roles with deployment-wide administration: projects expose an `admin` role while the actual system administrator is hard-coded separately. A cluster needs a durable user and administrator model so people can create and own projects while cluster-wide policy and oversight remain with cluster administrators.

## What Changes

- Add cluster membership with `user` and `admin` roles plus an active/suspended account state; active cluster users can authenticate, connect clients, and create projects.
- Replace hard-coded administrator identity checks with server-authoritative cluster roles and expose the current user's cluster role through authenticated APIs.
- Add a cluster administration surface for managing cluster users and viewing every project's owner, members, connectivity/activity status, and effective policy.
- Let cluster administrators suspend or reactivate a project and stop its active runtime from the project inventory without joining the project.
- Allow cluster administrators to manage any project's owner and users from the control plane without automatically becoming a project member or gaining access to project conversations.
- Make the creator of a new project its owner and reduce project roles to `owner` and `user`; owners retain project collaboration and user-management responsibilities.
- Reserve per-project capability policy for cluster administrators, including whether team mode is available and which model/provider combinations may be launched; enforce that policy server-side and at the CLI boundary.
- **BREAKING** Remove the project-level `admin` role and stop treating project owners as administrators for cluster-governed settings.
- Migrate existing accounts to active cluster users, bootstrap the existing system administrator as a cluster administrator, and downgrade legacy project-admin memberships to ordinary project users rather than granting cluster-wide authority.

## Capabilities

### New Capabilities

- `cluster-user-administration`: Cluster membership, cluster roles, account lifecycle, administrator authorization, and the cluster-wide user/project inventory.
- `project-access-control`: Project ownership and membership using only owner/user roles, including creation, access, membership management, and cluster-admin oversight.
- `project-policy-governance`: Cluster-admin-only per-project model/provider and team-mode policy with consistent UI, server, and CLI enforcement.

### Modified Capabilities

None.

## Impact

- Requires durable database changes for cluster role/status, canonical projects and project memberships, project lifecycle status, project policy, and migration from session-scoped ownership/shares.
- Affects authentication responses and guards, admin/share/project APIs, WebSocket authorization, session/project discovery, and CLI policy synchronization/enforcement.
- Reworks the web admin dashboard, project access management, project settings controls, navigation guards, and role-aware state.
- Requires compatibility handling for existing users, sessions, invitation codes, project-admin shares, older clients, and projects whose policy currently lives in `.apas`.
