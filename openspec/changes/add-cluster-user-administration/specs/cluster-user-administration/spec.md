## Purpose

Defines deployment-wide cluster membership and administration independently from project collaboration, including durable roles, account lifecycle, inventory visibility, and auditable control-plane actions.

## ADDED Requirements

### Requirement: Cluster identity is server-authoritative
Every authenticated account SHALL have exactly one cluster role (`admin` or `user`) and one account state (`active` or `suspended`) stored by the server. Authenticated identity responses SHALL include both values, and authorization SHALL use the stored values rather than hard-coded emails or user IDs.

#### Scenario: Active cluster user authenticates
- **WHEN** an active account completes authentication
- **THEN** the system returns its cluster role and active state with its identity
- **AND** server authorization uses that persisted role for subsequent requests

#### Scenario: Hard-coded identity has no implicit authority
- **WHEN** an account has a historically privileged email or user ID but is not assigned the cluster-admin role
- **THEN** the system denies cluster-administration operations

### Requirement: Cluster administrators manage membership lifecycle
The system SHALL allow a cluster administrator to list cluster accounts, create or invite an account, activate or suspend it, and promote or demote it between cluster user and cluster administrator. An account SHALL NOT gain cluster access until it is active, and the system SHALL prevent removal, suspension, or demotion of the last active cluster administrator.

#### Scenario: Administrator adds a cluster user
- **WHEN** a cluster administrator creates or approves an account as an active cluster user
- **THEN** that account can authenticate, connect its clients, and use cluster-user capabilities

#### Scenario: Suspended account attempts access
- **WHEN** a suspended account attempts to authenticate, connect a client, or use an existing credential
- **THEN** the system denies the operation and grants no project access

#### Scenario: Administrator attempts to remove the final administrator
- **WHEN** an operation would leave the cluster with no active cluster administrator
- **THEN** the system rejects the operation without changing the account

### Requirement: Cluster administrators have a complete project inventory
The cluster administration surface SHALL list every project in the cluster with its owner, member count, effective policy, connected/offline state, active-session summary, and last activity. This inventory SHALL be available regardless of whether the administrator is a project member.

#### Scenario: Administrator views an unshared project
- **WHEN** a cluster administrator opens the project inventory for a project they do not own and have not joined
- **THEN** the system shows the project's administrative metadata and current status
- **AND** does not add the administrator to the project's membership

#### Scenario: Ordinary user requests cluster inventory
- **WHEN** a cluster user without the administrator role requests the all-project inventory
- **THEN** the system denies the request

### Requirement: Cluster administrators control project lifecycle
Every project SHALL have an active or suspended administrative state. A cluster administrator SHALL be able to suspend or reactivate a project and stop its active runtime without becoming a project member. Suspension SHALL preserve project data while denying new member attachments, client registrations, and pane or team launches for that project.

#### Scenario: Administrator suspends an active project
- **WHEN** a cluster administrator suspends a project with connected clients or running panes
- **THEN** the system stops or disconnects the active project runtime
- **AND** denies further project operations until the project is reactivated
- **AND** preserves the project's data and membership

#### Scenario: Administrator reactivates a project
- **WHEN** a cluster administrator reactivates a suspended project
- **THEN** authorized project members can reconnect and resume normal project operations

#### Scenario: Owner attempts to reactivate a project
- **WHEN** a project owner attempts to change the project's administrative lifecycle state
- **THEN** the system denies the operation

### Requirement: Cluster control-plane actions are auditable
The system SHALL record successful cluster-role, account-state, project-lifecycle, project-owner, project-membership, and project-policy changes with the acting administrator, target, action, and timestamp. Cluster administrators SHALL be able to review these audit records.

#### Scenario: Administrator changes project policy
- **WHEN** a cluster administrator changes a project's governed policy
- **THEN** the system records an audit event identifying the administrator, project, changed policy, and time

#### Scenario: Unauthorized mutation is rejected
- **WHEN** a non-administrator attempts a cluster control-plane mutation
- **THEN** the system rejects the mutation
- **AND** does not record it as a successful administrative action

### Requirement: Existing installations receive a safe cluster-role migration
On upgrade, existing accounts SHALL become active cluster users, and the identity previously recognized by the system dashboard SHALL become an active cluster administrator. Migration SHALL NOT infer cluster-administrator authority from any project-level role.

#### Scenario: Existing project administrator is migrated
- **WHEN** an existing account has a project-admin membership but was not the system administrator
- **THEN** the account is migrated as a cluster user rather than a cluster administrator

#### Scenario: Existing system administrator is migrated
- **WHEN** the deployment containing the previously configured system administrator is upgraded
- **THEN** that account is assigned the active cluster-administrator role before hard-coded authorization is removed
