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
For an attached project, the web interface SHALL offer Reboot CLI as its only CLI lifecycle action, once for each machine that runs an instance of the project. Opening and closing the project are defined in `web-project-workspace`. It SHALL NOT offer a transport-reconnect action, because transport recovery is automatic and is not a decision a user is equipped to make. Reboot confirmation, progress, and results SHALL communicate that the CLI process is replaced and which pane kinds cannot be adopted across it.

#### Scenario: User opens lifecycle actions

- **WHEN** a user opens lifecycle actions for an attached project
- **THEN** the interface offers `Reboot CLI`, per machine when the project runs on several, and no transport-reconnect action
- **AND** does not describe reboot as a remedy for a lost connection

#### Scenario: User chooses full reboot

- **WHEN** the project has terminal panes and the user confirms `Reboot CLI`
- **THEN** the confirmation identifies whether those live terminal agents can be preserved
- **AND** separately warns about any legacy structured panes that will restart or resume

#### Scenario: Reboot reports progress

- **WHEN** a reboot is in progress
- **THEN** the interface reports its phase against the originating request
- **AND** reports success only after the replacement CLI registers and reconciles its pane roster

### Requirement: Pane toolbar offers autopilot on OMP terminal tabs

For an active OMP terminal tab on a server that supports autopilot, the web interface SHALL show an autopilot control in the pane toolbar, placed before the Terminal/Conversation switch, and SHALL place nothing between that switch and the provider usage status where one is shown. The control SHALL show the tab's reported autopilot condition. When the tab has no autopilot or an ended one, the control SHALL show the ended reason, if any, and SHALL open the start dialog. When the tab is on autopilot or has a paused autopilot, the control SHALL offer the actions that apply to that condition and a way to read the prompt, the timing, the limits or that none is set, when it started, the progress and, when a time limit is set, its end. For a viewer who may not send input to the tab, the control SHALL show the state and SHALL disable every action with the reason. For an autopilot that is paused because the tab's agent process exited, the control SHALL offer no Resume, Stop or Stop now and SHALL say "This tab's agent is not running. Reboot it first; its autopilot comes back paused." When the tab's autopilot state is not reported while its machine is connected and supports autopilot, the control SHALL say that the state is not reported, SHALL offer Stop and Stop now, and SHALL NOT offer starting, Pause or Resume. When autopilot is unavailable for the tab, the control SHALL be shown disabled with the reason rather than hidden. The control SHALL NOT appear for a non-terminal pane or for a terminal tab of another provider. Using the control SHALL NOT recreate the tab's terminal instance or change which view is selected.

#### Scenario: OMP terminal tab that is not on autopilot
- **WHEN** an OMP terminal tab that supports autopilot is active and has no autopilot
- **THEN** the toolbar shows the autopilot control before the Terminal/Conversation switch
- **AND** activating it opens the start dialog without sending anything

#### Scenario: Tab on autopilot
- **WHEN** the active tab is waiting for its next iteration
- **THEN** the control shows that condition and offers Pause, Stop, Stop now and the autopilot's details

#### Scenario: Tab with an ended autopilot
- **WHEN** the active tab's last autopilot ended at its time limit
- **THEN** the control shows "Ended: time limit reached" and opens the start dialog with that autopilot's prompt, timing and limits

#### Scenario: A collaborator who may use the tab
- **WHEN** a project member who may type into a tab hosted by another account opens the control of that tab, which has no autopilot
- **THEN** starting is offered as it is to the hosting account

#### Scenario: A viewer who may not use the tab
- **WHEN** a viewer without runtime access to the tab's machine opens the control of a tab whose autopilot is paused
- **THEN** the control shows the condition and its details, and Resume and Stop are disabled with the reason

#### Scenario: The tab's state is not reported
- **WHEN** the active tab's autopilot state is not reported while its machine is connected and supports autopilot
- **THEN** the control says the state is not reported and offers Stop and Stop now only

#### Scenario: The tab's agent process has exited
- **WHEN** the active tab's autopilot is shown as "Paused: the agent process exited"
- **THEN** the control offers no Resume, Stop or Stop now and says "This tab's agent is not running. Reboot it first; its autopilot comes back paused."

#### Scenario: Autopilot is unavailable for the tab
- **WHEN** the active OMP terminal tab's agent process predates autopilot support
- **THEN** the control is shown disabled with "Reboot this tab's agent to use autopilot. Rebooting ends its turn in progress."

#### Scenario: Other tabs
- **WHEN** a Claude terminal tab or a legacy structured pane is active
- **THEN** the toolbar shows no autopilot control

#### Scenario: Using the control keeps the terminal
- **WHEN** a person starts or stops autopilot while the tab's Terminal view is selected
- **THEN** the same terminal instance stays mounted with its scrollback, and the selected view does not change

### Requirement: Pane toolbar does not offer Interrupt on terminal tabs

The web interface SHALL NOT show the Interrupt action in the pane toolbar of a terminal tab, where it has no effect on the agent the terminal hosts. It SHALL keep offering Interrupt for a non-terminal pane as before.

#### Scenario: A working terminal tab
- **WHEN** a terminal tab is active and Working
- **THEN** the toolbar shows no Interrupt action

#### Scenario: A working legacy structured pane
- **WHEN** a legacy structured pane is active and Working
- **THEN** the toolbar shows the Interrupt action as before

