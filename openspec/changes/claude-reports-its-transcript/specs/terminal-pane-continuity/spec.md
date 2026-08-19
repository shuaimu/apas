## ADDED Requirements

### Requirement: A pane's transcript is identified by what the provider reports

Where a provider can report which transcript it is writing, the system SHALL use that report to identify the pane's transcript, in preference to inferring it from the pane's working directory or from an identifier the system assigned. A report SHALL be treated as the provider stating that its session changed, and SHALL re-point the pane immediately.

#### Scenario: The provider moves its session

- **WHEN** a provider relocates its session so that it writes somewhere other than where the pane's working directory implies
- **THEN** the pane follows the reported location

#### Scenario: The user resumes a different session

- **WHEN** a user switches the pane's provider onto a different session
- **THEN** the pane follows the transcript that session writes
- **AND** does not continue reading the session the system originally assigned

#### Scenario: Nothing has been reported

- **WHEN** no report exists for a pane, because its provider is older or the report could not be made
- **THEN** the system falls back to inferring the transcript as it did before
- **AND** the pane is no worse off than without the mechanism

#### Scenario: A report that names nothing usable

- **WHEN** a report carries no usable transcript location
- **THEN** it is ignored rather than followed

### Requirement: Reporting is scoped to the system's own panes

The reporting mechanism SHALL identify which pane a report belongs to, and SHALL record nothing for a provider the system did not start. Installing the mechanism SHALL NOT change any other behaviour the pane's owner has configured for that provider.

#### Scenario: A provider the user started themselves

- **WHEN** a person runs the provider by hand, outside the system
- **THEN** nothing is recorded for any pane
- **AND** that session cannot be mistaken for a pane's own

#### Scenario: The owner's configuration is preserved

- **WHEN** the mechanism is installed for a pane
- **THEN** the provider still honours the settings its owner configured

#### Scenario: The mechanism cannot be installed

- **WHEN** the mechanism cannot be installed for a pane
- **THEN** the pane still starts
