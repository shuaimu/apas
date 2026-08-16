## 1. Stop replacing the instance on a timer

- [x] 1.1 Remove the upgrade-check interval and its tick arm from the daemon's connection loop, leaving the requested-restart path untouched
- [x] 1.2 Remove the helpers that existed only for it, and their tests, rather than leaving a mechanism that looks live but never runs
- [x] 1.3 Test: the requested restart still plans an update-then-replace when one is available and a replace-in-place when it is not — already covered; the version-ordering test was renamed off the removed mechanism so it stops pointing at something that no longer exists

## 2. Stop a launch from killing a running instance

- [x] 2.1 Make the version branch in `ensure_daemon_running` report rather than stop the running daemon, keeping the start-when-absent path unchanged
- [x] 2.2 Say what is running, that it is older, and where to replace it
- [x] 2.3 Tests: a newer launch against an older running daemon leaves it running and reports; a launch with no daemon still starts one; a launch at the same version is unchanged

## 3. Make stopping the instance reach its teardown

- [x] 3.1 Handle termination as well as interrupt, so an ordinary stop sets the shutdown flag instead of ending the process outright
- [x] 3.2 Test that the handler covers termination, so the graceful path is not silently unreachable again

## 4. Documentation and verification

- [x] 4.1 Rewrite the "daemon upgrades itself" section of `CLAUDE.md`: what replaces it, why the old trade-off no longer holds, and how a fleet is rolled forward now
- [x] 4.2 `cargo test` for the workspace and `cargo clippy` clean
- [ ] 4.3 Roll every host onto this build using the existing mechanism first, confirm all report the new version, and only then confirm no host upgrades itself afterwards
