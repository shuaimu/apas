# shared-cluster-membership Specification

## Purpose
Defines explicit, revocable membership in another account's virtual cluster without conflating cluster access with deployment authority or project membership.
## Requirements
### Requirement: Cluster owners add existing active accounts directly
The owner of a virtual cluster SHALL be able to add an existing active APAS account to that cluster by its email address. The membership SHALL be active at once, with no invitation link and no acceptance step. Adding a member SHALL NOT create an account or grant deployment-wide authority. Adding an account that is already a member SHALL replace that membership's machine access and default launch profile. Only the cluster owner SHALL add, list, change, or revoke members of that cluster, and an owner SHALL NOT add their own account.

#### Scenario: Owner adds an active account
- **WHEN** a cluster owner adds the email address of an existing active account
- **THEN** that account is an active member of the owner's cluster immediately
- **AND** the addition is recorded in the cluster's audit history

#### Scenario: Owner adds an unavailable account
- **WHEN** a cluster owner adds an email that has no APAS account, or whose account is suspended
- **THEN** the system refuses without creating an account or a membership

#### Scenario: Non-owner manages membership
- **WHEN** a cluster member or unrelated account attempts to add, list, change, or revoke a member of the cluster
- **THEN** the system denies the operation

### Requirement: A membership names its machines and its default launch profile
Each active membership SHALL carry either an explicit list of the owner's machines the member may use, or the legacy value that permits every current and future machine of the cluster. The web interface SHALL require at least one selected machine when adding a member. Every listed machine SHALL belong to the owner's cluster. A membership MAY name a default launch profile, which SHALL be a supported profile that the deployment default and the cluster default both allow. The owner SHALL be able to change both settings for an active member, and the change SHALL apply to the member's connected clients without a new sign-in.

#### Scenario: Owner limits a member to one machine
- **WHEN** an owner adds a member with one selected machine
- **THEN** the member can discover and use only that machine of the cluster

#### Scenario: Owner names a machine outside the cluster
- **WHEN** an owner submits a machine that is not registered under their account
- **THEN** the system refuses the change

#### Scenario: Owner chooses a default launch profile that policy disallows
- **WHEN** an owner names a default launch profile that is unsupported, retired, or disallowed by the deployment or cluster default policy
- **THEN** the system refuses the change

#### Scenario: Owner removes a machine from a connected member
- **WHEN** an owner removes a machine from an active member's list while that member is connected
- **THEN** the machine leaves the member's machine list
- **AND** later runtime requests by that member for that machine are denied

### Requirement: Earlier invitation links keep working
The system SHALL keep the earlier single-use, expiring cluster-membership invitation endpoints and the acceptance page so that older clients and outstanding links keep working. The ordinary web interface SHALL NOT offer to create a cluster invitation. Accepting a link SHALL require the addressed active account and an explicit confirmation of the trust warning. Cluster-membership invitations SHALL stay distinct from deployment account-registration invitations.

#### Scenario: Addressed account accepts an outstanding link
- **WHEN** the addressed active account confirms the trust warning and accepts an unexpired, unused invitation
- **THEN** the account becomes an active member of that cluster
- **AND** retains ownership of its own virtual cluster

#### Scenario: Wrong account accepts
- **WHEN** an authenticated account other than the invitation's addressee attempts acceptance
- **THEN** the system denies the request without revealing cluster resources

#### Scenario: Invitation is expired or revoked
- **WHEN** the addressed account attempts to accept an expired, used, or revoked invitation
- **THEN** the system refuses membership creation

### Requirement: Cluster ownership and membership are distinct roles
The account that owns the virtual cluster SHALL be its sole cluster owner. An account SHALL continue to own its own virtual cluster and MAY at the same time be a member of several other clusters. A cluster member SHALL be permitted only the shared-machine discovery, project provisioning, and project-scoped runtime operations defined for members, on the machines the membership permits. Membership SHALL NOT permit machine reboot, provider credential inspection or mutation, cluster membership administration, cluster default policy mutation, cluster-wide audit access, cluster-wide usage access, or administration of unrelated hosted projects. Cluster membership SHALL NOT create project membership or expose projects belonging to other cluster members.

#### Scenario: Owner reviews members
- **WHEN** the cluster owner opens cluster membership administration
- **THEN** the system lists active members with their machine access and default launch profile

#### Scenario: Member attempts an owner-only action
- **WHEN** a cluster member attempts to reboot a shared daemon, change its provider credentials, change cluster policy, or inspect cluster-wide administration data
- **THEN** the system denies the operation
- **AND** does not reveal stored credentials or unrelated project data

#### Scenario: Membership is not project access
- **WHEN** a member requests content from another member's project that they neither own nor belong to
- **THEN** the system denies project access

### Requirement: Membership is revocable and security-sensitive
The interface for adding a member SHALL tell the owner that the member can run code on every selected machine. A member's view of a shared cluster SHALL state that projects run on the owner's machines and that the owner can access the files, processes, terminal output, and credentials exposed to those processes. The cluster owner SHALL be able to revoke an active membership. Revocation SHALL immediately deny new provisioning and new runtime mutations by that former member on the cluster, while preserving project ownership, project data, usage history, and hosting placement for owner administration.

#### Scenario: Both parties see the trust notice
- **WHEN** an owner adds a member, or a member opens a cluster shared with them
- **THEN** the interface presents the trust notice for that side

#### Scenario: Owner revokes a member
- **WHEN** the cluster owner revokes an active member
- **THEN** the former member loses shared-machine discovery and mutation permission for that cluster
- **AND** subsequent requests are denied even from an already connected client

#### Scenario: Revoked member owns a hosted project
- **WHEN** membership is revoked while the member owns a project placed in the cluster
- **THEN** the project, its data, ownership, placement, and usage history remain intact
- **AND** the owner can suspend, transfer, or delete the hosted project through normal cluster controls

