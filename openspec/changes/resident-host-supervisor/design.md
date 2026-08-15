## Context

See proposal.md — Why. The current process model per host:

- **One daemon** (`apas daemon`): host registration, machine listing, project start/stop, cross-host claims in an NFS-shared registry, self-upgrade by `exec` every 15 minutes.
- **N project CLIs** (`apas --headless -d <path>`), each in its own tmux session on its own tmux socket, spawned by the daemon but not owned by it.
- **M pane hosts** (`apas pane-host`), each in a project-scoped tmux session, owning one PTY.
- Optionally an **interactive `apas`**, which runs `dual_pane` in the foreground and is nobody's child.

The daemon finds project CLIs by scanning `/proc` for a `-d <path>` match (`headless_pid_for`), which is also how `snapshot_projects` answers `is_running`. `mode/dual_pane.rs` is ~14.9k lines and holds everything a project does: panes, worktrees, team files, transcripts, MCP servers, the server WebSocket.

Two pieces of existing machinery matter because this reuses rather than invents them: `pane_host.rs` already runs a supervised child behind a Unix socket with a runtime directory, an owner-only credential, an adoption grace period, and a reboot handoff. `daemon_registry.rs` already solves cross-host exclusion over NFS.

## Goals / Non-Goals

**Goals:**

- One process per host that knows, without inspecting strangers, which projects are running there.
- A project CLI that attaches to a project instead of becoming a second owner of it.
- Identical surface: same commands, same Machines page, same lifecycle controls.
- No regression in what already survives today — pane hosts, terminal panes, and headless projects keep surviving CLI and daemon replacement.

**Non-Goals:**

- Running every project inside the supervisor process. One project's panic must not take down the host's supervision, and `dual_pane` is far too large to make process-shared safely in one change.
- Removing cross-host claims, which solve a problem per-host supervision does not.
- Changing what a project *does* once running: panes, worktrees, team mode, and transcripts are untouched.
- A new user-visible command or page.

## Decisions

1. **The supervisor is the daemon, extended — not a new process.** It gains a Unix socket, a project table, and worker supervision. Alternative considered: a new `apas supervisor` process alongside the daemon — rejected, because it would add a third role to a change whose entire purpose is that there are already two too many.

2. **Project workers stay separate processes, reached over a socket.** They keep their own tmux session, so a crash or an OOM kill is contained to one project, and the supervisor keeps a socket to each rather than a `/proc` match. This is the pane-host relationship one level up, and it is deliberately the same shape: runtime directory, owner-only credential, adoption grace, tombstone on deliberate stop. Alternative considered: running projects as threads inside the supervisor — rejected on blast radius and on the size of `dual_pane`.

3. **`apas <project>` becomes attach-or-start.** It asks the supervisor for the project, which starts a worker if there is none, then renders the TUI against that worker. This is what removes the duplicate-CLI foot-gun, and it is also what makes closing a terminal harmless. The interactive process stops being an owner and becomes a viewer.

4. **The supervisor's table is the only intra-host answer to "is it running".** `headless_pid_for` stops being consulted for state. It survives only as an *adoption* probe at supervisor startup — finding workers that outlived a supervisor is exactly the case where there is no socket to ask yet.

5. **Adoption, not duplication, on supervisor restart.** A starting supervisor reconnects to the sockets of workers already running before it creates any. This subsumes `reconcile_running_claims`: that function exists because a restarted daemon came back owning nothing, which stops being true once ownership is a socket rather than a memory of having spawned something.

6. **Cross-host claims are untouched.** They gate *whether this host may run the project at all*; the supervisor gates *how many workers exist here*. Collapsing the two would reintroduce the multi-host duplication the claim system was written for.

7. **Rollout is by ordinary self-upgrade, and the two models coexist.** A supervisor that finds pre-existing workers adopts them by probe rather than by socket, so a host mid-upgrade is a host with some adopted-by-probe workers and some socket-attached ones. That has to work anyway for crash recovery, so it is not extra machinery for the rollout.

## Risks / Trade-offs

- [The attach path is new for the interactive TUI, which today reads its own in-process state] → The TUI already renders from the server's stream for remote panes; attaching means rendering from the worker's stream instead of from its own. Where that is not yet true, the honest scope is to make the attached CLI a thin renderer, and any state it cannot get over the socket is a gap to find early rather than to discover at the end.
- [One socket per project is more moving parts than one `/proc` scan] → It is also the difference between knowing and guessing, and pane-host already proves the pattern in this codebase. The parts are bounded: a runtime directory per project, cleaned on deliberate stop, self-terminating on lease expiry.
- [A partially upgraded host runs both models at once] → Adoption-by-probe is required for crash recovery regardless, so the mixed state is the recovery path, exercised rather than special.
- [Behaviour people rely on could change silently: closing a terminal no longer stops a project] → It already does not stop pane hosts; making it explicit for projects is the point. Worth calling out in the release note, because "I closed the window" has been an informal way to stop things.
- [`dual_pane` is 14.9k lines and every seam is load-bearing] → Nothing about what a project does changes here; only who starts it and who is told about it. The change should be provable by the existing project tests continuing to pass untouched.

## Migration Plan

Behaviour-preserving and self-upgrading: hosts pick it up on `apas update` or the next Reboot CLI, both of which already leave pane hosts running. A host running the old model keeps working; a host running the new one adopts whatever it finds. Rollback is a binary swap, after which projects started by a supervisor are once again found by `/proc` — which is precisely why decision 4 keeps that probe rather than deleting it.

## Open Questions

- Whether an attached CLI should follow a rebooted project automatically or report that the attachment ended. Both satisfy the spec; the choice is a UX preference that can be made when the reboot path is wired, and it does not change the socket design.
