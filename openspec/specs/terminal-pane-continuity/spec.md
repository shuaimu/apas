# terminal-pane-continuity Specification

## Purpose

Ensures terminal panes preserve their last useful presentation and accurately communicate process state when CLI or browser transports disconnect and reconnect.

## Requirements

### Requirement: Transport loss preserves terminal presentation
The system SHALL treat loss of the CLI transport as distinct from termination of the terminal process. When a connected terminal loses its CLI transport, the system SHALL retain its bounded in-memory scrollback and mark its lifecycle as disconnected rather than discarding its presentation or claiming that the process exited.

#### Scenario: Browser attaches during a CLI transport outage
- **WHEN** a terminal has produced output and its CLI transport disconnects
- **THEN** a browser attaching during the outage receives the retained terminal snapshot and a disconnected lifecycle state

#### Scenario: Same terminal reconnects
- **WHEN** the CLI reconnects and identifies the same terminal process instance as running
- **THEN** the system preserves the retained presentation and updates the lifecycle state to running without replay duplication

#### Scenario: Exited state survives a later transport outage
- **WHEN** a terminal is already recorded as exited and its CLI transport later disconnects
- **THEN** the system retains the exited lifecycle and exit status rather than replacing it with disconnected

### Requirement: Terminal snapshots communicate lifecycle state
Every terminal snapshot SHALL communicate the terminal's last known lifecycle state in addition to its output bytes, sequence position, and truncation indicator. An exited snapshot SHALL include the recorded exit status when one is available, and a disconnected or unknown snapshot SHALL not be presented as confirmed running.

#### Scenario: Terminal exits without a browser attached
- **WHEN** the terminal process exits while no browser is subscribed
- **THEN** a later browser attachment receives the retained screen, an exited lifecycle state, and the recorded exit status

#### Scenario: Terminal exits before producing output
- **WHEN** a terminal process exits before producing any output
- **THEN** a later browser attachment still receives an exited lifecycle state and displays a process-ended indication instead of an unexplained blank pane

#### Scenario: Running terminal attaches normally
- **WHEN** a browser attaches to a terminal whose current process instance is confirmed running
- **THEN** the browser renders the snapshot and presents no disconnected or exited warning

### Requirement: Terminal process instances are reconciled
The system SHALL associate terminal output and lifecycle events with a specific terminal process instance. Reconnection of the same instance SHALL preserve its snapshot, while creation or restoration of a different instance for the same pane SHALL start a fresh presentation and SHALL NOT be overwritten by delayed output or exit events from the prior instance.

#### Scenario: New process replaces retained terminal
- **WHEN** a CLI reports a new terminal process instance for a pane with retained state
- **THEN** the system clears the prior instance's presentation and begins the new instance with an independent sequence

#### Scenario: Delayed exit arrives from replaced process
- **WHEN** an exit event for an older terminal instance arrives after a newer instance is running in the same pane
- **THEN** the system ignores the stale exit for purposes of the current pane lifecycle

#### Scenario: Reconnected process continues output
- **WHEN** the same terminal instance reconnects and emits output after a transport outage
- **THEN** the output continues from the retained instance state without being discarded or confused with a new instance

### Requirement: CLI reconnect reconciles every terminal pane
After establishing or re-establishing a session transport, the CLI SHALL report the current process instance and lifecycle of every configured terminal pane. The server SHALL use that report to resolve retained disconnected state before presenting the pane as running or exited.

#### Scenario: Live PTY survives a WebSocket timeout
- **WHEN** the APAS process and terminal PTY remain alive across a CLI WebSocket timeout
- **THEN** the reconnect report identifies the same running instance and the terminal becomes running without losing its retained screen

#### Scenario: Configured terminal has no live process
- **WHEN** a configured terminal pane has no live process when the CLI reconnects
- **THEN** the reconnect report marks the pane exited or unavailable and later snapshots expose that state

### Requirement: Terminal retention remains bounded and non-durable
The system SHALL enforce the existing per-pane scrollback byte bound while a terminal is connected or disconnected. Raw terminal output and lifecycle continuity state SHALL remain in server memory and SHALL NOT be written to chat history, JSONL session storage, or SQLite as part of this capability.

#### Scenario: Disconnected terminal exceeds the retention limit
- **WHEN** retained output for a disconnected terminal exceeds the configured byte limit
- **THEN** the system evicts the oldest bytes, preserves the newest bounded snapshot, and marks the snapshot as truncated

#### Scenario: Server restarts
- **WHEN** the APAS server process restarts
- **THEN** no raw terminal snapshot is recovered from durable storage and terminal lifecycle remains unknown until reconciled by a CLI report

### Requirement: Lifecycle metadata is rollout-compatible
Terminal protocol participants SHALL tolerate peers that omit terminal instance or lifecycle metadata during a rolling upgrade. Missing metadata SHALL degrade to an unknown lifecycle and SHALL NOT cause message rejection or falsely confirm a terminal as running.

#### Scenario: New web client receives a legacy snapshot
- **WHEN** a new web client receives a terminal snapshot without lifecycle metadata from an older server
- **THEN** the client renders any available bytes and treats the lifecycle as unknown

#### Scenario: New server receives legacy terminal output
- **WHEN** a new server receives terminal output without process-instance metadata from an older CLI
- **THEN** the server accepts the output using compatibility handling without treating it as proof of a reconnecting instance's identity

### Requirement: OpenCode terminal conversations are recovered from retained sessions
For an OpenCode terminal pane, the project host SHALL recover conversation turns from an OpenCode session whose recorded directory exactly matches the pane working directory. It SHALL expose real user and completed assistant text as pane conversation history, exclude synthetic, ignored, reasoning, tool, and attachment parts, and preserve reported model and token usage when available.

#### Scenario: OpenCode session exists for the pane directory
- **WHEN** the transcript watcher finds one or more retained OpenCode sessions whose directory exactly matches the pane working directory
- **THEN** it selects the most recently updated matching session and exports its conversation
- **AND** it does not select a session belonging to another directory

#### Scenario: Export contains internal OpenCode parts
- **WHEN** an exported conversation includes reasoning, tool, synthetic, ignored, or attachment parts alongside human-visible text
- **THEN** APAS emits only real user and assistant text into conversation history
- **AND** internal parts are not exposed as conversation messages

#### Scenario: Assistant response is still streaming
- **WHEN** an exported OpenCode assistant message has not reached its recorded completion boundary
- **THEN** APAS withholds that assistant message from persisted conversation history
- **AND** a later poll may emit the complete response without leaving an unrecoverable partial turn

#### Scenario: Final OpenCode response reports usage
- **WHEN** a completed assistant response contains model and token usage metadata
- **THEN** APAS attributes that metadata to the owning pane and completion boundary
- **AND** the terminal pane becomes idle only at a final non-tool-call completion

### Requirement: OpenCode terminal restoration continues retained work
When an APAS CLI process restores a persisted OpenCode terminal pane, it SHALL start a new PTY process using OpenCode's continuation behavior and SHALL NOT replay the pane's original initial instruction.

#### Scenario: APAS CLI restores an OpenCode terminal pane
- **WHEN** the APAS CLI restarts with a persisted OpenCode terminal pane
- **THEN** it re-executes the configured OpenCode CLI in the pane working directory using continuation mode
- **AND** it reports the new terminal process instance through normal lifecycle reconciliation
- **AND** it does not submit the original prompt again
