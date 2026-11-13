# project-policy-governance Specification

## Purpose

Defines cluster-governed project capability policy for model/provider availability, with visible effective settings and enforcement at every launch boundary.
## Requirements
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

### Requirement: Model and provider policy is enforced at launch
The effective project policy SHALL identify which supported agent frontend, API backend, model, and terminal combinations may be launched. Web interfaces SHALL offer only allowed combinations, and the server and project host SHALL independently reject disallowed requests to create a pane or to change an existing pane's combination.

The allowlist SHALL govern which combinations may be brought into existence, not which existing panes may run. Resuming, rebooting, or restarting an existing pane SHALL NOT be refused because its combination is no longer allowed, and such a pane SHALL NOT prevent the project host from being restarted. Restrictions that are not the allowlist are unaffected: a pane whose backend has been retired SHALL still refuse to launch, and a legacy bot pane SHALL stay read-only history.

#### Scenario: User launches an allowed model
- **WHEN** a project member selects a combination permitted by the effective project policy
- **THEN** the system allows the pane to launch

#### Scenario: Stale client requests a disallowed model
- **WHEN** a client requests to create a pane with a model/provider combination that the current effective policy disallows
- **THEN** the server or project host rejects the request with a policy-specific error
- **AND** no disallowed process is launched

#### Scenario: Policy changes while a model is running
- **WHEN** an administrator disallows a combination already used by a running pane
- **THEN** the system prevents new panes and backend switches to that combination
- **AND** reports the existing pane as policy-noncompliant without silently terminating it

#### Scenario: Relaunching a pane whose combination is no longer allowed
- **WHEN** a member resumes, reboots, or restarts an existing pane whose combination the current policy disallows
- **THEN** the pane relaunches
- **AND** the request is not refused on account of the allowlist

#### Scenario: Switching an existing pane to a disallowed combination
- **WHEN** a member changes an existing pane's model or provider to a combination the current policy disallows
- **THEN** the request is rejected
- **AND** the pane keeps the combination it had

#### Scenario: A noncompliant pane does not block the project host
- **WHEN** a project host restart is requested for a project containing a pane whose combination is no longer allowed
- **THEN** the restart proceeds

#### Scenario: Retired backends are still refused
- **WHEN** a member resumes or reboots an existing pane whose backend has been retired
- **THEN** the request is still refused

#### Scenario: A legacy bot pane is not relaunched
- **WHEN** a member resumes or reboots an existing pane that belonged to the retired bot
- **THEN** the request is refused with the retired-bot explanation
- **AND** the pane's history stays readable

#### Scenario: Noncompliance is reported without implying a restriction
- **WHEN** a project contains panes outside the current allowlist
- **THEN** the system may identify them
- **AND** SHALL NOT state that they cannot be relaunched

### Requirement: Every project has a deterministic effective policy
Policy SHALL be expressed as the deployment default set by the system administrator, the cluster default of every durable hosting placement, and the per-project override. Effective allowed launch profiles SHALL be the monotone intersection of every applicable level, so no level or placement may widen a higher or peer profile restriction. New projects SHALL immediately inherit the effective default of their initial hosting cluster. A project that a cluster member creates under a membership naming a default launch profile SHALL also receive a project override allowing only that profile. Adding or removing a hosting placement SHALL deterministically recompute and redistribute effective policy. Removing a cluster or project override SHALL resume inheritance from the remaining applicable levels. Existing projects SHALL preserve effective tab-type behavior during migration, while newly introduced model restrictions SHALL default to the deployment compatibility policy until changed.

#### Scenario: New project is created
- **WHEN** an active member creates a project in a shared cluster
- **THEN** the project immediately receives the deployment and hosting-cluster effective defaults

#### Scenario: Member with a default launch profile creates a project
- **WHEN** a member whose membership names a default launch profile creates a project in that cluster
- **THEN** the project's override allows only that profile

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
- **AND** connected clients receive the recomputed policy

#### Scenario: Existing project is upgraded
- **WHEN** an existing project's hosting evidence and policy are migrated to durable placements
- **THEN** its effective tab-type availability after migration matches its pre-upgrade behavior

#### Scenario: Project override is removed
- **WHEN** a project-specific override is removed
- **THEN** the project resumes the intersection of deployment and every hosting-cluster default

### Requirement: Retained policy fields for removed features decide nothing
Where a policy field survives only so that older clients keep parsing responses, the system SHALL NOT let it affect any decision, and SHALL NOT present it as a setting a person can change.

#### Scenario: A stored policy still carries the field
- **WHEN** a project's stored policy still carries a field for a removed feature
- **THEN** the effective policy is unchanged by it
- **AND** no launch is permitted or refused on account of it

#### Scenario: An older client reads the policy
- **WHEN** a client that predates the removal reads a policy response
- **THEN** the response still parses

#### Scenario: The field is not offered
- **WHEN** a person views the policy they may edit
- **THEN** the retained field is not presented as a setting

### Requirement: Operational project settings remain distinct from cluster policy
Moving capability policy to cluster administration SHALL NOT by itself remove an owner's existing non-policy project operations, such as restricting which tab types the project allows or using capabilities that policy permits. Authorization SHALL distinguish cluster-governed availability from ordinary operation within that boundary.

#### Scenario: Owner uses an allowed project operation
- **WHEN** a project owner changes the project's allowed tab types or launches a policy-allowed pane
- **THEN** the system permits the operation under normal project access rules

#### Scenario: Owner changes a combined legacy settings payload
- **WHEN** a legacy settings request contains both owner-operable values and cluster-governed policy values
- **THEN** the system prevents unauthorized policy changes
- **AND** does not treat ownership as cluster-administrator authority

### Requirement: DeepSeek is governed by one terminal launch profile
The supported launch-profile catalog SHALL identify DeepSeek as the single profile `terminal:claude:deepseek:deepseek-flash`. Effective project policy SHALL authorize it independently of the official Claude terminal profile, although both use the Claude runtime. When the server normalizes stored policy, an allowlist entry `terminal:claude:deepseek:deepseek-v4-flash` or `agent:claude:deepseek:deepseek-v4-flash` SHALL be carried forward to the current profile. A stored DeepSeek Pro profile SHALL be removed and SHALL NOT be replaced by another profile.

#### Scenario: Administrator reviews DeepSeek capabilities
- **WHEN** an administrator opens a launch-policy editor
- **THEN** DeepSeek 4.1 Flash Terminal appears as one allowlist capability
- **AND** it does not ask for a separate API key

#### Scenario: Policy permits the DeepSeek profile
- **WHEN** the effective project policy contains `terminal:claude:deepseek:deepseek-flash`
- **THEN** new-pane interfaces offer the DeepSeek terminal choice
- **AND** a launch request for it is authorized

#### Scenario: Policy lacks the DeepSeek profile
- **WHEN** the effective project policy contains the official Claude terminal profile but not the DeepSeek profile
- **THEN** the DeepSeek choice is not offered
- **AND** a stale request for it is rejected

#### Scenario: Stored allowlist names the older Flash key
- **WHEN** the server normalizes a stored allowlist that contains `terminal:claude:deepseek:deepseek-v4-flash` or `agent:claude:deepseek:deepseek-v4-flash`
- **THEN** the allowlist contains `terminal:claude:deepseek:deepseek-flash` in its place

#### Scenario: Stored allowlist names only DeepSeek Pro
- **WHEN** the server normalizes a stored allowlist whose only entry is a DeepSeek Pro profile
- **THEN** the result is an explicit empty allowlist
- **AND** no other profile is granted in its place

