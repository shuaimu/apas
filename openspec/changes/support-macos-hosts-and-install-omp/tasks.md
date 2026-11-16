## 1. Contract

- [x] 1.1 `MachineInfo.providers` with `MachineProviderStatus {provider, installed, version, can_install}`, lenient decoding; server passes it through; protocol regenerated with a fixture

## 2. CLI

- [x] 2.1 Process checks without `/proc`; daemon lock; `daemon.json` kept current; other daemons named, never stopped
- [x] 2.2 macOS runtime dir; first update takes the old resume manifest and lists old pane hosts
- [x] 2.3 Login-shell PATH merged on macOS; pane-host peer uid via `getpeereid`; socket path length error; Mach-O OMP accepted by the host-local store
- [x] 2.4 Every terminal launch failure reported as an exited terminal with its reason, also after reconnect
- [x] 2.5 Provider availability reported at registration and on change; OMP `can_install` from the launch's own decision
- [x] 2.6 Pinned, verified OMP install with lock, timeouts, progress in the tab and cleanup of stale partial downloads; tests against a local fake release server

- [x] 2.7 The OMP extension runs on macOS: no start-time requirement on any platform, launch id and pid kept, liveness from the provider's process group, the extension reports only as the direct child of the process that started it, a silent extension logged; `process_start` still written on Linux for daemons not yet rebooted
- [x] 2.8 `scripts/macos-check/check.sh` type-checks the CLI for macOS on Linux

## 3. Web

- [x] 3.1 "Installs OMP first" and "<Provider> is not installed on <host>." on the provider choice, desktop and phone
- [x] 3.2 Providers line on the Machines page and the phone's Machines card

## 4. Checks

- [x] 4.1 cargo test (shared, server, CLI), static musl CLI build, protocol and extension tests, web suite, typecheck and lint, `openspec validate --strict`
- [ ] 4.2 On the owner's Mac after the CLI update: one daemon, no reconnect loop, an OMP tab installs OMP and starts, a failed launch shows its reason
- [ ] 4.4 On the owner's Mac after a daemon reboot and one agent reboot: live status, a question answered from the web, autopilot start and stop
- [ ] 4.3 A real download of the pinned assets checked against the pins (the review compared the pins with the published checksums and the GitHub digests, without installing)
