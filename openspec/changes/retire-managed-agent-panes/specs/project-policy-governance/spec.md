## ADDED Requirements

### Requirement: Team launches are governed by the same profiles as ordinary work

The effective project policy SHALL govern managed team launches using the same launch profiles as ordinary panes. The catalogue SHALL NOT retain profiles that exist only to describe a retired pane kind.

#### Scenario: A team role whose profile is allowed

- **WHEN** a team is started and its roles' profiles are permitted by the effective policy
- **THEN** the roles launch

#### Scenario: A team role whose profile is not allowed

- **WHEN** a team role's profile is not permitted by the effective policy
- **THEN** the launch is refused
- **AND** the refusal names a profile that exists in the catalogue

#### Scenario: Policy that still names a retired profile

- **WHEN** a stored policy still allows a profile for the retired pane kind
- **THEN** it is ignored rather than offered
- **AND** the project's other permitted profiles are unaffected
