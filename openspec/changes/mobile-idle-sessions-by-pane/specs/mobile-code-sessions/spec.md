## ADDED Requirements

### Requirement: Idle work is listed per agent, not per project

The application SHALL offer a list of the agents that are not currently working, with one entry per agent rather than one per project. An agent that is idle SHALL appear there even when other agents in the same project are working. Each entry SHALL identify the project it belongs to before the agent, giving both equal prominence, and SHALL identify its host. The project places the entry and the agent is what opening it selects, so neither is subordinate to the other.

#### Scenario: A project with both busy and idle agents

- **WHEN** a project has one agent working and others idle
- **THEN** the idle agents appear in the list
- **AND** the working one does not

#### Scenario: Every agent in a project is working

- **WHEN** every agent in a project is working
- **THEN** none of them appears in the list

#### Scenario: Placing an entry

- **WHEN** a user reads an entry in the list
- **THEN** it names the agent, and the project and host it belongs to

#### Scenario: Reading an entry

- **WHEN** a user reads an entry
- **THEN** the project is named before the agent
- **AND** neither is presented less prominently than the other

#### Scenario: Opening an idle agent

- **WHEN** a user opens an entry
- **THEN** the session opens with that agent selected, rather than whichever was last used

#### Scenario: Nothing is idle

- **WHEN** no agent is idle
- **THEN** the list says so rather than appearing broken

### Requirement: Per-agent state is reported to mobile

The system SHALL report, for each session a mobile client can reach, the agents in it and whether each is currently working. This SHALL be derived from the same source as the session-level working flag, so the two cannot disagree. A client receiving no per-agent detail SHALL treat it as unknown rather than as idle.

#### Scenario: A session's agents are reported

- **WHEN** a mobile client loads what it can reach
- **THEN** each session carries its agents and whether each is working

#### Scenario: Agreement with the session-level flag

- **WHEN** a session reports that it is working
- **THEN** at least one of its agents is reported as working

#### Scenario: An older server omits the detail

- **WHEN** the per-agent detail is absent from a response
- **THEN** the client still functions
- **AND** does not present those agents as idle

### Requirement: Mobile groups usage-limited agents beneath idle agents

The application SHALL treat provider availability as separate from whether a pane is currently working. The Idle sessions view SHALL show ordinary idle panes first and provider-blocked panes in a labelled Usage limited subsection beneath them, with the limiting window and reset time when known.

#### Scenario: A non-working pane is usage limited

- **WHEN** a pane is not working and its provider reports an active usage limit
- **THEN** it does not appear among the ordinary idle rows
- **AND** it appears in the Usage limited subsection with a usage-limited label rather than an idle label

#### Scenario: The provider permits extra usage

- **WHEN** included utilization reaches 100 percent and the provider reports that extra usage is enabled and available
- **THEN** the pane is not classified as usage limited from utilization alone

#### Scenario: The reported reset passes

- **WHEN** the usage limit's reset time passes before a refreshed payload arrives
- **THEN** the client stops presenting the pane as usage limited

### Requirement: Mobile ranks recently idle agents first

The application SHALL record when each pane transitions from working to idle and SHALL order Idle sessions from the most recent transition to the oldest. Repeated idle observations SHALL preserve the existing transition time. A client receiving an older pane summary without that timestamp SHALL still show it after timestamped idle panes.

#### Scenario: A pane becomes idle after another pane

- **WHEN** two idle panes have different idle-transition times
- **THEN** the pane that became idle later appears first

#### Scenario: The same idle state is replayed

- **WHEN** an idle pane receives another idle observation without working in between
- **THEN** its idle-transition time does not change

#### Scenario: An older server omits idle recency

- **WHEN** an idle pane has no idle-transition timestamp
- **THEN** it remains in the list after timestamped idle panes
