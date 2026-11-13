## Purpose

Defines the single resident instance each project host runs for a user: how a launch defers to it instead of becoming a rival, that a project has one runtime per host, and how this relates to cross-host claims.

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

A launch SHALL NOT start, attach to, or duplicate any project. It SHALL make sure the user's resident instance is running, starting it detached when none is, and SHALL NOT become that instance. When it was run from a project directory it SHALL register that project so it becomes visible for management, then report where the project can be managed and exit.

Creating a project from a local directory stays a thing a launch does. `apas register [path]` and the web's Existing folder flow also register a folder without starting it.

#### Scenario: Launched from a project directory

- **WHEN** a user runs it in a project directory while their instance is already running
- **THEN** the project is registered and becomes visible for management
- **AND** the user is told where to manage it
- **AND** no second process is left running for that project

#### Scenario: Launched from a directory that is not yet a project

- **WHEN** a user runs it in a directory that is not yet a project while their instance is already running
- **THEN** that directory becomes a project and is registered, as it is today
- **AND** it reports where to manage it and exits

#### Scenario: Launched on a host with no resident instance

- **WHEN** a user runs it in a project directory on a host where no instance of theirs is running
- **THEN** the resident instance is started detached from the launch
- **AND** the launch registers the project, reports where to manage it and exits

#### Scenario: The same project directory twice

- **WHEN** a user runs it twice in the same project directory
- **THEN** the project is registered once
- **AND** no process is left running for it either time

### Requirement: A project runs at most once per host

The system SHALL NOT run two runtimes for the same project on one host, regardless of how each was initiated.

#### Scenario: A person acts on a project the host already runs

- **WHEN** a project that is already running on a host is started there again
- **THEN** no second runtime is created for it

#### Scenario: A remote start races a local registration

- **WHEN** a project is started remotely at the same moment as it is registered locally on the same host
- **THEN** exactly one runtime exists for that project afterwards

### Requirement: Single instance does not replace cross-host claims

One instance per user per host SHALL NOT be treated as deciding whether two hosts may run the same project. Where hosts share project storage, the cross-host claims SHALL continue to decide that. Hosts that support project instances each claim the project for their own machine and run it together. A runtime that cannot share a project SHALL keep every other host out while it runs.

#### Scenario: Two capable hosts share one project directory

- **WHEN** two hosts that support project instances share storage and each is asked to run the same project
- **THEN** each claims the project for its own machine and both run it
- **AND** each restores only its own tabs

#### Scenario: One host cannot share the project

- **WHEN** a project runs on a host whose runtime cannot share it with other machines
- **THEN** the claims keep every other host from starting it
- **AND** the per-host single-instance rule does not by itself permit both
