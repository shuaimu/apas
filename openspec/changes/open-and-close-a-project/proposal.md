## Why

A project can be started from its workspace but not stopped there. The tab bar
offers "Boot" when nothing is running, and once it is running there is no way
back — stopping means leaving for the Machines page and finding the project in a
list of machines.

That asymmetry now costs more than a click. Idle agents are listed across every
running project, so a project left open contributes rows to that list
indefinitely, and the way to quiet it is the one action its own workspace does
not offer.

"Boot" is also the wrong word. It names the process APAS starts on the host,
not the thing the person is opening.

## What Changes

- **The Boot control becomes "Open"**, in the same place.
- **That slot becomes a toggle.** A project that is running offers "Close"
  instead, which shuts down every agent in it.
- **Closing says what it costs before it happens** — every agent stops, and the
  project stops appearing in the idle list until it is opened again.
- No new mechanism: closing reuses the project stop the Machines page already
  performs, against the same machine and project the Open control resolves.

## Capabilities

### Modified Capabilities

- `web-project-workspace`: a project can be opened and closed from its own
  workspace, not only started.

## Impact

- `packages/web/src/components/tabs/TabBar.tsx`: the renamed control and its
  closing counterpart.
- `packages/web/src/components/tabs/TabbedView.tsx`: the close handler, reusing
  the target the boot control already resolves.
- No server, protocol or CLI change.
