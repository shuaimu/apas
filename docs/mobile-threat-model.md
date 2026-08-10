# Mobile Code companion threat model

Status: engineering review complete on 2026-08-08. No release-blocking design or
implementation finding remains open. Physical-device validation, signing, and
store review are separate release gates recorded in the OpenSpec change.

## Scope and trust boundaries

Protected assets are account credentials, project membership, session history,
task instructions, decisions, source/diff/test output, terminal bytes, push
reachability, and cached metadata. The trusted computing base is the APAS
server and SQLite database, the authenticated CLI/daemon runtime, the native
application, platform SecureStore/Keychain/Keystore, and the locally bundled
terminal document. The public network, deep-link source applications, Expo Push
Service, lock screen, and all strings received by the terminal WebView are
untrusted.

The native application owns credentials, authorization refresh, the APAS
WebSocket, terminal routing, and external navigation. The terminal WebView is a
renderer/input device only. Push and deep links are hints, never authority.

## Reviewed threats and controls

| Threat | Controls and evidence | Residual risk / disposition |
| --- | --- | --- |
| Refresh-token theft or replay | Refresh tokens are opaque, stored with `WHEN_UNLOCKED_THIS_DEVICE_ONLY`, sent only over HTTPS, stored server-side as keyed hashes, rotated atomically, and recorded in one-way history. Reuse revokes the device session and its push tokens. Access tokens default to 15 minutes; refresh sessions default to 30 days. Login/refresh are rate limited and token values are excluded from audit/log fields. | A fully compromised unlocked device can act as its user. The user or cluster administrator can enumerate and revoke the device. Accepted. |
| WebView credential or data-boundary escape | The terminal HTML is bundled locally, has `default-src`, `connect-src`, and `frame-src` set to `none`, cannot use DOM storage or file access, and cannot navigate or create windows. A strict allowlisted bridge rejects unknown fields and credential-shaped payloads. Credentials never cross the bridge. External HTTPS links require native host allowlisting and user confirmation. | Platform WebView compromise is handled through supported-OS policy and native updates. Device-matrix validation remains a release gate. |
| Deep-link spoofing or stale notification | Only `apas://code/...` and `https://apas.mpaxos.com/code/...` shapes with bounded UUID targets are parsed. Every open refreshes authentication and bootstrap authorization, updates the encrypted cache, and routes only to a currently accessible session. Links cannot approve, answer, interrupt, launch, or submit a draft. | Custom schemes can be claimed by another local app; they contain no authority. Universal/app-link association must be published before external release. Accepted for internal builds. |
| Push privacy leakage | Payloads contain a generic APAS title/body, event category, and opaque route/session identifiers only. Serialization tests reject prompt, output, code, diff, terminal, secret, project-name, and filesystem-path keys. Project-name previews do not exist. Receipt errors and token rotation are stored without project content. | Opaque identifiers and notification timing remain visible to the OS/provider. Documented and accepted. |
| Offline cache extraction | Expo SQLite is built with SQLCipher, receives a random 256-bit key held only in SecureStore, and is wiped with credentials on logout/revocation. Inaccessible session rows, events, and watermarks are removed after bootstrap reconciliation. Android backup of SecureStore is disabled. | An attacker controlling an unlocked device may read process memory. Accepted as equivalent to interactive user compromise. |
| Cross-project authorization | Bootstrap filters inaccessible projects. Every HTTP/WebSocket mutation reloads active account, project membership, lifecycle, session, and pane state. Mobile actions carry exact session/pane identifiers. Decisions use atomic first-response claims; opaque mutation IDs retain/replay acknowledgements so retries do not cross or duplicate actions. Project deletion cascades mobile launch and notification records and cache reconciliation removes stale client data. | In-memory ACK retention does not survive a server crash; after restart a decision fails closed as stale instead of being re-applied. Accepted. |
| Terminal data leakage or stale input | Terminal scrollback remains in bounded server memory and is not written to message history. The client attaches only after ready authorization, reconciles snapshot before live frames, resets on process generation change, disables input on background/disconnect, clears content on access loss, cancels pending resize, and sends exact-pane UTF-8 input. Clipboard reads require an explicit action and confirmation. | Screenshot/shoulder-surfing risk is a platform/user concern. Accepted. |
| Notification replay, duplication, or invalid endpoint | Logical events and per-token deliveries have unique keys. The SQLite outbox recovers `sending` rows after restart, applies bounded retry, polls receipts, retires invalid tokens, and removes delivery reachability on logout/revocation. A late duplicate push still undergoes fresh authorization. | Providers remain best effort. Push is never the correctness channel. Accepted. |
| Cleartext downgrade | Production app configuration rejects non-HTTPS/non-WSS endpoints. TLS/WSS is served by the production reverse proxy; localhost cleartext is available only through an explicit server development exception. | HTTP/WS compatibility endpoints remain during staged migration. Mobile never selects them; redirect/HSTS is a later operational gate. |
| Sensitive logging | Mobile audit records contain actor, opaque installation/device/session/pane/request identifiers, versions, action, and outcome—not credentials, task instructions, project output, source, diffs, terminal bytes, push payload bodies, or paths. Server logging uses structured fields at authorization/transport boundaries. | Reverse-proxy/device crash logs are governed by infrastructure retention. Accepted with the documented runbook. |

## Findings resolved during review

- Approval output without a server session identifier is now discarded by the
  mobile normalizer instead of being cached under a synthetic target.
- A repeated decision or interrupt request now atomically reserves an opaque
  per-user request identifier and replays the original bounded acknowledgement,
  preventing an ACK-loss retry from applying the action twice.
- Access loss now clears terminal presentation and cancels a pending resize;
  terminal input and resize both require current synchronized authorization.
- Terminal URLs are HTTPS-only at the bridge and native navigation is limited
  to the explicit host allowlist.

## Privacy review

The optional lock-screen project-name preview is not approved. It is absent
from server DTOs, preferences, native UI settings, and push payload generation.
Generic payloads are mandatory for all installations. Introducing any preview
requires a new proposal, explicit opt-in UX, payload/schema changes, lock-screen
and provider data-flow review, and updated regression tests. Until that review,
the server transport must reject or ignore preview-like content.

## Release re-review triggers

Re-open this model when adding a direct terminal ticket, remote WebView content,
notification actions, biometrics, attachment/file transfer, project-name push
previews, direct APNs/FCM, a second server process, or a new credential storage
backend. Also re-review after the physical-device security matrix identifies a
platform-specific exception.
