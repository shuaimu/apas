## Context

See `proposal.md` for motivation and the three delta specs for required behavior. Today `users` has no durable cluster role or state; server admin authorization compares one email while the web compares one user ID. Project ownership lives on each `sessions.user_id`, access lives in session-scoped `session_shares` with `owner/admin/user` semantics, and the newer `sessions.project_id` is only a grouping key. Project capability flags are stored in each host's `.apas` file and owner/admin authorization is applied to one combined WebSocket update.

The change crosses SQLite migrations, HTTP and WebSocket authorization, the in-memory session manager, web identity/navigation/admin surfaces, shared protocol types, and CLI-side launch enforcement. Existing deployments and mixed-version clients must remain recoverable throughout rollout.

## Goals / Non-Goals

**Goals:**

- Establish one server-authoritative cluster principal for every HTTP, web WebSocket, CLI, and daemon connection.
- Make projects durable entities whose ownership, membership, lifecycle, and policy do not depend on a particular live session.
- Keep control-plane administration and project data-plane access as separate authorization checks.
- Make policy enforcement resistant to stale or modified web clients and local `.apas` edits.
- Migrate existing identities and access without silently granting cluster-wide authority.

**Non-Goals:**

- Hosting multiple independent clusters or organizations in one server database.
- Custom roles, per-action RBAC editors, multiple project owners, or owner-defined cluster policy.
- Giving cluster administrators implicit access to conversations, files, terminals, or diffs.
- Automatically terminating an already-running ordinary pane solely because its model is later disallowed; it is reported noncompliant and blocked from relaunch instead.
- Billing, usage quotas, or automatic project deletion/retention policy.

## Decisions

### 1. Normalize cluster identity and canonical projects in the server database

Add `cluster_role` (`admin|user`) and `account_status` (`active|suspended`) to `users`. Add canonical `projects` keyed by the existing stable `project_id`, with one `owner_user_id`, `lifecycle_status` (`active|suspended`), and timestamps. Add `project_members` keyed by `(project_id, user_id)`; the owner is represented only by `projects.owner_user_id`, so duplicate or contradictory owner rows cannot exist. Sessions continue to represent runtime instances and reference the canonical project.

Add cluster-default policy, nullable project-policy overrides, and monotonically increasing policy versions. Add append-only administrative audit events. Cluster invitations and project invitations remain distinct token types and project invitations target `project_id`, not a session.

This normalization is preferred over extending `session_shares`: one project can have several sessions or cloned instances, and copying membership/policy to each row would create inconsistent authority. Keeping an explicit owner column is preferred over an owner membership row because the product requires exactly one owner and ownership transfer must be atomic.

### 2. Use centralized guards with an explicit control-plane/data-plane split

Every authenticated entry point loads the current user from the database; JWTs continue to carry only the stable user ID so role and suspension changes take effect without waiting for token expiry. Shared guards enforce:

- `require_active_cluster_user`: authentication, WebSocket/CLI/daemon registration, and project creation.
- `require_cluster_admin`: cluster user/project inventory, account lifecycle and role changes, project lifecycle, ownership override, membership override, policy, and audit APIs.
- `require_project_member`: project conversations, messages, files, diffs, terminals, panes, and ordinary interactive controls.
- `require_project_owner`: owner-managed invitations and ordinary member revocation.

Cluster-admin status deliberately does not satisfy `require_project_member`. Administrative project responses use an explicit metadata DTO rather than reusing session/detail DTOs, preventing accidental content leakage as those DTOs evolve. Suspending an account closes its active web/CLI/daemon connections after the database mutation; all reconnect paths re-check status.

This replaces scattered owner/admin predicates and hard-coded identity checks. Embedding roles in JWTs was rejected because demotion and suspension would otherwise remain stale until token expiration.

### 3. Make onboarding administrator-mediated in cluster mode

The cluster-admin user directory creates time-limited account invitations. Redeeming one establishes credentials and activates the cluster-user account. The existing open registration route either requires a valid cluster invitation or is disabled when cluster administration is enabled. Existing accounts bypass invitations during migration and are activated directly.

The authenticated identity response includes `cluster_role` and `account_status`; the web uses it for navigation and read-only presentation, while the server remains authoritative. A transactional last-active-admin check protects promotion, demotion, suspension, and deletion paths. A one-time deployment bootstrap setting is consulted only when no cluster administrator exists, then ignored once the durable role is established.

Administrator-mediated onboarding is chosen over auto-activating public registrations because “cluster user” represents admission to shared compute and project creation, not merely possession of an APAS login.

### 4. Define a server-owned project capability policy

Represent launch permission as stable launch-profile keys covering pane kind, agent frontend, API backend, and model. The shared supported-profile registry supplies the admin UI and validation; effort remains a per-launch choice and is not governed by this feature. The cluster default contains an explicit allowlist, and each project can inherit or override individual policy fields. Explicit allowlists make newly added models unavailable until a cluster administrator approves them.

The effective policy is computed by overlaying nullable project overrides on the current cluster default. It includes `team_available`, the launch-profile allowlist, and a version. The server persists and distributes it to web and CLI clients. Launch requests are validated server-side before routing and again by the CLI immediately before process creation; both report a policy-specific error. The CLI caches the last accepted version for reconnects and fails closed for new launches if a known cluster project has no valid policy.

Split the current combined project-flags mutation into cluster-policy updates and ordinary project-operation updates. Team availability and model/provider/tab-type restrictions move to the admin policy channel. Owner-operable values such as project goal and permitted workflow behavior retain their existing project authorization instead of accidentally inheriting cluster-admin-only status from a combined payload.

Server ownership is preferred over leaving policy in `.apas`: a project owner controls that file, disconnected instances could disagree, and the cluster inventory could not reliably show or enforce effective policy.

### 5. Treat lifecycle suspension as a durable gate, not only a runtime command

Suspending a project first commits `lifecycle_status=suspended`, then instructs the session manager/daemon to stop connected project runtimes and closes project attachments. The persisted state blocks new session registration, attachment, pane/team launch, and project-instance start even if a host misses the initial stop command. Reactivation removes the gate but does not automatically start compute. A separate “stop runtime” control stops current compute without suspending future access.

This order is chosen so an offline host cannot reconnect into an active state after the administrator has suspended the project.

### 6. Replace the current dashboard and project-role UI around the new boundaries

The admin area becomes role-gated from authenticated identity and provides Users, Projects, and Audit views. The Projects view is paginated/filterable and exposes metadata/status, membership/owner administration, lifecycle controls, and effective policy. It never fetches project content APIs.

The normal sidebar lists only projects the viewer owns or has joined. Project access management offers one immutable Owner display plus ordinary Users; owners can invite/revoke users, while cluster admins use the admin project detail for overrides and ownership transfer. Overview policy controls become read-only indicators for owners/users, with editable controls moved to the admin project detail. All hard-coded admin IDs/emails and project-admin labels/options are removed.

### 7. Roll out with additive compatibility and explicit feature negotiation

New server protocol messages advertise cluster-role and policy support. During the compatibility window, legacy project role `admin` is read as project `user`, and assignment of new project admins is rejected. The server can dual-read legacy session ownership/shares while canonical project backfill completes, but new writes target canonical projects and memberships. A governed project refuses launch mutations from a client version that cannot consume/enforce the effective policy; read-only status can remain available.

For projects whose policy exists only in `.apas`, the first compatible owner-connected CLI sends a migration snapshot. The server imports legacy `team_enabled` and tab-type behavior once, records the import version, and thereafter remains authoritative. Conflicting snapshots are not overwritten silently; they are surfaced to the cluster administrator while the first accepted snapshot remains effective.

## Risks / Trade-offs

- **[Canonical project IDs contain conflicting historical owners]** → Choose the owner from the earliest project session, retain every other historical owner as a project user, and emit a migration audit warning for administrator review.
- **[Mixed client versions bypass host-side policy]** → Require capability negotiation for launch mutations, enforce at the server first, and block launch-capable operations from clients lacking policy support.
- **[An offline host misses suspension or policy updates]** → Persist lifecycle/policy centrally and require a current state/version check at every registration and launch before routing work.
- **[Explicit model allowlists become stale as providers evolve]** → Generate choices from a shared registry, show unavailable/new profiles in the admin UI, and require intentional default-policy updates.
- **[Administrator inventory leaks project data]** → Return a dedicated metadata allowlist, test that content fields never appear, and continue to gate all data-plane endpoints by membership.
- **[Role migration locks out administration]** → Bootstrap and verify at least one durable administrator transactionally before removing legacy checks; reject the last-admin transition.
- **[Large clusters make the admin inventory expensive]** → Use indexed project/member/status queries, summary aggregation, pagination, and filters instead of loading sessions and users per row.
- **[Stopping a project runtime can lose in-flight work]** → Require confirmation in the UI, audit the action, preserve project data, and distinguish stop-runtime from suspend-project.

## Migration Plan

1. Back up the SQLite database and deploy additive schema creation for user role/status, projects, memberships, cluster defaults, project overrides, invitations, and audit events.
2. In one migration transaction, activate existing users, bootstrap the legacy system administrator, create canonical projects from `COALESCE(project_id, session_id)`, select deterministic owners, and collapse session shares into project users with legacy admins downgraded to users.
3. Deploy the server with centralized guards, admin APIs, dual-read compatibility, policy versions, project lifecycle gates, and protocol feature negotiation. Verify an active administrator before disabling legacy authorization.
4. Deploy compatible CLI and daemon code to import each project's legacy `.apas` policy once and enforce server-issued policy/lifecycle state locally.
5. Deploy the web identity state, admin Users/Projects/Audit views, owner/user-only project access UI, and read-only project policy presentation.
6. Verify owner/member access, non-member administrator privacy, suspension, ownership transfer, model/team enforcement, last-admin protection, and migration/audit counts before ending dual-read mode.
7. Retain legacy session-share data for one release as rollback evidence; remove legacy project-admin branches and obsolete columns only in a later cleanup after production verification.

Rollback before step 7 restores the database backup and previous binaries. During the compatibility release, disabling new routes and reverting reads to legacy tables remains possible because the migration is additive and legacy rows are retained; administrative changes made only in new tables must be exported before rollback to avoid losing them.
