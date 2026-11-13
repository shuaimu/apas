# host-supervision Specification

## Purpose
Defines the single resident instance each project host runs for a user: how a launch defers to it instead of becoming a rival, that a project has one runtime per host, and how this relates to cross-host claims.
## Requirements
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

### Requirement: One project stops and starts without disturbing the others

Stopping or starting one project SHALL affect that project only. It SHALL NOT interrupt other projects on the host and SHALL NOT replace the resident instance. Replacing the CLI from a project (Reboot CLI) is a replacement of the resident instance: it SHALL replace the instance and start again every project the instance was running, including the project that asked.

#### Scenario: One project is stopped and started again

- **WHEN** a project is stopped and then started on a host that runs other projects
- **THEN** that project stops and starts again
- **AND** the other projects on the host are undisturbed

#### Scenario: A project asks for the CLI to be replaced

- **WHEN** a person requests Reboot CLI from one project
- **THEN** the resident instance is replaced
- **AND** every project it was running, including that one, is running again afterwards

#### Scenario: The instance cannot replace itself

- **WHEN** the instance cannot replace itself after a project asked
- **THEN** the instance keeps running
- **AND** starts that project again

#### Scenario: The instance is upgraded

- **WHEN** the resident instance is replaced to apply an upgrade
- **THEN** the projects it was running are restored to running afterwards

### Requirement: A project's activity remains attributable

Every diagnostic record a project produces SHALL identify which project produced it. Sharing one process SHALL NOT make it impossible to tell one project's activity from another's.

#### Scenario: Two projects log at once

- **WHEN** two projects on a host produce diagnostic records
- **THEN** each record identifies the project it came from

### Requirement: The resident instance is never replaced automatically

The system SHALL NOT replace, restart, or stop a running resident instance except when a person asks for it. Discovering that a newer version is available SHALL NOT by itself cause a replacement, however it is discovered and however long the instance has been running.

#### Scenario: A newer version becomes available

- **WHEN** a newer version becomes available on a host whose instance is running
- **THEN** the running instance is left alone
- **AND** it keeps running the version it started with until someone asks for a replacement

#### Scenario: A host nobody visits

- **WHEN** a host runs for a long period with nobody acting on it
- **THEN** its instance is not replaced on its own

#### Scenario: A requested replacement

- **WHEN** a person requests a restart for a machine, or Reboot CLI from one of its projects
- **THEN** the instance is replaced, applying an available update first

### Requirement: A launch never stops the instance that is already running

A launch that finds a resident instance already running SHALL leave it running, including when the launching program is a newer version than the running instance. It SHALL report that the running instance is older and how to replace it, rather than replacing it. Starting an instance where none is running SHALL be unaffected.

#### Scenario: Launched from a newer version

- **WHEN** a user runs a newer version on a host whose older instance is running
- **THEN** the running instance is not stopped
- **AND** the projects it is running are undisturbed
- **AND** the user is told the running instance is older and how to replace it

#### Scenario: Launched where nothing is running

- **WHEN** a user runs it on a host with no instance running
- **THEN** an instance starts, as it does today

### Requirement: Stopping the instance stops its projects properly

When the resident instance is asked to stop, it SHALL stop the projects it is running before it exits, so that each project's state is saved and the processes it owns are ended. An ordinary request to terminate SHALL reach this path rather than ending the instance outright.

#### Scenario: The instance is asked to stop

- **WHEN** the resident instance is asked to terminate while running projects
- **THEN** each project is stopped through the same teardown an ordinary stop takes
- **AND** the instance exits afterwards

#### Scenario: Terminal hosts end with a requested stop

- **WHEN** the resident instance is asked to terminate while terminal panes are hosted
- **THEN** each project's stop ends its terminal hosts and their providers, as an ordinary project stop does

#### Scenario: Terminal hosts outlive a replacement or a loss

- **WHEN** the resident instance is replaced by a requested restart, or is lost without being asked to stop
- **THEN** the pane hosts keep their terminals
- **AND** an instance started within their adoption grace picks them back up

