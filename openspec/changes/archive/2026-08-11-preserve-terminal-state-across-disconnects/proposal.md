## Why

A transient CLI WebSocket timeout currently erases terminal scrollback even when the APAS process and its PTY are still alive, so a reattached browser receives an empty snapshot and shows a blank pane. Terminal exit state is also delivered only as a live event, leaving later viewers unable to distinguish an idle terminal from one whose provider process has ended.

## What Changes

- Preserve each terminal pane's bounded in-memory scrollback when its CLI transport disconnects, while separately marking the pane as disconnected rather than assuming its PTY died.
- Track the last known terminal lifecycle state and exit status in server session state.
- Include lifecycle state and exit status in terminal snapshots so browser reconnects render running, disconnected, and exited panes accurately.
- Reconcile terminal state when a CLI reconnects or a new CLI process restores the project, preventing retained output from being mistaken for a confirmed-live PTY.
- Update the web terminal to retain the replayed screen and show an explicit disconnected or exited status instead of a silent blank pane.
- Add regression coverage for transport timeouts, reattachment, terminal exits without an attached browser, and restored terminal processes.

## Capabilities

### New Capabilities

- `terminal-pane-continuity`: Defines terminal scrollback retention, lifecycle reconciliation, and reconnect snapshot behavior across CLI and browser transport interruptions.

### Modified Capabilities

None.

## Impact

- Shared WebSocket terminal message schemas will carry lifecycle metadata in terminal snapshots and CLI reconciliation messages.
- Server session state and CLI disconnect/reconnect handling will distinguish transport connectivity from terminal process lifetime.
- The web terminal renderer will consume snapshot lifecycle state and display disconnected/exited status consistently after reattachment.
- Existing bounded, in-memory handling of raw PTY bytes remains in place; this change does not persist terminal contents to SQLite or JSONL and does not make terminal state durable across a server restart.
