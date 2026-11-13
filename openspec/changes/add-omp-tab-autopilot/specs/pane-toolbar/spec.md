## ADDED Requirements

### Requirement: Pane toolbar offers autopilot on OMP terminal tabs

For an active OMP terminal tab on a server that supports autopilot, the web interface SHALL show an autopilot control in the pane toolbar, placed before the Terminal/Conversation switch, and SHALL place nothing between that switch and the provider usage status where one is shown. The control SHALL show the tab's reported autopilot condition. When the tab has no autopilot or an ended one, the control SHALL show the ended reason, if any, and SHALL open the start dialog; for a viewer who is not the tab's hosting account, starting SHALL be disabled with "Only the account that runs this tab's machine can start autopilot." When the tab is on autopilot or has a paused autopilot, the control SHALL offer the actions that apply to that condition and a way to read the prompt, timing, limits, progress and time limit end; for a viewer who is not the tab's hosting account, Resume SHALL be disabled with "Only the account that runs this tab's machine can resume autopilot." For an autopilot that is paused because the tab's agent process exited, the control SHALL offer no Resume, Stop or Stop now and SHALL say "This tab's agent is not running. Reboot it first; its autopilot comes back paused." When the tab's autopilot state is not reported while its machine is connected and supports autopilot, the control SHALL say that the state is not reported, SHALL offer Stop and Stop now, and SHALL NOT offer starting, Pause or Resume. When autopilot is unavailable for the tab, the control SHALL be shown disabled with the reason rather than hidden. The control SHALL NOT appear for a non-terminal pane or for a terminal tab of another provider. Using the control SHALL NOT recreate the tab's terminal instance or change which view is selected.

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

#### Scenario: A viewer who is not the hosting account
- **WHEN** a project member who is not the tab's hosting account opens the control of a tab that has no autopilot
- **THEN** starting is disabled with "Only the account that runs this tab's machine can start autopilot."

#### Scenario: A paused autopilot seen by a viewer who is not the hosting account
- **WHEN** that member opens the control of a tab whose autopilot is paused
- **THEN** Resume is disabled with "Only the account that runs this tab's machine can resume autopilot.", and Stop is still offered

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
