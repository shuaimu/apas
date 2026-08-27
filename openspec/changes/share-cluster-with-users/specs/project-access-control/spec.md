## MODIFIED Requirements

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

### Requirement: Accounts access projects within authorized cluster boundaries
An active account SHALL receive project content access for every project it owns, every project it belongs to, and every project placed in its own virtual cluster. Membership in another account's cluster SHALL grant shared-machine use only for projects the member owns or belongs to and SHALL NOT grant content access to all projects in that cluster. Runtime mutations on a shared cluster SHALL additionally require current cluster membership and a matching project placement. A suspended account SHALL receive neither project access nor cluster compute access.

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
