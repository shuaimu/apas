## Why

Autopilot exists only for OMP terminal tabs, and the owner asked for the same on Codex tabs. OMP's loop runs in an extension inside the OMP process; Codex has no in-process extension, so the loop needs another way to submit a prompt to a tab and to see its turns end. A provider check on codex 0.160.0 (2026-11-14, `~/.cache/apas-codex-check/NOTES.md`) found one that works: Codex's app-server control protocol.

## What Changes

- A Codex terminal tab can be put on autopilot from the same control, dialog, templates, lists and phone sheet as an OMP tab. The person-facing rules are the same: limits, Pause, Resume, Stop, Stop now, a person's interrupt pauses, three failed iterations pause, holds while APAS cannot control the tab, nothing is ever resent.
- APAS drives the tab's Codex conversation as a second client of its app-server: it starts each iteration with `turn/start` only while the conversation is idle, learns how the turn ended from `turn/completed` (completed, interrupted, failed), interrupts with `turn/interrupt`, and marks each iteration it submits with a client message id that the rollout keeps.
- The loop runs in the APAS project instance on the tab's machine, not in the tab. A restart of that instance (a daemon reboot) leaves the autopilot paused, never running.
- Codex's own goal mode is observed and treated like OMP's goal mode: autopilot holds while it is active and cannot start then. APAS never switches it.
- Iterations submitted by autopilot appear as iteration markers in the Conversation view, as on OMP tabs.
- Rejected: a Codex Stop hook as the engine. It works, but it continues inside one endless turn (no completion per iteration, no compaction between text-only iterations), cannot start an iteration from idle, spills prompts over about 10 KB to a world-readable file, and forces the tab off the shared app-server.
- Open, decided by task 1: whether APAS talks to the shared app-server daemon that Codex already runs for the user's tabs (no change to how tabs are launched), or gives each Codex tab a private app-server (which moves a running tab only at its next agent launch and meets a 60-second conversation lock).
- New capability markers keep older CLIs, servers and viewers unaffected.

## Capabilities

### New Capabilities
- `codex-tab-autopilot`: autopilot on Codex terminal tabs: how iterations are submitted and observed, when they hold, pause and end, how the tab is identified, and what is offered on which tab.

### Modified Capabilities
- `pane-toolbar`: the autopilot control is offered on Codex terminal tabs as well as OMP ones.

## Impact

- CLI (`crates/client-cli`): a Codex app-server client per project instance (the control socket APAS already queries in `transcript.rs`, or a per-tab server, per task 1), the loop itself, its private state file, and iteration markers in the Codex transcript reader.
- Server (`crates/server/src/session/autopilot.rs`, `routes/ws_cli.rs`, `routes/ws_web.rs`): accept autopilot state and route requests for Codex terminal tabs from instances that advertise the new capability; send them only to viewers that negotiated it.
- Shared types (`crates/shared/src/autopilot.rs`): new capability names, Codex-specific pause and support details; the wire frames are reused.
- Web (`packages/web`): the control, status line, lists and phone sheet on Codex tabs; the start dialog and templates unchanged.
- Docs: `AGENTS.md` (autopilot section), the Codex harness notes.
- No change to OMP autopilot, to native mobile, or to nginx.
