## Context

See proposal.md — Why. What exists already:

- **Mobile already receives machines.** `/mobile/v1/bootstrap` returns `machines: Vec<MachineWithProjects>` (`shared/src/mobile.rs`), built from `ws_web::list_accessible_machines_for_user`. `MobileCodeHome` fetches that response and uses only `sessions`.
- **There is no daemon reboot anywhere.** `ServerToDaemon` carries `StartProjectCli`, `StopProjectCli`, `CreateProjectInstance`, `RefreshProjects`, and three backend-config messages. Nothing restarts a daemon.
- **The daemon already knows how to replace itself.** Its upgrade tick stats the installed binary and, when it changed, calls `exec_into_newer_binary` — `exec`, not spawn-and-exit, so the pid survives, the process stays `setsid`-detached, and destructors never run (which is what keeps `RegistrationGuard` from withdrawing the host record and releasing project claims).
- **`prepare_cli_restart` is the discipline for a fallible update.** It completes pull, build, and install while the current process is still fully operational, so a failure changes nothing.
- **Machine-scoped routing exists.** `StartMachineProjectCli` / `StopMachineProjectCli` already authorize a machine id against the requester's own daemon registrations and route to that daemon.

## Goals / Non-Goals

**Goals:**

- Machines visible and actionable from the phone, using data already on the wire.
- A daemon restart that is worth requesting: it picks up an update, which is the reason someone would reach for it.
- No project, pane, or agent disturbed by restarting a daemon.

**Non-Goals:**

- Starting or stopping projects from the machine list — that already exists on the machines page and is not what was asked for.
- Replacing the per-project CLI reboot, which acts on a different process.
- Installing a daemon where none is running: this restarts one that is connected, and an absent daemon has nothing to route to.
- Any change to how projects are supervised.

## Decisions

1. **Reboot is authorized by machine, reusing the existing machine-scoped path.** The request carries a machine id and is checked against the machines that account can reach, exactly as `StartMachineProjectCli` is. Alternative considered: deriving the machine from a project on it — rejected, since the whole point is that a daemon is not per-project, and a project-derived route would fail for a machine running nothing.

2. **The daemon handles it by reusing its own replacement path.** Update-if-available, then `exec`. Reusing `exec` rather than spawn-and-exit is not a style choice: it is what preserves the pid for `daemon.json`, keeps the process detached, and stops destructors from withdrawing the host record and releasing project claims — the exact race the claim system exists to prevent.

3. **A restart applies an available update, and every fallible step runs first.** Pull, build, and install complete while the current daemon is still serving; only then does it replace itself. This is `prepare_cli_restart`'s discipline, and it makes the failure mode "nothing happened, here is why" rather than a machine with no daemon. Alternative considered: restart-only, with updating left to `apas update` over SSH — rejected, because then the phone still cannot do the thing that made the control worth having.

4. **The outcome is reported from routing, not assumed.** A request to a machine whose daemon is not connected is reported undelivered. Full progress reporting is deliberately not attempted: the daemon replaces its own process image mid-operation, so anything past "requested" would have to survive the very process that would report it.

5. **Machines are a third selection beside the session filters, not a separate screen.** The home surface already switches lists; adding a screen would add navigation for a list of a handful of rows. It reuses the bootstrap data already fetched, so selecting it costs no request.

## Risks / Trade-offs

- [A user reboots the daemon expecting it to fix a stuck project] → It will not, because the daemon does not own project processes. The confirmation says work keeps running, which is both the reassurance and the correction.
- [Update-on-reboot makes a fast action slow] → A pull and build can take minutes, and the control cannot honestly show progress through an `exec`. Mitigated by reporting that the restart was requested rather than completed, and by the daemon staying up throughout the fallible part.
- [A daemon that fails to come back leaves a machine unmanaged] → Projects keep running regardless, and `ensure_daemon_running` restores a daemon on the next interactive CLI start on that host. The failure is degraded management, not lost work.
- [Rebooting is one tap from a list of machines] → Same mitigation as the project control: confirm first, and name the machine in the confirmation.
- [`resident-host-supervisor` will make this daemon the supervisor] → Its spec already requires that projects outlive their supervisor and that a starting supervisor adopts what it finds, which is what keeps this control safe afterwards. Worth landing in that order, not the reverse.

## Migration Plan

Additive: a new request the server refuses for unreachable machines, and a new command older daemons never receive because nothing sends it to them. Server and web first; a daemon that predates the change simply never gets asked. Rollback is a binary swap, after which the control reports the request as undelivered.
