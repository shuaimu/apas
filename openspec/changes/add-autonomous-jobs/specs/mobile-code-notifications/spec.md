## ADDED Requirements

### Requirement: Autonomous job attention uses privacy-safe notification events
The system SHALL integrate actionable worker questions, failures/recovery and draft review with existing notification preferences and durable delivery. Routine successful inspection/wakeup completion SHALL NOT produce completion spam. Payloads SHALL contain only generic text and opaque worker/run/question/review identities, never titles, instructions, code, paths, answers, credentials or raw output. Repeated delivery SHALL not create distinct logical notifications. Recipient access and device validity SHALL be checked before dispatch. Existing finite-job notification links SHALL retain authorized read-only destinations.

#### Scenario: A job needs a human answer
- **WHEN** an authorized recipient has question notifications enabled and a pending job question is persisted
- **THEN** the notification identifies a generic coding decision and its opaque destination
- **AND** lock-screen content does not reveal the question or project content

#### Scenario: Recipient loses access while delivery is queued
- **WHEN** a queued notification's account, device or project access is revoked before delivery
- **THEN** that notification is not sent to the revoked destination
- **AND** previously issued links still require current authorization before opening details

### Requirement: Notification links resolve exact durable job context
Notification links SHALL resolve the exact worker and optional run/question/review through authenticated lookup independently of live session bootstrap; worker-only failures with no execution SHALL open worker detail without inventing a run. Retained finite-job links SHALL resolve their original evidence. Cold start, fresh login and foreground resume SHALL navigate without performing mutations. Inaccessible destinations SHALL show a safe outcome and authorized Workers/Attention navigation rather than another user's data or an arbitrary active session.

#### Scenario: User opens a review notification after worker exit
- **WHEN** an authorized user taps the notification from a cold native app or supported HTTPS link
- **THEN** authentication and current job lookup precede display of the exact retained review
- **AND** no terminal attachment is required

#### Scenario: Question is already answered
- **WHEN** a question link is opened after its provider-confirmed resolution
- **THEN** the client displays the resolved question in its job context
- **AND** does not resubmit a saved answer or manufacture another pending decision

### Requirement: Attention remains available without push permission
Job attention SHALL remain discoverable in both clients through the authorized in-app Attention view and stable web/native destinations when push permission is denied or notifications are disabled. Browser background push is not required for this capability.

#### Scenario: User disables mobile push
- **WHEN** a job later requires an answer or has a reviewable result
- **THEN** the next authorized foreground synchronization includes that item in Attention
- **AND** normal job operation does not depend on receiving a push notification
