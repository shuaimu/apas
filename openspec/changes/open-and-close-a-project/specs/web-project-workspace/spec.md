## ADDED Requirements

### Requirement: A project can be opened and closed from its own workspace

The workspace SHALL offer, in one place, opening a project that is not running and closing one that is. Closing SHALL stop every agent in the project. Both SHALL be confirmed first, and the confirmation for closing SHALL state that the project's agents are stopped, since that is what is lost.

#### Scenario: A project that is not running

- **WHEN** a user views the workspace of a project that is not running
- **THEN** it offers to open the project
- **AND** does not offer to close it

#### Scenario: A project that is running

- **WHEN** a user views the workspace of a running project
- **THEN** it offers to close the project
- **AND** does not offer to open it

#### Scenario: Closing a project

- **WHEN** a user confirms closing a project
- **THEN** the project's agents are stopped

#### Scenario: Declining

- **WHEN** a user declines the confirmation
- **THEN** nothing is stopped

#### Scenario: A closed project's agents are not listed as idle

- **WHEN** a project has been closed
- **THEN** its agents no longer appear among the idle agents
