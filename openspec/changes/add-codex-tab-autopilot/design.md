## Context

See proposal.md for why. OMP autopilot (archived change `2026-11-13-add-omp-tab-autopilot`, main spec `omp-tab-autopilot`) runs its loop inside the OMP process; the server, wire frames, web control, start dialog and templates are already provider-neutral apart from the checks that a tab is an OMP terminal tab.

Codex tabs today run the Codex TUI in a pane host. Codex 0.159 and later run conversations in a shared, per-user app-server daemon that the TUI is a client of; APAS already connects to that daemon's control socket to find each tab's conversation (`transcript.rs`, the managed-daemon locator). On zoo-005 the TUI binary is 0.160.0 and the shared daemon is 0.166.0: the daemon updates itself separately.

Evidence is the 2026-11-14 provider checks in `~/.cache/apas-codex-check/NOTES.md`, both with a stub model and no credential: codex 0.160.0 against a private ws app-server, and (task 1.2, with the owner's consent) the owner's TUI version 0.160.0 attached to a private managed daemon of 0.166.0 and 0.165.0 on zoo-005, whose socket sat in `/tmp/codex-daemon-3000/` under its own hash. The owner's own daemon, `~/.codex` and sessions were never touched (independent before/after comparison identical). Also the source of tag `rust-v0.160.0`.

## Goals / Non-Goals

**Goals:**
- The same person-facing autopilot on Codex tabs as on OMP tabs, through the same controls and wire frames.
- Nothing typed into the terminal; no change to the user's shared `~/.codex` configuration, hooks or trust state.
- Correct behaviour around a person's own use of the tab: their typing, Escape, questions, goal mode, new conversations.

**Non-Goals:**
- Claude Code tabs (a separate change; its mechanism is a Stop hook, which suits Claude's turn model and not Codex's).
- Answering Codex's questions or approvals on the person's behalf. Autopilot holds while one waits.
- Switching Codex's goal mode, or using it as the loop.
- Native mobile controls (none exist for OMP either).

## Decisions

### 1. The engine is Codex's app-server protocol, not a Stop hook
APAS acts as a second JSON-RPC client of the app-server that hosts the tab's conversation: `initialize` (client name `codex_app_server_daemon`), `thread/resume {threadId, excludeTurns: true}` to subscribe, `turn/start` to submit, `turn/interrupt` for Stop now, and the notifications `thread/status/changed`, `turn/started`, `turn/completed`, `thread/goal/updated|cleared`.

Alternatives:
- **Stop hook** (proven to continue a session): rejected. The continuation stays inside one turn, so there is no `turn/completed` per iteration and no compaction between text-only iterations; it cannot start an iteration from idle (the first one, a resume, an interval); prompts over about 10 KB are cut to a preview and written to `$TMPDIR/hook_outputs` with mode 0644; any `-c hooks` setting forces the tab off the shared daemon; and `--dangerously-bypass-hook-trust` would also trust repository hooks, so it must never be used.
- **`codex queue` / `thread/queue/*`**: rejected. A queued item is not dispatched after an Escape and stays in the queue for later; dispatch was about 15 s late; the API is experimental.
- **Typing into the PTY** and reading the rollout to observe turns: rejected by the owner on 2026-11-14 after a side-by-side comparison. It would work today for Codex, Claude and OpenCode with no protocol dependency, and Codex tabs have no approval prompts to land on, but it is blind to menus and to a person's half-typed text, cannot mark iterations, learns of idleness with a 3-second lag and cannot tell a failed turn apart. The control socket was chosen as the more informative and reliable route.

### 2. The loop runs in the project instance, with a private state file
Unlike OMP, Codex has no place inside the tab to run code. The loop runs in the APAS project instance that hosts the tab, one task per Codex tab on autopilot, and keeps its state (spec, iteration count, failures, condition, the conversation id) in a private file in the pane's runtime directory, written by temp-and-rename. The state report and journal fields are the same `PaneAutopilotState` the OMP extension reports, so the server and web need no new frame shape.

A daemon reboot re-execs the CLI and ends the loop. The new instance reads the state file and reports the autopilot paused ("APAS on <host> restarted"), never running: the same rule as an OMP agent restart, and it avoids double submission across the handoff.

### 3. Submission rules
- Submit only while the conversation is not running a turn: status `idle`, or `systemError`, where a conversation rests after a failed turn (status goes active → systemError with no idle; `turn/start` is accepted there and a successful turn returns it to idle). Never submit with a `waitingOnUserInput` or `waitingOnApproval` flag, or while `thread/goal/get` is active. Status and goal notifications reach every connected client for every conversation, so filter by conversation id.
- `turn/start` carries `clientUserMessageId = "apas-autopilot:<autopilot id>:<n>"`; the rollout keeps it as `client_id` on the user message item, which is how the transcript reader shows iteration markers and how a restarted instance recognises its own last iteration.
- `turn/start` on a conversation whose turn is already running steers into that turn and returns its id. When the returned turn id is not a new turn, the submission is not counted and the loop waits for that turn's `turn/completed`.
- `turn/completed.turn.status`: `completed` → done; `failed` → failed iteration (an `error` notification with `willRetry: false` precedes it); `interrupted` seen live → a person's interrupt, unless the loop sent `turn/interrupt` for that turn. A turn found `interrupted` only by reconciling after the loop lost its connection while the turn ran is NOT a person's interrupt: a crashed daemon's in-flight turn reads exactly that way. Reconcile every reconnect with `thread/read {includeTurns: true}` by the last turn's client id and status, because a graceful daemon restart drains and completes the turn but the `turn/completed` frame can be lost.
- Never pass `collaborationMode`, `approvalPolicy`, `sandboxPolicy` or `model` to `turn/start`: they persist for the conversation (`thread/settings/updated`).
- Server requests (questions, approvals) are delivered to every subscribed client, including APAS, whoever started the turn. APAS never answers them; the person answers in the TUI, which also receives them.
- A person typing while an iteration runs steers into that turn; their message appears in it with their own client id. That turn still counts as the iteration.
- `turn/interrupt` gets a bounded wait: on 0.160 an interrupt of a turn that has already ended never answers.
- `thread/resume` fails until the conversation has a first turn ("no rollout found"). For a fresh tab the first iteration is sent unsubscribed and completion is observed through `thread/status/changed` and `thread/read`.

### 4. Identifying the tab's conversation
The existing locator (`transcript/codex_daemon.rs`) binds a tab only after the conversation's first turn: before it, `thread/read.path` names a rollout file that does not exist yet and the locator skips the thread, although the TUI's `codex_tui` MCP origin is already connected. The autopilot binds through that origin, owned by the tab's process group, without requiring the file, so the first iteration can start a fresh tab. `/new` does not move the origin: the old conversation keeps it, stays loaded while a client is subscribed and accepts turns silently. The end of the autopilot is therefore detected from the broadcast `thread/started` of a new user conversation (`threadSource: "user"`, not ephemeral) with the same origin and cwd, and the loop never submits to the previous conversation after that. On a per-tab server (decision 5, option B) the server would hold only that tab's conversations. The locator itself initializes as `apas_transcript`, which rewrites the daemon-wide User-Agent suffix of the owner's own model requests until another client connects; it must initialize as `codex_app_server_daemon` like the loop (task 2.3).

### 5. Transport: OPEN, decided by task 1
- **Option A, the shared daemon. CHOSEN (2026-11-14).** Connect to the control socket APAS already queries. No change to how Codex tabs are launched, so running tabs gain autopilot at their machine's daemon reboot. Checked on 0.166.0 (and 0.165.0) with a 0.160.0 TUI: `canAcceptDirectInput` is true for a TUI-owned conversation; a turn started by APAS renders live in the TUI, byte-exact for the 3,703-byte default template, and keeps its client id in the rollout and the model request, also on a `codex resume` tab; Escape gives interrupted and a provider error gives failed; another tab on the daemon sees no turn or item frames of it.
- **Option B, a private app-server per tab.** Not chosen. Proven on 0.160 only, and moving a running tab costs a 60-second conversation lock (measured: 58 s after the last subscriber on 0.166) behind a "press r to retry" dialog, a separate Codex stack per tab, and the loss of APAS's shared-daemon locator.

### 6. Capabilities and rollout
The CLI advertises `codex_tab_autopilot_v1`, echoed by the server in `Registered`; viewers negotiate a new `codex_tab_autopilot_v1` marker, and the server sends Codex autopilot frames only to viewers that did, so the web deployed today (which assumes OMP) never receives one. Deploy server, web, then CLI.

## Risks / Trade-offs

- [The shared daemon's version changes without APAS] → the owner's daemon runs a pid-update-loop and 0.167.0 to 0.169.0 are already on disk, so the next daemon restart will probably run a version that was not checked. `thread/read.cliVersion` and the `initialize` user agent report the daemon's version, not the TUI's; the instance records both (`codex app-server daemon version` reports `cliVersion` and `appServerVersion`) and reports a combination it has not been checked against as unsupported. The provider check is repeated when either minor version changes, as for OMP.
- [The daemon is fragile outside a graceful restart, and APAS does not control it] → a draining daemon refuses `thread/resume` and `turn/start` with `serverShuttingDown` (autopilot holds and retries after reconnecting); after a crash nothing restarts it, even with `daemon_auto_start`, and TUIs give up reconnecting after about two minutes and need an agent reboot (autopilot holds, then pauses after four hours); `codex app-server daemon start` drops the saved feature overrides, after which a 0.160 TUI shows a blocking "incompatible feature settings" dialog whose default is Cancel. APAS never starts, stops or restarts the shared daemon.
- [A person's typing races an iteration] → the submission may land inside the person's turn as extra text. It is not counted, and the loop waits; this is stated in the runbook. The race window is the time between the idle notification and `turn/start`.
- [Hidden server requests] → autopilot never answers approvals or questions; a pending request is not re-sent to a client that subscribes later, so the loop relies on the status flags, not on seeing the request.
- [Unanswered interrupts and stale "active" errors] → bounded waits; a not-confirmed Stop now is reported as such.
- [Daemon reboot pauses every Codex autopilot on the machine] → accepted: resuming is one click, and it is the only way to guarantee no double submission.
- [Option B lock-out] → only matters if B is chosen; then the first agent launch of each existing tab must wait out or detect the lock dialog.
- Covered by task 1.2: an in-flight turn across a graceful restart (drained and completed; `turn/completed` may be lost) and across a crash (reads interrupted); a failed turn (failed, then systemError); questions and approvals (delivered to every subscribed client). Not covered: mid-turn compaction, and the owner's single host-local SQLite home shared by the TUIs and the daemon (the harness, on NFS, needed one SQLite home per process).

## Migration Plan

Deploy server → web → CLI. Existing Codex tabs gain autopilot when their machine's daemon is rebooted from the Machines page; no agent reboot is needed, because nothing about how a Codex tab is launched changes. Rollback: stop running Codex autopilots, then roll back the CLI; a server rolled back alone makes Codex autopilots hold, then pause.

## Open Questions

None. Option A was chosen with the owner after task 1.2 (2026-11-14). Task 1.2 ran on zoo-005 against a private managed daemon with its own `CODEX_HOME`, with the owner's consent.
