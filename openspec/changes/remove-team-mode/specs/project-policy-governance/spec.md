## REMOVED Requirements

### Requirement: Team-mode availability is enforced consistently

**Reason**: Managed team mode is removed. There is no managed pane for the policy to permit or refuse, and no team to interrupt when it is disabled — across the deployment there were zero managed panes when this was decided.

**Migration**: None is needed for behaviour, since nothing was running a team. The `team_available` field remains on the wire and in stored policy so that older web and mobile builds keep parsing what they are sent; it decides nothing, in the same way `cluster_role` is retained and inert. A project whose stored policy still carries it is unaffected.

## ADDED Requirements

### Requirement: Retained policy fields for removed features decide nothing

Where a policy field survives only so that older clients keep parsing responses, the system SHALL NOT let it affect any decision, and SHALL NOT present it as a setting a person can change.

#### Scenario: A stored policy still carries the field

- **WHEN** a project's stored policy still carries a field for a removed feature
- **THEN** the effective policy is unchanged by it
- **AND** no launch is permitted or refused on account of it

#### Scenario: An older client reads the policy

- **WHEN** a client that predates the removal reads a policy response
- **THEN** the response still parses

#### Scenario: The field is not offered

- **WHEN** a person views the policy they may edit
- **THEN** the retained field is not presented as a setting
