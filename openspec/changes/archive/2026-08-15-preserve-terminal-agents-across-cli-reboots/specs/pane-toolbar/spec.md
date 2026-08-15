## ADDED Requirements

### Requirement: Pane toolbar exposes only a full CLI reboot
For an attached project, the web interface SHALL offer exactly one project lifecycle action: a full project CLI reboot. It SHALL NOT offer a transport-reconnect action, because transport recovery is automatic and is not a decision a user is equipped to make. Reboot confirmation, progress, and results SHALL communicate that the CLI process is replaced and which pane kinds cannot be adopted across it.

#### Scenario: User opens lifecycle actions

- **WHEN** a user opens lifecycle actions for an attached project
- **THEN** the interface offers `Reboot CLI` and no transport-reconnect action
- **AND** does not describe reboot as a remedy for a lost connection

#### Scenario: User chooses full reboot

- **WHEN** the project has terminal panes and the user confirms `Reboot CLI`
- **THEN** the confirmation identifies whether those live terminal agents can be preserved
- **AND** separately warns about any legacy structured panes that will restart or resume

#### Scenario: Reboot reports progress

- **WHEN** a reboot is in progress
- **THEN** the interface reports its phase against the originating request
- **AND** reports success only after the replacement CLI registers and reconciles its pane roster
