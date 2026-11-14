## Purpose
Lets a person leave an interactive Codex terminal tab repeating an instruction on its own, with the same controls, limits and safeguards as autopilot on OMP tabs, driven through Codex's own app-server protocol rather than through anything typed into the terminal.

## ADDED Requirements

### Requirement: A Codex terminal tab can be put on autopilot like an OMP tab

A person who may send input to a Codex terminal tab SHALL be able to start, pause, resume, stop and stop now an autopilot on it from the same pane toolbar control, start dialog, prompt templates, project and idle lists, and mobile browser actions that OMP terminal tabs offer. The prompt, the repeat timing, the optional iteration and time limits, the option to end after an iteration with no tool call, the 16,000-byte prompt limit, the one-autopilot-per-tab rule, the answer-once rule for requests and the rule that a request is never resent SHALL be the same as for OMP tabs. The prompt SHALL be submitted as text; slash commands in it SHALL NOT be run.

#### Scenario: Starting on a Codex tab
- **WHEN** a person opens the autopilot control of a Codex terminal tab that supports autopilot and starts one with the default template and no limit
- **THEN** the tab receives the template's prompt as a new turn of its own conversation, and the control shows the autopilot running

#### Scenario: The same answer rules
- **WHEN** a start request for a Codex tab is not confirmed in time
- **THEN** the person is told it was not confirmed, and the request is not sent again

### Requirement: Codex autopilot submits an iteration only while the conversation is at rest

Autopilot SHALL submit an iteration only when it is due, the tab's conversation is not running a turn (it is idle, or at rest after a failed turn), no question or approval request is waiting in it, and Codex's own goal mode is not active in it. "As soon as each turn ends" SHALL mean five seconds after the conversation became idle; "every N minutes" SHALL be measured from one submission to the next and SHALL wait for the conversation to be idle. Each submission SHALL carry an identifier that marks it as an autopilot iteration. When a person's turn starts at the same moment and the submission is added to that turn instead of starting a new one, that submission SHALL NOT count as an iteration, and autopilot SHALL wait for that turn to end before submitting again.

#### Scenario: A turn ends
- **WHEN** an iteration's turn completes and the repeat timing is "as soon as each turn ends"
- **THEN** the next iteration is submitted five seconds after the conversation is idle

#### Scenario: A question is waiting
- **WHEN** the agent in a tab on autopilot is waiting for an answer to a question or an approval
- **THEN** no iteration is submitted until it has been answered and the conversation is idle

#### Scenario: A person types at the same moment
- **WHEN** a person submits a message in the terminal just before an iteration is due, and the iteration's prompt is added to the person's turn
- **THEN** that submission does not count as an iteration, and the next one is submitted after that turn has ended

### Requirement: How a Codex iteration ended decides what autopilot does next

Autopilot SHALL treat an iteration whose turn completed as done, SHALL count one whose turn failed as failed, and SHALL pause after three failed iterations in a row, waiting 60 seconds after each failure first. A turn reported as interrupted while autopilot was connected to the conversation, when autopilot did not request the interrupt, SHALL pause autopilot with the reason that the turn was interrupted, whether the person pressed Escape in the terminal or interrupted the turn another way. A turn found interrupted only after autopilot had lost its connection while that turn ran, as happens when the app-server stops abruptly, SHALL NOT be treated as a person's interrupt: autopilot SHALL hold until it is connected again and then continue. Stop now SHALL interrupt the turn in progress and end the autopilot; it SHALL NOT be reported as a person's interrupt. An interrupt request that receives no answer SHALL be given up after a bounded wait and reported as not confirmed.

#### Scenario: Escape during an iteration
- **WHEN** a person presses Escape in a Codex tab while an iteration's turn is running
- **THEN** autopilot is shown as "Paused: the turn was interrupted", and nothing is submitted until someone resumes it

#### Scenario: A failed turn
- **WHEN** an iteration's turn ends as failed for the third time in a row
- **THEN** autopilot is shown as paused after three failed iterations

#### Scenario: Stop now
- **WHEN** a person chooses Stop now while an iteration is running
- **THEN** the turn is interrupted, and autopilot ends as stopped rather than paused

### Requirement: Codex autopilot does not continue without the APAS instance that runs it

The loop of a Codex tab's autopilot SHALL run in the APAS project instance on the tab's machine. When that instance stops or restarts, including at a daemon reboot, the autopilot SHALL be paused, with a reason that says APAS on that machine restarted, and SHALL NOT continue until someone resumes it. While the instance runs, the holds of OMP autopilot SHALL apply: autopilot SHALL hold when the instance has not reached the server for more than ten minutes or when the server it last registered with cannot control autopilot, SHALL continue when control returns, and SHALL pause after four hours of holding. When the instance loses its connection to the tab's Codex conversation, autopilot SHALL hold until it is connected again.

#### Scenario: A daemon reboot
- **WHEN** the machine's daemon is rebooted while one of its Codex tabs is on autopilot
- **THEN** after the reboot that tab's autopilot is shown paused because APAS on that machine restarted, and its agent and conversation are unchanged

#### Scenario: The Codex app-server restarts
- **WHEN** the app-server that hosts a tab's conversation restarts while the tab is on autopilot
- **THEN** autopilot holds while the app-server is unreachable or shutting down, and continues once the conversation is reachable again
- **AND** if the app-server does not come back, or the tab's Codex has stopped reconnecting and must be rebooted, autopilot keeps holding and pauses after four hours like any other hold

#### Scenario: A failed turn followed by a resume
- **WHEN** an iteration's turn fails and the conversation is left at rest in an error state
- **THEN** the next iteration is submitted when it is due, without waiting for an idle state that does not come

### Requirement: Codex's own goal mode is shown and respected

APAS SHALL show when Codex's goal mode is active or paused in a tab and SHALL NOT switch it. While goal mode is active in a tab, autopilot SHALL NOT start or resume there, and a running autopilot SHALL pause with the reason that Codex's goal mode is on.

#### Scenario: Goal mode switched on in the terminal
- **WHEN** a person sets a goal in a Codex tab that is on autopilot
- **THEN** autopilot is shown paused because Codex's goal mode is on, and the goal keeps running as Codex runs it

### Requirement: Leaving the conversation ends a Codex autopilot

When the tab's Codex conversation is replaced, for example by starting a new one or resuming another one in the terminal, the autopilot SHALL end with the reason that the conversation changed and SHALL NOT submit to the new conversation. The change SHALL be detected when the tab starts another user conversation, even though the previous conversation stays reachable through the same tab for a while; autopilot SHALL NOT submit to the previous conversation after that.

#### Scenario: A new conversation in the tab
- **WHEN** a person starts a new conversation in a Codex tab that is on autopilot
- **THEN** autopilot ends with "Ended: the conversation changed"

### Requirement: Codex iterations are kept apart from a person's messages

In the Conversation view of a Codex tab, each iteration that autopilot submitted SHALL appear as one iteration marker with its number, with the prompt on the first one, and SHALL NOT appear as a message a person sent. Messages a person typed SHALL keep appearing as their messages.

#### Scenario: Reading a Codex tab's conversation
- **WHEN** a person opens the Conversation view of a Codex tab after three iterations
- **THEN** it shows three iteration markers and the agent's replies, and no user bubble for those prompts

### Requirement: Codex autopilot is offered only where every part supports it

The control SHALL be offered on a Codex terminal tab only when the server, the instance that runs the tab's machine, the tab's Codex conversation, the tab's Codex version and the version of the Codex app-server hosting that conversation all support it; otherwise it SHALL be shown disabled with the reason, as for OMP tabs. A Codex version, of the tab or of the app-server, that APAS has not been checked against SHALL be reported as unsupported with "This tab's Codex version cannot run autopilot." A viewer, instance or server that does not support Codex autopilot SHALL receive nothing it cannot interpret and SHALL behave as before. APAS SHALL NOT change the user's shared Codex configuration or hooks to provide autopilot, SHALL NOT start, stop or restart the shared Codex app-server, and SHALL NOT answer a question or approval request that Codex sends to it.

#### Scenario: An older machine
- **WHEN** a Codex tab runs on a machine whose APAS does not support Codex autopilot
- **THEN** the control is disabled with "<host> runs an older APAS that cannot run autopilot. Reboot <host> to update it."

#### Scenario: An unverified Codex version
- **WHEN** the tab's Codex version has not been checked for autopilot
- **THEN** the control is disabled with "This tab's Codex version cannot run autopilot."

#### Scenario: An older viewer
- **WHEN** a viewer that does not support Codex autopilot opens a project with a Codex tab on autopilot
- **THEN** it receives no autopilot frame for that tab and shows the tab as before
