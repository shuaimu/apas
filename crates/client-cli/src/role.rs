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
}
