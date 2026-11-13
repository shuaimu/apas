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
OMP tab repeating an instruction, until a limit they set or until someone
stops it, and sees, pauses and stops that from the desktop web and the mobile
browser. It is for work its owner would already trust to a tab. It is not
contained, verified or budgeted the way a worker is, it has no backlog or
checklist, it opens no draft pull request, and it says so. It is not folded
into the worker product, because worker readiness must fail closed, and one
name for two trust models is how a person ends up believing a tab was
sandboxed.

## What Changes

- A person can put an OMP terminal tab on **autopilot**: a prompt that is
  submitted again each time the tab settles, either at once or on a set
  interval of one minute to 24 hours. An iteration limit and a time limit
  can each be set, and neither is required. The start dialog proposes no
  limit, and an autopilot without one runs until someone stops it or one of
  the rules below pauses or ends it. There is no token limit. There is no
  default prompt; the dialog offers what was last used in that tab, so an
  ended autopilot can be started again in one step.
- Autopilot ends by itself at a limit it was given, or, optionally, after an
  iteration that makes no tool call. Limits are checked between turns, so a
  turn in progress is never cut off and can outlast them. A time limit ends
  autopilot at its deadline, whatever the tab is doing then.
- The loop runs inside the tab's OMP process, in the extension APAS already
  loads there to report activity. OMP's own `/goal` and `/loop` are **not**
  driven by APAS. They are shown: a tab in either mode is marked as continuing
  by itself, with how to stop it in the terminal. Not using them gives up what
  they offer, such as the agent declaring a goal met, a stop condition on
  project state, and repeating a slash command; the design lists it.
- Pause, Resume, Stop and Stop now work from the desktop web and the mobile
  browser. A request is answered as accepted, refused or not confirmed. The
  displayed state changes only when the tab reports it, or when its machine
  reports that the tab's agent process exited, and a request is never resent.
  A stop that was not confirmed still applies when the tab next reads its
  requests, unless its agent is restarted first, and then autopilot comes
  back paused anyway. A start is never left pending: the tab takes it within
  a few seconds or never, and one it did not take is answered as refused.
- An interrupt in the terminal pauses autopilot. Esc during a turn does, as
  it pauses OMP's own loop. So does Esc that cancels a retry the agent is
  waiting to make, and Esc that ends a wait for background work, once that
  work has finished. Killing the work a turn was waiting for pauses autopilot
  too, because the tab cannot tell the two apart. Without this rule the
  prompt would come back seconds later.
- Autopilot state is its own attribute of a tab, beside Working, Pending answer
  and Idle. A tab waiting for its next iteration is not listed as an idle tab,
  and a paused or ended autopilot says why.
- Anyone who may type into a tab can start, pause, resume and stop its
  autopilot, on their own machine or on one shared with them. No further
  permission is needed. An autopilot spends the OMP login of the machine it
  runs on, whoever started it.
- Autopilot starts no new iteration without its controller. It holds once the
  tab's APAS process has been absent for a minute or its machine has been off
  the server for ten minutes, whether or not an iteration is due then. It
  continues when that hold ends within four hours and pauses once the hold
  has lasted four hours. It comes back paused after the agent process
  restarts, and it ends when the tab is switched to another conversation.
- Iterations are recorded as autopilot's, not as the person's messages.
- Three consecutive failed iterations pause autopilot instead of retrying
  without end. Reaching the login's usage limit therefore pauses a
  back-to-back autopilot within a few minutes, until someone resumes it.
- The start dialog states what this mode is: the tab keeps running on its host
  as the account APAS runs under, with the tab's own permissions, and APAS
  adds no sandbox, independent review or spending limit. With no limit set,
  it says that autopilot runs until someone stops it. Limits that are set are
  kept by code inside the agent's own process. They bound the loop and are
  not a security boundary.
- This reverses, for interactive OMP tabs only, the worker cutover's position
  that unattended work belongs to workers alone. Autopilot is never counted,
  listed or offered as a worker, and legacy bot history stays read-only. A
  retired bot is not converted: its owner recreates it by hand on an OMP tab.
- Autopilot is offered only where the server, the tab's machine, the tab's own
  agent process and its OMP version support it. The OMP version must be at
  least 18.18.2; the first task group finds the real minimum. A machine gains
  autopilot when its daemon is rebooted to update. A tab started earlier
  needs an agent reboot, which ends that tab's turn in progress and is never
  done automatically. Older clients receive no new frame. They show each
  iteration as a generic tool entry and cannot see or stop the autopilot.
- The pane toolbar stops showing Interrupt on terminal tabs. The button has
  never had an effect there, and it would now sit beside the autopilot
  control.
- First version: OMP terminal tabs, desktop web and mobile browser. Left out:
  the native app; push notifications, which reach only native-app
  installations (an end or a pause is shown in the tab and in the lists);
  stopping OMP's own modes remotely; a token or spending limit; other
  providers; starting autopilot when a tab is created; and a project or
  cluster permission.

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

- **Shared protocol:** a start request with its optional limits,
  pause/resume/stop
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
  by its own thread with real connection state, the state relay, the pause
  it reports itself after an agent process has exited, a feature marker in
  the tab's launch record, and the transcript handling that marks
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
  worker change needs a reconciliation note and, with its owner's agreement,
  the retitle of one of its requirements; if that change is archived first,
  a RENAMED delta on `autonomous-worker-surfaces` is added here.
- **Depends on:** the worker cutover (e02c0f7) and multi-machine tabs
  (`run-project-tabs-across-machines`). Both have run on the production
  server and web since 2026-11-13, and their CLI is published; a machine
  takes it at its next daemon reboot. Nothing here deploys, enables a policy
  or restarts a running agent.
- **Known and left alone:** the OMP question bridge. Since OMP 18.18.3 the
  `ask` tool sends one HTML form, which APAS's question bridge does not
  recognise, so question cards and remote answers for OMP tabs are likely not
  working on current OMP. This comes from reading the code, not from a run.
  The owner decided to leave it out: this change does not repair it, check it
  or wait for a repair. A tab that asks a question waits until someone
  answers, by default without a time limit. The Pending answer status should
  still work, so such a tab shows red, but the question may have to be
  answered in the terminal. The start dialog says so.
