# provider-support Specification

## Purpose

Defines which model providers and API backends APAS actively supports, how retired providers are blocked, and how existing projects remain recoverable when provider support is removed.
## Requirements
### Requirement: The supported launch catalog is terminal-only
APAS SHALL advertise exactly the supported Claude, Codex, OpenCode, Antigravity, and OMP terminal profiles plus the supported DeepSeek 4.1 Flash terminal profile. Structured `agent:*` profiles, Cursor Agent, Fable, MiniMax, GLM, Pi, DeepSeek Pro, and unknown profiles SHALL NOT be offered for new work.

#### Scenario: User opens a launch surface
- **WHEN** a user opens a new-pane, shared-member default, or policy launch surface
- **THEN** only Claude Terminal, Codex Terminal, OpenCode Terminal, Antigravity Terminal, OMP Terminal, and DeepSeek 4.1 Flash Terminal are presented when allowed by effective project policy
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

#### Scenario: A policy the client holds still lists the Pi profile
- **WHEN** a launch surface reads an effective policy that still lists `terminal:pi:official:default`
- **THEN** it does not present Pi as a choice
- **AND** the other allowed profiles are presented as before

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

### Requirement: Antigravity runs in an interactive terminal

APAS SHALL offer Google Antigravity CLI under `terminal:antigravity:official:default` when permitted by the effective launch policy. The receiving CLI SHALL advertise `terminal_antigravity_v1` before the server routes a launch or provider switch to it.

#### Scenario: Open a tab on a machine missing Antigravity

- **WHEN** the default `agy` executable is absent from PATH and its user install directory on a supported machine
- **THEN** the tab downloads the pinned release for that operating system and architecture, verifies the archive checksum, and starts the installed executable in its terminal
- **AND** a failed verification leaves the destination untouched

#### Scenario: Use an existing installation

- **WHEN** the configured executable is installed and the user opens an Antigravity tab
- **THEN** the host starts that executable in the pane's working directory and passes any initial task as an interactive prompt
- **AND** an unavailable custom executable produces a launch error

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

### Requirement: Pi is retired as a provider
APAS SHALL NOT create a Pi tab, switch a tab to Pi, or restart or resume a Pi tab. The server and the project host SHALL each refuse such a request, whether it names Pi by provider value or by the launch-profile key `terminal:pi:official:default`, with the sentence "Pi is no longer supported. Use OMP instead." A refused request SHALL start no Pi process and SHALL start no other provider in its place. The Pi terminal capability `terminal_pi_v1` SHALL NOT be advertised by a project CLI, and no decision SHALL depend on whether a CLI advertises it. `pi_path` SHALL NOT be a CLI setting.

#### Scenario: A stale web client asks for a new Pi tab
- **WHEN** a stale or modified web client submits a request to create a Pi terminal tab
- **THEN** the server refuses it with "Pi is no longer supported. Use OMP instead."
- **AND** no request is routed to a project host

#### Scenario: An older CLI still advertises the Pi capability
- **WHEN** a request to create a Pi tab arrives for a project whose connected CLI still advertises `terminal_pi_v1`
- **THEN** the server refuses it with the same sentence
- **AND** nothing is routed to that CLI

#### Scenario: A tab is switched to Pi
- **WHEN** a client asks to switch an existing Claude, Codex, OpenCode or OMP tab to Pi
- **THEN** the request is refused with "Pi is no longer supported. Use OMP instead."
- **AND** the tab keeps its provider and keeps running

#### Scenario: An older server routes a Pi launch to an upgraded project host
- **WHEN** an upgraded project host receives a request to create a Pi tab
- **THEN** it refuses the request before starting any process
- **AND** it reports "New pane refused. Pi is no longer supported. Use OMP instead."

#### Scenario: A person asks the CLI for the retired setting
- **WHEN** a person runs the CLI's configuration command to set or read `pi_path`
- **THEN** the command fails with "pi_path is no longer a setting. Pi is no longer supported. Use OMP instead (omp_path)."
- **AND** `pi_path` is not listed among the settings

### Requirement: Stored Pi data still parses
The provider value `pi` SHALL remain valid for reading. A `.apas` file, a stored pane roster, a stored policy, and a frame from an older client that carry it SHALL load without an error and without converting the tab to another provider. Writing such a tab back SHALL keep the value `pi`. A CLI configuration file that still holds `pi_path` SHALL load, and the key SHALL be dropped the next time the file is saved.

#### Scenario: A project file holds a saved Pi tab
- **WHEN** the CLI loads a `.apas` file that holds a Pi terminal tab beside an OMP tab
- **THEN** loading completes and both tabs are listed with their own providers
- **AND** saving the file again keeps the Pi tab with the value `pi`

#### Scenario: An older client sends a frame that names Pi
- **WHEN** an upgraded server or client receives a compatible frame whose provider value is `pi`
- **THEN** the frame parses
- **AND** the receiver does not treat Pi as a supported provider

#### Scenario: A configuration file still holds the Pi path
- **WHEN** the CLI loads a configuration file that holds `pi_path` beside `omp_path`
- **THEN** the file loads and `omp_path` is read as before
- **AND** the next save writes the file without `pi_path`

### Requirement: A stored policy entry for the Pi profile is dropped
On upgrade, the server SHALL remove `terminal:pi:official:default` from every stored launch-profile allowlist and SHALL keep the order and meaning of every other entry. A changed policy SHALL receive a newer version. Running the upgrade again SHALL change nothing. A policy write that lists the Pi profile SHALL be rejected as naming a retired profile. A stored or cached allowlist that still lists the Pi profile SHALL NOT make Pi launchable.

#### Scenario: A stored default policy lists Pi
- **WHEN** the server starts with a stored deployment policy that allows Claude, Pi and OMP terminal profiles
- **THEN** the policy it keeps allows the Claude and OMP profiles, in that order
- **AND** its version is newer than the stored one

#### Scenario: The upgrade runs a second time
- **WHEN** the server starts again with the policy the upgrade already wrote
- **THEN** the policy and its version are unchanged

#### Scenario: A project override lists only Pi
- **WHEN** a stored project override allowlist holds only the Pi profile
- **THEN** the upgrade produces an explicit empty allowlist, not inheritance and not allow-all

#### Scenario: Someone writes the Pi profile back
- **WHEN** an administrator or cluster owner saves a policy that lists `terminal:pi:official:default`
- **THEN** the write is rejected as naming a retired launch profile
- **AND** the stored policy is unchanged

### Requirement: A tab of a retired provider takes no input
The server SHALL refuse a typed message addressed to a tab whose provider is retired, SHALL store nothing for it and SHALL send nothing to a project CLI. For a saved Pi tab the refusal SHALL read "Pi is no longer supported. Use OMP instead. This tab is read-only." For a tab of another retired provider it SHALL read "This pane uses a retired provider and is read-only." Raw terminal keystrokes for such a tab SHALL be dropped without an error for each key. A request to switch the provider or model of a saved Pi tab SHALL be refused with "Tab N uses Pi, which is no longer supported. Close it and open an OMP tab.", where N is the tab's number, and SHALL leave the pane roster unchanged.

#### Scenario: A message is typed to a saved Pi tab
- **WHEN** a client sends a conversation message to a saved Pi tab
- **THEN** the server answers "Pi is no longer supported. Use OMP instead. This tab is read-only."
- **AND** nothing is added to the tab's history and nothing reaches a CLI

#### Scenario: Keys are pressed in a saved Pi tab's terminal
- **WHEN** a client sends raw terminal input for a saved Pi tab
- **THEN** the server drops it and sends no error

#### Scenario: A saved Pi tab is switched to OMP
- **WHEN** a client asks to switch saved Pi tab 9 to OMP
- **THEN** the server answers "Tab 9 uses Pi, which is no longer supported. Close it and open an OMP tab."
- **AND** the roster still lists tab 9 as a Pi tab, no roster is broadcast, and nothing reaches a CLI

#### Scenario: A message is typed to a saved MiniMax or GLM tab
- **WHEN** a client sends a typed message to a tab whose provider is MiniMax or GLM
- **THEN** the server answers "This pane uses a retired provider and is read-only."
- **AND** nothing is stored or sent to a CLI

### Requirement: Retiring Pi leaves OMP unchanged
The retirement of Pi SHALL NOT change OMP. The OMP terminal profile `terminal:omp:official:default` SHALL remain in the supported catalog and in the default policy. An OMP tab SHALL be created, launched, restored and switched to as before, and SHALL still require a project CLI that advertises the OMP terminal capability.

#### Scenario: A user creates an OMP tab after the retirement
- **WHEN** the effective project policy permits the OMP terminal profile and the connected CLI advertises the OMP terminal capability
- **THEN** a request to create an OMP tab is authorized and routed to that CLI

#### Scenario: An older CLI cannot host OMP
- **WHEN** a request to create an OMP tab arrives for a CLI that does not advertise the OMP terminal capability
- **THEN** the server refuses it with an update-and-reconnect error, as before

#### Scenario: An existing tab is switched to OMP
- **WHEN** a client asks to switch a Codex tab to OMP on a capable CLI
- **THEN** the switch is sent to that tab's CLI and the roster shows the tab as OMP

