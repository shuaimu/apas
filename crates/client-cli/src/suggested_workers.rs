//! Suggested-workers queue: parse + serialize `suggested-workers.md`.
//!
//! Format:
//!
//! ```markdown
//! # Suggested Workers
//!
//! ## SUG-001 — Frontend Developer
//! - role: developer
//! - goal: Build the new dashboard UI based on Figma designs
//! - backstory: React expert; familiar with Tailwind + Zustand
//! - needs_worktree: yes
//!
//! ## SUG-002 — QA Tester
//! - role: qa
//! - goal: Write integration tests for the auth flow
//! - backstory: ...
//! - needs_worktree: no
//! ```
//!
//! Designed for the Manager pane to edit directly via Write/Edit. The
//! parser is forgiving — missing bullets default to empty, unknown bullets
//! are ignored. The Overview renders one card per `##` heading with
//! Accept (spawn the pane) and Dismiss (drop the section) buttons.

use anyhow::{Context, Result};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub const SUGGESTED_WORKERS_FILENAME: &str = "suggested-workers.md";

pub fn path(project_dir: &Path) -> PathBuf {
    project_dir.join(SUGGESTED_WORKERS_FILENAME)
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SuggestedWorkers {
    pub entries: Vec<SuggestedWorker>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestedWorker {
    pub id: String,
    pub label: String,
    pub role: String,
    pub goal: String,
    pub backstory: String,
    pub needs_worktree: bool,
}

pub fn load(project_dir: &Path) -> Result<SuggestedWorkers> {
    let p = path(project_dir);
    if !p.exists() {
        return Ok(SuggestedWorkers::default());
    }
    let s = std::fs::read_to_string(&p)
        .with_context(|| format!("reading {}", p.display()))?;
    Ok(parse(&s))
}

pub fn save(project_dir: &Path, sw: &SuggestedWorkers) -> Result<()> {
    let p = path(project_dir);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let body = serialize(sw);
    let tmp = p.with_extension("md.tmp");
    std::fs::write(&tmp, body)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &p)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), p.display()))?;
    Ok(())
}

pub fn dismiss(project_dir: &Path, suggestion_id: &str) -> Result<SuggestedWorkers> {
    let mut sw = load(project_dir)?;
    if sw.remove(suggestion_id) {
        save(project_dir, &sw)?;
    }
    load(project_dir)
}

pub fn parse(src: &str) -> SuggestedWorkers {
    let mut entries: Vec<SuggestedWorker> = Vec::new();
    let mut current: Option<SuggestedWorker> = None;

    for raw_line in src.lines() {
        let line = raw_line.trim_end();
        if let Some(rest) = line.strip_prefix("## ") {
            if let Some(prev) = current.take() {
                entries.push(prev);
            }
            // Heading is "SUG-NNN — label" or "SUG-NNN - label" or just "SUG-NNN".
            let (id, label) = split_heading(rest);
            if id.is_empty() {
                continue;
            }
            current = Some(SuggestedWorker {
                id,
                label,
                role: String::new(),
                goal: String::new(),
                backstory: String::new(),
                needs_worktree: false,
            });
            continue;
        }
        if let Some(entry) = current.as_mut() {
            if let Some(bullet) = line.trim_start().strip_prefix("- ") {
                if let Some((k, v)) = bullet.split_once(':') {
                    let key = k.trim().to_ascii_lowercase();
                    let val = v.trim();
                    match key.as_str() {
                        "role" => entry.role = val.to_string(),
                        "goal" => entry.goal = val.to_string(),
                        "backstory" => entry.backstory = val.to_string(),
                        "needs_worktree" | "needs-worktree" | "worktree" => {
                            let v = val.to_ascii_lowercase();
                            entry.needs_worktree =
                                matches!(v.as_str(), "yes" | "y" | "true" | "1");
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    if let Some(last) = current.take() {
        entries.push(last);
    }
    SuggestedWorkers { entries }
}

fn split_heading(s: &str) -> (String, String) {
    let s = s.trim();
    // Try em-dash first, then hyphen, then whitespace fallback.
    if let Some((id, rest)) = s.split_once('—') {
        return (id.trim().to_string(), rest.trim().to_string());
    }
    if let Some((id, rest)) = s.split_once(" - ") {
        return (id.trim().to_string(), rest.trim().to_string());
    }
    if let Some((id, rest)) = s.split_once(char::is_whitespace) {
        return (id.trim().to_string(), rest.trim().to_string());
    }
    (s.to_string(), String::new())
}

pub fn serialize(sw: &SuggestedWorkers) -> String {
    let mut out = String::new();
    out.push_str("# Suggested Workers\n\n");
    out.push_str(
        "<!-- Manager: append sections below to suggest workers for the team. \
The Overview shows each section as a card with Accept / Dismiss buttons. \
Format: '## SUG-NNN — label' then bullets for role / goal / backstory / needs_worktree. -->\n\n",
    );
    if sw.entries.is_empty() {
        out.push_str("_(no suggestions yet)_\n");
        return out;
    }
    for e in &sw.entries {
        let _ = writeln!(out, "## {} — {}", e.id, e.label);
        let _ = writeln!(out, "- role: {}", e.role);
        let _ = writeln!(out, "- goal: {}", e.goal);
        let _ = writeln!(out, "- backstory: {}", e.backstory);
        let _ = writeln!(
            out,
            "- needs_worktree: {}",
            if e.needs_worktree { "yes" } else { "no" }
        );
        out.push('\n');
    }
    out
}

impl SuggestedWorkers {
    /// Next free SUG-NNN id past the existing max.
    pub fn next_id(&self) -> String {
        let max = self
            .entries
            .iter()
            .filter_map(|e| {
                e.id.strip_prefix("SUG-")
                    .and_then(|n| n.parse::<u32>().ok())
            })
            .max()
            .unwrap_or(0);
        format!("SUG-{:03}", max + 1)
    }

    /// Drop the entry with `id`. Returns true if removed.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        self.entries.len() != before
    }

    pub fn get(&self, id: &str) -> Option<&SuggestedWorker> {
        self.entries.iter().find(|e| e.id == id)
    }
}

pub fn to_wire(sw: &SuggestedWorkers) -> Vec<shared::SuggestedWorkerMsg> {
    sw.entries
        .iter()
        .map(|e| shared::SuggestedWorkerMsg {
            id: e.id.clone(),
            label: e.label.clone(),
            role: e.role.clone(),
            goal: e.goal.clone(),
            backstory: e.backstory.clone(),
            needs_worktree: e.needs_worktree,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_basic() {
        let s = "# Suggested Workers\n\n\
## SUG-001 — Frontend Developer\n\
- role: developer\n\
- goal: Build the UI\n\
- backstory: React expert\n\
- needs_worktree: yes\n\
\n\
## SUG-002 — QA Tester\n\
- role: qa\n\
- goal: Write tests\n\
- backstory: pytest experience\n\
- needs_worktree: no\n";
        let sw = parse(s);
        assert_eq!(sw.entries.len(), 2);
        assert_eq!(sw.entries[0].id, "SUG-001");
        assert_eq!(sw.entries[0].label, "Frontend Developer");
        assert_eq!(sw.entries[0].role, "developer");
        assert!(sw.entries[0].needs_worktree);
        assert_eq!(sw.entries[1].id, "SUG-002");
        assert!(!sw.entries[1].needs_worktree);
    }

    #[test]
    fn parse_empty_file() {
        let sw = parse("");
        assert!(sw.entries.is_empty());
    }

    #[test]
    fn parse_just_header() {
        let sw = parse("# Suggested Workers\n\n_(no suggestions yet)_\n");
        assert!(sw.entries.is_empty());
    }

    #[test]
    fn parse_hyphen_separator() {
        let sw = parse("## SUG-005 - My Worker\n- role: dev\n");
        assert_eq!(sw.entries.len(), 1);
        assert_eq!(sw.entries[0].id, "SUG-005");
        assert_eq!(sw.entries[0].label, "My Worker");
    }

    #[test]
    fn next_id_starts_at_001() {
        let sw = SuggestedWorkers::default();
        assert_eq!(sw.next_id(), "SUG-001");
    }

    #[test]
    fn next_id_past_max() {
        let sw = SuggestedWorkers {
            entries: vec![
                SuggestedWorker {
                    id: "SUG-001".into(),
                    label: String::new(),
                    role: String::new(),
                    goal: String::new(),
                    backstory: String::new(),
                    needs_worktree: false,
                },
                SuggestedWorker {
                    id: "SUG-007".into(),
                    label: String::new(),
                    role: String::new(),
                    goal: String::new(),
                    backstory: String::new(),
                    needs_worktree: false,
                },
            ],
        };
        assert_eq!(sw.next_id(), "SUG-008");
    }

    #[test]
    fn remove_drops_entry() {
        let mut sw = SuggestedWorkers {
            entries: vec![SuggestedWorker {
                id: "SUG-001".into(),
                label: String::new(),
                role: String::new(),
                goal: String::new(),
                backstory: String::new(),
                needs_worktree: false,
            }],
        };
        assert!(sw.remove("SUG-001"));
        assert!(sw.entries.is_empty());
        assert!(!sw.remove("SUG-001"));
    }

    #[test]
    fn dismiss_removes_only_requested_section_from_file() {
        let tmp = TempDir::new().expect("tmpdir");
        let original = SuggestedWorkers {
            entries: vec![
                SuggestedWorker {
                    id: "SUG-001".into(),
                    label: "Frontend".into(),
                    role: "developer".into(),
                    goal: "Build UI".into(),
                    backstory: "React".into(),
                    needs_worktree: true,
                },
                SuggestedWorker {
                    id: "SUG-002".into(),
                    label: "QA".into(),
                    role: "qa".into(),
                    goal: "Test flows".into(),
                    backstory: "Playwright".into(),
                    needs_worktree: false,
                },
            ],
        };
        save(tmp.path(), &original).expect("save original suggestions");

        let remaining = dismiss(tmp.path(), "SUG-001").expect("dismiss suggestion");

        assert_eq!(remaining.entries.len(), 1);
        assert_eq!(remaining.entries[0].id, "SUG-002");
        let file = std::fs::read_to_string(path(tmp.path())).expect("read suggested-workers.md");
        assert!(!file.contains("SUG-001"));
        assert!(file.contains("## SUG-002"));
        assert!(file.contains("- needs_worktree: no"));
    }

    #[test]
    fn round_trip_stability() {
        let original = SuggestedWorkers {
            entries: vec![SuggestedWorker {
                id: "SUG-001".into(),
                label: "Test".into(),
                role: "dev".into(),
                goal: "do things".into(),
                backstory: "background".into(),
                needs_worktree: true,
            }],
        };
        let serialized = serialize(&original);
        let reparsed = parse(&serialized);
        assert_eq!(reparsed, original);
    }
}
