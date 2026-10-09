## 1. Define the local-registration contract

- [x] 1.1 Add the `RegisterLocalProject` and correlated `LocalProjectRegistered` messages in `crates/shared/src/messages.rs`, with explicit success/error data and required request IDs, without changing clone-message semantics.
- [x] 1.2 Advertise `local_project_registration_v1` from supporting daemons and project a default-false owner-only availability flag through server machine responses and web machine types.
- [x] 1.3 Regenerate the committed web/mobile schemas and protocol TypeScript using the existing export pipeline, including new messages and default-false capability projection, and keep the existing validators consistent.

## 2. Implement non-destructive directory registration

- [x] 2.1 Add a daemon registration-only helper that accepts absolute and `~/` paths, resolves the existing canonical directory without a shell, rejects invalid targets, and runs blocking filesystem work off the async runtime.
- [x] 2.2 Reuse strict project metadata loading and zero-pane construction while preserving existing `.apas` bytes and configuration; reject malformed, symlinked, or non-regular metadata, recover an existing registry identity when metadata is absent, and prevent concurrent creation from replacing an identity.
- [x] 2.3 Reuse `project.rs`'s checked registry persistence with canonical-path/ID conflict checks under the same registry lock. Propagate read/write failures; keep matching registrations idempotent and preserve unrelated callers' existing replacement semantics through an explicit policy or extracted primitive.
- [x] 2.4 Handle the new daemon request, refresh normal machine inventory, and acknowledge only confirmed registration. Do not clone, initialize Git, alter branches/remotes/source files, start projects/providers, or delete user directories on failure.
- [x] 2.5 Keep deterministic filesystem regressions for plain directories, non-GitHub/no-origin repositories, existing dirty GitHub checkouts, unchanged existing metadata, symlink aliases, repeat/concurrent requests, malformed metadata, identity conflicts, and failed registry persistence followed by retry. Use isolated temporary registries/directories, not live user configuration.

## 3. Enforce server authority and result routing

- [x] 3.1 Authorize local registration against the active authenticated account and daemon-owned target machine, reject shared-cluster members and mismatched cluster context before forwarding any path, and enforce online/capability checks.
- [x] 3.2 Bind local request correlation to user and machine, handle daemon acknowledgements in the existing daemon WebSocket route, and return correlated terminal errors for all pre-dispatch/daemon failures. Handle connection loss without an indefinite pending operation or automatic replay.
- [x] 3.3 Before web success, finalize canonical identity/hosting placement through `Database::authorize_project_registration` under the existing project-operation guard, recheck current authorization/lifecycle, resolve effective policy, and ensure machine inventory is ready for the existing Start gate. Preserve existing ownership, memberships, and overrides; do not synthesize a session or start a runtime.
- [x] 3.4 Cover owner success, suspended/non-owner denial before filesystem dispatch, unsupported/offline targets, wrong-machine or stale results, request isolation, and unchanged shared GitHub cloning with behavior-level protocol/authorization regressions.
- [x] 3.5 Verify that fresh register-only projects pass a subsequent explicit Start, and that existing unauthorized/suspended/deleting identities or failed finalization return errors without deleting local metadata/directories, transferring ownership, or automatically launching agents.

## 4. Extend the shared web creation flow

- [x] 4.1 Add an accessible Clone from GitHub / Existing folder choice to general `CreateInstanceModal` usage. Keep repository-group New instance clone-specific and isolate clone/local validation and payload fields.
- [x] 4.2 Add the host-qualified local path input, owned/capable machine filtering, registration-only explanation, and Register folder submission. Preserve entered values on send/error and explain unavailable shared/older targets without silently switching cluster context.
- [x] 4.3 Generalize the existing request-keyed creation feedback/state and `/machines` pending consumers to distinguish cloning from local registration, preserving concurrent-request isolation and current clone behavior. Retain local terminal results long enough for the modal to show confirmed completion or a correctable error.
- [x] 4.4 Add a local success view with canonical path and View on Machines navigation scoped to the owning cluster and target machine/project. Refresh inventory, expose the existing explicit Start control, and do not synthesize sessions or change the active workspace automatically.
- [x] 4.5 Make general Sidebar, SidebarRail, mobile home, and machines-page creation labels source-neutral. Preserve the existing GitHub clone fields, defaults, shared restrictions, and Create & start lifecycle.
- [x] 4.6 Update existing behavioral coverage for source-switch validation, retained drafts on failure, owner/capability gating, mixed-operation correlation, completion visibility, and no automatic start. Remove incidental GitHub-label/wiring assertions instead of repinning copy; do not add mock-forwarding or source-text tests.

## 5. Verify the complete feature and document it

- [x] 5.1 Run the affected Rust checks/tests and the existing web checks once after integration. Fix contract regressions without broad unrelated cleanup.
- [x] 5.2 Exercise the real daemon/server registration path against disposable GitHub-origin, other-origin/no-origin, and plain directories; observe canonical registry entries, zero new panes, preserved files/Git state, explicit failures, and duplicate/alias identity reuse.
- [x] 5.3 Smoke the actual desktop and mobile web surfaces: register a local folder, inspect pending/error/success states, follow View on Machines to its stopped project, then explicitly start a disposable empty project and open its authorized zero-pane workspace. Also exercise the unchanged GitHub clone path. Do not use production projects or start real provider agents for this smoke.
- [x] 5.4 After smoke proof, update the canonical `AGENTS.md` project-creation guidance with machine-local path semantics, owner-only authority, metadata preservation, explicit Start behavior, and capability-based rollout requirements.

## 6. Correct registered-project visibility

- [x] 6.1 Merge owned machine registrations into the shared project-list projection using stable project identity and explicit absent session identity; deduplicate hosts and later sessions, retain authority boundaries, and reuse scoped Machines navigation.
- [x] 6.2 Show registered/stopped entries in the desktop sidebar and collapsed rail with accessible navigation to explicit Start; preserve real-session selection, unread state, ordering, and project actions.
- [x] 6.3 Show the same registered/stopped entries in mobile All projects from bootstrap and live inventory, without changing existing session recency, Idle sessions, or attachment behavior.
- [x] 6.4 Run integrated web checks and real browser smoke, update the canonical runbook after proof, and deploy the web-only correction. Verify q-index is visible while it remains stopped and existing production runtimes are unchanged.
