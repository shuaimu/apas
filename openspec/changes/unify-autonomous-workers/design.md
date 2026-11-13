## Context

See `proposal.md` for the product change. This document is a proposed implementation, not a claim that persistent workers currently run.

The existing `add-autonomous-jobs` implementation provides immutable `JobTaskSpec`/limits, exact project/job/run/owner/machine/epoch identity, SQLite receipts and outbox, bounded OMP `rpc-ui` execution, structured questions, leases, independent verification, and draft publication. `db/jobs.rs` explicitly rejects task/limit mutation. The service's five-second dispatcher and `JobState` represent one bounded task; terminal history is not a standing responsibility. Retaining one job forever would also grow its serialized snapshot and repeated history scans.

Legacy bots instead live in pane metadata and `dual_pane.rs`. The Claude streaming path sends native `/loop` once and watches for lost wakeups; the Codex path has a per-iteration driver. Neither is the durable worker model. Desktop still has Start/Stop bot, modal and secondary pane-grid controls; mobile browser has separate pane/workspace code. Existing jobs have browser and native creation, detail, questions, review, cache, notification and replay surfaces.

Seven predecessor operational gates remain open, including real OMP/delegation/question execution, containment/stop/recovery, independent verification and real draft delivery, physical mobile and native-device interaction. Source-level integration and previous synthetic UI smokes do not close those gates.

## Goals / Non-Goals

**Goals:** keep one persistent human-approved responsibility above immutable bounded work; make autonomous continuation restart-safe and inspectable; preserve existing containment and evidence; remove competing unattended creation/runtime paths; keep every automatic dispatch within current authority and cumulative limits.

**Non-goals:** immortal OMP processes, unlimited transcripts or spending, semantic proof that arbitrary model work matches natural-language intent, automatic coding retries after terminal failure, auto-merge or stacked PRs, arbitrary external issue/CI integrations, additional worker providers, automatic machine failover, managed team roles, or changes to ordinary interactive panes. Repository content and existing APAS outcome evidence are the initial discovery inputs; no new GitHub issue ingestion or repository polling integration is implied.

## Decisions

### 1. Put a persistent worker above the bounded job engine

Use `Worker -> WorkItem -> execution job -> run attempts`; inspection jobs have a worker but no selected item. The UI calls the lasting resource a **worker**, the bounded activity a **run**, and the selected task a **work item**. `JobWorkerProfile` remains an execution profile, not the persistent worker identity.

Add normalized durable records rather than append every cycle to one job snapshot:

| Record | Essential fields and invariant |
|---|---|
| Worker | Stable project/worker IDs, approving account, active mandate revision, desired Active/Paused state, effective status/reason, current execution, next check, schedule generation, checkpoint reference, state/event versions |
| Mandate revision | Description, ordered standing checklist, canonical repository and base branch, exact cluster-owner/machine target, pinned execution/model/verifier profiles, cadence, per-run and rolling-worker limits, policy digest/source revisions, approving actor/time |
| Work item | Stable worker-local ID, revision, origin/evidence references, scope rationale, task/criteria, priority, dependencies, status, cumulative attempts, execution/result links; only unclaimed items are directly editable |
| Checkpoint | Run and mandate/backlog/schedule revisions, observed committed base, bounded inspection/implementation result and evidence references; no credentials or executable continuation commands |
| Budget reservation/ledger | Worker and exact execution identity, reserved allowance, monotonically reconciled usage, trusted accounting timestamps, unresolved/released status and deduplication identity |
| Worker event/receipt | Scoped version/watermark or actor/request/fingerprint with durable result; reuse existing transaction, receipt and event patterns |

Extend job records with immutable worker context: purpose, worker/mandate revision, optional item/revision and reservation identity. Preserve existing task and limit immutability. One unresolved execution chain per worker is enforced transactionally in addition to the existing one-unresolved-run-per-job constraint. Worker lookups must not deserialize all historical job snapshots: index due workers, active worker jobs, item priority, event cursors and outcome pages.

Alternative rejected: leave the job nonterminal and repeatedly change its instruction. That loses approved-input identity, couples worker health to one attempt and makes history grow without bounds.

### 2. A standing grant approves responsibility, not each selected task

Mandate approval explicitly delegates task discovery and selection within the description/checklist and approved execution envelope. Future tasks resolve the current committed tip of the approved base branch; the preview says this is a moving branch input, unlike a single historical job's fixed base. Every actual execution still pins its exact SHA. A missing repository/branch or changed canonical repository identity blocks instead of using a dirty workspace or another ref.

Use the existing unattended authorization predicate, not interactive session access: active account plus project-owner or selected hosting-operator authority, current exact-machine permission, durable placement, active project and effective policy. The approving account remains accountable independently of its device. Project membership alone does not authorize recurring spending.

Description, checklist, repository, target, profile or budget edits go through a reviewed mandate revision. Fence new dispatch before replacement; apply it only after prior execution is positively stopped or finished. Preserve earlier revisions/evidence and reassess retained unclaimed backlog. Widening must fit current policy; narrowing/revocation takes the immediate-stop path where the old execution is no longer authorized. Changing the selected pane or restoring access does not retarget or resume a worker. Reapproval by a currently authorized actor records a new grant rather than silently replacing the original approver.

Natural-language scope is an instruction and review contract, not an ACL that APAS can prove. Backlog entries carry their scope rationale; prompts require escalation when outside or uncertain. Hard authority remains repository/target/profile/egress/budget/publisher containment and human review. Do not invent a validator that claims to prove semantic scope from the model's own explanation.

Alternative rejected: silently reinterpret a prior finite job approval or a pane's role/goal/backstory as recurring permission.

### 3. Separate inspection from immutable implementation

Two execution purposes reuse the existing host, RPC, lease, sandbox, gateway, question and report plumbing:

1. **Inspection:** a read-only committed checkout plus the approved mandate, standing checklist, bounded active backlog and last checkpoint. It discovers/refines/prioritizes items and proposes a selected item or `no_work`. It cannot publish or change the source checkout. Its successful result completes the bounded execution without entering candidate verification/publication.
2. **Implementation:** a separate immutable job for one accepted item/revision, from a newly resolved approved base. It gets a private writable checkout and existing candidate -> verifier -> publisher flow. Normal OMP reasoning/tools can repair mistakes within the bounded run. Independent verification remains outside that conversation.

Start with a fresh isolated OMP session for each bounded execution, supplied with durable context rather than replaying an unbounded conversation or trusting a native wakeup. This trades some context reconstruction for clean authority revisions and predictable cost/recovery. The persistent identity is APAS's worker, not that session.

The inspector ranks the backlog. After accepted inspection, APAS chooses the proposed eligible item and commits its immutable execution input in the same transition; no second human task approval is required. Do not dispatch unseen stale selection: repository, item, mandate, schedule and backlog versions are rechecked before claim. Active backlog is bounded by both item count and encoded bytes; completed/dismissed history is separately paginated, not stuffed into every prompt. Exceeding a limit is explicit and never silently truncates live work or mistakes an incomplete page for an empty backlog.

Alternative rejected: ask an unbounded TUI to remember the checklist forever. Another rejected approach is mutating a live coding task to whatever the model discovers next, which breaks immutable input and independent verification boundaries.

### 4. Use a bounded result document, not a new model-chosen reporting service

The trusted bootstrap supplies a versioned result schema and exact output location in private session storage, outside the candidate checkout. OMP writes that document with its ordinary tools. Inspection results contain the observed context revisions, proposed item mutations/ranking, selection or no-work disposition, checklist observations and bounded checkpoint summary. Implementation results identify the claimed item and a candidate-ready, blocked or no-change disposition plus bounded notes/follow-up proposals. They cannot carry shell commands, chosen filesystem paths, authority updates, confirmed usage or external completion assertions.

After native settlement, the host first disables model access and positively stops the complete execution unit. It reads only the configured regular result file using bounded no-follow/safe-path handling, validates schema/size/identity, and sends it as an authenticated exact-run report. The server compares trusted admitted context, not model-supplied identity alone. Commit checkpoint, accepted backlog updates, outcome, next eligible transition and event atomically. Replay of the same result is idempotent; conflicting revisions reject the mutation set. Inspection with missing/malformed output fails visibly. Implementation output cannot substitute for a frozen candidate or passing verifier; no-change implementation is retained for human disposition rather than silently called done.

A schedule-generation change fences future work, not retention of the already admitted run's trusted outcome. Validate that result against its recorded execution context, then separately check current mandate/backlog versions before applying model-proposed mutations and current desired state before scheduling anything else. A graceful pause can therefore retain the current inspection checkpoint or delivered candidate without starting its proposed next task. If a human edit makes proposed backlog mutations stale, reject those mutations while retaining the run's usage, stop and candidate evidence; never discard trusted outcomes merely because future scheduling was fenced.

This document is requested task output, not activity telemetry. Running/settled/child/question/usage/stop facts still come from the qualified native/trusted-host channels. No `record_turn` MCP self-report, text scraping, invented OMP API or global extension is required. Qualification must demonstrate reliable result production/validation with the pinned bootstrap; a model omitting the file fails safely rather than being assumed successful.

Alternative rejected: infer checklist mutations from assistant prose or add a second externally reachable control service inside the sandbox. Structured output is still untrusted, just simpler to validate.

### 5. Persist scheduling; do not persist an ever-running prompt

Use the existing server dispatcher cadence to claim indexed due workers in a write-reserving SQLite transaction. A claim verifies desired state, active grant, exact target readiness, no unresolved execution/review, backlog/schedule versions, remaining budget and existing project/machine/cluster capacity. Commit reservation, new job/run, worker pointer, event and outbox before sending. The host performs its existing admission/lease checks again. Worker-originated jobs must not also be admitted through the old public one-shot creation path.

At most one execution chain runs per worker, including inspection, implementation, verification, publication and uncertain recovery. Idle/paused/review-wait workers consume no active execution slot, but unresolved ownership continues to reserve it. Avoid having a not-due or independently blocked worker hold the entire machine's queue head; preserve capacity fairness across eligible workers and ordinary bounded jobs during the drain period.

A valid no-work result sets `next_check_at` using trusted server time and the approved interval, releases execution resources, and becomes Idle. Initial cadence can offer the existing 15-minute bot default, but it must be explicitly reviewed and clamped to policy, not silently imported. Check now coalesces with existing due/active work and cannot bypass the minimum interval, budget or capacity. Missed checks coalesce into one. Periodic scheduling never starts a provider merely to maintain a Working badge.

State presentation separates desired state from the active chain: queued/preparing, inspecting or working, needs-input, verifying, publishing, waiting-review, waiting-budget, idle, pausing/paused, stopping, blocked and recovery-required. Exact state/reason and allowed actions come from the server, not cached chat. A last run being review-ready does not mean the worker is completed.

Alternative rejected: daemon-local timers or OMP's `/loop` as the only scheduler. They cannot atomically fence a restart, human edit, revoked grant and budget reservation.

### 6. Reserve finite recurring allowance before each run

Require explicit finite per-run limits plus rolling-24-hour elapsed-time, reported-model-spend and execution-count limits. Per-item attempts are finite and default to one; terminal failure is not an automatic retry. Inspection and delegated work count, not just successful implementation. The approval warns that this allowance renews as old consumption leaves the window until paused; no finite predecessor allowance is promoted into it.

Allocate a bounded run within its per-run ceiling and remaining worker allowance, reserve its full admitted limits transactionally, and carry that allocation into the host/gateway. Existing immutable per-job cumulative limits still constrain any explicit retry. Item attempts aggregate across item revisions/execution jobs so renaming or revising an item cannot reset them.

Record trusted monotone usage deltas with report deduplication and server accounting times. Reservations remain held until both usage and positive stop are reconciled. Incomplete accounting cannot age out an unresolved reservation at the window boundary. Late reports charge conservatively when reconciled; duplicate cumulative reports do not charge twice. Backward clock movement cannot manufacture a replenished window. Unknown metering blocks new work; real overruns remain visible. Resource, lease and elapsed bounds stay host-enforced, including question wait and child processes; reported spend is not a promise about a final provider invoice or external CI costs.

Pause/resume, grant replacement and restart preserve the ledger. A window becoming eligible may automatically wake a still-active, authorized worker, but never a manually paused/revoked/recovery-required one. Reducing policy limits rechecks consumption and cancels when required; concurrency reduction merely prevents new dispatch.

Alternative rejected: give every iteration the old job's full finite allowance. That would make an ostensibly bounded worker unlimited by repetition.

### 7. Review is a waiting state; failures are not silent retries

Retain the trusted frozen-candidate verifier and draft-only publisher unchanged in authority. One worker may have only one unresolved delivered draft in this initial contract. After delivery, mark the item Awaiting review and the worker Waiting for review; no provider remains running. Extend the existing review-refresh path into bounded, authorized background disposition checks without model calls. Confirmed merge marks the item integrated and schedules a fresh inspection from the current approved base. Closed-unmerged or externally changed/unverifiable output requires explicit human disposition. Do not auto-merge, stack branches, reopen rejected work or use unmerged output as the next base.

A run, result validation or independent-verification failure blocks the worker with retained evidence. Explicit retry requires positive prior stop, current policy and remaining item/worker limits; it is not an automatic repair loop around the verifier. Publication retry remains the same retained candidate and never reruns OMP. A human may dismiss a failed item and resume other eligible work, but the model cannot evade its failed item's limit by creating an equivalent replacement while blocked.

Every UI and notification link includes the execution identity. The current browser review path defaults to a job's latest review while native is stricter about `run_id`; the worker cutover must fix that asymmetry rather than carry it into multi-run history.

Alternative rejected: allow unlimited unmerged changes or treat successful draft publication as the responsibility being finished.

### 8. Distinguish graceful pause, immediate stop and authority loss

- **Pause after this run:** atomically set desired Paused and advance/fence scheduling. Current already-admitted bounded work and authorized delivery can finish; show Pausing until it does. Idle becomes Paused immediately.
- **Stop now:** additionally fence answer/new publication and invoke complete trusted-host termination. Retain Stopping/Recovery required until positive evidence; retain partial artifacts, failed/stopped item and consumption. Resume does not silently retry that item.
- **Resume:** explicit authorized mutation; reconcile ownership and current grant first, then allow eligible work. A revoked grant requires reviewed renewed approval. It cannot revive unresolved dialogs or override a failed item without the corresponding explicit disposition.
- **Mandate edit:** pause the scheduling boundary, finish or explicitly stop old work, approve the replacement revision, then reassess retained items.
- **Project stop/suspend/delete or authority loss:** fence active, idle and paused workers immediately and stop affected runs, regardless of current listing page or host connectivity. Project reopening or clearing the existing job fence does not auto-resume workers.

Extend job lifecycle/delete integration to worker inventory and all historical run purge obligations. Keep leases, same-host reattachment, uncertain prompt/answer handling and positive descendant-stop evidence. Ordinary device/logout events remain separate from account/runtime grant loss.

Alternative rejected: use one ambiguous Stop button for hiding a view, pausing a responsibility, aborting a run and deleting history.

### 9. Share canonical worker APIs and exact-context client state

Rust contracts remain authoritative and flow through the existing protocol generator/validators; do not add handwritten browser/native DTOs. Negotiate `autonomous_workers_v1` on clients and `omp_persistent_worker_v1` on hosts with an explicit compatible execution-protocol revision and bootstrap digest. Old finite-job/terminal capability does not qualify recurring execution. New worker phases/results must not be sent to clients that only understand finite jobs.

Use existing proxied API prefixes and existing authentication/socket owners:

| API family | Contract |
|---|---|
| `/projects/workers` and `/projects/:project/workers` | Authorized global/project bounded inventories; project filtering before LIMIT; full approved description in project summaries without per-row detail reads |
| `/projects/:project/worker-targets`, `/worker-previews` | Eligible exact targets, effective recurring policy/readiness, reviewed create/revise/reapprove inputs |
| `/projects/:project/workers/:worker` | Snapshot, canonical status/allowed actions and watermark; approved mandate revisions |
| `.../:worker/pause`, `/stop`, `/resume`, `/wake` | Durable versioned lifecycle receipts |
| `.../:worker/items`, `/items/:item` | Bounded backlog/history and versioned human edits; explicit dismiss/retry decisions |
| `.../:worker/runs`, `/runs/:run/...` | Exact questions/answers, artifacts, checks, PR disposition and explicit publication recovery; never substitute the latest run |
| `.../:worker/events` and `/projects/worker-requests/:request` | Authorized ordered catch-up and actor-scoped receipt resolution |

UI destinations use `/code/workers/...`, not the API prefix. Reuse existing job execution/question/review components where their bounded-run contracts remain correct, but replace worker creation/control/navigation callers rather than re-export old public names. Worker callbacks resolve internal job/run IDs server-side without accepting unrelated identities from clients.

Keep separate account/project/worker/item/run/question cache keys, atomic event-plus-cursor persistence, revocation generations, secure native storage, and foreground/authenticated catch-up gates. Do not automatically replay mutations on reconnect. Keep one authenticated socket; live events remain repairable hints/replay data, not the sole durable state. Existing read-only job links remain exact archived-result destinations, not aliases that launch workers. New lists exclude internal inspection/implementation job rows from the retired standalone-job inventory.

Alternative rejected: represent workers as synthetic panes or rename all existing job DTOs while preserving task-only semantics underneath.

### 10. One visible worker product across all clients

Replace project + choices with Interactive agent / Autonomous worker. Project Overview shows worker inventory before the retained Interactive project section; selecting workers/details preserves mounted xterm instances. Hidden views are layout only. Global navigation becomes Workers and Attention. Native receives the same mandate/backlog/lifecycle flow and a project-filtered worker entry, not a WebView or just renamed finite-job screens.

Worker detail separates Description & standing checklist, Current work, Backlog, Runs/results and Controls. Show checklist observations per inspection, not an eternal percent-complete bar. Display next check or blocking dependency, recurring usage/window eligibility and current per-run usage distinctly. Policy editors expose recurring enablement/constraints separately from terminal permissions. Both clients must use server-computed allowed actions and intersect profile eligibility identically; server checks remain authoritative.

Reuse existing notification preferences/outbox with worker/run/item destinations. Notify decisions, actionable blocks/recovery and delivered review, not every idle check. Preserve generic notification bodies and authorization before navigation; no action is performed on tap. Browser gets in-app Attention and links, not a new browser-push subsystem.

Migrate old Overview/typed workspace preferences without overwriting a selection made during hydration. A saved old job destination becomes an earlier-result view, not a synthetic worker. Preserve cached-label/version fencing and purge hidden labels on revocation. The adjacent cross-machine change shares creation-menu and project-stop files; worker identity remains stable project ID, and an approved target never follows the currently selected pane or primary instance.

### 11. Clean cutover requires draining old unattended execution

Do not run both schedulers for one responsibility or ship an indefinite compatibility driver. Stage new schema/server capability while recurring enablement remains off. Inventory active finite jobs and legacy bots using durable job ownership and existing pane/host evidence. Drain finite executions and reconcile publication/stop before closing their mutation path. Existing real-worker qualification is not assumed complete.

Retain old bot prompts/provider/interval/history read-only. The user explicitly removed bot-to-worker conversion during implementation because the legacy runtime has no complete-descendant stop ledger; idle flags or a StopBot write cannot supply that evidence. No bot source API, conversion draft, live adoption, or stop-proof claim is introduced. People create and approve fresh workers independently, with an explicit OMP profile and recurring allowance. Historical deadloop metadata alone does not permanently veto those new workers; known-live legacy ownership still requires operator drain before rollout. Ordinary non-bot interactive panes remain untouched.

After drain, new code rejects legacy StartBot/one-shot admission/retry mutation requests with a clear upgrade-required response. Remove both bot drivers, start modal/default TODO loop, bot-only pause/resume branches, creation callers and obsolete team-gated bot controls. Retain only narrow persisted-data decoding/conversion and read-only historical interpretation needed to preserve user data; historical mode flags never start a loop on restore. Existing completed job records remain earlier results with original IDs, not automatically created workers. No code deletes the user's TODO.md, team files, prompts, conversations or worktrees as a shortcut.

Alternative rejected: automatically grant recurring authority to old jobs, auto-switch a Claude/Codex bot to OMP, kill unrelated panes on upgrade, or keep both old/new unattended features indefinitely.

## Risks / Trade-offs

- **Semantic scope cannot be proved from natural language** -> visible task rationale, immutable selected work, hard capability boundaries and human draft review; no false semantic-authorization claim.
- **Fresh inspection and implementation sessions add overhead** -> bounded active context/checkpoints and no provider while idle/review-waiting; choose auditability over an unbounded shared conversation.
- **The model may omit or misformat a checkpoint** -> bounded typed output, strict identity/version checks, explicit failure, and real qualification; never infer success from prose.
- **Recurring spend renews without another click** -> explicit reviewed recurring grant, outside-model reservations/metering, persistent accounting, finite cadence/run count and visible pause/stop.
- **Waiting for one PR reduces throughput** -> deliberate conservative default; avoid hidden branch stacking and duplicate fixes. Independent workers remain separate responsibilities, not a new team orchestrator.
- **A revocation or restart happens between two runs** -> durable scheduling generation, current authorization checks, one unresolved chain and no timeout-based ownership transfer.
- **Indefinite history grows forever** -> normalized bounded current state, indexed due/active queries, paginated result history and existing artifact/event retention; preserve current checkpoint/accounting and cleanup evidence separately.
- **Old bot/provider lifetime is difficult to establish** -> no bot conversion or live adoption; preserve read-only history and require operator drain during rollout without treating metadata as positive stop evidence. This implementation restarts no host or provider.
- **Adjacent multi-machine work touches the same UI/lifecycle files** -> preserve project/worker/host identities explicitly and reconcile those callsites during apply; do not assume that proposal is implemented.

## Migration Plan

1. Implement canonical worker contracts, default-denied recurring policy and additive durable records above the bounded engine. Keep all previous evidence/receipts immutable. Use current code, not stale planning text, to reconcile the adjacent project-placement work.
2. Implement server admission/grants, indexed scheduling, structured result transitions, reservations, lifecycle/revocation/deletion and exact-run APIs. Add the host's inspection/result support behind the new capability; qualify it before advertising readiness.
3. Cut over browser and native flows/caches/deep links together with generated contracts. Preserve read-only earlier results and ordinary terminal behavior. Inventory/drain legacy unattended execution; only then remove its dispatch/creation paths and activate the new compatible contract.
4. Exercise real pinned OMP inspections (no work and discovered item), selected implementation, structured questions/delegation, independent failed/passing verification and authorized draft delivery. Prove idle resource release, recurring wake, review-wait/merge continuation, budgets across runs/restart, pause-vs-stop races, revocation while idle, stale results, uncertain ownership and full deletion/purge. No production synthetic jobs or unapproved real provider/PR actions.
5. Exercise actual desktop/mobile-browser UI, physical mobile keyboard/zoom and native iOS/Android device/emulator flows. Preserve all predecessor gates still relevant; synthetic records, component tests or typechecks cannot close real-worker/device gates.
6. After smoke proof, update the canonical runbook and existing mobile operations documentation. Deployment/enablement is a separate authorized operation: server/shared -> web -> compatible host CLI, with the native release following its normal process. Explicitly approve intended recurring policies and each new worker, including finite-job conversion drafts; no migration enables spend.
7. Rollback first disables recurrence and fences new dispatch, then drains or positively stops every worker execution. Retain a compatible control/host version until unresolved runtime and artifact ownership are reconciled. Preserve additive worker/history data read-only; an older daemon must never interpret worker state as a bot or auto-resume drained deadloop flags.

### Predecessor specification reconciliation

`add-autonomous-jobs` remains the implementation/evidence record for the bounded engine. This change replaces its public single-task admission/creation boundary only for the new worker product and keeps its safety/delivery guarantees. Because its capability deltas are not yet main specs, this proposal uses new worker capabilities rather than inventing MODIFIED entries for nonexistent main requirements. Before either change is synchronized/archived, reconcile the predecessor's task-only admission, explicit retry, UI creation, and protocol wording with the worker-originated internal execution boundary; retain historical finite-job semantics and all unfinished qualification evidence. Do not archive either change as complete while that reconciliation or its actual acceptance gates remain open.
