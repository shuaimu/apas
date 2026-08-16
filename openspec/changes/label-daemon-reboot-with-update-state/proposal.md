## Why

Restarting a daemon is how a CLI update reaches a machine — the restart applies
an available update before replacing the process, which is what makes rolling an
update out from a phone possible instead of an SSH session. But the control says
only "Reboot daemon" on every machine, whether that machine is nine versions
behind or already current, so the operator cannot tell which machines the action
is actually *for*. The information needed to say so is already on the client and
displayed nowhere: each machine reports its daemon version, and the server
reports its own.

The desktop machines page, which has the most room to show this, has no restart
control at all. `machine-lifecycle-control` already requires that a daemon be
restartable from its machine entry without naming a surface, so this is an
unimplemented requirement rather than a new idea.

## What Changes

- **A machine entry shows the version its daemon is running.** Today the value
  reaches the client and is rendered nowhere, so "behind" is unverifiable even
  when the label claims it.
- **The restart control states whether it will also update that machine.** It
  reads "Reboot to update" when the machine is known to be behind, and "Reboot"
  when it is not.
- **"Behind" means strictly older than the newest version the client can see** —
  the server's own version, and the highest daemon version among the machines
  that account can reach — compared as the `YY.MM.COMMIT` ordering the CLI
  already defines. Both sources matter: the server's version catches every
  machine lagging a deployment, and the highest peer version catches the window
  during a rollout when one host has upgraded and its neighbours have not, which
  is the case this control exists to serve.
- **A version that is missing or unparseable never claims an update is
  available.** It shows the plain label, mirroring the CLI's existing refusal to
  act on a version it cannot read rather than gambling on a downgrade.
- **The desktop machines page gains the restart control** the mobile machine list
  already has, including the confirmation that names the machine.
- The label describes what is *known*, not a guarantee. A restart always attempts
  an update first, so a machine showing the plain label can still come back
  newer; the label never asserts the reverse — that a machine known to be behind
  will stay behind.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `machine-lifecycle-control`: machine entries gain a reported daemon version;
  the restart control gains a state that distinguishes a restart that is known to
  also update from one that is not; and the restart control is required wherever
  machines are listed, rather than being satisfied by a single surface offering
  it.

## Impact

- `packages/web/src/components/mobile/MobileCodeHome.tsx`: the existing reboot
  button and its confirmation take the label, and the machine row shows the
  version.
- `packages/web/src/app/machines/page.tsx`: gains the restart control and
  confirmation it never had, plus the version on each machine row.
- `packages/web/src/lib/`: one shared helper for the `YY.MM.COMMIT` comparison
  and the "latest seen" reduction, so the two surfaces cannot disagree about
  which machines are behind.
- No server, protocol, CLI, or daemon change: `daemon_version` and
  `server_version` are already delivered to the client, and the restart path
  itself is untouched.
- `machine-lifecycle-control` has no main spec yet — it is introduced by the
  unarchived `machine-list-and-daemon-reboot` change, which should archive first
  so this change's delta merges onto it rather than creating it.
