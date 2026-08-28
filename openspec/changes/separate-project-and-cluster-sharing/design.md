## Context

See `proposal.md` for motivation. APAS persists project ownership/membership separately from cluster membership and project placement, but `check_session_runtime_access` currently reduces every foreign runtime to the cluster-membership path. The browser compounds a rejected attach by setting and persisting `sessionId` before the server confirms it, while Overview treats a missing effective policy as enabled.

The authorization model must preserve the shared-cluster trust boundary: a member-owned project hosted on another person's OS account cannot let its owner extend that machine access to arbitrary project invitees. APAS also supports a canonical project with instances in more than one cluster, so runtime authority must be resolved per selected session rather than once per project.

## Goals / Non-Goals

**Goals:**

- Express project access and cluster machine access as independent, composable grants.
- Restore project-scoped live collaboration for a project hosted by its owner.
- Keep third-party hosting permission revocable and machine-specific.
- Correlate web attachment success or failure to the exact requested session before navigation commits.
- Prevent cached content and policy from crossing an unconfirmed or rejected navigation boundary.

**Non-Goals:**

- Grant project users general shell, machine inventory, daemon, provider, provisioning, or cluster-policy access.
- Let a project owner invite users onto a third-party host without that host's cluster authorization.
- Merge duplicate APAS accounts or infer identity equivalence from similar email addresses.
- Change project invitation acceptance or cluster direct-add lifecycle in this change.

## Decisions

### 1. Resolve runtime access from the selected session's host and canonical owner

After confirming ordinary project content access, runtime authorization will use this ordered matrix:

1. The session host may operate its own runtime.
2. If the session host is also the canonical project owner, an explicit project owner/user role may operate that project runtime. This is the project-scoped share.
3. Otherwise the runtime is third-party-hosted and the caller must have an active cluster membership that permits the session's exact machine. This is the cluster-scoped share.

The check remains server-authoritative and is reevaluated for every attach and runtime mutation. It uses canonical project identity and the selected session's host, so the same user may be authorized for an owner-hosted instance and denied for another instance of the same project on a third-party cluster.

Alternative considered: make every project membership sufficient for every placement. Rejected because an owner of a member-provisioned project could then invite arbitrary accounts to execute code as the hosting cluster owner's OS user, bypassing cluster membership and machine allowlists.

Alternative considered: automatically create cluster membership when a project is shared. Rejected because that silently widens a one-project grant into machine discovery and new-project provisioning, defeating the distinction this change establishes.

### 2. Return a session-correlated attachment rejection

The server-to-web protocol will add a typed attachment-rejection message carrying the requested `session_id` and a safe reason such as missing project access, missing host-machine access, unavailable project, or missing session. The attach handler will use it instead of a context-free global error for expected attachment failures.

This permits multiple background subscriptions and rapid project selections without guessing which request failed. Existing generic errors remain for unrelated operations. The new message is additive; older clients safely ignore it, and the server still enforces authorization.

Alternative considered: clear pending navigation on any generic `Error`. Rejected because background cached-session attachment and unrelated control failures share that channel and could cancel the wrong foreground navigation.

### 3. Separate requested navigation from confirmed workspace state

The web store will add a pending attachment record rather than immediately replacing `sessionId`. `attachSession` will send the request and retain enough intent for cache/catch-up and pane selection, but it will not persist the target or expose its cached workspace. A matching successful attachment confirmation commits the existing cache-switch transaction and then requests catch-up. A matching rejection clears the pending record and retains the last confirmed workspace; if none exists, the project list/no-project state remains visible.

Reconnect and background cache subscriptions remain multi-attach subscriptions but never become navigation unless they match the foreground pending request. Stale confirmations and rejections are ignored for navigation.

### 4. Treat effective policy absence as unknown, not enabled

Overview visibility will require `projectPolicies[sessionId]?.teamAvailable === true`. A confirmed session with no authoritative policy yet may show real panes and a neutral loading/empty state, but not Overview. Existing cached policy can be reused only for the same confirmed session. The first-pane control remains available under its independent launch-policy rules when Overview is disabled.

### 5. Explain scope at the point of sharing

The Project Access modal will state that it shares only the named project and grants live use on owner-hosted instances; it will warn that third-party-hosted instances also require that host's cluster permission. The Machines page will label cluster sharing as machine access, retain the explicit machine/default-agent controls, and state that it does not reveal unrelated projects.

## Risks / Trade-offs

- **[Existing callers relied on optimistic project switching]** → Preserve instant switching after confirmation by restoring the existing per-session cache at commit time; display a small pending state rather than an unauthorized workspace.
- **[Mixed web/server versions do not share the new rejection event]** → Keep the protocol addition backward-compatible and deploy server before web. Authorization remains fail-closed; an old server with a new web can time out pending navigation without exposing the target.
- **[Owner transfer changes whether project membership grants scoped runtime]** → Resolve the canonical owner on every runtime check rather than persisting a derived grant. The behavior follows current ownership immediately.
- **[A project has several placements]** → Authorize per session host and machine, never from a project-wide cached decision.
- **[Policy arrives after pane data]** → Keep Overview hidden until policy is authoritative; pane rendering and first-pane creation remain independently policy-guarded.

## Migration Plan

1. Add database authorization tests and implement the runtime-access matrix without schema changes.
2. Add the correlated rejection protocol and server attach behavior, regenerate protocol artifacts, and verify older web message handling remains tolerant.
3. Add pending attachment state, success/rejection reconciliation, fail-closed Overview visibility, and sharing copy with focused web tests.
4. Run full Rust and web verification, then deploy server before web.
5. Verify an existing owner-hosted project share (Mako/Zeyu), a cluster-only member, and a third-party-hosted project in production.

Rollback restores the previous binaries and web build; no database rollback is required. The prior behavior will again require both grants for foreign runtime attachment, but no access will be broadened beyond persisted membership during rollback.
