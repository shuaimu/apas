## Purpose

Defines which model providers and API backends APAS actively supports, how retired providers are blocked, and how existing projects remain recoverable when provider support is removed.

## ADDED Requirements

### Requirement: The supported provider catalog excludes MiniMax and GLM
APAS SHALL NOT advertise MiniMax or GLM as a supported provider, API backend, model, launch profile, managed-team choice, machine capability, or usage source. All other currently supported choices, including Claude, Codex, DeepSeek, OpenCode, Cursor Agent, and supported terminal profiles, SHALL remain available according to project policy.

#### Scenario: User opens a launch surface
- **WHEN** a user opens any pane, team-member, model, provider, or backend selection surface
- **THEN** no MiniMax or GLM option is presented
- **AND** the remaining supported choices continue to be presented when allowed by effective project policy

#### Scenario: Administrator reviews supported launch profiles
- **WHEN** a cluster administrator reads the supported-profile catalog or edits launch policy
- **THEN** the catalog contains no MiniMax or GLM profile key
- **AND** no retired profile can be newly selected

### Requirement: Retired provider launches fail closed at every boundary
The server and project host SHALL independently reject any pane creation, restart, resume, model switch, backend switch, team start, or team-member addition that identifies MiniMax or GLM through a provider value, backend value, model name, or retired launch-profile key. The system SHALL NOT silently map a retired configuration to another provider.

#### Scenario: Stale web client requests a retired backend
- **WHEN** a stale or modified web client submits a launch or switch request for MiniMax or GLM
- **THEN** the server rejects the request with an explicit unsupported-provider error
- **AND** no request is routed to a project host

#### Scenario: Mixed-version server routes a retired launch
- **WHEN** an upgraded project host receives a MiniMax or GLM launch request from an older server
- **THEN** the project host rejects the request before spawning a process
- **AND** no Claude, Codex, DeepSeek, or terminal fallback is launched

#### Scenario: Existing retired pane is restarted
- **WHEN** a user attempts to restart, resume, or change the model of a persisted MiniMax or GLM pane
- **THEN** the operation is rejected as unsupported
- **AND** the pane remains stopped

#### Scenario: Retired pane is running during upgrade
- **WHEN** an upgraded project host discovers a running MiniMax or GLM pane
- **THEN** it gracefully interrupts and stops that pane
- **AND** it leaves supported panes in the same project running

### Requirement: Legacy project state remains readable without retaining provider support
The upgraded system SHALL safely read persisted project and compatible wire data that contains historical MiniMax or GLM values. It SHALL identify affected panes as unsupported and non-relaunchable while keeping unrelated panes and project administration usable.

#### Scenario: Project contains a historical retired pane
- **WHEN** the CLI loads a `.apas` project containing a MiniMax or GLM provider or model
- **THEN** project loading completes without a panic or silent provider conversion
- **AND** the affected pane is reported as unsupported and stopped
- **AND** other supported panes can still run

#### Scenario: Upgraded peer receives legacy status data
- **WHEN** an upgraded peer receives a compatible legacy message containing MiniMax or GLM machine, pane, or usage state
- **THEN** it handles the message without disconnecting or exposing a launch control
- **AND** it does not treat the retired provider as supported

### Requirement: Persisted launch policy is normalized away from retired profiles
On upgrade, APAS SHALL remove all MiniMax and GLM profile keys from cluster-default and project-override allowlists while preserving the order and meaning of every remaining supported entry. A changed policy SHALL receive a newer policy version and SHALL be distributed through the normal effective-policy channel.

#### Scenario: Default policy includes retired profiles
- **WHEN** the server migrates a cluster-default allowlist containing MiniMax or GLM profile keys
- **THEN** it removes only the retired keys
- **AND** increments the policy version
- **AND** retains all supported keys

#### Scenario: Project override contains only retired profiles
- **WHEN** a project override allowlist contains only MiniMax or GLM profile keys
- **THEN** migration produces an explicit empty allowlist rather than interpreting it as allow-all or inheritance
- **AND** the effective policy permits no agent launch until an administrator changes the override

#### Scenario: Policy is already free of retired profiles
- **WHEN** a cluster or project policy contains no MiniMax or GLM profile key
- **THEN** migration leaves its contents and version unchanged

### Requirement: Retired credentials and telemetry are inert
The upgraded CLI, daemon, server, and web application SHALL NOT request, read for runtime use, transmit, display, refresh, cache, or report MiniMax or GLM credentials, backend status, or usage limits. Legacy credential fields that remain in an older local configuration file SHALL be ignored and SHALL NOT be logged or copied during migration.

#### Scenario: Existing machine config contains a retired credential
- **WHEN** an upgraded CLI or daemon loads configuration containing a MiniMax or GLM API key
- **THEN** it does not use or transmit the credential
- **AND** it continues loading supported configuration

#### Scenario: User opens machine configuration and usage views
- **WHEN** a user views machine settings or provider usage after the upgrade
- **THEN** no MiniMax or GLM configuration, status, quota, or usage control is displayed

#### Scenario: Older peer sends a retired configuration mutation
- **WHEN** an older peer attempts to update MiniMax or GLM machine configuration
- **THEN** the upgraded receiver rejects or safely ignores the mutation without changing supported machine configuration
- **AND** it does not echo any supplied credential

### Requirement: Provider removal is verifiable across product surfaces
The release SHALL include compatibility and regression coverage proving that retired providers cannot be selected or launched and that supported providers remain functional across shared protocol, CLI, daemon, server, and web boundaries.

#### Scenario: Removal verification runs
- **WHEN** the complete Rust and web verification suites run against the upgraded code
- **THEN** tests cover retired-value rejection, policy normalization, legacy-state loading, credential inertness, and absence from user-facing catalogs
- **AND** the existing Claude, Codex, DeepSeek, OpenCode, Cursor Agent, and terminal launch tests continue to pass
