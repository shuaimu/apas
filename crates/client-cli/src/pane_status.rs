//! Phase 4.1a: turn a claude `tool_use` block into a short human-friendly
//! status string for the pane header pill (e.g. "Editing src/foo.rs…",
//! "Running: cargo test", "Looking at .apas-team.jsonl").
//!
//! Pure conversion, no side effects. 4.1b will emit `PaneStatus` from
//! the reader thread when this returns Some.
//!
//! For unrecognized tools we still return Some(tool_name) so the pill
//! shows *something* — better signal than blank when the agent is in the
//! middle of a turn.

use serde_json::Value;

const MAX_CMD: usize = 60;
const MAX_PATH: usize = 40;

/// Build a status pill string from a tool_use block. Returns None for
/// tool names that don't merit a pill (e.g. TodoWrite — purely
/// internal bookkeeping).
pub fn pane_status_from_tool_use(tool_name: &str, input: &Value) -> Option<String> {
    let path_field = |key: &str| {
        input
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| truncate_middle(s, MAX_PATH))
    };
    match tool_name {
        "Read" => Some(format!(
            "Reading {}",
            path_field("file_path").unwrap_or_else(|| "file".to_string()),
        )),
        "Glob" => Some(format!(
            "Looking for {}",
            input
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("files"),
        )),
        "Grep" => Some(format!(
            "Searching for {}",
            truncate(
                input.get("pattern").and_then(|v| v.as_str()).unwrap_or("…"),
                40,
            ),
        )),
        "LS" => Some(format!(
            "Listing {}",
            path_field("path").unwrap_or_else(|| "directory".to_string()),
        )),
        "Edit" | "MultiEdit" => Some(format!(
            "Editing {}",
            path_field("file_path").unwrap_or_else(|| "file".to_string()),
        )),
        "Write" => Some(format!(
            "Writing {}",
            path_field("file_path").unwrap_or_else(|| "file".to_string()),
        )),
        "NotebookEdit" => Some(format!(
            "Editing {}",
            path_field("notebook_path").unwrap_or_else(|| "notebook".to_string()),
        )),
        "Bash" => Some(format!(
            "Running: {}",
            truncate(
                input.get("command").and_then(|v| v.as_str()).unwrap_or("…"),
                MAX_CMD,
            ),
        )),
        "Task" => Some(format!(
            "Delegating: {}",
            truncate(
                input
                    .get("description")
                    .and_then(|v| v.as_str())
                    .or_else(|| input.get("subagent_type").and_then(|v| v.as_str()))
                    .unwrap_or("subagent"),
                40,
            ),
        )),
        "AskUserQuestion" => Some("Asking the user…".to_string()),
        "WebFetch" => Some(format!(
            "Fetching {}",
            truncate(
                input.get("url").and_then(|v| v.as_str()).unwrap_or("URL"),
                MAX_CMD,
            ),
        )),
        "WebSearch" => Some(format!(
            "Searching web: {}",
            truncate(
                input.get("query").and_then(|v| v.as_str()).unwrap_or("…"),
                40,
            ),
        )),
        // Pure-internal bookkeeping — don't clutter the pill.
        "TodoWrite" => None,
        // Unknown tool — show the name itself, better than blank.
        other => Some(format!("Calling {}", other)),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// For paths, keep the head + tail so the user can see both the project
/// prefix and the filename. `crates/client-cli/src/main.rs` becomes
/// `crates/client-cli/…/main.rs` if it exceeds `max`.
fn truncate_middle(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head_len = (max / 2).saturating_sub(1);
    let tail_len = max.saturating_sub(head_len + 1);
    let head: String = s.chars().take(head_len).collect();
    let tail_start = s.chars().count().saturating_sub(tail_len);
    let tail: String = s.chars().skip(tail_start).collect();
    format!("{}…{}", head, tail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn edit_pulls_file_path() {
        let got = pane_status_from_tool_use(
            "Edit",
            &json!({"file_path": "src/foo.rs", "old_string": "x", "new_string": "y"}),
        );
        assert_eq!(got.as_deref(), Some("Editing src/foo.rs"));
    }

    #[test]
    fn bash_pulls_command_and_truncates_long_ones() {
        let got = pane_status_from_tool_use("Bash", &json!({"command": "echo hello"}));
        assert_eq!(got.as_deref(), Some("Running: echo hello"));

        let long = "echo ".to_string() + &"a".repeat(200);
        let got = pane_status_from_tool_use("Bash", &json!({"command": long})).unwrap();
        assert!(got.starts_with("Running: echo "));
        assert!(
            got.ends_with('…'),
            "long commands should be truncated: {}",
            got
        );
        assert!(got.len() < 90, "got: {} (len {})", got, got.len());
    }

    #[test]
    fn todowrite_returns_none() {
        let got = pane_status_from_tool_use("TodoWrite", &json!({"todos": []}));
        assert!(got.is_none(), "TodoWrite should not produce a pill");
    }

    #[test]
    fn unknown_tool_shows_calling_name() {
        let got = pane_status_from_tool_use("SomeNewTool", &json!({}));
        assert_eq!(got.as_deref(), Some("Calling SomeNewTool"));
    }

    #[test]
    fn ask_user_question_returns_constant() {
        let got = pane_status_from_tool_use("AskUserQuestion", &json!({}));
        assert_eq!(got.as_deref(), Some("Asking the user…"));
    }

    #[test]
    fn long_paths_get_middle_truncated() {
        let path = "crates/client-cli/src/very/deeply/nested/module/with/a/lot/of/segments/file.rs";
        let got = pane_status_from_tool_use("Read", &json!({"file_path": path})).unwrap();
        assert!(got.starts_with("Reading "));
        assert!(got.contains("…"), "expected middle ellipsis: {}", got);
        // Tail (file.rs) should be visible.
        assert!(got.ends_with("file.rs"), "tail should survive: {}", got);
    }

    #[test]
    fn read_falls_back_to_file_when_path_missing() {
        let got = pane_status_from_tool_use("Read", &json!({}));
        assert_eq!(got.as_deref(), Some("Reading file"));
    }
}
