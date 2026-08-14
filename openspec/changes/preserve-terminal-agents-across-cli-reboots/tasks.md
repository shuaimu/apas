## 1. Lifecycle Protocol and Server State

- [x] 1.1 Add shared request-ID-based reconnect/reboot operation, capability, preservation inventory, progress/result, and terminal-runtime reconciliation types with backward-compatible serde defaults.
- [x] 1.2 Extend Rust serialization tests and regenerate the web/mobile protocol schemas and TypeScript bindings without changing legacy `RebootCli` decoding.
- [x] 1.3 Implement authorized server routing for lifecycle requests, scoped to the currently owning project CLI, with request deduplication, bounded pending state, timeout outcomes, and access-revocation cleanup.
- [x] 1.4 Broadcast lifecycle capabilities, preservation inventory, and operation progress only to currently authorized project viewers, with mixed-version server tests proving reconnect never falls back to reboot.

## 2. Transport-Only Reconnect

- [x] 2.1 Add an in-process reconnect signal to the project CLI connection loop that deliberately closes only the current WebSocket, skips retry backoff, and leaves all pane state, queues, processes, watchers, and terminal handles untouched.
- [x] 2.2 Correlate reconnect acceptance and completion to the request ID, reporting success only after registration, session start, pane roster, and terminal lifecycle reconciliation complete.
- [x] 2.3 Add CLI/server integration tests for reconnect during terminal output, reconnect during a structured-agent turn, repeated/overlapping requests, connection failure, and unchanged pane process/instance identities.

## 3. Secure Persistent Pane Host

- [x] 3.1 Define the versioned length-delimited local pane-host protocol for create/adopt, lifecycle, sequenced output, input, resize, detach, acknowledgement, and shutdown, including compatibility and malformed-frame limits.
- [x] 3.2 Add host-local runtime-directory, descriptor, random credential, runtime/controller identity, and atomic handoff-marker helpers with `0700`/`0600` permission enforcement and no secret placement in argv, logs, `.apas`, or shared/NFS configuration.
- [x] 3.3 Implement the hidden `apas pane-host` mode for one terminal pane, owning its PTY, stable instance ID, provider process group, lifecycle, resize state, and bounded volatile whole-chunk output ring.
- [x] 3.4 Implement authenticated Unix-socket adoption with exact project/pane/runtime matching, peer-UID verification where supported, exclusive controller generations, idempotent same-controller retry, and stale-controller rejection.
- [x] 3.5 Add project-scoped tmux supervision helpers to create, discover, probe, and terminate per-pane host sessions, and advertise persistent hosting only when tmux, Unix sockets, permissions, and host protocol validation succeed.
- [x] 3.6 Implement detached/reboot lease timers and graceful process-group escalation so unexpected controller loss permits bounded adoption while authenticated pane/project shutdown is immediate and idempotent.
- [x] 3.7 Add unit and local integration tests for permissions, credential secrecy, identity mismatch, duplicate controllers, incompatible protocol, malformed input, ring eviction, host/provider exit, lease expiry, and complete process-tree cleanup.

## 4. Terminal Routing, Adoption, and Replay

- [x] 4.1 Refactor terminal routing behind CLI-owned and host-backed runtime implementations so input, resize, lifecycle, and output retain one server-facing behavior and the existing direct PTY remains a safe fallback.
- [x] 4.2 Launch new supported Claude, Codex, and OpenCode terminal panes through validated pane hosts, persist only non-secret runtime identity in host-local state, and report per-pane `live_adoptable` versus `restart_required_on_cli_reboot` inventory.
- [x] 4.3 On project CLI startup, validate `.apas` roster membership and adopt matching live hosts before attempting provider restoration; remove stale descriptors and never adopt a runtime for a missing pane or another host/project.
- [x] 4.4 Preserve the host's terminal instance ID and sequence across controller replacement, replay retained chunks before live output, expose oldest/current sequence plus truncation, and rely on same-instance server deduplication for retry and server-restart recovery.
- [x] 4.5 Fall back to provider-specific restart/continuation with a new instance ID when the runtime is missing, exited, incompatible, moved to another host, or persistent hosting is unavailable, and report the fallback rather than claiming live adoption.
- [x] 4.6 Add end-to-end tests that replace the project CLI during active Claude, Codex, and OpenCode terminal turns and verify unchanged provider PID/process group, ordered detached output, working input/resize after adoption, no duplicates, truncation reporting, and independent multi-pane adoption.

## 5. Full CLI Reboot Handoff

- [x] 5.1 Reorder self-update so fetch/build/validation/install completes while the current CLI and panes remain attached, and report preparation failure without setting shutdown or detaching runtimes.
- [x] 5.2 Persist the pane roster and atomic non-secret reboot handoff marker, grant host-backed panes the expected controller generation/deadline, flush handoff progress, and replace the CLI only after preparation succeeds.
- [x] 5.3 Make the replacement CLI validate and consume the marker, adopt each listed runtime before fallback restoration, reconcile the complete roster with the server, report success for the original request ID, and remove the marker.
- [x] 5.4 Handle exec failure, corrupt/expired markers, partial multi-pane adoption, and replacement timeout with explicit failure/recovery status while leaving host-backed providers protected by bounded leases.
- [x] 5.5 Preserve current restart/resume behavior for legacy structured panes and pre-feature CLI-owned terminals, but include accurate per-pane preservation counts and consequences before reboot confirmation.

## 6. Cleanup and Lifecycle Integration

- [x] 6.1 Update pane close/reboot/provider-switch paths to issue authenticated host shutdown when replacement is intended, terminate the provider subtree, and erase the socket, credential, descriptor, ring, and tmux session without touching sibling panes.
- [x] 6.2 Extend daemon project-stop and stale-runtime reconciliation to terminate every project pane-host session even when the project CLI is absent, while ordinary daemon restart continues adopting existing project CLIs and hosts.
- [x] 6.3 Integrate persistent-host tombstoning and cleanup with project suspension/deletion so adoption fails closed once cleanup begins and deletion cannot complete while a pane host or local runtime artifact remains.
- [x] 6.4 Add race/failure tests for close versus adoption, reboot versus stop, delete versus delayed controller, stale tmux/descriptor state, CLI crash, daemon crash/restart, and idempotent cleanup recovery.

## 7. Web Lifecycle Experience

- [x] 7.1 Add store state/actions for lifecycle capabilities, per-pane preservation inventory, request-ID progress/results, timeout handling, and cleanup on project detach or access revocation.
- [x] 7.2 Replace the direct ambiguous reboot confirmation with a compact project lifecycle menu offering capability-gated `Reconnect Server` and `Reboot CLI` actions without adding controls to the mobile or terminal view selector.
- [x] 7.3 Present reconnect as the recommended transport recovery, and make reboot confirmation enumerate adoptable terminals, terminals that will restart, and legacy structured panes that may resume.
- [x] 7.4 Add web tests for operation routing, authorization errors, old-CLI upgrade guidance, reconnect progress without pane-reboot labels, preservation-aware reboot confirmation, success/failure/timeout, and navigation while an operation is pending.

## 8. Verification, Documentation, and Rollout

- [x] 8.1 Add structured metrics/logs for reconnect duration, reboot preparation/handoff, adoption success/fallback, host version, detached runtime age/count, truncation, timeout, and cleanup failure without terminal content or credentials.
- [x] 8.2 Document lifecycle semantics, host-local paths and permissions, tmux/fallback prerequisites, adoption grace configuration, operational inspection/cleanup, mixed-version rollout, and rollback interruption risk in the canonical contributor/operator runbook.
- [x] 8.3 Run formatting, strict OpenSpec validation, full Rust tests, protocol generation checks, complete web tests/build, and dependency audits; resolve every regression before release.
- [ ] 8.4 Exercise a staging upgrade with active terminal and structured panes, forced WebSocket loss, failed update preparation, project stop, and project deletion; record process IDs and output sequences to prove preservation and cleanup.
- [ ] 8.5 Deploy server/web compatibility support before upgraded CLIs, verify health and mixed-version warnings, then enable persistent hosting and confirm no abandoned pane hosts remain after the observation window.
