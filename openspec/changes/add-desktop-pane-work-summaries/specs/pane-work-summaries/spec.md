## Purpose

Defines durable, concise summaries of each agent pane's work in fixed three-hour windows and a desktop interface for reviewing that history without rereading the full conversation.

## ADDED Requirements

### Requirement: Activity is partitioned into stable per-pane windows
The system SHALL assign persisted conversation activity to the pane that produced it and to fixed, non-overlapping three-hour UTC windows. A summary SHALL cover exactly one session, one pane, and one window, and SHALL NOT combine activity from sibling panes. Window labels shown to a user SHALL be formatted in the browser's local time while retaining the shared UTC boundaries.

#### Scenario: Two panes work during the same window
- **WHEN** two panes in one session produce conversation activity during the same three-hour interval
- **THEN** the system creates independent source windows for the two panes
- **AND** neither pane's summary includes the other pane's activity

#### Scenario: Users in different time zones view one summary
- **WHEN** authorized users in different browser time zones view the same cached window
- **THEN** they receive the same summary and UTC coverage
- **AND** each browser formats the window range in its own local time

#### Scenario: Terminal pane produces raw output and captured conversation
- **WHEN** a terminal pane produces raw PTY bytes and persisted user or assistant conversation turns
- **THEN** only the persisted conversation activity is eligible for summarization
- **AND** raw terminal scrollback is neither persisted nor included for this capability

### Requirement: Summaries are concise and grounded in retained activity
For every window containing meaningful persisted activity, the system SHALL produce a plain-text account of the work attempted, completed, validated, failed, or left blocked during that window. The summary body SHALL contain 50–100 words when the source contains enough semantic content, SHALL use fewer words rather than inventing detail for a sparse window, and SHALL NOT reproduce secrets, long code excerpts, diffs, terminal output, or large tool payloads.

#### Scenario: Agent completes and validates work
- **WHEN** a source window records an instruction, implementation activity, and successful validation
- **THEN** its summary identifies the requested work, the material outcome, and the validation result
- **AND** stays within the summary length and disclosure bounds

#### Scenario: Agent remains blocked
- **WHEN** a source window ends with an unresolved error, question, approval, or external blocker
- **THEN** its summary states the blocker and the latest known status
- **AND** does not claim that the work completed

#### Scenario: Window has little semantic activity
- **WHEN** a window contains too little meaningful content to support 50 words without repetition or invention
- **THEN** the system emits a shorter grounded summary or omits a status-only window
- **AND** does not pad the result with unsupported claims

### Requirement: Completed and current windows expose freshness
The system SHALL automatically queue a completed meaningful window for summarization and SHALL cache a source digest with the result. A completed summary SHALL be regenerated while its raw source remains available if late activity changes that digest. The current open window MAY be summarized on request, but its result SHALL be marked partial with a source-through timestamp and SHALL NOT be presented as final.

#### Scenario: Three-hour window closes
- **WHEN** a meaningful source window reaches its end boundary
- **THEN** the system queues it for summary generation
- **AND** publishes the completed cached summary when generation succeeds

#### Scenario: Late source activity changes a completed window
- **WHEN** persisted activity is added to a summarized window while that source remains available
- **THEN** the cached source digest no longer matches
- **AND** the system marks the summary stale and regenerates it before presenting it as current

#### Scenario: User requests the current window
- **WHEN** a desktop user opens or refreshes summaries for a pane with activity in the current window
- **THEN** the system may generate a partial summary through the latest included activity timestamp
- **AND** the drawer identifies that summary as in progress rather than completed

### Requirement: Missing retained history is backfilled without blocking live work
When summary generation is introduced or has fallen behind, the system SHALL identify unsummarized meaningful windows whose raw activity is still retained and queue them newest-first. Unavailable history that was already removed by message retention SHALL NOT be reconstructed or represented as summarized.

#### Scenario: Existing pane has seven days of retained history
- **WHEN** the summary capability first becomes available for that pane
- **THEN** the system queues its missing retained windows newest-first
- **AND** exposes completed summaries incrementally as they arrive

#### Scenario: Raw history predates summary support
- **WHEN** a window's raw messages were deleted before any summary was generated
- **THEN** the system does not fabricate a summary for that window
- **AND** the UI may explain that the source history is no longer available

### Requirement: Summary generation is isolated from the working agent
Summary generation SHALL run outside the active pane conversation through a capability-advertising summarizer configured on the project CLI host. The summarizer SHALL start a new non-resumed invocation without repository write access or access to the active terminal; SHALL treat source messages as untrusted data; and SHALL NOT add messages, status changes, or terminal bytes to the pane being summarized. An adapter with a true tool-disable mode SHALL use it. A Codex adapter MAY retain its headless command tool only after explicit operator selection and SHALL use a fresh empty working directory, ephemeral execution, ignored user configuration and rules, a read-only sandbox, structured output, and a prompt that forbids following or executing instructions found in source data. Documentation SHALL disclose that these controls reduce but do not eliminate prompt-injection-driven host reads. The summarizer provider SHALL be independent of the pane provider, and a CLI SHALL execute at most one summary job concurrently.

#### Scenario: Summary is generated while the pane is working
- **WHEN** a summary job runs while the active agent continues its task
- **THEN** the active agent process, conversation, working status, and terminal remain unchanged
- **AND** only the separate summarizer's quota is consumed

#### Scenario: Source text contains instructions for the summarizer
- **WHEN** retained conversation text attempts to make the summarizer run a tool, reveal a secret, or ignore the summary contract
- **THEN** the invocation prompt and source delimiters identify that text only as material to summarize
- **AND** a tool-free adapter cannot execute it, while an explicitly enabled Codex adapter applies its read-only controls and documented residual-risk policy

#### Scenario: Operator explicitly enables Codex summaries
- **WHEN** the host operator selects the Codex summary adapter and enables summary generation
- **THEN** the CLI validates the required headless isolation flags before advertising summary capability
- **AND** runs each job ephemerally in a fresh empty directory without loading user configuration, project rules, a pane session, or repository write access
- **AND** identifies the remaining host-read risk in operator documentation

#### Scenario: No eligible summarizer is configured
- **WHEN** the project CLI has no enabled summarizer adapter that satisfies its provider-specific startup validation
- **THEN** it rejects the job with an actionable unavailable reason
- **AND** the agent pane continues operating normally

### Requirement: Generation work is bounded and failure-tolerant
The system SHALL bound normalized source size, generation time, retry count, and concurrent work. Oversized windows SHALL be reduced in bounded chunks before the final summary. Transient generation failures SHALL use bounded retry with deduplication by session, pane, window, and source digest; permanent failures SHALL remain visible and manually retryable without blocking message delivery or agent execution.

#### Scenario: Window contains high-volume tool activity
- **WHEN** normalized activity exceeds one summarizer request's configured input bound
- **THEN** the system creates bounded intermediate notes and reduces them into one final summary
- **AND** does not send an unbounded payload or silently drop the newest activity

#### Scenario: Provider is temporarily unavailable
- **WHEN** summary generation fails with a retryable provider or transport error
- **THEN** the system records the failure and retries with bounded backoff using the same logical job identity
- **AND** does not create duplicate cached summaries

#### Scenario: Generation fails permanently
- **WHEN** a summary job exhausts its retries or returns invalid output
- **THEN** the desktop drawer shows a failed state and an available retry action
- **AND** normal pane messaging and execution remain available

### Requirement: Summaries follow conversation authorization and lifecycle
The server SHALL authorize every summary list, refresh, and update operation against current project access. Cached summaries SHALL be stored with their session data, SHALL remain available after their raw messages age out, and SHALL be removed with all other project/session information when an owner deletes the project. Closing a pane SHALL NOT by itself delete its historical summaries.

#### Scenario: Authorized user opens summaries
- **WHEN** a current project owner, project user, or cluster administrator requests summaries for an accessible pane
- **THEN** the server returns only summaries from that authorized session and pane

#### Scenario: User loses project access
- **WHEN** a user attempts to list or refresh summaries after leaving the project or having access revoked
- **THEN** the server denies the operation
- **AND** discloses no summary content or generation metadata

#### Scenario: Owner deletes the project
- **WHEN** project deletion completes
- **THEN** every cached summary and summary-generation artifact for that project is deleted
- **AND** no summary is returned through a stale route or connection

#### Scenario: Raw message retention runs
- **WHEN** source messages age out after a completed summary was cached
- **THEN** the cached summary remains readable
- **AND** the system does not require the deleted raw source to serve it

### Requirement: Desktop users can inspect summaries for the active pane
The desktop web interface SHALL show a `Summary` action only for a real active pane and SHALL open a docked side drawer scoped to that pane. The drawer SHALL list the newest window first and show each window's localized range, completed or partial state, source-through time when partial, and generation, stale, unavailable, or failed status. Switching panes while the drawer is open SHALL switch its contents to the newly active pane.

#### Scenario: User opens the summary drawer
- **WHEN** a desktop user activates a pane and selects `Summary`
- **THEN** the conversation remains visible beside a docked summary drawer
- **AND** the drawer requests and renders only that pane's summary windows newest-first

#### Scenario: User switches panes with the drawer open
- **WHEN** the user selects another real pane while the drawer remains open
- **THEN** the drawer replaces its contents with summaries for the newly active pane
- **AND** does not retain cards from the prior pane

#### Scenario: Overview or no project is selected
- **WHEN** the active desktop view is Overview or no project is selected
- **THEN** the pane Summary action is not displayed

#### Scenario: Summary feature is viewed on mobile
- **WHEN** the responsive mobile browser experience or native mobile application is used
- **THEN** neither the Summary action nor the desktop drawer is rendered
- **AND** existing mobile navigation and conversation behavior remain unchanged

### Requirement: Mixed-version deployments degrade explicitly
Summary protocol participants SHALL negotiate a versioned summary capability. A new server and web client SHALL continue normal pane operation with an older CLI, serve any already cached summaries, and identify missing generation support without sending unknown jobs. Malformed or mismatched job results SHALL be rejected without replacing a valid cached summary.

#### Scenario: Project runs an older CLI
- **WHEN** the web requests summaries from a session whose CLI does not advertise the summary capability
- **THEN** the server returns any cached summaries
- **AND** identifies unsummarized windows as requiring a client update rather than dispatching unsupported work

#### Scenario: Result does not match the queued source
- **WHEN** the server receives a result with an unknown job identity or a source digest different from the queued window
- **THEN** it rejects the result
- **AND** retains any previously valid cached summary
