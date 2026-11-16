## Context

See `proposal.md` for the motivation. What the code does today, from a
read of it on 2026-11-16:

- **Nothing records that a project should run.** The server's `projects`
  table has `lifecycle_status` (`active`, `suspended`, `deleting`) and nothing
  about running. What runs is known only in memory, from each daemon's
  inventory (`MachineProjectInfo.is_running`, cleared when the daemon
  disconnects) and from attached instances.
- **Two requests start and stop.** `WebToServer::StartMachineProjectCli` and
  `StopMachineProjectCli` carry a machine, a project and `all_instances`.
  The project view sends `all_instances: true`; the Machines page, "Start on
  \<host\>" and the phone's banner send it false. The server answers neither
  with a result: refusals arrive as plain `Error` frames and success is seen
  when the polled machine list changes.
- **A project-wide start reaches only what is online at that moment**
  (`project_start_machines`): connected machines with multi-instance support
  that own tabs, else the named machine. Offline and older machines are left
  out without a word. A project-wide stop reaches machines that run the
  project (`project_stop_machines`), not machines that own tabs, never an
  offline one, and only daemons of the accounts the project's sessions run
  under, because another account's checkout can carry the same project id.
- **The mixed-fleet refusals read attached instances**
  (`beside_older_instance_refusal`, `older_daemon_beside_instance_refusal`).
  With nothing attached, as when a closed project is opened or just after a
  server restart, they refuse nothing.
- **Nothing is sent when a daemon registers** except `Registered`. A machine
  that was away never catches up. `register_daemon` clears the daemon's
  capabilities, which are set again a few lines later in `ws_daemon.rs`.
- **Other paths start projects:** shared provisioning's finalize
  (`ws_daemon.rs`), a trusted owner's create-and-start inside the daemon,
  the two mobile launch paths, and a tab request on a stopped machine.
- **The daemon starts projects by itself in one case:** after replacing
  itself, from the resume manifest in its volatile runtime directory, before
  it has a server connection and without a policy check. A project closed
  during the replacement comes back.
- **A daemon start is idempotent** (`AlreadyRunning` is success), a stop of a
  project that does not run is success, and a start that carries a
  `request_id` is answered with exactly one `ProjectRuntimeStarted` by a
  daemon with multi-instance support; an older daemon never answers.
- **The roster can be read right after a server restart:** the session id is
  the project id, and the persisted roster carries each tab's machine, or
  for an untagged tab its host.
- **There is no machines table.** A machine is known to the server only
  while, or after, its daemon has registered since the last restart.
- **The UI models the half-open project in full:** "Not running on \<host\>"
  dots, a stopped tab state, "Start on \<host\>", and Open and Close buttons
  that each look at one resolved machine, so a project running on one of two
  machines can show neither where it is needed.
- **Native sends no start or stop** except its task launch over REST. It
  drops its socket on a frame type or enum value its bundled schema does not
  know; a new optional field is safe.

## Goals / Non-Goals

**Goals:**

- One recorded answer to "is this project open", the same for every viewer
  and after every restart.
- Every machine that owns tabs of an open project runs them, including a
  machine that was offline, rebooting or reconnecting when it was opened.
- No machine keeps running a closed project once the server can reach it.
- A machine that cannot follow is named with its reason; nothing is left
  out silently.
- A rollout that starts no agent and ends none, also through older clients.

**Non-Goals:**

- Keeping one machine out of an open project, or restarting or stopping one
  machine's instance alone (Decision 8).
- Restarting a tab whose agent exited, or a project that keeps failing.
- Changing the daemon, the CLI, the resume manifest or pane hosts.
- Starting a daemon on a machine. A machine follows when its daemon runs and
  registers; nothing in this change starts a daemon at boot.
- A Close control in the mobile browser's project screen, which has none
  today. It gains "open again" only.
- Autonomous workers. Closing a project neither cancels nor pauses them.
- Changing what deletion erases. Delete project keeps the server-side
  deletion of `project-lifecycle-management`: a machine's folder, its
  `.apas` and its registration stay, so a deleted project's folder is still
  listed as a registered folder on its machines. Removing that registration
  needs a daemon request that does not exist; it is left as an open
  question.
- A Delete control on the phone.

## Decisions

### 1. The state is five nullable columns on `projects`

`open_state` (`NULL`, `'open'`, `'closed'`), `open_state_set_by` (an account
id; empty for the system administrator, who is not an account),
`open_state_set_kind` (`'request'`, or `'administrative'` for a suspension,
a stop-runtime or a deletion, which a follow step treats differently;
Decision 6), `open_state_set_at`, and `open_state_machine_id` (the machine
an open named, used only while no machine owns a tab; Decision 4). They are added with the `PRAGMA table_info`
guard the placement column used.

- *Not `lifecycle_status`.* Its parser reads an unknown value as `deleting`,
  its triggers reject one, and nine queries compare the raw string
  `'active'`.
- *Not `.apas`.* Separate checkouts each have their own, and a machine that
  is offline cannot be told. The server is the only place every machine
  meets.
- *Not the daemon.* A second durable record on each machine would have to be
  reconciled with the server's after every outage.

Only an active project can be open. Opening a suspended project or one being
deleted is refused and records nothing; suspending and beginning a deletion
record closed (Decision 9). So "open and suspended" never exists, and
reactivation finds the project closed.

### 2. "Not recorded" is a third value, and the server leaves such projects alone

At deployment the server cannot know what the owner wants for a project that
runs on one of its two machines. Recording it open because something runs
would start the other machine's agents at its next registration, unasked.
So nothing is recorded until someone opens or closes the project from a
client that says what that does, creates it with a start, or an
administrator stops it. Until then the project is displayed from what runs,
as today, each machine's part is still reported, and the server neither
starts nor stops it. Its workspace and its Machines row always offer Open,
which records it; while the project already runs, that offer reads as
keeping it open, because recording is all it does. On the first day every
project of the deployment is in this state, so this one click per project
is how following begins.

*Alternative considered:* record every running project as open at the first
instance attach after the upgrade. Rejected for the reason above.

### 3. No new request: `all_instances: true` is Open and Close; anything else reaches one machine at most

The two existing requests keep their shape. With `all_instances: true` they
become "open the project" and "close the project". The named `machine_id`
keeps its use as the machine to start when no machine owns a tab, and as the
machine that places a registration-only project.

Requests that name one machine come from clients that could not have told
the person what opening does: `all_instances: false` (an older web bundle's
Machines page, "Start on \<host\>" and the phone's banner), a tab request
from a viewer without `project_open_state_v1`, and native task launches,
which get no screen change. The rule for all of them is that they never
start or stop a machine other than the one they name:

| Project | Start / tab / task naming one machine | Stop naming one machine |
| --- | --- | --- |
| not recorded | as before the change; nothing recorded | as before; nothing recorded |
| open | start that machine | Close if no other machine owns tabs or runs it, else refused |
| closed | Open if no other machine owns tabs, else refused | stop that machine |

The older project view and the phone's "Open project" send
`all_instances: true` without `project_open_state_v1`. On a not-recorded
project those too act as before the change (today's
`project_start_machines` and `project_stop_machines`) and record nothing;
on a recorded project they are an Open or a Close, which is what their
confirmations already describe.

The closed row applies also when the named machine still has an instance
attached (its stop is under way, or a close could not reach it): a tab
request or a task launch there opens the project or is refused, and is never
admitted into a project that stays recorded closed.
A tab request that names no machine is judged as naming the machine of the
instance it would go to. Opening through such a request takes the authority
to set the state (Decision 6), so an account whose Open would be refused is
refused here with the same answer. The chosen machine is then sent the start
whether or not it owns a tab, because its stop may still be taking effect. A
task chosen for a machine whose daemon is not connected fails as offline and
records nothing.

So on a not-recorded project every button of an older client does exactly
what it did, an older "Start on \<host\>" can retry a machine of an open
project, and the common single-machine case works as before. Only a client
that negotiated `project_open_state_v1` records the state of a project
nobody has recorded. The new web never sends `all_instances: false`.

*Alternative considered:* new `OpenProject` / `CloseProject` variants.
Cleaner names, but every client path, fixture and authorization check
already exists for the two requests, and older clients would still need the
old ones handled.

*Alternative considered:* treat every per-machine start as an Open. It would
start other machines' agents from a button labelled "Start on zoo-006", and
from a phone that shows nothing about it.

### 4. Which machines an open starts is decided from the roster and daemon support

The tab owners are read from the persisted roster: the machines tabs are
recorded for, and the hosts untagged tabs are attributed to. While the
roster has no tab at all, `open_state_machine_id` stands in as the one
owner, so a new, tabless project comes back after its machine restarts.
An unknown roster (a session from before rosters were persisted, which
`project_roster` flattens to empty today) is not an empty one: the owners
are then unknown, an Open starts the named machine only, and no follow step
starts anything.

Whether a machine can run the project beside the others is decided from
that roster and the daemon's `project_instances_v1`, never from which
instances are attached at the moment:

- a daemon with support that owns tabs is started;
- a daemon without support is started only when it is the only tab owner
  (it then runs the project alone, as today); otherwise it is sent nothing
  and reported as not started with the older-APAS sentence.

The attached-instance refusals stay as a second line for what the roster
cannot show (an older CLI that is running and has taken every tab untagged).
This closes a hole a naive reuse of today's checks would open: after a
server restart an older daemon can register before the capable instance has
reattached, pass the attached-instance check, start, and get the capable
instance's session rejected.

### 5. Machines follow at registration, once, not in a loop

In `ws_daemon.rs`, after the daemon's capabilities are set and `Registered`
is written, a spawned task goes through the recorded projects the daemon's
`Register` frame lists. Per project it takes an exclusive per-project
open-state lock (a new keyed mutex like `mobile_task_launch_guard`;
`project_operation_guard` is a shared read permit and serializes nothing),
re-reads the state, and:

- open, this machine is a tab owner, `is_running` is false: sends
  `StartProjectCli` with a `request_id` and the project's policy, after the
  checks of Decisions 4 and 6 and only for an active project. The web
  connection's capability check does not apply; there is no web connection;
- closed, `is_running` is true: sends `StopProjectCli`, after the check of
  Decision 6.

Open, Close, the tab request of Decision 10 and the recording paths of
Decision 9 take the same lock, from reading the state to sending the last
frame, so the server's starts and stops for one project go out in the order
the state changed.

The lock orders what the server sends, not what a machine has finished. A
daemon keeps reporting a project as running, and its instance stays
attached, until the stop it was sent has been carried out, which can take up
to 30 seconds. An Open that changes the record (from closed or from not
recorded) therefore sends the start to every connected tab owner, also one
that still reads as running: the daemon takes its frames in order and
answers "already running" when nothing was stopping. Without this, a Close
followed at once by an Open would leave the project recorded open and
running nowhere. Opening a project that is already open sends nothing to a
machine that runs it.

Every server deploy makes every daemon register again. From the second
deploy on, that is also a retry of each machine that had failed to start
and a stop of each machine still running a closed project. Nothing is sent
for a machine that is in line.

The server starts a machine by itself at no other time. A machine that
refused or failed stays as it is until its daemon registers again or someone
opens the project again; Open on an open project is the retry.

*Alternative considered:* a periodic reconcile loop, or following on every
heartbeat that shows a difference. Either would restart a project that
crashes at start again and again, and would fight a person debugging a
machine. Registration is the one event that means "this machine was away".

### 6. Authority: who may set the state, and whose authority a follow step uses

Runtime authority is per machine (`has_machine_project_runtime_access`: the
daemon lists the project, the project is placed in the machine owner's
cluster, and the account owns the machine or is an allowed member with a
project role). It can be evaluated only for a machine whose daemon has
registered.

- **Setting the state.** Open and Close are accepted from an account that
  passes that check on at least one connected machine listing the project,
  or, when none is connected, that owns the project or holds a placement for
  it. A read-only collaborator can therefore not record a close during a
  deploy and have every machine stopped as it returns.
- **Acting at once.** Each connected machine is started or stopped only
  where the requester passes the check; the others are reported with the
  reason.
- **Following later.** A follow-start is checked against
  `open_state_set_by`, a follow-stop against the closer, at the moment the
  daemon registers. That same check keeps a follow-stop away from another
  account's checkout that carries the same project id, since that project
  is not placed in that account's cluster. A close whose kind is
  administrative is followed on every machine that stop reaches today,
  whoever the operator is.
- **The opener is recorded on the transition.** Opening a project that is
  already open starts what the requester may start and leaves
  `open_state_set_by` and `open_state_machine_id` as they were, so a second
  account's retry does not take the first account's machines out of
  following. They change only through a close and a new open.
- **The closer is replaced only by wider authority.** Closing a project a
  request already closed leaves the recorded closer as it was, unless the
  new requester may use every machine listing the project that the recorded
  closer may use, and at least one more. So an owner's Close after a
  member's makes the owner's machines follow, and a member's Close after the
  owner's takes nothing out of following.
- **A follow-start needs an active account.** The recorded opener must
  still exist and be active when a machine registers. A suspended account's
  open starts nothing later. A suspended account's close still stops
  machines.

One limit remains and is accepted: one recorded actor cannot stand for
machines of two accounts. In a project whose tabs sit on machines of
different owners, a machine the recorded actor may not use is started or
stopped only by a direct request from someone who may, and does not follow
by itself afterwards. Its part says so.

### 7. Each machine's part is computed by the server

The reasons a machine is not running are known only to the server (policy,
access, daemon support) or arrive from the daemon. The server keeps, in
memory, one outcome per project and machine in a registry of its own
(`InstanceStarts` holds a start only while a tab waits for it): filled from
`ProjectRuntimeStarted`, the daemon's `last_error`, a `SessionStart` the
server rejected, the refusing check, and a time limit for a daemon that
never answers, as an older one does not.

From the roster, inventory, attached instances, pending starts and that
outcome it derives each machine's part: `running`, `starting`,
`waiting_offline`, `not_started` with a reason, or `still_running` for a
closed project. Parts are reported for not-recorded projects too.

- `ServerToWeb::ProjectOpenState { project_id, state, set_by, set_at,
  machines: [...] }` goes to viewers that negotiated
  `project_open_state_v1`, on attach and on every change. It is the
  confirmation an Open or Close never had.
- The machine list's project entries gain optional, server-filled
  `open_state`, `part`, `part_reason` and `other_hosts` (as
  `preferred_machine_id` is server-filled today), so the Machines page needs
  no attachment. `SessionInfo` and the mobile bootstrap's session entries
  gain `open_state`. Older web and native builds ignore all of these;
  native is sent no new frame.

The outcome is not durable. After a server restart a reason is unknown until
that machine registers.

### 8. No per-machine control

The owner's rule is that an open project runs every agent it owns. A flag
that keeps one machine out would bring back the half-open project under
another name. What a person can no longer do, and what remains:

- *Run a project on one of two machines that both hold its tabs.* Close the
  tabs on the other machine, or forget them: a machine with no tabs in the
  roster is not started. Forgotten tabs come back only if that machine runs
  the project again, which now takes adding a tab there.
- *Stop one machine's instance to free memory, or restart it alone.* Close
  that machine's tabs, or close and open the project. "Reboot CLI" still
  exists and restarts every project on that host.
- *Stop an instance left empty after its last tab was closed.* It ends when
  the project is closed, or with the daemon; it is not started again.
- *Another account's project on your machine.* Remove the machine from that
  member's allowlist, suspend the project, or stop its runtime, which now
  closes it.

This is the proposal's deliberate loss of function and is marked BREAKING
there.

### 9. Other paths record the state

- Suspension and the operator's and administrator's "Stop runtime" record
  closed, kind administrative, with the actor. `stop_project_runtime` is a
  `SessionManager` method with neither a database nor an actor, so the
  recording is done by its callers in `routes/cluster.rs` and
  `routes/admin.rs`, and for a suspension inside `set_project_lifecycle`'s
  transaction. Otherwise the next registration would start an open project
  the operator just stopped. Reactivation keeps starting nothing.
- `begin_project_deletion` records closed, and a follow-start requires an
  active project, so a host that returns during a deletion is not started.
- A creation that starts the project records open, so a new project is
  followed from its first day. Shared provisioning records it at finalize,
  where the server sends the start. For a trusted owner's create-and-start
  the server keeps nothing and no project row exists until the first
  `SessionStart`, so the state is recorded there: a `SessionStart` that
  creates the project's row records it open by that account, naming that
  machine. A `SessionStart` for a project that already has a row, as after
  "Clone here" or `apas register` joining by remote, records nothing.
- Open and close are audited, with the actor and the machines asked. A start
  or stop the follow step sends at a registration is audited too, with the
  recorded actor, that machine and the source "registration"; a registration
  that sends nothing writes nothing.

### 10. A tab that needs a machine started

For a viewer with `project_open_state_v1`, `add_pane_on_machine` first runs
its existing `start_target` checks for the chosen machine. Only when they
pass does it open the project as the requester (starting the other tab
owners as an Open does) and start the chosen machine; the correlated
`AddPaneResult` is unchanged. A refused tab request therefore starts
nothing anywhere. The machine choice's "Starts the project first" group
becomes "Opens the project", with the other machines named.

For other viewers and for native launches, the table in Decision 3 applies.

### 11. Wording

- State: **Open** / **Closed** on the Machines page, sidebar and phone
  lists, replacing Running / Stopped for the project. "r of t projects
  running" on a machine card keeps counting what that machine runs.
- A machine's part: "Running", "Starting…", "Waiting for zoo-006 (offline)",
  "Not started: \<reason\>", "No tabs here", "Still running".
- A tab on a machine that is not running the project is refused with
  "\<host\> is not running this project. Open the project again to start it
  there." while that daemon is connected, and "\<host\> is not running this
  project because it is offline." while it is not (both were "… Start it on
  \<host\> first."). Native composes the old sentence itself and is updated
  in its next build.
- Pane dots keep "Not running on \<host\>" for such a tab and "Not running"
  for a closed project that runs nowhere. Tabs of a machine that still runs
  a closed project keep their real status, and Close is offered again.
- The Open of a not-recorded project that already runs reads "Keep open",
  with "Its tabs start again on \<hosts\> after those machines have been
  away."

### 12. Closing deletes nothing; deleting is a control of its own

The owner's rule (2026-11-16): closing a project must not delete anything,
and deletion is offered explicitly.

- **Close.** A close sends what a project stop sends today
  (`StopProjectCli`): the daemon ends the project's task and its pane hosts.
  It removes no tab from `.apas` or the roster, no worktree, no provider
  conversation and no registration, and the server removes no session,
  message, share, policy or saved prompt. No path of this change may call
  `ForgetMachinePanes`, a pane close or a deletion as part of a close. The
  Close confirmation ends with "Nothing is deleted. Its tabs come back when
  you open it again." A server test pins that the roster and the session's
  history are the same before a close and after the open that follows.
- **Delete.** The deletion that exists (`POST /projects/:id/delete`, owner
  only, `begin_project_deletion`, then the resumable cleanup) is unchanged
  on the server apart from recording the project closed (Decision 9). It is
  hard to find and hard to use: a "Danger zone" at the bottom of the
  project-access dialog that asks for the project's UUID. The web gains one
  shared `DeleteProjectDialog`, opened from **Delete project…** beside Close
  in the workspace's project controls and from the cluster page's project
  panel, for the project's owner only. The dialog:
  - says what is erased (the project on the APAS server: its tabs,
    conversations shown in APAS, history, usage, sharing, invitations and
    policy) and that it cannot be undone;
  - says what stays: the folder and its files on every machine, and that
    each of those machines keeps the folder registered, so it stays in the
    lists as a registered folder, and opening it there creates a new, empty
    project;
  - says that agents running in the project are stopped first, naming the
    machines;
  - is confirmed by typing the project's name; the client then sends the
    project id as the server's confirmation value;
  - shows "Deleting…" until the server reports the deletion done, as the
    old control did, and sends nothing twice.
  The project-access dialog's danger zone becomes a button that opens the
  same dialog. A user who is not the owner is offered no Delete; the
  existing "leave project" stays where it is.
- **A machine that is offline during a deletion** is not stopped by it, as
  today, and when it returns still running the project its `SessionStart`
  creates a new project under the same id, which Decision 9 records as
  open. The dialog names an offline machine that owns tabs and says the
  project comes back from it when it reconnects still running.

*Alternative considered:* have Delete also remove the registration on each
machine, so the folder leaves the lists. It needs a new daemon request, a
capability and a daemon reboot per machine, and the lifecycle spec says a
deletion leaves local APAS configuration alone. Not done here.

## Risks / Trade-offs

- [A reconnecting machine has its agents ended] A project closed while a
  machine was cut off is stopped the moment that machine registers. That is
  what Close means, but it now happens minutes or days later. → The Close
  confirmation names every machine that owns tabs, also offline ones, and
  the project reads Closed everywhere in between.
- [Starts in a burst after a machine reboot] Every open project with tabs
  there starts when the daemon registers, as a daemon's own resume does
  today, with each project's terminal tabs spawned one after another. → The
  server sends the starts in sequence; a deliberate stagger is left open.
- [An external `apas --headless` debugging run of a closed project is
  stopped at the daemon's next registration] The daemon reports such a run
  as running and its stop kills it. → Documented in `AGENTS.md`: open the
  project, or debug a project that is not recorded closed.
- [A stop reported done while the project still ends] The daemon gives up
  waiting after 30 s and reports success. → The machine's part comes from
  its next inventory, so it reads "Still running" and Close can be sent
  again.
- [A slow re-registration] A daemon's own channel can take about 30 s to
  re-register after a reboot. Following happens at registration, so it is
  that late. A Close in that window, lost today, is now caught.
- [Projects spanning machines of two accounts] See the limit in Decision 6.
- [Legacy session rows] A session whose id is not the project id is not
  seen by the roster lookup until it is attached, as today; such a
  project's tab owners may be unknown at registration and it is then not
  followed until someone opens it.
- [Losing per-machine control] See Decision 8.
- [Many pinned tests change] The web tests that assert the Machines page
  never sends `all_instances`, and the "Start on \<host\>" tests, are
  rewritten, not deleted without replacement.
- [An older server after rollback] It ignores the columns and behaves as
  before, so people start and stop projects by hand again while the recorded
  states go stale. The next start of a server with this change would then
  follow those stale states at every registration: start projects someone
  stopped by hand, and stop projects someone started. The rollback therefore
  clears the recorded states (Migration Plan).
- [Scenario titles] OpenSpec keeps a modified requirement's scenario titles.
  "Start one machine from the Machines page", "A project that is not
  running" and "A project that is running" keep their titles with new
  content.

## Migration Plan

1. **Server**, with the shared types and regenerated schemas. The columns
   are added on first start; every project is not recorded. Older web
   clients keep working by Decision 3: on a not-recorded project every
   button of theirs does what it did and records nothing.
2. **Web.** From here the Machines page and the workspace show the state,
   and the first Open or Close of each project records it.
3. **Native** needs no release for the protocol. Check an existing build
   stays connected while a project is opened and closed. Its refusal
   sentence changes with its next build.
4. No daemon or agent restart. Nothing is started or stopped by this
   deployment, because no project is recorded yet. Later server deploys
   bring machines in line as their daemons register again (Decision 5).

Rollback: redeploy the previous web, then the previous server. The columns
stay and the older server ignores them. No stop is needed first, because
nothing durable depends on the new server except the follow step, which
simply no longer happens. **Clear the recorded states as part of every
server rollback**, with the server stopped, also when the rollback is the
deploy procedure's "restore the backed-up binary":

```sql
UPDATE projects SET open_state = NULL, open_state_set_by = NULL,
  open_state_set_kind = NULL, open_state_set_at = NULL,
  open_state_machine_id = NULL;
```

Every project is then not recorded again, as after the first deployment.
Without this, the next deploy of a server with this change would start and
stop machines from states recorded before the rollback.

Smoke on the two-host harness before production: open with one machine
offline and bring it back; close with one daemon frozen and release it;
close during a daemon replacement; an older daemon as one of two tab owners
that registers first after a server restart; a server restart between an
open and a machine's return; an older web bundle's per-machine buttons on a
not-recorded, an open and a closed two-machine project.

## Open Questions

- Whether follow-starts after a machine reboot should be spaced out (for
  example one project every few seconds). It changes no behaviour a person
  sees except load, and can be added after the first real reboot is
  measured.
- Whether the phone's project screen should gain Close. It has none today
  and this change adds none.
- Whether Delete project should also take the folder out of each machine's
  registry, so a deleted project leaves the lists entirely (Decision 12).
