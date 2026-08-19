## 1. Record what the provider reports

- [x] 1.1 A hidden subcommand that reads the hook payload on stdin and records the transcript path it names
- [x] 1.2 Resolve which pane the report belongs to from the environment the provider was started with
- [x] 1.3 Write it atomically into a per-pane runtime directory, owner-only and outside the project
- [x] 1.4 Never fail: a hook that errors is noise in the user's terminal as their agent starts, and derivation remains
- [x] 1.5 Tests: a payload becomes a report, an unusable one records nothing, a corrupt file reads as absent, and the settings document carries only hooks

## 2. Install it on Claude panes

- [x] 2.1 Write the settings document and pass it when spawning, on both the hosted and direct paths
- [x] 2.2 Set the pane environment variable the hook resolves itself from
- [x] 2.3 Leave other providers untouched
- [x] 2.4 Tests: the spawned command carries the settings flag ahead of the pinned session id

## 3. Prefer the report

- [x] 3.1 The watcher reads the reported transcript, falling back to derivation when nothing is reported
- [x] 3.2 Re-point whenever the reported path changes, since that is the provider stating the session changed
- [x] 3.3 Verified end to end against the real provider: the hook fires, the subcommand records, and the recorded path exists

## 4. Documentation and verification

- [x] 4.1 Update `CLAUDE.md`, which documents the derivation as exact
- [x] 4.2 Workspace tests and clippy clean
- [ ] 4.3 Live on a real pane: resume a different session in the TUI and confirm the conversation follows
