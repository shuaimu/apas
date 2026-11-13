## Purpose

Define persistent project workers whose approved responsibilities outlive individual tasks, while keeping autonomous discovery, durable continuation, human control, and execution authority explicit.

## ADDED Requirements

### Requirement: A worker owns an ongoing approved responsibility
A worker SHALL have stable project-scoped identity, a job description, an ordered standing checklist, a versioned approved mandate, a concrete backlog, and retained execution history. Approval SHALL authorize recurring work within the displayed repository/base-branch, exact hosting target, profiles, permissions, cadence, and finite allowances. A work item completing, an execution ending, or a draft PR being delivered SHALL NOT complete or delete the worker. Worker identity SHALL NOT be a project session, pane, process, or OMP conversation identifier.

#### Scenario: A task finishes while the responsibility remains active
- **WHEN** an active worker finishes a work item with an independently established outcome
- **THEN** the worker retains its identity, mandate, checklist, remaining backlog, and cumulative accounting
- **AND** it evaluates the next eligible work or enters a visible waiting state without another task-creation confirmation

#### Scenario: A worker exists without interactive panes
- **WHEN** its project has no pane or attached interactive session
- **THEN** authorized users can still inspect and control the worker
- **AND** its execution eligibility is determined by its own mandate and exact host, not a pane roster

### Requirement: Standing instructions and concrete backlog have different owners
Only an authorized human SHALL approve or replace the job description, standing checklist, or execution envelope. The worker SHALL be able to discover, add, refine, and prioritize concrete work items within that mandate. Each item SHALL retain stable identity, origin, scope rationale, acceptance criteria, dependencies, priority, status, and outcome references. Authorized humans SHALL be able to add, edit, reprioritize, or dismiss unclaimed items. Model output SHALL NOT grant authority or count as proof that a task is in scope or complete.

#### Scenario: Inspection finds an in-scope improvement
- **WHEN** the worker reports a concrete finding and proposed work within its assigned responsibility
- **THEN** the proposal can become a persisted backlog item without another individual admission approval
- **AND** users can inspect why it was selected, its criteria, and later results

#### Scenario: The model requests broader permissions
- **WHEN** a worker attempts to change its description, checklist, budget, repository, execution target, profile, or permissions
- **THEN** the change is rejected as outside worker authority
- **AND** it cannot continue under the proposed broader envelope without human approval

#### Scenario: A human dismisses an item during inspection
- **WHEN** a stale inspection result tries to recreate or reprioritize that item using an older backlog version
- **THEN** the stale update is rejected without undoing the human decision
- **AND** the worker must reconcile the current backlog before further selection

### Requirement: Each execution receives immutable selected work
Before an implementation execution starts, APAS SHALL bind a selected item revision, acceptance criteria, approved mandate revision, current policy, exact target/profiles, and a committed base SHA resolved from the approved repository/base branch. Future branch-tip changes within that approved branch SHALL be disclosed at mandate approval and SHALL NOT require per-task human approval. An active execution's task and base SHALL remain immutable; later backlog changes SHALL NOT silently rewrite it.

#### Scenario: The approved branch advances between tasks
- **WHEN** a new eligible task starts after the branch has advanced
- **THEN** APAS records the new task's exact committed base before dispatch
- **AND** previous candidates and evidence remain tied to their original bases

#### Scenario: The repository or branch cannot be resolved
- **WHEN** the approved repository identity or branch is missing or unverifiable
- **THEN** the worker becomes blocked with a concrete reason
- **AND** it does not substitute another repository, branch, dirty working tree, or hosting target

### Requirement: Mandate edits form an explicit authority boundary
Mandate edits SHALL use an explicit reviewed confirmation and optimistic version check. New work SHALL be fenced during replacement approval. A replacement mandate SHALL become active only after prior execution is positively stopped or finished; older checkpoints and scheduled wakeups SHALL NOT authorize work under it. Backlog entries SHALL remain visible but require reassessment against the replacement mandate. Authority or permission revocation SHALL take the immediate stop path rather than waiting for ordinary editing.

#### Scenario: A person narrows the worker's responsibility
- **WHEN** the approved description or checklist changes
- **THEN** the system retains the prior revision and its historical outcomes
- **AND** does not automatically continue previously selected work under the changed mandate

### Requirement: Scheduling and checkpoints survive client and controller lifetimes
APAS SHALL durably retain desired worker state, next check time, schedule generation, backlog revisions, current execution identity, and the last accepted checkpoint. Due work SHALL be claimed atomically with its execution intent before dispatch. At most one unresolved execution chain SHALL belong to a worker. Restart, duplicate delivery, browser disconnect, or repeated wake requests SHALL NOT create overlapping execution or replay an uncertain prompt. Missed periodic checks SHALL coalesce into one eligible inspection rather than an accumulated catch-up burst.

#### Scenario: Two schedulers observe the same due worker
- **WHEN** both attempt to dispatch its next inspection
- **THEN** one durable execution identity is claimed
- **AND** duplicate work reconciles that identity instead of submitting a second instruction

#### Scenario: The server restarts after a completed checkpoint
- **WHEN** scheduling resumes from persisted state
- **THEN** the worker continues from that checkpoint and current backlog under current authority
- **AND** a retained process or uncertain prior execution is reconciled before replacement work

#### Scenario: A worker misses several scheduled checks
- **WHEN** its host returns after the worker was due multiple times
- **THEN** it has at most one current due inspection
- **AND** missed ticks do not create one model run per missed interval

### Requirement: No-work inspection is an explicit idle outcome
A settled, confirmed inspection with no eligible work SHALL record a checkpoint and a next check time and release execution resources. The worker SHALL display Idle with that next check, not Completed or Working. It SHALL NOT invent work, publish an empty change, or continuously run the model solely to remain active. Manual Check now SHALL be idempotent, respect policy and budget/rate limits, and leave an existing active execution unduplicated.

#### Scenario: The checklist finds nothing actionable
- **WHEN** the inspection settles with a valid no-work result
- **THEN** no implementation execution or draft PR is created
- **AND** the worker waits without a provider process until its next eligible check

### Requirement: Pause and immediate stop have distinct observable semantics
Pause after this run SHALL fence all future dispatch immediately, allow the already admitted bounded execution and its authorized delivery to settle, then display Paused. When no execution is active, pausing SHALL take effect immediately. Stop now SHALL additionally fence further publication and request complete runtime termination; it SHALL remain Stopping until positive stop evidence arrives. Neither operation SHALL erase the backlog, history, or cumulative consumption. Resume SHALL be an explicit currently authorized action and SHALL NOT blindly retry a failed, stopped, or uncertain task.

#### Scenario: A pause races completion
- **WHEN** a person pauses as a task finishes
- **THEN** the outcome remains recorded but no next inspection or task starts
- **AND** the worker becomes Paused after its current execution chain settles

#### Scenario: The execution host is offline during stop
- **WHEN** Stop now cannot reach the recorded host
- **THEN** the durable stop fence remains in force and the UI retains an unresolved stopping or recovery state
- **AND** the worker is not reported safely stopped merely because its heartbeat expired

### Requirement: Worker authority remains account-owned and continuously checked
Worker reads SHALL require current project-content authority. Creating, approving, resuming, or waking a worker SHALL require the existing unattended-admission authority for the exact hosting machine, not merely project membership or interactive-session access. Runtime-authority loss by the approving account SHALL disable recurrence and stop active execution. Restoration of a revoked grant SHALL NOT automatically reactivate it. Browser/app disconnect, ordinary device logout, or device revocation alone SHALL NOT cancel otherwise authorized account-owned work.

#### Scenario: A hosting grant is revoked while the worker is idle
- **WHEN** the approving account loses permission for the selected machine
- **THEN** the worker is fenced even though it has no current run
- **AND** it cannot wake after the grant is restored without explicit renewed approval

#### Scenario: The phone disconnects
- **WHEN** the approving account and its execution authority remain valid
- **THEN** the worker's responsibility and bounded scheduling continue independently of the device
- **AND** a later authorized client can recover its confirmed state

### Requirement: Worker mutations and event history are durable and ordered
Approve, edit, pause, stop, resume, wake, backlog, answer, and retry operations SHALL use actor-scoped request identities and expected versions. Same-identity same-payload replay SHALL recover the original result; changed payload SHALL conflict. Worker snapshots and typed events SHALL have ordered durable versions and watermarks, with bounded replay and explicit resynchronization after retention gaps. Current mandate, backlog, checkpoints, unresolved ownership, and accounting SHALL NOT depend on indefinitely retaining every transcript or event.

#### Scenario: A resume acknowledgement is lost
- **WHEN** the client resolves the original request after reconnect
- **THEN** it receives the original receipt or pending state
- **AND** resolving the receipt does not create another run or reset the next check

### Requirement: Project lifecycle includes idle and paused workers
Project stop, suspension, deletion, account revocation, and applicable policy changes SHALL enumerate all affected workers and execution history rather than only active runs or a listing page. Project stop SHALL disable recurrence until explicit resume; clearing a transient runtime fence or starting an interactive project SHALL NOT resurrect it. Deletion SHALL preserve cleanup ownership until every started execution is positively stopped and required host artifacts are purged.

#### Scenario: Project deletion finds only paused workers
- **WHEN** those workers retain historical execution artifacts on a disconnected machine
- **THEN** deletion includes those artifacts and remains pending until cleanup is confirmed
- **AND** absence of a live provider does not discard the required purge record

### Requirement: Existing finite approvals never become perpetual authority
Historical jobs SHALL retain their task, runs, questions, artifacts, results, and original identities as read-only earlier results after cutover. Reusing a finite job SHALL open an unapproved draft and require fresh mandate, provider, and recurring-budget approval. Existing bot prompts, providers, cadence, and history SHALL remain read-only; APAS SHALL NOT offer bot-to-worker conversion or claim that historical metadata proves the old runtime stopped. Users MAY create and approve fresh workers independently. Old bot restore or creation paths SHALL NOT start another loop in the new runtime, and no live terminal SHALL be adopted as a worker.

#### Scenario: A previously completed job is opened after upgrade
- **WHEN** a user follows its retained history link
- **THEN** the original evidence is displayed without creating a worker or scheduling work
- **AND** choosing to reuse its instructions opens a new unapproved worker draft

#### Scenario: A historical bot is opened after upgrade
- **WHEN** a user views its retained prompt and history
- **THEN** no conversion action, provider switch, or autonomous launch occurs
- **AND** an independently created worker requires its own mandate and recurring-budget approval
