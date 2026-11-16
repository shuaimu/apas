## Purpose

Make open and closed one durable state of a project, so that every machine
that owns tabs of an open project runs them, no machine runs a closed
project, and a machine that was away follows the state when it returns.

## ADDED Requirements

### Requirement: A project has one recorded open state

The server SHALL keep, for each project, one open state: open, closed, or not recorded. It SHALL record the state, the account or administrator that set it, whether it was set by an administrative stop, the time, and for an open the machine the request named, SHALL keep them across server restarts, and SHALL NOT keep them in the project's own metadata on any machine. A project nobody has opened or closed since the server gained this capability SHALL be not recorded. For a project that is not recorded, the server SHALL NOT start or stop anything by itself, and SHALL report it as open while at least one machine runs it and as closed otherwise. A machine starting or ending its run of a project by itself, as when a daemon restores its projects after replacing itself, SHALL NOT change the recorded state. Only a project whose administrative state is active SHALL be recorded open: a request that would open a suspended project or one being deleted SHALL be refused and SHALL record nothing. An open, a close and a follow step for one project SHALL be applied one at a time, each on the state as it is at that moment, so a stop decided for a closed project is never sent after a later open.

#### Scenario: The state survives a server restart
- **WHEN** a user opens a project and the server then restarts
- **THEN** the project is still recorded open
- **AND** machines are started or stopped only as their daemons register again and are found out of line with it

#### Scenario: A project nobody has touched since the upgrade
- **WHEN** the server is upgraded while a project runs on zoo-005 and its tabs on zoo-006 are stopped
- **THEN** the project is not recorded, and is reported as open because zoo-005 runs it
- **AND** zoo-006 is not started when its daemon registers

#### Scenario: A daemon restoring its projects does not open them
- **WHEN** a daemon replaces itself and starts the projects it was running again, one of which is not recorded
- **THEN** that project stays not recorded

#### Scenario: Opening a suspended project
- **WHEN** a user asks to open a suspended project
- **THEN** the request is refused with the reason
- **AND** the project's recorded state does not change

### Requirement: Opening a project starts it on every machine that owns its tabs

Opening SHALL record the project as open and SHALL start it on every machine that owns tabs of it and whose daemon is connected. A machine owns a tab when the server's roster records the tab for that machine or attributes an untagged tab to that machine's host. When no machine owns a tab, opening SHALL start the project on the machine the request names, and that machine SHALL count as owning the project's tabs for as long as none does. A machine that owns no tab and is not so named SHALL NOT be started. When the server holds no roster for the project, as for a session from before rosters were kept, the tab owners are unknown: opening SHALL start the named machine only, and no machine SHALL be started by a follow step. Which machines can run the project together SHALL be decided from the roster and each daemon's support, not from which instances happen to be attached: a machine whose daemon lacks multi-instance support SHALL be started only when no other machine owns tabs of the project, and SHALL otherwise be reported as not started, with the instruction to reboot it to update, and sent nothing. Every other existing reason to refuse a start on one machine SHALL apply to that machine alone and SHALL NOT keep the other machines from starting.

An account SHALL be able to open a project when it may use the project's runtime on at least one connected machine that lists the project, or, when no such machine is connected, when it owns or hosts the project. A request from any other account SHALL be refused and SHALL record nothing. Each machine SHALL be started only where the opening account may use the project's runtime on it; a machine it may not use SHALL be reported as not started, with that reason. Opening a project that is already open SHALL start the machines that own tabs and are not running it, SHALL change nothing on a machine that runs it, and SHALL NOT change the recorded opener or named machine. Opening a project that was closed or not recorded SHALL ask every connected tab owner to start, also one that still reports running it because an earlier stop has not finished there, so that a close followed at once by an open leaves the project running. Opening SHALL be confirmed by the user first, and the confirmation SHALL name the machines whose tabs start, and SHALL say of each tab owner that is offline that its tabs start when it reconnects.

#### Scenario: Open a project whose tabs span machines
- **WHEN** a closed project has tabs owned by zoo-005 and zoo-006, both online with multi-instance support, and its owner opens it
- **THEN** the project is recorded open and an instance starts on zoo-005 and on zoo-006

#### Scenario: Open a project with no tabs
- **WHEN** a closed project has no tabs and its owner opens it from zoo-005's entry
- **THEN** the project is recorded open and starts on zoo-005 only
- **AND** zoo-005 is the machine that follows the project until some machine owns a tab

#### Scenario: Open while one tab owner is offline
- **WHEN** a closed project has tabs owned by zoo-005 and by a laptop that is offline, and its owner opens it
- **THEN** the confirmation names zoo-005 and says the laptop's tabs start when it reconnects
- **AND** confirming records the project open and starts zoo-005

#### Scenario: Open again to retry one machine
- **WHEN** a project is open, zoo-005 runs it, zoo-006 owns tabs and failed to start, and a user opens the project again
- **THEN** zoo-006 is asked to start
- **AND** nothing changes on zoo-005

#### Scenario: An older daemon among the tab owners
- **WHEN** a user opens a closed project whose tabs are owned by zoo-005 and by a laptop whose connected daemon lacks multi-instance support, and no instance is attached yet
- **THEN** zoo-005 starts and the project is recorded open
- **AND** the laptop is sent nothing and is reported as not started with "laptop runs an older APAS that cannot run this project beside other machines. Reboot laptop to update it."

#### Scenario: An older daemon as the only tab owner
- **WHEN** a user opens a project whose tabs are all owned by a laptop whose daemon lacks multi-instance support
- **THEN** the laptop starts the project as its only runtime, as it does today

#### Scenario: A request from an account that may use none of its machines
- **WHEN** an account that may read a project but may use its runtime on none of its machines, and neither owns nor hosts it, asks to open it
- **THEN** the request is refused and the recorded state does not change

### Requirement: Closing a project stops it on every machine

Closing SHALL record the project as closed and SHALL stop it on every connected machine that runs it, has an instance of it attached, or is starting it. Stopping SHALL end the project's agents on that machine as a project stop does today. The same accounts that may open a project SHALL be able to close it, and a request from any other account SHALL be refused and SHALL record nothing. Each machine SHALL be stopped only where the closing account may use the project's runtime on it; a machine it may not use SHALL be reported as still running, with that reason. Closing a project that a request already closed SHALL keep the recorded closer, unless the new requester may use every machine the recorded closer may use and at least one more; then the new requester SHALL be recorded. Closing SHALL be confirmed by the user first, and the confirmation SHALL say that the project's agents are stopped on every machine and SHALL name every machine that owns tabs, also one that is offline.

#### Scenario: Close a project running on two machines
- **WHEN** a project runs on zoo-005 and zoo-006 and a user closes it
- **THEN** the project is recorded closed and both instances stop

#### Scenario: Close while one machine is still starting
- **WHEN** a project is open, zoo-005 runs it, zoo-006 is still starting it, and a user closes the project
- **THEN** both machines are asked to stop

#### Scenario: Close a project whose machines are all offline
- **WHEN** a project is recorded open, every machine that owns its tabs is offline, and its owner closes it
- **THEN** the project is recorded closed

#### Scenario: A reader cannot close a project while its machines are away
- **WHEN** an account that may read a project, neither owns nor hosts it, and may use none of its machines asks to close it while its machines are disconnected
- **THEN** the request is refused and the project stays recorded open

### Requirement: Closing a project deletes nothing

Closing a project, by any path, SHALL remove nothing. Its tabs, their conversations and worktrees, its history on the server, its sharing, policy and saved prompts, and its registration on every machine SHALL remain as they were, and the project SHALL stay listed, as closed, wherever it was listed while open. Opening it again SHALL bring back every tab as a start of the project does today. A close SHALL NOT forget a machine's tabs, close a tab or begin a deletion. The confirmation for closing SHALL say that nothing is deleted. Removing a project SHALL be possible only through the separate, explicit deletion.

#### Scenario: Close and open again
- **WHEN** a project with three tabs on zoo-005 and two on zoo-006 is closed and later opened
- **THEN** the same five tabs are listed, each on its machine, with the history they had
- **AND** each tab's agent starts again as it does when a project is started today

#### Scenario: Close and open again at once
- **WHEN** a project that runs on zoo-005 and zoo-006 is closed and opened again before either machine has finished stopping it
- **THEN** each machine is asked to start it after its stop
- **AND** the project runs on both machines again

#### Scenario: A closed project stays listed
- **WHEN** a user closes a project
- **THEN** the project is still in the project list, shown as closed, with its tabs
- **AND** its conversations can still be read

#### Scenario: Close does not delete
- **WHEN** a user confirms closing a project
- **THEN** the confirmation said that nothing is deleted
- **AND** no tab, worktree, message, share or registration of the project is removed

### Requirement: A machine follows the recorded state when its daemon registers

When a machine's daemon has registered with the server, the server SHALL bring that machine in line with each recorded project the daemon lists. For a project recorded open, where the machine owns tabs and the daemon does not report running it, the server SHALL ask the daemon to start it, provided the project is active, the account that opened it is still active and may use the project's runtime on that machine at that moment, and the machine may run the project beside the other tab owners as an open decides it. For a project recorded closed that the daemon reports running, the server SHALL ask the daemon to stop it, provided the account that closed it may use the project's runtime on that machine at that moment; a close made by an administrative stop SHALL be followed on every machine that stop reaches today. A machine left as it is for want of that access SHALL be reported with the reason. The server SHALL do neither for a project that is not recorded. The server SHALL NOT, at any other time, start a machine by itself: a machine whose start was refused or failed SHALL stay as it is until its daemon registers again or someone opens the project again.

#### Scenario: A machine that was offline when the project was opened
- **WHEN** a project was opened while zoo-006, which owns tabs of it, was offline, and zoo-006's daemon later registers
- **THEN** the server asks zoo-006 to start the project and its tabs come back

#### Scenario: A machine that restarted
- **WHEN** a machine running an open project is restarted, and its daemon later starts and registers
- **THEN** the server asks it to start the project again

#### Scenario: A project closed while a machine was away
- **WHEN** a project was closed by its owner while zoo-006's daemon had lost its connection and still ran it, and the daemon registers again
- **THEN** the server asks zoo-006 to stop the project

#### Scenario: A project closed while its daemon was being replaced
- **WHEN** a project is closed while zoo-005's daemon is replacing itself, and the returning daemon starts the projects it was running and registers
- **THEN** the server asks zoo-005 to stop that project

#### Scenario: An older daemon registers before the others have returned
- **WHEN** the server has restarted, an open project's tabs are owned by zoo-005 and by a laptop whose daemon lacks multi-instance support, and the laptop's daemon registers before zoo-005's instance has reattached
- **THEN** the laptop is not asked to start the project

#### Scenario: The opener lost access to the machine
- **WHEN** a member opened a project hosted on another account's machine, that membership no longer allows the machine, and the machine's daemon registers
- **THEN** the server does not start the project there
- **AND** the machine is reported as not started with the reason

#### Scenario: A member's close does not reach a machine the member may not use
- **WHEN** a member who may use zoo-005 but not zoo-006 closed a project, and zoo-006's daemon registers still running it
- **THEN** zoo-006 is not asked to stop
- **AND** zoo-006 is reported as still running, with the reason

#### Scenario: A project being deleted
- **WHEN** a project recorded open is being deleted and a machine that owns tabs of it registers
- **THEN** the server does not ask it to start the project

#### Scenario: A failed start is not retried by itself
- **WHEN** zoo-006 refuses to start an open project and stays connected
- **THEN** the server does not ask it again until its daemon registers again or someone opens the project again

### Requirement: Each machine's part in a project is reported

The server SHALL report to every viewer that may read a project, for each machine that owns tabs of it or runs it, one of: running; starting; waiting for the machine, which is offline; not started, with the reason; or, for a closed project, still running, with the reason when one is known. The reason SHALL be the one the refusing check or the daemon gave, and a start the daemon never answered SHALL be reported as not started after a time limit. The report SHALL cover a project that is not recorded as well, where a tab owner that is offline SHALL be reported as not started, since nothing will start it when it returns, SHALL be sent when a viewer attaches to the project and whenever a part changes, and SHALL say who set the state and when. Machine-list entries SHALL carry, for each project, its open state, that machine's part with its reason, and the hosts of the other machines that own tabs of the project or run it, so that a machine list can show and confirm without attaching to the project. A project's open state SHALL also be carried on the entries of session lists and the mobile bootstrap. All of these SHALL be values an older client ignores, and a viewer that did not negotiate support for the report SHALL be sent no frame of a kind it does not know.

#### Scenario: One machine is offline
- **WHEN** a viewer opens an open project that zoo-005 runs and whose other tabs belong to zoo-006, which is offline
- **THEN** the viewer is told zoo-005 is running and zoo-006 is waiting because it is offline

#### Scenario: A start was refused
- **WHEN** zoo-006's daemon refuses to start an open project
- **THEN** every attached viewer that negotiated support is told zoo-006 is not started, with the daemon's reason

#### Scenario: A project that is not recorded and runs on one of two machines
- **WHEN** a viewer opens a project that is not recorded, which zoo-005 runs and of which zoo-006 owns tabs it is not running
- **THEN** the viewer is told the project is not recorded, zoo-005 is running and zoo-006 is not started

#### Scenario: The machine list without an attached project
- **WHEN** a user's machine list shows an open project on zoo-006 that zoo-006 failed to start, and the user has not opened that project's workspace
- **THEN** the entry carries that the project is open, that zoo-006 is not started and why, and that zoo-005 also owns tabs of it

#### Scenario: An older native build
- **WHEN** a native build that did not negotiate support views an open project
- **THEN** it receives no frame of a new kind and stays connected
- **AND** the session entries it already parses carry the project's open state as a field it ignores

### Requirement: A new tab on a machine that is not running opens the project

When a viewer that negotiated support for the open state asks to create a tab on a machine that is not running the project, the server SHALL first check that the tab can be started on that machine, and only then SHALL open the project as the requesting account, start the chosen machine as well, and create the tab there. If that check fails, nothing SHALL be recorded and no machine SHALL be started. Before the request is sent, the new-tab flow SHALL say that choosing that machine opens the project and SHALL name the other machines whose tabs start with it. The correlated result of the tab request SHALL keep its meaning: starting, then routed or failed, for the chosen machine only.

#### Scenario: New tab in a closed project
- **WHEN** a project is closed, zoo-005 and zoo-006 own tabs of it, and a user adds a tab on zoo-005
- **THEN** the project is recorded open, zoo-005 and zoo-006 start it, and the tab is created on zoo-005
- **AND** the machine choice said beforehand that the project opens and that zoo-006's tabs start too

#### Scenario: New tab on a machine that joins an open project
- **WHEN** a project is open on zoo-005 and a user adds a tab on a laptop where the project is registered and has no tabs
- **THEN** the laptop starts its instance and the tab is created there
- **AND** nothing changes on zoo-005

#### Scenario: The chosen machine cannot take the tab
- **WHEN** a user adds a tab to a closed project on a machine that refuses the start
- **THEN** the request fails with the reason
- **AND** the project stays closed and no other machine starts

### Requirement: A client that does not know the open state never reaches beyond the machine it names

A request that names one machine and comes from a client that cannot have said what opening or closing does (a start or stop of the project on one machine, a tab request from a viewer that did not negotiate support for the open state, and a native task launch) SHALL never start or stop a machine other than the one it names. For a project that is not recorded it SHALL act on that machine as it did before the open state and SHALL record nothing; and a request to start or stop a not-recorded project on every machine, from a viewer that did not negotiate support for the open state, SHALL likewise act as it did before and SHALL record nothing. For a project recorded open: a start, a tab request or a task launch SHALL start the machine it names; a stop SHALL close the project when no other machine owns tabs of it or runs it, and SHALL otherwise be refused, with nothing stopped, with "This project runs on other machines too. Close it from the project." For a project recorded closed: a start, a tab request or a task launch SHALL open the project when no other machine owns tabs of it, and SHALL otherwise be refused, with nothing started, with "This project is closed and has tabs on other machines. Open it from the project.", also when the named machine still has an instance of the project attached; a stop SHALL stop the machine it names. A tab request that names no machine SHALL be judged as naming the machine of the instance it would be sent to. A request that opens a project this way SHALL be accepted only from an account that may open the project, and SHALL otherwise be refused with nothing recorded. A task launch chosen for a machine whose daemon is not connected SHALL fail and SHALL record nothing.

#### Scenario: A task for a machine that is away
- **WHEN** a closed project has its tabs on zoo-006 only, zoo-006's daemon is not connected, and a native task launch names zoo-006
- **THEN** the launch fails because the machine is offline
- **AND** the project stays recorded closed

#### Scenario: An older web bundle starts one machine of a project nobody has recorded
- **WHEN** a web bundle from before the open state asks to start a not-recorded project on zoo-006 while zoo-005 runs it
- **THEN** zoo-006 starts, as before
- **AND** the project stays not recorded

#### Scenario: An older web bundle's project-view button on a project nobody has recorded
- **WHEN** a web bundle from before the open state starts a not-recorded project from its project view
- **THEN** the machines it reached before the open state start, as before
- **AND** the project stays not recorded, so no machine is started or stopped for it later

#### Scenario: An older web bundle retries a machine of an open project
- **WHEN** a project is open, zoo-006 owns tabs and is not running it, and a web bundle from before the open state asks to start it on zoo-006
- **THEN** zoo-006 is asked to start

#### Scenario: An older client starts a single-machine project
- **WHEN** a web bundle from before the open state asks to start a closed project on zoo-005, the only machine that owns tabs of it
- **THEN** the project is recorded open and zoo-005 starts it

#### Scenario: An older client stops one machine of two
- **WHEN** a project is open on zoo-005 and zoo-006 and a web bundle from before the open state asks to stop it on zoo-006 only
- **THEN** the request is refused with "This project runs on other machines too. Close it from the project."
- **AND** neither machine stops

#### Scenario: A native task in a closed project with tabs elsewhere
- **WHEN** a user launches a native task choosing zoo-005 in a closed project of which zoo-006 also owns tabs
- **THEN** the launch fails with "This project is closed and has tabs on other machines. Open it from the project."
- **AND** nothing starts

#### Scenario: A native task in a closed single-machine project
- **WHEN** a user launches a native task choosing zoo-005 in a closed project whose tabs are all on zoo-005
- **THEN** the project is recorded open and the task runs on zoo-005

### Requirement: Other paths that start or end a project record its state

Creating a project in a way that starts it SHALL record it open, by the creating account, when the new project first runs. Joining an existing project from another checkout SHALL NOT change that project's recorded state. Suspending a project, an operator or system administrator stopping its runtime, and beginning its deletion SHALL record the project as closed by that actor, in addition to what they do today. Reactivating a suspended project SHALL start nothing and SHALL leave it closed. Opening and closing a project, by any path, SHALL be recorded in the audit history of the clusters hosting it, with the actor and the machines asked to start or stop. A start or stop the server sends when a machine's daemon registers SHALL be recorded there too, with the recorded actor and that machine. Closing a project SHALL NOT by itself cancel or pause autonomous workers or their jobs; an administrative stop keeps cancelling them as it does today.

#### Scenario: A project created and started
- **WHEN** an owner creates a project from a clone with "Create & start" on zoo-005
- **THEN** the project is recorded open
- **AND** it starts again on zoo-005 after zoo-005's daemon next registers without running it

#### Scenario: An operator stops a project's runtime
- **WHEN** a cluster operator stops the runtime of an open project
- **THEN** the project is recorded closed
- **AND** a machine of that project whose daemon registers later is not started

#### Scenario: Reactivating after a suspension
- **WHEN** an operator suspends an open project and later reactivates it
- **THEN** no machine starts it until someone opens it

#### Scenario: Opening is audited
- **WHEN** a user opens a project hosted in a cluster
- **THEN** that cluster's audit history shows who opened it, when, and which machines were asked to start
