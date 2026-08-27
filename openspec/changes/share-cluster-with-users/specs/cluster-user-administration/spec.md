## MODIFIED Requirements

### Requirement: Every account operates one virtual cluster
Every account SHALL own exactly one virtual cluster consisting of the machines whose clients registered under that account and the projects durably placed in it. An active account MAY also be an invited member of multiple other account-owned clusters. Project hosting SHALL be represented independently from project ownership, project membership, daemon identity, and session identity. Owning or belonging to a project SHALL NOT by itself create a hosting placement, and hosting a project SHALL NOT create project membership. A project MAY be owned by one account while hosted in another account's virtual cluster and MAY have placements in more than one virtual cluster.

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
- **WHEN** an account accepts a cluster invitation
- **THEN** it retains its own virtual cluster
- **AND** gains only the member capabilities of the shared cluster

### Requirement: Cluster operators administer their own cluster from the machines surface
The machines surface SHALL be available to every active account. For a cluster owner, it SHALL present that account's machines, every durably placed hosted project with owner, member count, connection state, lifecycle state, effective policy and usage, cluster members and invitations, cluster default policy, and cluster audit history. For a cluster member, it SHALL present shared-machine provisioning choices and only projects and usage otherwise available through project access. Owner-only controls and unrelated projects SHALL be absent from member responses and interfaces. No cluster surface SHALL require deployment-wide authority.

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
