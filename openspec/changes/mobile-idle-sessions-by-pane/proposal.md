## Why

The mobile home's second list is "Idle projects", and a project counts as
working when *any* pane in it is working. So a project with one busy pane and
three idle ones is absent from the list entirely — the panes actually waiting
for you are the ones you cannot see. The busier the project, the more it hides.

Idleness is a property of an agent, not of a directory. What a person wants from
that list is "which agent is waiting for me", and the answer is per pane.

## What Changes

- **The list becomes "Idle sessions", one row per agent pane** rather than one
  per project. A pane names its project and host so it is placeable.
- **Usage-limited panes have their own subsection and status.** They are
  unavailable, not waiting for input, so they are grouped beneath ordinary
  idle panes and carry the provider's limiting window and reset time when known.
- **The most recently idle pane appears first.** The idle list follows the pane's
  latest working-to-idle transition rather than project or roster order.
- **Panes from a working project appear** when they are themselves idle, which
  is the case the project-level view could not express.
- **Opening a row lands on that pane**, not merely on its project, reusing the
  remembered-pane mechanism the session screen already reads on entry.
- **Returning from a conversation restores its source list.** Opening an agent
  from Idle sessions and going back returns to Idle sessions instead of
  resetting the mobile home to All projects.
- **The mobile payload gains a per-pane breakdown.** It currently carries one
  `is_working` per project, described as "at least one pane is working", which
  cannot answer a per-pane question.
- The "All projects" list is unchanged: whole projects are the right unit for
  "where do I want to go", and only the idle list is asking a per-agent question.

## Capabilities

### Modified Capabilities

- `mobile-code-sessions`: idleness is reported and listed per agent pane rather
  than per project, and opening an idle pane selects it.

## Impact

- `crates/shared/src/messages.rs`: a per-pane summary type, and a `panes` field
  on the mobile session summary. Defaulted, so an older server omitting it reads
  as "no pane detail" rather than failing to parse.
- `crates/server/src/routes/mobile.rs`: populate it from the panes the session
  manager already holds and the pane statuses it already tracks — the same
  source the project-level flag is derived from, so the two cannot disagree.
- `packages/web/src/components/mobile/MobileCodeHome.tsx`: the idle view renders
  ordinary panes first and provider-blocked panes in a Usage limited subsection;
  tapping either remembers the pane before navigating, while the mobile shell
  owns the selected home list across the conversation screen.
- `packages/web/src/lib/`: the remembered-pane helpers move out of the session
  screen so the home can write what the session screen reads, rather than the
  storage key being spelled in two places.
- No CLI change: pane rosters and statuses already reach the server.
