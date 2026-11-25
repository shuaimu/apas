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
The project host SHALL keep every live terminal runtime it can host persistently, for the Claude, Codex, OpenCode, Antigravity and OMP providers, outside the replaceable project CLI process. A successful full CLI reboot SHALL preserve the provider process, PTY state, terminal process-instance identifier, active turn, and descendant processes, then reconnect the replacement CLI to that same runtime. A terminal tab of a retired provider SHALL NOT be among the runtimes the project host keeps: when a project starts, a persistent runtime left for such a tab by an older CLI SHALL be ended.

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

#### Scenario: An updated CLI finds a runtime left for a saved Pi tab
- **WHEN** an updated project CLI starts a project that holds a saved Pi tab and an OMP tab, each with a persistent runtime left by the older CLI
- **THEN** the Pi tab's runtime is not kept and is removed
- **AND** the OMP tab's runtime is kept and adopted

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

### Requirement: Terminal conversations surface agent questions

A terminal pane's conversation history SHALL include a question the agent asks, carrying the question text and every offered option, even when the underlying turn contains no prose. A question that has been answered SHALL also carry the recorded answer. Turns that are only tool activity and ask nothing SHALL continue to be omitted.

#### Scenario: Agent asks a question in a terminal pane

- **WHEN** an agent in a terminal pane asks the human a question
- **THEN** the conversation view shows that question with its options
- **AND** shows it as awaiting an answer

#### Scenario: Question is answered outside the conversation view

- **WHEN** the question is answered directly in the terminal instead
- **THEN** the conversation view shows the question with the answer that was recorded
- **AND** no longer presents it as awaiting an answer

#### Scenario: Ordinary tool activity stays out of the history

- **WHEN** the agent runs a tool without asking anything
- **THEN** that activity does not appear as a conversation turn

### Requirement: A pending terminal question can be answered from the conversation view

The conversation view SHALL let the human answer a pending question, and the system SHALL deliver that answer to the agent running in the pane. Delivery SHALL be reported as complete only once the agent has recorded the answer, and the recorded answer SHALL be what the view then shows. An answer that cannot be delivered SHALL leave the question pending rather than appear answered.

#### Scenario: Human answers from the conversation view

- **WHEN** the human selects an option for a pending question in the conversation view
- **THEN** the system delivers that selection to the agent in that pane
- **AND** the question settles once the agent records an answer
- **AND** the view shows the answer the agent recorded

#### Scenario: Recorded answer differs from the submitted one

- **WHEN** the answer the agent records is not the option that was submitted
- **THEN** the conversation view shows the recorded answer rather than the submitted one

#### Scenario: Answer is submitted while the pane is unreachable

- **WHEN** the human answers a question whose pane cannot currently be reached
- **THEN** the system does not present the question as answered
- **AND** the question remains answerable once the pane is reachable again

### Requirement: An answered terminal question is never answered twice

Once an answer has been recorded for a question, the system SHALL refuse further answers to that same question and SHALL send nothing to the pane for it. Input on behalf of an answer SHALL only ever be delivered for a question that is still pending.

#### Scenario: Stale client answers an already-answered question

- **WHEN** a client that has not seen the answer submits one for a question the agent already answered
- **THEN** the system refuses the submission
- **AND** sends nothing to the pane
- **AND** the recorded answer is unchanged

#### Scenario: The same answer arrives twice

- **WHEN** an answer for a pending question is submitted more than once
- **THEN** the agent receives the selection at most once

### Requirement: A pane waiting on a question has a distinct status

The application SHALL report an unresolved agent question as Pending answer rather than Working or Idle. The status SHALL remain pending while delivery is unconfirmed, and SHALL return to Working when the provider records the answer and continues the turn. Project lists, waiting-agent lists, pane selectors, and conversation status surfaces SHALL preserve this distinction. Pending answer SHALL take presentation precedence over provider availability because it names the action the human can take now.

#### Scenario: Agent blocks on a question

- **WHEN** a pane publishes an `AskUserQuestion` without a matching recorded answer
- **THEN** the pane and its project are presented as Pending answer
- **AND** the pane is not presented as actively working or ordinarily idle

#### Scenario: Submitted answer is not yet recorded

- **WHEN** an answer has been routed to the pane but no matching tool result has been recorded
- **THEN** the pane remains Pending answer

#### Scenario: Provider records the answer

- **WHEN** the matching tool result is recorded
- **THEN** the Pending answer state clears
- **AND** the pane is presented as Working until the provider completes the resumed turn

#### Scenario: Pending pane is visible among waiting agents

- **WHEN** a running project has a pane awaiting an answer
- **THEN** that pane appears ahead of ordinary idle and usage-limited panes in the waiting-agent list
- **AND** its badge reads Pending answer

### Requirement: OMP questions use structured native answers

OMP terminal conversations SHALL expose native `ask` questions and results using the existing question-card surface. Answers SHALL preserve native question IDs, selected-option arrays, and custom text separately. The answer bridge SHALL resolve the current native dialog without sending terminal keystrokes or changing the provider to RPC mode.

#### Scenario: Multiple questions with ambiguous display text

- **WHEN** an OMP ask contains repeated question text, comma-containing labels, multiple selections, or a custom response
- **THEN** the conversation view preserves each native question ID and exact answer fields
- **AND** OMP records those values in its native tool result before the card confirms them

#### Scenario: Native terminal wins the answer race

- **WHEN** a question is answered in the terminal before a submitted web answer is consumed
- **THEN** the native terminal answer wins
- **AND** the web displays that recorded answer rather than its submitted selection
- **AND** no answer bytes enter the provider's ordinary input

#### Scenario: A question is cancelled or leaves the active branch

- **WHEN** a native question is cancelled, its session changes, or its branch is abandoned
- **THEN** the old card becomes cancelled or unavailable, not successfully answered
- **AND** an old web submission cannot answer a newer dialog

#### Scenario: Invalid or duplicate submission

- **WHEN** an OMP answer has an invalid question ID, option, pane, session, or dialog generation, or repeats a settled request
- **THEN** the system rejects it without altering native input
- **AND** a still-valid native question remains answerable

#### Scenario: CLI restore or metadata update

- **WHEN** the CLI restores a pending OMP question or discovers its current answer bridge
- **THEN** the pending card and answerability recover without replaying ordinary historical turns
- **AND** repeated metadata does not create duplicate cards

#### Scenario: Native bridge is unavailable

- **WHEN** an older OMP process has no compatible loaded question bridge
- **THEN** its captured question remains readable with terminal fallback
- **AND** the conversation view does not claim it can submit an answer
- **AND** deployment does not silently restart the active provider

### Requirement: Claude terminal conversations follow in-TUI session switches

For a Claude terminal pane, the transcript watcher SHALL follow the conversation the user is in. When the provider reports the transcript it writes, that report SHALL decide which file the watcher reads, and a changed report SHALL move the watcher at once. A search for a more recently written session file SHALL be a fallback only. It SHALL run only after the tracked file has been unchanged for two consecutive polls, SHALL look in the directory of the tracked file, SHALL NOT select a file named for the conversation of another pane that the runtime runs or that `.apas` records for any machine, and SHALL NOT select a file that has not advanced since the watch began. A file selected by the fallback SHALL NOT keep the watcher away from a reported or assigned transcript that exists. Every change of source SHALL re-baseline to the new source's end without replaying prior turns.

#### Scenario: User resumes a different Claude session inside the TUI

- **WHEN** a user switches to another Claude session inside a terminal pane and the provider reports the new transcript
- **THEN** the watcher reads the reported file and later turns appear in the pane's conversation history
- **AND** the conversation history does not replay the turns from before the switch

#### Scenario: The tracked conversation is still in flight

- **WHEN** the tracked file changed in size or modification time within the two most recent polls
- **THEN** the watcher does not look for another file

#### Scenario: Another pane's conversation is never adopted

- **WHEN** a newer session file in the same directory is named for the conversation of another pane of the project, on this machine or on another machine that shares the home directory
- **THEN** the watcher never switches to that file

#### Scenario: A new pane has written nothing yet

- **WHEN** a pane's own transcript does not exist yet and older session files are present in its directory
- **THEN** the watcher adopts none of them unless one advances after the watch began

#### Scenario: User switches back to the assigned session

- **WHEN** a user resumes the pane's assigned session again inside the TUI
- **THEN** the watcher returns to that session's file
- **AND** the conversation history continues from its end without duplicates

### Requirement: A pane's transcript is identified by what the provider reports

Where a provider can report which transcript it is writing, the system SHALL use that report to identify the pane's transcript, in preference to inferring it from the pane's working directory or from an identifier the system assigned. A report SHALL be treated as the provider stating that its session changed, and SHALL re-point the pane immediately.

#### Scenario: The provider moves its session

- **WHEN** a provider relocates its session so that it writes somewhere other than where the pane's working directory implies
- **THEN** the pane follows the reported location

#### Scenario: The user resumes a different session

- **WHEN** a user switches the pane's provider onto a different session
- **THEN** the pane follows the transcript that session writes
- **AND** does not continue reading the session the system originally assigned

#### Scenario: Nothing has been reported

- **WHEN** no report exists for a pane, because its provider is older or the report could not be made
- **THEN** the system falls back to inferring the transcript as it did before
- **AND** the pane is no worse off than without the mechanism

#### Scenario: A report that names nothing usable

- **WHEN** a report carries no usable transcript location
- **THEN** it is ignored rather than followed

### Requirement: Reporting is scoped to the system's own panes

The reporting mechanism SHALL identify which pane a report belongs to, and SHALL record nothing for a provider the system did not start. Installing the mechanism SHALL NOT change any other behaviour the pane's owner has configured for that provider.

#### Scenario: A provider the user started themselves

- **WHEN** a person runs the provider by hand, outside the system
- **THEN** nothing is recorded for any pane
- **AND** that session cannot be mistaken for a pane's own

#### Scenario: The owner's configuration is preserved

- **WHEN** the mechanism is installed for a pane
- **THEN** the provider still honours the settings its owner configured

#### Scenario: The mechanism cannot be installed

- **WHEN** the mechanism cannot be installed for a pane
- **THEN** the pane still starts

### Requirement: A saved Pi terminal tab is never launched
A project CLI SHALL keep a saved Pi tab in the project file and in the roster it reports, and SHALL NOT start the Pi executable for it on any path: project start, restore, resume, reboot of the tab, or a provider or model switch. At every registration with the server the CLI SHALL report that tab's terminal as exited, with the status "Pi is no longer supported. Use OMP instead." The CLI SHALL NOT send a message about the retirement into the tab's stored conversation. The CLI SHALL NOT look for or read Pi session files.

#### Scenario: A project with a saved Pi tab starts
- **WHEN** a project CLI starts a project whose file holds a Pi terminal tab
- **THEN** the tab is listed in the roster with its provider
- **AND** no Pi process is started, even when a `pi` executable is installed on the host
- **AND** the other tabs of the project start as usual

#### Scenario: The CLI registers with the server
- **WHEN** the CLI reports its terminals after registering
- **THEN** the saved Pi tab's terminal is reported as exited with the status "Pi is no longer supported. Use OMP instead."
- **AND** a configured OMP tab without a process is reported as it was before, not with that sentence

#### Scenario: Someone asks to restart the saved Pi tab
- **WHEN** a restart or resume of the saved Pi tab reaches the CLI
- **THEN** the CLI refuses it and the tab stays stopped

#### Scenario: The saved Pi tab is closed
- **WHEN** a person closes the saved Pi tab
- **THEN** it leaves the roster and the project file like any other tab

### Requirement: OMP terminal conversations are recovered from the tab's private session directory
For an OMP terminal tab, the project host SHALL recover conversation turns from the session file in the tab's own private session directory, the directory passed to OMP on every launch of that tab, and SHALL read only the active branch of that session. It SHALL expose real user and completed assistant text as the tab's conversation history, SHALL NOT expose thinking, tool-result, shell-execution, extension, compaction, branch-summary, label, or model bookkeeping entries as conversation messages, and SHALL preserve reported model, token usage, and cost when available. Native `ask` questions and their results are shown as question cards, as "OMP questions use structured native answers" specifies, and how an autopilot iteration is marked in that history is specified by `omp-tab-autopilot`.

#### Scenario: Tab session contains conversation turns
- **WHEN** the transcript watcher reads the session file in an OMP tab's private session directory
- **THEN** it emits each real user message and each assistant text response as conversation history for that tab

#### Scenario: Session contains non-conversation entries
- **WHEN** an OMP session includes thinking blocks, tool results, shell-execution records, extension entries, compaction or branch summaries, labels, or model and thinking-level changes
- **THEN** APAS does not expose those entries as conversation messages
- **AND** internal data is not published as user or assistant text

#### Scenario: Assistant response continues into tool use
- **WHEN** a persisted assistant message records a tool-use continuation rather than a completed turn
- **THEN** APAS records the assistant text but does not mark the tab idle on account of that message

#### Scenario: Completed response reports usage and cost
- **WHEN** a completed assistant message carries model, token usage, or cost metadata
- **THEN** APAS attributes that metadata to the owning tab
- **AND** reports the recorded cost rather than a fabricated value when the provider supplies one

#### Scenario: Two OMP tabs share a working directory
- **WHEN** more than one OMP terminal tab runs in the same working directory
- **THEN** each tab's history is recovered from its own private session directory
- **AND** no tab displays another tab's conversation

### Requirement: A saved Pi tab is shown as read-only history
The desktop web and the mobile browser SHALL show a saved Pi tab's history and SHALL say "Pi is no longer supported. Use OMP instead." on it. They SHALL offer no message composer, no raw terminal, no resume and no reboot for that tab, and SHALL keep rename and close. A client SHALL send no typed message and no terminal input for such a tab.

#### Scenario: A saved Pi tab on the desktop
- **WHEN** a person selects a saved Pi tab in the desktop workspace
- **THEN** the tab shows its history and a notice that begins "Pi is no longer supported. Use OMP instead." and says the tab cannot receive input or be restarted
- **AND** its tab icon is the unsupported-provider mark, labelled with the same sentence

#### Scenario: A saved Pi tab on a phone
- **WHEN** a person selects a saved Pi tab on the mobile-browser session screen
- **THEN** the same notice is shown, the composer is disabled, the raw terminal cannot be opened and "Reboot this pane" is not offered
- **AND** rename and close remain available

#### Scenario: A message is sent to the tab anyway
- **WHEN** a client attempts to send a conversation message to a saved Pi tab
- **THEN** the client refuses with "Pi is no longer supported. Use OMP instead. This tab is read-only." and sends nothing

### Requirement: A saved Pi tab blocks the project's CLI reboot until it is closed
A request to reboot the CLI from a project that holds a saved Pi tab SHALL be refused, by the server and by the project CLI, with a message that names the tab and tells the person to close it first. A daemon restart requested from the machine's own entry SHALL NOT be blocked by a saved Pi tab.

#### Scenario: Reboot CLI in a project with a saved Pi tab
- **WHEN** a person requests Reboot CLI in a project whose roster holds saved Pi tab 9
- **THEN** the server answers "Tab 9 uses Pi, which is no longer supported. Close that tab, then reboot the CLI."
- **AND** no reboot is sent to the project CLI

#### Scenario: The tab has been closed
- **WHEN** the saved Pi tab has been closed and the person requests Reboot CLI again
- **THEN** the reboot proceeds as for any other project

#### Scenario: The machine is restarted from its own entry
- **WHEN** a person restarts the daemon of that machine from the Machines page
- **THEN** the restart is not refused on account of the saved Pi tab

