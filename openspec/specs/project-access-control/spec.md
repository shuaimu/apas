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

### Requirement: Cluster administrators oversee project access without implicit membership
A cluster administrator SHALL be able to inspect membership, add or remove project users, and transfer project ownership to an active cluster user. These control-plane operations SHALL NOT make the administrator a project member.

#### Scenario: Administrator transfers ownership
- **WHEN** a cluster administrator transfers a project to an active cluster user
- **THEN** the target becomes the project's sole owner
- **AND** the former owner becomes a project user unless the administrator explicitly removes them

#### Scenario: Administrator manages membership externally
- **WHEN** a cluster administrator who is not a project member adds or removes a project user
- **THEN** the requested membership change succeeds
- **AND** the administrator remains outside the project membership

### Requirement: Control-plane authority does not grant project-content access
A cluster administrator who is not the project owner or a project user SHALL NOT receive project conversations, files, diffs, terminal access, or ordinary interactive project controls. Content access SHALL require explicit project membership.

#### Scenario: Non-member administrator opens project content
- **WHEN** a cluster administrator attempts to attach to project conversations without project membership
- **THEN** the system denies content and interactive access
- **AND** continues to allow the administrator to view administrative metadata and status

### Requirement: Legacy project-admin memberships migrate without elevation
During upgrade, every legacy project-admin membership SHALL become an ordinary project-user membership. Existing ownership SHALL be retained, and duplicate session-scoped memberships for the same canonical project and user SHALL collapse to one project membership.

#### Scenario: Legacy administrator share is migrated
- **WHEN** a project has a legacy share with role `admin`
- **THEN** the shared account becomes a project user
- **AND** receives no cluster-administrator authority

#### Scenario: Membership exists on multiple project sessions
- **WHEN** the same account has shares on multiple sessions of one canonical project
- **THEN** migration creates one project-user membership that applies to all of them
