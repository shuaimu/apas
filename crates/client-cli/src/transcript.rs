//! Reading a terminal pane's history out of the provider's own transcript.
//!
//! A terminal pane hosts the provider's real TUI on a pty, so there is no
//! stream-json for the CLI to parse — which is why terminal panes had no
//! history and no usage counters.
//!
//! The first attempt asked the agent to self-report each turn through an MCP
//! tool, with the requirement stated in the MCP server's `initialize`
//! instructions. Tested against both providers, that does not work: claude and
//! codex each connect to the server and will call the tool when told to
//! directly, but neither acts on the `initialize` instructions, so an ordinary
//! task recorded nothing at all. Those instructions are advisory and the
//! clients treat them as such.
//!
//! The providers already retain a complete transcript, so this reads that
//! instead. Claude and Codex expose local files; OpenCode exposes the same
//! data through its stable `session list --format json` and `export` CLI
//! commands. It needs no agent cooperation and carries token usage the agent
//! would otherwise have had to volunteer.
//!
//! **Locating the file differs by provider, and so does the confidence:**
//!
//! * **claude** — `--session-id <uuid>` pins the id at spawn, and APAS already
//!   mints one per pane, so the path is exact:
//!   `~/.claude/projects/<cwd with / as ->/<session-id>.jsonl`. No guessing.
//! * **codex** — cannot be given an APAS-chosen id at creation. On Linux, APAS
//!   identifies the pane's provider process group and its Codex app-server
//!   child, then follows the user rollout they actually have open. This remains
//!   exact when several codex panes share a cwd and when one resumes an older
//!   session. The rollout's real id is then persisted for exact future resume. Other
//!   platforms fall back to the newest user rollout whose `session_meta.cwd`
//!   matches exactly, but never persist that ambiguous identity.
//! * **opencode** — session IDs are generated as `ses_*`, so APAS cannot pin
//!   its UUID at creation. The newest session whose exported `directory`
//!   exactly matches the pane cwd is selected, then exported by ID. This has
//!   the same shared-cwd ambiguity as Codex but never crosses directories.
//! * **pi** — `--session-id <uuid>` pins the id at spawn, and the file name
//!   carries it: `~/.pi/agent/sessions/--<cwd slug>--/<timestamp>_<uuid>.jsonl`.
//!   The file is found by the exact id anywhere under the sessions root, so
//!   sibling panes and provider subprocesses sharing a directory can never be
//!   mistaken for this pane. The file appears only once the session's first
//!   assistant message exists; before that the pane simply has no history.

use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

use crate::conversation::TurnRecord;

#[cfg(target_os = "linux")]
pub(crate) mod codex_daemon;

/// Directory name claude derives from a working directory: the absolute path
/// with every `/` replaced by `-`. Verified against a live transcript rather
/// than assumed — `/home/users/shuai/apas` becomes `-home-users-shuai-apas`.
fn claude_dir_slug(cwd: &Path) -> String {
    cwd.to_string_lossy().replace('/', "-")
}

/// Exact transcript path for a claude pane, given the session id we pinned.
pub fn claude_transcript_path(home: &Path, cwd: &Path, session_id: &str) -> PathBuf {
    home.join(".claude")
        .join("projects")
        .join(claude_dir_slug(cwd))
        .join(format!("{session_id}.jsonl"))
}

/// Where a pinned claude session's transcript actually is.
///
/// `claude_transcript_path` is where it *should* be: the slug of the pane's
/// cwd. Claude Code can move a session into one of its own worktrees
/// (`.claude/worktrees/<name>`), and it then writes under the slug of that
/// directory instead — a pane whose provider relocated leaves no file where
/// APAS is watching, looks permanently idle, and (before the floor fix) adopted
/// whatever else was in the project-root slug. Observed on a live pane whose
/// provider had cwd `…/mako/.claude/worktrees/masstree-rocks`.
///
/// The session id is minted by APAS and names the file wherever it lands, so it
/// can be found by name. The cwd slug is still tried first and costs one
/// `exists` call; the scan only happens while no file is there, which is also
/// the pane's first moments before its first turn.
pub fn locate_claude_transcript(home: &Path, cwd: &Path, session_id: &str) -> PathBuf {
    let expected = claude_transcript_path(home, cwd, session_id);
    if expected.exists() {
        return expected;
    }
    let file_name = format!("{session_id}.jsonl");
    let Ok(entries) = std::fs::read_dir(home.join(".claude").join("projects")) else {
        return expected;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let candidate = entry.path().join(&file_name);
        if candidate.is_file() {
            return candidate;
        }
    }
    expected
}

/// Find the newest session file a claude terminal pane should follow after an
/// in-TUI session switch.
///
/// Candidates are `.jsonl` files in the pane's cwd slug directory that are not
/// the currently tracked file, whose names are not in `excluded_ids` (the
/// session ids pinned to *other* panes of the project), and whose modification
/// time is strictly newer than `last_growth` (the currently tracked file's
/// last observed change). The pane's own pinned id is deliberately NOT in
/// `excluded_ids` so a user who switches back to the pinned session is
/// followed again.
///
/// Returns `None` when no switch has happened.
pub fn find_claude_switch_candidate(
    current_path: &Path,
    last_growth: std::time::SystemTime,
    excluded_ids: &HashSet<String>,
) -> Option<PathBuf> {
    // The directory of the file being tracked, not of the pane's cwd: once a
    // session has moved into a worktree, its switches happen where it now is.
    let dir = current_path.parent()?.to_path_buf();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(_) => return None,
    };
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if !path.is_file() || path == current_path {
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if excluded_ids.contains(stem) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let Ok(mtime) = meta.modified() else {
            continue;
        };
        if mtime <= last_growth {
            continue;
        }
        let better = best
            .as_ref()
            .map(|(newest, _)| mtime > *newest)
            .unwrap_or(true);
        if better {
            best = Some((mtime, path));
        }
    }
    best.map(|(_, path)| path)
}

/// Tools whose call *is* the conversation: the agent is asking the human
/// something and the pane is blocked until it is answered. Everything else a
/// tool does stays out of the history.
const QUESTION_TOOLS: &[&str] = &["AskUserQuestion"];

/// The question a claude assistant record is asking, if it is asking one.
fn claude_question(content: &Value) -> Option<crate::conversation::TurnQuestion> {
    content.as_array()?.iter().find_map(|b| {
        if b.get("type").and_then(Value::as_str) != Some("tool_use") {
            return None;
        }
        let name = b.get("name").and_then(Value::as_str)?;
        if !QUESTION_TOOLS.contains(&name) {
            return None;
        }
        Some(crate::conversation::TurnQuestion {
            tool_use_id: b.get("id").and_then(Value::as_str)?.to_string(),
            tool_name: name.to_string(),
            input: b.get("input").cloned().unwrap_or(Value::Null),
        })
    })
}

/// The recorded answer a claude user record carries, if it is answering a
/// question. Only the answer to a question is kept; ordinary tool results are
/// not conversation.
fn claude_answer(
    content: &Value,
    pending: &HashSet<String>,
) -> Option<crate::conversation::TurnAnswer> {
    content.as_array()?.iter().find_map(|b| {
        if b.get("type").and_then(Value::as_str) != Some("tool_result") {
            return None;
        }
        let id = b.get("tool_use_id").and_then(Value::as_str)?;
        if !pending.contains(id) {
            return None;
        }
        let raw = b.get("content");
        let recorded = match raw {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other
                .as_array()
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter_map(|x| x.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default(),
            None => String::new(),
        };
        Some(crate::conversation::TurnAnswer {
            tool_use_id: id.to_string(),
            recorded,
        })
    })
}

/// Pull the text out of a claude content field, which is either a bare string
/// or an array of typed blocks.
fn claude_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Turns from a claude transcript, oldest first.
///
/// Only `user` and `assistant` records become turns. The file also carries
/// `mode`, `permission-mode`, `ai-title` and similar bookkeeping that is not
/// conversation and would be noise in the pane's history.
pub fn parse_claude(raw: &str) -> Vec<TurnRecord> {
    let mut out: Vec<TurnRecord> = Vec::new();
    // tool_use_ids of questions seen so far, so a tool_result is recognised as
    // an answer to one rather than as ordinary tool output.
    let mut asked: HashSet<String> = HashSet::new();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(d) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let record_type = d.get("type").and_then(Value::as_str);
        if record_type == Some("system")
            && d.get("subtype").and_then(Value::as_str) == Some("turn_duration")
        {
            // Claude writes this after the final assistant record. It is also
            // a useful completion edge if the preceding message was observed
            // before its stop_reason had reached disk.
            if let Some(turn) = out.iter_mut().rev().find(|turn| turn.is_assistant()) {
                turn.completes_work = true;
            }
            continue;
        }
        let role = match record_type {
            Some(r @ ("user" | "assistant")) => r,
            _ => continue,
        };
        let msg = d.get("message").unwrap_or(&Value::Null);
        let content = msg.get("content").unwrap_or(&Value::Null);
        let text = claude_text(content);
        let completes_work = role == "assistant"
            && msg
                .get("stop_reason")
                .and_then(Value::as_str)
                .is_some_and(|reason| reason != "tool_use");
        let question = (role == "assistant")
            .then(|| claude_question(content))
            .flatten();
        if let Some(q) = &question {
            asked.insert(q.tool_use_id.clone());
        }
        let answer = (role == "user")
            .then(|| claude_answer(content, &asked))
            .flatten();
        if text.trim().is_empty() && question.is_none() && answer.is_none() {
            // Tool-use-only turns carry no text. Skipping keeps the history
            // readable; the tool calls themselves are not conversation. A
            // question is the exception: it has no text either, but the pane
            // is blocked on it, so dropping it hides the one turn the human
            // has to act on.
            if completes_work {
                if let Some(turn) = out.iter_mut().rev().find(|turn| turn.is_assistant()) {
                    turn.completes_work = true;
                }
            }
            continue;
        }
        let usage = msg.get("usage");
        let tok = |k: &str| usage.and_then(|u| u.get(k)).and_then(Value::as_u64);
        out.push(TurnRecord {
            ts: d
                .get("timestamp")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            // Filled in by the caller, which knows which pane this file is for.
            pane_id: 0,
            role: role.to_string(),
            text,
            model: msg.get("model").and_then(Value::as_str).map(str::to_string),
            input_tokens: tok("input_tokens"),
            output_tokens: tok("output_tokens"),
            cost_usd: None,
            completes_work,
            question,
            answer,
            autopilot_iteration: None,
        });
    }
    out
}

/// Turns from a codex rollout, oldest first.
///
/// Codex nests the real record under `payload`. Only `message` items with a
/// `user` or `assistant` role are conversation: `developer` messages are the
/// harness's own injected context (permissions, plugin lists), and `reasoning`
/// / `custom_tool_call` items are not turns.
pub fn parse_codex(raw: &str) -> Vec<TurnRecord> {
    let mut out: Vec<TurnRecord> = Vec::new();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(d) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if d.get("type").and_then(Value::as_str) == Some("event_msg")
            && d.get("payload")
                .and_then(|payload| payload.get("type"))
                .and_then(Value::as_str)
                == Some("task_complete")
        {
            // Codex emits task_complete just after its final assistant item.
            // A poll can land between those two records, so completion must
            // be tracked independently from the turn-count cursor.
            if let Some(turn) = out.iter_mut().rev().find(|turn| turn.is_assistant()) {
                turn.completes_work = true;
            }
            continue;
        }
        if d.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }
        let p = d.get("payload").unwrap_or(&Value::Null);
        if p.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let role = match p.get("role").and_then(Value::as_str) {
            Some(r @ ("user" | "assistant")) => r,
            _ => continue,
        };
        let text = p
            .get("content")
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        out.push(TurnRecord {
            ts: d
                .get("timestamp")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            pane_id: 0,
            role: role.to_string(),
            text,
            // Codex reports usage per-request in event_msg records rather than
            // on the message, so turns carry none here. Better to report no
            // usage than to attribute a number to the wrong turn.
            model: None,
            input_tokens: None,
            output_tokens: None,
            cost_usd: None,
            completes_work: false,
            // Codex and OpenCode have their own approval interfaces; only
            // the claude parser recognises questions.
            question: None,
            answer: None,
            autopilot_iteration: None,
        });
    }
    out
}

fn millis_timestamp(value: Option<i64>) -> String {
    value
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|timestamp| timestamp.to_rfc3339())
        .unwrap_or_default()
}

/// Turns from `opencode export <session-id>`, oldest first.
///
/// OpenCode persists one message info object plus typed parts. Only completed
/// assistant messages are emitted: exporting while text is streaming can
/// otherwise advance APAS's turn cursor with a partial reply that will never
/// be corrected. Synthetic/ignored text, reasoning, tools, and attachments are
/// intentionally excluded from the human conversation.
pub fn parse_opencode(raw: &str) -> Vec<TurnRecord> {
    let Ok(export) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let Some(messages) = export.get("messages").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut turns = Vec::new();
    for entry in messages {
        let info = entry.get("info").unwrap_or(&Value::Null);
        let Some(role @ ("user" | "assistant")) = info.get("role").and_then(Value::as_str) else {
            continue;
        };
        if role == "assistant"
            && info
                .get("time")
                .and_then(|time| time.get("completed"))
                .and_then(Value::as_i64)
                .is_none()
        {
            continue;
        }
        let text = entry
            .get("parts")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                    .filter(|part| {
                        !part
                            .get("ignored")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                            && !part
                                .get("synthetic")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                    })
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        let finish = info.get("finish").and_then(Value::as_str);
        let completes_work = role == "assistant"
            && finish
                .is_some_and(|reason| !matches!(reason, "tool-calls" | "tool_calls" | "tool_use"));
        let tokens = info.get("tokens");
        let token = |key: &str| {
            tokens
                .and_then(|value| value.get(key))
                .and_then(Value::as_u64)
        };
        let model = if role == "assistant" {
            info.get("modelID").and_then(Value::as_str)
        } else {
            info.get("model")
                .and_then(|model| model.get("modelID"))
                .and_then(Value::as_str)
        };
        turns.push(TurnRecord {
            ts: millis_timestamp(
                info.get("time")
                    .and_then(|time| time.get("created"))
                    .and_then(Value::as_i64),
            ),
            pane_id: 0,
            role: role.to_string(),
            text,
            model: model.map(str::to_string),
            input_tokens: token("input"),
            output_tokens: token("output"),
            cost_usd: None,
            completes_work,
            // Codex and OpenCode have their own approval interfaces; only
            // the claude parser recognises questions.
            question: None,
            answer: None,
            autopilot_iteration: None,
        });
    }
    turns
}

fn same_directory(left: &Path, right: &Path) -> bool {
    let left = std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    left == right
}

/// Select the newest OpenCode session scoped to this exact working directory.
pub fn parse_opencode_session_list(raw: &str, cwd: &Path) -> Option<String> {
    let sessions = serde_json::from_str::<Value>(raw).ok()?;
    sessions
        .as_array()?
        .iter()
        .filter_map(|session| {
            let directory = session.get("directory").and_then(Value::as_str)?;
            if !same_directory(Path::new(directory), cwd) {
                return None;
            }
            let id = session.get("id").and_then(Value::as_str)?;
            let updated = session
                .get("updated")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            Some((updated, id.to_string()))
        })
        .max_by_key(|(updated, _)| *updated)
        .map(|(_, id)| id)
}

/// Ask OpenCode for the newest retained session in `cwd`.
pub fn find_opencode_session(binary: &str, cwd: &Path) -> Result<Option<String>> {
    let output = Command::new(binary)
        .args(["session", "list", "--format", "json", "--max-count", "100"])
        .current_dir(cwd)
        .output()
        .with_context(|| format!("run {binary} session list"))?;
    if !output.status.success() {
        anyhow::bail!(
            "{binary} session list failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(parse_opencode_session_list(
        &String::from_utf8_lossy(&output.stdout),
        cwd,
    ))
}

/// Export and parse one exact OpenCode session, stamping its pane identity.
pub fn read_opencode_turns(
    binary: &str,
    cwd: &Path,
    session_id: &str,
    pane_id: u32,
) -> Result<Vec<TurnRecord>> {
    let output = Command::new(binary)
        .args(["export", session_id])
        .current_dir(cwd)
        .output()
        .with_context(|| format!("run {binary} export {session_id}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "{binary} export failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let mut turns = parse_opencode(&String::from_utf8_lossy(&output.stdout));
    for turn in &mut turns {
        turn.pane_id = pane_id;
    }
    Ok(turns)
}

/// Root holding Pi's per-directory session buckets.
///
/// Pi's own environment overrides are honored so a host that relocates its
/// agent directory does not leave every Pi pane with no recoverable history.
fn pi_sessions_root(home: &Path) -> PathBuf {
    if let Some(dir) = std::env::var_os("PI_CODING_AGENT_SESSION_DIR") {
        return PathBuf::from(dir);
    }
    std::env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".pi").join("agent"))
        .join("sessions")
}

/// The exact Pi session file carrying this pane's pinned identity.
///
/// The id names the file (`<timestamp>_<uuid>.jsonl`) wherever Pi wrote it, so
/// the lookup is by suffix under the sessions root rather than by directory
/// slug: a pane whose provider moved into a worktree keeps its own history, and
/// a sibling pane or provider subprocess sharing a directory is never adopted.
/// Returns `None` until Pi writes its first assistant message.
pub fn find_pi_session_file(home: &Path, session_id: Uuid) -> Option<PathBuf> {
    let suffix = format!("_{session_id}.jsonl");
    let mut stack = vec![pi_sessions_root(home)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(&suffix))
            {
                return Some(path);
            }
        }
    }
    None
}

/// Root holding OMP's session buckets.
///
/// OMP is Pi-derived and reads the same `PI_CODING_AGENT_*` overrides, but its
/// own default lives under `.omp`, so a host that relocated its Pi directory
/// does not silently redirect OMP too unless it set the shared override.
fn omp_sessions_root(home: &Path) -> PathBuf {
    if let Some(dir) = std::env::var_os("PI_CODING_AGENT_SESSION_DIR") {
        return PathBuf::from(dir);
    }
    std::env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".omp").join("agent"))
        .join("sessions")
}

/// The private session directory a pane's OMP process writes into.
///
/// OMP has no `--session-id`, so a pane cannot pin an exact identity the way a
/// Pi pane does. `--session-dir` is stronger in practice: the pane owns the
/// directory outright, so a sibling pane or a provider subprocess sharing a
/// working directory can never select this pane's session, and `--continue`
/// inside it is unambiguous without consulting recency across panes.
///
/// Verified against omp 18.1.20: a run writes one flat `<ts>_<uuid>.jsonl`
/// here, `--continue` appends to that same file rather than starting a new
/// one, and the shared default store is left untouched.
pub fn omp_session_dir(home: &Path, conversation_id: Uuid) -> PathBuf {
    omp_sessions_root(home).join(format!("apas-{conversation_id}"))
}

/// The session file OMP wrote for this pane, if it has written one yet.
///
/// The pane owns its directory, so the newest `.jsonl` in it is unambiguous —
/// unlike Pi, where directory recency could pick up a sibling and the lookup
/// has to go by pinned id. OMP names files `<timestamp>_<uuid>.jsonl`, and a
/// resumed session keeps appending to the file it started, so there is
/// normally exactly one.
pub fn find_omp_session_file(home: &Path, conversation_id: Uuid) -> Option<PathBuf> {
    let dir = omp_session_dir(home, conversation_id);
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(&dir).ok()?.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(modified) = entry.metadata().ok().and_then(|meta| meta.modified().ok()) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(newest, _)| modified > *newest) {
            newest = Some((modified, path));
        }
    }
    newest.map(|(_, path)| path)
}

/// Best durable evidence on the active Pi/OMP branch, independent of visible
/// conversation. Streaming reasoning and queued recovery are not journaled:
/// a live provider report is more authoritative than this snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptActivity {
    Working,
    PendingAnswer,
    Idle,
}

#[derive(Default)]
pub struct PiSnapshot {
    pub turns: Vec<TurnRecord>,
    pub activity: Option<TranscriptActivity>,
    /// Native header identity, never the APAS pane's generated conversation id.
    pub session_id: Option<String>,
}

#[derive(PartialEq, Eq)]
struct PiFileStamp {
    size: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed: (i64, i64),
}

impl PiFileStamp {
    fn new(metadata: &std::fs::Metadata) -> std::io::Result<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            size: metadata.len(),
            modified: metadata.modified()?,
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}

/// One pane's cached Pi/OMP transcript. Unchanged polls only stat the file and
/// borrow its snapshot; metadata changes invalidate rather than imply work.
#[derive(Default)]
pub struct PiTranscriptReader {
    path: PathBuf,
    stamp: Option<PiFileStamp>,
    pane_id: u32,
    omp: bool,
    snapshot: PiSnapshot,
}

impl PiTranscriptReader {
    pub fn read(&mut self, path: &Path, pane_id: u32) -> Result<&PiSnapshot> {
        self.read_provider(path, pane_id, false)
    }

    pub fn read_omp(&mut self, path: &Path, pane_id: u32) -> Result<&PiSnapshot> {
        self.read_provider(path, pane_id, true)
    }

    fn read_provider(&mut self, path: &Path, pane_id: u32, omp: bool) -> Result<&PiSnapshot> {
        use std::io::Read;

        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                self.stamp = None;
                self.snapshot = PiSnapshot::default();
                return Ok(&self.snapshot);
            }
            Err(err) => return Err(err).with_context(|| format!("stat {}", path.display())),
        };
        let stamp = PiFileStamp::new(&metadata)?;
        if self.path != path
            || self.stamp.as_ref() != Some(&stamp)
            || self.omp != omp
            || self.pane_id != pane_id
        {
            let mut file =
                std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
            // Stamp the opened inode before reading. An append/replacement
            // racing this read will invalidate it on the next poll.
            let stamp = PiFileStamp::new(&file.metadata()?)?;
            let mut raw = Vec::new();
            file.read_to_end(&mut raw)
                .with_context(|| format!("read {}", path.display()))?;
            self.snapshot = parse_pi_provider_snapshot(&raw, pane_id, omp);
            self.omp = omp;
            self.path = path.to_path_buf();
            self.stamp = Some(stamp);
            self.pane_id = pane_id;
        }
        Ok(&self.snapshot)
    }
}

/// Pull the text out of a Pi content field: a bare string or an array of typed
/// blocks. Thinking, tool-call, image, and tool-result blocks are not
/// conversation.
fn pi_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn pi_timestamp(entry: &Value, message: &Value) -> String {
    entry
        .get("timestamp")
        .and_then(Value::as_str)
        .filter(|ts| !ts.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| millis_timestamp(message.get("timestamp").and_then(Value::as_i64)))
}

#[derive(serde::Deserialize)]
struct PiEntryLink<'a> {
    #[serde(borrow)]
    id: std::borrow::Cow<'a, str>,
    #[serde(rename = "parentId", borrow)]
    parent: Option<std::borrow::Cow<'a, str>>,
}

/// The autopilot journal of the OMP session being read, as far along the
/// active branch as the reader has walked. It alone decides which user
/// messages are iterations: text is never compared, because OMP stores the
/// expanded text of a prompt.
#[derive(Default)]
struct AutopilotJournal {
    /// The spec of each autopilot's start entry, for its prompt and limit.
    specs: HashMap<Uuid, shared::AutopilotSpec>,
    /// Autopilots whose next marker carries the prompt: after a start, and
    /// after each resume.
    prompt_due: HashSet<Uuid>,
    /// Iteration entries that no message has paired with yet, oldest first.
    unpaired: Vec<(Uuid, u64)>,
}

impl AutopilotJournal {
    fn record(&mut self, data: &Value, session_id: Option<&str>) {
        use crate::omp_autopilot::journal_kind;
        use serde::Deserialize;

        let Ok(entry) = crate::omp_autopilot::JournalEntry::deserialize(data) else {
            return;
        };
        // A fork copies the journal of the session it was made from. Those
        // entries describe that session, not this one.
        if session_id != Some(entry.session_id.as_str()) {
            return;
        }
        let autopilot = entry.autopilot_id;
        match entry.kind.as_str() {
            journal_kind::START => {
                if let Some(spec) = entry.spec {
                    self.specs.insert(autopilot, spec);
                }
                self.prompt_due.insert(autopilot);
            }
            journal_kind::RESUME => {
                self.prompt_due.insert(autopilot);
            }
            journal_kind::ITERATION => {
                if let Some(iteration) = entry.iteration {
                    // One submission is outstanding at a time, so an earlier
                    // entry of this autopilot that never paired never will.
                    self.unpaired.retain(|(id, _)| *id != autopilot);
                    self.unpaired.push((autopilot, iteration));
                }
            }
            journal_kind::NOT_SUBMITTED => {
                // OMP did not accept the submission: it cancels the
                // iteration entry it follows.
                if let Some(index) = self.unpaired.iter().rposition(|(id, _)| *id == autopilot) {
                    self.unpaired.remove(index);
                }
            }
            _ => {}
        }
    }

    /// The marker of an agent-attributed user message, when it pairs with
    /// the nearest earlier iteration entry that is still unpaired.
    fn pair(&mut self) -> Option<shared::AutopilotIterationMarker> {
        let (autopilot_id, iteration) = self.unpaired.pop()?;
        let spec = self.specs.get(&autopilot_id);
        let prompt_due = self.prompt_due.remove(&autopilot_id);
        Some(shared::AutopilotIterationMarker {
            autopilot_id,
            iteration,
            limit: spec.and_then(|spec| spec.iteration_limit.value()),
            prompt: spec.filter(|_| prompt_due).map(|spec| spec.prompt.clone()),
        })
    }
}

#[cfg(test)]
fn parse_pi_snapshot(raw: &[u8], pane_id: u32) -> PiSnapshot {
    parse_pi_provider_snapshot(raw, pane_id, false)
}

/// Follow the last valid entry's parent chain in linear time. Index only
/// links and raw slices, not the potentially huge tool arguments/results.
/// Full records are decoded one at a time on the selected branch.
fn parse_pi_provider_snapshot(raw: &[u8], pane_id: u32, omp: bool) -> PiSnapshot {
    let entries: Vec<_> = raw
        .split(|byte| *byte == b'\n')
        .filter_map(|line| {
            serde_json::from_slice::<PiEntryLink<'_>>(line)
                .ok()
                .map(|link| (link, line))
        })
        .collect();
    let mut by_id = HashMap::with_capacity(entries.len());
    for (index, (link, _)) in entries.iter().enumerate() {
        by_id.entry(link.id.as_ref()).or_insert(index);
    }
    let mut active = Vec::new();
    let mut visited = vec![false; entries.len()];
    let mut cursor = entries.len().checked_sub(1);
    while let Some(index) = cursor {
        if visited[index] {
            break;
        }
        visited[index] = true;
        active.push(index);
        cursor = entries[index]
            .0
            .parent
            .as_deref()
            .and_then(|parent| by_id.get(parent).copied());
    }

    let mut snapshot = PiSnapshot::default();
    snapshot.session_id = raw
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .find(|header| header["type"] == "session")
        .and_then(|header| header["id"].as_str().map(str::to_owned));
    let mut omp_asks = HashMap::new();
    let mut pending_asks = HashSet::new();
    let mut autopilot = AutopilotJournal::default();
    let mut halted = false;
    // Do not search backward through all visible turns for every tool-only
    // completion. Long autonomous runs may have no assistant text at all.
    let mut last_assistant: Option<usize> = None;
    for index in active.into_iter().rev() {
        let Ok(entry) = serde_json::from_slice::<Value>(entries[index].1) else {
            continue;
        };
        let kind = entry.get("type").and_then(Value::as_str);
        let custom_type = entry.get("customType").and_then(Value::as_str);
        match (kind, custom_type) {
            (Some("custom"), Some("session_exit")) => {
                pending_asks.clear();
                halted = true;
                snapshot.activity = Some(TranscriptActivity::Idle);
            }
            (Some("custom"), Some("tool_execution_start")) => {
                halted = false;
                let data = entry.get("data").unwrap_or(&Value::Null);
                if data.get("toolName").and_then(Value::as_str) == Some("ask") {
                    if let Some(id) = data.get("toolCallId").and_then(Value::as_str) {
                        pending_asks.insert(id.to_owned());
                    }
                }
                snapshot.activity = Some(if pending_asks.is_empty() {
                    TranscriptActivity::Working
                } else {
                    TranscriptActivity::PendingAnswer
                });
            }
            (Some("custom_message"), Some("async-result" | "launch-completion")) => {
                halted = false;
                snapshot.activity = Some(if pending_asks.is_empty() {
                    TranscriptActivity::Working
                } else {
                    TranscriptActivity::PendingAnswer
                });
            }
            (Some("custom"), Some(crate::omp_autopilot::JOURNAL_ENTRY_TYPE)) if omp => {
                autopilot.record(
                    entry.get("data").unwrap_or(&Value::Null),
                    snapshot.session_id.as_deref(),
                );
            }
            _ => {}
        }
        if kind != Some("message") {
            continue;
        }
        let message = entry.get("message").unwrap_or(&Value::Null);
        let content = message.get("content").unwrap_or(&Value::Null);
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let out = &mut snapshot.turns;
        match role {
            "user" => {
                halted = false;
                snapshot.activity = Some(if pending_asks.is_empty() {
                    TranscriptActivity::Working
                } else {
                    TranscriptActivity::PendingAnswer
                });
                // An iteration is whatever the session's own journal says
                // autopilot submitted. A message a person sent is never one,
                // whatever was journaled before it.
                if message.get("attribution").and_then(Value::as_str) == Some("agent") {
                    if let Some(marker) = autopilot.pair() {
                        let mut turn = omp_tool_turn(pane_id, pi_timestamp(&entry, message));
                        turn.autopilot_iteration = Some(marker);
                        out.push(turn);
                        continue;
                    }
                }
                let text = pi_text(content);
                if text.trim().is_empty() {
                    continue;
                }
                out.push(TurnRecord {
                    ts: pi_timestamp(&entry, message),
                    pane_id,
                    role: "user".to_string(),
                    text,
                    model: None,
                    input_tokens: None,
                    output_tokens: None,
                    cost_usd: None,
                    completes_work: false,
                    question: None,
                    answer: None,
                    autopilot_iteration: None,
                });
            }
            "assistant" => {
                // Native ask calls are conversation, but only for OMP. Keep
                // them separate from prose/usage: the stream converter treats
                // question records as tools rather than assistant text.
                if omp {
                    if let (Some(session_id), Some(blocks)) =
                        (snapshot.session_id.as_deref(), content.as_array())
                    {
                        for block in blocks {
                            if block["type"] != "toolCall" || block["name"] != "ask" {
                                continue;
                            }
                            let (Some(id), Some(questions)) = (
                                block["id"].as_str(),
                                block["arguments"]["questions"].as_array(),
                            ) else {
                                continue;
                            };
                            let question =
                                normalize_omp_question(pane_id, session_id, id, questions);
                            omp_asks.insert(id.to_owned(), question.clone());
                            pending_asks.insert(id.to_owned());
                            let mut turn = omp_tool_turn(pane_id, pi_timestamp(&entry, message));
                            turn.question = Some(question);
                            out.push(turn);
                        }
                    }
                }
                let text = pi_text(content);
                let reason = message.get("stopReason").and_then(Value::as_str);
                let has_tools = content.as_array().is_some_and(|blocks| {
                    blocks
                        .iter()
                        .any(|block| block.get("type").and_then(Value::as_str) == Some("toolCall"))
                });
                let paused = message
                    .get("stopDetails")
                    .and_then(|details| details.get("type"))
                    .and_then(Value::as_str)
                    == Some("pause_turn");
                halted = matches!(reason, Some("error" | "aborted"));
                let continuing = !halted
                    && (matches!(
                        reason,
                        Some("toolUse" | "tool_use" | "pending" | "deferred")
                    ) || (matches!(reason, Some("stop" | "length") | None) && has_tools)
                        || (reason == Some("stop") && paused));
                // Older OMP releases persist completedAt without stopReason.
                let completes_work = halted
                    || (!continuing
                        && (matches!(reason, Some("stop" | "length"))
                            || message.get("completedAt").is_some_and(Value::is_number)));
                if completes_work {
                    pending_asks.clear();
                    snapshot.activity = Some(TranscriptActivity::Idle);
                } else {
                    snapshot.activity = Some(if pending_asks.is_empty() {
                        TranscriptActivity::Working
                    } else {
                        TranscriptActivity::PendingAnswer
                    });
                }
                if text.trim().is_empty() {
                    // A silent completion can finish the last visible response,
                    // but executable tools/pause_turn must never mark it idle.
                    if completes_work {
                        if let Some(index) = last_assistant {
                            out[index].completes_work = true;
                        }
                    }
                    // OMP tool-only rounds still carry real billable usage.
                    // Pi keeps its existing text-only conversation behavior.
                    if !omp || message.get("usage").is_none() {
                        continue;
                    }
                }
                let usage = message.get("usage");
                let token = |key: &str| {
                    usage
                        .and_then(|usage| usage.get(key))
                        .and_then(Value::as_u64)
                };
                let cost_usd = usage
                    .and_then(|usage| usage.get("cost"))
                    .and_then(|cost| cost.get("total"))
                    .and_then(Value::as_f64);
                out.push(TurnRecord {
                    ts: pi_timestamp(&entry, message),
                    pane_id,
                    role: "assistant".to_string(),
                    text,
                    model: message
                        .get("model")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    input_tokens: token("input"),
                    output_tokens: token("output"),
                    cost_usd,
                    completes_work,
                    question: None,
                    answer: None,
                    autopilot_iteration: None,
                });
                last_assistant = Some(out.len() - 1);
            }
            "toolResult" => {
                if let Some(id) = message.get("toolCallId").and_then(Value::as_str) {
                    pending_asks.remove(id);
                    if let Some(question) = omp_asks.remove(id) {
                        let mut turn = omp_tool_turn(pane_id, pi_timestamp(&entry, message));
                        turn.role = "user".into();
                        turn.answer = Some(normalize_omp_answer(&question, message));
                        out.push(turn);
                    }
                }
                // Aborted/error assistant records can be followed by synthetic
                // result pairs; those do not restart the stopped run.
                snapshot.activity = Some(if halted {
                    TranscriptActivity::Idle
                } else if pending_asks.is_empty() {
                    TranscriptActivity::Working
                } else {
                    TranscriptActivity::PendingAnswer
                });
            }
            _ => {}
        }
    }
    snapshot
}

pub(crate) fn omp_tool_turn(pane_id: u32, ts: String) -> TurnRecord {
    TurnRecord {
        pane_id,
        ts,
        role: "assistant".into(),
        text: String::new(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        cost_usd: None,
        completes_work: false,
        question: None,
        answer: None,
        autopilot_iteration: None,
    }
}

fn normalize_omp_question(
    pane_id: u32,
    session_id: &str,
    id: &str,
    native_questions: &[Value],
) -> crate::conversation::TurnQuestion {
    let mut questions = native_questions.to_vec();
    for question in &mut questions {
        if let Some(object) = question.as_object_mut() {
            if let Some(multi) = object.remove("multi") {
                object.insert("multiSelect".into(), multi);
            }
        }
    }
    crate::conversation::TurnQuestion {
        tool_use_id: crate::omp_activity::question_tool_use_id(pane_id, session_id, id),
        tool_name: "AskUserQuestion".into(),
        input: serde_json::json!({
            "provider": "omp", "questions": questions, "omp": { "dialog_id": null },
        }),
    }
}

pub(crate) fn omp_unavailable_answer(
    tool_use_id: String,
    message: &str,
) -> crate::conversation::TurnAnswer {
    crate::conversation::TurnAnswer {
        tool_use_id,
        recorded: serde_json::json!({
            "type": "omp_question_result", "status": "unavailable",
            "results": [], "message": message,
        })
        .to_string(),
    }
}

fn normalize_omp_answer(
    question: &crate::conversation::TurnQuestion,
    message: &Value,
) -> crate::conversation::TurnAnswer {
    let details = &message["details"];
    let text = pi_text(&message["content"]);
    let mut results = details["results"].as_array().cloned().unwrap_or_default();
    if details["selectedOptions"].is_array() {
        let mut result = details.clone();
        result["id"] = question.input["questions"][0]["id"].clone();
        results.push(result);
    }
    let successful = message["isError"] != true && !results.is_empty();
    // These messages are the native AskTool's explicit cancellation errors.
    let status = if successful {
        "answered"
    } else if text.contains("Ask tool was cancelled by the user")
        || text.contains("Ask input was cancelled")
    {
        "cancelled"
    } else {
        "unavailable"
    };
    crate::conversation::TurnAnswer {
        tool_use_id: question.tool_use_id.clone(),
        recorded: serde_json::json!({
            "type": "omp_question_result", "status": status,
            "results": results, "message": text,
        })
        .to_string(),
    }
}

/// Newest codex rollout whose `session_meta.cwd` matches `cwd`.
///
/// Heuristic by necessity — Codex can resume an id but cannot be told which id
/// to mint for a fresh TUI. Requiring an exact cwd match keeps the portable
/// fallback ambiguity to panes sharing a directory.
/// The identifying header of a codex rollout, taken from its `session_meta`.
struct CodexRolloutMeta {
    /// Codex's actual resumable conversation identity. APAS cannot choose this
    /// id when it starts a fresh interactive TUI; it learns it from the
    /// process-owned rollout after Codex creates the session.
    session_id: Option<Uuid>,
    cwd: String,
    /// Immutable session start. Selection keys on this rather than mtime
    /// precisely because it cannot change while the file is being appended to.
    started_at: String,
    /// `user` for a session a person drives, `subagent` for a thread codex
    /// spawned itself. Absent on rollouts written by older codex versions.
    thread_source: Option<String>,
    /// Newer Codex versions use `source: { "subagent": ... }` and an
    /// `agent_path` instead of the flat `thread_source` discriminator.
    modern_subagent: bool,
}

impl CodexRolloutMeta {
    /// Whether this rollout belongs to a thread codex spawned on its own.
    ///
    /// Anything that is not explicitly a non-user thread counts as the user's:
    /// codex versions predating `thread_source` omit the field entirely, and
    /// treating those as subagents would leave their panes with no transcript.
    fn is_subagent(&self) -> bool {
        self.modern_subagent
            || self
                .thread_source
                .as_deref()
                .is_some_and(|source| !source.eq_ignore_ascii_case("user"))
    }
}

/// Read just the `session_meta` header that leads every codex rollout.
///
/// Deliberately reads one line rather than the whole file. These rollouts reach
/// hundreds of megabytes (one live example was 458 MB), and the previous
/// `read_to_string` pulled every candidate fully into memory on every poll of
/// every terminal pane just to look at line 1.
fn read_codex_rollout_meta(path: &Path) -> Option<CodexRolloutMeta> {
    use std::io::BufRead;

    let file = std::fs::File::open(path).ok()?;
    let mut first = String::new();
    std::io::BufReader::new(file).read_line(&mut first).ok()?;
    let d: Value = serde_json::from_str(first.trim()).ok()?;
    let meta = d.get("payload").unwrap_or(&d);
    Some(CodexRolloutMeta {
        session_id: meta
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| Uuid::parse_str(id).ok()),
        cwd: meta.get("cwd").and_then(Value::as_str)?.to_string(),
        started_at: meta
            .get("timestamp")
            .and_then(Value::as_str)
            .or_else(|| d.get("timestamp").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string(),
        thread_source: meta
            .get("thread_source")
            .and_then(Value::as_str)
            .map(str::to_string),
        modern_subagent: meta
            .get("source")
            .and_then(Value::as_object)
            .is_some_and(|source| source.contains_key("subagent"))
            || meta
                .get("agent_path")
                .and_then(Value::as_str)
                .is_some_and(|path| !path.is_empty()),
    })
}

/// The resumable Codex id recorded in one rollout's `session_meta` header.
///
/// Callers must establish ownership of `path` first. A UUID read from the
/// newest file in a shared cwd is valid Codex data but not necessarily this
/// pane's conversation.
pub(crate) fn codex_rollout_session_id(path: &Path) -> Option<Uuid> {
    read_codex_rollout_meta(path)?.session_id
}

fn codex_rollout_paths(home: &Path) -> Vec<PathBuf> {
    let root = home.join(".codex").join("sessions");
    let mut candidates = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("jsonl") {
                candidates.push(path);
            }
        }
    }
    candidates
}

/// Whether a persisted UUID still names a local, user-owned Codex session.
///
/// This is also the legacy discriminator: older APAS versions stored a random
/// pane UUID in the same field. Only a UUID that appears in a real rollout is
/// safe to pass to `codex resume <id>`; an unknown UUID must retain the picker.
pub(crate) fn codex_user_session_exists(home: &Path, session_id: Uuid) -> bool {
    codex_rollout_paths(home).into_iter().any(|path| {
        read_codex_rollout_meta(&path)
            .is_some_and(|meta| meta.session_id == Some(session_id) && !meta.is_subagent())
    })
}

/// Locate the rollout for the codex session running in `cwd`.
///
/// Two properties matter more than they look:
///
/// * **Subagent threads are skipped.** Codex spawns its own threads and each
///   writes a separate rollout carrying the *parent's* cwd, so matching on cwd
///   alone returns several live files for one pane.
/// * **Selection keys on the session's start timestamp, not mtime.** Those
///   sibling rollouts are appended to concurrently, so the newest-mtime file
///   changes from poll to poll. The start timestamp is fixed once written.
///
/// Together those caused a production outage: the selection flapped between a
/// pane's real session and three subagent threads, and every flip looked like a
/// new transcript and republished it from the beginning — 4 million duplicate
/// messages, a 2.9 GB message store, and a server that exhausted its memory
/// limit. A newer *user* session still wins, which is the intended behaviour
/// when someone restarts codex in the same directory.
pub fn find_codex_rollout(home: &Path, cwd: &Path) -> Option<PathBuf> {
    select_codex_rollout(codex_rollout_paths(home), cwd)
}

/// Newest user rollout for `cwd` among a bounded set of candidate files.
///
/// The stable start timestamp and path tie-break keep selection independent of
/// append mtimes. That matters both for the directory fallback and for a codex
/// process group that still has old resumed sessions or subagent rollouts open.
fn select_codex_rollout(
    candidates: impl IntoIterator<Item = PathBuf>,
    cwd: &Path,
) -> Option<PathBuf> {
    let expected_cwd = cwd.to_string_lossy();
    let mut best: Option<(String, PathBuf)> = None;
    for path in candidates {
        let Some(meta) = read_codex_rollout_meta(&path) else {
            continue;
        };
        if meta.cwd != expected_cwd || meta.is_subagent() {
            continue;
        }
        let key = meta.started_at;
        let better = match best.as_ref() {
            None => true,
            Some((best_key, best_path)) => (&key, &path) > (best_key, best_path),
        };
        if better {
            best = Some((key, path));
        }
    }
    best.map(|(_, path)| path)
}

/// User rollout currently owned by one terminal's provider process group or
/// its directly launched Codex app-server.
///
/// Codex 0.157 moves the rollout writer into an app-server child with its own
/// process group. Inspect that child's descriptors too, while retaining the
/// parent relationship that distinguishes panes sharing a checkout. Do not
/// search arbitrary descendants: a tool can launch another independent Codex
/// session, whose user rollout must not replace this pane's conversation.
#[cfg(target_os = "linux")]
pub fn find_codex_rollout_for_process_group(
    home: &Path,
    cwd: &Path,
    process_group_id: i32,
) -> Option<PathBuf> {
    find_codex_rollout_in_proc(home, cwd, process_group_id, Path::new("/proc"))
}

#[cfg(target_os = "linux")]
fn find_codex_rollout_in_proc(
    home: &Path,
    cwd: &Path,
    process_group_id: i32,
    proc_root: &Path,
) -> Option<PathBuf> {
    let sessions_root = home.join(".codex").join("sessions");
    let mut candidates = HashSet::new();
    let mut processes = Vec::new();
    let entries = std::fs::read_dir(proc_root).ok()?;
    for entry in entries.filter_map(|entry| entry.ok()) {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // `comm` is parenthesized and may contain spaces. Fields after its
        // closing ')' begin with state, ppid, then pgrp (proc_pid_stat(5)).
        let Some(mut fields) = stat
            .rfind(')')
            .and_then(|end| stat.get(end + 1..))
            .map(str::split_whitespace)
        else {
            continue;
        };
        fields.next(); // state
        let (Some(parent), Some(group)) = (
            fields.next().and_then(|value| value.parse::<u32>().ok()),
            fields.next().and_then(|value| value.parse::<i32>().ok()),
        ) else {
            continue;
        };
        processes.push((pid, parent, group));
    }
    let group_members: HashSet<u32> = processes
        .iter()
        .filter_map(|(pid, _, group)| (*group == process_group_id).then_some(*pid))
        .collect();
    for (pid, parent, _) in processes {
        let process_dir = proc_root.join(pid.to_string());
        if !group_members.contains(&pid) {
            if !group_members.contains(&parent) {
                continue;
            }
            let Ok(command) = std::fs::read(process_dir.join("cmdline")) else {
                continue;
            };
            let mut args = command.split(|byte| *byte == 0);
            let executable = args.next().and_then(|arg| std::str::from_utf8(arg).ok());
            let named_codex = executable.and_then(|arg| Path::new(arg).file_name())
                == Some(std::ffi::OsStr::new("codex"));
            // Native providers use content-addressed `bin-<hash>` names on
            // local storage. A direct app-server running the TUI's exact
            // executable is just as specific as the ordinary `codex` name.
            let same_executable = std::fs::read_link(process_dir.join("exe"))
                .ok()
                .zip(std::fs::read_link(proc_root.join(parent.to_string()).join("exe")).ok())
                .is_some_and(|(child, parent)| child == parent);
            if (!named_codex && !same_executable) || args.next() != Some(b"app-server".as_slice()) {
                continue;
            }
        }
        let Ok(fds) = std::fs::read_dir(process_dir.join("fd")) else {
            continue;
        };
        for fd in fds.filter_map(|fd| fd.ok()) {
            let Ok(path) = std::fs::read_link(fd.path()) else {
                continue;
            };
            if path.starts_with(&sessions_root)
                && path.extension().and_then(|extension| extension.to_str()) == Some("jsonl")
            {
                candidates.insert(path);
            }
        }
    }
    select_codex_rollout(candidates, cwd)
}

/// Read a transcript and stamp every turn with the pane it belongs to.
pub fn read_turns(path: &Path, pane_id: u32, is_codex: bool) -> Result<Vec<TurnRecord>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };
    let mut turns = if is_codex {
        parse_codex(&raw)
    } else {
        parse_claude(&raw)
    };
    for t in &mut turns {
        t.pane_id = pane_id;
    }
    Ok(turns)
}

/// A Codex transcript plus its provider-confirmed current task state.
///
/// A restored pane suppresses old turns so they are not republished, but an
/// unmatched `task_started` still has to restore Working after a reconnect.
pub(crate) fn read_codex_snapshot(
    path: &Path,
    pane_id: u32,
) -> Result<(Vec<TurnRecord>, Option<bool>)> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), None));
        }
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };
    let mut turns = parse_codex(&raw);
    for turn in &mut turns {
        turn.pane_id = pane_id;
    }
    Ok((turns, codex_working_state(&raw)))
}

/// Latest explicit task lifecycle recorded by Codex. File order is
/// authoritative because a TUI runs one root task at a time.
fn codex_working_state(raw: &str) -> Option<bool> {
    for line in raw.lines().rev().filter(|line| !line.trim().is_empty()) {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) != Some("event_msg") {
            continue;
        }
        match record
            .get("payload")
            .and_then(|payload| payload.get("type"))
            .and_then(Value::as_str)
        {
            Some("task_started") => return Some(true),
            Some("task_complete" | "turn_aborted") => return Some(false),
            _ => {}
        }
    }
    None
}

/// An OMP session file with two autopilots in it, for the tests here and in
/// the project runtime.
///
/// The entries have the shape OMP 18.20.0 writes: copied from sessions
/// captured for this change's provider checks, with their own ids, parent
/// links and `message.attribution`. The `apas-autopilot` entries are what the
/// APAS extension appends; the provider checks confirmed how `pi.appendEntry`
/// records a custom entry, not this journal's content, which is the contract
/// in `omp_autopilot.rs`.
#[cfg(test)]
pub(crate) mod autopilot_fixture {
    use serde_json::{json, Value};

    pub(crate) const SESSION: &str = "01a1d594-4d99-729d-a0a1-0d838bc273a9";
    /// The session this one was forked from. A fork copies its entries.
    pub(crate) const FORKED_FROM: &str = "01a1d59b-9acf-71dc-aa87-55d2826a616e";
    pub(crate) const FIRST: &str = "11111111-1111-4111-8111-111111111111";
    pub(crate) const SECOND: &str = "22222222-2222-4222-8222-222222222222";
    pub(crate) const FIRST_PROMPT: &str = "Check the build and fix what fails.";
    pub(crate) const SECOND_PROMPT: &str = "Look at the alerts.\n\tPage nobody.";

    pub(crate) struct Session {
        lines: Vec<String>,
        parent: Value,
        entries: u32,
    }

    impl Session {
        pub(crate) fn new() -> Self {
            let title = json!({
                "type": "title", "v": 1, "title": "Check the build", "source": "auto",
                "updatedAt": "2026-11-13T15:44:41.119Z", "pad": "        ",
            });
            let header = json!({
                "type": "session", "version": 3, "id": SESSION,
                "timestamp": "2026-11-13T15:43:48.634Z",
                "cwd": "/home/users/shuai/.cache/apas-autopilot-check/work",
            });
            let mut session = Self {
                lines: vec![title.to_string(), header.to_string()],
                parent: Value::Null,
                entries: 0,
            };
            session.push(json!({
                "type": "model_change", "model": "deepseek/deepseek-v4-flash",
                "resolvedModelIsFallback": false,
            }));
            session.push(json!({
                "type": "thinking_level_change", "thinkingLevel": "high", "configured": null,
            }));
            session
        }

        fn push(&mut self, mut entry: Value) {
            self.entries += 1;
            let id = format!("{:08x}", 0x7c02_b4bb + self.entries);
            entry["id"] = json!(id);
            entry["parentId"] = std::mem::replace(&mut self.parent, json!(id));
            entry["timestamp"] = json!(format!("2026-11-13T15:44:{:02}.756Z", self.entries % 60));
            entry["runtime"] = json!({ "version": "18.20.0" });
            self.lines.push(entry.to_string());
        }

        /// One `apas-autopilot` entry of `autopilot` in `session`.
        pub(crate) fn journal(&mut self, session: &str, autopilot: &str, kind: &str, more: Value) {
            let mut data = json!({
                "v": 1, "autopilot_id": autopilot, "session_id": session, "kind": kind,
                "at": "2026-11-13T15:44:06Z",
            });
            for (key, value) in more.as_object().into_iter().flatten() {
                data[key] = value.clone();
            }
            self.push(json!({ "type": "custom", "customType": "apas-autopilot", "data": data }));
        }

        /// A recorded prompt, behind the reminder entry OMP writes before it.
        pub(crate) fn user(&mut self, attribution: &str, text: &str) {
            let reminder = json!({
                "conversation": "user:1794584646075:agent", "turn": "user:1794584646075:agent",
                "reminder": "<system-reminder>\nToday: 2026-11-13\n</system-reminder>",
                "placement": "prefix", "firstTurn": "d7d5f0de0ed049c1",
            });
            self.push(json!({
                "type": "custom", "customType": "date-cwd-reminder", "data": reminder,
            }));
            let message = json!({
                "role": "user", "content": [{ "type": "text", "text": text }],
                "attribution": attribution, "timestamp": 1794584646075u64,
            });
            self.push(json!({ "type": "message", "message": message }));
        }

        pub(crate) fn assistant(&mut self, text: &str) {
            self.push(json!({
                "type": "custom", "customType": "system-prompt-digest", "data": "9a14208174d86536",
            }));
            let usage = json!({
                "input": 8421, "output": 3, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 8424,
                "cost": {
                    "input": 0.00126315, "output": 1.8e-06, "cacheRead": 0, "cacheWrite": 0,
                    "total": 0.00126495,
                },
            });
            let message = json!({
                "role": "assistant", "content": [{ "type": "text", "text": text }],
                "api": "openai-completions", "provider": "deepseek", "model": "deepseek-v4-flash",
                "usage": usage, "stopReason": "stop", "timestamp": 1794584646759u64,
                "responseId": "44878d81-a763-474e-b5ed-ac91534664be",
                "duration": 1112.5, "ttft": 1096.5, "completedAt": 1794584647873u64,
                "contextSnapshot": {
                    "promptTokens": 8421, "nonMessageTokens": 6826, "toolSchemaTokens": 4595,
                    "compactionEpoch": 0,
                },
            });
            self.push(json!({ "type": "message", "message": message }));
        }

        pub(crate) fn file(&self) -> String {
            self.lines.join("\n") + "\n"
        }
    }

    /// The data of a start entry, less what [`Session::journal`] adds.
    pub(crate) fn start(prompt: &str, iteration_limit: Value) -> Value {
        json!({
            "spec": {
                "prompt": prompt, "repeat": { "kind": "after_each_turn" },
                "iteration_limit": iteration_limit, "time_limit": { "kind": "none" },
                "end_without_tool_call": false,
            },
            "started_at": "2026-11-13T15:44:06Z",
        })
    }

    /// Two autopilots, a submission OMP dropped, a person's messages sent
    /// during autopilot, a pause and resume, and the journal a fork copied.
    pub(crate) fn session() -> String {
        let mut session = Session::new();
        let no_limit = json!({ "kind": "none" });

        // Copied by the fork, with the prompt that autopilot had submitted.
        // Its entries name the other session.
        let copied = "33333333-3333-4333-8333-333333333333";
        let instruction = "Copied instruction.";
        session.journal(
            FORKED_FROM,
            copied,
            "start",
            start(instruction, no_limit.clone()),
        );
        session.journal(FORKED_FROM, copied, "iteration", json!({ "iteration": 4 }));
        session.user("agent", instruction);
        session.assistant("Copied reply.");
        let left = json!({ "iterations": 4, "reason": "conversation_changed" });
        session.journal(FORKED_FROM, copied, "end", left);

        let twenty = json!({ "kind": "at_most", "value": 20 });
        session.journal(SESSION, FIRST, "start", start(FIRST_PROMPT, twenty));
        session.journal(SESSION, FIRST, "iteration", json!({ "iteration": 1 }));
        // OMP stores the expanded text of a prompt, so it need not be the
        // prompt that was submitted.
        session.user("agent", "Check the build and fix what fails. (expanded)");
        session.assistant("The build is green.");

        // A person steers the tab between two iterations.
        session.user("user", "Also update the changelog.");
        session.assistant("Changelog updated.");

        // A person's message is recorded after the iteration was journaled
        // and before OMP records its prompt.
        session.journal(SESSION, FIRST, "iteration", json!({ "iteration": 2 }));
        session.user("user", "And bump the version.");
        session.user("agent", FIRST_PROMPT);
        session.assistant("Nothing failed.");

        // OMP dropped the third submission. The person asks about it.
        session.journal(SESSION, FIRST, "iteration", json!({ "iteration": 3 }));
        session.journal(SESSION, FIRST, "not_submitted", json!({ "iteration": 3 }));
        let interrupted = json!({ "iterations": 2, "reason": "interrupted" });
        session.journal(SESSION, FIRST, "pause", interrupted);
        session.user("user", "Why did you stop?");
        session.assistant("You interrupted me.");

        session.journal(SESSION, FIRST, "resume", json!({ "iterations": 2 }));
        session.journal(SESSION, FIRST, "iteration", json!({ "iteration": 3 }));
        session.user("agent", FIRST_PROMPT);
        session.assistant("Still green.");
        let stopped = json!({ "iterations": 3, "reason": "stopped" });
        session.journal(SESSION, FIRST, "end", stopped);

        // A second autopilot, with no iteration limit.
        session.journal(SESSION, SECOND, "start", start(SECOND_PROMPT, no_limit));
        session.journal(SESSION, SECOND, "iteration", json!({ "iteration": 1 }));
        session.user("agent", SECOND_PROMPT);
        session.assistant("No open alerts.");
        session.journal(SESSION, SECOND, "iteration", json!({ "iteration": 2 }));
        session.user("agent", SECOND_PROMPT);
        session.file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn omp_ask_fixture(text: &str) -> Vec<Value> {
        vec![
            serde_json::json!({"type":"title","v":1,"title":"","pad":"   "}),
            serde_json::json!({"type":"session","id":"native-session","version":3}),
            serde_json::json!({"type":"message","id":"ask-entry","parentId":null,
            "timestamp":"2026-10-10T00:00:00Z","message":{
                "role":"assistant","stopReason":"toolUse","model":"native-model",
                "content":[{"type":"text","text":text},{"type":"toolCall","id":"call_ask","name":"ask",
                    "arguments":{"questions":[{"id":"choice","question":"Choose","multi":true,
                        "options":[{"label":"A, B","description":"One label"},{"label":"C"}]}]}}],
                "usage":{"input":17,"output":9,"cost":{"total":0.25}}
            }}),
            serde_json::json!({"type":"custom","id":"start","parentId":"ask-entry",
                "customType":"tool_execution_start","data":{"toolName":"ask","toolCallId":"call_ask"}}),
        ]
    }

    fn omp_fixture_bytes(entries: &[Value]) -> Vec<u8> {
        entries
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes()
    }

    #[test]
    fn an_iteration_is_a_marker_by_the_sessions_own_journal_and_never_a_user_turn() {
        use autopilot_fixture::{FIRST, FIRST_PROMPT, SECOND, SECOND_PROMPT};

        let raw = autopilot_fixture::session();
        let snapshot = parse_pi_provider_snapshot(raw.as_bytes(), 7, true);
        assert_eq!(
            snapshot.session_id.as_deref(),
            Some(autopilot_fixture::SESSION)
        );

        // What a person sent stays theirs, in order; so does a prompt that a
        // fork copied from another session's autopilot. No iteration is here.
        let said: Vec<&str> = snapshot
            .turns
            .iter()
            .filter(|turn| turn.role == "user")
            .map(|turn| turn.text.as_str())
            .collect();
        assert_eq!(
            said,
            [
                "Copied instruction.",
                "Also update the changelog.",
                "And bump the version.",
                "Why did you stop?",
            ]
        );

        let marker = |autopilot: &str, iteration, limit, prompt: Option<&str>| {
            shared::AutopilotIterationMarker {
                autopilot_id: Uuid::parse_str(autopilot).unwrap(),
                iteration,
                limit,
                prompt: prompt.map(str::to_string),
            }
        };
        let markers: Vec<_> = snapshot
            .turns
            .iter()
            .filter_map(|turn| turn.autopilot_iteration.clone())
            .collect();
        assert_eq!(
            markers,
            [
                // The prompt comes from the start entry, not from the text
                // OMP stored, and only on the first marker.
                marker(FIRST, 1, Some(20), Some(FIRST_PROMPT)),
                marker(FIRST, 2, Some(20), None),
                // The dropped third submission has no marker. The one after
                // the resume is the third, and carries the prompt again.
                marker(FIRST, 3, Some(20), Some(FIRST_PROMPT)),
                // No iteration limit: `limit` is left out.
                marker(SECOND, 1, None, Some(SECOND_PROMPT)),
                marker(SECOND, 2, None, None),
            ]
        );

        // A marker is its own kind of turn: no text, no question, nothing a
        // completion or the question replay would act on.
        for turn in snapshot
            .turns
            .iter()
            .filter(|turn| turn.autopilot_iteration.is_some())
        {
            assert!(turn.is_assistant() && turn.text.is_empty());
            assert!(turn.question.is_none() && turn.answer.is_none());
            assert!(!turn.completes_work && !turn.has_usage());
        }
        // Each one stands where its prompt was recorded, before the reply.
        let order: Vec<&str> = snapshot
            .turns
            .iter()
            .map(
                |turn| match (&turn.autopilot_iteration, turn.role.as_str()) {
                    (Some(_), _) => "iteration",
                    (None, role) => role,
                },
            )
            .collect();
        assert_eq!(
            order,
            [
                "user",
                "assistant",
                "iteration",
                "assistant",
                "user",
                "assistant",
                "user",
                "iteration",
                "assistant",
                "user",
                "assistant",
                "iteration",
                "assistant",
                "iteration",
                "assistant",
                "iteration",
            ]
        );
        // The second autopilot's last iteration is still running.
        assert_eq!(snapshot.activity, Some(TranscriptActivity::Working));

        // Pi has no APAS extension: the same file read as Pi's has no markers.
        let pi = parse_pi_snapshot(raw.as_bytes(), 7);
        assert!(pi
            .turns
            .iter()
            .all(|turn| turn.autopilot_iteration.is_none()));
    }

    /// A session written by OMP 18.20.0 with the built extension loaded, on
    /// 2026-11-13, with only its working directory shortened. Four autopilots
    /// ran in it and nobody typed a message: one ended at its iteration limit;
    /// one was paused, resumed and stopped; one was stopped at once in the
    /// middle of its turn; and one was interrupted with Esc, resumed, and
    /// ended when `/new` left the conversation.
    const OMP_AUTOPILOT_CAPTURE: &str =
        include_str!("testdata/omp-18.20.0-autopilot-session.jsonl");

    #[test]
    fn a_captured_autopilot_session_reads_as_iteration_markers_and_no_user_turns() {
        let snapshot = parse_pi_provider_snapshot(OMP_AUTOPILOT_CAPTURE.as_bytes(), 7, true);
        let markers: Vec<_> = snapshot
            .turns
            .iter()
            .filter_map(|turn| turn.autopilot_iteration.as_ref())
            .map(|marker| {
                (
                    marker.autopilot_id.to_string()[..8].to_string(),
                    marker.iteration,
                    marker.limit,
                )
            })
            .collect();
        let expected: Vec<(String, u64, Option<u64>)> = [
            ("07396902", 1, Some(3)),
            ("07396902", 2, Some(3)),
            ("07396902", 3, Some(3)),
            ("41eb2b26", 1, None),
            ("41eb2b26", 2, None),
            ("41eb2b26", 3, None),
            ("d1954ac9", 1, None),
            ("5eb22679", 1, None),
            ("5eb22679", 2, None),
            ("5eb22679", 3, None),
        ]
        .into_iter()
        .map(|(id, iteration, limit)| (id.to_string(), iteration, limit))
        .collect();
        assert_eq!(markers, expected);

        // OMP wrapped each prompt in a reminder entry and stored it as a user
        // message; none of them is shown as something a person sent.
        assert!(snapshot.turns.iter().all(|turn| turn.role != "user"));
        // Each autopilot's first marker carries the prompt from its start
        // entry, backticks and all.
        let first = snapshot
            .turns
            .iter()
            .find_map(|turn| turn.autopilot_iteration.as_ref())
            .unwrap();
        assert_eq!(
            first.prompt.as_deref(),
            Some("Run `date +%s` with the bash tool and reply with only the number.")
        );
        for id in ["41eb2b26", "d1954ac9", "5eb22679"] {
            let marker = snapshot
                .turns
                .iter()
                .filter_map(|turn| turn.autopilot_iteration.as_ref())
                .find(|marker| marker.autopilot_id.to_string().starts_with(id))
                .unwrap();
            assert!(marker.prompt.as_deref().is_some_and(|p| p.contains("date +%s")));
        }
        // The replies are still there, between the markers.
        assert!(snapshot
            .turns
            .iter()
            .any(|turn| turn.autopilot_iteration.is_none() && turn.text == "1794594534"));

        // The same bytes read as Pi's have no markers: Pi has no extension.
        let pi = parse_pi_snapshot(OMP_AUTOPILOT_CAPTURE.as_bytes(), 7);
        assert!(pi
            .turns
            .iter()
            .all(|turn| turn.autopilot_iteration.is_none()));
    }

    #[test]
    fn only_a_journaled_agent_message_pairs_and_a_dropped_submission_pairs_with_nothing() {
        use autopilot_fixture::{start, Session, FIRST, SESSION};
        use serde_json::json;

        let limit = json!({ "kind": "at_most", "value": 5 });
        let iterations = |session: &Session| -> Vec<u64> {
            parse_pi_provider_snapshot(session.file().as_bytes(), 7, true)
                .turns
                .iter()
                .filter_map(|turn| turn.autopilot_iteration.as_ref())
                .map(|marker| marker.iteration)
                .collect()
        };

        // An agent-attributed message with nothing journaled before it is
        // some other extension's, and stays a message.
        let mut session = Session::new();
        session.user("agent", "Reply with exactly: pong");
        assert!(iterations(&session).is_empty());

        // "not submitted" cancels the entry it follows, so a later
        // agent-attributed message has nothing to pair with.
        session.journal(SESSION, FIRST, "start", start("go", limit));
        session.journal(SESSION, FIRST, "iteration", json!({ "iteration": 1 }));
        session.journal(SESSION, FIRST, "not_submitted", json!({ "iteration": 1 }));
        session.user("agent", "go");
        assert!(iterations(&session).is_empty());

        // One entry pairs with one message. A person's message with the very
        // text of the prompt is still a person's message.
        session.journal(SESSION, FIRST, "iteration", json!({ "iteration": 1 }));
        session.user("user", "go");
        session.user("agent", "something else entirely");
        session.user("agent", "go");
        assert_eq!(iterations(&session), [1]);
        let snapshot = parse_pi_provider_snapshot(session.file().as_bytes(), 7, true);
        let said: Vec<&str> = snapshot
            .turns
            .iter()
            .filter(|turn| turn.autopilot_iteration.is_none())
            .map(|turn| turn.text.as_str())
            .collect();
        assert_eq!(said, ["Reply with exactly: pong", "go", "go", "go"]);

        // A start entry whose spec this build cannot read still lets its
        // iterations be told apart; only the prompt and the limit are unknown.
        let mut newer = Session::new();
        let mut unknown = start("go", json!({ "kind": "none" }));
        unknown["spec"]["token_limit"] = json!(1000);
        newer.journal(SESSION, FIRST, "start", unknown);
        newer.journal(SESSION, FIRST, "iteration", json!({ "iteration": 1 }));
        newer.user("agent", "go");
        let snapshot = parse_pi_provider_snapshot(newer.file().as_bytes(), 7, true);
        let marker = snapshot.turns[0].autopilot_iteration.as_ref().unwrap();
        assert_eq!(
            (marker.iteration, marker.limit, &marker.prompt),
            (1, None, &None)
        );
        assert_eq!(snapshot.turns.len(), 1);
    }

    #[test]
    fn omp_native_ask_preserves_prose_usage_and_identity_without_enabling_pi() {
        let raw = omp_fixture_bytes(&omp_ask_fixture("Please choose first."));
        let snapshot = parse_pi_provider_snapshot(&raw, 7, true);
        assert_eq!(snapshot.session_id.as_deref(), Some("native-session"));
        assert_eq!(snapshot.activity, Some(TranscriptActivity::PendingAnswer));
        let question = snapshot
            .turns
            .iter()
            .find_map(|turn| turn.question.as_ref())
            .unwrap();
        assert_eq!(question.tool_use_id, "omp:7:native-session:call_ask");
        assert_eq!(question.input["questions"][0]["multiSelect"], true);
        assert_eq!(
            question.input["questions"][0]["options"][0]["label"],
            "A, B"
        );
        assert!(question.input["omp"]["dialog_id"].is_null());
        let prose = snapshot
            .turns
            .iter()
            .find(|turn| !turn.text.is_empty())
            .unwrap();
        assert_eq!(prose.text, "Please choose first.");
        assert_eq!(prose.input_tokens, Some(17));
        assert_eq!(prose.cost_usd, Some(0.25));
        assert!(prose.question.is_none());
        assert_eq!(
            snapshot
                .turns
                .iter()
                .filter(|turn| turn.has_usage())
                .count(),
            1
        );
        let pi = parse_pi_snapshot(&raw, 7);
        assert!(pi
            .turns
            .iter()
            .all(|turn| turn.question.is_none() && turn.answer.is_none()));
        assert_eq!(pi.turns.len(), 1);
    }

    #[test]
    fn omp_native_single_result_recovers_id_and_preserves_empty_multi_and_custom_text() {
        let mut entries = omp_ask_fixture("");
        entries.push(
            serde_json::json!({"type":"message","id":"result","parentId":"start",
            "message":{"role":"toolResult","toolCallId":"call_ask","toolName":"ask","isError":false,
                "content":[{"type":"text","text":"User chose none"}],
                "details":{"question":"Choose","options":["A, B","C"],"multi":true,
                    "selectedOptions":[],"customInput":"separate, exact text","timedOut":false}}}),
        );
        let snapshot = parse_pi_provider_snapshot(&omp_fixture_bytes(&entries), 7, true);
        let answer = snapshot
            .turns
            .iter()
            .find_map(|turn| turn.answer.as_ref())
            .unwrap();
        let result: Value = serde_json::from_str(&answer.recorded).unwrap();
        assert_eq!(result["status"], "answered");
        assert_eq!(result["results"][0]["id"], "choice");
        assert_eq!(
            result["results"][0]["selectedOptions"],
            serde_json::json!([])
        );
        assert_eq!(result["results"][0]["customInput"], "separate, exact text");
        assert!(!answer.is_error());
        assert_eq!(
            snapshot
                .turns
                .iter()
                .filter(|turn| turn.has_usage())
                .count(),
            1
        );
        assert_eq!(
            snapshot
                .turns
                .iter()
                .filter_map(|turn| turn.input_tokens)
                .sum::<u64>(),
            17
        );
    }

    #[test]
    fn omp_native_multiple_results_and_errors_are_not_inferred_from_submission() {
        let question = normalize_omp_question(
            2,
            "native",
            "ask",
            &[
                serde_json::json!({"id":"a","question":"Same","options":[]}),
                serde_json::json!({"id":"b","question":"Same","options":[]}),
            ],
        );
        let details = serde_json::json!({"results":[
            {"id":"a","question":"Same","options":["A, B"],"multi":false,"selectedOptions":["A, B"]},
            {"id":"b","question":"Same","options":[],"multi":true,"selectedOptions":[],"note":"none"}
        ]});
        let answer = normalize_omp_answer(
            &question,
            &serde_json::json!({
                "isError":false,"details":details,"content":[{"type":"text","text":"Native answer"}],
            }),
        );
        let result: Value = serde_json::from_str(&answer.recorded).unwrap();
        assert_eq!(result["results"], details["results"]);
        for (text, status) in [
            ("Ask tool was cancelled by the user", "cancelled"),
            ("Ask input was cancelled", "cancelled"),
            ("Provider failed", "unavailable"),
        ] {
            let answer = normalize_omp_answer(
                &question,
                &serde_json::json!({
                    "isError":true,"content":[{"type":"text","text":text}],
                }),
            );
            assert!(answer.is_error());
            let result: Value = serde_json::from_str(&answer.recorded).unwrap();
            assert_eq!(result["status"], status);
        }
    }

    #[test]
    fn omp_equal_length_branch_replacement_and_reader_provider_changes_are_observed() {
        let mut entries = omp_ask_fixture("Before");
        entries.push(serde_json::json!({"type":"message","id":"other","parentId":null,
            "message":{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"Other"}]}}));
        let snapshot = parse_pi_provider_snapshot(&omp_fixture_bytes(&entries), 1, true);
        assert!(snapshot.turns.iter().all(|turn| turn.question.is_none()));
        assert_eq!(snapshot.turns[0].text, "Other");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, omp_fixture_bytes(&omp_ask_fixture("Before"))).unwrap();
        let mut reader = PiTranscriptReader::default();
        assert!(reader
            .read(&path, 1)
            .unwrap()
            .turns
            .iter()
            .all(|turn| turn.question.is_none()));
        assert!(reader
            .read_omp(&path, 1)
            .unwrap()
            .turns
            .iter()
            .any(|turn| turn.question.is_some()));
        let rescaled = reader.read_omp(&path, 3).unwrap();
        assert_eq!(
            rescaled.turns[0].question.as_ref().unwrap().tool_use_id,
            "omp:3:native-session:call_ask"
        );
        let replacement = omp_fixture_bytes(&omp_ask_fixture("Afters"));
        assert_eq!(
            replacement.len(),
            std::fs::metadata(&path).unwrap().len() as usize
        );
        std::fs::write(&path, replacement).unwrap();
        assert_eq!(reader.read_omp(&path, 3).unwrap().turns[1].text, "Afters");
        std::fs::write(&path, omp_fixture_bytes(&entries)).unwrap();
        assert!(reader
            .read_omp(&path, 3)
            .unwrap()
            .turns
            .iter()
            .all(|turn| turn.question.is_none()));
    }

    /// Claude Code can move a session into one of its own worktrees, after
    /// which it writes under that directory's slug rather than the pane's cwd.
    /// A live pane hit this: its provider had cwd
    /// `…/mako/.claude/worktrees/masstree-rocks`, no file ever appeared where
    /// APAS was watching, and the pane showed an unrelated conversation
    /// instead. The session id is ours and names the file wherever it lands.
    #[test]
    fn a_relocated_session_is_found_by_its_id_in_another_directory() {
        let home = tempfile::tempdir().unwrap();
        let session = "11111111-2222-4333-8444-555555555555";
        let cwd = Path::new("/home/users/shuai/mako");

        // Nothing anywhere yet: the pinned path is still the answer, so a pane
        // that has not had its first turn watches where the file should appear.
        assert_eq!(
            locate_claude_transcript(home.path(), cwd, session),
            claude_transcript_path(home.path(), cwd, session),
        );

        // The provider relocated into a Claude worktree and wrote there.
        let moved_dir = claude_projects_dir(
            home.path(),
            "/home/users/shuai/mako/.claude/worktrees/masstree-rocks",
        );
        let moved = moved_dir.join(format!("{session}.jsonl"));
        std::fs::write(&moved, "{}").unwrap();

        assert_eq!(
            locate_claude_transcript(home.path(), cwd, session),
            moved,
            "the pane's own transcript is found where the provider actually wrote it"
        );
    }

    #[test]
    fn the_expected_location_wins_when_the_file_is_where_it_should_be() {
        let home = tempfile::tempdir().unwrap();
        let session = "11111111-2222-4333-8444-555555555555";
        let cwd = Path::new("/wanted");
        let dir = claude_projects_dir(home.path(), "/wanted");
        let expected = dir.join(format!("{session}.jsonl"));
        std::fs::write(&expected, "{}").unwrap();
        // A same-named file elsewhere must not win over the cwd slug.
        let other = claude_projects_dir(home.path(), "/elsewhere");
        std::fs::write(other.join(format!("{session}.jsonl")), "{}").unwrap();

        assert_eq!(
            locate_claude_transcript(home.path(), cwd, session),
            expected
        );
    }

    #[test]
    fn claude_transcript_path_is_exact_for_a_pinned_session() {
        // Verified against a live transcript: the directory is the absolute
        // cwd with every slash turned into a dash.
        let p = claude_transcript_path(
            Path::new("/home/u"),
            Path::new("/home/users/shuai/apas"),
            "11111111-2222-4333-8444-555555555555",
        );
        assert_eq!(
            p,
            Path::new(
                "/home/u/.claude/projects/-home-users-shuai-apas/11111111-2222-4333-8444-555555555555.jsonl"
            )
        );
    }

    #[test]
    fn claude_turns_carry_text_and_usage() {
        let raw = r#"
{"type":"user","timestamp":"2026-08-04T00:00:01Z","message":{"content":"what is 6x7?"}}
{"type":"assistant","timestamp":"2026-08-04T00:00:02Z","message":{"model":"claude-opus-5","content":[{"type":"text","text":"42"}],"stop_reason":"end_turn","usage":{"input_tokens":2,"output_tokens":3}}}
"#;
        let turns = parse_claude(raw);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[0].text, "what is 6x7?");
        assert_eq!(turns[1].text, "42");
        assert_eq!(turns[1].model.as_deref(), Some("claude-opus-5"));
        // Usage the agent would otherwise have had to volunteer.
        assert_eq!(turns[1].input_tokens, Some(2));
        assert_eq!(turns[1].output_tokens, Some(3));
        assert!(turns[1].completes_work);
    }

    #[test]
    fn claude_tool_preambles_do_not_complete_work() {
        let raw = r#"
{"type":"assistant","timestamp":"t1","message":{"content":[{"type":"text","text":"I will inspect that."}],"stop_reason":"tool_use"}}
{"type":"assistant","timestamp":"t2","message":{"content":[{"type":"text","text":"All done."}],"stop_reason":"end_turn"}}
"#;
        let turns = parse_claude(raw);
        assert_eq!(turns.len(), 2);
        assert!(!turns[0].completes_work);
        assert!(turns[1].completes_work);
    }

    #[test]
    fn claude_turn_duration_marks_a_previously_seen_reply_complete() {
        let before = r#"{"type":"assistant","timestamp":"t","message":{"content":"done"}}"#;
        assert!(!parse_claude(before)[0].completes_work);

        let after = format!(
            "{before}\n{{\"type\":\"system\",\"subtype\":\"turn_duration\",\"duration_ms\":10}}"
        );
        assert!(parse_claude(&after)[0].completes_work);
    }

    #[test]
    fn claude_bookkeeping_records_are_not_turns() {
        // The transcript also holds mode / ai-title / last-prompt entries.
        // Rendering those as conversation would be noise.
        let raw = r#"
{"type":"mode","timestamp":"t"}
{"type":"ai-title","timestamp":"t"}
{"type":"assistant","timestamp":"t","message":{"content":[{"type":"text","text":"real"}]}}
"#;
        let turns = parse_claude(raw);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].text, "real");
    }

    #[test]
    fn a_tool_only_turn_is_skipped_rather_than_recorded_blank() {
        let raw = r#"{"type":"assistant","timestamp":"t","message":{"content":[{"type":"tool_use","id":"1","name":"Bash","input":{}}]}}"#;
        assert!(parse_claude(raw).is_empty());
    }

    /// Shapes taken verbatim from a real transcript, not invented: a question
    /// is an assistant record whose only content is the tool call, and its
    /// answer is a tool_result on a user record.
    #[test]
    fn a_question_survives_with_its_options_and_its_recorded_answer() {
        let raw = concat!(
            r#"{"type":"assistant","timestamp":"t1","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","id":"toolu_1","name":"AskUserQuestion","input":{"questions":[{"question":"Pick a fruit","header":"Fruit","options":[{"label":"Apple"},{"label":"Banana"}]}]}}]}}"#,
            "\n",
            r#"{"type":"user","timestamp":"t2","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"The user answered: \"Pick a fruit\"=\"Banana\""}]}}"#,
        );
        let turns = parse_claude(raw);
        assert_eq!(turns.len(), 2, "question and answer are both turns");

        let question = turns[0].question.as_ref().expect("question kept");
        assert_eq!(question.tool_use_id, "toolu_1");
        assert_eq!(question.tool_name, "AskUserQuestion");
        let options = question.input["questions"][0]["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["label"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(options, vec!["Apple", "Banana"]);

        let answer = turns[1].answer.as_ref().expect("answer kept");
        assert_eq!(answer.tool_use_id, "toolu_1");
        assert!(answer.recorded.contains("Banana"));
    }

    #[test]
    fn an_unanswered_question_yields_no_answer_turn() {
        let raw = r#"{"type":"assistant","timestamp":"t","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","id":"toolu_1","name":"AskUserQuestion","input":{"questions":[]}}]}}"#;
        let turns = parse_claude(raw);
        assert_eq!(turns.len(), 1);
        assert!(turns[0].question.is_some());
        assert!(turns[0].answer.is_none());
    }

    /// A tool_result for something that was never a question is ordinary tool
    /// output, and stays out of the history.
    #[test]
    fn an_ordinary_tool_result_is_not_an_answer() {
        let raw = r#"{"type":"user","timestamp":"t","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_bash","content":"file listing"}]}}"#;
        assert!(parse_claude(raw).is_empty());
    }

    #[test]
    fn codex_turns_come_from_message_items_only() {
        let raw = r#"
{"type":"session_meta","payload":{"cwd":"/p"}}
{"type":"response_item","timestamp":"t1","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"<permissions instructions>"}]}}
{"type":"response_item","timestamp":"t2","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"do it"}]}}
{"type":"response_item","timestamp":"t3","payload":{"type":"reasoning","summary":[]}}
{"type":"response_item","timestamp":"t4","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}}
{"type":"response_item","timestamp":"t5","payload":{"type":"custom_tool_call","name":"shell"}}
{"type":"event_msg","timestamp":"t6","payload":{"type":"task_complete"}}
"#;
        let turns = parse_codex(raw);
        assert_eq!(
            turns.len(),
            2,
            "developer/reasoning/tool items are not turns"
        );
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[0].text, "do it");
        assert_eq!(turns[1].role, "assistant");
        assert_eq!(turns[1].text, "done");
        assert!(turns[1].completes_work);
    }

    #[test]
    fn codex_task_complete_can_arrive_after_the_assistant_turn() {
        let before = r#"{"type":"response_item","timestamp":"t","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}}"#;
        assert!(!parse_codex(before)[0].completes_work);

        let after = format!(
            "{before}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_complete\"}}}}"
        );
        assert!(parse_codex(&after)[0].completes_work);
    }

    #[test]
    fn codex_task_lifecycle_reconstructs_an_in_flight_turn() {
        let active = r#"
{"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-1"}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"still working"}]}}
"#;
        assert_eq!(codex_working_state(active), Some(true));

        let complete = format!(
            "{active}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"turn-1\"}}}}"
        );
        assert_eq!(codex_working_state(&complete), Some(false));

        let restarted = format!(
            "{complete}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\",\"turn_id\":\"turn-2\"}}}}"
        );
        assert_eq!(codex_working_state(&restarted), Some(true));

        let aborted = format!(
            "{restarted}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"turn_aborted\",\"turn_id\":\"turn-2\"}}}}"
        );
        assert_eq!(codex_working_state(&aborted), Some(false));
        assert_eq!(codex_working_state("{\"type\":\"session_meta\"}"), None);
    }

    #[test]
    fn opencode_export_keeps_only_real_completed_conversation() {
        let raw = r#"{
          "info":{"id":"ses_1","directory":"/work"},
          "messages":[
            {"info":{"role":"user","time":{"created":1786593600000},"model":{"modelID":"gpt-5"}},"parts":[
              {"type":"text","text":"fix it"},
              {"type":"text","text":"hidden","synthetic":true}
            ]},
            {"info":{"role":"assistant","time":{"created":1786593601000,"completed":1786593602000},"modelID":"gpt-5","finish":"tool-calls","tokens":{"input":12,"output":3}},"parts":[
              {"type":"text","text":"I will inspect it."},
              {"type":"reasoning","text":"private chain"}
            ]},
            {"info":{"role":"assistant","time":{"created":1786593603000},"modelID":"gpt-5"},"parts":[
              {"type":"text","text":"partial response"}
            ]},
            {"info":{"role":"assistant","time":{"created":1786593603000,"completed":1786593604000},"modelID":"gpt-5","finish":"stop","tokens":{"input":20,"output":8}},"parts":[
              {"type":"text","text":"Fixed and tested."},
              {"type":"tool","state":{"status":"completed"}}
            ]}
          ]
        }"#;

        let turns = parse_opencode(raw);
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[0].text, "fix it");
        assert_eq!(turns[1].text, "I will inspect it.");
        assert!(!turns[1].completes_work, "tool-call preamble is not idle");
        assert_eq!(turns[2].text, "Fixed and tested.");
        assert_eq!(turns[2].model.as_deref(), Some("gpt-5"));
        assert_eq!(turns[2].input_tokens, Some(20));
        assert_eq!(turns[2].output_tokens, Some(8));
        assert!(turns[2].completes_work);
        assert!(turns.iter().all(|turn| !turn.text.contains("partial")));
        assert!(turns.iter().all(|turn| !turn.text.contains("private")));
    }

    #[test]
    fn opencode_session_selection_is_newest_and_cwd_scoped() {
        let root = tempfile::tempdir().unwrap();
        let wanted = root.path().join("wanted");
        let other = root.path().join("other");
        std::fs::create_dir_all(&wanted).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let raw = serde_json::json!([
            {"id":"ses_old","directory":wanted,"updated":1},
            {"id":"ses_other","directory":other,"updated":999},
            {"id":"ses_new","directory":wanted,"updated":2}
        ])
        .to_string();

        assert_eq!(
            parse_opencode_session_list(&raw, &wanted).as_deref(),
            Some("ses_new")
        );
        assert!(parse_opencode_session_list(&raw, &root.path().join("missing")).is_none());
    }

    #[test]
    fn developer_messages_never_leak_into_history() {
        // These are the harness's own injected context, not something the
        // human or the agent said — showing them would be confusing and long.
        let raw = r#"{"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"secret harness prompt"}]}}"#;
        assert!(parse_codex(raw).is_empty());
    }

    #[test]
    fn malformed_lines_do_not_cost_the_rest_of_the_history() {
        // A transcript read while the provider is mid-write ends in a partial
        // line; losing the whole conversation over it would be absurd.
        let raw = "{not json\n{\"type\":\"assistant\",\"timestamp\":\"t\",\"message\":{\"content\":\"kept\"}}\n{\"typ";
        let turns = parse_claude(raw);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].text, "kept");
    }

    #[test]
    fn read_turns_stamps_the_owning_pane() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"assistant","timestamp":"t","message":{"content":"hi"}}"#,
        )
        .unwrap();
        let turns = read_turns(&path, 77, false).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].pane_id, 77);
    }

    #[test]
    fn a_missing_transcript_reads_empty_rather_than_failing() {
        // Normal before the agent's first turn.
        let dir = tempfile::tempdir().unwrap();
        assert!(read_turns(&dir.path().join("nope.jsonl"), 1, false)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn codex_rollout_lookup_matches_on_cwd() {
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join(".codex/sessions/2026/08/04");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(
            sessions.join("rollout-a.jsonl"),
            r#"{"type":"session_meta","payload":{"cwd":"/other","id":"a"}}"#,
        )
        .unwrap();
        std::fs::write(
            sessions.join("rollout-b.jsonl"),
            r#"{"type":"session_meta","payload":{"cwd":"/wanted","id":"b"}}"#,
        )
        .unwrap();

        let found = find_codex_rollout(home.path(), Path::new("/wanted"));
        assert_eq!(
            found
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str()),
            Some("rollout-b.jsonl")
        );
        assert!(find_codex_rollout(home.path(), Path::new("/nothing-here")).is_none());
    }

    #[test]
    fn codex_rollout_identity_is_read_and_found_without_trusting_random_pane_ids() {
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join(".codex/sessions/2026/08/22");
        std::fs::create_dir_all(&sessions).unwrap();
        let actual = Uuid::parse_str("11111111-2222-4333-8444-555555555555").unwrap();
        let rollout = sessions.join("rollout-user.jsonl");
        std::fs::write(
            &rollout,
            format!(
                r#"{{"type":"session_meta","payload":{{"cwd":"/repo","id":"{actual}","thread_source":"user"}}}}"#
            ),
        )
        .unwrap();

        assert_eq!(codex_rollout_session_id(&rollout), Some(actual));
        assert!(codex_user_session_exists(home.path(), actual));
        assert!(!codex_user_session_exists(home.path(), Uuid::new_v4()));
    }

    #[test]
    fn a_subagent_rollout_is_not_a_resumable_pane_identity() {
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join(".codex/sessions/2026/08/22");
        std::fs::create_dir_all(&sessions).unwrap();
        let subagent = Uuid::parse_str("aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee").unwrap();
        std::fs::write(
            sessions.join("rollout-subagent.jsonl"),
            format!(
                r#"{{"type":"session_meta","payload":{{"cwd":"/repo","id":"{subagent}","thread_source":"subagent"}}}}"#
            ),
        )
        .unwrap();

        assert!(!codex_user_session_exists(home.path(), subagent));

        let modern_subagent = Uuid::parse_str("bbbbbbbb-cccc-4ddd-8eee-ffffffffffff").unwrap();
        std::fs::write(
            sessions.join("rollout-modern-subagent.jsonl"),
            format!(
                r#"{{"type":"session_meta","payload":{{"cwd":"/repo","id":"{modern_subagent}","source":{{"subagent":{{"thread_spawn":{{"parent_thread_id":"parent"}}}}}},"agent_path":"/root/audit"}}}}"#
            ),
        )
        .unwrap();

        assert!(!codex_user_session_exists(home.path(), modern_subagent));
    }

    /// Write a rollout header; `source` of `None` mimics a codex old enough to
    /// predate the `thread_source` field.
    fn write_rollout(dir: &Path, name: &str, cwd: &str, started_at: &str, source: Option<&str>) {
        let thread_source = match source {
            Some(s) => format!(r#","thread_source":"{s}""#),
            None => String::new(),
        };
        std::fs::write(
            dir.join(name),
            format!(
                r#"{{"type":"session_meta","payload":{{"cwd":"{cwd}","timestamp":"{started_at}"{thread_source}}}}}"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn codex_rollout_lookup_ignores_subagent_threads_in_the_same_cwd() {
        // Codex spawns its own threads and each writes a rollout carrying the
        // *parent's* cwd. Following one means tailing a transcript that is not
        // the pane's, and alternating between them republishes history.
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join(".codex/sessions/2026/08/13");
        std::fs::create_dir_all(&sessions).unwrap();
        write_rollout(
            &sessions,
            "rollout-user.jsonl",
            "/repo",
            "2026-08-08T00:09:14Z",
            Some("user"),
        );
        // Started later than the user's session — under the old newest-wins
        // rule these would win outright.
        write_rollout(
            &sessions,
            "rollout-sub-1.jsonl",
            "/repo",
            "2026-08-13T18:56:57Z",
            Some("subagent"),
        );
        write_rollout(
            &sessions,
            "rollout-sub-2.jsonl",
            "/repo",
            "2026-08-14T00:33:34Z",
            Some("subagent"),
        );
        std::fs::write(
            sessions.join("rollout-modern-sub.jsonl"),
            r#"{"type":"session_meta","payload":{"cwd":"/repo","timestamp":"2026-08-15T00:00:00Z","source":{"subagent":{"thread_spawn":{"parent_thread_id":"root"}}},"agent_path":"/root/audit"}}"#,
        )
        .unwrap();

        assert_eq!(
            find_codex_rollout(home.path(), Path::new("/repo"))
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str()),
            Some("rollout-user.jsonl")
        );
    }

    #[test]
    fn codex_rollout_lookup_is_stable_while_siblings_are_appended_to() {
        // The selection must not depend on mtime: sibling rollouts in one cwd
        // are written concurrently, so whichever was touched last changes from
        // poll to poll, and every change looked like a brand-new transcript.
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join(".codex/sessions/2026/08/13");
        std::fs::create_dir_all(&sessions).unwrap();
        write_rollout(
            &sessions,
            "rollout-old.jsonl",
            "/repo",
            "2026-08-08T00:00:00Z",
            Some("user"),
        );
        write_rollout(
            &sessions,
            "rollout-new.jsonl",
            "/repo",
            "2026-08-09T00:00:00Z",
            Some("user"),
        );

        let first = find_codex_rollout(home.path(), Path::new("/repo"));
        // Touch the older file, exactly as a concurrent append would.
        std::fs::write(
            sessions.join("rollout-old.jsonl"),
            std::fs::read(sessions.join("rollout-old.jsonl")).unwrap(),
        )
        .unwrap();
        let second = find_codex_rollout(home.path(), Path::new("/repo"));

        assert_eq!(
            first, second,
            "selection flapped after a sibling was written"
        );
        assert_eq!(
            first
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str()),
            Some("rollout-new.jsonl"),
            "the newest user session should still win"
        );
    }

    #[test]
    fn codex_rollout_lookup_keeps_pre_thread_source_rollouts() {
        // Older codex writes no thread_source. Those are real user sessions and
        // must not be filtered out as subagents.
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join(".codex/sessions/2026/08/13");
        std::fs::create_dir_all(&sessions).unwrap();
        write_rollout(
            &sessions,
            "rollout-legacy.jsonl",
            "/repo",
            "2026-08-08T00:00:00Z",
            None,
        );

        assert!(find_codex_rollout(home.path(), Path::new("/repo")).is_some());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn codex_process_group_lookup_does_not_adopt_a_newer_sibling_panes_rollout() {
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join(".codex/sessions/2026/08/22");
        std::fs::create_dir_all(&sessions).unwrap();
        write_rollout(
            &sessions,
            "rollout-owned.jsonl",
            "/repo",
            "2026-08-22T01:00:00Z",
            Some("user"),
        );
        write_rollout(
            &sessions,
            "rollout-newer-sibling.jsonl",
            "/repo",
            "2026-08-22T02:00:00Z",
            Some("user"),
        );
        std::fs::write(
            sessions.join("rollout-owned-modern-subagent.jsonl"),
            r#"{"type":"session_meta","payload":{"cwd":"/repo","timestamp":"2026-08-22T03:00:00Z","source":{"subagent":{"thread_spawn":{"parent_thread_id":"root"}}},"agent_path":"/root/audit"}}"#,
        )
        .unwrap();

        // Holding this descriptor open models the live codex TUI. The newer
        // sibling file exists in the same cwd but belongs to another pane and
        // therefore is not open in this terminal's process group.
        let owned = std::fs::File::open(sessions.join("rollout-owned.jsonl")).unwrap();
        // Subagents share the root TUI's process group and keep their rollout
        // open too, so process ownership alone is not sufficient.
        let modern_subagent =
            std::fs::File::open(sessions.join("rollout-owned-modern-subagent.jsonl")).unwrap();
        let found = find_codex_rollout_for_process_group(home.path(), Path::new("/repo"), unsafe {
            libc::getpgrp()
        });
        drop(owned);
        drop(modern_subagent);

        assert_eq!(
            found
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str()),
            Some("rollout-owned.jsonl")
        );
    }

    #[cfg(target_os = "linux")]
    fn fake_codex_process(
        proc_root: &Path,
        pid: u32,
        parent: u32,
        group: u32,
        command: &[u8],
        rollouts: &[PathBuf],
    ) {
        let dir = proc_root.join(pid.to_string());
        std::fs::create_dir_all(dir.join("fd")).unwrap();
        // Include ')' and spaces in comm so the stat parser cannot assume
        // whitespace-separated fields until after the final ')'.
        std::fs::write(
            dir.join("stat"),
            format!("{pid} (codex ) child) S {parent} {group} 0"),
        )
        .unwrap();
        std::fs::write(dir.join("cmdline"), command).unwrap();
        for (fd, rollout) in rollouts.iter().enumerate() {
            std::os::unix::fs::symlink(rollout, dir.join("fd").join(fd.to_string())).unwrap();
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn codex_rollout_lookup_follows_its_app_server_child_in_a_separate_group() {
        let home = tempfile::tempdir().unwrap();
        let proc_root = home.path().join("proc");
        let sessions = home.path().join(".codex/sessions/2026/09/25");
        std::fs::create_dir_all(&sessions).unwrap();
        for (name, time) in [
            ("owned", "10:00"),
            ("sibling", "11:00"),
            ("nested", "12:00"),
        ] {
            write_rollout(
                &sessions,
                &format!("{name}.jsonl"),
                "/repo",
                &format!("2026-09-25T{time}:00Z"),
                Some("user"),
            );
        }
        let owned = sessions.join("owned.jsonl");
        let sibling = sessions.join("sibling.jsonl");
        let nested = sessions.join("nested.jsonl");
        let subagent = sessions.join("subagent.jsonl");
        std::fs::write(&subagent, r#"{"type":"session_meta","payload":{"cwd":"/repo","timestamp":"2026-09-25T13:00:00Z","source":{"subagent":{}},"agent_path":"/root/audit"}}"#).unwrap();

        fake_codex_process(&proc_root, 100, 1, 100, b"/bin/codex\0resume\0", &[]);
        fake_codex_process(
            &proc_root,
            101,
            100,
            101,
            b"/releases/codex\0app-server\0--managed-daemon\0",
            &[owned.clone(), subagent],
        );
        // Another pane's newer session is in the same directory.
        fake_codex_process(&proc_root, 200, 1, 200, b"/bin/codex\0resume\0", &[]);
        fake_codex_process(
            &proc_root,
            201,
            200,
            201,
            b"/releases/codex\0app-server\0",
            &[sibling.clone()],
        );
        // A tool launched another Codex; following all descendants would
        // incorrectly select this newest user session.
        fake_codex_process(
            &proc_root,
            102,
            101,
            102,
            b"/bin/codex\0app-server\0",
            &[nested.clone()],
        );
        fake_codex_process(&proc_root, 103, 100, 103, b"/bin/codex\0exec\0", &[nested]);

        assert_eq!(
            find_codex_rollout_in_proc(home.path(), Path::new("/repo"), 100, &proc_root),
            Some(owned.clone())
        );
        assert_eq!(
            find_codex_rollout_in_proc(home.path(), Path::new("/repo"), 200, &proc_root),
            Some(sibling)
        );
        assert!(
            find_codex_rollout_in_proc(home.path(), Path::new("/different"), 100, &proc_root)
                .is_none()
        );

        // The local executable cache renames native Codex. Only the exact
        // parent executable qualifies; an arbitrary app-server child does not.
        std::fs::write(
            proc_root.join("101/cmdline"),
            b"/cache/bin-hash\0app-server\0",
        )
        .unwrap();
        std::os::unix::fs::symlink("/cache/bin-hash", proc_root.join("100/exe")).unwrap();
        std::os::unix::fs::symlink("/cache/bin-hash", proc_root.join("101/exe")).unwrap();
        assert_eq!(
            find_codex_rollout_in_proc(home.path(), Path::new("/repo"), 100, &proc_root),
            Some(owned)
        );
        std::fs::remove_file(proc_root.join("101/exe")).unwrap();
        std::os::unix::fs::symlink("/other/app-server", proc_root.join("101/exe")).unwrap();
        assert!(
            find_codex_rollout_in_proc(home.path(), Path::new("/repo"), 100, &proc_root).is_none()
        );

        std::fs::remove_dir_all(proc_root.join("101")).unwrap();
        assert!(
            find_codex_rollout_in_proc(home.path(), Path::new("/repo"), 100, &proc_root).is_none(),
            "never fall back to a sibling or tool's session when the app-server disappears"
        );
    }

    fn claude_projects_dir(home: &Path, cwd: &str) -> PathBuf {
        let dir = home
            .join(".claude")
            .join("projects")
            .join(claude_dir_slug(Path::new(cwd)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_session(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), "{}").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    /// A brand-new pane watches a transcript Claude has not written yet, so it
    /// has no mtime of its own. Passing the epoch as the floor made every older
    /// conversation in the directory a valid switch target, and a new pane
    /// silently adopted the most recently touched one — arriving with a
    /// stranger's messages before its terminal had produced a byte. The floor
    /// is the moment the pane started watching, so anything that predates the
    /// pane cannot be something the pane switched to.
    #[test]
    fn a_pane_whose_transcript_does_not_exist_yet_adopts_nothing_older_than_itself() {
        let home = tempfile::tempdir().unwrap();
        let dir = claude_projects_dir(home.path(), "/wanted");
        // Conversations that were already in this directory, one of them large
        // and recently touched — exactly the shape that was adopted.
        write_session(&dir, "old-conversation.jsonl");
        write_session(&dir, "recently-touched.jsonl");

        // The floor as the caller computes it: the newest transcript already
        // present. Taken from the filesystem, never the clock — these live on
        // NFS, and mtime-versus-now is a cross-domain comparison.
        let pane_started = std::fs::metadata(dir.join("recently-touched.jsonl"))
            .unwrap()
            .modified()
            .unwrap();
        // The pane's own session id: pinned, and no file behind it yet.
        let pinned = dir.join("brand-new-pane.jsonl");
        assert!(!pinned.exists(), "the pane has not had a turn yet");

        assert!(
            find_claude_switch_candidate(&pinned, pane_started, &HashSet::new(),).is_none(),
            "a pane must not adopt a conversation that predates it"
        );

        // A genuine in-TUI switch after the pane started is still followed.
        write_session(&dir, "switched-to.jsonl");
        assert_eq!(
            find_claude_switch_candidate(&pinned, pane_started, &HashSet::new(),)
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str()),
            Some("switched-to.jsonl"),
        );
    }

    #[test]
    fn claude_switch_candidate_follows_the_newest_unpinned_file() {
        let home = tempfile::tempdir().unwrap();
        let dir = claude_projects_dir(home.path(), "/wanted");
        write_session(&dir, "stale-unpinned.jsonl");
        write_session(&dir, "pinned.jsonl");
        let pinned = dir.join("pinned.jsonl");
        let last_growth = std::fs::metadata(&pinned).unwrap().modified().unwrap();
        // Another pane's pinned session: newer, but excluded.
        write_session(&dir, "other-pane.jsonl");
        write_session(&dir, "switch.jsonl");
        write_session(&dir, "newest.jsonl");

        let excluded: HashSet<String> = ["other-pane".to_string()].into_iter().collect();
        let found = find_claude_switch_candidate(&pinned, last_growth, &excluded);
        assert_eq!(
            found
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str()),
            Some("newest.jsonl"),
            "the newest unpinned session wins; other panes' pins are ignored"
        );
    }

    #[test]
    fn claude_switch_candidate_ignores_stale_and_excluded_files() {
        let home = tempfile::tempdir().unwrap();
        let dir = claude_projects_dir(home.path(), "/wanted");
        write_session(&dir, "pinned.jsonl");
        let pinned = dir.join("pinned.jsonl");
        let last_growth = std::fs::metadata(&pinned).unwrap().modified().unwrap();
        // Excluded ids stay excluded even when newer.
        write_session(&dir, "other-pane.jsonl");

        let excluded: HashSet<String> = ["other-pane".to_string()].into_iter().collect();
        assert!(find_claude_switch_candidate(&pinned, last_growth, &excluded,).is_none());
    }

    #[test]
    fn claude_switch_candidate_follows_back_to_the_own_pinned_file() {
        let home = tempfile::tempdir().unwrap();
        let dir = claude_projects_dir(home.path(), "/wanted");
        write_session(&dir, "pinned.jsonl");
        write_session(&dir, "switched-away.jsonl");
        let current = dir.join("switched-away.jsonl");
        let last_growth = std::fs::metadata(&current).unwrap().modified().unwrap();
        // The user resumes the pinned session again after the switch.
        write_session(&dir, "pinned.jsonl");

        // Only OTHER panes' pins are excluded; the pane's own pinned file is
        // eligible, so the switch back is followed.
        let excluded: HashSet<String> = ["other-pane".to_string()].into_iter().collect();
        let found = find_claude_switch_candidate(&current, last_growth, &excluded);
        assert_eq!(
            found
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str()),
            Some("pinned.jsonl")
        );
    }

    #[test]
    fn claude_switch_candidate_never_returns_the_current_file() {
        let home = tempfile::tempdir().unwrap();
        let dir = claude_projects_dir(home.path(), "/wanted");
        write_session(&dir, "pinned.jsonl");
        let pinned = dir.join("pinned.jsonl");
        let last_growth = std::fs::metadata(&pinned).unwrap().modified().unwrap();
        let excluded: HashSet<String> = HashSet::new();
        assert!(find_claude_switch_candidate(&pinned, last_growth, &excluded,).is_none());
    }

    // --- pi ---------------------------------------------------------------

    /// A real omp 18.1.20 session, trimmed. Two turns written across a
    /// `--continue` boundary, so the parent chain crosses the resume and the
    /// `custom` session_exit record sits between them.
    const OMP_REAL_SESSION: &str = concat!(
        r#"{"type":"session","version":3,"id":"01a09e12-5973-77d9-aa66-861b4273e940","timestamp":"2026-09-14T03:59:57.811Z","cwd":"/tmp/work"}"#,
        "\n",
        r#"{"type":"message","id":"49dfb5bf","parentId":"e9131629","timestamp":"2026-09-14T03:59:59.252Z","message":{"role":"user","content":[{"type":"text","text":"Remember the word: ALBATROSS. Reply OK."}],"timestamp":1789358399252}}"#,
        "\n",
        r#"{"type":"message","id":"8be2a596","parentId":"49dfb5bf","timestamp":"2026-09-14T04:00:17.992Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":""},{"type":"text","text":"OK"}],"model":"muse-spark-1.3-contributor","usage":{"input":17820,"output":19,"totalTokens":17839},"completedAt":1789358417992,"timestamp":1789358417992}}"#,
        "\n",
        r#"{"type":"custom","customType":"session_exit","data":{"reason":"dispose","kind":"normal"},"id":"97a25f4a","parentId":"8be2a596","timestamp":"2026-09-14T04:00:18.027Z"}"#,
        "\n",
        r#"{"type":"message","id":"76ed4686","parentId":"97a25f4a","timestamp":"2026-09-14T04:00:28.836Z","message":{"role":"user","content":[{"type":"text","text":"What word did I ask you to remember? One word."}],"timestamp":1789358428836}}"#,
        "\n",
        r#"{"type":"message","id":"1863543b","parentId":"76ed4686","timestamp":"2026-09-14T04:00:31.986Z","message":{"role":"assistant","content":[{"type":"text","text":"ALBATROSS"}],"model":"muse-spark-1.3-contributor","usage":{"input":18010,"output":7,"totalTokens":18017},"completedAt":1789358431986,"timestamp":1789358431986}}"#,
        "\n",
    );

    #[test]
    fn omp_sessions_parse_with_the_pi_reader() {
        // OMP is Pi-derived and writes the same records, which is why it needs
        // no reader of its own. This asserts that against a real capture
        // rather than an assumption: if OMP ever diverges, this fails instead
        // of the conversation view silently going empty.
        let snapshot = parse_pi_snapshot(OMP_REAL_SESSION.as_bytes(), 0);
        assert_eq!(snapshot.activity, Some(TranscriptActivity::Idle));
        let turns = snapshot.turns;
        let roles: Vec<&str> = turns.iter().map(|t| t.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant", "user", "assistant"]);
        assert_eq!(turns[0].text, "Remember the word: ALBATROSS. Reply OK.");
        assert_eq!(turns[3].text, "ALBATROSS");

        // The second exchange is only reachable through the `custom`
        // session_exit record that `--continue` wrote between them, so this
        // also pins that a resumed OMP pane keeps its earlier history.
        assert!(
            turns.len() == 4,
            "the parent chain must survive the --continue boundary"
        );

        // Empty thinking blocks are not turns, and usage rides the assistant.
        assert!(turns.iter().all(|t| !t.text.trim().is_empty()));
        assert!(turns[1].model.as_deref() == Some("muse-spark-1.3-contributor"));
    }

    #[test]
    fn omp_session_directories_are_private_to_one_pane() {
        let home = std::path::Path::new("/home/someone");
        let a = Uuid::parse_str("11111111-2222-4333-8444-555555555555").unwrap();
        let b = Uuid::parse_str("11111111-2222-4333-8444-666666666666").unwrap();
        assert_ne!(omp_session_dir(home, a), omp_session_dir(home, b));
        // Under OMP's own store, not Pi's, so relocating one never silently
        // redirects the other.
        assert!(omp_session_dir(home, a).starts_with("/home/someone/.omp/agent/sessions"));
        assert!(omp_session_dir(home, a).ends_with(format!("apas-{a}")));
    }

    fn pi_message(id: &str, parent: Option<&str>, message: serde_json::Value) -> String {
        let parent = parent
            .map(|parent| format!("\"{parent}\""))
            .unwrap_or_else(|| "null".to_string());
        format!(
            r#"{{"type":"message","id":"{id}","parentId":{parent},"timestamp":"2024-12-03T14:00:00.000Z","message":{}}}"#,
            serde_json::to_string(&message).unwrap()
        )
    }

    fn pi_user(id: &str, parent: Option<&str>, text: &str) -> String {
        pi_message(
            id,
            parent,
            serde_json::json!({ "role": "user", "content": text }),
        )
    }

    fn pi_assistant(
        id: &str,
        parent: Option<&str>,
        text: &str,
        stop_reason: &str,
        usage: Option<serde_json::Value>,
    ) -> String {
        let mut message = serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": text}],
            "provider": "anthropic",
            "model": "claude-sonnet-4-5",
            "stopReason": stop_reason,
        });
        if let Some(usage) = usage {
            message["usage"] = usage;
        }
        pi_message(id, parent, message)
    }

    fn pi_session(lines: &[String]) -> String {
        let mut out = vec![
            r#"{"type":"session","version":3,"id":"22222222-3333-4444-8555-666666666666","timestamp":"2024-12-03T14:00:00.000Z","cwd":"/wanted"}"#.to_string(),
        ];
        out.extend_from_slice(lines);
        out.join("\n")
    }

    #[test]
    fn pi_reads_only_the_active_branch_of_a_session_tree() {
        // `/tree` appends entries whose parentId points back into history, so
        // file order alone would fold an abandoned branch into the conversation.
        let raw = pi_session(&[
            pi_user("u1", None, "Hello"),
            pi_assistant("a1", Some("u1"), "First answer", "stop", None),
            pi_user("u2", Some("a1"), "Branch seed"),
            pi_assistant("a2", Some("u2"), "Abandoned reply", "stop", None),
            pi_user("u3", Some("u2"), "Kept follow-up"),
            pi_assistant("a3", Some("u3"), "Kept reply", "stop", None),
        ]);

        let turns = parse_pi_snapshot(raw.as_bytes(), 0).turns;
        let texts: Vec<&str> = turns.iter().map(|turn| turn.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "Hello",
                "First answer",
                "Branch seed",
                "Kept follow-up",
                "Kept reply"
            ]
        );
        assert!(!texts.contains(&"Abandoned reply"));
    }

    #[test]
    fn pi_extension_and_bookkeeping_entries_never_become_turns() {
        // The oh-my-pi orchestrator persists `custom` / `custom_message`
        // entries; rendering those as conversation is the failure this guards.
        let raw = pi_session(&[
            r#"{"type":"model_change","id":"m1","parentId":null,"timestamp":"t","provider":"openai","modelId":"gpt-4o"}"#.to_string(),
            pi_user("u1", Some("m1"), "Do the thing"),
            r#"{"type":"custom","id":"c1","parentId":"u1","timestamp":"t","customType":"oh-my-pi","data":{"mode":"orchestrator"}}"#.to_string(),
            r#"{"type":"custom_message","id":"c2","parentId":"c1","timestamp":"t","customType":"oh-my-pi","content":"Injected context","display":true}"#.to_string(),
            pi_assistant("a1", Some("c2"), "Working on it", "toolUse", None),
            pi_message(
                "t1",
                Some("a1"),
                serde_json::json!({
                    "role": "toolResult",
                    "toolCallId": "call_1",
                    "toolName": "bash",
                    "content": [{"type": "text", "text": "some output"}],
                    "isError": false,
                }),
            ),
            r#"{"type":"compaction","id":"k1","parentId":"t1","timestamp":"t","summary":"Earlier work","firstKeptEntryId":"u1","tokensBefore":1000}"#.to_string(),
            r#"{"type":"branch_summary","id":"b1","parentId":"k1","timestamp":"t","fromId":null,"summary":"Another branch"}"#.to_string(),
            r#"{"type":"label","id":"l1","parentId":"b1","timestamp":"t","targetId":"u1","label":"checkpoint"}"#.to_string(),
            r#"{"type":"session_info","id":"s1","parentId":"l1","timestamp":"t","name":"Refactor"}"#.to_string(),
            pi_assistant("a2", Some("s1"), "Done", "stop", None),
        ]);

        let turns = parse_pi_snapshot(raw.as_bytes(), 0).turns;
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[1].text, "Working on it");
        assert_eq!(turns[2].text, "Done");
        assert!(turns.iter().all(|turn| turn.question.is_none()));
        assert!(turns.iter().all(|turn| turn.answer.is_none()));
    }

    #[test]
    fn pi_completion_and_billing_follow_the_recorded_stop_reason() {
        let raw = pi_session(&[
            pi_user("u1", None, "Run it"),
            pi_assistant(
                "a1",
                Some("u1"),
                "Starting",
                "toolUse",
                Some(serde_json::json!({
                    "input": 120,
                    "output": 15,
                    "totalTokens": 135,
                    "cost": {"total": 0.0021},
                })),
            ),
            pi_assistant(
                "a2",
                Some("a1"),
                "Finished",
                "stop",
                Some(serde_json::json!({
                    "input": 200,
                    "output": 30,
                    "totalTokens": 230,
                    "cost": {"total": 0.0042},
                })),
            ),
        ]);

        let turns = parse_pi_snapshot(raw.as_bytes(), 0).turns;
        let working = &turns[1];
        assert!(!working.completes_work, "a tool-use preamble is not idle");
        assert_eq!(working.input_tokens, Some(120));
        assert_eq!(working.output_tokens, Some(15));
        assert_eq!(working.cost_usd, Some(0.0021));
        assert_eq!(working.model.as_deref(), Some("claude-sonnet-4-5"));
        let done = &turns[2];
        assert!(done.completes_work);
        assert_eq!(done.cost_usd, Some(0.0042));
    }

    #[test]
    fn pi_silent_completion_carries_to_the_previous_assistant_turn() {
        // A response without visible text can still complete the work, but
        // this is not true of a response carrying executable tool calls.
        let raw = pi_session(&[
            pi_user("u1", None, "Check the logs"),
            pi_assistant("a1", Some("u1"), "On it", "toolUse", None),
            pi_assistant("a2", Some("a1"), "", "stop", None),
        ]);

        let turns = parse_pi_snapshot(raw.as_bytes(), 0).turns;
        assert_eq!(turns.len(), 2);
        assert!(turns[1].completes_work);
    }

    #[test]
    fn omp_activity_recovers_after_abort_and_tracks_only_started_unanswered_asks() {
        use TranscriptActivity::{Idle, PendingAnswer, Working};

        let stages = [
            (pi_user("u1", None, "Run it"), Working),
            (pi_assistant("a1", Some("u1"), "Interrupted", "aborted", None), Idle),
            (r#"{"type":"custom_message","id":"c1","parentId":"a1","customType":"launch-completion","content":"Background process finished"}"#.to_string(), Working),
            (pi_message("a2", Some("c1"), serde_json::json!({
                "role": "assistant", "stopReason": "stop",
                "content": [{"type": "toolCall", "id": "bash1", "name": "bash", "arguments": {}}]
            })), Working),
            (r#"{"type":"custom","id":"s1","parentId":"a2","customType":"tool_execution_start","data":{"toolCallId":"bash1","toolName":"bash"}}"#.to_string(), Working),
            (pi_message("r1", Some("s1"), serde_json::json!({
                "role": "toolResult", "toolCallId": "bash1", "toolName": "bash", "content": []
            })), Working),
            (pi_message("a3", Some("r1"), serde_json::json!({
                "role": "assistant", "stopReason": "toolUse",
                "content": [{"type": "toolCall", "id": "ask1", "name": "ask", "arguments": {}}]
            })), Working),
            (r#"{"type":"custom","id":"s2","parentId":"a3","customType":"tool_execution_start","data":{"toolCallId":"ask1","toolName":"ask"}}"#.to_string(), PendingAnswer),
            (r#"{"type":"custom_message","id":"c2","parentId":"s2","customType":"async-result","content":"Unrelated job finished"}"#.to_string(), PendingAnswer),
            (pi_message("r2", Some("c2"), serde_json::json!({
                "role": "toolResult", "toolCallId": "other", "toolName": "bash", "content": []
            })), PendingAnswer),
            (pi_message("r3", Some("r2"), serde_json::json!({
                "role": "toolResult", "toolCallId": "ask1", "toolName": "ask",
                "content": [{"type": "text", "text": "The recorded answer"}]
            })), Working),
            (pi_assistant("a4", Some("r3"), "Done", "stop", None), Idle),
        ];
        let mut raw = String::new();
        for (entry, expected) in stages {
            raw.push_str(&entry);
            raw.push('\n');
            assert_eq!(
                parse_pi_snapshot(raw.as_bytes(), 131).activity,
                Some(expected)
            );
        }
        let turns = parse_pi_snapshot(raw.as_bytes(), 131).turns;
        assert_eq!(
            turns
                .iter()
                .map(|turn| turn.text.as_str())
                .collect::<Vec<_>>(),
            ["Run it", "Interrupted", "Done"]
        );
        assert!(turns
            .iter()
            .all(|turn| turn.question.is_none() && turn.answer.is_none()));
    }

    #[test]
    fn omp_executable_stop_and_pause_turn_do_not_complete_visible_work() {
        for message in [
            serde_json::json!({
                "role": "assistant", "stopReason": "stop",
                "content": [{"type": "toolCall", "id": "call1", "name": "bash", "arguments": {}}]
            }),
            serde_json::json!({
                "role": "assistant", "stopReason": "stop", "stopDetails": {"type": "pause_turn"},
                "content": [{"type": "thinking", "thinking": "continue"}]
            }),
            serde_json::json!({
                "role": "assistant", "stopReason": "length",
                "content": [{"type": "toolCall", "id": "call1", "name": "bash", "arguments": {}}]
            }),
            serde_json::json!({
                "role": "assistant", "completedAt": 1789358431986u64,
                "content": [{"type": "toolCall", "id": "call1", "name": "bash", "arguments": {}}]
            }),
        ] {
            let raw = pi_session(&[
                pi_user("u1", None, "Continue"),
                pi_assistant("a1", Some("u1"), "On it", "toolUse", None),
                pi_message("a2", Some("a1"), message),
            ]);
            let snapshot = parse_pi_snapshot(raw.as_bytes(), 0);
            assert_eq!(snapshot.activity, Some(TranscriptActivity::Working));
            assert!(!snapshot.turns[1].completes_work);
        }
    }

    #[test]
    fn omp_tool_only_resume_recovers_working_without_a_user_or_custom_message() {
        let mut entries = vec![
            pi_user("u1", None, "Continue"),
            pi_assistant("a1", Some("u1"), "Done", "stop", None),
        ];
        assert_eq!(
            parse_pi_snapshot(pi_session(&entries).as_bytes(), 0).activity,
            Some(TranscriptActivity::Idle)
        );
        entries.push(pi_message(
            "a2",
            Some("a1"),
            serde_json::json!({
                "role": "assistant", "stopReason": "toolUse",
                "content": [{"type": "toolCall", "id": "call1", "name": "bash", "arguments": {}}]
            }),
        ));
        let snapshot = parse_pi_snapshot(pi_session(&entries).as_bytes(), 0);
        assert_eq!(snapshot.activity, Some(TranscriptActivity::Working));
        assert_eq!(
            snapshot
                .turns
                .iter()
                .map(|turn| turn.text.as_str())
                .collect::<Vec<_>>(),
            ["Continue", "Done"]
        );
    }

    #[test]
    fn omp_abort_and_exit_clear_pending_asks_without_synthetic_result_resume() {
        let initial = [
            pi_user("u1", None, "Continue"),
            r#"{"type":"custom","id":"s1","parentId":"u1","customType":"tool_execution_start","data":{"toolCallId":"ask1","toolName":"ask"}}"#.to_string(),
        ];
        for stopped in [
            pi_assistant("end", Some("s1"), "", "aborted", None),
            pi_assistant("end", Some("s1"), "", "error", None),
            r#"{"type":"custom","id":"end","parentId":"s1","customType":"session_exit","data":{"kind":"signal"}}"#.to_string(),
        ] {
            let mut entries = initial.to_vec();
            entries.push(stopped);
            entries.push(pi_message("result", Some("end"), serde_json::json!({
                "role": "toolResult", "toolCallId": "ask1", "toolName": "ask",
                "content": [], "isError": true
            })));
            let snapshot = parse_pi_snapshot(pi_session(&entries).as_bytes(), 0);
            assert_eq!(snapshot.activity, Some(TranscriptActivity::Idle));
        }
    }

    #[test]
    fn omp_activity_follows_the_selected_branch_not_abandoned_asks() {
        let mut entries = vec![
            pi_user("u1", None, "Start"),
            pi_assistant("a1", Some("u1"), "Done", "stop", None),
            r#"{"type":"custom","id":"ask","parentId":"a1","customType":"tool_execution_start","data":{"toolCallId":"ask1","toolName":"ask"}}"#.to_string(),
            pi_user("branch", Some("a1"), "New branch"),
        ];
        let snapshot = parse_pi_snapshot(pi_session(&entries).as_bytes(), 0);
        assert_eq!(snapshot.activity, Some(TranscriptActivity::Working));
        entries.push(pi_assistant("a2", Some("branch"), "Finished", "stop", None));
        let snapshot = parse_pi_snapshot(pi_session(&entries).as_bytes(), 0);
        assert_eq!(snapshot.activity, Some(TranscriptActivity::Idle));
        // Switching back through a bookkeeping leaf restores the old ask,
        // without adopting the other branch's user/assistant messages.
        entries
            .push(r#"{"type":"label","id":"label","parentId":"ask","label":"resume"}"#.to_string());
        let snapshot = parse_pi_snapshot(pi_session(&entries).as_bytes(), 0);
        assert_eq!(snapshot.activity, Some(TranscriptActivity::PendingAnswer));
        assert_eq!(
            snapshot
                .turns
                .iter()
                .map(|turn| turn.text.as_str())
                .collect::<Vec<_>>(),
            ["Start", "Done"]
        );
    }

    #[test]
    fn pi_reader_recovers_incomplete_appends_after_pane_changes() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, pi_user("u1", None, "Start")).unwrap();
        let mut reader = PiTranscriptReader::default();
        let first = reader.read(&path, 131).unwrap();
        assert_eq!(first.activity, Some(TranscriptActivity::Working));
        let restamped = reader.read(&path, 132).unwrap();
        assert_eq!(restamped.turns[0].pane_id, 132);

        let completed = pi_assistant("a1", Some("u1"), "Finished", "stop", None);
        let split = completed.len() / 2;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        write!(file, "\n{}", &completed[..split]).unwrap();
        let partial = reader.read(&path, 132).unwrap();
        assert_eq!(partial.activity, Some(TranscriptActivity::Working));
        assert_eq!(
            partial
                .turns
                .iter()
                .map(|turn| turn.text.as_str())
                .collect::<Vec<_>>(),
            ["Start"]
        );
        file.write_all(completed[split..].as_bytes()).unwrap();
        let finished = reader.read(&path, 132).unwrap();
        assert_eq!(finished.activity, Some(TranscriptActivity::Idle));
        assert_eq!(finished.turns[1].text, "Finished");
        assert!(finished.turns.iter().all(|turn| turn.pane_id == 132));
    }

    #[test]
    fn pi_reader_invalidates_replacement_rewrite_truncation_and_path_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let replacement = dir.path().join("replacement.jsonl");
        let original = pi_user("u1", None, "old");
        let changed = pi_user("u2", None, "new");
        std::fs::write(&path, &original).unwrap();
        let mut reader = PiTranscriptReader::default();
        assert_eq!(reader.read(&path, 1).unwrap().turns[0].text, "old");

        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::write(&replacement, &changed).unwrap();
        std::fs::File::open(&replacement)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        assert_eq!(reader.read(&path, 1).unwrap().turns[0].text, "new");

        // A same-size rewrite is not an append, and still invalidates.
        std::fs::write(&path, &original).unwrap();
        std::fs::File::open(&path)
            .unwrap()
            .set_modified(modified + std::time::Duration::from_secs(1))
            .unwrap();
        assert_eq!(reader.read(&path, 1).unwrap().turns[0].text, "old");
        std::fs::write(&path, "").unwrap();
        let empty = reader.read(&path, 1).unwrap();
        assert!(empty.turns.is_empty());
        assert_eq!(empty.activity, None);

        std::fs::write(&replacement, changed).unwrap();
        assert_eq!(reader.read(&replacement, 2).unwrap().turns[0].text, "new");
        assert_eq!(reader.read(&replacement, 2).unwrap().turns[0].pane_id, 2);
        std::fs::remove_file(&replacement).unwrap();
        assert!(reader.read(&replacement, 2).unwrap().turns.is_empty());
        std::fs::write(&replacement, original).unwrap();
        assert_eq!(reader.read(&replacement, 2).unwrap().turns[0].text, "old");
    }

    #[test]
    fn pi_session_file_is_found_by_exact_id_in_any_bucket() {
        let home = tempfile::tempdir().unwrap();
        let wanted = Uuid::parse_str("11111111-2222-4333-8444-555555555555").unwrap();
        let other = Uuid::new_v4();
        let dir = home.path().join(".pi/agent/sessions/--wanted--");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("2024-12-03T14-00-00-000Z_{other}.jsonl")),
            "{}\n",
        )
        .unwrap();
        let expected = dir.join(format!("2024-12-03T14-00-00-000Z_{wanted}.jsonl"));
        std::fs::write(&expected, "{}\n").unwrap();

        assert_eq!(find_pi_session_file(home.path(), wanted), Some(expected));
        assert_eq!(find_pi_session_file(home.path(), Uuid::new_v4()), None);
    }

    #[test]
    fn pi_reads_the_exact_session_shape_written_by_a_real_pi_release() {
        // Captured from pi 0.85.1 driven against a local mock provider: array
        // user content, a tool-use assistant record, a tool result, and a
        // model/thinking bookkeeping pair. This is the format that must keep
        // parsing across pi releases.
        let raw = r#"{"type":"session","version":3,"id":"22222222-3333-4444-8555-666666666666","timestamp":"2026-09-11T04:46:53.559Z","cwd":"/tmp/work"}
{"type":"model_change","id":"1ccfefe7","parentId":null,"timestamp":"2026-09-11T04:46:53.610Z","provider":"mock","modelId":"mock-1"}
{"type":"thinking_level_change","id":"ff11e651","parentId":"1ccfefe7","timestamp":"2026-09-11T04:46:53.610Z","thinkingLevel":"off"}
{"type":"message","id":"b064a4be","parentId":"ff11e651","timestamp":"2026-09-11T04:46:53.635Z","message":{"role":"user","content":[{"type":"text","text":"Use the tool"}],"timestamp":1789102013634}}
{"type":"message","id":"754a126b","parentId":"b064a4be","timestamp":"2026-09-11T04:46:53.697Z","message":{"role":"assistant","content":[{"type":"toolCall","id":"call_mock","name":"bash","arguments":{"command":"echo hi"}}],"api":"openai-completions","provider":"mock","model":"mock-1","usage":{"input":12,"output":7,"cacheRead":0,"cacheWrite":0,"reasoning":0,"totalTokens":19,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"toolUse","timestamp":1789102013652,"responseId":"chatcmpl-mock","rawStopReason":"tool_calls"}}
{"type":"message","id":"0430658b","parentId":"754a126b","timestamp":"2026-09-11T04:46:53.715Z","message":{"role":"toolResult","toolCallId":"call_mock","toolName":"bash","content":[{"type":"text","text":"hi\n"}],"isError":false,"timestamp":1789102013715}}
{"type":"message","id":"61f5ef91","parentId":"0430658b","timestamp":"2026-09-11T04:46:53.722Z","message":{"role":"assistant","content":[{"type":"text","text":"The tool said hi."}],"api":"openai-completions","provider":"mock","model":"mock-1","usage":{"input":12,"output":7,"cacheRead":0,"cacheWrite":0,"reasoning":0,"totalTokens":19,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"stop","timestamp":1789102013716,"responseId":"chatcmpl-mock","rawStopReason":"stop"}}"#;

        let turns = parse_pi_snapshot(raw.as_bytes(), 0).turns;
        assert_eq!(
            turns.len(),
            2,
            "tool-only and tool-result records are not turns"
        );
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[0].text, "Use the tool");
        assert!(turns[1].completes_work);
        assert_eq!(turns[1].text, "The tool said hi.");
        assert_eq!(turns[1].model.as_deref(), Some("mock-1"));
        assert_eq!(turns[1].input_tokens, Some(12));
        assert_eq!(turns[1].output_tokens, Some(7));
        assert_eq!(turns[1].cost_usd, Some(0.0));
    }
}
