## MODIFIED Requirements

### Requirement: The supported launch catalog is terminal-only
APAS SHALL advertise exactly the supported Claude, Codex, OpenCode, and OMP terminal profiles plus the supported DeepSeek 4.1 Flash terminal profile. Structured `agent:*` profiles, Cursor Agent, Fable, MiniMax, GLM, Pi, DeepSeek Pro, and unknown profiles SHALL NOT be offered for new work.

#### Scenario: User opens a launch surface
- **WHEN** a user opens a new-pane, shared-member default, or policy launch surface
- **THEN** only Claude Terminal, Codex Terminal, OpenCode Terminal, OMP Terminal, and DeepSeek 4.1 Flash Terminal are presented when allowed by effective project policy
- **AND** no structured agent profile is presented

#### Scenario: Operator reviews supported launch profiles
- **WHEN** the system administrator or a cluster owner reads the supported-profile catalog or edits launch policy
- **THEN** every catalog key begins with `terminal:`
- **AND** the catalog contains exactly the five supported terminal profiles

#### Scenario: Persisted structured-agent policy is upgraded
- **WHEN** stored deployment, cluster, project, member-default, or provisioning policy contains a structured-agent launch profile
- **THEN** Claude, Codex, OpenCode, and supported DeepSeek profiles are mapped to their terminal equivalents
- **AND** Fable, Cursor Agent, retired, and unknown profiles are removed
- **AND** policy ordering, explicit empty allowlists, and version monotonicity are preserved

#### Scenario: A policy the client holds still lists the Pi profile
- **WHEN** a launch surface reads an effective policy that still lists `terminal:pi:official:default`
- **THEN** it does not present Pi as a choice
- **AND** the other allowed profiles are presented as before

## ADDED Requirements

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

## REMOVED Requirements

### Requirement: Pi is available as a policy-controlled terminal provider
**Reason**: Pi was retired as a provider on 2026-11-13. OMP, which is derived from Pi, is the provider in use, and keeping both meant two launch paths, two session stores and two sets of rollout gates for one family of agent.
**Migration**: Open an OMP tab instead. A saved Pi tab stays listed with its history until someone closes it. "Pi is retired as a provider" states what a request for Pi is answered with.

### Requirement: Pi launches preserve the host's trust boundary
**Reason**: APAS no longer launches Pi, so it passes Pi no flag at all.
**Migration**: None. OMP is launched with its own approval flag, as before.

### Requirement: Pi terminal launches use the provider's exact session identity
**Reason**: APAS no longer launches or restores Pi, and no longer looks for Pi session files.
**Migration**: None. A saved Pi tab is never launched; see "A saved Pi terminal tab is never launched" in `terminal-pane-continuity`. An OMP tab is still pinned by its private session directory.
