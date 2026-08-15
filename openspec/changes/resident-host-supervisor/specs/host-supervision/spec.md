## Purpose

Defines the single resident supervisor each project host runs: how it owns the projects running there, how a project CLI attaches to one instead of starting a competing copy, and what survives when the supervisor or a project worker dies.

## ADDED Requirements

### Requirement: A host runs one resident supervisor

A project host SHALL have at most one resident supervisor process, and it SHALL be the authority on which projects are running on that host. Starting a supervisor while one is already running SHALL NOT produce a second one. Every project started on the host SHALL be recorded by the supervisor, whether it was started remotely, on a schedule, or by a person at a terminal.

#### Scenario: A second supervisor is started

- **WHEN** a supervisor is started on a host that already has one running
- **THEN** the host still has exactly one supervisor
- **AND** the projects running on it are undisturbed

#### Scenario: A project is started from anywhere

- **WHEN** a project is started on the host by any means
- **THEN** the supervisor records it as running on that host

### Requirement: Running state comes from the supervisor, not from process inspection

The system SHALL answer "is this project running on this host" from the supervisor's own record of its workers. It SHALL NOT depend on inspecting unrelated processes' command lines to decide whether a project is running, and the reported state SHALL NOT be able to disagree with the host's actual workers.

#### Scenario: Running state is reported

- **WHEN** the running projects of a host are reported
- **THEN** the report reflects the supervisor's own record of its workers

#### Scenario: A worker exits unexpectedly

- **WHEN** a project's worker exits without being asked to stop
- **THEN** the supervisor stops reporting that project as running
- **AND** the project can be started again without manual cleanup

### Requirement: A project runs at most once per host

The system SHALL NOT run two workers for the same project on one host. A request to start a project that is already running there SHALL attach to the running one rather than start another, regardless of how either was initiated.

#### Scenario: A person starts a project the host already runs

- **WHEN** someone starts a project in a directory whose project is already running on that host
- **THEN** no second worker is created for it
- **AND** they are attached to the project that is already running

#### Scenario: A remote start races a local one

- **WHEN** a project is started remotely at the same moment as locally on the same host
- **THEN** exactly one worker exists for that project afterwards

### Requirement: A project CLI attaches to a project rather than owning it

A project CLI SHALL attach to a project the supervisor is running and present it, and its own exit SHALL NOT stop that project. Attaching SHALL be possible more than once for the same project, and each attachment SHALL see that project's current state.

#### Scenario: The person closes their terminal

- **WHEN** an attached project CLI exits for any reason
- **THEN** the project keeps running on the host
- **AND** its panes and agents are undisturbed

#### Scenario: Attaching to a project that is not running yet

- **WHEN** someone attaches to a project that is not currently running on the host
- **THEN** the supervisor starts it
- **AND** the attachment presents it once it is running

#### Scenario: Two attachments to one project

- **WHEN** a project is attached to twice
- **THEN** both attachments present the same project
- **AND** neither is a separate copy of it

### Requirement: Projects outlive their supervisor

Loss of the supervisor SHALL NOT stop the projects running on the host. When a supervisor starts and finds workers already running there, it SHALL adopt them rather than duplicate or orphan them, and SHALL report them as running.

#### Scenario: The supervisor is replaced

- **WHEN** the supervisor is restarted or upgraded while projects are running
- **THEN** those projects keep running throughout
- **AND** the new supervisor reports them as running and can stop them

#### Scenario: The supervisor is absent

- **WHEN** projects are running on a host with no supervisor
- **THEN** they keep running
- **AND** a supervisor started afterwards adopts them

### Requirement: Host supervision does not replace cross-host exclusion

Single supervision per host SHALL NOT be treated as protection against two hosts running the same project. Where hosts share project storage, the existing cross-host exclusion SHALL continue to decide which host may run a project.

#### Scenario: Two hosts share one project directory

- **WHEN** two hosts sharing storage would each run the same project
- **THEN** cross-host exclusion still decides which one does
- **AND** per-host supervision does not by itself permit both
