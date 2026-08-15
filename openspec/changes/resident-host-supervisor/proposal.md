## Why

The CLI is no longer the agent's parent. Pane hosts own the PTYs in their own tmux sessions, the daemon owns no long-lived children, and a self-upgrade `exec`s in place. "Daemon" and "project CLI" have converged into the same thing — a process that connects to the server and supervises work it does not own — and the duplication shows in how they find each other: they don't. They infer each other's existence from `/proc`.

That inference is the source of the awkward machinery around them. `headless_pid_for` matches a `-d <path>` substring in another process's command line. `snapshot_projects` reports `is_running` from `/proc` rather than from the daemon's own map, because the map and reality drift. `reconcile_running_claims` exists because a restarted daemon comes back owning nothing while its projects keep running. And nothing guards the interactive path at all: `apas` in a project directory calls `ensure_daemon_running` and goes straight into `dual_pane` without ever asking whether that project is already running here — `is_headless_running_for` guards only the daemon's own spawns. Two CLIs for one project, one `.apas`, and one set of worktrees is reachable today by typing `apas` in the wrong directory.

## What Changes

- A host has **one resident supervisor**. It is the process `apas daemon` already starts; it gains an authoritative record of which projects are running here and a real channel to each of them.
- **Project workers become the supervisor's known children rather than strangers.** Each keeps its own process and its own tmux session — one project must not be able to take down the host's supervision — but it is reached over a Unix socket instead of located by scanning `/proc`.
- **`apas` in a project directory attaches instead of competing.** It asks the resident supervisor to ensure that project is running and then renders its TUI against it. Closing the terminal leaves the project running, and typing `apas` twice attaches twice rather than starting a second CLI over the same `.apas` and worktrees.
- **Intra-host truth comes from the supervisor.** `/proc` scanning stops being the source of "is this project running", and the in-memory map stops being something that can silently disagree with it.
- **Cross-host claims stay.** They solve a different problem — two hosts sharing one NFS home must not both run a project — which one resident process per host does not address.
- **The surface does not move.** `apas`, `apas daemon`, `apas --headless`, the Machines page, and the lifecycle controls behave exactly as they do now. This is an internal consolidation, and a user should not be able to tell the difference except that the duplicate-CLI foot-gun is gone.

## Capabilities

### New Capabilities

- `host-supervision`: the resident per-host supervisor — single ownership of the host's running projects, an authoritative running-state record, attachment by a project CLI, and what happens when the supervisor or a worker dies.

### Modified Capabilities

- `cli-lifecycle-control`: reboot and transport recovery are expressed against the supervisor's record of a project rather than against a `/proc` match, and an attached CLI's exit is distinguished from the project stopping.

## Impact

- `crates/client-cli/src/mode/daemon.rs`: gains the supervisor socket, the project table, and worker supervision; loses `headless_pid_for` / `is_headless_running_for` as the intra-host source of truth.
- `crates/client-cli/src/main.rs`: the default project path becomes attach-or-start against the supervisor rather than an unconditional `dual_pane` launch.
- `crates/client-cli/src/pane_host.rs`: its runtime-directory, credential, socket, and adoption-grace machinery is the proven pattern this reuses rather than reinvents.
- `crates/client-cli/src/daemon_registry.rs`: cross-host claims stay; the intra-host reconciliation that existed to paper over `/proc` drift goes away.
- No server or web change: the Machines page already derives project state from what the daemon reports.
- Docs: the daemon and pane-host sections of `CLAUDE.md`, which currently document `/proc`-derived running state as deliberate.
