//! Self-reported conversation history for `PaneKind::Terminal` panes.
//!
//! An agent pane is observed: the CLI parses its stream-json and knows every
//! turn without cooperation. A terminal pane hosts the provider's real TUI on a
//! pty, so there is nothing structured to parse — which is why terminal panes
//! have had no history, no usage, and no status.
//!
//! This closes that gap by having the agent report its own turns through the
//! per-pane MCP server (`record_turn`). Two consequences follow from the fact
//! that it is **self-reported** rather than observed, and both are deliberate
//! trade-offs rather than oversights:
//!
//! 1. **History is only as complete as the agent's cooperation.** An agent that
//!    ignores the instruction records nothing. The MCP server states the
//!    requirement in its `initialize` instructions, which claude and codex both
//!    surface to the model, but nothing enforces it. Observing the provider's
//!    own transcript (`~/.claude/projects/**.jsonl`) would be complete; it
//!    would also only ever work for providers whose format we track.
//! 2. **The agent could report anything.** `pane_id` is stamped server-side by
//!    the MCP server, so a pane cannot forge history *for another pane*, but
//!    within its own pane the content is whatever it says. Treat this as a
//!    record of what the agent claims, not proof of what happened.
//!
//! Storage is one append-only JSONL file per pane under
//! `.apas-conversations/`. Per-pane rather than one shared file because turns
//! are high-volume and single-writer: exactly one MCP server exists per pane,
//! so appends need no cross-process lock, unlike `team-todo.md`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

const CONVERSATION_DIR: &str = ".apas-conversations";

/// One reported turn.
///
/// Field names match what the CLI needs to rebuild a `ClaudeStreamMessage`, so
/// a terminal pane's history flows through the same server storage, web
/// rendering, and usage accounting as an agent pane's — see
/// `dual_pane::conversation_turn_to_stream_message`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnRecord {
    /// RFC3339. Written by the reporting agent's server, not the reader.
    pub ts: String,
    /// Stamped by the MCP server from its `--pane-id`, never taken from the
    /// agent — a pane must not be able to write history for another pane.
    pub pane_id: u32,
    /// `user` or `assistant`. Anything else is preserved verbatim rather than
    /// rejected: a provider we have not seen yet should degrade to "recorded
    /// but rendered plainly", not to a dropped turn.
    pub role: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
}

impl TurnRecord {
    pub fn is_assistant(&self) -> bool {
        self.role.eq_ignore_ascii_case("assistant")
    }

    /// Whether this turn carries usage worth billing to the pane.
    pub fn has_usage(&self) -> bool {
        self.input_tokens.unwrap_or(0) > 0 || self.output_tokens.unwrap_or(0) > 0
    }
}

pub fn conversation_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(CONVERSATION_DIR)
}

pub fn path_for(project_dir: &Path, pane_id: u32) -> PathBuf {
    conversation_dir(project_dir).join(format!("pane-{pane_id}.jsonl"))
}

/// Append one turn. Creates the directory on first use.
///
/// A single `write_all` of one newline-terminated line to an `O_APPEND` handle
/// is atomic for the sizes involved, which is what lets concurrent readers tail
/// the file without ever seeing a half-written record.
pub fn append(project_dir: &Path, record: &TurnRecord) -> Result<()> {
    let dir = conversation_dir(project_dir);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create {}", dir.display()))?;
    let path = path_for(project_dir, record.pane_id);
    let mut line = serde_json::to_string(record).context("serialize turn record")?;
    line.push('\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    file.write_all(line.as_bytes())
        .with_context(|| format!("failed to append to {}", path.display()))?;
    Ok(())
}

/// Every turn recorded for a pane, oldest first.
///
/// Malformed lines are skipped rather than failing the read: a truncated final
/// line is normal while another process is mid-append, and one bad line must
/// not cost us the entire history.
pub fn read_all(project_dir: &Path, pane_id: u32) -> Result<Vec<TurnRecord>> {
    let path = path_for(project_dir, pane_id);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };
    Ok(raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<TurnRecord>(l).ok())
        .collect())
}

/// Pane ids that have a conversation file, so a tailer can discover panes
/// without being told about them.
pub fn panes_with_history(project_dir: &Path) -> Vec<u32> {
    let dir = conversation_dir(project_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut ids: Vec<u32> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|n| n.strip_prefix("pane-"))
                .and_then(|n| n.strip_suffix(".jsonl"))
                .and_then(|n| n.parse().ok())
        })
        .collect();
    ids.sort_unstable();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(pane_id: u32, role: &str, text: &str) -> TurnRecord {
        TurnRecord {
            ts: "2026-08-04T00:00:00Z".to_string(),
            pane_id,
            role: role.to_string(),
            text: text.to_string(),
            model: None,
            input_tokens: None,
            output_tokens: None,
        }
    }

    #[test]
    fn turns_round_trip_in_order() {
        let dir = tempfile::tempdir().unwrap();
        append(dir.path(), &turn(7, "user", "hello")).unwrap();
        append(dir.path(), &turn(7, "assistant", "hi")).unwrap();

        let all = read_all(dir.path(), 7).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].text, "hello");
        assert_eq!(all[1].text, "hi");
        assert!(all[1].is_assistant());
    }

    #[test]
    fn panes_get_their_own_files() {
        // One file per pane is what removes the need for a write lock, so the
        // separation matters beyond tidiness.
        let dir = tempfile::tempdir().unwrap();
        append(dir.path(), &turn(1, "user", "for pane one")).unwrap();
        append(dir.path(), &turn(2, "user", "for pane two")).unwrap();

        assert_eq!(read_all(dir.path(), 1).unwrap().len(), 1);
        assert_eq!(read_all(dir.path(), 1).unwrap()[0].text, "for pane one");
        assert_eq!(read_all(dir.path(), 2).unwrap()[0].text, "for pane two");
        assert_eq!(panes_with_history(dir.path()), vec![1, 2]);
    }

    #[test]
    fn a_pane_with_no_history_reads_empty_rather_than_failing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_all(dir.path(), 99).unwrap().is_empty());
        assert!(panes_with_history(dir.path()).is_empty());
    }

    #[test]
    fn a_truncated_final_line_does_not_cost_us_the_history() {
        // Normal while another process is mid-append; dropping everything on
        // one bad line would lose a whole conversation.
        let dir = tempfile::tempdir().unwrap();
        append(dir.path(), &turn(3, "user", "kept")).unwrap();
        let path = path_for(dir.path(), 3);
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{\"ts\":\"2026\",\"pane_id\":3,\"rol").unwrap();

        let all = read_all(dir.path(), 3).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].text, "kept");
    }

    #[test]
    fn an_unknown_role_is_preserved_rather_than_dropped() {
        // A provider we have not integrated should degrade to "recorded but
        // rendered plainly", never to a silently missing turn.
        let dir = tempfile::tempdir().unwrap();
        append(dir.path(), &turn(4, "tool", "some output")).unwrap();
        let all = read_all(dir.path(), 4).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].role, "tool");
        assert!(!all[0].is_assistant());
    }

    #[test]
    fn usage_is_only_reported_when_present() {
        let mut t = turn(5, "assistant", "done");
        assert!(!t.has_usage());
        t.output_tokens = Some(120);
        assert!(t.has_usage());
    }
}
