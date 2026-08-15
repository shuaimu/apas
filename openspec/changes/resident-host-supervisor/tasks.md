## 1. Establish the current behaviour

- [x] 1.1 Establish what an interactive `apas` holds that a headless one does not, and which of it the TUI reads directly rather than from the server stream — done: `run()` and `run_headless()` are one `run_inner` differing only by the TUI block, and `App` renders from two receivers with no input path
- [ ] 1.2 Reproduce the duplicate-CLI case (`apas` in a directory whose project the daemon already runs) and capture what actually breaks, so the fix is verified against the real failure
- [ ] 1.3 Confirm what a deferring launch must register for a project to appear on the Machines page, and whether an already-registered project needs anything further

## 2. One instance per user per host

- [ ] 2.1 Apply the existing `detect_running_daemon` check to the default `apas` path, reusing it rather than writing a second check
- [ ] 2.2 When an instance is already running, register the project directory if there is one, report where to manage it, and exit without starting anything
- [ ] 2.3 When no instance is running, become it
- [ ] 2.4 Leave the rule off `--headless` workers: they are the daemon's children, not user-launched instances
- [ ] 2.5 Tests: a second launch starts nothing and leaves the first undisturbed; a stale pid record does not block a new instance; a deferring launch registers its project exactly once; a launch outside a project registers nothing

## 3. Retire the interactive path

- [ ] 3.1 Stop making the TUI the default for `apas` in a project directory
- [ ] 3.2 Put it behind an explicit `--attach`, reusing the attachment built in 3233108
- [ ] 3.3 Record that `--attach` is provisional: delete it, and `attach.rs` with it, if nothing uses it

## 4. Documentation and verification

- [ ] 4.1 Update `CLAUDE.md`: one instance per user per host, projects launched from the web, and what `apas` in a project directory now does
- [ ] 4.2 Release note for the two visible losses: no terminal UI, and `apas` no longer starts the project it is run in
- [ ] 4.3 `cargo test` for the workspace and `cargo clippy` clean; the existing project tests pass untouched
- [ ] 4.4 End-to-end on a real host: run `apas` in a new project directory and confirm it registers, reports, and exits; confirm the project starts from the Machines page; confirm running projects survive replacing the instance
