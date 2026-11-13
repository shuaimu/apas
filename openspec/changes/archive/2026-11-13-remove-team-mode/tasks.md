## 1. Web surfaces

- [x] 1.1 Remove the overview team panels: goal bar, TODO panel, delegation board, scratchpad ticker, suggested workers, team setup, team-mode switch, autonomy toggles, and their tests
- [x] 1.2 Remove the store actions and state that fed them
- [x] 1.3 Leave the launch-policy card, pane grid and usage rollups intact — they are not team mode
- [x] 1.4 Web lint, type-check and tests clean

## 2. Project host

- [x] 2.1 Delete the role prompts, TODO parsing, goal helpers, scratchpad and suggested workers
- [x] 2.2 Delete the delegation MCP server and its subcommand, whose every tool served the team protocol
- [x] 2.3 Remove the team runtime from the pane loop: starting and stopping a team, promoting a pane to managed, the role deadloops, and the team file watchers
- [x] 2.4 Stop reading and writing the team artefacts, leaving any existing files on disk untouched
- [x] 2.5 Treat a pane still marked managed as an ordinary pane rather than dispatching it
- [x] 2.6 Tests: a project with a stored managed pane loads and runs it as an ordinary pane

## 3. Server and protocol

- [x] 3.1 Remove team routing and the team launch authorization
- [x] 3.2 Keep `team_available` on the wire and in stored policy, deciding nothing, so older clients keep parsing
- [x] 3.3 Remove the team message variants that no sender or receiver remains for
- [x] 3.4 Tests: a policy carrying the retained field resolves identically to one without it; no launch is refused on account of it

## 4. Documentation and verification

- [x] 4.1 Rewrite the team-mode sections of `CLAUDE.md`, including the pane-kind note that exists because managed panes needed the structured stream
- [x] 4.2 Sync the affected spec and remove the team requirement
- [x] 4.3 Workspace tests and clippy clean; web lint, type-check and tests clean
- [ ] 4.4 Live: open a project that holds a stored managed pane and confirm it behaves as an ordinary pane
  - Not run. Archived on 2026-11-13 as implemented and in production; this check was never recorded. Production held no managed pane when team mode was removed, so there was no project to open. A unit test covers the behaviour.
- [ ] 4.5 Remove the leftovers found on 2026-11-13. They contradict "Retained policy fields for removed features decide nothing" and are code work for a later change.
  - The policy editor on `/admin` and `/machines` still shows a "Team launch available" checkbox and saves its value (`packages/web/src/components/PolicyEditor.tsx`). The value decides nothing on the server.
  - A pane whose role contains "manager" still gets the "Talk to your Manager" empty state that names `project_goal.md` (`packages/web/src/components/tabs/TabbedView.tsx`).
  - The CLI still refuses a new pane flagged `managed` when `team_available` is false (`crates/client-cli/src/mode/dual_pane.rs`, the AddPane launch check). Nothing creates a managed pane.
  - The web refuses Resume, Reboot and a backend change of a pane that still carries `managed: true` when `team_available` is false, and refuses a new pane flagged managed (`packages/web/src/lib/store.ts`, the `pane.managed && !policy.teamAvailable` checks). The CLI keeps and reports the stored marker, so a stored managed pane is affected. `team_available` is false by default.
  - `useTeamEnabled` in `packages/web/src/lib/projectRole.ts` still reads the field. Nothing calls it.
