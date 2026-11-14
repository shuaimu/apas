## Why

On 2026-11-13 the owner asked for four corrections to the interactive workspace. The "+" control had gained a step that asks "Interactive agent or Autonomous worker" before every new tab, although workers are off in this deployment. Raw Pi stayed a provider beside OMP, which is derived from it and is the one in use. The Overview tab was about three screens long and put an empty worker inventory first. The Start autopilot dialog opened with an empty prompt field.

The same day's archive review found two places where the interface contradicted requirements already in the main specs. This change records all of that work as it is in the tree.

## What Changes

- **"+" creates an interactive agent directly.** On the desktop tab bar and in the phone's pane sheet, "+" opens the provider list at once, with the "Run on" step where it applies. The step that chose between an interactive agent and an autonomous worker is removed. An autonomous worker is created from **New autonomous worker** in the Overview tab's Autonomous workers section, or from `/code/workers`.
- **BREAKING: Pi is retired as a provider. OMP stays.**
  - Nothing can create a Pi tab, switch a tab to Pi, or relaunch one. Server, project CLI and web each refuse with "Pi is no longer supported. Use OMP instead."
  - The wire value `pi` still parses, so an old `.apas`, a stored roster, a stored policy and a frame from an older client still load.
  - A saved Pi tab stays listed until someone closes it. No CLI launches it. Its CLI reports it as an exited terminal whose status is that sentence. Desktop and phone show its history read-only.
  - The capability `terminal_pi_v1` is deleted: the CLI does not advertise it and the server does not read it.
  - The stored policy entry `terminal:pi:official:default` is dropped at server start and cannot be written back.
  - `pi_path` is no longer a CLI setting. A configuration file that still holds the key loads.
  - The server refuses typed input to a tab of any retired provider instead of passing it to the CLI.
- **The Overview is one short page, tabs first.** Desktop order: the project's name, Tabs, Usage, Resource use, Allowed launch profiles, Autonomous workers with Earlier results. The phone's Overview uses the same layout and parts: the project's name, Tabs as one row per tab, then Autonomous workers. The Usage table lists open tabs and folds closed tabs into one row. Launch profiles are shown by their catalog names. "Back to interactive agents" is removed from the Overview; a worker's screen, the composer and an earlier result keep it.
- **The Start autopilot dialog offers prompt templates.** A Template control above the prompt offers "Implement from a design document" (the default), "Work through TODO.md", "Keep the tests green" and "My own text". A tab with no earlier autopilot opens with the default template in the prompt field. What is sent is the text the prompt field holds. The dialog's Cancel and Start autopilot buttons stay visible while its content scrolls.
- **Two contradictions with the main specs are removed.** Both were found on 2026-11-13 while the shipped changes were archived.
  - The last team-mode interface and reads are gone: the "Team launch available" checkbox in the policy editor, the "Talk to your Manager" empty state and the team task bar, `useTeamEnabled`, and the checks that read `team_available` for a pane marked `managed` in the web, the CLI and the server. The field stays on the wire and in stored policy and decides nothing.
  - The one-machine daemon reboot confirmation on the Machines page and the phone's home said "Projects, panes, and agents on this machine keep running" and that the daemon does not own them. It now says "Every project running on <host> restarts with it. Terminal agents normally keep running during the restart, because their hosts are separate processes."
- **A new OMP tab is named for what it is.** A tab created from the desktop "+" is labelled "OMP N", not "Claude N", and its tab icon is titled OMP.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `provider-support`: Pi leaves the supported launch catalog. The three Pi requirements are removed. New requirements state the retirement: the refusal, the deleted capability, the stored forms that still parse, the dropped policy entry, and that a retired provider's tab takes no input. OMP is unaffected.
- `terminal-pane-continuity`: Pi leaves the list of persistently hosted providers. The three Pi requirements (session-file recovery, restoration, questions) are removed. A saved Pi tab is never launched, is reported as an exited terminal with the retirement sentence, and keeps no persistent runtime. The scenarios of the removed session-file recovery requirement that still hold for OMP, which is read through the same parser, move to a new requirement for OMP tabs.
- `mobile-terminal-access`: the mobile task-launch requirement for Pi is removed. Mobile task launch never offers Pi and answers a stale Pi request with the retirement sentence.
- `omp-tab-autopilot`: the start requirement no longer says that the system supplies no prompt of its own. A new requirement defines the prompt templates. A new requirement keeps the dialog's actions in view.
- `web-project-workspace`: the new-tab control creates an interactive agent directly; where an autonomous worker is created; the Overview's order of sections; the Usage table's handling of closed tabs; how allowed launch profiles are named.
- `pane-model-selection`: a terminal tab's Overview card names what the tab runs, where it was labelled "Terminal". It still shows no agent, backend or model control.
- `machine-lifecycle-control`: the confirmation scenario of "A daemon restart restores the work on that machine" names what the one-machine confirmation says.

### Capabilities checked and left unchanged

- `project-policy-governance`: "Retained policy fields for removed features decide nothing" already says that a retained field affects no decision and is not presented as a setting, with the scenario "The field is not offered". Removing the "Team launch available" checkbox and the `team_available` reads brings the code to that requirement. No delta is needed.
- `pane-toolbar`: it does not describe the "+" control. No delta is needed.

## Impact

- **Shared and protocol:** `Provider::Pi` becomes a decode-only value; `PI_TERMINAL_CAPABILITY` is deleted; the Pi launch profile leaves `supported_launch_profiles` and `all_tab_types`; the JSON schemas keep `pi`, marked deprecated. Generated TypeScript is unchanged.
- **Server:** launch, switch and relaunch authorization; typed-input refusal for retired tabs; mobile task launch; the launch-profile migration, which now also drops the Pi key.
- **CLI:** terminal hosting, launch arguments, session lookup and the transcript watcher lose their Pi branches; `pi_path` leaves the configuration; a saved Pi tab is reported, never started, and keeps no pane host. The Pi session record parser stays, because OMP writes the same format.
- **Web:** the tab bar and the phone's pane sheet, the Overview and its panels, the worker workspace's Overview layout, the Start autopilot dialog, the policy editor, the Machines page and the phone's home.
- **Active changes:** `unify-autonomous-workers` and `add-autonomous-jobs` described the removed chooser and the old Overview order. Their text is corrected in place. Their open gate tasks are untouched.
- **Rollout:** server, then web, then CLI. The deployment has not happened. On its first start the new server rewrites stored policies that list the Pi profile. An older CLI keeps advertising `terminal_pi_v1`, which nothing reads. A machine gains the new CLI at its next reboot from the Machines page, which stays its owner's action.
- **Not affected:** OMP tabs, their launch, their transcript reading and their autopilot; the native app, which has no Pi entry and no chooser; worker creation, approval and execution.
