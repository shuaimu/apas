## ADDED Requirements

### Requirement: Projects run inside the resident instance

The resident instance SHALL run the host's projects itself rather than as separate processes it cannot observe. What is running on a host SHALL be answered from what the instance holds, not from inspecting the process table.

#### Scenario: Projects are started on a host

- **WHEN** projects are started on a host
- **THEN** they run within the resident instance
- **AND** the host reports them as running from what that instance holds

#### Scenario: A project ends

- **WHEN** a running project ends for any reason
- **THEN** the host stops reporting it as running
- **AND** it can be started again without manual cleanup

### Requirement: One project's failure does not end the others

A project that fails SHALL be stopped and reported, and SHALL NOT stop any other project on that host or the instance itself. Conditions that would previously have ended the process — a rejected session, an unrecoverable project error — SHALL end only that project.

#### Scenario: The server rejects one project's session

- **WHEN** the server rejects the session of one project
- **THEN** that project stops and the reason is reported
- **AND** every other project on the host keeps running

#### Scenario: A project fails unexpectedly

- **WHEN** a project fails in a way it cannot recover from
- **THEN** it is reported as stopped
- **AND** the other projects and the instance are unaffected

### Requirement: A project restarts without restarting the host's instance

Restarting one project SHALL replace that project only. It SHALL NOT interrupt other projects, and SHALL NOT require replacing the resident instance.

#### Scenario: One project is restarted

- **WHEN** a project is restarted
- **THEN** that project stops and starts again
- **AND** the other projects on the host are undisturbed

#### Scenario: The instance is upgraded

- **WHEN** the resident instance is replaced to apply an upgrade
- **THEN** the projects it was running are restored to running afterwards

### Requirement: A project's activity remains attributable

Every diagnostic record a project produces SHALL identify which project produced it. Sharing one process SHALL NOT make it impossible to tell one project's activity from another's.

#### Scenario: Two projects log at once

- **WHEN** two projects on a host produce diagnostic records
- **THEN** each record identifies the project it came from
