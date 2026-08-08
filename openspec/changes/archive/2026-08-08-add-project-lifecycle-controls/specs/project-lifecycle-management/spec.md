## Purpose

Defines safe project self-service lifecycle controls so ownership, membership, live access, and retained server data can be changed without leaving orphaned authority or history.

## ADDED Requirements

### Requirement: An owner can transfer a project to an existing user
The system SHALL allow the authenticated project owner to transfer an active project to an active ordinary user who is already a member of that project. The transfer SHALL atomically make the selected user the project's sole owner and retain the former owner as an ordinary project user across every project session and instance.

#### Scenario: Owner transfers ownership to a project user
- **WHEN** the project owner confirms a transfer to an active ordinary user of the project
- **THEN** the selected user becomes the project's sole owner
- **AND** the former owner becomes an ordinary project user
- **AND** both users retain project access under their new roles across every project session and instance

#### Scenario: Owner selects an ineligible recipient
- **WHEN** the project owner attempts to transfer ownership to a non-member, suspended account, or the current owner
- **THEN** the server rejects the transfer with an actionable validation error
- **AND** ownership and membership remain unchanged

#### Scenario: Non-owner attempts a transfer
- **WHEN** an ordinary project user or non-member requests an ownership transfer
- **THEN** the server rejects the request
- **AND** ownership and membership remain unchanged

#### Scenario: Transfer races with another membership change
- **WHEN** ownership transfer and removal or departure of the selected user are requested concurrently
- **THEN** the server serializes the mutations so either a valid complete transfer occurs or the transfer is rejected
- **AND** the project never has zero owners or more than one owner

### Requirement: An ordinary user can leave a project
The system SHALL allow an authenticated ordinary project user to leave the project on their own behalf. A successful departure SHALL remove all canonical and compatibility membership records for that user, revoke their access across every project session and instance, and leave all other project data and memberships unchanged.

#### Scenario: Project user leaves
- **WHEN** an ordinary project user confirms that they want to leave a project
- **THEN** the server removes that user's project-wide membership
- **AND** the project disappears from that user's accessible project list
- **AND** the remaining owner and users retain their existing roles and data

#### Scenario: Departing user has an active project connection
- **WHEN** an ordinary user leaves while viewing or operating a session of that project
- **THEN** the server detaches that user's project connections and rejects further project operations using the revoked membership
- **AND** the web client navigates away from the inaccessible project and refreshes its project list

#### Scenario: Owner attempts to leave
- **WHEN** the sole project owner requests to leave the project
- **THEN** the server rejects the request without changing access
- **AND** the interface directs the owner to transfer ownership or delete the project instead

#### Scenario: User attempts to remove another user through the leave operation
- **WHEN** an ordinary project user submits a departure request for another account
- **THEN** the server rejects the request
- **AND** neither membership changes

### Requirement: An owner can permanently delete a project
The system SHALL allow the authenticated project owner to permanently delete an active or administratively suspended project only after an explicit destructive confirmation bound to that project. Ordinary users, non-members, and stale clients whose owner role has changed SHALL NOT be allowed to delete it.

#### Scenario: Owner confirms project deletion
- **WHEN** the current project owner supplies the required project-specific confirmation and requests deletion
- **THEN** the system begins permanent server-side deletion of that project
- **AND** no other project is affected

#### Scenario: Deletion confirmation is absent or mismatched
- **WHEN** an owner submits a deletion request without the required confirmation or with confirmation for a different project
- **THEN** the server rejects the request without deleting or disconnecting the project

#### Scenario: Unauthorized actor requests deletion
- **WHEN** an ordinary user, non-member, or former owner requests project deletion
- **THEN** the server rejects the request
- **AND** all project data and runtime state remain unchanged

### Requirement: Successful deletion erases all APAS-managed project data
Before reporting deletion as complete, the system SHALL erase every APAS-managed server artifact associated with the project. This includes the canonical project, sessions and instances, messages and persisted terminal or pane history, pane metadata and usage, memberships and compatibility shares, invitations, policy overrides, project-identifying audit entries, and project-specific in-memory caches. The deletion SHALL NOT erase a source checkout or local APAS configuration on a connected machine.

#### Scenario: Project with multiple sessions and retained history is deleted
- **WHEN** deletion completes for a project with multiple instances, members, invitations, messages, terminal state, usage records, policy data, and audit events
- **THEN** none of those project-associated artifacts can be retrieved from APAS application storage or APIs
- **AND** project list, administrative inventory, invitation redemption, and historical-session lookups no longer expose the deleted project

#### Scenario: Deleted project had a connected runtime
- **WHEN** deletion begins while project CLI, daemon, or web clients are connected
- **THEN** the system prevents new project mutations, stops the project runtime, and detaches connected viewers before erasing persisted state
- **AND** delayed messages or reconnect attempts cannot restore deleted history during the deletion

#### Scenario: Local checkout remains after server deletion
- **WHEN** server-side project deletion completes
- **THEN** the local checkout and local APAS configuration remain untouched
- **AND** the checkout may still appear as a local machine project
- **AND** a later explicit start may register a fresh server project with no restored history, invitations, or memberships from the deletion

### Requirement: Project deletion is fail-safe and restart-resumable
The system SHALL serialize deletion against ownership, membership, registration, storage append, and session mutations. Once destructive cleanup begins, the project SHALL remain inaccessible until cleanup either completes or is safely recovered, and the system SHALL NOT report success while any APAS-managed project artifact remains. An interrupted deletion SHALL resume after server restart without making partially deleted project state accessible.

#### Scenario: Cleanup fails before completion
- **WHEN** a database or file-storage deletion step fails
- **THEN** the server reports deletion as incomplete rather than successful
- **AND** blocks project access and registration while preserving enough cleanup state to retry safely

#### Scenario: Server restarts during deletion
- **WHEN** the server stops after a deletion has entered its destructive phase but before every artifact is erased
- **THEN** startup recovery resumes the idempotent cleanup
- **AND** the project remains unavailable throughout recovery
- **AND** final success leaves no project-specific cleanup marker behind

#### Scenario: A delayed client event arrives during cleanup
- **WHEN** a disconnected client sends a delayed message, session registration, or runtime update for a project being deleted
- **THEN** the server rejects or discards the event
- **AND** the event does not recreate project data or history

### Requirement: The web interface exposes role-appropriate lifecycle controls
The web interface SHALL expose ownership transfer and permanent deletion to the current owner and project departure to ordinary users, with clear role consequences and confirmation before each mutation. After a successful operation, every affected client SHALL refresh project roles and accessible-project state so stale owner or member controls are not retained.

#### Scenario: Owner reviews lifecycle actions
- **WHEN** an owner opens project access or project settings
- **THEN** the interface offers transfer only to current eligible project users
- **AND** explains that the owner will become an ordinary user
- **AND** presents project deletion as a distinct irreversible action with project-specific confirmation

#### Scenario: Ordinary user reviews lifecycle actions
- **WHEN** an ordinary project user opens project actions
- **THEN** the interface offers leaving the project
- **AND** does not offer ownership transfer or project deletion

#### Scenario: Roles change in another client
- **WHEN** ownership transfer, member departure, or deletion succeeds in one client while another client has the project open
- **THEN** the other client receives or obtains refreshed authorization state before its next privileged mutation
- **AND** removes controls and content that its current role no longer permits
