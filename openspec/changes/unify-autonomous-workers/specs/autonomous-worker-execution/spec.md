## Purpose

Define how a persistent worker performs bounded inspections and selected implementation work without weakening isolation, usage accounting, structured questions, independent verification, or recovery guarantees.

## ADDED Requirements

### Requirement: Persistent workers use qualified bounded execution
Each inspection or implementation SHALL use a separately identified bounded execution under the approved OMP profile, exact target, current authority, finite allowance, and trusted local execution lease. Native OMP tools, asynchronous work, and subagents SHALL remain execution mechanisms rather than owners of APAS scheduling. Provider-native loops, transcript inactivity, terminal readiness, or a model's promise to continue SHALL NOT establish durable recurring-worker readiness. Missing required runtime behavior SHALL fail closed without a terminal or unrestricted fallback.

#### Scenario: An OMP terminal is available but the worker protocol is not
- **WHEN** a target lacks the qualified inspection/result, settlement, budget, or stop contract
- **THEN** worker activation or dispatch is unavailable with an actionable reason
- **AND** the existing terminal provider remains separate and unchanged

### Requirement: Inspection can discover work without creating a code change
Inspection SHALL receive the approved mandate, standing checklist, bounded current backlog/checkpoint, and a committed snapshot of the approved repository. Its project snapshot SHALL be read-only, and it SHALL have no publication authority. It SHALL produce a bounded structured result containing proposed backlog changes, selection or no-work disposition, and checkpoint evidence. That result SHALL be treated as untrusted proposed state, not a command to execute arbitrary code, modify policy, or publish a PR.

#### Scenario: Inspection selects the next item
- **WHEN** inspection settles, its execution is positively stopped, and its result matches the admitted worker/run and current revisions
- **THEN** APAS can atomically commit the accepted backlog changes and selected work
- **AND** a separate immutable implementation execution is admitted within the continuing grant

#### Scenario: Inspection reports no work
- **WHEN** its valid settled result contains no eligible item
- **THEN** APAS records the checkpoint and schedules the next allowed inspection
- **AND** no candidate verification, PR, or continuously running provider is required

### Requirement: Backlog and checkpoint updates are validated and fenced
Updates SHALL be bounded structured data bound to worker, mandate, execution, schedule, and backlog revisions. APAS SHALL reject malformed, oversized, unsafe, mismatched, or stale results without partially applying them. A repeated accepted result SHALL be idempotent. The model SHALL NOT mark independently controlled execution outcomes, answered questions, budget consumption, or external review disposition as confirmed. Missing structured output SHALL be an explicit failure, never inferred from chat prose or a successful process exit.

#### Scenario: The model claims a task and PR are complete
- **WHEN** independent verification or publication has not confirmed that outcome
- **THEN** its checkpoint cannot mark the task delivered or merged
- **AND** progress remains distinguishable from trusted execution and delivery evidence

#### Scenario: A late checkpoint follows human editing
- **WHEN** the backlog or mandate revision no longer matches the inspection input
- **THEN** APAS rejects the stale mutation set as a whole
- **AND** records the spent allowance while retaining the newer human-approved state

### Requirement: Settlement and stop evidence precede continuation
A root response, tool result, process exit, or disconnected transport SHALL NOT by itself authorize the next execution. APAS SHALL require qualified settlement, resolved question state, no active delegated work, and positive evidence that the prior execution cannot continue mutating its workspace. Uncertain prompt admission or ownership SHALL become Recovery required. Duplicate commands and controller replacement SHALL reconcile the original execution without resubmitting its prompt.

#### Scenario: Root yields while a delegated task continues
- **WHEN** the root response finishes but a child remains active
- **THEN** the worker remains executing that run
- **AND** neither inspection-result commitment nor a subsequent implementation is allowed

#### Scenario: The machine loses its server connection
- **WHEN** the bounded local lease expires
- **THEN** the trusted host terminates the complete execution unit and retains stop evidence
- **AND** APAS does not start replacement work until that ownership is accounted for

### Requirement: Allowances cover all runs and delegated work
An approved worker SHALL have finite per-execution time, reported-spend, resource, and output ceilings; finite rolling-24-hour aggregate elapsed-time, reported-spend, and execution-count ceilings; and finite per-item attempt limits. Inspection, implementation, retries, and delegated model use SHALL consume the same worker accounting. Automatic rolling-window renewal SHALL be explicitly disclosed and approved. Accounting SHALL survive restarts, pause/resume, mandate edits, and duplicate reports; none SHALL mint a fresh allowance for already consumed work. Spend SHALL retain unknown/estimated/overrun semantics rather than claim an exact provider-invoice cap.

#### Scenario: Many small runs exhaust the recurring budget
- **WHEN** their aggregate reaches an approved rolling-window ceiling even though each run was individually within its limit
- **THEN** further dispatch waits for the displayed eligible budget time or an explicitly approved policy-permitted change
- **AND** the system does not bypass the ceiling by starting another item or inspection

#### Scenario: The worker is paused and resumed
- **WHEN** its previous consumption still lies within the rolling window
- **THEN** that consumption remains charged
- **AND** resume does not reset work-item attempts or an active execution's approved limits

### Requirement: Budget reservations cannot be reused while execution is uncertain
Before dispatch, APAS SHALL reserve the complete admitted execution allowance against the worker's remaining recurring allowance and applicable capacity. Reservations SHALL be released or reconciled only from trusted usage and positive termination evidence. An unresolved reservation SHALL remain unavailable even when a budget window advances. Unknown required metering SHALL block further execution rather than count as zero; late and duplicate usage SHALL reconcile without subtracting real consumption or charging the same report twice.

#### Scenario: A stopped host returns after the budget window changed
- **WHEN** its old execution's consumption or stop was previously unconfirmed
- **THEN** the unresolved reservation prevented replacement spending in the meantime
- **AND** the returned evidence reconciles the same reservation and execution identity

### Requirement: Human questions retain exact execution identity
Questions SHALL retain worker, work item when applicable, execution, epoch, dialog, and question identities. The first valid currently authorized answer SHALL win, and only matching provider-confirmed evidence SHALL resolve it. No recommended option, elapsed timeout, offline client, or checkpoint summary SHALL count as a human answer. Waiting for a person SHALL remain subject to the run's finite elapsed and resource bounds. An expired or stopped dialog SHALL NOT be answered in another run.

#### Scenario: A question outlives its execution deadline
- **WHEN** the host stops the execution before an answer was confirmed
- **THEN** the question is shown as no longer answerable and the worker retains a concrete blocked outcome
- **AND** a later continuation cannot replay the old answer into a new dialog

### Requirement: Implementation retains independent draft-only delivery
Implementation SHALL operate on one immutable selected item and exact base in a private bounded checkout. After settlement APAS SHALL freeze the complete safe candidate, verify it independently in the approved restricted environment, and publish only that exact passing candidate through the separate trusted draft-only publisher. Workers and verifiers SHALL have no publication credentials or automatic merge/approval authority. Candidate, verification, publication, and external PR disposition SHALL remain separate evidence. A claimed no-change implementation SHALL require human disposition rather than silently completing an item or fabricating a PR.

#### Scenario: The worker's tests pass but independent verification fails
- **WHEN** a required independent check fails on the frozen candidate
- **THEN** no verified draft delivery is reported
- **AND** the worker retains the failed item and evidence in an actionable blocked state

#### Scenario: Publication response is uncertain
- **WHEN** the exact candidate may already have been pushed or a draft opened
- **THEN** APAS reconciles that publication identity without another coding run or duplicate PR
- **AND** the worker does not continue as if delivery or cancellation were confirmed

### Requirement: Review gates continuation without completing the worker
The initial worker contract SHALL permit at most one unresolved delivered draft per worker. While that draft awaits review, APAS SHALL expose Waiting for review and SHALL NOT start another implementation or periodically call the model merely to inspect unchanged review state. An authorized refresh confirming merge SHALL make a fresh inspection of the approved base branch eligible. Closing without merge, a changed external head, or an unverifiable result SHALL retain the item and require explicit human disposition. APAS SHALL NOT automatically merge, stack dependent branches, or treat delivery as integration.

#### Scenario: A draft is waiting for a reviewer
- **WHEN** its implementation run is finished but the PR remains open
- **THEN** the worker remains active in responsibility but waiting without a provider process
- **AND** its next action and review link are visible

#### Scenario: A human merges the draft
- **WHEN** APAS confirms the external merge and current authority, budget, and approved base remain valid
- **THEN** the worker can schedule a fresh inspection without the person launching another task
- **AND** the historical candidate and checks are not relabeled as checks on the new branch tip

### Requirement: Failure is not an unbounded repair loop
Within a bounded implementation the agent SHALL be able to investigate and repair its work using its ordinary tools. After a terminal execution, verification, or checkpoint failure, the worker SHALL become blocked rather than automatically starting a fresh model attempt, inventing an equivalent item to reset attempts, or selecting a different model. A human-authorized retry SHALL require positive prior stop evidence, current authority and policy, and remaining worker and work-item allowances. Publication recovery SHALL remain separate from coding retry.

#### Scenario: A user retries a failed item
- **WHEN** the old execution is positively stopped and all applicable remaining allowances permit the explicit retry
- **THEN** the new attempt has its own identity while retaining the item's history and cumulative attempt accounting
- **AND** a transport reconnect alone cannot cause the same retry
