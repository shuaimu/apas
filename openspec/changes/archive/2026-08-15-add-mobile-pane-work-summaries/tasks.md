## 1. Protocol and Server Compatibility

- [x] 1.1 Classify `list_pane_work_summaries` as a read-only mobile request while keeping `refresh_pane_work_summary` mutation-gated, and extend server allowlist tests.
- [x] 1.2 Add protocol regression coverage proving mobile validation accepts summary snapshots/updates and exact list/refresh request shapes without a protocol-version bump.
- [x] 1.3 Verify list and refresh continue to enforce current session access, active-project guards, and capability negotiation for mobile connections.

## 2. Responsive Mobile Web

- [x] 2.1 Extract reusable DOM summary formatting, status, availability, metadata, and retry-eligibility helpers/cards without changing the desktop drawer behavior.
- [x] 2.2 Build an accessible responsive summary overlay with loading, empty, complete, partial, queued, generating, stale, failed, source-expired, disabled, unavailable, and update-required presentations.
- [x] 2.3 Integrate a selected-pane Summary action into `MobileSessionActivity`, list the active pane with `include_current`, refresh the current window, retry an exact failed window, and disable controls when disconnected or unsupported.
- [x] 2.4 Preserve the mounted conversation, selected pane, and per-pane scroll position across summary open/close and pane switches.
- [x] 2.5 Add web component/store regressions for pane isolation, atomic pane switching, localized newest-first cards, exact refresh/retry payloads, unsupported capability, and unchanged conversation position.

## 3. Native Summary State and Transport

- [x] 3.1 Advertise `pane_work_summary_v1`, retain negotiated server capabilities in the Expo store, and expose an exact summary-support selector.
- [x] 3.2 Add native summary cache state keyed by session/pane with authoritative snapshot replacement, incremental window upsert, newest-first ordering, loading/error timestamps, and inaccessible-session cleanup.
- [x] 3.3 Route `pane_work_summaries` and `pane_work_summary_updated` messages through the native connection runtime before generic event normalization, without leaking them into the activity timeline.
- [x] 3.4 Re-request the visible summary pane after the supervisor becomes ready following reconnect, while preventing duplicate cards and cross-pane updates.
- [x] 3.5 Add store/runtime/supervisor tests for capability negotiation, snapshot/update races, pane isolation, reconnect reconciliation, protocol incompatibility, and access revocation.

## 4. Native Offline Persistence

- [x] 4.1 Add the encrypted SQLite `pane_work_summary_snapshots` table and typed read/write helpers that retain at most the newest 56 windows per session/pane.
- [x] 4.2 Hydrate a pane snapshot before its online list request, persist accepted snapshots/updates, and expose cached `updated_at` for offline freshness text.
- [x] 4.3 Extend inaccessible-session removal and cache-wipe coverage to summary snapshots, including logout, authentication loss, account suspension, and project-access loss paths.
- [x] 4.4 Add storage tests for bounded retention, replacement/upsert behavior, malformed-cache rejection, pane isolation, and complete erasure.

## 5. Native Mobile Experience

- [x] 5.1 Build testable native summary formatting/state helpers and compact cards covering every summary status, availability state, freshness field, provider label, and retry rule.
- [x] 5.2 Add a React Native summary modal sheet tied to the selected pane, with independent scrolling, explicit empty/loading/offline states, and a clear close affordance.
- [x] 5.3 Wire native list, current-window refresh, and exact failed-window retry requests; disable network controls while offline, synchronizing, mutation-incompatible, unsupported, or already loading.
- [x] 5.4 Keep the conversation list mounted and prove selected-pane and scroll restoration remain unchanged after opening, scrolling, refreshing, switching, and closing summaries.
- [x] 5.5 Add native component tests for phone-sized layout, pane switching without sibling leakage, offline cached viewing, reconnect refresh, exact request payloads, and stale-route access removal.

## 6. Documentation and Verification

- [x] 6.1 Update user/operator documentation to describe mobile summary access, three-hour semantics, current-window refresh, retry behavior, provider availability, offline cache bounds, and summary limitations.
- [x] 6.2 Run Rust formatting and affected shared/server tests plus protocol schema generation and fixture validation.
- [x] 6.3 Run responsive web lint, focused and full tests, production build, and both supported web dependency audits.
- [x] 6.4 Run Expo lint, TypeScript checks, Jest tests, dependency audit, and Android/iOS export or equivalent release-build validation available in the workspace.
- [x] 6.5 Perform narrow-browser, Android, and iOS acceptance checks for pane isolation, status rendering, refresh/retry, offline behavior, reconnect, access revocation, and preserved conversation position.
