## 1. Secure Transport and Protocol Foundation

- [x] 1.1 Provision a trusted TLS certificate and nginx HTTPS/WSS listeners for `apas.mpaxos.com` while retaining the existing HTTP/WS endpoints during the compatibility window.
- [x] 1.2 Add production configuration guards that reject cleartext mobile API and WebSocket endpoints, with localhost-only development exceptions that cannot enter release builds.
- [x] 1.3 Update the web client's production WebSocket default to WSS and verify existing web, CLI, and daemon clients reconnect through the secure endpoint.
- [x] 1.4 Add Rust JSON Schema export for the public authentication, bootstrap, session, pane, timeline, access-change, mutation, review, and terminal wire DTOs used by clients.
- [x] 1.5 Create `packages/protocol` with a pinned schema-to-TypeScript generation command, generated discriminated unions, runtime validators, and documented regeneration workflow.
- [x] 1.6 Add shared golden fixtures and matching Rust serialization and TypeScript decoding tests for tagged enums, defaults, maps, UUIDs, dates, opaque JSON, terminal frames, and protocol errors.
- [x] 1.7 Extend the WebSocket authenticate handshake additively with client kind, application version, protocol version, and advertised capabilities while preserving older clients.
- [x] 1.8 Implement server protocol bounds and capability negotiation, including a read-only upgrade-required response for incompatible mobile clients.
- [x] 1.9 Add CI checks that regenerate `packages/protocol`, fail on contract drift, and run both sides of the golden-fixture tests.
- [x] 1.10 Add independent server-advertised feature flags for mobile bootstrap, coding mutations, terminal access, notifications, and deep links.

## 2. Mobile Device Authentication

- [x] 2.1 Add SQLite migrations for mobile installations and device sessions with user binding, configurable expiry, revocation metadata, last-use metadata, and indexed keyed refresh-token hashes.
- [x] 2.2 Implement mobile login that creates a revocable device session and returns a short-lived access JWT plus a one-time opaque refresh token only over HTTPS.
- [x] 2.3 Implement atomic refresh-token rotation with previous-token invalidation, concurrent-use handling, reuse detection, and device-session revocation on suspected theft.
- [x] 2.4 Implement mobile logout plus user- and administrator-facing device listing and revocation endpoints, including revocation of associated push tokens.
- [x] 2.5 Invalidate all affected mobile device sessions on account suspension, password reset, and other existing global credential-revocation events.
- [x] 2.6 Apply current account, project membership, role, target session/pane, and lifecycle authorization checks to every mobile HTTP and WebSocket action.
- [x] 2.7 Add endpoint rate limits, structured non-sensitive audit events, token-log redaction, and configurable 15-minute access and 30-day refresh defaults.
- [x] 2.8 Add migration and integration tests for login, expiry, rotation, replay, concurrent refresh, logout, administrative revocation, suspension, password reset, and legacy-client compatibility.

## 3. Mobile API and Application Foundation

- [x] 3.1 Add `/mobile/v1/bootstrap` returning current identity, authorized project/session summaries, eligible launch targets, protocol bounds, and feature flags without inaccessible project data.
- [x] 3.2 Create `packages/mobile` with the project-approved stable Expo SDK, React Native, TypeScript strict mode, New Architecture, Expo Router, and development-build support.
- [x] 3.3 Configure repository workspace scripts, pinned dependencies, linting, formatting, type checking, unit tests, and one-platform CI smoke builds for the mobile package.
- [x] 3.4 Add EAS development, internal-preview, and production profiles with environment separation and an explicit runtime-version policy for compatible updates.
- [x] 3.5 Implement the code-only route tree for login, Code home, Attention, new task, session activity, review, terminal, account, and notification settings with authenticated route guards.
- [x] 3.6 Build reusable native design tokens and accessible components for compact status, attention, event, decision, form, error, offline, and empty states.
- [x] 3.7 Implement responsive phone navigation and a tablet list-detail adaptation that preserve identical authorization and action semantics.
- [x] 3.8 Implement a platform-secure credential vault for refresh/access credentials, installation identity, and cache encryption material, including wipe and recovery behavior.
- [x] 3.9 Add an encrypted Expo SQLite cache with migrations for session summaries, timeline pages, event identities, watermarks, and freshness metadata, with its key held in secure storage.
- [x] 3.10 Configure TanStack Query for HTTP resources and small Zustand slices for auth, connection, session index, active session, attention, and terminal state without importing the web store.
- [x] 3.11 Add tests proving production builds reject insecure endpoints, credentials never enter AsyncStorage or terminal payloads, and logout/revocation wipes protected and cached state.

## 4. Connection Lifecycle and Reconciliation

- [x] 4.1 Implement the offline, connecting, authenticating, synchronizing, and ready connection supervisor with bounded exponential backoff, jitter, heartbeat handling, and observable transitions.
- [x] 4.2 Integrate network and AppState events so backgrounding immediately makes the socket and all mutations unusable instead of assuming background connectivity.
- [x] 4.3 On foreground or network recovery, refresh authentication, open a new WebSocket, negotiate capabilities, refresh bootstrap authorization, and reattach relevant sessions in dependency order.
- [x] 4.4 Persist per-session and per-pane server watermarks only after local acceptance, then request incremental catch-up and deduplicate events by stable identity and ordering key.
- [x] 4.5 Reconcile live events that race bootstrap or catch-up without duplication, reordering accepted server history, or leaving silent gaps.
- [x] 4.6 Handle project-access changes, account revocation, deleted sessions, and incompatible protocol responses by clearing inaccessible state and disabling or removing affected routes and controls.
- [x] 4.7 Enforce explicit offline read-only mode with last-updated labels and hard-disable task launch, steering, decisions, interruption, and terminal input until synchronization reaches ready.
- [x] 4.8 Add fake-network and fake-timer integration tests for initial connect, heartbeat timeout, suspension, token expiry, reconnect storms, catch-up races, authorization loss, and protocol downgrade.

## 5. Code Session Discovery and Activity

- [x] 5.1 Implement Code home queries and live updates for accessible active and recent sessions, including project, machine/instance, activity, attention, and latest-update context.
- [x] 5.2 Add active, attention-required, completed, recent, and project filters with a non-blank zero-session state and an available new-task action.
- [x] 5.3 Define the framework-neutral `CodeEvent` model and adapters for instructions, agent status, tools, questions, approvals, plans, TODOs, tests, diffs, pull requests, terminal lifecycle, completion, interruption, and errors.
- [x] 5.4 Extract or implement shared pure helpers for event identity, ordering, role interpretation, watermarks, attention derivation, and idempotent reducer application in `packages/protocol`.
- [x] 5.5 Build the session activity screen with virtualized incremental rendering, concise grouped events, pagination to older persisted history, current-state emphasis, and expandable retained detail.
- [x] 5.6 Build the Attention screen from unresolved server-authoritative decisions and selected failures, with links to the exact session, pane, and event.
- [x] 5.7 Cache session summaries and accepted timeline pages transactionally so offline screens remain coherent and clearly identify freshness and truncation.
- [x] 5.8 Build mobile review surfaces for plan requests, file-grouped unified diffs, test/error summaries, truncation/errors, and pull-request results with allowlisted external URL opening.
- [x] 5.9 Add unit and component tests for event normalization, ordering, duplicate delivery, pagination, high-volume virtualization, empty states, offline rendering, inaccessible sessions, and narrow-screen source formatting.

## 6. Coding Actions and Decisions

- [x] 6.1 Extend bootstrap and live state with only currently eligible machines, projects/instances, profiles, providers, and modes for new mobile coding tasks.
- [x] 6.2 Add a server task-launch request identifier, durable or retained idempotency result, and acknowledgement that returns the created or attached session without duplicate launches.
- [x] 6.3 Build the new-task composer with target and supported-option selection, non-empty instruction validation, local draft persistence, explicit review, and submission only while ready.
- [x] 6.4 Preserve the draft and display actionable server errors when a target or option becomes unavailable, and safely recover an uncertain submission by reusing the same request identifier.
- [x] 6.5 Implement exact-session-and-pane follow-up steering with client message identifiers, server acknowledgements, deduplication, and convergence across web, CLI, and mobile clients.
- [x] 6.6 Implement question answering and approval/rejection flows that refresh pending state, accept only the first current authorized response, and surface stale-resolution failures.
- [x] 6.7 Implement interruption for eligible running panes with explicit confirmation, server lifecycle reauthorization, acknowledgement, and refreshed activity state.
- [x] 6.8 Add server authorization and idempotency tests plus mobile component and end-to-end tests for launch retry, steering, decision races, stale permissions, interruption, and cross-surface convergence.

## 7. Trusted Mobile Terminal

- [x] 7.1 Extract terminal sequence/instance reconciliation, lifecycle interpretation, and shared theme tokens into framework-neutral tested modules without changing existing web behavior.
- [x] 7.2 Create `packages/terminal-web` as a reproducible bundled local HTML/TypeScript xterm.js asset with fit behavior, canvas rendering, theme support, and no remote application content.
- [x] 7.3 Apply a restrictive content security policy, block navigation and new windows, allowlist explicit external-link handoff, and verify no mobile credential or arbitrary bridge message reaches the WebView.
- [x] 7.4 Define and validate the narrow React Native-to-WebView bridge for reset, snapshot, output, lifecycle, theme, focus, ready, input, resize, paste request, and link request messages.
- [x] 7.5 Batch high-frequency terminal output per animation frame across the bridge and preserve server sequence and instance metadata through every reset, snapshot, and live frame.
- [x] 7.6 Build the native terminal route and accessory controls for Escape, Tab, Control, arrows, common chords, keyboard dismissal, focus, safe areas, and explicit user-confirmed paste.
- [x] 7.7 Implement authorized attach, initial snapshot-before-live reconciliation, truncation reset, process restart separation, valid debounced resize, and exact-pane input routing.
- [x] 7.8 Render running, disconnected, exited, and unknown lifecycle states while retaining final or disconnected scrollback and enabling input only for ready, authorized, running panes.
- [x] 7.9 On background, disconnect, or access loss, disable and discard pending input; on foreground request a fresh snapshot and clear all terminal content if reauthorization fails.
- [x] 7.10 Add reconciliation, bridge-validation, CSP/navigation, clipboard, rotation, IME, access-revocation, and existing-web regression tests.
- [ ] 7.11 Benchmark retained scrollback, truncated snapshots, rapid full-screen TUI repaint, rotation, and background/foreground recovery on representative iOS and Android devices; record thresholds and results.
- [ ] 7.12 If the recorded bridge thresholds fail, implement and security-test the documented one-time, short-lived, session/pane-scoped direct terminal ticket before enabling the terminal feature flag.

## 8. Notifications and Deep Links

- [x] 8.1 Add SQLite migrations for device push tokens, notification preferences, logical events, and per-installation delivery attempts with deduplication keys, retry indexes, and deletion-safe cleanup.
- [x] 8.2 Implement authenticated push-token registration/rotation, notification-preference update, device revocation, logout cleanup, and permanent-invalid-token retirement.
- [x] 8.3 Create notification events for unresolved questions/approvals, selected failures, pull-request readiness, and completion only after checking account state, current project access, preferences, and relevance.
- [x] 8.4 Implement a provider-neutral Rust push transport trait and an Expo Push Service implementation that batches sends, records tickets, polls receipts, retries transient failures with bounded backoff, and recovers idempotently after restart.
- [x] 8.5 Enforce generic default payloads containing only event category and opaque routing identifiers, and add tests rejecting prompts, output, code, diffs, terminals, secrets, and filesystem paths.
- [x] 8.6 Integrate Expo Notifications for permission prompting, token rotation, authenticated registration, foreground presentation, tap handling, logout cleanup, and per-device settings.
- [x] 8.7 Implement `apas://code`, session, and new-task app links plus equivalent `https://apas.mpaxos.com/code/...` routes with validated opaque identifiers and explicit-review prefill behavior.
- [ ] 8.8 Publish and verify Apple association and Android asset-link files from the production domain and configure the native bundle identifiers to match the final signed applications.
- [x] 8.9 Route every notification or deep link through authentication, current bootstrap authorization, catch-up, and safe fallback without directly approving or mutating any coding state.
- [x] 8.10 Add server and mobile tests for duplicate events, retries, receipts, invalid tokens, token rotation, multiple devices, logout/revocation, project deletion, stale notifications, unauthorized links, and missing targets.

## 9. Security, Operations, and Release Decisions

- [x] 9.1 Complete a threat-model review covering refresh-token theft/reuse, WebView isolation, deep-link spoofing, push privacy, offline cache extraction, cross-project authorization, and terminal data leakage; resolve all release-blocking findings.
- [x] 9.2 Add redacted structured logs and operational metrics for mobile authentication, protocol negotiation, reconnect/catch-up, mutation acknowledgements, terminal attach/bridge health, outbox depth, push tickets/receipts, and app versions.
- [x] 9.3 Add bounded-load and restart-recovery tests for mobile WebSocket connections and the SQLite notification outbox, documenting the measured single-process operating envelope.
- [ ] 9.4 Finalize the app display name, Apple bundle identifier, Android application ID, icons, signing ownership, store ownership, and public privacy/support metadata before production signing.
- [ ] 9.5 Run the WebView, secure-storage, encrypted-database, notification, accessibility, and performance matrix on target devices and set documented minimum iOS and Android versions from the results.
- [ ] 9.6 Review measured delivery volume and compliance needs after beta and record the decision to retain Expo Push Service or schedule a direct APNs/FCM migration behind the existing transport trait.
- [x] 9.7 Complete a privacy review before offering any optional lock-screen project-name preview; keep previews unavailable and generic payloads mandatory unless explicitly approved.
- [x] 9.8 Document mobile development, protocol regeneration, EAS/local builds, environment setup, device-session administration, feature flags, push diagnosis, TLS migration, and rollback runbooks.

## 10. Verification and Staged Rollout

- [x] 10.1 Run the complete Rust and web tests, protocol fixture tests, mobile lint/type/unit/component tests, terminal bundle tests, and production builds; resolve all introduced regressions.
- [ ] 10.2 Produce internal iOS and Android development builds and complete Maestro flows for login, zero-session home, resume, new task, reconnect, approval, review, terminal, deep link, notification, and logout.
- [x] 10.3 Verify cross-surface continuity by starting and steering work from mobile while web/CLI are attached, resolving decisions on another client, and confirming exactly-once convergence after suspension.
- [x] 10.4 Verify account suspension, mobile-device revocation, project membership loss, ownership changes, project deletion, and session deletion remove mobile access, cached sensitive data, terminal content, and future notifications.
- [ ] 10.5 Roll out TLS in the documented order—certificate and dual endpoints, client-default updates, WSS verification, HTTP redirects, then HSTS—and prove rollback at each irreversible boundary.
- [ ] 10.6 Enable the read-only mobile app for internal users first, then coding actions, terminal, notifications, and deep links independently after each capability's acceptance checks pass.
- [ ] 10.7 Complete physical-device acceptance on supported iOS and Android phones and tablets, accessibility checks, crash-free internal soak, and app-store privacy/review submissions.
- [ ] 10.8 Progressively enable production users while monitoring authentication, reconnect, crash, terminal, protocol-version, and push-delivery metrics, retaining prior server/web artifacts and prior store builds for rollback.
