## Purpose

Defines privacy-scoped reporting of observed project token and cost counters so cluster owners can understand shared-cluster consumption.

## ADDED Requirements

### Requirement: Usage is aggregated across stable time windows
The system SHALL aggregate observed prompt, response, token, cache-token, and reported cost counters for hosted projects into lifetime, trailing-seven-day, and current-UTC-day windows. Aggregation SHALL include all persisted sessions and panes of each canonical project without double counting retries, and SHALL label unavailable or provider-unreported values as unavailable rather than inventing them.

#### Scenario: Project has multiple panes and sessions
- **WHEN** usage is requested for a project with counters across multiple panes and sessions
- **THEN** each time window contains their aggregate and supports a per-project breakdown

#### Scenario: Provider omits a cost
- **WHEN** a provider reports tokens but no cost for an event
- **THEN** token counters remain visible
- **AND** the system does not present a fabricated monetary cost for that event

### Requirement: Cluster owners can inspect all hosted-project usage
The cluster owner SHALL be able to inspect total observed usage for every project placed in their cluster, including projects owned by members. The view SHALL support breakdown by canonical project and project owner and SHALL identify projects that have no reported usage. Project-owner grouping SHALL be presented as organizational attribution, not proof of which human caused every individual provider charge.

#### Scenario: Owner opens cluster usage
- **WHEN** a cluster owner opens usage administration
- **THEN** the system returns only usage from projects placed in that cluster
- **AND** includes member-owned projects in totals and breakdowns

#### Scenario: Project is placed in multiple clusters
- **WHEN** a canonical project has usage while placed in more than one cluster
- **THEN** each hosting owner can inspect the project's usage in their own cluster inventory
- **AND** deployment-wide totals do not duplicate the underlying usage records

### Requirement: Members see only project-scoped usage
An active account SHALL be able to inspect observed usage for projects it owns or belongs to. Cluster membership alone SHALL NOT reveal whole-cluster totals, another member's project usage, provider credential details, or the owner's provider subscription quota. Revoked membership SHALL NOT remove access to usage of a project the account still owns or belongs to.

#### Scenario: Member views their project usage
- **WHEN** a cluster member requests usage for a project they own or belong to
- **THEN** the system returns that project's observed counters

#### Scenario: Member requests cluster-wide usage
- **WHEN** a cluster member who is not the owner requests cluster totals or another member's project usage
- **THEN** the system denies the request without revealing aggregate values

#### Scenario: Member views machine readiness
- **WHEN** a member selects a shared machine for an allowed project operation
- **THEN** the system may report provider availability needed for that operation
- **AND** does not expose API keys, credential contents, or owner subscription-quota details

### Requirement: Reporting does not imply enforcement or billing accuracy
Cluster usage reporting SHALL be informational and SHALL NOT claim billing-grade accuracy or enforce hard spending quotas. Cluster owners SHALL manage consumption through membership revocation, project lifecycle controls, and existing monotone policy restrictions.

#### Scenario: Usage crosses an owner-selected expectation
- **WHEN** reported usage exceeds an amount the owner informally considers a budget
- **THEN** the system does not claim that an automatic hard quota was enforced
- **AND** retains the existing owner controls for restricting future work
