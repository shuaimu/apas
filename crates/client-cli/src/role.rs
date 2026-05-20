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

/// Render the three optional fields into an `--append-system-prompt` body.
/// Returns None when all three are empty/None so the caller can skip
/// pushing the flag entirely.
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
        assert_eq!(got, "# Role\nreviewer");
    }

    #[test]
    fn all_three_separated_by_blank_lines() {
        let got = compose_system_prompt(
            Some("backend implementer"),
            Some("make auth tests green"),
            Some("project uses sqlx, NOT diesel; tests live in tests/"),
        )
        .unwrap();
        let expected = "# Role\nbackend implementer\n\n# Goal\nmake auth tests green\n\n# Backstory\nproject uses sqlx, NOT diesel; tests live in tests/";
        assert_eq!(got, expected);
    }

    #[test]
    fn skips_only_empty_section_in_the_middle() {
        let got = compose_system_prompt(Some("r"), Some("   "), Some("b")).unwrap();
        // Empty goal dropped; role + backstory remain.
        assert_eq!(got, "# Role\nr\n\n# Backstory\nb");
    }
}
