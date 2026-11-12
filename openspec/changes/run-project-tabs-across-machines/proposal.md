## Why

A project runs on exactly one machine. Its `.apas` id is also its server
session id, the server's in-memory session holds one `cli_client_id`, and a
claim keyed by project id stops any second host from starting it. So every tab
in a project shares one machine's CPU, memory and toolchain, and a checkout of
the same repository on another machine is a different, unrelated project.

Two concrete cases motivate this:

- On the zoo cluster the home directory is NFS-shared, so every project is
  already registered on zoo-005 and zoo-006 with the same id and path. Running
  a heavy agent on zoo-006 next to the ones on zoo-005 is impossible today:
  zoo-006's daemon logs "skipping spawn" and does nothing.
- A laptop checkout of `chiral-network` registered as a separate project
  (`5762761c…`) with no relation to the zoo-005 project (`3d5ff717…`), and it
  appeared in a "(no remote)" group away from its repository.

## What Changes

- A project may run one **instance per machine** at the same time. Each
  instance is the ordinary project runtime inside that machine's daemon, owns
  the tabs tagged with its machine id, and attaches to the project's **single
  existing session**. History, sharing, policy and the sidebar row stay one per
  project.
- Every pane records the machine that runs it. The server merges each
  instance's roster into one project roster, routes every pane-scoped control
  to the owning instance, and refuses pane traffic from an instance that does
  not own the pane. One instance disconnecting affects only its own tabs.
  Panes written before this change belong to no machine until a capable
  instance reports them; until then they are shown, and routed, under the host
  they were last recorded on.
- On a shared filesystem, instances share one `.apas`. Pane writes become a
  locked merge that rewrites only the writer's own panes; project-level fields
  are never overwritten from a stale in-memory copy.
- Claims become per project **and machine**, are released when an instance
  stops, and still keep an older daemon from starting a project that a newer
  instance is running.
- The web tab bar shows which machine each tab runs on. The new-tab menu gains a
  machine choice: the project's running instances, machines where it is
  registered (the instance is started, then the tab added), and, for owned
  machines without it, **clone there and join**. Each launch gets a correlated
  result, and a lost one is shown as unconfirmed, never replayed. Native task
  launches target the chosen machine in the same way once a capable instance
  runs the project.
- The project view's Start starts an instance on every online machine that
  owns tabs, and its Close stops the project on every machine running it,
  including one whose instance is still starting or reconnecting; the Machines
  page's per-machine Start and Stop still act on one machine.
- `apas register` matches a checkout's git remote against the user's existing
  projects and offers to join one, so a separate checkout becomes another
  instance of the same project instead of a new project. It never
  re-identifies a checkout whose project has panes or server history.
  `--project <id>` joins explicitly and `--new` keeps it separate.
- Older CLIs, daemons, web and mobile builds keep today's one-machine
  behaviour; nothing changes for a project until a capable instance runs it.
  While an older CLI runs a project, starting it on another machine is refused
  with the update instruction before that machine starts anything. Older web
  and native builds never receive the new frames, and a capable daemon runs an
  instance beside another machine's only on a server that confirms support;
  on any other server it runs a project as an older daemon would, as the
  project's only runtime with every tab.

## Capabilities

### New Capabilities

- `multi-machine-project-instances`: per-machine instances of one project,
  pane ownership and routing, shared-`.apas` safety, machine choice for new
  tabs, joining checkouts by git remote, and clone-to-join.

### Modified Capabilities

None. Existing workspace, terminal-continuity, lifecycle and access
requirements continue to apply to each instance.

## Impact

- **Shared protocol (already committed):** optional `machine_id`/`hostname`
  on `PaneConfig` and `MobilePaneSummary`; the `project_instances_v1`
  capability for CLIs, daemons and viewers; `ServerToCli::scope()`, which
  classifies every CLI-bound message for routing; a target machine and request
  id on `AddPane` with the correlated `AddPaneResult`; `SessionInstanceInfo`
  in session summaries and the capability-gated `SessionInstances` frame; a
  target machine on reboot and lifecycle requests and on lifecycle reports;
  `all_instances` on project Start/Stop; the web's roster-only
  `ForgetMachinePanes` (no CLI-bound forget); `StartProjectCli.request_id`
  with `ProjectRuntimeStarted`; the capability echoed to daemons in
  `ServerToDaemon::Registered`; `daemon_connected`,
  `project_instances_available` and a canonical `git_remote` on machine
  entries; the join id on clone requests; the `ProjectsByRemoteResponse`
  types; `MutationKind::Input` refusals of typed input for viewers that
  negotiated the capability, on which every client settles the send for good;
  and the instance meaning of a native launch's machine. Regenerated
  web/mobile schemas, protocol TypeScript and fixtures.
- **Server:** per-machine attachments on a session; roster merge with adoption
  of untagged panes and attributed hosts; routing by scope, capability and
  runtime-access checks per pane machine; disconnect scoping; inbound
  pane-ownership validation; per-machine lifecycle inventories; the instance
  set for capable viewers; in-flight pane-id reservations; remembered ids of
  forgotten tabs; correlated refusals of input for a stopped or inaccessible
  machine's tab; start-then-add correlated with the daemon's start outcome;
  project-wide start/stop, with the stop also reaching a machine whose
  instance is starting or reconnecting, and stop/delete across all instances;
  refusing a start beside an older CLI before any daemon is asked; refusals
  that name a machine's host only to a requester who knows that machine;
  placing a project only a registration knows when its owner starts it; a
  `/projects/by-remote` lookup.
- **CLI/daemon:** machine id threaded into the project runtime; pane tagging,
  adoption and locked merge-save in `.apas`; restore of own panes only;
  cross-machine transcript exclusion; per-machine claims with release on stop;
  instances only on a server that echoes the capability, and otherwise a
  project's only runtime under an older daemon's claims, kept across a daemon
  replacement; a correlated report of every start outcome; each project's
  `git_remote` in the inventory; `apas register` join flow; clone-to-join
  provisioning.
- **Web/mobile:** host badges, machine picker, pending/failed/unconfirmed tab
  launches, project-wide Start/Close, forgetting a stopped machine's tabs,
  machine resolution for multi-machine projects, and a view that stays put
  when a secondary machine's instance attaches or leaves; native mobile shows
  host badges and targets launches at the chosen machine. Web and native show
  why a send was refused and never resend it by themselves.
- **Rollout:** shared → server → web → mobile → CLI/daemon. No migration of
  existing `.apas` files is required; untagged panes are adopted when a
  capable instance reports them.
