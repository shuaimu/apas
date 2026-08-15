## 1. DB authorization core

- [x] 1.1 Add a private `user_is_cluster_admin(user_id) -> Result<bool>` helper in crates/server/src/db/mod.rs that reads `cluster_role` + `account_status` from `users`
- [x] 1.2 In `authorize_project_registration` (db/mod.rs:2392), after the active-account and lifecycle checks, short-circuit the owner/member assertion when the caller is an active cluster admin; keep first-registration-owns behavior intact
- [x] 1.3 Make `get_sessions_for_user` (db/mod.rs:3498) return all sessions (same ORDER/LIMIT, real owner `user_id`) when the requester is an active cluster admin
- [x] 1.4 Make `get_shared_sessions_for_user` (db/mod.rs:3729) return an empty list for active cluster admins so the web/mobile bootstrap does not double-list
- [x] 1.5 Extend the db unit tests: admin passes registration without membership, suspended admin is rejected, ordinary non-member user is still rejected, admin sees all sessions

## 2. Web control plane

- [x] 2.1 In routes/ws_web.rs bootstrap paths, allow admins to receive the full machine list (fetch cluster role via `state.db.get_user_by_id` and bypass `get_machines_for_user` filtering)
- [x] 2.2 In the `StartMachineProjectCli` allowed check (ws_web.rs:3293), add an active-admin bypass alongside the existing machine-owner and shared-project checks
- [x] 2.3 Audit the remaining ws_web "Access denied" sites (≈1262, 1356, 4943, 5373, 5532, 5612) and confirm admins are not blocked once their session/machine listings include the cluster

## 3. Verification

- [x] 3.1 `cargo test -p apas-server` passes, including the new admin-bypass tests
- [x] 3.2 `cargo test` for the workspace and `cargo clippy` are clean
- [x] 3.3 Manually verify the reported case: with an admin token, `SessionStart` for project e77879a8 (mako-soumojit) is accepted without any `project_members` row for the admin
- [x] 3.4 Manually verify an ordinary non-member account is still rejected with the existing message
