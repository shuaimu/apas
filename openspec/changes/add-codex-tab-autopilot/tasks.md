## 1. Provider check on a shared daemon (gate)

- [x] 1.1 The owner chooses where the shared-daemon check may run (design, open question 2). Nothing in groups 2 to 7 starts before 1.3. (done 2026-11-14: a private managed daemon with its own CODEX_HOME on zoo-005, with the owner's consent)
- [x] 1.2 In that environment, with a stub model, repeat the 0.160 checks against a real shared app-server daemon over its unix socket, on the daemon version the owner runs: `canAcceptDirectInput` for a TUI-owned conversation; `turn/start` from idle rendered live in the TUI and recorded in its rollout; `turn/completed` status for completed, Escape and `turn/interrupt`; steering when a person's turn is running; the client id in the rollout; goal mode notifications; that a second client leaves the daemon's other conversations and clients untouched; and a daemon restart with an autopilot subscribed. Also, not run on 0.160: a failed turn, an approval request, an in-flight turn across a restart. (done 2026-11-14 on daemon 0.166.0, decisive items repeated on 0.165.0, TUI 0.160.0, stub model; challenged and re-run independently; results in NOTES.md "Shared daemon (task 1.2)" and design decisions 3 to 5. Not covered: mid-turn compaction; one SQLite home shared by TUIs and daemon)
- [x] 1.3 Record the results in `~/.cache/apas-codex-check/NOTES.md` and decide option A or B with the owner; update design.md decision 5 and remove the option not chosen from groups 2 to 7. (done 2026-11-14: option A, the shared daemon)

## 2. Shared contract

- [ ] 2.1 Add `codex_tab_autopilot_v1` for CLIs and viewers in `crates/shared/src/autopilot.rs`, with the Codex pause reasons (APAS restarted, Codex goal mode on) and the unsupported-version text; keep every existing frame shape.
- [ ] 2.2 Regenerate `@apas/protocol` and extend its fixtures with a Codex tab's state frame.
- [ ] 2.3 The existing Codex locator (`transcript/codex_daemon.rs`) initializes as `codex_app_server_daemon` instead of `apas_transcript`, so it stops rewriting the shared daemon's User-Agent on the owner's own model requests; a test pins the name.

## 3. CLI

- [ ] 3.1 A Codex app-server client over the shared daemon's control socket: connect, `initialize` as `codex_app_server_daemon`, subscribe to one conversation, bounded request timeouts, reconnect with backoff, never answer server requests.
- [ ] 3.2 The loop per Codex tab: the submission rules, outcome rules, holds and limits of design decisions 2 and 3 (including submitting from systemError after a failure, reconciling every reconnect with `thread/read {includeTurns: true}`, never treating an interrupt found after a lost connection as a person's, and holding on `serverShuttingDown`), with the same `PaneAutopilotState` as OMP; a private state file by temp-and-rename; Stop now via `turn/interrupt`.
- [ ] 3.3 Request exchange: the server's `ControlPaneAutopilot` for a Codex tab goes to the loop and is answered with one receipt; a start that cannot be applied is refused with its reason.
- [ ] 3.4 Restart: on instance start, report any Codex autopilot found in a state file as paused because APAS restarted; never submit before someone resumes it.
- [ ] 3.5 The Codex transcript reader shows `client_id` "apas-autopilot:*" user messages as iteration markers, not as a person's messages.
- [ ] 3.6 Bind a tab through its `codex_tui` origin before the first turn (design decision 4), and end the autopilot when the tab starts another user conversation.
- [ ] 3.7 Report support per Codex tab (supported, not reported, unsupported Codex or app-server version, older instance), recording both the TUI's and the app-server's version at every registration.
- [ ] 3.8 Tests: a fake app-server over a socket pair covering idle submission, steering, completed, failed three times, interrupted by a person, Stop now with no answer, goal mode, a conversation change, an app-server restart, an instance restart and every limit.

## 4. Server

- [ ] 4.1 Accept autopilot state and route autopilot requests for Codex terminal tabs from instances that advertise `codex_tab_autopilot_v1`, with the same authorization (`check_session_runtime_access`) and owning-instance routing as OMP tabs.
- [ ] 4.2 Send Codex autopilot frames only to viewers that negotiated `codex_tab_autopilot_v1`; add the instance-update-required support state for an older instance.
- [ ] 4.3 Tests for both, including an older viewer that receives nothing.

## 5. Web

- [ ] 5.1 Offer the control, status line, start dialog, templates, list grouping, pane dot ring and phone actions on Codex terminal tabs when the viewer negotiated the capability; Codex texts for the new reasons.
- [ ] 5.2 Tests for a Codex tab in each support state and condition, and that Claude and OpenCode tabs still show no control.

## 6. Documentation

- [ ] 6.1 `AGENTS.md`: Codex in the autopilot section (how it is driven, what differs from OMP: the loop lives in the instance, a daemon reboot pauses it, a person's typing can merge with an iteration), the rollout order and the rule to repeat the provider check on a Codex minor version change.

## 7. Verification and rollout

- [ ] 7.1 Gates: Rust workspace, static musl CLI build, protocol, web suite with lint, typecheck and build, native unchanged, `openspec validate --strict`.
- [ ] 7.2 A real Codex tab with a real cheap model in the isolated harness: start, an iteration ends, Escape pauses, resume, Stop, Stop now, a daemon reboot leaves it paused, goal mode pauses it, `/new` ends it.
- [ ] 7.3 A browser run of the control on a Codex tab at desktop and phone width.
- [ ] 7.4 Deploy server, then web, then push the CLI; existing Codex tabs gain autopilot at their machine's daemon reboot.
