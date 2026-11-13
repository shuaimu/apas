## Purpose

Provide one browser and native-mobile workflow for approving, observing, steering, and reviewing persistent autonomous workers without conflating their identity, status, or authority with interactive panes or individual execution results.

## ADDED Requirements

### Requirement: Users create a responsibility rather than a single task
Desktop web, mobile browser, and native mobile SHALL provide one Autonomous worker creation flow. It SHALL collect the job description, human-owned standing checklist, eligible exact host and profiles, repository/base branch, cadence, per-run ceilings, and recurring allowance. Review SHALL explain autonomous discovery, future committed branch-tip selection, automatic budget renewal, draft-review waiting, and pause/stop behavior. Draft editing SHALL not start work; confirmed approval SHALL open the exact worker only after a durable receipt. Unavailable authority, policy, or runtime readiness SHALL have distinct actionable reasons.

#### Scenario: A person creates a worker from a project
- **WHEN** the person approves the reviewed responsibility and recurring allowance
- **THEN** the client opens the acknowledged worker inside that same project on web or its exact native worker screen
- **AND** no interactive pane or one-shot task is silently created

#### Scenario: A saved target is no longer eligible
- **WHEN** a person restores a draft whose selected host or profile is unavailable
- **THEN** the draft retains that choice and explains the problem
- **AND** no different machine, model, or provider is silently selected

### Requirement: Oversight separates responsibility, current execution, and outcomes
Worker lists and detail SHALL expose the full approved job description, standing checklist, current work, canonical status/reason, last confirmed update, exact host, next eligible check, recurring and per-run usage, and allowed controls. The backlog and execution history SHALL be independently paginated. Working, Idle, Waiting for answer, Waiting for review, Waiting for budget, Pausing, Paused, Stopping, Blocked, and Recovery required SHALL remain distinguishable. Checklist progress SHALL be scoped to its inspection or work item rather than presented as permanent completion of the responsibility.

#### Scenario: A worker finishes an inspection with no task
- **WHEN** the confirmed state is idle
- **THEN** the view shows the next check and last inspected evidence
- **AND** does not show a finished worker or manufacture a task/progress percentage

#### Scenario: A long description appears in a crowded list
- **WHEN** the full description exceeds the compact row's display space
- **THEN** the user can expand it in place and open the full standing checklist
- **AND** the client does not substitute the shortened title or fetch every worker's detail just to populate descriptions

### Requirement: Backlog edits and lifecycle controls are explicit
Authorized users SHALL be able to inspect discovery rationale, add/edit/reprioritize/dismiss unclaimed items, review current immutable work, and explicitly request Pause after this run, Stop now, Resume, or Check now. Labels SHALL explain their different effects. Controls SHALL follow current server-provided allowed actions and exact synchronization state; requests SHALL expose durable acceptance separately from actual stopping or execution. Hidden views SHALL not change worker lifecycle.

#### Scenario: A person hides a running worker tab
- **WHEN** the view is closed or hidden
- **THEN** only the scoped layout preference changes
- **AND** the worker remains inspectable through Overview without a pause, cancel, or pane command

#### Scenario: A person edits an already claimed item
- **WHEN** the client submits a stale or ineligible item edit
- **THEN** it shows the conflict and refreshes the current item
- **AND** it does not claim the running execution received the new instruction

### Requirement: Global Workers and Attention remain bounded cross-project views
Both clients SHALL expose accessible workers and attention destinations across projects, with status filtering and explicit bounded pagination. Worker lists SHALL retain idle and paused responsibilities rather than hiding them as completed tasks. Attention SHALL identify the exact worker and relevant question, review, blocked item, or recovery reason. Counts derived from a loaded subset SHALL be labeled as such unless the server supplies a separately authoritative total.

#### Scenario: A project has workers beyond the first global page
- **WHEN** the user opens its project-specific worker view
- **THEN** that view uses project-authorized inventory filtered before pagination
- **AND** it does not infer that the project has no workers from the global page

### Requirement: Questions and results navigate to exact historical execution
Browser and native destinations SHALL retain project, worker, execution, and question/review identities. Opening a result or notification for an older execution SHALL display that execution, not the worker's latest run. Questions SHALL preserve first-writer and provider-confirmation semantics. Candidate diffs, checks, logs, PR disposition, and expired/truncated evidence SHALL remain independently inspectable without terminal attachment, approval, merge, or implicit mutation.

#### Scenario: An old review link is opened after several later inspections
- **WHEN** the linked execution is still retained and accessible
- **THEN** the original candidate, base, checks, and PR are displayed
- **AND** the client does not silently substitute the latest execution's review

#### Scenario: An old question link is opened after its run stopped
- **WHEN** the dialog no longer accepts answers
- **THEN** the exact historical question is shown read-only with its disposition
- **AND** no answer draft is redirected into another run

### Requirement: Offline recovery preserves identity and rejects stale authority
Both clients SHALL persist drafts, mutation request identities, accepted snapshots/events, and watermarks under account/project/worker and exact item/run/question scopes. A connected socket alone SHALL not enable mutation before authorized catch-up. Offline caches SHALL be visibly stale and read-only. Uncertain mutations SHALL first resolve the original receipt; clients SHALL NOT automatically replay posts. Discovered revocation/logout SHALL purge protected worker data, histories, labels, and downloaded evidence and fence stale asynchronous writes. Offline devices SHALL not claim to know undiscovered remote revocations.

#### Scenario: The phone reconnects after an uncertain worker approval
- **WHEN** the original receipt is recovered
- **THEN** the client opens the same worker and retires the pending draft safely
- **AND** it does not create another worker or recurring grant

#### Scenario: A late response arrives after project access is revoked
- **WHEN** the response predates the local revocation barrier
- **THEN** it cannot repopulate the visible or persisted worker cache
- **AND** later pages, hidden labels, and earlier results remain inaccessible

### Requirement: Notifications reflect decisions rather than routine loop activity
Notifications SHALL use the existing preference, outbox, deduplication, and current-recipient authorization contracts. Questions, actionable failures/recovery, and review-ready results SHALL carry exact worker/execution destinations using generic notification text without descriptions, source paths, answers, or code. Routine idle checks and recurring wakeups SHALL not generate completion spam. Notification taps SHALL authorize and navigate only; they SHALL not resume, answer, retry, approve, or merge work. Browser attention SHALL not imply browser push support.

#### Scenario: A worker repeatedly finds no work
- **WHEN** several valid idle checkpoints are recorded
- **THEN** the next-check state updates in the worker view
- **AND** the user does not receive a completed-task notification for each inspection

### Requirement: The worker flow replaces legacy creation paths
All supported clients SHALL remove Start bot and new standalone-job creation after cutover and expose the worker flow instead. Earlier result links SHALL remain read-only. Reusable finite-job instructions and saved standalone-job drafts MAY open an unapproved conversion draft with explicit provider and recurring-budget review; bot records SHALL remain read-only without a conversion action. Old cached drafts SHALL not become approved mandates. Unsupported clients SHALL receive an upgrade-required refusal for retired unattended mutations rather than a differently interpreted operation.

#### Scenario: A client sends an old Start bot request
- **WHEN** the deployment has completed the worker cutover
- **THEN** the request is rejected without launching or converting a pane
- **AND** the current client directs the person to independent worker creation

#### Scenario: A legacy job draft survives in local storage
- **WHEN** a newer client encounters it
- **THEN** it can offer its text as an unapproved worker draft
- **AND** it requires new recurring settings and review before any execution

### Requirement: Worker controls remain usable on real mobile surfaces
The separate mobile-browser project screen and native application SHALL implement the full worker flow, not only a narrowed desktop tab bar. Ordinary content SHALL fit a 320-CSS-pixel browser width without horizontal page scrolling; controls SHALL retain at least 44-pixel web and 48-dp native targets, accessible labels, large-text support, browser zoom, safe areas, and usable keyboard behavior. Code/diff regions SHALL remain explicitly bounded scrolling surfaces. Real mobile-browser and native device/emulator interaction SHALL be qualified separately from desktop emulation or type checks.

#### Scenario: A person edits a checklist with the phone keyboard open
- **WHEN** the editor is focused at supported narrow width or large text size
- **THEN** the active field and confirmation/error controls remain reachable above the keyboard and safe area
- **AND** the person can complete or cancel editing without terminal interaction
