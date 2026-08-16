## Why

Replacing the daemon used to be nearly free. It owned no long-lived children —
project CLIs lived in their own tmux sessions and pane hosts owned the PTYs — so
a self-upgrade every 15 minutes was a good trade: unattended hosts stayed
current, and nothing running noticed. zoo-002 sitting nine versions behind was
the problem it solved.

Projects now run *inside* the daemon. Replacing it is no longer invisible; it is
the same act as stopping every project on that host and starting them again.
Doing that on a timer, with nobody watching, is not a trade anyone chose.

Two paths do it today, and the quieter one is the more destructive:

- The **15-minute tick** re-execs when the installed binary is newer. `exec`
  preserves the pid, the claims, and the resume manifest, so the projects come
  back — but on a schedule nobody asked for.
- **A launch** (`apas` in a project directory) whose binary is newer than the
  running daemon calls `stop_daemon_process`, which sends SIGTERM, waits four
  seconds, then SIGKILL. There is no `exec`, no manifest, and no handoff. And
  SIGTERM is not even handled — the signal handler is registered for SIGINT
  only — so the daemon dies at once and takes every project with it, with no
  pane roster saved and agent subtrees left behind. Typing `apas` in a directory
  is enough to do this to a colleague's running work.

The replacement for both already exists and shipped this week: restarting a
daemon from the Machines page, which applies an available update first and now
says whether it will.

## What Changes

- **The daemon no longer upgrades itself on a timer.** The 15-minute check is
  removed; a machine's version changes when someone asks for it.
- **A launch no longer stops a running daemon to upgrade it.** It reports that
  the daemon is older and where to update it, and leaves it running. Starting a
  daemon when none is running is unchanged.
- **A daemon asked to stop stops its projects first.** The termination signal is
  handled, not only interrupt, so the shutdown path that saves each project's
  pane roster and ends its agents actually runs when something stops the daemon.
- **BREAKING**: a host left alone keeps its version indefinitely. Upgrades are
  now something a person does, from the Machines page or by restarting the
  daemon on the host.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `host-supervision`: the resident instance gains a rule about its own
  replacement — that it is never replaced automatically while it hosts projects,
  and that stopping it stops those projects properly rather than dropping them.

## Impact

- `crates/client-cli/src/mode/daemon.rs`: the upgrade interval and its tick arm
  are removed; the requested-restart path is untouched.
- `crates/client-cli/src/main.rs`: the version branch in `ensure_daemon_running`
  stops killing the running daemon and reports instead.
- `crates/client-cli/Cargo.toml`: the signal handler covers termination, so an
  ordinary stop reaches the shutdown path rather than killing the process
  outright.
- `crates/client-cli/src/update.rs`: helpers that existed only for the timer
  (`newer_installed_version`, `apas_binary_fingerprint`) lose their caller.
- Docs: the "daemon upgrades itself" section of `CLAUDE.md`, which currently
  describes the behaviour being removed and the reasoning that no longer holds.
- Operationally: installing a binary to the shared path no longer propagates on
  its own. Rolling it out means restarting each daemon, which the Machines page
  now labels per machine.
