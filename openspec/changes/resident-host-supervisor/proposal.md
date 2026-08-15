## Why

The CLI is no longer the agent's parent. Pane hosts own the PTYs in their own tmux sessions, the daemon owns no long-lived children, and a self-upgrade `exec`s in place. "Daemon" and "project CLI" have converged into the same thing — a process that connects to the server and supervises work it does not own — and the duplication shows in how they find each other: they don't. They infer each other's existence from `/proc`.

Nothing guards the interactive path at all: `apas` in a project directory calls `ensure_daemon_running` and goes straight into `dual_pane` without ever asking whether that project is already running here — `is_headless_running_for` guards only the daemon's own spawns. Two CLIs for one project, one `.apas`, and one set of worktrees is reachable today by typing `apas` in the wrong directory.

The fix is not to make the second process cooperate with the first. It is to not have a second process. `detect_running_daemon` already implements exactly the check — a pid state file, a verification that the process really is `apas`, and cleanup of a stale record — and `apas daemon` already just quits when one is running. That rule simply was never applied to plain `apas`.

## What Changes

- **A user launches at most one `apas` instance per host.** Running it again does not start anything: it defers to the instance that is already there and exits. This is the existing `apas daemon` behaviour, applied to the command people actually type.
- **`apas` in a project directory registers that project and exits**, pointing at the web UI. The project becomes visible on the Machines page, where it is started.
- **Projects are launched from the web.** `apas` no longer starts one as a side effect of being run in its directory.
- **The interactive TUI goes away by default.** It displays nothing but tab names, and under a single-instance rule it would essentially never run, since `ensure_daemon_running` means an instance almost always exists. It stays reachable behind an explicit `--attach` for a local view when the web is unavailable.
- **Headless project workers are unchanged.** The daemon still spawns one per project in its own tmux session. They are its children, not instances a user launched, so the rule does not apply to them.
- **Cross-host claims are unchanged.** They decide which of several hosts sharing one NFS home may run a project — a different question from how many instances one user has on one host.
- **BREAKING**: two visible losses. `apas` no longer opens a terminal UI, and it no longer starts the project in the directory it was run from.

## Capabilities

### New Capabilities

- `host-supervision`: one resident instance per host — how a second launch defers to it, what still runs when it is gone, and why this does not replace cross-host exclusion.

### Modified Capabilities

(none)

## Impact

- `crates/client-cli/src/main.rs`: the default project path becomes register-and-defer instead of launching `dual_pane`; the existing singleton check is reused rather than a second one written.
- `crates/client-cli/src/mode/dual_pane.rs`: the interactive branch stops being the default path. The `headless` flag's other half is what every project now runs as.
- `crates/client-cli/src/attach.rs`: retained behind `--attach` only. With no default caller it is a candidate for deletion, and it should be deleted rather than carried indefinitely if `--attach` finds no use.
- No server or web change: the Machines page already starts and stops projects.
- Docs: the daemon section of `CLAUDE.md`, and whatever tells people to run `apas` in a project directory to start it.
