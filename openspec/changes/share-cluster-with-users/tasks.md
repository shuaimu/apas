## 1. Persistence and Migration

- [x] 1.1 Add cluster membership and hashed shared-cluster invitation schema, models, lifecycle operations, indexes, and database tests without changing the existing account-registration invitation table.
- [x] 1.2 Add durable project-cluster placements and transactionally backfill the exact union of existing project owners and canonical-session users.
- [x] 1.3 Convert cluster inventory, host authorization, audit attribution, and project host lookup queries to placements, with migration parity and multi-placement tests.
- [x] 1.4 Convert effective-policy cluster inputs to placements and test monotone intersection, placement addition, and compatibility of migrated projects.
- [x] 1.5 Add idempotent project provisioning request persistence with requester, cluster, machine, normalized repository, server project ID, state transitions, and tests.
- [x] 1.6 Add reported-cost coverage to usage persistence and cluster/project usage aggregations for lifetime, seven-day, and UTC-today windows.

## 2. Cluster Membership and Control APIs

- [x] 2.1 Add centralized authorization helpers that independently resolve active account, owned/active-member cluster access, daemon ownership, project placement, and project role.
- [x] 2.2 Add owner-only APIs to create, list, resend/replace, and revoke shared-cluster invitations and to list/revoke active members, including audit events and trust-warning confirmation.
- [x] 2.3 Add authenticated invitation inspection and acceptance with addressee, expiry, single-use, suspension, and duplicate-membership enforcement.
- [x] 2.4 Add owned/shared cluster discovery and explicit cluster-context endpoints while retaining `/cluster/*` compatibility aliases for the caller's owned cluster.
- [x] 2.5 Update hosted-project, membership, ownership, lifecycle, policy, and audit APIs so only the hosting cluster owner receives administrative authority.
- [x] 2.6 Add cluster-owner usage endpoints and project-scoped member usage endpoints with pagination, optional cost coverage, and authorization tests.

## 3. Shared Protocol and Daemon Safety

- [x] 3.1 Extend shared machine and provisioning protocol types with cluster access scope, daemon capability negotiation, provisioning ID, server project ID, clone mode, and discard acknowledgement while preserving old owner flows.
- [x] 3.2 Implement strict canonical public-GitHub URL parsing and safe error scrubbing with unit tests for credentials, alternate schemes/ports, ambiguous hosts, fragments, queries, and malformed repositories.
- [x] 3.3 Implement daemon-enforced shared clone isolation that ignores sibling origins and member base paths, disables Git credential/askpass/SSH inputs, and remains inside the managed projects root.
- [x] 3.4 Implement marker-bound local provisioning receipts so retry returns the same project/checkout and cleanup can remove only a matching unregistered shared checkout.
- [x] 3.5 Split clone/register from runtime start so a shared project cannot register a session until the server has finalized requester ownership and placement.

## 4. Server WebSocket Orchestration and Revocation

- [x] 4.1 Implement the two-phase create-instance state machine, including preflight authorization, persisted request, daemon retry, final authorization check, atomic project/placement creation, start, requester-routed result, and safe cancellation.
- [x] 4.2 Build separate owner and member machine projections that redact secrets and unrelated projects, and use the same filtering for explicit refresh and heartbeat broadcasts.
- [x] 4.3 Permit members to start, stop, attach, and mutate only accessible projects on matching placements while keeping reboot and provider configuration owner-only.
- [x] 4.4 Enforce current persisted membership on every shared runtime mutation and provisioning finalization so revocation works on existing WebSocket connections.
- [x] 4.5 Add integration tests for forged cluster/machine/project combinations, revoked in-flight requests, old-daemon refusal, requester acknowledgement routing, and absence of secret fields.

## 5. Desktop Web Experience

- [x] 5.1 Add typed owned/shared cluster state, selection, API clients, and redacted shared-machine handling to the web store without persisting authority claims client-side.
- [x] 5.2 Add owner membership/invitation administration to the cluster page with pending/active states, copyable invitation links, explicit trust confirmation, revocation, and responsive behavior.
- [x] 5.3 Add the owner usage overview with time-window selection, totals, project/project-owner breakdown, unavailable-cost presentation, and links to hosted-project management.
- [x] 5.4 Add a shared-cluster view that shows owner identity, trust notice, eligible machines, accessible projects and project usage while omitting owner-only controls.
- [x] 5.5 Extend project creation to group owned/shared machine targets, enforce and explain public-GitHub-only input for shared targets, display pending state, and surface safe clone errors.
- [x] 5.6 Add invitation acceptance across login redirects and component/store tests for role-specific rendering, revocation refresh, and secret redaction.

## 6. Mobile Experience

- [x] 6.1 Add owned/shared cluster selection and redacted shared-machine summaries to the mobile Machines view.
- [x] 6.2 Add mobile invitation acceptance, trust confirmation, shared public-GitHub project creation, pending/error states, and accessible-project usage drill-down.
- [x] 6.3 Add mobile interaction tests verifying member navigation retains cluster context and never exposes owner-only controls or unrelated projects.

## 7. Verification and Rollout

- [x] 7.1 Add end-to-end authorization tests covering owner, active member, revoked member, unrelated account, suspended account, project owner/member, multi-cluster placement, and system administrator.
- [x] 7.2 Run Rust formatting, targeted and full Rust tests, web unit tests, lint/type checks, npm and pnpm frozen-lockfile audits, and production builds; fix every regression introduced by the change.
- [x] 7.3 Update operator/user documentation with the trusted-code warning, account-provisioning prerequisite, public-repository limit, role matrix, revocation semantics, usage limitations, rollout order, and rollback restriction.
- [x] 7.4 Validate the OpenSpec change strictly and conduct a clean-tree migration smoke test that compares pre/post-upgrade inventory and effective policy before enabling shared provisioning.
