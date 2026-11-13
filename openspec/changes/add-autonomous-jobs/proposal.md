## Why

OMP now supplies native background workers and a structured RPC lifecycle, but APAS has no durable, bounded unit of autonomous work that survives client disconnects and ends in independently verified, reviewable delivery. Restore that capability as APAS-owned jobs, not the retired role-team protocol, and make the complete workflow usable from both a phone browser and the native mobile app.

## What Changes

- Introduce one explicitly approved coding task per autonomous job: immutable instruction and acceptance criteria, canonical project/repository/base commit, selected host/profile, finite limits, durable run attempts, and an ordered event/evidence history. Saving a local draft never starts an agent. Admission requires the project owner or selected hosting operator, existing exact-machine runtime authority, and enabled effective policy; ordinary project users retain authorized read access without gaining unattended compute authority.
- Add an APAS-owned queue and fenced dispatch/reconciliation through the existing machine daemon. Preserve one execution owner per attempt across reconnect and deliberate daemon replacement. An uncertain or lost worker becomes recovery-required rather than an automatically duplicated run. Retry is explicit and bounded by the originally approved cumulative allowance.
- Add a versioned, non-PTY OMP RPC execution backend to the existing persistent host infrastructure, in a separate job-runtime namespace. Pin the worker image/binary and negotiate the required RPC/question/subagent/settlement capabilities. Existing terminal panes, their hosts and their restoration contracts remain unchanged; no `kind: agent` creation or managed-team machinery returns.
- Require an operationally verified rootless sandbox profile, private job-owned checkout and session storage, bounded resources, constrained network access and explicitly scoped model credentials. Never inherit host HOME, SSH/deployment/publication credentials or runtime sockets, and never fall back to an unrestricted host process. Runaway limits and authorization leases are enforced outside the model process.
- Support structured human questions, observable subagent progress, cumulative usage and limits, explicit cancellation, and interruption recovery without terminal keystrokes or model-chosen status-reporting MCP tools. Acknowledgement, recorded answer, session settlement and verified completion remain different states.
- Freeze candidate content, run configured checks independently in a clean restricted verifier environment, and publish only that verified commit to one reconciled draft GitHub PR through a separate trusted publisher. Preserve code/evidence on failure. Human review and merge stay in GitHub; this change adds no automatic merge or APAS-to-GitHub approval action.
- Make the existing project's `+` the responsive-web entry point for Interactive agent or Autonomous agent. Keep interactive provider creation unchanged; embed fixed-project autonomous composition, admission review, job tabs/status, progress/questions, limits/cancellation and diff/check/PR review in the project workspace. Retain global Jobs/Attention as cross-project overviews and the existing native workflow. Preserve mounted terminal panes, separate hiding a job view from cancelling execution, and keep completed results accessible in project history. Include keyboard-safe controls, small-screen layout, accessible target sizes, persisted account/project-scoped drafts and view preferences, durable same-request recovery, and explicit offline/synchronizing states.
- Add a visible per-project Overview on desktop and mobile web, independent of retired team flags or worker readiness. Show every autonomous job's full task description and authoritative status, including hidden and completed jobs through bounded project-scoped pagination, and open the existing detail view without disturbing interactive panes.
- Extend existing native notification/deep-link handling for opaque job/question/review destinations, with generic lock-screen text and authorization before displaying details. Browser background push is not added; browser users have an in-app attention view and stable URLs.
- Add separately typed, fail-closed autonomous-job policy and operational worker readiness. Existing terminal permissions and vestigial team/auto-merge flags never enable jobs. Roll out additively with jobs disabled until a host passes its readiness checks.

This is a bounded-task service, not an unapproved backlog crawler, automatic issue prioritizer, multi-repository campaign scheduler, or replacement for ordinary interactive panes. Tasks are entered or pasted into APAS explicitly; a GitHub issue link may be retained as attribution but does not authorize background polling or execution. No external orchestration service is introduced.

## Capabilities

### New Capabilities

- `autonomous-jobs`: approved immutable jobs, durable admission and event receipts, narrow authorization, scheduling, cancellation, retry and recovery semantics.
- `omp-autonomous-worker`: pinned native OMP RPC execution, real sandbox readiness, independent limits/leases, questions, delegation visibility and persistent-host lifecycle.
- `autonomous-job-delivery`: frozen candidates, independent verification, safe idempotent draft-PR publication and retained review evidence.
- `mobile-autonomous-jobs`: equivalent phone-browser and native job operation, accessible layouts, durable drafts, reconnect-safe decisions and review without a terminal.

### Modified Capabilities

- `project-policy-governance`: separately governed autonomous-job enablement, profiles and ceilings, monotone effective policy, and immediate job admission/publication/cancellation consequences.
- `mobile-code-sessions`: discoverable Jobs and Attention navigation while retaining the existing coding-session experience.
- `mobile-code-notifications`: privacy-safe, authorized job/question/review notifications and exact deep-link destinations.

## Impact

- Server/shared: additive job/run/event/mutation/question/artifact persistence; account-authenticated REST under the existing `/projects/` prefix; capability-gated events on existing WebSockets; daemon dispatch/inventory/stop contracts; lifecycle/revocation and policy integration; generated mobile/web DTOs and validators.
- CLI/host: extend existing daemon and persistent-host ownership with a distinct pipe-based job backend and job namespace; add rootless worker/verifier profiles, immutable repository export, a trusted publisher and positive stop evidence. Existing advisory NFS project claims are not a job execution lock.
- Web/mobile: Next.js `/code/jobs/...` routes and native Expo Jobs/detail/question/review routes, shared wire contracts with platform-specific rendering/storage, protected caches, accessible navigation and existing notification plumbing. No native WebView replacement for the new screens.
- Operations: a supported rootless runtime, approved immutable worker/verifier images, bounded host-local storage/network profiles, scoped model credentials and separate publication credentials are explicit prerequisites. The inspected host has Podman installed but its current rootless preflight fails on a misconfigured runroot; the new feature must report unavailable until this is fixed. Native interaction acceptance also requires a real device or emulator; the existing Android export command is not such a test.
- Verification: actual OMP/sandbox lifecycle and draft-PR smoke runs, authority/idempotency/recovery regressions, browser checks at 320px and common phone sizes, and native iOS/Android keyboard, suspend/reconnect, accessibility and review checks. Existing terminal/OMP-question work and all unrelated working-tree changes must be preserved.

## Applicability after persistent-worker cutover

`unify-autonomous-workers` supersedes this change's public standalone-task
admission, retry, composition and Jobs navigation. The bounded engine, immutable
job/run evidence, containment, questions, independent verification and trusted
draft delivery remain in use. New jobs are internal executions claimed under an
explicitly approved worker mandate and recurring allowance; they are not new
standalone user admissions. Earlier finite-job links and receipts remain
read-only, and instruction reuse opens an unapproved worker draft.

Worker inspection is a successful bounded `completed` outcome with no candidate
delivery. Worker implementation still requires this change's verifier and
publisher. Protocol v1 remains a legacy drain/history contract; worker-originated
execution requires `omp_persistent_worker_v1`, execution protocol v2 and fresh
bootstrap/result qualification. New worker socket variants require
`autonomous_workers_v1`. Missing recurring policy remains denied independently
of the existing finite-job enablement.

The successor's worker capabilities govern recurring selection, budget, lifecycle
and client behavior where this historical proposal described a task-only product.
The delta specifications below have been scoped accordingly; synchronize/archive
the related changes together rather than restoring standalone creation.
Completed implementation tasks and historical verification below remain evidence
of what was exercised at that time, not evidence that any of the seven remaining
real-host/device qualification gates passed. Bot conversion is not provided:
bot records stay read-only and fresh workers require separate approval.
