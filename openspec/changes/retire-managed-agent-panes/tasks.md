## 1. Managed terminal panes (no behaviour change for existing teams)

- [ ] 1.1 Allow a terminal pane to be managed: stop forcing `managed: false` and let promotion accept them
- [ ] 1.2 Wire the delegation MCP server into the terminal spawn path for the providers that support it, as the agent path already does
- [ ] 1.3 Derive a working/idle signal for managed terminal panes from the provider's recorded turns, and report it where pane status is reported
- [ ] 1.4 Make diffs available for managed terminal panes — the computation is already git-based, only the gate is missing
- [ ] 1.5 Tests: a managed terminal pane is permitted, reaches the delegation tools, reports working and idle, and produces a diff from its worktree

## 2. Repeating work through the pane's session

- [ ] 2.1 Deliver an iteration into the running session rather than spawning a provider for it
- [ ] 2.2 Complete an iteration when the provider records the turn, and schedule the next after the interval
- [ ] 2.3 Do not deliver while the previous iteration is still working
- [ ] 2.4 Report a pane whose iteration is never recorded as stalled, without repeating the prompt
- [ ] 2.5 Tests: completion advances the loop, an unrecorded iteration stalls and reports once, and a busy pane is left alone

## 3. Move the roles

- [ ] 3.1 Spawn the four roles as terminal panes, keeping their prompts, worktrees and modes
- [ ] 3.2 Authorize team launches against terminal profiles
- [ ] 3.3 Report an existing managed pane of the retired kind as no longer runnable, rather than ignoring it
- [ ] 3.4 Tests: a team starts as terminal panes, a retired-kind pane is reported, and authorization refuses a disallowed terminal profile

## 4. Live run, then delete

- [ ] 4.1 Run a full team on a real project: goal to approved TODO to worktree to diff to review to PR
- [ ] 4.2 Remove the agent spawn path and its stream plumbing once nothing uses it
- [ ] 4.3 Remove the `agent:*` entries from the launch catalogue, and normalize stored policies that still name them
- [ ] 4.4 Update `CLAUDE.md`: the pane-kinds section, the team-mode sections, and the note that terminal panes get no team integrations
- [ ] 4.5 Workspace tests and clippy clean; web lint, type-check and tests clean
