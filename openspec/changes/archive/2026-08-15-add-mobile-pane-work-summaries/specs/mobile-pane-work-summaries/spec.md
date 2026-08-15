## Purpose

Defines secure, pane-scoped access to APAS work summaries from responsive mobile browsers and the native iOS and Android application.

## ADDED Requirements

### Requirement: Mobile session views expose summaries for the selected pane
The system SHALL provide a compact work-summary action from an attached mobile session in both the responsive browser and native application. Opening the summary surface SHALL show only records for the currently selected real pane and SHALL identify that pane in the surface.

#### Scenario: User opens summaries for a selected pane
- **WHEN** an authorized user selects a pane in a mobile session and opens work summaries
- **THEN** the system requests and displays summaries for that exact session and pane
- **AND** does not include summary records from sibling panes

#### Scenario: Session has no selectable pane
- **WHEN** an attached mobile session has no real pane selected
- **THEN** the summary action is unavailable
- **AND** the session view continues to render its normal empty-pane experience

#### Scenario: User switches panes while summaries are open
- **WHEN** the user selects another pane while the mobile summary surface remains open
- **THEN** the surface atomically changes to the new pane's cached or loading state
- **AND** never renders the previous pane's cards under the new pane label

### Requirement: Mobile summary cards preserve work-summary meaning and status
The mobile clients SHALL render summary windows newest first using localized time labels and SHALL distinguish complete, partial, queued, generating, stale, failed, source-expired, unavailable, disabled, and client-update-required states. Each rendered record SHALL preserve available freshness, source coverage, and provider metadata without representing the summary as an audit record.

#### Scenario: Completed and current summaries are available
- **WHEN** a pane has completed-window summaries and a partial current-window summary
- **THEN** the mobile surface displays the newest window first
- **AND** labels the partial record with its available through-time or equivalent freshness indicator

#### Scenario: Generation cannot currently proceed
- **WHEN** the server reports a disabled summarizer, unavailable summarizer, incompatible project CLI, failed window, or expired source
- **THEN** the mobile surface explains the applicable state without hiding previously cached summary text
- **AND** offers retry only for a state the server permits the user to retry

#### Scenario: No meaningful activity is summarized
- **WHEN** the selected pane has no summary records
- **THEN** the mobile surface renders an explicit empty state
- **AND** does not render a blank sheet, route, or panel

### Requirement: Online mobile users can refresh or retry exact windows
While online and authorized, a mobile user SHALL be able to refresh the selected pane's current three-hour window and retry a selected failed window. Every request SHALL identify the exact session, pane, and optional window and SHALL remain subject to the server's current access authorization, throttling, deduplication, and generation limits.

#### Scenario: User refreshes current work
- **WHEN** an authorized online user refreshes without selecting a historical window
- **THEN** the server reconciles only the selected pane's current three-hour window
- **AND** the mobile surface reflects queued, generating, or updated state from server responses

#### Scenario: User retries one failed window
- **WHEN** an authorized online user retries a failed summary card
- **THEN** the request targets that card's window start for the selected session and pane
- **AND** does not regenerate completed sibling windows

#### Scenario: Access changed after the summary opened
- **WHEN** a stale mobile client sends a refresh after project access has been revoked
- **THEN** the server rejects the request without disclosing or modifying summary state
- **AND** the client clears inaccessible summary content through its normal access-revocation handling

### Requirement: Summary navigation preserves conversation context
Opening, closing, refreshing, or scrolling the mobile summary surface SHALL NOT replace the selected conversation pane or reset the remembered conversation position. Summary scrolling SHALL be independent from conversation and terminal scrolling.

#### Scenario: User returns from summaries to conversation
- **WHEN** a user opens summaries after scrolling a pane conversation and then closes the summary surface
- **THEN** the same pane remains selected
- **AND** the conversation returns to its remembered position or follow-newest state

#### Scenario: Summary updates arrive while conversation is visible
- **WHEN** a summary update arrives after the user closes the summary surface
- **THEN** the client updates the pane-scoped summary cache without navigating or scrolling the conversation

### Requirement: Mobile continuity is explicit across offline and mixed-version states
The native application SHALL persist bounded summary snapshots for accessible sessions using the existing protected mobile cache boundary and SHALL permit read-only viewing of those snapshots while offline with a visible freshness indicator. Both mobile clients SHALL disable refresh and retry when disconnected or protocol support is unavailable, and SHALL reconcile the selected pane after reconnect without duplicating cards.

#### Scenario: Native user views cached summaries offline
- **WHEN** the native application is offline and has a cached summary snapshot for the selected pane
- **THEN** it displays the cached cards with offline and last-updated context
- **AND** disables refresh and retry controls

#### Scenario: Native cached access is revoked
- **WHEN** logout, account suspension, device-session revocation, or project-access revocation is processed
- **THEN** cached summaries for inaccessible sessions are removed with other sensitive session data
- **AND** they are not available through a stale route

#### Scenario: Mobile client reconnects
- **WHEN** a mobile client reconnects while a summary surface is open
- **THEN** it requests an authoritative snapshot for the selected session and pane
- **AND** merges subsequent window updates by pane and window identity without duplicates

#### Scenario: Summary protocol is unsupported
- **WHEN** the connected server or installed client cannot safely exchange work-summary messages
- **THEN** the summary action is hidden or rendered unavailable
- **AND** ordinary mobile conversation and terminal features continue to operate
