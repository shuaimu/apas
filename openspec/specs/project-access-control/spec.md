# project-access-control Specification

## Purpose

Defines project-scoped ownership and collaboration separately from cluster administration so project access has only owner and user roles and applies consistently to every instance of a project.
## Requirements
### Requirement: Project creation assigns one owner
An active account SHALL be allowed to create or register a project where it has valid machine authority and SHALL become that project's single owner, even when the project is created through another account's shared cluster. The system SHALL record hosting placement separately from ownership. A suspended account or an account without current permission to use the target cluster SHALL NOT create or register a project there.

#### Scenario: Active user creates a project
- **WHEN** an active account creates or first registers a project on its own machine
- **THEN** the system creates the canonical project record
- **AND** assigns that account as its owner
- **AND** marks the project active and placed in that account's cluster

#### Scenario: Active member creates a project in a shared cluster
- **WHEN** an active cluster member successfully creates a project on an eligible shared machine
- **THEN** the system assigns the member as project owner
- **AND** records the machine owner's cluster as a hosting placement

#### Scenario: Suspended user registers a project from a client
- **WHEN** a suspended account attempts to register a new project on the target cluster
- **THEN** the server rejects the registration

#### Scenario: Revoked member registers a project
- **WHEN** a revoked cluster member attempts to register a new project on that shared cluster
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
The project owner SHALL be able to invite an active account to the project, view project membership, and revoke an ordinary user's access. The owner SHALL be able to transfer ownership only to an existing project user, as `project-lifecycle-management` defines. The owner SHALL NOT create project administrators and SHALL NOT change cluster-governed project policy.

#### Scenario: Owner invites an active account
- **WHEN** the project owner invites an active account and the invitation is accepted
- **THEN** the invited account becomes a project user across the project

#### Scenario: Owner attempts a cluster-governed action
- **WHEN** a project owner who does not operate the hosting cluster attempts to change cluster-governed policy or to assign a project-admin role
- **THEN** the system denies the operation without changing project access or policy

### Requirement: Cluster operators and the system administrator oversee project access without implicit membership
The owner of a virtual cluster hosting a project SHALL be able to inspect its membership, add or remove project users, and transfer its ownership to an active account, and the system administrator SHALL be able to do the same for any project in the deployment. A cluster member who does not own that cluster SHALL NOT receive these hosting-administrator powers. These control-plane operations SHALL NOT make the cluster owner or system administrator a project member, and SHALL NOT be available to accounts that neither own the hosting cluster nor hold the project's ordinary owner authority.

#### Scenario: Cluster operator transfers ownership
- **WHEN** the owner of a cluster hosting a project transfers it to an active account
- **THEN** the target becomes the project's sole owner
- **AND** the former owner becomes a project user unless explicitly removed

#### Scenario: Cluster operator manages membership externally
- **WHEN** a hosting cluster owner who is not a project member adds or removes a project user
- **THEN** the requested membership change succeeds
- **AND** the cluster owner remains outside project membership

#### Scenario: Shared member attempts hosting administration
- **WHEN** a cluster member who does not own the cluster attempts to manage another hosted project's membership or ownership
- **THEN** the system denies the operation

#### Scenario: System administrator manages membership in any cluster
- **WHEN** the system administrator adds or removes a project user on a project in any virtual cluster
- **THEN** the requested membership change succeeds
- **AND** the system administrator remains outside project membership

#### Scenario: Unrelated account attempts membership management
- **WHEN** an account that neither owns the project nor owns a hosting cluster attempts to change its membership or ownership
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

### Requirement: Accounts access projects within authorized cluster boundaries
An active account SHALL receive project content access for every project it owns, every project it belongs to, and every project placed in its own virtual cluster. Membership in another account's cluster SHALL grant shared-machine use only for projects the member owns or belongs to and SHALL NOT grant content access to all projects in that cluster. Runtime use of an instance hosted by an account that is neither the caller nor the project's owner SHALL additionally require the caller's active membership of that hosting cluster with permission for the instance's machine. A suspended account SHALL receive neither project access nor cluster compute access.

#### Scenario: Cluster owner opens a hosted project
- **WHEN** an active cluster owner opens a project durably placed in its cluster but owned by another account
- **THEN** the system grants content access without adding project membership

#### Scenario: Shared member opens their project
- **WHEN** an active cluster member opens a project they own or belong to that is placed in the shared cluster
- **THEN** the system grants project access and permitted project-scoped runtime operations

#### Scenario: Shared member opens another member's project
- **WHEN** a cluster member requests a hosted project they neither own nor belong to
- **THEN** the system denies content and runtime access

#### Scenario: Listings respect both boundaries
- **WHEN** an active account opens project or session listings
- **THEN** the system includes projects it owns, projects it belongs to, and projects placed in its own cluster
- **AND** excludes unrelated projects that are visible only because it belongs to a shared cluster

#### Scenario: Revoked member retains project ownership but loses compute permission
- **WHEN** a former cluster member still owns a project placed in that cluster
- **THEN** the account retains project content and ownership access
- **AND** the system denies new runtime mutations on that cluster until access is restored or placement changes

#### Scenario: Suspended account is denied
- **WHEN** a suspended account attempts to open or start any project session
- **THEN** the system denies the attempt

### Requirement: Project sharing and cluster sharing grant independent scopes
An active project user SHALL receive project content access independently of cluster membership. For a runtime instance hosted in the project owner's cluster, that project membership SHALL also grant project-scoped runtime access without granting machine discovery, project provisioning, cluster administration, or access to unrelated projects. For a runtime instance hosted by a third-party cluster, project membership SHALL NOT bypass the hosting cluster's active membership and machine restrictions. Cluster membership SHALL grant only the configured shared-machine capabilities and SHALL NOT grant content access to a project the member neither owns nor belongs to.

#### Scenario: Owner shares an owner-hosted project
- **WHEN** a project owner adds an active account as a project user and the selected runtime instance is hosted in that owner's cluster
- **THEN** the project user can open the project's conversations and operate that runtime
- **AND** the project user receives no access to the owner's machine inventory, project creation surface, cluster controls, or unrelated projects from that project share

#### Scenario: Project user opens a third-party-hosted instance
- **WHEN** a project user opens an instance hosted by a cluster that neither the project user nor the project owner operates
- **THEN** the user retains project content access
- **AND** runtime attachment and mutations require active permission for the hosting cluster and the exact machine

#### Scenario: Cluster member requests an unrelated project
- **WHEN** an active cluster member requests a project they neither own nor belong to
- **THEN** the system denies project content and runtime access
- **AND** does not reveal the project's pane, conversation, usage, or policy data

#### Scenario: User holds both grants
- **WHEN** an active account belongs to a project and has permission for the third-party cluster and machine hosting its selected instance
- **THEN** the account can attach to and operate that project instance
- **AND** each grant remains independently revocable

#### Scenario: Third-party cluster access is revoked
- **WHEN** a project user loses membership or machine permission for the third-party cluster hosting an instance
- **THEN** subsequent attachment and runtime mutations for that instance are denied
- **AND** the project membership and persisted project content remain intact

### Requirement: Sharing interfaces explain the granted authority
The project-sharing interface SHALL identify its grant as project access and explain whether the project's current hosting permits runtime use without cluster access. The cluster-sharing interface SHALL identify its grant as machine access, name the allowed machines and default launch profile, and explain that it does not expose unrelated projects. Neither interface SHALL describe one grant as conferring the other.

#### Scenario: Owner opens project sharing
- **WHEN** a project owner opens the project-access interface
- **THEN** the interface explains that the share applies only to that project
- **AND** distinguishes owner-hosted runtime access from a third-party host that separately requires cluster permission

#### Scenario: Owner opens cluster sharing
- **WHEN** a cluster owner opens the cluster-sharing interface
- **THEN** the interface explains that the member can use only selected machines under the configured defaults
- **AND** states that project content remains limited to projects the member owns or belongs to

