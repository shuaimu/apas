## 1. Database Model and Migration

- [x] 1.1 Add typed cluster role/account status models and the additive SQLite columns, constraints, and indexes on users.
- [x] 1.2 Add canonical projects, project memberships, cluster defaults, project policy overrides, cluster invitations, and administrative audit-event tables.
- [x] 1.3 Backfill existing users as active cluster users and transactionally bootstrap the legacy system administrator from one-time deployment configuration.
- [x] 1.4 Backfill canonical projects from session project IDs, choose deterministic owners, retain conflicting owners as users, and emit migration warnings.
- [x] 1.5 Collapse session shares into project memberships, downgrade legacy project admins to users, and migrate outstanding project invitations to canonical project IDs.
- [x] 1.6 Add database repositories for cluster accounts, canonical project access, lifecycle, effective policy/version, inventory summaries, and paginated audit queries.

## 2. Authentication and Authorization Foundation

- [x] 2.1 Extend login and current-identity responses with server-loaded cluster role and account status while leaving mutable authorization data out of JWT claims.
- [x] 2.2 Implement shared active-cluster-user, cluster-admin, project-owner, and project-member guards with an explicit control-plane/data-plane boundary.
- [x] 2.3 Apply active-account checks to HTTP authentication, web WebSocket, CLI, and daemon entry points and disconnect live connections when an account is suspended.
- [x] 2.4 Replace hard-coded admin email/user-ID authorization and add a transactional last-active-administrator safeguard.
- [x] 2.5 Add administrator-created cluster invitation and redemption flows, and prevent uninvited open registration from granting cluster access.

## 3. Canonical Project Access Control

- [x] 3.1 Create or resolve the canonical project during CLI project registration, assign the first active creator as owner, and reject unauthorized or suspended registrations.
- [x] 3.2 Change session discovery and attachment authorization to derive owner/user access from canonical project membership across every project instance.
- [x] 3.3 Replace session-share owner/admin/user APIs with project-scoped owner/user listing, invitation, acceptance, and user-revocation APIs for project owners.
- [x] 3.4 Add cluster-admin project membership and ownership-transfer APIs that operate without adding the administrator as a project member.
- [x] 3.5 Reject all new project-admin assignments and provide compatibility reads that interpret legacy project admins as users during rollout.
- [x] 3.6 Verify administrative metadata endpoints never return conversations, messages, files, diffs, terminal data, or other project-content fields.

## 4. Cluster-Governed Project Policy

- [x] 4.1 Define shared stable launch-profile keys and a supported-profile registry covering pane kind, agent frontend, API backend, and model.
- [x] 4.2 Implement cluster-default plus nullable project-override policy resolution with explicit model/profile allowlists, team availability, and monotonic versions.
- [x] 4.3 Add cluster-admin APIs for reading and updating defaults and per-project overrides, including validation and audit events.
- [x] 4.4 Split cluster policy mutations from owner-operable project settings so legacy combined payloads cannot change governed fields without cluster-admin authority.
- [x] 4.5 Add protocol capability negotiation and effective-policy distribution to web, CLI, and daemon connections, blocking launch mutations from incompatible clients.
- [x] 4.6 Import each existing project's `.apas` team/tab policy once, preserve the first accepted snapshot, and surface conflicting snapshots for administrator review.
- [x] 4.7 Enforce allowed launch profiles and backend switches on the server before routing requests, with policy-specific errors for stale or disallowed requests.
- [x] 4.8 Enforce the current policy version again in CLI pane/team creation and refuse local `.apas` changes that conflict with cluster policy.
- [x] 4.9 Report running panes that become disallowed as noncompliant while blocking relaunch instead of silently terminating them.

## 5. Cluster Project Inventory and Lifecycle

- [x] 5.1 Add paginated/filterable cluster-admin user and project inventory endpoints with owner, member count, effective policy, connectivity, active-session summary, and last activity.
- [x] 5.2 Implement durable project suspension/reactivation gates across registration, attachment, instance start, pane launch, and team launch.
- [x] 5.3 Route suspend and stop-runtime actions through the session manager/daemon, disconnect affected attachments, preserve data, and require explicit reactivation after suspension.
- [x] 5.4 Implement append-only audit recording for cluster account, role, project lifecycle, ownership, membership, and policy mutations.

## 6. Web Identity, Administration, and Project UI

- [x] 6.1 Store cluster role/status from authenticated identity and replace all hard-coded admin navigation and page guards with role-aware state.
- [x] 6.2 Build the admin Users view for invitation, activation/suspension, promotion/demotion, last-admin errors, and account status.
- [x] 6.3 Build the admin Projects list/detail views for status, owner/members, ownership transfer, membership override, suspension/reactivation, stop-runtime, and effective policy.
- [x] 6.4 Build the admin Audit view with pagination and actor/action/target/time details.
- [x] 6.5 Update the normal sidebar and access modal to show only owner/user project roles, apply membership across project instances, and remove project-admin options and labels.
- [x] 6.6 Make model/provider and team-availability policy read-only for owners/users and expose editable controls only in the cluster-admin project detail.
- [x] 6.7 Filter pane and team launch choices by effective policy and present clear suspended, incompatible-client, and policy-rejection states.

## 7. Verification and Rollout

- [x] 7.1 Add migration tests for existing users, bootstrap administration, conflicting owners, duplicate shares, legacy admin downgrade, invitations, and legacy policy import.
- [x] 7.2 Add an authorization matrix covering cluster user/admin and project owner/user/non-member behavior across HTTP, WebSocket, CLI, and daemon paths.
- [x] 7.3 Add privacy tests proving non-member cluster administrators receive project metadata/status but cannot attach to project content or interactive controls.
- [x] 7.4 Add policy tests for inheritance/overrides, owner rejection, model/provider filtering, stale clients, server/CLI double enforcement, and running-pane noncompliance.
- [x] 7.5 Add lifecycle and audit tests for suspension, reactivation, stop-runtime, offline-host reconnect, successful mutation events, and rejected mutations.
- [x] 7.6 Add web component and integration tests for admin Users/Projects/Audit views, owner/user access management, role-aware navigation, and read-only policy presentation.
- [x] 7.7 Run the complete Rust and web test suites, lint, production builds, and mixed-version compatibility tests; resolve all introduced regressions.
- [x] 7.8 Document database backup/bootstrap configuration, staged server/CLI/web deployment, verification queries, rollback, and later legacy-table cleanup.
