# pane-toolbar Specification

## Purpose

Defines a concise pane toolbar that keeps view selection next to provider usage while leaving model and effort configuration to the interactive provider terminal.

## Requirements

### Requirement: Pane toolbar presents only supported pane actions
The web interface SHALL omit the Timeline/Chat action and inline model and reasoning-effort selectors from the pane toolbar. It SHALL continue to present other applicable pane actions, including provider switching, according to the active pane's state.

#### Scenario: Interactive agent pane toolbar
- **WHEN** an attached interactive agent pane is active
- **THEN** its toolbar does not contain a Timeline, Chat, model, or reasoning-effort control
- **AND** other actions that apply to the pane remain available

#### Scenario: Pane has stored model and effort settings
- **WHEN** a pane with persisted model or effort values becomes active
- **THEN** the toolbar does not render selectors for those values
- **AND** activating the pane does not clear or alter the persisted values

### Requirement: Non-terminal panes use the conversation view
The web interface SHALL render a non-terminal pane with the standard structured conversation view and SHALL NOT offer an alternate per-action timeline view.

#### Scenario: User views a non-terminal pane with tool activity
- **WHEN** the active non-terminal pane contains messages and tool activity
- **THEN** the standard conversation view is displayed
- **AND** no control can replace it with a timeline view

### Requirement: Terminal view switch is located beside usage
For an active terminal pane, the web interface SHALL show the Terminal/Conversation switch in the shared pane toolbar immediately before the provider usage status when that status is available. The switch SHALL NOT appear for non-terminal panes.

#### Scenario: Terminal pane has provider usage status
- **WHEN** a terminal pane with provider usage information is active
- **THEN** the Terminal/Conversation switch appears in the pane toolbar
- **AND** the switch precedes the provider usage status in visual and document order

#### Scenario: Terminal pane has no provider usage status
- **WHEN** a terminal pane without provider usage information is active
- **THEN** the Terminal/Conversation switch still appears in the pane toolbar

#### Scenario: Non-terminal pane is active
- **WHEN** the active pane is not a terminal pane
- **THEN** the Terminal/Conversation switch is not displayed

### Requirement: Terminal view switching preserves existing behavior
The Terminal/Conversation selection SHALL remain independent per session and pane. Switching to Conversation SHALL show the captured structured turns and terminal chat input, while switching to Terminal SHALL show the live interactive terminal without recreating its existing terminal instance.

#### Scenario: User switches a terminal pane to Conversation
- **WHEN** the user selects Conversation for an active terminal pane
- **THEN** the captured conversation and terminal chat input are displayed
- **AND** the live terminal remains mounted but hidden

#### Scenario: User returns to Terminal
- **WHEN** the user selects Terminal after viewing Conversation
- **THEN** the existing live terminal is displayed with its retained state

#### Scenario: User revisits a terminal pane
- **WHEN** the user selects a terminal pane that has a stored view preference for the current session
- **THEN** the toolbar switch and pane content reflect that pane's stored preference

### Requirement: Pane toolbar exposes only a full CLI reboot
For an attached project, the web interface SHALL offer exactly one project lifecycle action: a full project CLI reboot. It SHALL NOT offer a transport-reconnect action, because transport recovery is automatic and is not a decision a user is equipped to make. Reboot confirmation, progress, and results SHALL communicate that the CLI process is replaced and which pane kinds cannot be adopted across it.

#### Scenario: User opens lifecycle actions

- **WHEN** a user opens lifecycle actions for an attached project
- **THEN** the interface offers `Reboot CLI` and no transport-reconnect action
- **AND** does not describe reboot as a remedy for a lost connection

#### Scenario: User chooses full reboot

- **WHEN** the project has terminal panes and the user confirms `Reboot CLI`
- **THEN** the confirmation identifies whether those live terminal agents can be preserved
- **AND** separately warns about any legacy structured panes that will restart or resume

#### Scenario: Reboot reports progress

- **WHEN** a reboot is in progress
- **THEN** the interface reports its phase against the originating request
- **AND** reports success only after the replacement CLI registers and reconciles its pane roster
