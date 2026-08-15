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
After establishing or re-establishing a session transport, including after project CLI process replacement, the CLI SHALL discover or adopt each configured terminal pane and report its current process instance, lifecycle, and output sequence. The server SHALL use that report to resolve retained disconnected state before presenting the pane as running, exited, or replaced.

#### Scenario: Live PTY survives a WebSocket timeout
- **WHEN** the APAS process and terminal PTY remain alive across a CLI WebSocket timeout
- **THEN** the reconnect report identifies the same running instance and the terminal becomes running without losing its retained screen

#### Scenario: Replacement CLI adopts a persistent runtime
- **WHEN** an APAS project CLI process is replaced while a terminal runtime remains alive
- **THEN** the replacement adopts and reports the same running instance and its latest output sequence
- **AND** the server preserves the retained presentation before replaying newer output

#### Scenario: Configured terminal has no live process
- **WHEN** a configured terminal pane has no live or adoptable process when the CLI reconnects
- **THEN** the reconnect report marks the pane exited or unavailable and later snapshots expose that state

### Requirement: Terminal retention remains bounded and non-durable
The system SHALL enforce configured per-pane byte bounds for server scrollback and persistent-runtime detached output. Raw terminal output and lifecycle continuity state SHALL remain in volatile server or host-runtime memory and SHALL NOT be written to chat history, JSONL session storage, SQLite, project source files, or an unencrypted durable spool as part of this capability.

#### Scenario: Disconnected terminal exceeds the retention limit
- **WHEN** retained server output for a disconnected terminal exceeds its configured byte limit
- **THEN** the system evicts the oldest bytes, preserves the newest bounded snapshot, and marks the snapshot as truncated

#### Scenario: Detached runtime exceeds its output limit
- **WHEN** terminal output produced without an attached project CLI exceeds the host-runtime byte limit
- **THEN** the runtime evicts the oldest output and reports the truncation boundary during adoption

#### Scenario: Server restarts
- **WHEN** the APAS server process restarts
- **THEN** no raw terminal snapshot is recovered from durable server storage and terminal lifecycle remains unknown until reconciled by a CLI report

#### Scenario: Host machine restarts
- **WHEN** the machine hosting the terminal runtime restarts
- **THEN** raw detached output is not recovered from disk
- **AND** the project CLI uses explicit unavailable or restart-and-resume behavior rather than claiming live adoption

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

### Requirement: Supported terminal runtimes survive project CLI replacement
The project host SHALL keep live unmanaged Claude, Codex, and OpenCode terminal runtimes outside the replaceable project CLI process. A successful full CLI reboot SHALL preserve the provider process, PTY state, terminal process-instance identifier, active turn, and descendant processes, then reconnect the replacement CLI to that same runtime.

#### Scenario: CLI reboots while a terminal agent is working
- **WHEN** a supported terminal pane is running a turn during a full project CLI reboot
- **THEN** the provider process and its descendants continue without receiving EOF, hangup, interrupt, or termination from the reboot
- **AND** the replacement CLI adopts the same terminal process instance

#### Scenario: Multiple terminal panes are active
- **WHEN** a project with multiple supported terminal panes performs a full CLI reboot
- **THEN** each live terminal runtime remains isolated and independently adoptable
- **AND** adopting one pane does not replace, merge, or terminate another pane

#### Scenario: Legacy structured pane is present
- **WHEN** a project contains both persistent terminal panes and legacy structured agent panes during a full CLI reboot
- **THEN** the system preserves supported terminal panes
- **AND** applies the existing restart or resume behavior to structured panes without claiming they were live-adopted

### Requirement: Detached terminal output is replayed without gaps or duplicates
The persistent terminal runtime SHALL retain a bounded sequence of raw output while no project CLI is attached. After adoption, the replacement CLI SHALL reconcile the runtime's stable instance identifier and sequence position, replay output newer than the last acknowledged position, and resume live streaming in order.

#### Scenario: Output is produced during CLI replacement
- **WHEN** a terminal emits output after the old CLI detaches and before the replacement CLI adopts it
- **THEN** the replacement receives the retained output in sequence before later live output
- **AND** the browser can reconstruct the current terminal without an unexplained gap

#### Scenario: Adoption retries after an uncertain response
- **WHEN** a CLI repeats adoption because acknowledgement was lost
- **THEN** the runtime uses the caller's acknowledged sequence to avoid duplicating already accepted output
- **AND** the same runtime remains exclusively owned

#### Scenario: Detached output exceeds its bound
- **WHEN** output produced while detached exceeds the runtime retention limit
- **THEN** the oldest detached output is evicted
- **AND** adoption reports truncation and supplies the newest retained state or snapshot

### Requirement: Terminal adoption is exclusive and authenticated
Only the locally authorized project CLI for the matching project and pane SHALL be allowed to adopt or control a persistent terminal runtime. Adoption SHALL validate project identity, pane identity, runtime identity, owner credentials, protocol compatibility, and host-local permissions, and one runtime SHALL have at most one active controlling CLI connection.

#### Scenario: Replacement CLI adopts its project runtime
- **WHEN** a replacement CLI presents valid host-local credentials and matching project, pane, and runtime identities
- **THEN** the runtime grants exclusive control and reports its current lifecycle and sequence

#### Scenario: Stale CLI attempts adoption
- **WHEN** an older or duplicate CLI attempts to adopt a runtime already controlled by the current project CLI
- **THEN** the runtime rejects the stale controller
- **AND** does not interrupt the provider process or current controller

#### Scenario: Another local project discovers the endpoint
- **WHEN** a CLI for a different project attempts to inspect, adopt, write to, resize, or stop the runtime
- **THEN** the operation is rejected without exposing terminal content
- **AND** the runtime continues unchanged

### Requirement: Persistent terminal runtimes have explicit cleanup ownership
Persistence across CLI replacement SHALL NOT make a terminal outlive its pane or project. Closing a pane, intentionally stopping the project runtime, or deleting the project SHALL stop the matching provider process and descendants, remove retained output and local credentials, and leave no adoptable runtime. Unexpected CLI loss SHALL instead enter a bounded detached state so a replacement can adopt it.

#### Scenario: User closes a persistent terminal pane
- **WHEN** the authorized project CLI processes a pane-close operation
- **THEN** the persistent runtime terminates the pane's process tree and removes its local state
- **AND** later CLI startup does not adopt or recreate that closed pane

#### Scenario: Project runtime is intentionally stopped or deleted
- **WHEN** APAS intentionally stops or deletes a project
- **THEN** every persistent terminal runtime for that project is terminated before lifecycle cleanup completes
- **AND** no terminal process or retained output remains adoptable

#### Scenario: Project CLI disappears unexpectedly
- **WHEN** the controlling CLI exits or crashes without an authenticated stop instruction
- **THEN** the terminal runtime remains detached and running for the configured adoption grace period
- **AND** a replacement CLI can adopt it without provider restart

#### Scenario: Detached runtime exceeds its adoption grace period
- **WHEN** no authorized CLI adopts a detached runtime before its configured deadline
- **THEN** the runtime terminates the provider process tree and erases its local state
- **AND** reports no future claim that the abandoned process is still running

### Requirement: Missing persistent runtimes fall back safely
If a configured terminal pane has no valid persistent runtime during CLI startup, APAS SHALL report that preservation was unavailable and MAY start a replacement provider process using the pane's existing provider-specific continuation behavior. The replacement SHALL use a new terminal process-instance identifier and SHALL NOT be represented as the original live process.

#### Scenario: Runtime died before CLI returned
- **WHEN** the replacement CLI finds the configured runtime absent or exited
- **THEN** it reports the prior lifecycle and starts a replacement only through the existing restoration policy
- **AND** the server clears the old terminal presentation before accepting the replacement instance

#### Scenario: Persistent hosting is unavailable on the machine
- **WHEN** the host cannot provide a validated persistent runtime mechanism
- **THEN** new terminal panes remain usable through the existing CLI-owned PTY path
- **AND** full reboot warns that those panes will restart rather than claiming preservation
