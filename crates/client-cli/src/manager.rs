//! Manager v2 — file-backed user↔manager channel.
//!
//! Two files live at the project root:
//!
//! - `project_goal.md` — the slowly-changing goal. Overwritten by
//!   `write_project_goal`; read by the deadloop manager at iteration start.
//! - `manager-directives.jsonl` — append-only stream of timestamped user
//!   directives. Read by the deadloop manager at iteration start (tail).
//!
//! Deliberately distinct from `.apas-team.jsonl` (the inter-pane scratchpad)
//! so user nudges don't get tangled with worker chatter.

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const GOAL_FILENAME: &str = "project_goal.md";
pub const DIRECTIVES_FILENAME: &str = "manager-directives.jsonl";

pub fn goal_path(project_dir: &Path) -> PathBuf {
    project_dir.join(GOAL_FILENAME)
}

pub fn directives_path(project_dir: &Path) -> PathBuf {
    project_dir.join(DIRECTIVES_FILENAME)
}

/// Overwrite `project_goal.md` with `goal`. Creates the file if absent.
pub fn write_project_goal(project_dir: &Path, goal: &str) -> Result<()> {
    let path = goal_path(project_dir);
    std::fs::write(&path, goal).with_context(|| {
        format!("writing project goal to {}", path.display())
    })?;
    Ok(())
}

/// Read the current project goal (or empty string if missing).
pub fn read_project_goal(project_dir: &Path) -> String {
    std::fs::read_to_string(goal_path(project_dir)).unwrap_or_default()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectiveRecord {
    pub ts: String,
    pub text: String,
}

/// Append a `{ts, text}` line to `manager-directives.jsonl`.
pub fn append_directive(project_dir: &Path, text: &str) -> Result<()> {
    let record = DirectiveRecord {
        ts: Utc::now().to_rfc3339(),
        text: text.to_string(),
    };
    let path = directives_path(project_dir);
    let mut json = serde_json::to_string(&record)?;
    json.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| {
            format!("opening directives file at {}", path.display())
        })?;
    file.write_all(json.as_bytes())
        .with_context(|| format!("writing directive to {}", path.display()))?;
    Ok(())
}

/// Read the last `n` directive records (oldest first within the returned
/// slice). Malformed lines are skipped. Returns empty when file is absent.
pub fn read_recent_directives(project_dir: &Path, n: usize) -> Vec<DirectiveRecord> {
    let path = directives_path(project_dir);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut records: Vec<DirectiveRecord> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<DirectiveRecord>(l).ok())
        .collect();
    if records.len() > n {
        records.drain(..records.len() - n);
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn fresh_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("apas-mgr-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn goal_round_trips() {
        let dir = fresh_dir();
        assert_eq!(read_project_goal(&dir), "");
        write_project_goal(&dir, "ship the thing").unwrap();
        assert_eq!(read_project_goal(&dir), "ship the thing");
        write_project_goal(&dir, "actually ship it tomorrow").unwrap();
        assert_eq!(read_project_goal(&dir), "actually ship it tomorrow");
    }

    #[test]
    fn directives_append_and_tail() {
        let dir = fresh_dir();
        assert!(read_recent_directives(&dir, 10).is_empty());
        append_directive(&dir, "first").unwrap();
        append_directive(&dir, "second").unwrap();
        append_directive(&dir, "third").unwrap();
        let recent = read_recent_directives(&dir, 2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].text, "second");
        assert_eq!(recent[1].text, "third");
    }

    #[test]
    fn directives_skip_malformed_lines_defensively() {
        let dir = fresh_dir();
        append_directive(&dir, "good").unwrap();
        // Stuff an unparseable line in the middle.
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(directives_path(&dir))
            .unwrap();
        f.write_all(b"not json\n").unwrap();
        drop(f);
        append_directive(&dir, "also good").unwrap();
        let recent = read_recent_directives(&dir, 10);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].text, "good");
        assert_eq!(recent[1].text, "also good");
    }
}
