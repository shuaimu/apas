## Purpose

Lets a person leave an interactive OMP terminal tab repeating an instruction on
its own within finite limits, and see, pause and stop that from the desktop web
and the mobile browser, as an explicitly uncontained mode beside the contained
autonomous worker.

## ADDED Requirements

### Requirement: A person can put an OMP terminal tab on autopilot

The system SHALL let the account that hosts a tab start autopilot on a terminal tab whose provider is OMP by giving a prompt, a repeat timing and finite limits. While a tab is on autopilot, the tab SHALL submit that prompt again each time an iteration is due and the tab has settled, until autopilot pauses or ends. A start SHALL carry a non-empty prompt of at most 16,000 bytes with no control characters other than newline and tab, an iteration limit between 1 and 1,000, and a time limit between one minute and seven days that is not shorter than the repeat interval; a token limit is optional. A start that breaks one of these bounds SHALL be refused with the reason, and the bounds SHALL be checked again on the tab's machine and by the tab itself. The system SHALL supply no prompt of its own; the start dialog SHALL offer the prompt, timing and limits last used in the tab's current conversation. A start for a tab that is on autopilot or has a paused autopilot SHALL be refused and SHALL leave that autopilot unchanged.

#### Scenario: Start on an idle tab
- **WHEN** the hosting account starts autopilot on an idle OMP terminal tab with a prompt, "as soon as each turn ends", 20 iterations and 8 hours
- **THEN** the tab submits the prompt once and reports autopilot as on with those limits
- **AND** the tab is shown as Working

#### Scenario: Start while the tab is working
- **WHEN** autopilot is started on a tab whose agent is in the middle of a turn
- **THEN** nothing is submitted until that turn ends, and autopilot is shown as starting
- **AND** the first iteration is submitted once the tab has settled

#### Scenario: A start outside the bounds is refused
- **WHEN** a start carries an empty prompt, no iteration limit, no time limit, 5,000 iterations, 30 days, or a time limit shorter than its repeat interval
- **THEN** it is refused with the reason and nothing is submitted to the tab

#### Scenario: A tab already on autopilot
- **WHEN** a second start arrives for a tab that is on autopilot or has a paused autopilot
- **THEN** it is refused with "This tab is already on autopilot. Stop it first."
- **AND** the existing autopilot keeps its prompt, limits and progress

#### Scenario: Start again after an autopilot ended
- **WHEN** the hosting account opens the start dialog on a tab whose last autopilot ended at its time limit
- **THEN** the dialog shows that autopilot's prompt, timing and limits, ready to change
- **AND** nothing is sent until the person confirms the start

#### Scenario: Tabs that cannot be put on autopilot
- **WHEN** the selected tab is not a terminal tab, runs another provider, or is read-only legacy bot history
- **THEN** no autopilot control is offered for it, and a start sent for it is refused

### Requirement: Autopilot submits an iteration only when it is due and the tab has settled

Autopilot SHALL submit an iteration only when the tab has no turn in progress, no queued message, no open question, no compaction in progress and no background work that will resume its turn, and the iteration is due. With "as soon as each turn ends", an iteration SHALL be due five seconds after the tab settles. With a timed repeat of between one minute and 24 hours, an iteration SHALL be due that long after the previous iteration was submitted, and never sooner than five seconds after the tab settles. The first iteration SHALL be due as soon as the tab has settled. A message a person sends to the tab SHALL be handled as an ordinary turn, SHALL NOT replace the repeated prompt and SHALL NOT move a timed repeat's schedule. While autopilot waits, the tab SHALL report whether it waits for the next iteration's time, for the tab to settle, or for background work.

#### Scenario: Timed repeat
- **WHEN** a tab on autopilot repeats every 15 minutes, submits an iteration at 14:15 and that turn ends at 14:18
- **THEN** the next iteration is submitted at 14:30
- **AND** until then the tab is shown as waiting for its next iteration at 14:30

#### Scenario: A turn outlasts the interval
- **WHEN** that iteration's turn instead ends at 14:40
- **THEN** the next iteration is submitted five seconds after the tab settles, and the one after it is due 15 minutes after that submission

#### Scenario: First iteration of a timed start
- **WHEN** autopilot is started with a timed repeat on an idle tab
- **THEN** the first iteration is submitted at once, not one interval later

#### Scenario: A person steers a tab on autopilot
- **WHEN** a person sends a message at 14:20 to a tab that is waiting for its next iteration at 14:30
- **THEN** the message is handled as an ordinary turn
- **AND** the next iteration is still due at 14:30, with the original prompt, once that turn has ended

#### Scenario: The agent asks a question
- **WHEN** the agent asks a question during an iteration
- **THEN** the tab is shown as Pending answer and no further iteration is submitted while the question is open

#### Scenario: Background work is still running
- **WHEN** a turn ends while background work it is waiting for will resume the tab
- **THEN** autopilot is shown as waiting for background work, and no iteration is submitted until that work has been delivered and the tab has settled

#### Scenario: Background work that never finishes
- **WHEN** the background work a turn is waiting for never finishes
- **THEN** autopilot stays shown as waiting for background work until a person stops it or its time limit ends it

### Requirement: Autopilot ends by itself within its limits

Autopilot SHALL submit no more iterations than its iteration limit and SHALL end when the turn of the last one ends. When its time limit passes it SHALL end then, whatever it is doing; time spent paused, waiting or holding SHALL count toward that limit, and autopilot SHALL also end at once when the limit would pass before the next iteration is due. When a token limit was given, autopilot SHALL end after a turn once the tokens the tab's session has recorded since the start reach that limit. No limit SHALL interrupt a turn that is already in progress, so a turn can outlast every limit. When the option "end when an iteration makes no tool call" is on, autopilot SHALL end after an iteration whose turn completes without the agent calling any tool; an iteration that failed or was interrupted SHALL NOT be treated as such an iteration. The start dialog SHALL propose that option only for "as soon as each turn ends". When autopilot ends, the tab SHALL report why and when.

#### Scenario: Iteration limit reached
- **WHEN** an autopilot with a limit of 20 iterations has submitted its twentieth and that turn ends
- **THEN** no further iteration is submitted and autopilot is shown as "Ended: iteration limit reached"

#### Scenario: Time limit passes during a turn
- **WHEN** an autopilot's time limit passes while a turn is in progress
- **THEN** autopilot is shown as "Ended: time limit reached" from that moment and no further iteration is submitted
- **AND** the turn runs to its end as an ordinary turn, and the tab stays Working until it does

#### Scenario: Time limit passes while waiting
- **WHEN** an autopilot's time limit passes while it waits for background work, for the tab to settle, or for APAS to reconnect
- **THEN** it is shown as "Ended: time limit reached" at that moment, not when it would next have submitted

#### Scenario: No time for another iteration
- **WHEN** an iteration's turn ends and the next iteration would be due after the time limit
- **THEN** autopilot is shown at once as "Ended: the time limit ends before the next iteration is due"

#### Scenario: Time limit passes while paused
- **WHEN** a paused autopilot's time limit passes
- **THEN** it is shown as "Ended: time limit reached" and can no longer be resumed

#### Scenario: Token limit reached
- **WHEN** an autopilot with a token limit ends a turn after which the tokens recorded since its start exceed that limit
- **THEN** no further iteration is submitted and autopilot is shown as "Ended: token limit reached"

#### Scenario: An iteration with nothing to do
- **WHEN** the option to end on an iteration without a tool call is on and an iteration's turn completes without the agent calling any tool
- **THEN** autopilot is shown as "Ended: the last iteration made no tool call"

#### Scenario: A timed check that finds nothing
- **WHEN** that option is off and an iteration's turn completes without the agent calling any tool
- **THEN** autopilot waits for its next iteration as usual

#### Scenario: A failed iteration is not an iteration with nothing to do
- **WHEN** that option is on and an iteration's turn ends in a provider error before the agent called any tool
- **THEN** autopilot does not end, and the iteration is handled as a failed iteration

### Requirement: Repeated failures pause autopilot

After an iteration that failed, because its prompt was not accepted or because its turn ended in an error, autopilot SHALL wait at least one minute before the next iteration. After three consecutive failed iterations it SHALL pause, report the reason, and submit nothing until it is resumed. An iteration that succeeds SHALL reset the count. An iteration that a person interrupted SHALL NOT count as failed.

#### Scenario: The provider keeps refusing
- **WHEN** three iterations in a row end in a provider error
- **THEN** autopilot is shown as "Paused after three failed iterations" and submits nothing further

#### Scenario: One failure then success
- **WHEN** one iteration fails and the next succeeds
- **THEN** autopilot continues and a later single failure does not pause it

#### Scenario: A usage limit overnight
- **WHEN** a tab on autopilot with "as soon as each turn ends" reaches its provider's usage limit
- **THEN** autopilot pauses after three failed iterations, within a few minutes
- **AND** it stays paused after the provider's limit has reset, until the hosting account resumes it

### Requirement: An interrupt in the terminal pauses autopilot

When a person interrupts a turn in a tab that is on autopilot, with Escape or any other terminal action that aborts the turn, autopilot SHALL pause with the reason that the turn was interrupted and SHALL submit nothing until it is resumed. This SHALL apply to a turn the person started as well as to an iteration, and to an iteration that is interrupted before its turn begins. The interrupt that Stop now requests SHALL NOT be treated as a person's interrupt.

#### Scenario: Escape during an iteration
- **WHEN** a person presses Escape in the terminal while an iteration's turn is in progress
- **THEN** autopilot is shown as "Paused: the turn was interrupted"
- **AND** the prompt is not submitted again until the hosting account resumes it

#### Scenario: A message that interrupts the turn
- **WHEN** a person sends a message from the terminal in a way that interrupts the turn in progress
- **THEN** their message is handled, and autopilot is shown as "Paused: the turn was interrupted"

#### Scenario: Escape between iterations
- **WHEN** a person presses Escape in the terminal while autopilot waits for its next iteration and no turn is in progress
- **THEN** nothing is interrupted and autopilot stays as it was; Pause and Stop are the controls for that

### Requirement: Autopilot can be paused, resumed and stopped from the desktop web and the mobile browser

Pause SHALL stop further iterations and let a turn in progress continue. Resume SHALL continue a paused autopilot with the iterations it had left and the same time limit end. Stop SHALL end autopilot and let a turn in progress continue. Stop now SHALL end autopilot and interrupt the turn in progress, whoever started that turn. An iteration already handed to the agent SHALL count as the turn in progress. Each request SHALL name its session and tab, SHALL be authorized again by the server, and SHALL be answered to the requester as accepted, as refused with a reason, or as not confirmed. The displayed state SHALL change only when the tab reports it. A pause or a stop that was not confirmed SHALL stay in force until the tab applies it and SHALL NOT be displaced by a weaker request; a start or a resume that was not confirmed SHALL NOT take effect later. A resume SHALL name the autopilot it resumes and SHALL be refused when that is not the tab's paused autopilot. A client SHALL NOT resend a request by itself, and a repeated copy of the same request SHALL have no further effect. Where Stop now is offered, the client SHALL say that background work the turn already launched keeps running and can start a further turn when it finishes.

#### Scenario: Stop during a turn
- **WHEN** a person stops an autopilot whose iteration is in progress
- **THEN** the turn runs to its end, no further iteration is submitted, and autopilot is shown as "Stopped" once the tab reports it

#### Scenario: Stop now
- **WHEN** a person chooses Stop now during an iteration
- **THEN** the turn in progress is interrupted and autopilot is shown as "Stopped" once the tab reports it

#### Scenario: Stop now and background work
- **WHEN** a person is offered Stop now
- **THEN** the control says that Stop now interrupts the turn in progress, and that background work it already launched keeps running and can start another turn when it finishes

#### Scenario: Pause and resume keep progress
- **WHEN** an autopilot that has submitted 7 of 20 iterations is paused and later resumed
- **THEN** it continues from 7 of 20 with the same time limit end

#### Scenario: A stop is not confirmed
- **WHEN** a client sends Stop and is told the request is not confirmed
- **THEN** it shows the request as not confirmed and does not send it again by itself
- **AND** the stop is applied when the tab next reads its requests, and the state shown changes when the tab reports it

#### Scenario: A start is not confirmed
- **WHEN** a client sends a start and is told the request is not confirmed
- **THEN** the tab does not start that autopilot later
- **AND** the person can start again deliberately

#### Scenario: Two requests at once
- **WHEN** a Stop arrives while another autopilot request for the same tab is still being answered
- **THEN** the Stop is taken, not refused as busy

#### Scenario: A weaker request does not displace a stop
- **WHEN** a Pause arrives while a Stop for the same tab has not been applied yet
- **THEN** the Stop is still applied

#### Scenario: Resume of another autopilot
- **WHEN** a resume names an autopilot that is no longer the tab's paused autopilot
- **THEN** it is refused with "This autopilot is no longer current."

#### Scenario: Offline
- **WHEN** a client is offline or its tab's machine is unreachable
- **THEN** the autopilot controls are disabled with the reason and no request is queued for later

### Requirement: Only the account that hosts a tab can start or resume its autopilot through APAS

A start or a resume sent through APAS SHALL be accepted only from the account whose APAS instance runs the tab; on a shared machine that is the machine's owner, also for a project another account created there. Pause, Stop and Stop now SHALL be accepted from anyone who may send input to that tab on its machine. Everyone who can see the tab SHALL see its autopilot state, including its prompt. This requirement governs requests that arrive through APAS. It does not limit what a person or a process with access to the tab's account on that machine can do, the tab's own agent included, and clients SHALL NOT present it as doing so.

#### Scenario: A collaborator tries to start autopilot
- **WHEN** a project member who may type into a tab hosted by another account starts autopilot on it
- **THEN** the start is refused with "Only the account that runs this tab's machine can start autopilot."
- **AND** nothing is submitted to the tab

#### Scenario: A project owner on another account's machine
- **WHEN** the owner of a project starts autopilot on a tab of that project that runs on another account's machine
- **THEN** the start is refused with the same reason

#### Scenario: A collaborator stops autopilot
- **WHEN** that member stops an autopilot the hosting account started
- **THEN** the stop is accepted and applied

#### Scenario: Someone who cannot use the tab
- **WHEN** an account without runtime access to the tab's machine sends any autopilot request
- **THEN** it is refused exactly as input to that tab would be

### Requirement: Autopilot state is reported by the tab and shown as its own attribute

The tab SHALL report whether it has an autopilot and, when it has, its prompt, timing, limits, progress and one of these conditions: starting; running an iteration; waiting, for the next iteration's time, for the tab to settle, or for background work; holding, for APAS on its machine or for the server; paused, and why; or ended, why and when. A tab is on autopilot while its autopilot is starting, running, waiting or holding. The desktop web and the mobile browser SHALL show this beside the tab's Working, Pending answer or Idle status and SHALL NOT replace that status with it; Pending answer SHALL keep precedence. A tab whose next iteration is due within ten seconds SHALL stay Working between iterations. A tab on autopilot that waits longer SHALL NOT be listed as an idle tab awaiting its user; lists of idle tabs SHALL show such tabs in a separately labelled group. A paused autopilot SHALL stay shown with its reason until it is resumed or stopped, and its tab SHALL be listed among idle tabs. An ended autopilot SHALL stay shown with its reason on the tab's autopilot control until another is started, and in lists until a turn next runs in the tab. The state SHALL be reported again after a reconnect or a server restart. While it has not been reported, or is no longer being refreshed, clients SHALL show it as not reported, never as off.

#### Scenario: Immediate repeat does not flicker
- **WHEN** a tab on autopilot with "as soon as each turn ends" finishes a turn and its next iteration is five seconds away
- **THEN** the tab stays Working, no Idle status is sent for it in between, and it does not enter any idle list

#### Scenario: Waiting for a later iteration
- **WHEN** a tab's next iteration is 12 minutes away
- **THEN** its autopilot is shown as waiting with that time, its project-list dot is labelled as on autopilot, and idle lists show it under "On autopilot" rather than among idle tabs

#### Scenario: A paused autopilot needs someone
- **WHEN** an autopilot pauses after three failed iterations
- **THEN** the tab is listed among idle tabs with "Paused after three failed iterations"

#### Scenario: An ended autopilot
- **WHEN** an autopilot ends at its iteration limit
- **THEN** the tab's control shows "Ended: iteration limit reached" with when it ended, and the tab is listed among idle tabs with that reason
- **AND** the tab no longer carries the on-autopilot marker

#### Scenario: A person's turn while on autopilot
- **WHEN** a person's message is being handled in a tab that is on autopilot
- **THEN** autopilot is shown as waiting for the tab to settle

#### Scenario: The tab's machine drops off
- **WHEN** the machine running a tab on autopilot disconnects from the server
- **THEN** clients show the tab's autopilot state as not reported until that machine reports it again

#### Scenario: The server restarts
- **WHEN** the server restarts while a tab is on autopilot
- **THEN** the tab's machine reports the autopilot state again after it reconnects and clients show it without anyone asking

#### Scenario: The tab stops refreshing its state
- **WHEN** a tab's autopilot state has not been refreshed for 90 seconds while its machine is connected
- **THEN** clients show it as not reported, with when it was last reported

### Requirement: Autopilot iterations are kept apart from a person's messages

An iteration SHALL be recorded and displayed as submitted by autopilot. It SHALL NOT be shown as a message the person sent, SHALL NOT advance the session's time of last user input, and SHALL NOT be counted as a person's prompt or as an unconfirmed send. The conversation view SHALL mark where each iteration starts. A message SHALL be treated as an iteration only when the tab's own session records that autopilot submitted it; a message a person sent SHALL never be shown as an iteration.

#### Scenario: Conversation view
- **WHEN** the third of 20 iterations is submitted
- **THEN** the conversation shows "Autopilot · iteration 3 of 20" before the agent's reply, and no user message bubble for it

#### Scenario: Lists are not reordered by iterations
- **WHEN** a tab on autopilot submits iterations for an hour while its user does nothing
- **THEN** lists ordered by the user's last input keep the session where the user's own last message put it

#### Scenario: An iteration that was never submitted
- **WHEN** the agent drops an iteration before its turn starts and a person then sends a message
- **THEN** the person's message is shown as theirs, and no iteration marker is shown for the dropped one

#### Scenario: A client without autopilot support
- **WHEN** a client that does not support autopilot shows a conversation that contains iterations
- **THEN** each iteration appears as a generic tool entry and not as a message from the person

### Requirement: Autopilot does not continue without its controller, through a long server outage, or across an agent restart

Autopilot SHALL submit no iteration once the APAS process that controls the tab has been absent for more than one minute, once the tab's machine has been disconnected from the server for more than ten minutes, or while the server it is connected to cannot control autopilot. It SHALL continue within the same limits when contact returns within one hour, and SHALL pause when it has been held for longer. When the tab's agent process is restarted, autopilot SHALL come back paused with the iterations it had left and the same time limit end, and SHALL NOT resume by itself. When the agent process exits without APAS having stopped it, the tab SHALL be shown as paused for that reason. Stopping or suspending the project SHALL stop autopilot's iterations together with the tab's agent process, after which autopilot comes back paused as after any agent restart; closing the tab or deleting the project SHALL discard it. When the tab leaves the conversation its autopilot was started in, autopilot SHALL end. Confirmations for closing a tab, rebooting its agent and closing the project SHALL say when a tab on autopilot is affected and what becomes of its autopilot.

#### Scenario: A long server outage
- **WHEN** a machine loses the server for 30 minutes while a tab is on autopilot
- **THEN** no iteration is submitted after the tenth minute, and a turn already in progress runs to its end
- **AND** iterations continue within the same limits after the machine reconnects

#### Scenario: An outage longer than an hour
- **WHEN** a tab on autopilot has been held for more than an hour
- **THEN** it is shown, once it can report again, as "Paused: APAS was unreachable for more than an hour"
- **AND** nothing is submitted until the hosting account resumes it

#### Scenario: The APAS process is replaced
- **WHEN** the machine's APAS process is rebooted to update while a tab whose agent survives that reboot is on autopilot
- **THEN** no iteration is submitted once that process has been absent for more than a minute, and autopilot continues after it has taken the tab back

#### Scenario: A server that cannot control autopilot
- **WHEN** a machine reconnects to a server that does not support autopilot while a tab is on autopilot
- **THEN** no iteration is submitted while that lasts

#### Scenario: The agent process is restarted
- **WHEN** a tab on autopilot has its agent rebooted, or its project stopped and started again
- **THEN** the tab comes back with autopilot shown as "Paused: the agent was restarted", with the iterations it had left and the same time limit end
- **AND** nothing is submitted until the hosting account resumes it

#### Scenario: The agent process exits by itself
- **WHEN** the agent process of a tab on autopilot exits without APAS having stopped it
- **THEN** the tab is shown with autopilot "Paused: the agent process exited"

#### Scenario: The tab is closed
- **WHEN** a person closes a tab that is on autopilot
- **THEN** the closing confirmation says the tab is on autopilot, and closing ends it with the tab's agent

#### Scenario: Rebooting the agent of a tab on autopilot
- **WHEN** a person asks to reboot the agent of a tab that is on autopilot
- **THEN** the confirmation says autopilot will come back paused and that only the hosting account can resume it

#### Scenario: Closing a project with a tab on autopilot
- **WHEN** a person asks to close a project in which a tab is on autopilot
- **THEN** the confirmation names that tab and says its autopilot will come back paused when the project is started again

#### Scenario: A new conversation in the terminal
- **WHEN** a person types `/new` into a tab that is on autopilot
- **THEN** autopilot is shown as "Ended: the conversation was changed in the terminal"

### Requirement: OMP's own goal and loop modes are shown

Where a tab's OMP reports that its built-in goal mode or loop mode is active, however it was switched on, the desktop web and the mobile browser SHALL show that the tab continues by itself under that mode, with what OMP reports about it, SHALL NOT present the tab as idle between that mode's turns, and SHALL say how to stop the mode in the terminal. A mode that OMP reports as paused SHALL be shown as paused. While loop mode is active or paused, the indication SHALL say that a message sent to the tab becomes the prompt the loop repeats. APAS SHALL NOT switch these modes on or off. Autopilot SHALL NOT start or resume while OMP's goal, loop or plan mode is in any state other than off, or while OMP's autoresearch is on, and SHALL pause if one of them is switched on while the tab is on autopilot. Where OMP does not report its modes, clients SHALL show nothing about them.

#### Scenario: A goal typed into the terminal
- **WHEN** a person types `/goal` with an objective into an OMP tab's terminal
- **THEN** the tab is shown as in OMP's own goal mode with that objective and its token budget, and with how to stop it in the terminal
- **AND** the tab does not enter an idle list between that goal's turns

#### Scenario: A paused loop
- **WHEN** OMP reports its loop mode as paused
- **THEN** the tab is shown as in OMP's loop mode, paused, and may be listed as idle
- **AND** the indication says that a message sent now becomes the prompt the loop repeats

#### Scenario: Autopilot and OMP's own mode
- **WHEN** a start arrives for a tab whose OMP goal mode is on
- **THEN** it is refused with "This tab is in OMP's own goal mode. Stop that first."

#### Scenario: OMP's mode is switched on during autopilot
- **WHEN** a person types `/loop` into a tab that is on autopilot
- **THEN** autopilot is shown as "Paused: OMP's own loop mode was switched on"

#### Scenario: Autoresearch
- **WHEN** a start arrives for a tab in which OMP's autoresearch is on
- **THEN** it is refused with "This tab is running OMP's autoresearch. Type /autoresearch off in the terminal first."

### Requirement: Autopilot is presented as an uncontained mode and never as a worker

Before a start is sent, the client SHALL state what the mode is and is not. It SHALL name the machine the tab keeps running on, and say that the tab runs there as the account APAS runs under, with the tab's own permissions including tools that run without asking, so that it can change, commit and push from that machine, and that it spends the OMP login configured there. It SHALL say that APAS adds no sandbox, independent review or spending limit. It SHALL show the limits chosen, and say that they are checked between turns, so that a turn in progress can outlast them, and that they are kept by code inside the agent's own process and are not a security boundary. It SHALL say that everyone who can open the session can read the prompt, that slash commands and skills in the prompt are not run, that a question the agent asks waits until someone answers it, and whether the tab works in the project's shared checkout. Clients SHALL NOT describe an ended autopilot's work as verified or delivered, and SHALL describe a token limit as approximate and not as a spending limit. An autopilot SHALL NOT be counted, listed, offered or converted as an autonomous worker and SHALL NOT satisfy any worker readiness or approval. Legacy bot history SHALL stay read-only and SHALL NOT be started or converted through autopilot.

#### Scenario: The start dialog
- **WHEN** a person opens the start dialog for a tab on zoo-006 that works in the project's shared checkout
- **THEN** the dialog names zoo-006, says the tab keeps running there with its own permissions and its OMP login and that APAS adds no sandbox, review or spending limit, shows the chosen limits, and says the tab works in the shared checkout
- **AND** it says the limits are checked between turns, are kept inside the agent's own process and are not a security boundary

#### Scenario: A timed check has no end of its own
- **WHEN** a person chooses a timed repeat
- **THEN** the dialog says it runs until a limit is reached or a person stops it

#### Scenario: Worker surfaces
- **WHEN** a project has a tab on autopilot and no autonomous worker
- **THEN** the project's worker inventory and the cross-project Workers view list nothing for it

#### Scenario: Legacy bot history
- **WHEN** a start is sent for a tab that is read-only legacy bot history
- **THEN** it is refused and the history is unchanged

### Requirement: Autopilot is offered only where every part supports it

The autopilot controls SHALL be offered only when the server, the instance that runs the tab's machine, the tab's own agent process and its OMP version all support autopilot. Otherwise the control SHALL be shown disabled with the reason, except that it SHALL NOT be shown at all when the server does not support autopilot. A tab whose agent process was started before that support SHALL be shown as needing an agent reboot and SHALL NOT be rebooted automatically; a tab that has not reported yet SHALL be shown as not reported, not as needing a reboot. Stop and Stop now SHALL stay available whenever the tab's machine supports autopilot, including while the tab's state is not reported. A client or an instance that does not support autopilot SHALL receive nothing it cannot interpret and SHALL behave as before. In a project that runs on several machines, a request SHALL reach only the machine that runs the tab and SHALL be refused, never redirected, when that machine is not running the project.

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
- **THEN** Stop and Stop now are still offered for that tab

#### Scenario: An older server
- **WHEN** the web interface is connected to a server that does not support autopilot
- **THEN** no autopilot control is shown

#### Scenario: An older client
- **WHEN** a client that does not support autopilot views a project with a tab on autopilot
- **THEN** it receives no autopilot frame and shows the tab as it did before

#### Scenario: The tab's machine is stopped
- **WHEN** a request arrives for a tab on zoo-005 while only zoo-006 runs the project
- **THEN** it is refused with "zoo-005 is not running this project. Start it on zoo-005 first."

### Requirement: The mobile browser shows and controls autopilot

On the mobile-browser session screen, starting autopilot SHALL be offered among the selected tab's occasional actions, disabled with the reason for a viewer who is not the hosting account. While the selected tab is on autopilot or has a paused autopilot, the screen SHALL show a state notice, which is not one of the occasional actions, with the condition, the Pause or Resume, Stop and Stop now actions, and a way to read the prompt, timing, limits, progress and time limit end; sending a message SHALL remain possible. The screen SHALL show an ended autopilot's reason with a way to start again, and SHALL show OMP's own modes as the desktop does. The tab's chip SHALL mark a tab that is on autopilot without widening the chip row.

#### Scenario: Starting from a phone
- **WHEN** the hosting account opens the tab actions of an OMP tab on a phone
- **THEN** "Start autopilot" is offered there and opens the same prompt, timing and limit choices as the desktop

#### Scenario: A viewer who is not the hosting account
- **WHEN** another account opens the tab actions of that tab on a phone
- **THEN** "Start autopilot" is disabled with "Only the account that runs this tab's machine can start autopilot."

#### Scenario: A tab on autopilot on a phone
- **WHEN** the selected tab is waiting for its next iteration
- **THEN** the screen shows that with the time, offers Pause and Stop, and the message composer stays usable

#### Scenario: Stop now from a phone
- **WHEN** a person chooses Stop on a phone while an iteration is in progress
- **THEN** they can choose between letting the turn finish and interrupting it now

#### Scenario: Reading the prompt on a phone
- **WHEN** a person opens the details of a tab's autopilot on a phone
- **THEN** the prompt, the timing, the limits, the progress and the time limit end are shown
