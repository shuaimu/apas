## ADDED Requirements

### Requirement: Project sharing and cluster sharing grant independent scopes
An active project user SHALL receive project content access independently of cluster membership. For a runtime instance hosted in the project owner's cluster, that project membership SHALL also grant project-scoped runtime access without granting machine discovery, project provisioning, cluster administration, or access to unrelated projects. For a runtime instance hosted by a third-party cluster, project membership SHALL NOT bypass the hosting cluster's active membership and machine restrictions. Cluster membership SHALL grant only the configured shared-machine capabilities and SHALL NOT grant content access to a project the member neither owns nor belongs to.

#### Scenario: Owner shares an owner-hosted project
- **WHEN** a project owner adds an active account as a project user and the selected runtime instance is hosted in that owner's cluster
- **THEN** the project user can open the project's conversations and operate that runtime
- **AND** the project user receives no access to the owner's machine inventory, project creation surface, cluster controls, or unrelated projects from that project share

#### Scenario: Project user opens a third-party-hosted instance
- **WHEN** a project user opens an instance hosted by a cluster that neither the project user nor the project owner operates
- **THEN** the user retains project content access
- **AND** runtime attachment and mutations require active permission for the hosting cluster and the exact machine

#### Scenario: Cluster member requests an unrelated project
- **WHEN** an active cluster member requests a project they neither own nor belong to
- **THEN** the system denies project content and runtime access
- **AND** does not reveal the project's pane, conversation, usage, or policy data

#### Scenario: User holds both grants
- **WHEN** an active account belongs to a project and has permission for the third-party cluster and machine hosting its selected instance
- **THEN** the account can attach to and operate that project instance
- **AND** each grant remains independently revocable

#### Scenario: Third-party cluster access is revoked
- **WHEN** a project user loses membership or machine permission for the third-party cluster hosting an instance
- **THEN** subsequent attachment and runtime mutations for that instance are denied
- **AND** the project membership and persisted project content remain intact

### Requirement: Sharing interfaces explain the granted authority
The project-sharing interface SHALL identify its grant as project access and explain whether the project's current hosting permits runtime use without cluster access. The cluster-sharing interface SHALL identify its grant as machine access, name the allowed machines and default launch profile, and explain that it does not expose unrelated projects. Neither interface SHALL describe one grant as conferring the other.

#### Scenario: Owner opens project sharing
- **WHEN** a project owner opens the project-access interface
- **THEN** the interface explains that the share applies only to that project
- **AND** distinguishes owner-hosted runtime access from a third-party host that separately requires cluster permission

#### Scenario: Owner opens cluster sharing
- **WHEN** a cluster owner opens the cluster-sharing interface
- **THEN** the interface explains that the member can use only selected machines under the configured defaults
- **AND** states that project content remains limited to projects the member owns or belongs to
