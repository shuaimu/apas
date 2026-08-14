## ADDED Requirements

### Requirement: Pane toolbar distinguishes reconnect from reboot
For an attached project, the web interface SHALL distinguish a transport-only server reconnect from a full project CLI reboot. Labels, confirmation text, progress, and results SHALL communicate that reconnect preserves the current CLI and panes, while reboot replaces the CLI and may restart pane kinds that cannot be adopted.

#### Scenario: Attached CLI supports both lifecycle controls
- **WHEN** a user opens lifecycle actions for a project whose CLI advertises reconnect and persistent-terminal support
- **THEN** the interface offers separate `Reconnect Server` and `Reboot CLI` actions
- **AND** explains the narrower effect of reconnect before the broader reboot action

#### Scenario: User chooses reconnect
- **WHEN** the user confirms `Reconnect Server`
- **THEN** the interface reports transport reconnection progress
- **AND** does not label panes as rebooting or warn that agents will restart

#### Scenario: User chooses full reboot
- **WHEN** the project has terminal panes and the user confirms `Reboot CLI`
- **THEN** the confirmation identifies whether those live terminal agents can be preserved
- **AND** separately warns about any legacy structured panes that will restart or resume

#### Scenario: Attached CLI is too old for reconnect
- **WHEN** the active project CLI does not advertise the reconnect capability
- **THEN** the interface does not offer a reconnect action that would be routed as a reboot
- **AND** provides an actionable CLI upgrade explanation

