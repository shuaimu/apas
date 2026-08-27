## Purpose

Defines safe, placement-aware creation and operation of member-owned projects on machines belonging to a shared virtual cluster.

## ADDED Requirements

### Requirement: Members discover only eligible shared machines
An active cluster member SHALL be able to discover online machines owned by that cluster which are eligible for shared project provisioning. Shared-machine responses SHALL identify the owning cluster and the capabilities needed for project creation, while omitting provider secrets, owner-only configuration values, unrelated local projects, and owner-only machine controls.

#### Scenario: Member selects a shared machine
- **WHEN** an active member opens project creation for a shared cluster with online eligible machines
- **THEN** the interface offers those machines and identifies their owning cluster
- **AND** does not list unrelated projects or owner-only controls

#### Scenario: Shared machine is offline or ineligible
- **WHEN** a shared machine is offline, behind an incompatible protocol, or disabled for provisioning
- **THEN** the system does not offer it as an eligible creation target

#### Scenario: Former member refreshes machines
- **WHEN** a revoked member refreshes or receives a machine update
- **THEN** machines from the revoked cluster are absent

### Requirement: Shared cloning accepts public GitHub repositories without owner credentials
Project creation by a cluster member on another account's machine SHALL accept only a canonical public GitHub HTTPS repository URL with no embedded credential. The clone SHALL run non-interactively without using the machine owner's Git credential helpers, askpass programs, SSH identities, or an existing checkout's authenticated origin. The destination SHALL remain inside the daemon-managed projects root regardless of member-supplied path input. Clone or validation failure SHALL leave no registered project or partial destination.

#### Scenario: Member clones a public GitHub repository
- **WHEN** an active member submits a valid public `https://github.com/<owner>/<repository>` URL, a valid project name, and an eligible shared machine
- **THEN** the system clones it without consulting owner Git credentials
- **AND** creates the checkout only under the daemon-managed projects root

#### Scenario: Member submits a private or credential-bearing URL
- **WHEN** a member submits an SSH URL, a non-GitHub host, embedded credentials, or a repository that cannot be cloned anonymously
- **THEN** the system rejects or fails the operation without falling back to owner credentials
- **AND** reports a safe error that contains no secret

#### Scenario: Member supplies a destination escape
- **WHEN** a member supplies an absolute base path or traversal-like project name
- **THEN** the system ignores or rejects the unsafe destination input
- **AND** writes nothing outside the daemon-managed projects root

### Requirement: Provisioning preserves requester ownership and hosting placement
A successful member provisioning request SHALL atomically associate the new canonical project with the requesting member as its sole project owner and the selected owner's virtual cluster as a hosting placement. The project SHALL inherit the effective policy of every hosting placement. Request retries SHALL be idempotent, and completion or failure SHALL be delivered to the requesting member even though the daemon belongs to the cluster owner.

#### Scenario: Member project is created successfully
- **WHEN** a member's clone succeeds and the project runtime registers
- **THEN** the requesting member becomes the canonical project's owner
- **AND** the selected cluster receives a durable hosting placement
- **AND** the cluster owner sees the project in hosted inventory

#### Scenario: Daemon identity differs from requester
- **WHEN** a cluster-owner daemon registers a project created by a member request
- **THEN** the system uses the authenticated provisioning provenance rather than daemon identity for project ownership

#### Scenario: Member retries a request
- **WHEN** the same member repeats a provisioning request with the same idempotency key
- **THEN** the system returns the original result and does not create a second checkout or project

#### Scenario: Requester loses membership before completion
- **WHEN** membership is revoked while a provisioning request is pending
- **THEN** the system cancels or rejects final registration
- **AND** cleans up any unregistered partial checkout

### Requirement: Shared members receive project-scoped runtime control
An active cluster member SHALL be able to start, stop, attach to, and operate a project on shared-cluster machines only when they own or belong to that project, the project is placed in that cluster, and effective policy permits the operation. The server SHALL authorize every mutation from current persisted membership and project access rather than trusting a client-supplied cluster, machine, project, or path association.

#### Scenario: Member starts their hosted project
- **WHEN** an active member starts a project they own that is placed on the shared cluster
- **THEN** the server permits the project-scoped start subject to lifecycle and policy

#### Scenario: Member targets an unrelated local project
- **WHEN** a member asks a shared daemon to start, stop, attach to, or mutate a project they do not own or belong to
- **THEN** the server denies the operation

#### Scenario: Member forges a machine and project pairing
- **WHEN** a member submits an eligible shared machine ID with a project that is not placed on that cluster or machine
- **THEN** the server denies the operation without forwarding it to the daemon
