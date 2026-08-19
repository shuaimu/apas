//! Claude tells us which transcript it is writing, instead of us guessing.
//!
//! A pane's transcript used to be *derived*: the slug of the pane's cwd plus
//! the session id APAS minted. Both halves are wrong the moment the user acts.
//! Claude Code can move a session into one of its own worktrees, so the slug
//! points at a directory the file was never in; and `/resume` onto another
//! session makes Claude append to *that* session's file, so the minted id names
//! a conversation the pane is no longer in. What was left was a heuristic —
//! follow the newest transcript in the directory — which cannot tell our pane
//! switching files from an unrelated `claude` writing in the same directory,
//! and duly adopted a stranger's conversation on a live pane.
//!
//! Claude Code fires a `SessionStart` hook on startup, resume, clear and
//! compact, and hands it the absolute `transcript_path` it is about to write.
//! That is the answer, from the only party that knows it.
//!
//! This is deliberately unlike the `record_turn` MCP tool that was tried and
//! rejected: that asked the *model* to cooperate with advisory instructions,
//! and neither provider did. A hook is run by the client, not chosen by the
//! agent.
//!
//! Pane identity travels in `APAS_PANE_RUNTIME`, which the pane's provider is
//! spawned with and the hook process inherits — verified, along with the fact
//! that `--settings` merges with the user's other settings layers rather than
//! replacing them, so a pane keeps the model and theme its owner configured.
//! A `claude` a person runs by hand has no such variable, so its hook writes
//! nothing and it can never be mistaken for a pane.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Set on a pane's provider process; read by the hook out of its environment.
pub const PANE_RUNTIME_ENV: &str = "APAS_PANE_RUNTIME";

const REPORT_FILE: &str = "claude-session.json";
const SETTINGS_FILE: &str = "claude-settings.json";

/// What the hook records, which is the hook's own payload plus when we saw it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeSessionReport {
    pub session_id: String,
    pub transcript_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// `startup`, `resume`, `clear` or `compact` — kept for diagnosis, since a
    /// pane that keeps reporting `clear` is a different story from one that
    /// never reports at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub recorded_at_unix_ms: u128,
}

/// Per-pane runtime directory, alongside the pane-host tree and with the same
/// rules: volatile, outside the project, owner-only.
pub fn pane_runtime_dir(project_id: Uuid, pane_id: u32) -> Result<PathBuf> {
    let dir = crate::config::Config::runtime_dir()?
        .join("panes")
        .join(crate::pane_host::short_uuid(project_id))
        .join(pane_id.to_string());
    crate::pane_host::ensure_private_dir(&dir)?;
    Ok(dir)
}

pub fn report_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(REPORT_FILE)
}

pub fn settings_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(SETTINGS_FILE)
}

/// The `--settings` document for a pane: hooks only.
///
/// Only hooks, because settings layers merge — writing anything else here would
/// override what the user configured for every other Claude they run.
pub fn settings_document(apas_executable: &Path) -> serde_json::Value {
    serde_json::json!({
        "hooks": {
            "SessionStart": [
                {
                    "hooks": [
                        {
                            "type": "command",
                            "command": format!("{} session-hook", apas_executable.display()),
                        }
                    ]
                }
            ]
        }
    })
}

/// Write the pane's settings file, returning the path to pass as `--settings`.
pub fn write_settings(runtime_dir: &Path, apas_executable: &Path) -> Result<PathBuf> {
    let path = settings_path(runtime_dir);
    let body = serde_json::to_vec_pretty(&settings_document(apas_executable))?;
    write_private_atomic(&path, &body)?;
    Ok(path)
}

/// Install the hook for a Claude pane: create its runtime directory, write the
/// settings document, and add the environment variable that tells the hook
/// which pane it is running for.
///
/// Returns the `--settings` path, or `None` for providers without this
/// mechanism and whenever anything fails — the pane must still start, and the
/// watcher's derivation remains the fallback.
pub fn prepare(
    provider: &shared::Provider,
    project_id: Uuid,
    pane_id: u32,
    env: &mut Vec<(String, String)>,
) -> Option<PathBuf> {
    if !matches!(provider, shared::Provider::Claude) {
        return None;
    }
    let runtime_dir = pane_runtime_dir(project_id, pane_id)
        .map_err(|err| tracing::warn!(%err, pane_id, "no runtime dir for the claude session hook"))
        .ok()?;
    let executable = crate::update::resolve_preferred_apas_executable();
    let settings = write_settings(&runtime_dir, &executable)
        .map_err(|err| tracing::warn!(%err, pane_id, "could not write the claude session hook settings"))
        .ok()?;
    env.push((
        PANE_RUNTIME_ENV.to_string(),
        runtime_dir.to_string_lossy().to_string(),
    ));
    Some(settings)
}

/// The transcript this pane's Claude last said it was writing.
pub fn reported_transcript(project_id: Uuid, pane_id: u32) -> Option<PathBuf> {
    let dir = pane_runtime_dir(project_id, pane_id).ok()?;
    let report = read_report(&dir)?;
    report.transcript_path.is_file().then_some(report.transcript_path)
}

/// Read what the pane's Claude last reported, if anything.
pub fn read_report(runtime_dir: &Path) -> Option<ClaudeSessionReport> {
    let raw = std::fs::read(report_path(runtime_dir)).ok()?;
    serde_json::from_slice(&raw).ok()
}

/// Parse a `SessionStart` payload into what we keep.
///
/// A payload without a transcript path is not an error worth failing a hook
/// over — the provider ran, it simply told us nothing usable — so it becomes
/// `None` and the caller writes nothing.
pub fn report_from_payload(payload: &serde_json::Value, now_unix_ms: u128) -> Option<ClaudeSessionReport> {
    let transcript_path = payload.get("transcript_path")?.as_str()?.trim();
    if transcript_path.is_empty() {
        return None;
    }
    let session_id = payload
        .get("session_id")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    Some(ClaudeSessionReport {
        session_id,
        transcript_path: PathBuf::from(transcript_path),
        cwd: payload
            .get("cwd")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        source: payload
            .get("source")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        recorded_at_unix_ms: now_unix_ms,
    })
}

/// The `apas session-hook` entry point: read the payload on stdin, resolve the
/// pane from the inherited environment, record it.
///
/// Exits successfully whatever happens. A hook that fails is noise in the
/// user's terminal at the exact moment their agent is starting, and nothing
/// here is important enough to interrupt that — the watcher's existing
/// derivation remains as the fallback.
pub fn run() -> Result<()> {
    use std::io::Read;

    let Some(runtime_dir) = std::env::var_os(PANE_RUNTIME_ENV).map(PathBuf::from) else {
        // Not one of our panes: a `claude` someone ran by hand, inheriting the
        // hook from a settings file but not the pane identity.
        return Ok(());
    };
    let mut raw = String::new();
    if std::io::stdin().read_to_string(&mut raw).is_err() {
        return Ok(());
    }
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Ok(());
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default();
    let Some(report) = report_from_payload(&payload, now) else {
        return Ok(());
    };
    if !runtime_dir.is_dir() {
        return Ok(());
    }
    let body = serde_json::to_vec(&report)?;
    let _ = write_private_atomic(&report_path(&runtime_dir), &body);
    Ok(())
}

/// Temp-and-rename, so the watcher polling this file can never read a half
/// written one.
fn write_private_atomic(path: &Path, body: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, body).with_context(|| format!("write {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).with_context(|| format!("rename into {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(transcript: &str) -> serde_json::Value {
        serde_json::json!({
            "session_id": "40579788-9c21-4b59-ae66-9d0f0b35c378",
            "transcript_path": transcript,
            "cwd": "/home/u/mako",
            "source": "resume",
            "hook_event_name": "SessionStart",
        })
    }

    #[test]
    fn a_payload_becomes_a_report() {
        let report = report_from_payload(&payload("/home/u/.claude/projects/-x/s.jsonl"), 7).unwrap();
        assert_eq!(
            report.transcript_path,
            PathBuf::from("/home/u/.claude/projects/-x/s.jsonl")
        );
        assert_eq!(report.session_id, "40579788-9c21-4b59-ae66-9d0f0b35c378");
        assert_eq!(report.source.as_deref(), Some("resume"));
        assert_eq!(report.recorded_at_unix_ms, 7);
    }

    #[test]
    fn a_payload_without_a_usable_path_records_nothing() {
        // Better to fall back to derivation than to record an empty path and
        // have the watcher follow it.
        for value in [serde_json::json!({}), payload(""), payload("   ")] {
            assert!(report_from_payload(&value, 0).is_none(), "{value}");
        }
    }

    #[test]
    fn the_settings_document_carries_only_hooks() {
        // Settings layers merge, so anything else here would silently override
        // what the pane's owner configured for every other Claude they run.
        let doc = settings_document(Path::new("/usr/local/bin/apas"));
        let keys: Vec<&String> = doc.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["hooks"]);
        let command = doc["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert_eq!(command, "/usr/local/bin/apas session-hook");
    }

    #[test]
    fn a_report_round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_report(dir.path()).is_none(), "nothing reported yet");

        let report = report_from_payload(&payload("/t/s.jsonl"), 11).unwrap();
        write_private_atomic(&report_path(dir.path()), &serde_json::to_vec(&report).unwrap())
            .unwrap();

        let read = read_report(dir.path()).unwrap();
        assert_eq!(read.transcript_path, PathBuf::from("/t/s.jsonl"));
        assert_eq!(read.source.as_deref(), Some("resume"));
    }

    #[test]
    fn a_corrupt_report_reads_as_absent() {
        // Falling back to derivation beats propagating a garbled path.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(report_path(dir.path()), b"{not json").unwrap();
        assert!(read_report(dir.path()).is_none());
    }
}
