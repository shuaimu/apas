## Why

Pane work summaries are currently available only in the desktop workspace, so users checking a long-running agent from a phone must reread the full conversation to understand recent progress. The server already produces durable, pane-scoped three-hour summaries; exposing that same state on mobile makes it useful where concise review matters most.

## What Changes

- Add a compact work-summary surface to both the responsive mobile browser session view and the Expo iOS/Android session view.
- Scope summaries to the currently selected pane and preserve the existing newest-first three-hour window ordering, localized time labels, freshness metadata, availability states, and failure states.
- Let an online authorized mobile user refresh the current window or retry one failed window, using the existing server authorization, throttling, and generation pipeline.
- Keep summary review separate from the conversation timeline so opening it does not disturb the remembered pane or conversation scroll position.
- Cache native-mobile summary snapshots for read-only offline viewing with visible freshness, while disabling refresh/retry until the connection and authorization are restored.
- Reuse the existing additive work-summary WebSocket messages and records; no summary source, retention, provider, or generation semantics change.

## Capabilities

### New Capabilities

- `mobile-pane-work-summaries`: Defines pane-scoped work-summary discovery, rendering, refresh/retry, offline behavior, and continuity for responsive web and native mobile clients.

### Modified Capabilities

None.

## Impact

- The responsive Next.js mobile session activity component gains a phone-sized summary sheet/panel backed by the existing Zustand summary state and requests.
- The Expo application gains summary state handling, native summary cards/sheet UI, persisted snapshot caching, reconnect refresh, and related tests.
- The generated mobile protocol already includes the existing summary request/response types; implementation must confirm the mobile validator and authenticated server route accept them without weakening session-access checks.
- No new model calls, dependencies, database schema, raw transcript retention, or desktop behavior are introduced.
