## ADDED Requirements

### Requirement: Mobile task launch supports capable OpenCode project hosts
The mobile task-launch catalog SHALL include `terminal:opencode:official:default` when effective project policy permits it. A mobile OpenCode task SHALL be routed only to a connected project CLI that advertises both the current mobile task-launch capability and explicit OpenCode terminal capability.

#### Scenario: Capable project host launches an OpenCode mobile task
- **WHEN** an authorized mobile user selects an allowed OpenCode terminal profile and the connected project CLI advertises OpenCode terminal capability
- **THEN** the server creates an OpenCode terminal pane for the exact project
- **AND** passes the submitted instruction as the pane's initial OpenCode prompt
- **AND** acknowledges the retained mobile launch operation only after the pane is reported

#### Scenario: Older project host lacks OpenCode terminal capability
- **WHEN** an authorized mobile user selects OpenCode but the connected project CLI does not advertise OpenCode terminal capability
- **THEN** the server rejects the launch with an update-and-reconnect error
- **AND** does not route an OpenCode pane request or report the operation as successful

#### Scenario: Effective policy removes OpenCode before submission
- **WHEN** a stale mobile catalog still shows OpenCode but the effective project policy no longer permits its profile at submission time
- **THEN** the server rejects the launch as no longer allowed
- **AND** no OpenCode pane is created
