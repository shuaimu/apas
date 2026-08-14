## Purpose

Defines safe, observable controls for reconnecting a project CLI transport or replacing the CLI process without conflating the two operations or unnecessarily interrupting active work.

## ADDED Requirements

### Requirement: Transport reconnect does not restart project work
The system SHALL allow an authorized user to request reconnection of an attached project's server transport without exiting or replacing the project CLI process. A transport reconnect SHALL leave every pane process, terminal instance, active turn, input queue, and project-local watcher unchanged.

#### Scenario: User reconnects an unhealthy server transport
- **WHEN** an authorized user requests a server reconnect for an attached project
- **THEN** the project CLI closes and re-establishes only its server transport
- **AND** all pane processes retain their process identity and continue running

#### Scenario: Agent produces output during reconnect
- **WHEN** a pane produces output while the project CLI is re-establishing its server transport
- **THEN** the transport recovery path preserves or replays that output through its existing bounded reconnect behavior
- **AND** does not restart the pane to recover delivery

#### Scenario: Reconnect is requested repeatedly
- **WHEN** multiple reconnect requests overlap for the same project CLI
- **THEN** the system coalesces them into at most one active reconnect attempt
- **AND** does not create concurrent project sessions or duplicate pane processes

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
Reconnect and reboot operations SHALL be correlated to one project and one request identifier. The system SHALL report accepted, in-progress, succeeded, or failed outcomes to authorized clients and SHALL NOT infer success solely from a transient disconnect.

#### Scenario: Transport reconnect succeeds
- **WHEN** a requested transport reconnect completes registration and session reconciliation
- **THEN** the requesting client receives a success outcome for that request
- **AND** ordinary project availability is restored without a CLI-reboot status

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
- **WHEN** a non-member or user whose membership was revoked requests reconnect or reboot
- **THEN** the server rejects the operation
- **AND** sends no lifecycle command to the project CLI

#### Scenario: Older CLI receives no reconnect command
- **WHEN** the attached project CLI does not advertise transport-reconnect support
- **THEN** the interface hides or disables the reconnect control with an upgrade explanation
- **AND** the server does not substitute a destructive full reboot

#### Scenario: Request targets another project
- **WHEN** a lifecycle request carries a stale or mismatched project session identifier
- **THEN** the server rejects or routes it only after resolving current project ownership and attachment
- **AND** no other project's transport or processes are affected

