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
