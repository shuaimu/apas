Phases are ordered so each one is independently shippable: Phase 1–3 make a
second instance on a shared home work (started from the Machines page), Phase 4
adds choosing a machine from the new-tab menu, Phase 5 joins separate
checkouts, Phase 6 clones onto a machine, Phase 7 covers native mobile.

## 1. Define the contract

- [ ] 1.1 Add optional `machine_id`/`hostname` to `shared::PaneConfig`, the `project_instances_v1` capability constant, and `ServerToCli::pane_id()` with a pane/decision/fan-out/process classification covered by an exhaustive-match test.
- [ ] 1.2 Add optional `machine_id`/`request_id` to `WebToServer::AddPane`, `ServerToWeb::AddPaneResult`, optional `machine_id` on process-scoped lifecycle requests, instance hostnames on session summaries, `project_instances_available` and a canonical `git_remote` on machine entries and their projects, `join_project_id` on clone requests, and web/CLI `ForgetMachinePanes`. All are optional or defaulted so older peers parse them.
- [ ] 1.3 Regenerate the web/mobile schemas and `packages/protocol` TypeScript through `export_mobile_schema`, and update the protocol fixtures.

## 2. Make the CLI runtime instance-safe

- [ ] 2.1 Derive the machine id for every project run (including standalone `--headless`) and thread it through restore, save and the server connection. Do not advertise the capability when `config.daemon.machine_id` is set.
- [ ] 2.2 Add an exclusive per-project lock at `<config dir>/locks/<project-id>.lock` (never a file in the project directory) and route every `.apas` writer through locked re-read/modify/write. Pane saves replace only this machine's panes; project-level writes change only their field.
- [ ] 2.3 Adopt untagged panes at start, under the lock, only when no other machine holds a live instance claim. Restore only this machine's panes, and never adopt a pane tagged for another machine even if that machine is stopped (tags are permanent). Handle `ForgetMachinePanes` by removing that machine's entries under the lock. Make the periodic and reconnect reads non-destructive.
- [ ] 2.4 Exclude every `.apas` pane session id (any machine) from Claude transcript switch and fallback discovery. Make `--attach`'s local pane-id choice avoid every pane in `.apas`.
- [ ] 2.5 Replace claims with per-(project, machine) claims plus the marked legacy guard. Release on every stop path, refresh only running instances, and use host- and pid-qualified temp names. Report a claim refusal as the project's `last_error` instead of returning silently. Report each registered project's canonical `git_remote` in the daemon inventory.
- [ ] 2.6 Advertise `project_instances_v1` from capable CLIs and daemons. Add regressions on temporary directories: two in-process instances saving concurrently, adoption with and without a live peer, empty-roster save, corrupt-file poll, and claim release/handoff/legacy exclusion.

## 3. Attach several instances to one session on the server

- [ ] 3.1 Add per-machine attachments to `SessionState` with primary selection. `SessionStart` from a capable CLI adds or replaces its machine's attachment; a non-capable one keeps replace semantics when alone and is rejected when another machine is attached. A capable one is rejected while a non-capable CLI from another machine is attached. Make the DB `create_session` upsert, `set_session_machine`/`set_session_project`, `attach_web_to_session` and legacy `assign_cli_to_session` primary-only or instance-aware.
- [ ] 3.2 Merge rosters per machine (`Some`/`Some([])`/`None`), with server-assigned tags, the global order, scoped status pruning, and tags persisted in `panes.json`. Apply the same merge to `SessionStart.panes` and `PaneList`.
- [ ] 3.3 Route `route_to_cli` by classification. Move `AnswerOmpQuestion`, work-summary dispatch and capability checks to the owning attachment. Evaluate runtime access per pane machine and for attach.
- [ ] 3.4 Validate pane ownership on inbound pane-scoped CLI messages. Scope disconnect, `SessionEnd`, terminal disconnection, status clearing, DB inactivity, presence broadcasts and lifecycle inventories to the departing machine, with primary handover.
- [ ] 3.5 Refuse pane controls and closes for a machine with no attachment, naming the host. Implement `ForgetMachinePanes` (owner confirmation, roster drop, fan-out). Make project Start launch an instance on every online machine that owns tabs. Reserve in-flight pane ids in `AddPane` allocation (also counting the mobile launch range). Make stop-runtime, suspension and deletion act on every attachment and wait for each machine's purge acknowledgement. Read machine `is_running` from every attachment.
- [ ] 3.6 Add server regressions for two-instance attach, foreign-pane rejection, scoped disconnect and handover, routing per variant class, legacy-CLI rejection, roster merge edge cases, and concurrent-add id uniqueness.

## 4. Choose a machine for new tabs

- [ ] 4.1 Server: route `AddPane` to an explicit machine; start-then-add for registered stopped instances with pending adds, timeouts and `AddPaneResult`; refuse incapable or unauthorized targets with a reason.
- [ ] 4.2 Web: group registration-only entries by the reported `git_remote`. Show a host badge and per-machine connection state on tabs once the roster spans machines. Show the primary host plus "+N" in the sidebar row and rail. Fix `resolveMachineProjectTarget` to prefer the primary attachment, then the preferred registration. Show a stopped machine's tabs as stopped on that host, with "Start on <host>" and a confirmed "Forget <host>'s tabs".
- [ ] 4.3 Web: add the machine step to the desktop `+` menu and the mobile-browser pane sheet, with grouped and disabled choices and the selected tab's machine as default. Track pending, starting, failed and unconfirmed launches per request id without replay.
- [ ] 4.4 Run web lint, typecheck and tests. Smoke on disposable projects: add tabs on two zoo machines, type into each, close one machine's last tab, disconnect one instance and confirm the other is unaffected, stop on one machine and start on the other (tabs stay put), and forget a stopped machine's tabs.

## 5. Join separate checkouts by git remote

- [ ] 5.1 Server: `GET /projects/by-remote` scoped to projects the caller owns or hosts. Smoke the route through nginx (JSON, not the app's HTML).
- [ ] 5.2 CLI: extend `apas register` with remote matching, a prompt, `--project`, `--new` and `--yes`. Re-identify only pristine local projects, and refuse when the registry already holds the project at another path.
- [ ] 5.3 Add regressions for no, one and several matches, a pristine re-identify, a refusal for a project with history, non-interactive runs, and the registry-conflict refusal.

## 6. Clone to join

- [ ] 6.1 Server: accept `join_project_id` on the owner path only (the requester hosts the project and owns the machine), forward the id, and chain into start-then-add.
- [ ] 6.2 Daemon: with a supplied id on the owner path, clone without a new branch, write `.apas` with that id and zero panes, register, do not auto-start, and remove the partial clone on failure.
- [ ] 6.3 Web: offer "Clone here" for owned machines without the project. Show clone, start and add progress under one request.

## 7. Native mobile

- [ ] 7.1 Parse and show each tab's host; keep add-pane defaulting to the primary instance. Run native lint, typecheck and tests.

## 8. Verify, document, roll out

- [ ] 8.1 Run the Rust workspace, protocol, web and native checks once after integration.
- [ ] 8.2 Real smoke on disposable projects: two shared-home instances; a laptop-style separate checkout joined by remote; clone-to-join; an older-daemon exclusion; restart of a secondary instance's daemon with pane-host adoption.
- [ ] 8.3 Update `AGENTS.md` (instances, pane ownership, claims, rollout and rollback) after smoke proof.
- [ ] 8.4 Deploy server → web → CLI with the standard backups and smoke checks. Reboot daemons from the Machines page only with the owner's go-ahead.
