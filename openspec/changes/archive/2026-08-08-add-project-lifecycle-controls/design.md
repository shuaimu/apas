## Context

See [proposal.md](proposal.md) for motivation and [specs/project-lifecycle-management/spec.md](specs/project-lifecycle-management/spec.md) for the behavioral contract.

Project identity and authorization are already canonicalized in SQLite: `projects.owner_user_id` holds the sole owner and `project_members` holds ordinary users. `session_shares` remains as compatibility data, and the cluster-administration API already has a permissive ownership-transfer primitive for administrators. The ordinary user-facing sharing API is still session-addressed and offers no transfer, self-leave, or deletion operation.

Project data spans several independently managed stores:

- SQLite rows in `projects`, `sessions`, `messages`, `pane_usage_stats`, `project_members`, `session_shares`, `invitation_codes`, `project_policy_overrides`, and `admin_audit_events`.
- File-backed session directories containing `messages.jsonl`, `panes.json`, and temporary message-GC files.
- `SessionManager` state for sessions, project/session mappings, terminal scrollback, recent input IDs, connection routing, and cached project access references.
- Connected CLI and daemon processes. The daemon already supports `StopProjectCli` and does not automatically restart stopped projects after reconnect, but its local project registry and source checkout intentionally remain on the machine.

SQLite foreign keys do not cover every project relationship: `sessions.project_id` and `pane_usage_stats.session_id` are not cascading foreign keys, and project identity sometimes uses the legacy `COALESCE(project_id, id)` form. Deletion therefore requires an explicit data inventory rather than relying on deletion of the `projects` row alone. File deletion and SQLite transactions also cannot be one atomic commit.

## Goals / Non-Goals

**Goals:**

- Keep owner/member authorization server-authoritative and re-check it at every lifecycle mutation.
- Make transfer and departure atomic across canonical membership and compatibility rows.
- Make permanent deletion exclusive with all project writes, idempotent, and restart-resumable.
- Revoke live access and refresh affected clients without treating client-side state as an authorization boundary.
- Preserve existing cluster-administrator control-plane semantics while adding owner/member self-service routes.

**Non-Goals:**

- Deleting a source checkout, `.apas` configuration, or daemon project-registry entry on a developer machine.
- Erasing infrastructure-managed backups, reverse-proxy access logs, or system service journals outside APAS application storage; those require the cluster's operational retention process.
- Allowing the sole owner to leave, supporting multiple owners, or allowing transfer directly to a non-member.
- Undo, recovery, export, or grace-period behavior after destructive cleanup begins.
- Giving project owners cluster-administrator powers or giving cluster administrators implicit project-content access.

## Decisions

### 1. Add canonical project-scoped self-service HTTP routes

Add authenticated routes addressed by canonical `project_id`:

- `PATCH /projects/:project_id/owner` with `{ "user_id": ... }` for owner transfer.
- `DELETE /projects/:project_id/members/me` for self-departure; the identity always comes from the bearer token, never from a request-supplied target user.
- `POST /projects/:project_id/delete` with `{ "confirmation": "<project_id>" }` for permanent deletion. An action endpoint avoids dependence on HTTP `DELETE` request bodies while retaining a project-bound confirmation.
- `GET /projects/:project_id/deletion` for the initiating owner to observe an in-progress deletion. A missing project after an accepted deletion is the terminal completed state; no durable completed-deletion record is retained.

Each handler loads the active authenticated account and current project row, checks the role in the same server operation that mutates state, and returns explicit authorization, validation, conflict, or not-found errors. Transfer requires an active project; leave is allowed for a member of an active or suspended project; deletion is allowed to the current owner of an active or suspended project. No self-service mutation is accepted once the internal deletion state begins.

This is preferable to overloading the session-oriented `/share` endpoints because the mutations affect every session and future instance. The existing administrator routes remain separate so their authorization and broader transfer target rules stay clear.

### 2. Separate owner-transfer policy from administrator-transfer policy

Refactor the existing database transfer implementation around one transactional role-swap primitive with two caller policies:

- The owner policy requires `actor_user_id` to equal the current owner, the lifecycle to be active, and the target to be an active existing `project_members` row.
- The cluster-administrator policy keeps the existing ability to select any active cluster user without first joining that user to the project.

Within one serialized SQLite write transaction, re-read the owner and target eligibility, conditionally update the owner, delete the new owner's ordinary membership rows, and add the former owner as an ordinary member. Synchronize `session_shares` for every session in the canonical project: remove compatibility-user rows for the new owner and add ordinary-user rows for the former owner. Record the successful transfer with an explicit project identifier for audit cleanup.

Self-departure similarly verifies that the actor is an ordinary member, deletes the canonical membership, and removes all of that user's compatibility `session_shares` for the project's sessions in one transaction. It records a `project.member_left` event but never accepts a target identity from the client.

A serialized transaction plus conditional predicates is preferred to a read-then-write handler sequence because concurrent transfer, removal, and departure must not create an ownerless project or promote a user who has already left.

### 3. Treat server authorization, not UI refresh, as the security boundary

Add a project-access-changed server-to-web notification carrying the project ID, affected user, and whether access was revoked, transferred, or deleted. Affected browser clients request a fresh session list and role data. The initiating HTTP client also refreshes immediately after success/acceptance.

Extend `SessionManager` with targeted operations that can:

- Detach one user's web connections from all sessions of a project and disconnect only that user's CLI associations when they leave, without stopping the owner's shared runtime.
- Notify the old and new owner after transfer without stopping the runtime.
- Stop all project CLI processes, send `StopProjectCli` to connected daemons, detach all viewers, and purge all project/session caches during deletion.

Every privileged HTTP and WebSocket operation continues to re-check current database authorization and lifecycle. Notifications improve UX but are not trusted to revoke authority; a stale client is rejected even if it missed the event.

### 4. Use an internal `deleting` lifecycle as the durable recovery marker

Extend the persisted project lifecycle with an internal `deleting` value. It is not selectable through the cluster-administrator lifecycle API. Registration, invitation redemption, attachment, history reads, membership changes, pane operations, and message/file persistence all fail closed for a deleting project. Administrator lifecycle updates cannot move a deleting project back to active or suspended.

The delete request atomically checks the owner and confirmation and changes `active` or `suspended` to `deleting`. Crossing that boundary is irreversible. The request then schedules server-owned cleanup and returns an accepted/in-progress response; it does not report deletion complete. Repeated valid requests for the same deleting project are idempotent and ensure cleanup is scheduled.

At startup, after migrations and before opening the router, the server enumerates every `deleting` project and completes its cleanup. This uses the project row and still-present session rows as the durable recovery manifest, avoiding a permanent tombstone or separate completed-deletion table.

Using the lifecycle row is preferable to immediately deleting the project or using only an in-memory job: it blocks reconnects after a process crash and leaves enough durable context to finish file cleanup. It is preferable to a retained soft-delete row because the final state must contain no project record.

### 5. Quiesce writes, delete files first, and commit relational deletion last

Introduce a project mutation gate shared by HTTP mutations, session registration, WebSocket persistence paths, and deletion. Normal project mutations acquire a shared operation permit and re-check lifecycle; deletion marks the project `deleting`, obtains the exclusive permit, and holds it through cleanup. Session-scoped operations resolve their canonical project before acquiring the permit. File operations continue to use the existing per-session locks, so an append already in progress finishes before its directory is removed and no later append can recreate it.

Cleanup proceeds idempotently in this order:

1. Collect the canonical session IDs and affected user IDs while database rows still exist.
2. Stop and detach runtime clients, drain project writers, and remove project-specific `SessionManager` state.
3. Under each session's storage lock, remove its entire session directory. Missing directories count as already cleaned; temporary GC files inside the directory are removed with it.
4. In one SQLite transaction, explicitly delete rows for those session IDs from `messages`, `pane_usage_stats`, `session_shares`, and `invitation_codes`; delete project-level invitations, policy overrides, members, and project-identifying audit rows; delete the sessions; then delete the `projects` row last.
5. Release the gate and notify affected browser clients only after the relational commit succeeds.

If file cleanup fails, the database still contains the `deleting` project and its session manifest, so recovery retries without exposing it. If the final database transaction fails after files were removed, the project remains deleting and recovery repeats the idempotent relational cleanup. The server never has to reconstruct session IDs after deleting the project row, and it never reports completion while file artifacts remain.

Direct file removal before the final database transaction is preferable to deleting database rows first: a crash after database commit would otherwise orphan files without a durable project-to-session mapping. A compensating rollback is intentionally not attempted after cleanup begins because deletion is irreversible and partially restored history would violate the contract.

### 6. Add an explicit project key to auditable events

Add a nullable indexed `project_id` column to `admin_audit_events`. Backfill it from project-targeted events and valid `details.project_id` values, and populate it for every future project owner/member/policy/lifecycle action. The deletion transaction can then reliably erase `WHERE project_id = ?` instead of relying on target-type conventions or substring matching in JSON.

Malformed legacy audit details are parsed and reviewed during migration; events that can be deterministically associated by `target_type`, `target_id`, or valid structured details receive the project key. This is preferable to broad text matching, which could delete an unrelated event whose free-form details happen to contain the same identifier.

The deletion action itself does not leave a project-identifying APAS audit event after completion. Operational metrics may count successful deletions without labels that identify the project. This is a deliberate privacy trade-off required by the erasure contract; infrastructure logs and backups remain subject to the non-goal above.

### 7. Place lifecycle actions in a role-aware project access surface

Keep the current invite/manage-access workflow, but make project actions reachable for both roles and carry both representative `sessionId` and canonical `projectId` in the sidebar's project view model.

- Owners see a transfer action beside each eligible ordinary user. Confirmation names the recipient and states that the current owner will become a user.
- Owners see a separate danger section for permanent deletion and must type the canonical project ID shown in the dialog.
- Ordinary users see a leave action and a confirmation explaining immediate loss of all project instances and history access.
- Owners do not see leave; users do not see transfer or delete.

After transfer, refresh the access list and session roles. After leave or accepted deletion, close the modal, detach the current view if affected, clear current-session state, navigate to the project chooser/overview, and refresh sessions. Server notifications apply the same behavior to other open clients.

## Risks / Trade-offs

- **[An unenumerated table retains project data]** → Centralize the relational deletion inventory, seed every project-linked table in an integration test, and assert no project/session key survives. Review this inventory whenever a new project-scoped table is added.
- **[A delayed writer recreates a deleted session directory]** → Put every persistence path behind the project mutation gate, re-check lifecycle, drain writers with the exclusive permit, and use per-session storage locks during removal.
- **[A crash leaves a partially deleted project]** → Persist `deleting` before cleanup, keep session rows until files are gone, run idempotent recovery before serving traffic, and fail closed throughout.
- **[A large project exceeds an HTTP or proxy timeout]** → Return accepted/in-progress after the irreversible state transition, run cleanup in a server-owned task, expose transient deletion status, and notify clients on completion.
- **[A stopped local checkout recreates the project]** → Send the existing daemon stop command, sever registered CLI sessions, and reject registration while `deleting`; a later user-initiated start after completion is treated as a fresh project by design.
- **[Role changes leave stale privileged UI]** → Push access-change notifications and refresh role/session state, while independently enforcing every mutation against current database state.
- **[Deleting audit history weakens administrator forensics]** → Make the trade-off explicit, remove project-identifying APAS events as required, and retain only unlabeled aggregate operational metrics.
- **[Legacy lifecycle parsing treats an unknown value as active]** → Add an explicit `Deleting` enum value and change unknown lifecycle parsing to fail closed before any row can be migrated to deleting.
- **[Deletion failure is irreversible]** → Do not offer cancellation after the lifecycle transition; display in-progress status and retry to completion rather than exposing or attempting to restore a partially erased project.

## Migration Plan

1. Deploy additive server migrations and code first: add/backfill/index `admin_audit_events.project_id`, update lifecycle validation to admit internal `deleting`, make unknown lifecycle values fail closed, add mutation gates and cleanup recovery, and keep the new routes unused by old web clients.
2. Before accepting traffic, recover any deleting projects. Verify that ordinary active/suspended projects and existing administrator routes retain their behavior.
3. Deploy the web client with canonical project IDs, role-aware actions, confirmations, access-change handling, and deletion-progress behavior.
4. Exercise transfer, self-leave, online/offline deletion, restart-during-deletion, and fresh local re-registration in staging. Verify SQLite, session directories, in-memory state, invitations, admin inventory, and web lists after completion.
5. Monitor unlabeled deletion counts and cleanup failures. Do not log project IDs or project content from the new deletion worker.

Rollback is safe before any deletion enters `deleting`. Once a deletion begins, it must be completed with the new server because cleanup is intentionally irreversible. Do not roll back to a binary that parses unknown lifecycle values as active while any deleting row exists; finish recovery first. Completed deletions remain deleted across a code rollback and can only be restored from infrastructure backup according to the cluster's separate backup policy.
