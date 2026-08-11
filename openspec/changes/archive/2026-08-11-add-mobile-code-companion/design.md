## Context

See `proposal.md` for motivation and scope. APAS currently has a Next.js 16/React 19 web client, a Rust/Axum server, SQLite persistence, and long-lived JSON WebSocket protocols connecting web, CLI, and daemon clients. The web client already implements authentication, multi-session state, reconnect catch-up, idempotent user input, project authorization events, terminal snapshots, and terminal sequence reconciliation, but most TypeScript wire types and behavior live inside a roughly 5,000-line Zustand store and DOM-specific components.

The mobile client must tolerate operating-system suspension, intermittent networks, a narrow touch interface, and lock-screen privacy. Production currently permits cleartext HTTP/WS and the existing browser JWT is stored in local storage; neither is an acceptable mobile foundation. The first mobile release must coexist with older web/CLI/daemon clients and must not require a new cloud execution model—agents continue running through existing APAS machines, projects, CLI processes, and daemons.

The capability contracts are in `specs/mobile-code-sessions/spec.md`, `specs/mobile-terminal-access/spec.md`, and `specs/mobile-code-notifications/spec.md`.

## Goals / Non-Goals

**Goals:**

- Ship one maintainable TypeScript application for iOS and Android with native navigation, mobile interaction patterns, secure device identity, push notifications, and app/universal links.
- Reuse APAS's server, runtime, authorization, project identity, persisted history, reconnect protocol, and terminal semantics.
- Make starting, monitoring, steering, approving, and reviewing concurrent coding sessions the primary mobile loop.
- Establish a generated, versioned TypeScript wire contract and reusable framework-neutral domain logic for both web and mobile.
- Make app suspension and reconnect a first-class state machine rather than assuming a browser socket remains alive.
- Preserve terminal correctness and security without rewriting a terminal emulator natively.
- Deliver vertical increments that remain deployable and rollbackable independently.

**Non-Goals:**

- Sharing React DOM/Tailwind components directly with React Native.
- Replacing the desktop pane grid, full file editor, local shell, or deep terminal workflow.
- Running agent processes, source checkouts, or arbitrary code on the phone.
- Keeping a WebSocket alive indefinitely in the background.
- General-purpose AI chat, voice, image/file chat, or non-coding conversations.
- Full cluster-administrator parity in the first release.
- Horizontal server scaling or replacing SQLite as part of the mobile launch.

## Decisions

### 1. Build a native-first Expo application in the existing repository

Create `packages/mobile` with the latest project-approved stable Expo SDK, React Native, TypeScript, the React Native New Architecture, Expo Router, and development builds rather than Expo Go. Use EAS profiles for development, internal preview, and production builds; native dependency or permission changes require a new binary, while compatible JavaScript-only fixes may use EAS Update with an explicit runtime-version policy.

The route model is code-specific:

```text
/(auth)/login
/(code)/index                         session home
/(code)/attention                     unresolved decisions and failures
/(code)/new                           coding-task composer
/(code)/session/[sessionId]           activity timeline
/(code)/session/[sessionId]/review    plan, diff, tests, PR
/(code)/session/[sessionId]/terminal/[paneId]
/(settings)/account
/(settings)/notifications
```

Phone navigation uses a session stack and bottom-level Code/Attention entry points. Tablets may use a list-detail split while retaining the same routes and actions.

**Alternatives considered:**

- **Capacitor around the current Next.js UI:** fastest initial wrapper and highest visual reuse, but preserves desktop layout, browser credential storage, and the monolithic web store. It does not produce the intended mobile Code experience.
- **Flutter:** strong cross-platform rendering but duplicates TypeScript domain/protocol work and still needs a terminal strategy.
- **Separate Swift and Kotlin clients:** maximum platform control at unacceptable implementation and protocol-maintenance cost for the current team.

### 2. Share contracts and domain logic, not UI components

Add a framework-neutral workspace package, `packages/protocol`, generated from the canonical Rust `shared` message model through a checked-in generation command. Rust exports JSON Schema for selected public wire messages and DTOs; a pinned TypeScript generator produces discriminated unions, UUID/date aliases, and validators. Generated output is committed so mobile builds do not require Rust, and CI fails when regeneration changes the tree.

Add cross-language golden fixtures for authentication, session/pane lists, timeline events, access changes, terminal frames, questions/approvals, diffs, PR results, and protocol errors. Rust serialization tests and TypeScript parsing tests consume the same fixtures. A protocol handshake carries `client_kind`, application version, protocol version, and capabilities; new fields remain optional/defaulted for existing clients. An incompatible mobile client becomes read-only with an upgrade message.

Extract only pure transformations—timeline normalization, event identity, watermark comparison, role interpretation, and terminal reconciliation—from the web client. React hooks, local storage, browser sockets, navigation, and DOM rendering remain client-specific.

**Alternative considered:** manually duplicating the Rust enums in a second TypeScript store. The current web duplication already makes drift easy; mobile would multiply that risk.

### 3. Keep the existing API topology and add narrowly scoped mobile foundations

Continue using the existing Axum server and `/ws/web` live protocol rather than adding GraphQL or a separate mobile BFF. Extend the authenticate frame additively with client/protocol metadata. Existing HTTP authentication and project routes remain usable; add versioned HTTP endpoints only where mobile needs durable request/response semantics:

- `/auth/mobile/login`, `/auth/mobile/refresh`, `/auth/mobile/logout`
- `/mobile/v1/devices` and `/mobile/v1/devices/:id/revoke`
- `/mobile/v1/push-token` and `/mobile/v1/notification-preferences`
- `/mobile/v1/bootstrap` for current identity, accessible project/session summaries, protocol bounds, and feature flags

Live session attachment, messages, pane state, input, approvals, signals, diffs, and terminals continue through the shared WebSocket protocol. Add client request identifiers and acknowledgements to task-launch and any mutation that currently lacks idempotency. The server remains authoritative and checks membership, role, account state, lifecycle, and target session/pane on every action.

TanStack Query owns HTTP resources and invalidation. A small Zustand store composed of `auth`, `connection`, `session-index`, `active-session`, `attention`, and `terminal` slices owns live state. Do not port the web store wholesale.

### 4. Introduce revocable mobile device sessions

Keep current web and CLI authentication compatible. Mobile login returns a short-lived access JWT plus an opaque rotating refresh token bound to a device-session row. Store only a keyed hash of the refresh token server-side. Rotation invalidates the previous token and detects token reuse; logout, account suspension, password reset, or administrator revocation invalidates the device session and associated push tokens. Durations are configurable, with initial defaults of 15 minutes for access and 30 days for refresh.

Store the refresh token, current access token, local-cache encryption key, and installation identifier with platform secure storage. Never inject them into terminal HTML. Avoid logging tokens, notification payload details, prompts, source, diffs, terminal bytes, or filesystem paths.

Production mobile configuration accepts only HTTPS/WSS. Before beta, provision a trusted TLS certificate for `apas.mpaxos.com`, redirect HTTP to HTTPS, add HSTS after verification, update the web default to WSS, and preserve a documented rollback. Development builds may allow explicit localhost exceptions that cannot enter production configuration.

### 5. Treat connectivity as a lifecycle-aware state machine

Implement one application-owned connection supervisor:

```text
offline ─▶ connecting ─▶ authenticating ─▶ synchronizing ─▶ ready
  ▲             │              │                 │            │
  └─────────────┴──────────────┴─────────────────┴────────────┘
                    close / timeout / background / auth loss
```

On background or inactivity, mark the socket unusable immediately; do not promise continuous background transport. On foreground, obtain a valid access token, open a fresh socket, negotiate capabilities, refresh bootstrap authorization, reattach the visible session, then fetch per-pane/timeline catch-up from the last committed server watermarks. Only enter `ready` after reconciliation. Existing heartbeat, terminal sequence, `client_msg_id`, and project-access-change semantics are reused.

Persist summaries, timeline pages, event identities, and watermarks in an encrypted Expo SQLite database whose key is held in secure storage. Cached screens are read-only offline. Mutations—including task launch, approvals, interrupts, and terminal input—require `ready`; destructive or time-sensitive actions are never queued. A task-launch draft may be saved locally, but submission requires a fresh server acknowledgement.

### 6. Model the mobile UI as a coding activity stream

Create a normalized `CodeEvent` view model with a stable event identity, session/pane target, server ordering key, timestamp, attention state, concise summary, and optional detail reference. Adapters translate existing persisted messages and live protocol variants into events for:

- user coding instructions and acknowledged follow-ups
- agent phase/status and streamed result summaries
- tool activity grouped by operation
- questions and approval requests
- plan review and team TODO changes
- test/build outcomes and errors
- diff availability and PR creation/status
- terminal lifecycle
- completion or interruption

Use virtualized lists, pagination, collapsed detail by default, and an explicit verbose expansion rather than rendering the desktop transcript. Attention state is derived from unresolved server-authoritative items, never notification payloads. New-task composition selects only server-reported eligible machines, projects/instances, profiles, providers, and modes; the draft survives a rejected or uncertain submission.

### 7. Host xterm.js in a locked-down local WebView

Create `packages/terminal-web` as a small bundled HTML/TypeScript asset using the same xterm.js major version, theme tokens, fit behavior, and terminal reconciliation state as web. Package the generated assets inside the native application; do not load the production website or arbitrary remote HTML.

React Native owns authentication and the single APAS WebSocket. The WebView receives a narrow validated bridge protocol (`reset`, `snapshot`, `write`, `lifecycle`, `theme`, `focus`) and returns only (`ready`, `input`, `resize`, `paste-request`, `open-link`). Batch high-frequency output per animation frame before crossing the bridge. Initially use the canvas renderer because WebGL availability and context loss differ across WKWebView and Android WebView; benchmark before enabling WebGL selectively.

Apply a restrictive content-security policy, disable arbitrary navigation and new windows, allowlist any explicit external link before handing it to the operating system, and never send credentials across the bridge. Provide a native accessory row for Escape, Tab, Control, arrows, common chords, and keyboard dismissal; require an explicit gesture before reading/pasting clipboard text. Rotation and safe-area changes produce non-zero debounced terminal sizes.

If real-device benchmarks show the React Native/WebView bridge cannot sustain representative full-screen TUI output, the planned fallback is a terminal-only direct WebSocket using a one-time, short-lived, session/pane-scoped ticket. The long-lived mobile credential remains outside the WebView.

### 8. Deliver push through a durable privacy-safe outbox

Persist mobile installation, device session, push token, preferences, logical notification event, and per-device delivery attempt records. Authorization-changing tables remain canonical; push records reference users/projects/sessions with deletion-safe cleanup. A unique logical event key deduplicates each event/installation combination.

The server creates notification events only for unresolved questions/approvals, selected failures, PR-ready outcomes, and completion according to user/device preferences. A background worker in the existing server process sends batches to Expo Push Service over HTTPS, records tickets, polls receipts, retries transient failures with bounded backoff, and revokes permanently invalid tokens. SQLite outbox state makes restart recovery idempotent. Provider transport is behind a Rust trait so direct APNs/FCM can replace Expo without changing product behavior.

Default payload text is generic and contains opaque project/session/pane/event identifiers but no prompt, output, source, diff, terminal, secret, or path. Tapping a push authenticates, fetches current authorization/state, and then routes; notification actions never approve or mutate directly.

### 9. Use verified code deep links and universal links

Support `apas://code`, `apas://code/session/<id>`, and `apas://code/new` plus HTTPS equivalents under `https://apas.mpaxos.com/code/...`. New-task links may prefill an instruction and eligible project/machine/profile identifiers but always require review and submission. Configure Apple association and Android asset-link files from the production domain. Unknown, deleted, or unauthorized targets fall back to the nearest authorized Code screen.

### 10. Test the protocol and lifecycle boundaries, not only screens

Use Rust unit/integration tests for device-session rotation/reuse, revocation, schema generation, WebSocket compatibility, mutation authorization, notification outbox/receipts, deletion cleanup, and deep-link bootstrap data. Use TypeScript unit tests for generated decoding, event normalization, reducer idempotency, watermarks, and terminal reconciliation. Use React Native Testing Library for screens and state transitions, fake-timer/network integration tests for the connection supervisor, and Maestro flows on iOS/Android for login, new task, reconnect, approval, deep link, logout, and notification handling.

Run real-device terminal benchmarks with retained snapshots, truncated buffers, rapid repaint TUIs, rotation, background/foreground, paste, IME input, and access revocation. CI gates merges on Rust/web regressions, mobile type/lint/unit tests, protocol-generation cleanliness, and at least one platform smoke build; release candidates require both platform builds and physical-device checks.

## Risks / Trade-offs

- **[Scope spans transport, auth, UI, terminal, notifications, and deployment]** → Deliver vertical phases behind server-advertised feature flags; do not wait for terminal or push before testing read-only sessions internally.
- **[A generated contract may not express every Serde edge case]** → Limit export to public client DTOs, add shared golden fixtures, and retain Rust/TypeScript round-trip tests for tagged enums, defaults, maps, UUIDs, and opaque JSON.
- **[Mobile operating systems suspend sockets unpredictably]** → Never depend on background WebSockets; use push as a hint and foreground reauthentication/catch-up as the correctness path.
- **[WebView bridge throughput may stutter for full-screen TUIs]** → Batch output, benchmark representative loads, start with canvas rendering, and retain the scoped direct-ticket fallback.
- **[Encrypted local caches increase native build complexity]** → Keep the cache schema narrow, hold its key in secure storage, test wipe/rekey paths, and allow cache disablement if a platform build cannot meet the security boundary.
- **[Refresh-token support expands authentication attack surface]** → Store only hashes, rotate on every use, detect reuse, bind tokens to device sessions, rate-limit endpoints, and add revocation/audit coverage before mobile beta.
- **[Notification providers are best-effort and can deliver late]** → Treat pushes as non-authoritative hints, recheck access/state on open, deduplicate logical events, and monitor receipts.
- **[EAS/Expo services add vendor dependency]** → Keep native source and configuration reproducible in-repo, support local EAS builds, abstract push transport, and use runtime-version compatibility for updates.
- **[Cleartext-to-TLS migration can disrupt existing clients]** → Deploy certificate and dual endpoints first, update defaults and clients, verify WSS reconnects, then enforce redirects/HSTS in a later reversible step.
- **[App-store review may scrutinize remote development control]** → Make it explicit that execution occurs on user-authorized APAS machines, require authentication for every action, and do not download or execute code on-device.
- **[Single-process SQLite outbox limits future notification scale]** → Use bounded batches and indexed idempotent rows now; defer a separate queue until measured delivery volume requires it.

## Migration Plan

1. **Protocol and secure transport foundation:** add generated contract tooling and fixtures; provision HTTPS/WSS while retaining compatible HTTP/WS during migration; add protocol handshake fields and capability flags without changing existing client behavior.
2. **Mobile device authentication:** migrate device-session tables, ship mobile login/refresh/logout/revoke endpoints, complete security/rate-limit tests, and keep the feature disabled until TLS verification succeeds.
3. **Read-only internal app:** scaffold Expo/EAS, secure storage, encrypted cache, routing, bootstrap, connection supervisor, session home, and activity timeline; distribute internal iOS/Android builds behind a server flag.
4. **Coding control:** add idempotent new-task creation, steering, questions/approvals, interrupts, diff/plan/PR review, and authorization/reconnect tests.
5. **Terminal:** ship the local terminal asset and locked-down bridge, run real-device performance/security tests, then enable it independently by capability flag.
6. **Notifications and links:** add the durable outbox, device preferences, Expo delivery/receipt processing, privacy review, universal/app association files, and deep-link flows.
7. **Beta and production:** run existing Rust/web regression suites plus both mobile platform suites, test account/project revocation and deletion cleanup, complete store privacy metadata, progressively enable users, and monitor authentication, reconnect, crash, terminal, and push metrics.

Each phase has an independent server feature flag. Rollback disables the affected capability and mobile route first, then restores the previous server/web/nginx artifacts if needed. Additive database tables may remain during rollback; destructive schema removal is deferred until after stable adoption. Mobile binary rollback uses the prior store build, while JavaScript updates are republished only within a compatible runtime version.

## Open Questions

- Final app display name, bundle identifier/application ID, icons, and store-listing ownership.
- Minimum supported iOS and Android versions after the first WebView, secure-store, and notification device matrix is measured.
- Whether production push remains on Expo Push Service or moves to direct APNs/FCM after real delivery volume and compliance requirements are known.
- Whether optional lock-screen project-name previews are offered after privacy review; the default remains generic regardless.
