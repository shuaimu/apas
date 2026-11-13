## Purpose

Preserve accessible bounded-execution questions, evidence and earlier finite results in phone-browser and native clients. After persistent-worker cutover, `autonomous-worker-surfaces` owns creation, approval, recurring controls and Workers/Attention navigation; bounded job/run views are reached through an exact worker execution or a read-only earlier-result link.

## ADDED Requirements

### Requirement: Both mobile surfaces expose the complete job workflow
Responsive web and native mobile SHALL each expose the persistent worker workflow and exact bounded-execution status, questions, limits, recovery, verification/diff review and draft-PR handoff. Earlier finite results SHALL remain read-only. New standalone-job composition/admission/retry SHALL be retired in favor of reviewed worker approval. Ordinary worker/run actions SHALL NOT require a terminal, desktop-only page or native WebView wrapper. Existing interactive sessions SHALL remain visibly distinct. Missing authority or readiness SHALL produce a reason rather than a misleading enabled action.

#### Scenario: User performs a job entirely from a phone
- **WHEN** an authorized user creates a worker or monitors, answers, stops or reviews its bounded execution on either mobile surface
- **THEN** every required action and its confirmed result are available in that surface
- **AND** the user is never directed to the raw terminal to complete an ordinary job decision

#### Scenario: Account can read jobs but cannot launch them
- **WHEN** a project user opens Workers or earlier results without unattended admission authority
- **THEN** authorized job progress and review evidence remain readable
- **AND** creation and control actions accurately explain their missing authority without exposing unauthorized machine inventory

### Requirement: Responsive web integrates autonomous work into each project
The responsive-web project's add control SHALL offer an interactive agent or autonomous worker. Interactive creation SHALL retain existing provider/worktree behavior. Worker creation SHALL keep the selected project fixed, prefer its current machine only when eligible and no saved or explicit choice exists, and require reviewed mandate/recurring approval. Missing authority, policy, worker readiness or connectivity SHALL be explained without silently changing target or substituting an interactive pane.

#### Scenario: User creates autonomous work from a project
- **WHEN** the user selects Autonomous worker from the project's add control
- **THEN** mandate composition and approval review remain in that project with its identity fixed
- **AND** an accepted or recovered receipt opens that exact worker without changing the selected session or creating a pane

#### Scenario: User switches between interactive and autonomous work
- **WHEN** the user selects an autonomous job tab and returns to an interactive tab
- **THEN** the job exposes its status, questions, usage, cancellation and review in the project workspace
- **AND** the interactive terminal remains mounted and is not restarted or reattached merely because the visible view changed

#### Scenario: User hides a job or opens completed results
- **WHEN** the user hides an autonomous tab
- **THEN** only the account/project-scoped view preference changes, with no cancel, pane-remove or runtime mutation
- **AND** the project Overview can reopen that job and completed results through authorized bounded pagination

#### Scenario: Project list pages and account context change
- **WHEN** a project has jobs beyond the global inbox's first page, or the user switches projects/accounts while a read is outstanding
- **THEN** the project Overview is filtered and authorized before pagination
- **AND** stale results, cached job views and hidden-tab preferences cannot leak into another account or project


### Requirement: Every web project exposes an autonomous-agent overview
Desktop and mobile web SHALL provide one visible project Overview entry, independent of retired team flags and worker readiness. It SHALL list persistent workers with full descriptions, canonical status, host and next check, and retain separately identified earlier finite results through bounded project-scoped pages. Hidden and completed historical views SHALL remain reachable. Opening a worker or exact earlier result SHALL not change session/pane identity, mutate runtime state or unmount an interactive terminal.

#### Scenario: User surveys current and retained autonomous work
- **WHEN** an authorized project user opens Overview
- **THEN** returned workers and earlier results expose their full description and current or retained status, including hidden views and ended executions
- **AND** descriptions beyond the shortened title remain available in place, with older jobs accessible through bounded project-scoped pages
- **AND** the page does not infer whole-project counts or percentage completion from a partial list

#### Scenario: Overview is empty or cannot synchronize
- **WHEN** a project has no jobs, the client is offline, or current project access is revoked
- **THEN** an empty authoritative list is distinct from loading or stale cached data
- **AND** cached statuses are visibly stale, missing cached descriptions are not fabricated from titles, and discovered revocation removes affected job content

### Requirement: Task composition persists drafts before uncertain network work
Worker composition SHALL persist account/project-scoped mandate/checklist, target/profile, cadence and finite per-run/rolling allowance drafts before submission, then present explicit recurring approval. A retained finite-job instruction MAY prefill an unapproved draft; no bot conversion is offered. Navigation, reload, suspension or failed requests SHALL preserve drafts and original mutation identities. Storage failure SHALL be shown before submission, and timeout SHALL remain unconfirmed rather than being treated as acceptance or replayed automatically.

#### Scenario: App closes during admission
- **WHEN** the server commits worker approval but the client closes before receiving its reply
- **THEN** the next authenticated launch resolves the saved approval request before offering another submission
- **AND** the recovered receipt opens the original worker without duplicate execution

#### Scenario: Selected target becomes unavailable
- **WHEN** current readiness or authority invalidates the admission preview
- **THEN** the client retains the draft, presents the current reason and requests a fresh review
- **AND** does not silently change the target, model, base or allowance

#### Scenario: User edits while offline
- **WHEN** the client has no authenticated synchronized connection
- **THEN** local draft editing and authorized cached reads remain available with a stale/offline indicator
- **AND** admission and other mutations remain disabled without automatic replay on reconnect

### Requirement: Job detail presents authoritative progress and honest limits
Job detail SHALL identify the project, job/run, target, current phase, last confirmed update, cumulative allowance and actionable attention state. Known usage SHALL be separated from unknown or estimated usage; missing cost SHALL NOT display as zero, and reported-spend thresholds SHALL disclose in-flight overrun risk. Progress SHALL use actual phases and bounded event rendering rather than invented percentage completion. Cancellation requested SHALL remain distinct from confirmed stopped.

#### Scenario: A long-running job delegates work
- **WHEN** the worker reports root and child activity
- **THEN** the detail view gives a concise stable timeline with expandable bounded details and aggregate usage
- **AND** a root reply alone does not show the job as finished while delegated work remains

#### Scenario: User cancels a disconnected job
- **WHEN** the cancellation request is durably accepted but host termination is unconfirmed
- **THEN** the view shows cancelling or recovery-required with its latest evidence
- **AND** does not replace the job with a successful cancellation toast or remove retained output

### Requirement: Question forms preserve all choices and confirmed outcomes
Both clients SHALL render all questions in a dialog, stable option identities, single/multiple choice, descriptions and custom text. Answer drafts SHALL survive ordinary navigation/restart within their account/job/run/question scope. Submitted answers SHALL remain visibly pending until provider confirmation; answered-elsewhere, stale-run, cancelled and uncertain-delivery states SHALL be explicit. Neither default selection, timeout, push tap nor reconnect SHALL submit an answer automatically.

#### Scenario: Dialog includes multiple questions and custom text
- **WHEN** the user answers all required fields and submits
- **THEN** the exact option identities and custom text reach the intended pending dialog
- **AND** both clients display the answer actually confirmed by the provider, not merely the locally clicked labels

#### Scenario: Question was resolved on another device
- **WHEN** a suspended client returns to a question answered elsewhere
- **THEN** catch-up displays the confirmed outcome before enabling mutation controls
- **AND** the saved draft is not replayed into that question or a newer dialog

### Requirement: Review is usable and complete on a narrow screen
The mobile review surface SHALL provide the admitted task/base, candidate identity, changed-file list, paginated per-file unified diff, check outcomes/log excerpts, failure reasons and the draft PR link. Truncated, unavailable, binary or expired content SHALL be explicit. Review SHALL remain available after execution ends and SHALL NOT imply a GitHub approval or merge. External PR navigation SHALL require user action and an allowed repository URL.

#### Scenario: Large multi-file candidate is reviewed on a phone
- **WHEN** a user opens the result at a narrow viewport
- **THEN** file navigation, line changes and verification outcomes remain accessible without overflowing the page or rendering the entire artifact at once
- **AND** wide source lines scroll only within the code region

#### Scenario: Verification failed before publication
- **WHEN** a job has a candidate and failing check evidence but no PR
- **THEN** mobile review still exposes the candidate and failed checks
- **AND** does not present a dead PR button or hide the result behind a desktop-only workflow

### Requirement: Phone layouts support touch keyboard zoom and assistive technology
Web layouts SHALL work at 320 CSS pixels and native layouts at 320 density-independent pixels without page-level horizontal overflow. Primary controls SHALL provide at least 44-by-44 CSS-pixel web targets and 48-by-48 density-independent-pixel native targets. Safe areas, keyboard occlusion, orientation, browser zoom, native large text, focus order and screen-reader labels SHALL be supported. Status SHALL not rely on color alone. Submit, answer and cancel actions SHALL remain reachable when the keyboard is open.

#### Scenario: User writes an answer with the keyboard open
- **WHEN** the phone keyboard covers the bottom portion of either client
- **THEN** focused input and the relevant submit control remain reachable above the obscured region
- **AND** safe-area padding and scrolling do not trap the action below a fixed toolbar

#### Scenario: User enlarges text or uses a screen reader
- **WHEN** web zoom, native accessibility text size, VoiceOver or TalkBack is enabled
- **THEN** job state, questions, validation errors and action outcomes remain named and operable in logical order
- **AND** enlarged content does not obscure or overlap the only available action

### Requirement: Reconnect is authorization and state synchronization before control
Returning clients SHALL reauthenticate, reauthorize the selected job, establish its snapshot/watermark and repair event gaps before enabling mutations. Live-connection status alone SHALL NOT mean a job is synchronized. Accepted events and their durable cursor SHALL be committed together. Cached job state SHALL be scoped by account and project, and logout or discovered revocation SHALL clear affected memory, persisted content, drafts and download/query caches.

#### Scenario: Socket reconnects before replay finishes
- **WHEN** transport is connected but the selected job has not completed authoritative catch-up
- **THEN** the view remains visibly synchronizing and read-only
- **AND** it does not answer or cancel from a stale snapshot

#### Scenario: User switches accounts on a shared phone
- **WHEN** the first account logs out and another account signs in
- **THEN** the second account cannot see the first account's job drafts, questions, diffs or retained identifiers through UI or cache restoration

### Requirement: Job destinations are stable and authorization-aware
Both surfaces SHALL support stable job, question and review destinations using opaque identities. Opening a destination SHALL refresh current authority and resolve the exact target, including jobs without a live coding session. A completed or inaccessible target SHALL produce the appropriate current detail or safe explanation, never navigation to an unrelated session. Navigating SHALL not perform a mutation.

#### Scenario: Old question link opens after completion
- **WHEN** a user follows a retained question notification or URL
- **THEN** the client opens the corresponding authorized job and resolved-question context
- **AND** does not submit an answer, start another run or attach whichever session is currently active
