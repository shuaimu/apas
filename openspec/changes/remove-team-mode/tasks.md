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

- [ ] 4.1 Rewrite the team-mode sections of `CLAUDE.md`, including the pane-kind note that exists because managed panes needed the structured stream
- [ ] 4.2 Sync the affected spec and remove the team requirement
- [ ] 4.3 Workspace tests and clippy clean; web lint, type-check and tests clean
- [ ] 4.4 Live: open a project that holds a stored managed pane and confirm it behaves as an ordinary pane
