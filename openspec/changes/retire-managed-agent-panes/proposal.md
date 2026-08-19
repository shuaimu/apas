## Why

`kind: "agent"` panes are retired for every purpose except one: managed team
roles. New unmanaged agent panes are already refused outright ("Conversation-only
panes are retired"), existing ones only run because the allowlist now guards
creation alone, and the deployment default was narrowed this week to three
terminal profiles. What remains is that the Manager, Tech Lead, Developer and
Reviewer are agent panes, so the `agent:*` launch profiles cannot be removed
without making team mode unstartable — they appear in the policy editor greyed
out, unusable and unexplained.

Keeping one production path alive for one feature is the cost. Agent panes are a
second way to run a provider: their own spawn path, their own stream parsing,
their own status, plan-review and diff plumbing, and their own failure modes,
maintained in parallel with the terminal path that everything else uses. Every
provider change has to be made twice, and the agent half is exercised only by a
feature that is currently switched off everywhere.

One thing that blocked this before no longer does. A terminal pane's turns are
recovered from the provider's transcript, and until this week that recovery was
guesswork — the wrong file could be adopted entirely. Claude now reports its
transcript through a `SessionStart` hook, so a terminal pane's turns can be
identified exactly, which is what a deadloop needs in order to know an iteration
finished.

## What Changes

- **Managed team roles run as terminal panes.** The four roles keep their
  prompts, worktrees, delegation and review protocol; only the pane kind beneath
  them changes.
- **Deadloop iteration is driven through the terminal.** Instead of spawning the
  provider headlessly per iteration and reading a structured result, the prompt
  is written into the pane's live TUI and the iteration completes when the
  provider records the turn.
- **Pane status for managed terminal panes is derived from the transcript**,
  which is what the Overview and the Tech Lead read to know a worker is busy.
- **Terminal panes may be managed.** Today they are forced `managed: false` and
  promotion refuses them.
- **The `agent:*` launch profiles are removed** from the catalogue, along with
  the agent spawn path, once nothing uses them.
- **BREAKING**: projects with existing managed agent panes must have their team
  re-created as terminal panes. Their history stays readable; the panes do not
  resume as agents.

## Capabilities

### Modified Capabilities

- `provider-support`: the supported catalogue loses its agent profiles, and the
  managed roles are terminal panes.
- `project-policy-governance`: team launches are checked against terminal
  profiles rather than agent ones.

## Impact

- `crates/client-cli/src/mode/dual_pane.rs`: the deadloop worker, pane status,
  managed-pane promotion, and the role spawn path.
- `crates/client-cli/src/terminal_pane.rs`: managed terminal panes, and MCP
  wiring for the delegation tools the roles use.
- `crates/server/src/routes/ws_web.rs`: team launch authorization, and the
  refusal that currently rejects managed agent panes.
- `crates/shared/src/messages.rs`: the launch profile catalogue.
- Not affected, contrary to expectation: pane diffs are computed from git by
  `compute_pane_diff`, not from the stream, so they work for terminal panes
  already; delegation, the TODO queue and the scratchpad are files; and terminal
  panes already accept a worktree as their working directory.
