## 1. Shared contracts and qualification inputs

- [x] 1.1 Add canonical Rust job/run/state/limits/readiness/question/artifact/receipt DTOs and exact `autonomous_jobs_v1` / `omp_job_worker_v1` capabilities in `crates/shared`, including defaulted compatibility fields and explicit allowed actions.
- [x] 1.2 Define daemon prepare/start/lease/answer/cancel/inventory/verify/publish commands and correlated results with project, owner, machine, run, epoch, command and source-sequence identities; keep existing terminal variants unchanged.
- [x] 1.3 Export REST-only job contracts through the existing mobile schema exporter, extend protocol fixture categories and REST/event validators, and prepare web consumption of `@apas/protocol` without handwritten duplicate DTOs. Regenerate final outputs in section 13 after contract edits converge.
- [ ] 1.4 Select immutable worker/verifier images and explicit model/publication profiles; establish operator-provided rootless Podman, cgroup/quota, private storage and credential prerequisites without changing shared provider installations. Record unavailable reasons, including the observed runroot failure, until each prerequisite is actually satisfied.

## 2. Durable job state and receipts

- [x] 2.1 Add the idempotent SQLite migration for jobs, runs, mutation receipts, questions, events, artifact manifests and command outbox, including one-unresolved-run and event-deduplication constraints and active-target/cursor indexes.
- [x] 2.2 Implement immutable admission preview/fingerprint and transactionally confirmed job creation with pinned base/task/profile/policy inputs; reject changed previews and preserve same-actor same-request results across restart.
- [x] 2.3 Implement atomic state transitions, state/event version allocation, FIFO target queues and project/cluster/machine capacity reservation; include explicit blocked/recovery/cancelling reasons rather than deriving success from process state.
- [x] 2.4 Implement cumulative elapsed/spend/attempt accounting and explicit coding retry from the approved base only after old execution is positively absent; never reset consumption on reconnect.
- [x] 2.5 Implement bounded artifact/event storage, declared retention/expiry and snapshot-resync responses; retain unresolved ownership and stop evidence and integrate post-stop project-content deletion.

## 3. Authorization policy and lifecycle enforcement

- [x] 3.1 Add job content, admission, answer, retry, publication and stop-specific authorization using canonical placement and exact-machine rules; retain separate system-admin credentials and reject project-share-only unattended admission.
- [x] 3.2 Add default-disabled typed autonomous policy to existing deployment/cluster/project policy services, with fail-closed parsing, monotone enablement/allowlists/ceilings, source-revision digest and unchanged administration authority.
- [x] 3.3 Expose requested/effective job policy, inherited bounds and safe readiness reasons in existing management surfaces; audit actor/project/cluster/policy identities without task content or secrets.
- [x] 3.4 Integrate one job authority-change hook into project leave and share/cluster/admin membership removal, owner transfer, machine-grant changes, account suspension and policy updates; cover all affected runs rather than the first project page.
- [x] 3.5 Extend project stop/suspend/delete and restart recovery to durable job inventory, including disconnected hosts; fence new work before stopping, preserve pending cleanup until positive evidence, and distinguish device revocation from account/runtime revocation.

## 4. Persistent host ownership and reconciliation

- [x] 4.1 Extend the existing persistent-host implementation with a versioned authenticated pipe-based job backend, separate stdout RPC/stderr handling and bounded frames; preserve existing PTY host compatibility.
- [x] 4.2 Add the private job runtime namespace and provisioned host-local durable storage, with local locking, fsynced launch/answer receipts, boot/container identity and bounded event spool; exclude jobs from pane-roster reconciliation and unknown-pane descriptor cleanup.
- [x] 4.3 Integrate daemon job inventory/adoption independently of running project panes; same-host re-exec must retain the exact attempt, prompt state, source sequence and remaining allowance.
- [x] 4.4 Implement independently enforced 120-second execution leases and 20-second authorized renewal, cumulative deadlines and whole-container stop escalation; persist stop evidence even when the server is unreachable.
- [x] 4.5 Implement startup/reconnect inventory reconciliation, stale connection/epoch rejection and recovery-required states for uncertain prompt admission, answer delivery or old ownership; never replace an unaccounted-for runtime.

## 5. Sandbox and scoped model access

- [x] 5.1 Implement standalone committed-base checkout preparation without linked-worktree metadata, shared alternates or dirty source-copy inclusion; construct a minimal explicit worker environment and private session directory.
- [x] 5.2 Implement rootless Podman worker/verifier profiles with pinned images, non-root execution, read-only image, private namespaces, dropped privileges, enforced CPU/memory/PID/storage/tmpfs limits and a container/cgroup stop unit; no unrestricted fallback.
- [x] 5.3 Implement the run-scoped egress bridge with no direct external route, fixed model upstreams and explicit dependency allowlists; deny arbitrary tunnels, host-control endpoints, private/metadata access and redirect/DNS rebinding escapes.
- [x] 5.4 Implement the trusted model gateway's dedicated upstream credentials and run-scoped worker tokens, qualified OpenAI Responses and Anthropic-compatible Messages forwarding, model/output validation and bounded concurrent requests.
- [x] 5.5 Implement durable aggregate gateway usage and reported-spend stopping across root/delegated calls, provider errors and reconnect; refuse unknown required metering and retain visible in-flight overruns rather than claiming hard billing enforcement.
- [x] 5.6 Implement timestamped worker readiness probes for actual sandbox/resource/network/credential/stop behavior and profile compatibility; expire stale reports, repeat launch-sensitive checks and expose safe actionable reasons.

## 6. Native OMP lifecycle and questions

- [x] 6.1 Implement the pinned NDJSON RPC adapter using tool-UI RPC without a PTY, negotiate protocol/frame limits, enable structured ask dialogs and keep command admission, prompt yield, background work and session settlement distinct.
- [x] 6.2 Configure isolated OMP task/async/model settings and `ask.timeout=0` at supported highest priority; reject effective timeout/default-answer behavior, unsupported capabilities or global/repository settings that widen the approved profile.
- [x] 6.3 Implement native subagent progress/usage subscriptions and the qualified child-needs-input to root-question contract; retain stable child identities without creating APAS panes or a managed-team loop.
- [x] 6.4 Persist complete multi-question dialogs, immutable option identity/label mappings, first-writer answer intent and provider-confirmed outcomes; reject ambiguous labels, stale run/dialog identities and blind answer retransmission.
- [x] 6.5 Normalize bounded progress, usage, questions, failure and settlement into job events; stop or quarantine malformed protocol behavior and never treat a root response or worker-authored success statement as verified completion.

## 7. Server dispatch and recovery

- [x] 7.1 Connect durable queue/outbox dispatch to current authenticated daemon registrations and fresh readiness; reserve capacity before launch and reauthorize at dispatch and every lease renewal.
- [x] 7.2 Implement idempotent delivery/reconciliation for start, answer and cancel commands with local host receipts; distinguish retrying command delivery from resubmitting an OMP prompt.
- [x] 7.3 Implement durable cancel fencing across queued, running, question, verification and publication phases, including positive termination reconciliation and preserved partial artifacts.
- [x] 7.4 Recover nonterminal jobs after server restart through exact recorded host inventory, not in-memory machine presence or advisory NFS claims; retain unresolved offline runs and prevent automatic machine failover.

## 8. Independent verification and draft publication

- [x] 8.1 Freeze worker mutation and package a complete bounded candidate, including relevant untracked files; reject traversal, escaping symlinks, special files and worker Git metadata without silently dropping intended source.
- [x] 8.2 Construct the immutable candidate commit in a clean trusted Git context and execute approved checks in a fresh restricted verifier; store candidate/base/image/profile/check identities, exit/timeout outcomes and bounded logs.
- [x] 8.3 Implement a separate trusted publisher with repository-scoped credentials, qualified protected-base/CI policy, explicit repository/base/head and expected-ref checks on `apas/job-<uuid>`; never run worker hooks/config or merge/approval commands.
- [x] 8.4 Implement durable draft-PR creation/reconciliation and explicit publication retry against the same verified candidate; handle lost GitHub replies, external branch changes and cancellation during in-flight publication without duplicate PRs or model reruns.
- [x] 8.5 Implement retained review manifests, paginated changed-file/unified-diff/check-log access, explicit truncation/expiry and separately refreshed external PR disposition after workers and sessions exit.

## 9. Shared API replay and mobile bootstrap

- [x] 9.1 Implement the account-authenticated `/projects/` job target/preview/create/list/detail/run/question/cancel/retry/receipt/review/artifact endpoints from the design, with stable reason codes and native HTTP feature gates.
- [x] 9.2 Add capability-gated job subscription, snapshot/high-watermark, ordered event replay, inbox hints and correlated denial/resync handling on existing WebSockets; reauthorize fan-out/downloads and repair dropped live frames through durable sequence.
- [x] 9.3 Extend native bootstrap with defaulted job availability/inbox/target summaries or shared-API cursors, without requiring a live session or inferring readiness from terminal OMP profiles.
- [x] 9.4 Connect safe generic job events to the existing notification outbox/preferences with event deduplication and current-recipient/device checks; add opaque job/run/question/review destinations and retain foreground Attention without push permission.

## 10. Responsive web workflow

- [x] 10.1 Add authenticated `/code/jobs` list/new/detail/question/review routes plus mobile Jobs/Attention and desktop navigation, reusing one existing authentication/socket owner and preserving session selection.
- [x] 10.2 Implement instruction/acceptance/target/profile/limit composition and immutable admission review, awaited account/project-scoped draft/request persistence, uncertain-receipt recovery and explicit same-ID retry.
- [x] 10.3 Implement phase/target/run/usage detail, bounded timeline/subagent expansion, actionable recovery/unavailable states and cancellation-requested versus stopped presentation; keep controls disabled until the selected job completes catch-up.
- [x] 10.4 Implement all-question single/multi-select/custom-text forms with persistent scoped drafts, first-writer/provider-confirmed outcomes and answered-elsewhere/stale/uncertain states, without PTY fallbacks.
- [x] 10.5 Implement retained narrow-screen review with file navigation, paginated unified diff, check/log evidence and safe draft-PR handoff, including failed verification with no PR.
- [x] 10.6 Implement transactional event/cursor persistence and complete account/access cache purging; enable browser zoom and meet 320px, 44px-target, keyboard/safe-area, focus and screen-reader requirements without breaking existing session/terminal interaction.

## 11. Native mobile workflow

- [x] 11.1 Add discoverable Jobs/Attention navigation and protected native job/new/detail/question/review screens, preserving the code-session home and exposing matching `/code/jobs/...` aliases without WebViews.
- [x] 11.2 Implement the native composer/admission review with encrypted awaited draft/request persistence, restored uncertain receipt resolution, eligibility changes and explicit offline/read-only behavior.
- [x] 11.3 Implement native job phase/limits/subagent timeline, cancellation and recovery controls with bounded list rendering and per-job synchronization rather than socket-connected readiness.
- [x] 11.4 Implement full multi-question/native choice and custom-text forms, persistent scoped answer drafts and provider-confirmed/answered-elsewhere/stale outcomes; remove reliance on the legacy first-question scalar renderer for jobs.
- [x] 11.5 Implement native retained candidate/diff/check/log review and draft-PR handoff with explicit unavailable/expired content and no desktop-only action.
- [x] 11.6 Integrate AppState/NetInfo reauthentication, ordered transactional SQLCipher event/cursor catch-up, memory/query/download purge and account isolation; meet 320dp, 48dp-target, keyboard/safe-area, large-text and VoiceOver/TalkBack requirements.
- [x] 11.7 Resolve cold-start/login/foreground job and question notification links through current authorization and exact durable identity; stale or inaccessible links must not attach an unrelated session or perform a mutation.

## 12. Consumer-visible regression coverage

- [x] 12.1 Add server behavior regressions for concurrent admission/capacity, durable duplicate receipts and changed-payload conflicts, stale preview/epoch, one unresolved run, restart dispatch and missing stop evidence; cover failure windows rather than implementation wiring.
- [x] 12.2 Add authorization/policy regressions for project-share-only denial, third-party exact-machine grants, multi-host narrowing, malformed policy, all revocation routes, more-than-one-page enforcement and device-versus-account revocation.
- [x] 12.3 Add host/RPC regressions for uncertain prompt/answer admission, question defaults, child work after root yield, lease/deadline expiry, complete descendant stop and same-attempt re-exec; keep terminal host behavior covered.
- [x] 12.4 Add delivery regressions for untracked candidate content, hostile paths/Git metadata, exact-head check binding, failed/missing checks, lost PR replies, external branch changes and cancellation/publication races.
- [x] 12.5 Add non-UI persistence/protocol regressions for atomic event/cursor application, gaps/resync, actor-scoped receipt recovery, account/access cache isolation and private notification routing. Validate UI behavior through the actual surfaces in section 13 rather than new markup/default-value tests.

## 13. Integrated execution and surface verification

- [x] 13.1 Regenerate shared schemas/TypeScript and run the affected Rust/shared/protocol checks plus existing web/native lint, typecheck and test gates once integration is complete; verify both web lockfile graphs and the shipped CLI build path when their dependencies/build inputs change.
- [ ] 13.2 On an explicitly provisioned host, run the actual pinned OMP worker through a bounded coding task with delegated work and a multi-question interaction, proving no timeout default, exact answer confirmation, aggregate metering and genuine settlement.
- [ ] 13.3 Exercise real sandbox denials and resource exhaustion, then lost admission reply, server restart, daemon re-exec, server-disconnect lease expiry, cancel and runtime-grant revocation; observe one attempt and complete process/container termination without disturbing unrelated panes.
- [ ] 13.4 Run a failing and passing independent verification on immutable candidates and create/reconcile one real draft PR in an authorized test repository; prove lost-response recovery and that the worker cannot access publishing credentials.
- [ ] 13.5 Drive the actual web flow at 320/390/768px and desktop, and on a mobile browser with its keyboard and zoom; inspect touch targets, safe areas, focus, narrow diffs, offline/reload recovery and no page overflow, with visual evidence.
- [ ] 13.6 Drive actual iOS and Android development builds on devices/emulators through creation, multi-question answering, cancellation, review, background/reconnect, large text/screen readers and cold/stale notification links. Expo Android export is an additional packaging check, not acceptance evidence for interaction.
- [ ] 13.7 Exercise edge routing for `/projects/` job JSON and `/code/jobs/` pages, compatibility with old clients/terminal hosts, deletion with an offline recorded job host and disabled-admission rollback while preserving retained evidence.

## 14. Operational handoff after smoke proof

- [x] 14.1 Update the canonical `AGENTS.md` runbook and existing mobile operations documentation with qualified images/profiles, rootless quota/network/storage provisioning, scoped credentials, reported-spend semantics, retention, stop/recovery, feature gates and rollout/rollback. Record only exercised verification and keep actual production deployment as a separate authorized operation.

### Qualification status — 2026-11-08

Implementation and integrated checks are complete; operational qualification is
not. The remaining unchecked items are intentional gates, not inferred passes.
Both host and deployment admission remain disabled.

- Local proof: Rust workspace (761 tests), static musl CLI build, deterministic
  protocol regeneration, protocol/terminal tests, web lint/typecheck/715 tests and
  production build, native lint/typecheck/81 tests and Android export, clean
  npm/pnpm audits, and the canonical runbook check.
- Actual Chromium proof: 320/390/768/1440px layouts, narrow UTF-8 diff paging,
  offline question drafts, immediate composer reload recovery, policy editing,
  and revocation removing all project-scoped IndexedDB records. Local nginx
  routed pages and JSON correctly. Real server restarts preserved receipts and
  retained deletion evidence for a synthetic offline-host record without
  inventing stop evidence. Minimal legacy WebSocket authentication succeeded.
- Missing host prerequisites: approved immutable worker/verifier image inputs,
  distinct finite workspace filesystems, dedicated model/publisher credentials
  supplied through private file paths, and an authorized test repository.
  Rootless Podman info succeeded with isolated storage, but this does not prove
  resource/network/termination qualification. No actual worker, upstream model,
  independent verifier or real PR was run.
- Missing surface prerequisites: real mobile-browser keyboard/safe-area checks
  and iOS/Android development devices or emulators for the full interaction and
  accessibility matrix. Expo export and desktop viewport emulation do not
  satisfy these gates.
- Returning-host purge, historical terminal-host compatibility and runtime
  rollback remain part of the real staging-host qualification. No production
  deployment or profile enablement was performed.

### Authorized production rollout — 2026-11-09

Deployed server and web 26.11.0, with job admission still disabled. Backups are
under `/opt/apas/backups/deploy-20261109T030439Z`. Production Jobs UI, authenticated
job JSON, public routes and 21 referenced static assets passed; an existing
26.10.9 CLI terminal retained the same running PTY identity across server
restart. No project daemon was restarted or native build released.

The rollout also fixed sibling-package deployment: explicit npm link semantics,
the parent-directory Turbopack root, and the protocol runtime's patched
`fast-uri` lock entry. Clean npm/pnpm web audits and the separately installed
protocol runtime audit passed; protocol 36 tests and web 715 tests passed after
the fixes. Operational qualification remains incomplete; this deployment does
not mark any real-worker, device, returning-host purge or runtime-rollback gate
complete.

## 15. Project-scoped web agent workflow

- [x] 15.1 Reconcile the accepted project-scoped agent workflow with the existing OpenSpec change and component contracts.
- [x] 15.2 Make the project + menu offer Interactive agent and Autonomous agent on desktop and the separate mobile-browser project screen without changing existing interactive creation.
- [x] 15.3 Add project-scoped autonomous composition with the current project fixed and the current machine preferred only when eligible.
- [x] 15.4 Display autonomous jobs alongside interactive tabs with progress, questions, usage, verification, diff and draft-PR review inside the project.
- [x] 15.5 Show job status badges and preserve project-accessible results after a job view is hidden; keep hiding separate from cancellation.
- [x] 15.6 Explain missing job authority or worker readiness in the project workflow while retaining cross-project Jobs and Attention views.
- [x] 15.7 Exercise desktop and narrow project workflows in the actual browser, verify navigation and lifecycle boundaries, run affected existing checks, and update the canonical runbook after smoke proof.

### Project workspace smoke — 2026-11-09

The isolated rebuilt API and actual Chromium UI exercised the desktop and
separate mobile-browser project screens at 320/390/768/1440px. Project-fixed
composition preserved drafts and explicit blank targets, selected a fresh
eligible current host, and never substituted a different eligible host.
Synthetic preview/admission responses verified receipt-driven opening of the
exact job without leaving the project; no actual worker was admitted.

Progress, question navigation, review and bounded UTF-8 diff rendering stayed
in the project. Hiding persisted across reload, made no HTTP mutation, and
remained reopenable through project history. Desktop/mobile terminal instances
survived job navigation; an injected access-denied response removed job content
and labels without destroying the mobile terminal.

The real scoped API returned 50 then 2 retained project records even when the
global inbox's first 50 records all belonged to another project. Reloading an
opened older result initially lost its tab label/status; the fix passed the
same browser scenario and now has a cached-label/revocation regression.
The existing interactive add-pane request and incompatible-host policy rejection
were observed without launching a provider. Global Jobs/Attention still worked.

These are local browser/API integration checks, not real worker, verifier, PR,
physical-mobile or native-device qualification. At this smoke checkpoint the
project-workspace change had not been deployed. These checks do not complete any
of the seven earlier operational gates or enable job admission.

Final integration gates passed: 18 server job tests and server build; web
TypeScript, 715 tests across 82 files, ESLint with zero errors (three existing
warnings), and the production build; strict OpenSpec validation and the canonical
runbook check. Temporary local servers, credentials, synthetic records and
browser storage were removed after the smoke run.

### Project workspace production rollout — 2026-11-11

Deployed the server and matching web/protocol pair after clean npm and pinned
pnpm dependency audits, a release server build and the staged production web
build. Both complete web graphs and the separately installed protocol runtime
had zero vulnerability findings. The version remains 26.11.0 because the
commit-count input did not change; release identity is `20261111T052521Z`.
Private rollback artifacts are under
`/opt/apas/backups/deploy-20261111T052521Z`.

Eight public routes returned 200 without redirects; all 20 referenced static
assets loaded. Authenticated global/project job listings and target readiness
returned JSON. Anonymous and inaccessible project reads were denied with the
existing 403 `authentication_required` / `job_access_denied` contract.
Desktop and 390px mobile-browser checks verified the deployed chooser,
project-fixed composition, readiness explanation and project history without
navigating away from the project.

All nine prior project clients reconnected, including two on 26.10.9. The APAS
terminal remained running with the same PTY UUID across the server restart and
new web attachment. Recent browser and service error counts were zero.
No project daemon/provider was restarted and no job was created. Deployment job
policy remains absent/default-disabled; no worker profile or native build was
enabled/released. The seven earlier operational qualification gates remain open.

## 16. Per-project autonomous-agent overview

- [x] 16.1 Resolve the existing Overview/history paths and agree on one desktop/mobile surface with a bounded project-list description contract.
- [x] 16.2 Replace the retired team-gated Overview pseudo-pane with a visible workspace Overview on desktop and mobile, preserving interactive panes and existing desktop overview panels.
- [x] 16.3 Return full task descriptions with project-scoped list rows and show authoritative status, hidden/completed jobs and explicit older-page navigation without per-job detail requests.
- [x] 16.4 Exercise populated, empty, paginated and stale/revoked Overview states in the actual browser; run affected checks and record the verified behavior in the canonical runbook.

### Project overview smoke — 2026-11-11

The rebuilt isolated API and production-built web UI passed actual Chromium
checks on desktop and the separate mobile-browser project screen at 390px and
320px. An empty project was distinct from loading/offline state. Full multiline
Unicode instructions expanded without clipping. The scoped endpoint returned
50 then 2 project jobs while the global first page contained only another
project's jobs; hidden/completed results reopened inside the project. Browser
detail requests named only the deliberately opened job, not every overview row.

Desktop and mobile retained their actual xterm elements through overview/job
navigation. Captured navigation sent no pane input, removal, reboot, creation or
reattachment frames and no HTTP mutations. At 320px, page width and scroll width
were both 320px. Offline records were visibly cached; suspending the synthetic
account produced a real 403, signed it out, and left zero project job records
in IndexedDB and no job content on screen. No application JavaScript exception
was observed; request failures were navigation-aborted prefetches and the
deliberately disconnected network.

Verification passed: 18 server job tests and server build; 37 protocol tests;
web/protocol/native TypeScript; 715 web tests across 82 files; production web
build and lint with zero errors and three existing warnings. These checks use
synthetic records and a local transport fixture, not a live worker, physical
mobile device, verifier or publisher. No deployment or job enablement was
performed. The seven earlier operational qualification gates remain open.

### Production state — 2026-11-13

Server and web release `20261113T013925Z`, built at e02c0f7, has run in production since 2026-11-13 01:48 UTC. It carries this change together with the persistent-worker cutover of `unify-autonomous-workers`. Web release `20261113T193707Z` followed at 19:39 UTC. No job policy row exists and there is no worker, so admission and recurring execution stay disabled. No worker, verifier or publisher profile has been qualified, and no real job has run. The seven operational qualification gates above remain open.

### Amended on 2026-11-13

`refine-interactive-workspace` removed the chooser that task 15.2 added: the project `+` creates an interactive agent only, and autonomous work is created from **New autonomous worker** in the project's Overview or from `/code/workers`. It also put the Overview's interactive panels before autonomous work. `proposal.md`, section 9 of `design.md` and the requirement "Responsive web integrates autonomous work into each project" in `mobile-autonomous-jobs` were corrected in place. Task 15.2, its smoke and its rollout record stand as history of what was built and deployed then. No gate task changed.
