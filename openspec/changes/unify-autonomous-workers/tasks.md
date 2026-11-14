## 1. Canonical contracts and compatibility

- [x] 1.1 Add canonical Rust worker, mandate-revision, standing-checklist, work-item, checkpoint, status/reason, allowed-action, preview, receipt and event DTOs with independent worker/item/job/run identities.
- [x] 1.2 Define inspection versus implementation inputs/results, immutable worker context on bounded jobs, safe bounded result-document schemas, and a successful no-work inspection outcome that cannot enter verification/publication.
- [x] 1.3 Define finite per-run and rolling-24-hour worker limits, usage/reservation summaries, per-item cumulative attempts, readiness evidence, and recurring-policy types without reinterpreting existing finite-job or terminal permission.
- [x] 1.4 Add negotiated `autonomous_workers_v1` and `omp_persistent_worker_v1` capabilities with an explicit compatible execution protocol/bootstrap revision; reject unqualified hosts and keep new worker variants out of old client contracts.

## 2. Durable worker state and immutable execution links

- [x] 2.1 Add an idempotent SQLite migration for workers, mandate revisions, bounded active backlog, checkpoints, reservations/usage ledger, worker events and receipts, with indexes for due/active workers and paginated history.
- [x] 2.2 Extend immutable job admission context with worker/purpose/item/revision/reservation references; enforce one unresolved chain per worker without weakening existing task/limit immutability or per-job unresolved-run constraints.
- [x] 2.3 Implement atomic worker snapshots/events/watermarks and actor-scoped request receipts, including payload conflicts, optimistic versions, ordered replay and explicit retained-cursor resynchronization.
- [x] 2.4 Implement bounded backlog mutation/selection and current checkpoint storage separately from historical executions; preserve current context/accounting/cleanup evidence when old event/log/artifact retention expires.

## 3. Approval, policy and continuous authority

- [x] 3.1 Implement reviewed create/revise/reapprove previews and confirmation for the standing mandate, exact repository/base branch, target/profiles, cadence and recurring allowance; explain future committed branch tips and automatically renewing budgets.
- [x] 3.2 Add independently default-denied recurring-worker policy to deployment/cluster/project resolution and administration, using intersected profiles, minimum ceilings, maximum minimum-cadence and current source-revision digests.
- [x] 3.3 Apply existing unattended exact-machine authorization to grant and runtime transitions; distinguish content reads, stop authority, account grant loss and ordinary device logout/revocation.
- [x] 3.4 Extend authority/lifecycle reconciliation to all workers, including idle/paused and later pages; revoke recurrence durably so restored access or a cleared transient project fence cannot silently resume it.
- [x] 3.5 Extend project stop/suspend/delete and database deletion barriers to current workers and every historical started execution's exact-host stop/purge obligation; retain cleanup records through disconnected-host recovery.

## 4. Scheduling and recurring allowance

- [x] 4.1 Add indexed due-worker claims to the existing dispatcher, atomically reserving worker ownership, capacity, allowance, job/run identity and outbox intent before dispatch; prevent overlap with the legacy job dispatcher.
- [x] 4.2 Implement no-work Idle checkpoints, next-check scheduling, due-tick coalescing, bounded manual Check now and fairness among eligible workers; idle/paused/review-wait workers consume no execution slot.
- [x] 4.3 Resolve each selected implementation against the approved repository/base branch and pin its exact SHA/item/mandate/profile revisions; reject stale selection, missing refs and silent retargeting.
- [x] 4.4 Implement full admitted-allowance reservations and deduplicated rolling usage reconciliation across inspection, implementation, children and explicit retries; retain unresolved reservations beyond window rollover and never treat unknown usage as zero.
- [x] 4.5 Preserve accounting and per-item attempts through restart, pause/resume and mandate/item revisions; expose budget eligibility time, delayed/estimated spend and actual overruns without claiming exact invoice enforcement.
- [x] 4.6 Keep future schedule-generation fences separate from current execution-outcome reconciliation; prove pause/edit/revoke races cannot launch another run or discard trusted usage, stop or candidate evidence.

## 5. Bounded OMP inspection and checkpoint execution

- [x] 5.1 Extend the qualified job-host launch path with read-only inspection checkouts and bounded mandate/backlog/checkpoint context; keep implementation's private writable checkout and existing credential/network/resource isolation.
- [x] 5.2 Extend the private OMP bootstrap/prompt with the versioned result schema and fixed session-storage output path, using ordinary tools rather than a new MCP reporter, global extension or provider-native forever loop.
- [x] 5.3 Read the bounded regular result file only after qualified settlement and positive complete-unit stop; reject symlinks, unsafe/malformed/oversized documents and mismatched context without interpreting output as commands or authority.
- [x] 5.4 Implement authenticated result reports and atomic server acceptance of backlog/checkpoint proposals, including duplicate result replay, stale human-edit rejection, successful no-work completion and retained failure evidence.
- [x] 5.5 Extend readiness and inventory/adoption to the new execution purpose/result contract while retaining child settlement, no-timeout structured questions, local leases, uncertain-prompt handling, complete descendant termination and no unrestricted fallback.

## 6. Outcomes, review and lifecycle controls

- [x] 6.1 Implement Pause after this run, Stop now, explicit Resume and reviewed mandate replacement, with distinct durable desired/effective states, publication fences and positive-stop completion semantics.
- [x] 6.2 Preserve root/question/run/epoch identity and provider-confirmed answers; make expired/stopped dialogs read-only and retain their blocking outcome without replaying answers into later executions.
- [x] 6.3 Bind selected items to the existing immutable candidate, independent verifier and trusted draft publisher; prevent model checkpoints and no-change implementation reports from claiming completion.
- [x] 6.4 Implement one-unresolved-draft review gating and bounded authorized background PR-disposition refresh without model calls; confirmed merge permits fresh inspection, while close-unmerged or changed/unknown heads require explicit disposition.
- [x] 6.5 Implement actionable terminal-failure blocking, explicit item dismissal/retry with remaining item/worker/job limits, and separate same-candidate publication recovery; do not create an automatic repair loop or reset attempts by creating another internal job.

## 7. Shared API, replay and protocol consumption

- [x] 7.1 Add canonical worker inventory/target/preview/detail/mandate/lifecycle/backlog/run/question/review/receipt APIs under `/projects/`, with project filtering before pagination and full descriptions without per-row detail fetches.
- [x] 7.2 Add capability-gated worker subscription/inbox hints and authorized ordered catch-up on the existing socket; reauthorize subsequent fan-out, evidence reads, receipts and all native HTTP mutations.
- [x] 7.3 Resolve exact worker/item/run/question relationships server-side and retain requested historical run identity through browser and native review APIs instead of defaulting to the latest result.
- [x] 7.4 Export worker REST/event contracts through the existing schema pipeline, update runtime validator categories and regenerate canonical protocol schemas/types after Rust interfaces converge; migrate every affected consumer without compatibility re-exports.

## 8. Browser worker workspace

- [x] 8.1 Replace one-shot composition with reviewed description/standing-checklist/target/cadence/recurring-budget approval, retaining project-fixed drafts, explicit target choices and receipt-driven navigation on desktop and mobile browser.
- [x] 8.2 Replace project job identity/navigation with typed worker and exact earlier-result views in both `TabbedView` and `MobileSessionActivity`; preserve mounted terminals, empty pane rosters and the selected project.
- [x] 8.3 Implement Overview and worker detail for full descriptions, checklist observations, current item, backlog editing, canonical status/next check, budgets, lifecycle controls and paginated run/result history.
- [x] 8.4 Cut over global Workers/Attention, sidebar/rail/mobile-home entries and `/code/workers` destinations; retain exact read-only earlier-result links and fix browser historical-run review selection.
- [x] 8.5 Extend IndexedDB/cache/replay scopes and migrate view/draft preferences without granting authority; retain atomic cursors, account/project purge barriers, stale-response fences, cached labels and explicit same-ID mutation recovery.
- [x] 8.6 Extend existing administrative policy editors with requested/effective recurring settings and readiness reasons; keep profile filtering and server-provided allowed actions consistent with native clients.

## 9. Native worker workflow

- [x] 9.1 Replace native finite-job creation and global Jobs navigation with worker approval, project-filtered inventory and Workers/Attention using native components and the existing authenticated stack.
- [x] 9.2 Implement native mandate/checklist, current work, editable backlog, pause/stop/resume/wake, budgets and paginated execution/result views with server-authoritative state and allowed actions.
- [x] 9.3 Extend SQLCipher/query-cache/draft/receipt/replay scopes for workers and exact item/run/question identity, preserving foreground reauthorization, all-cached-project revocation discovery and no automatic POST replay.
- [x] 9.4 Update authenticated deep links and notification routing to exact worker/run/question/review destinations; retain authorized read-only earlier-result links without changing selected sessions or executing actions on navigation.

## 10. Notification and legacy cutover

- [x] 10.1 Extend the existing notification outbox, preferences, deduplication and recipient authorization for worker decisions, failures/recovery and draft review; suppress routine inspection/wakeup completion spam and retain generic payloads.
- [x] 10.2 Implement explicit finite-job conversion drafts requiring fresh provider/mandate/recurring approval; preserve old bot prompt/provider/cadence/history read-only with no bot conversion or live terminal adoption.
- [x] 10.3 Add cutover inventory/readiness and clear retired-mutation rejection so known-live finite jobs/bots must drain before legacy unattended rollout; disconnected or uncertain execution ownership remains visible, while historical bot metadata alone does not permanently block fresh workers.
- [x] 10.4 Remove legacy bot drivers, Start/Stop bot creation controls and modal/default-TODO-loop code, bot-only pane-grid branches, stale team-gated bot behavior and standalone-job admission/retry callers after the cutover contract is implemented; keep only finite-job draft conversion and nonexecuting history interpretation.
- [x] 10.5 Preserve earlier job receipts/results and pane history as read-only evidence, migrate old view/draft references safely, and reject deadloop restoration without deleting user TODO/team files or restarting unrelated interactive panes.

## 11. Behavioral verification and operational qualification

- [x] 11.1 Add focused regressions for one-worker claim races, duplicate requests/results, stale mandate/backlog edits, pause-versus-checkpoint completion, coalesced missed checks and safe empty-backlog Idle behavior; avoid wiring/source-text tests.
- [x] 11.2 Add focused regressions for rolling allowance across runs/restart, duplicate/late/unknown usage, unresolved reservations across a window boundary, clock rollback, per-item attempt persistence and current-policy narrowing.
- [x] 11.3 Add focused regressions for revocation/restoration while idle, device-versus-account authority, full-inventory deletion/purge, stale epochs, expired questions, verified-draft review gating, exact historical run links and rejection of old creation/restore paths.
- [ ] 11.4 On an explicitly authorized isolated host/repository, qualify the pinned OMP/bootstrap and existing containment prerequisites, then run actual no-work and discovered-item inspections through structured checkpoint acceptance and a separate selected implementation; observe real idle resource release and next scheduled wake.
- [ ] 11.5 Exercise real root/child settlement, human-question confirmation/expiry, aggregate metering, pause/stop/revoke races, daemon/server replacement, lease expiry, returning-host reconciliation and budget exhaustion/replenishment without duplicate prompts or unaccounted-for processes.
- [ ] 11.6 Exercise real failing/passing independent verification and authorized draft publication, lost publication response, review-wait without model calls, external merge continuation and closed-unmerged blocking; retain candidate/PR identities as evidence and do not substitute synthetic UI fixtures for this gate.
- [x] 11.7 Smoke the actual desktop and separate mobile-browser project/global flows at 320/390/768px and desktop, including zero panes, older paginated results, terminal continuity, offline/revocation, late preference hydration, exact review links and finite-job conversion/read-only bot history without unexpected lifecycle commands.
- [ ] 11.8 Qualify physical mobile-browser keyboard/zoom/safe-area and native iOS/Android device/emulator creation, backlog editing, lifecycle, accessibility/large text, background/reconnect and notification links; distinguish these results from desktop emulation and native typechecks.
- [x] 11.9 After integration, run affected Rust/protocol/web/native checks and production web build once, validate generated contracts, run strict OpenSpec validation and the canonical runbook check; report only exercised verification and preserve uncompleted predecessor qualification gates.

## 12. Documentation and release boundary

- [x] 12.1 After smoke proof, update the canonical runbook and existing mobile operations documentation with recurring approval/accounting, OMP qualification, inspection/result behavior, control semantics, finite-job conversion/read-only bot history, recovery and retention; do not create another runbook.
- [x] 12.2 Reconcile the predecessor's task-only admission/UI/protocol requirements with worker-originated internal execution before final spec synchronization/archive, preserving its historical evidence and remaining acceptance gates; reconcile shared callsites with the then-current cross-machine project change.
- [x] 12.3 Document the default-disabled server/web/CLI/native rollout and rollback/drain checks; deploy, enable recurring policy or approve real workers only through a separate authorized operational action.

## Local verification and remaining qualification — 2026-11-12

Implementation and local integration are complete: **56 of 60 tasks**. Tasks
11.4, 11.5, 11.6 and 11.8 remain operational gates, not inferred passes.
The user-approved cutover preserves historical bots read-only without conversion;
finite-job reuse remains an unapproved draft requiring fresh recurring approval.

### Exercised checks

- Rust workspace run: CLI **560 passed, 1 ignored**; shared **80 unit and 3
  integration tests passed**. The server initially exposed a default-stack
  WebSocket crash. After boxing the two large snapshot payloads,
  `cargo test -p apas-server` passed **429 tests**, including real socket
  authentication/subscription and the existing cross-machine registration path.
  CLI/server debug builds passed, with the server rebuilt after the repair.
- Protocol TypeScript and **57 tests passed**. All six schemas exported from
  the final Rust contracts, and the generated TypeScript, matched the retained
  generated files byte-for-byte. Snapshot boxing changed no JSON contract.
- Web: **104 suites / 956 tests passed**; final production build and TypeScript
  passed. ESLint reported three warnings (share-page dependency and two ref
  cleanup warnings), no errors.
- Native: TypeScript, zero-warning lint and **22 suites / 136 tests passed**;
  Android Hermes export succeeded. This is not device interaction qualification.
- Strict OpenSpec validation passed for this change and `add-autonomous-jobs`;
  `python3 scripts/check_agent_runbooks.py` passed. Predecessor historical
  evidence and its unchecked operational gates were retained.

### Actual browser/API smoke

The real server and production web bundle ran on isolated localhost ports with
an owner-private temporary database/configuration. Chromium exercised desktop
and separate mobile-browser surfaces at **320, 390, 768 and 1440px**:

- Zero-pane Overview, 61-worker inventory paging/filtering, and 53-run history
  paging; an older candidate link retained its distinct SHA rather than opening
  the latest run.
- Project-fixed draft recovery across navigation/reload, offline draft editing,
  stale read-only detail, and unavailable recurring approval without a qualified
  target.
- Discovered project revocation returned 403, removed visible protected content
  and left zero project-scoped IndexedDB records. Restored access required an
  explicit recheck.
- A deliberately delayed IndexedDB preference read could not replace an
  intervening Overview selection; the selected view was durably retained.
- Desktop/mobile worker navigation retained the same mounted xterm node.
  Selecting another pane now hides that retained terminal; historical bot
  instructions remain read-only. The smoke found and fixed this wrong-pane
  visibility defect and added its regression.
- Earlier finite instructions copied only after explicit confirmation into an
  unapproved draft; no worker or permission was created. Retired standalone
  admission returned a rejected receipt, and unqualified worker preview returned
  `worker_target_unavailable`.

All records and terminal frames were explicitly synthetic. No provider process,
real verification, publication, model request or pane input/lifecycle command was
used. Recurring policy stayed disabled. Browser instrumentation, transports and
services were closed; the temporary smoke data is disposable, not qualification
evidence for an execution profile.

### Missing operational prerequisites

- **11.4–11.6:** operator-approved immutable worker/verifier inputs and v2
  bootstrap qualification, dedicated finite workspace filesystems, scoped
  model/publisher credential files and an explicitly authorized execution/PR
  repository. Rootless Podman info succeeded with isolated private storage after
  the default runroot was found to belong to another UID; that proves neither
  containment nor actual OMP settlement/accounting/delivery.
- **11.8:** physical mobile-browser devices and iOS/Android development devices
  or emulators for keyboard, zoom/safe-area, accessibility, lifecycle and
  notification-link qualification. This host had no `adb`, Android emulator or
  `xcrun`; desktop viewport emulation and Android export do not satisfy the gate.

No production deployment, policy enablement, real-worker approval or archive was
performed. Continue the four open tasks only in the separately authorized
operational environment.

### Production state — 2026-11-13

The persistent-worker cutover reached production with server and web release `20261113T013925Z`, built at e02c0f7, at 01:48 UTC on 2026-11-13. Web release `20261113T193707Z` followed at 19:39 UTC. No job policy row exists and there is no worker, so recurring execution stays disabled. No policy was enabled, no real worker was approved and no execution profile was qualified. The four open tasks above remain open. Task 8.6 of `add-omp-tab-autopilot` was applied here on the same day: the proposal gained the section "Applicability after `add-omp-tab-autopilot`" and one requirement in `autonomous-worker-surfaces` was retitled "The worker flow replaces legacy creation paths".

## 13. Amendments after `refine-interactive-workspace` (2026-11-13)

- [x] 13.1 Correct "Worker navigation preserves terminal and project identity" in the `web-project-workspace` delta: `+` creates an interactive agent only, and a worker is created from the Overview or the Workers page. Add the scenario "A person opens the new-tab control".
- [x] 13.2 Correct "Project Overview presents persistent workers independently of panes" in the same delta: the interactive sections come before the worker inventory.
- [x] 13.3 Correct section 10 of `design.md`: `+` no longer offers Interactive agent / Autonomous worker, and the Overview shows the interactive sections first.
- [x] 13.4 Add "Applicability after `refine-interactive-workspace`" to `proposal.md`.

These are text corrections only. The code they describe is recorded in `refine-interactive-workspace`. Tasks 11.4, 11.5, 11.6 and 11.8 stay open and unchanged. Nothing in this change named Pi.
