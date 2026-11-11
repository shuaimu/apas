# APAS Code mobile

Expo SDK 56 / React Native application for the APAS coding-session companion.
It intentionally contains no general chat inbox and never executes project
code on the device.

Use a development build because SecureStore, SQLCipher SQLite, notifications,
and the bundled terminal surface require native modules. Copy `.env.example`
only for local overrides; production profiles accept HTTPS/WSS endpoints only.

The display name and bundle/application identifiers are provisional until the
production signing and store ownership decision is finalized.

## Autonomous jobs

The coding-session home retains its interactive workflow and links to Jobs and
Attention. Native job composition, all-question dialogs, cancellation/recovery,
and retained diff/check/PR review live under the authenticated stack; matching
`/code/jobs/...` links resolve the exact authorized job without attaching a session.
Push permission is optional for the in-app Attention inbox.

Job REST contracts are generated from `@apas/protocol`. Admission depends on the
server's effective autonomous policy and fresh isolated-worker readiness, not
an interactive terminal profile. Reported-spend thresholds are not final-invoice
guarantees. A cancellation receipt is not evidence that descendants stopped.

Composer/answer drafts and mutation identities are awaited in the existing
SQLCipher cache before sending. Reopening resolves the original actor-scoped
receipt; reconnect never replays a mutation. Offline/background clients are
read-only except for local draft editing. Returning clients reauthorize and
transactionally catch up each selected job before controls become available.
Logout and discovered account/project revocation clear protected job records and
query/evidence memory. An offline device cannot discover revocation immediately.

Native acceptance requires real iOS and Android development builds: exercise
320dp layouts, keyboard/safe areas, enlarged text, VoiceOver/TalkBack, full
multi-question forms, interrupted admission, background/offline recovery, revoked
access, retained evidence, and cold/stale notification links. These interactions
have not been qualified by an Expo export or by the implementation alone.
