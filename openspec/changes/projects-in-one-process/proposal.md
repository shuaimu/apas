## Why

A host runs one APAS instance per user, but it is still one process per project underneath: the daemon spawns `apas --headless -d <path>` into a per-project tmux session and then cannot see it, which is why running state is inferred from `/proc` and why a restarted daemon comes back owning nothing. The single-instance rule fixed who may *launch* a project; it did not reduce what runs.

The obstacles to merging turn out to be smaller than they look. There is no `set_current_dir` anywhere in the CLI, so projects do not fight over the process working directory. There are no mutable statics in the project path, so nothing collides through globals. Nothing in the project path reads stdin and nothing installs a signal handler. The build does not set `panic = "abort"`, so a panicking project task or thread unwinds alone rather than taking the process with it.

What is actually in the way is three `std::process::exit` calls that mean "stop this project" and would mean "stop every project", and a supervision model built around a child process rather than a task.

## What Changes

- **The daemon runs each project in-process** as a supervised task, instead of spawning a headless CLI into tmux. A host goes from one daemon plus N project processes to one process.
- **The three exits become per-project teardown.** A project whose session the server rejects stops that project and reports it, rather than ending the process. A reboot restarts that project's task, rather than `exec`ing the binary.
- **Per-project failure is contained.** A project that panics unwinds its own task and is reported as stopped; the others keep running.
- **Running state stops being inferred.** With projects as tasks the daemon holds them directly, so `/proc` scanning is no longer how it answers what is running here.
- **Pane hosts are deliberately not merged.** They own the PTYs and exist precisely so a provider survives the CLI being replaced; folding them in would undo that. A host ends up running one `apas` plus one pane host per terminal pane.
- **Per-project logs and diagnostics survive the merge.** Today each project has its own tmux session and its own stderr log; merged, they share a process, so per-project identification has to come from the log records themselves.
- **BREAKING**: `apas --headless -d <path>` stops being how a project runs. It stays as a way to run one project alone for debugging, but nothing spawns it.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `host-supervision`: projects become tasks of the resident instance rather than separate processes, which changes what "running" is answered from, what a project restart does, and what one project's failure costs the others.

## Impact

- `crates/client-cli/src/mode/dual_pane.rs`: `run_inner` becomes callable many times in one process — the three `process::exit` sites become returns, and shutdown becomes per-project rather than process-wide.
- `crates/client-cli/src/mode/daemon.rs`: `start_project` spawns a task instead of a tmux session; `stop_project` cancels it; `snapshot_projects` reports from the task table; the tmux and `/proc` machinery for project CLIs retires.
- Thread count: each project spawns roughly thirty threads for blocking readers, and they now share a process. This is the main resource consequence and needs measuring rather than assuming.
- Cross-host claims are unchanged; they answer which host may run a project, which merging does not touch.
- Docs: the daemon and process-model sections of `CLAUDE.md`.
