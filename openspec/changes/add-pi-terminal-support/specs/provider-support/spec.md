## ADDED Requirements

### Requirement: Pi is available as a policy-controlled terminal provider
APAS SHALL offer Pi as a user-created terminal provider on supported launch surfaces when `terminal:pi:official:default` is permitted by the effective project policy. The project host SHALL run the configured Pi interactive CLI, SHALL deliver a fresh launch instruction through Pi's supported initial-message interface, and SHALL use Pi's exact session identity when launching and restoring the pane.

#### Scenario: User creates an allowed Pi terminal
- **WHEN** the effective project policy permits the Pi terminal profile and a user selects Pi from a terminal launch surface
- **THEN** APAS creates an unmanaged terminal pane that hosts the real Pi interactive CLI
- **AND** the pane receives terminal input, resize, lifecycle, and output through the same terminal transport used by other supported providers

#### Scenario: Project policy disallows Pi terminal launch
- **WHEN** a user attempts to create a Pi terminal while its launch profile is absent from the effective project allowlist
- **THEN** the server and project host reject the launch
- **AND** no Pi process is spawned

#### Scenario: Pi binary is unavailable
- **WHEN** an authorized Pi terminal launch reaches a project host whose configured Pi binary cannot be executed
- **THEN** the pane reports an actionable spawn error identifying the configured binary
- **AND** other panes in the project remain available

### Requirement: Pi launches preserve the host's trust boundary
APAS SHALL NOT automatically grant Pi trust over project-local resources. A Pi terminal launch SHALL NOT pass an approval flag that ignores the provider's project trust decision, so project-local settings and extensions remain subject to Pi's own trust flow.

#### Scenario: Project-local Pi resources are present
- **WHEN** a Pi terminal launches in a directory containing project-local Pi settings or extensions that the host has not trusted
- **THEN** APAS does not represent those resources as trusted on the host's behalf
- **AND** Pi's own trust behavior determines whether they load

### Requirement: Pi terminal launches use the provider's exact session identity
A fresh Pi terminal launch SHALL pin the pane's conversation identity as the provider session identity, and a restored pane SHALL open the exact retained session rather than selecting a session by directory recency. APAS SHALL NOT associate a Pi pane with a session belonging to another pane or working directory.

#### Scenario: Fresh Pi terminal launch
- **WHEN** APAS starts a new Pi terminal pane
- **THEN** the launched Pi process receives the pane's conversation identity as its exact session identity
- **AND** subsequent recovery reads the session file carrying that identity

#### Scenario: Pi session file is retained
- **WHEN** APAS restores a persisted Pi terminal pane whose exact session file exists
- **THEN** it opens that exact session file
- **AND** it does not choose a session by directory modification time

#### Scenario: Pi session file is absent
- **WHEN** APAS restores a persisted Pi terminal pane whose exact session file no longer exists
- **THEN** it starts Pi with the same pinned session identity, creating a new session under that identity
- **AND** it does not adopt an unrelated session in the same directory
