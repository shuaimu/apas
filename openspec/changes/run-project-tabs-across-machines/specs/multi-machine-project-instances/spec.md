## Purpose

Let one project run tabs on several machines at once — on a shared filesystem
or in separate checkouts — under one session, without any instance erasing,
hijacking or disconnecting another's tabs, and without changing behaviour for
older components.

## ADDED Requirements

### Requirement: A project runs at most one instance per machine under one session

A project SHALL be able to run one instance on each of several machines at the same time. Every instance SHALL attach to the project's single existing session, so the project keeps one history, one sharing and policy unit, and one project-list entry. A reconnect from the same machine SHALL replace only that machine's attachment. The earliest attached instance SHALL be the primary, supplying the session's displayed host and the default target for controls that name no machine.

#### Scenario: Start a second instance on a shared home
- **WHEN** a project is running on zoo-005 and its owner starts it on zoo-006, which sees the same directory
- **THEN** zoo-006 runs an instance attached to the same session
- **AND** the zoo-005 instance and its tabs keep running unchanged

#### Scenario: Reconnect from the same machine
- **WHEN** zoo-006's instance reconnects after a transport loss
- **THEN** it replaces only zoo-006's attachment and zoo-005's attachment is untouched

### Requirement: Every pane belongs to exactly one machine

Each pane reported by an instance with multi-instance support SHALL record the machine that runs it. The server SHALL set that record from the authenticated instance that reports the pane, not from the pane data the instance supplies, and SHALL record no machine for panes reported by an instance without that support. A pane with no recorded machine SHALL belong to no machine until an instance with multi-instance support reports it, which then adopts it. The server SHALL attribute such a pane to the host it last recorded the pane under, for display and routing only, and SHALL NOT refuse or drop a pane on account of that host when an instance with support reports it. A control or close for such a pane SHALL go to the attached instance on its attributed host, else to the sole attached instance when that instance lacks support, else to the primary instance when the pane has no attributed host. A control or close for a pane recorded for a machine SHALL go only to that machine's instance, even while an instance without support is the sole instance. While any instance of the project is attached, a control or close that cannot go to an instance this way SHALL be refused as stopped on the pane's machine or attributed host; with no instance attached it SHALL fail as it does today. A pane's history, terminal and controls SHALL stay with its machine for the pane's lifetime. No instance SHALL restore or adopt a pane recorded for another machine, even while that machine is stopped. A tab whose machine is not running SHALL be shown as stopped on that machine and SHALL NOT be closable until that machine runs the project. When the server refuses typed input that carries the client's id for that message because of its tab's machine, which is not running the project or which the sender may not use, it SHALL also send the sending connection a refusal correlated to that input and carrying the reason, in addition to the error it sends today, if that connection advertised multi-instance support; such a correlated answer SHALL only ever be a refusal. When a close for a tab with no recorded machine is delivered to an instance with multi-instance support, the tab SHALL leave the roster, although that instance never reports it. The owner SHALL be able to forget all tabs of a stopped machine after confirmation, and SHALL be told that they come back if that machine runs the project again. Forgetting SHALL change only the server's roster: no instance SHALL be told and no project metadata SHALL change, so that machine restores and reports those tabs, with their ids, when it runs the project again. The server SHALL NOT allocate the ids of forgotten tabs, or of untagged tabs dropped on a close, to new tabs while any other id is free. Starting a stopped project from the project view SHALL start an instance on every online machine with multi-instance support that owns tabs, or on the project view's resolved machine when no machine qualifies.

#### Scenario: Pane ownership is assigned by the server
- **WHEN** an instance on zoo-006 reports a pane whose data names zoo-005
- **THEN** the server records the pane as zoo-006's

#### Scenario: Legacy untagged panes
- **WHEN** a persisted roster from before multi-machine instances holds panes with no machine recorded, and the session was last recorded on zoo-005
- **THEN** those tabs are shown under zoo-005 and their controls reach zoo-005's instance whenever it is attached
- **AND** the first instance with multi-instance support that reports them records them as its own

#### Scenario: A joined checkout starting first keeps a stopped host's tabs
- **WHEN** a project's untagged tabs were last recorded on zoo-005, zoo-005 is stopped, and a laptop checkout joined to the project starts first and reports no tabs
- **THEN** zoo-005's tabs stay in the project, shown as stopped on zoo-005
- **AND** a control or close for one of them is refused with "zoo-005 is not running this project. Start it on zoo-005 first."

#### Scenario: Tabs reported by an older CLI stay adoptable
- **WHEN** a CLI without multi-instance support reports its tabs
- **THEN** the server records their host but no machine
- **AND** an instance with support on a shared home that later reports those tabs records them as its own

#### Scenario: Untagged tab with no known host
- **WHEN** a tab has no recorded machine and no attributed host
- **THEN** its controls go to the primary instance

#### Scenario: A sole older CLI does not receive another machine's tab
- **WHEN** a CLI without multi-instance support on zoo-005 is the project's only instance and a tab is recorded for a stopped laptop
- **THEN** a control for that tab is refused with "laptop is not running this project. Start it on laptop first."
- **AND** nothing is sent to the CLI on zoo-005

#### Scenario: Close an untagged tab that no instance runs
- **WHEN** a tab with no recorded machine is attributed to zoo-005, whose instance has multi-instance support and runs the project without reporting that tab, and the user closes it
- **THEN** the close is delivered to zoo-005's instance
- **AND** the tab leaves the project roster

#### Scenario: Tabs stay on their machine
- **WHEN** a project's tabs belong to zoo-005, zoo-005 is stopped, and the project starts on zoo-006 in the same directory
- **THEN** zoo-006 starts with none of zoo-005's tabs
- **AND** zoo-005's tabs remain listed as stopped on zoo-005 until zoo-005 runs the project again

#### Scenario: Close a tab whose machine is not running
- **WHEN** a user closes a tab owned by zoo-006 while the project runs on zoo-005 and not on zoo-006
- **THEN** the request is refused with "zoo-006 is not running this project. Start it on zoo-006 first."

#### Scenario: Send to a tab whose machine is not running
- **WHEN** a native client that advertised multi-instance support sends a message to a tab owned by zoo-006 while the project runs on zoo-005 and not on zoo-006
- **THEN** the message is refused with "zoo-006 is not running this project. Start it on zoo-006 first."
- **AND** the refusal is correlated to that message, so the client shows the reason instead of resending the message until it gives up

#### Scenario: Forget a stopped machine's tabs
- **WHEN** the owner confirms forgetting a stopped machine's tabs
- **THEN** those tabs, including untagged tabs attributed to that machine's host, leave the project roster
- **AND** no instance is told and no project metadata changes
- **AND** the confirmation says the tabs come back if that machine runs the project again

#### Scenario: Forgotten tabs come back
- **WHEN** zoo-006's tabs were forgotten, a tab was added on zoo-005 since, and zoo-006 then runs the project again from the same shared home
- **THEN** the tab added on zoo-005 has an id that none of zoo-006's tabs had
- **AND** zoo-006 restores and reports its tabs, which are listed again with their ids

#### Scenario: Forget a machine that is running
- **WHEN** the owner asks to forget the tabs of a machine with a running instance of the project, or whose host runs the project with a CLI without multi-instance support
- **THEN** the request is refused and the tabs stay

#### Scenario: Start a project whose tabs span machines
- **WHEN** a stopped project has tabs owned by zoo-005 and zoo-006 and the user starts the project from the project view
- **THEN** an instance starts on each of those machines that is online and has multi-instance support

#### Scenario: Start one machine from the Machines page
- **WHEN** a user starts the project on zoo-006 from the Machines page while zoo-005 also owns tabs
- **THEN** only zoo-006 starts an instance

#### Scenario: Close the project from the project view
- **WHEN** a project runs on zoo-005 and zoo-006 and the user closes it from the project view
- **THEN** both instances stop

### Requirement: Instance rosters merge without erasing each other

The roster report of an instance with multi-instance support SHALL replace only the panes recorded for that instance's machine, and SHALL adopt the reported panes that have no recorded machine. A reported pane recorded for another machine SHALL be dropped and logged. Panes with no recorded machine that the instance did not report SHALL be left in place. An empty report SHALL mean that instance has no panes and SHALL leave every other pane in place; an absent report SHALL change nothing. A report from an instance without multi-instance support, which is only ever the sole instance, SHALL take each pane it reports in place of that id's entry, with no recorded machine and its host as the attributed host, whatever machine the entry recorded, and SHALL remove the panes it did not report that have no recorded machine or are recorded for its own machine. Panes recorded for another machine that it did not report SHALL stay, shown as stopped on that machine, with their ids still allocated. The server SHALL keep one project-wide tab order and persist each pane's machine and host with the roster so stopped instances' tabs survive server restarts.

#### Scenario: One machine closes its last tab
- **WHEN** zoo-006's instance reports an empty roster while zoo-005 has two tabs
- **THEN** the project shows zoo-005's two tabs and none from zoo-006

#### Scenario: Server restart keeps hosts
- **WHEN** the server restarts while a stopped zoo-006 instance has saved tabs
- **THEN** those tabs are listed with zoo-006 as their host

#### Scenario: An instance reports another machine's pane
- **WHEN** zoo-006's instance reports a pane that the roster records for zoo-005
- **THEN** the pane stays zoo-005's and the server logs the conflict

#### Scenario: An older CLI's report keeps a separate checkout's tabs
- **WHEN** a stopped laptop checkout has tabs recorded for the laptop, and a CLI without multi-instance support on zoo-005, the project's only instance, reports its roster
- **THEN** the laptop's tabs stay in the project, shown as stopped on the laptop
- **AND** the tabs zoo-005 reported replace every other tab

### Requirement: Controls reach the instance that owns the pane

The server SHALL deliver every pane-scoped control, including terminal input and resize, question and plan answers, pause, resume, reboot, close, rename, model and effort changes, diff and pull-request requests, and work-summary generation, to the instance that owns the pane and to no other. It SHALL deliver an answer that names no pane to the owner of the pane whose decision is pending, and input that names no pane and answers no decision to the owner of the legacy pane it means, else the primary instance. It SHALL deliver a reboot or lifecycle request to the instance on the machine the request names, else the primary instance. It SHALL send project-wide updates to every attached instance. It SHALL evaluate provider capability and runtime access against the owning machine. It SHALL accept pane-scoped output, state and status only from an attached instance; for a pane recorded for a machine only from that machine's instance, even while an instance without multi-instance support is the sole instance; and for a new tab the roster does not list yet, whose id it reserved for a machine, only from that machine's instance. When a project membership is revoked, the server SHALL reject every attached instance that the revocation applies to, not only the primary.

#### Scenario: Type into a tab on another machine
- **WHEN** a user types into a tab owned by zoo-006 while zoo-005 is the primary
- **THEN** the input reaches zoo-006's instance only

#### Scenario: Reject foreign pane traffic
- **WHEN** a CLI that does not own a pane sends terminal output for it
- **THEN** the server discards the output and logs the mismatch

#### Scenario: Project-wide policy change
- **WHEN** the owner changes project flags
- **THEN** every attached instance receives the change

#### Scenario: Answer a question on another machine
- **WHEN** a tab owned by zoo-006 is waiting on a question and the user answers it while zoo-005 is the primary
- **THEN** the answer reaches zoo-006's instance only

#### Scenario: Reboot one machine's instance
- **WHEN** a user reboots the project's instance on zoo-006
- **THEN** the request reaches zoo-006's instance and zoo-005's instance keeps running

### Requirement: An instance leaving affects only its own tabs

When an instance disconnects or stops, only its tabs SHALL become disconnected or stopped and only their statuses SHALL clear. The session SHALL become inactive only when no instance remains. If the primary leaves, the next earliest instance SHALL become primary. An instance ending its session SHALL end the session only if it is the last instance. Each instance's lifecycle inventory SHALL be kept and reported per machine, so one instance's report or departure does not replace another's. A viewer that did not advertise multi-instance support, which keeps one inventory per session and whose reboot requests reach the primary instance, SHALL receive only the primary's inventory, and the new primary's after a handover.

#### Scenario: Secondary machine goes offline
- **WHEN** zoo-006 loses its connection while zoo-005 stays connected
- **THEN** zoo-006's tabs show disconnected, zoo-005's tabs remain live, and the project stays active

#### Scenario: Lifecycle per machine
- **WHEN** zoo-005 and zoo-006 both report lifecycle inventories and a viewer that advertised multi-instance support opens the project
- **THEN** the viewer receives one inventory for each machine
- **AND** a reboot confirmation for zoo-006 counts only zoo-006's tabs

#### Scenario: Lifecycle for an older viewer
- **WHEN** zoo-005 is the primary, zoo-006 also runs the project, and a viewer that did not advertise multi-instance support opens it
- **THEN** that viewer receives only zoo-005's inventory, and its reboot request reaches zoo-005's instance

#### Scenario: Last instance leaves
- **WHEN** the only remaining instance disconnects
- **THEN** the session becomes inactive as it does today

### Requirement: Instances sharing a directory never overwrite each other's tabs

When several instances use one project directory, every write of project metadata SHALL hold an exclusive lock, re-read the file, and change only what the writer owns: its own panes, or the single project-level field being changed. Project-level fields SHALL never be rewritten from an instance's stale in-memory copy. An instance SHALL restore only its own panes. Untagged panes SHALL be adopted only at instance start, under the lock, and only by the machine holding the earliest live instance claim on the project, with ties broken by machine id. No instance SHALL adopt them while another host runs the project without multi-instance support, and a run with no claim of its own SHALL NOT adopt them while any other machine runs the project. Forgetting a machine's tabs SHALL leave the metadata unchanged, so that machine restores them when it runs the project again. A periodic or background read SHALL never delete or regenerate project metadata.

#### Scenario: Concurrent saves on a shared home
- **WHEN** zoo-005 adds a tab and zoo-006 renames one of its own tabs at the same time
- **THEN** the metadata file afterwards contains both changes

#### Scenario: Second instance starts empty
- **WHEN** a project with untagged legacy panes is running on zoo-005 and an instance starts on zoo-006
- **THEN** zoo-006 restores no panes and leaves the legacy panes to zoo-005

#### Scenario: Two instances start together
- **WHEN** zoo-005 and zoo-006 start a project with untagged legacy panes at the same moment, and zoo-005 claimed it first
- **THEN** zoo-005 adopts every legacy pane and zoo-006 starts with none, whichever of them reaches the metadata first

#### Scenario: A stopped machine's entries survive a forget
- **WHEN** zoo-006 is stopped, its tabs are forgotten, and zoo-005 keeps saving its own tabs to the shared metadata
- **THEN** zoo-006's entries stay in the metadata, and zoo-006 restores them when it runs the project again

#### Scenario: Unparseable metadata during a poll
- **WHEN** a periodic read cannot parse the metadata
- **THEN** the instance keeps its last good copy and the file is not deleted

### Requirement: Claims are per machine, released on stop, and exclude older daemons

A capable daemon SHALL claim a project per machine and SHALL release that claim when the instance stops for any reason. While any capable instance runs a project, a daemon without multi-instance support SHALL be unable to start it. A stopped project SHALL be startable on another machine without waiting for the first machine's daemon to exit. A daemon that refuses to start a project because of a claim SHALL report the reason as the project's last error rather than skip it silently.

#### Scenario: Stop on one machine, start on another
- **WHEN** a project is stopped on zoo-005 and then started on zoo-006
- **THEN** zoo-006 starts it immediately

#### Scenario: Older daemon cannot join
- **WHEN** a capable instance runs a project and an older daemon is asked to start it
- **THEN** the older daemon does not start it

#### Scenario: A claim refusal is visible
- **WHEN** a capable daemon is asked to start a project whose claim another host holds exclusively
- **THEN** it does not start the project and reports the reason as the project's last error

### Requirement: Mixed fleets keep single-machine behaviour

A CLI or daemon that does not advertise multi-instance support SHALL behave as today when no other machine runs the project. Its session start SHALL be rejected with an update instruction when another machine's instance is attached. Machine choice for new tabs SHALL be offered only for machines whose daemon is connected and advertises support, and the server SHALL refuse a request naming any other machine with a correlated reason. Machine entries SHALL distinguish a connected daemon that lacks support, which needs a reboot to update, from an offline one. A host configured with an explicit shared machine id SHALL not advertise support. The server SHALL confirm support to a daemon that advertised it when the daemon registers. A daemon that advertised support SHALL rely on it only on a connection where the server confirmed it: on any other connection it SHALL handle every start request as daemons did before multi-machine instances, refusing a project another live host runs rather than starting an instance beside it. It SHALL decide again at every registration and SHALL NOT stop a running instance because of it. A web or native client SHALL receive the frame listing a session's instances, and correlated refusals of its typed input, only if it advertised support, and a tab-launch result only for a request it sent with a correlation id. Starting and stopping a project on every machine SHALL be requested explicitly, so the Machines page's per-machine controls and older clients act on one machine.

#### Scenario: Older CLI meets a running instance
- **WHEN** an older CLI starts a session for a project already running on another machine
- **THEN** the server rejects it and tells the user to update that machine

#### Scenario: Older target machine
- **WHEN** a user requests a new tab on a machine whose daemon lacks support
- **THEN** the tab is not created and the user is told to reboot that machine to update

#### Scenario: Connected older daemon in the machine choice
- **WHEN** zoo-006's daemon is connected but lacks support and the user opens the new-tab machine choice
- **THEN** zoo-006 is shown disabled with "Reboot zoo-006 to update", not as offline

#### Scenario: Older native build stays connected
- **WHEN** a native build that does not advertise support views a project running on two machines
- **THEN** it receives no instance-list frame and stays connected
- **AND** the session list and pane records it already parses carry each instance's and tab's host
- **AND** a message it sends to a stopped machine's tab is refused only with the error it already understands

#### Scenario: Capable daemon on an older server
- **WHEN** a daemon with multi-instance support is registered with a server that did not confirm that support, and it is asked to start a project another live host runs
- **THEN** it does not start an instance beside that host
- **AND** once a server that confirms support registers it, it starts instances beside other machines again

### Requirement: Users choose a machine when creating a tab

When a user can launch on more than one eligible machine, or the only eligible machine is not running the project, the new-tab flow SHALL let them choose, and SHALL say what choosing a machine does (start the project there, or clone it there) before anything starts. The choices are the project's running instances, machines where the project is registered, and owned online machines without it. Disabled choices SHALL state why. The default SHALL be the selected tab's machine, and a machine the project would be cloned onto SHALL never be preselected. Choosing a registered machine that is not running the project SHALL start the instance there and then create the tab. A request naming a machine SHALL never create the tab on another machine. Each request SHALL receive a correlated result of routed, starting, or failed, sent only to the connection that made the request; routed SHALL mean delivered to the instance, and starting SHALL always be followed by routed or failed. The start of a stopped instance SHALL be correlated with the daemon's reported outcome, so a refused or failed start fails the request with its reason, and a session start of that instance that the server refuses SHALL fail the request at once with that refusal's reason. Pane ids SHALL be unique across machines, including concurrent requests. Lost confirmations SHALL be reported as unconfirmed rather than replayed.

#### Scenario: New tab on a running instance
- **WHEN** a user picks zoo-006, where the project is running
- **THEN** the tab is created on zoo-006 and shows zoo-006 as its host

#### Scenario: New tab on a registered, stopped machine
- **WHEN** a user picks zoo-006, where the project is registered but not running
- **THEN** the result is starting, zoo-006 starts its instance, and the tab is created there

#### Scenario: Start fails
- **WHEN** the instance on the chosen machine fails to start within the time limit
- **THEN** the result is failed with the reason and no tab is created

#### Scenario: Start refused by the daemon
- **WHEN** the chosen machine's daemon refuses to start the project, for example because its policy is suspended
- **THEN** the result is failed with the daemon's reason and no tab is created

#### Scenario: Concurrent requests
- **WHEN** two users add tabs to the project at the same moment on different machines
- **THEN** the two tabs receive different pane ids

#### Scenario: Confirmation lost while starting
- **WHEN** the server restarts, or the requesting connection drops, while a launch is starting
- **THEN** the client shows the launch as unconfirmed and does not resend it

#### Scenario: Single eligible machine already running the project
- **WHEN** only one machine is eligible and it already runs the project
- **THEN** the new-tab flow is unchanged

#### Scenario: Single eligible machine that would start the project
- **WHEN** the project is stopped and the only eligible machine is one where it is registered
- **THEN** the new-tab flow shows that machine, preselected, as starting the project first
- **AND** choosing a tab type starts the project there and then creates the tab

#### Scenario: Single eligible machine that would be cloned
- **WHEN** the only eligible machine is an owned machine without the project
- **THEN** the new-tab flow offers it as a clone, preselects nothing, and creates nothing until the user picks that machine and confirms the clone

### Requirement: Native task launches target the chosen machine

A native task launch SHALL keep its existing meaning until an instance with multi-instance support runs the project: start the project on the chosen machine when nothing runs it, otherwise add the task to the running session. Once such an instance runs the project, the launch SHALL run the task on the chosen machine, starting a registered, stopped instance there first when that machine's daemon has support, and SHALL fail with the reason rather than run the task on another machine.

#### Scenario: Before any capable instance runs the project
- **WHEN** only an instance without multi-instance support runs the project, on zoo-005, and a user launches a native task choosing zoo-006
- **THEN** the task is added to the running session, as it is today

#### Scenario: Launch on a stopped capable machine
- **WHEN** a capable instance runs the project on zoo-005 and a user launches a native task choosing zoo-006, where the project is registered, stopped, and the daemon has support
- **THEN** zoo-006 starts its instance and the task runs there

#### Scenario: Chosen machine cannot run an instance
- **WHEN** a capable instance runs the project on zoo-005 and a user launches a native task choosing a machine whose daemon lacks support
- **THEN** the launch fails with the reason and no task starts on any other machine

### Requirement: Tabs and project lists show where work runs

Once a project's tabs span more than one machine, the tab bar SHALL show each tab's host and whether its machine's instance is connected. A tab with no recorded machine SHALL be shown under its attributed host, and without a host when it has none. The project list SHALL keep one entry per project, show the primary host with the number of additional running instances, and include every machine's panes in its pane status. Mobile web and native SHALL show each tab's host. The server SHALL tell each viewer, per instance, whether that viewer may use the instance's runtime, and clients SHALL disable the tabs and machine choice of an instance the viewer may not use and say why.

#### Scenario: Two machines
- **WHEN** a project has tabs on zoo-005 and zoo-006
- **THEN** each tab shows its host and the sidebar row shows "zoo-005 +1"

#### Scenario: Instance the viewer cannot use
- **WHEN** a viewer has project access but no runtime access to zoo-006, which runs one of the project's instances
- **THEN** zoo-006's tabs and its machine choice are disabled for that viewer with the reason
- **AND** zoo-005's tabs stay usable

### Requirement: A checkout can join an existing project by git remote

`apas register` SHALL compare the checkout's canonical git remote with projects the user owns or hosts, including registered projects that have never started, and excluding any project whose session belongs to another account, since no checkout of the user's could start as an instance of it. When exactly one matches and the checkout has no project history, it SHALL offer to join that project. When several match, it SHALL list them and require an explicit choice. The checkout's own current identity SHALL never be offered as a match. Joining SHALL give the checkout the existing project's identity with no panes, replacing only a pristine local identity: one with no panes for which the server holds no session, whatever remote that project recorded. An explicit project option SHALL join directly, and a separate-project option SHALL skip matching. Non-interactive runs SHALL require explicit confirmation. Joining SHALL be refused when the local registry already lists that project at a different folder. A project registered on the owner's own machine but not yet placed in the owner's cluster, as when it has never started anywhere, SHALL be startable there by that owner from the Machines page or a native task launch: the start SHALL first place it in the owner's cluster, creating its project record if missing, as that instance's own session start would. A cluster member SHALL still need a role in the project first.

#### Scenario: Laptop joins the zoo project
- **WHEN** the owner runs `apas register` in a never-started laptop checkout whose remote matches one existing project
- **THEN** after confirmation the checkout carries that project's id and appears as a machine for that project

#### Scenario: Checkout with history is not re-identified
- **WHEN** the checkout's existing project already has panes or a session
- **THEN** it is registered under its existing identity without a join prompt

#### Scenario: History without a recorded remote is not re-identified
- **WHEN** the checkout's project has a server session whose recorded remote is missing or different, and the checkout has no panes
- **THEN** it is registered under its existing identity without a join prompt

#### Scenario: Own identity is not offered
- **WHEN** the checkout's own current identity is among the projects carrying its remote
- **THEN** that identity is not offered as a project to join

#### Scenario: Ambiguous match
- **WHEN** several existing projects share the remote
- **THEN** the command lists them and requires `--project`

#### Scenario: Start a checkout that was only registered
- **WHEN** the owner registered a checkout with `apas register`, it has never started anywhere, and the owner starts it on that machine from the Machines page
- **THEN** the project starts there rather than being refused with "Machine not found"

### Requirement: An owned machine without the project can clone and join

For an online machine the requester owns that does not have the project and whose daemon advertises multi-instance support, the new-tab flow SHALL offer to clone the repository there as an instance of the same project. Only a requester who hosts the project and owns the machine SHALL be able to request it. The request SHALL be refused for a project whose session belongs to another account, and for a machine that already has the project. The clone SHALL receive the existing project identity and no panes. It SHALL NOT create a new branch or start automatically. The clone SHALL be acknowledged only after the machine's inventory lists the new checkout. The client SHALL then request the tab on that machine as an ordinary correlated launch, which starts the instance and creates the tab. If the clone's acknowledgement is lost, the client SHALL report the launch as unconfirmed and SHALL NOT resend the clone. A failure before registration SHALL remove the partial clone. Shared-cluster provisioning SHALL be unchanged.

#### Scenario: Clone onto the laptop
- **WHEN** the owner picks their laptop, which lacks the project, and confirms cloning
- **THEN** the laptop clones the repository, registers it under the existing project id, starts the instance, and creates the tab there

#### Scenario: Clone acknowledgement lost
- **WHEN** the connection drops after a clone-to-join request was sent and before its acknowledgement arrived
- **THEN** the client shows the launch as unconfirmed and does not send the clone again

#### Scenario: Machine without support
- **WHEN** the owner's laptop is online but its daemon lacks multi-instance support
- **THEN** cloning there to join the project is not offered
