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

Each pane SHALL record the machine that runs it. The server SHALL set that record from the authenticated instance that reports the pane, not from the pane data the instance supplies. A pane's history, terminal and controls SHALL stay with its machine for the pane's lifetime. No instance SHALL restore or adopt a pane recorded for another machine, even while that machine is stopped. A tab whose machine is not running SHALL be shown as stopped on that machine and SHALL NOT be closable until that machine runs the project. The owner SHALL be able to forget all tabs of a stopped machine after confirmation. Starting a stopped project SHALL start an instance on every online machine that owns tabs.

#### Scenario: Pane ownership is assigned by the server
- **WHEN** an instance on zoo-006 reports a pane whose data names zoo-005
- **THEN** the server records the pane as zoo-006's

#### Scenario: Legacy untagged panes
- **WHEN** a persisted roster contains panes with no machine recorded
- **THEN** they belong to the primary instance

#### Scenario: Tabs stay on their machine
- **WHEN** a project's tabs belong to zoo-005, zoo-005 is stopped, and the project starts on zoo-006 in the same directory
- **THEN** zoo-006 starts with none of zoo-005's tabs
- **AND** zoo-005's tabs remain listed as stopped on zoo-005 until zoo-005 runs the project again

#### Scenario: Close a tab whose machine is not running
- **WHEN** a user closes a tab whose machine has no running instance
- **THEN** the request is refused with an instruction to start the project on that machine

#### Scenario: Forget a machine that will not return
- **WHEN** the owner confirms forgetting a stopped machine's tabs
- **THEN** those tabs leave the project roster, and running instances sharing the same project metadata remove them from it

#### Scenario: Start a project whose tabs span machines
- **WHEN** a stopped project has tabs owned by zoo-005 and zoo-006 and the user starts the project
- **THEN** an instance starts on each of those machines that is online

### Requirement: Instance rosters merge without erasing each other

An instance's roster report SHALL replace only the panes that instance owns. An empty report SHALL mean that instance has no panes and SHALL leave other machines' panes in place; an absent report SHALL change nothing. The server SHALL keep one project-wide tab order and persist each pane's machine with the roster so stopped instances' tabs survive server restarts.

#### Scenario: One machine closes its last tab
- **WHEN** zoo-006's instance reports an empty roster while zoo-005 has two tabs
- **THEN** the project shows zoo-005's two tabs and none from zoo-006

#### Scenario: Server restart keeps hosts
- **WHEN** the server restarts while a stopped zoo-006 instance has saved tabs
- **THEN** those tabs are listed with zoo-006 as their host

### Requirement: Controls reach the instance that owns the pane

The server SHALL deliver every pane-scoped control, including terminal input and resize, question and plan answers, pause, resume, reboot, close, rename, model and effort changes, diff and pull-request requests, to the instance that owns the pane. It SHALL send project-wide updates to every attached instance. It SHALL evaluate provider capability and runtime access against the owning machine. It SHALL accept pane-scoped output, state and status only from the instance that owns that pane.

#### Scenario: Type into a tab on another machine
- **WHEN** a user types into a tab owned by zoo-006 while zoo-005 is the primary
- **THEN** the input reaches zoo-006's instance only

#### Scenario: Reject foreign pane traffic
- **WHEN** a CLI that does not own a pane sends terminal output for it
- **THEN** the server discards the output and logs the mismatch

#### Scenario: Project-wide policy change
- **WHEN** the owner changes project flags
- **THEN** every attached instance receives the change

### Requirement: An instance leaving affects only its own tabs

When an instance disconnects or stops, only its tabs SHALL become disconnected or stopped and only their statuses SHALL clear. The session SHALL become inactive only when no instance remains. If the primary leaves, the next earliest instance SHALL become primary. An instance ending its session SHALL end the session only if it is the last instance.

#### Scenario: Secondary machine goes offline
- **WHEN** zoo-006 loses its connection while zoo-005 stays connected
- **THEN** zoo-006's tabs show disconnected, zoo-005's tabs remain live, and the project stays active

#### Scenario: Last instance leaves
- **WHEN** the only remaining instance disconnects
- **THEN** the session becomes inactive as it does today

### Requirement: Instances sharing a directory never overwrite each other's tabs

When several instances use one project directory, every write of project metadata SHALL hold an exclusive lock, re-read the file, and change only what the writer owns: its own panes, or the single project-level field being changed. Project-level fields SHALL never be rewritten from an instance's stale in-memory copy. An instance SHALL restore only its own panes. Untagged panes SHALL be adopted by an instance only when no other machine holds a live instance of the project. A periodic or background read SHALL never delete or regenerate project metadata.

#### Scenario: Concurrent saves on a shared home
- **WHEN** zoo-005 adds a tab and zoo-006 renames one of its own tabs at the same time
- **THEN** the metadata file afterwards contains both changes

#### Scenario: Second instance starts empty
- **WHEN** a project with untagged legacy panes is running on zoo-005 and an instance starts on zoo-006
- **THEN** zoo-006 restores no panes and leaves the legacy panes to zoo-005

#### Scenario: Unparseable metadata during a poll
- **WHEN** a periodic read cannot parse the metadata
- **THEN** the instance keeps its last good copy and the file is not deleted

### Requirement: Claims are per machine, released on stop, and exclude older daemons

A capable daemon SHALL claim a project per machine and SHALL release that claim when the instance stops for any reason. While any capable instance runs a project, a daemon without multi-instance support SHALL be unable to start it. A stopped project SHALL be startable on another machine without waiting for the first machine's daemon to exit.

#### Scenario: Stop on one machine, start on another
- **WHEN** a project is stopped on zoo-005 and then started on zoo-006
- **THEN** zoo-006 starts it immediately

#### Scenario: Older daemon cannot join
- **WHEN** a capable instance runs a project and an older daemon is asked to start it
- **THEN** the older daemon does not start it

### Requirement: Mixed fleets keep single-machine behaviour

A CLI or daemon that does not advertise multi-instance support SHALL behave as today when no other machine runs the project. Its session start SHALL be rejected with an update instruction when another machine's instance is attached. Machine choice for new tabs SHALL be offered only for machines whose daemon advertises support, and the server SHALL refuse a request naming any other machine with a correlated reason. A host configured with an explicit shared machine id SHALL not advertise support.

#### Scenario: Older CLI meets a running instance
- **WHEN** an older CLI starts a session for a project already running on another machine
- **THEN** the server rejects it and tells the user to update that machine

#### Scenario: Older target machine
- **WHEN** a user requests a new tab on a machine whose daemon lacks support
- **THEN** the tab is not created and the user is told to reboot that machine to update

### Requirement: Users choose a machine when creating a tab

When a user can launch on more than one eligible machine, the new-tab flow SHALL let them choose. The choices are the project's running instances, machines where the project is registered, and owned online machines without it. Disabled choices SHALL state why. The default SHALL be the selected tab's machine. Choosing a registered machine that is not running the project SHALL start the instance there and then create the tab. Each request SHALL receive a correlated result of routed, starting, or failed. Pane ids SHALL be unique across machines, including concurrent requests. Lost confirmations SHALL be reported as unconfirmed rather than replayed.

#### Scenario: New tab on a running instance
- **WHEN** a user picks zoo-006, where the project is running
- **THEN** the tab is created on zoo-006 and shows zoo-006 as its host

#### Scenario: New tab on a registered, stopped machine
- **WHEN** a user picks zoo-006, where the project is registered but not running
- **THEN** the result is starting, zoo-006 starts its instance, and the tab is created there

#### Scenario: Start fails
- **WHEN** the instance on the chosen machine fails to start within the time limit
- **THEN** the result is failed with the reason and no tab is created

#### Scenario: Single eligible machine
- **WHEN** only one machine is eligible
- **THEN** the new-tab flow is unchanged

### Requirement: Tabs and project lists show where work runs

Once a project's tabs span more than one machine, the tab bar SHALL show each tab's host and whether its machine's instance is connected. The project list SHALL keep one entry per project, show the primary host with the number of additional running instances, and include every machine's panes in its pane status. Mobile web and native SHALL show each tab's host.

#### Scenario: Two machines
- **WHEN** a project has tabs on zoo-005 and zoo-006
- **THEN** each tab shows its host and the sidebar row shows "zoo-005 +1"

### Requirement: A checkout can join an existing project by git remote

`apas register` SHALL compare the checkout's canonical git remote with projects the user owns or hosts. When exactly one matches and the checkout has no project history, it SHALL offer to join that project. When several match, it SHALL list them and require an explicit choice. Joining SHALL give the checkout the existing project's identity with no panes, replacing only a pristine local identity. An explicit project option SHALL join directly, and a separate-project option SHALL skip matching. Non-interactive runs SHALL require explicit confirmation. Joining SHALL be refused when the local registry already lists that project at a different folder.

#### Scenario: Laptop joins the zoo project
- **WHEN** the owner runs `apas register` in a never-started laptop checkout whose remote matches one existing project
- **THEN** after confirmation the checkout carries that project's id and appears as a machine for that project

#### Scenario: Checkout with history is not re-identified
- **WHEN** the checkout's existing project already has panes or a session
- **THEN** it is registered under its existing identity without a join prompt

#### Scenario: Ambiguous match
- **WHEN** several existing projects share the remote
- **THEN** the command lists them and requires `--project`

### Requirement: An owned machine without the project can clone and join

For an online machine the requester owns that does not have the project, the new-tab flow SHALL offer to clone the repository there as an instance of the same project. The clone SHALL receive the existing project identity and no panes. It SHALL NOT create a new branch or start automatically. The instance SHALL then start and the requested tab be created through the same correlated launch flow. A failure before registration SHALL remove the partial clone. Shared-cluster provisioning SHALL be unchanged.

#### Scenario: Clone onto the laptop
- **WHEN** the owner picks their laptop, which lacks the project, and confirms cloning
- **THEN** the laptop clones the repository, registers it under the existing project id, starts the instance, and creates the tab there
