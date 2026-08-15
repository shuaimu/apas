# project-access-control Spec Delta

## REMOVED Requirements

### Requirement: Control-plane authority does not grant project-content access

**Reason**: Cluster administrators operate every project that runs in their own virtual cluster; requiring project membership to open such a project conflates cluster administration with project membership and blocks legitimate admin workflows (for example, launching the CLI for a project directory whose canonical project is owned by another user but whose session runs under the administrator's account).

**Migration**: None. The replacement requirement below grants access scoped to the administrator's own virtual cluster. No data migration is required; existing ownership and membership rows keep their meaning.

## ADDED Requirements

### Requirement: Cluster administrators access projects within their own cluster

An active cluster administrator SHALL receive project content access for projects present in the administrator's own virtual cluster: projects the administrator owns, projects the administrator belongs to, and projects with at least one session created under the administrator's account. An administrator SHALL NOT receive content access to projects that exist only in other accounts' clusters. A suspended administrator account SHALL NOT receive this access. Project ownership and membership SHALL continue to gate ordinary (non-admin) users.

#### Scenario: Administrator opens a project running under their account

- **WHEN** an active cluster administrator's client starts or attaches to a session whose canonical project is owned by another user but has an existing session created under the administrator's account
- **THEN** the system grants the session start or attach

#### Scenario: Administrator listings stay within their own cluster

- **WHEN** an active cluster administrator opens the project or session listings
- **THEN** the system includes projects the administrator owns, projects the administrator belongs to, and sessions created under the administrator's account
- **AND** excludes projects and sessions that exist only under other accounts

#### Scenario: Administrator starts a project CLI on their own machines

- **WHEN** an active cluster administrator starts a project CLI for a machine registered under the administrator's account
- **THEN** the system allows the start without project ownership or membership

#### Scenario: Administrator opening a foreign project is denied

- **WHEN** an active cluster administrator attempts to open or attach to a project that has no session under the administrator's account and no ownership or membership row for the administrator
- **THEN** the system denies the attempt

#### Scenario: Suspended administrator is denied

- **WHEN** a suspended administrator account attempts to open or start any project session
- **THEN** the system denies the attempt

#### Scenario: Ordinary non-member user is still denied

- **WHEN** an ordinary cluster user who is neither the owner nor a member of a project attempts to open that project's sessions or content
- **THEN** the system denies the attempt as before
