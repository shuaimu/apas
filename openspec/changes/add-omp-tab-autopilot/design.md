## Context

See `proposal.md` for the motivation. This document is a proposed design; no
part of it is implemented, and no claim here was checked by running OMP. Every
statement about OMP comes from reading its source at tag v18.18.9, the version
installed on the zoo hosts, and is marked where it is an inference. The first
task group checks those statements before anything is built on them.

What exists and shapes the approach:

- **The OMP extension.** Every OMP terminal tab that APAS launches on Linux
  loads `omp_activity.mjs` (`--extension`). It reports Working, Pending answer
  and Idle from OMP's own lifecycle events, and bridges question answers. It
  talks to the CLI only through private files in the tab's runtime directory,
  `$XDG_RUNTIME_DIR/apas/panes/<project>/<pane>/`. A report is trusted only
  when its launch id, provider pid, process start time and session directory
  all match. The extension and a launch record (`omp-launch.json`) are written
  together when APAS spawns the provider, and the extension is not reloaded
  when a running pane host is adopted.
- **Pane status.** A pane has one status slot on the wire. Server, web and
  native all read any text other than exactly "Pending answer" as Working, and
  no text as Idle. Session lists carry a separate `usage_limited` block as the
  precedent for an attribute that is not activity. The server also clears a
  terminal tab's status itself when a turn's last message arrives marked
  complete.
- **Conversation records.** A terminal tab's turn reaches the server in a few
  stored forms only: a user input, assistant text, an assistant tool-use
  block, or a result. Nothing stores a neutral notice line.
- **Multi-machine routing.** A pane control is classified by
  `ServerToCli::scope()`, reaches only the instance that owns the pane, and is
  refused when that machine is not running the project. Inbound pane frames
  are accepted only from the owning instance. Older native builds drop their
  socket on a frame type or enum value their bundled schema lacks, but accept
  unknown optional properties. Viewers are not told an instance's
  capabilities.
- **The CLI's connection.** One thread per project polls its tabs every three
  seconds and sends what it finds on the project's single bounded outbound
  queue. That queue is drained only while the CLI is connected. The CLI keeps
  no connection state beyond a counter of registrations, ignores Pongs and
  does not time the server's Pings.
- **Notifications.** Push is delivered to native-app installations only. The
  web has no push path, and no native build has been released.
- **The two neighbours.** The contained worker runs OMP in `--mode rpc-ui`
  inside a container and is default-disabled. The retired bot drove legacy
  `agent` panes from outside the provider; it never ran on OMP or on terminal
  tabs, had no limit of any kind, restarted itself on every CLI start, and
  needed a watchdog for lost wakeups.

What OMP 18.18.9 offers an extension:

- `ctx.sessionModes` (since 18.13.2, interactive TUI only) reads and switches
  the built-in plan, vibe, goal and loop modes; `mode_changed` and
  `goal_updated` report them.
- `pi.sendUserMessage(text, { attribution, deliverAs })` submits a prompt as
  typed text, without running slash commands, skills or prompt templates. On
  an idle session it starts a turn. `attribution: "agent"` records the message
  as agent-initiated. Since 18.18.2 the call rejects when the prompt is
  dropped before a turn starts, and OMP hands a dropped prompt back to the
  terminal's input line.
- `agent_end` reports the end of each agent run, with `willContinue` and
  `awaitingAsyncWork` when it is not a terminal settle. `turn_end` carries the
  assistant message with its stop reason. A turn interrupted with Esc ends
  `aborted` with an "Interrupted by user" marker, and so does a turn aborted
  by an extension's own `ctx.abort()`.
- `ctx.isIdle()` (true whenever nothing is streaming, a manual compaction
  included), `ctx.hasPendingMessages()`, `ctx.abort()` (stops the run; it
  cancels neither background jobs nor queued input),
  `ctx.getAsyncJobSnapshot()`, and `auto_compaction_start`/`_end`.
- Session events: `session_start` once per process, and
  `session_before_switch`/`session_switch` (new, resume, fork),
  `session_before_branch`/`session_branch` and
  `session_before_tree`/`session_tree` when the tab's conversation changes.
- Managed timers (`ctx.setTimeout`, `ctx.setInterval`), `pi.appendEntry()` for
  journal entries, `ctx.sessionManager.getBranch()` and
  `getUsageStatistics()`, `ctx.ui.setStatus()`, `getEditorText()` and
  `setEditorText()`, and the SDK's exported `VERSION`.

## Goals / Non-Goals

**Goals:**

- A stop that needs no mode switch and is never lost. Pause and Stop cancel
  APAS's own timer, Stop now also interrupts the turn, and a stop request
  stays in force until the tab applies it. One iteration already handed to OMP
  cannot be recalled; decision 3 says what happens to it.
- Limits that client code enforces between turns and that a person can read
  back at any time, with a time limit that always ends autopilot.
- A state the server and clients can show truthfully: what the tab reported,
  and when it has not reported.
- No iteration without a controller, and no silent restart of unattended work.
- No change to what a tab does until someone starts autopilot on it.
- Copy that claims only what holds. The limits bound a loop run by code inside
  the agent's own process; they are not a security boundary.

**Non-Goals:**

- Containment, independent verification, credential separation or a spending
  ledger. Those are the worker's, and the copy says autopilot has none of them.
- Driving or stopping OMP's `/goal`, `/loop` or autoresearch from APAS (see
  decisions 1 and 11).
- Push notifications (see decision 10).
- Other providers, the native app, starting autopilot when a tab is created,
  and editing a running autopilot in place.
- Letting the agent end autopilot through a tool. A tool that is active only
  while autopilot is on is the one way a timed check could end itself, but it
  changes what the model sees in the tab and needs prompt guidance to be used
  well. Until it exists, a timed check runs until a limit or a person stops
  it, and the start dialog says so.
- A project or cluster permission (see decision 9).
- Repairing the question bridge for OMP 18.18.3 and later (see Risks).
- Converting or reviving legacy bots.

## Decisions

### 1. The loop is APAS's own, run by its extension; OMP's own modes are observed, not driven

The extension submits each iteration itself with `pi.sendUserMessage`, from a
timer it owns, after the tab has settled. Autopilot's state, counters and
limits live in the extension. OMP's built-in goal and loop modes are read and
reported (decision 11) but never switched on or off by APAS.

Driving `ctx.sessionModes` was the first idea and reading the source ruled it
out. In 18.18.9:

- `set()` changes a mode only while the session is fully idle and answers
  `busy` during a turn, a retry wait, a compaction and startup restore, for
  stop transitions too (`modes/interactive-mode.ts:3746-3789`). A stop cannot
  be one call.
- `ctx.abort()` pauses a goal but leaves the `goal` tool active, so the model
  can resume the goal itself on a later prompt. It does not stop a loop at
  all: only the Escape key pauses one, so a remote stop would have to abort
  and then win an 800 ms race to exit (inferred from
  `controllers/input-controller.ts:595-606`).
- Both modes re-submit 800 ms after each run with no backoff. A loop whose
  runs fail keeps re-submitting until its limit; a goal has no time limit and
  no iteration limit, only a soft token budget and a no-progress latch.
- A goal that hits its budget still reads `active`; a loop that ends reads
  only `inactive`; the remaining loop budget and the loop prompt cannot be
  read. A draft left in the terminal's input line silently stops goal
  continuation.
- Loop mode is not saved: after a provider restart it is gone. An idle prompt
  typed into a looping tab silently becomes the new repeated prompt.
- Neither mode has a wait between iterations, which is what a periodic check
  needs. A loop's only delay is a shell condition that sleeps.

Owning the loop removes each of these. Pause and Stop cancel a timer. Limits
are counters the extension checks. The interval is a timer. State is whatever
the extension says it is.

What this gives up, because OMP's modes are not used:

- the model being told on every prompt that the run continues and how much
  budget is left;
- the model declaring the work complete through the goal tool;
- a stop condition on real project state (the loop's `--until` and `--while`
  shell conditions);
- OMP's own stop when two continuations show the same tool evidence;
- compaction or a fresh session between iterations;
- repeating a slash command, a skill or a prompt template, because
  `sendUserMessage` runs none of them.

What owning the loop costs: the extension sees less than OMP's own loop does.
It cannot see a manual compaction or a prompt that OMP has accepted and not
yet started, so a submission can start later than it was sent (decision 3).
Esc does not know about APAS's loop, so pausing on an interrupt has to be
built (decision 4). A dropped prompt lands in the terminal's input line and
has to be cleared (decision 3).

The design needs OMP 18.18.2 or later: before it, `sendUserMessage` resolved
when a prompt was dropped, so a dropped iteration would leave autopilot
waiting for a turn that never comes. A check that each function exists cannot
see that change, so the extension reads the SDK's `VERSION` as well as
checking for every call it uses, `ctx.sessionModes` included. Task 1.7
confirms the minimum.

Alternative rejected: drive `ctx.sessionModes` for goal mode and use the
extension only for timed repeats. It keeps the racy stop and the invisible
states for half the feature and doubles what must be tested against every OMP
update.

### 2. One autopilot is a prompt, a timing and finite limits

A start carries:

| Field | Meaning | Bounds |
|---|---|---|
| prompt | Submitted as typed text each iteration | 1 to 16,000 bytes (the size of a first instruction today); no control characters other than newline and tab |
| repeat | When the next iteration is due | "as soon as each turn ends" (five seconds after the tab settles), or every 60 s to 24 h, measured from the previous iteration's submission |
| iteration limit | Most iterations it may submit; the first counts | 1 to 1,000 |
| time limit | Autopilot ends at start + this; paused, waiting and holding time counts | 60 s to 7 days, and not shorter than the repeat interval |
| token limit | Optional; tokens the session records after the start | positive |
| end without tool call | End after an iteration whose turn calls no tool | proposed on for "as soon as each turn ends", off otherwise |

A timed repeat is measured from submission to submission, as the retired
bot's interval was. A person's message then does not move the schedule, and
"every 15 minutes" means what it says. When a turn outlasts the interval the
next iteration follows five seconds after the tab settles.

The dialog proposes 20 iterations and 8 hours, or the interval times the
iteration limit when that is longer, capped at seven days. It proposes
nothing for the prompt except what was last used in the tab's current
conversation, which the tab's state report carries (decision 5), so that an
autopilot that ended overnight can be started again without retyping. These
values are looser than the worker's default allowance of two hours and 24
runs a day. That is intended: the worker's allowance is policy for work
nobody starts by hand, and these are editable proposals for the account that
spends its own login.

The bounds are checked three times. The server checks them so that a refusal
carries a reason: on this wire a frame that fails to parse is dropped without
an answer, so the viewer's request keeps the limits optional and the server
validates them. The server forwards a strict form (limits required, unknown
fields rejected, as `JobLimits` and `WorkerLimits` are), which the CLI
re-checks. The extension checks them once more, because it enforces them and
may be older than the CLI that feeds it.

A second start is refused; changing an autopilot is stop and start. Each
start gets an `autopilot_id` (UUID). Conditions: `starting`, `running`,
`waiting` (for the next iteration, for the tab to settle, or for background
work), `holding` (for the controller or for the server), `paused` and
`ended`, the last two with a reason.

Alternative rejected: allow "until I stop it". The retired bot did, and a
standing responsibility with no end is what the worker tier is for. There is
a second reason: every iteration is appended to one OMP conversation, and
the CLI re-reads that file whenever it changes, so the caps also bound how
far one conversation grows. A week is long enough to cover real use and short
enough to force a look. This is the first of the choices listed for the owner
below.

### 3. An iteration is submitted only from a settled tab, as an agent-attributed message

The extension already tracks whether the tab is settled, for activity
reporting, and that tracking handles retries and automatic compaction.
Autopilot builds on it. The tab is settled for autopilot when all of these
hold:

- The last agent run ended terminally (`agent_end` with neither
  `willContinue` nor `awaitingAsyncWork`). Or it ended waiting for background
  work and that wait is over: `ctx.getAsyncJobSnapshot()` shows nothing
  running, nothing queued for delivery and nothing being delivered. OMP emits
  no event when such a wait ends without a wake (a killed job, Esc on the
  wait row), so the extension polls the snapshot from its timer while in that
  state and reports "waiting for background work". This reads OMP's own job
  state; it is not a guess from elapsed time.
- `ctx.isIdle()`, no pending messages, and no open `ask`.
- No automatic compaction or retry is in flight.
- No submission of its own is outstanding (below).
- None of OMP's own self-continuing modes is on (decision 11).

After an ordinary terminal `agent_end` the job snapshot is not consulted. OMP
has already ruled out a wake then, and a background job that never ends, such
as a dev server, must not block autopilot.

When the tab is settled and an iteration is due, the extension does the rest
in one synchronous step from a managed timer. It reads its request files once
more (decision 5), checks the limits, the heartbeat (decision 7) and OMP's
modes, journals the iteration (decision 6), marks a submission outstanding,
and calls
`pi.sendUserMessage(prompt, { attribution: "agent", deliverAs: "aside" })`.
It never awaits that call inside an event handler, which has a 30-second
budget.

`deliverAs: "aside"` starts a turn on an idle session. If a run began in the
gap between the idle check and the call, OMP takes the prompt at that run's
next step boundary. The default delivery would queue it as a steer, which
aborts the run's tool batch.

A submission is outstanding from the call until the extension sees its own
agent-attributed user message start, or the call rejects. An iteration that
OMP has accepted cannot be recalled. `ctx.isIdle()` is also true during a
manual `/compact` or a handoff, which an extension cannot watch without side
effects (a `session_before_compact` handler switches off OMP's background
compaction for the whole session), and a prompt sent then waits behind it. So
while a submission is outstanding the extension submits nothing else, and
Pause and Stop treat it as the turn in progress. After Stop now, if the
outstanding prompt starts, the extension aborts that run as well and reports
"Stopped" only when it has ended. This behaviour is inferred from the source
and is the first thing task 1.2 checks.

A rejected call is journaled as "not submitted". OMP hands a dropped prompt
back to the terminal's input line when that line is empty. The conversation
composer types blind into the same line, so the extension clears the line
when it holds exactly the prompt (`getEditorText()`, `setEditorText("")`). A
rejection caused by a person's interrupt pauses autopilot (decision 4); any
other rejection is a failed iteration.

A turn the person starts is not autopilot's. The extension sees its
`agent_end` like any other and submits the next iteration when it is due and
the tab has settled again.

**How an iteration appears in the conversation.** The marker needs a stored
form, and none of the existing ones is neutral: a user input is stored as the
person's message and moves their counters, and assistant text is stored as the
agent's own words. The marker is therefore carried as an assistant tool-use
block with the reserved name `ApasAutopilotIteration` and the input
`{ autopilot_id, iteration, limit }`, sent with
`terminal_turn_complete: false`. The server already stores tool-use blocks and
acts on one name only (`AskUserQuestion`), so it needs no change. The web
renders this name as the line "Autopilot · iteration 3 of 20". A client that
does not know the name, an older web build or the native app, shows a generic
tool entry. Nothing is published as `UserInput`, so the server's
last-user-input time and prompt counts do not move.

The CLI's transcript reader treats a user message as an iteration only when
both of these hold: its attribution is `agent`, and it pairs with the nearest
earlier unpaired iteration entry of the same autopilot on the active branch of
the session. Other entries and messages may lie between the two. A
user-attributed message is never an iteration, and a "not submitted" entry
cancels the entry it follows. Text is not compared, because OMP stores the
expanded text of a prompt.

Alternative rejected: submit a hidden custom message, as OMP's goal
continuation does. The person at the terminal would then see replies to a
prompt that appears nowhere.

### 4. Limits, the failure brake and interrupts are enforced in the extension

- **Iterations:** a counter incremented when an iteration is submitted. A
  submission OMP rejects does not use one up. Autopilot ends when the turn of
  the last iteration ends.
- **Time:** an absolute deadline fixed at start, with its own managed timer.
  When it fires, autopilot ends at once, whatever it is doing. A turn in
  progress is not interrupted and finishes as an ordinary turn of the tab.
  Autopilot also ends at once when the next iteration would be due after the
  deadline. The deadline is checked again on resume and at `session_start`.
  A turn or a tab that never settles therefore cannot keep a tab "on
  autopilot" past its time limit.
- **Tokens:** `getUsageStatistics()` is read at start and after each turn;
  input, output and cache-write tokens since the start are compared with the
  limit. The journal records the tokens used so far at each pause and end,
  and a resume takes a new baseline, so the count survives an agent restart.
  The check is between turns, so the limit can be overshot by one turn. A
  provider that records no usage never reaches it, and cache reads are not
  counted. The clients call it approximate for these reasons.
- **No tool call:** the extension counts `tool_execution_start` events during
  the iteration's turn. The rule applies only to an iteration whose turn
  completed normally; the failure and interrupt rules take precedence.
- **Failures:** an iteration fails when its submission is rejected for a
  reason other than an interrupt, or when the last assistant message of its
  turn (from `turn_end`) has the stop reason `error`, or `aborted` without
  OMP's user-interrupt marker. After a failure the next iteration waits at
  least 60 s. Three in a row pause autopilot. No backoff is added: for a usage
  limit OMP has its own opt-in, `retry.waitForUsageReset`, which makes the
  turn itself wait for a reset time the provider states and stay abortable.
  The dialog and the runbook point to it.
- **Interrupts:** a turn that ends `aborted` with the user-interrupt marker
  while the tab is on autopilot, and that the extension did not abort itself,
  pauses autopilot with "the turn was interrupted". This covers any turn, not
  only an iteration. Esc, the "." continue shortcut, interrupt-and-send and a
  cancelled `ask` all end a turn that way. The extension's own `ctx.abort()`
  carries the same marker, so it flags its own aborts. A submission dropped by
  an interrupt during prompt setup produces no `agent_end`; it pauses
  autopilot the same way.

All of these are soft in one sense: none stops a turn already running. Stop
now does, with `ctx.abort()`. That call does not cancel background jobs the
turn launched, and on a tab that is only waiting for such a job it does not
end the wait, so a job's result can still start a turn after Stop now. The
clients say so where they offer Stop now.

Alternative rejected: OMP's `--max-time` and `session.maxCost`. `--max-time`
leaves an open tab that silently does no model work, and a cost cap's refusal
emits no `agent_end`. Cost is also zero for subscription logins.

Alternative rejected for now: interrupt the turn in progress when the time
limit passes. It would make the time limit hard, at the price of cutting a
turn off in the middle of an edit. It is listed for the owner below.

### 5. Control and state use the pane's private runtime files

Autopilot adds files beside the question bridge's, with the same trust checks
(private directory and file, launch id, pid and process start):

- `omp-autopilot-stop.json`: a pause, stop or stop now, written by the CLI;
- `omp-autopilot-request.json`: a start or a resume, written by the CLI;
- `omp-autopilot-ack.json`: the extension's results for the last few
  requests, by request id;
- `omp-autopilot.json`: the extension's state report;
- `apas-controller.json`: the CLI's heartbeat (decision 7).

`omp-launch.json` gains a feature marker for autopilot, written when the
provider is spawned (decision 8). The question files are left alone: their
lock is exclusive per pane, and the extension reads that mailbox only while a
dialog is open.

The two request files behave differently on purpose:

- A **pause or stop** stays in its file until the extension has applied it.
  The CLI waits two seconds for the result and otherwise reports "not
  confirmed", but leaves the request in place. A later pause or stop replaces
  a pending one only when it is equal or stronger (pause, then stop, then stop
  now). The request is bound to the launch only, not to the OMP session, and
  the CLI builds it from the launch record without needing the state report,
  so stopping works when the state is unreadable or stale.
- A **start or resume** carries the time it was issued. The CLI removes it
  when its two-second wait ends, and the extension refuses one that is more
  than ten seconds old, so a start that was reported as not confirmed cannot
  begin unattended work later.

The CLI takes its own lock for these files with a bounded wait, and neither
that lock nor the server refuses a stop because another request for the tab
is in flight. When both files hold a request, the extension applies them in
the order they were issued. It remembers the request ids it has applied, so a
repeated request has no effect. It reads both files from a managed timer once
a second, started only for the main agent of a TUI session, and once more in
the same step as every submission, which is what makes a pending stop reach
the next iteration in time.

The state report is the only source for what the tab is doing; an
acknowledgement means only that the extension took a request. The report
carries a format version and a counter, and the extension rewrites it on
every change and at least every 30 seconds. The CLI treats a report it has
not seen refreshed for 90 seconds as not reported. The report holds the
current autopilot with its prompt, timing, limits, progress, condition and
deadline, or the last ended one with its reason and end time and whether a
turn has run since; and OMP's own modes (decision 11). It stays within 64 KiB:
a prompt without control characters encodes to at most twice its 16,000
bytes, and text that OMP supplies, such as a goal's objective, is cut to 500
characters with a marker. The request files have the same cap.
`omp-activity.json` keeps its six fields, so an older CLI still reads it. The
types that read reports accept unknown fields, because a tab's extension can
be newer than the CLI reading it.

These files are not an authority boundary. They are private to the OS account,
and the checks show which launch a file belongs to, not who wrote it. Any
process of that account, the tab's own agent included, can write a start, a
heartbeat or a state report. That fits a mode that is not contained, and the
requirements and the start dialog say so instead of promising more.

No pane-host frame is added and `HOST_PROTOCOL_VERSION` is unchanged: a
running host treats an undecodable frame as a lost controller.

Alternative rejected: write `/goal` or a prompt into the PTY. That is a blind
write, the thing the conversation composer already has to apologise for.

Alternative rejected: copy the question bridge's exchange as it stands. It
removes its request when its wait ends and refuses a second submitter, which
is right for an answer and would drop or refuse a stop.

### 6. The autopilot is journaled in the OMP session, comes back paused, and ends when the tab leaves that conversation

The extension appends a custom entry (`apas-autopilot`) to the tab's OMP
session at start, at each submission, at each submission that was not
accepted, and at every pause, resume and end. Only the start entry holds the
prompt. Every entry names its autopilot and its session, and the extension
reads the active branch (`getBranch()`), ignoring entries of another session.

On `session_start` the extension reads the latest entry back. If it shows an
autopilot that had not ended, the extension reports it as paused with the
reason "the agent was restarted", the iterations it had left and the same
deadline, or as ended when the deadline has passed. It never resumes by
itself. If it shows an ended autopilot, that one is reported as the last
autopilot, which is where "start again" gets its values.

The journal travels with the conversation, so it survives Reboot agent,
project stop and start, and a host reboot, and it is not left behind in a
runtime directory that is never cleaned. If an entry cannot be written, the
autopilot is simply gone after a restart; that is the safe direction.

An autopilot belongs to the conversation it was started in. When the tab
leaves it, through `/new`, `/resume`, a fork, a branch or a move in the
session tree, autopilot ends with the reason "the conversation was changed in
the terminal". The extension writes the end entry where it will be read
back: in the conversation being left, from OMP's `session_before_*` events,
and at the new position when that still carries the autopilot's entries. If
an end could not be written before a switch, the conversation's last entry
still shows a running autopilot, and the worst outcome is that it comes back
paused when that conversation is opened again after a restart. OMP's reload
switches to the same session and changes nothing. Task 1.4 checks these
events, and that registering the `session_before_*` handlers has no side
effect of its own.

Alternative rejected: keep the autopilot in `.apas`. `PaneConfig` is replaced
wholesale by instance reports, is stripped by older CLIs, has about forty
literal construction sites, and still carries the retired bot's fields.

Alternative rejected: carry a paused autopilot into the new conversation.
Resume would then run the prompt in a conversation it was not started in, and
the token count and the journal would have to be rebuilt across files.

### 7. A heartbeat keeps autopilot from running without its controller

A small thread per project, separate from the three-second tab watcher,
writes `apas-controller.json` for each OMP tab every three seconds. It never
sends on the project's outbound queue. The watcher cannot do this job: during
an outage that queue fills from terminal output and the watcher blocks at its
next send, which would make every busy tab look abandoned after a minute.

The file holds a counter that advances on every write, whether the project is
connected to the server and for how long it has not been, and whether the
server can control autopilot. The durations are measured on the CLI's
monotonic clock. The extension notes, on its own monotonic clock, when it
last saw the counter advance, and treats the heartbeat as fresh for 60
seconds after that. No wall-clock times are compared, so a clock step cannot
keep a dead controller's heartbeat fresh.

The CLI needs real connection state for this. It sets a shared "connected"
state when a registration completes and clears it at the top of every
reconnect attempt, and it records when the last frame arrived from the
server. It reports "connected" only while a frame arrived within 90 seconds,
the bound the server already uses on its side. Without that, a silent network
drop reads as connected until the kernel gives up on the socket.

"The server can control autopilot" comes from a new optional
`negotiated_capabilities` on `ServerToCli::Registered`, as the daemon channel
already has: it is true while the last registration reply echoed
`omp_tab_autopilot_v1`. A server that is rolled back by itself keeps a newer
CLI connected but can relay neither state nor controls. Without the echo, the
tab would keep working where no client can see or stop it.

Before submitting, the extension holds when the heartbeat is not fresh, when
the server has been unreachable for more than 600 seconds, or when the server
cannot control autopilot. A hold is not a pause: iterations continue when its
cause clears. A hold that lasts more than one hour becomes a pause ("APAS was
unreachable for more than an hour"), so a machine that was cut off for a day
does not start work again by itself when contact returns. The deadline keeps
running throughout.

While a hold lasts because the controller or the server is gone, the path
that would carry the condition to clients is gone too. Clients show the state
as not reported, and the terminal's own status line shows the hold.

This is not the worker's lease. A turn in progress is not stopped, and the
bound is the tab's own machine clock. It does mean an autopilot cannot keep
starting work for a week while nothing can see or stop it. The pane host's
existing adoption grace (600 s, or 900 s around a reboot) still ends the
provider when the controller never returns. A tab on a direct PTY does not
survive a CLI reboot at all; its agent is restarted and its autopilot comes
back paused.

An older CLI writes no heartbeat, so an extension left running after a
rollback holds and, after an hour, pauses.

Alternative rejected: pause on every disconnect and require a person to
resume. A server deploy or a daemon reboot would then end every overnight
autopilot.

Alternative rejected: hold without a bound. It lets unattended work start
again by itself days after anyone last saw it.

### 8. One control variant, a receipt and a state frame, routed to the tab's machine

The wire follows the `AnswerOmpQuestion` path, which is the existing
structured command to an OMP tab:

- `WebToServer::ControlPaneAutopilot { session_id, pane_id, request_id, action }`,
  with `action` one of start (with its spec), pause, resume and stop (with
  `now`). A resume names its `autopilot_id` and is refused when that is not
  the tab's paused autopilot. A pause or stop may carry the id as a hint and
  applies to whatever autopilot is current, because a stop must not be
  refused for a stale view. `session_id` is required.
- `ServerToCli::ControlPaneAutopilot`, classified `Pane(pane_id)` in
  `scope()`, with its arm in `remote.rs` and its row in the pinned
  classification table.
- `CliToServer::PaneAutopilotReceipt { session_id, pane_id, request_id, outcome, reason }`,
  with `outcome` one of accepted, refused and uncertain, and
  `CliToServer::PaneAutopilotState { session_id, pane_id, support, state }`.
  Both carry the session and the pane, as the pane class of `inbound_scope`
  requires.
- `ServerToWeb::PaneAutopilotReceipt` to the requesting connection only, and
  `ServerToWeb::PaneAutopilotState` to viewers that negotiated the capability.
- An optional `autopilot` block on `MobilePaneSummary` (availability,
  condition, reason, next iteration, progress; no prompt), which older native
  builds ignore.
- The optional `negotiated_capabilities` on `ServerToCli::Registered`
  (decision 7).

Capabilities: `omp_tab_autopilot_v1` for the project instance and
`tab_autopilot_v1` for viewers. The server checks the owning instance's
capability before sending, because an older CLI drops an unknown frame
silently. It never sends the new viewer frames through bare `route_to_web`.

The CLI capability says the build speaks the protocol. Whether a given tab
can run autopilot is data. The CLI reports one `support` value per OMP tab:

- **supported**;
- **not reported**: the launch record has the autopilot marker but no fresh
  state report exists, as in the first seconds after a launch, or when the
  extension failed to load or stopped refreshing;
- **needs an agent reboot**: the launch record has no marker, so the agent
  was started by an APAS that had no autopilot;
- **unsupported OMP**: the extension found an OMP below the minimum, or a
  missing call;
- **unavailable on this platform**.

Viewers are not sent an instance's capabilities, so the server adds two
values of its own to what viewers receive: **instance update required**, when
the owning instance is attached without `omp_tab_autopilot_v1`, and **not
reported**, when nothing has arrived from the owning instance or it is
detached. The control can then be disabled with the right reason before any
request is sent.

The server handles a request in this order. It requires that the requester
negotiated `tab_autopilot_v1`. It claims the request id. It resolves the
session with `resolve_target_session`. It requires a terminal OMP tab that is
not legacy bot history. It finds the owning instance and checks access to its
machine. It checks the instance capability and refuses an older machine with
the update text. It applies decision 9 to a start or resume. It validates a
start's bounds. It forwards to the owner only, and answers the requester from
a spawned task, so a slow tab cannot block that connection's other frames. If
the CLI's receipt has not arrived after five seconds it answers "uncertain".
Requests are tracked by request id, not one per tab.

The server caches the last state per pane in memory and prunes it wherever
pane status is pruned. When the instance detaches it drops the cached state
and tells viewers the state is not reported; the provider may well still be
running under its pane host. The instance reports every OMP tab's state again
after each registration. The server keeps no record of which autopilots it
forwarded: an autopilot outlives a server restart in its tab's journal, and
any such record would be wrong after one.

Nothing reuses the retired bot's variants or fields, `PaneConfig`,
`PaneMode`, the status string or `MutationKind`.

Alternative rejected: carry the state in `PaneConfig` or encode it in the
status text. The first is persisted and merged wholesale; the second makes
every client read "on autopilot" as Working.

Alternative rejected: derive "needs an agent reboot" from a missing state
file. The file is also missing in the first seconds after every launch, so
the control would ask for a reboot right after one.

### 9. The hosting account starts; anyone who can type stops; no policy switch yet

Start and resume are accepted only from the account whose instance runs the
tab. Pause and stop need what typing into the tab needs: runtime access on
that machine. Stopping must never need more authority than starting something
does.

The reason is whose login and machine are spent. An autopilot runs for hours
on the hosting account's OMP login, with that account's files. On a shared
machine the hosting account is the machine's owner, also for a project a
cluster member created there, so such a member cannot start autopilot on
their own project. A collaborator with runtime access can already type
`/loop` into the terminal, and nothing here prevents that; this rule decides
only what APAS's own control, and a phone, will do on someone else's machine.
It governs requests that arrive through APAS, and decision 5 says why it is
not a boundary against the tab's own agent.

Alternative rejected: runtime access for everything, as the retired bot had.
It is consistent with typing, but it makes a shared tab a way to spend
another account's quota unattended from a phone.

Alternative rejected for now: a permission on `EffectiveProjectPolicy`,
narrowing over deployment, cluster and project, default denied. It is the
right shape if collaborators should be allowed to start autopilot, and it can
widen this rule later without changing anything else. It adds three editors
and a second enforcement point for a first version whose only users are tab
owners.

### 10. Autopilot is a second attribute in status, lists and the conversation; no push notification yet

Working, Pending answer and Idle keep their meaning and their accessible
names. The extension keeps publishing Working between iterations while the
next one is scheduled within ten seconds, and falls back to Idle when no run
has started 15 seconds after it was due. The autopilot state says why an idle
tab is not waiting for anyone.

Publishing Working is not enough to keep the status steady. The server clears
a terminal tab's status when a turn's last message arrives marked complete,
and the web does the same. So while the live OMP report says Working, the CLI
sends that message with `terminal_turn_complete: false` and skips its own
clear. No Idle frame then goes out between two immediate iterations.

- **Dots and tab:** an autopilot tab's dot keeps its colour and gains a ring;
  its label gains ", on autopilot". The desktop tab gains a marker with a
  spelled-out label. A paused autopilot keeps the marker in a paused style; an
  ended one has none. The Bot icon, the words bot and worker, and the noun
  "run" are not used.
- **Idle lists:** a tab waiting for its next iteration, or holding, goes
  under "On autopilot", as "Usage limited" tabs have their own group. A
  paused autopilot is listed with the idle tabs, with its reason, because it
  does need someone. An ended one is listed with the idle tabs with its
  reason until a turn next runs in the tab.
- **Conversation:** the marker line of decision 3, and a second status line
  with the condition.

No push notification is sent for autopilot in this version. Push reaches only
native-app installations, the native app shows nothing about autopilot, and
no native build is released, so the events would reach nobody who could act
on them. A person learns that an autopilot ended or paused from the tab, the
project list and the idle lists. When notifications are added, with a native
view, the review of this draft settled their shape: a question keyed in the
question path's own format, from the opaque question id the CLI already
composes, so that the two paths deduplicate in either order; a pause keyed by
the autopilot and a pause id that is kept in the journal; the end keyed by
the autopilot; no per-turn completion for an iteration's turn; and the
hosting account as a recipient, which the enqueue and claim checks do not
allow today.

Alternative rejected: a fourth pane status. Every client and the server would
have to learn it at once, and older native builds would drop their socket.

### 11. OMP's own modes are shown, not stopped

The extension reports goal and loop mode from `getState()`, `mode_changed`
and `goal_updated`: active or paused, the goal's objective (cut to 500
characters), its token budget, tokens used and its real status, including
budget-limited. While a mode is active and continuing, the extension keeps
publishing Working across OMP's 800 ms gap, falling back to Idle if no run
starts within 15 seconds. A paused mode is shown as paused and the tab may be
idle.

The indication says how to stop the mode in the terminal: Esc pauses either
mode, `/loop` switches a loop off, and `/goal drop` removes a goal. It is
shown only while OMP reports the mode, because `/loop` is a toggle and typing
it after the loop has ended would switch a new one on. While a loop is active
or paused it also says that a message sent to the tab becomes the prompt the
loop repeats, since the conversation composer types into the same terminal.

APAS does not stop these modes remotely. Both stop sequences are possible
from an extension, but each has the problem decision 1 describes: the loop's
is an abort followed by a race against OMP's 800 ms re-submit, and the goal's
has to drop the goal record without the confirmation OMP's own command asks
for. A phone can already stop either mode with the Esc key of its raw
terminal. What was missing is that nothing showed the mode, and this decision
supplies that.

Autopilot refuses to start or resume while goal, loop or plan mode is in any
state other than off, and pauses if one is switched on, because OMP's loop
suppresses its own goal continuation and two drivers on one tab cannot both
be right. The same applies to autoresearch, a third self-continuing mode that
`ctx.sessionModes` does not report. The extension detects it from its
experiment tool being active (`pi.getActiveTools()`), checks before each
submission, and refuses or pauses with "Type /autoresearch off in the
terminal". Autoresearch is not otherwise shown.

Alternative rejected: show nothing for modes APAS did not start. Then the
existing behaviour stays invisible: a tab in goal mode looks finished after
every turn.

Alternative rejected for now: a remote stop for these modes. It is listed for
the owner below.

### 12. Where the controls live

- **Desktop:** a control in the pane toolbar before the Terminal/Conversation
  switch (the pane-toolbar delta). It opens the start dialog or, when on, a
  small menu with Pause or Resume, Stop, Stop now and Details (prompt,
  timing, limits, progress, time limit end). For an ended autopilot it shows
  the reason and opens the start dialog with that autopilot's values. For a
  viewer who is not the hosting account, starting is disabled with the
  reason. The start dialog carries the statement the spec requires. The
  confirmations for closing a tab, rebooting its agent and closing the
  project say when a tab on autopilot is affected. The Overview's pane cards
  show the condition beside the status.
- **Interrupt:** the toolbar's Interrupt button is shown today on every
  Working tab, and the CLI ignores it for a terminal tab. A tab on autopilot
  is Working most of the time, so the button a person reaches for first would
  do nothing, beside a Stop now that works. The delta removes it from
  terminal tabs.
- **Mobile browser:** "Start autopilot" in the tab's "Pane actions" sheet; a
  state notice with the condition, Pause or Resume, Stop (which asks whether
  to let the turn finish or interrupt it) and Details, shown while the tab is
  on autopilot or paused; the ended reason with "Start again"; the indication
  of OMP's own modes; a glyph on the tab's chip, which has no room for
  another word; and the same confirmations as the desktop. The notice is a
  state notice, like the existing notice for an unconfirmed send, not one of
  the occasional pane actions.
- **Native app:** nothing. It advertises no capability, receives no new frame
  and ignores the optional summary block. A tab on autopilot looks there like
  any other tab.
- **"+" menu:** unchanged. Its first choice stays Interactive agent or
  Autonomous worker.

Alternative rejected: add "start on autopilot" to the "+" menu. One click
there creates a tab with no prompt field, a launch can end unconfirmed, and it
would sit beside the worker choice it must not be confused with.

### 13. Tier boundaries, and reconciliation with unarchived changes

Autopilot shares nothing with the worker: no job, policy, ledger, verifier or
publisher, and none of its wire types. It satisfies no worker gate. It also
shares nothing with the retired bot: no variant, no `.apas` field, no restart
on restore, no conversion.

`unify-autonomous-workers` is committed but not archived, so its requirements
are not in main specs and this change writes no MODIFIED delta against them.
One of its titles ("One unattended product replaces legacy creation paths")
and one runbook sentence ("recurring work belongs to autonomous workers") stop
being true as written. Its binding sentences still hold: Start bot stays
removed, no old path starts a loop, and no terminal is adopted as a worker.
The reconciliation section below says what is proposed to its owner.

That change's design rejected OMP's `/loop` "as the only scheduler" because
it "cannot atomically fence a restart, human edit, revoked grant and budget
reservation". Autopilot does not claim to. It answers the restart case by
coming back paused, has no grant or reservation to fence, and says in its
start dialog what it does not do. Revoking the hosting account's access to a
project, or suspending the project, already ends the tab's provider through
the existing session rejection, and autopilot with it.

Alternative rejected: fold autopilot into the worker product as an
"uncontained profile". Worker readiness must fail closed without a terminal
fallback, and one name for two trust models is how a person ends up believing
a tab was sandboxed.

## Risks / Trade-offs

- **OMP changes under it.** Hosts update OMP in the background, and the
  extension has only ever been checked by hand. → The extension checks OMP's
  version and every call it uses, and reports "unsupported" otherwise; its
  JavaScript tests join a real gate; one hermetic check against the installed
  OMP is part of verification and is repeated when OMP's minor version
  changes.
- **Limits are soft, and kept in the agent's own process.** A single turn can
  run for hours and spend past the token limit, and the agent could rewrite
  its own limits. → Stated in the dialog; the time limit always ends
  autopilot; Stop now interrupts.
- **One iteration cannot be recalled.** A prompt OMP has accepted can start
  after Pause or Stop. → It counts as the turn in progress; after Stop now it
  is aborted when it starts.
- **A question blocks an unattended tab, and nobody is pushed a
  notification.** → Pending answer keeps precedence, so the tab shows red in
  every list. But since OMP 18.18.3 `ask` sends an HTML form that the
  question bridge does not recognise, so the question's content and remote
  answering are probably unavailable on current OMP and the person must
  answer in the terminal. That repair is a separate change and should land
  first. If the hosting account's OMP settings give `ask` a timeout,
  questions answer themselves while nobody is there.
- **The hosting account's quota.** An autopilot can exhaust the login its
  owner's other tabs use, and a usage limit then pauses it within minutes. →
  Three failures pause it instead of hammering the provider; the dialog says
  which machine's OMP login is spent and points to OMP's own setting for
  waiting out a reset.
- **Unreviewed changes.** The tab can commit and push with the account's
  credentials, and PR hand-off is a non-draft `gh pr create`. → Same as any
  tab; the dialog says so and says whether the tab uses the shared checkout.
- **The authority rule is not containment.** Anyone with terminal access can
  type a loop, and any process of the account can write the request files. →
  Said in the spec, the dialog and the runbook.
- **Background work after Stop now.** Jobs a turn launched keep running and
  can start another turn. → Said where Stop now is offered.
- **"No tool call" is a heuristic.** It can end an autopilot early or never.
  → It is optional, labelled as what it is, and the limits remain.
- **A bigger extension.** A fault in it would also take down activity
  reporting and the question bridge. → Autopilot code is isolated behind its
  own guard and uses only managed timers; a broken optional import cannot
  stop the extension loading.
- **False holds.** A stalled heartbeat writer stops iterations, and after an
  hour pauses them. The pane host and the CLI each work out the runtime
  directory from their own environment; if they ever differed, the heartbeat
  would land where the extension does not look. → Both fail in the safe
  direction, and the terminal's status line shows the hold.
- **Two unattended tiers.** People may take autopilot for the safe one. →
  Different name, no worker icon or vocabulary (decision 10), and the
  statement in the start dialog.
- **Prompt text.** The prompt is handled like a typed message: OMP's keywords,
  `@path` mentions and a standalone `+500k`-style budget token all apply, and
  slash commands and skills are not run. → Said in the dialog and the
  runbook; the prompt never appears in a list or a summary.
- **One conversation grows.** Every iteration is appended to one OMP session,
  and the CLI re-reads that file whenever it changes. → The caps bound it;
  compacting or starting a fresh session between iterations is a later
  option.
- **Billing attribution.** OMP documents the attribution of a message as a
  billing field: on a GitHub Copilot login an agent-initiated request is
  counted differently from a typed one, and OMP's own loop submits as typed.
  This was read, not checked, and matters only for such a login. → Recorded
  for the owner; task 1.2 notes what the session records.

## Migration / Rollout

1. Shared contract and generated schemas, server, web, then the CLI and
   daemon. With a new server and an older CLI no autopilot can be started,
   and the control says the machine needs an update. A new web on an older
   server negotiates no capability and shows nothing.
2. A machine gains autopilot when its daemon is rebooted to update, which
   needs its owner's go-ahead as always. A tab gains it at its next agent
   launch, which ends that tab's turn in progress. Nothing reboots an agent
   to load the extension.
3. No stored data changes. `.apas`, `panes.json` and SQLite are untouched;
   the journal entries are ignored by an older extension.
4. Rollback: stop running autopilots, then roll back CLI, web and server. An
   autopilot left running holds and then pauses, whichever part goes first:
   an older CLI writes no heartbeat, and an older server does not echo the
   capability.
5. This change depends on the worker cutover and on multi-machine tabs, both
   undeployed. It deploys nothing by itself, and production still runs the
   pre-cutover bot.

### Predecessor specification reconciliation

At apply time, and only with the agreement of whoever owns
`unify-autonomous-workers`:

- Add an "Applicability after `add-omp-tab-autopilot`" note to that change's
  proposal, saying that its "one unattended product" position covers
  contained, approved, recurring work, and that an interactive OMP tab may
  additionally be left on autopilot under this change's requirements.
- Retitle its requirement "One unattended product replaces legacy creation
  paths", for example to "The worker flow replaces legacy creation paths".
  That change's own reconciliation with `add-autonomous-jobs` did both
  things: it added an applicability section and edited the predecessor's
  delta specs in place. A note in a proposal is not carried into main specs
  at archive, and the title would otherwise arrive there beside a second
  unattended mode. If that change is archived first, the retitle becomes a
  RENAMED delta on `autonomous-worker-surfaces` in this change.

Neither change is archived on the strength of the other.

Two more unarchived changes are touched without a conflict.
`mobile-session-chrome-and-project-management` keeps occasional pane actions
away from the composer; the autopilot notice is a state notice, and starting
autopilot is placed among the occasional actions. `share-cluster-with-users`
lets members operate projects on shared machines; decision 9 keeps one
operation, starting autopilot, with the machine's owner.

No main spec defines the OMP terminal provider yet; `provider-support` lists
five terminal profiles without it. That drift predates this change, which
states its own preconditions and modifies nothing there.

## Decisions for the owner

The draft takes a position on each of these. Each can be changed without
reworking the rest.

1. **An end date is required.** At most 1,000 iterations and seven days, with
   "start again" prefilled. The alternative is no end date for the hosting
   account, with finite values still proposed.
2. **Only the hosting account starts.** The alternatives are anyone who may
   type, as the bot allowed, or members where a policy allows it.
3. **The time limit lets a turn in progress finish.** The alternative is to
   interrupt it after a grace period.
4. **A hold longer than an hour becomes a pause.** The alternative is to hold
   until contact returns.
5. **No push notification in this version.** The alternative is to add the
   events of decision 10 together with a read-only line in the native app.
6. **OMP's own modes are shown, not stopped.** The alternative is a
   best-effort remote stop, which would first need a check of how reliably an
   extension wins the race to exit OMP's loop.
7. **A token limit is offered.** It is approximate, and it is the easiest
   part to leave out.
8. **The question bridge is repaired first, as a separate change.** The
   alternative is to ship with the sentence in the start dialog.
9. **Group 1 runs against a stub model or a short real-model run.** Task 1.1
   asks before building either.

## Open Questions

- Whether to add an in-terminal `/autopilot` command for status, pause and
  stop. It would never start or resume, which the server could not authorize.
- Whether the owner of `unify-autonomous-workers` agrees to the retitle.
- Whether any host uses a GitHub Copilot login for OMP, which decides whether
  the billing attribution needs a decision.
- The marker glyph and the exact wording of the start dialog's statement.
