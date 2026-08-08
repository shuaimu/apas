## 1. Persistence Invariants and Lifecycle Migration

- [x] 1.1 Add the internal `deleting` project lifecycle value, migrate the SQLite lifecycle triggers to allow it, and make unknown lifecycle values fail closed without exposing `deleting` through administrator lifecycle updates.
- [x] 1.2 Add and index `admin_audit_events.project_id`, backfill deterministic project associations from structured legacy events, and update all project audit writers to populate the column.
- [x] 1.3 Add database helpers that atomically mark an owner-confirmed project as deleting, list deletion manifests (session IDs and affected users), report deletion status, and enumerate interrupted deletions for startup recovery.
- [x] 1.4 Add migration/database tests for active and suspended deletion transitions, unauthorized or mismatched confirmation rejection, deleting-state immutability, fail-closed parsing, and audit-key backfill.

## 2. Project Mutation and Storage Exclusion

- [x] 2.1 Add a project-scoped shared/exclusive mutation gate to application state, including session-to-project resolution that rejects missing or non-operable project state.
- [x] 2.2 Put CLI registration and CLI-to-server message, terminal, pane-list, usage, and file-persistence mutations behind the shared project gate and an active-lifecycle re-check.
- [x] 2.3 Put web-to-server message, pane, history, and other persisted project mutations behind the same gate and lifecycle checks so a deleting project cannot accept delayed writes.
- [x] 2.4 Add an idempotent `FileStorage` bulk session-directory deletion operation that uses the existing per-session locks, treats missing paths as success, removes GC temporary files with the directory, and releases obsolete lock entries safely.
- [x] 2.5 Add concurrency tests proving an in-flight append is either removed by deletion or rejected after the deletion boundary and cannot recreate a cleaned session directory.

## 3. Ownership Transfer and Self-Departure

- [x] 3.1 Refactor ownership transfer into one atomic role-swap primitive with distinct cluster-administrator and owner policies, preserving the administrator's ability to choose any active cluster user.
- [x] 3.2 Enforce owner-initiated transfer eligibility for an active existing project user and synchronize former/new owner roles across `project_members` and every compatibility `session_shares` row.
- [x] 3.3 Implement atomic self-departure for ordinary users, removing canonical and compatibility membership project-wide while rejecting owner and other-user departure attempts.
- [x] 3.4 Add database concurrency and regression tests for transfer-versus-leave/removal races, exactly-one-owner invariants, former-owner downgrade, ineligible targets, administrator behavior, and multi-session compatibility rows.

## 4. Runtime Revocation and Client Notifications

- [x] 4.1 Add a shared project-access-changed server-to-web protocol event for transfer, access revocation, and deletion completion, with compatibility-safe handling for older web clients.
- [x] 4.2 Add `SessionManager` support to detach one user's project web/CLI associations without stopping other users' runtime and to refresh affected users after transfer or departure.
- [x] 4.3 Add `SessionManager` project purge support that stops CLI runtime, sends `StopProjectCli` to daemons, detaches all viewers, and clears sessions, project mappings, terminal scrollback, pane/status caches, recent input IDs, and access-reference caches.
- [x] 4.4 Add session-manager tests proving self-departure only revokes the departing user, transfer leaves runtime running, and deletion removes every project-specific in-memory entry and connection route.

## 5. Self-Service HTTP API

- [x] 5.1 Add project-scoped authentication/authorization helpers and routes for owner transfer, member self-departure, owner-confirmed deletion, and transient deletion status.
- [x] 5.2 Wire successful transfer and departure to runtime revocation/notification behavior and return stable authorization, validation, conflict, accepted, and not-found responses for stale clients.
- [x] 5.3 Add route tests covering owner, ordinary user, non-member, former owner, suspended account, active/suspended project, ineligible transfer target, owner leave, cross-project confirmation, and repeated deletion requests.

## 6. Permanent Deletion Coordinator and Recovery

- [x] 6.1 Implement the server-owned deletion coordinator that takes the exclusive project gate, collects the manifest, quiesces runtime and writers, deletes session directories, and then invokes relational cleanup.
- [x] 6.2 Implement one idempotent relational deletion transaction covering messages, pane usage, compatibility shares, session/project invitations, sessions, policy overrides, memberships, project-keyed audit events, and the project row last.
- [x] 6.3 Run interrupted-deletion recovery after migrations and before opening the HTTP/WebSocket router, and add bounded background retry/reporting for cleanup failures without logging project identifiers or content.
- [x] 6.4 Add end-to-end deletion tests that seed every APAS-managed project artifact across multiple sessions, verify runtime disconnection and API invisibility, and assert no database row, file, cache, invitation, audit event, or admin inventory entry survives completion.
- [x] 6.5 Add failure-injection/restart tests for file-removal and database-commit failures, delayed client events, idempotent retry, and fresh explicit re-registration with no restored history or membership.

## 7. Web Lifecycle Controls

- [x] 7.1 Preserve both representative session ID and canonical project ID in the sidebar project view model and expose a project-actions/access entry point to owners and ordinary users.
- [x] 7.2 Add owner transfer controls for eligible listed users with recipient and former-owner-downgrade confirmation, then refresh access and role state after success.
- [x] 7.3 Add the ordinary-user leave action with immediate-access-loss confirmation, and clear attachment/session state, navigate away, and refresh projects after success.
- [x] 7.4 Add the owner-only danger section and deletion dialog requiring the canonical project ID, show accepted/in-progress state, and remove/navigate away from the project only when completion is observed.
- [x] 7.5 Handle project-access-changed events in the web store so other tabs/clients refresh roles and sessions, detach revoked users, and remove deleted project content and controls.
- [x] 7.6 Add component/store regression tests for role-based action visibility, transfer choices and confirmation, owner-leave prevention, exact deletion confirmation, API failures, stale-role events, and post-leave/delete navigation.

## 8. Verification and Operational Handoff

- [x] 8.1 Run focused Rust database, route, storage, session-manager, WebSocket, and deletion-recovery tests, then run the full Rust workspace test suite.
- [x] 8.2 Run the web unit tests, lint/type checks, and production build, fixing any lifecycle-flow or protocol regressions.
- [x] 8.3 Review the project-data deletion inventory against the final schema and file layout, verify deletion workers emit only unlabeled operational metrics, and document the local-checkout and infrastructure-retention boundaries in administrator-facing help.
