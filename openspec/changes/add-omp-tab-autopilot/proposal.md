## Why

APAS has one way to leave an agent working unattended: the contained
autonomous worker. It cannot run yet. Its hosts need dedicated filesystems, a
pinned image and their own API key, and four of its own qualification gates and
seven of its predecessor's are open. The same cutover removes the old Start
bot, so once it ships nothing in APAS keeps an agent going while its owner is
away.

An OMP tab can already keep itself going. Typing `/goal` or `/loop` into its
terminal does it, with the tab's own login and permissions. APAS has no control
that shows or stops this: the tab looks idle between turns, nothing says it is
running by itself, and stopping it from a phone means opening the raw terminal
and pressing its Esc key.

This change adds a light, in-tab tier beside the worker. A person leaves an
OMP tab repeating an instruction within finite limits, and sees, pauses and
stops that from the desktop web and the mobile browser. It is for work its
owner would already trust to a tab. It is not contained, verified or budgeted
the way a worker is, it has no backlog or checklist, it opens no draft pull
request, and it says so. It is not folded into the worker product, because
worker readiness must fail closed, and one name for two trust models is how a
person ends up believing a tab was sandboxed.

## What Changes

- A person can put an OMP terminal tab on **autopilot**: a prompt that is
  submitted again each time the tab settles, either at once or on a set
  interval of one minute to 24 hours. A start must carry an iteration limit
  (at most 1,000) and a time limit (at most seven days), and may carry a token
  limit. The start dialog proposes 20 iterations and 8 hours. There is no
  default prompt; the dialog offers what was last used in that tab, so an
  ended autopilot can be started again in one step.
- Autopilot ends by itself at any limit, or, optionally, after an iteration
  that makes no tool call. The limits are checked between turns, so a turn in
  progress is never cut off and can outlast them. The time limit always ends
  autopilot at its deadline, whatever the tab is doing then.
- The loop runs inside the tab's OMP process, in the extension APAS already
  loads there to report activity. OMP's own `/goal` and `/loop` are **not**
  driven by APAS. They are shown: a tab in either mode is marked as continuing
  by itself, with how to stop it in the terminal. Not using them gives up what
  they offer, such as the agent declaring a goal met, a stop condition on
  project state, and repeating a slash command; the design lists it.
- Pause, Resume, Stop and Stop now work from the desktop web and the mobile
  browser. A request is answered as accepted, refused or not confirmed. The
  displayed state changes only when the tab reports it, and a request is never
  resent. A stop that was not confirmed still applies when the tab next reads
  its requests; a start that was not confirmed never applies later.
- Interrupting a turn in the terminal (Esc) pauses autopilot, as it pauses
  OMP's own loop. Without this rule the prompt would come back five seconds
  later.
- Autopilot state is its own attribute of a tab, beside Working, Pending answer
  and Idle. A tab waiting for its next iteration is not listed as an idle tab,
  and a paused or ended autopilot says why.
- Only the account that hosts a tab can start or resume its autopilot through
  APAS. On a shared machine that is the machine's owner, also for a project a
  cluster member created there. Anyone who may type into the tab can pause or
  stop it.
- Autopilot starts no new iteration without its controller. It holds once the
  tab's APAS process has been absent for a minute or its machine has been off
  the server for ten minutes. It continues when contact returns within an
  hour and pauses after that. It comes back paused after the agent process
  restarts, and it ends when the tab is switched to another conversation.
- Iterations are recorded as autopilot's, not as the person's messages.
- Three consecutive failed iterations pause autopilot instead of retrying
  without end. Reaching the login's usage limit therefore pauses a
  back-to-back autopilot within a few minutes, until its owner resumes it.
- The start dialog states what this mode is: the tab keeps running on its host
  as the account APAS runs under, with the tab's own permissions, and APAS
  adds no sandbox, independent review or spending limit. The limits are kept
  by code inside the agent's own process. They bound the loop and are not a
  security boundary.
- This reverses, for interactive OMP tabs only, the worker cutover's position
  that unattended work belongs to workers alone. Autopilot is never counted,
  listed or offered as a worker, and legacy bot history stays read-only. A
  retired bot is not converted: its owner recreates it by hand on an OMP tab.
- Autopilot is offered only where the server, the tab's machine, the tab's own
  agent process and its OMP version (18.18.2 or later) support it. A machine
  gains it when its daemon is rebooted to update. A tab started earlier needs
  an agent reboot, which ends that tab's turn in progress and is never done
  automatically. Older clients are unaffected.
- The pane toolbar stops showing Interrupt on terminal tabs. The button has
  never had an effect there, and it would now sit beside the autopilot
  control.
- First version: OMP terminal tabs, desktop web and mobile browser. Left out:
  the native app; push notifications, which reach only native-app
  installations (an end or a pause is shown in the tab and in the lists);
  stopping OMP's own modes remotely; other providers; starting autopilot when
  a tab is created; and a project or cluster permission.

## Capabilities

### New Capabilities

- `omp-tab-autopilot`: starting, bounding, pausing and stopping a repeated
  prompt on an OMP terminal tab; how its state is reported and shown; what
  happens when its controller, server or agent process goes away, or when a
  person interrupts it in the terminal; how OMP's own goal and loop modes are
  shown; and how the mode is kept apart from autonomous workers and the
  retired bot.

### Modified Capabilities

- `pane-toolbar`: adds the autopilot control to the pane toolbar of an OMP
  terminal tab, placed so the existing Terminal/Conversation requirements
  still hold, and removes Interrupt from terminal tabs, where it does nothing.

## Impact

- **Shared protocol:** a start request with its limits, pause/resume/stop
  requests, a correlated receipt with three outcomes, and a per-tab autopilot
  state frame, all gated by a new instance capability and a new viewer
  capability; an optional autopilot summary on the per-pane session summary;
  an optional capability echo on the CLI's registration reply; every new
  CLI-bound and inbound variant classified for multi-machine routing.
  Regenerated schemas, protocol TypeScript and fixtures. No new HTTP route, so
  no nginx change.
- **CLI:** the OMP extension gains the loop, its limits, interrupt handling,
  two request files, a state report and the journal entries that survive a
  restart. The CLI gains the exchange with it, a controller heartbeat written
  by its own thread with real connection state, the state relay, a feature
  marker in the tab's launch record, and the transcript handling that marks
  iterations. The extension's JavaScript tests join a real gate.
- **Server:** authorization and routing of the requests to the tab's own
  machine, a cached per-tab state with the same pruning as pane status, the
  availability it fills in for viewers, the session-summary field and viewer
  gating. No notification change.
- **Web:** the toolbar control and start dialog, a tab marker, the status-dot
  label, the idle-list group, the conversation marker, confirmations that
  mention autopilot, the Interrupt guard, and the mobile-browser action, state
  notice and details.
- **Documentation:** `AGENTS.md` gains the autopilot section, and several of
  its existing sentences become incomplete and are corrected. The unarchived
  worker change needs a reconciliation note, with its owner's agreement.
- **Depends on:** the worker cutover (e02c0f7) and multi-machine tabs
  (`run-project-tabs-across-machines`), both committed and neither deployed.
  Nothing here deploys, enables a policy or restarts a running agent.
- **Should land first, as its own change:** a repair of the OMP question
  bridge. Since OMP 18.18.3 the `ask` tool sends one HTML form, which APAS's
  question bridge does not recognise, so question cards and remote answers for
  OMP tabs are likely not working on current OMP. This comes from reading the
  code, not from a run; the first task group checks it and reports the result.
  A tab that asks a question waits until someone answers, by default without a
  time limit. The Pending answer status should still work, so such a tab
  shows red, but the question may have to be answered in the terminal. The
  start dialog says so.
