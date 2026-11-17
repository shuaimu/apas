## Why

Whether a project runs is decided machine by machine today: the Machines page
starts and stops one machine's instance, the project view starts or stops
every machine it can reach at that moment, and nothing remembers what was
asked. A project can therefore sit half open, with some tabs running and
others "Not running on zoo-006", and a machine that was offline or rebooted
when the project was opened never catches up.

The owner's decision (2026-11-16): open and closed are one state of the
project. While a project is open, every agent it owns runs; closed means none
does. The machine is not a place to make that choice.

## What Changes

- A project has one **open state**, kept durably by the server: open or
  closed. It is recorded when someone opens or closes the project, or, for a
  project nobody has recorded, open when the server first sees it running
  (stored as not recorded until then), and survives server restarts. A project nobody has recorded that nothing
  runs reads closed. A person sees Open or Closed and one button.
- **Opening** starts the project on every machine that owns tabs of it.
  **Closing** stops it on every machine. Both are remembered, so a machine
  that was offline, rebooted or still reconnecting follows when its daemon
  next registers: it starts an open project's tabs and stops a closed
  project that is still running there.
- **BREAKING** The Machines page loses its per-machine Start and Stop. Each
  project row shows the project's open state and that machine's part in it,
  and its Open and Close act on the whole project. A person can no longer
  run a project on one of two machines that both hold its tabs, or stop or
  restart one machine's instance alone. To keep a machine out, close or
  forget its tabs there.
- **BREAKING** "Start on \<host\>" for a tab whose machine is not running is
  replaced by opening the project again, which starts every machine that is
  not following, on the desktop and on the phone. The refusal for such a tab
  changes its wording.
- **Closing deletes nothing** (owner, 2026-11-16). A closed project keeps
  its tabs, their conversations and worktrees, its history, sharing, policy
  and saved prompts, and its registration on every machine, and stays listed
  as closed. Opening it again brings its tabs back as a project start does
  today. The Close confirmation says so.
- **Delete project becomes an explicit control.** Permanent deletion exists
  today only at the bottom of the project-access dialog, confirmed by typing
  the project's id. The owner gets a **Delete project…** control beside
  Close in the workspace and in the cluster page's project panel, with a
  dialog that says what is erased and what stays and is confirmed by typing
  the project's name. What deletion erases does not change: the server's
  record of the project, never a folder or a machine's registration of it.
- An open project names each machine that is not following it and why: it is
  offline, it runs an older APAS, the start was refused or failed, or the
  account that opened the project may not use that machine.
- Adding a tab on a machine that is not running the project opens the
  project, and the machine choice says so first.
- A request from a client that cannot say what opening does (an older web
  bundle's per-machine buttons, a native task launch) never starts or stops
  a machine other than the one it names; where that is not possible it is
  refused with the instruction to open or close the project.
- Suspending a project, an operator's "Stop runtime" and beginning a
  deletion close it. Reactivating starts nothing. Creating a project with a
  start records it open.
- An account can open or close a project when it may use one of the
  project's connected machines, or, when none is connected, when it owns or
  hosts the project. A machine is started or stopped later only where the
  account that opened or closed the project may still use it.
- A project nobody has opened or closed since the upgrade is recorded open
  the first time the server sees it running (its instance attaching), and
  reads closed while nothing runs it. The first release left such projects
  unrecorded and offered "Keep open"; the owner removed that on 2026-11-16
  for one Open/Close button (design Decision 2). Consequence accepted: a
  project running on one of its two tab-owner machines starts on the other
  at that machine's next registration.
- No daemon change is needed. The server sends the start and stop requests
  daemons already understand, now also when a daemon registers.
- Not changed: a machine's daemon still restarts its own running projects
  when it is replaced; tabs still never move between machines; a tab whose
  agent has exited stays exited until someone reboots it; there is no
  automatic retry of a machine that failed to start.

## Capabilities

### New Capabilities

- `project-open-state`: the project's durable open state, what opening and
  closing do on every machine, how machines follow it when they register,
  how each machine's part is reported, and what else opens or closes a
  project.

### Modified Capabilities

- `multi-machine-project-instances`: the project-view start and close rule
  leaves "Every pane belongs to exactly one machine" (it moves to the new
  capability) and the refusal for a tab on a machine that is not running is
  reworded; "Mixed fleets keep single-machine behaviour" drops per-machine
  start and stop as a separate decision; "A project runs at most one
  instance…", "Users choose a machine when creating a tab" and "Native task
  launches target the chosen machine" say open where they said start.
- `web-project-workspace`: "A project can be opened and closed from its own
  workspace" offers Open and Close by the project's open state, and offers
  both ways out of a project that is open but not running everywhere.
- `machine-lifecycle-control`: the machine list shows each project's open
  state and that machine's part, and opens and closes projects as a whole.
- `local-folder-project-creation`: "Registration does not start a project or
  its agents" and "Successful registration is ready for an explicit project
  start" name the registered project as closed and its control as Open.
- `omp-tab-autopilot`: "Autopilot is offered only where every part supports
  it" quotes the reworded refusal.
- `project-lifecycle-management`: "The web interface exposes
  role-appropriate lifecycle controls" names Delete project as a control of
  its own, where it is offered and what its dialog says.

## Impact

- **Server** (`crates/server`): five nullable columns on `projects` (the
  state, who set it and how, when, and the machine an open named); start and stop
  handling in `routes/ws_web.rs`; a follow step after a daemon registers in
  `routes/ws_daemon.rs`; `AddPane`, mobile task launch and the creation
  paths that start a project; suspension, stop-runtime and deletion; audit
  events for open and close.
- **Shared and protocol** (`crates/shared`, `packages/protocol`): optional
  fields on session and machine-project entries (the open state, and on
  machine entries that machine's part), one new server-to-web frame with
  every machine's part, sent only to viewers that negotiate a new
  capability, and regenerated schemas. No daemon or CLI frame changes.
- **Web** (`packages/web`): the project view's Open and Close and its
  not-following notice, the Delete project control and its dialog, the
  Machines page rows, the sidebar's and phone's
  Running/Stopped wording, tab host states and refusals, the new-tab machine
  choice, and the tests that pin per-machine Start and Stop.
- **Native** (`packages/mobile`): no control changes; it must keep parsing
  session entries with the new optional field; its task launch can now be
  refused for a closed project with tabs on another machine; the refusal
  sentence it composes itself is reworded for its next build.
- **CLI/daemon**: none. Older daemons take part as far as today's refusals
  allow.
- **Rollout**: server, then web. A rollback to a server older than release
  `20261116T224049Z` clears the recorded states while the server is stopped
  (design, Migration Plan): such a server ignores them, and they would go
  stale. A rollback to `20261116T224049Z` keeps them.
- **Docs**: `AGENTS.md` sections "A project's tabs can run on several
  machines", "The daemon is replaced only when asked" and the Machines view.
