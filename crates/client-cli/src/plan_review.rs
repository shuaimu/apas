//! Phase 3.2b1: plan-review gating decision.
//!
//! Pure helper that answers: "should this incoming `can_use_tool`
//! request be held for user approval, given the pane's policy?"
//! Lives separately from the reader thread so it can be unit-tested
//! and so 3.2b2 can wire it up with a clean import surface.
//!
//! AskUserQuestion is INTENTIONALLY excluded from gating — it has its
//! own approval flow already (Phase 1 AskUserQuestion plumbing), and
//! gating it would mean "the agent's question to the user blocks until
//! the user approves the agent's question," which is silly.

use shared::PlanReviewMode;

/// Tools that mutate the project (file system / shell). When the pane's
/// policy is `RiskyOnly` we gate exactly these; the rest auto-approve.
/// Conservative list: any tool that could persist a change, plus Task
/// (since a subagent could do anything).
const RISKY_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit", "Bash", "Task"];

/// Tools that bypass gating regardless of mode. AskUserQuestion has its
/// own user-approval mechanism (Phase 1) and gating it would deadlock.
const ALWAYS_PASS_TOOLS: &[&str] = &["AskUserQuestion"];

pub fn should_hold_tool(mode: PlanReviewMode, tool_name: &str) -> bool {
    if ALWAYS_PASS_TOOLS.iter().any(|t| *t == tool_name) {
        return false;
    }
    match mode {
        PlanReviewMode::Never => false,
        PlanReviewMode::Always => true,
        PlanReviewMode::RiskyOnly => RISKY_TOOLS.iter().any(|t| *t == tool_name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_mode_never_holds() {
        for tool in [
            "Write",
            "Edit",
            "Bash",
            "Read",
            "AskUserQuestion",
            "Unknown",
        ] {
            assert!(
                !should_hold_tool(PlanReviewMode::Never, tool),
                "Never should not hold {}",
                tool,
            );
        }
    }

    #[test]
    fn always_mode_holds_everything_except_ask_user_question() {
        for tool in ["Write", "Edit", "Bash", "Read", "Glob", "Task"] {
            assert!(
                should_hold_tool(PlanReviewMode::Always, tool),
                "Always should hold {}",
                tool,
            );
        }
        assert!(
            !should_hold_tool(PlanReviewMode::Always, "AskUserQuestion"),
            "AskUserQuestion always bypasses (own approval flow)",
        );
    }

    #[test]
    fn risky_only_mode_holds_risky_tools() {
        for tool in &["Write", "Edit", "MultiEdit", "NotebookEdit", "Bash", "Task"] {
            assert!(
                should_hold_tool(PlanReviewMode::RiskyOnly, tool),
                "RiskyOnly should hold {}",
                tool,
            );
        }
    }

    #[test]
    fn risky_only_mode_lets_safe_tools_through() {
        for tool in &[
            "Read",
            "Glob",
            "Grep",
            "LS",
            "TodoWrite",
            "WebFetch",
            "AskUserQuestion",
        ] {
            assert!(
                !should_hold_tool(PlanReviewMode::RiskyOnly, tool),
                "RiskyOnly should NOT hold {}",
                tool,
            );
        }
    }

    #[test]
    fn unknown_tool_in_risky_only_is_treated_as_safe() {
        // Conservative-by-name: only the named risky tools are held.
        // If claude adds a new mutating tool, we'll need to update
        // RISKY_TOOLS — but in the meantime an unknown tool passes
        // through to avoid blocking legitimate work.
        assert!(!should_hold_tool(PlanReviewMode::RiskyOnly, "SomeNewTool"));
    }
}
