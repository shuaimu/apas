## ADDED Requirements

### Requirement: A failed terminal launch is reported in its tab
When a terminal tab's provider cannot be started, on creation, restore, reboot or provider switch, the CLI SHALL report the tab at once as an exited terminal whose status is the reason, and SHALL report the same reason after every reconnect while the tab stays unstarted. A tab SHALL NOT be left with no reported terminal state.

#### Scenario: OMP missing and not installable
- **WHEN** an OMP tab is opened on a machine whose explicitly configured `omp_path` does not exist
- **THEN** the tab shows an ended terminal naming the missing executable and how to configure it

### Requirement: Machines report their providers
A daemon SHALL report, for Claude, Codex, OpenCode and OMP, whether it finds the executable a launch would use, its version when known, and whether APAS can install it there; it SHALL report an install as possible only where a launch would perform one. A viewer SHALL treat a missing report as unknown, never as not installed. When creating a tab, the provider choice for the machine that will get it SHALL say "Installs OMP first" where the machine will install OMP, and SHALL be disabled with "<Provider> is not installed on <host>." where the provider is missing and cannot be installed. A machine itself SHALL NOT be disabled because one provider is missing.

#### Scenario: A Mac without OMP
- **WHEN** the person adds a tab to a project whose only instance runs on a Mac that reports OMP missing and installable
- **THEN** OMP is offered with "Installs OMP first", and Claude is offered unchanged if the Mac has it

#### Scenario: An older daemon
- **WHEN** a machine's daemon reports no provider list
- **THEN** the provider choices are shown as before

### Requirement: OMP is installed when an OMP tab needs it
When an OMP tab is opened, restored or rebooted on a machine with no `omp` on the daemon's PATH, no verified host-local copy and nothing at `~/.local/bin/omp`, and `omp_path` is the bare name or `~/.local/bin/omp`, the CLI SHALL install the OMP version APAS has pinned and then start the tab. The install SHALL use the official release asset for the machine's platform, SHALL refuse unless the asset matches the pinned SHA-256 and size and the release's published checksum list names the same hash, SHALL on macOS also require OMP's Developer ID signature, SHALL check the installed version, and SHALL run no downloaded script. It SHALL NOT replace an existing file, install to any other configured path, or edit shell configuration. Progress SHALL show in the tab's terminal, at most one install SHALL run per machine at a time, and a failure SHALL end the tab with the cause and the manual install command. Signing OMP in to a model provider SHALL remain the person's step.

#### Scenario: Checksum mismatch
- **WHEN** the downloaded asset's SHA-256 differs from the pinned one
- **THEN** nothing is published at `~/.local/bin/omp`, the partial file is removed, and the tab ends with the reason

#### Scenario: Two tabs at once
- **WHEN** two OMP tabs on one machine need OMP at the same time
- **THEN** one downloads it and the other waits and then starts from the installed copy
