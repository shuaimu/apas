## ADDED Requirements

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
