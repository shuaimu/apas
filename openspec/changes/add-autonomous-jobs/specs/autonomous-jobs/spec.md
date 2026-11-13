## Purpose

Define the internal bounded job engine and retained finite-job evidence, with immutable identity, current authorization and honest recovery semantics independent of interactive panes or a connected client. After persistent-worker cutover, the `autonomous-workers` capability owns public approval, recurrence and controls; this capability does not expose a second standalone creation or retry product.

## ADDED Requirements

### Requirement: Admission binds one immutable task and finite allowance
Every admitted job SHALL bind immutable purpose, instruction, acceptance criteria, project, canonical repository, exact base commit, execution target, worker and verification profiles, and finite cumulative limits before dispatch. After worker cutover, only an atomic worker execution claim under a currently approved standing mandate and recurring allowance SHALL create a new job. Its immutable context SHALL pin worker, mandate/backlog/item revisions, execution purpose and reservation identity. Historical finite approvals SHALL remain unchanged and SHALL NOT grant recurrence. Local drafts SHALL NOT start work; retired standalone admission and retry requests SHALL receive an upgrade-required refusal.

#### Scenario: An approved worker selects eligible work
- **WHEN** the coordinator atomically claims an inspection or eligible implementation under current authority and remaining allowance
- **THEN** it records the exact bounded input, policy digest, target, reservation and job/run identities before dispatch
- **AND** it creates no interactive pane and requests no redundant per-item approval inside the approved mandate

#### Scenario: A selected input is stale
- **WHEN** the selected item's revision, inspection context, approved profile or effective policy no longer permits that claim
- **THEN** the old selection cannot dispatch
- **AND** the worker requires fresh inspection or reviewed mandate approval according to the changed boundary

### Requirement: Job access does not widen project or compute grants
Job content SHALL require an active account and current project-content authority. Admission and explicit retry SHALL additionally require the project owner or selected hosting operator, exact-machine launch authority, active project lifecycle and effective job policy. Answering a question SHALL require current runtime authority and the job initiator, project owner or selected hosting operator. Cancellation SHALL permit those same actors through a stop-specific authority check even while a project is suspended or deleting. System-administration credentials SHALL remain separate from ordinary job credentials; authorized administration stop operations SHALL still stop jobs.

#### Scenario: A project share is not a compute grant
- **WHEN** a project member without the required admission authority can read a project's jobs
- **THEN** the member can inspect authorized history and evidence
- **AND** cannot start, retry or answer a job merely because an owner-hosted interactive session is accessible

#### Scenario: Owner uses third-party compute
- **WHEN** the project owner attempts admission on another account's machine
- **THEN** current hosting-cluster membership, exact-machine permission and durable project placement are required in addition to ownership
- **AND** an empty, malformed or unverifiable machine grant denies admission

#### Scenario: Content authority is revoked
- **WHEN** an account loses project-content access or its device session is revoked
- **THEN** affected subscriptions, replay, receipts and artifact reads cease to reveal protected job content
- **AND** clients remove that scope from visible and persisted caches after receiving the revocation or discovering it during reauthentication

### Requirement: Mutations have durable actor-scoped receipts
Admission, answer, cancellation and retry SHALL accept a persisted request identifier bound to the actor, operation and normalized inputs. Repeating the same request SHALL return its original result or pending status without repeating its side effect. Reusing an identifier with different inputs SHALL conflict. Receipt retrieval SHALL be reauthorized and SHALL NOT depend on the original phone connection or device remaining active.

#### Scenario: Admission response is lost after commit
- **WHEN** the client reconnects and resolves or resends the same request identifier and payload
- **THEN** it receives the original job/run receipt
- **AND** no second job or worker is created

#### Scenario: A request identifier is reused incorrectly
- **WHEN** a caller changes the task, target or operation while reusing an existing request identifier
- **THEN** the system returns a conflict without altering the original operation
- **AND** another account cannot retrieve that actor's receipt using the identifier

### Requirement: Execution is queued and fenced durably
The system SHALL reserve capacity and record dispatch intent durably before contacting a worker. It SHALL permit only one nonterminal run per job, enforce configured project, cluster and machine concurrency, and authenticate every worker result against its project, run, hosting owner, machine and current execution epoch. Duplicate command delivery SHALL reconcile the existing attempt rather than submit another prompt. Unavailable targets SHALL remain visibly queued or blocked without silently migrating to another machine.

#### Scenario: Concurrent requests contend for one slot
- **WHEN** several admissions or dispatchers compete for the last permitted execution slot
- **THEN** at most the permitted number of runs execute
- **AND** remaining jobs retain explicit queue positions or capacity reasons without hidden workers

#### Scenario: An acknowledgement is lost
- **WHEN** a dispatch acknowledgement is lost after the worker reserved or started the attempt
- **THEN** retransmission identifies and reconciles that same attempt
- **AND** ambiguous prompt admission becomes recovery-required rather than an automatic second prompt

#### Scenario: An obsolete worker reports success
- **WHEN** a superseded connection, different machine or stale execution epoch reports progress or completion
- **THEN** the report cannot advance the current run or authorize publication

### Requirement: Job lifecycle distinguishes evidence and uncertainty
Jobs SHALL expose queued, preparing, running, needs-input, verifying, publishing, review-ready, completed, blocked, recovery-required, cancelling, cancelled and failed states with explicit reasons and allowed actions. Review-ready SHALL mean verified draft delivery, never merged code. Completed SHALL be a validated successful inspection with positive stop evidence and SHALL NOT imply verification or publication. A worker yield, process exit, socket loss or absence of progress SHALL NOT establish successful completion. Worker-item retry SHALL be explicit, require positive prior stop and current authority, and retain item/rolling allowances; it SHALL NOT reopen standalone job retry or automatically repair terminal failures.

#### Scenario: Work reaches review
- **WHEN** the immutable candidate passes independent verification and its draft PR is reconciled
- **THEN** the job becomes review-ready with exact candidate and PR evidence
- **AND** its GitHub open, closed or merged disposition remains separate from execution success

#### Scenario: The worker disappears
- **WHEN** execution state cannot be established after reconnect or restart
- **THEN** the job becomes recovery-required with retained evidence
- **AND** neither automatic rerun nor a replacement owner is allowed while old execution may still exist

#### Scenario: User explicitly retries a stopped worker item
- **WHEN** current worker authority and policy permit retry and the old execution is confirmed stopped
- **THEN** the next immutable execution receives new job/run identities and consumes the same item's remaining allowance and attempts
- **AND** connection recovery alone never creates that execution

### Requirement: Cancellation and lifecycle cleanup require stop evidence
Cancellation SHALL durably fence further dispatch, answer delivery and new publication before requesting runtime termination. Cancelling SHALL remain visible until all worker, delegated, verifier and publisher activity is reconciled. Project stop, suspension and deletion SHALL cover durable job inventory even when no interactive project process or machine connection is present. Deletion SHALL retain cleanup records until termination and artifact cleanup are positively established.

#### Scenario: A disconnected host receives a cancellation request
- **WHEN** the server cannot immediately contact the recorded execution host
- **THEN** the request remains durable and the job remains cancelling or recovery-required
- **AND** a missing heartbeat or expired server timer alone does not report cancellation complete

#### Scenario: Publication was already in flight
- **WHEN** cancellation races an external draft-PR creation request already sent
- **THEN** the system reconciles whether the PR exists and retains its link as a partial result
- **AND** does not claim the request was undone, automatically delete the PR or rerun the worker

#### Scenario: A project is deleted after a server restart
- **WHEN** a job's recorded host is absent from the new server's live inventory
- **THEN** cleanup still consults the durable job/run ownership records
- **AND** keeps deletion pending until the old runtime and retained artifacts are accounted for

### Requirement: Authority changes stop execution without tying jobs to devices
Runtime-authority loss by the admitting actor, project suspension, or effective policy denial SHALL durably stop affected work and prevent further publication. Ordinary browser disconnect, app backgrounding, token refresh, device logout or device revocation SHALL NOT by themselves cancel an admitted account-owned job. Job-specific policy and access enforcement SHALL cover every affected run, not a bounded first page of projects.

#### Scenario: Phone goes offline
- **WHEN** the admitting user's phone loses connectivity while account and runtime authority remain valid
- **THEN** the bounded job continues independently of that device
- **AND** later authorized clients can recover its receipt and state

#### Scenario: Hosting permission is revoked
- **WHEN** the admitting actor loses the selected host-machine permission
- **THEN** APAS records cancellation, stops renewing execution authority and requests runtime termination
- **AND** any remaining project-content grant continues to permit history reads without runtime control

### Requirement: Job history supports authorized ordered catch-up
Job state changes and bounded typed events SHALL have durable monotonically increasing per-job sequence numbers and a consistent snapshot watermark. Clients SHALL recover dropped, duplicated or out-of-order live notifications through authorized replay. Large logs and diffs SHALL be paginated or artifact-backed. Job history SHALL remain available after worker/session exit, within the published retention policy.

#### Scenario: Live output is dropped or reordered
- **WHEN** a client detects a sequence gap or returns from suspension
- **THEN** it obtains a current authorized snapshot and ordered missing events
- **AND** advances its durable cursor only with the corresponding accepted local data

#### Scenario: A retained cursor is no longer available
- **WHEN** a client requests events older than retained replay data
- **THEN** the server explicitly requires snapshot resynchronization
- **AND** the client does not silently treat the gap as a complete history
