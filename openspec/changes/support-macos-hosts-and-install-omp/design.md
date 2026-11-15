## Context

The CLI grew on Linux. A MacBook Air joined the owner's cluster and ran into the places that assumed `/proc`, `XDG_RUNTIME_DIR` and an OMP already installed. Research on OMP's installer, release assets and signing is in the session scratchpad (`omp-research/`); nothing downloaded was run.

## Decisions

**Process checks.** Liveness is `kill(pid, 0)` (EPERM counts as alive). Arguments, the process list and memory come from `/proc` on Linux and from `/bin/ps` or `/usr/bin/ps` elsewhere, never a `ps` from the user's PATH, with a 5 s timeout. The match rule for a daemon is unchanged.

**Daemon lock.** `apas daemon` holds an exclusive lock on `<runtime dir>/daemon.lock` for its life. The descriptor is close-on-exec, so a requested restart releases it and the new image, with the same pid, takes it again. The running daemon rewrites `daemon.json` every 20 s if it is missing or names another pid. Daemons from an older APAS take no lock; a new daemon names them and never kills them, because stopping a daemon stops its projects.

**macOS runtime dir.** `~/Library/Application Support/apas/run` (0700) when `XDG_RUNTIME_DIR` is unset. `$TMPDIR` differs between a GUI session, ssh and launchd, and the system cleans it. A first update from an older APAS execs with the old `$TMPDIR`, so the new image also takes the resume manifest the old one wrote there and lists pane hosts under the old root; it deletes nothing there.

**Install OMP, do not bundle it.** The OMP binary is 224 to 290 MB per platform and updates on its own schedule. APAS pins one version it has checked (18.20.2) with the SHA-256 and size of each asset for darwin-arm64, darwin-x64, linux-x64 and linux-arm64, downloads the GitHub release asset, and requires the release's `SHA256SUMS.txt` to list the pinned hash and the file to match it. On macOS it also requires `codesign --verify --strict` and the Developer ID team `VTLHGQC72S`. It runs `--version` before publishing. The official `install.sh` is not used: its only checksum comes from the same service that hands out the download, and it would mean running downloaded code. musl hosts and Windows are refused with the reason.

**Where.** `~/.local/bin/omp`, the place the official installer uses and where the Linux machines already have it, published with a hard link so an existing file is never replaced and a partial download never appears under that name. From there the existing host-local copy and its `omp update` path take over. The install runs only when no `omp` is on PATH, no verified host-local copy exists and `~/.local/bin/omp` is missing, and only when `omp_path` is the bare name or `~/.local/bin/omp`. One install runs per machine under a lock; its 15-minute limit starts when the lock is taken; partial downloads older than that are removed by the next install.

**Report and launch decide the same way.** A machine reports `can_install` for OMP from the same function the launch uses, so "Installs OMP first" appears only where the launch installs.

**Where the note goes.** On the provider button for the machine that will get the tab, not on the machine row: a Mac without OMP still takes a Claude tab.

## Risks

- None of the macOS-only code was compiled or run: no macOS target is installed on the build hosts and there is no Mac here. Linux runs the same portable code paths in tests where possible.
- On macOS, OMP tabs get status from the transcript only, questions are answered in the terminal, and autopilot is unavailable (the extension's process identity is Linux-only). This is logged, not fixed.
- The machine id derives from user@hostname; a Mac's hostname can change (Bonjour `-2`, DHCP), making it a new machine. Logged only.
- Any user with runtime access to a project on a machine can trigger an OMP install there.
- "Reboot to update" on a Mac needs git and a Rust toolchain.

## Migration

A Mac that ran APAS 26.11.54 or older and has several daemons: stop every `apas … daemon` by hand, keeping none; update the CLI; start one daemon; start the project again from the web. Do not rely on Reboot to update there: the new image can read the old `daemon.json`, find another old daemon alive and exit.
