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
InstanceAttachment { cli_id, hostname, working_dir, git_remote, attached_at }
```

The **primary** attachment is the earliest still attached. It supplies the
legacy single-valued fields (`cli_client_id`, DB `hostname`/`working_dir`), the
default target for controls that name no machine, and the session summary
hostname. A second `SessionStart` from the *same* machine replaces that
machine's attachment, which is today's reconnect behaviour.

Every existing writer of those single-valued fields becomes primary-only or
instance-aware. Otherwise a secondary instance's `SessionStart` would flip the
session's host:

- the DB `create_session` upsert in the `SessionStart` handler, which also
  migrates shares by working dir and hostname;
- `set_session_machine`, where the last writer wins even with `None`, and
  which feeds `session_machine_id` and cluster machine enforcement, and
  `set_session_project`;
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

Add to `PaneConfig`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub machine_id: Option<Uuid>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub hostname: Option<String>,   // display only
```

The server overwrites both on every pane it accepts from an instance, using the
authenticated attachment, so a CLI cannot claim another machine's panes. An
untagged pane in a persisted roster belongs to the primary attachment.

Roster merge, replacing `set_session_panes`' whole-list replacement for
capable instances:

- `Some(list)` from machine M replaces exactly the panes owned by M. `Some([])`
  means "M has no panes" and leaves every other machine's panes alone. `None`
  changes nothing. This is the same empty-versus-unknown distinction the
  runbook requires at project level, applied per instance.
- An incoming pane id already owned by another machine is dropped and logged;
  server allocation makes this unreachable except through a bug.
- The server owns the global order. Existing panes keep their positions and new
  ones are appended; labels come from the cache. Status pruning is limited to
  M's removed panes.
- The persisted roster (`panes.json`) stores the tags, so stopped instances'
  tabs survive a server restart and render with their host.

### 3. Routing, capability and access by pane machine

Add `ServerToCli::pane_id(&self) -> Option<u32>` and a `PaneTarget`
classification. `route_to_cli` becomes:

- **pane-scoped** (`Input` with a pane, `TerminalInput/Resize`, pause/resume/
  reboot/remove/interrupt/label/role/effort/model/review/manual modes, diff,
  `CreatePr`, `StartBot/StopBot`): the attachment owning that pane;
- **decision-scoped** (`AnswerQuestion`, `PlanReviewAnswer`, approve/reject):
  the pane of the pending decision, which the server already resolves;
- **project fan-out** (`ProjectPolicy`, `UpdateProjectOperations`,
  `RequestPaneList`): every attachment;
- **process-scoped** (`RebootCli`, `CliLifecycleRequest`, `Signal`): an
  explicit optional `machine_id` on the web request, else the primary.

`AnswerOmpQuestion` and pane work-summary dispatch use the same
pane-owner lookup instead of reading `cli_client_id`. `session_supports_
capability` takes the target attachment. `has_session_runtime_access` is
evaluated against the pane's machine for pane mutations and the target machine
for `AddPane`. Attaching to a session requires runtime access to at least one
attached machine, or none attached, which is today's rule when there is one.

### 4. Inbound ownership and disconnect scoping

A pane-scoped message from a CLI is accepted only if that CLI is the attachment
for the pane's machine; otherwise it is dropped with a warning. This closes the
existing hole where a replaced CLI keeps writing into a session.

On disconnect of machine M's instance: remove M's attachment, move only M's
running terminals to `disconnected`, clear only M's pane statuses, and keep
M's panes in the roster. The DB row becomes inactive, `clear_session_cli`
runs, and `SessionAttached { has_active_cli: false }` is broadcast only when
the last attachment leaves; otherwise the primary moves to the next-earliest
attachment and the DB row is updated to it. `SessionEnd` from one instance
detaches it; only the last one ends the session. Lifecycle inventories are kept
per (session, machine). Machine `is_running` enrichment reads every attachment.

### 5. Mixed fleets degrade to one machine

Capable CLIs and daemons advertise `project_instances_v1`. A `SessionStart`
from a CLI without it keeps today's replace semantics when no other machine is
attached, and is rejected with `SessionRejected` ("running on <host>; update
this CLI") when another machine's instance is attached. Accepting it would let
its untagged full roster overwrite the others. In the other direction, a
capable `SessionStart` that arrives while a non-capable CLI from another
machine is attached is also rejected with a reason. That can happen through
standalone `--headless`, which takes no claim. Replacing would orphan an old
CLI that keeps writing into the session.

The server routes an `AddPane` naming a non-primary machine only when that
machine's daemon advertises the capability. Otherwise it returns a correlated
failure saying to reboot the machine to update it. The web offers the machine
choice only for machines whose machine-list entry reports
`project_instances_available`.

A host whose `config.daemon.machine_id` is set does not advertise the
capability. That id is shared through the NFS home, so it cannot distinguish
instances.

### 6. Shared `.apas`: locked merge of own panes only

Each instance knows its machine id from start; standalone `--headless` derives
it with `derive_machine_id` instead of passing `None`. All `.apas` writes take
an exclusive `fs2` lock on `<config dir>/locks/<project-id>.lock`, then
re-read, modify, write temp and rename. The registry lock already works this
way. The lock is not a sidecar in the project directory: repositories ignore
`.apas` by exact name (chiral-network's `.gitignore` does), so a `.apas.lock`
would show up in `git status` where an agent could commit it. Instances that
share a `.apas` share the NFS config dir and therefore the lock. A separate
checkout has its own `.apas` and needs no shared lock.

- **Pane saves** (`save_pane_configs` and every caller) replace only the panes
  whose `machine_id` is this machine, keep every other pane, and keep
  project-level fields as read from disk.
- **Project-level writes** (`update_project_operations`, the legacy `is_paused`
  writes, worktree assignment) change only their field under the same lock.
- **Adoption.** At instance start, under the lock, untagged panes are tagged
  with this machine *only if* no other machine holds a live instance claim for
  the project. A new instance on a second machine therefore starts with no
  panes. With one machine this tags the existing panes in place, the only
  on-disk change to existing projects.
- **Tags are permanent** (owner decision). An instance never restores or adopts
  a pane tagged for another machine, even when that machine is stopped. This
  is a deliberate change from today's shared-home behaviour, where starting
  the project on zoo-006 after stopping it on zoo-005 resumed zoo-005's tabs.
  Those tabs now stay listed as stopped on zoo-005 until zoo-005 runs the
  project again, and moving a tab between machines is not supported.
- **Restore** starts only this machine's panes, so `reconcile_project_hosts`
  never sees another machine's panes as its own.
- **Periodic reads** (5 s flags, reconnect policy snapshot) use a
  non-destructive reader that never deletes or regenerates `.apas`; a parse
  failure is logged and the last good copy kept.
- **Pane ids** are allocated by the server against the merged roster, so ids
  are unique across machines. `--attach`'s local add checks every pane in
  `.apas`, not just its own.

Older binaries drop unknown fields on rewrite and would strip tags. That is
safe because of Decision 7: an older runtime can only run a project when no
capable instance does, and its untagged panes are re-adopted by the next
capable instance.

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

Claim records are written temp-and-rename with host- and pid-qualified temp
names; the current `.tmp.<pid>` name can collide across hosts.

### 8. Transcript discovery across machines

On a shared home, provider transcript stores are shared. Claude's
`find_claude_switch_candidate` and the derived-path fallback exclude every pane
session id recorded in `.apas` for any machine, not only this instance's.
Without that, an idle pane on zoo-005 could adopt the transcript written by a
pane on zoo-006 in the same cwd. Codex's Linux discovery is process-based and
host-local, and Pi/OMP are pinned by id or directory; no change. OpenCode keeps
its documented same-cwd ambiguity.

### 9. Choosing a machine for a new tab

`WebToServer::AddPane` gains optional `machine_id` and `request_id`. The
server answers with `ServerToWeb::AddPaneResult { session_id, request_id,
status: Routed | Starting | Failed, machine_id, pane_id, error }`.

- **Instance attached on M:** allocate an id against the merged roster *plus
  in-flight reservations*, tag the config with M, and route. Reservations
  expire when the id appears in M's roster or after 60 s. This also fixes
  today's race between two quick adds.
- **P registered on M, not running, M's daemon online and capable:** send
  `StartProjectCli` to M (existing policy checks), keep a pending add keyed by
  (session, M) and reply `Starting`. On M's `SessionStart`, route the pending
  add; on start failure, daemon offline, or 90 s without attachment, reply
  `Failed`. Pending adds live in memory; a server restart loses them, the web
  shows "unconfirmed", and nothing replays automatically.
- **Not registered on M, M owned by the requester:** the web offers clone to
  join (Decision 11).

The picker lists the project's attached instances first (with host), then
owned machines where it is registered, then owned online machines without it,
then offline or older machines disabled with the reason. It defaults to the
machine of the selected tab. With a single eligible machine the menu is
unchanged.

The project view's machine resolution (`resolveMachineProjectTarget`) prefers
the primary attachment's machine, then the preferred registration machine,
instead of returning `null` on ambiguity. Because tags are permanent,
project-level Start starts an instance on every online machine that owns tabs.
With no tabs anywhere it starts on the resolved machine. Stop and stop-runtime
stop every instance.

**Tabs of a machine that is not running.** Their controls are disabled with
"Start on <host>". Closing one is refused with that reason, because the
owning instance holds the tab's state. For a machine that will not come back,
**Forget <host>'s tabs** asks for confirmation. It then drops that machine's
panes from the server roster and sends `ForgetMachinePanes { machine_id }` to
the attached instances. An instance sharing the same `.apas` removes those
entries under the lock. A separate checkout's `.apas` is untouched; if that
machine returns, its instance reports its tabs again, because each instance is
authoritative for its own panes.

### 10. Joining a checkout by git remote

Daemons add the canonical `git_remote` to each `MachineProjectInfo` they
report; `normalized_git_remote` already exists. The web then groups
registration-only project entries under their repository rather than
"(no remote)", which fixes the misplacement that prompted this change.

`GET /projects/by-remote?git_remote=<host/owner/repo>` (account bearer token;
`/projects/` is already proxied by nginx) returns projects the caller owns or
hosts whose sessions, or whose owned-machine inventory, carry that canonical
remote. Each result has its id, name, and known instance hostnames and paths,
so a registered but never-started project also matches. `apas register [path]` then:

1. computes the canonical remote with `normalized_git_remote`;
2. if `.apas` exists and that project has panes or a server session, registers
   it as-is (current behaviour);
3. otherwise looks up the remote. With no match it registers a new project.
   With one match it asks `Join <name> (<id>, on <host>:<path>)? [Y/n]`. With
   several it lists them and requires `--project`. Non-interactive runs need
   `--yes` or `--project`;
4. joining writes `.apas` with the existing id and zero panes, under the
   registry lock, replacing a pristine local identity (the laptop's
   `5762761c…` case) and its registry entry.

`--project <id>` joins explicitly and `--new` skips matching. A registry is
one path per project id. Joining is refused when the shared registry already
has the project at another path, because a shared-home machine already sees
that checkout: start it there instead of cloning a second copy into the same
home.

### 11. Clone to join

`CreateProjectInstance` gains optional `join_project_id`. It is allowed only on
the owner path: the requester must host the project and own the target machine.
The server forwards `project_id: Some(P)`. With an id under the owner path, the
daemon clones, skips the unique-branch step, writes `.apas` with P and zero
panes, registers it, and does **not** auto-start. The server then runs the
pending start-then-add from Decision 9. A failure before registration deletes
the partial clone as today. Shared-cluster provisioning is unchanged.

### 12. Display

The web tab bar shows a host badge on every tab once the roster spans two
machines, plus a running/disconnected state per machine. The sidebar keeps one
row per project, shows the primary host plus "+N" when several instances are
attached, and pane dots cover every machine. Mobile web and native show the
badge.

## Risks / Trade-offs

- **NFS locking.** The registry already relies on `fs2` locks over NFS; the
  `.apas.lock` uses the same mechanism. If locking fails, the save fails
  rather than writing unlocked.
- **Untagged-pane adoption race.** Two capable instances starting at the same
  moment are serialized by the lock and the per-machine claim. The second sees
  the first's live claim and does not adopt.
- **Stripping by an older binary** is bounded by the claim guard (Decision 7)
  and repaired by adoption.
- **Primary handover** changes the session summary hostname when the earliest
  instance stops. That is accepted: it reports a running host.
- **Pending adds are in memory.** A server restart drops them; the web reports
  them unconfirmed instead of replaying.

## Migration / Rollout

Deploy shared protocol → server → web → mobile → CLI/daemon, the order the
runbook uses for every protocol change. All new fields are optional, and a new
server with only old CLIs behaves exactly as today. Hosts gain the capability
when their daemon is rebooted to update from the Machines page. Existing
`.apas` files need no migration; their panes are tagged on the first run by a
capable instance.

An older daemon can still start a project on a shared home whenever no capable
instance is running it. It then restores **every** pane in `.apas`, including
panes tagged for other machines, and strips the tags when it saves. That is
today's host-migration behaviour, and the claim guard bounds it. Nobody should
be surprised when an un-rebooted zoo-006 daemon picks up zoo-005's tabs.

**Rollback.** Stop extra instances first, by stopping the project on
non-primary machines, then roll back CLI/daemon, web and server. An older CLI
ignores tags, and an older server treats a multi-machine roster as one list,
which is safe once only one instance runs.
