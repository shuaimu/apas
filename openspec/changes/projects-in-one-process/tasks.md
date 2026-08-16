## 1. Make one project stoppable without stopping the process

- [x] 1.1 Replace `dual_pane.rs:12626` (server rejected the session) with a return that stops this project and reports why
- [x] 1.2 Replace the two reboot exits (`3382`, `3400`) with a signal the caller acts on, so a reboot replaces a project rather than the process
- [~] 1.3 Make `run_inner` return cleanly on cancellation — partially done: it now returns an outcome instead of calling `process::exit`, and the dead interactive TUI path is gone so there is one body to reason about. Cancellation itself arrives with the task supervision in section 2
- [x] 1.4 Tests: a rejected session ends one project and reports it; cancellation returns rather than exits

## 2. Run projects inside the daemon

- [x] 2.1 Hold a task handle and a cancellation signal per project in the daemon
- [x] 2.2 Start a project by spawning the task rather than a tmux session
- [x] 2.3 Stop a project by cancelling and awaiting it
- [~] 2.4 Restart a project by cancelling and starting a fresh task — stop and start are both in place and independent per project, but nothing yet turns a project's own `RebootRequested` into a restart; the daemon logs it
- [x] 2.5 Report running state from the task table rather than from `/proc`
- [x] 2.6 Restore the projects that were running after the instance is replaced by an upgrade
- [~] 2.7 Tests — the resume manifest is covered; the table's start/stop/report behaviour needs a daemon harness that does not exist yet, since `DaemonState` reaches the network and the filesystem on every path

## 3. Contain failure

- [x] 3.1 Treat a panicking project task as that project stopping, reported, with the others untouched
- [x] 3.2 Hold the line on unwind: a test that fails if `panic = "abort"` is ever set, since that would turn every project panic into a host outage
- [x] 3.3 Tests: one project panicking leaves the others running and the instance alive

## 4. Keep activity attributable

- [x] 4.1 Give each project a tracing span carrying its identity, so every record says which project it came from
- [ ] 4.2 Replace the per-project stderr log the tmux session was providing
- [ ] 4.3 Tests: records from two concurrent projects are distinguishable

## 5. Retire the process-per-project machinery

- [x] 5.1 Stop spawning `apas --headless` for projects; keep the flag for running one project alone
- [x] 5.2 Retire the per-project tmux session and socket handling for project CLIs, leaving pane hosts untouched
- [ ] 5.3 Retire `/proc` scanning as the source of running state
- [ ] 5.4 Ensure an upgrade stops tmux'd projects from the older model rather than leaving them running alongside the new tasks

## 6. Documentation and verification

- [ ] 6.1 Update `CLAUDE.md`: the process model, why pane hosts stay separate, and that blocking work must not go on the runtime
- [ ] 6.2 Measure thread count and memory with several projects running in one process, and record the numbers rather than an impression
- [ ] 6.3 `cargo test` for the workspace and `cargo clippy` clean
- [ ] 6.4 End-to-end on a real host: several projects at once, stop and restart one and confirm the others are undisturbed, kill one and confirm containment, upgrade the instance and confirm the projects come back
