## 1. Shared Summary Protocol

- [x] 1.1 Add shared pane-summary records, window/status/availability enums, staged job types, and correlated result/error types with backward-compatible serde defaults.
- [x] 1.2 Add web-to-server list/refresh and server-to-web snapshot/update variants scoped by session and pane.
- [x] 1.3 Add server-to-CLI generation and CLI-to-server result variants carrying capability version, job identity, source digest, stage, and bounded content.
- [x] 1.4 Add `pane_work_summary_v1` capability negotiation and serialization/legacy-peer tests for server, web, and CLI message paths.

## 2. Windowing and Durable Storage

- [x] 2.1 Implement three-hour UTC boundary calculation and legacy/single-pane identity normalization with cross-pane isolation tests.
- [x] 2.2 Implement meaningful-record selection, deterministic ordering, bounded text extraction, tool/result compaction, secret-pattern redaction, and terminal-PTY exclusion.
- [x] 2.3 Implement canonical source hashing and late-message stale detection, including stable digest fixtures and post-GC behavior.
- [x] 2.4 Implement message-boundary chunking and staged-reduction manifests that stay within configured payload bounds and always retain the newest chunk.
- [x] 2.5 Add a versioned `pane-work-summaries.json` session sidecar with per-session locking, atomic replacement, status/error metadata, and restart recovery of in-progress states.
- [x] 2.6 Test sidecar corruption handling, concurrent update safety, survival across message GC, persistence after pane closure, and removal through project/session deletion.

## 3. Server Summary Service

- [x] 3.1 Add an `AppState` summary service with a deduplicated queue keyed by session, pane, window, digest, and stage.
- [x] 3.2 Implement bounded in-flight dispatch, one-job-per-CLI enforcement, timeouts, transient classification, three-attempt backoff, and permanent failure state.
- [x] 3.3 Implement startup and 15-minute reconciliation for completed windows, bounded session scanning, newest-first retained-history backfill, and source-expired reporting.
- [x] 3.4 Implement demand-driven current-window generation, source-through timestamps, refresh throttling, and final replacement when the window closes.
- [x] 3.5 Route staged jobs only to the session-owning CLI advertising the summary capability and validate every returned job/scope/digest before updating cache.
- [x] 3.6 Implement chunk-note collection, final reduction dispatch, summary whitespace/word-count/output validation, and one formatting-correction attempt.
- [x] 3.7 Implement authorized list and refresh handlers using current project access and active-project operation guards, including revocation/leave/delete race tests.
- [x] 3.8 Broadcast summary snapshots and incremental queued/generating/stale/complete/partial/failed/unavailable updates only to currently authorized attached web clients.
- [x] 3.9 Add structured logs and counters for scan duration/bytes, queue depth, generation latency, retries, failures, unavailable clients, provider/model, and cache growth without logging source content.

## 4. Isolated CLI Summarizer

- [x] 4.1 Add CLI configuration for summary enablement, verified adapter selection, optional model, timeout/input limits, and explicit cross-provider consent, defaulting generation and cross-provider transfer off.
- [x] 4.2 Add a summary-runner interface separate from pane spawn/resume code and advertise capability only after the configured adapter passes startup validation.
- [x] 4.3 Implement the initial Claude print-mode adapter in a fresh temporary directory with an empty tool set, no resume/session persistence, no project/worktree/MCP/plugin context, bounded input, and captured structured output.
- [x] 4.4 Implement sequential notes/final stage execution with one in-flight job per CLI, cancellation/timeout cleanup, and safe correlated success/unavailable/failure responses.
- [x] 4.5 Enforce same-provider generation by default and reject cross-provider jobs unless the local operator explicitly enabled transfer.
- [x] 4.6 Add fake-provider tests proving the adapter passes no resume ID, no project directory, no tool-capable flags, and never emits pane messages, status, terminal input, or terminal output.
- [x] 4.7 Add malicious-source and malformed-output tests proving the tool-free Claude adapter treats instructions as data and invalid/oversized responses are rejected safely.
- [x] 4.8 Implement an explicitly selected Codex headless adapter using a fresh empty directory, ephemeral execution, ignored user configuration/rules, read-only sandboxing, structured output, bounded stdin, no resume/project context, and startup flag validation.
- [x] 4.9 Add Codex prompt/argument tests, preserve one-job concurrency and pane-side-effect isolation, and document that the prompt and sandbox reduce but do not eliminate prompt-injection-driven host reads.

## 5. Desktop Web Experience

- [x] 5.1 Add Zustand summary state keyed by session/pane, list/refresh actions, capability gating, incremental update handling, and cleanup on logout/access loss/project switch.
- [x] 5.2 Build an accessible desktop `PaneWorkSummaryDrawer` with localized newest-first window cards and complete, partial-through-time, queued, generating, stale, failed/retry, update-required, unavailable, and source-expired states.
- [x] 5.3 Add the `Summary` action only for real active desktop panes and dock the drawer beside the conversation without unmounting the active terminal or message view.
- [x] 5.4 Switch drawer content atomically when the active pane changes, prevent sibling-pane card leakage, and hide the action on Overview and the no-project fallback.
- [x] 5.5 Add loading, empty, quota/provider error, manual retry, and current-window refresh behavior with duplicate-request throttling.
- [x] 5.6 Add component/store tests for per-pane scoping, local-time labels, current freshness, retry behavior, drawer layout, and mixed-version availability.
- [x] 5.7 Add regression tests proving responsive mobile and native mobile surfaces render no Summary action/drawer and send no summary-generation requests.

## 6. End-to-End Verification and Operations

- [x] 6.1 Add an integration harness with a fake capable CLI to verify retained messages become staged jobs, valid results persist, and authorized desktop updates arrive in order.
- [x] 6.2 Verify two simultaneous panes and multiple users cannot mix summary sources or receive updates after access revocation.
- [x] 6.3 Verify late messages invalidate retained-source summaries, current partial summaries become final, and completed summaries remain readable after raw-message GC.
- [x] 6.4 Verify high-volume windows remain bounded, reduce all accepted chunks, include newest activity, and fail explicitly instead of silently truncating required coverage.
- [x] 6.5 Verify an old server hides the web action, an old CLI continues normal pane work while cached summaries remain readable, and mismatched results cannot replace valid cache.
- [x] 6.6 Update operator/user documentation for summary semantics, seven-day backfill limits, terminal-history limits, quota usage, provider privacy, CLI configuration, enablement, and rollback.
- [x] 6.7 Run Rust formatting and the affected shared/server/CLI suites, web lint and full tests, protocol generation checks, both web dependency audits, and a production web build.
- [ ] 6.8 Deploy in server → web → CLI order to internal clients, explicitly enable the selected validated adapter (including Codex risk acceptance when applicable), and record acceptance results for generation latency, quota impact, scan I/O, cache size, authorization, project deletion, desktop layout, and unchanged mobile behavior.
