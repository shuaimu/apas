## Why

A Claude pane's transcript was *derived* — the slug of the pane's cwd plus the
session id APAS minted — and both halves stop being true the moment the user
acts. Claude Code can move a session into one of its own worktrees, so the slug
names a directory the file was never in. Typing `/resume` onto another session
makes Claude append to *that* session's file, so the minted id names a
conversation the pane has left.

What covered the gap was a heuristic: follow the newest transcript in the
directory. It cannot distinguish our pane switching files from an unrelated
`claude` writing in the same directory, and on a live pane it adopted a
stranger's conversation — 607 records of someone else's work published as that
pane's history, while its real conversation sat unread in a worktree directory.

Claude Code already answers this question. It fires a `SessionStart` hook on
startup, resume, clear and compact, and hands it the absolute `transcript_path`
it is about to write.

## What Changes

- **Claude panes are spawned with a `SessionStart` hook** that records the
  transcript path the provider reports, and the watcher reads that instead of
  deriving it.
- **Pane identity travels in the environment**, so the hook can be identical for
  every pane and a `claude` a person runs by hand — which has no such variable —
  records nothing and can never be mistaken for a pane.
- **The hook only adds hooks.** Settings layers merge, so the pane keeps the
  model, effort and theme its owner configured.
- **Derivation stays as the fallback**, for an older provider or a hook that
  could not run. Nothing degrades below today's behaviour.
- **A reported change re-points the watcher immediately**, because a report is
  the provider stating the session changed — which no amount of watching the old
  file would reveal.

## Capabilities

### Modified Capabilities

- `terminal-pane-continuity`: a pane's transcript is identified by what the
  provider reports rather than inferred from its working directory and a pinned
  session id.

## Impact

- `crates/client-cli/src/claude_session_hook.rs`: the hook payload, where it is
  recorded, and the settings document.
- `crates/client-cli/src/main.rs`: a hidden `session-hook` subcommand, the hook's
  command.
- `crates/client-cli/src/terminal_pane.rs` and `pane_host.rs`: both spawn paths
  install it.
- `crates/client-cli/src/mode/dual_pane.rs`: the watcher prefers what was
  reported.
- Verified against Claude Code before building: the hook fires on `startup` and
  `resume` carrying `transcript_path`, `--settings` merges rather than replaces,
  and the hook process inherits the provider's environment.
