## Why

When a user switches conversations inside a Claude Code terminal pane (the in-TUI `/resume` picker, or `/new`), Claude starts writing to a different session file than the one APAS pinned with `--session-id`. APAS keeps reading the pinned file, so the web conversation view silently stops updating until the pane is rebooted. Codex and OpenCode terminal panes do not have this gap because their transcript lookup is already heuristic (newest matching session in the pane cwd).

## What Changes

- The claude branch of the terminal conversation poller follows in-TUI session switches: when the pinned transcript stops growing and a newer, unpinned session file appears in the same cwd slug directory, tracking moves to that file.
- The pinned-id set is the filter that makes this safe: every APAS-spawned claude session (agent panes and terminal panes) carries a pane-known `--session-id`, so a session file in the pane's cwd slug directory whose name is not any pane's pinned id is necessarily a human-created session from inside a TUI (or a manual claude run on the same machine).
- Switching back to the pinned session (or another unpinned one) is followed the same way; each switch re-baselines to the new source's end, exactly like the existing source-change handling, so no history is replayed.
- A conversation in flight is never abandoned mid-turn: the pinned file must be idle (size/mtime unchanged) across consecutive polls before a switch is considered.
- No wire protocol, web UI, or storage changes.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `terminal-pane-continuity`: Claude terminal conversation recovery gains a new requirement — the transcript watcher follows sessions the user switches to inside the Claude TUI, not only the APAS-pinned session.

## Impact

- `crates/client-cli/src/mode/dual_pane.rs`: claude branch of the terminal transcript poller gains the switch-detection logic (directory scan for unpinned session files, idle guard, source re-baseline).
- `crates/client-cli/src/transcript.rs`: helper to enumerate candidate session files in a cwd slug directory (unpinned, ordered by freshness).
- Tests: poller-level unit tests for the switch, the idle guard, pinned-set filtering, and switching back to the pinned session.
