Group 1 comes first on purpose: the design rests on reading OMP's source, and
nothing in it was run. Groups 2 to 7 are in build order (contract, extension,
CLI, server, web). Deployment order is different: shared contract and server,
then web, then CLI and daemon, as in `design.md`. Nothing here deploys anything
or restarts a running agent.

## 1. Check the provider behaviour the design assumes

- [ ] 1.1 Ask the owner whether a short run against a real, cheap model is acceptable, or whether the check must use a local stub model endpoint. Then build the check outside the repository: the installed OMP's TUI in its own tmux server, with an isolated agent directory, loading a throwaway extension. It reads no provider login other than the one agreed.
- [ ] 1.2 Confirm what `pi.sendUserMessage` does with agent attribution and `deliverAs: "aside"`: that it starts a turn when idle; what happens when a run starts in the gap, during a manual `/compact`, and during an automatic compaction; whether an accepted prompt can start after `ctx.abort()`; that it rejects when the prompt is dropped and what the rejection says about the cause; whether the dropped prompt lands in the input line and can be cleared; and what the session file records, including the attribution.
- [ ] 1.3 Record the events and flags after: a plain turn; a turn that waits for background work, with the wait ended by the job, by Esc on the wait row and by a killed job, and with a job that never ends; a retry, including one that fails before `agent_start`; a compaction; an open `ask` and a cancelled one; an Esc interrupt during a turn and during prompt setup; the "." continue shortcut; interrupt-and-send; a user shell command; and a handoff. Correct decisions 3 and 4 to what is observed.
- [ ] 1.4 Confirm that entries written with `pi.appendEntry` are readable from the active branch at `session_start` after `--continue`; what `/new`, `/resume`, a fork, a branch, a move in the session tree and a reload each emit, before and after; where an entry appended in a `session_before_*` handler lands; and that registering those handlers changes nothing else.
- [ ] 1.5 Confirm that `getUsageStatistics()` moves with each turn, how it reads after an agent restart, when delegated work is counted, and what it holds for a provider that reports no usage.
- [ ] 1.6 Confirm how goal, loop and plan mode are reported, active and paused, and that autoresearch can be detected from its active tool.
- [ ] 1.7 Confirm the minimum OMP version: check on the installed version every call and behaviour the design uses, find the earliest version that has all of them, and confirm that the SDK's `VERSION` is readable from an extension in the compiled binary.
- [ ] 1.8 Load the shipped `omp_activity.mjs`, have the agent call `ask`, and record whether a question file appears and whether Pending answer is reported. Tell the owner the result at once: it decides whether the question bridge repair must land first.
- [ ] 1.9 Write the results into `design.md` and revise any decision an observation contradicts before starting group 2.

## 2. Define the contract

- [ ] 2.1 Add the autopilot spec with its bounds, in two forms: the viewer's request with optional limits, which the server validates, and the strict form the server forwards, which rejects unknown fields. Add the action (resume names its autopilot; pause and stop may), the receipt outcome (accepted, refused, uncertain), the state with its conditions, causes and reasons, the last ended autopilot, the CLI's `support` values, the viewer-facing availability with the two values the server adds, the summary block, and the capabilities `omp_tab_autopilot_v1` and `tab_autopilot_v1`.
- [ ] 2.2 Add `WebToServer::ControlPaneAutopilot`; `ServerToCli::ControlPaneAutopilot` with its `scope()` arm, `remote.rs` arm and classification row; `CliToServer::PaneAutopilotReceipt` and `PaneAutopilotState`, both carrying session and pane, with their `inbound_scope` arms and rows; `ServerToWeb::PaneAutopilotReceipt` and `PaneAutopilotState`; the optional `autopilot` block on `MobilePaneSummary`; and the optional `negotiated_capabilities` on `ServerToCli::Registered`. Add refusing arms for the three inbound variants in the server's handlers, so the workspace builds and both classification tests run before group 5. Box the large payloads.
- [ ] 2.3 Regenerate the schemas and `packages/protocol` TypeScript, add a fixture for every new frame, and assert that the schema the native app bundles today accepts a summary carrying the new block and a registration reply carrying the new field.

## 3. Run the loop in the OMP extension

- [ ] 3.1 Read the two request files from a managed timer once a second, started only for the main agent of a TUI session. Keep a pause or stop until it is applied; refuse a start or resume issued more than ten seconds ago; apply pending requests in the order they were issued; remember applied request ids; write results to the acknowledgement file; put the launch, pid and process-start fields and a format version on every file.
- [ ] 3.2 Implement the actions: start, validating the bounds again and rejecting unknown fields, and refusing a second start; pause; resume, only for the named paused autopilot and never after its deadline; stop; and stop now, which flags its own abort, aborts an outstanding submission when it starts, and reports "Stopped" only after that turn has ended.
- [ ] 3.3 Implement the loop on the extension's existing settled state: the settle conditions of decision 3, the poll that ends a background wait, the due time for both timings, the outstanding-submission marker, the single synchronous step before a submission, submission with agent attribution and `aside` delivery, clearing a dropped prompt from the input line, and continuing after a turn the person started.
- [ ] 3.4 Implement the limits: the iteration counter; the deadline on its own timer, ending autopilot in every condition and when the next iteration would be due after it; the token count with its baseline and journaled use; the no-tool-call ending, which never applies to a failed or interrupted iteration; the one-minute wait after a failure and the pause after three consecutive failures.
- [ ] 3.5 Classify how each turn ended from `turn_end`, and pause autopilot when a turn was interrupted by a person or a submission was dropped by an interrupt, but not for the extension's own aborts.
- [ ] 3.6 Journal the autopilot in the OMP session with the prompt in the start entry only; read the active branch and ignore other sessions' entries; restore at `session_start` as paused, as ended when the deadline has passed, or as the last ended autopilot; and end autopilot when the tab leaves its conversation, writing the end entry where it is read back.
- [ ] 3.7 Hold while the controller heartbeat has not advanced for 60 s on the extension's own clock, while it reports the server unreachable for more than 600 s, or while it reports that the server cannot control autopilot. Continue when the cause clears, and pause after one hour held.
- [ ] 3.8 Write the state report on every change and at least every 30 s, with its version and counter, the last ended autopilot and whether a turn has run since, and OMP-supplied text cut to 500 characters. Show the condition in the terminal's own status line.
- [ ] 3.9 Report OMP's goal and loop modes, active and paused. Refuse a start or resume, and pause autopilot, while goal, loop or plan mode is in any state other than off or autoresearch is on, checking again before each submission.
- [ ] 3.10 Keep publishing Working while the next iteration is scheduled within ten seconds, and across a built-in mode's gap, falling back to Idle 15 s after a run was due and did not start.
- [ ] 3.11 Check OMP's version against the minimum and detect every call autopilot uses, reporting "unsupported" when one is missing. Isolate autopilot so that a fault in it cannot stop activity reporting or the question bridge.
- [ ] 3.12 Cover 3.1 to 3.11 in `omp_activity.test.mjs`, and run that file from the protocol package's test script, the gate that already needs Node, so that a missing runner fails the gate instead of skipping the tests.

## 4. Relay it through the CLI

- [ ] 4.1 Have `install()` clear the autopilot files at each provider launch, as it already clears the activity and question files, and record the autopilot feature marker in `omp-launch.json`.
- [ ] 4.2 Implement the exchange: its own lock with a bounded wait, a two-second wait for the acknowledgement, and an "uncertain" result on timeout with no resend. Leave an unacknowledged pause or stop in place, replacing a pending one only with an equal or stronger one, and build it from the launch record alone. Remove an unacknowledged start or resume.
- [ ] 4.3 Read the state report with the existing trust checks, treat one not refreshed for 90 s as not reported, derive the tab's `support` value from the launch marker and the report, and send `PaneAutopilotState` on every change and for every OMP tab after each registration. Accept unknown fields in the report.
- [ ] 4.4 Add explicit connection state to the project runtime: connected, set when a registration completes and cleared at the top of each reconnect attempt; the time of the last inbound frame; and whether the server echoed `omp_tab_autopilot_v1`. Write the heartbeat for each OMP tab every three seconds from a thread of its own that never sends on the outbound queue, with a counter and durations from the monotonic clock.
- [ ] 4.5 Handle `ControlPaneAutopilot` in the project runtime, validating a start again, and answer with `PaneAutopilotReceipt`. Refuse it in modes that cannot serve it.
- [ ] 4.6 Make the transcript reader publish an iteration as the reserved tool-use block and never as a user turn, by the pairing rule of decision 3. Test it with a captured OMP session file that includes a dropped submission and a person's message sent during autopilot.
- [ ] 4.7 While the live OMP report says Working, send a turn's completing message with `terminal_turn_complete: false` and skip the explicit clear.
- [ ] 4.8 When the agent process of a tab whose last relayed autopilot state was on or paused exits without the CLI having stopped it, report "Paused: the agent process exited" for that tab.
- [ ] 4.9 Advertise `omp_tab_autopilot_v1`.
- [ ] 4.10 Test the exchange with a responder thread as the question bridge's tests do, plus the trust rejections, a stale report, the reconnect re-report, and a heartbeat that keeps being written while the outbound queue is full.

## 5. Authorize, route and remember it on the server

- [ ] 5.1 Handle the request in the order of decision 8: the requester's capability; claim the request id; resolve the session; require a terminal OMP tab that is not legacy bot history; find the owning instance and check access to its machine; check the instance capability and refuse an older machine with the update text; apply the hosting-account rule to start and resume; validate a start's bounds and answer a refusal with its reason; forward to the owner only; answer the requester from a spawned task, with "uncertain" when the CLI's receipt has not arrived after five seconds. Track requests by request id, so a stop is never refused because another request for the tab is in flight.
- [ ] 5.2 Cache the last state per tab, prune it wherever pane status is pruned, mark it not reported when the instance detaches, fill in "instance update required" and "not reported" for viewers, replay it to capable viewers on attach, and never send it through an unfiltered broadcast.
- [ ] 5.3 Fill the summary block in the one place pane summaries are built, add the viewer capability to the negotiated list, and echo the instance's negotiated capabilities in `ServerToCli::Registered`.
- [ ] 5.4 Test the authorization matrix (hosting account, project owner on another account's machine, collaborator, no access), routing in a two-machine session, the stopped-machine refusal, capability gating in both directions, two requests in flight for one tab, pruning, and that no status-none frame is sent between two immediate iterations.

## 6. Desktop web

- [ ] 6.1 Add the negotiated capability, per-tab state and availability, receipts with their three outcomes, and unconfirmed requests to the store; a request is never resent.
- [ ] 6.2 Add the toolbar control, its menu with the details view, and the start dialog with the required statement, the proposed limits, the values last used in the tab, the refusal of a time limit shorter than the interval, and every disabled reason, including the one for a viewer who is not the hosting account and the one for a machine where autopilot cannot run.
- [ ] 6.3 Add the tab marker, the dot ring and label, the "On autopilot" group in the idle list, paused and ended autopilots among the idle tabs with their reasons, and the condition on the Overview's pane cards.
- [ ] 6.4 Add the conversation marker line for the reserved tool-use block, the second status line, the indication of OMP's own modes with how to stop them, and the confirmations for closing a tab, rebooting its agent and closing the project when a tab on autopilot is affected.
- [ ] 6.5 Stop showing Interrupt in the pane toolbar of terminal tabs.
- [ ] 6.6 Test each, including that accessible names of tabs not on autopilot are unchanged and that the terminal instance is not recreated.

## 7. Mobile browser

- [ ] 7.1 Add "Start autopilot" to the Pane actions sheet with its start sheet and disabled reasons; the state notice above the composer with Pause or Resume, Stop with its choice between finishing and interrupting the turn, and the details view; the ended reason with "Start again"; the indication of OMP's own modes; the chip glyph; and the same three confirmations as the desktop. Check the layout in Chromium at 360 to 390 px.
- [ ] 7.2 Show autopilot in the All projects cards and the Idle sessions list as on desktop.
- [ ] 7.3 Test each, including that the composer stays usable and that nothing is queued offline.

## 8. Verify, document, hand over

- [ ] 8.1 Run the Rust, protocol, web and native gates and `openspec validate --strict`; native must still pass unchanged.
- [ ] 8.2 Run the check of 1.1 against the built extension: start, both timings, each limit including the time limit during a turn and while waiting, the failure pause, an Esc interrupt, pause and resume, Stop and Stop now including a submission that starts late, restart to paused, a change of conversation, both holds and the pause after an hour, and the reporting of OMP's own modes.
- [ ] 8.3 Run a two-host smoke on isolated servers and daemons: a request reaches only the tab's machine, the stopped-machine and older-machine refusals, an older viewer unaffected, the re-report after a server restart, and a server rolled back by itself.
- [ ] 8.4 On the same isolated setup, run one autopilot end to end, from a browser through the server and the CLI to the real extension and OMP, on the desktop and at phone width.
- [ ] 8.5 Add the autopilot section to `AGENTS.md`, including how a prompt is handled (slash commands and skills are not run; OMP's keywords, `@path` mentions and a `+500k`-style token apply) and OMP's `retry.waitForUsageReset`. Correct the sentences this change makes incomplete: that interactive panes are controlled by a person, the retired-bot paragraph, the pane status dots, "recurring work belongs to autonomous workers", and that the extension's reports contain no conversation. Run `python3 scripts/check_agent_runbooks.py`, and record what was and was not exercised.
- [ ] 8.6 With its owner's agreement, add the applicability note to `unify-autonomous-workers` and retitle its one requirement; if that change has been archived, add a RENAMED delta here instead.
- [ ] 8.7 Leave deployment, daemon reboots and agent reboots as separate actions that each need the owner's go-ahead.
