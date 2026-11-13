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

## MODIFIED Requirements

### Requirement: Operational project settings remain distinct from cluster policy
Moving capability policy to cluster administration SHALL NOT by itself remove an owner's existing non-policy project operations, such as restricting which tab types the project allows or using capabilities that policy permits. Authorization SHALL distinguish cluster-governed availability from ordinary operation within that boundary.

#### Scenario: Owner uses an allowed project operation
- **WHEN** a project owner changes the project's allowed tab types or launches a policy-allowed pane
- **THEN** the system permits the operation under normal project access rules

#### Scenario: Owner changes a combined legacy settings payload
- **WHEN** a legacy settings request contains both owner-operable values and cluster-governed policy values
- **THEN** the system prevents unauthorized policy changes
- **AND** does not treat ownership as cluster-administrator authority
