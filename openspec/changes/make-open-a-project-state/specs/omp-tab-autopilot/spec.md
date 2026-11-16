## MODIFIED Requirements

### Requirement: Autopilot is offered only where every part supports it

The autopilot controls SHALL be offered only when the server, the instance that runs the tab's machine, the tab's own agent process and its OMP version all support autopilot. Otherwise the control SHALL be shown disabled with the reason, except that it SHALL NOT be shown at all when the server does not support autopilot. A tab whose agent process was started before that support SHALL be shown as needing an agent reboot and SHALL NOT be rebooted automatically; a tab that has not reported yet SHALL be shown as not reported, not as needing a reboot. While the tab's machine is connected and supports autopilot, Stop and Stop now SHALL stay available when the tab's state is not reported, and starting, Pause and Resume SHALL NOT be offered then; the exception is a tab that its machine reports as paused because its agent process exited. A client or an instance that does not support autopilot SHALL receive nothing it cannot interpret and SHALL behave as before. In a project that runs on several machines, a request SHALL reach only the machine that runs the tab and SHALL be refused, never redirected, when that machine is not running the project.

#### Scenario: A tab started before the update
- **WHEN** a person opens the autopilot control of an OMP tab whose agent process predates autopilot support
- **THEN** the control is disabled with "Reboot this tab's agent to use autopilot. Rebooting ends its turn in progress." and the agent keeps running

#### Scenario: A tab that was just launched
- **WHEN** an OMP tab's agent was launched with autopilot support and has not reported yet
- **THEN** the control says the tab has not reported its autopilot state, and does not ask for a reboot

#### Scenario: An older machine
- **WHEN** a tab runs on zoo-006 whose APAS does not support autopilot
- **THEN** the control is disabled with "zoo-006 runs an older APAS that cannot run autopilot. Reboot zoo-006 to update it."

#### Scenario: An OMP version without the needed support
- **WHEN** the tab's OMP lacks what autopilot needs
- **THEN** the control is disabled with "This tab's OMP version cannot run autopilot."

#### Scenario: A machine where autopilot cannot run
- **WHEN** a tab runs on a machine whose platform has no autopilot support
- **THEN** the control is disabled with "Autopilot is not available for tabs on this machine."

#### Scenario: Stopping while the state is not reported
- **WHEN** a tab's autopilot state is not reported while its machine is connected and supports autopilot
- **THEN** Stop and Stop now are still offered for that tab, and starting, Pause and Resume are not

#### Scenario: An older server
- **WHEN** the web interface is connected to a server that does not support autopilot
- **THEN** no autopilot control is shown

#### Scenario: An older client
- **WHEN** a client that does not support autopilot views a project with a tab on autopilot
- **THEN** it receives no autopilot frame and shows the tab as it did before

#### Scenario: The tab's machine is stopped
- **WHEN** a request arrives for a tab on zoo-005 while only zoo-006 runs the project
- **THEN** it is refused with "zoo-005 is not running this project. Open the project again to start it there."
