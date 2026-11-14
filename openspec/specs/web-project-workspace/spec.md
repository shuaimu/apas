# web-project-workspace Specification

## Purpose

Defines how the web workspace represents selected projects, including empty projects and creation of their first pane.
## Requirements
### Requirement: Selected zero-pane projects remain operable
When a project session is selected and its authoritative pane configuration is empty, the web client SHALL render the normal project workspace with the Overview and pane-management controls. The web client MUST treat zero panes as a valid project state rather than as the absence of a project.

#### Scenario: Open a newly created project
- **WHEN** a user selects an attached project whose authoritative pane configuration contains zero panes
- **THEN** the web client displays the normal project workspace with the Overview selected
- **AND** the pane creation control is visible and usable
- **AND** the no-project fallback is not displayed

### Requirement: Users can create the first pane from the web
The web workspace SHALL allow a user to submit an allowed pane configuration when the selected project currently has no panes, using the same creation flow and policy filtering used for projects that already have panes.

#### Scenario: Create the first pane
- **WHEN** a user chooses an allowed pane type from the creation control in a selected zero-pane project
- **THEN** the web client submits a pane creation request for the selected project session
- **AND** the pane appears as a selectable workspace tab when the authoritative pane configuration is updated

#### Scenario: Apply pane policy to the first pane
- **WHEN** a selected zero-pane project disallows a pane launch profile
- **THEN** the creation control does not offer that profile as an available first-pane choice

### Requirement: No-project fallback remains distinct
The web client SHALL display the no-project fallback only when there is no selected project session. That fallback SHALL NOT expose project-specific pane-management controls.

#### Scenario: No project is selected
- **WHEN** the web client has no selected project session
- **THEN** it displays the no-project fallback
- **AND** it does not display the project Overview or pane creation control

### Requirement: The workspace can list idle agents as well as projects

The project sidebar SHALL offer both the projects an account can reach and the agents within them that are not currently working, and SHALL make clear which is being shown. An agent that is idle SHALL be listed even when other agents in its project are working. Each entry SHALL name the project it belongs to before the agent, giving both equal prominence, and SHALL name its host.

#### Scenario: A project with both busy and idle agents

- **WHEN** a project has one agent working and others idle
- **THEN** the idle agents are listed
- **AND** the working one is not

#### Scenario: Reading an entry

- **WHEN** a user reads an entry
- **THEN** the project is named before the agent
- **AND** neither is presented less prominently than the other

#### Scenario: Agents in a project that is not running

- **WHEN** a project is not running
- **THEN** its agents are not listed as idle

#### Scenario: An agent whose machine is not running the project

- **WHEN** a project runs on one machine and an agent belongs to another machine that is not running it
- **THEN** that agent is not listed as idle

#### Scenario: No per-agent detail is available

- **WHEN** a session reports no per-agent detail
- **THEN** its agents are not listed as idle

#### Scenario: Returning to the projects

- **WHEN** a user switches back to the projects
- **THEN** the project list is shown as before

### Requirement: Opening an idle agent opens that agent

Opening an entry that names an agent SHALL open its session with that agent selected, in preference to whichever the project was last left on.

#### Scenario: Opening an agent from the list

- **WHEN** a user opens an idle agent
- **THEN** its session is attached
- **AND** that agent is selected rather than the remembered one

#### Scenario: The agent is no longer there

- **WHEN** the named agent does not exist in the attached session
- **THEN** the selection falls back to the ordinary behaviour

### Requirement: Usage-limited agents are grouped beneath idle agents

The workspace SHALL distinguish an agent waiting for input from an agent whose provider is actively refusing work because a usage limit is in effect. It SHALL NOT encode usage limitation as pane work. The Idle sessions view SHALL show a labelled Usage limited subsection below the ordinary idle agents, including the limiting window and reset time when the provider reports them. Agents with an open question SHALL be grouped above the ordinary idle agents.

#### Scenario: An inactive turn has an active provider limit

- **WHEN** an agent is not working and its provider reports an active usage limit
- **THEN** the agent does not appear among the ordinary idle rows
- **AND** it appears in the Usage limited subsection beneath them
- **AND** it is labelled usage limited rather than idle

#### Scenario: Included usage is exhausted but extra usage remains available

- **WHEN** an included usage meter reaches 100 percent but the provider reports that paid extra usage remains available
- **THEN** the agent is not classified as usage limited from utilization alone

#### Scenario: A reset time has passed

- **WHEN** a previously reported usage limit has a reset time that is no longer in the future
- **THEN** the agent is no longer presented as usage limited

### Requirement: The most recently idle agent is first

The workspace SHALL record when each pane transitions from working to idle and SHALL order Idle sessions from the most recent such transition to the oldest. Repeated idle observations SHALL NOT make an already-idle pane look newer. Panes from an older payload without an idle timestamp SHALL remain usable and SHALL sort after panes with a known timestamp.

#### Scenario: Two agents became idle at different times

- **WHEN** two idle agents report different idle-transition times
- **THEN** the agent with the later transition appears first

#### Scenario: An idle state is observed again

- **WHEN** an already-idle agent reports idle again
- **THEN** its original idle-transition time is preserved

#### Scenario: An older payload has no idle timestamp

- **WHEN** one idle agent has a transition timestamp and another does not
- **THEN** the timestamped agent appears first
- **AND** the older payload remains visible

### Requirement: A project can be opened and closed from its own workspace

The workspace SHALL offer, in one place, opening a project that is not running and closing one that is. Closing SHALL stop every agent in the project. Both SHALL be confirmed first, and the confirmation for closing SHALL state that the project's agents are stopped, since that is what is lost.

#### Scenario: A project that is not running

- **WHEN** a user views the workspace of a project that is not running
- **THEN** it offers to open the project
- **AND** does not offer to close it

#### Scenario: Opening from a mobile project workspace

- **WHEN** a user views a project that is not running in the mobile session view
- **THEN** its pane-actions menu offers to open the project
- **AND** confirmation starts it on every online machine that has the project's tabs, or on the session's machine when no machine has any

#### Scenario: A project that is running

- **WHEN** a user views the workspace of a running project
- **THEN** it offers to close the project
- **AND** does not offer to open it

#### Scenario: Closing a project

- **WHEN** a user confirms closing a project
- **THEN** the project's agents are stopped

#### Scenario: Declining

- **WHEN** a user declines the confirmation
- **THEN** nothing is stopped

#### Scenario: A closed project's agents are not listed as idle

- **WHEN** a project has been closed
- **THEN** its agents no longer appear among the idle agents

### Requirement: Project selection commits only after authorized attachment
The web client SHALL treat a requested project selection as pending until the server confirms attachment to that exact session. While pending, it SHALL NOT make the target session the active workspace, restore its cached content, persist it as the current session, or emit project-scoped follow-on messages. A rejected or stale attachment SHALL clear the pending selection, show one actionable error, and retain the last confirmed workspace or return to the project list when none exists.

#### Scenario: Authorized attachment succeeds
- **WHEN** a user selects a project and the server confirms attachment to that exact session
- **THEN** the client commits that session as the active and persisted workspace
- **AND** renders only the data and controls received or retained for that authorized session

#### Scenario: Attachment is denied
- **WHEN** the server rejects a pending attachment for insufficient project or hosting-cluster authority
- **THEN** the client does not render or persist the rejected project as active
- **AND** emits no pane, usage, policy, terminal, or other project-scoped follow-on requests for it
- **AND** displays one error that distinguishes missing project access from missing host-machine access when the server supplies that distinction

#### Scenario: Attachment confirmations arrive out of order
- **WHEN** a confirmation arrives for an older pending request after the user selected another project
- **THEN** the client ignores the stale confirmation for active navigation
- **AND** does not replace the newer pending or confirmed workspace

### Requirement: The new-tab control creates an interactive agent directly
The new-tab control of a project workspace, "+" in the desktop tab bar and "Add agent" on the mobile-browser session screen, SHALL open the list of interactive providers the project's launch policy allows, with no earlier step. It SHALL offer the "Run on" step where that step applies, and on the desktop the isolated git worktree option. It SHALL NOT ask the person to choose between an interactive agent and an autonomous worker, and SHALL NOT offer autonomous worker creation. Choosing a provider SHALL submit the pane creation request for the selected project session, as before.

#### Scenario: A person presses "+" on the desktop
- **WHEN** a person opens the new-tab control in the desktop tab bar
- **THEN** the allowed interactive providers are listed at once, with the isolated git worktree option
- **AND** no "Interactive agent" or "Autonomous worker" entry is shown

#### Scenario: A person adds an agent on a phone
- **WHEN** a person opens "Add agent" on the mobile-browser session screen
- **THEN** the sheet lists the profiles the project's policy allows at once
- **AND** no step asks for the kind of agent first

#### Scenario: The project runs on several machines
- **WHEN** the new-tab control is opened in a project where the "Run on" step applies
- **THEN** that step is shown in the same menu, and a provider cannot be chosen before a machine is

#### Scenario: A project with no tab
- **WHEN** a person opens the new-tab control in a selected project that has no tab
- **THEN** the same provider list is offered and choosing one creates the project's first tab

#### Scenario: No provider is allowed
- **WHEN** the project's launch policy allows no interactive provider
- **THEN** the control says that no interactive provider is allowed by the project's launch policy

### Requirement: An autonomous worker is created from the Overview or the Workers page
The web client SHALL offer autonomous worker creation in two places: the "New autonomous worker" action in the Autonomous workers section of a project's Overview, on the desktop and on the mobile-browser screen, and the cross-project Workers page at `/code/workers`. From the Overview the action SHALL open the worker composer with that project fixed. In the desktop workspace, read-only legacy bot history SHALL direct the person to the Overview's action.

#### Scenario: A person creates a worker from the Overview
- **WHEN** a person chooses "New autonomous worker" in a project's Overview
- **THEN** the worker composer opens inside that project, with the project fixed
- **AND** no interactive tab is created

#### Scenario: A person looks at legacy bot history
- **WHEN** a person selects a tab that is read-only legacy bot history in the desktop workspace
- **THEN** the tab says "Legacy bot history is read-only. To set up recurring work, use New autonomous worker on the Overview tab."

### Requirement: The Overview presents the project in a fixed order
A project's Overview SHALL be one page that starts with the project's name, continues with the project's interactive sections, and ends with the Autonomous workers section, which includes Earlier results. In the desktop workspace the interactive sections SHALL be, in this order: Tabs, with one card per tab; Usage; Resource use; Allowed launch profiles. On the mobile-browser session screen the interactive part SHALL be the Tabs section, with one row per tab, and the page SHALL use the same parts and the same order: the project's name, Tabs, Autonomous workers. The tab cards and rows SHALL follow the order of the tab bar and SHALL NOT be reordered by pane number or by a tab's status. A desktop tab card SHALL show the tab's status, its name, what it runs, when it was last active, and its autopilot condition when it has one, and SHALL offer opening the tab and closing it. A phone tab row SHALL show the tab's name, its status, its machine where that is shown, and a mark when it is on autopilot, and SHALL open the tab. The Overview SHALL NOT offer a "Back to interactive agents" action: the tab bar leads back to every tab. The worker, composer and earlier-result screens SHALL keep that action.

#### Scenario: A person opens the desktop Overview of a project with tabs
- **WHEN** a person opens the Overview of a project that has tabs and no autonomous worker in the desktop workspace
- **THEN** the Tabs section comes first under the project's name, followed by Usage, Resource use and Allowed launch profiles
- **AND** the Autonomous workers section closes the page and says that the project has no autonomous worker yet

#### Scenario: A person opens the Overview on a phone
- **WHEN** a person opens the Overview of the same project on the mobile-browser session screen
- **THEN** the page shows the project's name, the Tabs section with one row per tab, and then the Autonomous workers section
- **AND** activating a row opens that tab

#### Scenario: The tab cards follow the tab bar
- **WHEN** the tab bar lists tabs with pane numbers 612, 265 and 44 in that order, and the last one is Working
- **THEN** the cards are in the same order

#### Scenario: A project with no tab
- **WHEN** a person opens the Overview of a project that has no tab
- **THEN** the Tabs section says that there is no tab yet and that "+" opens one
- **AND** the other sections are in their places

#### Scenario: A tab on autopilot
- **WHEN** a tab is on autopilot
- **THEN** its card or row shows that beside its status

#### Scenario: No way back is offered on the Overview
- **WHEN** a person views the Overview on the desktop or on a phone
- **THEN** no "Back to interactive agents" action is shown
- **AND** the same action is still offered on a worker's screen, in the worker composer and on an earlier result

### Requirement: The Usage section lists open tabs and folds closed tabs
The Usage section of the desktop Overview SHALL show the project's totals over every tab, open or closed, for the selected period. Its table SHALL list the open tabs first, the most recently active first, then the usage no tab accounts for as "Unattributed". Tabs that were closed SHALL be folded into one row that carries their sum and their most recent activity, so that the rows add up to the totals. That row SHALL expand to list each closed tab, the most recently active first, and SHALL collapse again. When the table has no open tab and no unattributed usage to list, the closed tabs SHALL be listed without the need to expand. A time ahead of the browser's clock, or less than a minute old, SHALL read "just now".

#### Scenario: A project with two open tabs and sixteen closed ones
- **WHEN** a person opens the Usage section
- **THEN** the table lists the two open tabs, then one row "Show 16 closed tabs" with the closed tabs' sum
- **AND** the totals above the table include the closed tabs

#### Scenario: The closed tabs are expanded
- **WHEN** the person activates "Show 16 closed tabs"
- **THEN** each closed tab is listed as "Tab" with its number, the most recently active first
- **AND** the row reads "Hide 16 closed tabs"

#### Scenario: Only closed tabs have usage
- **WHEN** three closed tabs have recorded usage, no open tab has, and no usage is unattributed
- **THEN** the three closed tabs are listed under a row "3 closed tabs" without a control to expand

#### Scenario: No usage yet
- **WHEN** the project has recorded no usage
- **THEN** the section says that no usage is recorded yet

### Requirement: The Overview names allowed launch profiles as the launch menu does
The Allowed launch profiles section of the desktop Overview SHALL show each allowed profile by the name the provider catalog gives it, with the policy's key available as the profile's tooltip. A key the catalog cannot name SHALL be shown as the key. A key of a retired provider SHALL be left out.

#### Scenario: A policy that allows Claude, OMP and DeepSeek
- **WHEN** a person reads the Allowed launch profiles section
- **THEN** each profile is shown by its catalog name, and hovering it shows its policy key

#### Scenario: A policy the client holds still lists the Pi profile
- **WHEN** the effective policy the client holds lists `terminal:pi:official:default`
- **THEN** the section does not show it

