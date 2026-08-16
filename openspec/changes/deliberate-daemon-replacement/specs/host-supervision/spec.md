## ADDED Requirements

### Requirement: The resident instance is never replaced automatically

The system SHALL NOT replace, restart, or stop a running resident instance except when a person asks for it. Discovering that a newer version is available SHALL NOT by itself cause a replacement, however it is discovered and however long the instance has been running.

#### Scenario: A newer version becomes available

- **WHEN** a newer version becomes available on a host whose instance is running
- **THEN** the running instance is left alone
- **AND** it keeps running the version it started with until someone asks for a replacement

#### Scenario: A host nobody visits

- **WHEN** a host runs for a long period with nobody acting on it
- **THEN** its instance is not replaced on its own

#### Scenario: A requested replacement

- **WHEN** a person requests a restart for a machine
- **THEN** the instance is replaced, applying an available update first

### Requirement: A launch never stops the instance that is already running

A launch that finds a resident instance already running SHALL leave it running, including when the launching program is a newer version than the running instance. It SHALL report that the running instance is older and how to replace it, rather than replacing it. Starting an instance where none is running SHALL be unaffected.

#### Scenario: Launched from a newer version

- **WHEN** a user runs a newer version on a host whose older instance is running
- **THEN** the running instance is not stopped
- **AND** the projects it is running are undisturbed
- **AND** the user is told the running instance is older and how to replace it

#### Scenario: Launched where nothing is running

- **WHEN** a user runs it on a host with no instance running
- **THEN** an instance starts, as it does today

### Requirement: Stopping the instance stops its projects properly

When the resident instance is asked to stop, it SHALL stop the projects it is running before it exits, so that each project's state is saved and the processes it owns are ended. An ordinary request to terminate SHALL reach this path rather than ending the instance outright.

#### Scenario: The instance is asked to stop

- **WHEN** the resident instance is asked to terminate while running projects
- **THEN** each project is stopped through the same teardown an ordinary stop takes
- **AND** the instance exits afterwards

#### Scenario: Pane hosts outlive it

- **WHEN** the resident instance stops while terminal panes are hosted
- **THEN** the pane hosts keep their terminals
- **AND** an instance started within their adoption grace picks them back up
