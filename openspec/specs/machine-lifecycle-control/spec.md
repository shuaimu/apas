# machine-lifecycle-control Specification

## Purpose
Defines how a user sees the machines their account can reach and restarts the daemon on one, including who may do it, what happens to the work on that machine, and how its outcome is reported.
## Requirements
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

The system SHALL allow restarting the daemon of a machine the account owns, targeted by that machine rather than by any project on it. The request SHALL be confirmed before it is sent, and the confirmation SHALL identify the machine.

#### Scenario: A user restarts a daemon

- **WHEN** a user confirms a restart for a machine their account owns
- **THEN** the system asks that machine's daemon to restart
- **AND** identifies the machine in the confirmation beforehand

#### Scenario: A restart is dismissed

- **WHEN** a user dismisses the confirmation
- **THEN** nothing is sent for that machine

#### Scenario: Restart of a machine the account does not own

- **WHEN** a restart is requested for a machine the account does not own, including one shared with it through cluster membership
- **THEN** the system refuses it
- **AND** the machine's daemon is not asked to restart

### Requirement: A daemon restart restores the work on that machine

Restarting a daemon SHALL replace the process that runs the machine's projects. The daemon SHALL record the projects that were running, replace itself in place, and start those projects again without anyone asking. Terminal agents SHALL keep running in their pane hosts through the restart and SHALL be adopted again by the returning projects. The restart SHALL NOT be presented as an action that ends work.

#### Scenario: Projects are running when the daemon restarts

- **WHEN** a daemon is restarted on a machine with projects running
- **THEN** the returning daemon starts each of those projects again
- **AND** they are reported as running once they have re-attached

#### Scenario: Terminal agents are running when the daemon restarts

- **WHEN** a daemon is restarted while terminal panes are hosted on the machine
- **THEN** their agents keep running during the restart
- **AND** each returning project adopts its hosted terminals that are still within their reboot grace

#### Scenario: The user is told what it affects

- **WHEN** the confirmation for a daemon restart is shown
- **THEN** it does not present the restart as ending the work on the machine
- **AND** it says that the machine's projects come back by themselves and that terminal agents keep running

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

### Requirement: A machine entry shows the version its daemon is running

The system SHALL show, for each machine an account can reach, the version that machine's daemon reports. When a machine reports no version, the entry SHALL say the version is unknown rather than omitting it silently or presenting a placeholder that reads as a real version.

#### Scenario: A machine reports its version

- **WHEN** a user views a machine whose daemon reports a version
- **THEN** the entry shows that version

#### Scenario: A machine reports no version

- **WHEN** a user views a machine whose daemon reports no version
- **THEN** the entry says the version is unknown
- **AND** does not present an invented or blank value as the machine's version

### Requirement: A machine is behind only when a newer version is known

The system SHALL treat a machine as behind only when the version its daemon reports is strictly older than the newest version the account can already see: the version of the server it is connected to, or the newest version reported by any machine that account can reach. Versions SHALL be compared by the deployment's release ordering rather than as text. A version that is missing, or that cannot be interpreted as a release version, SHALL be treated as unknown and never as behind — and SHALL NOT make any other machine behind.

#### Scenario: A machine older than a peer

- **WHEN** one reachable machine reports an older version than another
- **THEN** the older machine is behind

#### Scenario: A machine older than the server

- **WHEN** every reachable machine reports a version older than the server's
- **THEN** each of those machines is behind

#### Scenario: The newest machine

- **WHEN** a machine reports the newest version the account can see
- **THEN** it is not behind

#### Scenario: A machine newer than the server

- **WHEN** a machine reports a version newer than the server's
- **THEN** it is not behind

#### Scenario: An uninterpretable version

- **WHEN** a machine reports a version that cannot be interpreted as a release version
- **THEN** that machine is not treated as behind
- **AND** no other machine is treated as behind on the strength of it

#### Scenario: Ordering is by release, not by text

- **WHEN** two versions differ in a component whose text ordering disagrees with its release ordering
- **THEN** the comparison follows the release ordering

### Requirement: The restart control says whether it will also update the machine

The restart control on a machine SHALL state whether restarting that machine is known to also update it. When the machine is behind, the control and its confirmation SHALL say the restart updates the machine; otherwise they SHALL offer a plain restart. The wording SHALL describe what is known rather than promising an outcome: a plain restart SHALL NOT assert that no update will be applied.

#### Scenario: Restarting a machine that is behind

- **WHEN** a user views the restart control for a machine that is behind
- **THEN** it says the restart will update that machine
- **AND** the confirmation says so as well

#### Scenario: Restarting a machine that is current

- **WHEN** a user views the restart control for a machine that is not behind
- **THEN** it offers a plain restart
- **AND** does not claim an update is available

#### Scenario: Restarting a machine whose version is unknown

- **WHEN** a user views the restart control for a machine reporting no interpretable version
- **THEN** it offers a plain restart rather than claiming an update

#### Scenario: The action itself is unchanged

- **WHEN** a user confirms a restart from either wording
- **THEN** the same restart is requested for that machine

### Requirement: Machine lists offer the restart control for machines the account operates

Every surface that lists an account's machines for management SHALL offer the restart control for each machine the account itself operates, with the same confirmation identifying the machine. A machine the account reaches only as a member of another account's cluster SHALL be listed with its version and without a restart control. A list that only asks where to run something is not a machine list in this sense.

#### Scenario: A surface that lists machines

- **WHEN** a user opens the Machines page or the mobile machine list
- **THEN** each machine they operate offers the restart control
- **AND** confirms with the machine identified before anything is sent

#### Scenario: A shared machine

- **WHEN** a machine is listed for a user who reaches it only through cluster membership
- **THEN** its entry shows the reported version
- **AND** offers no restart control

#### Scenario: Consistency between surfaces

- **WHEN** the same machine appears on more than one surface
- **THEN** each shows the same restart wording for it

