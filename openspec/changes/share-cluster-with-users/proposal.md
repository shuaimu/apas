## Why

APAS currently treats every account as the sole operator of an implicit, account-owned cluster, so another user cannot safely use the owner's machines to create a project and the owner cannot durably attribute or govern that user's hosted work. Cluster sharing needs an explicit membership boundary that preserves project ownership, protects machine credentials, and gives the cluster owner complete policy and usage oversight.

## What Changes

- Let a cluster owner invite existing active APAS accounts to join their cluster, list pending and active members, and revoke access. Cluster invitations remain separate from deployment account-registration invitations.
- Let an active cluster member discover eligible shared machines and create a new project there from a public GitHub HTTPS repository. The creating member owns the project while the selected cluster remains its host.
- Isolate shared clone operations from the cluster owner's Git/SSH credentials, reject embedded credentials and unsupported repository hosts, constrain destinations to the daemon-managed projects root, and warn both parties that cluster membership permits user-controlled code to run on the owner's machines.
- Persist hosting-cluster placement independently from project ownership and session identity, including a migration for existing projects that preserves today's multi-cluster hosting and effective-policy behavior.
- Give the cluster owner an inventory of every project hosted in the cluster, including member-owned projects, with existing lifecycle, ownership, membership, and per-project policy controls.
- Add cluster usage reporting for lifetime, trailing-seven-day, and today windows, with totals and project/project-owner breakdowns. Members see usage for projects they can access; only the cluster owner sees the whole cluster. These are observed token/cost counters, not billing-grade attribution or hard spending quotas.
- Restrict shared members to project-scoped runtime and provisioning actions. Machine reboot, provider credential configuration, cluster membership, cluster default policy, cluster-wide audit, and cluster-wide usage remain owner-only.
- On membership revocation, immediately prevent new provisioning and runtime mutations on that cluster. Existing member-owned projects and their data remain placed in the cluster and readable through normal project ownership until the cluster owner suspends, transfers, or deletes them.

## Capabilities

### New Capabilities

- `shared-cluster-membership`: Cluster invitations, acceptance, member discovery, owner-only administration, revocation, and security disclosures.
- `shared-cluster-project-provisioning`: Placement-aware shared-machine discovery and credential-isolated GitHub project creation.
- `cluster-usage-reporting`: Owner and member views of observed usage at cluster and project scope.

### Modified Capabilities

- `cluster-user-administration`: Replace sole-account operator assumptions with owner/member cluster views and durable hosted-project administration.
- `project-access-control`: Separate project ownership/content access from permission to consume a hosting cluster's machines.
- `project-policy-governance`: Resolve cluster defaults through durable hosting placements and keep cluster/project policy mutation owner-only.

## Impact

- Server database migrations and queries for cluster membership, invitations, project placements, provisioning provenance, audit scope, and usage aggregation.
- HTTP and WebSocket authorization for machine discovery/control, create-instance requests and acknowledgements, project registration, and runtime mutations.
- Shared Rust protocol types and daemon clone handling, including a public-only credential-isolation mode and constrained destination behavior.
- Web cluster administration and project-creation interfaces on desktop and mobile, with owner/member-specific controls and usage views.
- Authorization, migration, protocol, daemon security, API, and UI tests. No new third-party service is required; private GitHub repository authentication and enforceable spend quotas are deferred.
