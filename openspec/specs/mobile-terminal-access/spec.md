# mobile-terminal-access Specification

## Purpose

Defines secure and reliable mobile access to APAS terminal panes while preserving terminal state, authorization, and input semantics across app lifecycle changes.

## Requirements

### Requirement: Mobile users can attach to authorized terminal panes
The mobile application SHALL allow an authorized project user to open a terminal pane and request its current APAS-managed snapshot and lifecycle state. Terminal content SHALL render only in a trusted application-owned terminal surface that cannot navigate to arbitrary remote content or access the user's long-lived device credential.

#### Scenario: User opens a running terminal pane
- **WHEN** an authorized user opens a terminal pane from a coding session
- **THEN** the client attaches to the exact session and pane
- **AND** renders the latest snapshot before applying newer live output

#### Scenario: User opens an unauthorized terminal pane
- **WHEN** a user without current project access attempts to attach through a stale route or deep link
- **THEN** the server rejects the attach
- **AND** no terminal snapshot, credential, or pane metadata is disclosed

#### Scenario: Terminal surface attempts external navigation
- **WHEN** terminal content or user interaction requests navigation outside the trusted bundled surface
- **THEN** the embedded surface blocks the navigation
- **AND** any allowed external link requires explicit user action outside the credential-bearing terminal context

### Requirement: Snapshot and live terminal output reconcile without duplication
The mobile terminal SHALL honor APAS terminal sequence and instance identifiers when reconciling snapshots, live output, restarts, and truncation. It SHALL neither replay already-rendered output nor combine output from different terminal process instances.

#### Scenario: Live output races with initial snapshot
- **WHEN** live output is received before the attach snapshot finishes rendering
- **THEN** the client buffers and applies only output newer than the snapshot sequence
- **AND** each byte sequence is rendered once

#### Scenario: Snapshot begins after retained scrollback was truncated
- **WHEN** the server marks the snapshot as truncated
- **THEN** the client resets unsafe partial terminal parser state before rendering it
- **AND** indicates that older scrollback is unavailable

#### Scenario: Terminal process restarts
- **WHEN** output arrives for a new terminal instance identifier
- **THEN** the client does not merge it into the prior instance as continuous output
- **AND** refreshes the terminal state for the new instance

### Requirement: Mobile terminal input and resize target the exact pane
While online and authorized, the mobile client SHALL forward terminal keystrokes, paste operations, and viewport dimensions to the exact attached session and pane. It SHALL expose touch-accessible controls for terminal keys that are difficult to produce with a mobile keyboard.

#### Scenario: User types in an attached terminal
- **WHEN** the terminal is focused and the user enters text or a supported control key
- **THEN** the resulting terminal bytes are sent to the exact attached pane
- **AND** are not interpreted as a coding-session instruction or sent to another pane

#### Scenario: Device rotates or terminal view changes size
- **WHEN** the measured terminal viewport changes to valid non-zero dimensions
- **THEN** the client reports the new columns and rows to APAS
- **AND** does not emit degenerate resize events for hidden or unmeasured views

#### Scenario: User pastes clipboard text
- **WHEN** the user explicitly confirms a terminal paste
- **THEN** the selected text is delivered as terminal input
- **AND** the application does not read or transmit clipboard contents without that user action

### Requirement: Terminal lifecycle is visible and constrains interaction
The terminal surface SHALL distinguish running, disconnected, exited, and unknown lifecycle states using server snapshots and live events. Input SHALL be enabled only while the pane is online, authorized, and capable of accepting it.

#### Scenario: CLI transport disconnects but process state is retained
- **WHEN** APAS reports the terminal as disconnected with retained scrollback
- **THEN** the client keeps the rendered terminal visible
- **AND** disables input while explaining that reconnection is pending

#### Scenario: Terminal process exits
- **WHEN** APAS reports an exited lifecycle and optional status
- **THEN** the client preserves the final output
- **AND** displays the exit state without pretending the pane is still interactive

### Requirement: Mobile terminal recovery follows app reconnect semantics
When the application foregrounds or reconnects, every visible terminal SHALL request a fresh attach snapshot and reconcile it against the last accepted instance and sequence. Terminal input SHALL never be queued for later replay while offline or backgrounded.

#### Scenario: App backgrounds while terminal is open
- **WHEN** the operating system suspends the application or its socket
- **THEN** the client immediately treats terminal input as unavailable
- **AND** does not retain typed keystrokes for automatic later delivery

#### Scenario: App returns to an open terminal
- **WHEN** the application reconnects and the user still has access
- **THEN** it requests a fresh terminal snapshot and lifecycle state
- **AND** resumes live output without duplicating already accepted sequences

#### Scenario: Access is revoked while app is backgrounded
- **WHEN** terminal reattach fails because project access changed
- **THEN** the client clears terminal content from active and persisted mobile state
- **AND** navigates away from the inaccessible session
