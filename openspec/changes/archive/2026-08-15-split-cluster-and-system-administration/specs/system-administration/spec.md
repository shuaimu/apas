## Purpose

Defines the single deployment-wide APAS system administrator: a credential held outside the ordinary account system, reached only through its own login on a separate administration URL, with authority over every account, every virtual cluster, and every project in the deployment.

## ADDED Requirements

### Requirement: The deployment has exactly one system administrator

The system SHALL recognize exactly one system-administrator identity for the whole deployment. That identity SHALL be stored separately from ordinary accounts, SHALL be seeded from server configuration, and SHALL NOT be creatable, promotable, or grantable through any user-facing surface or API. No ordinary account SHALL acquire system-administrator authority by any role, membership, ownership, or email match.

#### Scenario: Configuration seeds the system administrator

- **WHEN** the server starts with a system-administrator credential configured and none stored yet
- **THEN** the system stores exactly one system-administrator identity from that configuration
- **AND** subsequent starts reuse the stored identity rather than reseeding it

#### Scenario: Account cannot be promoted to system administrator

- **WHEN** any request attempts to grant an ordinary account system-administrator authority
- **THEN** the system rejects the request
- **AND** the stored system-administrator identity is unchanged

#### Scenario: Second system administrator is refused

- **WHEN** an attempt is made to store an additional system-administrator identity
- **THEN** the system keeps exactly one identity and reports the conflict

### Requirement: System administration requires its own login

Access to the system-administration surface SHALL require authenticating the system-administrator credential through a login that is separate from ordinary account login. A successful login SHALL issue a credential-scoped token that authorizes only system-administration operations, and SHALL NOT authorize any project, session, machine, or cluster-user operation. An ordinary account token SHALL be rejected by every system-administration operation, and a system-administrator token SHALL be rejected by every ordinary account operation.

#### Scenario: System administrator logs in

- **WHEN** the correct system-administrator credential is submitted to the system-administration login
- **THEN** the system issues a token scoped to system administration
- **AND** that token authorizes the system-administration operations

#### Scenario: Ordinary account token is presented to system administration

- **WHEN** a request carrying an authenticated ordinary account token is made to a system-administration operation
- **THEN** the system denies the request regardless of that account's role, ownership, or membership

#### Scenario: System-administrator token is presented elsewhere

- **WHEN** a system-administrator token is presented to a project, session, machine, or cluster operation
- **THEN** the system denies the request

#### Scenario: Ordinary login never issues system-administrator authority

- **WHEN** any account completes ordinary account login
- **THEN** the issued token carries no system-administration authority

### Requirement: System-administrator credentials are protected and rotatable

The system-administrator credential SHALL be stored only as a verifier from which the secret cannot be recovered, SHALL be changeable by the authenticated system administrator, and changing it SHALL invalidate previously issued system-administration tokens. Issued system-administration tokens SHALL expire. Repeated failed authentication attempts SHALL be throttled. While the credential remains the value seeded from configuration, the system-administration surface SHALL warn that it has not been rotated.

#### Scenario: Credential is rotated

- **WHEN** the authenticated system administrator changes the credential
- **THEN** later logins require the new secret
- **AND** tokens issued before the change no longer authorize system administration

#### Scenario: Repeated failures are throttled

- **WHEN** system-administration login fails repeatedly for the same source
- **THEN** the system delays or refuses further attempts for a bounded period
- **AND** does not reveal whether the submitted identity exists

#### Scenario: Unrotated bootstrap credential is surfaced

- **WHEN** the system administrator signs in while the credential is still the configured bootstrap value
- **THEN** the surface warns that the credential must be rotated

### Requirement: The system-administration surface is unadvertised to users

Ordinary user interfaces SHALL NOT link to, mention, or conditionally render the system-administration surface, and no ordinary account state SHALL change what a user sees about it. The surface SHALL be reachable only by navigating directly to its own URL, and SHALL present its own login rather than adopting any existing account session.

#### Scenario: User interface omits the administration entry

- **WHEN** any account uses the ordinary web interface
- **THEN** no navigation entry, link, or control for the system-administration surface is presented

#### Scenario: Unauthenticated visit to the administration URL

- **WHEN** someone opens the system-administration URL without a system-administration token
- **THEN** the surface presents its own login and no administrative data

#### Scenario: Signed-in user opens the administration URL

- **WHEN** an account signed in to the ordinary web interface opens the system-administration URL
- **THEN** the surface still requires the separate system-administration login
- **AND** does not authorize the visitor on the strength of the existing account session

### Requirement: The system administrator manages every account

The system administrator SHALL be able to list every account in the deployment, invite or create an account, and activate or suspend it. A suspended account SHALL be denied authentication, client registration, and all use of existing credentials. Account management SHALL NOT be available to ordinary accounts.

#### Scenario: Administrator suspends an account

- **WHEN** the system administrator suspends an active account
- **THEN** that account is denied authentication and its existing credentials stop granting access
- **AND** its data, project ownership, and memberships are preserved

#### Scenario: Administrator invites an account

- **WHEN** the system administrator invites an address
- **THEN** the system produces a registration path that creates an active account

#### Scenario: Ordinary account requests the account list

- **WHEN** an ordinary account requests the deployment account list
- **THEN** the system denies the request

### Requirement: The system administrator sees every virtual cluster and project

The system-administration surface SHALL list every virtual cluster in the deployment with its operating account, machine count, project count, and activity summary, and SHALL list every project with its owner, its virtual cluster, member count, effective policy, connection state, and lifecycle state. This inventory SHALL be complete regardless of ownership or membership, and SHALL NOT create membership anywhere.

#### Scenario: Administrator lists clusters

- **WHEN** the system administrator opens the cluster inventory
- **THEN** the system lists every account's virtual cluster with its summary counts

#### Scenario: Administrator opens an unrelated project

- **WHEN** the system administrator opens administrative details for a project in another account's cluster
- **THEN** the system shows the project's administrative metadata and status
- **AND** adds no membership for the system administrator

### Requirement: The system administrator controls any project's lifecycle

The system administrator SHALL be able to suspend or reactivate any project in the deployment and stop any project's active runtime, regardless of which virtual cluster hosts it. Suspension SHALL preserve project data and membership while denying new member attachments, client registrations, and pane or team launches.

#### Scenario: Administrator suspends a project in another cluster

- **WHEN** the system administrator suspends a running project hosted in another account's cluster
- **THEN** the system stops or disconnects that project's active runtime
- **AND** denies further project operations until it is reactivated
- **AND** preserves its data and membership

#### Scenario: Cluster operator cannot reach another cluster

- **WHEN** an ordinary account attempts a lifecycle operation on a project outside its own virtual cluster
- **THEN** the system denies the operation

### Requirement: System-administration actions are attributed and auditable

Every successful system-administration mutation SHALL be recorded with the acting identity attributed as the system administrator rather than as an account, together with the target, action, and timestamp. The system administrator SHALL be able to review the deployment's complete audit history, including actions taken by cluster operators. A rejected mutation SHALL NOT be recorded as a successful administrative action.

#### Scenario: Administrator action is recorded

- **WHEN** the system administrator suspends an account or a project
- **THEN** the system records an audit event attributed to the system administrator with the target, action, and time

#### Scenario: Administrator reviews cluster-operator actions

- **WHEN** the system administrator opens the audit history
- **THEN** the system includes control-plane actions taken by cluster operators in any cluster

#### Scenario: Rejected mutation is not recorded as successful

- **WHEN** an unauthorized request attempts a system-administration mutation
- **THEN** the system rejects it and records no successful administrative action
