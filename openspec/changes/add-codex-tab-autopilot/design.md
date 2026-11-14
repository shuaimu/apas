## Context

See proposal.md for why. OMP autopilot (archived change `2026-11-13-add-omp-tab-autopilot`, main spec `omp-tab-autopilot`) runs its loop inside the OMP process; the server, wire frames, web control, start dialog and templates are already provider-neutral apart from the checks that a tab is an OMP terminal tab.

Codex tabs today run the Codex TUI in a pane host. Codex 0.159 and later run conversations in a shared, per-user app-server daemon that the TUI is a client of; APAS already connects to that daemon's control socket to find each tab's conversation (`transcript.rs`, the managed-daemon locator). On zoo-005 the TUI binary is 0.160.0 and the shared daemon is 0.166.0: the daemon updates itself separately.

Evidence for every decision below is the 2026-11-14 provider check: `~/.cache/apas-codex-check/NOTES.md` (codex 0.160.0 with a stub model, isolated; the shared daemon was never touched) and the source of tag `rust-v0.160.0`.

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
- **Typing into the PTY**: rejected for the same reasons as on OMP (blind to the TUI's state).

### 2. The loop runs in the project instance, with a private state file
Unlike OMP, Codex has no place inside the tab to run code. The loop runs in the APAS project instance that hosts the tab, one task per Codex tab on autopilot, and keeps its state (spec, iteration count, failures, condition, the conversation id) in a private file in the pane's runtime directory, written by temp-and-rename. The state report and journal fields are the same `PaneAutopilotState` the OMP extension reports, so the server and web need no new frame shape.

A daemon reboot re-execs the CLI and ends the loop. The new instance reads the state file and reports the autopilot paused ("APAS on <host> restarted"), never running: the same rule as an OMP agent restart, and it avoids double submission across the handoff.

### 3. Submission rules
- Submit only on `thread/status/changed` idle (or `thread/read` status idle), with no `waitingOnUserInput` or `waitingOnApproval` flag, and `thread/goal/get` not active.
- `turn/start` carries `clientUserMessageId = "apas-autopilot:<autopilot id>:<n>"`; the rollout keeps it as `client_id` on the user message item, which is how the transcript reader shows iteration markers and how a restarted instance recognises its own last iteration.
- `turn/start` on a conversation whose turn is already running steers into that turn and returns its id. When the returned turn id is not a new turn, the submission is not counted and the loop waits for that turn's `turn/completed`.
- `turn/completed.turn.status`: `completed` → done; `failed` → failed iteration; `interrupted` → a person's interrupt unless the loop sent `turn/interrupt` for that turn.
- `turn/interrupt` gets a bounded wait: on 0.160 an interrupt of a turn that has already ended never answers.
- `thread/resume` fails until the conversation has a first turn ("no rollout found"). For a fresh tab the first iteration is sent unsubscribed and completion is observed through `thread/status/changed` and `thread/read`.

### 4. Identifying the tab's conversation
The existing Codex locator already binds a tab to its conversation id (from the TUI's MCP origin on the shared daemon). The loop uses that id and treats a change of the tab's bound conversation (`/new`, resume) as the end of the autopilot. On a per-tab server (decision 5, option B) the server holds only that tab's conversations, filtered by `ephemeral == false` and `threadSource == "user"` to skip title-generation threads.

### 5. Transport: OPEN, decided by task 1
- **Option A, the shared daemon.** Connect to the control socket APAS already queries. No change to how Codex tabs are launched, so running tabs could gain autopilot without an agent reboot. Unknowns: nothing was run against a real shared daemon or against 0.166; whether `canAcceptDirectInput` is true for a TUI-owned conversation there; whether a second client's turns render live in the TUI; whether the daemon's other clients (the user's non-APAS sessions) are affected. The daemon updates itself, so its version is outside APAS's control.
- **Option B, a private app-server per tab.** Launch `codex app-server --listen ws://127.0.0.1:<port> --ws-auth capability-token --ws-token-file <0600>` beside the pane host and the TUI with `--remote ws://… --remote-auth-token-env … -C <cwd>`. Proven on 0.160: live rendering, Escape status, client ids, two tabs isolated. Costs: an existing tab moves only at its next agent launch, and that launch meets `$CODEX_HOME/thread-writer-locks/<thread>.lock` held by the shared daemon for 60 s after its last subscriber leaves, behind a dialog that needs a keypress; each tab runs its own MCP servers and auth refresh against the shared `auth.json`; APAS's shared-daemon locator no longer applies to such a tab; a loopback ws server needs its capability token or any local user could drive the agent.

Recommendation: A if task 1 shows it behaves as B did on 0.160, because it fits what APAS already does and needs no migration; B otherwise.

### 6. Capabilities and rollout
The CLI advertises `codex_tab_autopilot_v1`, echoed by the server in `Registered`; viewers negotiate a new `codex_tab_autopilot_v1` marker, and the server sends Codex autopilot frames only to viewers that did, so the web deployed today (which assumes OMP) never receives one. Deploy server, web, then CLI.

## Risks / Trade-offs

- [The shared daemon's version changes without APAS] → the instance records the daemon's version at connection and reports a version it has not been checked against as unsupported; the real-provider check is repeated when Codex's minor version changes, as it is for OMP.
- [A person's typing races an iteration] → the submission may land inside the person's turn as extra text. It is not counted, and the loop waits; this is stated in the runbook. The race window is the time between the idle notification and `turn/start`.
- [Hidden server requests] → autopilot never answers approvals or questions; a pending request is not re-sent to a client that subscribes later, so the loop relies on the status flags, not on seeing the request.
- [Unanswered interrupts and stale "active" errors] → bounded waits; a not-confirmed Stop now is reported as such.
- [Daemon reboot pauses every Codex autopilot on the machine] → accepted: resuming is one click, and it is the only way to guarantee no double submission.
- [Option B lock-out] → only matters if B is chosen; then the first agent launch of each existing tab must wait out or detect the lock dialog.
- Not covered by the 0.160 check and to be covered by task 1 or later: an in-flight turn across an app-server restart, a failed turn status, approvals, mid-turn compaction, the managed `allow_managed_hooks_only` policy (irrelevant to option A).

## Migration Plan

Deploy server → web → CLI. Existing Codex tabs: under option A they gain autopilot when their machine's daemon is rebooted; under option B, at their next agent launch. Rollback: stop running Codex autopilots, then roll back the CLI; a server rolled back alone makes Codex autopilots hold, then pause.

## Open Questions

1. Option A or B (decision 5). Gated on task 1.
2. Where task 1 may run against a real shared daemon without touching the owner's: a separate Unix account, a host where the owner's daemon is not running, or, with the owner's consent, a test daemon with its own `CODEX_HOME` on zoo-005, whose socket file would sit in the owner's `/tmp/codex-daemon-<uid>/` directory beside the real one.
