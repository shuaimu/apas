## MODIFIED Requirements

### Requirement: A project runs at most one instance per machine under one session

A project SHALL be able to run one instance on each of several machines at the same time. Every instance SHALL attach to the project's single existing session, so the project keeps one history, one sharing and policy unit, and one project-list entry. A reconnect from the same machine SHALL replace only that machine's attachment. The earliest attached instance SHALL be the primary, supplying the session's displayed host and the default target for controls that name no machine.

#### Scenario: Start a second instance on a shared home
- **WHEN** a project is running on zoo-005 and its owner adds a tab on zoo-006, which sees the same directory
- **THEN** zoo-006 runs an instance attached to the same session
- **AND** the zoo-005 instance and its tabs keep running unchanged

#### Scenario: Reconnect from the same machine
- **WHEN** zoo-006's instance reconnects after a transport loss
- **THEN** it replaces only zoo-006's attachment and zoo-005's attachment is untouched

### Requirement: Every pane belongs to exactly one machine

Each pane reported by an instance with multi-instance support SHALL record the machine that runs it. The server SHALL set that record from the authenticated instance that reports the pane, not from the pane data the instance supplies, and SHALL record no machine for panes reported by an instance without that support. A pane with no recorded machine SHALL belong to no machine until an instance with multi-instance support reports it, which then adopts it. The server SHALL attribute such a pane to the host it last recorded the pane under, for display and routing only, and SHALL NOT refuse or drop a pane on account of that host when an instance with support reports it. A control or close for such a pane SHALL go to the attached instance on its attributed host, else to the sole attached instance when that instance lacks support, else to the primary instance when the pane has no attributed host. A control or close for a pane recorded for a machine SHALL go only to that machine's instance, even while an instance without support is the sole instance. While any instance of the project is attached, a control or close that cannot go to an instance this way SHALL be refused as stopped on the pane's machine or attributed host; with no instance attached it SHALL fail as it does today. A pane's history, terminal and controls SHALL stay with its machine for the pane's lifetime. No instance with multi-instance support SHALL restore or adopt a pane recorded for another machine, even while that machine is stopped; a runtime without that support restores every pane, as before multi-machine instances. A tab whose machine is not running SHALL be shown as stopped on that machine and SHALL NOT be closable until that machine runs the project. When the server refuses typed input that carries the client's id for that message because of its tab's machine, which is not running the project or which the sender may not use, it SHALL also send the sending connection a refusal correlated to that input and carrying the reason, in addition to the error it sends today, if that connection advertised multi-instance support; such a correlated answer SHALL only ever be a refusal. A client that tracks that input SHALL settle it on that refusal, whichever attempt of it the refusal answers: it SHALL NOT send the input under that id again, by retransmission or by replay after a reconnect, SHALL show the reason where it shows the input rather than showing it as unconfirmed, and SHALL keep the typed text recoverable, so that sending it again is a new input. When a close for a tab with no recorded machine is delivered to an instance with multi-instance support, the tab SHALL leave the roster, although that instance never reports it. The owner SHALL be able to forget all tabs of a stopped machine after confirmation, and SHALL be told that they come back if that machine runs the project again. Forgetting SHALL change only the server's roster: no instance SHALL be told and no project metadata SHALL change, so that machine restores and reports those tabs, with their ids, when it runs the project again. The server SHALL NOT allocate the ids of forgotten tabs, or of untagged tabs dropped on a close, to new tabs while any other id is free. A refusal on account of a tab's machine not running the project SHALL read "<host> is not running this project. Open the project again to start it there." while that machine's daemon is connected, and "<host> is not running this project because it is offline." while it is not. Which machines run the project SHALL follow the project's open state: no control SHALL start or stop one machine's instance as a choice of its own.

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
- **AND** a control or close for one of them is refused with "zoo-005 is not running this project. Open the project again to start it there."

#### Scenario: Tabs reported by an older CLI stay adoptable
- **WHEN** a CLI without multi-instance support reports its tabs
- **THEN** the server records their host but no machine
- **AND** an instance with support on a shared home that later reports those tabs records them as its own

#### Scenario: Untagged tab with no known host
- **WHEN** a tab has no recorded machine and no attributed host
- **THEN** its controls go to the primary instance

#### Scenario: A sole older CLI does not receive another machine's tab
- **WHEN** a CLI without multi-instance support on zoo-005 is the project's only instance and a tab is recorded for a stopped laptop
- **THEN** a control for that tab is refused with "laptop is not running this project. Open the project again to start it there."
- **AND** nothing is sent to the CLI on zoo-005

#### Scenario: Close an untagged tab that no instance runs
- **WHEN** a tab with no recorded machine is attributed to zoo-005, whose instance has multi-instance support and runs the project without reporting that tab, and the user closes it
- **THEN** the close is delivered to zoo-005's instance
- **AND** the tab leaves the project roster

#### Scenario: Tabs stay on their machine
- **WHEN** a project's tabs belong to zoo-005, zoo-005 is stopped, and the project starts on zoo-006, in the same directory, as an instance with multi-instance support
- **THEN** zoo-006 starts with none of zoo-005's tabs
- **AND** zoo-005's tabs remain listed as stopped on zoo-005 until zoo-005 runs the project again

#### Scenario: Close a tab whose machine is not running
- **WHEN** a user closes a tab owned by zoo-006 while the project runs on zoo-005 and not on zoo-006
- **THEN** the request is refused with "zoo-006 is not running this project. Open the project again to start it there."

#### Scenario: Send to a tab whose machine is not running
- **WHEN** a web or native client that advertised multi-instance support sends a message to a tab owned by zoo-006 while the project runs on zoo-005 and not on zoo-006
- **THEN** the message is refused with "zoo-006 is not running this project. Open the project again to start it there."
- **AND** the refusal is correlated to that message, so the client shows the reason with the message and keeps its text
- **AND** the client never sends that message again by itself, even after it reconnects or zoo-006 runs the project again

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
- **WHEN** a closed project has tabs owned by zoo-005 and zoo-006 and the user opens the project from the project view
- **THEN** an instance starts on each of those machines whose daemon is connected and has multi-instance support
- **AND** one that is offline starts when its daemon next registers

#### Scenario: Start one machine from the Machines page
- **WHEN** a user opens the project from zoo-006's entry on the Machines page while zoo-005 also owns tabs
- **THEN** the project is opened as a whole: zoo-005 and zoo-006 both start an instance
- **AND** the Machines page offers no control that starts zoo-006 alone

#### Scenario: Close the project from the project view
- **WHEN** a project runs on zoo-005 and zoo-006 and the user closes it from the project view
- **THEN** both instances stop and the project is recorded closed

#### Scenario: Close the project while one machine reconnects
- **WHEN** a project runs on zoo-005 and zoo-006, zoo-006's instance has lost its connection to the server while zoo-006's daemon still reports running the project, and the user closes the project from the project view
- **THEN** the project stops on both zoo-005 and zoo-006

#### Scenario: A tab on a machine that is not following an open project
- **WHEN** a project is open, zoo-005 runs it, and a user sends a control to a tab owned by zoo-006, whose daemon is connected and failed to start the project
- **THEN** the control is refused with "zoo-006 is not running this project. Open the project again to start it there."
- **AND** nothing is sent to zoo-005

#### Scenario: A tab on a machine that is offline
- **WHEN** a project is open, zoo-005 runs it, and a user sends a control to a tab owned by zoo-006, whose daemon is not connected
- **THEN** the control is refused with "zoo-006 is not running this project because it is offline."

### Requirement: Mixed fleets keep single-machine behaviour

A CLI or daemon that does not advertise multi-instance support SHALL behave as today when no other machine runs the project. Its session start SHALL be rejected with an update instruction when another machine's instance is attached. While it runs the project, a request to start the project on another machine SHALL be refused, before that machine starts anything, with the instruction to stop the project on that CLI's host or reboot that host to update it; a request to start the project on that CLI's own machine SHALL proceed as today. While a capable instance runs the project, a request to start it on a machine whose connected daemon lacks support SHALL be refused, before that machine starts anything, with the instruction to reboot that machine to update it. Machine choice for new tabs SHALL be offered only for machines whose daemon is connected and advertises support, and the server SHALL refuse a request naming any other machine with a correlated reason. Machine entries SHALL distinguish a connected daemon that lacks support, which needs a reboot to update, from an offline one. A host configured with an explicit shared machine id SHALL not advertise support. The server SHALL confirm support to a daemon that advertised it when the daemon registers. A daemon that advertised support SHALL rely on it only on a connection where the server confirmed it. On any other connection it SHALL handle every start request as daemons did before multi-machine instances: it SHALL refuse a project another live host runs rather than starting an instance beside it, SHALL run a project it does start as that project's only runtime, which restores every tab in the project's metadata, other machines' included, and reports and saves them with no recorded machine, and SHALL claim the project as such daemons do. It SHALL decide again at every registration and SHALL NOT stop or change a running project because of it. When it replaces itself, a project it was running SHALL start again as its only runtime if the server it was connected to did not confirm support and no other host runs the project, and otherwise as an instance under the ordinary claim rules. A web or native client SHALL receive the frame listing a session's instances, and correlated refusals of its typed input, only if it advertised support, and a tab-launch result only for a request it sent with a correlation id. Starting and stopping SHALL follow the project's open state on every machine, and a request that names one machine SHALL reach no other machine.

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

#### Scenario: Capable daemon runs a project alone on an older server
- **WHEN** a daemon with multi-instance support on zoo-006 is registered with a server that did not confirm that support, and it is asked to start a project that no other host runs, whose shared metadata holds tabs recorded for zoo-005
- **THEN** zoo-006 runs the project as its only runtime, with zoo-005's tabs among its own, as a daemon without support would
- **AND** a daemon with multi-instance support on zoo-005 that is asked to start the project does not start it beside zoo-006, even on a server that confirms support
- **AND** once a server that confirms support registers zoo-006's daemon, rebooting zoo-006 brings the project back as an instance

#### Scenario: Start beside an older CLI
- **WHEN** a CLI without multi-instance support on zoo-005 runs the project and the user starts the project on zoo-006
- **THEN** the start is refused with "This project is running on zoo-005 with an APAS CLI that cannot share it with other machines; stop it there, or reboot zoo-005 from the Machines page to update it, then start it here"
- **AND** zoo-006 starts nothing

#### Scenario: Start on an older daemon beside an instance
- **WHEN** a capable instance on zoo-005 runs the project and the user starts the project on zoo-006, whose connected daemon lacks multi-instance support
- **THEN** the start is refused with "zoo-006 runs an older APAS that cannot run this project beside other machines. Reboot zoo-006 to update it."
- **AND** zoo-006 starts nothing

#### Scenario: A request naming one machine reaches no other
- **WHEN** a client from before the open state asks to start a project on zoo-006 while zoo-005 also owns tabs of it
- **THEN** nothing is sent to zoo-005 because of that request

### Requirement: Users choose a machine when creating a tab

When a user can launch on more than one eligible machine, or the only eligible machine is not running the project, the new-tab flow SHALL let them choose, and SHALL say what choosing a machine does (open the project, start it there, or clone it there) before anything starts, and SHALL name the other machines whose tabs start when the choice opens the project. The choices are the project's running instances, machines where the project is registered, and owned online machines without it. Disabled choices SHALL state why. The default SHALL be the selected tab's machine, and a machine the project would be cloned onto SHALL never be preselected. Choosing a registered machine that is not running the project SHALL open the project where it is not open, start the instance there, and then create the tab. A request naming a machine SHALL never create the tab on another machine. Each request SHALL receive a correlated result of routed, starting, or failed, sent only to the connection that made the request; routed SHALL mean delivered to the instance, and starting SHALL always be followed by routed or failed. The start of a stopped instance SHALL be correlated with the daemon's reported outcome, so a refused or failed start fails the request with its reason, and a session start of that instance that the server refuses SHALL fail the request at once with that refusal's reason. Pane ids SHALL be unique across machines, including concurrent requests. Lost confirmations SHALL be reported as unconfirmed rather than replayed.

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
- **WHEN** the project is closed and the only eligible machine is one where it is registered
- **THEN** the new-tab flow shows that machine, preselected, as opening the project first
- **AND** choosing a tab type opens the project, which starts it there, and then creates the tab

#### Scenario: Single eligible machine that would be cloned
- **WHEN** the only eligible machine is an owned machine without the project
- **THEN** the new-tab flow offers it as a clone, preselects nothing, and creates nothing until the user picks that machine and confirms the clone

### Requirement: Native task launches target the chosen machine

A native task launch SHALL keep its existing meaning until an instance with multi-instance support runs the project: start the project on the chosen machine when nothing runs it, otherwise add the task to the running session. Once such an instance runs the project, the launch SHALL run the task on the chosen machine, starting a registered, stopped instance there first when that machine's daemon has support, and SHALL fail with the reason rather than run the task on another machine. A launch SHALL never start a machine other than the chosen one: in a project recorded closed of which another machine owns tabs, it SHALL fail with the instruction to open the project.

#### Scenario: Before any capable instance runs the project
- **WHEN** only an instance without multi-instance support runs the project, on zoo-005, and a user launches a native task choosing zoo-006
- **THEN** the task is added to the running session, as it is today

#### Scenario: Launch on a stopped capable machine
- **WHEN** a capable instance runs the project on zoo-005 and a user launches a native task choosing zoo-006, where the project is registered, stopped, and the daemon has support
- **THEN** zoo-006 starts its instance and the task runs there

#### Scenario: Chosen machine cannot run an instance
- **WHEN** a capable instance runs the project on zoo-005 and a user launches a native task choosing a machine whose daemon lacks support
- **THEN** the launch fails with the reason and no task starts on any other machine

#### Scenario: Launch in a closed project with tabs on another machine
- **WHEN** a project is recorded closed, zoo-006 owns tabs of it, and a user launches a native task choosing zoo-005
- **THEN** the launch fails with "This project is closed and has tabs on other machines. Open it from the project."
- **AND** neither machine starts
