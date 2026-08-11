# project-policy-governance Specification

## Purpose

Defines cluster-admin-governed project capability policy for model/provider availability and team mode, with visible effective settings and enforcement at every launch boundary.

## Requirements

### Requirement: Only cluster administrators modify governed project policy
The system SHALL reserve changes to a project's allowed model/provider combinations and team-mode availability for active cluster administrators. Project owners and project users SHALL be able to view the effective policy but SHALL NOT modify it.

#### Scenario: Cluster administrator updates policy
- **WHEN** an active cluster administrator changes a project's allowed models or team-mode availability
- **THEN** the system persists the policy and distributes the new effective policy to connected project clients

#### Scenario: Project owner attempts to update policy
- **WHEN** a project owner submits a model-policy or team-mode-policy change
- **THEN** the server rejects the request
- **AND** the effective policy remains unchanged

### Requirement: Model and provider policy is enforced at launch
The effective project policy SHALL identify which supported agent frontend, API backend, model, and terminal combinations may be launched. Web interfaces SHALL offer only allowed combinations, and the server and project host SHALL independently reject disallowed launch or backend-switch requests.

#### Scenario: User launches an allowed model
- **WHEN** a project member selects a combination permitted by the effective project policy
- **THEN** the system allows the pane or team member to launch

#### Scenario: Stale client requests a disallowed model
- **WHEN** a client requests a model/provider combination that the current effective policy disallows
- **THEN** the server or project host rejects the request with a policy-specific error
- **AND** no disallowed process is launched

#### Scenario: Policy changes while a model is running
- **WHEN** an administrator disallows a combination already used by a running pane
- **THEN** the system prevents new launches and backend switches to that combination
- **AND** reports the existing pane as policy-noncompliant without silently terminating it

### Requirement: Team-mode availability is enforced consistently
The effective project policy SHALL state whether managed team mode is available. When unavailable, users SHALL NOT start or add managed team panes; disabling it SHALL interrupt and pause currently running managed panes while preserving ordinary side chats.

#### Scenario: User starts an allowed team
- **WHEN** team mode is available and a project member starts the team
- **THEN** the system permits managed team panes subject to the model policy

#### Scenario: Administrator disables team mode
- **WHEN** a cluster administrator disables team mode for a project with running managed panes
- **THEN** the system interrupts and pauses the managed panes
- **AND** leaves unmanaged project panes intact

#### Scenario: Project owner enables team mode locally
- **WHEN** a project owner uses a stale client or edits local project state to enable team mode contrary to cluster policy
- **THEN** the server and project host continue to refuse managed-team launch

### Requirement: Every project has a deterministic effective policy
New projects SHALL inherit the cluster's current default policy, and a cluster administrator SHALL be able to override it per project. Existing projects SHALL preserve their effective team-mode and tab-type behavior during migration, while newly introduced model restrictions SHALL default to the cluster's compatibility policy until an administrator changes them.

#### Scenario: New project is created
- **WHEN** an active cluster user creates a project
- **THEN** the project immediately receives the current cluster-default capability policy

#### Scenario: Existing project is upgraded
- **WHEN** an existing project's local flags are migrated
- **THEN** its effective team-mode and tab-type availability after migration matches its pre-upgrade behavior

#### Scenario: Project override is removed
- **WHEN** a cluster administrator removes a project-specific override
- **THEN** the project resumes using the current cluster-default value for that policy field

### Requirement: Operational project settings remain distinct from cluster policy
Moving capability policy to cluster administration SHALL NOT by itself remove an owner's existing non-policy project operations, such as managing the project goal or using capabilities that policy permits. Authorization SHALL distinguish cluster-governed availability from ordinary operation within that boundary.

#### Scenario: Owner uses an allowed project operation
- **WHEN** a project owner updates the project goal or launches a policy-allowed pane
- **THEN** the system permits the operation under normal project access rules

#### Scenario: Owner changes a combined legacy settings payload
- **WHEN** a legacy settings request contains both owner-operable values and cluster-governed policy values
- **THEN** the system prevents unauthorized policy changes
- **AND** does not treat ownership as cluster-administrator authority
