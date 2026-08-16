## Why

Opening a project announces that some of its tabs are outside cluster policy and
cannot be relaunched. It is the first thing a user sees, it names pane numbers
rather than anything actionable, and the only remedy — change the cluster policy
— is one most users do not have. On mobile it arrives as a toast on every entry
to the project.

Behind the notice, the launch-profile allowlist is applied to panes that already
exist: resuming, rebooting, or starting a bot in a pane whose profile has since
been disallowed is refused, and a single such pane blocks rebooting the whole
project CLI. The pane keeps running in the meantime, so the rule does not remove
the thing it disapproves of — it just makes that thing impossible to restart,
which is worst when a user most needs it back.

The allowlist is meant to decide what may be *created*. A pane that exists is
evidence of a decision already taken, under whatever policy was in force then.

## What Changes

- **The launch-profile allowlist governs creating panes only.** Resuming,
  rebooting, and starting a bot in an existing pane are no longer refused for
  being outside it, on the server and on the project host alike.
- **A noncompliant pane no longer blocks rebooting the project CLI.** One pane's
  profile falling out of the allowlist stopped the whole CLI being restarted.
- **Creating a pane is unchanged.** New panes, and switching a pane to a
  different model or provider, still have to satisfy the allowlist — a switch
  chooses a profile rather than resuming one.
- **Two things that are not the allowlist stay exactly as they are.** A retired
  provider still refuses to launch, because its backend genuinely no longer
  exists; and managed team panes still require team mode to be available, which
  is a kill switch rather than a catalogue.
- **The notice goes.** With nothing restricted, the toast has no action attached
  to it. The policy card keeps naming the panes, reworded to say what is
  actually true: they run and relaunch, and their profile cannot be chosen for
  something new.

## Capabilities

### Modified Capabilities

- `project-policy-governance`: the launch enforcement requirement narrows to
  creation and profile changes, and states that an existing pane's lifecycle is
  not restricted by the allowlist.

## Impact

- `crates/server/src/routes/ws_web.rs`: `authorize_existing_pane_launch` stops
  applying the allowlist (`ResumePane`, `RebootPane`, `ResumeDeadloop`,
  `StartBot`); the two CLI-reboot gates stop blocking on it.
- `crates/client-cli/src/mode/dual_pane.rs`: the same three actions stop
  applying it; `AddPane` and the model switch keep it; the noncompliance status
  message is removed.
- `packages/web/src/lib/store.ts`: the toast on entering a project is removed.
- `packages/web/src/components/overview/AllowedTabTypesCard.tsx`: reworded.
- `noncompliant_pane_ids` stays on the wire as information — it no longer
  predicts a refusal, so nothing keys behaviour off it.
