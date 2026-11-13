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
  `awaitingAsyncWork` when it is not a terminal settle. `turn_end` carries
  the assistant message of one model response, with its stop reason, and
  `message_start` reports each message as OMP records it, a prompt's user
  message included. A turn interrupted with Esc ends `aborted` with the
  `errorMessage` "Interrupted by user", and so does a turn aborted by an
  extension's own `ctx.abort()`.
- `ctx.isIdle()` (true whenever nothing is streaming, a manual compaction
  included), `ctx.hasPendingMessages()`, `ctx.abort()` (stops the run, a
  retry wait, a compaction or handoff in progress and a shell command typed
  in the terminal, and drops a prompt that is still in setup; it cancels
  neither background jobs nor queued input), `ctx.getAsyncJobSnapshot()`,
  `auto_compaction_start`/`_end` (the end says whether it was aborted) and
  `auto_retry_start`/`_end` (a cancelled retry ends with `success: false`
  and the final error "Retry cancelled").
- Session events: `session_start` once per process, and
  `session_before_switch`/`session_switch` (new, resume, fork; a reload is a
  resume of the current file), `session_before_branch`/`session_branch`
  (branch, fork, or a promoted `/btw` answer) and
  `session_before_tree`/`session_tree` when the tab's conversation changes.
- Managed timers (`ctx.setTimeout`, `ctx.setInterval`), `pi.appendEntry()` for
  journal entries, `ctx.sessionManager.getBranch()`, `ctx.ui.setStatus()`,
  `getEditorText()` and `setEditorText()`, and the SDK's exported `VERSION`.

## Goals / Non-Goals

**Goals:**

- A stop that needs no mode switch and is not lost while the tab's agent
  keeps running. Pause and Stop cancel APAS's own timer, Stop now also
  interrupts the turn, and a stop request stays in force until the tab
  applies it or its agent is restarted, after which autopilot is paused
  anyway. One iteration already handed to OMP cannot be recalled; decision 3
  says what happens to it.
- Limits that a person may set, that client code enforces between turns and
  that can be read back at any time. A time limit, when one is set, always
  ends autopilot. No limit is the default.
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
- A token or spending limit. The draft offered an approximate token limit,
  and the owner left it out.
- Repairing the question bridge for OMP 18.18.3 and later. The owner left it
  out, and nothing here checks it or waits for it (see Risks).
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
yet started, so a submission can start later than it was sent, and a
`/compact` typed during a turn hides that turn's end from it (decision 3).
Esc does not know about APAS's loop, so pausing on an interrupt has to be
built from the signals an extension can observe (decision 4). A dropped
prompt lands in the terminal's input line and has to be cleared
(decision 3).

The design needs at least OMP 18.18.2: before it, `sendUserMessage` resolved
when a prompt was dropped, so a dropped iteration would leave autopilot
waiting for a turn that never comes. A check that each function exists cannot
see that change, so the extension reads the SDK's `VERSION` as well as
checking for every call it uses, `ctx.sessionModes` included. Task 1.6 finds
the real minimum, which may be higher: this tag's changelog does not say
when `awaitingAsyncWork`, the job snapshot's delivery fields and aside
delivery for `sendUserMessage` arrived.

Alternative rejected: drive `ctx.sessionModes` for goal mode and use the
extension only for timed repeats. It keeps the racy stop and the invisible
states for half the feature and doubles what must be tested against every OMP
update.

### 2. One autopilot is a prompt, a timing and optional limits

A start carries:

| Field | Meaning | Bounds |
|---|---|---|
| prompt | Submitted as typed text each iteration | 1 to 16,000 bytes (the size of a first instruction today); no control characters other than newline and tab |
| repeat | When the next iteration is due | "as soon as each turn ends" (five seconds after the tab settles), or every 60 s to 24 h, measured from the previous iteration's submission |
| iteration limit | Optional. Most iterations it may submit; the first counts | none, or 1 to 1,000,000 |
| time limit | Optional. Autopilot ends at start + this; paused, waiting and holding time counts | none, or 60 s to 365 days and not shorter than the repeat interval |
| end without tool call | End after an iteration whose turn calls no tool | proposed on for "as soon as each turn ends", off otherwise |

A timed repeat is measured from submission to submission, as the retired
bot's interval was. A person's message then does not move the schedule, and
"every 15 minutes" means what it says. When a turn outlasts the interval the
next iteration follows five seconds after the tab settles.

**No limit is the default.** The dialog proposes no iteration limit and no
time limit, and a person sets either or both when they want one. This is the
owner's decision; the draft required both, capped at 1,000 iterations and
seven days. The upper bounds in the table are not policy: they refuse a
mistyped number, and someone who wants more chooses no limit. There is no
token limit. The draft offered an approximate one, and the owner left it out.

The dialog proposes nothing for the prompt except that of the last autopilot
the tab reports (decisions 5 and 6), with that autopilot's timing and limits,
so that an autopilot that ended overnight can be started again without
retyping.

What pauses or ends an autopilot that has no limit:

- a person pauses or stops it, or interrupts the tab in the terminal
  (decision 4);
- three iterations in a row fail, which is what a usage limit causes
  (decision 4);
- an iteration makes no tool call, when that option is on;
- APAS could not control the tab for four hours (decision 7);
- the tab's agent exits or restarts, which a project stop also causes, and
  autopilot comes back paused (decisions 6 and 8);
- the tab leaves its conversation (decision 6), or one of OMP's own modes is
  switched on (decision 11);
- the tab is closed.

Nothing else does. The retired bot also ran without an end. What differs here
is that an autopilot never starts again by itself after a restart, pauses on
failures and interrupts, holds without its controller, and is shown and can
be stopped on every surface.

Each limit travels as an explicit value: a number, or "none". A start in
which a limit is missing altogether is refused at each of the three checks
below, so a field that one version drops can never turn a limited autopilot
into an unlimited one.

The bounds are checked three times. The server checks them so that a refusal
carries a reason: on this wire a frame that fails to parse is dropped without
an answer, so the viewer's request declares the limits as optional fields and
the server validates them. The server forwards a strict form (each limit
required, as a number or "none", and unknown fields rejected, as `JobLimits`
and `WorkerLimits` are), which the CLI re-checks. The extension checks them
once more, because it enforces them and may be older than the CLI that feeds
it.

A second start is refused; changing an autopilot is stop and start. The
extension creates an `autopilot_id` (UUID) when it accepts a start; no
request carries one except resume, pause and stop. Control characters are
U+0000 to U+001F except newline (U+000A) and tab (U+0009), U+007F, and U+0080
to U+009F, so a carriage return is refused. Conditions: `starting`, `running`,
`waiting` (for the next iteration, for the tab to settle, or for background
work), `holding` (for the controller or for the server), `paused` and
`ended`, the last two with a reason.

Alternative rejected: require an end date, as the draft did, with at most
1,000 iterations and seven days. It guarantees that every autopilot ends
without anyone acting, and it forces a look at least once a week. The owner
decided against it: a limit is the choice of whoever starts the autopilot.
The cost is in Risks: an autopilot that nobody stops keeps spending its
machine's login, and its one conversation keeps growing.

### 3. An iteration is submitted only from a settled tab, as an agent-attributed message

The extension already tracks whether the tab is settled, for activity
reporting, and that tracking handles retries and automatic compaction.
Autopilot builds on it. The tab is settled for autopilot when all of these
hold:

- The last agent run ended terminally (`agent_end` with neither
  `willContinue` nor `awaitingAsyncWork`). Or it ended waiting for background
  work and that wait is over: `ctx.getAsyncJobSnapshot()` shows nothing
  running, nothing queued for delivery and nothing being delivered. OMP emits
  no extension event when such a wait ends without a wake, so the extension
  polls the snapshot from its timer while in that state and reports "waiting
  for background work". The poll reads the wait as over once the jobs have
  ended or been killed and nothing is left to deliver. It cannot see Esc on
  OMP's wait row: OMP then stops waking on those jobs, but they keep running
  and stay listed, so autopilot stays "waiting for background work" until
  they end. When the poll reads the wait as over and no run follows, the
  wait ended without a wake, and autopilot pauses instead of submitting
  (decision 4). This reads OMP's own job state; it is not a guess from
  elapsed time.
- `ctx.isIdle()`, no pending messages, and no open `ask`.
- No automatic compaction or retry is in flight.
- No submission of its own is outstanding (below).
- None of OMP's own self-continuing modes is on (decision 11).

After a terminal `agent_end` the job snapshot is not consulted. For a run
that stopped normally or on an error, OMP has checked its own jobs for a
pending wake before reporting the end as terminal, so a job still running
then will not wake the tab. A run that was aborted, or that ended on a
message with tool calls, is reported terminal without that check. A job's
result can then still arrive later; when it starts a turn, autopilot treats
that turn like any other. A job with a pending wake that never ends, such as
a dev server the agent started in the background, makes every later
`agent_end` the waiting kind, and autopilot then waits for it until a person
stops it or a time limit, if one was set, ends it.

A `/compact` typed during a turn aborts that turn after OMP has detached its
event stream, so no end of that turn reaches the extension. OMP resumes the
turn when the compaction commits or is a no-op and `compaction.autoContinue`
is on. When it is cancelled or fails nothing follows, and autopilot stays
"running an iteration" until a later run ends, a person pauses or stops it,
or a time limit, if one was set, ends it. This is read from the source, and
task 1.3 observes it.

When the tab is settled and an iteration is due, the extension does the rest
in one synchronous step from a managed timer. It reads its request files and
the heartbeat once more (decisions 5 and 7), checks the limits and OMP's
modes, journals the iteration (decision 6), marks a submission outstanding,
and calls
`pi.sendUserMessage(prompt, { attribution: "agent", deliverAs: "aside" })`.
It never awaits that call inside an event handler, which has a 30-second
budget.

`deliverAs: "aside"` starts a turn on an idle session. If a run began in the
gap between the idle check and the call, OMP takes the prompt at that run's
next step boundary. The default delivery would queue it as a steer. A queued
steer skips interruptible tools that have not started and, more important,
restarts the run immediately after an abort, so Stop now could not hold.

A submission is outstanding from the call until the extension sees OMP
record its own user message (`message_start`, role user, attribution agent,
not earlier than the call), or the call rejects, or the session is switched
or reloaded, which discards a queued prompt without rejecting the call.
Task 1.2 settles how its own message is told from OMP's other
agent-attributed user messages.

An iteration that OMP has accepted cannot be recalled. `ctx.isIdle()` is
also true during a manual `/compact` or a handoff, which an extension cannot
watch without side effects (a `session_before_compact` handler switches off
OMP's background compaction for the whole session), and a prompt sent then
waits behind it. So while a submission is outstanding the extension submits
nothing else, and Pause and Stop treat it as the turn in progress. After
Stop now, if the outstanding prompt starts a run, the extension aborts that
run as well and reports "Stopped" when it has ended. OMP can also record the
prompt without a run, when the run that was to take it is interrupted first;
the prompt then stays in the conversation and the next turn answers it. The
extension then reports "Stopped" once the message is recorded and
`ctx.isIdle()` is true, and does not wait for an `agent_end`. This behaviour
is inferred from the source and is the first thing task 1.2 checks.

A call that rejects while its submission is still outstanding is journaled
as "not submitted" and uses up no iteration. OMP hands a dropped prompt back
to the terminal's input line when that line is empty, before it rejects. The
conversation composer types blind into the same line, so the extension
clears the line when it holds the prompt as OMP's editor stores it, which is
with each tab replaced by three spaces (`getEditorText()`,
`setEditorText("")`).

The rejection does not say who aborted. OMP's `PromptDroppedError` carries a
refusal reason only when its cost cap refused the prompt. Every abort gives
the same default text, whether it came from Esc, from the extension's own
`ctx.abort()`, or from the internal abort of a `/compact`, a fork or a move
in the session tree. So a drop without a refusal reason, which the
extension's own abort did not cause, is treated as a person's interrupt and
pauses autopilot (decision 4). A drop that carries a refusal reason, or a
rejection with any other error, is a failed iteration. Task 1.2 checks what
a rejection carries. A rejection that arrives after the extension saw its
own message recorded journals nothing and changes no count: that iteration
is classified once, by how its turn ended (decision 4), and as failed when
no `turn_end` was seen.

A turn the person starts is not autopilot's. The extension sees its
`agent_end` like any other and submits the next iteration when it is due and
the tab has settled again.

**How an iteration appears in the conversation.** The marker needs a stored
form, and none of the existing ones is neutral: a user input is stored as the
person's message and moves their counters, and assistant text is stored as the
agent's own words. The marker is therefore carried as an assistant tool-use
block with the reserved name `ApasAutopilotIteration` and the input
`{ autopilot_id, iteration, limit }`, sent with
`terminal_turn_complete: false`; `limit` is left out when the autopilot has
no iteration limit. The first marker of each autopilot, and the first after
each resume, also carries the prompt, which the reader takes from the
journal's start entry. The stored conversation then holds the instruction
once, and not a copy per iteration. Everyone who can open the session can
read the prompt already. The server already stores tool-use blocks and acts
on one name only (`AskUserQuestion`), so status, pending decisions and
notifications need no change. The web renders this name as the line
"Autopilot · iteration 3 of 20", or "Autopilot · iteration 3" without an
iteration limit, with the prompt behind the first one, and leaves the name
out of `isToolLikeMessage`, as it does `AskUserQuestion`, so consecutive
markers are not folded into a tool-call group. The pane work summary lists
the block as a tool call named `ApasAutopilotIteration`; that is accepted. A
client that does not know the name, an older web build or the native app,
shows a generic tool entry. Nothing is published as `UserInput`, so the
server's last-user-input time and prompt counts do not move.

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
  submission that OMP rejects while it is still outstanding does not use one
  up (decision 3). With an iteration limit, autopilot ends when the turn of
  the last iteration ends. Without one, the counter only reports progress.
- **Time:** with a time limit, an absolute deadline fixed at start, with its
  own managed timer. When it fires, autopilot ends at once, whatever it is
  doing. A turn in progress is not interrupted and finishes as an ordinary
  turn of the tab. Autopilot also ends at once when the next iteration would
  be due after the deadline. The deadline is checked again on resume. A timer
  does not outlive the process, so at `session_start` the extension checks
  the deadline of a restored autopilot and arms the timer again. A JavaScript
  timer cannot be relied on further ahead than about 24 days, so the
  extension arms this one for at most a day at a time and arms it again
  until the deadline. A turn or a tab that never settles therefore cannot
  keep a tab "on autopilot" past its time limit. Without a time limit there
  is no deadline, and such an autopilot runs until one of the cases in
  decision 2 pauses or ends it.
- **No tool call:** the extension counts `tool_execution_start` events during
  the iteration's turn. The rule applies only to an iteration whose turn
  completed normally; the failure and interrupt rules take precedence.
- **Failures:** an iteration fails when its submission is dropped with a
  refusal reason or rejected with another error (decision 3), or when its
  turn ended in an error: the last `turn_end` before the terminal `agent_end`
  carries a message whose stop reason is `error`, or `aborted` with an
  `errorMessage` other than "Interrupted by user". OMP emits one `turn_end`
  per model response, so only the last one counts. A turn whose retry was
  cancelled falls under the interrupt rule below. After a failure the next
  iteration waits at least 60 s. Three in a row pause autopilot, and a
  resume starts the count again. No backoff
  is added: for a usage limit OMP has its own opt-in,
  `retry.waitForUsageReset`, which makes the turn itself wait for a reset
  time the provider states and stay abortable. The runbook points to it and
  to its conditions (task 8.5).
- **Interrupts:** a person's interrupt while the tab is on autopilot pauses
  autopilot. An extension cannot see who aborted, so the rule is written
  over the signals it can observe. The extension's own `ctx.abort()` gives
  the same marker, cancelled retry and dropped prompt as a person's
  interrupt, so the extension flags its own aborts and Stop now is not taken
  for one:
  1. A turn ends `aborted` with the user-interrupt marker, and the extension
     did not abort it: "Paused: the turn was interrupted". This covers any
     turn, not only an iteration. Esc during a running turn, the "."
     continue shortcut during one, interrupt-and-send and a cancelled `ask`
     all end a turn that way.
  2. A wait for background work ends without a wake (decision 3): "Paused:
     the wait for background work was ended in the terminal". The extension
     pauses five seconds after the poll first reads the wait as over, unless
     a run has started in between. Five seconds is the delay after a settle,
     and it covers a result that OMP has taken from its job list and not yet
     injected. This is what follows Esc on the wait row, once the jobs it
     was waiting for have ended. A killed job looks the same to an
     extension, so killing the job a turn was waiting for pauses autopilot
     too.
  3. OMP reports a cancelled retry (`auto_retry_end` with `success: false`
     and the final error "Retry cancelled") that the extension did not
     cause: "Paused: the turn was interrupted". Esc during a retry wait, the
     long wait of `retry.waitForUsageReset` included, only cancels the
     retry. The turn's last message keeps the stop reason `error`, but this
     is not a failed iteration.
  4. OMP reports a cancelled automatic compaction (`auto_compaction_end`
     with `aborted: true`), an automatic handoff included, that the
     extension did not cause: the same pause. That OMP sends this for Esc is
     read from the source, and task 1.3 observes it. A compaction or a
     handoff that a person started in the terminal emits no extension event,
     so Esc on one cannot be seen and does not pause autopilot.
  5. A submission is dropped with no refusal reason, and the extension did
     not abort (decision 3): the same pause. Such a drop produces no
     `agent_end`.

More than one of these rules can apply at the end of the same turn. A rule
that ends autopilot then wins over one that pauses it, because a pause with
nothing left to resume helps nobody. The order is: a stop that was requested;
the time limit; the iteration limit; then the pause for an interrupt; then
the pause after three failures. The end for an iteration without a tool call
applies only when no other rule does. So a last iteration that is
interrupted or fails ends autopilot at its iteration limit.

All of these are soft in one sense: none stops a turn already running. Stop
now does, with `ctx.abort()`. What that call does about background work
depends on the tab's state. This is read from the source, and task 1.3
checks it.

- When Stop now interrupts a running turn, the jobs that turn launched keep
  running. OMP then records their results in the conversation without
  starting a turn, until the next deliberate prompt.
- When the tab is only waiting for such a job, the call does not end the
  wait, and the job's result can still start a turn after Stop now.
  Autopilot has ended by then, so that turn is an ordinary turn of the tab.

The same call also cancels a retry wait, a compaction or handoff in progress
and a shell command a person started in the terminal, and it drops a prompt
that is still in setup. Where the clients offer Stop now they say that
background work already launched keeps running, and that a tab that was only
waiting for such work can still start a turn when it finishes.

Alternative rejected: OMP's `--max-time` and `session.maxCost`. `--max-time`
leaves an open tab that silently does no model work, and a cost cap's refusal
emits no `agent_end`. Cost is also zero for subscription logins.

Alternative rejected: interrupt the turn in progress when the time limit
passes. It would make the time limit hard, at the price of cutting a turn
off in the middle of an edit. The owner confirmed that the turn finishes.

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
  now); a weaker one is not written, and the CLI answers it with the pending
  request's result. The request is bound to the launch only, not to the OMP
  session, and the CLI builds it from the launch record without needing the
  state report, so stopping works when the state is unreadable or stale. Only
  the CLI writes, replaces or removes this file, always under its lock. The
  extension never deletes it: it records each request id it has applied in
  the acknowledgement file, a request whose id is acknowledged is no longer
  pending, and the CLI removes it the next time it holds the lock. A pause or
  stop that passes the trust checks is applied whatever its format version
  and unknown fields; only start and resume are strict.
- A **start or resume** is never left pending. The extension takes one by
  renaming the request file to a name of its own before it applies it, and
  refuses one issued more than two seconds ago, which is the CLI's wait. When
  that wait ends the CLI removes the file. If the removal succeeds, the tab
  never took the request, and the CLI answers refused with "The tab did not
  take the request in time. Nothing was started." If the file is gone, the
  extension has taken it; the CLI waits one more second for the result and
  answers that, or "not confirmed" when none arrives. Only in that last case
  may a start have been applied without its sender knowing, and the tab's
  reported state then shows it.

Every request carries an issue time, which the CLI stamps from the host's
clock when it writes the file, after it has taken its lock; the extension
orders and ages requests by it. The CLI waits at most one second for its
lock, so that lock wait, two seconds and one more stay inside the server's
five. Neither that lock nor the server refuses a stop because another request
for the tab is in flight. A pending pause or stop does not survive a provider
launch: `install()` clears these files, the launch binding would reject the
request anyway, and the autopilot comes back paused. A request that changes
nothing, a pause while paused or a stop when the tab has no autopilot, is
acknowledged as accepted. When both files hold a request, the extension
applies them in the order they were issued. It remembers the request ids it
has applied, so a repeated request has no effect. It reads both files, and
the heartbeat of decision 7, from a managed timer once a second, started
only for the main agent of a TUI session, and once more in the same step as
every submission, which is what makes a pending stop reach the next
iteration in time.

The state report is the only source for what the tab is doing, with one
exception: after the tab's agent process has exited by itself, the CLI
reports a pause of its own (decision 8). An acknowledgement means only that
the extension took a request. The report carries a format version and a
counter, and the extension rewrites it on every change and at least every 30
seconds. The CLI treats a report it has not seen refreshed for 90 seconds as
not reported. The report holds the current autopilot with its prompt,
timing, limits, start time, progress, condition and, when it has a time
limit, its deadline; or the last ended one with its prompt, timing and
limits, its reason and end time, and whether a turn has run since; and OMP's
own modes (decision 11). It stays within 64 KiB: a prompt without control
characters encodes to at most twice its 16,000 bytes, and text that OMP
supplies, such as a goal's objective, is cut to 500 characters with a
marker. The request files have the same cap. `omp-activity.json` keeps its
six fields, so an older CLI still reads it. The types that read reports
accept unknown fields, because a tab's extension can be newer than the CLI
reading it.

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
reads the active branch (`getBranch()`), ignoring entries of another
session.

The extension reads the latest entry of the current session back whenever
the tab arrives in a conversation: at `session_start`, which fires once per
process, and after every `session_switch`, `session_branch` and
`session_tree`, through which a person returns with `/resume` or moves by a
branch or in the session tree. The shipped extension already handles these
four events with one handler. What follows depends on how the tab arrived:

- At `session_start`, an autopilot that the entry shows as not ended is
  reported as paused with the reason "the agent was restarted", its
  iteration count and, where it has them, the same limits and deadline; or
  as ended when it has a deadline that has passed. It never resumes by
  itself. When the agent process had exited by itself, this report replaces
  the pause the CLI showed meanwhile (decision 8).
- After one of the other three events, a reload excepted (below), an
  autopilot that the new position still shows as not ended is ended there,
  with "the conversation was changed in the terminal".
- An entry that shows an ended autopilot gives the last autopilot, which is
  where "start again" gets its values. The exception is an autopilot that
  ended in this process because of the change: that one stays the reported
  last autopilot.

The journal travels with the conversation, so it survives Reboot agent,
project stop and start, and a host reboot, and it is not left behind in a
runtime directory that is never cleaned. If an entry cannot be written, the
autopilot is simply gone after a restart; that is the safe direction.

An autopilot belongs to the conversation it was started in. When the tab
leaves it, through `/new`, `/resume`, a fork, a branch or a move in the
session tree, autopilot ends with the reason "the conversation was changed in
the terminal". The extension writes the end entry where it will be read
back: in the conversation being left, from `session_before_switch` (not when
its target is the current session file, which is a reload), from
`session_before_tree`, and from `session_before_branch` unless its reason is
`btw`; and at the new position when that still carries the autopilot's
entries. OMP's `/btw` branch compares the session leaf before and after the
`session_before_branch` handlers and refuses when a handler appended an
entry, so for `btw` the extension appends nothing in the before event and
ends autopilot at `session_branch`. A before event comes ahead of other
handlers' cancels and of OMP's own refusals; an autopilot whose end was
journaled there stays ended when the change does not happen. If an end could
not be written before a switch, the conversation's last entry still shows a
running autopilot. It is ended when the tab next arrives there inside the
same process, and the worst outcome is that it comes back paused when that
conversation is opened again after a restart.

After a change of conversation the state report keeps the ended autopilot,
with its prompt, timing and limits, until another autopilot starts or the
agent process restarts. After `/new` the tab therefore shows "Ended: the
conversation was changed in the terminal" and offers that autopilot's values
for a new start. After a later agent restart the new conversation's journal
holds none of its entries, so the tab reports no autopilot, and neither the
reason nor the values are shown.

OMP's reload is a switch to the same session file. The extension recognises
it by the session file, which does not change, and keeps the autopilot. It
resets its settled state and its outstanding-submission marker, because a
reload aborts a running turn without an event reaching the extension and
discards queued input. Task 1.4 checks these events, and that registering
the `session_before_*` handlers has no side effect of its own.

Alternative rejected: keep the autopilot in `.apas`. `PaneConfig` is replaced
wholesale by instance reports, is stripped by older CLIs, has about forty
literal construction sites, and still carries the retired bot's fields.

Alternative rejected: carry a paused autopilot into the new conversation.
Resume would then run the prompt in a conversation it was not started in, and
the journal would have to be rebuilt across files.

### 7. A heartbeat keeps autopilot from running without its controller

A small thread per project, separate from the three-second tab watcher,
writes `apas-controller.json` for each OMP tab every three seconds. It never
sends on the project's outbound queue. The watcher cannot do this job: during
an outage that queue fills from terminal output and the watcher blocks at its
next send, which would make every busy tab look abandoned after a minute.

The file holds a writer id, chosen at random when the CLI process starts,
and a counter that changes on every write; whether the project is connected
to the server and for how long it has not been; and whether the server can
control autopilot. The durations are measured on the CLI's monotonic clock.
The extension reads the file on the same one-second timer as the request
files (decision 5) and again in the step before a submission. It notes, on
its own monotonic clock, when it last saw the writer id or the counter
change, and treats the heartbeat as fresh for 60 seconds after that. It
compares for difference, never for order, so a replaced CLI process, which
writes a new id and starts its counter again, is recognised at once. No
wall-clock times are compared, so a clock step cannot keep a dead
controller's heartbeat fresh.

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

The extension holds whenever the heartbeat is not fresh, the server has been
unreachable for more than 600 seconds, counted from the last frame that
arrived from it, or the server cannot control autopilot. A hold starts at
the moment one of these three conditions is first true and lasts until none
is, whether or not an iteration is due and whether or not a turn is in
progress. The four hours below are counted from that moment. While a hold
lasts the extension submits nothing. While it lasts and no turn is in
progress, the condition the extension reports is holding; a turn in progress
is not stopped and keeps its condition. A hold is not a pause: iterations
continue as before when it ends within four hours. As soon as a hold has
lasted four hours it becomes a pause ("Paused: APAS could not control this
tab for more than four hours"), whatever its cause, so a machine that was
cut off for a day does not start work again by itself when contact returns.
A deadline, where there is one, keeps running throughout.

While a hold lasts because the controller or the server is gone, the path
that would carry the condition to clients is gone too. Clients show the state
as not reported, and the terminal's own status line shows the hold.

This is not the worker's lease. A turn in progress is not stopped, and the
bound is the tab's own machine clock. It does mean an autopilot cannot keep
starting work without end while nothing can see or stop it, which matters
more now that an autopilot need not have a limit. The pane host's existing
adoption grace (600 s, or 900 s around a reboot) still ends the provider
when the controller never returns. A tab on a direct PTY does not survive a
CLI reboot at all; its agent is restarted and its autopilot comes back
paused.

An older CLI writes no heartbeat, so an extension left running after a
rollback holds and, after four hours, pauses.

Alternative rejected: pause on every disconnect and require a person to
resume. A server deploy or a daemon reboot would then end every overnight
autopilot.

Alternative rejected: hold without a bound. It lets unattended work start
again by itself days after anyone last saw it.

Alternative rejected: one hour, the draft's bound. The owner chose four
hours, so that an autopilot rides out a longer outage or a maintenance
window. The cost is that work can start again by itself up to four hours
after anyone could last see or stop it.

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
  `CliToServer::PaneAutopilotState { session_id, pane_id, support, state, reported_at }`.
  Both carry the session and the pane, as the pane class of `inbound_scope`
  requires. `state` holds the autopilot (decision 5) and OMP's own modes
  (decision 11). `reported_at` is optional: the time at which the CLI last saw
  the tab refresh its report, which a client shows when the state stops
  being refreshed. The server replays it from its cache while the instance
  stays attached.
- `ServerToWeb::PaneAutopilotReceipt` to the requesting connection only, and
  `ServerToWeb::PaneAutopilotState` to viewers that negotiated the capability.
- An optional `autopilot` block on `MobilePaneSummary` (availability,
  condition, reason, next iteration, progress; no prompt), which older native
  builds ignore.
- The optional `negotiated_capabilities` on `ServerToCli::Registered`
  (decision 7).
- Nothing new on `SessionInstances`. A client already receives
  `runtime_access` for each instance, and disables the autopilot actions
  with the tab's existing reason when it is false (decisions 9 and 12).

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
detached. The server sends the availability it computes for every OMP
terminal tab to capable viewers at three moments: when a viewer attaches;
when an instance attaches, is replaced or detaches, where it sends
`SessionInstances` today; and when a roster report adds an OMP terminal tab.
A client that holds nothing for an OMP terminal tab shows not reported. The
control is disabled for instance update required. For not reported it says
so and offers Stop and Stop now only; like every control of the tab, these
are disabled with the tab's existing reason while its machine's instance is
not attached.

One state is reported by the CLI itself, as the exception to decision 5's
rule that the state report is the only source. When a tab's agent process
exits and the CLI did not ask for that exit, and the last state the CLI
relayed for that launch was on or paused, the CLI reports that autopilot as
paused with the reason "the agent process exited". It builds this from what
it last relayed, not from the tab's files, which fail the trust checks once
the process is gone. The frame keeps the `support` value supported, carries
the autopilot's id, prompt, timing, limits and progress as last relayed, and
is marked as reported by the CLI. It takes the place of "not reported", the
90-second rule does not apply to it, and it is sent again after each
registration. No Resume, Stop or Stop now is offered for it, because there
is no agent to resume and nothing that could apply a stop. The control says
"This tab's agent is not running. Reboot it first; its autopilot comes back
paused.", and the CLI refuses a request for such a tab with the same text.
When the agent is launched again the extension's own report replaces this
state (decision 6): the autopilot is then paused because the agent was
restarted, or ended if it had a deadline that passed meanwhile. The CLI
keeps this state in memory only, so after the CLI process is replaced the
tab reads "not reported" until the agent is launched again.

The server handles a request in this order. It requires that the requester
negotiated `tab_autopilot_v1`. It claims the request id. It resolves the
session with `resolve_target_session`. It requires a terminal OMP tab that is
not legacy bot history. It finds the owning instance and checks the
requester's runtime access to its machine, which is the whole of decision 9
and the same for every action. It checks the instance capability and refuses
an older machine with the update text. It validates a start's bounds. It
forwards to the owning instance only, and answers the requester from a
spawned task, so a slow tab cannot block that connection's other frames. If
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

### 9. Anyone who can type into the tab can start and stop; no policy switch yet

Every autopilot action, start, pause, resume, stop and stop now, needs what
typing into the tab needs: runtime access on the tab's machine
(`check_session_runtime_access`). A start needs nothing more, and a stop
nothing less.

This is the owner's decision. The draft let only the hosting account start
and resume. What the rule does:

- **It matches typing.** A collaborator with runtime access can already type
  a prompt, or `/loop`, into the terminal. The same person can now leave the
  tab on autopilot through APAS's own control, and from a phone.
- **It closes the shared-machine gap.** Under the draft's rule a cluster
  member could not start autopilot on their own project when it ran on a
  shared machine, and the machine's owner could do it only after being added
  to the project.
- **It lets one account spend another's login.** An autopilot runs on the
  hosting account's OMP login, with that account's files, whoever started
  it, and by decision 2 it has no end unless a limit is set. The start
  dialog names the machine and says that the OMP login configured there is
  spent, to everyone alike.
- **It stays visible and stoppable.** Everyone who can open the session sees
  the state and the prompt, and anyone with runtime access can pause or stop
  it. Taking a person's access away later does not stop an autopilot they
  started; someone who still has access stops it.

The rule governs requests that arrive through APAS, and decision 5 says why
it is not a boundary against the tab's own agent.

Alternative rejected: only the hosting account starts and resumes, as the
draft had it. It keeps an account's quota from being spent unattended by
someone else. The owner decided that whoever may use a tab may also leave it
running.

Alternative rejected for now: a permission on `EffectiveProjectPolicy`,
narrowing over deployment, cluster and project. It is the right shape if
starting autopilot should later be narrower than typing, and it can be added
without changing anything else. It adds three editors and a second
enforcement point.

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
  reason until a turn that started after it ended has run.
- **Conversation:** the marker line of decision 3, and a second status line
  with the condition.

No push notification is added for autopilot in this version, which the owner
confirmed. The existing ones are unchanged: a tab's first completed turn and
a published question still notify as they do today. Push reaches only
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
budget-limited. `getState()` gives only the status, the objective and the
budget. Tokens used and the real status arrive with `goal_updated`, so until
one arrives the indication shows the objective, the budget and active or
paused only. While a mode is active and continuing, the extension keeps
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
for. A phone can already pause either mode with the Esc key of its raw
terminal. What was missing is that nothing showed the mode, and this decision
supplies that.

Autopilot refuses to start or resume while goal, loop or plan mode is in any
state other than off, and pauses if one is switched on, because OMP's loop
suppresses its own goal continuation and two drivers on one tab cannot both
be right. Plan mode is in the rule for another reason: by reading, a prompt
submitted in plan mode may not start a turn, and task 1.5 confirms it. The
same applies to autoresearch, a third self-continuing mode that
`ctx.sessionModes` does not report. The extension detects it from its
experiment tool `init_experiment` being among `pi.getActiveTools()` and
checks before each submission. Each case has its own refusal and pause text
in the spec, for example "This tab is running OMP's autoresearch. Type
/autoresearch off in the terminal first." and "Paused: OMP's plan mode was
switched on". Plan mode and autoresearch are not otherwise shown.

Alternative rejected: show nothing for modes APAS did not start. Then the
existing behaviour stays invisible: a tab in goal mode looks finished after
every turn.

Alternative rejected for now: a remote stop for these modes. The owner
confirmed that they are shown and not stopped.

### 12. Where the controls live

- **Desktop:** a control in the pane toolbar before the Terminal/Conversation
  switch (the pane-toolbar delta). It opens the start dialog or, when on, a
  small menu with Pause or Resume, Stop, Stop now and Details (prompt,
  timing, limits or "no limit", start time, progress, and the time limit end
  when there is one). For an ended autopilot it shows the reason and opens
  the start dialog with that autopilot's values. For a viewer without
  runtime access on the tab's machine (`runtime_access` false on that
  instance), every action is disabled with the reason the tab already gives
  for why it cannot be used. That reason applies only while the tab's
  instance is attached; when it is not, the reason for a machine that is not
  running the project comes first. While the state is not reported and the
  machine is connected and supports autopilot, the control says so and
  offers Stop and Stop now only. The start dialog carries the statement the
  spec requires. The confirmations for closing a tab, rebooting its agent
  and closing the project say when a tab on autopilot is affected. The
  Reboot CLI confirmation and the Machines page's controls are left as they
  are, because neither holds per-tab state; what happens to an autopilot
  then is what decision 7 gives: a hosted tab holds, and a direct-PTY tab
  comes back paused. The Overview's pane cards show the condition beside the
  status.
- **Interrupt:** the toolbar's Interrupt button is shown today on every
  Working tab, and the CLI ignores it for a terminal tab. A tab on autopilot
  is Working most of the time, so the button a person reaches for first would
  do nothing, beside a Stop now that works. The delta removes it from
  terminal tabs.
- **Mobile browser:** "Start autopilot" in the tab's "Pane actions" sheet; a
  state notice with the condition, Pause or Resume, Stop (which asks whether
  to let the turn finish or interrupt it) and Details, shown while the tab is
  on autopilot or paused; every action disabled for a viewer without runtime
  access on the tab's machine, as on the desktop; Stop and Stop now among the
  occasional actions, and no notice, while the state is not reported, which
  is the ordinary case in the first seconds after every launch; the ended
  reason with "Start again"; the indication of OMP's own modes; a glyph on
  the tab's chip, which has no room for another word; and the phone's two
  confirmations, for closing a tab and for rebooting its agent. The notice
  is a state notice, like the existing notice for an unconfirmed send, not
  one of the occasional pane actions.
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
the existing session rejection. Its autopilot stops with it and comes back
paused only if that agent is started again.

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
- **Limits are optional, soft, and kept in the agent's own process.** No
  limit is the default, a single turn can run for hours past one that is
  set, and the agent could rewrite its own limits. → Stated in the dialog; a
  time limit, when set, always ends autopilot; Stop now interrupts.
- **An autopilot nobody stops.** Without a limit it runs until one of the
  cases in decision 2 pauses or ends it, and each of those needs a failure,
  an interrupt, a lost controller or a person. A healthy autopilot that
  keeps finding work spends its machine's login for as long as the provider
  allows. → The owner's decision. The dialog says it; the state is on every
  surface, with the iteration count and the start time; anyone who can use
  the tab can stop it; and a usage limit pauses it through the failure rule.
- **One iteration cannot be recalled.** A prompt OMP has accepted can start
  after Pause or Stop. → It counts as the turn in progress. After Stop now it
  is aborted if it starts a run; if OMP only records it, it stays in the
  conversation and is answered with the next turn.
- **A question blocks an unattended tab, and nobody is pushed a
  notification.** → Pending answer keeps precedence, so the tab shows red in
  every list. But since OMP 18.18.3 `ask` sends an HTML form that the
  question bridge does not recognise, so the question's content and remote
  answering are probably unavailable on current OMP and the person must
  answer in the terminal; the start dialog says that this can happen. This
  was read from the code and not run. The owner left the repair out: this
  change does not check it or wait for it. If the hosting account's OMP
  settings give `ask` a timeout, questions answer themselves while nobody is
  there.
- **The hosting account's quota, spent by anyone who can use the tab.** An
  autopilot can exhaust the login the hosting account's other tabs use,
  whoever started it, and a usage limit then pauses it within minutes. →
  Three failures pause it instead of hammering the provider; the dialog says
  which machine's OMP login is spent; the state shows on every surface; and
  the runbook points to OMP's own setting for waiting out a reset.
- **Unreviewed changes.** The tab can commit and push with the account's
  credentials, and PR hand-off is a non-draft `gh pr create`. → Same as any
  tab; the dialog says so and says whether the tab uses the shared checkout.
- **Access is not containment.** Anyone who can type into the tab can start
  autopilot, and any process of the account can write the request files. →
  Said in the spec, the dialog and the runbook.
- **Background work after Stop now.** Jobs a turn launched keep running.
  After Stop now interrupts a running turn, OMP records their results without
  starting a turn; a tab that was only waiting for them can still start one
  (decision 4). → Said where Stop now is offered.
- **Interrupts and settles are inferred.** An extension is not told who
  aborted, or that a `/compact` cut a turn short. A killed job, or an abort
  from another part of OMP, can pause autopilot, and a turn hidden by a
  cancelled `/compact` leaves it "running an iteration". → Both fail towards
  doing less: a pause states its reason, and a stuck autopilot submits
  nothing. A time limit, when set, still ends it; without one it stays shown
  as running until a later run ends or a person stops it.
- **"No tool call" is a heuristic.** It can end an autopilot early or never.
  → It is optional, labelled as what it is, and limits can be set beside it.
- **A bigger extension.** A fault in it would also take down activity
  reporting and the question bridge. → Autopilot code is isolated behind its
  own guard and uses only managed timers; a broken optional import cannot
  stop the extension loading.
- **False holds.** A stalled heartbeat writer stops iterations, and after
  four hours pauses them. The pane host and the CLI each work out the runtime
  directory from their own environment; if they ever differed, the heartbeat
  would land where the extension does not look. → Both fail in the safe
  direction, and the terminal's status line shows the hold.
- **Two unattended tiers.** People may take autopilot for the safe one. →
  Different name, no worker icon or vocabulary (decision 10), and the
  statement in the start dialog.
- **Prompt text.** The prompt is handled like a typed message: OMP's keywords,
  `@path` mentions and a standalone `+500k`-style budget token all apply, and
  slash commands and skills are not run. → The dialog says that slash
  commands and skills are not run; the runbook also covers keywords, `@path`
  mentions and budget tokens. The prompt never appears in a list or a
  summary.
- **One conversation grows.** Every iteration is appended to one OMP session,
  and the CLI re-reads that file whenever it changes. → Nothing bounds it
  unless a limit is set. Compacting or starting a fresh session between
  iterations is a later option, and it matters more now that no limit is the
  default.
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
   autopilot left running holds and then pauses when the CLI or the server
   goes first: an older CLI writes no heartbeat, and an older server does not
   echo the capability. The web is the exception. After a web-only rollback
   the server still echoes the capability to the CLI, so a running autopilot
   continues and no browser shows or stops it until the web is rolled
   forward. The terminal's status line still shows it, and Esc during a
   turn, closing the tab, rebooting its agent and stopping the project still
   work. Stop running autopilots before rolling the web back alone.
5. This change depends on the worker cutover and on multi-machine tabs. Both
   have run on the production server and web since 2026-11-13, and their CLI
   is published; a machine takes it at its next daemon reboot. This change
   deploys nothing by itself.

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
lets members operate projects on shared machines; by decision 9 autopilot
follows the rule for input there, so a member who may type into a tab on a
shared machine may also start and stop its autopilot.

No main spec defines the OMP terminal provider yet; `provider-support` lists
five terminal profiles without it. That drift predates this change, which
states its own preconditions and modifies nothing there.

## Decisions taken by the owner

The draft took a position on nine questions and listed them here. The owner
answered them on 2026-11-13. Five answers changed the draft.

1. **Limits are optional, and no limit is the default.** The draft required
   an end date, at most 1,000 iterations and seven days. Changed in
   decisions 2 and 4.
2. **Anyone who can type into the tab can start and stop.** The draft let
   only the hosting account start and resume. Changed in decision 9, which
   also drops the `viewer_hosts` field the draft added.
3. **The time limit lets a turn in progress finish.** As drafted.
4. **A hold longer than four hours becomes a pause.** The draft said one
   hour. Changed in decision 7.
5. **No push notification in this version.** As drafted.
6. **OMP's own modes are shown, not stopped.** As drafted.
7. **No token limit.** The draft offered an approximate one. Removed from
   decisions 2, 4 and 6, together with the task that checked OMP's usage
   statistics.
8. **The question bridge is left alone for now.** The draft wanted its
   repair to land first as a separate change, and had a task whose result
   decided whether to wait. Nothing here checks it or waits for it. The risk
   and the dialog's sentence about questions stay.
9. **An ended autopilot's reason clears by itself.** As drafted: it stays on
   the control until another start, and in lists until a later turn has run.

## Open Questions

- Whether group 1 runs against a stub model or a short run on a real, cheap
  model. Task 1.1 asks before building either.
- Whether to add an in-terminal `/autopilot` command for status, pause and
  stop. The draft kept start and resume out of it because the server could
  not authorize them. Anyone who can type may now start, so that reason is
  gone, and the command could start too if the start dialog's statement were
  shown some other way.
- Whether the state should say who started an autopilot, now that it need
  not be the hosting account. Nothing records it today.
- Whether the heartbeat's "connected" should mean that the tab's instance is
  attached to its session, not only that the CLI is registered. Today a CLI
  whose instance was replaced by another runtime is neither closed nor told.
- Whether the owner of `unify-autonomous-workers` agrees to the retitle.
- Whether any host uses a GitHub Copilot login for OMP, which decides whether
  the billing attribution needs a decision.
- The marker glyph and the exact wording of the start dialog's statement.
