# cli-lifecycle-control Specification

## Purpose

Defines how a project CLI recovers its server transport automatically, and the separate, observable control for replacing the CLI process, so that recovering a connection is never conflated with interrupting active work.

## Requirements

### Requirement: Transport recovery is automatic and does not restart project work
A project CLI SHALL re-establish a lost or unhealthy server transport on its own, retrying with bounded exponential backoff, without exiting or replacing the CLI process. Transport recovery SHALL leave every pane process, terminal instance, active turn, input queue, and project-local watcher unchanged.

Transport recovery SHALL NOT be exposed as a user-facing control. It is plumbing: a control would ask a user to diagnose a connection state they cannot observe, and offering one invites reaching for a full reboot when the connection is merely degraded. A full CLI reboot SHALL NOT be presented as the remedy for a transport-only problem.

#### Scenario: Transport drops while the project is attached
- **WHEN** an attached project CLI loses its server transport
- **THEN** the CLI re-establishes only that transport, without user action
- **AND** all pane processes retain their process identity and continue running

#### Scenario: Agent produces output during recovery
- **WHEN** a pane produces output while the project CLI is re-establishing its server transport
- **THEN** the transport recovery path preserves or replays that output through its existing bounded reconnect behavior
- **AND** does not restart the pane to recover delivery

#### Scenario: Recovery keeps failing
- **WHEN** successive reconnection attempts fail
- **THEN** the CLI backs off between attempts up to a bounded maximum interval and keeps retrying
- **AND** does not create concurrent project sessions or duplicate pane processes

#### Scenario: No transport control is offered
- **WHEN** a user opens lifecycle actions for an attached project
- **THEN** no transport-reconnect action is presented, whatever the CLI's version
- **AND** the interface does not substitute a full reboot for connection recovery

### Requirement: Full CLI reboot remains a distinct operation
The system SHALL retain a separate full CLI reboot operation for installing or activating a new APAS binary. Before starting it, the interface SHALL explain which live pane kinds the connected CLI can preserve and which pane kinds will restart or resume. A full reboot SHALL NOT be presented as the normal remedy for a transport-only problem.

#### Scenario: Project supports persistent terminal adoption
- **WHEN** a user initiates a full CLI reboot for a project whose terminal runtimes can be adopted
- **THEN** the interface identifies supported terminal panes as preservable
- **AND** warns that legacy structured agent panes may restart

#### Scenario: Project lacks persistent terminal adoption
- **WHEN** a user initiates a full CLI reboot against a project CLI that cannot preserve terminal runtimes
- **THEN** the interface clearly warns that terminal agents will be restarted and resumed where supported
- **AND** requires explicit confirmation before continuing

#### Scenario: Update preparation fails
- **WHEN** the CLI cannot fetch, build, validate, or install the replacement binary
- **THEN** the existing project CLI and all pane processes remain running
- **AND** the user receives an actionable failure result instead of a false reboot success

### Requirement: Lifecycle operations report authoritative progress and outcome
Reboot operations SHALL be correlated to one project and one request identifier. The system SHALL report accepted, in-progress, succeeded, or failed outcomes to authorized clients and SHALL NOT infer success solely from a transient disconnect.

#### Scenario: Full reboot disconnects during handoff
- **WHEN** the old CLI transport closes after accepting a full reboot
- **THEN** the web interface presents the operation as in progress
- **AND** reports success only after the replacement CLI registers and reconciles its pane roster

#### Scenario: Replacement CLI does not return
- **WHEN** a full reboot exceeds its bounded completion deadline
- **THEN** the operation is reported as failed or timed out with recovery guidance
- **AND** the system does not claim that terminal adoption succeeded

### Requirement: Lifecycle controls are authorized and rollout-compatible
The server SHALL authorize each lifecycle request against current project access and route it only to the requested project's owning CLI. New controls SHALL be capability-gated so mixed-version participants fail safely without disconnecting compatible sessions.

#### Scenario: User lacks current project access
- **WHEN** a non-member or user whose membership was revoked requests a reboot
- **THEN** the server rejects the operation
- **AND** sends no lifecycle command to the project CLI

#### Scenario: Request targets another project
- **WHEN** a lifecycle request carries a stale or mismatched project session identifier
- **THEN** the server rejects or routes it only after resolving current project ownership and attachment
- **AND** no other project's transport or processes are affected
