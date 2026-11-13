# cluster-user-administration Specification

## Purpose

Defines the virtual cluster each account operates — the machines it registered and the projects hosted on them — including how cluster identity is established, what an operator may administer without project membership, and how those control-plane actions are audited. Deployment-wide administration lives in `system-administration`.
## Requirements
### Requirement: Cluster identity is server-authoritative
Every authenticated account SHALL have exactly one account state (`active` or `suspended`) stored by the server, and authorization SHALL use the stored state rather than hard-coded emails or user IDs. Identity responses SHALL include that state. No account attribute SHALL confer deployment-wide authority: an account's authority SHALL be limited to its own virtual cluster, and deployment-wide authority SHALL exist only behind the separate system-administrator credential.

#### Scenario: Active account authenticates
- **WHEN** an active account completes authentication
- **THEN** the system returns its active state with its identity
- **AND** server authorization uses that persisted state for subsequent requests

#### Scenario: Hard-coded identity has no implicit authority
- **WHEN** an account has a historically privileged email or user ID
- **THEN** the system grants it no authority beyond its own virtual cluster

#### Scenario: Suspended account attempts access
- **WHEN** a suspended account attempts to authenticate, connect a client, or use an existing credential
- **THEN** the system denies the operation and grants no project or cluster access

### Requirement: Every account operates one virtual cluster
Every account SHALL own exactly one virtual cluster consisting of the machines whose clients registered under that account and the projects durably placed in it. An active account MAY also be a member of several other account-owned clusters. Project hosting SHALL be represented independently from project ownership, project membership, daemon identity, and session identity. Owning or belonging to a project SHALL NOT by itself create a hosting placement, and hosting a project SHALL NOT create project membership. A project MAY be owned by one account while hosted in another account's virtual cluster and MAY have placements in more than one virtual cluster.

#### Scenario: Project owned elsewhere is hosted in my cluster
- **WHEN** a cluster member creates a project on a machine in my cluster
- **THEN** the project is durably placed in my virtual cluster
- **AND** the member remains its project owner

#### Scenario: Foreign project is not in my cluster
- **WHEN** a project has no durable placement in my virtual cluster
- **THEN** the project is absent from my cluster listings even if I belong to it

#### Scenario: Cluster inclusion is not membership
- **WHEN** I administer a project hosted in my cluster that I do not belong to
- **THEN** the system does not add me to the project's membership

#### Scenario: Account belongs to another cluster
- **WHEN** a cluster owner adds an account to their cluster
- **THEN** it retains its own virtual cluster
- **AND** gains only the member capabilities of the shared cluster

### Requirement: Cluster operators administer their own cluster from the machines surface
The machines surface SHALL be available to every active account. For a cluster owner, it SHALL present that account's machines, every durably placed hosted project with owner, member count, connection state, lifecycle state, effective policy and usage, cluster members with their machine access and default launch profile, cluster default policy, and cluster audit history. For a cluster member, it SHALL present the shared machines its membership permits as provisioning choices and only projects and usage otherwise available through project access. Owner-only controls and unrelated projects SHALL be absent from member responses and interfaces. No cluster surface SHALL require deployment-wide authority.

#### Scenario: Ordinary account opens the machines surface
- **WHEN** an active account opens the machines surface for its own cluster
- **THEN** the system presents its machines, membership administration, and all projects durably placed in the cluster

#### Scenario: Member opens a shared cluster
- **WHEN** an active cluster member opens a cluster shared with them
- **THEN** the system presents eligible shared machines and the member's accessible hosted projects
- **AND** omits owner-only controls and unrelated hosted projects

#### Scenario: Another cluster's project is requested
- **WHEN** an account requests cluster-administration details for a project outside a cluster it owns
- **THEN** the system denies the request

#### Scenario: Suspended account opens the machines surface
- **WHEN** a suspended account requests any owned or shared cluster inventory
- **THEN** the system denies the request

### Requirement: Cluster operators control the lifecycle of projects in their cluster
Every project SHALL have an active or suspended administrative state. The operator of the virtual cluster hosting a project SHALL be able to suspend or reactivate that project and stop its active runtime without being a project member or owner. Suspension SHALL preserve project data and membership while denying new member attachments, client registrations, and pane launches for that project.

#### Scenario: Operator suspends a hosted project
- **WHEN** a cluster operator suspends a running project hosted in their cluster
- **THEN** the system stops or disconnects that project's active runtime
- **AND** denies further project operations until it is reactivated
- **AND** preserves the project's data and membership

#### Scenario: Operator reactivates a hosted project
- **WHEN** a cluster operator reactivates a suspended project in their cluster
- **THEN** authorized project members can reconnect and resume normal project operations

#### Scenario: Owner outside the hosting cluster attempts lifecycle control
- **WHEN** a project owner whose account does not host the project attempts to change its administrative lifecycle state
- **THEN** the system denies the operation

### Requirement: Cluster control-plane actions are auditable within their cluster
The system SHALL record successful account-state, project-lifecycle, project-owner, project-membership, and project-policy changes with the acting identity, target, action, and timestamp, and SHALL attribute each record to the virtual cluster it affected. A cluster operator SHALL be able to review the audit records of their own cluster and SHALL NOT see records of other clusters. A rejected mutation SHALL NOT be recorded as a successful administrative action.

#### Scenario: Operator changes a hosted project's policy
- **WHEN** a cluster operator changes the policy of a project hosted in their cluster
- **THEN** the system records an audit event identifying the operator, project, changed policy, cluster, and time

#### Scenario: Operator reviews cluster audit history
- **WHEN** a cluster operator opens their cluster audit history
- **THEN** the system returns only records attributed to their own virtual cluster

#### Scenario: Unauthorized mutation is rejected
- **WHEN** an account attempts a control-plane mutation outside its own virtual cluster
- **THEN** the system rejects the mutation
- **AND** does not record it as a successful administrative action

### Requirement: Existing installations receive a safe cluster-role migration
On upgrade, every existing account SHALL become an active ordinary account whose authority is limited to its own virtual cluster, including accounts that previously held the cluster-administrator role. Deployment-wide authority SHALL move to the separate system-administrator credential, and migration SHALL NOT infer that credential or any authority from an account's email, project role, or previous cluster role.

#### Scenario: Existing cluster administrator is migrated
- **WHEN** a deployment containing accounts with the previous cluster-administrator role is upgraded
- **THEN** those accounts keep their projects, memberships, and machines
- **AND** hold authority only over their own virtual cluster

#### Scenario: Existing project administrator is migrated
- **WHEN** an existing account has a project-admin membership
- **THEN** the account is migrated as an ordinary account with no deployment-wide authority

#### Scenario: Existing system administrator is migrated
- **WHEN** the deployment containing a previously configured administrator email is upgraded
- **THEN** that account receives no deployment-wide authority
- **AND** deployment-wide operations require the separate system-administrator credential

