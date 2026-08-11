## 1. Shared Terminal Protocol

- [x] 1.1 Add the defaultable terminal lifecycle enum and optional terminal-instance metadata to the shared terminal output, exit, and snapshot messages.
- [x] 1.2 Add CLI-to-server and server-to-web terminal state-report messages carrying pane ID, instance ID, lifecycle, and optional status.
- [x] 1.3 Add shared serialization tests proving legacy messages without the new fields still deserialize as unknown and new messages round-trip all lifecycle metadata.

## 2. CLI Terminal Lifecycle Reporting

- [x] 2.1 Give each `TerminalHandle` a generated instance UUID and retained running/exited lifecycle record, and include the UUID in output and exit events.
- [x] 2.2 Make terminal spawn and reader-exit paths emit idempotent state reports while retaining enough status to report a process that already ended.
- [x] 2.3 Reconcile every configured terminal pane immediately after each WebSocket `SessionStart`, reporting live handles, ended handles, and configured panes whose restore produced no handle before queued output is drained.
- [x] 2.4 Add CLI tests for stable instance identity across transport reconnects, changed identity after respawn, exit reporting, and missing/failed terminal handles.

## 3. Server Retained Terminal State

- [x] 3.1 Replace the scrollback-only ring entry with a bounded terminal state entry containing bytes, sequence, truncation, instance ID, lifecycle, and optional exit status.
- [x] 3.2 Implement instance-aware state transitions that preserve a reconnecting instance, reset state for a replacement instance, and ignore delayed output or exit from stale instances.
- [x] 3.3 Change CLI disconnect cleanup to transition running terminals to disconnected without clearing bytes, while preserving exited state and keeping explicit pane-removal cleanup.
- [x] 3.4 Handle CLI state reports and lifecycle-bearing output/exit events, and fan authoritative live state changes out to attached web clients.
- [x] 3.5 Include instance ID, lifecycle, and exit status in every terminal attach snapshot, including snapshots with no output bytes.
- [x] 3.6 Add server tests for transient disconnect retention, same-instance reconnect, exit without viewers, exited-state preservation, replacement generations, stale events, bounded eviction, and legacy metadata.

## 4. Web Terminal Reattachment

- [x] 4.1 Extend terminal message decoding and the terminal event bus with optional instance IDs plus snapshot/live lifecycle state, defaulting missing metadata to unknown.
- [x] 4.2 Make `TerminalPane` track the current instance and last rendered sequence so same-instance snapshots do not duplicate replay and replacement or behind snapshots reset before cumulative replay.
- [x] 4.3 Replace reconnect-time exit clearing with lifecycle-authoritative banners for disconnected, unknown, and exited panes, including empty snapshots.
- [x] 4.4 Add web tests for legacy snapshots, same-instance reconnects, missed-frame replay, instance replacement, stale live events, and exited/disconnected banners without terminal bytes.

## 5. Documentation and Compatibility

- [x] 5.1 Update the terminal-pane architecture documentation to distinguish PTY lifetime from WebSocket lifetime and document reconciliation, lifecycle snapshots, and the non-durable retention boundary.
- [x] 5.2 Document and verify the server-first rolling deployment behavior for legacy CLI messages and the degradation path when lifecycle or instance metadata is absent.

## 6. Verification

- [x] 6.1 Run Rust formatting plus targeted shared, server, and client-cli tests for terminal protocol and lifecycle behavior.
- [x] 6.2 Run the web terminal unit tests, lint checks, and production build.
- [x] 6.3 Exercise a local end-to-end terminal by dropping only its CLI WebSocket, verifying disconnected replay, reconnecting the same PTY to running, then exiting it without a browser and reattaching to the exit banner.
- [x] 6.4 Run strict OpenSpec validation for `preserve-terminal-state-across-disconnects` and resolve every reported artifact or scenario error.
