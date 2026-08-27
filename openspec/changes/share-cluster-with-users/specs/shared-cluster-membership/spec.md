## Purpose

Defines explicit, revocable membership in another account's virtual cluster without conflating cluster access with deployment authority or project membership.

## ADDED Requirements

### Requirement: Cluster owners invite existing active accounts
The owner of a virtual cluster SHALL be able to create a single-use, expiring cluster-membership invitation addressed to an existing active APAS account. Cluster-membership invitations SHALL be distinct from deployment account-registration invitations and SHALL NOT create an account or grant deployment-wide authority. Only the cluster owner SHALL list, resend, or revoke invitations for that cluster.

#### Scenario: Owner invites an active account
- **WHEN** a cluster owner invites the email address of an existing active account
- **THEN** the system creates a pending invitation scoped to that owner's cluster
- **AND** does not add the account until the invitation is accepted

#### Scenario: Owner invites an unavailable account
- **WHEN** a cluster owner invites an email that has no active APAS account
- **THEN** the system refuses the invitation without creating an account
- **AND** explains that deployment registration is separately administered

#### Scenario: Non-owner manages an invitation
- **WHEN** a cluster member or unrelated account attempts to create, list, resend, or revoke an invitation for the cluster
- **THEN** the system denies the operation

### Requirement: Membership requires explicit acceptance
An invited active account SHALL become a cluster member only after accepting an unexpired invitation while authenticated as its addressed account. An account SHALL continue to own its own virtual cluster and MAY simultaneously belong to multiple other clusters. Cluster membership SHALL NOT create project membership or expose projects belonging to other cluster members.

#### Scenario: Addressed account accepts
- **WHEN** the addressed active account accepts an unexpired pending invitation
- **THEN** the account becomes an active member of that cluster
- **AND** retains ownership of its own virtual cluster

#### Scenario: Wrong account accepts
- **WHEN** an authenticated account other than the invitation's addressee attempts acceptance
- **THEN** the system denies the request without revealing cluster resources

#### Scenario: Invitation is expired or revoked
- **WHEN** the addressed account attempts to accept an expired, used, or revoked invitation
- **THEN** the system refuses membership creation

#### Scenario: Member enters a shared cluster
- **WHEN** an active member views a cluster shared with them
- **THEN** the system does not grant access to unrelated member-owned projects merely because they share that cluster

### Requirement: Cluster ownership and membership are distinct roles
The account that owns the virtual cluster SHALL be its sole cluster owner. A cluster member SHALL be permitted only the shared-machine discovery, project provisioning, and project-scoped runtime operations defined for members. Membership SHALL NOT permit machine reboot, provider credential inspection or mutation, cluster invitation or membership administration, cluster default policy mutation, cluster-wide audit access, cluster-wide usage access, or administration of unrelated hosted projects.

#### Scenario: Owner reviews members
- **WHEN** the cluster owner opens cluster membership administration
- **THEN** the system lists active members and pending invitations with their status and relevant timestamps

#### Scenario: Member attempts an owner-only action
- **WHEN** a cluster member attempts to reboot a shared daemon, change its provider credentials, change cluster policy, or inspect cluster-wide administration data
- **THEN** the system denies the operation
- **AND** does not reveal stored credentials or unrelated project data

#### Scenario: Membership is not project access
- **WHEN** a member requests content from another member's project that they neither own nor belong to
- **THEN** the system denies project access

### Requirement: Membership is revocable and security-sensitive
Before creating an invitation and before accepting it, the system SHALL disclose that a member can cause user-controlled repository code and agents to run on the owner's machines and consume configured provider capacity. The cluster owner SHALL be able to revoke an active membership. Revocation SHALL immediately deny new provisioning and new runtime mutations by that former member on the cluster, while preserving project ownership, project data, usage history, and hosting placement for owner administration.

#### Scenario: Both parties see the trust warning
- **WHEN** an owner creates an invitation or an invited account accepts one
- **THEN** the interface presents the remote-code-execution and provider-consumption warning before confirmation

#### Scenario: Owner revokes a member
- **WHEN** the cluster owner revokes an active member
- **THEN** the former member loses shared-machine discovery and mutation permission for that cluster
- **AND** subsequent requests are denied even from an already connected client

#### Scenario: Revoked member owns a hosted project
- **WHEN** membership is revoked while the member owns a project placed in the cluster
- **THEN** the project, its data, ownership, placement, and usage history remain intact
- **AND** the owner can suspend, transfer, or delete the hosted project through normal cluster controls
