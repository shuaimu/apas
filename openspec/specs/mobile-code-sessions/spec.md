# mobile-code-sessions Specification

## Purpose

Defines a secure, code-focused iOS and Android companion for starting, monitoring, steering, and reviewing APAS coding sessions away from the desktop workspace.
## Requirements
### Requirement: The mobile application provides a code-only APAS experience
The system SHALL provide supported iOS and Android clients whose primary navigation is limited to APAS coding projects, sessions, work requiring attention, and account settings. The mobile application SHALL NOT expose a general-purpose AI chat surface or imply that coding agents execute locally on the phone.

#### Scenario: User opens the mobile application
- **WHEN** an authenticated user launches the mobile application
- **THEN** the application opens a code-session home showing work relevant to that user
- **AND** does not present a general chat inbox or consumer-chat composer

#### Scenario: User opens the application on a tablet
- **WHEN** the application runs on a supported tablet layout
- **THEN** it may show additional session navigation beside the active detail view
- **AND** preserves the same capabilities and authorization behavior as the phone layout

### Requirement: Mobile authentication and transport are secure and revocable
The mobile client SHALL communicate with production APAS services only over authenticated HTTPS and WSS connections. Long-lived device credentials SHALL be protected by platform secure storage, independently revocable, and never made available to terminal-rendering content. The server SHALL remain the authorization boundary for every mobile request and event.

#### Scenario: User signs in on a trusted device
- **WHEN** valid credentials are supplied over a secure connection
- **THEN** the server creates a revocable mobile device session
- **AND** the application stores only the credentials required to resume that session in platform-protected storage

#### Scenario: Production endpoint is cleartext
- **WHEN** the mobile application is configured with an HTTP or WS production endpoint
- **THEN** it refuses to transmit credentials or connect
- **AND** explains that a secure endpoint is required

#### Scenario: Device session is revoked
- **WHEN** the user logs out, an administrator suspends the account, or the device session is revoked elsewhere
- **THEN** subsequent requests and reconnects are rejected
- **AND** cached sensitive session content and credentials are cleared from the device

### Requirement: Users can discover and resume accessible coding sessions
The application SHALL list active and recent coding sessions the authenticated user may access, grouped or filterable by project and state. Each entry SHALL identify the project, instance or machine when known, current activity state, attention state, and latest meaningful update without exposing projects the user cannot access.

#### Scenario: User has parallel coding sessions
- **WHEN** the user opens the code-session home with several active or recent sessions
- **THEN** the application shows each accessible session with enough project and status context to distinguish it
- **AND** supports filtering active, attention-required, completed, and recent work

#### Scenario: User opens an existing session
- **WHEN** the user selects an accessible session
- **THEN** the application attaches to that same APAS session and displays its retained activity and current state
- **AND** actions subsequently taken on web or CLI appear in the mobile view

#### Scenario: User has no accessible sessions
- **WHEN** the authenticated user has no accessible sessions
- **THEN** the application renders an empty code-session home with an available new-task action
- **AND** does not render a blank or broken workspace

### Requirement: Authorized users can start coding tasks from mobile
The application SHALL allow an authorized user to start a coding task by selecting an eligible machine and project or existing project instance, entering a coding instruction, and selecting only server-supported execution options. Task creation SHALL be idempotent and SHALL not report success until the server acknowledges the resulting session or operation.

#### Scenario: User starts a task in an available project
- **WHEN** the user selects an online eligible project instance, enters a non-empty instruction, and confirms creation
- **THEN** the server starts or attaches the requested coding session exactly once
- **AND** the application navigates to that session's live activity view

#### Scenario: User retries after an uncertain response
- **WHEN** task creation is retransmitted with the same client request identifier after a timeout
- **THEN** the server returns the original result or continues the original operation
- **AND** does not create a duplicate project instance or coding session

#### Scenario: Selected execution target is unavailable
- **WHEN** the chosen machine, project, provider, or launch option becomes unavailable or disallowed before creation completes
- **THEN** the server rejects the operation with an actionable reason
- **AND** the application preserves the user's draft instruction for correction or retry

### Requirement: Mobile sessions expose a concise live coding activity timeline
The session view SHALL present coding work as an ordered, incrementally updated activity timeline that distinguishes user instructions, agent progress, tool activity, plans, questions, approvals, diffs, test outcomes, pull requests, completion, and errors. The client SHALL support a concise default presentation with access to additional detail when retained by APAS. For a terminal pane the timeline SHALL hold the user and assistant turns and the questions recovered from the provider's transcript, with the pane's working, pending-answer and idle state. Tool activity, plan review and approvals SHALL be shown only for a retained structured pane, which is the only kind that reports them.

#### Scenario: Session runs a terminal pane
- **WHEN** a connected session's pane is a terminal pane
- **THEN** the timeline shows its recovered turns and questions in order
- **AND** it does not show tool, plan or approval items for that pane

#### Scenario: Agent performs a sequence of coding actions
- **WHEN** a connected session emits status, tool, test, diff, and completion events
- **THEN** the mobile timeline renders those events in their server-defined order
- **AND** highlights the current state and any item requiring user attention

#### Scenario: High-volume output is received
- **WHEN** a session produces more detail than is practical to render at once on a phone
- **THEN** the application keeps the timeline responsive through bounded rendering and pagination
- **AND** retains access to older server-persisted history on demand

### Requirement: Authorized users can steer and decide from mobile
The application SHALL allow authorized project users to send coding instructions, answer agent questions, approve or reject gated actions, and interrupt eligible running work. Each mutation SHALL name its target session and pane, be reauthorized by the server, and expose acknowledgement or failure without optimistic claims of completion.

#### Scenario: User redirects running work
- **WHEN** an authorized user sends a follow-up coding instruction to an active pane
- **THEN** the instruction is delivered to that exact session and pane
- **AND** appears once in every attached authorized client after acknowledgement

#### Scenario: User answers an approval request
- **WHEN** the application displays a still-pending approval or question and the authorized user responds
- **THEN** the server applies the first valid response according to current session state
- **AND** all clients converge on the resolved outcome

#### Scenario: Stale mobile client attempts a privileged action
- **WHEN** project access, ownership, account status, or the pending decision changed after the mobile view loaded
- **THEN** the server rejects the stale action
- **AND** the application removes or refreshes controls that are no longer permitted

### Requirement: Users can review coding outcomes on mobile
The application SHALL provide mobile-readable plan-review requests, pane diffs, test or error summaries, and pull-request outcomes available for an accessible session. Review surfaces SHALL preserve source formatting while adapting navigation and line presentation to a narrow screen.

#### Scenario: Session has a reviewable diff
- **WHEN** the user opens a diff event for an accessible pane
- **THEN** the application shows the affected files and additions or removals with horizontal or unified navigation suitable for the device
- **AND** clearly identifies truncation or server-side errors

#### Scenario: Session creates a pull request
- **WHEN** APAS reports a pull-request URL or creation error
- **THEN** the session timeline shows the outcome
- **AND** an allowed URL can be opened through the operating system after user action

### Requirement: Mobile continuity survives normal app suspension and reconnects
The client SHALL treat backgrounded or silent WebSocket connections as potentially stale. On foregrounding or network recovery it SHALL reauthenticate, refresh authorization, reattach relevant sessions, request incremental catch-up from persisted server watermarks, and reconcile events without duplication or silent gaps.

#### Scenario: Operating system suspends the application
- **WHEN** the application returns to the foreground after its socket was suspended or lost
- **THEN** it establishes a fresh authenticated connection rather than trusting the old socket state
- **AND** catches up every open or attention-relevant session from its last confirmed watermark

#### Scenario: Event arrives during disconnection
- **WHEN** a coding event is persisted while the mobile client is disconnected
- **THEN** reconnect catch-up adds the missing event exactly once in server order
- **AND** advances the persisted watermark only after the event is accepted locally

#### Scenario: Client protocol is incompatible
- **WHEN** the server and installed application cannot negotiate a safe protocol capability set
- **THEN** mutations are disabled
- **AND** the user receives an upgrade-required message instead of undefined behavior

### Requirement: Offline behavior is explicit and safe
The application SHALL permit read-only viewing of locally cached session summaries when offline, clearly label their freshness, and disable network mutations and terminal input until connectivity and authorization are restored. It SHALL NOT represent locally queued destructive or time-sensitive decisions as accepted.

#### Scenario: User opens a cached session offline
- **WHEN** recent session data exists locally but the server cannot be reached
- **THEN** the application displays the cached data with an offline and last-updated indicator
- **AND** disables approvals, interrupts, task launch, and terminal input

#### Scenario: Connectivity returns
- **WHEN** the application reconnects after offline viewing
- **THEN** it refreshes authorization and catches up before re-enabling mutations
- **AND** visibly reconciles any session state that changed while offline

### Requirement: Mobile presents machines beside coding sessions

The mobile browser home SHALL let a user move between their coding sessions and the machines those sessions run on, without leaving it. The native application does not list machines. Machines SHALL be reachable in the same way the session filters are.

#### Scenario: A user switches to machines

- **WHEN** a user selects machines on the mobile browser home
- **THEN** the machines they can reach are listed in place of the session list

#### Scenario: A user switches back to sessions

- **WHEN** a user selects a session filter again
- **THEN** their coding sessions are listed as before

### Requirement: The session screen favours the conversation over its chrome

On a session screen the controls used while following a conversation SHALL be reachable without displacing the conversation itself. Moving between the session's panes SHALL be available in the topmost row of the screen. Controls that navigate away from the session SHALL NOT appear there. Actions used occasionally against the selected pane, and project management, SHALL be gathered behind a single control in that same row rather than each taking space of its own — neither above the conversation nor alongside the message composer, which is for composing.

#### Scenario: Opening a session

- **WHEN** a user opens a session on a phone
- **THEN** the pane list is in the topmost row, beside the control that returns to the session list
- **AND** the project's identity and state are still shown

#### Scenario: Switching panes

- **WHEN** a user switches panes
- **THEN** they do so from that same topmost row without scrolling the conversation away

#### Scenario: Occasional pane actions

- **WHEN** a user opens the control that gathers the occasional actions
- **THEN** it offers the raw terminal and the work summary for the selected pane, and rebooting or closing that pane
- **AND** each acts on the selected pane

#### Scenario: The composer is for composing

- **WHEN** a user looks at the message composer
- **THEN** it carries only composing and sending
- **AND** the occasional pane actions are not duplicated there

#### Scenario: Rebooting the selected pane

- **WHEN** a user chooses to reboot the selected pane
- **THEN** the system confirms before anything is restarted
- **AND** states that the agent returns on the same session with its prior conversation, since the action otherwise reads as losing it

#### Scenario: Closing the selected pane

- **WHEN** a user chooses to close the selected pane
- **THEN** the system confirms before anything is closed
- **AND** a pane holding an isolated worktree offers the same choice of what to do with that work as other surfaces do, rather than discarding it silently

#### Scenario: Leaving for account settings

- **WHEN** a user is on a session screen
- **THEN** no control there navigates to account settings

### Requirement: A project's allowed tab types can be managed from mobile

The application SHALL let a user reach project management for the session they are viewing from the control that gathers the session screen's occasional actions, and there see which tab types the project permits. A user who may manage the project SHALL be able to change that set; any other user SHALL see it without being able to change it. A project that has never been restricted SHALL show every tab type as permitted.

#### Scenario: An owner restricts what may be created

- **WHEN** a user who may manage the project clears a tab type from the permitted set
- **THEN** the project stops permitting new tabs of that type
- **AND** the change applies to the project rather than to that user's device

#### Scenario: A user who may not manage the project

- **WHEN** a user who may not manage the project opens project management
- **THEN** they see which tab types are permitted
- **AND** cannot change them

#### Scenario: A project with no restriction

- **WHEN** a project has never had its tab types restricted
- **THEN** every tab type shows as permitted
- **AND** nothing suggests the project is restricted

#### Scenario: A tab type the cluster policy already forbids

- **WHEN** a tab type is not permitted by the effective cluster policy for the project
- **THEN** project management SHALL NOT present it as permitted
- **AND** SHALL indicate that the restriction is not the project's to change
- **AND** no user, including one who may manage the project, may permit it there

#### Scenario: Existing panes are unaffected

- **WHEN** a tab type is removed from the permitted set while panes of that type are open
- **THEN** those panes keep running and can still be relaunched
- **AND** only the creation of new tabs of that type is refused

### Requirement: Idle work is listed per agent, not per project

The application SHALL offer a list of the agents that are not currently working, with one entry per agent rather than one per project. An agent that is idle SHALL appear there even when other agents in the same project are working. Each entry SHALL identify the project it belongs to before the agent, giving both equal prominence, and SHALL identify its host. The project places the entry and the agent is what opening it selects, so neither is subordinate to the other.

#### Scenario: A project with both busy and idle agents

- **WHEN** a project has one agent working and others idle
- **THEN** the idle agents appear in the list
- **AND** the working one does not

#### Scenario: Every agent in a project is working

- **WHEN** every agent in a project is working
- **THEN** none of them appears in the list

#### Scenario: A project that is not running

- **WHEN** a project is not running
- **THEN** none of its agents appears in the list

#### Scenario: An agent whose machine is not running the project

- **WHEN** a project runs on one machine and an agent belongs to another machine that is not running it
- **THEN** that agent does not appear in the list

#### Scenario: Placing an entry

- **WHEN** a user reads an entry in the list
- **THEN** it names the agent, and the project and host it belongs to

#### Scenario: Reading an entry

- **WHEN** a user reads an entry
- **THEN** the project is named before the agent
- **AND** neither is presented less prominently than the other

#### Scenario: Opening an idle agent

- **WHEN** a user opens an entry
- **THEN** the session opens with that agent selected, rather than whichever was last used

#### Scenario: Returning to the idle list

- **WHEN** a user opens an agent from Idle sessions and returns from its conversation
- **THEN** Idle sessions remains selected
- **AND** the user returns to the list they entered from rather than All projects

#### Scenario: Nothing is idle

- **WHEN** no agent is idle
- **THEN** the list says so rather than appearing broken

### Requirement: Per-agent state is reported to mobile

The system SHALL report, for each session a mobile client can reach, the agents in it and whether each is currently working. This SHALL be derived from the same source as the session-level working flag, so the two cannot disagree. A client receiving no per-agent detail SHALL treat it as unknown rather than as idle.

#### Scenario: A session's agents are reported

- **WHEN** a mobile client loads what it can reach
- **THEN** each session carries its agents and whether each is working

#### Scenario: Agreement with the session-level flag

- **WHEN** a session reports that it is working
- **THEN** at least one of its agents is reported as working

#### Scenario: An authoritative idle snapshot follows a missed clear

- **WHEN** the client retains an earlier working indicator for an agent
- **AND** a refreshed session snapshot reports that agent as idle
- **THEN** the conversation clears the stale working indicator
- **AND** reopening the same session does not restore it

#### Scenario: An older server omits the detail

- **WHEN** the per-agent detail is absent from a response
- **THEN** the client still functions
- **AND** does not present those agents as idle

### Requirement: Mobile groups usage-limited agents beneath idle agents

The application SHALL treat provider availability as separate from whether a pane is currently working. The Idle sessions view SHALL show provider-blocked panes in a labelled Usage limited subsection below the ordinary idle panes, with the limiting window and reset time when known. Panes with an open question SHALL be grouped above the ordinary idle panes.

#### Scenario: A non-working pane is usage limited

- **WHEN** a pane is not working and its provider reports an active usage limit
- **THEN** it does not appear among the ordinary idle rows
- **AND** it appears in the Usage limited subsection with a usage-limited label rather than an idle label

#### Scenario: The provider permits extra usage

- **WHEN** included utilization reaches 100 percent and the provider reports that extra usage is enabled and available
- **THEN** the pane is not classified as usage limited from utilization alone

#### Scenario: The reported reset passes

- **WHEN** the usage limit's reset time passes before a refreshed payload arrives
- **THEN** the client stops presenting the pane as usage limited

### Requirement: Mobile ranks recently idle agents first

The application SHALL record when each pane transitions from working to idle and SHALL order Idle sessions from the most recent transition to the oldest. Repeated idle observations SHALL preserve the existing transition time. A client receiving an older pane summary without that timestamp SHALL still show it after timestamped idle panes.

#### Scenario: A pane becomes idle after another pane

- **WHEN** two idle panes have different idle-transition times
- **THEN** the pane that became idle later appears first

#### Scenario: The same idle state is replayed

- **WHEN** an idle pane receives another idle observation without working in between
- **THEN** its idle-transition time does not change

#### Scenario: An older server omits idle recency

- **WHEN** an idle pane has no idle-transition timestamp
- **THEN** it remains in the list after timestamped idle panes

### Requirement: An agent that has not started a conversation is distinguishable from a quiet one

Where a pane's turns are recovered from what the agent records, a pane with no recorded turns SHALL be presented as not having started a conversation, rather than as having no activity. The presentation SHALL note that the agent may be starting up or waiting at a prompt of its own, and SHALL offer the terminal view where that state is visible and answerable.

#### Scenario: A pane at its provider's startup prompt

- **WHEN** a user opens a terminal pane whose agent has recorded no turns
- **THEN** the view says the agent has not started a conversation
- **AND** offers the terminal view

#### Scenario: A pane whose turns are observed rather than recovered

- **WHEN** a user opens a pane whose activity is observed directly rather than recovered from a transcript
- **THEN** the ordinary empty state is shown, since no such inference is available

### Requirement: A message the agent never records is reported as unconfirmed

Where a message is delivered to an agent as keystrokes, the system SHALL NOT treat a successful write as delivery. It SHALL confirm delivery only by the agent recording the message, and SHALL report a message that remains unrecorded after a grace period as unconfirmed, offering the terminal view. A message recorded later SHALL clear the report. An unconfirmed message SHALL NOT be sent again automatically and SHALL stay listed across a page reload until the agent records it or the user dismisses it.

#### Scenario: A message that lands

- **WHEN** the agent records a message the user sent
- **THEN** nothing is reported

#### Scenario: A message sent while the agent cannot receive it

- **WHEN** the agent has not recorded a sent message once the grace period has passed
- **THEN** the system reports it as not recorded
- **AND** offers the terminal view

#### Scenario: Confirmation arrives late

- **WHEN** a message is reported as unconfirmed and the agent then records it
- **THEN** the report is withdrawn

#### Scenario: Within the grace period

- **WHEN** a message has been sent and the grace period has not passed
- **THEN** nothing is reported, since recording legitimately lags

#### Scenario: The agent merely repeats the text

- **WHEN** the agent's own output contains the text but the user's message was never recorded
- **THEN** the message is still reported as not recorded

### Requirement: Provider prompts are operable from a mobile terminal

The mobile raw-terminal view SHALL provide touch controls for terminal keys
that ordinary phone keyboards omit. The controls SHALL send terminal input to
the selected pane through the same byte-preserving path as xterm input and
SHALL remain generic to the provider's TUI rather than recognizing a particular
startup screen.

#### Scenario: Navigate a resume picker without arrow keys

- **WHEN** a mobile user opens a terminal pane at a provider selection prompt
- **THEN** the user can send Up and Down arrow keys and Enter from touch controls

#### Scenario: Dismiss or interrupt a terminal prompt

- **WHEN** a mobile user needs to dismiss a prompt or interrupt the running command
- **THEN** the user can send Escape or Ctrl-C from touch controls

#### Scenario: Terminal input is disconnected

- **WHEN** the browser is not connected to the pane transport
- **THEN** the terminal key controls are disabled

### Requirement: A Codex terminal pane resumes its exact known conversation

APAS SHALL associate a Codex terminal pane with the actual user-session id of
the rollout it attributes to that pane's own Codex process: a rollout that
process holds open, or the thread that Codex's shared daemon reports as loaded
by that pane's TUI. Once known,
the id SHALL be persisted in the pane configuration and supplied to Codex on a
future restore. APAS SHALL NOT select the most recent cwd conversation as an
identity substitute.

#### Scenario: Restore after Codex identity was captured

- **WHEN** a Codex terminal pane is restored after APAS captured its session id
- **THEN** APAS launches `codex resume` with that exact id
- **AND** Codex does not require the user to choose among unrelated conversations

#### Scenario: Restore a legacy pane without a captured identity

- **WHEN** a Codex terminal pane has no verified Codex session id
- **THEN** APAS launches the ordinary Codex resume picker
- **AND** the mobile terminal controls make that picker operable

#### Scenario: Several Codex panes share a working directory

- **WHEN** multiple Codex conversations exist in the same working directory
- **THEN** APAS never uses `--last` to restore a pane
- **AND** only a session id attributed to that pane's provider process is persisted

### Requirement: Mobile users can rename the selected conversation

The mobile session conversation view SHALL let an authorized user rename its
currently selected real pane from the pane's More actions. The rename editor
SHALL start with the pane's current display label, accept only a trimmed,
non-empty label, and apply the saved label to that exact pane everywhere its
name is shown. The control SHALL not offer a usable mutation when no real pane
is selected, the project is not running, or the client is offline.

#### Scenario: Rename the selected conversation

- **WHEN** an online user opens More actions for a selected pane in a running project
- **AND** chooses Rename conversation, edits the prefilled label, and saves a non-empty name
- **THEN** surrounding whitespace is removed
- **AND** the selected pane's conversation heading and selector display the new name
- **AND** the rename follows the existing pane-label persistence and synchronization behavior

#### Scenario: Cancel a rename

- **WHEN** a user opens the rename editor and cancels or dismisses it
- **THEN** the selected pane's label remains unchanged
- **AND** no rename mutation is sent

#### Scenario: Reject an empty name

- **WHEN** the rename draft contains only whitespace
- **THEN** the user cannot save it
- **AND** the existing label remains visible

#### Scenario: No renameable pane is available

- **WHEN** no real pane is selected, the project is stopped, or the client is offline
- **THEN** Rename conversation is unavailable
- **AND** no pane-label mutation is sent

