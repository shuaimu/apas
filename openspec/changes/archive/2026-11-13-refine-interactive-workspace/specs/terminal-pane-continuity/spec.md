## MODIFIED Requirements

### Requirement: Supported terminal runtimes survive project CLI replacement
The project host SHALL keep every live terminal runtime it can host persistently, for the Claude, Codex, OpenCode and OMP providers, outside the replaceable project CLI process. A successful full CLI reboot SHALL preserve the provider process, PTY state, terminal process-instance identifier, active turn, and descendant processes, then reconnect the replacement CLI to that same runtime. A terminal tab of a retired provider SHALL NOT be among the runtimes the project host keeps: when a project starts, a persistent runtime left for such a tab by an older CLI SHALL be ended.

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

## ADDED Requirements

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

## REMOVED Requirements

### Requirement: Pi terminal conversations are recovered from the provider's session files
**Reason**: Pi was retired as a provider on 2026-11-13. The project host no longer locates or reads Pi session files.
**Migration**: History already recovered for a Pi tab stays stored and readable. An OMP tab's conversation is still read from the session files in its private directory, which use the same record format; that behaviour is unchanged and is now stated in "OMP terminal conversations are recovered from the tab's private session directory".

### Requirement: Pi terminal restoration continues the exact retained session
**Reason**: A saved Pi tab is never restored. See "A saved Pi terminal tab is never launched".
**Migration**: Close the saved Pi tab and open an OMP tab.

### Requirement: Pi structured questions are not answered from the conversation view
**Reason**: No Pi tab runs, so no Pi tab can ask a question.
**Migration**: None. Cards already in a saved Pi tab's history cannot be answered, like everything else in a read-only tab.
