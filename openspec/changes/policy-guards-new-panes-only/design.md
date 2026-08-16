## Context

See proposal.md — Why. The enforcement already exists in the right shape and is
simply applied too widely. The server distinguishes `authorize_new_pane_launch`
from `authorize_existing_pane_launch`, and both end in the same
`authorize_profile_launch`; the project host applies
`launch_allowed_by_server_policy` at five points. So the change is a boundary
move, not new machinery.

The five host sites, classified: `AddPane` creates; the model switch chooses a
profile; `StartBot`, `ResumePane`, and `RebootPane` bring back something that is
already there. The last three are what this narrows.

## Goals / Non-Goals

**Goals:**

- One boundary — created versus already existing — applied identically on the
  server and the project host, since either alone is enough to refuse.
- Nothing about retired backends or team availability changes.

**Non-Goals:**

- Removing `noncompliant_pane_ids` from the wire. It stays as information.
- Letting a pane be *switched* into a disallowed profile.
- Any change to how the effective policy is computed or distributed.

## Decisions

1. **The line is "does this pane already exist", not "is this a launch".** All
   five sites launch a process, so "launch" cannot be the test. What separates
   them is whether the profile is being chosen now or was chosen earlier under
   whatever policy applied then.

2. **A model switch stays enforced, even though the pane exists.** It is the one
   existing-pane action that picks a profile, and exempting it would leave an
   obvious way around the allowlist: create an allowed pane, then switch it.

3. **Retired backends and team availability keep refusing, on both paths.** They
   are not catalogue entries. A retired backend cannot run at all, and team
   availability is the switch that stops a running team — honouring it on resume
   is what stops a disabled team coming back one pane at a time.

4. **A noncompliant pane stops blocking the CLI reboot.** That gate exists to
   avoid restarting into a state policy forbids, which only made sense while
   relaunch was forbidden. With relaunch allowed, it blocks the operation most
   likely to be needed on a project that has drifted from policy.

5. **The notice is removed rather than reworded down.** Its content was the
   restriction. What remains — this pane's profile could not be chosen again —
   belongs on the policy surface that lists profiles, not in a toast on entry.

## Risks / Trade-offs

- [An administrator narrows the allowlist expecting existing panes to wind down
  as they are restarted] → They no longer do; a pane persists until someone
  removes it. This is the intended reading of the allowlist as a catalogue, and
  the panes remain identified on the policy card. Stopping such panes outright
  remains available and is a separate, deliberate act.
- [Three enforcement layers could drift, since the web only hides menu entries]
  → The web was never authoritative here. Server and host both keep enforcing
  creation, and the tests pin the created/existing boundary on each.

## Migration Plan

Additive in effect: it only permits requests that were previously refused, so an
older host or an older server paired with a new counterpart is safe — the older
side keeps refusing, which is the behaviour being retired rather than a fault.
Deploy in any order; roll back by reverting.
