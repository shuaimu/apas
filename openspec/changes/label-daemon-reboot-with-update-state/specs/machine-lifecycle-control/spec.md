## ADDED Requirements

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

### Requirement: Machine listings offer the restart control wherever they appear

Every surface that lists the machines an account can reach SHALL offer the restart control for each of them, with the same confirmation identifying the machine. A surface that lists machines without offering it SHALL be treated as not meeting this capability rather than as an accepted variation.

#### Scenario: A surface that lists machines

- **WHEN** a user opens any surface listing the machines they can reach
- **THEN** each machine there offers the restart control
- **AND** confirms with the machine identified before anything is sent

#### Scenario: Consistency between surfaces

- **WHEN** the same machine appears on more than one surface
- **THEN** each shows the same restart wording for it
