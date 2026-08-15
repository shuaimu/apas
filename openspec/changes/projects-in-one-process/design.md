## Context

See proposal.md — Why. What the code actually looks like, checked rather than assumed:

- **`run_inner` is already close to reentrant.** `run()` and `run_headless()` are one function differing by a `headless: bool`. It takes `working_dir` as a parameter, and there is no `set_current_dir` anywhere in the CLI, so two projects in one process do not fight over the process working directory.
- **No mutable statics in the project path**, so nothing collides through globals. Per-project runtime directories are already keyed by project id.
- **Nothing in the project path reads stdin**, and nothing installs a signal handler, so there is no single-owner resource to contend for.
- **`panic = "abort"` is not set.** A panicking task or thread unwinds alone. Shared fate is therefore narrower than it appears: it is `process::exit`, abort, and OOM — not panics.
- **Three `std::process::exit` calls** (`dual_pane.rs:3382`, `3400`, `12626`) are the real obstacle. Each means "stop this project" and would come to mean "stop every project". Two are the reboot path; the third fires when the server rejects a session.
- **Roughly thirty `thread::spawn` per project**, almost all blocking readers over subprocess output. They must stay threads; what changes is that N projects means ~30N in one process.

## Goals / Non-Goals

**Goals:**

- One process per user per host running every project on it.
- One project's failure — panic, rejected session, unrecoverable error — costs only that project.
- Running state held rather than inferred.
- Restarting one project without replacing the instance.

**Non-Goals:**

- Merging pane hosts. They own the PTYs so a provider survives the CLI being replaced; folding them in would undo the reason they exist.
- Rewriting what a project does. Panes, worktrees, team mode, transcripts, and the server connection are untouched; what changes is who hosts them.
- Reducing the thread count. That is a consequence to measure, not a goal to chase in this change.
- Removing cross-host claims, which answer a different question.

## Decisions

1. **A project becomes a supervised task, not a thread or a child process.** `run_inner` is already async and already takes its project as a parameter; the daemon holds a `JoinHandle` and a cancellation signal per project. This is the smallest change that makes the daemon hold what it currently guesses at.

2. **The three exits become returns.** Each is "stop this project" expressed as "stop the process" because a process only ever had one project. `12626` in particular — a rejected session — must stop one project and leave the rest, which is the difference between a contained failure and a host-wide outage.

3. **Reboot becomes replace-the-task.** Today a project reboot `exec`s the binary, which is only correct when the process is the project. Merged, it cancels the task and starts a fresh one. Upgrading the *instance* still replaces the process, and the projects it was running are started again after.

4. **Failure containment relies on unwind, and is verified rather than assumed.** Because `panic = "abort"` is unset, a panicking project task unwinds alone. The change must not introduce `panic = "abort"`, and a test should hold that line, since flipping it would silently convert every project panic into a host outage.

5. **Per-project attribution comes from tracing spans.** Today separation is physical — a tmux session and a stderr log per project. Merged, every project's records must carry its identity, or the first production incident becomes unreadable. This is not cosmetic: it replaces the diagnostic property the process boundary was providing for free.

6. **Pane hosts stay separate, and that is what keeps this safe.** The terminal panes people actually watch already survive the CLI being replaced. Merging projects therefore risks the supervision layer, not the agent processes.

## Risks / Trade-offs

- [~30 threads per project now share one process] → Ten projects is ~300 threads plus pane hosts. Threads are cheap but not free, and this is the one resource consequence that could bite at scale. Measure with several projects running before rolling out, and treat converting blocking readers to tasks as a follow-up if it matters.
- [OOM and abort become host-wide] → Unchanged for abort (nothing aborts today), real for OOM: one project's runaway memory now takes the others. This is the genuine cost of the merge and cannot be designed away, only monitored.
- [Losing the tmux boundary loses free per-project isolation and logging] → Decision 5 replaces the logging half deliberately. The isolation half is what decisions 2 and 4 replace, and the parts that must not be lost — surviving a CLI replacement — belong to pane hosts, which are untouched.
- [A blocking call on the async runtime would stall other projects] → The blocking work already lives on threads; the risk is new code putting blocking work on the runtime. Worth stating in `CLAUDE.md` next to the merge, because it is the failure mode that will not show up in a two-project test.
- [`--headless` stops being how projects run] → Kept as a way to run one project alone for debugging, which is also the escape hatch if a project turns out to misbehave in a shared process.

## Migration Plan

The instance is replaced by the ordinary self-upgrade, and it starts its projects itself afterwards. A host part-way through has an older daemon with tmux'd project processes and a newer one with tasks; the newer daemon adopts nothing from the older model, so the tmux'd projects must be stopped as part of the upgrade rather than left running — otherwise a project runs twice on one host. That makes this the first change in this sequence that is not purely additive at rollout, and it is why the cross-host claim stays: it is what prevents the same mistake between hosts.

Rollback is a binary swap plus stopping the in-process projects, for the same reason in reverse.
