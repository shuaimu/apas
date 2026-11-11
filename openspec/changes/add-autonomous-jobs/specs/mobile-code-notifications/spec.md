## ADDED Requirements

### Requirement: Autonomous job attention uses privacy-safe notification events
The system SHALL integrate job questions, failures/recovery needs, review-ready results and completion/cancellation with the existing device notification preferences and durable delivery mechanism. Push payloads SHALL contain only generic text and opaque job/run/question/review routing identities, never task titles, instructions, code, paths, answers, credentials or raw worker output. Repeated delivery of the same event SHALL not create distinct logical notifications. Recipient access and device validity SHALL be checked before dispatch.

#### Scenario: A job needs a human answer
- **WHEN** an authorized recipient has question notifications enabled and a pending job question is persisted
- **THEN** the notification identifies a generic coding decision and its opaque destination
- **AND** lock-screen content does not reveal the question or project content

#### Scenario: Recipient loses access while delivery is queued
- **WHEN** a queued notification's account, device or project access is revoked before delivery
- **THEN** that notification is not sent to the revoked destination
- **AND** previously issued links still require current authorization before opening details

### Requirement: Notification links resolve exact durable job context
Job notification links SHALL resolve the exact job and optional run/question/review context through authenticated lookup, independently of live session bootstrap. Link handling SHALL support fresh login, app cold start and foreground resume without performing a job mutation. Stale or inaccessible destinations SHALL show a safe current outcome and authorized Jobs or Attention navigation rather than another user's data or an arbitrary active session.

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
