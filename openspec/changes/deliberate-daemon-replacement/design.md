## Context

See proposal.md — Why. Three mechanisms exist today and they are not equivalent,
which is what makes "disable the upgrade" ambiguous until it is written down:

- `perform_requested_restart` — the Machines page path. Runs
  `check_for_update_available` (which syncs the git repo), builds and installs
  when the change is build-relevant, then `exec`s. **Kept exactly as is.**
- The 15-minute tick — stats the installed binary and `exec`s when it is newer.
  No git, no build; it only adopts what someone else installed.
- `ensure_daemon_running`'s version branch — `stop_daemon_process` (SIGTERM,
  4s, SIGKILL) then spawns a fresh daemon.

The third is the dangerous one and the reason this is not a one-line deletion.
`exec` keeps the pid, keeps the session, and deliberately skips destructors so
`RegistrationGuard` never withdraws the host record or releases claims — plus
the resume manifest brings the projects back. An external kill has none of that.
And because `ctrlc` is declared without the `termination` feature, only SIGINT is
handled, so SIGTERM ends the process at once: the graceful shutdown that stops
each project never runs. For a `setsid`-detached daemon, SIGINT essentially never
arrives, which means that shutdown path is currently close to unreachable.

## Goals / Non-Goals

**Goals:**

- No unattended replacement of an instance that is hosting projects.
- A stop that reaches the teardown which saves pane rosters and ends agents.
- The requested restart keeps working exactly as it does, since it is now the
  only upgrade path and the surface for it just shipped.

**Non-Goals:**

- Removing the ability to upgrade. It moves entirely to the requested restart.
- Upgrading opportunistically when a host happens to be idle. It sounds
  appealing and it reintroduces the same surprise, only rarer and therefore
  harder to reason about when it does bite.
- Changing cross-host claims, pane-host adoption, or the resume manifest.

## Decisions

1. **Remove the timer rather than gate it on having no projects.** A conditional
   timer is still an unattended replacement; it just waits for a quiet moment,
   which on a busy host may be the middle of the night and on an idle host is
   immediately. The honest version of "only when asked" is to only do it when
   asked.

2. **A launch reports instead of replacing.** The version branch existed because
   `ensure_daemon_running` was the only upgrade path, so a launch was the natural
   moment to catch up a stale daemon. That is exactly the reasoning the merge
   invalidates: a launch is now a bystander to running work, and the daemon it
   would replace is serving projects that belong to whoever started them. It
   prints what is running and where to replace it.

3. **Handle termination, not only interrupt.** Enabling `ctrlc`'s `termination`
   feature makes SIGTERM set the same shutdown flag, so an ordinary `kill`, a
   service stop, or a shutdown reaches the path that stops each project. Without
   this the graceful shutdown added with the merge is unreachable in practice,
   which is worth fixing regardless of the rest of this change.

4. **Delete the helpers the timer left behind rather than keep them dead.**
   `newer_installed_version` and `apas_binary_fingerprint` exist only to answer
   "has the installed binary changed" on a schedule. Keeping them behind an
   allow-dead-code attribute preserves the appearance of a mechanism that no
   longer runs. Their tests go with them; the ordering they exercised is still
   covered where `parse_version` is tested, which the requested restart uses.

## Risks / Trade-offs

- [The problem the timer solved comes back: a host nobody logs into keeps its
  version forever] → Accepted deliberately, and the mitigation is what makes this
  the right moment. Restarting a daemon is now a per-machine control on both the
  desktop and mobile machine lists, it applies the update itself, and it says
  which machines are behind. The reason zoo-002 went unnoticed for nine versions
  was that nothing surfaced it; now something does.
- [Rolling an update across a fleet becomes N clicks instead of waiting] →
  True, and it is the point: each click is someone deciding that this machine's
  projects can be restarted now.
- [SIGTERM now runs teardown, and `stop_daemon_process` allows only four seconds
  before SIGKILL] → That caller is the one being removed. Any remaining external
  stop gets a best-effort teardown that is strictly better than the immediate
  death it gets today, and pane hosts keep their terminals either way.
- [A daemon that crashes still comes back on the old binary] → Unchanged by this,
  and consistent with it: coming back as something different from what was
  running is the surprise being removed.

## Migration Plan

Ordering matters once and only once. Hosts must be moved onto this build by the
*existing* mechanism before it is gone — install the binary, let each daemon
adopt it on its last automatic tick, and confirm every host reports the new
version. A host still on an older build keeps the old behaviour, including the
launch-time kill, until it is restarted.

Rollback is a binary swap; the removed timer returns with it. Nothing persisted
changes, so a mixed fleet is safe in both directions.
