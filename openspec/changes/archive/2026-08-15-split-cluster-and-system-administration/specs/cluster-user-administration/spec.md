## MODIFIED Requirements

### Requirement: Cluster identity is server-authoritative
Every authenticated account SHALL have exactly one account state (`active` or `suspended`) stored by the server, and authorization SHALL use the stored state rather than hard-coded emails or user IDs. Identity responses SHALL include that state. No account attribute SHALL confer deployment-wide authority: an account's authority SHALL be limited to its own virtual cluster, and deployment-wide authority SHALL exist only behind the separate system-administrator credential.

#### Scenario: Active cluster user authenticates
- **WHEN** an active account completes authentication
- **THEN** the system returns its active state with its identity
- **AND** server authorization uses that persisted state for subsequent requests

#### Scenario: Hard-coded identity has no implicit authority
- **WHEN** an account has a historically privileged email or user ID
- **THEN** the system grants it no authority beyond its own virtual cluster

#### Scenario: Suspended account attempts access
- **WHEN** a suspended account attempts to authenticate, connect a client, or use an existing credential
- **THEN** the system denies the operation and grants no project or cluster access

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

## ADDED Requirements

### Requirement: Every account operates one virtual cluster
Every account SHALL operate exactly one virtual cluster, consisting of the machines whose client registered under that account together with the projects hosted in it. A project SHALL be hosted in an account's virtual cluster when the account owns it or when at least one of its sessions was created under that account. Belonging to a project SHALL NOT place that project in the member's own cluster, and hosting a project SHALL NOT create or imply project membership. A project MAY be owned by one account while being hosted in another account's virtual cluster, and MAY be hosted in more than one virtual cluster.

#### Scenario: Project owned elsewhere is hosted in my cluster
- **WHEN** a project owned by another account has a session created under my account
- **THEN** the project appears in my virtual cluster
- **AND** its ownership and member list are unchanged

#### Scenario: Foreign project is not in my cluster
- **WHEN** a project has no session created under my account and I do not own it
- **THEN** the project is absent from my virtual cluster and from my cluster listings
- **AND** belonging to that project does not place it in my cluster

#### Scenario: Cluster inclusion is not membership
- **WHEN** I administer a project hosted in my cluster that I do not belong to
- **THEN** the system does not add me to the project's membership

### Requirement: Cluster operators administer their own cluster from the machines surface
The machines surface SHALL be available to every active account and SHALL present that account's own virtual cluster: its machines, its hosted projects with owner, member count, connection state, lifecycle state, and effective policy, its cluster default policy, and its cluster audit history. The surface SHALL NOT require any deployment-wide authority, and SHALL NOT expose any other account's cluster.

#### Scenario: Ordinary account opens the machines surface
- **WHEN** an active account with no deployment-wide authority opens the machines surface
- **THEN** the system presents its machines and the projects hosted in its virtual cluster

#### Scenario: Another cluster's project is requested
- **WHEN** an account requests cluster details for a project outside its own virtual cluster
- **THEN** the system denies the request

#### Scenario: Suspended account opens the machines surface
- **WHEN** a suspended account requests its cluster inventory
- **THEN** the system denies the request

### Requirement: Cluster operators control the lifecycle of projects in their cluster
Every project SHALL have an active or suspended administrative state. The operator of the virtual cluster hosting a project SHALL be able to suspend or reactivate that project and stop its active runtime without being a project member or owner. Suspension SHALL preserve project data and membership while denying new member attachments, client registrations, and pane or team launches for that project.

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

## REMOVED Requirements

### Requirement: Cluster administrators manage membership lifecycle

**Reason**: Account lifecycle is deployment-wide authority, not cluster authority. It moves to the `system-administration` capability, where exactly one credential — not an account role — can list, invite, activate, and suspend accounts. The cluster-administrator role that this requirement depended on no longer exists, so the "last active cluster administrator" protection it defined has nothing to protect.

**Migration**: Account listing, invitation, activation, and suspension are now reachable only through the system-administration surface and its separate login. Deployments that relied on promoting an account to cluster administrator to manage accounts must use the configured system-administrator credential instead.

### Requirement: Cluster administrators have a complete project inventory

**Reason**: A complete deployment-wide inventory is system-administration authority. The replacement requirement "Cluster operators administer their own cluster from the machines surface" gives every account an inventory scoped to its own virtual cluster, and the `system-administration` capability keeps the deployment-wide inventory behind the separate credential.

**Migration**: Accounts that previously held the cluster-administrator role see the projects hosted in their own cluster on the machines surface. The deployment-wide project inventory moves to the system-administration surface.

### Requirement: Cluster administrators control project lifecycle

**Reason**: Lifecycle authority is re-anchored on the virtual cluster that hosts a project rather than on a deployment-wide account role. It is replaced by "Cluster operators control the lifecycle of projects in their cluster" here, and by the system administrator's deployment-wide lifecycle authority in `system-administration`.

**Migration**: None for project data. The account that hosts a project — the account under which its sessions run — controls its lifecycle; the system administrator retains the same control over every project.

### Requirement: Cluster control-plane actions are auditable

**Reason**: Replaced by "Cluster control-plane actions are auditable within their cluster", which adds the cluster attribution needed to show an operator their own records without exposing other clusters, and by the system administrator's deployment-wide audit review.

**Migration**: Existing audit records are retained. Records that predate cluster attribution remain visible to the system administrator and are attributed to a cluster where their project is known.
