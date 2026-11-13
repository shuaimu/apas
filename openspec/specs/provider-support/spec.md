# provider-support Specification

## Purpose

Defines which model providers and API backends APAS actively supports, how retired providers are blocked, and how existing projects remain recoverable when provider support is removed.
## Requirements
### Requirement: The supported launch catalog is terminal-only
APAS SHALL advertise exactly the supported Claude, Codex, OpenCode, Pi, and OMP terminal profiles plus the supported DeepSeek 4.1 Flash terminal profile. Structured `agent:*` profiles, Cursor Agent, Fable, MiniMax, GLM, DeepSeek Pro, and unknown profiles SHALL NOT be offered for new work.

#### Scenario: User opens a launch surface
- **WHEN** a user opens a new-pane, shared-member default, or policy launch surface
- **THEN** only Claude Terminal, Codex Terminal, OpenCode Terminal, Pi Terminal, OMP Terminal, and DeepSeek 4.1 Flash Terminal are presented when allowed by effective project policy
- **AND** no structured agent profile is presented

#### Scenario: Operator reviews supported launch profiles
- **WHEN** the system administrator or a cluster owner reads the supported-profile catalog or edits launch policy
- **THEN** every catalog key begins with `terminal:`
- **AND** the catalog contains exactly the six supported terminal profiles

#### Scenario: Persisted structured-agent policy is upgraded
- **WHEN** stored deployment, cluster, project, member-default, or provisioning policy contains a structured-agent launch profile
- **THEN** Claude, Codex, OpenCode, and supported DeepSeek profiles are mapped to their terminal equivalents
- **AND** Fable, Cursor Agent, retired, and unknown profiles are removed
- **AND** policy ordering, explicit empty allowlists, and version monotonicity are preserved

### Requirement: Retired provider launches fail closed at every boundary
The server and project host SHALL independently reject any pane creation, restart, resume, model switch, or backend switch that identifies MiniMax or GLM through a provider value, backend value, model name, or retired launch-profile key. The system SHALL NOT silently map a retired configuration to another provider.

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
- **THEN** APAS creates a terminal pane that hosts the real OpenCode interactive CLI
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

### Requirement: DeepSeek is one model reached through the Claude runtime
APAS SHALL support DeepSeek as one model, `deepseek-flash`, labelled DeepSeek 4.1 Flash. It SHALL be reached through the Claude runtime, one Anthropic-compatible DeepSeek endpoint and one machine-level DeepSeek API key. The generic DeepSeek choice SHALL resolve to that model. The older identifier `deepseek-v4-flash` SHALL be accepted and stored as `deepseek-flash`. Every Claude Code model slot, including the small, Sonnet, Opus and subagent slots, SHALL resolve to `deepseek-flash`. DeepSeek Pro (`deepseek-v4-pro`) is withdrawn. APAS SHALL NOT offer or launch it and SHALL NOT substitute another model for it. APAS SHALL validate the requested model before launch and SHALL NOT rely on the upstream API's fallback for unknown model names.

#### Scenario: User launches the default DeepSeek choice
- **WHEN** a user selects the Claude / DeepSeek choice without naming a model
- **THEN** APAS launches Claude Code with `deepseek-flash` as the primary model
- **AND** Claude Code's small and subagent model routing also uses `deepseek-flash`

#### Scenario: A pane stored under the older Flash identifier
- **WHEN** a pane whose stored model is `deepseek-v4-flash` is launched or restored
- **THEN** APAS launches it with `deepseek-flash` against the same endpoint and API key

#### Scenario: An existing pane uses DeepSeek Pro
- **WHEN** a pane whose stored model is `deepseek-v4-pro` is displayed or relaunched
- **THEN** the pane is shown as an unsupported provider and keeps its history
- **AND** the launch fails with a message that the model was withdrawn
- **AND** no other model is started in its place

#### Scenario: Requested DeepSeek model is unknown
- **WHEN** a stale or modified client requests an unknown DeepSeek model identifier
- **THEN** the server or project host rejects the launch or switch
- **AND** APAS does not silently run another model as a fallback

### Requirement: Pi is available as a policy-controlled terminal provider
APAS SHALL offer Pi as a user-created terminal provider on supported launch surfaces when `terminal:pi:official:default` is permitted by the effective project policy. The project host SHALL run the configured Pi interactive CLI, SHALL deliver a fresh launch instruction through Pi's supported initial-message interface, and SHALL use Pi's exact session identity when launching and restoring the pane.

#### Scenario: User creates an allowed Pi terminal
- **WHEN** the effective project policy permits the Pi terminal profile and a user selects Pi from a terminal launch surface
- **THEN** APAS creates a terminal pane that hosts the real Pi interactive CLI
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

