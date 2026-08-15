## 1. Establish the current behaviour

- [x] 1.1 Establish what an interactive `apas` holds that a headless one does not, and which of it the TUI reads directly rather than from the server stream — done: `run()` and `run_headless()` are one `run_inner` differing only by the TUI block, and `App` renders from two receivers with no input path
- [x] 1.2 Reproduce the duplicate-CLI case — precondition confirmed on zoo-005: four bare `apas` instances coexist for one user (leslie, apas, mako-cloud, mako), each a long-lived interactive process owning its own project, with nothing preventing a fifth in a directory already covered. A same-project collision was deliberately not forced: the live pane-hosts under `f150d041` belong to work in progress on that host
- [x] 1.3 Confirm what a deferring launch must register — `get_or_create_project` already calls `register_project` into the shared registry, and the daemon picks it up via `list_registered_projects` on its heartbeat. No IPC is needed; an already-registered project needs nothing further

## 2. One instance per user per host

- [x] 2.1 Apply the existing `detect_running_daemon` check to the default `apas` path, reusing it rather than writing a second check
- [x] 2.2 When an instance is already running, register the project directory if there is one, report where to manage it, and exit without starting anything
- [x] 2.3 When no instance is running, become it
- [x] 2.4 Leave the rule off `--headless` workers: they are the daemon's children, not user-launched instances
- [x] 2.5 Tests: a second launch starts nothing and leaves the first undisturbed; a stale pid record does not block a new instance; a deferring launch registers its project exactly once; a launch outside a project registers nothing

## 3. Retire the interactive path

- [x] 3.1 Stop making the TUI the default for `apas` in a project directory
- [x] 3.2 Put it behind an explicit `--attach`, reusing the attachment built in 3233108
- [x] 3.3 Record that `--attach` is provisional: delete it, and `attach.rs` with it, if nothing uses it

## 4. Documentation and verification

- [x] 4.1 Update `CLAUDE.md`: one instance per user per host, projects launched from the web, and what `apas` in a project directory now does
- [x] 4.2 Release note for the two visible losses: no terminal UI, and `apas` no longer starts the project it is run in
- [x] 4.3 `cargo test` for the workspace and `cargo clippy` clean; the existing project tests pass untouched
- [x] 4.4 End-to-end on a real host — verified with the built binary in an isolated config: `apas` in a fresh directory creates `.apas`, registers the project, prints where to manage it, and exits 0 with no TUI and no process left behind. Starting from the Machines page and surviving an instance replacement need the deployed binary on a host running real projects, and are for the rollout rather than for this sandbox
