# provider-support Specification

## Purpose

Defines which model providers and API backends APAS actively supports, how retired providers are blocked, and how existing projects remain recoverable when provider support is removed.

## Requirements

### Requirement: The supported launch catalog is terminal-only
APAS SHALL advertise exactly the supported Claude, Codex, and OpenCode terminal profiles plus the supported DeepSeek Pro and Flash terminal profiles. Structured `agent:*` profiles, Cursor Agent, Fable, MiniMax, GLM, and unknown profiles SHALL NOT be offered for new work.

#### Scenario: User opens a launch surface
- **WHEN** a user opens a new-pane, shared-member default, or policy launch surface
- **THEN** only Claude Terminal, Codex Terminal, OpenCode Terminal, DeepSeek Pro Terminal, and DeepSeek Flash Terminal are presented when allowed by effective project policy
- **AND** no structured agent profile is presented

#### Scenario: Administrator reviews supported launch profiles
- **WHEN** a cluster administrator reads the supported-profile catalog or edits launch policy
- **THEN** every catalog key begins with `terminal:`
- **AND** the catalog contains exactly the five supported terminal profiles

#### Scenario: Persisted structured-agent policy is upgraded
- **WHEN** stored deployment, cluster, project, member-default, or provisioning policy contains a structured-agent launch profile
- **THEN** Claude, Codex, OpenCode, and supported DeepSeek profiles are mapped to their terminal equivalents
- **AND** Fable, Cursor Agent, retired, and unknown profiles are removed
- **AND** policy ordering, explicit empty allowlists, and version monotonicity are preserved

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
- **AND** the supported Claude, Codex, DeepSeek, and OpenCode terminal launch tests continue to pass

### Requirement: OpenCode is available as a policy-controlled terminal provider
APAS SHALL offer OpenCode as a user-created terminal provider on supported launch surfaces when `terminal:opencode:official:default` is permitted by the effective project policy. The project host SHALL run the configured OpenCode interactive CLI with a non-blocking permission mode, SHALL deliver a fresh launch instruction using OpenCode's supported initial-prompt interface, and SHALL use OpenCode's continuation interface when restoring the pane.

#### Scenario: User creates an allowed OpenCode terminal
- **WHEN** the effective project policy permits the OpenCode terminal profile and a user selects OpenCode from a terminal launch surface
- **THEN** APAS creates an unmanaged terminal pane that hosts the real OpenCode interactive CLI
- **AND** the pane receives terminal input, resize, lifecycle, and output through the same terminal transport used by other supported providers

#### Scenario: Project policy disallows OpenCode terminal launch
- **WHEN** a user attempts to create an OpenCode terminal while its launch profile is absent from the effective project allowlist
- **THEN** the server and project host reject the launch
- **AND** no OpenCode process is spawned

#### Scenario: OpenCode binary is unavailable
- **WHEN** an authorized OpenCode terminal launch reaches a project host whose configured OpenCode binary cannot be executed
- **THEN** the pane reports an actionable spawn error identifying the configured binary
- **AND** other panes in the project remain available

### Requirement: Retained headless OpenCode panes use native OpenCode event semantics
For persisted legacy panes that still use the structured agent path, APAS SHALL invoke OpenCode with its supported non-interactive JSON interface and SHALL translate OpenCode text, tool-use, completion, usage, and error events into the shared pane message model. APAS SHALL NOT pass an APAS UUID as though it were an OpenCode-generated session identifier.

#### Scenario: Headless OpenCode emits text and completion events
- **WHEN** a retained structured OpenCode pane emits native JSON text followed by a final completion event
- **THEN** APAS displays the assistant text and records a successful turn completion
- **AND** preserves reported token usage and cost when present

#### Scenario: OpenCode emits an intermediate tool-call completion
- **WHEN** OpenCode finishes a tool-call step but continues executing the same user turn
- **THEN** APAS does not mark the pane idle at that intermediate boundary
- **AND** waits for a final assistant completion or error

#### Scenario: Headless OpenCode resumes previous work
- **WHEN** a retained structured OpenCode pane continues after its first invocation
- **THEN** APAS uses OpenCode's continuation behavior for the pane working directory
- **AND** does not submit the APAS pane session UUID as an OpenCode session ID
