## Why

APAS currently requires a desktop browser to start, monitor, steer, and review coding sessions, which makes it difficult to supervise long-running agent work away from a workstation. A dedicated code-focused mobile companion can provide the high-value orchestration loop—launch work, follow progress, answer decisions, review results, and return to the same session—without reproducing a general-purpose AI chat product or shrinking the desktop workspace onto a phone.

## What Changes

- Add a dedicated APAS mobile application for iOS and Android using Expo, React Native, and TypeScript, with native navigation and phone/tablet layouts.
- Provide a code-only home and session experience: browse active and recent project sessions, filter by project or state, open an existing session, and start a coding task against an eligible machine and project.
- Present a mobile activity timeline that emphasizes agent status, tool activity, plans, decisions, diffs, tests, pull requests, completion, and errors rather than the desktop multi-pane grid.
- Allow authorized users to steer a running coding session, answer questions, approve or reject gated actions, interrupt work, and resume the same session from mobile, web, or CLI.
- Add code-review surfaces for pane diffs, plan-review requests, PR outcomes, and concise completion summaries.
- Add an interactive terminal screen backed by the existing terminal protocol and a trusted, bundled xterm.js WebView, with mobile keyboard controls, lifecycle state, resize, scrollback replay, and reconnect handling.
- Add push notifications for actionable or terminal coding events such as approval requests, questions, failures, PR readiness, and task completion, with deep links to the exact project, session, and pane.
- Harden mobile authentication and transport with HTTPS/WSS-only production connectivity, secure device credential storage, revocable mobile sessions, protocol compatibility negotiation, and lifecycle-aware reconnect/catch-up.
- Extract or generate a shared TypeScript wire contract from the Rust protocol so web and mobile do not independently hand-copy message shapes; preserve the existing Rust server, CLI, daemon, authorization boundary, and project data model.
- Deliver the first release as a companion, not a desktop replacement. General Claude-style chat, local code execution on the phone, a desktop pane grid, a full file editor, background WebSocket execution, and full cluster-administration parity are out of scope.

## Capabilities

### New Capabilities
- `mobile-code-sessions`: Native authentication, project/session discovery, task launch, live coding activity, steering, approvals, review, cross-surface continuity, secure persistence, and reconnect behavior.
- `mobile-terminal-access`: Safe mobile attachment to APAS terminal panes with xterm rendering, touch/keyboard controls, snapshots, sequencing, lifecycle state, and recovery.
- `mobile-code-notifications`: Device registration, privacy-safe push notifications, notification preferences, delivery handling, and deep links into coding sessions.

### Modified Capabilities

None. Existing project lifecycle and web dependency requirements remain unchanged; the mobile client consumes their established server behavior without redefining it.

## Impact

- Adds a new mobile workspace, expected at `packages/mobile`, with Expo/React Native dependencies, native platform configuration, test tooling, and EAS build/update profiles.
- Adds a framework-neutral TypeScript protocol/domain package shared by web and mobile, plus Rust-side schema or fixture generation and compatibility tests.
- Extends Axum authentication and API/WebSocket behavior for revocable mobile device sessions, mobile client metadata, protocol capabilities, push-token registration, notification preferences, and event delivery.
- Updates nginx and production deployment to serve HTTPS/WSS, redirect cleartext traffic, support universal/app links, and expose mobile association metadata.
- Adds persisted mobile-device and notification-delivery records while retaining SQLite and the current single-server runtime architecture.
- Refactors selected web connection/protocol logic for reuse without attempting to share React DOM components with React Native.
- Introduces APNs/FCM delivery through Expo Notifications initially, EAS build and release automation, and mobile observability/privacy requirements.
