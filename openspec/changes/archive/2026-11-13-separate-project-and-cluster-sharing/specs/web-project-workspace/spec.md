## ADDED Requirements

### Requirement: Project selection commits only after authorized attachment
The web client SHALL treat a requested project selection as pending until the server confirms attachment to that exact session. While pending, it SHALL NOT make the target session the active workspace, restore its cached content, persist it as the current session, or emit project-scoped follow-on messages. A rejected or stale attachment SHALL clear the pending selection, show one actionable error, and retain the last confirmed workspace or return to the project list when none exists.

#### Scenario: Authorized attachment succeeds
- **WHEN** a user selects a project and the server confirms attachment to that exact session
- **THEN** the client commits that session as the active and persisted workspace
- **AND** renders only the data and controls received or retained for that authorized session

#### Scenario: Attachment is denied
- **WHEN** the server rejects a pending attachment for insufficient project or hosting-cluster authority
- **THEN** the client does not render or persist the rejected project as active
- **AND** emits no pane, usage, policy, terminal, or other project-scoped follow-on requests for it
- **AND** displays one error that distinguishes missing project access from missing host-machine access when the server supplies that distinction

#### Scenario: Attachment confirmations arrive out of order
- **WHEN** a confirmation arrives for an older pending request after the user selected another project
- **THEN** the client ignores the stale confirmation for active navigation
- **AND** does not replace the newer pending or confirmed workspace
