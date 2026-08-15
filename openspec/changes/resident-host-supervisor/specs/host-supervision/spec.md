## Purpose

Defines the single resident instance each project host runs for a user: how a second launch defers to it rather than becoming a rival, what keeps running when it is gone, and why this does not replace cross-host exclusion.

## ADDED Requirements

### Requirement: A host runs one resident instance per user

A user SHALL have at most one resident instance running on a host. Launching one while another is already running SHALL NOT produce a second, and SHALL leave the running one and its work undisturbed. A record of an instance that is no longer running SHALL NOT prevent a new one from starting.

#### Scenario: A second launch while one is running

- **WHEN** a user launches an instance on a host where theirs is already running
- **THEN** the host still has exactly one
- **AND** the projects it is running are undisturbed

#### Scenario: A stale record from a crash

- **WHEN** a user launches an instance on a host whose previous one died without cleaning up
- **THEN** the new instance starts
- **AND** the stale record does not prevent it

### Requirement: A second launch defers instead of competing

A launch that finds an instance already running SHALL NOT start, attach to, or duplicate any project. When it was run from a project directory it SHALL register that project so it becomes visible for management, then report where the project can be managed and exit.

#### Scenario: Launched from a project directory

- **WHEN** a user runs it in a project directory while their instance is already running
- **THEN** the project is registered and becomes visible for management
- **AND** the user is told where to manage it
- **AND** no second process is left running for that project

#### Scenario: Launched from a directory that is not a project

- **WHEN** a user runs it outside any project while their instance is already running
- **THEN** it reports the running instance and exits
- **AND** registers nothing

#### Scenario: The same project directory twice

- **WHEN** a user runs it twice in the same project directory
- **THEN** the project is registered once
- **AND** no process is left running for it either time

### Requirement: A project runs at most once per host

The system SHALL NOT run two workers for the same project on one host, regardless of how each was initiated.

#### Scenario: A person acts on a project the host already runs

- **WHEN** a project that is already running on a host is started there again
- **THEN** no second worker is created for it

#### Scenario: A remote start races a local registration

- **WHEN** a project is started remotely at the same moment as it is registered locally on the same host
- **THEN** exactly one worker exists for that project afterwards

### Requirement: Projects outlive the instance that started them

Loss of the resident instance SHALL NOT stop the projects running on the host. When an instance starts and finds project workers already running there, it SHALL adopt them rather than duplicate or orphan them, and SHALL report them as running.

#### Scenario: The instance is replaced

- **WHEN** the resident instance is restarted or upgraded while projects are running
- **THEN** those projects keep running throughout
- **AND** the new instance reports them as running and can stop them

#### Scenario: The instance is absent

- **WHEN** projects are running on a host with no resident instance
- **THEN** they keep running
- **AND** an instance started afterwards adopts them

### Requirement: Single instance does not replace cross-host exclusion

One instance per user per host SHALL NOT be treated as protection against two hosts running the same project. Where hosts share project storage, the existing cross-host exclusion SHALL continue to decide which host may run a project.

#### Scenario: Two hosts share one project directory

- **WHEN** two hosts sharing storage would each run the same project
- **THEN** cross-host exclusion still decides which one does
- **AND** the per-host single-instance rule does not by itself permit both
