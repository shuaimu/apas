//! Manager v2 — file-backed user↔manager channel.
//!
//! `project_goal.md` lives at the project root: the slowly-changing goal,
//! overwritten by `write_project_goal` and read by the deadloop manager at
//! iteration start.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub const GOAL_FILENAME: &str = "project_goal.md";

pub fn goal_path(project_dir: &Path) -> PathBuf {
    project_dir.join(GOAL_FILENAME)
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
}
