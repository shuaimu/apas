## Purpose

Defines how a user sees the machines their account can reach and restarts the daemon on one, including who may do it, what it disturbs, and how its outcome is reported.

## ADDED Requirements

### Requirement: Users can see the machines they can reach

The system SHALL present the machines an account can reach, each identified by its hostname and showing whether it is currently connected and how much work it is running. A machine the account cannot reach SHALL NOT appear.

#### Scenario: A user opens the machine list

- **WHEN** a user opens the machine list
- **THEN** it shows each machine they can reach, its connection state, and how many projects are running on it

#### Scenario: Another account's machine

- **WHEN** a machine belongs to an account the user cannot reach
- **THEN** it does not appear in their machine list

#### Scenario: No machines yet

- **WHEN** an account can reach no machines
- **THEN** the list says so rather than appearing broken or empty without explanation

### Requirement: A daemon can be restarted from its machine entry

The system SHALL allow restarting the daemon of a machine the user can reach, targeted by that machine rather than by any project on it. The request SHALL be confirmed before it is sent, and the confirmation SHALL identify the machine.

#### Scenario: A user restarts a daemon

- **WHEN** a user confirms a restart for a machine they can reach
- **THEN** the system asks that machine's daemon to restart
- **AND** identifies the machine in the confirmation beforehand

#### Scenario: A restart is dismissed

- **WHEN** a user dismisses the confirmation
- **THEN** nothing is sent for that machine

#### Scenario: Restart of an unreachable machine

- **WHEN** a restart is requested for a machine the account cannot reach
- **THEN** the system refuses it
- **AND** the machine's daemon is not asked to restart

### Requirement: Restarting a daemon does not disturb the work on that machine

Restarting a daemon SHALL leave the projects, panes, and agents running on that machine running. It SHALL NOT be presented as an action that stops work.

#### Scenario: Projects are running when the daemon restarts

- **WHEN** a daemon is restarted on a machine with projects running
- **THEN** those projects keep running
- **AND** they are reported as running once the daemon is back

#### Scenario: The user is told what it affects

- **WHEN** the confirmation for a daemon restart is shown
- **THEN** it states that work on the machine keeps running

### Requirement: A daemon restart applies an available update

When an update is available for a machine, restarting its daemon SHALL apply that update rather than restarting the same version. Every step that can fail SHALL complete while the current daemon is still running, so a failure leaves that daemon working and the machine unchanged.

#### Scenario: An update is available

- **WHEN** a daemon is restarted on a machine with an update available
- **THEN** the daemon that comes back is the updated one

#### Scenario: The update fails

- **WHEN** applying the update fails during a restart
- **THEN** the existing daemon keeps running
- **AND** the failure is reported rather than reported as a successful restart

#### Scenario: No update is available

- **WHEN** a daemon is restarted on a machine that is already current
- **THEN** it restarts on the version it was running

### Requirement: A daemon restart reports what happened

The system SHALL report the outcome of a restart request. A request that could not be delivered SHALL be reported as undelivered rather than as done.

#### Scenario: The daemon cannot be reached

- **WHEN** a restart is requested for a machine whose daemon is not connected
- **THEN** the system reports that it could not be delivered

#### Scenario: The request is delivered

- **WHEN** a restart request reaches the daemon
- **THEN** the system reports that the restart was requested
