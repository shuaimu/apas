## ADDED Requirements

### Requirement: The machine list opens and closes projects as a whole

For each project a machine lists, the machine list SHALL show the project's open state and that machine's part in it: running, starting, not started with the reason, still running after a close, or that the machine owns no tabs of the project. It SHALL show these from what the machine list itself is sent, without the user having opened the project. It SHALL offer opening a closed project and closing an open one, opening a project that is not recorded, and closing any project a machine still runs, and each SHALL act on the whole project, as it does from the project's workspace. It SHALL NOT offer starting or stopping the project on that machine alone. When opening or closing from a machine's entry reaches other machines, the confirmation SHALL name them before anything is sent. Opening a project that has no tabs from a machine's entry SHALL start it on that machine. Opening a project from the entry of a machine that owns none of its tabs, while other machines own some, SHALL say before anything is sent that the project starts on those machines and not on this one. The count of projects running on a machine SHALL keep counting what that machine runs. While a request is pending the entry SHALL say so, SHALL claim nothing for a request that was not sent, and SHALL NOT send it again by itself.

#### Scenario: Open a project from one machine's entry
- **WHEN** a closed project has tabs on zoo-005 and zoo-006 and a user chooses Open in zoo-005's entry
- **THEN** the confirmation says the project also starts on zoo-006
- **AND** confirming opens the project, and both machines start it

#### Scenario: Close a project from one machine's entry
- **WHEN** a project runs on zoo-005 and zoo-006 and a user chooses Close in zoo-006's entry
- **THEN** the confirmation says the project's agents stop on zoo-005 and zoo-006
- **AND** confirming closes the project on both

#### Scenario: A registered project that never ran
- **WHEN** a project was registered on zoo-005, has no tabs and has never run, and its owner chooses Open in zoo-005's entry
- **THEN** the project is recorded open and starts on zoo-005

#### Scenario: A machine with no tabs of an open project
- **WHEN** a project is open on zoo-005 and is registered on zoo-006, which owns no tabs of it
- **THEN** zoo-006's entry shows the project as open with no tabs on zoo-006
- **AND** it offers no control that starts the project on zoo-006 alone

#### Scenario: A project nobody has recorded that runs on this machine
- **WHEN** zoo-005's entry lists a project that is not recorded and that zoo-005 runs
- **THEN** the entry offers to keep the project open and to close it

#### Scenario: The page is not connected
- **WHEN** a user chooses Open while the page has no connection to the server
- **THEN** the entry says that nothing was sent
- **AND** nothing is sent later without the user asking again

#### Scenario: Open from a machine that owns none of the project's tabs
- **WHEN** a closed project has tabs on zoo-005 only, is also registered on a laptop, and a user chooses Open in the laptop's entry
- **THEN** the confirmation says the project starts on zoo-005 and not on the laptop
- **AND** confirming opens the project and starts zoo-005
