# APAS Code mobile

Expo SDK 56 / React Native application for the APAS coding-session companion.
It intentionally contains no general chat inbox and never executes project
code on the device.

Use a development build because SecureStore, SQLCipher SQLite, notifications,
and the bundled terminal surface require native modules. Copy `.env.example`
only for local overrides; production profiles accept HTTPS/WSS endpoints only.

The display name and bundle/application identifiers are provisional until the
production signing and store ownership decision is finalized.

## Autonomous workers and earlier results

Home links to Workers and Attention while interactive sessions remain separate.
The native worker flow approves a continuing responsibility, human-owned standing
checklist, exact host and execution/model/verifier profiles, base branch, cadence,
per-run ceilings and rolling-24-hour allowance. Review explains autonomous work
discovery, future committed branch tips, automatically renewing spend, one-draft
review waiting, and the difference between Pause after this run and Stop now.
No policy is enabled by installing this client.

Global and project-filtered inventories are independently paginated. Worker
detail separates the mandate, immutable current run, editable unclaimed backlog,
per-run and recurring usage/reservations, checkpoint observations and paginated
execution history. Idle and paused responsibilities are not completed tasks.
Durable control acceptance is not proof of execution or complete descendant stop.
Reported spend may be delayed, estimated, unknown or overrun; it is not an exact
provider-invoice guarantee.

Canonical REST/event contracts come from generated `@apas/protocol` types and
validators. Bootstrap worker support, current server-provided allowed actions and
authorized HTTP catch-up gate controls; a connected socket alone never does.
Capability-negotiated worker notifications are repairable synchronization hints,
not authority. All `/code/workers/:worker/runs/:run/...` question and review routes
retain the requested historical execution. Notification taps authorize and
navigate only: they cannot answer, resume, retry, approve or merge.

Drafts, account/project/worker/item/run/question identities, receipts, accepted
snapshots and cursors use the existing SQLCipher store. Snapshot, bounded event
window and watermark are committed together. Reopening resolves the original
receipt; reconnect never replays a POST. A user may explicitly retry the same
identity only after receipt recovery finds no receipt. Background/offline caches
are visibly stale and read-only; local draft editing grants no authority.
Foreground recovery checks all cached projects, including later inventory pages.
Logout and discovered revocation purge protected drafts, histories, hidden labels,
global pages and downloaded evidence, with generation fences against late writes.
An offline device cannot discover a remote revocation immediately.

Earlier `/code/jobs/...` links retain read-only results and receipt recovery.
Finite-job creation redirects to an unapproved worker draft; legacy instruction
text and model choice are preserved for review, never promoted into recurring
permission. Earlier result instructions can be explicitly reused, requiring fresh
target/profile and recurring approval.
Old bot prompts, provider identity and conversation history remain read-only.
Bot conversion is deliberately unsupported: approve a fresh worker separately;
no terminal process is adopted and no old bot authority becomes recurring spend.

Native acceptance still requires real iOS and Android development builds:
exercise narrow layouts, keyboard/safe areas, enlarged text, VoiceOver/TalkBack,
approval/backlog editing, exact historical questions/reviews, interrupted receipt
recovery, background/offline recovery, revocation and cold notification links.
No device, emulator, provider, deployment or production approval was exercised
as part of this implementation; source tests and type checks do not close those
operational qualification gates.
