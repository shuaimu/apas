# mobile-code-notifications Specification

## Purpose

Defines privacy-safe mobile notifications and deep links that bring users back to APAS coding work requiring attention without relying on a background WebSocket.

## Requirements

### Requirement: Users can register and revoke mobile notification devices
The system SHALL bind each mobile push token to an authenticated user, installation, platform, and revocable device session. Logging out, revoking the device session, suspending the account, or receiving a permanent invalid-token response SHALL stop future delivery to that token.

#### Scenario: User enables notifications
- **WHEN** the operating system grants notification permission and the authenticated application registers its current push token
- **THEN** the server associates that token with the user's current device session
- **AND** does not allow another user to claim it without a new authenticated registration

#### Scenario: Push token rotates
- **WHEN** the platform replaces a device's push token
- **THEN** the application registers the replacement for the same installation
- **AND** the server retires the superseded token

#### Scenario: User logs out
- **WHEN** the user logs out of the application
- **THEN** the server revokes notification delivery for that device session
- **AND** the device removes local notification identity and sensitive routing data

### Requirement: Notifications target actionable coding events
The system SHALL support notifications for user-configurable coding events including questions or approvals requiring attention, terminal or agent failures, pull-request readiness, and task completion. Notification generation SHALL respect current account status, project access, per-device preferences, and event relevance at delivery time.

#### Scenario: Coding session requires a decision
- **WHEN** an accessible active session produces an unresolved question or approval request matching the user's preferences
- **THEN** the system sends at most one notification for that pending decision to each eligible installation
- **AND** resolves or withdraws its actionable state after another client answers it

#### Scenario: Task completes while mobile app is inactive
- **WHEN** an accessible session reaches a completed or PR-ready state and the user enabled that event type
- **THEN** the system sends a completion notification
- **AND** the application can retrieve authoritative current details after it opens

#### Scenario: User loses project access before delivery
- **WHEN** a queued notification refers to a project the user can no longer access
- **THEN** the system suppresses delivery when authorization can be rechecked
- **AND** a delivered stale notification reveals no sensitive project content

### Requirement: Lock-screen notification content is privacy-safe
By default, notification payloads SHALL contain only the minimum non-sensitive information needed to identify APAS and the event category. Prompt text, model output, source code, diffs, terminal output, secrets, and full filesystem paths SHALL NOT appear in lock-screen-visible content. Any richer preview SHALL require an explicit user preference and remain access-controlled when opened.

#### Scenario: Default approval notification is delivered
- **WHEN** an approval request notification is shown with default privacy settings
- **THEN** it indicates that APAS coding work needs attention
- **AND** does not include the command, source code, prompt, project path, or terminal contents

#### Scenario: Device is locked
- **WHEN** a user taps a coding notification from the lock screen
- **THEN** the application requires the operating system and APAS session to authorize access before showing sensitive details
- **AND** falls back to sign-in when the device session is unavailable

### Requirement: Notifications deep-link to the exact authorized context
Every actionable notification SHALL carry opaque routing identifiers sufficient to open the relevant project, session, pane, and decision after authentication. The application SHALL support equivalent verified universal or app links for opening the code home, an existing session, and an eligible new-task composer.

#### Scenario: User taps a valid session notification
- **WHEN** the user taps a notification for an accessible existing session
- **THEN** the application authenticates, opens that session, catches up current state, and focuses the referenced item when it still exists
- **AND** does not act on the decision merely because the notification was tapped

#### Scenario: Deep-linked item no longer exists
- **WHEN** the project, session, pane, or pending decision referenced by a link is gone or inaccessible
- **THEN** the application lands on the nearest authorized code-session screen
- **AND** explains that the original item is unavailable without disclosing it

#### Scenario: User opens a new-task link
- **WHEN** a verified link supplies an allowed project, machine, launch option, or prefilled coding instruction
- **THEN** the application opens the new-task composer with eligible values preselected
- **AND** still requires user review and explicit submission

### Requirement: Notification delivery is deduplicated and observable
The server SHALL assign stable notification event identifiers, avoid repeated delivery for the same event and installation within the configured policy, record non-sensitive delivery state, process provider receipts, and retire permanently invalid tokens. Push delivery SHALL be treated as a hint; authoritative session state SHALL always be fetched from APAS.

#### Scenario: Delivery is retried after a transient provider failure
- **WHEN** a push provider reports a retryable failure
- **THEN** the server retries with bounded backoff using the same logical event identifier
- **AND** does not create a new user-visible event record for each attempt

#### Scenario: Provider reports device not registered
- **WHEN** a delivery receipt permanently invalidates a device token
- **THEN** the server stops using that token
- **AND** retains enough non-sensitive status for administrators to diagnose delivery health

#### Scenario: Notification arrives after session changed
- **WHEN** the user opens a delayed push after the referenced coding state has advanced
- **THEN** the application displays current server state rather than trusting the notification payload
- **AND** does not resurrect a resolved approval or stale session status
