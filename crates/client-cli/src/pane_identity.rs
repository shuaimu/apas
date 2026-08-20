//! A pane's own identity, as a system prompt.
//!
//! Role, goal and backstory are per-pane metadata the user sets from the role
//! modal; they are appended to whatever the provider is told at spawn. This
//! used to live in `role.rs` beside the four managed team roles, and used to
//! append the team protocol — how to read the scratchpad, how to receive a
//! delegation, how to publish a diff. Team mode is gone, so what remains is the
//! part that was never about a team: a pane can be told who it is.

/// Compose a pane's system prompt from its identity fields.
///
/// Returns `None` when the pane has no identity at all, so a pane the user
/// never described is spawned exactly as it would have been.
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
    (!sections.is_empty()).then(|| sections.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pane_with_no_identity_gets_no_system_prompt() {
        // Not an empty prompt: the pane must spawn exactly as an undescribed
        // pane always did.
        assert_eq!(compose_system_prompt(None, None, None), None);
        assert_eq!(compose_system_prompt(Some("  "), Some(""), None), None);
    }

    #[test]
    fn each_field_becomes_its_own_section() {
        let prompt = compose_system_prompt(Some("reviewer"), Some("review diffs"), None).unwrap();
        assert!(prompt.contains("# Role\nreviewer"));
        assert!(prompt.contains("# Goal\nreview diffs"));
        assert!(!prompt.contains("# Backstory"));
    }

    #[test]
    fn a_recognised_role_name_no_longer_carries_a_team_protocol() {
        // "tech lead" used to append the orchestration protocol, and every
        // other role the worker protocol. A role is just a description now.
        let lead = compose_system_prompt(Some("tech lead"), None, None).unwrap();
        let other = compose_system_prompt(Some("anything else"), None, None).unwrap();
        for prompt in [&lead, &other] {
            assert!(!prompt.contains("scratchpad"), "{prompt}");
            assert!(!prompt.contains("delegate"), "{prompt}");
            assert!(!prompt.contains("team-todo"), "{prompt}");
        }
        assert_eq!(lead, "# Role\ntech lead");
    }
}
