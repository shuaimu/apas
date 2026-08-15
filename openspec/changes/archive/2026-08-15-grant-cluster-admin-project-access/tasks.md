## 1. DB authorization core

- [x] 1.1 Add a private `user_is_cluster_admin(user_id) -> Result<bool>` helper in crates/server/src/db/mod.rs that reads `cluster_role` + `account_status` from `users`
- [x] 1.2 In `authorize_project_registration`, keep the active-account and lifecycle checks and the owner/member assertion, but let an active cluster admin pass when a session exists with `COALESCE(project_id, id) = project` and `user_id = admin`; keep first-registration-owns behavior intact
- [x] 1.3 Make `get_sessions_for_user` return, for active cluster admins, sessions the admin owns, belongs to, or created under their account (same ORDER/LIMIT), keeping other accounts' cluster projects out
- [x] 1.4 Make `get_shared_sessions_for_user` return an empty list for active cluster admins so the union in 1.3 does not double-list
- [x] 1.5 Extend the db unit tests: admin with a session passes registration without membership, admin without any session in a foreign project is rejected, suspended admin is rejected, ordinary non-member user is still rejected, admin listings stay within their own cluster

## 2. Web control plane

- [x] 2.1 Machine listings and machine-scoped operations stay unchanged: `get_machines_for_user` and the `StartMachineProjectCli`/`StopMachineProjectCli` allowed checks are already scoped to the requester's own daemon registrations (their virtual cluster); no admin branch is added
- [x] 2.2 `check_session_access` grants active cluster admins attach access to sessions created under their account, while falling through to the regular owner/member check otherwise
- [x] 2.3 Audit the ws_web "Access denied" sites (attach, messages, pane summaries, control-message resolution) and confirm admins are not blocked for their own cluster's sessions once `check_session_access` honors session authorship

## 3. Verification

- [x] 3.1 `cargo test -p apas-server` passes, including the new cluster-scoped admin tests
- [x] 3.2 `cargo test` for the workspace and `cargo clippy` are clean
- [x] 3.3 Manually verify the reported case: with an admin token, `SessionStart` for project e77879a8 (mako-soumojit) is accepted because the admin authored its session, without any `project_members` row for the admin
- [x] 3.4 Manually verify an ordinary non-member account is still rejected with the existing message
