## Context

See proposal.md — Why. The current process model per host:

- **One daemon** (`apas daemon`): host registration, machine listing, project start/stop, cross-host claims in an NFS-shared registry, self-upgrade by `exec` every 15 minutes.
- **N project workers** (`apas --headless -d <path>`), each in its own tmux session on its own tmux socket, spawned by the daemon.
- **M pane hosts** (`apas pane-host`), each in a project-scoped tmux session, owning one PTY.
- Optionally an **interactive `apas`**, which runs `dual_pane` in the foreground and is nobody's child.

Two facts decided the approach:

- **The singleton check already exists.** `detect_running_daemon` reads a pid state file, verifies the process really is `apas`, and deletes the record when it is stale. `Commands::Daemon` already prints and returns when one is running. Nothing of this had to be designed; it had to be applied to the command people type.
- **The TUI is not worth preserving.** `App` holds only `output_rx: Receiver<PaneOutput>` and `command_rx: Receiver<TuiCommand>` — the `input_tx` and `event_tx` handed to `App::new` are dropped immediately — and it renders little beyond tab names. That is what made attaching *possible* (a snapshot plus two one-way streams, no input path); it is also why attaching is not worth building.

## Goals / Non-Goals

**Goals:**

- One user-launched instance per host, enforced where a person can actually trip over it.
- A launch that finds one already running does something useful — registers the project — rather than failing or duplicating.
- No new IPC, no new protocol, no new runtime state.

**Non-Goals:**

- Running every project inside one process. One project's panic must not take down the host, and `dual_pane` is ~14.9k lines of per-project state; making it multi-project in-process would be a rewrite with a far worse failure mode than the bug it fixes.
- Removing cross-host claims, which answer a different question.
- Preserving the interactive TUI as the default experience.
- A supervisor control socket, per-project worker sockets, or attachment. The earlier draft of this change specified all three; the single-instance rule removes the problem they were solving.

## Decisions

1. **The rule is enforced by refusing to be a second instance, not by cooperating with the first.** A second launch defers and exits. This is a smaller change than any protocol between the two, and it cannot drift: there is no second process left to disagree with anything.

2. **Reuse `detect_running_daemon` rather than write a second check.** It already handles the stale-record case, which is the only genuinely fiddly part — a pid file outlives a crash, and a check that trusted it would refuse to ever start again.

3. **A deferring launch registers the project.** Otherwise `apas` in a fresh project directory would appear to do nothing at all, and the project would never become visible to manage. Registration is what makes "launch from the web" workable rather than a dead end.

4. **Projects are started from the web, not by being run in.** `start_project` is only reached from instance creation and an explicit start from the Machines page; making a deferring launch also *start* the project would add a request path for a convenience the web already provides.

5. **Headless workers are out of scope for the rule.** They are `apas` processes, but they are the daemon's children rather than instances a user launched. Applying the rule to them would stop the daemon running more than one project.

6. **The TUI stays behind `--attach`, and is expected to be deleted.** The attachment machinery exists and is tested, so keeping it costs nothing today. It has no default caller, which makes it a liability rather than an asset over time: it should go if `--attach` finds no use, not be carried indefinitely because it was built.

7. **The `/proc` probe stays.** The earlier draft retired it in favour of an authoritative table reachable over a socket. With no socket there is no table, and `is_headless_running_for` remains how the daemon avoids spawning a duplicate for a project that survived it.

## Risks / Trade-offs

- [Someone types `apas` expecting it to start their project and it exits instead] → It prints what it did and where to go. This is the visible cost of the change, and it should be in the release note rather than discovered.
- [Losing the TUI removes the only local view when the server is unreachable] → `--attach` remains for that case. If it turns out to matter, the evidence will be people using it; if nobody does, that is the signal to delete it.
- [One instance per user per host is not one per project directory] → Intentional. A user working in three project directories registers three projects and manages them from one place, rather than running three processes that cannot see each other.
- [The rule keys on the user, so two accounts on one host still get two instances] → Correct: their projects, claims, and runtime directories are already per-user, and merging them would be a different and worse change.

## Migration Plan

Behaviour-preserving for everything except the two documented losses, and self-upgrading: hosts pick it up from the shared binary, and each daemon re-execs into it on its own upgrade tick. A host part-way through the rollout has an older `apas` that still opens a TUI and a newer one that defers; both leave the running projects alone. Rollback is a binary swap.
