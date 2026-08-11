## Context

See `proposal.md` for motivation and `specs/terminal-pane-continuity/spec.md` for required behavior.

Terminal panes currently stream opaque PTY bytes from the CLI to a server-side ring buffer keyed by `(session_id, pane_id)`. The buffer stores bytes, the newest sequence number, and a truncation flag. `TerminalExited` is broadcast only to browsers that are connected at that moment. On any CLI WebSocket cleanup, the server deletes the ring buffer because it assumes the CLI process and PTY died together.

A WebSocket timeout does not prove process death: the same APAS process can reconnect with its terminal child still alive. Conversely, a configured pane can remain visible after its provider process has exited. The protocol therefore needs an identity for each PTY process and retained lifecycle state independent of transport connectivity.

Raw terminal output must remain bounded and in memory. It must continue to bypass chat persistence and high-frequency application state, and the CLI, server, and web may run mixed versions during deployment.

## Goals / Non-Goals

**Goals:**

- Distinguish CLI transport connectivity from terminal process lifetime.
- Preserve the last useful terminal screen across transient transport loss.
- Make reattachment accurately show running, disconnected, unknown, or exited state.
- Reconcile the same PTY across reconnects and safely replace state for a new PTY in the same pane.
- Keep message changes compatible enough for a staged server, web, and CLI rollout.

**Non-Goals:**

- Persist raw PTY output or terminal lifecycle state across server restarts.
- Resume a provider process that has actually exited; pane reboot remains responsible for spawning or resuming it.
- Change transcript-derived conversation history or usage accounting.
- Diagnose or prevent the network/server stall that triggers a transport timeout.
- Route terminal bytes through Zustand or another durable browser store.

## Decisions

### 1. Give every spawned PTY an instance identifier

`TerminalHandle` will mint a UUID when it spawns a provider process and retain it for the handle's lifetime. Output, exit, and state-report messages will carry that identifier. Sequence numbers remain local to one instance and may restart at zero only when the instance identifier changes.

The server will accept an event only for the current instance of a pane. A state report for a different instance replaces the prior entry and clears its scrollback before the new instance produces output. Delayed output or exit events from an older instance cannot overwrite a replacement.

Alternatives considered:

- Treat a changed CLI connection ID as a new terminal. Rejected because one live PTY legitimately spans multiple WebSocket connections.
- Infer replacement from a sequence reset. Rejected because reconnect ordering and delayed messages make the inference ambiguous.

### 2. Store presentation and lifecycle in one server entry

The server's terminal map will hold a per-pane entry containing:

- bounded scrollback bytes;
- newest sequence and truncation state;
- optional terminal instance identifier;
- lifecycle: `unknown`, `running`, `disconnected`, or `exited`;
- optional exit status.

On CLI transport cleanup, entries that were running transition to disconnected and retain their bytes. Entries already exited remain exited. Pane removal explicitly deletes the entry. A report for a new process instance replaces it. State remains subject to the session manager's lifetime and the existing per-pane byte cap; it is not copied to SQLite or JSONL.

Alternatives considered:

- Keep only scrollback and derive lifecycle from session connectivity. Rejected because an exited pane and a live pane behind a broken connection are observably different.
- Persist entries to the database. Rejected because raw terminal screens can contain sensitive data, are high-volume, and are intentionally ephemeral.
- Clear disconnected entries after an arbitrary short grace period. Rejected because it recreates the blank-pane failure during longer but recoverable outages. Memory remains bounded per pane and entries have explicit lifecycle cleanup points.

### 3. Reconcile terminal state immediately after every CLI session start

After sending `SessionStart` and the pane roster on each WebSocket connection, the CLI will send a state report for every configured terminal pane before it drains queued terminal output. A report contains pane ID, instance ID when available, lifecycle, and exit/error status when applicable.

`TerminalHandle` will retain enough lifecycle information to report an exit even after its reader thread has finished. Reconciliation will also compare configured terminal pane metadata with the handle registry, so a restore failure or missing handle is reported as unavailable/exited rather than silently omitted.

State changes are also reported when a PTY is spawned or exits. Exit reporting becomes idempotent: an exit queued during a transport outage and a reconnect state report may both arrive, but the matching instance ID makes them the same transition.

Alternatives considered:

- Ask the CLI for state only when a browser attaches. Rejected because server state would remain misleading until a viewer appears and attach would regain a CLI round trip.
- Trigger a terminal redraw with resize or input after reconnect. Rejected because it mutates the TUI, does not work for dead processes, and still cannot communicate lifecycle state.

### 4. Extend terminal wire messages with rollout-safe metadata

The shared protocol will define a terminal lifecycle enum whose default is `unknown`. Existing terminal output and exit messages gain optional instance identifiers. Terminal snapshots gain optional instance ID, lifecycle, and exit status. A CLI-to-server terminal state report and corresponding server-to-web live state event will communicate reconciliation without requiring output.

New fields will deserialize with defaults so messages from older peers remain readable, while older JSON consumers ignore additional fields. A new server should be deployed before CLIs begin emitting the new state-report variant. If a new CLI reaches an older server during rollback, the older server may ignore the unknown state-report message while continuing to process the backward-compatible output and exit messages.

For legacy output without an instance ID, the server accepts the bytes but does not use them to prove identity across connections. A new connection treats such a terminal as unknown until output or other legacy behavior updates it; it must never be upgraded to confirmed running solely from retained data.

Alternatives considered:

- Make instance and lifecycle fields mandatory immediately. Rejected because CLI self-updates and web/server deployment are not atomic.
- Encode lifecycle in terminal bytes. Rejected because the byte channel must remain opaque PTY data.

### 5. Make browser replay instance- and sequence-aware

The terminal event bus continues to carry bytes directly to the mounted terminal component, but its events will also expose instance and lifecycle metadata. The component will retain the current instance ID, last rendered sequence, and displayed lifecycle in refs/state.

On snapshot:

- a different instance resets xterm before replay;
- the same instance at an already-rendered sequence updates lifecycle without duplicating replay;
- the same instance with missed output resets and replays the cumulative snapshot;
- a truncated snapshot resets before replay as it does today;
- an empty exited, disconnected, or unknown snapshot still renders a visible status banner.

A live state event updates the banner even when no bytes are emitted. The component will no longer clear an exit status merely because the browser WebSocket reconnected; snapshot or live lifecycle data is authoritative. Raw frames remain outside Zustand.

Alternatives considered:

- Preserve only the browser's existing xterm canvas and skip server snapshots after reconnect. Rejected because reloads and missed frames still require an authoritative replay source.
- Put lifecycle and bytes in Zustand. Rejected because full-screen terminal output would cause high-frequency global rerenders.

## Risks / Trade-offs

- **[Disconnected terminal entries remain in memory longer]** → Keep the existing byte cap, clear entries on pane removal or replacement, and continue using session-manager lifetime cleanup.
- **[Out-of-order events could regress a new pane to an old exit]** → Require matching terminal instance IDs for authoritative lifecycle transitions and ignore stale generations.
- **[Legacy CLIs cannot prove PTY identity across reconnects]** → Degrade to unknown/disconnected state without rejecting their output; full continuity begins after the CLI upgrade.
- **[A server restart still loses the screen]** → Preserve the intentional non-durable boundary and reconcile lifecycle from the CLI after restart; the UI must label missing state rather than imply a live blank terminal.
- **[Replay logic can duplicate or corrupt a full-screen TUI]** → Track instance plus sequence, reset before cumulative replay when behind, and add browser tests for same-instance and replacement snapshots.

## Migration Plan

1. Deploy the server changes so both legacy terminal messages and new lifecycle reports are accepted, while disconnects retain scrollback as disconnected.
2. Deploy the web changes so lifecycle-less snapshots default to unknown and lifecycle-aware snapshots render banners and generation-safe replay.
3. Release the CLI changes that attach instance IDs and send state reconciliation after every session start, spawn, and exit.
4. Verify with a live terminal by interrupting only its WebSocket transport, confirming retained replay and disconnected state, then allowing the same CLI process to reconnect and return to running.
5. Verify exit behavior by ending a terminal with no browser attached and attaching afterward to confirm the persisted in-memory exit banner.

Rollback can proceed in reverse. Rolling back the CLI removes authoritative reconciliation but leaves the server/web compatibility path. Rolling back the server after CLI rollout causes the new state-report variant to be ignored by the older server; existing output and exit flows continue using their legacy fields. No data migration or durable-state cleanup is required.
