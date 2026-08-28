## 1. Runtime Authorization

- [x] 1.1 Add database tests covering owner-hosted project users, third-party-hosted project users with and without active machine permission, cluster-only members, revocation, owner transfer, and multiple placements.
- [x] 1.2 Update session runtime authorization to grant project-scoped access on owner-hosted instances while preserving active cluster membership and exact-machine checks on third-party-hosted instances.
- [x] 1.3 Audit WebSocket attach and every project runtime mutation to ensure they all reevaluate the same session-specific runtime predicate while content-only operations retain ordinary project access.

## 2. Correlated Attachment Results

- [x] 2.1 Add an additive server-to-web attachment-rejection message with session identity and safe project-access, host-machine-access, unavailable-project, and missing-session reasons; update protocol schemas and generated types.
- [x] 2.2 Update the server attachment handler to distinguish access failure reasons and return the correlated rejection without registering an unauthorized subscription or leaking project state.
- [x] 2.3 Add server protocol and route tests for successful project shares, third-party host denial, stale/background attachments, and absence of pane, policy, usage, and message replay after rejection.

## 3. Transactional Web Navigation

- [x] 3.1 Add web-store tests for pending project selection, exact-session success commit, correlated rejection rollback, out-of-order results, reconnect/background subscriptions, deferred cache restoration, and deferred catch-up.
- [x] 3.2 Refactor web attachment state so the active and persisted session changes only after a matching server confirmation and a rejection produces one actionable error without follow-on project requests.
- [x] 3.3 Add workspace tests and fail-closed Overview visibility for absent, loading, stale, disabled, and enabled effective policies, including enabled and disabled zero-pane projects.
- [x] 3.4 Update Project Access and Share This Cluster copy and component tests so each interface names its scope, host prerequisite, selected machines, default agent, and unrelated-project boundary.

## 4. Documentation and Verification

- [x] 4.1 Update the canonical contributor documentation and shared-cluster user documentation with the project-share/cluster-share authorization matrix and third-party-host rule.
- [x] 4.2 Run Rust formatting, targeted authorization/route tests, the full Rust workspace tests and clippy, web unit tests, lint/type checks, protocol validation, and production builds; fix all introduced regressions.
- [x] 4.3 Run `openspec validate separate-project-and-cluster-sharing --strict` and review the final diff for authorization broadening, stale cached content, and unrelated user changes.
