## Why

APAS already models OpenCode as a provider, but ordinary user-created panes and mobile task launch are limited to Claude and Codex terminal hosts. Adding a verified OpenCode terminal path gives users a third interactive coding agent without reviving the retired conversation-only pane experience.

## What Changes

- Offer an OpenCode terminal profile in desktop, mobile, and administrator launch-policy catalogs.
- Launch the real OpenCode TUI with its documented automatic-permission, initial-prompt, and continuation flags.
- Recover OpenCode user/assistant conversation history, token usage, and completion state from directory-scoped session exports.
- Adapt OpenCode's documented JSON events for retained legacy managed/headless panes instead of assuming Claude-compatible output.
- Advertise a provider-specific CLI capability and fail closed during rolling upgrades when an older project CLI cannot host OpenCode terminals.
- Keep OpenCode terminal launches subject to the existing effective project launch policy and require OpenCode to be installed and authenticated on the project host.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `provider-support`: Define OpenCode as a supported user-created terminal provider across catalogs, policy enforcement, launch, and legacy JSON event adaptation.
- `terminal-pane-continuity`: Extend provider transcript recovery and terminal restoration behavior to OpenCode sessions.
- `mobile-terminal-access`: Allow mobile task launch to create OpenCode terminal panes only when the project CLI advertises explicit OpenCode terminal capability.

## Impact

- CLI: terminal PTY spawning, configured binary resolution, OpenCode session/export parsing, completion inference, and legacy headless event adaptation.
- Server/shared protocol: supported profile catalogs, mobile/web launch authorization, and a rolling-upgrade capability marker.
- Web and mobile: OpenCode appears wherever server-authoritative terminal launch profiles are allowed.
- Operations: project hosts need an installed/authenticated OpenCode CLI; existing explicit cluster/project allowlists must enable `terminal:opencode:official:default` before users can launch it.
