## 1. Make one project stoppable without stopping the process

- [x] 1.1 Replace `dual_pane.rs:12626` (server rejected the session) with a return that stops this project and reports why
- [x] 1.2 Replace the two reboot exits (`3382`, `3400`) with a signal the caller acts on, so a reboot replaces a project rather than the process
- [~] 1.3 Make `run_inner` return cleanly on cancellation — partially done: it now returns an outcome instead of calling `process::exit`, and the dead interactive TUI path is gone so there is one body to reason about. Cancellation itself arrives with the task supervision in section 2
- [x] 1.4 Tests: a rejected session ends one project and reports it; cancellation returns rather than exits

## 2. Run projects inside the daemon

- [x] 2.1 Hold a task handle and a cancellation signal per project in the daemon
- [x] 2.2 Start a project by spawning the task rather than a tmux session
- [x] 2.3 Stop a project by cancelling and awaiting it
- [x] 2.4 Restart a project by cancelling and starting a fresh task — a project's own `RebootRequested` now travels a channel the connection loop selects on, which stops the finished task properly (its entry and pane hosts outlive it) before starting a fresh one
- [x] 2.5 Report running state from the task table rather than from `/proc`
- [x] 2.6 Restore the projects that were running after the instance is replaced by an upgrade
- [~] 2.7 Tests — the resume manifest is covered; the table's start/stop/report behaviour needs a daemon harness that does not exist yet, since `DaemonState` reaches the network and the filesystem on every path

## 3. Contain failure

- [x] 3.1 Treat a panicking project task as that project stopping, reported, with the others untouched
- [x] 3.2 Hold the line on unwind: a test that fails if `panic = "abort"` is ever set, since that would turn every project panic into a host outage
- [x] 3.3 Tests: one project panicking leaves the others running and the instance alive

## 4. Keep activity attributable

- [x] 4.1 Give each project a tracing span carrying its identity, so every record says which project it came from
- [x] 4.2 Replace the per-project stderr log — the `project` span carries the id and the default `Full` log format prints span fields, verified by capturing output rather than assumed
- [x] 4.3 Tests: records from two concurrent projects are distinguishable

## 5. Retire the process-per-project machinery

- [x] 5.1 Stop spawning `apas --headless` for projects; keep the flag for running one project alone
- [x] 5.2 Retire the per-project tmux session and socket handling for project CLIs, leaving pane hosts untouched
- [x] 5.3 Retire `/proc` scanning as the source of running state — the table answers it; `/proc` survives only to notice an externally started `--headless` run and to find the older model's leftovers at startup, neither of which the table can know about
- [x] 5.4 Ensure an upgrade stops tmux'd projects from the older model rather than leaving them running alongside the new tasks

## 6. Documentation and verification

- [x] 6.1 Update `CLAUDE.md`: the process model, why pane hosts stay separate, and that blocking work must not go on the runtime
- [x] 6.2 Measure thread count and memory — done on zoo-005 with four live projects: 66-thread runtime baseline, 11–13 marginal per project, 377 threads across five processes today versus ~113 projected merged. The estimate in the risk list was wrong in the safe direction and is corrected
- [x] 6.3 `cargo test` for the workspace and `cargo clippy` clean — and one flaky failure fixed on the way, since it was misreporting this change as a regression. `pane_host` mutated `XDG_RUNTIME_DIR` outside the lock serialising the `HOME`/`XDG_CONFIG_HOME` writers; concurrent `setenv` races on the environ array, so a registry test read a stale `XDG_CONFIG_HOME` and migrated a file out from under its own assertion, then poisoned the lock guard and took eight more tests with it
- [~] 6.4 End-to-end on a real host — partially verified live on zoo-005 and zoo-006:
  - [x] A project starts and runs inside the instance: `started project in this instance project_id=4366dc38…`, its pane host spawned, and no `apas --headless -d` process for it anywhere
  - [x] Records stay attributable: every project record carries the `project{id=…}` span in the daemon's own log, which is what replaced the per-project tmux session and stderr file
  - [x] Upgrading the instance leaves the host working: zoo-002 and zoo-006 re-exec'd onto 26.08.74 off the shared binary on their own tick, unattended
  - [ ] Several projects in one instance, stopping and restarting one while the others keep running, and killing one for containment — needs more than one project running on a single host, so it waits on projects being started rather than on code
  - [ ] Upgrade with projects running, confirming the resume manifest brings them back — the two self-upgrades so far both happened while nothing was running, so this path is still untested live
