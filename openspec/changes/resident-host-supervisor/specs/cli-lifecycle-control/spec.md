## ADDED Requirements

### Requirement: Lifecycle operations act on the supervised project

Reboot, stop, and transport recovery SHALL act on the project the supervisor is running, identified by the supervisor's own record rather than by matching a process's command line. An attached CLI exiting SHALL be reported as an attachment ending, never as the project stopping.

#### Scenario: A project is rebooted while someone is attached

- **WHEN** a project is rebooted while a CLI is attached to it
- **THEN** the project is replaced as the lifecycle control describes
- **AND** the attachment either follows the replacement or reports that it ended

#### Scenario: An attachment ends

- **WHEN** an attached CLI exits while the project keeps running
- **THEN** the project is still reported as running
- **AND** no lifecycle failure is reported for it
