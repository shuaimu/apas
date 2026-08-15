# project-access-control Specification

## Purpose

Defines project-scoped ownership and collaboration separately from cluster administration so project access has only owner and user roles and applies consistently to every instance of a project.
## Requirements

### Requirement: Project creation assigns one owner
An active cluster user SHALL be allowed to create or register a project and SHALL become that project's single owner. A suspended account SHALL NOT create or register a project.

#### Scenario: Active user creates a project
- **WHEN** an active cluster user creates or first registers a project
- **THEN** the system creates the canonical project record
- **AND** assigns that user as its owner
- **AND** marks the project active

#### Scenario: Suspended user registers a project from a client
- **WHEN** a suspended account's client attempts to register a new project
- **THEN** the server rejects the registration

### Requirement: Projects have only owner and user access roles
A project SHALL have exactly one owner and zero or more project users. The system SHALL NOT expose, accept, or persist a project-level administrator role.

#### Scenario: Project access is displayed
- **WHEN** an authorized viewer opens project access management
- **THEN** the interface labels the owner and lists all other members as users
- **AND** offers no project-admin role

#### Scenario: Client submits project-admin role
- **WHEN** any client attempts to assign the project role `admin`
- **THEN** the server rejects the request as an invalid project role

### Requirement: Membership is scoped to the canonical project
Ownership and user membership SHALL be attached to the canonical project identity rather than an individual session or instance. A project member SHALL receive the same project access across all current and future sessions and instances of that project.

#### Scenario: Member opens another project instance
- **WHEN** a project user opens a different instance whose canonical project identity matches a project they can access
- **THEN** the system grants access using the existing project membership

#### Scenario: Unrelated project uses the same host
- **WHEN** a user has access to one project on a host but not another project on that host
- **THEN** the system denies access to the unrelated project

### Requirement: Project owners manage project users
The project owner SHALL be able to invite an active cluster user, view project membership, and revoke an ordinary user's access. The owner SHALL NOT create project administrators, transfer ownership, modify cluster roles, or act on cluster-governed project policy.

#### Scenario: Owner invites a cluster user
- **WHEN** the project owner invites an active cluster user and the invitation is accepted
- **THEN** the invited account becomes a project user across the project

#### Scenario: Owner attempts an administrator-only action
- **WHEN** a project owner attempts to transfer ownership, assign a cluster role, or modify cluster-governed policy
- **THEN** the system denies the operation without changing project access or policy

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

### Requirement: Legacy project-admin memberships migrate without elevation
During upgrade, every legacy project-admin membership SHALL become an ordinary project-user membership. Existing ownership SHALL be retained, and duplicate session-scoped memberships for the same canonical project and user SHALL collapse to one project membership.

#### Scenario: Legacy administrator share is migrated
- **WHEN** a project has a legacy share with role `admin`
- **THEN** the shared account becomes a project user
- **AND** receives no cluster-administrator authority

#### Scenario: Membership exists on multiple project sessions
- **WHEN** the same account has shares on multiple sessions of one canonical project
- **THEN** migration creates one project-user membership that applies to all of them

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
