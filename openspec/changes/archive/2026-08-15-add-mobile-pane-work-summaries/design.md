## Context

See `proposal.md` for motivation and `specs/mobile-pane-work-summaries/spec.md` for the behavioral contract. The deployed summary service already owns windowing, durable cache records, generation, authorization, refresh throttling, and incremental updates. Its additive `list_pane_work_summaries`, `refresh_pane_work_summary`, `pane_work_summaries`, and `pane_work_summary_updated` messages are already present in the shared Rust types and generated TypeScript mobile protocol.

The responsive browser session uses `MobileSessionActivity` but shares the desktop Zustand store, which already normalizes summaries by `sessionId/paneId` and negotiates `pane_work_summary_v1`. The Expo application has a separate Zustand store and connection runtime; its validator can decode summary responses, but it neither advertises the capability nor consumes those responses. Native offline data lives in an encrypted SQLite cache and is already erased on logout, authentication loss, and inaccessible-session reconciliation.

Both mobile session views remember a selected pane and conversation scroll independently. The summary surface must preserve those mounted views and must not reintroduce an all-pane activity scope.

## Goals / Non-Goals

**Goals:**

- Reuse the authoritative server summary records and generation pipeline without a parallel mobile representation on the wire.
- Give responsive web and native mobile comparable compact behavior while respecting each client's existing state and storage boundaries.
- Keep summary state keyed by exact session and pane, and make reconnect, access loss, and offline state deterministic.
- Avoid remounting or scrolling the underlying conversation when summaries are reviewed.

**Non-Goals:**

- Changing summary prompts, window size, generation provider, backfill, retention, quota, or server-side scheduling.
- Summarizing raw terminal bytes or creating a mobile-only summary.
- Adding summaries to the session-list cards, push notifications, or terminal screen.
- Making native offline cache an unbounded archive of every historical summary.

## Decisions

### 1. Both mobile clients use an overlay tied to the selected pane

Each session view gets a compact `Summary` action beside its existing pane/session controls. It opens a platform-native overlay: a fixed bottom/full-height sheet in responsive web and a React Native modal sheet in Expo. The underlying conversation remains mounted. The overlay receives the selected session, pane ID, and pane label and renders its own scroll container.

On pane change, the open overlay derives a new `sessionId/paneId` key and immediately shows only that key's cached or loading state. Closing it restores the unchanged conversation. A route-based detail screen was rejected because navigating away can trigger list restoration and would require duplicating selected-pane state; inserting cards into the conversation was rejected because summary windows are a review index, not timeline events, and would disturb scroll/order semantics.

### 2. Presentation semantics are shared, implementations remain platform-native

Status labels, availability copy, localized window formatting, retry eligibility, and metadata rules are factored into small pure helpers that can be tested in each package. The actual web and React Native cards remain separate because sharing JSX across DOM and native primitives would add a cross-platform component abstraction larger than this feature.

Cards are newest first and show summary text when present even if regeneration is pending or unavailable. Refresh targets the current window by omitting `window_start`; retry sends the exact failed card's `window_start`. The control is disabled while disconnected, protocol-incompatible, or already loading. The server remains authoritative for throttling and deduplication, so clients do not invent completion state.

### 3. Responsive web reuses the existing desktop summary store

`MobileSessionActivity` reads the existing `paneWorkSummaries` cache and calls `listPaneWorkSummaries`/`refreshPaneWorkSummary`. Opening the sheet and switching the selected pane while open triggers a list request with `include_current: true`. Existing snapshot/update handlers already merge by pane and window, and existing logout/access/project cleanup remains the single browser cleanup path.

The desktop drawer's status/presentation helpers should be extracted or mirrored through a shared DOM summary-card module so desktop and mobile copy cannot drift. The desktop drawer layout and breakpoint behavior remain unchanged. Building a second mobile-only WebSocket cache was rejected because it would create competing state for the same records within one browser connection.

### 4. Native mobile adds a normalized summary slice and advertises the existing capability

The Expo client adds `paneWorkSummaries` keyed by `sessionId:paneId`, containing summaries, availability, loading state, and a local `updatedAt`. Its authentication capabilities include `pane_work_summary_v1`; the negotiated-capability list from `authenticated` is retained so UI support is based on server negotiation rather than merely on app version.

The connection runtime handles full snapshots and incremental updates before generic code-event normalization. Full snapshots replace one pane key; incremental updates upsert by `(window_start, source_digest)`/window identity, sort newest first, and never touch another pane. Reconnection while a sheet is visible issues a new authoritative list request after the supervisor reaches `ready`. Unknown or malformed records remain rejected by the generated protocol validator.

No mobile protocol version bump is required: the message variants and schema are already additive, and explicit capability negotiation gates their use. Adding the list request to the server's protocol-incompatible read-only allowlist makes cached server summaries readable when safe, while refresh stays a mutation and remains blocked for an incompatible mobile protocol.

### 5. Native offline persistence is bounded and follows existing cache erasure

SQLite gains a `pane_work_summary_snapshots` table keyed by `(session_id, pane_id)` with a validated JSON payload and `updated_at`. Every accepted full snapshot or incremental update persists at most the newest 56 windows for that pane (seven days of three-hour windows); the online in-memory response may contain the full server result. This gives useful offline context without turning every phone into a second unbounded archive.

Opening the native sheet hydrates its pane key from SQLite before requesting the server. Offline cards display the persisted timestamp, and refresh/retry are disabled. `removeInaccessibleSessions` deletes matching snapshot rows, and the existing database deletion covers logout/authentication loss. A table of individual summary rows was rejected because the server sends authoritative pane snapshots and the client does not need independent retention or querying per window.

### 6. Existing server authorization remains the security boundary

List and refresh continue through `check_session_access` and the active-project operation guard. The mobile client never treats cached membership, pane metadata, or a stale deep link as authorization. Access-change synchronization removes in-memory and persisted summary keys not present in the new bootstrap before a stale route can display them.

Summary text is ordinary protected project content: it is not included in telemetry, notifications, logs, session-list previews, or terminal WebView messages. The native storage uses the existing encrypted cache key and lifecycle; no credential is exposed to the summary UI.

## Risks / Trade-offs

- **[A sheet can consume most of a phone screen]** → Use a dedicated scroll surface with a clear close affordance and keep the underlying conversation mounted for instant return.
- **[Desktop and mobile status copy can drift]** → Centralize pure web formatting/status helpers and add table-driven native helper tests against the same status set.
- **[Incremental update races with cache hydration]** → Merge by exact pane/window identity and let the first online full snapshot authoritatively replace hydrated data.
- **[Old servers do not negotiate the capability]** → Hide or disable the action and leave conversation/terminal behavior untouched.
- **[Native cached summaries outlive raw server messages]** → Treat them as protected project content, show cache freshness, bound them to 56 windows per pane, and erase them on every existing access/logout cleanup path.
- **[Refresh has no mutation acknowledgement envelope]** → Use the existing returned pane snapshot/update as authoritative feedback and retain a bounded loading/error state; do not claim generation completed locally.

## Migration Plan

1. Add server allowlist coverage and protocol/capability regression tests, then deploy the compatible server first. Existing clients are unaffected.
2. Deploy responsive web with the mobile sheet. It already negotiates the summary capability and can roll back independently to the desktop-only interface.
3. Release the Expo update after native store/cache/runtime/component tests and platform export checks. The first open creates the additive cache table; no destructive data migration is required.
4. Verify on iOS, Android, and narrow browser layouts that pane switching is isolated, conversation position survives open/close, refresh targets only the current window, retry targets one failed window, and access revocation removes cached cards.

Rollback removes the mobile UI and stops new native cache writes. The additive SQLite table may remain inert until logout or a later release; server summary records and desktop behavior are unchanged.
