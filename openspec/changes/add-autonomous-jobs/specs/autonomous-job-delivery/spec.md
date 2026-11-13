## Purpose

Ensure autonomous job results are independently checked, tied to immutable candidate content and delivered as reviewable draft pull requests without granting the coding worker publishing or merge authority.

After persistent-worker cutover, these delivery requirements apply to
implementation executions with an accepted `candidate_ready` result. Successful
read-only inspections and implementation `no_change`/`blocked` results never
enter this pipeline. The worker's one-unresolved-draft and external-review
continuation rules add constraints; they do not weaken candidate verification or
permit automatic merge. Historical finite-job evidence remains readable.

## ADDED Requirements

### Requirement: Candidate content is frozen and complete before verification
After worker settlement, APAS SHALL stop or make the worker unable to mutate its candidate, capture the full reviewable difference from the admitted base including relevant untracked files, and produce an immutable candidate identity. Candidate packaging SHALL enforce safe repository paths, file types, size limits and the approved repository boundary. APAS SHALL preserve worker logs and failure evidence without treating worker-authored Git metadata, commit hooks or reported test results as trusted authority.

#### Scenario: Worker leaves uncommitted and untracked changes
- **WHEN** execution settles with changed tracked files and newly created source files
- **THEN** the candidate and diff include the complete accepted change set, not only pre-existing commits
- **AND** excluded or unsupported files are explained rather than silently omitted from a successful result

#### Scenario: Worker modifies its repository configuration
- **WHEN** the candidate contains worker-controlled hooks, remote configuration or unsafe paths
- **THEN** privileged packaging/publication does not execute that metadata or follow paths outside the job boundary
- **AND** an unpackageable candidate blocks delivery with retained diagnostic evidence

### Requirement: Independent verification binds to exact candidate identity
APAS SHALL run the admission-selected verification profile in a fresh restricted environment against the exact immutable candidate. Verification evidence SHALL identify the candidate, base, image/profile/check versions, individual check outcomes, exit statuses and bounded logs. Skipped, timed-out, missing or failed required checks SHALL NOT count as passing. Passing configured checks SHALL be described as verification evidence, not proof that arbitrary acceptance criteria or code are correct.

#### Scenario: Worker reports success but a required check fails
- **WHEN** APAS's independent check fails on the frozen candidate
- **THEN** the job is not review-ready and no draft PR is published as verified delivery
- **AND** the failed check, candidate diff and logs remain reviewable from both mobile surfaces

#### Scenario: Candidate changes after a passing check
- **WHEN** the content intended for publication differs from the verified candidate
- **THEN** prior verification cannot authorize its publication
- **AND** the mismatch is reported instead of publishing a moving branch

#### Scenario: Required verification cannot run
- **WHEN** an approved verifier image, command or dependency is unavailable
- **THEN** the job is blocked or failed with the precise verification reason
- **AND** APAS does not substitute worker-reported results or a weaker check profile

### Requirement: Publication is trusted draft-only and idempotent
Only an APAS-controlled publisher outside the worker and verifier boundaries SHALL hold repository publication credentials. After current authorization/policy and passing-candidate checks, it SHALL publish that exact candidate to the approved repository and base on a unique job-owned branch and reconcile at most one draft PR for the job. It SHALL use a trusted repository/configuration context and SHALL NOT invoke worker-controlled hooks or general shell instructions with publication credentials. No automatic merge, GitHub approval, force overwrite of another branch or use of legacy auto-merge flags is permitted.

#### Scenario: Verified candidate is published
- **WHEN** delivery is authorized and required checks passed on the exact candidate
- **THEN** APAS creates or locates the job's draft PR and records repository, number, URL, base and head identity
- **AND** the coding worker never receives the publication credential

#### Scenario: GitHub response is lost
- **WHEN** push or PR creation may have succeeded but its response was lost
- **THEN** publication is reconciled against the recorded repository and unique job branch before any retry
- **AND** neither a second PR nor another coding run is created to recover the response

#### Scenario: Another actor changed the publication branch
- **WHEN** the existing job branch or PR head differs from APAS's recorded expected state
- **THEN** APAS blocks publication and reports the conflict
- **AND** does not force-push over the external change or attach old verification to the new head

### Requirement: Delivery evidence outlives execution and failed publication
Authorized users SHALL be able to inspect the task, admitted base, each run's candidate, per-file diff, verification results, usage and publication outcome after the worker or interactive project session exits. Failed publication SHALL remain separate from code-generation or verification failure, and recovery SHALL NOT rerun the model. Evidence SHALL be size-bounded, access-controlled and retained under an explicit policy, with visible truncation or expiry. Project deletion SHALL remove its retained content after runtime cleanup is established.

#### Scenario: Publication credentials fail after verification
- **WHEN** publishing a passing candidate fails because authorization or the publication service is unavailable
- **THEN** the job retains its passing-candidate evidence and an actionable publication-blocked status
- **AND** a currently authorized explicit publication retry uses the same candidate and receipt workflow without another model run

#### Scenario: User opens an old result on a phone
- **WHEN** the job worker and its project session are no longer running
- **THEN** retained diff, check results and draft-PR evidence remain available without a terminal attachment
- **AND** expired logs or diffs are labeled expired rather than empty or successfully checked

#### Scenario: Human closes or merges the PR externally
- **WHEN** APAS refreshes the draft PR's external state
- **THEN** it reports that disposition separately from the historical execution and verification result
- **AND** opening review or tapping a PR link never triggers approval or merge
