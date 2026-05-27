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

/// v3 split: the user-facing **Manager** is the primary point of contact
/// for the human. They clarify requirements, keep `project_goal.md` in
/// sync with the conversation, and hand off autonomous orchestration to
/// a Tech-Lead pane via the same `delegate-to:` scratchpad protocol.
/// Crucially, the Manager does NOT delegate directly to workers — that's
/// the Tech Lead's responsibility — so the user has one coherent layer
/// to talk to and one autonomous layer to grind.
const MANAGER_NOTE: &str = "\
# Manager protocol
You are this project's manager — the user-facing role. You chat directly with the human, ask clarifying questions, and keep `project_goal.md` in sync with what the human wants. \
When you need autonomous orchestration (workers running in the background), delegate to the Tech-Lead pane: append a record to `.apas-team.jsonl` with `kind: \"delegation\"` and `tags` containing `delegate-to:<tech_lead_pane_id>`. \
The CLI watches the file and routes the record's `body` into the Tech Lead's input queue as if a user had typed it. \
The Tech Lead replies via `tags: [\"reply-to:<task_id>\"]` on the same file. \
Do NOT delegate to worker panes yourself — workers receive their assignments from the Tech Lead. Your job is conversation, not tactical orchestration. \
You CAN use the Write tool on `project_goal.md` directly — that file is yours to maintain. \
Read `.apas` in the project root to discover the Tech-Lead pane id (look for the pane whose role contains \"tech lead\").";

/// v3 split: the autonomous **Tech Lead** is the deadloop orchestrator
/// (this is what was previously called the "manager" role). Reads the
/// goal + scratchpad each iteration and dispatches work to workers.
/// Receives delegations from the Manager pane on `.apas-team.jsonl`
/// (kind: "delegation", delegate-to:<this_pane_id>).
const TECH_LEAD_NOTE: &str = "\
# Tech Lead protocol
You are this project's tech lead — the autonomous orchestrator. You read `project_goal.md` and `.apas-team.jsonl` each iteration to understand the state of the world, and dispatch work to specialist worker panes. \
To dispatch work to a worker, append a record to `.apas-team.jsonl` with `kind: \"delegation\"` and `tags` containing `delegate-to:<worker_pane_id>` and (optionally) a unique `task-id:<uuid>` tag. \
The CLI watches the file and routes the record's `body` into the target pane's input queue as if a user had typed it. \
Workers reply by appending their own record with `tags: [\"reply-to:<task_id>\"]`; \
poll the scratchpad to collect replies. \
You also receive delegations from the Manager pane — look for records on `.apas-team.jsonl` with `tags` containing `delegate-to:<your_pane_id>`. Treat these as high-priority goal updates from the human. \
You can discover the available workers by reading `.apas` in the project root (`panes[]` lists each pane's id, label, role, goal). \
Do NOT chat directly with the human — that's the Manager's job. If you have a question for the human, escalate it as a `kind: \"escalation\"` record on the scratchpad and let the Manager surface it.";

/// Phase 3.3a: additional protocol paragraph for panes whose role is
/// reviewer-shaped. Teaches the diff-subscribe / review-publish loop.
/// Kept symmetric with [`MANAGER_NOTE`] — different role, same
/// scratchpad-as-bus pattern.
const REVIEWER_NOTE: &str = "\
# Reviewer protocol
You are an auto-reviewer for this project. Subscribe to \
`.apas-team.jsonl` (tail -f or periodic re-read) and look for records \
with `kind: \"diff\"`. For each one, read the diff body, evaluate it \
against the project's conventions, and append your verdict as a new \
record with `kind: \"review\"` and `tags` containing either \
`approves:<task_id>` or `rejects:<task_id>` (the task_id comes from the \
original diff record's tags, if present). \
Keep reviews short — the goal is to give the human a one-line read on \
each change so they can rubber-stamp common cases. \
Do NOT publish diffs yourself unless you're also a worker — your job \
is to react to other panes' output.";

/// v3: "manager" substring → user-facing Manager. Excludes "tech lead"
/// so the legacy role string "team manager / tech lead" routes to the
/// Tech-Lead protocol instead (it's an orchestrator role, not a
/// user-facing chat role).
fn role_is_manager(role: Option<&str>) -> bool {
    role.map(str::trim)
        .map(|r| {
            let lower = r.to_ascii_lowercase();
            lower.contains("manager") && !lower.contains("tech lead")
        })
        .unwrap_or(false)
}

/// v3: "tech lead" substring → autonomous orchestrator. Matches both the
/// new role string `"tech lead"` and the legacy `"team manager / tech lead"`
/// so existing panes keep their orchestrator addendum after the split.
fn role_is_tech_lead(role: Option<&str>) -> bool {
    role.map(str::trim)
        .map(|r| r.to_ascii_lowercase().contains("tech lead"))
        .unwrap_or(false)
}

fn role_is_reviewer(role: Option<&str>) -> bool {
    role.map(str::trim)
        .map(|r| r.to_ascii_lowercase().contains("reviewer"))
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
        if role_is_tech_lead(role) {
            sections.push(TECH_LEAD_NOTE.to_string());
        } else if role_is_manager(role) {
            sections.push(MANAGER_NOTE.to_string());
        }
        if role_is_reviewer(role) {
            sections.push(REVIEWER_NOTE.to_string());
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
    fn manager_role_gets_user_facing_protocol_addendum() {
        let got = compose_system_prompt(Some("team manager"), None, None).unwrap();
        assert!(got.contains("# Manager protocol"));
        assert!(got.contains("user-facing role"));
        assert!(got.contains("delegate-to:<tech_lead_pane_id>"));
        // Manager owns project_goal.md.
        assert!(got.contains("project_goal.md"));
        // Section ordering: role → scratchpad → manager
        let role_pos = got.find("# Role").unwrap();
        let scratchpad_pos = got.find("# Team scratchpad").unwrap();
        let manager_pos = got.find("# Manager protocol").unwrap();
        assert!(role_pos < scratchpad_pos);
        assert!(scratchpad_pos < manager_pos);
    }

    #[test]
    fn tech_lead_role_gets_orchestrator_protocol_addendum() {
        let got = compose_system_prompt(Some("tech lead"), None, None).unwrap();
        assert!(got.contains("# Tech Lead protocol"));
        assert!(got.contains("autonomous orchestrator"));
        assert!(got.contains("delegate-to:<worker_pane_id>"));
        // Tech Lead does NOT get the user-facing Manager protocol — they
        // never chat with the human.
        assert!(!got.contains("# Manager protocol"));
    }

    #[test]
    fn legacy_manager_tech_lead_role_routes_to_tech_lead() {
        // Pre-v3 panes were templated with role "team manager / tech lead".
        // The substring "tech lead" wins so the addendum behaviour matches
        // the old orchestrator semantics for migrated panes.
        let got = compose_system_prompt(Some("team manager / tech lead"), None, None).unwrap();
        assert!(got.contains("# Tech Lead protocol"));
        assert!(!got.contains("# Manager protocol"));
    }

    #[test]
    fn manager_detection_excludes_tech_lead() {
        for r in ["manager", "Manager", "MANAGER", "team manager", "manager-agent"] {
            assert!(role_is_manager(Some(r)), "role {:?} should be manager", r);
        }
        // "tech lead" substring routes to tech-lead protocol, not manager.
        for r in ["tech lead", "team manager / tech lead", "Tech Lead"] {
            assert!(!role_is_manager(Some(r)), "role {:?} should NOT match manager", r);
            assert!(role_is_tech_lead(Some(r)), "role {:?} should match tech lead", r);
        }
        for r in ["reviewer", "backend", "", "mgr"] {
            assert!(!role_is_manager(Some(r)), "role {:?} should NOT be manager", r);
        }
        assert!(!role_is_manager(None));
    }

    #[test]
    fn non_manager_non_tech_lead_role_skips_orchestration_addenda() {
        let got = compose_system_prompt(Some("backend implementer"), None, None).unwrap();
        assert!(!got.contains("# Manager protocol"));
        assert!(!got.contains("# Tech Lead protocol"));
        assert!(!got.contains("# Reviewer protocol"));
        // But scratchpad note still rides along.
        assert!(got.contains("# Team scratchpad"));
    }

    #[test]
    fn reviewer_role_gets_protocol_addendum() {
        let got = compose_system_prompt(Some("reviewer"), None, None).unwrap();
        assert!(got.contains("# Reviewer protocol"));
        assert!(got.contains("approves:<task_id>"));
        assert!(got.contains("rejects:<task_id>"));
        // And not the manager one.
        assert!(!got.contains("# Manager protocol"));
    }

    #[test]
    fn reviewer_detection_is_case_insensitive_and_substring() {
        for r in ["reviewer", "Reviewer", "REVIEWER", "code reviewer", "reviewer-bot"] {
            assert!(role_is_reviewer(Some(r)), "role {:?} should be reviewer", r);
        }
        for r in ["manager", "backend", "", "review"] {
            assert!(!role_is_reviewer(Some(r)), "role {:?} should NOT be reviewer", r);
        }
        assert!(!role_is_reviewer(None));
    }

    #[test]
    fn role_can_be_both_manager_and_reviewer() {
        let got = compose_system_prompt(Some("manager-reviewer"), None, None).unwrap();
        assert!(got.contains("# Manager protocol"));
        assert!(got.contains("# Reviewer protocol"));
    }
}
