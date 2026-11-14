## MODIFIED Requirements

### Requirement: A daemon restart restores the work on that machine

Restarting a daemon SHALL replace the process that runs the machine's projects. The daemon SHALL record the projects that were running, replace itself in place, and start those projects again without anyone asking. Terminal agents SHALL keep running in their pane hosts through the restart and SHALL be adopted again by the returning projects. The restart SHALL NOT be presented as an action that ends work. The confirmation for restarting one machine SHALL say that every project running on that machine restarts with the daemon, and SHALL NOT say that the machine's projects keep running or that the daemon does not own them.

#### Scenario: Projects are running when the daemon restarts

- **WHEN** a daemon is restarted on a machine with projects running
- **THEN** the returning daemon starts each of those projects again
- **AND** they are reported as running once they have re-attached

#### Scenario: Terminal agents are running when the daemon restarts

- **WHEN** a daemon is restarted while terminal panes are hosted on the machine
- **THEN** their agents keep running during the restart
- **AND** each returning project adopts its hosted terminals that are still within their reboot grace

#### Scenario: The user is told what it affects

- **WHEN** the confirmation for a daemon restart is shown
- **THEN** it does not present the restart as ending the work on the machine
- **AND** it says that the machine's projects come back by themselves and that terminal agents keep running

#### Scenario: The confirmation for one machine

- **WHEN** a user opens the restart confirmation for the machine zoo-005 on the Machines page or the mobile machine list
- **THEN** it says "Every project running on zoo-005 restarts with it. Terminal agents normally keep running during the restart, because their hosts are separate processes."
- **AND** it does not say that projects on the machine keep running
