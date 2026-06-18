//! Phase 2.2a: project-scoped team scratchpad.
//!
//! The scratchpad is an append-only JSONL file at
//! `<project>/.apas-team.jsonl`. Each line is one [`TeamRecord`]. Panes
//! publish artifacts they want other panes (or the human) to see —
//! diffs, reviews, decisions, status pings — and the file is the
//! single source of truth that survives restarts.
//!
//! Scope for this leaf:
//!   * the record shape (kept simple and forward-compatible via
//!     `#[serde(default)]` on the optional fields)
//!   * append + read helpers operating on the project directory
//!   * a tag-filter convenience used by the future `tail -f` / web
//!     timeline path
//!
//! Wire/UI plumbing arrives in Phase 2.2b; the system-prompt mention
//! that tells the agent the file exists ships in 2.2c.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Path of the team scratchpad relative to the project root.
pub const SCRATCHPAD_REL_PATH: &str = ".apas-team.jsonl";

/// One published artifact in the team scratchpad. Forward-compat fields
/// use `#[serde(default)]` so older readers don't choke on extras.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamRecord {
    /// RFC 3339 timestamp.
    pub ts: String,
    /// Pane that published the record. None for human-written entries
    /// (e.g. a manager pasting a decision in the web timeline directly).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<u32>,
    /// Free-form labels for filtering (`tail -f` matchers, web filters).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Short category label ("delegation", "diff", "review",
    /// "decision", "status", etc.). Task correlation belongs in tags
    /// like `task:TODO-123`. Convention only — not validated.
    pub kind: String,
    /// Free-form payload. Kept as a string so we don't lock callers
    /// into a particular sub-shape; large bodies are fine.
    pub body: String,
}

impl TeamRecord {
    /// Convenience constructor that fills `ts` with the current UTC time.
    pub fn now(pane_id: Option<u32>, kind: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            ts: chrono::Utc::now().to_rfc3339(),
            pane_id,
            tags: Vec::new(),
            kind: kind.into(),
            body: body.into(),
        }
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Convert to the wire-stable shape used in CliToServer / ServerToWeb.
    pub fn to_wire(&self) -> shared::TeamScratchpadRecord {
        shared::TeamScratchpadRecord {
            ts: self.ts.clone(),
            pane_id: self.pane_id,
            tags: self.tags.clone(),
            kind: self.kind.clone(),
            body: self.body.clone(),
        }
    }
}

/// Resolve the absolute path of the scratchpad for a given project dir.
pub fn scratchpad_path(project_dir: &Path) -> PathBuf {
    project_dir.join(SCRATCHPAD_REL_PATH)
}

/// Phase 3.1a: scan a record's tags for `delegate-to:<pane_id>` and
/// return the parsed pane id. Returns None when no such tag exists or
/// when the suffix doesn't parse as `u32`. Used by the watcher to
/// route delegated task bodies into the target pane's input channel.
pub fn delegate_target_pane(record: &TeamRecord) -> Option<u32> {
    for tag in &record.tags {
        if let Some(rest) = tag.strip_prefix("delegate-to:") {
            if let Ok(id) = rest.parse::<u32>() {
                return Some(id);
            }
        }
    }
    None
}

/// Append a single record to the project's scratchpad. The project's
/// `.apas` is metadata stored as a file, so the scratchpad lives beside
/// it as `.apas-team.jsonl`.
pub fn append(project_dir: &Path, record: &TeamRecord) -> Result<()> {
    let path = scratchpad_path(project_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
        .with_context(|| format!("opening {} for append", path.display()))?;
    let line = serde_json::to_string(record).context("serializing TeamRecord")?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

/// Read all records from the scratchpad. Skips malformed lines with a
/// trace warning so a single bad write doesn't poison the entire log.
pub fn read_all(project_dir: &Path) -> Result<Vec<TeamRecord>> {
    let path = scratchpad_path(project_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = std::fs::File::open(&path)
        .with_context(|| format!("opening {} for read", path.display()))?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for (lineno, line_res) in reader.lines().enumerate() {
        let line = match line_res {
            Ok(l) => l,
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    lineno,
                    error = %err,
                    "scratchpad: read error, skipping line",
                );
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<TeamRecord>(&line) {
            Ok(r) => out.push(r),
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    lineno,
                    error = %err,
                    "scratchpad: malformed line, skipping",
                );
            }
        }
    }
    Ok(out)
}

/// Read records and keep only those whose `tags` overlap with `wanted`.
/// Empty `wanted` = no filtering (returns all). Match is exact-string.
pub fn read_filtered_by_tags(project_dir: &Path, wanted: &[&str]) -> Result<Vec<TeamRecord>> {
    let all = read_all(project_dir)?;
    if wanted.is_empty() {
        return Ok(all);
    }
    Ok(all
        .into_iter()
        .filter(|r| r.tags.iter().any(|t| wanted.iter().any(|w| t == w)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn append_then_read_roundtrips_records() {
        let tmp = TempDir::new().unwrap();
        let r1 = TeamRecord::now(Some(2), "diff", "first body").with_tag("pr-review");
        let r2 = TeamRecord::now(Some(3), "review", "second body").with_tag("approves:42");
        append(tmp.path(), &r1).unwrap();
        append(tmp.path(), &r2).unwrap();
        assert!(scratchpad_path(tmp.path()).exists());
        let got = read_all(tmp.path()).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].kind, "diff");
        assert_eq!(got[0].body, "first body");
        assert_eq!(got[0].tags, vec!["pr-review"]);
        assert_eq!(got[1].pane_id, Some(3));
        assert_eq!(got[1].tags, vec!["approves:42"]);
    }

    #[test]
    fn scratchpad_path_points_to_sibling_file() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(
            scratchpad_path(tmp.path()),
            tmp.path().join(".apas-team.jsonl")
        );
        assert_eq!(
            scratchpad_path(tmp.path()),
            tmp.path().join(SCRATCHPAD_REL_PATH)
        );
    }

    #[test]
    fn append_and_read_use_public_sibling_path() {
        let tmp = TempDir::new().unwrap();
        let public_path = scratchpad_path(tmp.path());
        let legacy_path = tmp.path().join(".apas").join("team.jsonl");

        append(
            tmp.path(),
            &TeamRecord::now(Some(8), "status", "using sibling scratchpad"),
        )
        .unwrap();

        assert!(public_path.exists());
        assert!(!legacy_path.exists());
        let got = read_all(tmp.path()).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].body, "using sibling scratchpad");
    }

    #[test]
    fn read_filtered_by_tags_keeps_only_matches() {
        let tmp = TempDir::new().unwrap();
        append(
            tmp.path(),
            &TeamRecord::now(Some(2), "diff", "a").with_tag("auth"),
        )
        .unwrap();
        append(
            tmp.path(),
            &TeamRecord::now(Some(3), "diff", "b").with_tag("api"),
        )
        .unwrap();
        let auth = read_filtered_by_tags(tmp.path(), &["auth"]).unwrap();
        assert_eq!(auth.len(), 1);
        assert_eq!(auth[0].body, "a");

        let either = read_filtered_by_tags(tmp.path(), &["auth", "api"]).unwrap();
        assert_eq!(either.len(), 2);
    }

    #[test]
    fn missing_file_returns_empty_vec() {
        let tmp = TempDir::new().unwrap();
        let got = read_all(tmp.path()).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn delegate_target_pane_picks_first_valid_tag() {
        let r = TeamRecord::now(Some(1), "delegation", "go fix the auth tests")
            .with_tag("priority:high")
            .with_tag("task:TODO-123")
            .with_tag("delegate-to:7")
            .with_tag("review-worker:42");
        assert_eq!(delegate_target_pane(&r), Some(7));
    }

    #[test]
    fn delegate_target_pane_none_when_no_match() {
        let r = TeamRecord::now(Some(1), "decision", "switch to sqlx");
        assert_eq!(delegate_target_pane(&r), None);

        let r = TeamRecord::now(Some(1), "task", "garbage").with_tag("delegate-to:not-a-number");
        assert_eq!(delegate_target_pane(&r), None);
    }

    #[test]
    fn malformed_lines_are_skipped_not_fatal() {
        let tmp = TempDir::new().unwrap();
        let path = scratchpad_path(tmp.path());
        std::fs::write(
            &path,
            r#"{"ts":"2026-01-01T00:00:00Z","kind":"diff","body":"ok"}
{ this is not json }
{"ts":"2026-01-02T00:00:00Z","kind":"review","body":"also ok"}
"#,
        )
        .unwrap();
        let got = read_all(tmp.path()).unwrap();
        assert_eq!(got.len(), 2, "malformed middle line skipped");
        assert_eq!(got[0].kind, "diff");
        assert_eq!(got[1].kind, "review");
    }
}
