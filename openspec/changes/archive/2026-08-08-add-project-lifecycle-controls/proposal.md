## Why

Project owners currently need cluster-administrator intervention to hand a project to another member, while ordinary project users cannot remove their own access and owners cannot permanently remove an obsolete project. These missing lifecycle controls leave ownership, membership, and retained project data stuck after the people responsible for them have changed.

## What Changes

- Let a project owner transfer ownership to an active user who is already a member of that project; the recipient becomes the sole owner and the former owner remains in the project as an ordinary user.
- Let an ordinary project user leave a project without owner or cluster-administrator action, immediately revoking that user's project-wide access. The sole owner must transfer ownership or delete the project instead of leaving it ownerless.
- Let the project owner permanently delete the project after an explicit destructive confirmation.
- On deletion, stop the connected runtime, detach viewers, remove the project's in-memory state, and erase all server-held project records and logs, including every session/instance, message, pane/usage record, invitation, membership/share, policy override, and project-identifying audit entry.
- Define deletion as server-side: it does not remove the source checkout or local APAS configuration from a developer machine. A later explicit start from that checkout may register a fresh empty server project, but no deleted server history or membership is restored.
- Scope the erasure guarantee to APAS-managed application storage; infrastructure-managed backups, reverse-proxy logs, and system service journals remain governed by cluster operational retention policy.
- Keep the existing cluster-administrator ownership and membership controls separate; these new self-service operations do not grant project content access or cluster-administrator authority.

## Capabilities

### New Capabilities

- `project-lifecycle-management`: Owner-initiated ownership transfer and permanent project deletion, member-initiated project departure, authorization rules, live-session revocation, and complete server-side data cleanup.

### Modified Capabilities

None.

## Impact

- Server HTTP routes and authorization for project-scoped transfer, leave, and delete operations.
- SQLite project/session persistence, compatibility share rows, invitation and usage data, policy overrides, and project-related audit data.
- File storage under server session directories, plus failure recovery for destructive cleanup.
- SessionManager runtime, terminal scrollback, cached membership references, web/CLI/daemon connections, and project-list refresh behavior.
- Web project access/settings UI, destructive confirmations, post-operation navigation, and role/list state refresh.
- Integration, authorization, storage-cleanup, disconnect, and regression tests across the Rust server and Next.js web client.
