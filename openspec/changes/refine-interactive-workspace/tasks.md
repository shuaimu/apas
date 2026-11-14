## 1. New-tab control

- [x] 1.1 Desktop: "+" in the tab bar opens the provider list at once, with the isolated git worktree option and the "Run on" step where it applies; remove the Interactive agent / Autonomous worker step, "Back to agent types" and the props that served them (`packages/web/src/components/tabs/TabBar.tsx`, `TabbedView.tsx`)
- [x] 1.2 Phone: "Add agent" on the mobile-browser session screen lists the allowed profiles at once; remove the same step from the sheet (`packages/web/src/components/mobile/MobileSessionActivity.tsx`)
- [x] 1.3 Point read-only legacy bot history at "New autonomous worker on the Overview tab" in place of the removed menu entry (`TabbedView.tsx`)
- [x] 1.4 Label a new OMP tab from the desktop "+" as "OMP N", and title and colour the OMP tab icon as OMP (`TabbedView.tsx`, `TabBar.tsx`)
- [x] 1.5 Tests: the menu lists providers with no chooser, a zero-tab project gets its first tab from "+", and a new OMP tab's label and icon (`TabBar.test.tsx`, `TabbedView.test.tsx`, `MobileSessionActivity.instances.test.tsx`)

## 2. Pi retired: shared types and protocol

- [x] 2.1 Keep `Provider::Pi` as a deprecated decode-only value and add it to `is_retired_provider`, so that `is_retired_launch` and the launch-profile key treat it as retired (`crates/shared/src/messages.rs`)
- [x] 2.2 Add `PI_RETIRED_MESSAGE`, `retired_launch_message` and `retired_launch_profile_message`, so that every refusal of Pi reads "Pi is no longer supported. Use OMP instead."
- [x] 2.3 Remove Pi from `all_tab_types` and `supported_launch_profiles`; recognise `terminal:pi:official:default` as a retired profile key
- [x] 2.4 Delete `PI_TERMINAL_CAPABILITY` (`terminal_pi_v1`)
- [x] 2.5 Keep `pi` in the three JSON schemas, marked deprecated; add a fixture test that a frame naming it still validates (`packages/protocol/schema/*.json`, `packages/protocol/tests/fixtures.test.ts`)
- [x] 2.6 Tests: stored Pi forms still parse, a policy that lists the Pi profile does not allow Pi, and OMP is untouched by the retirement

## 3. Pi retired: server

- [x] 3.1 Refuse creating a Pi tab, switching a tab to Pi and relaunching a saved Pi tab with the retirement sentence, whatever the CLI advertises (`crates/server/src/routes/ws_web.rs`)
- [x] 3.2 Refuse typed input and terminal conversation input to a tab of a retired provider before anything is stored, and drop raw keystrokes for it without an error
- [x] 3.3 Refuse a provider or model switch of a saved Pi tab before the roster is edited, with "Tab N uses Pi, which is no longer supported. Close it and open an OMP tab."
- [x] 3.4 Name the tab in the Reboot CLI refusal for a project that holds a saved Pi tab
- [x] 3.5 Mobile task launch: leave Pi out of the catalog also when a policy lists it, answer a stale Pi profile key with the retirement sentence, and stop reading a Pi capability (`crates/server/src/routes/mobile.rs`)
- [x] 3.6 Test that the launch-profile migration drops the Pi key from a stored policy, keeps the other entries in order, is idempotent, and that writing the key back is rejected (`crates/server/src/db/mod.rs`)

## 4. Pi retired: project CLI

- [x] 4.1 Stop hosting Pi: no terminal binary, no launch or resume arguments, no session-file lookup and no watcher branch for Pi; stop advertising `terminal_pi_v1` (`crates/client-cli/src/terminal_pane.rs`, `transcript.rs`, `mode/dual_pane.rs`)
- [x] 4.2 Remove `pi_path` from the configuration and from `apas config`; a file that still holds the key loads, and asking for the key by name says it is no longer a setting (`config.rs`, `main.rs`)
- [x] 4.3 Keep a saved Pi tab in `.apas` and in the roster, never launch it on any path, and report its terminal as exited with the retirement sentence at every registration
- [x] 4.4 Leave a retired provider's tab out of the pane hosts the CLI keeps, so a Pi host left by an older CLI is ended when the project starts
- [x] 4.5 Word the CLI's own refusals for Pi: new tab, model switch, and CLI reboot while a saved Pi tab is present
- [x] 4.6 Keep the Pi session record parser and its tests, which OMP reads through
- [x] 4.7 Tests: a fake `pi` executable is never run, the terminal state report for a Pi tab and an OMP tab, an old `.apas` round-trips, and only the Pi tab's pane-host files are removed

## 5. Pi retired: web

- [x] 5.1 Remove Pi from the tab-type catalog, the provider options, the desktop menu, the provider switcher and the phone's sheet; ignore a Pi profile that a held policy still lists (`packages/web/src/lib/tabTypes.ts`, `providerOptions.ts`, `TabBar.tsx`, `MobileSessionActivity.tsx`)
- [x] 5.2 Desktop: show a saved Pi tab as read-only history with the retirement notice and the unsupported-provider icon; no input, resume or reboot (`TabbedView.tsx`, `TabBar.tsx`)
- [x] 5.3 Phone: the same notice, the composer disabled, the raw terminal disabled and "Reboot this pane" hidden; rename and close stay (`MobileSessionActivity.tsx`)
- [x] 5.4 Store: send no conversation message and no terminal input for a retired tab, and answer with the read-only sentence (`packages/web/src/lib/store.ts`)
- [x] 5.5 Tests for each of the above (`tabTypes.test.ts`, `providerOptions.test.ts`, `store.terminal.test.ts`, `TabBar.test.tsx`, `TabbedView.test.tsx`, `MobileSessionActivity.instances.test.tsx`)

## 6. Overview

- [x] 6.1 Add the Overview's shared parts: page column, section, card, buttons, chip, empty state and notice (`packages/web/src/components/overview/OverviewElements.tsx`)
- [x] 6.2 Order the page: project name, Tabs, Usage, Resource use, Allowed launch profiles, then Autonomous workers with Earlier results (`OverviewView.tsx`, `packages/web/src/components/workers/ProjectWorkersWorkspace.tsx`)
- [x] 6.3 Tab cards: the tab bar's order, what the tab runs by its catalog name, one status line with last activity, the autopilot line, and real buttons for Diff, Role and Close tab; a saved Pi tab's card reads "Unsupported provider" (`PaneGrid.tsx`, `ActivitySparkline.tsx`)
- [x] 6.4 Usage: open tabs first by last activity, then Unattributed, then one folded row for closed tabs with their sum; compact counts; a saved Pi tab's row named "Pi" (`UsageStatsRollup.tsx`, `packages/web/src/lib/formatCount.ts`, `packages/web/src/lib/launchProfileLabels.ts`)
- [x] 6.5 One relative-time formatter for the cards and the table; a time ahead of the browser's clock reads "just now" (`packages/web/src/lib/formatRelativeTime.ts`)
- [x] 6.6 Resource use and Allowed launch profiles: names from the provider catalog, the policy key as tooltip, retired keys left out; a saved Pi tab adds no Resource use card (`ResourceUseRollup.tsx`, `AllowedTabTypesCard.tsx`, `packages/web/src/lib/launchProfileLabels.ts`)
- [x] 6.7 Autonomous workers as a compact last section that keeps every function: worker cards, New autonomous worker, Refresh, load more, Earlier results, the stale and error notices and the access recheck (`ProjectWorkersWorkspace.tsx`)
- [x] 6.8 Give the phone's Overview the same layout and parts: the project's name, Tabs as one row per tab, then Autonomous workers (`MobileSessionActivity.tsx`, `ProjectWorkersWorkspace.tsx`)
- [x] 6.9 Remove "Back to interactive agents" from the Overview; a worker's screen, the composer and an earlier result keep it (`ProjectWorkersWorkspace.tsx`, and the tests that clicked it: `TabbedView.start-bot.test.tsx`, `TabbedView.stop-bot.test.tsx`, `MobileSessionActivity.test.tsx`)
- [x] 6.10 Tests (`PaneGrid.test.tsx`, `UsageStatsRollup.test.tsx`, `ResourceUseRollup.test.tsx`, `AllowedTabTypesCard.test.tsx`, `ProjectWorkersWorkspace.test.tsx`, `formatCount.test.ts`, `formatRelativeTime.test.ts`, `launchProfileLabels.test.ts`)

## 7. Autopilot prompt templates

- [x] 7.1 Add the three built-in templates, the default, and the own-text entry (`packages/web/src/lib/autopilotTemplates.ts`)
- [x] 7.2 Add the Template control above the prompt in the dialog and the phone's sheet; a tab with no earlier autopilot opens on the default template (`packages/web/src/components/tabs/AutopilotStartDialog.tsx`)
- [x] 7.3 Derive the control from the prompt text, so it names a template only for that template's unchanged text
- [x] 7.4 Choosing a template changes the prompt only; the line under the control says how the run ends with the no-tool-call option as it stands
- [x] 7.5 Keep the person's own text when a template is chosen and give it back with "My own text"; an edit taken back leaves nothing kept
- [x] 7.6 Write each template for one step per iteration, with rules for a shared checkout (no push, no history rewriting, other people's changes left alone). The first two also get the two-turn ending, a notes file and a mark for a step that cannot be done; Keep the tests green keeps no notes and relies on the conversation alone to skip a failure it tried twice
- [x] 7.7 Keep Cancel and Start autopilot visible while the dialog's content scrolls, in the dialog and in the sheet, with a refusal shown beside them (`AutopilotStartDialog.tsx`)
- [x] 7.8 Tests (`autopilotTemplates.test.ts`, `AutopilotStartDialog.test.tsx`, `AutopilotControl.test.tsx`, `TabbedView.autopilot.test.tsx`)

## 8. Interface brought back to the main specs

- [x] 8.1 Remove the "Team launch available" checkbox from the policy editor on `/admin` and `/machines`; a save sends back the value the editor was given (`packages/web/src/components/PolicyEditor.tsx`, `packages/web/src/app/admin/page.tsx`)
- [x] 8.2 Remove the "Talk to your Manager" empty state and the team task bar above a pane (`TabbedView.tsx`, `WorkerTaskBar.tsx` deleted)
- [x] 8.3 Remove the web's refusals that read `team_available` for a pane marked `managed`, and `useTeamEnabled` (`packages/web/src/lib/store.ts`, `projectRole.ts`)
- [x] 8.4 Remove the CLI's refusal of a new pane marked `managed` when `team_available` is false, and stop the server reporting a stored managed pane as outside policy on account of that field (`crates/client-cli/src/mode/dual_pane.rs`, `crates/server/src/session/mod.rs`)
- [x] 8.5 Correct the one-machine daemon reboot confirmation on the Machines page and the phone's home: "Every project running on <host> restarts with it. Terminal agents normally keep running during the restart, because their hosts are separate processes." (`packages/web/src/lib/daemonVersion.ts`, `packages/web/src/app/machines/page.tsx`, `packages/web/src/components/mobile/MobileCodeHome.tsx`)

## 9. Specs and records

- [x] 9.1 Write this change's deltas against the main specs: `provider-support`, `terminal-pane-continuity`, `mobile-terminal-access`, `omp-tab-autopilot`, `web-project-workspace`, `pane-model-selection`, `machine-lifecycle-control`
- [x] 9.2 Correct `unify-autonomous-workers` in place where it described the chooser and the old Overview order, and list the amendments in its `tasks.md`
- [x] 9.3 Correct `add-autonomous-jobs` in place where it described the chooser and the old Overview order
- [x] 9.4 `openspec validate --all --strict` passes
- [x] 9.5 Update the runbook and the docs for the four changes and the two corrections, with what was checked and what was not exercised (`AGENTS.md`, `README.md`, `docs/dev/5.1-overview-tab.md`, `docs/mobile-development-and-operations.md`, `docs/provider-retirement.md`)

## 10. Not done

- [x] 10.1 Run the complete gates at the final commit: the Rust workspace, the static musl CLI build, the protocol tests, the whole web suite with lint, typecheck and the production build, and the native checks. During the work only each stream's own test files were run (done 2026-11-14 at 58a548d: shared 92 + 3 + 1, server 457, CLI 596 with 1 ignored, musl build, protocol 68 and 137 extension tests, web 1,242 with lint, typecheck and production build, native 136; no two-host smoke)
- [x] 10.2 Deploy the server, then the web and protocol pair, then push the CLI. Check that the first server start drops `terminal:pi:official:default` from the stored policies and that no project is left with an empty allowlist by it (done 2026-11-14: release 20261114T002349Z; the first start rewrote the deployment default, one cluster default and nine project overrides; none is empty, the smallest allows one profile)
- [ ] 10.3 Run a template on a real OMP tab: the default template through a step and a commit, the two-turn ending with "End when an iteration makes no tool call" ticked, a blocked step, and a checkout with someone else's uncommitted changes
- [ ] 10.4 Look at a saved Pi tab on a real CLI: listed and stopped at project start, the exited state and its sentence on the desktop and the phone, the Reboot CLI refusal, and closing it. A Pi pane host left by an older CLI is ended when the updated CLI starts the project
- [ ] 10.5 Look at the work on a real phone: "Add agent", the Overview, the Start autopilot sheet with the on-screen keyboard open, and iOS
- [ ] 10.6 Look in a browser at the states seen so far only in unit tests or with stand-in data: worker cards with real workers and Earlier results with real results; a project with no tabs, a legacy agent-kind tab, a tab with a worktree or a role, and the access-changed state on the Overview; the Start autopilot dialog for a tab with an earlier autopilot; and the phone at 360 px
