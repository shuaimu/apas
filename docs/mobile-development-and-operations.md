# Mobile Code development and operations

The mobile client lives in `packages/mobile`; generated wire contracts live in
`packages/protocol`, and the bundled xterm renderer lives in
`packages/terminal-web`. It is an Expo SDK 56 / React Native / TypeScript app
that controls APAS coding sessions. It does not execute project code locally.

## Local setup and checks

Install the three pinned workspaces from the repository root:

```bash
npm run mobile:install
```

Copy `packages/mobile/.env.example` for local values. Production and EAS
profiles use `https://apas.mpaxos.com` and `wss://apas.mpaxos.com`; production
configuration fails if either endpoint is cleartext. A local server may use
explicit localhost endpoints only in a non-production build and only when the
server's `mobile.allow_insecure_localhost` option is enabled.

Use a development build, not Expo Go, because SecureStore, SQLCipher,
notifications, and the bundled terminal require native configuration:

```bash
npm --prefix packages/mobile start
npm --prefix packages/mobile run android
npm --prefix packages/mobile run ios
```

The standard local gates are:

```bash
npm run mobile:lint
npm run mobile:typecheck
npm run mobile:test
npm run mobile:smoke:android
cargo test --workspace
```

## Protocol and terminal generation

Rust `shared` DTOs are canonical. After changing a public mobile/auth/session,
autonomous-job REST contract or WebSocket DTO, regenerate and commit the schema
and TypeScript output:

```bash
npm run protocol:generate
npm run protocol:check
```

The check reruns the Rust schema exporter and fails when
`packages/protocol/schema` or `packages/protocol/src/generated.ts` drifts.
Update shared golden fixtures and both Rust/TypeScript decoding tests whenever
Serde behavior changes.

After changing `packages/terminal-web/src`, rebuild the self-contained local
asset and test CSP/bridge constraints:

```bash
npm --prefix packages/terminal-web run build
npm --prefix packages/terminal-web test
```

Do not hand-edit `dist/terminal.html` or `dist/terminalHtml.ts`.

## EAS and local native builds

`packages/mobile/eas.json` defines `development`, `preview`, and `production`
profiles. Supply `EXPO_PUBLIC_EAS_PROJECT_ID` through the profile/secret store
before requesting a push token. Runtime compatibility follows `appVersion`, so
native module, permission, SQLCipher, or WebView changes require a new binary;
compatible JavaScript-only changes may use the matching update channel.

Typical commands from `packages/mobile` are:

```bash
eas build --profile development --platform ios
eas build --profile development --platform android
eas build --profile preview --platform all
eas build --profile production --platform all
```

For a reproducible local native project/build, use `npx expo prebuild --clean`
in a disposable clean worktree followed by `npx expo run:ios` or
`npx expo run:android`. Generated `ios/` and `android/` directories are ignored;
the checked-in app config remains canonical.

`APAS Code`, `com.mpaxos.apas.code`, artwork, signing teams, and store ownership
are provisional until the release-owner checklist is completed. Never publish
an association file or production build using credentials that do not match the
final signed identifiers.

## Server configuration and staged flags

All mobile capabilities default off. A representative `/etc/apas/server.toml`
section is:

```toml
[auth]
mobile_access_expiry_minutes = 15
mobile_refresh_expiry_days = 30

[mobile]
allow_insecure_localhost = false

[mobile.features]
bootstrap = true
coding_mutations = false
terminal = false
notifications = false
deep_links = false

[mobile.push]
batch_size = 100
# access_token is optional and must come from protected production config.
```

Enable in this order after each acceptance gate: `bootstrap` (read-only),
`coding_mutations`, `terminal`, `notifications`, then `deep_links`. Disabling a
flag is the first rollback action. Protocol negotiation also advertises bounds
and capabilities; incompatible apps remain read-only.

## Autonomous workers and native qualification

Workers/Attention is separate from the interactive session home. Native
`/code/workers/...` screens cover approved responsibilities, standing checklists,
backlog editing, recurring/per-run budgets, lifecycle requests and exact-run
questions/review. They use canonical protocol contracts, not a WebView.
Worker/run/question identities remain exact across cold links and reconnect;
an older review never opens the newest candidate instead. Attention does not
require push permission.

The responsive **web** project screen has its own `+` → Interactive agent /
Autonomous worker chooser and embedded worker views. **Overview** is present
with zero interactive panes. Responsibilities, project history, backlog and runs
use bounded pagination rather than treating a loaded page as a total. Selecting
or hiding a worker does not unmount an interactive terminal or change execution.
Global Workers/Attention remain cross-project views.

`/code/jobs/...` is read-only Earlier results. A safely stopped finite job or
saved standalone draft can prefill a fresh worker proposal; admission still
requires a new preview and approval. Old bot prompt/provider/cadence/history is
read-only and cannot be converted or restarted. No migration creates a grant or
restarts an unrelated pane.

Host provisioning, pinned images, scoped credentials, recurring accounting,
structured results, retention and recovery remain canonical in `AGENTS.md`,
under **Autonomous workers: deployment remains qualification-gated**. No
recurring execution profile has been qualified. OMP 18.18.0 source pins,
container metadata, unit tests and Android export are not qualification.

The additive `autonomous_workers` bootstrap and `autonomous_workers_v1`
capability advertise worker-aware client support. New execution requires
`omp_persistent_worker_v1`, protocol v2 and current profile/bootstrap evidence.
Effective recurring policy and fresh machine readiness decide target usability.
Finite-job policy alone never enables recurrence; deployment, cluster and
project controls narrow the same allowance. Existing bootstrap,
coding-mutations, notifications and deep-link flags still apply, and none can
widen compute authority. Unavailable targets remain unavailable rather than
falling back to an interactive pane.

Composer/backlog/answer drafts and request identities are persisted in SQLCipher
before submission; event bytes and the applied cursor commit together.
Foreground, reconnect and receipt resolution recheck authority. Reconnect never
automatically resends approval, prompts, answers or controls. Unknown outcomes
remain visible until the original actor-scoped receipt resolves them. Offline
snapshots are stale and controls are read-only. Recommended question choices
are not preselected consent.

**Pause after this run** fences new dispatch; **Stop now** also requests complete
termination and fences publication. Neither accepted receipt proves execution
has stopped. Resume retains consumed allowance and item attempts; an explicit
item retry is distinct from resuming a worker. Idle and review-wait retain the
responsibility without a provider process. Inspection completion is not a
delivered change.

Account changes, logout and discovered revocation purge worker/job records and
in-memory/download caches. An offline device cannot discover revocation until
reconnect. Revoking a device invalidates that device's credentials, not an
otherwise authorized account's worker; account/runtime-grant revocation is the
separate stop path.

Notifications use privacy-safe worker/run/question identities, never task text,
answers or diffs. Decisions, failures/recovery and draft review are actionable;
routine inspection/wakeup completion is not notification-worthy. Routing first
reauthenticates and authorizes the exact target. Stale or inaccessible questions
remain read-only or return to the nearest authorized Code screen, never answer
automatically or attach another session.

Before enabling, run **both iOS and Android development builds** through mandate
approval/replacement, backlog editing, every question, pause/stop/resume, retained
checks/diff/log review and draft-PR handoff. Include lost responses,
offline/background/reconnect, stale/cold notification links, account switching,
revocation, 320dp layouts, 48dp targets, keyboard/safe areas, enlarged text and
VoiceOver/TalkBack. Unknown/expired verification must never appear as passing.

Local successor checks passed native lint, TypeScript and all **136 tests**,
plus Android Hermes export. No native device/emulator was available; **device
interaction qualification remains open**. Desktop Chromium viewport emulation
does not qualify mobile keyboards, zoom/safe areas or screen readers. Local
browser/API checks use explicitly synthetic retained records, not actual
qualified OMP execution, independent verification or real PR publication.

Roll out server/shared protocol first, web/native clients second, CLI/daemon
last, with recurrence and host execution disabled until qualification. Drain
known-live legacy runtimes before enabling and separately approve new workers.
For rollback, disable new admission and affected mobile flags, retain capable
controllers until whole-unit termination and every required host purge is
confirmed, and preserve durable receipts/journals/evidence before downgrading.
An unreachable host stays pending cleanup. Deployment, policy enablement and
real-worker approval require separate authorization.

## Device-session administration

Users manage their signed-in devices from Account. Cluster administrators use
the authenticated mobile device-list/revoke endpoints or the admin surface;
revocation immediately invalidates refresh, removes associated push tokens, and
causes the next foreground/bootstrap to wipe inaccessible protected state.

Useful privacy-safe database checks are:

```sql
SELECT id, user_id, installation_id, app_version, created_at, last_used_at,
       expires_at, revoked_at, revocation_reason
  FROM mobile_device_sessions ORDER BY created_at DESC;
SELECT platform, COUNT(*) FROM mobile_push_tokens
 WHERE retired_at IS NULL GROUP BY platform;
SELECT status, COUNT(*) FROM mobile_notification_deliveries GROUP BY status;
```

Never query, print, or copy `refresh_token_hash`, refresh history, raw push
tokens, task instructions, or cached event payloads into tickets or logs.

Cluster administrators can read `GET /admin/mobile/metrics` with the normal
Bearer token. It exposes process-lifetime counters for auth, mobile WebSocket
reconnect/authentication, catch-up, mutation ACKs, terminal attach/bridge
health, and push ticket/receipt outcomes, plus persistent counts for active
devices, app versions, push tokens, pending launches, and outbox states. It has
no user, installation, project, session, token, path, or content labels. A
server restart resets process counters; persistent gauges remain authoritative.

## Push diagnosis

Notifications are best-effort hints. Confirm the feature flag, active account
and project membership, device preference, active push token, and Expo project
ID first. Inspect delivery counts/status and redacted worker logs. `retry` means
bounded transient backoff; `ticketed` awaits an Expo receipt; `delivered` means
the provider accepted the receipt; `permanent_failure` retires an invalid token.
On restart the worker returns stuck `sending` rows to `retry`.

Test a push by opening the app after delivery and confirming it reauthenticates,
refreshes bootstrap/catch-up, and then routes. A stale or unauthorized target
must remain on the nearest authorized Code screen. Do not add notification
actions or project content to diagnose delivery.

## TLS migration and rollback

Roll out transport in reversible stages:

1. Install a trusted certificate and serve HTTPS/WSS while HTTP/WS remains.
2. Deploy server/web/CLI/daemon versions that understand secure endpoints.
3. Verify web WSS reconnect, CLI session continuity, daemon project reporting,
   mobile login/refresh, and reverse-proxy WebSocket upgrade logs.
4. Redirect HTTP to HTTPS only after the compatibility inventory is empty.
5. Add HSTS last, initially with a short lifetime; extend it only after rollback
   drills succeed.

Before HSTS, rollback by disabling mobile flags, restoring the previous
server/web binaries and nginx configuration, and leaving additive mobile tables
in place. After HSTS, clients cannot use HTTP until the header expires, so HSTS
is the irreversible boundary and requires explicit operator approval. Preserve
the previous server/web artifacts and signed store build for every rollout.

## Release and incident checklist

- Run the complete repository test/build matrix and both physical-device
  security/performance/accessibility matrices.
- Verify association files against final signing identities, notification
  permissions/token rotation, cold and warm links, and revocation/cache wipe.
- Record supported iOS/Android versions, terminal throughput results, crash-free
  soak, store privacy/support URLs, signing and store owners, and Expo-versus-
  direct-push decision.
- During an auth or privacy incident, disable all mobile flags, revoke affected
  device sessions (or all sessions for affected users), rotate the JWT/keyed
  hashing secret only with a coordinated global credential reset, and retain
  redacted audit/outbox evidence according to cluster policy.

## Measured single-process test envelope

On 2026-08-08, the repository's debug test build on the APAS development host
authenticated and held 128 concurrent mobile WebSocket clients in 206 ms. The
SQLite test inserted, reopened after 100 in-flight rows, recovered, and drained
1,000 notification deliveries in batches of at most 100 in 1.616 seconds. The
tests are `bounded_mobile_websocket_connections_authenticate_concurrently` and
`notification_outbox_recovers_and_drains_a_bounded_load_after_reopen`.

These are repeatable acceptance floors, not production capacity claims. The
supported initial single-process envelope is therefore capped operationally at
128 simultaneous mobile sockets and a 1,000-row pending notification backlog;
alert before those floors and load-test on production-equivalent hardware before
raising them. Push batch size remains at or below 100. A second server process
requires replacing in-memory mutation/connection coordination and is outside
this architecture.
