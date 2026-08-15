## 1. Protocol

- [x] 1.1 Add the web request carrying a machine id, and the matching daemon command
- [x] 1.2 Tests: both round-trip, and an older daemon's message set is unaffected

## 2. Server

- [x] 2.1 Authorize the request against the machines the account can reach, reusing the machine-scoped check that `StartMachineProjectCli` uses rather than a second one
- [x] 2.2 Route it to that machine's daemon; report undelivered when the daemon is not connected
- [x] 2.3 Tests: a reachable machine routes, an unreachable one is refused and routes nothing, a disconnected daemon reports undelivered

## 3. Daemon

- [x] 3.1 Handle the command by applying an available update, completing every fallible step while still serving
- [x] 3.2 Replace the process with `exec`, preserving pid, detachment, and the registration guard
- [x] 3.3 Leave a failed update running on the current daemon, and log why
- [x] 3.4 Tests: the update-then-replace decision, and that a failed preparation does not replace anything

## 4. Mobile surface

- [x] 4.1 Render machines from the bootstrap response the home already fetches
- [x] 4.2 Add machines as a selection beside the session filters, switching the list in place
- [x] 4.3 Show hostname, platform, connection state, and running project count per machine; say so plainly when there are none
- [x] 4.4 Add the reboot control, confirmed first and naming the machine, stating that work keeps running
- [x] 4.5 Store action targeting a machine by id, reporting failures rather than assuming success
- [x] 4.6 Tests: the list renders and switches, reboot routes the tapped machine's id, cancel sends nothing, a disconnected machine reports rather than silently failing

## 5. Documentation and verification

- [x] 5.1 Update `CLAUDE.md`: the daemon section documents self-upgrade as the only replacement path and must now cover a requested restart
- [x] 5.2 `cargo test` for the workspace and `cargo clippy` clean
- [x] 5.3 `npm run lint` and `npm test` clean in `packages/web`
- [ ] 5.4 End-to-end on a real host: reboot a daemon from the phone with projects running, confirm the projects stay up and the daemon returns on the newer version
