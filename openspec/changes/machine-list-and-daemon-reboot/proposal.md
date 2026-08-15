## Why

A daemon is per machine, not per project, so the control that restarts one does not belong on a project row. The mobile home lists projects and filters them by whether they are idle; there is nowhere to see the machines those projects run on, and no way to act on one.

There is also no way to restart a daemon at all, from anywhere. The daemon replaces itself only when it notices a newer binary already installed on that host, which someone has to put there by running `apas update` or rebooting a project CLI over an SSH session. So the one machine-wide process in APAS is the one thing that cannot be managed from APAS — including from a phone, where SSH is the least available option and the need is most likely.

That is safe to fix now for the same reason the per-project reboot was: the daemon owns no long-lived children. Headless project CLIs live in their own tmux sessions, pane hosts own their PTYs, and a self-upgrade `exec`s in place precisely so its destructors never run. Restarting a daemon disturbs no agent.

## What Changes

- The mobile home gains a **Machines** list alongside the existing project filters, listing the machines the account can reach with their hostname, platform, connection state, and how many projects are running on each. The data already arrives in the mobile bootstrap and is currently discarded.
- Each machine gains a **reboot control for its daemon**, confirmed before it fires and named so it is unmistakable which host it targets.
- A **daemon reboot exists in the protocol** for the first time: requested for a specific machine, authorized against the machines that account can reach, routed to that daemon, and reported back rather than assumed.
- **Rebooting a daemon updates it first when an update is available**, completing every fallible step — pull, build, install — while the current daemon is still running and only then replacing it. A reboot that merely restarts the same binary would leave the phone unable to do the one thing SSH was needed for.
- **Running projects are untouched.** The reboot replaces the daemon process only; projects, panes, and agents on that host keep running, and the confirmation says so.
- The per-project CLI reboot stays. It is a different action on a different thing, and the machine list does not replace it.

## Capabilities

### New Capabilities

- `machine-lifecycle-control`: seeing the machines an account can reach and restarting the daemon on one — its authorization, what it disturbs, and how its outcome is reported.

### Modified Capabilities

- `mobile-code-sessions`: the mobile surface gains machines as a first-class list beside coding sessions, so a user can act on the host rather than only on the work running there.

## Impact

- `crates/shared/src/messages.rs`: a daemon-reboot request from the web, and the matching command to the daemon.
- `crates/server/src/routes/ws_web.rs`: authorize the request against the machines the account can reach, then route it to that machine's daemon.
- `crates/server/src/routes/ws_daemon.rs`: deliver the command.
- `crates/client-cli/src/mode/daemon.rs`: handle it by reusing the update-then-`exec` path the daemon already uses to replace itself unattended.
- `packages/web`: a machines list on the mobile home, and a store action that targets a machine by id.
- No change to how projects run: this is deliberately a machine-level control that leaves project supervision alone.
- Related: `resident-host-supervisor` makes this daemon the host's supervisor. Its requirement that projects outlive their supervisor is what keeps this reboot safe once that lands.
