# project-policy-governance Specification

## Purpose

Defines cluster-admin-governed project capability policy for model/provider availability and team mode, with visible effective settings and enforcement at every launch boundary.

## Requirements

### Requirement: Policy authority follows the deployment and cluster levels
The system administrator SHALL be the only identity that can change the deployment default policy. The operator of the virtual cluster hosting a project SHALL be able to change that cluster's default policy and that project's override, within the launch combinations the deployment default allows. A project owner or project user who does not operate the hosting cluster SHALL be able to view the effective policy but SHALL NOT modify it, and no account SHALL modify policy in a cluster it does not operate.

#### Scenario: System administrator changes the deployment default
- **WHEN** the system administrator changes the deployment default policy
- **THEN** the system persists it and redistributes the resulting effective policy to connected project clients

#### Scenario: Cluster operator changes a hosted project's policy
- **WHEN** a cluster operator changes the policy of a project hosted in their cluster, within the deployment default
- **THEN** the system persists the policy and distributes the new effective policy to that project's connected clients

#### Scenario: Project owner outside the hosting cluster attempts a policy change
- **WHEN** a project owner who does not operate the hosting cluster submits a model-policy or team-mode-policy change
- **THEN** the server rejects the request
- **AND** the effective policy remains unchanged

#### Scenario: Account attempts to change the deployment default
- **WHEN** any account submits a change to the deployment default policy
- **THEN** the server rejects the request

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
Policy SHALL be expressed at three levels — the deployment default set by the system administrator, the cluster default set by the operator of the hosting virtual cluster, and the per-project override. The allowed launch combinations SHALL be the monotone narrowing of the three: a level SHALL only restrict what the level above it allows and SHALL NOT widen it. Team-mode availability SHALL instead resolve to the value stated by the lowest level that states one, because the deployment value is a default rather than a prohibition and individual projects have always been able to turn team mode on against it. New projects SHALL inherit the effective default of the cluster hosting them. Removing a level's override SHALL make the project resume the level above. Existing projects SHALL preserve their effective team-mode and tab-type behavior during migration, while newly introduced model restrictions SHALL default to the deployment's compatibility policy until they are changed.

#### Scenario: New project is created
- **WHEN** an active account creates a project
- **THEN** the project immediately receives the effective default of the virtual cluster hosting it

#### Scenario: Cluster default cannot widen the deployment default
- **WHEN** a cluster operator sets a cluster default that permits a launch combination the deployment default disallows
- **THEN** the system rejects the change
- **AND** the effective policy still disallows that combination

#### Scenario: Team mode is turned on for one project
- **WHEN** a project override enables team mode while the deployment default leaves it off
- **THEN** that project has team mode available
- **AND** projects that state no value keep the default

#### Scenario: Project override cannot widen its cluster default
- **WHEN** a project override permits a launch combination the effective cluster default disallows
- **THEN** the effective policy for that project still disallows that combination

#### Scenario: Existing project is upgraded
- **WHEN** an existing project's policy is migrated
- **THEN** its effective team-mode and tab-type availability after migration matches its pre-upgrade behavior

#### Scenario: Project override is removed
- **WHEN** a project-specific override is removed
- **THEN** the project resumes using the effective default of its hosting cluster

### Requirement: Operational project settings remain distinct from cluster policy
Moving capability policy to cluster administration SHALL NOT by itself remove an owner's existing non-policy project operations, such as managing the project goal or using capabilities that policy permits. Authorization SHALL distinguish cluster-governed availability from ordinary operation within that boundary.

#### Scenario: Owner uses an allowed project operation
- **WHEN** a project owner updates the project goal or launches a policy-allowed pane
- **THEN** the system permits the operation under normal project access rules

#### Scenario: Owner changes a combined legacy settings payload
- **WHEN** a legacy settings request contains both owner-operable values and cluster-governed policy values
- **THEN** the system prevents unauthorized policy changes
- **AND** does not treat ownership as cluster-administrator authority
