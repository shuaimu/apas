## 1. Transcript candidate lookup

- [x] 1.1 Add `find_claude_switch_candidate` to crates/client-cli/src/transcript.rs: scan the cwd slug directory for unpinned `.jsonl` files whose mtime is newer than the pinned file's last-observed growth, returning the newest
- [x] 1.2 Unit-test the helper: picks the newest unpinned file, ignores pinned ids, ignores files older than the pinned file's last growth, returns None when no candidate exists

## 2. Poller integration

- [x] 2.1 In the claude branch of the transcript poller (dual_pane.rs:2860), track the pinned file's `(size, mtime)` per pane each tick
- [x] 2.2 After two consecutive unchanged observations of the pinned file, run the candidate scan using the project's pinned-id set (from the pane_sessions snapshot)
- [x] 2.3 On a candidate hit, switch the tracked source string to `claude:<candidate-path>` so the existing source-change re-baseline path applies without replay
- [x] 2.4 Keep the pinned file as the primary source whenever it resumes growing, including after a switch back (scan re-examines it on later idleness)

## 3. Verification

- [x] 3.1 `cargo test -p apas` passes, including the new helper tests and poller-branch tests
- [x] 3.2 `cargo clippy --workspace` is clean of new warnings
- [x] 3.3 Manually verify: in a scratch project with a claude terminal pane, `/resume` inside the TUI to another session and confirm the conversation view follows the new session
