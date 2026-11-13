## Why

Mobile can answer "which agent is waiting for me"; the desktop cannot. Its
sidebar lists projects grouped by repository, and a project counts as working
when any pane in it is working — so a project with one busy agent and three idle
ones reads as busy, and the agents actually waiting are invisible. That is the
same defect the mobile list had before it moved to per-agent rows, and the
desktop is where most of this work happens.

## What Changes

- **The sidebar gains views** for "All projects" and "Idle sessions". Provider-
  blocked agents remain distinct in a "Usage limited" subsection beneath the
  ordinary idle rows instead of consuming a third top-level view.
- **The idle view lists one row per agent**, naming its project and host, and
  includes agents inside projects that are otherwise working.
- **Opening a row opens that agent**, not merely its project. The remembered tab
  is keyed by CLI client, which a caller cannot know before attaching, so the
  intent is carried through the store and consumed once the session is attached.
- **The web session list gains the per-pane detail** it was previously denied.
  When that detail was added for mobile the web path was stubbed empty on the
  grounds that the web learns its panes on its own channel — true for the
  attached project, and useless for a list spanning all of them.
- Both surfaces now derive the list from one server helper, so a pane cannot
  read as idle in one place and working in the other.
- Provider quota state remains separate from transient pane work. A pane whose
  provider is actively blocking requests is grouped below the idle agents and
  labelled usage limited, with its limiting window and reset time when known.
- Idle agents are ordered by when they most recently became idle, newest first,
  so the agent that just finished work or started waiting is easiest to find.

## Capabilities

### Modified Capabilities

- `web-project-workspace`: the project sidebar can list the idle agents across
  reachable projects and open one directly.

## Impact

- `crates/server/src/routes/ws_web.rs`: one `pane_summaries` helper, now used by
  the web list as well as mobile's.
- `crates/server/src/routes/mobile.rs`: uses the shared helper instead of its
  own copy.
- `packages/web/src/lib/store.ts`: `panes` on the session type, plus a pending
  pane selection an entry point can set before attaching.
- `packages/web/src/components/Sidebar.tsx`: the switch and the agent list.
- `crates/client-cli/src/usage.rs`: preserve the provider's active-limit and
  extra-usage result instead of inferring a block from a rounded meter.
- `packages/web/src/components/tabs/TabbedView.tsx`: honours a named agent over
  the remembered tab.
