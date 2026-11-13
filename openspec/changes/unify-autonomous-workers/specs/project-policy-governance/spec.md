## ADDED Requirements

### Requirement: Recurring worker permission is independent and default denied
Effective policy SHALL separately permit recurring autonomous workers and define allowed execution profiles, finite per-run and rolling-worker allowances, concurrency, a minimum inspection interval, and draft-publication permission. Missing, false, or malformed recurring-worker enablement SHALL deny activation and automatic dispatch. Existing finite-job enablement, pane launch policy, role metadata, team flags, or legacy bot/automatic-approval flags SHALL NOT grant recurring authority. Existing deployment/cluster/project policy-administration authority SHALL remain unchanged.

#### Scenario: A deployment already permits finite jobs
- **WHEN** it upgrades without explicitly enabling recurring workers
- **THEN** no worker can start or renew recurring authority merely because finite jobs were allowed
- **AND** ordinary interactive panes remain governed by their existing policy

#### Scenario: A worker tries to change policy through its tools
- **WHEN** it proposes a more permissive profile, interval, or budget
- **THEN** that proposal cannot modify effective policy or its approved mandate
- **AND** only an authorized human policy write can change the applicable ceiling

### Requirement: Recurring constraints narrow across all hosting placements
Recurring-worker policy SHALL combine deployment defaults, every applicable durable hosting cluster, and the project override through monotone narrowing. Enablement SHALL require all applicable levels, allowlists SHALL intersect, maximum allowances/concurrency SHALL take the minimum, and minimum inspection intervals SHALL take the maximum. Lower-level widening SHALL be rejected. Omitted lower-level values SHALL inherit; malformed explicit values SHALL deny. Mandate approval SHALL record the resolved policy and source revisions and clearly show automatic rolling-budget renewal.

#### Scenario: Another hosting cluster requires a slower cadence
- **WHEN** the project's applicable policies set different minimum inspection intervals
- **THEN** the effective worker interval respects the longest minimum
- **AND** choosing another machine or repeatedly using Check now cannot bypass it

#### Scenario: A person reviews the recurring budget
- **WHEN** approving a worker with a rolling-24-hour allowance
- **THEN** the review explains that eligible work can spend that allowance again as earlier usage leaves the window until the worker is paused or stopped
- **AND** it does not describe that value as a one-time total or exact invoice cap

### Requirement: Every automatic dispatch reauthorizes the standing grant
Approval, automatic inspection, implementation selection, dispatch, lease renewal, retry, and publication SHALL recheck current authority and effective policy. Loss of a required profile, permission, or narrowed consumption allowance SHALL fence recurrence and durably stop affected execution. Concurrency reductions SHALL block additional dispatch rather than arbitrarily terminating otherwise permitted work. Enforcement SHALL cover idle and paused workers and all pages of affected projects. Restoration of a grant SHALL not erase the recorded stop or implicitly resume the worker.

#### Scenario: Policy is disabled while no run is active
- **WHEN** a worker is idle and later becomes due
- **THEN** its retained mandate cannot bypass the current denial
- **AND** the worker displays the policy reason instead of launching and immediately cancelling a process

#### Scenario: A policy change spans more workers than a listing page
- **WHEN** affected workers include later pages and disconnected hosts
- **THEN** all are fenced or reconciled according to the same policy transition
- **AND** presentation pagination does not limit enforcement

### Requirement: Permission and operational readiness remain separate
Policy permission SHALL NOT imply that an execution target is provisioned or compatible. Administration and approval surfaces SHALL show safe readiness and inherited-policy reasons separately. Audits SHALL retain actor, project/cluster, worker, mandate revision, and execution identities for authority-changing operations without raw descriptions, questions, answers, credentials, or transcripts.

#### Scenario: The recurring policy permits an unqualified image
- **WHEN** its target cannot prove the required inspection, result, isolation, metering, or complete-stop behavior
- **THEN** activation or dispatch is blocked with a readiness reason
- **AND** no broader provider or unrestricted host execution is substituted
