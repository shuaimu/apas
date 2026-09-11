## ADDED Requirements

### Requirement: Mobile task launch supports capable Pi project hosts
The mobile task-launch catalog SHALL include `terminal:pi:official:default` when effective project policy permits it. A mobile Pi task SHALL be routed only to a connected project CLI that advertises both the current mobile task-launch capability and explicit Pi terminal capability.

#### Scenario: Capable project host launches a Pi mobile task
- **WHEN** an authorized mobile user selects an allowed Pi terminal profile and the connected project CLI advertises Pi terminal capability
- **THEN** the server creates a Pi terminal pane for the exact project
- **AND** passes the submitted instruction as the pane's initial Pi message
- **AND** acknowledges the retained mobile launch operation only after the pane is reported

#### Scenario: Older project host lacks Pi terminal capability
- **WHEN** an authorized mobile user selects Pi but the connected project CLI does not advertise Pi terminal capability
- **THEN** the server rejects the launch with an update-and-reconnect error
- **AND** does not route a Pi pane request or report the operation as successful
- **AND** compatible Claude, Codex, and OpenCode launches on that host are unaffected

#### Scenario: Effective policy removes Pi before submission
- **WHEN** a stale mobile catalog still shows Pi but the effective project policy no longer permits its profile at submission time
- **THEN** the server rejects the launch as no longer allowed
- **AND** no Pi pane is created
