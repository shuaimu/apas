## Context

See `proposal.md` for motivation and the delta specs for observable behavior. Today a cluster has no row of its own: the authenticated account ID is both cluster identity and daemon owner. Cluster inventory and effective policy are inferred from project ownership plus `sessions.user_id`, while a daemon clone starts a CLI with the daemon owner's credential. Consequently, a shared-user clone would be attributed to the wrong project owner, its result would be sent to the wrong browser, and Git could reuse machine-owner credentials.

The existing system intentionally allows one canonical project to run in several account-owned virtual clusters and intersects their policy defaults. That behavior must survive migration. The daemon and project runtime execute as the machine owner's operating-system user; this change is a trusted-collaborator feature, not a tenant sandbox.

## Goals / Non-Goals

**Goals:**

- Make cluster membership explicit, accepted, revocable, and independent of project access.
- Preserve one owned cluster per account while allowing membership in multiple shared clusters.
- Make hosting placement durable and independent of project, session, and daemon ownership.
- Attribute shared provisioning to the requesting member and make retries safe.
- Keep shared callers away from owner-only machine data and control-plane operations.
- Give cluster owners useful project-level usage visibility using existing APAS counters.

**Non-Goals:**

- Sandboxing mutually untrusted users from the owner OS account. Invited members can direct agents that execute on the owner's machine and therefore must be trusted accordingly.
- Creating APAS accounts from a cluster invitation; deployment registration remains system-admin controlled.
- Private GitHub repository authentication, arbitrary Git hosts, member-supplied credentials, or owner-credential delegation.
- Billing-grade charge attribution, budgets, rate limits, reservations, or hard usage caps.
- Moving a checkout between clusters or deleting a checkout automatically when membership is revoked.

## Decisions

### 1. Keep account IDs as owned-cluster IDs

Each active account continues to own one implicit cluster whose stable ID is its user ID. Add:

- `cluster_memberships(cluster_owner_user_id, user_id, status, invited_at, accepted_at, revoked_at, updated_at)` with a unique owner/member pair;
- `shared_cluster_invitations(id, token_hash, cluster_owner_user_id, invitee_user_id, email, expires_at, accepted_at, revoked_at, created_at)`; and
- indexes by owner, member/status, and invitation token hash.

The owner is implicit and cannot be removed or represented as a membership row. Retaining the existing cluster ID avoids a new top-level cluster migration and keeps `cluster_default_policies.user_id` valid. A stored membership status retains revocation history and supports audited re-invitation without making a stale token reusable.

Alternative considered: introduce a general `clusters` table and make accounts choose a primary cluster. That is more flexible for organizations with several clusters, but it adds naming, ownership transfer, billing, and selector semantics not required by this feature.

Invitation tokens are returned only at creation, stored as hashes, bound to a known active invitee, and accepted only by that authenticated user. The existing `cluster_invitations` table is actually for system-admin account registration and remains unchanged to avoid combining authorities.

### 2. Make project placement the authoritative hosting relation

Add `project_cluster_placements(project_id, cluster_owner_user_id, created_by_user_id, source, created_at)` with a composite primary key. All cluster inventory, host-operator authorization, audit attribution, and effective-policy cluster inputs use this table after migration.

Migration backfills the union of:

- each existing project's owner account, preserving the old “ownership implies my cluster” listing; and
- each distinct `sessions.user_id` for that canonical project, preserving every inferred hosting cluster and the old multi-cluster policy intersection.

After migration, ownership alone does not create a new placement. A project created on an account's own daemon receives that account's placement; a project created by a shared member receives the selected machine owner's placement. Session creation never silently adds a placement: an authorized start must reference an existing placement. This eliminates accidental authority changes caused by session history.

Alternative considered: add one `host_cluster_user_id` column to `projects`. It cannot represent today's legitimate multi-cluster projects and would broaden policy on migration.

### 3. Use an authenticated two-phase provisioning state machine

Add a server-side `project_provisioning_requests` record keyed by the client request ID, containing requester, target cluster, machine, normalized GitHub URL, desired name/branch, server-generated project ID, status, error, and timestamps. A unique requester/request-key pair makes retries return the existing result.

Provisioning proceeds as follows:

1. The server verifies the active requester is either the target cluster owner or an active member, the target machine belongs to that cluster, the daemon is compatible, and the repository input/mode is allowed.
2. It persists a pending request and a server-generated project ID before sending work to the daemon.
3. The daemon clones and registers local `.apas` metadata using that project ID but does not start the runtime. It records a local provisioning marker/receipt so a retransmitted request returns the same checkout instead of cloning again.
4. On success, the server rechecks current membership and transactionally creates the canonical project owned by the requester, inserts its placement, marks the request complete, and computes effective policy.
5. Only then does the server ask the daemon to start the project and deliver the result to the requesting user's clients. A start failure leaves a valid stopped project, matching existing behavior.
6. If authorization disappeared before finalization, the server marks the request cancelled and asks the daemon to discard only the checkout whose provisioning marker and project ID match. It never accepts a caller-supplied deletion path.

Owner-originated clone behavior may retain the existing trusted clone mode, including private origins already available to that owner. Member-originated provisioning always carries an explicit `public_github` mode through shared protocol types. The daemon must enforce that mode even if an older or compromised web client bypasses server validation.

This ordering avoids the current race where auto-start registers a session under the daemon owner before the server knows the true requester. It also routes acknowledgements by provisioning requester rather than daemon owner.

### 4. Enforce public-only, credential-isolated shared cloning at both boundaries

The normalized member URL is restricted to `https://github.com/<owner>/<repo>[.git]` without username, password, query, fragment, alternate port, Unicode-host ambiguity, or local/file syntax. The daemon reconstructs the canonical URL instead of trusting the raw string. Shared mode does not consult sibling checkout origins and ignores member `base_path`; it uses the daemon-managed projects root and existing single-component name sanitization.

The shared clone subprocess disables terminal prompts, askpass, configured credential helpers, and SSH command/auth inputs, and receives a minimal Git-relevant environment. Errors are scrubbed before returning to the requester. A failed pre-registration operation removes only the newly allocated, marker-bound destination.

This prevents accidental use of owner Git credentials during clone. It does not sandbox the eventual project runtime from the owner's OS account; the mandatory invitation/acceptance warning communicates that trust boundary.

Alternative considered: accept a member personal access token. Secure secret storage, revocation, log scrubbing, subprocess delivery, and GitHub account connection UX are a separate feature and are deliberately deferred.

### 5. Centralize owner, member, project, and machine authorization

Server authorization resolves four independent facts on every relevant request:

- caller account is active;
- caller owns the target cluster or has an active membership;
- target machine's authenticated daemon owner matches the target cluster;
- target project is placed in that cluster and caller has the required project access, unless this is a new provisioning request.

Owner-only capabilities require `caller == cluster_owner_user_id`: full inventory, invitations/members, audit, cluster usage, cluster default and hosted-project policy mutation, hosted-project lifecycle/access administration, daemon reboot, and provider configuration. Member capabilities additionally require project owner/member access: provisioning, start/stop, attach, panes, prompts, and other ordinary runtime work.

WebSocket handlers query persisted membership rather than caching authorization for a connection lifetime, so revocation takes effect on the next command. The session manager exposes daemon ownership as an input to authorization but does not decide membership itself.

Machine messages become audience-specific DTOs. Owners retain the full management view; members receive cluster identity, safe machine metadata, compatibility/readiness, and only accessible projects. Provider API keys, credential values, exact subscription quota, unrelated paths/projects, reboot, and provider configuration are never serialized to member clients. Push broadcasts use the same projection as explicit refresh to prevent data from reappearing on heartbeat.

### 6. Preserve compatible APIs while adding explicit cluster context

Existing `/cluster/*` owner-administration routes remain aliases for the caller's owned cluster. New discovery returns the owned cluster plus accepted shared clusters, and explicit `/clusters/{cluster_owner_user_id}/*` read/provisioning routes carry cluster context. Every server handler resolves the path ID rather than trusting owner/member flags from the client.

Invitation acceptance uses a token-scoped route outside an owner-only cluster path so an authenticated invitee can accept without already being a member. Mutations return generic not-found/forbidden responses when revealing the target would cross an authorization boundary.

Shared Rust protocol messages add optional fields or new variants for access scope, cluster owner, provisioning ID/mode, and discard acknowledgement. Compatibility checks hide shared provisioning on old daemons; existing owner-only flows continue working during a rolling update.

### 7. Compute policy from placements

`cluster_default_policies` remains keyed by cluster owner user ID. Effective-policy queries replace the owner/session union with all rows in `project_cluster_placements`, then monotonically intersect launch profiles across deployment default, every placement's cluster default, and the project override. The existing `team_available` default semantics remain unchanged: the lowest applicable explicitly stated project/cluster value wins, so a deployment default of `false` is not converted into a new prohibition. Placement addition/removal and default/override changes recompute and broadcast policy deterministically.

A single project override remains shared across placements because it is a project property. Any hosting owner can narrow it, but validation and effective evaluation prevent widening past another placement. All successful changes retain cluster-attributed audit records.

### 8. Add scoped usage queries without changing the event pipeline

Existing per-session, per-pane, per-UTC-day counters remain the source of truth. Add database aggregations that join sessions to canonical projects and placements and return:

- cluster totals plus project and project-owner groups for a cluster owner; and
- one project or the caller's accessible projects for ordinary project members.

Grouping by current project owner is an administrative organization aid, not per-human causal attribution. A project placed in two clusters appears in each hosting owner's view, but its underlying counters remain one set and deployment-wide aggregation must query counters rather than summing cluster views.

Add an explicit reported-cost marker when ingesting new usage so an unavailable cost can be distinguished from a genuine zero. Historical zero costs remain marked unknown unless an existing event proves a cost was reported. API types carry optional cost totals and coverage metadata.

Provider usage-limit snapshots and credentials are machine-owner data and are not part of the member usage API. This change adds no enforcement loop; owners use policy, lifecycle, and revocation controls.

### 9. Present owned and shared clusters as distinct web contexts

The desktop machines page gains an owned/shared cluster selector. The owned view adds invitation/member administration, the existing complete machine controls, hosted-project controls, audit, and cluster usage. A shared view shows the owner identity, trust notice, eligible machines, create-project action, and only the caller's accessible projects/project usage. The create modal groups targets by cluster and clearly labels public-GitHub-only shared targets.

The mobile Machines view uses the same contexts and authorization-derived DTOs, with compact invitation acceptance and project creation. Owner-only controls are omitted rather than merely disabled for members. Invitation links survive login by retaining the token until the addressed account authenticates.

## Risks / Trade-offs

- **[Invited users can execute code as the owner's OS user]** → Require explicit warnings on both sides, keep membership owner-controlled and revocable, and describe the feature as trusted collaboration. Strong tenant isolation requires a future container/VM execution architecture.
- **[Clone isolation may be mistaken for runtime isolation]** → UI and documentation state that it prevents accidental Git credential use only; it does not sandbox agents after launch.
- **[Revocation races an in-flight request]** → Recheck persisted membership before forwarding every mutation and again before provisioning finalization; use marker-bound cleanup for cancelled clones.
- **[Old daemons do not understand safe shared provisioning]** → Advertise shared eligibility only after a protocol/version capability handshake; never downgrade a member request to the trusted owner clone mode.
- **[Placement backfill changes a foundational query]** → Backfill the exact old owner/session union, compare old and new inventory/policy results in migration tests, and switch reads only after the transaction succeeds.
- **[Project-owner usage grouping is imperfect]** → Label it organizational attribution and retain project-level drill-down rather than claiming per-person billing.
- **[Owner APIs accidentally serialize secrets to members]** → Use separate audience DTO construction and response-shape tests; do not rely on client-side hiding.
- **[Large cluster aggregation becomes expensive]** → Index placement/project/day joins, paginate project rows, bound default date windows, and aggregate in SQL. Introduce rollups only if measurements require them.

## Migration Plan

1. Deploy additive schema migrations for memberships, hashed invitations, placements, provisioning requests, and usage cost coverage.
2. In the same migration transaction, backfill placements from existing project owners and distinct session users. Assert that every project has at least one placement and that old/new effective-policy inputs match.
3. Deploy server support for placement-backed reads, owner-compatible routes, new authorization helpers, usage APIs, and protocol negotiation while the sharing UI remains feature-disabled.
4. Deploy compatible daemon/CLI support for two-phase IDs, local provisioning receipts, isolated public-GitHub mode, and marker-bound discard.
5. Deploy the web UI, then enable cluster invitations and shared provisioning only for daemons advertising the required capability.
6. Verify owner inventories, policy intersections, secret-redacted member responses, revocation, clone failure cleanup, usage totals, and audit records before general enablement.

Schema rollback is additive, but rolling back application binaries after shared projects exist is not authorization-safe because old code infers cluster scope from owner/session identity. Before binary rollback, disable invitations and shared provisioning, revoke or temporarily suspend shared compute access, and keep the new placement/membership tables intact for the corrected redeployment.
