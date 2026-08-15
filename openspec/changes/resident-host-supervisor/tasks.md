## 1. Find the seams before moving anything

- [ ] 1.1 Establish what an interactive `apas` holds that a headless one does not, and which of it the TUI reads directly rather than from the server stream — this is the gap that decides how thin an attached CLI can be
- [ ] 1.2 Enumerate every consumer of `headless_pid_for` / `is_headless_running_for` / `snapshot_projects` and record which need running-state truth versus a pid
- [ ] 1.3 Reproduce the duplicate-CLI case (`apas` in a directory whose project the daemon already runs) and capture what actually breaks, so the fix is verified against the real failure

## 2. Supervisor socket and project table

- [ ] 2.1 Give the supervisor a runtime directory, owner-only credential, and Unix socket, following `pane_host.rs` rather than a second convention
- [ ] 2.2 Add the project table: project id, path, worker socket, lifecycle state; authoritative for the host
- [ ] 2.3 Serve the queries the host already answers — running projects, start, stop — from the table instead of from `/proc`
- [ ] 2.4 Tests: the table is the reported state; a worker that exits is no longer reported; a stopped project can be started again

## 3. Workers as supervised children

- [ ] 3.1 Start a project worker with its runtime directory and socket, keeping its own tmux session so one project cannot take down supervision
- [ ] 3.2 Supervise it: detect exit, tombstone a deliberate stop, self-terminate an orphan on lease expiry
- [ ] 3.3 Adopt workers found at supervisor startup, by socket when there is one and by process probe when there is not
- [ ] 3.4 Tests: adoption after supervisor restart, no duplicate worker for an adopted project, orphan expiry

## 4. Attach instead of compete

- [ ] 4.1 Make the default project path ask the supervisor for the project rather than launching `dual_pane` directly
- [ ] 4.2 Start the worker when the project is not running, then attach once it is
- [ ] 4.3 Render the attached TUI against the worker; an attached CLI's exit must not stop the project
- [ ] 4.4 Support more than one attachment to the same project
- [ ] 4.5 Tests: `apas` twice in one directory yields one worker and two attachments; attachment exit leaves the project running; a remote start racing a local one yields one worker

## 5. Retire the inference

- [ ] 5.1 Stop consulting `/proc` for running state, keeping the probe only for adoption
- [ ] 5.2 Remove `reconcile_running_claims`, now subsumed by adoption, and keep cross-host claims exactly as they are
- [ ] 5.3 Tests: cross-host exclusion still decides between two hosts sharing storage

## 6. Lifecycle

- [ ] 6.1 Point reboot, stop, and transport recovery at the supervisor's record of the project
- [ ] 6.2 Report an attachment ending as an attachment ending, never as the project stopping
- [ ] 6.3 Tests: reboot with an attachment present; attachment exit reports no lifecycle failure

## 7. Documentation and verification

- [ ] 7.1 Update `CLAUDE.md`: the daemon section documents `/proc`-derived running state as deliberate and must now describe the supervisor, adoption, and what cross-host claims still cover
- [ ] 7.2 Release note for the one behaviour people may notice: closing the terminal no longer stops a project
- [ ] 7.3 `cargo test` for the workspace and `cargo clippy` clean; the existing project tests pass untouched
- [ ] 7.4 End-to-end on a real host: start remotely and attach locally, attach twice, close a terminal, restart the supervisor with projects running, and reboot a project with an attachment present
