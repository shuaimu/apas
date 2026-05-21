//! Phase 2.1b: compose role/goal/backstory into a system-prompt prefix.
//!
//! The three fields live on `PaneConfig` / `PaneMeta` (Phase 2.1a). When
//! any of them is set on a pane that runs claude, we render a small
//! markdown-styled string and pass it via `--append-system-prompt` at
//! spawn time so the agent self-identifies. Empty sections are dropped
//! so a pane with only a goal doesn't see empty `# Role` / `# Backstory`
//! blocks.
//!
//! Non-claude providers (codex, opencode, cursor) currently ignore this —
//! the plan calls for env-var or system-message fallback as a follow-up.

/// Static one-paragraph note about the team scratchpad. Appended when at
/// least one of role/goal/backstory is set, so the agent already has an
/// identity to anchor the note to. Phase 2.2c.
const SCRATCHPAD_NOTE: &str = "\
# Team scratchpad
Other panes in this project share a project-local append-only log at \
`.apas-team.jsonl` (one JSON record per line: \
`{\"ts\":\"...\",\"pane_id\":<id>,\"tags\":[...],\"kind\":\"diff|review|decision|status\",\"body\":\"...\"}`). \
Publish anything worth other panes seeing — diffs, reviews, decisions — by appending a line with `>>` redirection or the Write tool. \
Read it (`tail -f` / cat) when you want to see what they've done.";

/// Phase 3.1b: additional protocol paragraph injected for panes whose
/// role is manager-shaped. Teaches the two scratchpad tag conventions
/// that the CLI watcher actually routes on (Phase 3.1a):
///   - `delegate-to:<pane_id>` → CLI sends `body` into that pane's input
///   - `reply-to:<task_id>` → bookkeeping for the manager's bookkeeping
/// We deliberately do NOT enumerate available worker pane ids here —
/// the agent can grep the project's `.apas` file (or ask the human) for
/// current roles. Hard-coding sibling info would force a plumbing leaf
/// (siblings list into compose_system_prompt) and rot quickly.
const MANAGER_NOTE: &str = "\
# Manager protocol
You are the manager for this project. To dispatch work to another pane, \
append a record to `.apas-team.jsonl` with `tags` containing \
`delegate-to:<pane_id>` and (optionally) a unique `task-id:<uuid>` tag. \
The CLI watches the file and routes the record's `body` into the target \
pane's input queue as if a user had typed it. \
Workers reply by appending their own record with `tags: [\"reply-to:<task_id>\"]`; \
poll the scratchpad to collect replies. \
You can discover the available workers by reading `.apas` in the project root \
(`panes[]` lists each pane's id, label, role, goal — Phase 2.1).";

fn role_is_manager(role: Option<&str>) -> bool {
    role.map(str::trim)
        .map(|r| r.to_ascii_lowercase().contains("manager"))
        .unwrap_or(false)
}

/// Render the three optional fields into an `--append-system-prompt` body.
/// Returns None when all three are empty/None so the caller can skip
/// pushing the flag entirely. When at least one is set, the team
/// scratchpad note (Phase 2.2c) is appended too so the agent knows
/// about the cross-pane communication channel.
pub fn compose_system_prompt(
    role: Option<&str>,
    goal: Option<&str>,
    backstory: Option<&str>,
) -> Option<String> {
    let mut sections: Vec<String> = Vec::new();
    if let Some(r) = role.map(str::trim).filter(|s| !s.is_empty()) {
        sections.push(format!("# Role\n{}", r));
    }
    if let Some(g) = goal.map(str::trim).filter(|s| !s.is_empty()) {
        sections.push(format!("# Goal\n{}", g));
    }
    if let Some(b) = backstory.map(str::trim).filter(|s| !s.is_empty()) {
        sections.push(format!("# Backstory\n{}", b));
    }
    if sections.is_empty() {
        None
    } else {
        sections.push(SCRATCHPAD_NOTE.to_string());
        if role_is_manager(role) {
            sections.push(MANAGER_NOTE.to_string());
        }
        Some(sections.join("\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_returns_none() {
        assert_eq!(compose_system_prompt(None, None, None), None);
        assert_eq!(compose_system_prompt(Some(""), Some("  "), None), None);
    }

    #[test]
    fn single_section_no_extra_newlines() {
        let got = compose_system_prompt(Some("reviewer"), None, None).unwrap();
        assert!(got.starts_with("# Role\nreviewer"));
        // Phase 2.2c always appends the scratchpad note.
        assert!(got.contains("# Team scratchpad"));
        assert!(got.contains(".apas-team.jsonl"));
    }

    #[test]
    fn all_three_separated_by_blank_lines() {
        let got = compose_system_prompt(
            Some("backend implementer"),
            Some("make auth tests green"),
            Some("project uses sqlx, NOT diesel; tests live in tests/"),
        )
        .unwrap();
        assert!(got.contains("# Role\nbackend implementer"));
        assert!(got.contains("# Goal\nmake auth tests green"));
        assert!(got.contains("# Backstory\nproject uses sqlx, NOT diesel; tests live in tests/"));
        // Sections separated by blank lines, in declared order.
        let role_pos = got.find("# Role").unwrap();
        let goal_pos = got.find("# Goal").unwrap();
        let backstory_pos = got.find("# Backstory").unwrap();
        let scratchpad_pos = got.find("# Team scratchpad").unwrap();
        assert!(role_pos < goal_pos);
        assert!(goal_pos < backstory_pos);
        assert!(backstory_pos < scratchpad_pos);
    }

    #[test]
    fn skips_only_empty_section_in_the_middle() {
        let got = compose_system_prompt(Some("r"), Some("   "), Some("b")).unwrap();
        // Empty goal dropped; role + backstory remain; scratchpad note appended.
        assert!(got.contains("# Role\nr"));
        assert!(got.contains("# Backstory\nb"));
        assert!(!got.contains("# Goal"));
        assert!(got.contains("# Team scratchpad"));
    }

    #[test]
    fn empty_inputs_dont_emit_scratchpad_note_alone() {
        // Phase 2.2c: scratchpad note rides along with role/goal/backstory.
        // Without any of those, no system prompt is emitted at all.
        assert_eq!(compose_system_prompt(None, None, None), None);
    }

    #[test]
    fn manager_role_gets_protocol_addendum() {
        let got = compose_system_prompt(Some("manager"), None, None).unwrap();
        assert!(got.contains("# Manager protocol"));
        assert!(got.contains("delegate-to:<pane_id>"));
        assert!(got.contains("reply-to:<task_id>"));
        // Section ordering: role → scratchpad → manager
        let role_pos = got.find("# Role").unwrap();
        let scratchpad_pos = got.find("# Team scratchpad").unwrap();
        let manager_pos = got.find("# Manager protocol").unwrap();
        assert!(role_pos < scratchpad_pos);
        assert!(scratchpad_pos < manager_pos);
    }

    #[test]
    fn manager_detection_is_case_insensitive_and_substring() {
        for r in ["manager", "Manager", "MANAGER", "team manager", "manager-agent"] {
            assert!(role_is_manager(Some(r)), "role {:?} should be manager", r);
        }
        for r in ["reviewer", "backend", "", "mgr"] {
            assert!(!role_is_manager(Some(r)), "role {:?} should NOT be manager", r);
        }
        assert!(!role_is_manager(None));
    }

    #[test]
    fn non_manager_role_skips_protocol_addendum() {
        let got = compose_system_prompt(Some("reviewer"), None, None).unwrap();
        assert!(!got.contains("# Manager protocol"));
        // But scratchpad note still rides along.
        assert!(got.contains("# Team scratchpad"));
    }
}
