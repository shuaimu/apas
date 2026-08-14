## Why

`Reboot CLI` currently replaces the APAS project process that owns every agent pipe and terminal PTY, so all pane agents are interrupted even when the user only needs to recover the server connection or upgrade APAS. Terminal panes should keep running through transport recovery and CLI replacement, with APAS reattaching to the same live agent instead of merely starting a replacement with provider-specific resume behavior.

## What Changes

- Add a transport-only **Reconnect Server** operation that rebuilds the project CLI's server WebSocket without stopping, respawning, or changing any pane process.
- Host each supported terminal pane in a project-scoped persistent runtime that survives replacement of the APAS project CLI process.
- Let a restarted or upgraded APAS project CLI securely discover and adopt its existing terminal runtimes, restore input/resize routing, and replay bounded output produced while detached.
- Preserve terminal process identity and lifecycle across successful adoption, while retaining explicit restart-and-resume as a fallback when the prior runtime no longer exists.
- Keep full **Reboot CLI** for binary replacement, but make its disruptive/fallback behavior distinct from transport reconnect and visible to the user.
- Clean up persistent terminal runtimes when a pane is closed, a project is stopped or deleted, or adoption determines that a runtime is invalid or unauthorized.
- Limit initial preservation to unmanaged Claude, Codex, and OpenCode terminal panes; legacy structured agent panes remain owned by the project CLI and may restart during a full CLI reboot.

## Capabilities

### New Capabilities

- `cli-lifecycle-control`: Defines transport-only reconnect, full CLI replacement, user-visible lifecycle states, authorization, compatibility, and fallback behavior.

### Modified Capabilities

- `terminal-pane-continuity`: Extends terminal continuity from WebSocket outages to project CLI process replacement through persistent hosting, adoption, and bounded detached-output replay.
- `pane-toolbar`: Adds distinct reconnect/reboot controls and communicates whether a full reboot can preserve the active terminal runtime.

## Impact

- Affects the Rust project CLI lifecycle, daemon-spawned headless projects, terminal PTY ownership, local IPC, process cleanup, and self-update/re-exec paths.
- Extends shared CLI/server/web protocol messages and capabilities for reconnect/reboot requests and progress/results while remaining compatible with older clients.
- Updates the web toolbar and status presentation.
- Introduces a runtime dependency on a verified persistent-host mechanism (prefer the existing tmux installation on supported Unix hosts), with capability detection and a safe restart-and-resume fallback when unavailable.
- Requires failure-injection and integration coverage for active turns, detached output, duplicate adoption, stale runtimes, pane close, project stop/delete, daemon restart, and rolling upgrades.
