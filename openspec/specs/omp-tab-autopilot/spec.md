# omp-tab-autopilot Specification

## Purpose
Lets a person leave an interactive OMP terminal tab repeating an instruction on
its own, until a limit they set or until someone stops it, and see, pause and
stop that from the desktop web and the mobile browser, as an explicitly
uncontained mode beside the contained autonomous worker.
## Requirements
### Requirement: A person can put an OMP terminal tab on autopilot

The system SHALL let anyone who may send input to a terminal tab whose provider is OMP start autopilot on it by giving a prompt, a repeat timing and, if they want them, limits. While a tab is on autopilot, the tab SHALL submit that prompt again each time an iteration is due and the tab has settled, until autopilot pauses or ends. A start SHALL carry a non-empty prompt of at most 16,000 bytes with no control characters other than newline and tab, and a repeat timing that is either "as soon as each turn ends" or an interval between one minute and 24 hours. An iteration limit and a time limit SHALL each be optional, and the start dialog SHALL propose none. A start SHALL say, for each of the two, either the limit or that there is none. When set, an iteration limit SHALL be between 1 and 1,000,000, and a time limit SHALL be between one minute and 365 days and not shorter than the repeat interval. A start that breaks one of these bounds, or that does not say whether it has an iteration limit or a time limit, SHALL be refused with the reason, and the bounds SHALL be checked again on the tab's machine and by the tab itself. The start dialog SHALL offer the prompt, timing and limits of the last autopilot the tab reports. For a tab that reports no earlier autopilot, the dialog SHALL offer the default prompt template as the text of the prompt field, where the person can read and change it. The prompt a start carries SHALL be the text the prompt field holds when the person confirms the start, and nothing else: the server, the tab's machine and the tab SHALL supply no prompt of their own. A start for a tab that is on autopilot or has a paused autopilot SHALL be refused and SHALL leave that autopilot unchanged.

#### Scenario: Start on an idle tab
- **WHEN** a person starts autopilot on an idle OMP terminal tab with a prompt, "as soon as each turn ends", 20 iterations and 8 hours
- **THEN** the tab submits the prompt once and reports autopilot as on with those limits
- **AND** the tab is shown as Working

#### Scenario: Start without a limit
- **WHEN** a person starts autopilot and leaves the iteration limit and the time limit at "No limit", as the start dialog proposes
- **THEN** the tab submits the prompt once and reports autopilot as on with no iteration limit and no time limit

#### Scenario: Start while the tab is working
- **WHEN** autopilot is started on a tab whose agent is in the middle of a turn
- **THEN** nothing is submitted until that turn ends, and autopilot is shown as starting
- **AND** the first iteration is submitted once the tab has settled

#### Scenario: A start outside the bounds is refused
- **WHEN** a start carries an empty prompt, an iteration limit of zero, a time limit of ten seconds, a repeat interval of ten seconds, or a time limit shorter than its repeat interval
- **THEN** it is refused with the reason and nothing is submitted to the tab

#### Scenario: A start that does not say whether it is limited
- **WHEN** a start carries neither a number nor "none" for its iteration limit or for its time limit
- **THEN** it is refused with the reason, and no autopilot is started

#### Scenario: A tab already on autopilot
- **WHEN** a second start arrives for a tab that is on autopilot or has a paused autopilot
- **THEN** it is refused with "This tab is already on autopilot. Stop it first."
- **AND** the existing autopilot keeps its prompt, limits and progress

#### Scenario: Start again after an autopilot ended
- **WHEN** a person opens the start dialog on a tab whose last autopilot ended at its time limit
- **THEN** the dialog shows that autopilot's prompt, timing and limits, ready to change
- **AND** nothing is sent until the person confirms the start

#### Scenario: Start on a tab with no earlier autopilot
- **WHEN** a person opens the start dialog on a tab that reports no earlier autopilot
- **THEN** the prompt field holds the whole text of the default template, "Implement from a design document", and both limits read "No limit"
- **AND** nothing is sent until the person confirms the start

#### Scenario: Tabs that cannot be put on autopilot
- **WHEN** the selected tab is not a terminal tab, runs another provider, or is read-only legacy bot history
- **THEN** no autopilot control is offered for it, and a start sent for it is refused

### Requirement: Autopilot submits an iteration only when it is due and the tab has settled

Autopilot SHALL submit an iteration only when the tab has no turn in progress, no queued message, no open question, no automatic compaction or retry in progress and no background work that will resume its turn, and the iteration is due. The tab cannot see a compaction or a handoff that a person starts in the terminal; an iteration submitted while one is running SHALL wait until it has ended and SHALL count as the turn in progress. With "as soon as each turn ends", an iteration SHALL be due five seconds after the tab settles. With a timed repeat of between one minute and 24 hours, an iteration SHALL be due that long after the previous iteration was submitted, and never sooner than five seconds after the tab settles. The first iteration SHALL be due as soon as the tab has settled. A message a person sends to the tab SHALL be handled as an ordinary turn, SHALL NOT replace the repeated prompt and SHALL NOT move a timed repeat's schedule. While autopilot waits, the tab SHALL report whether it waits for the next iteration's time, for the tab to settle, or for background work.

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
- **THEN** autopilot stays shown as waiting for background work until a person stops it or its time limit, if it has one, ends it

#### Scenario: A compaction typed in the terminal
- **WHEN** an iteration becomes due while a `/compact` that a person typed in the terminal is running
- **THEN** the iteration is submitted, and the agent holds it until the compaction has ended
- **AND** it counts as the turn in progress from its submission, so nothing else is submitted meanwhile

### Requirement: Autopilot ends by itself at a limit it was given

An autopilot with an iteration limit SHALL submit no more iterations than that limit and SHALL end when the turn of the last one ends. An autopilot with a time limit SHALL end when that limit passes, whatever it is doing; time spent paused, waiting or holding SHALL count toward the limit, and autopilot SHALL also end at once when the limit would pass before the next iteration is due. An autopilot with neither limit SHALL continue until a person stops it or another rule of this specification pauses or ends it, and SHALL be shown as having no limit. A limit SHALL NOT interrupt a turn that is already in progress, so a turn can outlast every limit. When the option "end when an iteration makes no tool call" is on, autopilot SHALL end after an iteration whose turn completes without the agent calling any tool; an iteration that failed or was interrupted SHALL NOT be treated as such an iteration. The start dialog SHALL propose that option only for "as soon as each turn ends". When more than one rule applies at the end of the same turn, a rule that ends autopilot SHALL take precedence over one that pauses it, in this order: a stop that was requested, the time limit and the iteration limit; the end for an iteration without a tool call SHALL apply only when no other rule does. When autopilot ends, the tab SHALL report why and when.

#### Scenario: Iteration limit reached
- **WHEN** an autopilot with a limit of 20 iterations has submitted its twentieth and that turn ends
- **THEN** no further iteration is submitted and autopilot is shown as "Ended: iteration limit reached"

#### Scenario: Time limit passes during a turn
- **WHEN** an autopilot's time limit passes while a turn is in progress
- **THEN** autopilot is shown as "Ended: time limit reached" from that moment and no further iteration is submitted
- **AND** the turn runs to its end as an ordinary turn, and the tab stays Working until it does

#### Scenario: Time limit passes while waiting
- **WHEN** an autopilot's time limit passes while it waits for background work or for the tab to settle
- **THEN** it is shown as "Ended: time limit reached" at that moment, not when it would next have submitted

#### Scenario: Time limit passes during a hold
- **WHEN** an autopilot's time limit passes while the tab is held
- **THEN** it ends at that moment, and is shown as "Ended: time limit reached", with that time, once its state is reported again

#### Scenario: No time for another iteration
- **WHEN** an iteration's turn ends and the next iteration would be due after the time limit
- **THEN** autopilot is shown at once as "Ended: the time limit ends before the next iteration is due"

#### Scenario: Time limit passes while paused
- **WHEN** a paused autopilot's time limit passes
- **THEN** it is shown as "Ended: time limit reached" and can no longer be resumed

#### Scenario: No limit
- **WHEN** an autopilot that was started with no iteration limit and no time limit has submitted 300 iterations over three days
- **THEN** it is still on, shown with its iteration count, when it started and "No limit"
- **AND** it continues until a person stops it or another rule pauses or ends it

#### Scenario: Only a time limit
- **WHEN** an autopilot with an 8-hour time limit and no iteration limit reaches that time
- **THEN** it is shown as "Ended: time limit reached", however many iterations it has submitted

#### Scenario: An iteration with nothing to do
- **WHEN** the option to end on an iteration without a tool call is on and an iteration's turn completes without the agent calling any tool
- **THEN** autopilot is shown as "Ended: the last iteration made no tool call"

#### Scenario: A timed check that finds nothing
- **WHEN** that option is off and an iteration's turn completes without the agent calling any tool
- **THEN** autopilot waits for its next iteration as usual

#### Scenario: A failed iteration is not an iteration with nothing to do
- **WHEN** that option is on and an iteration's turn ends in a provider error before the agent called any tool
- **THEN** autopilot does not end, and the iteration is handled as a failed iteration

#### Scenario: The last iteration is interrupted
- **WHEN** the twentieth of 20 iterations is interrupted with Escape
- **THEN** autopilot is shown as "Ended: iteration limit reached", not as paused, because nothing is left to resume

### Requirement: Repeated failures pause autopilot

After an iteration that failed, because the agent refused its prompt or because its turn ended in an error, autopilot SHALL wait at least one minute before the next iteration. After three consecutive failed iterations it SHALL pause, report the reason, and submit nothing until it is resumed. An iteration that succeeds SHALL reset the count, and so SHALL a resume. An iteration that was interrupted, during its turn, before its turn began or while the agent waited to retry it, SHALL NOT count as failed.

#### Scenario: The provider keeps refusing
- **WHEN** three iterations in a row end in a provider error
- **THEN** autopilot is shown as "Paused after three failed iterations" and submits nothing further

#### Scenario: One failure then success
- **WHEN** one iteration fails and the next succeeds
- **THEN** autopilot continues and a later single failure does not pause it

#### Scenario: The agent refuses a prompt
- **WHEN** the agent refuses an iteration's prompt and gives a reason, such as its cost limit
- **THEN** the iteration counts as failed and the next one is submitted no sooner than one minute later

#### Scenario: A usage limit overnight
- **WHEN** a tab on autopilot with "as soon as each turn ends" reaches its provider's usage limit
- **THEN** autopilot pauses after three failed iterations, within a few minutes
- **AND** it stays paused after the provider's limit has reset, until someone resumes it

### Requirement: An interrupt in the terminal pauses autopilot

When a person interrupts a tab that is on autopilot, autopilot SHALL pause and SHALL submit nothing until it is resumed. This SHALL apply when a turn in progress is aborted, with Escape or any other terminal action that aborts it, whether the person or autopilot started that turn; when a retry that the agent is waiting to make, or an automatic compaction, is cancelled; and when an iteration is dropped by an interrupt before its turn begins. In these cases the reason SHALL be that the turn was interrupted. It SHALL also apply when a wait for background work ends without that work resuming the turn, which is what follows Escape on that wait once the work has finished, and what follows when the work a turn was waiting for is killed; autopilot SHALL then pause five seconds after the wait is over, unless a turn has started in between, and the reason SHALL be that the wait for background work was ended in the terminal. The tab cannot tell who interrupted it, so every such interrupt that autopilot did not request itself SHALL be treated as a person's, and the interrupt that Stop now requests SHALL NOT be. An interrupted iteration SHALL NOT count as failed.

#### Scenario: Escape during an iteration
- **WHEN** a person presses Escape in the terminal while an iteration's turn is in progress
- **THEN** autopilot is shown as "Paused: the turn was interrupted"
- **AND** the prompt is not submitted again until someone resumes it

#### Scenario: A message that interrupts the turn
- **WHEN** a person sends a message from the terminal in a way that interrupts the turn in progress
- **THEN** their message is handled, and autopilot is shown as "Paused: the turn was interrupted"

#### Scenario: Escape while the agent waits to retry
- **WHEN** an iteration's turn has ended in a provider error, the agent is waiting to retry it or for a usage limit to reset, and a person presses Escape in the terminal
- **THEN** autopilot is shown as "Paused: the turn was interrupted"
- **AND** the iteration does not count as failed, and nothing is submitted until someone resumes it

#### Scenario: Escape during an automatic compaction
- **WHEN** a person presses Escape in the terminal while the agent is compacting the conversation automatically in a tab on autopilot
- **THEN** autopilot is shown as "Paused: the turn was interrupted"

#### Scenario: An iteration interrupted before its turn begins
- **WHEN** a person interrupts the tab after an iteration was submitted and before its turn has begun, and the agent drops that prompt without giving a reason
- **THEN** autopilot is shown as "Paused: the turn was interrupted", and the iteration does not count as failed

#### Scenario: Escape on a wait for background work
- **WHEN** a person presses Escape in the terminal while a tab on autopilot waits for background work
- **THEN** autopilot stays shown as waiting for background work while that work is still running
- **AND** five seconds after the work has finished without a turn starting, autopilot is shown as "Paused: the wait for background work was ended in the terminal"

#### Scenario: The background job is killed
- **WHEN** the background job that a turn in a tab on autopilot was waiting for is killed
- **THEN** five seconds later, unless a turn has started in between, autopilot is shown as "Paused: the wait for background work was ended in the terminal"

#### Scenario: Escape between iterations
- **WHEN** a person presses Escape in the terminal while autopilot waits for its next iteration and no turn is in progress
- **THEN** nothing is interrupted and autopilot stays as it was; Pause and Stop are the controls for that

### Requirement: Autopilot can be paused, resumed and stopped from the desktop web and the mobile browser

Pause SHALL stop further iterations and let a turn in progress continue. Resume SHALL continue a paused autopilot from the iteration count it had reached, with the same limits and, when it has a time limit, the same time limit end. Stop SHALL end autopilot and let a turn in progress continue. Stop now SHALL end autopilot and interrupt the turn in progress, whoever started that turn. An iteration already handed to the agent SHALL count as the turn in progress. Each request SHALL name its session and tab, SHALL be authorized again by the server, and SHALL be answered to the requester as accepted, as refused with a reason, or as not confirmed. The displayed state SHALL change only when the tab reports it, or when its machine reports that the tab's agent process exited. A pause or a stop that was not confirmed SHALL stay in force until the tab applies it or its agent process is restarted, and SHALL NOT be displaced by a weaker request; after a restart the autopilot is paused as after any agent restart, and the unapplied request is not carried over. A start or a resume SHALL NOT be left pending: the tab takes it while its machine is still waiting for the answer, a few seconds at most, or never. One that the tab did not take SHALL be answered as refused, saying that nothing was started; one that was not confirmed may have been taken, and the tab's reported state then shows it. A request that changes nothing, such as a stop for a tab that has no autopilot, SHALL be answered as accepted. A resume SHALL name the autopilot it resumes and SHALL be refused when that is not the tab's paused autopilot. A client SHALL NOT resend a request by itself, and a repeated copy of the same request SHALL have no further effect. Where Stop now is offered, the client SHALL say that background work already launched keeps running, and that a tab that was only waiting for such work can still start a turn when it finishes.

#### Scenario: Stop during a turn
- **WHEN** a person stops an autopilot whose iteration is in progress
- **THEN** the turn runs to its end, no further iteration is submitted, and autopilot is shown as "Stopped" once the tab reports it

#### Scenario: Stop now
- **WHEN** a person chooses Stop now during an iteration
- **THEN** the turn in progress is interrupted and autopilot is shown as "Stopped" once the tab reports it

#### Scenario: Stop now and background work
- **WHEN** a person is offered Stop now
- **THEN** the control says that Stop now interrupts the turn in progress and that background work already launched keeps running
- **AND** it says that a tab that was only waiting for such work can still start a turn when it finishes

#### Scenario: Pause and resume keep progress
- **WHEN** an autopilot that has submitted 7 of 20 iterations is paused and later resumed
- **THEN** it continues from 7 of 20 with the same time limit end

#### Scenario: A stop is not confirmed
- **WHEN** a client sends Stop and is told the request is not confirmed
- **THEN** it shows the request as not confirmed and does not send it again by itself
- **AND** the stop is applied when the tab next reads its requests, and the state shown changes when the tab reports it
- **AND** if the tab's agent is restarted first, the autopilot comes back paused and the stop is not carried over

#### Scenario: A start is not confirmed
- **WHEN** a client sends a start and is told the request is not confirmed
- **THEN** the tab's reported state shows whether that autopilot started, and one that had not started does not start later
- **AND** the person can start again deliberately

#### Scenario: A start the tab did not take
- **WHEN** the tab does not take a start while its machine waits for the answer
- **THEN** the start is refused with "The tab did not take the request in time. Nothing was started."

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

### Requirement: Anyone who may send input to a tab can control its autopilot through APAS

A start, pause, resume, stop or stop now sent through APAS SHALL be accepted from anyone who may send input to that tab on its machine, and from nobody else. No further permission SHALL be required, and it SHALL NOT matter which account hosts the tab or who started the autopilot. Everyone who can see the tab SHALL see its autopilot state, including its prompt; a viewer who may not send input to the tab SHALL see the autopilot actions disabled with the reason. This requirement governs requests that arrive through APAS. It does not limit what a person or a process with access to the tab's account on that machine can do, the tab's own agent included, and clients SHALL NOT present it as doing so.

#### Scenario: A collaborator starts autopilot
- **WHEN** a project member who may type into a tab hosted by another account starts autopilot on it
- **THEN** the start is accepted and the tab reports autopilot as on
- **AND** the autopilot runs with the OMP login of the tab's machine, as the start dialog said

#### Scenario: A collaborator resumes autopilot
- **WHEN** that member resumes the tab's paused autopilot, which another person started
- **THEN** the resume is accepted and autopilot continues

#### Scenario: A collaborator stops autopilot
- **WHEN** that member stops an autopilot the hosting account started
- **THEN** the stop is accepted and applied

#### Scenario: Someone who cannot use the tab
- **WHEN** an account without runtime access to the tab's machine sends any autopilot request
- **THEN** it is refused exactly as input to that tab would be

#### Scenario: A viewer who cannot use the tab
- **WHEN** a project member without runtime access to the tab's machine opens the tab's autopilot control
- **THEN** the autopilot state and its prompt are shown, and every action is disabled with the reason

### Requirement: Autopilot state is reported by the tab and shown as its own attribute

The tab SHALL report whether it has an autopilot and, when it has, its prompt, timing, limits, the time it started, progress and one of these conditions: starting; running an iteration; waiting, for the next iteration's time, for the tab to settle, or for background work; holding, for APAS on its machine or for the server; paused, and why; or ended, why and when. A tab is on autopilot while its autopilot is starting, running, waiting or holding. The desktop web and the mobile browser SHALL show this beside the tab's Working, Pending answer or Idle status and SHALL NOT replace that status with it; Pending answer SHALL keep precedence. A tab whose next iteration is due within ten seconds SHALL stay Working between iterations. A tab on autopilot that waits longer SHALL NOT be listed as an idle tab awaiting its user; lists of idle tabs SHALL show such tabs in a separately labelled group. A paused autopilot SHALL stay shown with its reason until it is resumed or stopped, and its tab SHALL be listed among idle tabs. An ended autopilot SHALL stay shown with its reason on the tab's autopilot control until another is started or, when it ended because the tab left its conversation, until the tab's agent process restarts, and in lists until a turn that started after the autopilot ended has run. The state SHALL be reported again after a reconnect or a server restart. While it has not been reported, or is no longer being refreshed, clients SHALL show it as not reported, never as off; the one exception is the pause that the tab's machine reports after the tab's agent process has exited.

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
- **WHEN** a tab's autopilot state has not been refreshed for 90 seconds while its machine is connected and its agent process is still running
- **THEN** clients show it as not reported, with when it was last reported

### Requirement: Autopilot iterations are kept apart from a person's messages

An iteration SHALL be recorded and displayed as submitted by autopilot. It SHALL NOT be shown as a message the person sent, SHALL NOT advance the session's time of last user input, and SHALL NOT be counted as a person's prompt or as an unconfirmed send. The conversation view SHALL mark where each iteration starts. The first marker of an autopilot, and the first after each resume, SHALL carry the prompt, so that the stored conversation holds the instruction once. A message SHALL be treated as an iteration only when the tab's own session records that autopilot submitted it; a message a person sent SHALL never be shown as an iteration.

#### Scenario: Conversation view
- **WHEN** the third of 20 iterations is submitted
- **THEN** the conversation shows "Autopilot · iteration 3 of 20" before the agent's reply, and no user message bubble for it

#### Scenario: Conversation view without an iteration limit
- **WHEN** the third iteration of an autopilot with no iteration limit is submitted
- **THEN** the conversation shows "Autopilot · iteration 3" before the agent's reply

#### Scenario: The prompt in the conversation
- **WHEN** the first iteration of an autopilot is submitted
- **THEN** its marker in the conversation can be opened to read the prompt, and the markers of later iterations show the iteration number only

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

Autopilot SHALL submit no iteration once the APAS process that controls the tab has been absent for more than one minute, once the tab's machine has been disconnected from the server for more than ten minutes, or while the server that machine last registered with cannot control autopilot. The third condition SHALL stay true after such a server goes away, until a server that can control autopilot has registered the machine, and SHALL be true while the machine's APAS process has not registered with any server since it started. A tab on autopilot is held from the moment one of these three conditions is true until none is, whether or not an iteration is due and whether or not a turn is in progress, and SHALL report the condition holding while it is held and no turn is in progress. It SHALL continue as before when the hold ends within four hours, and SHALL pause as soon as a hold has lasted four hours. When the tab's agent process is restarted, autopilot SHALL come back paused with its iteration count and, where it has them, the same limits and time limit end, and SHALL NOT resume by itself. When the agent process exits without APAS having stopped it, the tab's machine SHALL report the autopilot as paused for that reason on the tab's behalf, in place of not reported, until the agent is started again, and Resume, Stop and Stop now SHALL NOT be offered for it meanwhile. Stopping or suspending the project SHALL stop autopilot's iterations together with the tab's agent process, after which autopilot comes back paused as after any agent restart; closing the tab or deleting the project SHALL discard it. When the tab leaves the conversation its autopilot was started in, autopilot SHALL end. Confirmations for closing a tab, rebooting its agent and closing the project SHALL say when a tab on autopilot is affected and what becomes of its autopilot.

#### Scenario: A long server outage
- **WHEN** a machine loses the server for 30 minutes while a tab is on autopilot
- **THEN** no iteration is submitted after the tenth minute, and a turn already in progress runs to its end
- **AND** iterations continue as before after the machine reconnects

#### Scenario: A hold longer than four hours
- **WHEN** a tab on autopilot has been held for four hours
- **THEN** it pauses, and is shown, once it can report again, as "Paused: APAS could not control this tab for more than four hours"
- **AND** nothing is submitted until someone resumes it

#### Scenario: A hold between two timed iterations
- **WHEN** a tab on autopilot repeats every six hours, submits an iteration at 00:00, and its machine is disconnected from the server from 01:00 to 05:30
- **THEN** the tab is held from 01:10 and pauses at 05:10, although no iteration was due in that time
- **AND** it is shown, once it reports again, as "Paused: APAS could not control this tab for more than four hours", and the iteration due at 06:00 is not submitted

#### Scenario: The APAS process is replaced
- **WHEN** the machine's APAS process is rebooted to update while a tab whose agent survives that reboot is on autopilot
- **THEN** no iteration is submitted once that process has been absent for more than a minute, and autopilot continues after the new process has taken the tab back and registered with a server that can control autopilot

#### Scenario: A server that cannot control autopilot
- **WHEN** a machine reconnects to a server that does not support autopilot while a tab is on autopilot
- **THEN** no iteration is submitted while that lasts

#### Scenario: A server that cannot control autopilot goes away
- **WHEN** a tab on autopilot is held because its machine registered with a server that does not support autopilot, and that server then becomes unreachable
- **THEN** the tab stays held and submits nothing
- **AND** it continues only after a server that supports autopilot has registered the machine, if the hold has lasted less than four hours

#### Scenario: The agent process is restarted
- **WHEN** a tab on autopilot has its agent rebooted, or its project stopped and started again
- **THEN** the tab comes back with autopilot shown as "Paused: the agent was restarted", with its iteration count and the same limits
- **AND** nothing is submitted until someone resumes it

#### Scenario: The agent process exits by itself
- **WHEN** the agent process of a tab on autopilot exits without APAS having stopped it
- **THEN** the tab is shown with autopilot "Paused: the agent process exited", not as not reported
- **AND** Resume, Stop and Stop now are not offered for it, and the control says "This tab's agent is not running. Reboot it first; its autopilot comes back paused."

#### Scenario: The tab is closed
- **WHEN** a person closes a tab that is on autopilot
- **THEN** the closing confirmation says the tab is on autopilot, and closing ends it with the tab's agent

#### Scenario: Rebooting the agent of a tab on autopilot
- **WHEN** a person asks to reboot the agent of a tab that is on autopilot
- **THEN** the confirmation says autopilot will come back paused and stay paused until someone resumes it

#### Scenario: Closing a project with a tab on autopilot
- **WHEN** a person asks to close a project in which a tab is on autopilot
- **THEN** the confirmation names that tab and says its autopilot will come back paused when the project is started again

#### Scenario: A new conversation in the terminal
- **WHEN** a person types `/new` into a tab that is on autopilot
- **THEN** autopilot is shown as "Ended: the conversation was changed in the terminal"
- **AND** the control offers that autopilot's prompt, timing and limits for a new start until another autopilot is started or the tab's agent process restarts

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

#### Scenario: Plan mode
- **WHEN** a start arrives for a tab whose OMP plan mode is on
- **THEN** it is refused with "This tab is in OMP's plan mode. Leave plan mode first."

#### Scenario: Plan mode is switched on during autopilot
- **WHEN** a person switches on OMP's plan mode in a tab that is on autopilot
- **THEN** autopilot is shown as "Paused: OMP's plan mode was switched on"

#### Scenario: Autoresearch
- **WHEN** a start arrives for a tab in which OMP's autoresearch is on
- **THEN** it is refused with "This tab is running OMP's autoresearch. Type /autoresearch off in the terminal first."

#### Scenario: Autoresearch is switched on during autopilot
- **WHEN** a person switches on OMP's autoresearch in a tab that is on autopilot
- **THEN** autopilot is shown as "Paused: OMP's autoresearch was switched on"

### Requirement: Autopilot is presented as an uncontained mode and never as a worker

Before a start is sent, the client SHALL state what the mode is and is not. It SHALL name the machine the tab keeps running on, and say that the tab runs there as the account APAS runs under, with the tab's own permissions including tools that run without asking, so that it can change, commit and push from that machine, and that it spends the OMP login configured there, whoever starts it. It SHALL say that APAS adds no sandbox, independent review or spending limit. It SHALL show the limits chosen and, when none is set, say that autopilot then runs until someone stops it. It SHALL say that limits are checked between turns, so that a turn in progress can outlast them, and that they are kept by code inside the agent's own process and are not a security boundary. It SHALL say that everyone who can open the session can read the prompt, that slash commands and skills in the prompt are not run, that a question the agent asks waits until someone answers it and may have to be answered in the terminal, and whether the tab works in the project's shared checkout. Clients SHALL NOT describe an ended autopilot's work as verified or delivered, and SHALL NOT describe a limit as a spending limit. An autopilot SHALL NOT be counted, listed, offered or converted as an autonomous worker and SHALL NOT satisfy any worker readiness or approval. Legacy bot history SHALL stay read-only and SHALL NOT be started or converted through autopilot.

#### Scenario: The start dialog
- **WHEN** a person opens the start dialog for a tab on zoo-006 that works in the project's shared checkout
- **THEN** the dialog names zoo-006, says the tab keeps running there with its own permissions and its OMP login and that APAS adds no sandbox, review or spending limit, shows the chosen limits, and says the tab works in the shared checkout
- **AND** it says the limits are checked between turns, are kept inside the agent's own process and are not a security boundary
- **AND** it says that a question the agent asks waits until someone answers it and may have to be answered in the terminal

#### Scenario: No limit chosen
- **WHEN** a person leaves the iteration limit and the time limit at "No limit" in the start dialog
- **THEN** the dialog says that no limit is set and that autopilot then runs until someone stops it

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
- **THEN** it is refused with "zoo-005 is not running this project. Start it on zoo-005 first."

### Requirement: The mobile browser shows and controls autopilot

On the mobile-browser session screen, starting autopilot SHALL be offered among the selected tab's occasional actions. While the selected tab is on autopilot or has a paused autopilot, the screen SHALL show a state notice, which is not one of the occasional actions, with the condition, the Pause or Resume, Stop and Stop now actions, and a way to read the prompt, the timing, the limits or that none is set, when it started, the progress and, when a time limit is set, its end; Resume, Stop and Stop now SHALL NOT be offered for an autopilot that is paused because the tab's agent process exited; sending a message SHALL remain possible. For a viewer who may not send input to the tab, the screen SHALL show the state and SHALL disable every autopilot action with the reason. While the selected tab's state is not reported and its machine is connected and supports autopilot, Stop and Stop now SHALL be offered among the tab's occasional actions, and no state notice is shown. The screen SHALL show an ended autopilot's reason with a way to start again, and SHALL show OMP's own modes as the desktop does. The tab's chip SHALL mark a tab that is on autopilot without widening the chip row.

#### Scenario: Starting from a phone
- **WHEN** a person who may type into an OMP tab opens its tab actions on a phone
- **THEN** "Start autopilot" is offered there and opens the same prompt, timing and limit choices as the desktop

#### Scenario: A collaborator on a phone
- **WHEN** a project member who may type into a tab hosted by another account opens its tab actions on a phone
- **THEN** "Start autopilot" is offered and enabled, as it is for the hosting account

#### Scenario: A viewer who cannot use the tab, on a phone
- **WHEN** a viewer without runtime access to the tab's machine views that tab on a phone while its autopilot is paused
- **THEN** the state notice shows the condition, and Resume and Stop are disabled with the reason

#### Scenario: A tab on autopilot on a phone
- **WHEN** the selected tab is waiting for its next iteration
- **THEN** the screen shows that with the time, offers Pause and Stop, and the message composer stays usable

#### Scenario: Stop now from a phone
- **WHEN** a person chooses Stop on a phone while a turn is in progress in the tab
- **THEN** they can choose between letting the turn finish and interrupting it now

#### Scenario: Stopping from a phone while the state is not reported
- **WHEN** the selected tab's state is not reported while its machine is connected and supports autopilot
- **THEN** Stop and Stop now are offered among the tab's actions, and no state notice is shown

#### Scenario: Reading the prompt on a phone
- **WHEN** a person opens the details of a tab's autopilot on a phone
- **THEN** the prompt, the timing, the limits or that none is set, when it started, the progress and, when a time limit is set, its end are shown

### Requirement: The start dialog offers prompt templates

The start dialog, on the desktop and in the phone's sheet, SHALL have a Template control above the prompt field. The control SHALL offer three built-in templates, "Implement from a design document", "Work through TODO.md" and "Keep the tests green", and the entry "My own text". "Implement from a design document" SHALL be the default: it is what a tab with no earlier autopilot is offered. Choosing a template SHALL replace the text of the prompt field with that template's whole text and SHALL change nothing else: the repeat timing, the limits and the "End when an iteration makes no tool call" option SHALL stay as they were. The control SHALL name a template only while the prompt field holds that template's text unchanged, and SHALL read "My own text" for any other text, so that it cannot disagree with what will be sent. A template is a starting text only: a start SHALL carry exactly the text the prompt field holds, and SHALL carry nothing that says which template the text came from.

Under the control the dialog SHALL show one line that describes the selected template and says how its run ends with the no-tool-call option as it stands. For "Implement from a design document" and "Work through TODO.md" the line SHALL say "It ends by itself when nothing is left to do." when the option is ticked and SHALL say that it ends by itself only if the option is ticked when it is not. For "Keep the tests green" the line SHALL say "It has no end of its own. Set a limit, or stop it yourself." For "My own text" the line SHALL say "The prompt below is submitted unchanged on every iteration."

Text the person wrote SHALL be kept when a template is chosen, and "My own text" SHALL bring it back. While a template has set such text aside, the dialog SHALL say that the text is kept, that "My own text" gets it back, and that editing the prompt replaces it. A template whose text the person edits SHALL become the person's own text. When nothing of the person's is kept, "My own text" SHALL give an empty prompt field, and a start with an empty prompt SHALL be refused as before.

Every built-in template SHALL be plain text within the bounds of a start, SHALL contain no slash command, SHALL describe one step for one iteration, and SHALL tell the agent to commit its step and not to push.

#### Scenario: The default template is started unchanged
- **WHEN** a person opens the start dialog on a tab with no earlier autopilot and confirms the start without changing anything
- **THEN** one start is sent whose prompt is the text of "Implement from a design document", with no iteration limit and no time limit

#### Scenario: Another template is chosen
- **WHEN** a person who has set a timed repeat and an iteration limit chooses "Work through TODO.md"
- **THEN** the prompt field holds that template's text
- **AND** the repeat timing, the limits and the no-tool-call option are as the person left them

#### Scenario: A template that ends by itself, with the option cleared
- **WHEN** "Implement from a design document" is selected and "End when an iteration makes no tool call" is not ticked
- **THEN** the line under the control says the run ends by itself only if that option is ticked
- **AND** the dialog does not tick the option

#### Scenario: A template that has no end of its own
- **WHEN** a person chooses "Keep the tests green"
- **THEN** the line under the control says "It has no end of its own. Set a limit, or stop it yourself."

#### Scenario: A template is edited
- **WHEN** a person changes one character of a template's text in the prompt field
- **THEN** the Template control reads "My own text"
- **AND** a start carries the edited text

#### Scenario: The person's own text is set aside and recovered
- **WHEN** a person types a prompt, then chooses a template, then chooses "My own text"
- **THEN** while the template is shown, the dialog says the person's text is kept
- **AND** after "My own text" the prompt field holds what the person typed

#### Scenario: Nothing of the person's is kept
- **WHEN** a person who has typed nothing chooses "My own text"
- **THEN** the prompt field is empty
- **AND** a start is refused until a prompt is entered

#### Scenario: A tab whose last autopilot ended
- **WHEN** a person opens the start dialog on a tab whose last autopilot ended with a prompt that is not a template's text
- **THEN** the prompt field holds that prompt and the Template control reads "My own text"
- **AND** choosing a template and then "My own text" brings that prompt back

#### Scenario: The last prompt was a template unchanged
- **WHEN** a person opens the start dialog on a tab whose last autopilot ended with the unchanged text of "Work through TODO.md"
- **THEN** the Template control reads "Work through TODO.md"

#### Scenario: The phone's sheet
- **WHEN** a person opens "Start autopilot" on a phone
- **THEN** the sheet has the same Template control above the prompt field, with the same entries and the same default

### Requirement: The start dialog keeps its actions in view

The start dialog, on the desktop and in the phone's sheet, SHALL keep Cancel and Start autopilot visible however far its content is scrolled: the content SHALL scroll and the two actions SHALL stay below it. The reason a start was refused SHALL be shown beside those actions, so that it cannot be scrolled out of sight.

#### Scenario: A window shorter than the dialog
- **WHEN** a person opens the start dialog in a window that is not tall enough to show all of it
- **THEN** the statement, the template, the prompt, the timing and the limits scroll
- **AND** Cancel and Start autopilot stay visible below them

#### Scenario: A start is refused
- **WHEN** a person presses Start autopilot with an empty prompt
- **THEN** the reason is shown beside the two actions, wherever the content is scrolled to

#### Scenario: The phone's sheet
- **WHEN** a person scrolls the start sheet on a phone
- **THEN** Cancel and Start autopilot stay at the bottom of the sheet

