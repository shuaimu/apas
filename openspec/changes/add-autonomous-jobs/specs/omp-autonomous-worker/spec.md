## Purpose

Define the externally observable contract for unattended OMP workers whose isolation, execution ownership, questions and limits are controlled by APAS rather than inferred from terminal text or model promises.

Worker-originated execution additionally follows `autonomous-worker-execution`:
protocol v2 and the exact bootstrap digest must qualify read-only inspection and
bounded result-document handling as well as implementation. Legacy v1 readiness
is sufficient only to drain already admitted finite work, never to start a
persistent worker. The containment and complete-stop requirements below remain
unchanged.

## ADDED Requirements

### Requirement: Worker availability proves the required runtime behavior
A job target SHALL advertise readiness separately from ordinary terminal-provider support. Readiness SHALL identify the pinned worker version/profile, supported structured lifecycle and question behavior, working isolation, enforced limits and stop/recovery capabilities. Missing, stale or failed readiness evidence SHALL block dispatch with an actionable reason. The selected host SHALL repeat required safety checks at launch; no failure SHALL fall back to an unrestricted process, interactive terminal or retired managed-agent mode.

#### Scenario: Sandbox executable exists but cannot operate
- **WHEN** a host has a sandbox binary installed but its runtime or resource-control preflight fails
- **THEN** autonomous jobs are unavailable on that host
- **AND** ordinary existing terminal capability is not misrepresented as proof of autonomous readiness

#### Scenario: Worker protocol is incompatible
- **WHEN** the installed or pinned OMP version cannot provide the required native RPC, question or settlement semantics
- **THEN** launch is rejected before submitting the instruction
- **AND** the UI names the missing capability rather than presenting a live job with no functioning controls

### Requirement: Workers cannot acquire ambient host authority
Each run SHALL execute within an operationally verified restricted environment, with a private checkout at the approved base commit, private session data, bounded writable storage and only explicitly permitted network destinations. The worker SHALL NOT receive the host home directory, shared repository administration directory, sibling workspaces, APAS controller credentials, SSH agent/keys, container-control endpoint or publication credentials. Only deliberately scoped model credentials and job content SHALL cross into the worker boundary. Those model credentials SHALL be treated as readable by worker tools, not described as secret from the model.

#### Scenario: Worker attempts host or sibling access
- **WHEN** worker code attempts to read a host secret, change another checkout, reach a host-control socket or contact a disallowed network destination
- **THEN** the runtime boundary denies access independently of the model's cooperation
- **AND** the approved job workspace remains the only writable project scope

#### Scenario: Repository contains hostile verification code
- **WHEN** a repository command attempts to escape its verification environment or obtain publishing authority
- **THEN** verification containment denies those capabilities just as worker containment does
- **AND** verification failure cannot downgrade execution to the trusted host

### Requirement: Limits are cumulative and enforced outside model cooperation
Admission SHALL require finite wall-time, resource, output, attempt and reported-spend limits. Hard resource and elapsed-time ceilings SHALL be enforced by the trusted execution host across the entire sandbox, including delegated workers and subprocesses. Time and spend consumption SHALL persist across reconnect and explicit retry. Spend reporting SHALL distinguish observed or estimated usage from exact billing: already in-flight requests can exceed a reported-spend threshold. A profile SHALL NOT claim a hard monetary guarantee without enforcing one, and missing required metering SHALL block or stop execution rather than display zero consumption.

#### Scenario: Root worker ignores cancellation or starts descendants
- **WHEN** a hard deadline, resource limit or cancellation is reached
- **THEN** the trusted host stops the complete restricted execution unit, including delegated work and subprocesses
- **AND** reports positive termination evidence rather than relying on the root agent to obey an abort prompt

#### Scenario: In-flight model requests finish over the spend threshold
- **WHEN** aggregate reported usage reaches the approved threshold while requests are already active
- **THEN** further model work is stopped and actual reported usage is retained
- **AND** the UI shows the threshold overrun rather than clamping usage or claiming exact prepaid enforcement

#### Scenario: Job is waiting on a person
- **WHEN** a question remains unanswered
- **THEN** the run remains subject to its approved elapsed-time and resource limits
- **AND** waiting does not silently reset or suspend the allowance

### Requirement: Native worker lifecycle is separate from pane transport
APAS SHALL control a standalone OMP worker through its qualified native structured interface. It SHALL distinguish command admission, active work, background delegation, settled execution and independently verified delivery. Native child-worker progress SHALL retain stable identities and aggregate resource use without creating APAS role panes. Existing interactive providers SHALL remain running and restorable under their existing terminal contracts.

#### Scenario: Root response yields while child work continues
- **WHEN** the root prompt returns while a delegated task or continuation remains active
- **THEN** the job stays running
- **AND** candidate verification cannot start until qualified settlement and process quiescence are established

#### Scenario: User opens a job beside existing terminal panes
- **WHEN** a job is admitted for the same project
- **THEN** the job has distinct execution and history identities
- **AND** no terminal pane is adopted as the worker, added to a managed team or removed from the pane roster

### Requirement: Human questions preserve identity and provider confirmation
Job questions SHALL be persisted with run, execution epoch, dialog and question/option identities, including multiple questions, multiple selections and custom text. The first valid answer intent SHALL win. Delivered bytes or a transport acknowledgement SHALL NOT resolve the question; the provider's matching accepted result SHALL establish the answer used. Stale, cancelled, answered-elsewhere or uncertain requests SHALL have explicit states. Unsupported delegated question paths SHALL be disabled or routed through a qualified root escalation before that execution profile is enabled.

#### Scenario: Two clients answer one question
- **WHEN** two authorized clients submit different answers to the same pending dialog
- **THEN** only the first valid intent is accepted
- **AND** all clients converge on the provider-confirmed answer, or an explicit uncertain-delivery state if confirmation is unavailable

#### Scenario: Answer delivery becomes uncertain
- **WHEN** the worker loses contact after an answer was dispatched but before provider confirmation
- **THEN** APAS reconciles the original run/dialog identity
- **AND** never blindly replays answer input into a later question or a fresh worker

#### Scenario: Dialog times out or the phone disconnects
- **WHEN** no human answer has been confirmed
- **THEN** timeout, a recommended option, a default value or client absence cannot count as human approval
- **AND** a worker profile that cannot prevent default answer continuation is unavailable for autonomous jobs

### Requirement: Execution authority expires locally without inventing stop evidence
A trusted host SHALL enforce a bounded renewable execution lease independently of the model process. Losing the server connection SHALL permit only the remaining lease grace, then terminate the execution unit. Ordinary same-host controller replacement SHALL reattach a known surviving attempt without resubmitting its prompt. Host reboot or an unverifiable local journal SHALL require reconciliation before any replacement execution.

#### Scenario: Server connection remains lost
- **WHEN** the locally enforced execution lease expires
- **THEN** the host stops the complete run and retains its stop record for later delivery
- **AND** the server keeps termination unconfirmed until it obtains trustworthy stop or absence evidence

#### Scenario: Daemon is deliberately replaced
- **WHEN** a compatible daemon reattaches within the bounded lease to a surviving job host
- **THEN** the same worker, run, event sequence and cumulative allowance continue
- **AND** no instruction is resubmitted or time allowance reset

#### Scenario: Old runtime ownership is uncertain
- **WHEN** a new controller cannot establish whether the previous execution unit still exists
- **THEN** APAS reports recovery-required
- **AND** neither a stale heartbeat nor an advisory project claim permits a second worker
