## Context

See `proposal.md` for motivation and the seven delta specs for behavior. This document describes proposed implementation, not functionality already present.

The machine daemon in `crates/client-cli/src/mode/daemon.rs` already supervises projects. `pane_host.rs` already keeps provider processes alive across controller replacement, but its protocol and spawn paths are PTY-specific. The declarations in `supervisor.rs` are not an implemented alternative job service. The existing volatile resume manifest and advisory NFS project claims are not durable execution locks.

The server has transactional SQLite migrations, durable project-provisioning/mobile-launch receipts, canonical project/cluster authorization, lifecycle cleanup and a notification outbox. Its ordinary WebSocket mutation receipts and machine inventory are process-local. Existing PR creation pushes a pane branch and calls `gh pr create --fill`; it is neither independently verified, draft-only nor durable/idempotent enough for jobs.

Responsive web currently ties mobile navigation to home/session state. Native Expo has durable-storage and reconnect primitives, but its `(tabs)` layout is a stack and question rendering is not equivalent to the new OMP dialog shape. A connected socket does not establish completed per-job catch-up. The web viewport currently disables user scaling. These are concrete integration points, not reasons to add a second application or business API.

The tree contains unrelated ongoing changes, including terminal OMP-question support. Preserve them. This proposal does not deploy, start a provider, enable jobs or modify host configuration.

**Successor boundary:** after `unify-autonomous-workers`, the task-only public
admission/UI decisions in this historical design no longer select the product.
The job engine is the internal bounded executor for approved standing mandates.
Worker/item/reservation context is immutable on each execution; inspection has a
read-only checkout and can complete without delivery. Implementation retains the
candidate/verifier/publisher boundary below. Explicit worker-item retry preserves
item and rolling allowances rather than reopening standalone job admission.
The successor's contracts, v2 host readiness, lifecycle and worker surfaces take
precedence. This does not close this change's outstanding operational gates.

## Goals / Non-Goals

**Goals:** one clear ownership chain from approved task to exact candidate and draft PR; independently enforced execution limits; honest uncertain-state handling; the same server contract for browser and native clients; mobile operation without a terminal.

**Non-goals:** restore managed roles/team files/MCP reporting; attach RPC to a running TUI; make process-local OMP workpools into APAS's durable scheduler; automatically run another model after failure; support every provider or sandbox engine; automatically merge or submit GitHub reviews; synchronize unfinished composer drafts between devices.

## Decisions

### 1. One job owns its approved task; a run is an execution attempt

Use one `autonomous_jobs` resource containing the immutable task specification rather than adding a separate task/backlog service. The composer owns local drafts. Admission is the user's explicit approval; there is no additional approval-request queue. Read-only project members can inspect jobs but cannot spend unattended compute merely by being shared into a project.

Admission preview resolves the exact repository/base SHA, host/profile identities, policy digest and finite allowance. Return an expiring server-bound preview fingerprint. Confirming it rechecks those inputs; drift requires another review, never silent rebasing. The repo's dirty working copy is not the job's input: the initial snapshot comes from the selected committed base, and the preview says so. An issue URL is optional attribution only.

Admission requires the project owner or selected hosting operator **and** current exact-machine launch authority. Reuse the placement/membership logic behind machine-project launch, not the narrower exception allowing a shared user to interact with an already-running owner-hosted session. Job reads use project-content access deliberately, preserving authorized history on suspension. Answer/cancel controls use the initiator/owner/selected-operator rule from the spec plus current runtime/stop authority. Existing cluster/system policy administration rules do not change.

### 2. Persist control-plane state before side effects

Add an idempotent named migration through the existing `db/mod.rs` migration path. Keep job business operations in focused server DB/service/route modules rather than expanding terminal message handlers with a parallel state machine.

| Record | Essential content and invariant |
|---|---|
| `autonomous_jobs` | Project, creator/approver, immutable instruction/acceptance JSON and digest, repository/base ref/SHA, target owner/machine, pinned profile digests, approved cumulative limits, resolved policy/source revisions, state/version, current run, next event sequence, timestamps and cancellation reason |
| `autonomous_job_runs` | Unique `(job_id, attempt_no)`, execution epoch/host generation, dispatch ID, worker/image versions, lease, local receipt identity, phase, consumed allowance, candidate/verification/publication references, stop evidence; partial unique index allows only one unresolved execution attempt per job |
| `autonomous_job_mutations` | Unique `(actor_user_id, request_id)`, operation/input fingerprint, durable pending/result status; device ID is provenance, not ownership |
| `autonomous_job_events` | Primary key `(job_id, seq)`, unique event ID and worker `(run_id, epoch, source_seq)` identity, typed bounded payload, actor and timestamps; state transition and event append are one transaction |
| `autonomous_job_questions` | Run/epoch/dialog/tool identity, original question/options, status, winning answer mutation, delivery and provider-confirmation evidence |
| `autonomous_job_artifacts` | Authorized project/job/run manifest, kind, digest, bounded size, storage key and expiry; immutable candidate/check/publication manifests reference these records |
| Durable command outbox | Run/command identity, expected owner/machine/epoch, payload digest, delivery/acknowledgement status; use a job-specific table or the existing outbox pattern, not volatile socket receipt storage |

Do not persist secrets in these rows. Task instructions intentionally become protected durable project content, unlike the existing mobile launch receipt which omits its instruction. Use existing server data-volume protection and owner-only artifact files; do not claim new server-side encryption at rest. Native cache remains encrypted through existing SQLCipher/SecureStore facilities; browser persistence remains origin/account scoped, not secret from an authenticated browser profile.

Use SQLite transactions/CAS state versions to reserve queue capacity, assign epochs and append dispatch intent. Never hold a DB transaction over provider, Git or network operations. Default operational concurrency is one active run per project and machine, narrowed by cluster/deployment policy; queue ordering is FIFO within the admitted target, with an explicit queue limit. No machine failover is automatic.

Retain task, receipts, check summaries and PR identities until project deletion or explicit administrative retention policy. Default bounded event/log/diff retention is 30 days after terminal execution; the API declares expiry and preserves a summary/tombstone. Never expire ownership/stop evidence for unresolved executions. Logs/events default to a combined 100 MiB/run, with bounded individual frames and truncation manifests; workspace quotas are separate. Project deletion drains runtimes first, then removes artifacts and content records; retryable deletion bookkeeping outlives that cleanup.

### 3. Model the lifecycle without false completion

Normal path:

`queued → preparing → running ↔ needs_input → verifying → publishing → review_ready`

Failure/control paths:

- A prerequisite failure becomes `blocked` with the blocked phase and explicit permitted recovery action.
- A terminal execution/check failure becomes `failed` after positive stop evidence.
- Unknown ownership, prompt admission or answer delivery becomes `recovery_required`.
- Cancel/revocation/limit exhaustion fences new side effects, moves to `cancelling`, stops all owned execution, then becomes `cancelled` with a reason and retained partial results. Limit exhaustion is not success.
- Reconnect/adoption continues the same run. Explicit coding retry creates another run only after previous execution is proven absent; it starts from the same admitted base and task, not an implicit continuation of an uncertain transcript.
- Verification or publication recovery uses the retained candidate, not another model run. Review-ready is terminal for automatic execution. GitHub open/draft/closed/merged state is a separate publication field, refreshed on review/explicit refresh with bounded caching.

Default admission offers a 60-minute cumulative active-execution allowance, a $10 **reported-spend threshold**, and one attempt. These are editable only within explicit deployment/profile policy; the deployment itself remains disabled by default. Retry requires an originally approved attempt allowance greater than one and remaining cumulative budget. Queue time does not consume execution time; preparation, human-question waits, verification and publication do. Increasing the approved allowance or changing the instruction creates a new reviewed job.

A failed publication can be retried explicitly with its retained candidate while authorization, policy and remaining execution allowance permit. The publication operation is reconciled before repeat side effects. A cancellation racing an already-issued GitHub request may leave a draft PR: retain and show it, never promise to undo the request or automatically delete external work.

### 4. Extend persistent hosting, but isolate job descriptors from pane rosters

Use the existing daemon connection and persistent-host implementation, adding an explicit pipe-based `omp_rpc_job` execution backend and a versioned authenticated host protocol. Share lifecycle/authentication primitives where they genuinely apply; do not route NDJSON through `TerminalInput` or PTY output.

Place live job sockets/descriptors under a separate host-local `apas/jobs/<project>/<run>` runtime namespace, with private durable receipts/events/workspaces under provisioned `/var/lib/apas/users/<uid>/jobs/`. Job support requires that secure persistent root; there is no NFS or temporary-directory fallback for durable attempts. Existing pane-host descriptor parsing can delete unknown descriptors, and pane reconciliation deletes hosts absent the pane roster: job descriptors must never enter either directory walk. Existing terminal protocol/PTY instances remain compatible.

Each trusted job host owns a per-run local lock and fsynced launch receipt, the Podman container/cgroup identity, stdout RPC pipe, separate bounded stderr log, event spool, deadline, usage and authorization lease. The daemon authenticates/adopts it after re-exec. A daemon startup reports its job inventory even when ordinary project tasks are stopped. Do not start existing project panes merely to run a job.

New daemon capability: `omp_job_worker_v1`. Commands: prepare/inspect target, start run, renew lease, answer dialog, cancel run, reconcile inventory, verify candidate and publish candidate. Every command/result names project/run/owner/machine/epoch and an opaque command ID. Validate the current daemon connection and target binding, not machine UUID alone. Capability strings describe trusted operator-managed hosts; they are not remote attestation.

Reserve a local attempt before spawning; persist whether a prompt was not yet submitted, submitted with confirmation, or uncertain. There is no atomic transaction spanning fsync and OMP's input pipe. If that window is uncertain, reconcile the retained process/session; otherwise require recovery rather than claiming exactly-once model execution. No replacement starts while the prior unit could still exist.

Use a 120-second locally enforced lease, renewed every 20 seconds after server authorization. Re-exec preserves the existing deadline, not a fresh grace interval. The job host terminates its container on expiry and retains stop evidence. Server-side lease expiry alone is not evidence of termination. Host reboot requires boot-ID/container inventory reconciliation; a new boot on the recorded machine plus verified absence can establish the prior unit is gone. A wholly unreachable machine leaves cancellation/recovery unresolved rather than authorizing duplicate execution.

### 5. Pin a real sandbox and keep credentials outside repository code

Support Linux rootless Podman first. A selected immutable image/profile pins OMP, compiler/toolchain, verification tools and dependency strategy. Run non-root with private user/mount/PID/network namespaces, dropped capabilities, no-new-privileges, default seccomp, read-only image and bounded writable mounts. CPU, memory, PID and elapsed-time limits are enforced outside OMP; the whole container/cgroup is the stop unit, not just the root PID. Bounded durable writable storage requires a verified filesystem/container quota; absence of quota support makes the profile unavailable rather than pretending a periodic size check is a hard bound. Temporary directories use bounded tmpfs. No host networking, engine socket or privileged nested container escape hatch.

Prepare a standalone private clone/export at the admitted base with its own `.git`, no shared-object alternates and no linked-worktree administrative path. Do not give the worker the source repository's `.git` or mutable shared branch. Only job-owned working/session/artifact locations are writable. Build an explicit environment from scratch; do not inherit daemon login environment, host HOME, provider OAuth store, SSH or global Git/OMP settings. Repository-controlled extensions/configuration execute only within the sandbox and cannot widen pinned host policy.

Networking defaults to no external route. A dedicated per-job proxy bridge is the only permitted egress: a loopback helper inside the isolated namespace reaches an authenticated, run-scoped host-side endpoint. That endpoint is not a general host-control socket. It accepts only the profile's model API and explicitly approved dependency destinations, rejects arbitrary CONNECT targets, host/metadata/private-address escapes and redirect/DNS rebinding to denied addresses. Direct outbound traffic, including ignored proxy environment settings, remains blocked. The verifier has no model route and only the dependency access its pinned profile requires.

Use a dedicated model-credential profile, not a user's ambient interactive login. The trusted model gateway retains the upstream credential and accepts a short-lived, run-scoped token from OMP. The token is readable by worker tools but grants only that run's approved provider/model endpoints and remaining limits. Initial adapters cover explicitly qualified OpenAI Responses and Anthropic-compatible Messages profiles, not arbitrary OMP authentication modes. Reject unsupported or unmetered profiles. Validate request shape/model/output caps, cap concurrent upstream requests (default two), record aggregate usage including delegated calls, and refuse new calls at the reported-spend threshold. OMP's native cost/time limits are defense in depth; the gateway/host own accounting and stopping. Already in-flight responses can exceed the threshold: show observed/estimated usage and disclose this on admission, never call it a hard billing guarantee.

A publisher profile uses a separate repository-scoped GitHub App installation credential (or explicitly provisioned equivalently restricted credential) held only by the trusted publisher. Never mount it into the worker/verifier, emit it in logs or use ambient `gh` authentication. Require protected base branches and an operator-qualified CI policy for untrusted job branches/PRs: a draft PR can still trigger CI, so draft status is not a secret-protection boundary.

Readiness probes check the actual pinned image, rootless runtime, quota/cgroup enforcement, denial of forbidden filesystem/network access, model gateway/metering, credential provisioning and complete stop/recovery behavior. Publish a safe, timestamped typed report; expire it after five minutes and recheck launch-sensitive facts at dispatch. Binary presence is insufficient. The current host's observed `podman info` fails because its configured runroot targets `/run/user/1000` while the process UID is 3000; do not alter that global configuration implicitly.

### 6. Qualify native RPC, structured questions and delegation

Pin and qualify OMP 18.18.0 or a specifically tested later image, not whichever shared `omp` binary was most recently installed. Source reference: [versioned RPC contract](https://github.com/can1357/oh-my-pi/blob/v18.18.0/docs/rpc.md). Run its JSONL RPC tool-UI mode (`--mode rpc-ui`) without a PTY or `--no-ui`, negotiate the ready/protocol limits and call `set_ask_dialog { enabled: true }` before the initial prompt. This enables structured tool questions; it does not introduce a browser-to-OMP attachment endpoint. Keep framing/chunk bounds explicit and reject oversized or invalid protocol input rather than buffering without limit.

`prompt` acknowledgement is admission, `prompt_result` is a yield, and `session_settled`/`get_state.isSettled` describe absence of pending work. None is verification. Enable only the qualified native task/async settings in the private worker profile; RPC defaults do not guarantee they are enabled. APAS observes stable child identities and aggregate usage through native subscriptions. No extra Manager/Reviewer/Developer panes are created. A native task needing a human decision must return that need to the root for `ask`; direct headless child questions are not assumed to reach the parent UI. The delegated profile must demonstrate this contract before readiness is enabled.

There is a concrete timeout control: OMP 18.18.0 registers `ask.timeout` with default `0`; `askTimeoutMs` maps zero to no deadline ([settings source](https://github.com/can1357/oh-my-pi/blob/v18.18.0/packages/coding-agent/src/modes/settings.ts), [ask source](https://github.com/can1357/oh-my-pi/blob/v18.18.0/packages/coding-agent/src/tools/ask.ts)). Pin that value through the worker's highest-priority supported configuration and verify its effective behavior, rather than trusting a mutable repository setting. A timed dialog from a supposedly qualified profile is a protocol violation: fence/abort the run, do not accept a default or send `timedOut:true`. No automatic answer on phone disconnect. The host's overall job deadline still applies.

Persist each incoming dialog before notifying clients. APAS option IDs are derived within the immutable dialog; OMP 18.18.0 answers use exact option labels, so retain the original labels and map IDs back only for that dialog. Ambiguous duplicate labels or malformed IDs fail the dialog rather than guessing. Preserve multi-question order, multi-select and custom input. Reserve the first valid answer intent transactionally, then dispatch it once with durable identity. The matching native tool result/retained session evidence establishes provider confirmation; a pipe write or command receipt does not. An ambiguous delivery never becomes a fresh prompt or terminal keystroke replay.

The existing terminal `omp_activity.mjs` bridge stays terminal-specific. Its correlation and question presentation patterns are useful, but neither its TUI-only event path nor a model-chosen reporting MCP tool becomes the job control plane.

### 7. Freeze, independently verify, then publish one draft

After qualified settlement, stop worker mutation and capture a bounded safe file manifest, including intended untracked changes. Reject unsafe paths, `.git` metadata, escaping symlinks and unsupported special files. Explain exclusions; do not silently drop candidate source and claim success. Use a clean trusted Git context to build the exact candidate commit with the admitted base as parent, hooks/config/helpers disabled and no worker-provided remote. Preserve a content digest and commit SHA.

Run the admission-selected commands in a fresh verifier container against that exact candidate, with no worker session state, model token or publication key. Commands come from the approved profile; repository scripts are untrusted code executed inside that boundary. Store per-check argv/profile digest, image digest, exit code, timeout and bounded logs. Require every mandatory check. A clean test run is evidence, not a proof of semantic correctness or security.

Before publishing, recheck current permission, cancellation fence, candidate digest and checks. In a fresh publisher-owned Git context, push the exact commit to `apas/job-<job-uuid>` with expected-ref protection; reject any unexpected existing head rather than force-overwriting it. Create a draft PR with explicit repository/base/head and a body linking task, base and verification evidence. Reconcile GitHub by repository plus unique job branch before retrying a lost response. Record the publication transaction/PR identity durably. One job gets at most one PR; explicit publication retry reuses it and never reruns OMP. No mainline merge, branch-protection bypass or automatic GitHub approval API is implemented.

### 8. Share job APIs and wire types, not session identity

Use active-account REST for both web and native. All API paths stay under the already-proxied `/projects/` prefix; UI paths use `/code/jobs/` so they do not collide with nginx's project API routing.

| API | Purpose |
|---|---|
| `GET /projects/jobs` | Authorized cross-project Jobs/Attention page, cursor/status filters |
| `GET /projects/:project/jobs` | Authorized project overview, including full task descriptions, filtered before bounded cursor/status pagination |
| `GET /projects/:project/job-targets` | Eligible target/profile summaries, effective limits and safe unavailable reasons |
| `POST /projects/:project/job-previews` | Resolve reviewed immutable inputs without dispatch |
| `POST /projects/:project/jobs` | Confirm preview with durable request ID; return 202 and job/run receipt |
| `GET /projects/:project/jobs/:job` | Snapshot, state version, allowed actions and event watermark |
| `GET .../:job/runs`, `GET .../:job/events?after_seq=...` | Attempt history and ordered bounded replay |
| `POST .../:job/cancel`, `POST .../:job/retry` | Durable cancellation or explicit new coding attempt |
| `POST .../:job/questions/:question/answers` | First-writer answer intent with run/epoch/version/request identity |
| `GET .../:job/review`, `GET .../:job/artifacts/:artifact` | Authorized candidate/check/diff/publication evidence, paginated or bounded downloads |
| `POST .../:job/retry-publication` | Reconcile/retry retained verified delivery, never another coding run |
| `GET /projects/job-requests/:request` | Actor-scoped durable mutation lookup after an uncertain response |

Reuse existing JSON error conventions, adding stable job reason codes and correlated identities. Same actor/request/payload returns the original receipt; different payload is 409. Native HTTP mutations must enforce existing mobile coding flags plus the job gate, not rely only on WebSocket negotiation. No session ID, last-attached-session fallback or pane ID represents a job.

On existing web sockets add capability-gated subscribe/unsubscribe, snapshot, event, small inbox-changed hints and correlated rejection/resync messages under exact capability `autonomous_jobs_v1`. Establish a snapshot high-water mark and replay after it; dropped nonblocking fan-out is repaired by durable sequence, never by message text matching. Reauthorize subscription, replay, download and subsequent fan-out. Inbox hints cannot leak inaccessible job IDs.

Rust DTOs remain authoritative. Export REST-only job types through `MobileProtocolContract`, regenerate schemas/TypeScript, add job REST/event runtime validators and fixture categories, and add explicit web consumption of `@apas/protocol` with its build/lockfile integration. Do not maintain a second handwritten job DTO set in web or reinterpret session-based `CodeEvent` as the job journal. Existing mobile protocol clients retain their old capabilities and receive no unnegotiated job variants.

### 9. Implement phone-first rendering on both existing clients

Web routes: `/code/jobs`, `/code/jobs/new`, `/code/jobs/<job>`, `/code/jobs/<job>/questions/<question>` and `/code/jobs/<job>/review`. Reuse/factor the existing authentication and socket owner; nested routes must not open a second connection. Add Jobs and Attention to the current mobile home/navigation and a desktop entry. Preserve existing project/session selection instead of replacing the whole workspace.

The responsive-web project workspace is the primary creation path: its existing `+` first offers **Interactive agent** or **Autonomous agent**. Interactive selection retains the provider/worktree picker and pane creation contract. Autonomous selection embeds the existing composer with the stable project fixed, and prefers the current machine only when it is eligible and no saved or explicit target choice exists. An unavailable current machine is explained, never silently replaced. The task still requires immutable admission review and confirmation.

Project job tabs are a separate typed view family, never synthetic `PaneConfig` entries or numeric pane IDs. They render current phase/attention labels and reuse the existing job detail, question, verification/artifact and PR components through embedded navigation callbacks. Selecting a job hides interactive content without unmounting its terminal or changing the selected session. Hiding a job tab is a scoped layout preference, not cancellation; the existing explicit cancellation/confirmation controls remain inside job detail. Project history can reopen hidden and completed results, with bounded pagination. The global Jobs/Attention routes and their deep links remain supported cross-project views.

Add an authorized `GET /projects/:project/jobs` alongside admission POST, filtering durable rows by project before pagination. Its `ProjectJobListResponse` contains flattened `ProjectJobSummary` entries: the existing summary plus the full persisted task instruction as `description`. Derive descriptions from each already-loaded row, not the shortened title or an extra detail request. Global `JobListResponse` and stored job/detail contracts remain unchanged. Preserve current allowed-actions computation and cursor conventions. The browser's list cache and view preferences are scoped by account and stable project ID, purged on discovered access loss, and protected against stale async results during project/account switches. Do not obtain a project's history by filtering a bounded global inbox page. Missing authority, worker readiness or connectivity is explained without falling back to interactive execution or enabling an unsafe job action.

Both `TabbedView` and the separate mobile-browser `MobileSessionActivity` integrate the same project workspace; narrowing the desktop tab bar alone does not cover mobile. Opened results outside the current history page restore labels from their scoped snapshot, with stale badges until authorized detail/list synchronization. Detail synchronization updates its tab without fetching every archived job. Revocation clears those labels as well as the detail while leaving the interactive subtree mounted.

The project history surface becomes a visible **Overview** entry before the interactive/job tabs on desktop and mobile. It includes active, hidden and completed jobs without pretending that a bounded page is the whole inventory. Each row exposes the task description, confirmed state/attention, target and update time, with navigation to existing details. Long descriptions expand in place; cached rows are explicitly stale, and an old cached row without an instruction does not substitute its title. Do not fabricate progress or aggregate counts from the loaded subset. Preserve the desktop's existing interactive pane/policy/usage panels below autonomous work. Replace the retired team-gated numeric Overview pseudo-pane with the typed workspace view, migrate its saved selection, and keep terminals mounted during overview navigation.

Native routes: protected Jobs list plus job/new, job/detail, question and review screens, with public `/code/jobs/...` aliases resolving into the protected stack. Update the current stack-based navigation so Jobs and Attention are actually reachable. Use native list/form/code components, not a WebView. Reuse existing QueryClient, connection supervisor, secure credentials and SQLCipher lifecycle.

Both clients use the same allowed-actions/readiness DTOs and these screen structures:

- **Compose/review:** project and instruction, acceptance criteria, eligible host/profile, finite allowance, resolved committed base, concise privilege/cost warning, explicit Start job. Do not show a live running page before a durable receipt.
- **Detail:** concise current phase, target/run, latest confirmed update, budget/time cards, Questions and Review destinations, bounded progress/subagent timeline and a reachable Cancel control. Surface failure/recovery reasons without logs-first overload.
- **Question:** all questions and options, single/multi-select and custom text; saved answer draft; submitting/awaiting-provider/confirmed/stale states. Do not pre-submit a recommended choice.
- **Review:** task/base/head, file list, paginated unified diff, check summaries and expandable logs, draft PR status/link. No APAS merge/approval button.

Persist composer/answer draft plus request ID in an awaited transaction before sending. On uncertain delivery, query the original receipt and current job first; offer only an explicit same-ID retry, never automatically replay mutations. Transactionally persist accepted events and watermark before treating the selected job as synchronized. A connected socket with pending catch-up keeps controls disabled. Preserve scroll and bounded pagination across status updates.

Use account/project/job/run/question cache keys and clear memory, IndexedDB/SQLCipher, query caches and downloaded evidence on logout/discovered revocation. Offline cache is visibly stale and read-only; an offline device cannot know a server-side revocation until it reconnects, so do not claim instantaneous remote erasure.

Meet the mobile spec's 320-width, 44px web/48dp native, safe-area/keyboard, screen-reader and large-text requirements. Remove the web's global zoom prohibition while checking that existing terminal/session interactions still work. A sticky action region must respond to the actual visual viewport/keyboard; CSS desktop emulation alone does not prove this. Only code/diff regions can scroll horizontally.

### 10. Integrate lifecycle, policy and notifications at every mutation path

Add a shared job-authority-change service invoked by project leave/member removal in projects/share/cluster/admin, owner transfer, cluster machine restrictions, account suspension, lifecycle stop/delete and autonomous policy updates. Existing live revocation is uneven and one cluster policy broadcast is capped at 200 projects; the job service must enumerate every affected durable run and invalidate subscriptions independently of those gaps. Do not broaden this change into rewriting unrelated session permissions.

Policy is separately typed: deployment absent/false disables, omitted lower levels inherit, explicit malformed data denies; booleans AND, allowlists intersect and ceilings take minimum across every durable hosting placement. Persist a canonical digest plus source revisions. Recheck at admission, dispatch, renewals, retry and publication. Profile revocation or an exceeded narrowed allowance cancels; lowering concurrency prevents new dispatch without arbitrarily selecting an active victim. Device revocation removes that device, not the admitted account job. Admitting-account/runtime grant loss does cancel the job.

Reuse the notification outbox/preferences and its ordinary project owner/member recipient audience, filtered again for current access. Add generic job decision/failure/review/completion categories mapped to existing preferences and opaque exact-context links. No prompt/title/path/code in push. A tap resolves authentication and job state before navigation, never mutation. Web has in-app Attention and URLs, not an unimplemented browser push promise.

## Risks / Trade-offs

- **Native RPC behavior differs by OMP release** → pin image/protocol, configure no-timeout structured questions explicitly, exercise real root/child/settlement/cancel behavior before enabling readiness. Documentation/source inspection is not an executed compatibility test.
- **Rootless Podman currently fails on the inspected host** → provision/fix the host deliberately during implementation rollout; fail closed. Quota, cgroup and egress denial are separate readiness proofs, not consequences of installing Podman.
- **Exactly-once execution across an opaque input pipe is impossible to assert** → durable local intent, fencing and inventory minimize the window; unresolved admission requires human recovery rather than replay.
- **Leases trade availability for bounded unauthorized work** → a server outage longer than 120 seconds stops jobs, while ordinary phone disconnect does not. Show this distinction; do not extend leases indefinitely to conceal outages.
- **Cost is delayed and provider-dependent** → trusted aggregate gateway metering, finite time/output/concurrency, explicit reported-spend wording and refusal of unmetered profiles. No exact invoice guarantee.
- **Model/repository output is untrusted even after tests pass** → fresh verifier, safe immutable packaging, isolated publisher and human PR review. Repository CI must also be qualified; draft PRs can run automation.
- **Dedicated job model credentials require operator setup** → report unavailable instead of copying a user's interactive OAuth store or accepting arbitrary environment overrides from the browser.
- **Mobile verification infrastructure is not available on PATH here** → `adb`/`emulator` were not found; arrange real iOS/Android development builds/devices during apply. Expo export or mocked component tests do not satisfy interactive acceptance.

## Migration Plan

1. Add shared contracts, additive DB migration, default-disabled policy, durable API/events and server lifecycle enforcement. Preserve legacy clients and terminal variants.
2. Add the new daemon/host backend and secure persistent job namespace. Qualify pinned images, credentials, sandbox quotas/network/stop behavior and model metering on an explicitly provisioned host. Do not update currently running terminal providers.
3. Integrate web and native navigation, persistence, notifications and review with the shared contracts; keep creation gated until backend/worker readiness agrees. Regenerate protocol artifacts once through the established generator.
4. Exercise the full acceptance matrix: actual bounded OMP task and child work, question confirmation, lost admission response, server restart, daemon re-exec, network lease expiry, stale epoch, revoke/cancel/delete, failing and passing independent verification, lost PR response, and one real draft PR in an authorized test repository.
5. Verify actual browser layouts at 320/390/768px and desktop, mobile browser keyboard/zoom, and native iOS/Android device or emulator keyboard, accessibility, background/reconnect and deep links. Run relevant existing regression/type/lint/build gates after integration; do not substitute them for these smoke runs.
6. After smoke proof, update the canonical runbook and existing mobile operations documentation with job setup, credential/retention policy, recovery and rollout. Enable only the intended deployment/cluster/project profiles. Deployment remains a separate operational action under the runbook's server → web → CLI order; native release follows its existing signing/device process.
7. Rollback first disables new admission, then drains or positively cancels jobs. Retain a compatible daemon/host to stop or reconcile existing runs; do not let an older pane-roster cleanup enumerate job descriptors. Additive DB/evidence remains readable and no rollback silently kills unrelated terminal panes.
