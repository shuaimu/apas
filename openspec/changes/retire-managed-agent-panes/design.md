## Context

See proposal.md — Why. What matters for the approach is which team-mode
machinery genuinely depends on the structured stream, which was worth checking
rather than assuming:

- **Diffs do not.** `compute_pane_diff` runs git against the pane's worktree.
  Terminal panes lack diffs by policy, not by necessity.
- **Delegation, the TODO queue and the scratchpad do not.** They are files
  (`team-todo.md`, `.apas-team.jsonl`), watched by the CLI, and reachable from
  either kind through the MCP server — which is already wired for Claude and
  Codex, though only on the agent spawn path today.
- **Worktrees do not.** A terminal pane already takes a worktree as its cwd.
- **The deadloop does.** An iteration spawns the provider headlessly, feeds it a
  prompt, and completes when the stream reports a result. This is the crux.
- **Pane status does.** "Working" is reported from stream events, and terminal
  panes deliberately report none.
- **Plan review does**, and is the one feature with no obvious terminal
  equivalent.

## Goals / Non-Goals

**Goals:**

- One production path for running a provider.
- The four roles keep their prompts, worktrees, delegation and review protocol;
  only the pane kind changes.
- No silent loss of a team that is mid-flight: an existing managed agent pane
  must be recognisable and reported, not quietly ignored.

**Non-Goals:**

- Changing the team protocol itself. The records, the TODO states, and who may
  approve what are untouched.
- Preserving plan review in its current form if it cannot be expressed without
  the structured stream — that is a decision to take deliberately, below.
- Migrating existing managed agent panes in place. They are re-created.

## Decisions

1. **A deadloop iteration writes its prompt into the live TUI and completes when
   the provider records the turn.** The transcript is the completion signal, and
   `read_turns` already reports which turns complete work. This is only viable
   now because the transcript a pane is writing is reported rather than guessed;
   with the old derivation, a deadloop could have driven a stranger's session.

   Considered keeping a headless spawn per iteration alongside the terminal pane.
   Rejected: two providers writing one conversation is the duplication problem in
   a worse form, and it keeps the second spawn path this change exists to delete.

2. **The prompt is written the way the conversation view writes one**, including
   the bracketed-paste framing for multi-line text. That path is already used
   from mobile and is understood, including its weakness — it is blind, and the
   provider may be mid-turn or in a menu. Which is why:

3. **An iteration that is not recorded within a grace period is reported, not
   retried.** The delivery-confirmation rule added for the conversation view
   applies here with more force: a deadloop that silently re-sends into a menu
   would loop forever doing nothing. Reporting a stalled worker is the honest
   failure, and matches the existing watchdog's nudge-once discipline.

4. **Status is derived from transcript activity**: a managed terminal pane is
   working while its transcript has grown more recently than its last completed
   turn. This is coarser than the stream's own signal and is the same source the
   conversation view already uses, so the two cannot disagree.

5. **Plan review is dropped for managed panes rather than approximated.** It
   depends on interrupting a structured turn at a decision point, which a TUI
   does not expose. Approximating it by pattern-matching the terminal would be
   the screen-scraping this codebase refuses elsewhere. If plan review is wanted,
   it should return as a protocol step in the scratchpad — an explicit record the
   human approves — rather than as a stream interception.

6. **The catalogue is emptied last.** Removing `agent:*` before the roles move
   would make team mode unstartable in the interim; removing it after leaves a
   window where the profiles exist but nothing uses them, which is harmless.

## Risks / Trade-offs

- [Driving a TUI is less deterministic than a headless spawn: a provider at a
  startup prompt, mid-compaction, or showing a menu will not take a prompt] →
  Decision 3 makes that visible instead of silent, and the roles are long-lived
  so a missed iteration is recoverable. This is the main behavioural risk and the
  reason for phasing.
- [Plan review is lost] → Named above as a deliberate removal, not an oversight.
  It should be reconsidered as a protocol step.
- [Existing teams break] → They are re-created rather than migrated, and the
  change reports a managed agent pane it can no longer run rather than pretending
  to run it.
- [Team mode is switched off everywhere today, so this ships largely unexercised]
  → Phase 4 is a live run of a full team on a real project before the agent path
  is deleted, because nothing else will find what a real Tech Lead does.

## Migration Plan

Phased, and each phase leaves a working system:

1. Managed terminal panes: allow them, wire MCP, derive status. Nothing changes
   for existing teams.
2. Terminal deadloop: drive iterations through the TUI, with stall reporting.
   Roles still spawn as agent panes.
3. Switch the roles to terminal panes, and change team authorization to check
   terminal profiles. Existing managed agent panes are reported as unrunnable.
4. Live run of a full team, then delete the agent spawn path and the `agent:*`
   catalogue entries.

Rollback before phase 4 is reverting the role kind; after phase 4 it is a binary
revert, since the agent path is gone.
