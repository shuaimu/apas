## Why

On 2026-11-15 the owner opened an OMP tab on their MacBook Air and no agent ever appeared. Three causes, all silent:

- The check for a running daemon read `/proc`, which macOS does not have, so every `apas` command started another daemon. The Mac had three, sharing one machine id and knocking each other off the server (48 reconnects in 23 minutes).
- OMP was not installed on the Mac. The launch failed, and the failure went only to a local log: the tab never reported a terminal and stayed blank.
- Nothing told the person, before opening the tab, that the machine had no OMP.

The owner asked to fix it and to "include omp in apas or auto install it".

## What Changes

- The daemon check works without `/proc` (process liveness and arguments from the system's `ps` on macOS), and a daemon holds a lock for its lifetime, so a second daemon says which one is running and exits. A daemon that finds other daemons of the same user (from an older APAS) names them and never stops them.
- On macOS without `XDG_RUNTIME_DIR`, daemon state lives in `~/Library/Application Support/apas/run`, the same in every launch context and never cleaned by the system. The first update still finds what the older APAS left in the temporary directory.
- Other Linux-only spots on the daemon and launch paths work or say what they cannot do on macOS: the login-shell PATH, the pane host's peer check, socket path length, Mach-O OMP in the host-local store.
- A terminal tab whose launch fails is reported at once as an exited terminal with the reason, and again after every reconnect, instead of staying blank.
- Each machine reports which of Claude, Codex, OpenCode and OMP it has, with versions, and whether APAS can install it. The `+` menu and the phone's Add agent sheet say "Installs OMP first" for a machine where it will, and disable a provider that is missing and cannot be installed. The Machines page lists each machine's providers.
- APAS installs OMP by itself when an OMP tab is opened, restored or rebooted on a machine with none: a pinned, verified official release, into `~/.local/bin/omp`, with progress in the tab. It runs no install script, never replaces an existing `omp`, never installs to a custom `omp_path`, and edits no shell configuration. Signing OMP in to a model provider stays the person's step.

## Capabilities

### New Capabilities

### Modified Capabilities
- `host-supervision`: one daemon per user and host on every platform, and macOS runtime state.
- `provider-support`: launch failures are reported; machines report their providers; OMP is installed when missing.

## Impact

- Shared: `MachineInfo.providers` (optional; older servers and viewers ignore it). Protocol regenerated with a fixture.
- Server: passes the field through to viewers.
- Web: `lib/machineChoice.ts`, `TabBar.tsx`, `MobileSessionActivity.tsx`, the Machines page and the phone's Machines card.
- CLI: new `process_info.rs`, `launch_status.rs`, `provider_availability.rs`, `omp_install.rs`; changes in `main.rs`, `config.rs`, `mode/daemon.rs`, `mode/dual_pane.rs`, `omp_local.rs`, `pane_host.rs`, `provider_local.rs`, `host_storage.rs`, `supervisor.rs`.
- Rollout: server, web, then CLI. A Mac that ran an older APAS must be cleaned up by hand once (see design).
