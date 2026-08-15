## ADDED Requirements

### Requirement: Accounts access projects within their own virtual cluster

An active account SHALL receive project content access for every project it owns, every project it belongs to, and every project hosted in its own virtual cluster — that is, every project with at least one session created under that account. An account SHALL NOT receive content access to projects that exist only in other accounts' virtual clusters. A suspended account SHALL NOT receive this access. This grant SHALL depend on cluster hosting alone and SHALL NOT depend on any account role.

#### Scenario: Account opens a project running under its own account

- **WHEN** an active account's client starts or attaches to a session whose canonical project is owned by another account but has an existing session created under this account
- **THEN** the system grants the session start or attach

#### Scenario: Listings stay within the account's own cluster

- **WHEN** an active account opens its project or session listings
- **THEN** the system includes projects it owns, projects it belongs to, and sessions created under it
- **AND** excludes projects and sessions that exist only under other accounts

#### Scenario: Account starts a project CLI on its own machines

- **WHEN** an active account starts a project CLI for a machine registered under that account
- **THEN** the system allows the start without project ownership or membership

#### Scenario: Opening a foreign project is denied

- **WHEN** an active account attempts to open or attach to a project that has no session under that account and no ownership or membership row for it
- **THEN** the system denies the attempt

#### Scenario: Suspended account is denied

- **WHEN** a suspended account attempts to open or start any project session
- **THEN** the system denies the attempt

### Requirement: Cluster operators and the system administrator oversee project access without implicit membership

The operator of the virtual cluster hosting a project SHALL be able to inspect its membership, add or remove project users, and transfer its ownership to an active account, and the system administrator SHALL be able to do the same for any project in the deployment. These control-plane operations SHALL NOT make the operator or the system administrator a project member, and SHALL NOT be available to accounts that neither host nor own the project.

#### Scenario: Cluster operator transfers ownership

- **WHEN** the operator of the cluster hosting a project transfers it to an active account
- **THEN** the target becomes the project's sole owner
- **AND** the former owner becomes a project user unless explicitly removed

#### Scenario: Cluster operator manages membership externally

- **WHEN** a cluster operator who is not a project member adds or removes a project user on a project hosted in their cluster
- **THEN** the requested membership change succeeds
- **AND** the operator remains outside the project membership

#### Scenario: System administrator manages membership in any cluster

- **WHEN** the system administrator adds or removes a project user on a project in any virtual cluster
- **THEN** the requested membership change succeeds
- **AND** the system administrator remains outside the project membership

#### Scenario: Unrelated account attempts membership management

- **WHEN** an account that neither owns the project nor hosts it in its virtual cluster attempts to change its membership or ownership
- **THEN** the system denies the operation

## REMOVED Requirements

### Requirement: Cluster administrators access projects within their own cluster

**Reason**: The grant was correct but was conditioned on the `cluster_role = admin` attribute, which no longer exists. Cluster hosting, not an account role, is what justifies the access, so the replacement requirement "Accounts access projects within their own virtual cluster" states the same rule for every active account. Leaving the role-conditioned form would re-break the case it was written for once existing administrators are migrated to ordinary accounts.

**Migration**: None. No data or membership rows change; the condition evaluated at authorization time drops its role test. Accounts that previously held the administrator role keep exactly the access they had within their own cluster, and every other account gains the same rule over its own cluster.

### Requirement: Cluster administrators oversee project access without implicit membership

**Reason**: Replaced by "Cluster operators and the system administrator oversee project access without implicit membership", which re-anchors the authority on the cluster that hosts the project and on the separate system-administrator credential, instead of on the removed deployment-wide account role.

**Migration**: Membership and ownership rows are unchanged. Project users and owners are managed by the hosting cluster's operator, by the project owner as before, or by the system administrator.
