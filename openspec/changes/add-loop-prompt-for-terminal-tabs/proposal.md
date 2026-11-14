## Why

Autopilot runs only on OMP tabs, because only OMP lets APAS run the loop inside the tab and watch every turn. For Codex, the owner first chose a full autopilot over Codex's control socket (change `add-codex-tab-autopilot`, researched and checked on 2026-11-14), then on reflection asked for the simplest thing for Codex, Claude and OpenCode instead: a shortcut that sends one prompt asking the agent to keep looping by itself, with a plain warning that APAS cannot monitor it the way it monitors OMP. That change is withdrawn; its findings stay in `~/.cache/apas-codex-check/NOTES.md`.

The Codex research also found a defect in production: APAS's Codex conversation locator introduces itself to the user's shared Codex daemon under its own client name, which Codex treats as a new originating client. That rewrites the daemon-wide User-Agent of the person's own model requests and can change the daemon's login and analytics identity. The owner asked for it to be fixed properly.

## What Changes

- On Codex, Claude (including DeepSeek, which is the Claude terminal) and OpenCode terminal tabs, the pane toolbar and the phone's pane actions offer **Loop prompt**. Its dialog offers the same templates and the project's saved prompts as the autopilot dialog, shows the loop instruction APAS puts before the prompt, and sends the result once, as an ordinary message to the tab.
- The dialog says plainly that this is a shortcut, not autopilot: APAS does not run or watch the loop, cannot count rounds, enforce a limit, pause or stop it, and cannot tell whether the agent is still looping; it is stopped in the terminal.
- Nothing about OMP autopilot changes. A loop prompt creates no autopilot state, marker or list entry.
- The Codex locator introduces itself to the daemon the way Codex's own daemon tooling does for read-only queries, so it no longer changes the daemon's identity.
- `add-codex-tab-autopilot` is withdrawn (deleted from `openspec/changes`; it is in git history).

## Capabilities

### New Capabilities
- `terminal-loop-prompt`: the loop prompt shortcut on Codex, Claude and OpenCode terminal tabs.

### Modified Capabilities
- `pane-toolbar`: the toolbar offers the loop prompt on those tabs.
- `terminal-pane-continuity`: locating a Codex tab's conversation does not change the identity of the user's Codex daemon.

## Impact

- Web: the toolbar control, the phone pane action, the dialog (sharing the template and saved-prompt pieces of the autopilot dialog), and its tests. It sends through the existing terminal conversation input, which already confirms a message only when the provider records it.
- CLI: `crates/client-cli/src/transcript/codex_daemon.rs` (client name) and its tests.
- No server or protocol change; no new capability marker.
