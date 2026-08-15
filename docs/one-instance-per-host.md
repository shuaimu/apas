# `apas` now registers and exits

Two things change for anyone who runs `apas` from a terminal. Both are
deliberate, and neither affects work that is already running.

## `apas` no longer opens a terminal UI

It printed pane names and little else, and the web and mobile surfaces have
been the real interface for a while. The terminal UI is still reachable:

```bash
apas --attach     # in a project directory that is already running here
```

That is a local view for when the web is unreachable. If nobody uses it, it
will be removed rather than carried indefinitely.

## `apas` no longer starts the project it is run in

Running it in a project directory now registers that project and exits,
printing where to manage it. Start it from the Machines page.

```
✓ apas is registered on this machine.
   Start and manage it at https://apas.mpaxos.com
   This host runs one APAS instance per user; it is already running.
```

Creating a project from a local directory still works exactly as before: run
`apas` in a directory that is not yet a project and it becomes one. What
changed is how many processes end up running, not how projects come into
being.

## Why

A host runs one APAS instance per user. `apas daemon` has always enforced
that — it prints and returns when one is already running — but plain `apas`
never did. It went straight to running the project in the foreground, so
typing it in a directory the daemon was already running produced two owners of
one project, one `.apas`, and one set of worktrees, with nothing to stop it.

Rather than teach a second process to cooperate with the first, there is no
second process.

## What is unaffected

- **Running projects.** Panes, agents, and terminal sessions are owned by
  their own processes, not by the instance you launched.
- **Creating projects from a local directory**, as above.
- **Two accounts on one host.** The rule is per user; their projects, claims,
  and runtime state were already separate.
- **Several hosts sharing one home directory.** Which host may run a project
  is still decided by the existing cross-host claim, which answers a different
  question from how many instances one user has on one machine.
