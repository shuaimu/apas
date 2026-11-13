## ADDED Requirements

### Requirement: Autonomous jobs have independent default-deny policy
The system SHALL govern autonomous jobs through a separately typed policy, disabled when deployment enablement is absent, false or malformed. Job policy SHALL define allowed worker/model, sandbox and verification profiles, draft-publication permission, finite resource/time/spend/attempt ceilings and concurrency bounds. Existing terminal launch permissions, team compatibility fields, managed flags and legacy automatic-approval or auto-merge flags SHALL NOT enable jobs. Existing policy administration authority SHALL remain unchanged.

Persistent-worker activation SHALL also require the independently default-denied
recurring policy from `unify-autonomous-workers`. The finite enablement above
alone SHALL NOT grant recurrence. Rolling ceilings narrow by minimum,
minimum inspection cadence narrows by maximum, and every hosting placement
participates in the same policy/source-revision digest.

#### Scenario: Existing deployment upgrades
- **WHEN** existing configuration and projects contain no autonomous-job policy
- **THEN** no autonomous job can start until explicitly enabled and provisioned
- **AND** existing interactive panes retain their current behavior and permissions

#### Scenario: Job policy data is malformed
- **WHEN** an applicable autonomous-job policy cannot be parsed or resolved safely
- **THEN** admission fails closed with a configuration reason
- **AND** supported-terminal defaults or inherited team fields do not grant execution

### Requirement: Job policy narrows across every hosting placement
Effective autonomous-job policy SHALL combine deployment policy, every durable hosting cluster's policy and the project override. Enablement SHALL require all applicable levels to permit it, allowlists SHALL intersect, and finite ceilings SHALL take the minimum. Omitted lower-level overrides SHALL inherit, while an explicit malformed override SHALL deny. Lower-level writes that widen higher-level bounds SHALL be rejected. Clients SHALL distinguish requested settings, effective settings and their inherited constraints.

#### Scenario: Multi-hosted project has different cluster ceilings
- **WHEN** one hosting cluster permits a smaller allowance or fewer profiles than another
- **THEN** the effective project job policy respects their intersection regardless of selection order
- **AND** choosing a more permissive target does not bypass the other applicable bound

#### Scenario: Ordinary project user changes job policy
- **WHEN** a user without existing policy-administration authority attempts to enable jobs or raise limits
- **THEN** the write is rejected
- **AND** content access or job visibility is not treated as policy authority

### Requirement: Policy snapshots are checked at privileged job transitions
Admission SHALL bind the canonical resolved policy and its source revisions, not merely a coarse maximum version. Dispatch, lease renewal, explicit retry and publication SHALL recheck current effective policy and authority. Disabling a required profile, reducing a per-run allowance below its consumption or revoking execution permission SHALL durably cancel affected execution. Reduced concurrency SHALL block additional dispatch until under the new ceiling rather than arbitrarily killing otherwise permitted runs. Policy enforcement SHALL enumerate all affected jobs.

#### Scenario: Policy narrows after admission
- **WHEN** a queued or active job no longer satisfies a current per-run permission or allowance
- **THEN** queued work cannot dispatch and active work enters the durable stop path
- **AND** a previously passing candidate cannot bypass current publication permission

#### Scenario: Cluster has more jobs than one page
- **WHEN** policy changes affect runs across more projects than a listing page returns
- **THEN** enforcement covers all affected runs, including offline hosts
- **AND** untouched later pages cannot continue under obsolete authority

### Requirement: Operational readiness is separate from permission
A policy-permitted profile SHALL still be unavailable until its target proves the required worker, isolation, credentials, verification and cancellation prerequisites. Administrative surfaces SHALL expose safe unavailable reasons without revealing secrets or private host paths. Administrative audit SHALL record actor kind, project/cluster scope, policy and job identities without task content, answers, credentials or raw transcripts.

#### Scenario: Policy permits a profile whose sandbox is broken
- **WHEN** a user selects that profile on a host with failed readiness
- **THEN** the server refuses admission or dispatch with a provisioning reason
- **AND** neither the UI nor the worker silently substitutes an unrestricted profile
