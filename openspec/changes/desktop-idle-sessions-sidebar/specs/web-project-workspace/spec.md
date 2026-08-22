## ADDED Requirements

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

### Requirement: Usage-limited agents are not idle

The workspace SHALL distinguish an agent waiting for input from an agent whose provider is actively refusing work because a usage limit is in effect. It SHALL NOT encode usage limitation as pane work, and SHALL show the limiting window and reset time when the provider reports them.

#### Scenario: An inactive turn has an active provider limit

- **WHEN** an agent is not working and its provider reports an active usage limit
- **THEN** the agent does not appear in Idle sessions
- **AND** it appears in the separate Usage limited view
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
