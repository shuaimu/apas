## Context

See proposal.md — Why. What shapes the approach is that everything needed is
already on the client and nothing needs to be asked for:

- Each machine's `daemon_version` arrives in the machine record and lands in the
  web store as `daemonVersion`, where nothing reads it.
- The server's own version arrives on authentication and is kept as
  `serverVersion`, where only the footer reads it.
- The CLI defines the version ordering: `YY.MM.COMMIT` parsed into a numeric
  triple, with anything else refused rather than guessed. The daemon's own
  self-upgrade gate uses it, so the client must agree with it or the label will
  contradict what the daemon actually does.

The desktop machines page renders its machine list from the store — the same
source the mobile list uses — while the cluster-administration section below it
comes from the `/cluster/*` API. Only the store-backed part is involved here.

## Goals / Non-Goals

**Goals:**

- One definition of "behind", used by both surfaces, so they cannot disagree
  about the same machine.
- Agreement with the CLI's ordering, including its refusal to act on a version
  it cannot parse.
- The two surfaces reach parity on the restart control itself, not only on its
  wording.

**Non-Goals:**

- Asking a machine whether an update exists for it. That answer lives in a git
  remote, and getting it would mean a fetch per machine on a page render.
- Any protocol, server, CLI, or daemon change.
- Reporting the outcome of the update a restart applies — the restart already
  reports its own progress, and this change only labels the button that starts
  it.
- Blocking or discouraging a restart on a current machine. Restarting a current
  machine is legitimate and stays one click away.

## Decisions

1. **"Latest" is the newest version the client can already see: the server's
   version, and the highest daemon version among the machines that account can
   reach.**

   Considered the server's version alone. It fails the case the control exists
   for: hosts here install the CLI from git and share one binary over NFS, so a
   freshly installed CLI is routinely *newer* than the deployed server. Judging
   against the server alone would call every machine current during exactly the
   rollout window where one host has upgraded and its neighbours have not.

   Considered the highest peer version alone. It fails the opposite case — every
   machine equally behind a newer deployment reads as current, because nothing
   they can see is newer.

   Taking the maximum of both is the union of what the client knows, and it is
   order-independent and stable: adding a machine can only move "latest" forward.

2. **The comparison is a small shared helper, not logic in two components.** Two
   surfaces computing "behind" independently is how they end up disagreeing about
   one machine, which the spec forbids. It parses `YY.MM.COMMIT` the way the CLI
   does and returns unknown for anything else.

3. **Unknown never means behind, and never makes anything else behind.** This
   mirrors the CLI, which refuses to act on an unparseable version rather than
   risk a downgrade. An unreadable version excluded from the maximum also means a
   garbled report cannot mark an entire fleet as behind.

4. **The label states what is known; it never promises the negative.** A restart
   always attempts an update first, so a machine showing the plain label can
   still come back newer. Wording the plain case as "no update available" would
   be a claim the client cannot support — the plain label says only "Reboot".

5. **The version is shown next to the control.** A label asserting a machine is
   behind is unverifiable unless the operator can see the version it is behind
   *at*. It also makes the failure mode legible: a machine showing an unknown
   version explains its own plain label.

6. **Desktop gains the control by reusing the mobile behaviour, confirmation
   included.** The existing requirement already demands a confirmation naming the
   machine, and a destructive-looking control that skips it on one surface is the
   kind of asymmetry that gets discovered by an accidental click.

## Risks / Trade-offs

- [The label is a client-side inference and can be wrong in the case where a
  newer version exists that no machine and no server has yet] → Wrong only in the
  safe direction: it under-claims, showing a plain restart that still updates.
  The opposite error — claiming an update that is not there — requires a machine
  reporting a version newer than it runs, which nothing produces.
- [A machine reporting a garbled version could distort the maximum] → Excluded
  from the maximum rather than parsed leniently, so it cannot mark its peers
  behind.
- [Two surfaces still render their own markup and could drift in wording] → The
  decision of *which* wording is shared; only presentation is per-surface, and a
  test asserts the same machine reads the same way on both.
- [Showing versions makes a lagging machine visible to anyone who can reach it] →
  That is the point of the change, and the machine list is already scoped to the
  machines an account can reach.

## Migration Plan

Web-only and additive: no server, protocol, or CLI change, so it deploys with the
web build and rolls back by reverting it. A client that has not yet learned a
machine's version shows the unknown state and the plain label, which is the
correct behaviour rather than a degraded one.

`machine-lifecycle-control` has no main spec yet — it is introduced by the
unarchived `machine-list-and-daemon-reboot` change. That change should archive
first so this delta merges onto its requirements instead of creating the
capability with no Purpose.
