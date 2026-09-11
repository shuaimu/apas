## Why

APAS offers Claude, Codex, and OpenCode as policy-controlled interactive terminal providers. Pi (`@earendil-works/pi-coding-agent`, binary `pi`) is a fourth minimal terminal coding harness whose sessions are exact, file-backed JSONL and whose extensions include the oh-my-pi orchestration layer. Adding Pi gives users another verified interactive agent and makes oh-my-pi usable inside APAS panes without reviving the retired conversation-only pane experience.

## What Changes

- Offer a Pi terminal profile in desktop, mobile, and administrator launch-policy catalogs.
- Host the real Pi TUI on the existing pane pty, pinning each pane to an exact Pi session id and restoring through the exact session file.
- Recover Pi user/assistant conversation history, completion state, token usage, and recorded cost from Pi's own session JSONL.
- Keep Pi structured questions out of the answerable conversation pipeline until a provider-confirmed question interface can be verified against the real TUI; assistant text still appears as ordinary conversation.
- Advertise a provider-specific CLI capability (`terminal_pi_v1`) and fail closed during rolling upgrades when an older project CLI cannot host Pi terminals.
- Support the oh-my-pi Pi package as an ordinary installed Pi extension: document global installation, config, and the project-trust boundary; verify its orchestrator prompt, session entries, and subagent output do not confuse Pi conversation recovery.
- Keep Pi terminal launches subject to the existing effective project launch policy and require Pi to be installed and authenticated on the project host.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `provider-support`: Define Pi as a supported user-created terminal provider across catalogs, policy enforcement, launch, and transcript recovery, alongside the existing supported providers.
- `terminal-pane-continuity`: Extend provider transcript recovery, completion state, usage, cost, and terminal restoration behavior to exact Pi session files.
- `mobile-terminal-access`: Allow mobile task launch to create Pi terminal panes only when the project CLI advertises explicit Pi terminal capability.

## Impact

- Shared protocol: a `Pi` provider variant, tab-type and launch-profile catalog entries, and a `terminal_pi_v1` capability marker.
- CLI: terminal binary resolution and `pi_path` configuration, pinned session-id launch and exact-file restore, session JSONL parsing, usage/cost recovery, and transcript watching.
- Server: launch authorization and mobile capability gating for Pi.
- Web and mobile: Pi appears wherever server-authoritative terminal launch profiles are allowed.
- Operations: project hosts need an installed and authenticated Pi CLI; existing explicit cluster/project allowlists must enable `terminal:pi:official:default` before users can launch it. The oh-my-pi package is installed into Pi, not managed by APAS.
