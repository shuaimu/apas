## Context

See `proposal.md` for motivation. Today the project's `.apas` id is the
session id (`dual_pane.rs` sends `SessionStart { session_id: metadata.id,
project_id: Some(metadata.id), machine_id, .. }`), and the following all assume
one runtime per project:

- **Server session.** `SessionState.cli_client_id` is a single field.
  `create_cli_session` silently *replaces* the previous CLI when another sends
  `SessionStart` for the same session; the old socket stays trusted for
  inbound messages but receives nothing. `route_to_cli` sends every control to
  that one CLI (33 call sites in `ws_web.rs`, plus `mobile.rs` and
  `broadcast_project_policy`); `AnswerOmpQuestion` and pane work summaries read
  `cli_client_id` directly.
- **Roster.** A `PaneList` (or `SessionStart.panes`) replaces the whole cached
  roster via `set_session_panes`, which also prunes statuses for absent ids.
  `ReorderPanes` is server-side only; `.apas` stays sorted by id.
- **Disconnect.** `ws_cli` cleanup marks the DB row inactive
  (`clear_session_cli`), flips every running terminal in the session to
  `disconnected`, and `unregister_cli` clears every pane status.
- **Inbound.** `TerminalOutput/State/Exited`, `Output`, `StreamMessage`,
  `PaneStatus`, `PaneList`, `SessionEnd` and others are accepted from any CLI
  without checking it owns the session or the pane.
- **Access.** `check_session_runtime_access` evaluates one host machine per
  session (`session_machine_id`).
- **`.apas` writes.** `save_pane_configs` (`dual_pane.rs`) re-reads `.apas` and
  then replaces `panes` with this process's in-memory panes, with no lock; it
  runs on nearly every pane mutation. `update_project_operations` re-reads and
  changes one field. The 5 s flag rebroadcast calls `get_or_create_project`,
  which **deletes** a `.apas` that fails to parse and regenerates it.
- **Claims.** `claims/<project_id>.json` in the NFS-shared config dir.
  `start_project` silently skips a project another live host holds. Claims are
  never released on stop — `refresh_own_claims` keeps refreshing them while
  the daemon lives — so after zoo-005 stops a project, zoo-006 still cannot
  start it.
- **Machine id.** `derive_machine_id` is a v5 UUID of `user@hostname` unless
  `config.daemon.machine_id` is set in the shared `config.toml`, in which case
  every host has the same id. It reaches only `run_server_connection`;
  standalone `--headless` passes `None`.
- **Web.** One attached session; pane state keyed by pane id.
  `resolveMachineProjectTarget` returns `null` when one project id is on
  several machines, which breaks the project view's boot/Start/Close target.
  The clone flow (`CreateProjectInstance`) always mints a new id on the owner
  path and always creates a new branch.

The wire contract for this change is already committed (task group 1): the new
types and fields in `crates/shared/src/messages.rs` and
`crates/shared/src/mobile.rs`, whose doc comments state their semantics. The
decisions below describe the behaviour behind them.

## Goals / Non-Goals

**Goals:** run tabs of one project on several machines at once, on a shared
filesystem or separate checkouts; keep one session, one history, one sharing
and policy unit per project; never let one instance erase, hijack or disconnect
another's tabs; keep every older component working with today's behaviour.

**Non-goals:** moving a running tab between machines; syncing files or git
state between separate checkouts; per-machine history or sharing; multiple
instances on one machine; shared-cluster (guest) clone-to-join; changing native
mobile navigation beyond host badges; resolving OpenCode's existing same-cwd
ambiguity.

## Decisions

### 1. One session per project, one instance per machine

An **instance** is the project runtime for project P on machine M, running in
M's daemon exactly as projects do today. All instances send `SessionStart`
with `session_id = P`. The server keeps them as attachments of the one
session:

```
SessionState {
    instances: HashMap<MachineId, InstanceAttachment>,  // new
    cli_client_id: Option<Uuid>,  // retained: the primary attachment
    ..
}
InstanceAttachment { cli_id, hostname, working_dir, git_remote, capable, attached_at }
```

The **primary** attachment is the earliest still attached. It supplies the
legacy single-valued fields (`cli_client_id`, DB `hostname`/`working_dir`), the
default target for controls that name no machine, and the session summary
hostname. A second `SessionStart` from the *same* machine replaces that
machine's attachment, which is today's reconnect behaviour.

A CLI that reports no machine id has no key in that map: standalone and older
CLIs omit it, and the server's legacy host inference fails closed when it is
ambiguous. Such a CLI lacks the capability, so it can only be the session's
sole instance (Decision 5) and is therefore its primary. It is kept as the
session's only attachment, without a machine. Requests that name no machine
reach it, and so do controls for untagged panes, but no machine-addressed
request can target it.

Viewers see the attachments as `SessionInstanceInfo { machine_id, hostname,
working_dir, primary, runtime_access }`, in `SessionInfo.instances` and in the
`SessionInstances` frame (Decision 9). The machine-less attachment is listed
with `machine_id: None`, so a live session never looks as if nothing were
attached.

Every existing writer of those single-valued fields becomes primary-only or
instance-aware. Otherwise a secondary instance's `SessionStart` would flip the
session's host:

- the DB `create_session` upsert in the `SessionStart` handler, which also
  migrates shares by working dir and hostname;
- `set_session_machine`, where the last writer wins even with `None`, and
  which feeds `session_machine_id` and cluster machine enforcement (the
  attachment now carries its machine instead), and `set_session_project`;
- `attach_web_to_session`, which re-points `cli_client_id` to the DB row's
  CLI on every web attach;
- the legacy `StartSession` → `assign_cli_to_session`, which picks the first
  online CLI of any user.

**Alternative rejected:** a session per instance with the view merging
sessions. The web store keys pane state by pane id within one active session
(83 references, 18 components) and the native app does the same; re-keying
both by (session, pane) and splitting history and sharing per machine costs far
more than routing by pane on the server, where `route_to_cli` is already a
single choke point and pane ids are already allocated per session.

### 2. Panes record their machine; the server is authoritative

`PaneConfig` and `MobilePaneSummary` carry:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub machine_id: Option<Uuid>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub hostname: Option<String>,   // display; attributed host if untagged
```

A capable CLI reports only its own panes, each tagged with its machine id and
hostname, in `SessionStart.panes` and `PaneList` (`Some([])` when it has none),
and never reports another machine's panes. The server re-tags every pane it
accepts from a capable instance from the authenticated attachment regardless of
what the pane data says, so a CLI cannot claim another machine's panes. Panes
from a CLI without the capability get no machine id from the server; see
untagged panes below.

Roster merge, replacing `set_session_panes`' whole-list replacement. When
capable instance M reports its roster:

- `Some(list)` replaces every pane tagged M, so a pane tagged M that M no
  longer reports is gone. `Some([])` means "M has no panes". `None` changes
  nothing. This is the same empty-versus-unknown distinction the runbook
  requires at project level, applied per instance.
- A reported pane whose roster entry is untagged is adopted: the server tags it
  M. This is how adoption in the `.apas` (Decision 6) surfaces on the server.
- A reported pane whose roster entry is tagged for another machine is dropped
  and logged; server allocation makes this unreachable except through a bug.
- Every untagged pane M did not report is left alone.
- The server owns the global order. Existing panes keep their positions and new
  ones are appended; labels come from the cache. Status pruning is limited to
  M's removed panes.
- The persisted roster (`panes.json`) stores the tags and stamped hostnames, so
  stopped instances' tabs survive a server restart and render with their host.

A CLI without the capability is only ever the session's sole instance
(Decision 5), and nothing it reports is tagged. Its report:

- takes each pane it reports in place of that id's entry, untagged and
  attributed to its host, whatever the entry's tag: on a shared home such a
  CLI restores and runs every pane in the `.apas`, as before instances;
- removes the untagged panes it did not report, and the panes tagged with its
  own machine (the one its `SessionStart` named, if any) that it did not
  report;
- keeps every pane tagged for another machine that it did not report, listed
  as stopped on that machine with its id still allocated.

The last rule is for a separate checkout, whose tabs such a CLI cannot see.
An earlier draft let this report replace the whole roster, as before
instances. That erased a stopped checkout's tabs, leaving nothing to start or
forget until that machine ran the project again, and freed their ids for
another pane. The binding statement is the `PaneConfig::machine_id` doc
comment.

**Untagged panes.** A pane with `machine_id` None was written before
multi-machine instances. Ownership is only ever established by an instance
reporting the pane: when a capable instance M reports a pane whose roster entry
is untagged, M adopts it (the server tags it M). Panes reported by a CLI
without `project_instances_v1` are not tagged with a machine id, even if that
CLI sent a machine id, because tagging them would make them impossible for a
capable shared-home instance to adopt later; they only get their hostname
stamped.

The server stamps `hostname` on every untagged pane with the host it last
recorded the pane under: the reporting CLI's hostname or, for a persisted
pre-upgrade roster, the session's recorded host. That stamped hostname is the
pane's **attributed host**: a display and routing hint only, never written to
`machine_id` and never a reason to refuse or drop a pane that a capable
instance reports.

A pane-scoped control or close for an untagged pane is routed:

1. to the attached instance whose hostname equals the attributed host, if any
   (the earliest attached, should two share that name);
2. else to the sole attached instance if it is a CLI without the capability,
   which runs every untagged pane, as before instances;
3. else, if the pane has no attributed host, to the primary as a fallback;
4. otherwise it is refused as stopped on the attributed host: "<host> is not
   running this project. Start it on <host> first."

A tagged pane is never routed this way. Its controls go only to the instance
on its machine, even while a CLI without the capability is the sole instance:
that CLI reports every pane it runs untagged, so a tab still tagged for
another machine is one it does not run, such as a separate checkout's. With no
instance on that machine they are refused as stopped there. The "primary as a
fallback" in the `ServerToCliScope::Pane` doc comment covers a pane with no
attributed host (step 3) and a pane the roster does not list. With no instance
attached at all, nothing is refused as stopped: a control fails as it always
has.

Forget: `ForgetMachinePanes { X }` removes panes tagged X and untagged panes
whose attributed host equals X's hostname, from the server roster only. No
instance is told and no `.apas` changes: an instance sharing the file would
delete exactly the entries X restores its tabs from, so they could never come
back. The removed ids are remembered for X rather than freed, across server
restarts, in `sessions/<id>/forgotten-panes.json` beside `panes.json` (whose
shape older servers read, so it does not change). New tabs get other ids while
any other is free, because a newer tab holding one of those ids would make X's
returning tab look like another machine's pane, and the merge would drop it.
When X runs the project again it restores and reports its tabs, which come
back with their ids; an untagged one comes back with whichever instance
reports it. Either way the id stops being remembered. So UI copy must say the
tabs "come back if <host> runs this project again", never promise permanent
deletion.

**Closing an untagged pane.** A capable instance reports every pane it runs
tagged, so an untagged entry routed to it, by attributed host or to the
primary as a fallback, is one it does not have: a stale entry persisted before
instances, a pane inferred from history, or one closed before the instance's
first report. Its report leaves untagged entries alone, as it must for other
hosts' tabs, so nothing would ever remove such a tab. Once a close for it is
delivered to a capable instance, the server therefore drops the entry with its
status, retained terminal, in-flight reboot and pending decisions, and
remembers its id as it does a forgotten one: an untagged entry left in some
`.apas` comes back when an instance adopts it. If the instance did run the
pane after all, the close it received ends it there too.

For display an untagged pane is shown under its attributed host, with no host
badge when it has none. For an `all_instances` Start (Decision 9), an untagged
pane counts as owned by the online machine whose hostname equals its
attributed host.

An earlier draft gave untagged panes to the primary attachment. That breaks
when a separate checkout joins, and becomes primary, while the pre-upgrade
host is stopped: the checkout's first, empty roster would erase every tab the
host still has, with no stopped badge and nothing to forget.

### 3. Routing, capability and access by pane machine

`ServerToCli::scope()` returns a `ServerToCliScope` for every CLI-bound
message. It matches every variant without a wildcard arm, so a new variant does
not compile until someone classifies it, and a test pins each classification
against the variants read from the type's schema. `route_to_cli` routes by it:

- **`Pane(pane_id)`**: the attachment that owns the pane, and no other.
  `Input` with a pane (approve, reject and typed input all reach the CLI as
  `Input`), `TerminalInput`/`TerminalResize`, pause, resume, reboot, remove,
  interrupt, label, role, effort, model, review and manual modes,
  `RequestPaneDiff`, `CreatePr`, `StartBot`/`StopBot`, `AnswerOmpQuestion`,
  and `GeneratePaneWorkSummary` for its job's pane, because the summary runs
  under the pane host's own summary configuration. The legacy
  `PauseDeadloop`/`ResumeDeadloop` name no pane, but the CLI applies them only
  to `PANE_ID_DEADLOOP`, so they are `Pane(PANE_ID_DEADLOOP)`; sent to the
  primary they would act on a pane another machine may own. A pane tagged for
  a machine with no attachment is refused as stopped (Decision 9), even while
  a CLI without the capability is the sole instance. An untagged pane, or one
  the roster does not list, follows the routing in Decision 2.
- **`NewPane { machine_id }`**: `AddPane` goes to the instance on the
  `machine_id` the server assigned to the new pane's config, or to the primary
  when that is `None` (Decision 9). It carries the machine because the
  instance that receives the pane is the one that tags, persists and runs it.
- **`Project`**: every attached instance. `ProjectPolicy`,
  `UpdateProjectFlags`, `UpdateProjectOperations`, `RequestPaneList`, and
  `SessionDisconnected`, which is never sent today. There is no CLI-bound
  forget: forgetting a machine's tabs changes only the server roster
  (Decision 2).
- **`Process`**: one instance process. `RebootCli`, `CliLifecycleRequest` and
  `Signal` go to the machine the originating web request names in its optional
  `machine_id` (Decision 4), else the primary. The CLI-bound message carries no
  machine because the server picks it from the request.
- **`Unscoped`**: `AnswerQuestion`, `PlanReviewAnswer`, and `Input` without a
  pane. The server resolves the target pane from the pending decision, which
  it already tracks, and routes to that pane's owner; an answer whose decision
  it cannot resolve goes to the primary. `Input` with no pane and no pending
  decision goes to the owner of the legacy pane it means
  (`PaneConfig::pane_id_from_legacy(pane_type)`, else `PANE_ID_INTERACTIVE`,
  which is where a CLI applies input that names no pane), then to the primary.
- **`Connection`**: registration, version refusal, session assignment and
  rejection, and heartbeats. These are addressed to one CLI connection and
  never routed by session.

`SessionRejected` is a connection message, so the server sends it to each
attachment it rejects on that attachment's own connection. On membership
revocation it goes to every attachment the revocation applies to, which today
means every attachment whose CLI runs under the revoked account, not only to
the primary. Stop-runtime, suspension and deletion reject every attachment the
same way (Decision 9).

`AnswerOmpQuestion` and pane work-summary dispatch use the same
pane-owner lookup instead of reading `cli_client_id`. `session_supports_
capability` takes the target attachment. `has_session_runtime_access` is
evaluated against the pane's machine for pane mutations and the target machine
for `AddPane`. Attaching to a session requires runtime access to at least one
attached machine, or none attached, which is today's rule when there is one.
The same per-machine evaluation is reported to each viewer as
`SessionInstanceInfo.runtime_access` (Decision 9).

### 4. Inbound ownership and disconnect scoping

A pane-scoped message from a CLI is accepted only from an attached instance,
and for a pane tagged with a machine only from that machine's instance, even
when the sender is a sole CLI without the capability, which reports every pane
it runs untagged. Anything else is dropped with a warning. An untagged pane
belongs to no machine until an instance reports it, so any attached instance
may report for it. A pane the roster does not list yet is accepted, because a
new tab's first frames can arrive before the roster report that lists it,
unless its id is reserved for a new pane on another machine (Decision 9). This
closes the existing hole where a replaced CLI keeps writing into a session.

On disconnect of machine M's instance: remove M's attachment, move only M's
running terminals to `disconnected`, clear only M's pane statuses, and keep
M's panes in the roster. The DB row becomes inactive, `clear_session_cli`
runs, and `SessionAttached { has_active_cli: false }` is broadcast only when
the last attachment leaves; otherwise the primary moves to the next-earliest
attachment and the DB row is updated to it. `SessionEnd` from one instance
detaches it; only the last one ends the session. Machine `is_running`
enrichment reads every attachment, and capable viewers are sent the new
instance set on every attach, detach and primary handover (Decision 9).

Lifecycle inventories are kept per (session, machine). Each instance has its
own hosting capability and panes, so a per-session inventory would flip to
whichever instance reported last, and a reboot confirmation would count
another machine's panes. The server tags every `CliLifecycleInventory` and
`CliLifecycleStatus` it relays with the `machine_id` of the sending attachment
(`None` for an attachment that reported no machine, which is the session's only
instance), and after `SessionAttached` it replays one inventory per attached
instance. Each replayed inventory is read as it is queued (Decision 9), so one
relayed meanwhile is never overtaken by an older copy, and an instance that
detached meanwhile gets none. A status that carries an inventory replaces that
machine's entry. `RebootCli` and `CliLifecycleRequest` take an optional
`machine_id` that picks that machine's instance; without one they go to the
primary. Projects run inside their machine's daemon, so rebooting one instance
replaces that host's daemon and every project in it, and leaves the other
machines' instances running.

A request naming a machine that runs no instance of the session is refused,
never delivered to another machine. The refusal names that machine's host only
when the requester knows the machine: it runs an instance of the session, a
tab of the session is tagged for it, or the requester owns it or may use it
through cluster membership. Then it reads "<host> is not running this project.
Start it on <host> first.", with the host a tab tagged for it carries, which
survives a server restart, else its daemon's. Any other machine gets "This
tab's machine is not running this project.", exactly what an id no daemon ever
used gets, before and after that machine's daemon disconnects. Machine ids
derive from `user@host`, and the request proves access only to the session, so
naming the host of whatever id it carries would confirm to anyone that another
account runs a daemon on a guessed host.

A viewer without `project_instances_v1` keeps one inventory per session, and
its `RebootCli` names no machine, so it reaches the primary. Relaying every
machine's inventory to it would flip its inventory to whichever machine
reported last and make its reboot confirmation count another machine's panes.
It therefore receives only the primary's inventory: another machine's
`CliLifecycleStatus` reaches it without its inventory, and after a primary
handover it is sent the new primary's once that instance has reported one.

### 5. Mixed fleets degrade to one machine

Capable CLIs and daemons advertise `project_instances_v1`. A `SessionStart`
from a CLI without it keeps today's replace semantics when no other machine is
attached, and is rejected with `SessionRejected` ("running on <host>; update
this CLI") when another machine's instance is attached. Accepting it would let
its untagged full roster overwrite the others. In the other direction, a
capable `SessionStart` that arrives while a non-capable CLI from another
machine is attached is also rejected with a reason, as it is while the attached
non-capable CLI reported no machine, since nothing then shows it is on the same
machine. That can happen through standalone `--headless`, which takes no
claim. Replacing would orphan an old CLI that keeps writing into the session.

For the same reason, while such a CLI runs the project, a Start of the project
on another machine, per machine or project-wide, is refused before any daemon
is sent anything, with the text that machine's `SessionStart` would be refused
with: "This project is running on <host> with an APAS CLI that cannot share it
with other machines; stop it there, or reboot <host> from the Machines page to
update it, then start it here". Sent on, the daemon would start the project
and relaunch its saved tabs only for the server to refuse the instance, and a
Start is not correlated, so its requester would hear nothing. A Start of that
CLI's own machine goes ahead as before. That machine is recognized by the
machine id the CLI reported, else by its hostname; a CLI that reported neither
could be on any machine, so every Start is refused. A native task launch needs
no such check: while such a CLI runs the project, the task goes to its running
session and no machine is started.

The other way round, while an instance runs the project, a Start naming a
machine whose connected daemon lacks the capability is refused before any
daemon frame too, with the reason a new tab targeted at that machine gets:
"<host> runs an older APAS that cannot run this project beside other machines.
Reboot <host> to update it." Sent on, that daemon would skip the start without
a word on a shared home, where it finds the instance's claim, and with its own
checkout would start a CLI the server refuses when it attaches. The smoke run
found the silent skip. A project-wide Start's fan-out never includes such a
machine; when nothing qualifies and the Start falls back to the machine it
names, that machine is refused as a per-machine Start is. The check reads
attached instances only. So in the seconds before a running instance has
reattached, after a server restart or while its connection is re-established,
such a Start still reaches that daemon, which skips it as before. Counting
machines whose daemons report running the project would close that window but
would refuse legitimate Starts for up to a heartbeat after a real stop, since a
report lags the stop.

The server routes an `AddPane` naming a non-primary machine only when that
machine's daemon advertises the capability. Otherwise it returns a correlated
failure saying to reboot the machine to update it, rather than landing the tab
on the primary, a machine the user did not pick. The web offers the machine
choice only for machines whose machine-list entry reports
`project_instances_available`: the daemon is connected and advertises the
capability. `daemon_connected` separates the two reasons a machine is not
available. `MachineInfo::last_seen` is stamped on disconnect as well as on
connect and heartbeat, so it cannot tell an offline daemon from a connected
older one. Connected but not available means an older daemon, shown as
"Reboot <host> to update"; not connected means offline.

The daemon-bound additions are gated the same way. `StartProjectCli.request_id`
and `CreateProjectInstance.join_existing` are sent only to daemons advertising
the capability: an older daemon ignores the first and reports nothing beyond
its heartbeat, and ignores the second and, on the owner path, would branch,
mint an unrelated project and start it. On the viewer side, web and native
clients advertise the capability to receive the frames, and the
acknowledgement kind, that older native builds would reject (Decision 9).

A capable daemon also needs to know whether the server runs instances. A
server that implements them echoes `project_instances_v1` to a daemon that
advertised it, in `ServerToDaemon::Registered.negotiated_capabilities`; older
servers send no list. A daemon that does not find the capability there has
reached a server that keeps one CLI per session, where a second machine's
`SessionStart` replaces the first machine's instance and that instance's tabs
can no longer be controlled. For as long as that connection lasts it handles
every `StartProjectCli` as daemons did before instances. It refuses a project
another live host runs rather than starting an instance beside it. What it
does start, it runs as the project's only runtime, not as an instance: the
runtime restores every pane in `.apas`, other machines' included, reports and
saves them untagged, and does not advertise the capability, though its
`SessionStart` still names its machine and host. It claims the project as an
older daemon does (Decision 7). An instance would report only its own
machine's panes, and such a server replaces the whole roster with what its CLI
reports, so every tab another machine's instance had tagged would vanish from
it, and a new tab could take one of their ids.

The daemon decides again at every registration, because the server can be
upgraded or rolled back while the daemon runs, and it never stops or changes a
project already running because of this: each keeps running as it started. To
an upgraded server, a project's only runtime is a CLI without the capability,
so it stays the project's sole instance until it is stopped or its host is
rebooted from the Machines page; started again on that server, by a Start or
by the reboot's resume, the project runs as an instance.

A replacement daemon resumes its projects before it has heard from any server,
so the daemon it replaces writes into the resume manifest whether its server
kept one CLI per session. Where it did, the replacement starts each project as
its only runtime where no other host runs it, and otherwise as an instance, as
it ran there beside that host, under the ordinary claim rules. A restart after
a failed replacement decides the same way from the current connection. Neither
is ever refused because of what the server runs. An entry without that record,
from a daemon whose server confirmed support or from a manifest written before
the record existed, resumes as configured. The binding statement is the
`Registered` doc comment, whose "as daemons did before instances" covers the
pane ownership and the claims.

A host whose `config.daemon.machine_id` is set does not advertise the
capability. That id is shared through the NFS home, so it cannot distinguish
instances.

### 6. Shared `.apas`: locked merge of own panes only

Each instance knows its machine id from start; standalone `--headless` derives
it with `derive_machine_id` instead of passing `None`. It also knows from start
whether it is capable, which it is not when `config.daemon.machine_id` is set,
or when its daemon starts it as the project's only runtime because the server
keeps one CLI per session (Decision 5); a daemon can take the capability away
but never grant it. A standalone `--headless` run has no daemon connection to
learn that from, so `config.toml` alone decides for it. A runtime without the
capability restores every pane, reports its whole roster untagged, saves it
untagged, and assumes it is the project's only runtime, as today. All `.apas`
writes take an exclusive `fs2` lock on `<config dir>/locks/<project-id>.lock`,
then re-read, modify, write temp and rename. The registry lock already works
this way. The lock is not a sidecar in the project directory: repositories
ignore `.apas` by exact name (chiral-network's `.gitignore` does), so a
`.apas.lock` would show up in `git status` where an agent could commit it.
Instances that share a `.apas` share the NFS config dir and therefore the lock.
A separate checkout has its own `.apas` and needs no shared lock.

- **Pane saves** (`save_pane_configs` and every caller) replace only the panes
  whose `machine_id` is this machine, keep every other pane, and keep
  project-level fields as read from disk.
- **Project-level writes** (`update_project_operations`, the legacy `is_paused`
  writes, worktree assignment) change only their field under the same lock.
- **Adoption.** At instance start, under the lock and on a fresh read,
  untagged panes are tagged with this machine *only if* it is the earliest
  live claimant: no other machine's live instance claim is older than its
  own, compared by claim time and then by machine id, so two machines
  claiming in the same second get one answer between them. A host that runs
  the project alone (a live legacy claim without the instances marker) always
  rules adoption out, because it runs every untagged pane itself. A run with
  no claim of its own, such as a standalone `--headless`, has no start to
  order, so any other live instance rules it out. The claims module answers
  all of this from the per-machine claims (Decision 7). A new instance on a
  second machine while the first runs therefore starts with no panes. Order
  rather than presence decides because each daemon writes its claim before
  its instance reaches this check: two machines starting together each see
  the other's claim, and if presence alone ruled adoption out, neither would
  adopt, leaving the untagged tabs on no machine, listed under a host that
  does not run them, until every instance stopped. Adopting beside a later
  claimant takes nothing it runs, since a capable instance never runs an
  untagged pane. With one machine this tags the existing panes in place, the
  only on-disk change to existing projects. The server sees the adoption when
  the instance reports those panes (Decision 2).
- **Tags are permanent** (owner decision). An instance never restores or adopts
  a pane tagged for another machine, even when that machine is stopped. This
  is a deliberate change from today's shared-home behaviour, where starting
  the project on zoo-006 after stopping it on zoo-005 resumed zoo-005's tabs.
  Those tabs now stay listed as stopped on zoo-005 until zoo-005 runs the
  project again, and moving a tab between machines is not supported. Untagged
  entries are the exception: they belong to no machine until one adopts them,
  and a save by any runtime without the capability, such as an older daemon's
  or a capable daemon's only runtime (Decision 5), makes entries untagged
  again (below).
- **Restore** starts only this machine's panes, so `reconcile_project_hosts`
  never sees another machine's panes as its own. The instance reports exactly
  those panes, tagged, in `SessionStart.panes` and `PaneList`, and `Some([])`
  when it has none.
- **Forget** writes nothing. No instance is told (Decision 2), so X's entries
  stay in the shared `.apas`, and the other instances' pane saves keep them as
  they keep any other machine's panes. When X runs the project again it
  restores them, and its tabs come back.
- **Periodic reads** (5 s flags, reconnect policy snapshot) use a
  non-destructive reader that never deletes or regenerates `.apas`; a parse
  failure is logged and the last good copy kept.
- **Pane ids** are allocated by the server against the merged roster and its
  in-flight reservations (Decision 9), so ids are unique across machines, and
  they avoid the remembered ids of forgotten tabs (Decision 2).
  `--attach`'s local add checks every pane in `.apas`, not just its own.

Older binaries drop unknown fields on rewrite and would strip tags. That is
safe because of Decision 7: an older runtime can only run a project when no
capable instance does, and its untagged panes are re-adopted by the next
capable instance. A capable daemon's only runtime (Decision 5) saves every pane
untagged the same way and is bounded the same way: it claims the project as an
older daemon does, so no instance starts beside it.

### 7. Claims per machine, released on stop

A capable daemon claims `claims/<project>/<machine>.json` and releases it when
the instance stops, including stop-runtime, deletion and failure. To keep older
daemons out, it also holds the legacy `claims/<project>.json` whenever no other
live holder exists. The record carries `"instances": true`; older readers
ignore the field and see a live peer. Capable daemons treat a live legacy claim
marked `instances: true` as compatible and one without the marker as
exclusive. The legacy claim is refreshed only by its holder; when its holder's
instance stops it is released, and any other running capable instance takes it
on its next refresh. `refresh_own_claims` refreshes only claims for instances
this daemon is running. That fixes today's stop-but-still-claimed bug.

A start that a capable daemon runs as the project's only runtime, because its
server keeps one CLI per session (Decision 5), is refused while any other host
has a live claim, marked or not, and otherwise claims as an older daemon does:
the legacy claim alone, without the marker, after removing any instance claim
an earlier run on this machine left. That runtime runs other machines' panes
too, so capable daemons must treat it as exclusive even once their own server
runs instances: an instance started beside it would restore panes it already
runs before the server refused that instance's session. The heartbeat
refreshes each running project's claims in the shape its start took.

Claim records are written temp-and-rename with host- and pid-qualified temp
names; the current `.tmp.<pid>` name can collide across hosts.

Releasing on stop makes one host's removal of a claim the usual state just
before another host writes it again, and an NFS client caches an absence for
as long as it trusts its cached attributes of the directory, 30 s on the shared
home. A claim is therefore believed absent only after the reader has looked
again having opened the nearest existing directory above it, which makes the
client revalidate that directory (close-to-open consistency); a re-created
claim is then visible at once. Listings need nothing extra, since opening the
directory is how they start. Without the second look, the 2026-11-12 smoke run
saw a host that had just released a project's guard read a peer's guard,
written a second earlier, as absent and write its own over it. That was benign
there, both being capable and the guard marked, but the same read could miss
an unmarked guard and start an instance beside a project's only runtime.

`start_project` no longer skips a claimed project silently. A refusal is
reported as the project's `last_error`, and a start that carried a
`request_id` is answered with `ProjectRuntimeStarted { success: false, error }`
(Decision 9). The claims module also reports a project's live claims: the
other machines with a live instance claim and when each claimed the project,
another host holding the legacy claim without the marker, and this machine's
own claim. Adoption orders by them (Decision 6).

### 8. Transcript discovery across machines

On a shared home, provider transcript stores are shared. Claude's
`find_claude_switch_candidate` and the derived-path fallback exclude every pane
session id recorded in `.apas` for any machine, not only this instance's.
Without that, an idle pane on zoo-005 could adopt the transcript written by a
pane on zoo-006 in the same cwd. Codex's Linux discovery is process-based and
host-local, and Pi/OMP are pinned by id or directory; no change. OpenCode keeps
its documented same-cwd ambiguity.

### 9. Choosing a machine for a new tab

`WebToServer::AddPane` gains optional `machine_id` and `request_id`.
`machine_id: None` means the primary instance, the only target an older server
knows. With a `request_id`, the server answers `ServerToWeb::AddPaneResult {
session_id, request_id, status, machine_id, pane_id, error }`, where `status`
is an `AddPaneStatus`:

- `Routed`: delivered to the target machine's running instance. This is
  delivery, not creation; the tab appears when that instance reports it in the
  pane list.
- `Starting`: the target machine has the project but was not running it, so
  its instance is being started. It is never the last word: `Routed` or
  `Failed` follows for the same request.
- `Failed`: no tab was created, and `error` says why.

The result goes only to the connection that sent the request, never to the
session's other viewers. Only a client that understands the frame sends a
`request_id`, and older native builds drop their socket on a frame their schema
does not list. If that connection is gone when a later result is ready, the
result is dropped. A client that sees no final result, including from an older
server that ignores `request_id`, reports the launch as unconfirmed and never
resends it: a replay can create a second tab. The web keeps its launches in
the browser tab's session storage, so after a reload one still in flight comes
back unconfirmed rather than vanishing. A browser copies that storage into a
duplicate of the tab, so a duplicate shows the original's launches in flight
as unconfirmed too, while the original carries them out; their advice, that
nothing was resent and to check before trying again, holds either way.

For a request naming machine M:

- **Instance attached on M:** check runtime access on M, allocate an id
  against the merged roster *plus in-flight reservations*, tag the config with
  M, and route it as `NewPane`. The allocation also avoids the remembered ids
  of forgotten tabs, and hands one out only when no other id is free
  (Decision 2). An instance without the capability can only be the primary;
  its new pane stays untagged (Decision 2) and goes to it as `NewPane {
  machine_id: None }`. Reservations expire when the id appears in M's roster
  or after 60 s, and they also count native task launches, whose ids come from
  a separate range. While an id is reserved for M and not yet listed, frames
  for it are accepted only from M's instance (Decision 4). This also fixes
  today's race between two quick adds. If M's instance has attached but the
  server has not finished handling its `SessionStart`, the project's policy
  included, the tab waits for it, answered `Starting`, with no further start.
  Only the connection still attached for M counts: a replaced connection whose
  `SessionStart` finishes late changes nothing, so it neither overwrites its
  replacement's start, which would leave later tabs for M waiting until the
  time limit, nor releases a tab before the replacement holds the policy.
- **P registered on M, not running, M's daemon online and capable:** send
  `StartProjectCli { request_id }` to M (existing policy checks), keep a
  pending add keyed by (session, M) and reply `Starting`. The daemon answers
  every start that carries a `request_id` with exactly one
  `DaemonToServer::ProjectRuntimeStarted { request_id, project_id, success,
  error }`, on every outcome path: a missing or suspended policy, a claim held
  by an exclusive live holder, an externally started `--headless` run, already
  running, a failed spawn, and success. Heartbeats cannot answer this: they
  carry only a sticky `last_error` that a successful spawn clears, so they
  cannot say whether a failure belongs to this attempt, and several refusals
  set no error at all. `success: true` only means the runtime started, or
  already ran; the server routes the pending add once it has finished handling
  the `SessionStart` of M's attached instance, as above. `success: false`
  fails the pending add with `error`. So does the
  server refusing M's `SessionStart` (its registration is refused, it runs
  under another account, or it cannot attach beside the instances already
  there), at once and with that refusal's reason rather than after the
  timeout. M's daemon going offline, or 90 s without M's attachment, also
  fails it. Pending adds
  live in memory; a server restart drops them without a reply, the web shows
  the launch as unconfirmed, and nothing replays automatically.
- **Not registered on M, M owned by the requester:** the web offers clone to
  join (Decision 11).
- **Anything else** fails with the reason: M offline, M's daemon older, no
  runtime access on M, or no instance of the project there.

The picker lists the project's attached instances first (with host), then
owned machines where it is registered, then owned online machines without it,
then offline or older machines disabled with the reason (Decision 5). An
instance whose `runtime_access` is `Some(false)` for this viewer is disabled
with the reason too. The picker defaults to the machine of the selected tab.

The machine step is hidden only when exactly one machine is eligible and it
already runs the project; the menu is then unchanged. A sole eligible
registered machine, typically a stopped single-machine project, is still
shown, preselected, under "Starts the project first": choosing a tab type
starts the project there, relaunching its saved tabs and unpaused bots, before
the tab is added, and the menu says so before the click, as the project's Open
asks before the same start. A clone is never preselected, so a sole clone
choice must be picked, and the clone confirmed, before a tab can be requested.

**Instances for viewers.** `ServerToWeb::SessionInstances { session_id,
instances }` carries the complete set of attached instances, so an empty list
means none is attached. It is sent only to web and native connections that
advertised `project_instances_v1` in `Authenticate.capabilities`. The server
adds the capability to its negotiated-capability allowlist and echoes it in
`Authenticated.negotiated_capabilities`, so a client knows whether the frame
will come. Such a connection receives the full set right after
`SessionAttached`, and again on every change: an instance attaches or
detaches, or the primary hands over. The replay after `SessionAttached` reads
the set only as it is queued, with nothing awaited in between, and leaves
`runtime_access` to the connection's outbound task, which fills it in for
broadcasts too. Read earlier, a set that changed while the viewer attached
would be broadcast first and the older copy would arrive after it, as the
viewer's last word: a running machine's tabs shown stopped until the next
change. Older native builds validate every frame
against their bundled schema and drop the socket on an unknown one, and a
replay on every attach would keep them reconnecting. A client without the
capability reads `SessionInfo.instances` from the session list and mobile
bootstrap, and the pane tags in `PaneList`, which older schemas accept.

Each `SessionInstanceInfo` carries `machine_id` (`None` only for the
machine-less sole attachment of Decision 1), `hostname` (display only),
`working_dir`, `primary`, and `runtime_access: Option<bool>`. The server
computes `runtime_access` per viewer connection, because runtime access is
evaluated per machine (Decision 3): on a third-party host it needs cluster
membership that covers that exact machine, so one viewer can use some of a
project's instances and not others. Clients disable an instance's tabs and its
machine choice when it is `Some(false)`, and say why, rather than offer
controls that can only fail with "Access denied". `None` means not evaluated:
treat the instance as usable and let the server decide.

**Project-level Start and Close.** The project view's machine resolution
(`resolveMachineProjectTarget`) prefers the primary attachment's machine, then
the preferred registration machine, instead of returning `null` on ambiguity.
`StartMachineProjectCli` and `StopMachineProjectCli` gain `all_instances`
(default false), and the project view sends `true`. Because tags are
permanent, Start then starts an instance on every online, capable machine that
owns tabs of the project, counting an untagged pane for the machine its
attributed host names (Decision 2), and starts on the given `machine_id` only
when no machine qualifies.

Close stops every machine running the project: the given `machine_id`, which a
Close always stops, every machine with an instance attached, and every machine
whose connected daemon reports running the project while its instance is not
attached, because it is still starting or is reconnecting after its own
connection dropped or the server restarted. Its machine entry shows the
project running, which is what the client offers Close on, so stopping only
the attached instances would leave its agents running, and the project would
come back when its instance reattached. The server reads that report from the
daemon's own inventory, not from the machine list's projection, and counts a
reporting daemon only when it belongs to an account the project's sessions run
under: another account's checkout can carry the same project id, and its
daemon reports running it until that instance's `SessionStart` is refused. The
`all_instances` doc comment states this filter too. A daemon that lists the
project without running it, or is disconnected, is sent nothing and causes no
refusal. A daemon's report lags a real stop by up to one heartbeat, so a
machine that has just stopped may be sent a redundant Stop, which is harmless.
A project with no session record yet, during its very first start, has no
account to check, so its Close skips reporting machines.

Only a requester with access to the project's content may ask for every
instance: the fan-out reads the project's roster and its refusals name hosts,
so anyone else gets the bare "Machine not found" a per-machine request would,
and no daemon is sent anything. Each machine is then authorized as a
single-machine request would be, and a refusal for one of several machines
names its host. The one exception is a Close's refusal for the machine the
request itself names, which can be any machine at all: it names that machine's
host only when the requester knows the machine (Decision 4), and otherwise
reads "<id>: Machine not found", as for an unknown machine. A Start is also
refused, before any daemon frame, for every machine that cannot run the
project beside a CLI without the capability that runs it, and for a machine
whose connected daemon lacks the capability while an instance runs the project
(Decision 5). A Start
first places a project that only its owner's machine has registered
(Decision 10). `false`, which every older client sends, keeps today's
single-machine meaning for the Machines page's per-machine Start and Stop,
which must not start or stop other machines; an older server ignores the flag
and acts on `machine_id` alone. Stop-runtime, suspension and deletion act on
every attachment, each rejected on its own connection, and deletion waits for
every machine's purge acknowledgement.

**Tabs of a machine that is not running.** Their controls are disabled with
"Start on <host>". While any instance of the project is attached, the server
refuses controls and closes for them with "<host> is not running this
project. Start it on <host> first.", because the owning instance holds the
tab's state; with none attached they fail as they always have. For a machine
that will not come back, **Forget <host>'s tabs** asks for confirmation, and
its copy says the tabs come back if <host> runs this project again. The
server refuses
`ForgetMachinePanes` while that machine has a running instance, which is
authoritative for its own panes and would report them again, and while a CLI
without the capability runs on its host, which may be that machine under an
older CLI. Otherwise it removes the panes tagged with that machine, and the
untagged panes attributed to its hostname, from the server roster, remembers
their ids, and tells no instance (Decision 2). Whether that machine shares the
`.apas` or has a separate checkout, its entries are untouched, so if it
returns, its instance restores and reports its tabs again, because each
instance is authoritative for its own panes.

Typed input refused this way, or because the sender may not use the tab's
machine, is also answered with `MutationAck { mutation: input, accepted:
false, error }`, whose `request_id` is the input's `client_msg_id` and whose
`pane_id` is the tab it was for. It goes only to the sending connection, only
when that connection negotiated `project_instances_v1` and the input carried a
`client_msg_id`, and always in addition to the `Error` with the same text, so a
client that does not track sends by id still shows the refusal. It is only
ever a refusal and is never registered as a pending decision: accepted input
is still confirmed by the `UserInput` echo. The server may answer other
refusals of such input the same way. Older native builds' schemas list only
the four earlier kinds, and they drop the socket on a frame that does not
validate, hence the gate.

Every client that tracks the send settles it on this acknowledgement,
whichever attempt it answers: the first send, a retransmission or a replay. It
never sends that `client_msg_id` again, neither as a retransmission nor as a
replay after a reconnect, including from stored sends its other connections
share. The server records the id only of input it delivers and judges every
resend afresh, so once the tab's machine runs the project again a resend would
be delivered as new input, possibly minutes after the user saw it refused. The
client shows the reason where it shows the send, instead of showing the send
as unconfirmed or still under way, and keeps the typed text recoverable; the
`Error` has already raised the notice. Sending the text again is the user's
choice, as a new send under a new id. An acknowledgement for an id the client
is not tracking changes nothing, and a `UserInput` echo of a settled id, from
an attempt sent before the acknowledgement and delivered after the machine ran
the project again, stands. The binding statement is the `MutationKind::Input`
doc comment.

The web removes a refused send from its pending queue, in memory and in
storage, so it is neither retransmitted nor replayed, and marks the send's
bubble "Not delivered" with the reason, keeping its text. On mobile web a
refused terminal message shows the reason at once rather than waiting to be
reported unconfirmed, and its draft can be restored. Native rejects the
pending acknowledgement with a refusal it can tell from a timeout or an
interruption, and only then retires the message's id: a send that was never
acknowledged keeps its id, so a retry is recognized as the same message if the
first one landed.

**Native task launch.** `MobileTaskLaunchRequest.machine_id`, the machine of
the chosen `MobileLaunchTarget`, keeps its original meaning until an instance
advertising the capability runs the project: start the project there when
nothing runs it, otherwise add the task to the running session. That is the
rule that nothing changes for a project before a capable instance runs it.
Once one does, the field means what `AddPane.machine_id` means: the instance
running on that machine receives the task, a registered but stopped machine
whose daemon advertises the capability is started first, and a machine that
cannot run an instance beside the attached ones fails the launch with the
reason. A launch picked by machine never lands silently on another machine.
Like a Start, a launch first places a project that only its owner's machine
has registered (Decision 10).

### 10. Joining a checkout by git remote

Daemons add the canonical `git_remote` to each `MachineProjectInfo` they
report, normalized by `normalized_git_remote` exactly as
`SessionStart.git_remote` is. A registered project that has never started has
no session to carry its remote. The server keeps the field in its machine
inventory for matching and passes it through to clients. The web then groups
registration-only project entries under their repository rather than
"(no remote)", which fixes the misplacement that prompted this change.

`GET /projects/by-remote?git_remote=<host/owner/repo>&project_id=<uuid>`
answers `ProjectsByRemoteResponse`. `project_id` is optional. The CLI sends
`Authorization: Bearer <account token>` with the token it already holds, and
`/projects/` is already proxied by nginx. The canonical form is
`normalized_git_remote`'s output, and the server compares remotes in that same
form.

- `projects` lists the projects the caller owns or hosts whose sessions, or
  whose owned-machine inventory, carry that remote, so a registered but
  never-started project also matches. It leaves out a project being deleted,
  and a project whose session runs under another account, such as one the
  caller owns that was provisioned on a cluster owner's machine: every
  instance runs under its session's account and a `SessionStart` from any
  other is refused, so a checkout joined to it could never start. Each
  `ProjectRemoteMatch` has its id,
  name, `has_session` (the server holds a session, and so history, for it),
  and its known checkouts, each with `machine_id`, `hostname`, `path` and
  `running`. The checkout's own current identity is listed too when its remote
  matches, so the CLI excludes its own id before offering a join.
- `local` answers the `project_id` parameter, the checkout's current local
  identity, whatever remote that project's sessions recorded. A project with
  history can be missing from `projects`: an older CLI never sent its remote,
  its origin changed, or no daemon has reported it yet. Zero panes is also a
  valid state for such a project, so without `local` the CLI could not tell it
  from a pristine identity, and re-identifying it would orphan its history.
  `local` is `None` when the query named no id, or when the caller neither owns
  nor hosts a project with that id.

`apas register [path]` then:

1. computes the canonical remote with `normalized_git_remote`;
2. looks it up, passing the local `.apas` id as `project_id` when `.apas`
   exists;
3. if that project has panes, or `local` reports a session, registers it as-is
   (current behaviour). It re-identifies a checkout only when the local project
   has no panes and the server holds no session for it: no `local`, or a
   `local` whose `has_session` is false. Without the server's answer it cannot
   establish that, so it never re-identifies then;
4. otherwise drops its own id from `projects`. With no match it registers as
   today. With one match it asks `Join <name> (<id>, on <host>:<path>)? [Y/n]`.
   With several it lists them and requires `--project`. Non-interactive runs
   need `--yes` or `--project`;
5. joining writes `.apas` with the existing id and zero panes, under the
   registry lock, replacing a pristine local identity (the laptop's
   `5762761c…` case) and its registry entry.

`--project <id>` joins explicitly, still only from a pristine local identity,
and `--new` skips matching. A registry is one path per project id. Joining is
refused when the shared registry already has the project at another path,
because a shared-home machine already sees that checkout: start it there
instead of cloning a second copy into the same home.

**Starting a project only a registration knows.** `apas register`, and a
bare `apas` in a new directory, register a project on the machine without
telling the server. Until something starts it there is no project record and
no cluster placement, so the runtime-access check refused every web Start of
it as "Machine not found", and a project registered that way could never be
started from the web. `StartMachineProjectCli`, for the machine it names,
therefore first places such a project when the requester owns that machine,
the machine lists the project, and the project has no placement in the
requester's cluster. Under the project-operation guard it calls
`authorize_project_registration`, exactly as the instance's `SessionStart`
would a moment later. It creates a missing project record owned by the
requester, with the direct placement, and refuses a suspended or deleting
project or one the requester has no access to; a refusal leaves everything as
it was, and the Start is refused as before. A cluster member still needs a
project role first.

A native task launch places a project the same way, but cannot yet offer one
that has never run: its launch targets come from project policy, which needs
the project record, so the placement in its launch path helps only a project
that already has a record but no placement in the requester's cluster.
Starting the project once from the web or the Machines page creates the
record. This predates instances and is left for a later change.

### 11. Clone to join

`WebToServer::CreateProjectInstance` gains optional `join_project_id`. It is
allowed only on the owner path: the requester must host the project and own
the target machine, whose daemon must advertise the capability. It is refused
for a project whose session runs under another account, for the reason
`/projects/by-remote` leaves such a project out (Decision 10), and for a
machine that already lists the project ("<host> already has this project. Add
the tab there instead."), since a second checkout there would be a second
instance on one machine. The server
then forwards `ServerToDaemon::CreateProjectInstance { project_id: Some(P),
join_existing: true, .. }`. `join_existing` is a separate flag because a
present `project_id` already means a fresh identity reserved for two-phase
provisioning. The daemon clones without creating a branch, writes `.apas` with
P and zero panes, registers it, and does **not** start it. It sends a
`Heartbeat` listing the new checkout *before* the `ProjectInstanceCreated`
acknowledgement. The server handles one daemon's messages in order, so its
inventory already lists the project on that machine when the acknowledgement
reaches the web; acknowledging first would race the web's next request against
the inventory refresh. A failure before registration deletes the partial clone
as today. Shared-cluster provisioning is unchanged.

The tab is a second step, chained by the client; the server does no chaining:

1. The web sends `CreateProjectInstance { join_project_id: P, request_id }`,
   only for a machine whose entry reports `project_instances_available`. An
   older server never sets that, and would ignore the field and create a
   separate project.
2. It waits for `ProjectInstanceCreated` carrying that `request_id`,
   `project_id` equal to P, and no `error`.
3. It sends `AddPane { machine_id, request_id: <new> }`, which starts the
   instance through Decision 9: `Starting`, then `Routed` or `Failed`.

"Clone, start and add" is one launch only in the UI. If the client loses the
acknowledgement, the checkout may exist without a tab; it reports that as
unconfirmed and never resends the clone. A late acknowledgement, which reaches
every connection of the owner, still says that the clone finished and, if its
tab does not appear, where to add one. It never says that no tab was added: in
a duplicated browser tab (Decision 9) the clone's launch is unconfirmed while
the page it was copied from goes on to add the tab.

The clone goes into a new folder named after the repository under
`~/apas_projects`, suffixed when that folder exists, and the confirmation says
so. The acknowledgement does not carry the path; the Machines page lists it.

### 12. Display

The web tab bar shows a host badge on every tab once the roster spans two
machines, plus a running/disconnected state per machine. An untagged pane is
shown under its attributed host, with no host badge when it has none
(Decision 2). Tabs of a stopped machine, and of an instance whose
`runtime_access` is `Some(false)` for this viewer, show why their controls are
disabled (Decision 9). On the web such a terminal tab keeps showing its
retained screen but sends no keystrokes or resizes, so a hidden view does not
provoke a refusal on every window resize. The sidebar keeps one row per
project, shows the primary host plus "+N" when several instances are attached,
and pane dots cover every machine. Mobile web and native show the badge from
the pane tags in `PaneList` and `MobilePaneSummary`.

The web keys a session's view state, such as its selected tab, its sidebar
layout and the usage limits it shows, by one CLI connection: the primary's
while it is live, else the one already in use while it is still attached, else
the first live one. Every instance reports the project's one session, so
following whichever is listed first would send the view back to its first tab
whenever another machine's instance attached or left. When the primary's own
connection leaves, the key moves to a remaining instance, as on any CLI
reconnect.

## Risks / Trade-offs

- **NFS locking.** The registry already relies on `fs2` locks over NFS; the
  per-project lock under `<config dir>/locks/` uses the same mechanism. If
  locking fails, the save fails rather than writing unlocked.
- **Untagged-pane adoption race.** Two capable instances starting at the same
  moment each see the other's claim, which both wrote before reaching
  adoption. The earlier claim, then the lower machine id, adopts, and the
  lock with a fresh read tags each pane once (Decision 6).
- **Remembered ids can be reused.** A forgotten tab's id is handed out again
  only when no other id in the allocation range is free, and a native task
  launch derives its id from its request rather than the allocator. A
  forgotten tab whose id was taken either way is dropped as another machine's
  pane when its machine returns.
- **Stripping by an older binary** is bounded by the claim guard (Decision 7)
  and repaired by adoption.
- **Primary handover** changes the session summary hostname when the earliest
  instance stops. That is accepted: it reports a running host.
- **Pending adds are in memory.** A server restart drops them; the web reports
  them unconfirmed instead of replaying.
- **Results reach one connection.** `AddPaneResult` goes only to the
  requesting connection and is dropped when that connection is gone, so a
  reconnect during a start shows the launch as unconfirmed even when the tab
  later appears. That is the price of never sending older native builds a
  frame they would reject.
- **Attributed hosts are hostnames.** They only hint where an untagged pane
  ran, and hostnames are neither stable nor unique: a host can be renamed, and
  two machines can share a name. A wrong or ambiguous hint can misdirect or
  refuse a control for that pane, but it never assigns the pane to a machine.
  Once a capable instance reports the pane it is tagged by machine id,
  whatever the hint said.
- **A capable daemon's only runtime looks capable.** It reports its machine
  (Decision 5) and its daemon advertises the capability, so after a server
  upgrade clients cannot tell it from an instance and may offer other machines
  for a new tab or a Start. The server refuses those with the instruction to
  reboot that host to update it, and the reboot brings the project back as an
  instance. That instance restores only its own and untagged tabs: another
  machine's tab that the only runtime ran but never saved untagged ends on
  this host, and the server lists it untagged under this host until its own
  machine runs the project again. Nothing is deleted from `.apas`.

## Migration / Rollout

Deploy shared protocol → server → web → mobile → CLI/daemon, the order the
runbook uses for every protocol change. The shared contract is already
committed (task group 1). All new fields are optional, and a new server with
only old CLIs behaves exactly as today. The new frames, `SessionInstances`
and `AddPaneResult`, and `MutationKind::Input` acknowledgements reach only
clients that asked for them, so older web and native builds keep working
against a new server. Hosts gain the capability
when their daemon is rebooted to update from the Machines page, and use it
only while the server echoes it at registration (Decision 5), so a daemon
updated before the server keeps running each project on one machine, as that
project's only runtime with every tab, as an older daemon would. Existing
`.apas` files need no migration. The server shows a pre-upgrade roster's
untagged panes under the session's recorded host, and they are tagged when a
capable instance adopts and reports them.

An older daemon can still start a project on a shared home whenever no capable
instance is running it, and so can a capable daemon whose server has not
confirmed support, whenever no other host runs it (Decision 5). Either then
restores **every** pane in `.apas`, including panes tagged for other machines,
and strips the tags when it saves. That is today's host-migration behaviour,
and the claim guard bounds it. Nobody should
be surprised when an un-rebooted zoo-006 daemon picks up zoo-005's tabs. Its
CLI reports them untagged, so the server stamps them with zoo-006 as their
attributed host and leaves them adoptable by the next capable instance
(Decision 2). Tabs tagged for a machine whose entries that `.apas` does not
hold, such as a separate checkout's, stay listed as stopped on their machine,
with their ids.

**Rollback.** Stop extra instances first, by stopping the project on
non-primary machines, then roll back CLI/daemon, web and server. An older CLI
ignores tags, and an older server treats a multi-machine roster as one list,
which is safe once only one instance runs. A capable daemon that re-registers
with a rolled-back server starts no further instance beside another live host
(Decision 5), but it never stops one already running, so stopping the extra
instances remains the first step. If the server is rolled back while capable
daemons still run, an instance left running still reports only its own
machine's tabs, which the rolled-back server takes as the whole roster. The
other machines' tabs stay in their `.apas`; restarting the project there runs
it as its only runtime (Decision 5), which lists every tab that `.apas` holds
again.
