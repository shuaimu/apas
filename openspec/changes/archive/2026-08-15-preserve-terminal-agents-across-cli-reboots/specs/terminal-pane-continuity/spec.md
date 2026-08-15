## ADDED Requirements

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

## MODIFIED Requirements

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
