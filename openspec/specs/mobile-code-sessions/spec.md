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
The session view SHALL present coding work as an ordered, incrementally updated activity timeline that distinguishes user instructions, agent progress, tool activity, plans, questions, approvals, diffs, test outcomes, pull requests, completion, and errors. The client SHALL support a concise default presentation with access to additional detail when retained by APAS.

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
