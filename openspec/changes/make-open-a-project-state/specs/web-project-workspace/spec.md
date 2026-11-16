## MODIFIED Requirements

### Requirement: A project can be opened and closed from its own workspace

The workspace SHALL offer, in one place, opening a project that is closed and closing one that is open, by the project's open state and not by whether one particular machine runs it. A project that is not recorded SHALL always be offered opening, which records it open and so makes its machines follow it from then on, and SHALL be offered closing while any machine runs it; while it runs, the offer to open SHALL say that it keeps the project open on every machine that owns its tabs. Opening SHALL start the project on every machine that owns its tabs, and closing SHALL stop every agent in the project on every machine. Both SHALL be confirmed first: the confirmation for opening SHALL name the machines whose tabs start, and the confirmation for closing SHALL state that the project's agents are stopped, since that is what is lost, SHALL name the machines, and SHALL say that nothing is deleted. Closing SHALL never delete anything; the workspace SHALL offer deleting the project to its owner as a separate control beside closing, named as deletion. While a project is open, or is not recorded and runs somewhere, and a machine that owns tabs of it is not running it, the workspace SHALL say which machine and why, SHALL offer opening the project again where that can start a machine, and SHALL still offer closing. It SHALL also offer opening again while an open project runs nowhere, and closing again while a machine still runs a closed project, whose tabs SHALL keep showing their real status. The confirmation for opening SHALL say of each tab owner that is offline that its tabs start when it reconnects. The mobile session view SHALL offer the same opening again in its pane-actions menu. The workspace SHALL NOT offer starting or stopping one machine's part of a project. After a request to open or close, the workspace SHALL show the state the server reports and SHALL NOT send the request again by itself.

#### Scenario: A project that is not running

- **WHEN** a user views the workspace of a closed project
- **THEN** it offers to open the project
- **AND** does not offer to close it

#### Scenario: Opening from a mobile project workspace

- **WHEN** a user views a closed project in the mobile session view
- **THEN** its pane-actions menu offers to open the project
- **AND** confirmation opens it, which starts every machine that owns the project's tabs, or the session's machine when no machine owns any

#### Scenario: A project that is running

- **WHEN** a user views the workspace of an open project that every machine owning its tabs runs
- **THEN** it offers to close the project
- **AND** does not offer to open it

#### Scenario: An open project with a machine that is not running it

- **WHEN** a user views an open project that zoo-005 runs while zoo-006, which owns tabs of it, failed to start
- **THEN** the workspace says that zoo-006 is not running the project and gives the reason
- **AND** it offers to open the project again and to close it
- **AND** it offers no control that starts zoo-006 alone

#### Scenario: An open project waiting for an offline machine

- **WHEN** a user views an open project whose tabs on zoo-006 wait because zoo-006 is offline
- **THEN** the workspace says those tabs start when zoo-006 reconnects
- **AND** it does not offer to open the project again for that machine

#### Scenario: Closing a project

- **WHEN** a user confirms closing a project
- **THEN** the project's agents are stopped on every machine

#### Scenario: Declining

- **WHEN** a user declines the confirmation
- **THEN** nothing is stopped

#### Scenario: A closed project's agents are not listed as idle

- **WHEN** a project has been closed
- **THEN** its agents no longer appear among the idle agents

#### Scenario: A project nobody has recorded that runs on one of two machines

- **WHEN** a user views a project that is not recorded, which zoo-005 runs and of which zoo-006 owns tabs it is not running
- **THEN** the workspace says zoo-006 is not running the project
- **AND** it offers to open the project, which records it open and starts zoo-006
- **AND** it offers to close the project

#### Scenario: A project nobody has recorded that runs everywhere

- **WHEN** a user views a project that is not recorded and that every machine owning its tabs runs
- **THEN** the workspace offers to keep it open, saying its machines will start it again after they have been away
- **AND** confirming records the project open and starts nothing

#### Scenario: A closed project a machine still runs

- **WHEN** a user views a closed project that zoo-006 still runs because the account that closed it may not use zoo-006
- **THEN** the workspace says zoo-006 still runs the project and why
- **AND** it offers to close the project again
- **AND** zoo-006's tabs show their real status, not "Not running"

#### Scenario: An open project that runs nowhere

- **WHEN** a user views an open project that no machine runs, and its machine is connected
- **THEN** the workspace offers to open the project again and to close it

#### Scenario: Opening again from the mobile session view

- **WHEN** a user views, in the mobile session view, an open project that zoo-005 runs while zoo-006, which owns tabs of it and is connected, is not running it
- **THEN** the pane-actions menu offers to open the project again
- **AND** it offers no control that starts zoo-006 alone

#### Scenario: Closing says that nothing is deleted

- **WHEN** a user chooses to close a project
- **THEN** the confirmation says its agents are stopped on the named machines and that nothing is deleted
- **AND** after confirming, the project's tabs and history are still there

#### Scenario: Delete is a control of its own

- **WHEN** the owner of a project views its workspace
- **THEN** the workspace offers "Delete project…" beside closing
- **AND** closing the project does not open the deletion dialog or delete anything

#### Scenario: A user who is not the owner

- **WHEN** a project user who is not its owner views its workspace
- **THEN** the workspace offers no control that deletes the project
