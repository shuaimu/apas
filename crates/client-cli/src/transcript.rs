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
//! * **codex** — has no equivalent flag. Its rollout files record `cwd` and a
//!   start `timestamp` in `session_meta`, so a pane is matched to the newest
//!   rollout in its own directory that started at or after the pane did. That
//!   is a heuristic: two codex panes started in the same directory within the
//!   same second could in principle be confused. Bounded by requiring the cwd
//!   to match exactly.
//! * **opencode** — session IDs are generated as `ses_*`, so APAS cannot pin
//!   its UUID at creation. The newest session whose exported `directory`
//!   exactly matches the pane cwd is selected, then exported by ID. This has
//!   the same shared-cwd ambiguity as Codex but never crosses directories.

use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::conversation::TurnRecord;

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
        let text = claude_text(msg.get("content").unwrap_or(&Value::Null));
        let completes_work = role == "assistant"
            && msg
                .get("stop_reason")
                .and_then(Value::as_str)
                .is_some_and(|reason| reason != "tool_use");
        if text.trim().is_empty() {
            // Tool-use-only turns carry no text. Skipping keeps the history
            // readable; the tool calls themselves are not conversation.
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
            completes_work,
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
            completes_work: false,
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
            completes_work,
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

/// Newest codex rollout whose `session_meta.cwd` matches `cwd`.
///
/// Heuristic by necessity — codex has no flag to pin a session id. Requiring an
/// exact cwd match keeps the ambiguity to panes sharing a directory.
/// The identifying header of a codex rollout, taken from its `session_meta`.
struct CodexRolloutMeta {
    cwd: String,
    /// Immutable session start. Selection keys on this rather than mtime
    /// precisely because it cannot change while the file is being appended to.
    started_at: String,
    /// `user` for a session a person drives, `subagent` for a thread codex
    /// spawned itself. Absent on rollouts written by older codex versions.
    thread_source: Option<String>,
}

impl CodexRolloutMeta {
    /// Whether this rollout belongs to a thread codex spawned on its own.
    ///
    /// Anything that is not explicitly a non-user thread counts as the user's:
    /// codex versions predating `thread_source` omit the field entirely, and
    /// treating those as subagents would leave their panes with no transcript.
    fn is_subagent(&self) -> bool {
        self.thread_source
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
    let root = home.join(".codex").join("sessions");
    let mut best: Option<(String, PathBuf)> = None;
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(meta) = read_codex_rollout_meta(&path) else {
                continue;
            };
            if meta.cwd != cwd.to_string_lossy() || meta.is_subagent() {
                continue;
            }
            // Tie-break on path so two rollouts sharing a start timestamp
            // still resolve to the same file on every poll.
            let key = meta.started_at;
            let better = match best.as_ref() {
                None => true,
                Some((best_key, best_path)) => {
                    (&key, &path) > (best_key, best_path)
                }
            };
            if better {
                best = Some((key, path));
            }
        }
    }
    best.map(|(_, p)| p)
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

#[cfg(test)]
mod tests {
    use super::*;

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

        assert_eq!(first, second, "selection flapped after a sibling was written");
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
}
