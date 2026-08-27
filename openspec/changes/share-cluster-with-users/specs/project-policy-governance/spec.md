## MODIFIED Requirements

### Requirement: Policy authority follows the deployment and cluster levels
The system administrator SHALL be the only identity that can change deployment default policy. The owner of a virtual cluster SHALL be able to change that cluster's default policy and a hosted project's override within the deployment's allowed launch profiles. A cluster member, project owner, or project user who does not own the hosting cluster SHALL be able to view effective policy for projects they can access but SHALL NOT modify cluster defaults or hosting-cluster project overrides. If a project has multiple hosting placements, each hosting cluster owner SHALL control only its own cluster default while the project override remains a shared launch-profile narrowing that any hosting owner may narrow but never widen beyond any placement.

#### Scenario: System administrator changes the deployment default
- **WHEN** the system administrator changes the deployment default policy
- **THEN** the system persists it and redistributes the resulting effective policy to connected project clients

#### Scenario: Cluster operator changes a hosted project's policy
- **WHEN** a cluster owner narrows the policy of a project hosted in their cluster within all higher-level bounds
- **THEN** the system persists the policy and distributes the new effective policy to connected project clients

#### Scenario: Project owner outside the hosting cluster attempts a policy change
- **WHEN** a cluster member or project owner who does not own the hosting cluster submits a cluster or project policy change
- **THEN** the server rejects the request
- **AND** the effective policy remains unchanged

#### Scenario: Account attempts to change the deployment default
- **WHEN** any account submits a change to the deployment default policy
- **THEN** the server rejects the request

### Requirement: Every project has a deterministic effective policy
Policy SHALL be expressed as the deployment default set by the system administrator, the cluster default of every durable hosting placement, and the per-project override. Effective allowed launch profiles SHALL be the monotone intersection of every applicable level, so no level or placement may widen a higher or peer profile restriction. `team_available` SHALL retain its existing default semantics: the lowest applicable project or cluster value that explicitly states a setting wins rather than treating the deployment default as a prohibition. New projects SHALL immediately inherit the effective default of their initial hosting cluster. Adding or removing a hosting placement SHALL deterministically recompute and redistribute effective policy. Removing a cluster or project override SHALL resume inheritance from the remaining applicable levels. Existing projects SHALL preserve effective tab-type behavior during migration, while newly introduced model restrictions SHALL default to the deployment compatibility policy until changed.

#### Scenario: New project is created
- **WHEN** an active member creates a project in a shared cluster
- **THEN** the project immediately receives the deployment and hosting-cluster effective defaults

#### Scenario: Cluster default cannot widen the deployment default
- **WHEN** a cluster owner sets a cluster default that permits a launch combination the deployment default disallows
- **THEN** the system rejects the change
- **AND** effective policy still disallows that combination

#### Scenario: Project override cannot widen its cluster default
- **WHEN** a project override permits a launch combination any hosting cluster default disallows
- **THEN** effective policy for that project still disallows that combination

#### Scenario: Second placement is added
- **WHEN** a project is durably placed in a second cluster with a narrower default
- **THEN** effective launch profiles become the intersection of deployment, both clusters, and project override
- **AND** an explicitly stated project team setting remains the lowest applicable team default
- **AND** connected clients receive the recomputed policy

#### Scenario: Existing project is upgraded
- **WHEN** an existing project's hosting evidence and policy are migrated to durable placements
- **THEN** its effective tab-type availability after migration matches its pre-upgrade behavior

#### Scenario: Project override is removed
- **WHEN** a project-specific override is removed
- **THEN** the project resumes the intersection of deployment and every hosting-cluster default
