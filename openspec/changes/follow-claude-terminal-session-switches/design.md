## Context

See proposal.md - Why. The transcript poller's claude branch (crates/client-cli/src/mode/dual_pane.rs:2860-2874) reads one fixed path: `~/.claude/projects/<cwd-slug>/<pane-session-id>.jsonl`, where the pane session id was pinned at spawn with `--session-id`. The poller already handles source identity changes generically: `seen: HashMap<u32, (String, usize)>` keys on a source string, and a changed source re-baselines the cursor to the new transcript's end (dual_pane.rs:2892-2906), which is exactly the "no replay" behavior the spec requires. Claude writes one `.jsonl` per session inside the cwd slug directory; APAS pins every claude session it spawns (agent and terminal panes), so the set of pinned ids is knowable from the pane registry (`.apas` `panes[].session_id`).

## Goals / Non-Goals

**Goals:**

- Follow in-TUI session switches for claude terminal panes with a safe, polling-based heuristic.
- Never abandon a transcript that is still growing; never replay history on a switch; never follow a sibling pane's pinned file.

**Non-Goals:**

- Changing codex or opencode lookup (already heuristic).
- Detecting switches with perfect certainty (no provider API exposes the active session; heuristic by design).
- Web/server changes: the emitted stream messages are identical in shape.

## Decisions

1. **Candidate rule: unpinned files in the cwd slug directory.** A session file is a switch candidate when its name is not in the project's pinned-id set and its metadata indicates it is newer than the pinned file's last growth. The pinned set is collected from the same `pane_sessions` map the poller already snapshots each tick. Rationale: APAS pins every claude session it spawns, so unpinned files are necessarily human-created inside a TUI (or a manual claude run); this is the strongest available filter. Alternative considered: newest-file-wins with no filter — rejected because sibling panes' pinned files and claude agent panes would steal tracking.

2. **Idle guard: size+mtime unchanged for two consecutive polls.** The poller records `(size, mtime)` for the pinned file each tick and only scans for candidates after two consecutive unchanged observations. Rationale: a long streaming turn keeps the file growing; one missed poll must not trigger a switch. Two polls ≈ 6 seconds of idleness, which also covers claude's write buffering between turns.

3. **Switch action reuses the existing source-change path.** When a candidate is chosen, the new source string becomes `claude:<candidate-path>` and the existing `tracked_source != source` branch re-baselines. The pinned path is only re-tracked when the user resumes it (candidate scan re-examines it on later idleness). No new wire messages.

4. **Candidate scan helper lives in transcript.rs.** `find_claude_switch_candidate(home, cwd_slug_dir, pinned_id, pinned_meta, pinned_ids) -> Option<PathBuf>` returns the newest-mtime unpinned `.jsonl` newer than the pinned file's last growth, if any. Keeps dual_pane's poller thin and the logic unit-testable against a temp dir.

5. **Candidate freshness is bounded by the pane, not the directory.** Only files whose mtime is strictly newer than the pinned file's last-observed growth are candidates, so a stale manual-claude session predating the switch does not win.

## Risks / Trade-offs

- [Two panes in one cwd both switch inside their TUIs] Both would follow the newest unpinned file. → Accepted (same class of ambiguity codex already has); the pinned-set filter still prevents cross-pane stealing of *pinned* sessions.
- [Manual `claude` run in the same cwd while the pane is idle] Could steal tracking. → Mitigated by idle+newer-than-pinned gating; accepted as a rare shared-machine edge case, documented in the spec's "unpinned file" definition.
- [Claude rotates or truncates the pinned file] The existing cursor-clamping already handles this; the idle guard adds no new hazard.
- [Poller scans the slug dir every idle tick] Directory listing of a few dozen files every 6s per claude pane is negligible.

## Migration Plan

CLI-only change. Ship with the normal CLI build; existing panes behave identically until their first in-TUI switch. No server dependency, no data migration, rollback is binary rollback.

## Open Questions

(none)
