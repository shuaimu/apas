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
- On a shared filesystem, instances share one `.apas`. Pane writes become a
  locked merge that rewrites only the writer's own panes; project-level fields
  are never overwritten from a stale in-memory copy.
- Claims become per project **and machine**, are released when an instance
  stops, and still keep an older daemon from starting a project that a newer
  instance is running.
- The web tab bar shows which machine each tab runs on. The new-tab menu gains a
  machine choice: the project's running instances, machines where it is
  registered (the instance is started, then the tab added), and, for owned
  machines without it, **clone there and join**.
- `apas register` matches a checkout's git remote against the user's existing
  projects and offers to join one, so a separate checkout becomes another
  instance of the same project instead of a new project. `--project <id>`
  joins explicitly and `--new` keeps it separate.
- Older CLIs, daemons, web and mobile builds keep today's one-machine
  behaviour; nothing changes for a project until a capable instance runs it.

## Capabilities

### New Capabilities

- `multi-machine-project-instances`: per-machine instances of one project,
  pane ownership and routing, shared-`.apas` safety, machine choice for new
  tabs, joining checkouts by git remote, and clone-to-join.

### Modified Capabilities

None. Existing workspace, terminal-continuity, lifecycle and access
requirements continue to apply to each instance.

## Impact

- **Shared protocol:** optional `machine_id`/`hostname` on `PaneConfig`, a
  `project_instances_v1` capability, an optional target machine and request id
  on `AddPane` with a correlated result, instance hostnames on session
  summaries, and a join id on clone requests. Regenerated web/mobile schemas
  and protocol TypeScript.
- **Server:** per-machine attachments on a session; roster merge; routing,
  capability and runtime-access checks per pane machine; disconnect scoping;
  inbound pane-ownership validation; in-flight pane-id reservations; start-then
  -add orchestration; stop/delete across all instances; a
  `/projects/by-remote` lookup.
- **CLI/daemon:** machine id threaded into the project runtime; pane tagging,
  adoption and locked merge-save in `.apas`; restore of own panes only;
  cross-machine transcript exclusion; per-machine claims with release on stop;
  `apas register` join flow; clone-to-join provisioning.
- **Web/mobile:** host badges, machine picker, pending/failed tab launches, and
  machine resolution for multi-machine projects; native mobile shows host badges.
- **Rollout:** shared → server → web → mobile → CLI/daemon. No migration of
  existing `.apas` files is required; untagged panes are adopted on first run.
