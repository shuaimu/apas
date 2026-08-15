# project-access-control Spec Delta

## REMOVED Requirements

### Requirement: Control-plane authority does not grant project-content access

**Reason**: Cluster administrators operate the whole cluster; requiring project membership to open a project they administer conflates cluster administration with project membership and blocks legitimate admin workflows (for example, launching the CLI for a project directory whose canonical project is owned by another user).

**Migration**: None. The replacement requirement below grants the previously denied access. No data migration is required; existing ownership and membership rows keep their meaning.

## ADDED Requirements

### Requirement: Cluster administrators access all cluster projects

An active cluster administrator SHALL receive the same project content access as the project owner across every project in the cluster, without being the project owner or a project member. A suspended administrator account SHALL NOT receive this access. Project ownership and membership SHALL continue to gate ordinary (non-admin) users.

#### Scenario: Administrator opens a non-member project session

- **WHEN** an active cluster administrator's client starts or attaches to a session whose canonical project is owned by another user and lists no membership for the administrator
- **THEN** the system grants the session start or attach

#### Scenario: Administrator browses cluster projects

- **WHEN** an active cluster administrator opens the project and session listings
- **THEN** the system includes projects and sessions from every owner in the cluster, not only the administrator's own or shared projects

#### Scenario: Administrator starts a project CLI on a machine

- **WHEN** an active cluster administrator asks the system to start a project CLI for any machine and project registered with the cluster
- **THEN** the system allows the start without project ownership or membership on that project

#### Scenario: Suspended administrator is denied

- **WHEN** a suspended administrator account attempts to open or start any project session
- **THEN** the system denies the attempt

#### Scenario: Ordinary non-member user is still denied

- **WHEN** an ordinary cluster user who is neither the owner nor a member of a project attempts to open that project's sessions or content
- **THEN** the system denies the attempt as before
