## MODIFIED Requirements

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

## ADDED Requirements

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

## REMOVED Requirements

### Requirement: Only cluster administrators modify governed project policy

**Reason**: The deployment-wide cluster-administrator role that held this authority no longer exists. Policy authority is replaced by "Policy authority follows the deployment and cluster levels", which splits it between the system administrator (deployment default) and the operator of the cluster that hosts a project (cluster default and project override), while keeping non-operating owners and users read-only.

**Migration**: Existing project overrides and the existing deployment default are retained unchanged and become the project and deployment levels of the new resolution order. Accounts that previously held the cluster-administrator role keep policy authority over the projects hosted in their own cluster and lose it elsewhere.
