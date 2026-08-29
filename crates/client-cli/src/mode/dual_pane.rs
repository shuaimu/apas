//! Tab-based mode: Multiple independent Claude sessions as tabs
//!
//! New projects start with one default tab:
//! - Interactive session
//!
//! Users can create and close tabs dynamically from both TUI and web UI.

use anyhow::Result;
use base64::Engine as _;
use shared::{
    ClaudeContentBlock, ClaudeStreamMessage, CliToServer, CodexStreamMessage, PaneType, Provider,
    ServerToCli,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::mpsc as tokio_mpsc;
use uuid::Uuid;

use crate::project::{get_or_create_project, save_project};
use crate::terminal_pane::{terminal_binary_for, TerminalPanes, TerminalRuntimeHandle};
use crate::tui::{PaneOutput, TuiCommand, TuiEvent};

type ProjectPolicyState = Arc<Mutex<Option<shared::EffectiveProjectPolicy>>>;

/// Classic/manual single-agent fallback. Managed team panes should be created
/// from role templates / canonical role prompts instead of this TODO.md loop.
const DEFAULT_PROMPT: &str = r#"Work on tasks defined in TODO.md. Do the following steps. Don't ask me for advice, just pick the best option you think that is honest, complete, and not corner-cutting:

1. Do a git pull to check if there are any remote updates. Pick the top high-priority undone task, choose its first leaf task. If there are no undone TODO items left, sleep a minute and exit.
2. Analyze the task, check if this can be done with not too many LOC (i.e., smaller than 500 lines code give or take). If not, try to analyze this task and break it down into several smaller tasks, expanding it in the TODO.md. The breakdown can be nested and hierarchical. Try to make each leaf task small enough (<500 lines LOC). You can document your analysis in the doc folder for future reference.
3. Try to execute the first leaf task. Make a plan for the task before execute. You can document key findings in either the TODO.md (a few sentences in the TODO item, or doc it in the docs folder for longer details and discussions.
4. Make sure to add comprehensive test for the task executed. Run the whole test suites to make sure no regression happens. If tests fail, fix them using the best, honest, complete approach, run test suites again to verify fixes work. Repeat this step until no tests fail.
5. Prepare for git commit, remove all temporary files, especially not to commit any binary files. For plan files, remove the implementation plan and keep the design rational and user manual and put it in the docs folder.
6. Git commit the changes. First do git pull --rebase, and fix conflicts if any. Then do git push."#;
const DEFAULT_MIN_ITERATION_INTERVAL_MINUTES: u64 = 15;

/// Pane input: (text, from_tui). from_tui=true means input came from TUI keyboard,
/// from_tui=false means it came from web (server already echoed it to web clients).
/// Resolve a binary name to an absolute path using the current PATH.
/// Falls back to the original name if resolution fails.
fn resolve_binary_path(name: &str) -> String {
    // Already absolute
    if name.starts_with('/') {
        return name.to_string();
    }
    // Use `which` via PATH lookup
    if let Ok(output) = Command::new("which").arg(name).output() {
        if output.status.success() {
            if let Ok(path) = String::from_utf8(output.stdout) {
                let path = path.trim();
                if !path.is_empty() {
                    return path.to_string();
                }
            }
        }
    }
    name.to_string()
}

fn is_deepseek_model(model: Option<&str>) -> bool {
    model
        .map(|m| {
            let normalized = m.trim().to_ascii_lowercase();
            !normalized.is_empty() && normalized.contains("deepseek")
        })
        .unwrap_or(false)
}

/// Slice a `&str` at no more than `max_bytes` while staying on a UTF-8
/// char boundary. Plain `&s[..max_bytes]` panics when `max_bytes` lands
/// inside a multi-byte codepoint (e.g. a `…` U+2026 at byte 139..142
/// crashed the streaming worker on byte-140 preview slicing).
fn truncate_str_at_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Persist only owner-operable workflow settings. Cluster-governed team and
/// launch-profile fields remain untouched even when an older server sends the
/// legacy combined message.
fn update_project_operations(
    project_dir: &Path,
    session_id: Uuid,
    auto_approve_todos: bool,
    auto_merge_prs: bool,
) -> Result<CliToServer> {
    let mut meta = get_or_create_project(project_dir)?;
    meta.auto_approve_todos = auto_approve_todos;
    meta.auto_merge_prs = auto_merge_prs;
    let team_enabled = meta.team_enabled;
    let disallowed_tab_types = meta.disallowed_tab_types.clone();
    save_project(project_dir, &meta)?;
    Ok(CliToServer::ProjectFlagsChanged {
        session_id,
        auto_approve_todos,
        auto_merge_prs,
        team_enabled,
        disallowed_tab_types,
    })
}

/// Whether this project permits creating a tab of the given kind + provider.
///
/// Read from `.apas` at the point of use for the same reason as
/// `team_enabled_for`: the restriction can change from the web at any moment,
/// and a stale answer would let a tab race the setting.
///
/// Fails **open** on an unreadable `.apas`, unlike `team_enabled_for`. The
/// worst case here is a user opening a tab an owner meant to block; failing
/// closed would make an unreadable file lock everyone out of the project
/// entirely, which is the worse outcome for a menu restriction.
fn launch_allowed_by_server_policy(
    policy_state: &ProjectPolicyState,
    kind: shared::PaneKind,
    provider: Provider,
    model: Option<&str>,
) -> bool {
    policy_state
        .lock()
        .ok()
        .and_then(|policy| policy.clone())
        .is_some_and(|policy| policy.allows(kind, provider, model))
}

/// Whether an **existing** pane may be brought back — resumed, rebooted, or
/// started.
///
/// Always, now. The launch-profile allowlist decides which combinations may be
/// brought into existence, and a pane that exists is a decision already taken
/// under whatever policy applied then; applying it here never removed the pane
/// it disapproved of, it only made that pane impossible to bring back.
///
/// Team availability used to gate a managed pane, because it was the switch
/// that stopped a running team. There is no team, so a pane marked managed by
/// some older `.apas` is simply a pane. Retired providers are refused
/// separately, before this is reached, since their backend genuinely no longer
/// exists.
fn existing_pane_relaunch_allowed() -> bool {
    true
}

/// Replace the provisional pane UUID with a provider identity only if the pane
/// still has the identity observed when its rollout was selected.
///
/// Transcript polling races pane removal, provider switching, and reboot. The
/// compare-before-replace prevents a late Codex poll from overwriting a newer
/// identity chosen by any of those paths.
fn remember_verified_codex_session(
    pane_sessions: &Arc<Mutex<HashMap<u32, Uuid>>>,
    pane_id: u32,
    observed_session_id: Uuid,
    verified_session_id: Uuid,
) -> bool {
    let Ok(mut sessions) = pane_sessions.lock() else {
        return false;
    };
    let Some(current) = sessions.get_mut(&pane_id) else {
        return false;
    };
    if *current != observed_session_id || *current == verified_session_id {
        return false;
    }
    *current = verified_session_id;
    true
}

/// Tell the web about the current pane roster and persist it to `.apas`.
///
/// Every path that creates or removes a pane owes both halves. There is no
/// per-pane "added" message — the web learns a pane exists only from a
/// `PaneList` — and `.apas` is what brings it back after a CLI restart. Miss
/// the broadcast and the tab stays invisible until something else triggers a
/// list (switching projects and back, which is how the terminal-pane bug
/// showed up); miss the save and the pane is gone on restart.
///
/// Factored out because this tail was copy-pasted at three call sites, and the
/// terminal-pane path returned early past all of them.
#[allow(clippy::too_many_arguments)]
fn announce_and_persist_panes(
    server_tx: &tokio_mpsc::Sender<CliToServer>,
    session_id: Uuid,
    working_dir: &str,
    pane_metas: &PaneMetas,
    input_channels: &InputChannels,
    pane_sessions: &Arc<Mutex<HashMap<u32, Uuid>>>,
    pane_pauses: &PanePauses,
    pane_stop_requests: &PaneStopRequests,
) {
    let _ = server_tx.blocking_send(CliToServer::PaneList {
        session_id,
        panes: build_pane_list(
            pane_metas,
            input_channels,
            session_id,
            pane_sessions,
            pane_pauses,
            pane_stop_requests,
        ),
    });
    save_pane_configs(
        working_dir,
        pane_sessions,
        pane_metas,
        pane_pauses,
        pane_stop_requests,
    );
}

/// Stop only panes whose persisted provider/model identifies a retired
/// backend. Their metadata remains in `pane_metas` for history and deletion,
/// but no input or process handle remains capable of relaunching them.
fn stop_retired_panes(
    pane_metas: &PaneMetas,
    pane_pauses: &PanePauses,
    input_channels: &InputChannels,
    terminal_panes: &TerminalPanes,
) -> Vec<u32> {
    struct Target {
        pane_id: u32,
        is_deadloop: bool,
        soft_interrupt: Option<mpsc::Sender<()>>,
        child_process: Arc<Mutex<Option<std::process::Child>>>,
    }

    let targets = pane_metas
        .lock()
        .map(|metas| {
            metas
                .iter()
                .filter(|(_, meta)| shared::is_retired_launch(meta.provider, meta.model.as_deref()))
                .map(|(pane_id, meta)| Target {
                    pane_id: *pane_id,
                    is_deadloop: meta.mode == shared::PaneMode::Deadloop,
                    soft_interrupt: meta
                        .streaming_interrupt_tx
                        .lock()
                        .ok()
                        .and_then(|sender| sender.as_ref().cloned()),
                    child_process: meta.child_process.clone(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    for target in &targets {
        if target.is_deadloop {
            if let Ok(pauses) = pane_pauses.lock() {
                if let Some(flag) = pauses.get(&target.pane_id) {
                    flag.store(true, Ordering::SeqCst);
                }
            }
        }
        if let Some(sender) = &target.soft_interrupt {
            let _ = sender.send(());
        }
        if let Ok(mut channels) = input_channels.lock() {
            channels.remove(&target.pane_id);
        }
        if let Some(handle) = terminal_panes
            .lock()
            .ok()
            .and_then(|mut terminals| terminals.remove(&target.pane_id))
        {
            handle.shutdown();
        }
        shutdown_pane_child(&target.child_process);
    }

    targets.into_iter().map(|target| target.pane_id).collect()
}

/// Turn a self-reported terminal-pane turn into the stream message an agent
/// pane would have produced for the same exchange.
///
/// This is the whole trick that makes terminal-pane history free: rather than
/// adding a parallel wire message, storage path, and renderer, a recorded turn
/// is dressed as a `ClaudeStreamMessage` and sent down the existing
/// `CliToServer::StreamMessage` channel. The server persists it to the same
/// `messages.jsonl`, the web renders it with the same components, and usage
/// accounting bills it to the same pane — none of which needed changing.
///
/// A turn with token counts yields a second `Result` message, because that is
/// the variant the server reads usage out of (`ws_cli` looks for
/// `extra.usage`). Without it the turn is recorded but bills nothing.
/// A question a terminal pane is waiting on. Held only while it is unanswered.
#[derive(Debug, Clone)]
struct TerminalQuestion {
    pane_id: u32,
    /// Question text → the option labels offered for it, in the order the TUI
    /// lists them. The order is what makes an answer addressable: the picker
    /// is navigated by position, not by label.
    options: Vec<(String, Vec<String>)>,
    /// Set once keystrokes have been written. A second answer for the same
    /// question sends nothing — retransmits are expected, double-typing is not.
    delivered: bool,
}

type TerminalQuestions = Arc<Mutex<HashMap<String, TerminalQuestion>>>;

/// Rebuild one pane's unanswered-question set from the provider transcript.
/// This runs over the full transcript so a restored pane can repopulate both
/// answer routing and its Pending answer status without replaying old chat
/// messages. Delivery flags survive ordinary polls, keeping retries safe.
fn reconcile_terminal_questions_for_pane(
    pending: &mut HashMap<String, TerminalQuestion>,
    pane_id: u32,
    turns: &[crate::conversation::TurnRecord],
) -> bool {
    let mut unresolved = HashMap::new();
    for turn in turns.iter().filter(|turn| turn.pane_id == pane_id) {
        if let Some(question) = &turn.question {
            unresolved.insert(
                question.tool_use_id.clone(),
                question_options(&question.input),
            );
        }
        if let Some(answer) = &turn.answer {
            unresolved.remove(&answer.tool_use_id);
        }
    }

    pending.retain(|tool_use_id, question| {
        question.pane_id != pane_id || unresolved.contains_key(tool_use_id)
    });
    for (tool_use_id, options) in &unresolved {
        pending
            .entry(tool_use_id.clone())
            .and_modify(|question| {
                question.pane_id = pane_id;
                question.options.clone_from(options);
            })
            .or_insert_with(|| TerminalQuestion {
                pane_id,
                options: options.clone(),
                delivered: false,
            });
    }
    !unresolved.is_empty()
}

/// The option labels a question offers, in order, from the tool call's input.
fn question_options(input: &serde_json::Value) -> Vec<(String, Vec<String>)> {
    input
        .get("questions")
        .and_then(serde_json::Value::as_array)
        .map(|questions| {
            questions
                .iter()
                .map(|q| {
                    let text = q
                        .get("question")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let labels = q
                        .get("options")
                        .and_then(serde_json::Value::as_array)
                        .map(|options| {
                            options
                                .iter()
                                .filter_map(|o| {
                                    o.get("label")
                                        .and_then(serde_json::Value::as_str)
                                        .map(str::to_string)
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    (text, labels)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Encode an answer as the keystrokes the provider's picker accepts.
///
/// Observed against Claude Code 2.1.233 by driving the real TUI on a pty: the
/// dialog prints its own contract — "Enter to select · ↑/↓ to navigate · Esc to
/// cancel" — arrow-down moves the `❯` marker, and digits do not. The agent's
/// own options occupy positions 1..N and the TUI appends its own entries
/// ("Type something", "Chat about this") below them, so stepping down by the
/// option's index from the default first entry can only ever land on an
/// option the agent offered.
///
/// Returns `None` when a selection names an option the question never offered,
/// rather than guessing — a wrong keystroke here answers on the human's behalf.
fn encode_answer_keystrokes(
    options: &[(String, Vec<String>)],
    answers: &HashMap<String, String>,
) -> Option<Vec<u8>> {
    const DOWN: &[u8] = b"\x1b[B";
    const CONFIRM: &[u8] = b"\r";
    let mut keys = Vec::new();
    for (question, labels) in options {
        let selected = answers.get(question)?;
        let index = labels.iter().position(|label| label == selected)?;
        for _ in 0..index {
            keys.extend_from_slice(DOWN);
        }
        keys.extend_from_slice(CONFIRM);
    }
    (!keys.is_empty()).then_some(keys)
}

/// What to do with an answer aimed at a terminal pane. Kept separate from the
/// IO so the guards — the part that stops a blind write going somewhere it
/// should not — are testable on their own.
#[derive(Debug, PartialEq)]
enum TerminalAnswerPlan {
    /// Write these keystrokes to this pane's pty.
    Deliver { pane_id: u32, keys: Vec<u8> },
    /// Keystrokes already went out for this question; sending them again would
    /// type into whatever the TUI is showing now.
    AlreadyDelivered { pane_id: u32 },
    /// The selection names an option this question never offered.
    UnknownOption { pane_id: u32 },
    /// Not a question this pane is waiting on — answered already, expired, or
    /// never asked here.
    NotPending,
}

fn plan_terminal_answer(
    questions: &HashMap<String, TerminalQuestion>,
    tool_use_id: &str,
    answers: &HashMap<String, String>,
) -> TerminalAnswerPlan {
    let Some(question) = questions.get(tool_use_id) else {
        return TerminalAnswerPlan::NotPending;
    };
    if question.delivered {
        return TerminalAnswerPlan::AlreadyDelivered {
            pane_id: question.pane_id,
        };
    }
    match encode_answer_keystrokes(&question.options, answers) {
        Some(keys) => TerminalAnswerPlan::Deliver {
            pane_id: question.pane_id,
            keys,
        },
        None => TerminalAnswerPlan::UnknownOption {
            pane_id: question.pane_id,
        },
    }
}

fn conversation_turn_to_stream_messages(
    turn: &crate::conversation::TurnRecord,
    session_id: Uuid,
    claude_session_id: Uuid,
) -> Vec<CliToServer> {
    let sid = claude_session_id.to_string();

    // Non-assistant turns go over `UserInput`, NOT as a `StreamMessage` with a
    // `ClaudeStreamMessage::User`. That variant means "tool result" to the
    // server: its converter walks the content blocks looking only for
    // `ToolResult` and silently ignores anything else, so a `Text` block was
    // dropped on the floor and every message the human typed vanished while
    // assistant turns came through fine. `UserInput` is the channel meant for
    // this — the server stores it as role "user" against the pane and routes
    // it to the web.
    // A question is published as the tool call it already is, so the web's
    // existing AskUserQuestionCard renders it — no new wire message, no new
    // renderer, no new storage path. An answer rides `User` + `ToolResult`,
    // deliberately the one variant the server's converter reads for tool
    // results, which is why ordinary non-assistant turns avoid it.
    if let Some(answer) = &turn.answer {
        return vec![CliToServer::StreamMessage {
            session_id,
            message: shared::ClaudeStreamMessage::User {
                message: shared::ClaudeUserMessage {
                    content: vec![shared::ClaudeContentBlock::ToolResult {
                        tool_use_id: answer.tool_use_id.clone(),
                        content: answer.recorded.clone(),
                        is_error: false,
                    }],
                    role: "user".to_string(),
                },
                session_id: sid,
                tool_use_result: None,
                extra: serde_json::Value::Null,
            },
            pane_type: None,
            pane_id: Some(turn.pane_id),
        }];
    }
    if let Some(question) = &turn.question {
        return vec![CliToServer::StreamMessage {
            session_id,
            message: shared::ClaudeStreamMessage::Assistant {
                message: shared::ClaudeAssistantMessage {
                    content: vec![shared::ClaudeContentBlock::ToolUse {
                        id: question.tool_use_id.clone(),
                        name: question.tool_name.clone(),
                        input: question.input.clone(),
                    }],
                    model: turn.model.clone().unwrap_or_default(),
                    extra: serde_json::Value::Null,
                },
                session_id: sid,
                // A pending question is not a finished turn: the pane is
                // blocked on the human, not idle.
                extra: serde_json::json!({ "terminal_turn_complete": false }),
            },
            pane_type: None,
            pane_id: Some(turn.pane_id),
        }];
    }
    let mut out = if turn.is_assistant() {
        vec![CliToServer::StreamMessage {
            session_id,
            message: shared::ClaudeStreamMessage::Assistant {
                message: shared::ClaudeAssistantMessage {
                    content: vec![shared::ClaudeContentBlock::Text {
                        text: turn.text.clone(),
                    }],
                    model: turn.model.clone().unwrap_or_default(),
                    extra: serde_json::Value::Null,
                },
                session_id: sid.clone(),
                // The server used to clear the terminal pane's working state
                // on every assistant fragment. Preserve a positive marker as
                // well as a negative one so updated clients can distinguish a
                // tool-use preamble from a provider-confirmed completion.
                extra: serde_json::json!({
                    "terminal_turn_complete": turn.completes_work,
                }),
            },
            pane_type: None,
            pane_id: Some(turn.pane_id),
        }]
    } else {
        vec![CliToServer::UserInput {
            session_id,
            text: turn.text.clone(),
            pane_type: None,
            pane_id: Some(turn.pane_id),
        }]
    };

    if turn.has_usage() {
        // `subtype: "success"` is what marks the turn complete for accounting;
        // cost is left at 0 because a self-reporting agent has no idea what it
        // was billed, and inventing a number would corrupt the roll-up.
        let usage = serde_json::json!({
            "usage": {
                "input_tokens": turn.input_tokens.unwrap_or(0),
                "output_tokens": turn.output_tokens.unwrap_or(0),
                "cache_read_input_tokens": 0,
                "cache_creation_input_tokens": 0,
            }
        });
        out.push(CliToServer::StreamMessage {
            session_id,
            message: shared::ClaudeStreamMessage::Result {
                subtype: "success".to_string(),
                result: String::new(),
                total_cost_usd: 0.0,
                duration_ms: 0,
                session_id: sid,
                is_error: false,
                extra: usage,
            },
            pane_type: None,
            pane_id: Some(turn.pane_id),
        });
    }

    out
}

/// The transcript belongs to the directory in which the provider process is
/// actually running, not necessarily the project's primary checkout.
fn terminal_transcript_cwd(project_dir: &Path, worktree_path: Option<&str>) -> std::path::PathBuf {
    worktree_path
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| project_dir.to_path_buf())
}

/// Existing history is suppressed only for panes present when the watcher was
/// installed. A pane created afterwards must be captured from its first turn,
/// including when that turn completes before the first polling tick.
fn initial_terminal_transcript_cursor(restored_at_watcher_start: bool, turn_count: usize) -> usize {
    if restored_at_watcher_start {
        turn_count
    } else {
        0
    }
}

/// Return a provider-confirmed working-state transition worth publishing.
///
/// An active state is published on first observation so a CLI reconnect can
/// restore a turn already in progress. Initial idle stays silent because idle
/// is the server default, and clearing here could race fresh web input before
/// Codex has written its task_started event.
fn observe_codex_working_state(
    states: &mut HashMap<u32, (bool, u64)>,
    pane_id: u32,
    observed: Option<bool>,
    connection_generation: u64,
) -> Option<bool> {
    let observed = observed?;
    match states.insert(pane_id, (observed, connection_generation)) {
        None if observed => Some(true),
        Some((previous, _)) if previous != observed => Some(observed),
        Some((true, previous_generation)) if previous_generation != connection_generation => {
            Some(true)
        }
        _ => None,
    }
}

/// Consecutive unchanged polls of the tracked claude transcript before an
/// in-TUI session switch is considered. Two polls ≈ six seconds of idleness,
/// which keeps a streaming turn (whose file grows continuously) from ever
/// being abandoned.
const CLAUDE_SWITCH_IDLE_POLLS: u32 = 2;

/// Watch state for one claude terminal pane's transcript source.
///
/// Tracks the currently read file plus its last observed size and mtime so
/// the poller can distinguish "still growing" from "the user switched to a
/// different session inside the TUI".
/// Newest mtime among the transcripts already in a directory, if any.
fn newest_transcript_mtime(dir: Option<&std::path::Path>) -> Option<std::time::SystemTime> {
    let entries = std::fs::read_dir(dir?).ok()?;
    entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
        .filter_map(|entry| entry.metadata().ok()?.modified().ok())
        .max()
}

struct ClaudeWatchState {
    path: std::path::PathBuf,
    size: u64,
    mtime: Option<std::time::SystemTime>,
    idle_polls: u32,
    /// The floor for what counts as a session switch while the watched file
    /// does not exist yet: the newest transcript already in the directory when
    /// this watch began.
    ///
    /// Claude writes a pane's transcript on its first turn, so a brand-new pane
    /// watches a path with no file behind it. That file never "grows", so the
    /// idle counter reaches the switch threshold within seconds — and with no
    /// mtime to compare against, every transcript in the cwd looked newer than
    /// the floor. A new pane therefore adopted the most recently touched
    /// conversation in its directory, which is how one arrived showing a
    /// stranger's messages before its terminal had produced a byte.
    ///
    /// Derived from the directory rather than the clock on purpose: these files
    /// live on NFS, so their mtimes and this host's `now()` are different clock
    /// domains, and skew either way would resurrect the bug or silently stop
    /// following real switches. Comparing an mtime against an mtime cannot
    /// skew. A file that existed before is followed only once it advances past
    /// where it was — which is exactly an in-TUI `/resume` onto it.
    started: std::time::SystemTime,
}

impl ClaudeWatchState {
    fn new(path: std::path::PathBuf) -> Self {
        let started = newest_transcript_mtime(path.parent())
            // An empty or unreadable directory has nothing to adopt, so the
            // clock is only ever the fallback.
            .unwrap_or_else(std::time::SystemTime::now);
        Self {
            path,
            size: 0,
            mtime: None,
            idle_polls: 0,
            started,
        }
    }

    /// The oldest a transcript may be and still count as somewhere this pane
    /// switched *to*. Never the epoch — see `started`.
    fn switch_floor(&self) -> std::time::SystemTime {
        self.mtime.unwrap_or(self.started)
    }
}

fn default_pane_label(pane_id: u32, model: Option<&str>) -> String {
    match pane_id {
        shared::PANE_ID_DEADLOOP => "Deadloop".to_string(),
        shared::PANE_ID_INTERACTIVE => "Interactive".to_string(),
        _ if shared::is_retired_model(model) => format!("Unsupported {}", pane_id),
        _ if is_deepseek_model(model) => format!("DeepSeek {}", pane_id),
        _ => format!("Tab {}", pane_id),
    }
}

fn is_generic_tab_label(label: &str, pane_id: u32) -> bool {
    label
        .trim()
        .eq_ignore_ascii_case(&format!("Tab {}", pane_id))
}

fn pane_label_or_default(raw_label: Option<&str>, pane_id: u32, model: Option<&str>) -> String {
    let default = default_pane_label(pane_id, model);
    let Some(label) = raw_label else {
        return default;
    };
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return default;
    }
    if shared::is_retired_model(model) && is_generic_tab_label(trimmed, pane_id) {
        return default;
    }
    if is_deepseek_model(model) && is_generic_tab_label(trimmed, pane_id) {
        return default;
    }
    trimmed.to_string()
}

fn resolve_pane_binary_path(
    provider: Provider,
    model: Option<&str>,
    claude_path: &str,
    codex_path: &str,
    opencode_path: &str,
    cursor_agent_path: &str,
) -> String {
    if shared::is_retired_launch(provider, model) {
        return String::new();
    }
    match provider {
        Provider::Claude | Provider::Deepseek => claude_path.to_string(),
        Provider::Codex => codex_path.to_string(),
        Provider::Opencode => opencode_path.to_string(),
        Provider::CursorAgent => cursor_agent_path.to_string(),
        #[allow(deprecated)]
        Provider::Minimax | Provider::Glm => String::new(),
    }
}

fn provider_display_name(provider: &Provider, model: Option<&str>) -> &'static str {
    if shared::is_retired_launch(*provider, model) {
        return "Unsupported provider";
    }
    match provider {
        Provider::Claude if is_deepseek_model(model) => "DeepSeek",
        Provider::Claude => "Claude",
        Provider::Codex => "Codex",
        #[allow(deprecated)]
        Provider::Minimax | Provider::Glm => "Unsupported provider",
        Provider::Deepseek => "DeepSeek",
        Provider::Opencode => "OpenCode",
        Provider::CursorAgent => "Cursor",
    }
}

fn provider_config_key(provider: &Provider, model: Option<&str>) -> &'static str {
    if shared::is_retired_launch(*provider, model) {
        return "unsupported_provider";
    }
    match provider {
        Provider::Claude => "claude_path",
        Provider::Codex => "codex_path",
        Provider::Deepseek => "claude_path",
        Provider::Opencode => "opencode_path",
        Provider::CursorAgent => "cursor_agent_path",
        #[allow(deprecated)]
        Provider::Minimax | Provider::Glm => "unsupported_provider",
    }
}

const DEEPSEEK_API_BASE_URL: &str = "https://api.deepseek.com/anthropic";
// Keep in sync with packages/web/src/lib/providerOptions.ts; the
// `deepseek_default_model_matches_web_provider_options` test guards drift.
const DEEPSEEK_DEFAULT_MODEL: &str = shared::DEEPSEEK_DEFAULT_MODEL;
const DEEPSEEK_PRO_RUNTIME_MODEL: &str = "deepseek-v4-pro[1m]";
const CODEX_SQLITE_HOME_ENV: &str = "CODEX_SQLITE_HOME";
const CODEX_SQLITE_HOME_ROOT: &str = "/var/tmp";

fn codex_sqlite_home_path(root: &Path, uid: u32) -> PathBuf {
    root.join(format!("apas-codex-sqlite-{uid}"))
}

fn ensure_private_codex_sqlite_home(path: &Path, uid: u32) -> Result<(), String> {
    use std::fs::{DirBuilder, Permissions};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() || metadata.uid() != uid {
                return Err(format!(
                    "Refusing insecure Codex SQLite directory {}",
                    path.display()
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = DirBuilder::new();
            builder.mode(0o700);
            if let Err(error) = builder.create(path) {
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(format!(
                        "Failed to create host-local Codex SQLite directory {}: {error}",
                        path.display()
                    ));
                }
            }
        }
        Err(error) => {
            return Err(format!(
                "Failed to inspect host-local Codex SQLite directory {}: {error}",
                path.display()
            ));
        }
    }

    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        format!(
            "Failed to inspect host-local Codex SQLite directory {}: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() || metadata.uid() != uid {
        return Err(format!(
            "Refusing insecure Codex SQLite directory {}",
            path.display()
        ));
    }
    std::fs::set_permissions(path, Permissions::from_mode(0o700)).map_err(|error| {
        format!(
            "Failed to protect host-local Codex SQLite directory {}: {error}",
            path.display()
        )
    })?;
    Ok(())
}

/// Prepare a persistent, host-local home for Codex's SQLite runtime state.
///
/// User homes on the APAS cluster are shared over NFS. Codex keeps its
/// resumable runtime index in WAL-mode SQLite databases, whose `-shm` state
/// cannot coordinate writers on different hosts. The rollout files remain in
/// the ordinary shared `CODEX_HOME`; only SQLite moves here.
fn prepare_codex_sqlite_home() -> Result<PathBuf, String> {
    let uid = unsafe { libc::getuid() };
    let path = codex_sqlite_home_path(Path::new(CODEX_SQLITE_HOME_ROOT), uid);
    ensure_private_codex_sqlite_home(&path, uid)?;
    Ok(path)
}

fn append_codex_sqlite_home_env(
    provider: &Provider,
    sqlite_home: &Path,
    env: &mut Vec<(String, String)>,
) {
    if matches!(provider, Provider::Codex) {
        env.push((
            CODEX_SQLITE_HOME_ENV.to_string(),
            sqlite_home.to_string_lossy().into_owned(),
        ));
    }
}

fn selected_deepseek_model(
    provider: &Provider,
    model: Option<&str>,
) -> Result<Option<&'static str>, String> {
    let model = model.map(str::trim).filter(|value| !value.is_empty());
    if let Some(model) = model {
        if let Some(canonical) = shared::canonical_deepseek_model(model) {
            return Ok(Some(canonical));
        }
        if matches!(provider, Provider::Deepseek) || is_deepseek_model(Some(model)) {
            return Err(format!(
                "Unsupported DeepSeek model '{model}'. Supported models are {} and {}.",
                shared::DEEPSEEK_PRO_MODEL,
                shared::DEEPSEEK_FLASH_MODEL,
            ));
        }
    } else if matches!(provider, Provider::Deepseek) {
        return Ok(Some(shared::DEEPSEEK_DEFAULT_MODEL));
    }
    Ok(None)
}

fn canonicalize_requested_model(model: Option<&str>) -> Option<String> {
    model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(|model| {
            shared::canonical_deepseek_model(model)
                .unwrap_or(model)
                .to_string()
        })
}

fn model_switch_can_use_fast_path(
    current_provider: Provider,
    current_model: Option<&str>,
    requested_provider: Provider,
    requested_model: Option<&str>,
) -> bool {
    matches!(current_provider, Provider::Claude)
        && matches!(requested_provider, Provider::Claude)
        && !is_deepseek_model(current_model)
        && !is_deepseek_model(requested_model)
        && !shared::is_retired_model(current_model)
        && !shared::is_retired_model(requested_model)
        && requested_model.is_some()
}

fn trim_to_option(raw: Option<String>) -> Option<String> {
    raw.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

/// Send SIGTERM (then SIGKILL after a grace period) to the entire process
/// group led by `pgid`. Used to reap a deadloop-pane's agent plus any
/// background children it left behind once the agent has emitted its
/// `result` event — without this the deadloop's next iteration never
/// starts because the agent process lingers indefinitely.
#[cfg(unix)]
fn kill_process_group(pgid: u32) {
    if pgid == 0 {
        return;
    }
    unsafe {
        libc::kill(-(pgid as i32), libc::SIGTERM);
    }
    let pgid_for_fallback = pgid;
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        unsafe {
            libc::kill(-(pgid_for_fallback as i32), libc::SIGKILL);
        }
    });
}

#[cfg(not(unix))]
fn kill_process_group(_pgid: u32) {}

/// How long a pane's agent gets to handle SIGTERM before the group is
/// SIGKILLed. Matches the deadloop's existing escalation window.
const PANE_KILL_GRACE: Duration = Duration::from_millis(500);

/// Put a pane's agent in its own process group at exec.
///
/// Every pane spawn does this, for two reasons. The deadloop group-kills the
/// agent the moment it emits its result event — one that lingers past result
/// wedges the next iteration. And closing a pane (or quitting APAS)
/// group-kills to reach the agent's *own* children: bash commands, subagents,
/// and this pane's `apas mcp-server`, none of which are children of ours.
///
/// Turn semantics are unchanged for interactive panes: nothing group-kills
/// them mid-turn, so background work a user launches still outlives the turn.
/// It no longer outlives the pane, which is the point.
///
/// The flip side is that agents no longer receive the Ctrl+C delivered to
/// APAS's own group — hence [`kill_all_pane_children`] on the shutdown paths.
#[cfg(unix)]
fn spawn_in_own_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn spawn_in_own_process_group(_command: &mut Command) {}

/// True when `pid` leads its own process group.
///
/// Guards every group kill. `kill(-pid)` on a pid that is *not* a group leader
/// signals whatever group that pid happens to sit in — for a pane spawned
/// without `setpgid` that is APAS's own group, so the "cleanup" would take
/// down the CLI along with every other pane. Checking leadership first makes
/// that unrepresentable.
#[cfg(unix)]
fn leads_own_process_group(pid: u32) -> bool {
    pid != 0 && unsafe { libc::getpgid(pid as i32) } == pid as i32
}

#[cfg(not(unix))]
fn leads_own_process_group(_pid: u32) -> bool {
    false
}

/// SIGTERM a pane's process group. Paired with [`sigkill_pane_child_group`]
/// after a grace period; split in two so a batch teardown can signal every
/// pane before waiting once, instead of paying the grace per pane.
///
/// Returns whether the group signal went out. `false` means the pane doesn't
/// lead a group and the caller must fall back to killing the child directly.
fn sigterm_pane_child_group(pid: u32) -> bool {
    if !leads_own_process_group(pid) {
        return false;
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), libc::SIGTERM);
    }
    true
}

/// SIGKILL a pane's process group, then reap the leader.
///
/// Only call this on a pid that [`sigterm_pane_child_group`] accepted, and only
/// while the leader is still unreaped — reaping frees the pid, and a recycled
/// pid would aim `-pid` at an unrelated group.
fn sigkill_pane_child_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    // `Child` does not reap on drop, so without this every closed pane leaves
    // a zombie for the lifetime of the CLI.
    let _ = child.wait();
}

/// Kill a pane's agent and everything it spawned, then reap it.
///
/// A bare `child.kill()` only SIGKILLs the agent itself. Agents are process
/// *parents*: claude/codex run bash commands, subagents, and this pane's own
/// `apas mcp-server`. Those are children of the agent, not of us, so killing
/// the agent alone reparents them to init and they keep running. When the
/// agent leads its own process group we signal the whole group instead.
///
/// Blocks for `PANE_KILL_GRACE`. We wait out the full grace rather than
/// stopping as soon as the agent exits: a grandchild that ignores SIGTERM is
/// exactly what the escalation is for, and the agent's own exit says nothing
/// about the rest of the subtree.
fn kill_pane_child_group(child: &mut std::process::Child) {
    if !sigterm_pane_child_group(child.id()) {
        let _ = child.kill();
        let _ = child.wait();
        return;
    }
    std::thread::sleep(PANE_KILL_GRACE);
    sigkill_pane_child_group(child);
}

/// Tear down a closed pane's agent off the TUI event loop.
///
/// Takes the child out of its slot so the shutdown paths won't also try to
/// kill it, then finishes on a detached thread — [`kill_pane_child_group`]
/// sleeps through the SIGTERM grace, and the event loop must stay responsive
/// while the tab disappears.
fn shutdown_pane_child(child_process: &Arc<Mutex<Option<std::process::Child>>>) {
    let Some(mut child) = child_process.lock().ok().and_then(|mut slot| slot.take()) else {
        return;
    };
    std::thread::spawn(move || kill_pane_child_group(&mut child));
}

/// Kill every pane's agent subtree at CLI shutdown.
///
/// Two-phase on purpose: SIGTERM every group, sleep the grace *once*, then
/// SIGKILL and reap. Doing it pane-by-pane would multiply the grace by the
/// pane count and make Ctrl+C feel hung.
fn kill_all_pane_children(pane_metas: &PaneMetas) {
    // Snapshot the child slots and release `pane_metas` immediately — this
    // runs from the Ctrl+C handler and from process exit, and holding the map
    // across the grace sleep would block every pane thread trying to wind down.
    let slots: Vec<Arc<Mutex<Option<std::process::Child>>>> = match pane_metas.lock() {
        Ok(metas) => metas.values().map(|m| m.child_process.clone()).collect(),
        Err(_) => return,
    };

    let mut group_led = Vec::new();
    for slot in slots {
        let Ok(mut guard) = slot.lock() else { continue };
        let Some(child) = guard.as_mut() else {
            continue;
        };
        if sigterm_pane_child_group(child.id()) {
            drop(guard);
            group_led.push(slot);
        } else {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    if group_led.is_empty() {
        return;
    }
    std::thread::sleep(PANE_KILL_GRACE);
    for slot in group_led {
        if let Ok(mut guard) = slot.lock() {
            if let Some(child) = guard.as_mut() {
                sigkill_pane_child_group(child);
            }
        }
    }
}

fn normalize_effort_level(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = trimmed.to_ascii_lowercase();
    // Valid claude levels (from `claude --help`): low, medium, high, xhigh,
    // max. Pass the user's selection through verbatim so xhigh and max stay
    // distinct — previously we coerced xhigh → max. `ultracode` is an
    // apas-only level (not a real claude effort) — it survives the
    // normalizer as a distinct string and is translated to the wire flag
    // `xhigh` plus an `ultracode ` prompt prefix at envelope-build time.
    match normalized.as_str() {
        "default" | "auto" | "none" | "off" => None,
        "low" => Some("low".to_string()),
        "medium" | "med" => Some("medium".to_string()),
        "high" => Some("high".to_string()),
        "xhigh" | "x-high" => Some("xhigh".to_string()),
        "max" => Some("max".to_string()),
        "ultracode" => Some("ultracode".to_string()),
        _ => None,
    }
}

/// Map a normalized effort string to the value passed via `claude --effort`.
/// `ultracode` is apas-only and not accepted by claude's CLI, so it spawns
/// as `xhigh` (and the workflow trigger is provided via a prompt prefix).
/// All other normalized levels pass through unchanged.
fn effort_to_claude_flag(normalized: &str) -> &str {
    match normalized {
        "ultracode" => "xhigh",
        other => other,
    }
}

/// Normalize an effort selection to a valid codex `model_reasoning_effort`
/// value. The gpt-5.6 models expose `low`/`medium`/`high`/`xhigh`/`max`/
/// `ultra` (see `~/.codex/models_cache.json`). Codex has no `minimal` for
/// these, so it floors to `low`; the apas-only `ultracode` maps to codex's
/// equivalent top tier `ultra`. `default`/empty → None (let codex fall back
/// to its config.toml default). Passed to codex as `-c
/// model_reasoning_effort=<level>`.
fn normalize_codex_effort(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "default" | "auto" | "none" | "off" => None,
        "minimal" | "min" | "low" => Some("low".to_string()),
        "medium" | "med" => Some("medium".to_string()),
        "high" => Some("high".to_string()),
        "xhigh" | "x-high" => Some("xhigh".to_string()),
        "max" => Some("max".to_string()),
        "ultra" | "ultracode" => Some("ultra".to_string()),
        _ => None,
    }
}

/// Build the JSONL user-input envelope line written to claude's stdin.
/// Wire format from happy-cli/src/claude/sdk/utils.ts:190 — plain string
/// `content`, not a content-block array. When the live effort is
/// `ultracode`, prefix the content with `ultracode ` so claude picks up
/// the apas-only workflow trigger; otherwise the prompt is passed through
/// unchanged.
fn build_user_envelope_line(prompt: &str, live_effort: Option<&str>) -> String {
    let content_owned;
    let content: &str = if live_effort == Some("ultracode") {
        content_owned = format!("ultracode {}", prompt);
        &content_owned
    } else {
        prompt
    };
    let envelope = serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": content },
    });
    format!("{}\n", envelope)
}

struct DeepseekBackendRuntimeConfig {
    api_key: Option<String>,
}

fn load_deepseek_backend_runtime_config() -> DeepseekBackendRuntimeConfig {
    let config = crate::config::Config::load().unwrap_or_default();
    DeepseekBackendRuntimeConfig {
        api_key: trim_to_option(config.local.deepseek_api_key),
    }
}

fn build_pane_env_overrides_from_keys(
    provider: &Provider,
    model: Option<&str>,
    deepseek_api_key: Option<String>,
) -> Result<Vec<(String, String)>, String> {
    if shared::is_retired_launch(*provider, model) {
        return Err("Unsupported provider: this backend has been retired".to_string());
    }
    if !matches!(provider, Provider::Claude | Provider::Deepseek) {
        return Ok(Vec::new());
    }
    let Some(primary_model) = selected_deepseek_model(provider, model)? else {
        return Ok(Vec::new());
    };

    let api_base_url = DEEPSEEK_API_BASE_URL.to_string();
    let api_key = deepseek_api_key;
    let missing_key_message = "DeepSeek backend is not configured (missing deepseek_api_key). Update it on the Machines page or run: apas config set deepseek_api_key <key>.".to_string();
    let api_key = api_key.ok_or(missing_key_message)?;

    let primary_runtime_model = if primary_model == shared::DEEPSEEK_PRO_MODEL {
        DEEPSEEK_PRO_RUNTIME_MODEL
    } else {
        shared::DEEPSEEK_FLASH_MODEL
    };
    let env = vec![
        ("ANTHROPIC_BASE_URL".to_string(), api_base_url),
        // Keep both names for compatibility across Claude CLI versions/wrappers.
        ("ANTHROPIC_AUTH_TOKEN".to_string(), api_key.clone()),
        ("ANTHROPIC_API_KEY".to_string(), api_key),
        (
            "ANTHROPIC_MODEL".to_string(),
            primary_runtime_model.to_string(),
        ),
        (
            "ANTHROPIC_DEFAULT_SONNET_MODEL".to_string(),
            DEEPSEEK_PRO_RUNTIME_MODEL.to_string(),
        ),
        (
            "ANTHROPIC_DEFAULT_OPUS_MODEL".to_string(),
            DEEPSEEK_PRO_RUNTIME_MODEL.to_string(),
        ),
        (
            "ANTHROPIC_DEFAULT_HAIKU_MODEL".to_string(),
            shared::DEEPSEEK_FLASH_MODEL.to_string(),
        ),
        (
            "CLAUDE_CODE_SUBAGENT_MODEL".to_string(),
            shared::DEEPSEEK_FLASH_MODEL.to_string(),
        ),
    ];
    Ok(env)
}

fn build_pane_env_overrides(
    provider: &Provider,
    model: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    let deepseek_runtime = load_deepseek_backend_runtime_config();
    let mut env = build_pane_env_overrides_from_keys(provider, model, deepseek_runtime.api_key)?;
    if matches!(provider, Provider::Codex) {
        let sqlite_home = prepare_codex_sqlite_home()?;
        append_codex_sqlite_home_env(provider, &sqlite_home, &mut env);
    }
    Ok(env)
}

fn format_spawn_error(
    provider: &Provider,
    model: Option<&str>,
    binary_path: &str,
    err: &std::io::Error,
) -> String {
    let display_name = provider_display_name(provider, model);
    if err.kind() == std::io::ErrorKind::NotFound {
        let config_key = provider_config_key(provider, model);
        format!(
            "[Error spawning {} binary '{}': {}. Configure with: apas config set {} <path>]",
            display_name, binary_path, err, config_key
        )
    } else {
        format!(
            "[Error spawning {} binary '{}': {}]",
            display_name, binary_path, err
        )
    }
}

/// Fail before creating a persistent pane host when the configured provider
/// binary cannot be executed. Without this preflight the host itself starts
/// successfully and the user only sees `setsid: ... No such file`, bypassing
/// the actionable APAS error (and leaving an unnecessary detached host).
fn resolve_terminal_binary(provider: &Provider, binary_path: &str) -> Result<String, String> {
    let resolved = resolve_binary_path(binary_path);
    let path = Path::new(&resolved);
    let metadata = std::fs::metadata(path);

    let executable = metadata.as_ref().is_ok_and(|metadata| {
        if !metadata.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    });

    if executable {
        return Ok(resolved);
    }

    let kind = match metadata {
        Ok(_) => std::io::ErrorKind::PermissionDenied,
        Err(error) => error.kind(),
    };
    let error = std::io::Error::new(
        kind,
        if kind == std::io::ErrorKind::PermissionDenied {
            "configured path is not an executable file"
        } else {
            "executable not found"
        },
    );
    Err(format_spawn_error(provider, None, binary_path, &error))
}

type PaneInput = (String, bool);

/// Per-pane input channel registry.
/// Maps pane_id -> Sender<PaneInput> for routing input to the correct session thread.
type InputChannels = Arc<Mutex<HashMap<u32, mpsc::Sender<PaneInput>>>>;

/// Allocate a pty and start the provider's interactive TUI for a
/// `PaneKind::Terminal` pane, registering the handle for `Terminal*`
/// routing.
///
/// Errors are returned as display strings rather than bubbled: a pane
/// that can't start should say so in its own tab, not abort CLI startup
/// or take down the other panes.
#[allow(clippy::too_many_arguments)]
fn spawn_terminal_pane(
    terminal_panes: &TerminalPanes,
    pane_id: u32,
    session_id: Uuid,
    // Pinned as claude's `--session-id`, which makes this pane's transcript
    // path exact instead of guessed.
    claude_session_id: Uuid,
    provider: &Provider,
    model: Option<&str>,
    binary_path: &str,
    working_dir: &str,
    worktree_path: Option<&str>,
    server_tx: &tokio_mpsc::Sender<CliToServer>,
    resume: bool,
    initial_input: Option<&str>,
) -> Result<(), String> {
    terminal_binary_for(provider).ok_or_else(|| {
        format!(
            "[{} cannot host a terminal pane; only claude, codex, and opencode are supported]",
            provider_display_name(provider, model)
        )
    })?;
    let binary_path = resolve_terminal_binary(provider, binary_path)?;
    let cwd = worktree_path.unwrap_or(working_dir);
    let env = build_pane_env_overrides(provider, model)?;

    let handle = TerminalRuntimeHandle::spawn(
        pane_id,
        session_id,
        claude_session_id,
        provider,
        &binary_path,
        cwd,
        &env,
        resume,
        initial_input,
        server_tx.clone(),
    )
    .map_err(|e| format!("[Error starting terminal pane: {e:#}]"))?;

    // Replace rather than shadow: reboot and the missing-input-channel
    // recovery path can both re-enter here for a live pane, and inserting
    // over the old handle without killing it would orphan a pty (and its
    // reader thread) for the lifetime of the CLI.
    let previous = terminal_panes
        .lock()
        .map_err(|_| "[terminal registry mutex poisoned]".to_string())?
        .insert(pane_id, handle);
    if let Some(previous) = previous {
        tracing::info!(pane_id, "replacing existing terminal pty");
        previous.shutdown();
    }
    Ok(())
}

/// Snapshot every configured terminal pane for reconnect reconciliation.
///
/// The configured roster is authoritative for which reports are owed; the
/// handle registry says whether a provider process actually exists. Keeping
/// this separate from `PaneList` lets rolling-upgrade servers ignore the new
/// messages while still accepting the established roster.
fn terminal_state_reports(
    session_id: Uuid,
    pane_metas: &PaneMetas,
    terminal_panes: &TerminalPanes,
) -> Vec<CliToServer> {
    let mut terminal_ids = pane_metas
        .lock()
        .map(|metas| {
            metas
                .iter()
                .filter_map(|(pane_id, meta)| meta.kind.is_terminal().then_some(*pane_id))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    terminal_ids.sort_unstable();

    terminal_ids
        .into_iter()
        .map(|pane_id| {
            let handle = terminal_panes
                .lock()
                .ok()
                .and_then(|panes| panes.get(&pane_id).cloned());
            match handle {
                Some(handle) => handle.state_message(session_id),
                None => CliToServer::TerminalState {
                    session_id,
                    pane_id,
                    instance_id: None,
                    lifecycle: shared::TerminalLifecycle::Exited,
                    status: Some("terminal process unavailable".to_string()),
                    runtime: None,
                },
            }
        })
        .collect()
}

fn build_lifecycle_inventory(
    pane_metas: &PaneMetas,
    terminal_panes: &TerminalPanes,
) -> shared::CliLifecycleInventory {
    let handles = terminal_panes.lock().ok();
    let mut panes = pane_metas
        .lock()
        .map(|metas| {
            metas
                .iter()
                .map(|(pane_id, meta)| shared::PanePreservationInfo {
                    pane_id: *pane_id,
                    mode: if meta.kind.is_terminal() {
                        handles
                            .as_ref()
                            .and_then(|handles| handles.get(pane_id))
                            .map(|handle| handle.preservation(*pane_id).mode)
                            .unwrap_or(shared::PanePreservationMode::RestartRequiredOnCliReboot)
                    } else {
                        shared::PanePreservationMode::StructuredPaneMayResume
                    },
                    runtime_id: if meta.kind.is_terminal() {
                        handles
                            .as_ref()
                            .and_then(|handles| handles.get(pane_id))
                            .and_then(|handle| handle.preservation(*pane_id).runtime_id)
                    } else {
                        None
                    },
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    panes.sort_by_key(|pane| pane.pane_id);
    shared::CliLifecycleInventory {
        persistent_terminal_hosting: crate::pane_host::persistent_hosting_available(),
        panes,
    }
}

/// Reboot markers a project left for its own restart, keyed by project.
///
/// This used to travel only through `APAS_REBOOT_HANDOFF`, which was sound
/// while a reboot meant a project `exec`ing its own process. Projects now share
/// one, and an environment variable is process-wide: whichever project started
/// next read the marker, found another project's id in it, and discarded it —
/// so the project it was addressed to came back without it and restarted its
/// terminals instead of adopting the pane hosts still holding them.
///
/// The environment variable is still set, and is still the only channel that
/// survives an `exec` — the standalone headless run, and a daemon self-upgrade
/// that replaces the process image and this map with it.
static REBOOT_HANDOFFS: std::sync::OnceLock<Mutex<HashMap<Uuid, std::path::PathBuf>>> =
    std::sync::OnceLock::new();

fn reboot_handoffs() -> &'static Mutex<HashMap<Uuid, std::path::PathBuf>> {
    REBOOT_HANDOFFS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn store_reboot_handoff(project_id: Uuid, path: &Path) {
    if let Ok(mut handoffs) = reboot_handoffs().lock() {
        handoffs.insert(project_id, path.to_path_buf());
    }
}

fn load_reboot_handoff(
    project_id: Uuid,
) -> Option<(std::path::PathBuf, crate::pane_host::RebootHandoffMarker)> {
    let in_process = reboot_handoffs()
        .lock()
        .ok()
        .and_then(|mut handoffs| handoffs.remove(&project_id));
    let path = match in_process {
        Some(path) => {
            // Ours, and it never travelled through the environment. Clear the
            // variable only if it points at this same marker.
            if std::env::var_os("APAS_REBOOT_HANDOFF").map(std::path::PathBuf::from)
                == Some(path.clone())
            {
                std::env::remove_var("APAS_REBOOT_HANDOFF");
            }
            path
        }
        None => {
            let path = std::env::var_os("APAS_REBOOT_HANDOFF").map(std::path::PathBuf::from)?;
            // Read before removing: a marker addressed to a sibling project is
            // left where it is, so that project can still claim it.
            match crate::pane_host::read_handoff_marker(&path) {
                Ok(marker) if marker.project_id != project_id => {
                    tracing::debug!(
                        marker_project = %marker.project_id,
                        "leaving a reboot handoff addressed to another project"
                    );
                    return None;
                }
                _ => {}
            }
            std::env::remove_var("APAS_REBOOT_HANDOFF");
            path
        }
    };
    match crate::pane_host::read_handoff_marker(&path) {
        Ok(marker)
            if marker.project_id == project_id
                && marker.expected_version == env!("APAS_VERSION")
                && marker.expected_executable
                    == crate::update::resolve_preferred_apas_executable() =>
        {
            Some((path, marker))
        }
        Ok(marker) => {
            tracing::warn!(
                marker_project = %marker.project_id,
                expected_project = %project_id,
                marker_version = %marker.expected_version,
                running_version = env!("APAS_VERSION"),
                "ignoring mismatched reboot handoff marker"
            );
            let _ = std::fs::remove_file(path);
            None
        }
        Err(error) => {
            tracing::warn!(%error, marker = %path.display(), "ignoring invalid reboot handoff marker");
            let _ = std::fs::remove_file(path);
            None
        }
    }
}

/// Per-pane pause flags (for deadloop panes).
type PanePauses = Arc<Mutex<HashMap<u32, Arc<AtomicBool>>>>;

/// Per-pane graceful stop requests (for deadloop panes).
type PaneStopRequests = Arc<Mutex<HashMap<u32, Arc<AtomicBool>>>>;

/// Per-pane metadata: mode/provider/prompt/model/effort, optional min interval, and child process handle.
#[derive(Clone)]
struct PaneMeta {
    mode: shared::PaneMode,
    provider: shared::Provider,
    label: String,
    prompt: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    min_iteration_interval_minutes: Option<u64>,
    child_process: Arc<Mutex<Option<std::process::Child>>>,
    /// Set by the streaming worker on entry; the `InterruptPane` handler
    /// uses it to signal a soft interrupt (control_request on stdin) instead
    /// of SIGKILL, so the long-lived process survives. `None` for
    /// non-streaming panes (legacy `--print`, codex, opencode, etc.) which
    /// fall back to the existing SIGINT-then-SIGKILL path.
    streaming_interrupt_tx: Arc<Mutex<Option<mpsc::Sender<()>>>>,
    /// Set by the streaming worker on entry; the `AnswerQuestion` handler
    /// pushes pre-serialized control_response JSON strings here, which the
    /// inner loop drains and writes to claude's stdin. This is how the
    /// canUseTool callback completes for AskUserQuestion. `None` for
    /// non-streaming panes (auto-approved via `--dangerously-skip-permissions`).
    control_response_tx: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    /// Set by the streaming worker on entry; the reader thread inserts an
    /// entry on each AskUserQuestion control_request so the AnswerQuestion
    /// handler can recover claude's request_id and original questions when
    /// the user's answers arrive from the web UI.
    pending_questions: Arc<Mutex<HashMap<String, PendingAskQuestion>>>,
    /// Live mirror of `effort`. The streaming worker's spawn loop reads
    /// this every iteration so a respawn (process crash, /loop iteration,
    /// etc.) picks up the latest effort the user selected — without it,
    /// the worker would keep using whatever effort was captured at first
    /// launch. The primary update path is `apply_flag_settings`
    /// control_request to the live claude (see UpdatePaneEffort handler);
    /// this Arc is the safety net for fresh process spawns.
    effort_arc: Arc<Mutex<Option<String>>>,
    /// Absolute path to an isolated git worktree the pane should run in.
    /// Mirrors `PaneConfig.worktree_path` from `.apas`. When `Some`,
    /// callers swap the project's `working_dir` for this path at process
    /// spawn time (claude's session-jsonl tailer, the child's cwd, etc.).
    /// `None` keeps the legacy "all panes share one tree" behaviour.
    /// The git worktree itself is created out-of-band; setting this
    /// field does not invoke git.
    worktree_path: Option<String>,
    /// Phase 2.1: role / goal / backstory triple that gets composed into a
    /// system-prompt prefix at spawn time. All three are persisted in
    /// `.apas` alongside the other pane config; default-None preserves
    /// legacy "no role identity" behaviour.
    role: Option<String>,
    goal: Option<String>,
    backstory: Option<String>,
    /// Phase 3.2a: editable-plan-checkpoint policy mirrored from
    /// `PaneConfig`. Default `Never` preserves legacy behaviour;
    /// the gating logic in 3.2b will read this at every turn.
    plan_review_mode: shared::PlanReviewMode,
    /// Live mirror of `plan_review_mode` for the streaming worker.
    /// `UpdatePaneReviewMode` (Phase 3.2c) writes here so the reader
    /// thread picks up the new policy without a respawn. Phase 3.2b2.
    plan_review_mode_arc: Arc<Mutex<shared::PlanReviewMode>>,
    /// Phase 3.2b2: parking lot for held tool_uses awaiting user
    /// approval. Keyed by `tool_use_id`; populated when the streaming
    /// worker decides to hold (per `plan_review::should_hold_tool`),
    /// drained when `ServerToCli::PlanReviewAnswer` arrives.
    pending_plan_reviews: Arc<Mutex<HashMap<String, PendingPlanReview>>>,
    /// v3.2: worker mode. `false` (default) = autonomous, available for
    /// Tech-Lead delegation. `true` = manual, only takes user chat.
    /// Mirrored from `PaneConfig.manual_mode` and persisted to `.apas`.
    manual_mode: bool,
    /// v3.5: managed vs unmanaged. Mirrored from `PaneConfig.managed`.
    /// See the field doc on `shared::PaneConfig` for semantics.
    managed: bool,
    /// Agent (headless stream-json worker) vs Terminal (pty-hosted TUI).
    /// Mirrored from `PaneConfig.kind`. A `Terminal` pane has no entry in
    /// `input_channels` and none of the streaming slots above are ever
    /// populated — its I/O lives in the `TerminalPanes` registry instead.
    kind: shared::PaneKind,
}

#[derive(Clone, Debug)]
struct StartBotPreservedFields {
    worktree_path: Option<String>,
    role: Option<String>,
    goal: Option<String>,
    backstory: Option<String>,
    plan_review_mode: shared::PlanReviewMode,
    manual_mode: bool,
    managed: bool,
}

impl Default for StartBotPreservedFields {
    fn default() -> Self {
        Self {
            worktree_path: None,
            role: None,
            goal: None,
            backstory: None,
            plan_review_mode: shared::PlanReviewMode::default(),
            manual_mode: false,
            managed: false,
        }
    }
}

fn start_bot_preserved_fields(meta: Option<&PaneMeta>) -> StartBotPreservedFields {
    match meta {
        Some(meta) => StartBotPreservedFields {
            worktree_path: meta.worktree_path.clone(),
            role: meta.role.clone(),
            goal: meta.goal.clone(),
            backstory: meta.backstory.clone(),
            plan_review_mode: meta.plan_review_mode,
            manual_mode: meta.manual_mode,
            managed: meta.managed,
        },
        None => StartBotPreservedFields::default(),
    }
}

fn restored_pane_mode_and_pause(
    pane: &shared::PaneConfig,
    legacy_deadloop_paused: bool,
) -> (shared::PaneMode, bool) {
    let mode = if pane.mode == shared::PaneMode::Deadloop && pane.stop_requested {
        shared::PaneMode::Interactive
    } else {
        pane.mode.clone()
    };

    let is_paused = if mode == shared::PaneMode::Deadloop {
        if pane.pane_id == shared::PANE_ID_DEADLOOP {
            pane.is_paused || legacy_deadloop_paused
        } else {
            pane.is_paused
        }
    } else {
        false
    };

    (mode, is_paused)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagedBuiltInPromptKind {
    Manager,
    TechLead,
    Reviewer,
    DefaultDeveloper,
}

fn boot_restore_try_resume_first(provider: &Provider, model: Option<&str>) -> bool {
    matches!(provider, Provider::Claude)
        && !shared::is_retired_model(model)
        && !is_deepseek_model(model)
}

fn build_agent_switch_respawn_event(
    pane_id: u32,
    label: String,
    claude_session_id: Uuid,
    mode: shared::PaneMode,
    provider: Provider,
    prompt: Option<String>,
    min_iteration_interval_minutes: Option<u64>,
    model: Option<String>,
    effort: Option<String>,
    worktree_path: Option<String>,
    role: Option<String>,
    goal: Option<String>,
    backstory: Option<String>,
    plan_review_mode: shared::PlanReviewMode,
    managed: bool,
) -> TuiEvent {
    TuiEvent::AddTabWithConfig {
        pane_id,
        label,
        claude_session_id,
        mode,
        provider,
        prompt,
        min_iteration_interval_minutes,
        model,
        effort,
        worktree_path,
        initial_input: None,
        role,
        goal,
        backstory,
        plan_review_mode,
        managed,
        try_resume_first: false,
        kind: shared::PaneKind::Agent,
    }
}

fn build_pane_reboot_events(
    pane_id: u32,
    meta: &PaneMeta,
    prior_session_id: Option<Uuid>,
) -> (TuiEvent, TuiEvent) {
    (
        TuiEvent::CloseTab {
            pane_id,
            cleanup_action: None,
        },
        TuiEvent::AddTabWithConfig {
            pane_id,
            label: meta.label.clone(),
            claude_session_id: prior_session_id.unwrap_or_else(Uuid::new_v4),
            mode: meta.mode.clone(),
            provider: meta.provider,
            prompt: meta.prompt.clone(),
            min_iteration_interval_minutes: meta.min_iteration_interval_minutes,
            model: meta.model.clone(),
            effort: meta.effort.clone(),
            worktree_path: meta.worktree_path.clone(),
            initial_input: None,
            role: meta.role.clone(),
            goal: meta.goal.clone(),
            backstory: meta.backstory.clone(),
            plan_review_mode: meta.plan_review_mode,
            managed: meta.managed,
            try_resume_first: true,
            // Rebooting a terminal pane must re-open a pty, not fall back
            // to the agent worker.
            kind: meta.kind,
        },
    )
}

/// One held tool_use waiting on user approval (Phase 3.2b2).
#[derive(Clone, Debug)]
struct PendingPlanReview {
    /// claude's `request_id` for the control_request — must echo back
    /// in the control_response or claude can't match it.
    request_id: String,
    /// Original tool input, replayed verbatim in the allow path so the
    /// agent's intent isn't accidentally clipped.
    input: serde_json::Value,
}

/// State stored for each in-flight AskUserQuestion call, keyed by tool_use_id.
/// Populated when the streaming reader sees a `can_use_tool` control_request
/// for `AskUserQuestion`, consumed when the AnswerQuestion handler arrives.
#[derive(Clone, Debug)]
struct PendingAskQuestion {
    /// claude's `request_id` for the control_request — must echo back in the
    /// control_response or claude can't match the response to the call.
    request_id: String,
    /// Original questions array from claude's tool input — must echo back in
    /// `updatedInput.questions` alongside the user's answers.
    questions: serde_json::Value,
}

/// Per-pane metadata registry.
type PaneMetas = Arc<Mutex<HashMap<u32, PaneMeta>>>;

const ASK_USER_QUESTION_AUTO_CANCEL_STATUS: &str =
    "[Pending question auto-cancelled: new message replaces it]";

#[derive(Clone, Debug, PartialEq, Eq)]
struct CancelledAskUserQuestion {
    tool_use_id: String,
    request_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaneInputRouteResult {
    Sent,
    MissingChannel,
    Disconnected,
}

fn auto_cancel_pending_questions_for_new_input(
    pane_metas: &PaneMetas,
    target_pane: u32,
) -> Vec<CancelledAskUserQuestion> {
    let pending_to_cancel: Vec<(String, String, mpsc::Sender<String>)> = {
        let metas = pane_metas.lock().unwrap();
        let Some(meta) = metas.get(&target_pane) else {
            return Vec::new();
        };
        let sender = meta
            .control_response_tx
            .lock()
            .ok()
            .and_then(|g| g.as_ref().cloned());
        let Some(tx) = sender else {
            return Vec::new();
        };
        let mut map = meta.pending_questions.lock().unwrap();
        let drained = map
            .iter()
            .map(|(tool_use_id, pending)| {
                (tool_use_id.clone(), pending.request_id.clone(), tx.clone())
            })
            .collect();
        map.clear();
        drained
    };

    let mut cancelled = Vec::new();
    for (tool_use_id, request_id, cr_tx) in pending_to_cancel {
        let response = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": {
                    "behavior": "deny",
                    "message": "User cancelled the question by sending a new prompt.",
                    "toolUseID": tool_use_id,
                }
            }
        });
        if cr_tx.send(response.to_string()).is_err() {
            tracing::warn!(
                pane_id = target_pane,
                tool_use_id = tool_use_id.as_str(),
                "auto-cancel: streaming worker channel dead",
            );
            continue;
        }
        tracing::info!(
            pane_id = target_pane,
            tool_use_id = tool_use_id.as_str(),
            "Auto-cancelled AskUserQuestion because user sent a new prompt",
        );
        cancelled.push(CancelledAskUserQuestion {
            tool_use_id,
            request_id,
        });
    }
    cancelled
}

fn manual_create_pr_worktree_path(
    pane_metas: &PaneMetas,
    target_pane: u32,
) -> Result<Option<String>, String> {
    let metas = pane_metas.lock().unwrap();
    match metas.get(&target_pane) {
        Some(meta) => Ok(meta.worktree_path.clone()),
        None => Ok(None),
    }
}

fn route_web_input_to_pane(
    input_channels: &InputChannels,
    target_pane: u32,
    data: &str,
) -> PaneInputRouteResult {
    let target_tx = {
        let channels = input_channels.lock().unwrap();
        channels.get(&target_pane).cloned()
    };

    match target_tx {
        Some(tx) => match tx.send((data.to_string(), false)) {
            Ok(()) => PaneInputRouteResult::Sent,
            Err(_) => PaneInputRouteResult::Disconnected,
        },
        None => PaneInputRouteResult::MissingChannel,
    }
}

/// Run in tab-based mode
/// Run one project.
///
/// There is no longer an interactive variant: `apas` registers a project and
/// exits, and `--attach` renders a running project from its own process. What
/// remains is the body every project has always had, minus the terminal UI
/// that only the removed foreground path used.
pub async fn run_headless(
    server_url: &str,
    token: &str,
    working_dir: &Path,
) -> Result<ProjectOutcome> {
    run_inner(
        server_url,
        token,
        working_dir,
        Arc::new(AtomicBool::new(false)),
        None,
    )
    .await
}

/// Run one project, stoppable by the caller.
///
/// Setting `shutdown` ends the project through the same teardown an ordinary
/// stop takes — panes killed, `.apas` saved, threads joined. Aborting the task
/// instead would strand roughly thirty threads and every pane child, which is
/// why cancellation is a flag the project observes rather than a cancelled
/// future.
pub async fn run_project(
    server_url: &str,
    token: &str,
    working_dir: &Path,
    shutdown: Arc<AtomicBool>,
    machine_id: Uuid,
) -> Result<ProjectOutcome> {
    run_inner(server_url, token, working_dir, shutdown, Some(machine_id)).await
}

async fn run_inner(
    server_url: &str,
    token: &str,
    working_dir: &Path,
    shutdown: Arc<AtomicBool>,
    machine_id: Option<Uuid>,
) -> Result<ProjectOutcome> {
    let config = crate::config::Config::load().unwrap_or_default();
    // Resolve binary paths to absolute paths at startup (while PATH is correct).
    // Systemd-run environments may have a minimal PATH that misses nvm/cargo bins.
    let claude_path = resolve_binary_path(&config.local.claude_path);
    let codex_path = resolve_binary_path(&config.local.codex_path);
    let opencode_path = resolve_binary_path(&config.local.opencode_path);
    let cursor_agent_path = resolve_binary_path(&config.local.cursor_agent_path);

    // Load or create project metadata
    let mut metadata = get_or_create_project(working_dir)?;
    let session_id = metadata.id;
    let reboot_handoff = load_reboot_handoff(session_id);
    // `None` means the authoritative server policy has not arrived yet and
    // launch-like mutations fail closed. Existing processes may keep running
    // and will be reported as noncompliant rather than terminated.
    let project_policy: ProjectPolicyState = Arc::new(Mutex::new(None));

    save_project(working_dir, &metadata)?;

    let configured_terminal_panes = metadata
        .panes
        .iter()
        .filter_map(|pane| pane.kind.is_terminal().then_some(pane.pane_id))
        .collect::<std::collections::HashSet<_>>();
    match crate::pane_host::reconcile_project_hosts(session_id, &configured_terminal_panes) {
        Ok(removed) if removed > 0 => {
            tracing::info!(removed, "removed unrostered pane-host runtimes")
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "could not reconcile persistent pane-host registry"),
    }

    let default_prompt = DEFAULT_PROMPT.to_string();

    let working_dir_str = working_dir.to_string_lossy().to_string();
    let server_url = server_url.to_string();
    let token = token.to_string();

    // Build startup pane list from persisted metadata.
    let mut tabs_to_restore: Vec<(
        u32,
        Uuid,
        String,
        shared::PaneMode,
        Provider,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<u64>,
        bool,
        Option<String>,         // worktree_path from .apas (Phase 1.1)
        Option<String>,         // role from .apas (Phase 2.1)
        Option<String>,         // goal from .apas (Phase 2.1)
        Option<String>,         // backstory from .apas (Phase 2.1)
        shared::PlanReviewMode, // plan_review_mode from .apas (Phase 3.2)
        bool,                   // manual_mode from .apas (v3.2)
        bool,                   // managed from .apas (v3.5)
        shared::PaneKind,       // Agent vs pty-hosted Terminal
    )> = metadata
        .panes
        .iter()
        .map(|pane| {
            let (mode, is_paused) = restored_pane_mode_and_pause(pane, metadata.is_paused);
            let label =
                pane_label_or_default(pane.label.as_deref(), pane.pane_id, pane.model.as_deref());

            (
                pane.pane_id,
                pane.session_id,
                label,
                mode,
                pane.provider,
                pane.prompt.clone(),
                pane.model.clone(),
                normalize_effort_level(pane.effort.as_deref()),
                pane.min_iteration_interval_minutes,
                is_paused,
                pane.worktree_path.clone(),
                pane.role.clone(),
                pane.goal.clone(),
                pane.backstory.clone(),
                pane.plan_review_mode,
                pane.manual_mode,
                pane.managed,
                pane.kind,
            )
        })
        .collect();
    tabs_to_restore.sort_by_key(|(pane_id, ..)| *pane_id);

    // Channel for sending to server
    let (server_tx, server_rx) = tokio_mpsc::channel::<CliToServer>(256);
    // The server owns pane status in memory and clears it when this CLI
    // disconnects. Transcript state lives in a separate watcher, so give it a
    // reconnect generation that makes an unchanged active turn publish once
    // after every successful session registration.
    let server_connection_generation = Arc::new(AtomicU64::new(0));

    // Shutdown flag
    // Supplied by the caller so a project can be stopped from outside; a
    // standalone run passes a fresh one and nothing ever sets it.

    // One file watcher per project, shared across all panes. Drives
    // event-based deadloop wake-ups so panes only consume tokens when
    // team-todo.md / .apas-team.jsonl / project_goal.md / .apas actually
    // change, instead of every `min_iteration_interval`.
    let file_watcher: Arc<crate::file_watcher::ProjectFileWatcher> = Arc::new(
        crate::file_watcher::ProjectFileWatcher::new(std::path::Path::new(&working_dir_str)),
    );

    // Per-pane pause flags (for deadloop panes)
    let pane_pauses: PanePauses = Arc::new(Mutex::new(HashMap::new()));

    // Per-pane graceful stop requests (for deadloop panes)
    let pane_stop_requests: PaneStopRequests = Arc::new(Mutex::new(HashMap::new()));

    // Reboot flag
    let reboot_requested = Arc::new(AtomicBool::new(false));

    // Per-pane metadata (mode, prompt, child process)
    let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::new()));

    // Per-pane input channels
    let input_channels: InputChannels = Arc::new(Mutex::new(HashMap::new()));

    // Live pty handles for `PaneKind::Terminal` panes. Kept apart from
    // `input_channels` so raw terminal I/O never crosses the agent path.
    let terminal_panes: TerminalPanes = Arc::new(Mutex::new(HashMap::new()));
    // Questions a terminal pane is currently blocked on, keyed by tool_use_id
    // and derived from the transcript rather than stored: a question is
    // pending once its tool_use is published and gone once its tool_result is.
    // The AnswerQuestion handler consults this to refuse an answer for a
    // question that is not pending, which is what makes a stale browser tab
    // harmless when the write itself is blind.
    let terminal_questions: TerminalQuestions = Arc::new(Mutex::new(HashMap::new()));
    // Why this project stopped, when it stops for a reportable reason. Read
    // after the server task ends, which is the project's teardown path.
    let stop_reason: ProjectStopReason = Arc::new(Mutex::new(None));

    // Track pane_id -> claude_session_id for persistence
    let pane_sessions: Arc<Mutex<HashMap<u32, Uuid>>> = Arc::new(Mutex::new(HashMap::new()));

    // TUI channels
    let (tui_input_tx, tui_input_rx) = mpsc::channel::<(u32, String)>();
    let (output_tx, output_rx) = mpsc::channel::<PaneOutput>();
    let (event_tx, event_rx) = mpsc::channel::<TuiEvent>();
    let (command_tx, command_rx) = mpsc::channel::<TuiCommand>();

    // Startup payloads for initial pane sessions.
    let mut deadloop_startups: Vec<(
        u32,
        Uuid,
        Provider,
        String,
        Option<String>,
        Option<String>,
        u64,
        Arc<AtomicBool>,
        Arc<AtomicBool>,
        Arc<Mutex<Option<std::process::Child>>>,
    )> = Vec::new();
    let mut interactive_startups: Vec<(
        u32,
        Uuid,
        Provider,
        Option<String>,
        Option<String>,
        mpsc::Receiver<PaneInput>,
        Arc<Mutex<Option<std::process::Child>>>,
    )> = Vec::new();
    // (pane_id, provider, model, pane_session_id, worktree_path) for pty-hosted terminal panes.
    let mut terminal_startups: Vec<(u32, Provider, Option<String>, Uuid, Option<String>)> =
        Vec::new();
    {
        let mut pauses = pane_pauses.lock().unwrap();
        let mut stop_requests = pane_stop_requests.lock().unwrap();
        let mut metas = pane_metas.lock().unwrap();
        let mut channels = input_channels.lock().unwrap();
        let mut sessions = pane_sessions.lock().unwrap();

        for (
            pane_id,
            pane_session_id,
            tab_label,
            mode,
            provider,
            tab_prompt,
            tab_model,
            tab_effort,
            min_interval_minutes,
            is_paused,
            tab_worktree,
            tab_role,
            tab_goal,
            tab_backstory,
            tab_plan_review_mode,
            tab_manual_mode,
            tab_managed,
            tab_kind,
        ) in &tabs_to_restore
        {
            let child_proc: Arc<Mutex<Option<std::process::Child>>> = Arc::new(Mutex::new(None));
            metas.insert(
                *pane_id,
                PaneMeta {
                    kind: *tab_kind,
                    mode: mode.clone(),
                    provider: *provider,
                    label: tab_label.clone(),
                    prompt: tab_prompt.clone(),
                    model: tab_model.clone(),
                    effort: tab_effort.clone(),
                    min_iteration_interval_minutes: *min_interval_minutes,
                    child_process: child_proc.clone(),
                    streaming_interrupt_tx: Arc::new(Mutex::new(None)),
                    control_response_tx: Arc::new(Mutex::new(None)),
                    pending_questions: Arc::new(Mutex::new(HashMap::new())),
                    effort_arc: Arc::new(Mutex::new(tab_effort.clone())),
                    worktree_path: tab_worktree.clone(),
                    role: tab_role.clone(),
                    goal: tab_goal.clone(),
                    backstory: tab_backstory.clone(),
                    plan_review_mode: *tab_plan_review_mode,
                    plan_review_mode_arc: Arc::new(Mutex::new(*tab_plan_review_mode)),
                    pending_plan_reviews: Arc::new(Mutex::new(HashMap::new())),
                    manual_mode: *tab_manual_mode,
                    managed: *tab_managed,
                },
            );
            sessions.insert(*pane_id, *pane_session_id);

            if shared::is_retired_launch(*provider, tab_model.as_deref()) {
                if *mode == shared::PaneMode::Deadloop {
                    pauses.insert(*pane_id, Arc::new(AtomicBool::new(true)));
                }
                let _ = output_tx.send(PaneOutput {
                    text: "[Unsupported provider — this historical pane remains stopped]"
                        .to_string(),
                    pane_id: *pane_id,
                });
                continue;
            }

            // Terminal panes host a real TUI on a pty. They register no
            // input channel and start no deadloop — the pty is driven
            // entirely by `Terminal*` messages — so branch before the
            // agent-worker startup bookkeeping below.
            if tab_kind.is_terminal() {
                terminal_startups.push((
                    *pane_id,
                    *provider,
                    tab_model.clone(),
                    *pane_session_id,
                    tab_worktree.clone(),
                ));
                continue;
            }

            if *mode == shared::PaneMode::Deadloop {
                let pause_flag = Arc::new(AtomicBool::new(*is_paused));
                let stop_flag = Arc::new(AtomicBool::new(false));
                pauses.insert(*pane_id, pause_flag.clone());
                stop_requests.insert(*pane_id, stop_flag.clone());
                let dl_prompt = tab_prompt
                    .clone()
                    .filter(|p| !p.trim().is_empty())
                    .unwrap_or_else(|| default_prompt.clone());
                let resolved_min_interval_minutes =
                    min_interval_minutes.unwrap_or(DEFAULT_MIN_ITERATION_INTERVAL_MINUTES);
                deadloop_startups.push((
                    *pane_id,
                    *pane_session_id,
                    *provider,
                    dl_prompt,
                    tab_model.clone(),
                    tab_effort.clone(),
                    resolved_min_interval_minutes,
                    pause_flag,
                    stop_flag,
                    child_proc,
                ));
            } else {
                let (input_tx, input_rx) = mpsc::channel::<PaneInput>();
                channels.insert(*pane_id, input_tx);
                interactive_startups.push((
                    *pane_id,
                    *pane_session_id,
                    *provider,
                    tab_model.clone(),
                    tab_effort.clone(),
                    input_rx,
                    child_proc,
                ));
            }
        }
    }

    // Restore pty-hosted terminal panes. `resume: true` because these are
    // panes that already existed in `.apas` — the pty itself did not
    // survive the restart, so re-exec the provider and resume its conversation
    // (claude `--resume <pane session id>` / codex `resume` / opencode
    // `--continue`).
    for (pane_id, provider, model, pane_session_id, worktree) in &terminal_startups {
        let binary_path = resolve_pane_binary_path(
            *provider,
            model.as_deref(),
            &claude_path,
            &codex_path,
            &opencode_path,
            &cursor_agent_path,
        );
        if let Err(err) = spawn_terminal_pane(
            &terminal_panes,
            *pane_id,
            session_id,
            *pane_session_id,
            provider,
            model.as_deref(),
            &binary_path,
            &working_dir_str,
            worktree.as_deref(),
            &server_tx,
            true,
            None,
        ) {
            tracing::error!(pane_id, %err, "failed to restore terminal pane");
        }
    }

    let initial_tabs: Vec<(u32, String, shared::PaneMode)> = tabs_to_restore
        .iter()
        .map(|(pane_id, _, label, mode, ..)| (*pane_id, label.clone(), mode.clone()))
        .collect();

    if initial_tabs.is_empty() {
        tracing::warn!("No panes available to restore; UI will start with no tabs.");
    }

    // Ctrl+C matters only when a project is the foreground process, which is
    // now just the standalone `--headless -d <path>` run. Inside the resident
    // instance the daemon has already registered its own handler, and only one
    // exists per process — so the second project to start would fail outright.
    // That is exactly what happened on the first live cutover: every project
    // died with "Ctrl-C signal handler already registered".
    let shutdown_for_handler = shutdown.clone();
    let metas_for_handler = pane_metas.clone();
    let sessions_for_handler = pane_sessions.clone();
    let pauses_for_handler = pane_pauses.clone();
    let stops_for_handler = pane_stop_requests.clone();
    let working_dir_for_handler = working_dir_str.clone();
    ctrlc::set_handler(move || {
        shutdown_for_handler.store(true, Ordering::SeqCst);
        // Persist the roster before tearing anything down. Ctrl+C is how a CLI
        // session normally ends, and without this any pane state changed since
        // the last explicit save is dropped — that is how a terminal tab
        // created under an older build vanished on restart.
        //
        // The ctrlc crate runs this on a dedicated thread, not in an
        // async-signal context, so file I/O here is fine. `save_project`
        // writes via temp+rename, so racing the normal exit path at worst
        // means last-writer-wins on identical content.
        save_pane_configs(
            &working_dir_for_handler,
            &sessions_for_handler,
            &metas_for_handler,
            &pauses_for_handler,
            &stops_for_handler,
        );
        // Kill every pane's agent *subtree*. Ctrl+C reaches APAS's own process
        // group, but panes run in groups of their own, so nothing reaches the
        // agents unless we signal them here.
        kill_all_pane_children(&metas_for_handler);
    })
    .unwrap_or_else(|err| {
        // Already taken means someone above us owns the signal, which is the
        // normal case in the resident instance. Not a reason to refuse to run
        // the project.
        tracing::debug!(%err, "not installing a project Ctrl+C handler");
    });

    // Spawn server connection task
    let server_task = {
        let shutdown = shutdown.clone();
        let pane_pauses = pane_pauses.clone();
        let pane_stop_requests = pane_stop_requests.clone();
        let reboot = reboot_requested.clone();
        let server_url = server_url.clone();
        let token = token.clone();
        let working_dir = working_dir_str.clone();
        let status_tx = output_tx.clone();
        let input_channels = input_channels.clone();
        let pane_metas = pane_metas.clone();
        let pane_sessions = pane_sessions.clone();
        let terminal_panes_for_server = terminal_panes.clone();
        let terminal_questions_for_server = terminal_questions.clone();
        let stop_reason_for_server = stop_reason.clone();
        let event_tx_for_server = event_tx.clone();
        let project_policy_for_server = project_policy.clone();
        let connection_generation_for_server = server_connection_generation.clone();
        tokio::spawn(async move {
            run_server_connection(
                &server_url,
                &token,
                session_id,
                &working_dir,
                server_rx,
                shutdown,
                pane_pauses,
                pane_stop_requests,
                reboot,
                input_channels,
                pane_metas,
                pane_sessions,
                terminal_panes_for_server,
                terminal_questions_for_server,
                stop_reason_for_server,
                status_tx,
                event_tx_for_server,
                project_policy_for_server,
                machine_id,
                connection_generation_for_server,
                reboot_handoff,
            )
            .await
        })
    };

    // Send initial messages for restored panes.
    for (pane_id, _, label, mode, _, _, _, _, _, is_paused, _, _, _, _, _, _, _, _) in
        &tabs_to_restore
    {
        let init_text = if *pane_id == shared::PANE_ID_DEADLOOP
            && *mode == shared::PaneMode::Deadloop
        {
            "[Deadloop pane initializing...]".to_string()
        } else if *pane_id == shared::PANE_ID_INTERACTIVE && *mode == shared::PaneMode::Interactive
        {
            "[Interactive pane initializing...]".to_string()
        } else {
            format!("[Restored tab: {}]", label)
        };
        let _ = output_tx.send(PaneOutput {
            text: init_text,
            pane_id: *pane_id,
        });

        if *mode == shared::PaneMode::Deadloop && *is_paused {
            let paused_text = if *pane_id == shared::PANE_ID_DEADLOOP {
                "[Deadloop starting in paused state (from previous session)]".to_string()
            } else {
                "[Bot starting in paused state (from previous session)]".to_string()
            };
            let _ = output_tx.send(PaneOutput {
                text: paused_text,
                pane_id: *pane_id,
            });
        }
    }

    // Spawn centralized input router — routes TUI input to correct pane via input_channels.
    spawn_centralized_input_router(tui_input_rx, input_channels.clone(), shutdown.clone());

    // Spawn pane session threads for restored panes.
    let mut pane_threads = Vec::new();

    for (
        pane_id,
        pane_session_id,
        provider,
        dl_prompt,
        model,
        effort,
        min_interval_minutes,
        pause_flag,
        stop_flag,
        child_process,
    ) in deadloop_startups
    {
        let output_tx = output_tx.clone();
        let server_tx = server_tx.clone();
        let shutdown = shutdown.clone();
        let event_tx = event_tx.clone();
        let working_dir = working_dir_str.clone();
        let input_channels_for_dl = input_channels.clone();
        let sid = session_id;
        let binary_path = resolve_pane_binary_path(
            provider,
            model.as_deref(),
            &claude_path,
            &codex_path,
            &opencode_path,
            &cursor_agent_path,
        );
        let (
            interrupt_slot,
            control_resp_slot,
            pending_qs,
            effort_arc,
            worktree_path,
            system_prompt,
            pr_mode_arc,
            pr_pending,
        ) = pane_metas
            .lock()
            .unwrap()
            .get(&pane_id)
            .map(|m| {
                (
                    m.streaming_interrupt_tx.clone(),
                    m.control_response_tx.clone(),
                    m.pending_questions.clone(),
                    m.effort_arc.clone(),
                    m.worktree_path.clone(),
                    crate::pane_identity::compose_system_prompt(
                        m.role.as_deref(),
                        m.goal.as_deref(),
                        m.backstory.as_deref(),
                    ),
                    m.plan_review_mode_arc.clone(),
                    m.pending_plan_reviews.clone(),
                )
            })
            .unwrap_or_else(|| {
                (
                    Arc::new(Mutex::new(None)),
                    Arc::new(Mutex::new(None)),
                    Arc::new(Mutex::new(HashMap::new())),
                    Arc::new(Mutex::new(None)),
                    None,
                    None,
                    Arc::new(Mutex::new(shared::PlanReviewMode::default())),
                    Arc::new(Mutex::new(HashMap::new())),
                )
            });
        let file_watcher_for_dl = file_watcher.clone();
        pane_threads.push(thread::spawn(move || {
            run_deadloop_session(
                &binary_path,
                &working_dir,
                worktree_path,
                system_prompt,
                pr_mode_arc,
                pr_pending,
                sid,
                pane_session_id,
                pane_id,
                &dl_prompt,
                model.clone(),
                effort.clone(),
                min_interval_minutes,
                &provider,
                output_tx,
                server_tx,
                shutdown,
                pause_flag,
                stop_flag,
                child_process,
                event_tx,
                interrupt_slot,
                control_resp_slot,
                pending_qs,
                effort_arc,
                input_channels_for_dl,
                // Claude reuses its on-disk session jsonl across boots,
                // so try_resume_first=true correctly continues prior chats.
                // Codex/Cursor/etc. keep their session state server-side
                // (codex thread, cursor chatId) and we don't always know
                // whether the saved id is still valid; passing false means
                // every reboot starts a fresh thread. Trade-off: no
                // cross-boot continuity for those backends, but we never
                // fail with "no rollout found" on a stale id either.
                boot_restore_try_resume_first(&provider, model.as_deref()),
                file_watcher_for_dl,
            )
        }));
    }

    for (pane_id, pane_session_id, provider, model, effort, input_rx, child_proc) in
        interactive_startups
    {
        let output_tx = output_tx.clone();
        let server_tx = server_tx.clone();
        let shutdown = shutdown.clone();
        let working_dir = working_dir_str.clone();
        let sid = session_id;
        let binary_path = resolve_pane_binary_path(
            provider,
            model.as_deref(),
            &claude_path,
            &codex_path,
            &opencode_path,
            &cursor_agent_path,
        );
        let (
            interrupt_slot,
            control_resp_slot,
            pending_qs,
            effort_arc,
            worktree_path,
            system_prompt,
            pr_mode_arc,
            pr_pending,
        ) = pane_metas
            .lock()
            .unwrap()
            .get(&pane_id)
            .map(|m| {
                (
                    m.streaming_interrupt_tx.clone(),
                    m.control_response_tx.clone(),
                    m.pending_questions.clone(),
                    m.effort_arc.clone(),
                    m.worktree_path.clone(),
                    crate::pane_identity::compose_system_prompt(
                        m.role.as_deref(),
                        m.goal.as_deref(),
                        m.backstory.as_deref(),
                    ),
                    m.plan_review_mode_arc.clone(),
                    m.pending_plan_reviews.clone(),
                )
            })
            .unwrap_or_else(|| {
                (
                    Arc::new(Mutex::new(None)),
                    Arc::new(Mutex::new(None)),
                    Arc::new(Mutex::new(HashMap::new())),
                    Arc::new(Mutex::new(None)),
                    None,
                    None,
                    Arc::new(Mutex::new(shared::PlanReviewMode::default())),
                    Arc::new(Mutex::new(HashMap::new())),
                )
            });
        pane_threads.push(thread::spawn(move || {
            run_pane_session(
                &binary_path,
                &working_dir,
                worktree_path,
                system_prompt,
                pr_mode_arc,
                pr_pending,
                sid,
                pane_session_id,
                pane_id,
                &provider,
                model.clone(),
                effort.clone(),
                input_rx,
                output_tx,
                server_tx,
                shutdown,
                child_proc,
                interrupt_slot,
                control_resp_slot,
                pending_qs,
                effort_arc,
                true,
            )
        }));
    }

    // TUI event handler thread — processes AddTab/CloseTab events
    // (runs in background, feeds events into the App via channels)
    let tui_event_thread = {
        let shutdown = shutdown.clone();
        let output_tx_event = output_tx.clone();
        let server_tx_event = server_tx.clone();
        let input_channels_event = input_channels.clone();
        let working_dir_event = working_dir_str.clone();
        let claude_path_event = claude_path.clone();
        let codex_path_event = codex_path.clone();
        let opencode_path_event = opencode_path.clone();
        let cursor_agent_path_event = cursor_agent_path.clone();
        let pane_sessions_event = pane_sessions.clone();
        let pane_pauses_event = pane_pauses.clone();
        let pane_stop_requests_event = pane_stop_requests.clone();
        let pane_metas_event = pane_metas.clone();
        let terminal_panes_event = terminal_panes.clone();
        let event_tx_event = event_tx.clone();
        let default_prompt_for_events = default_prompt.clone();
        let file_watcher_for_events = file_watcher.clone();
        let project_policy_for_events = project_policy.clone();
        thread::spawn(move || {
            handle_tui_events(
                event_rx,
                event_tx_event,
                shutdown,
                output_tx_event,
                server_tx_event,
                input_channels_event,
                session_id,
                &claude_path_event,
                &codex_path_event,
                &opencode_path_event,
                &cursor_agent_path_event,
                &working_dir_event,
                command_tx,
                pane_sessions_event,
                pane_pauses_event,
                pane_stop_requests_event,
                pane_metas_event,
                terminal_panes_event,
                &default_prompt_for_events,
                file_watcher_for_events,
                project_policy_for_events,
            )
        })
    };

    // Team panes (Manager / Tech Lead / Reviewer / Developer) are no
    // longer auto-spawned at boot. The user clicks "Start team" on the
    // Overview to trigger the same spawn logic — `spawn_missing_team_panes`
    // is the shared helper, called from the StartTeam wire handler. It's
    // idempotent (only spawns roles that aren't already present), so a
    // double-click is safe.

    // Phase 1.2b: auto-refresh diff poller. Scans pane_metas every few
    // seconds for panes with worktree_path set; when any one's branch tip
    // (HEAD in the worktree) has moved since the last tick, it recomputes
    // the diff and pushes a fresh PaneDiff over the wire so the web modal
    // updates live. Sleep tick is short — git rev-parse is cheap (one
    // file read) so polling is essentially free.
    {
        let pane_metas_for_poll = pane_metas.clone();
        let server_tx_for_poll = server_tx.clone();
        let shutdown_for_poll = shutdown.clone();
        let working_dir_for_poll = working_dir_str.clone();
        thread::spawn(move || {
            let mut state = crate::worktree::DiffPollState::new();
            let project = std::path::PathBuf::from(working_dir_for_poll);
            while !shutdown_for_poll.load(Ordering::SeqCst) {
                let panes: Vec<(u32, String)> = {
                    let metas = pane_metas_for_poll.lock().unwrap();
                    metas
                        .iter()
                        .filter_map(|(id, m)| m.worktree_path.as_ref().map(|wt| (*id, wt.clone())))
                        .collect()
                };
                let updates = crate::worktree::poll_changed_diffs(&project, &mut state, &panes);
                for (pane_id, branch, base, diff) in updates {
                    let _ = server_tx_for_poll.blocking_send(CliToServer::PaneDiff {
                        session_id,
                        pane_id,
                        branch: Some(branch),
                        base: Some(base),
                        diff: Some(diff),
                        error: None,
                    });
                }
                thread::sleep(Duration::from_secs(3));
            }
        });
    }
    // Project flag poller. Re-reads .apas every ~5s and pushes the
    // current `auto_approve_todos` / `auto_merge_prs` values upstream
    // so the web's Overview toggles hydrate on attach and survive a
    // server restart that wiped the in-memory cache. Cheap: .apas is
    // a few KB and only one message per 5s per active CLI.
    {
        let server_tx_for_flags = server_tx.clone();
        let shutdown_for_flags = shutdown.clone();
        let working_dir_for_flags = working_dir_str.clone();
        thread::spawn(move || {
            let project = std::path::PathBuf::from(working_dir_for_flags);
            while !shutdown_for_flags.load(Ordering::SeqCst) {
                if let Ok(meta) = crate::project::get_or_create_project(&project) {
                    let _ = server_tx_for_flags.blocking_send(CliToServer::ProjectFlagsChanged {
                        session_id,
                        auto_approve_todos: meta.auto_approve_todos,
                        auto_merge_prs: meta.auto_merge_prs,
                        team_enabled: meta.team_enabled,
                        disallowed_tab_types: meta.disallowed_tab_types.clone(),
                    });
                }
                thread::sleep(Duration::from_secs(5));
            }
        });
    }
    // Phase 2.2b: team scratchpad watcher. Tails `.apas-team.jsonl` and
    // pushes new records to the server (which forwards to web). On
    // first tick we send the existing history so newly-attached web
    // clients see what came before. Polls mtime+size — cheap; only
    // Terminal panes have no stream-json to observe, so their history is read
    // out of the provider's own transcript. Self-reporting via an MCP tool was
    // tried first and does not work: providers may connect to the server and
    // call the tool when told directly, but do not reliably act on MCP
    // `initialize` instructions, so an ordinary task recorded nothing.
    //
    // Polling rather than the file watcher: the watcher is mtime-gated for
    // deadloop wake-ups and reports only *that* something changed, while this
    // needs to know *how much* was appended so it can resume from a cursor.
    {
        let server_tx_for_turns = server_tx.clone();
        let terminal_questions_for_turns = terminal_questions.clone();
        let shutdown_for_turns = shutdown.clone();
        let project_for_turns = std::path::PathBuf::from(working_dir_str.clone());
        let working_dir_for_turns = working_dir_str.clone();
        let metas_for_turns = pane_metas.clone();
        let sessions_for_turns = pane_sessions.clone();
        let input_channels_for_turns = input_channels.clone();
        let pauses_for_turns = pane_pauses.clone();
        let stop_requests_for_turns = pane_stop_requests.clone();
        let connection_generation_for_turns = server_connection_generation.clone();
        #[cfg(target_os = "linux")]
        let terminal_panes_for_turns = terminal_panes.clone();
        let opencode_for_turns = opencode_path.clone();
        // Only panes restored above should suppress their existing history.
        // A pane created after this snapshot must capture from turn zero even
        // if it manages to finish its first turn before the watcher's first
        // three-second poll.
        let startup_terminal_panes: HashSet<u32> = pane_metas
            .lock()
            .map(|metas| {
                metas
                    .iter()
                    .filter_map(|(pane_id, meta)| meta.kind.is_terminal().then_some(*pane_id))
                    .collect()
            })
            .unwrap_or_default();
        thread::spawn(move || {
            let home = dirs::home_dir();
            // Per-pane cursor and selected transcript. Tracking the source as
            // part of the identity prevents a provider/session/worktree
            // change from applying an old file's cursor to a new transcript.
            let mut seen: HashMap<u32, (String, usize)> = HashMap::new();
            let mut seen_completions: HashMap<u32, usize> = HashMap::new();
            // Once a process-owned codex rollout is found, keep following it
            // through brief descriptor gaps. A newly observed open user
            // rollout replaces it when the TUI resumes or forks a session.
            let mut codex_paths: HashMap<u32, std::path::PathBuf> = HashMap::new();
            // Explicit task lifecycle is tracked independently from the turn
            // cursor so reconnect can restore an in-flight pane without
            // replaying its existing conversation.
            let mut codex_working: HashMap<u32, (bool, u64)> = HashMap::new();
            // Per-pane claude transcript watch state (growth + idle tracking
            // for in-TUI session-switch detection).
            let mut claude_watch: HashMap<u32, ClaudeWatchState> = HashMap::new();
            while !shutdown_for_turns.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_secs(3));

                // Snapshot terminal panes: (pane_id, provider, conversation
                // id, actual terminal cwd override, provider process group).
                let panes: Vec<(u32, Provider, Uuid, Option<String>, Option<i32>)> = {
                    let Ok(metas) = metas_for_turns.lock() else {
                        continue;
                    };
                    let sessions = sessions_for_turns.lock().ok();
                    #[cfg(target_os = "linux")]
                    let runtimes = terminal_panes_for_turns.lock().ok();
                    metas
                        .iter()
                        .filter(|(_, m)| m.kind.is_terminal())
                        .filter_map(|(id, m)| {
                            let sid = sessions.as_ref()?.get(id).copied()?;
                            #[cfg(target_os = "linux")]
                            let process_group_id = runtimes
                                .as_ref()
                                .and_then(|runtimes| runtimes.get(id))
                                .and_then(TerminalRuntimeHandle::process_group_id);
                            #[cfg(not(target_os = "linux"))]
                            let process_group_id = None;
                            Some((
                                *id,
                                m.provider.clone(),
                                sid,
                                m.worktree_path.clone(),
                                process_group_id,
                            ))
                        })
                        .collect()
                };

                for (pane_id, provider, mut conv_id, worktree_path, process_group_id) in panes {
                    let transcript_cwd =
                        terminal_transcript_cwd(&project_for_turns, worktree_path.as_deref());
                    let mut provider_working = None;
                    let (source, turns, verified_codex_session_id) = match provider {
                        Provider::Codex => {
                            let Some(home) = home.as_deref() else {
                                continue;
                            };
                            #[cfg(target_os = "linux")]
                            let process_path = process_group_id.and_then(|group| {
                                crate::transcript::find_codex_rollout_for_process_group(
                                    home,
                                    &transcript_cwd,
                                    group,
                                )
                            });
                            #[cfg(not(target_os = "linux"))]
                            let process_path = {
                                let _ = process_group_id;
                                None::<std::path::PathBuf>
                            };

                            let (path, process_owned) = if let Some(path) = process_path {
                                codex_paths.insert(pane_id, path.clone());
                                (path, true)
                            } else if let Some(path) = codex_paths.get(&pane_id) {
                                // Every cached Linux path entered through the
                                // process-group branch above, so retaining it
                                // across a brief descriptor gap retains the
                                // same ownership proof.
                                (path.clone(), cfg!(target_os = "linux"))
                            } else {
                                #[cfg(target_os = "linux")]
                                {
                                    // Wait for codex to open this process's
                                    // rollout. Falling back to cwd here is the
                                    // cross-pane mismatch this path prevents.
                                    continue;
                                }
                                #[cfg(not(target_os = "linux"))]
                                {
                                    let Some(path) = crate::transcript::find_codex_rollout(
                                        home,
                                        &transcript_cwd,
                                    ) else {
                                        continue;
                                    };
                                    codex_paths.insert(pane_id, path.clone());
                                    (path, false)
                                }
                            };
                            let verified_session_id = process_owned
                                .then(|| crate::transcript::codex_rollout_session_id(&path))
                                .flatten();
                            let Ok((turns, working)) =
                                crate::transcript::read_codex_snapshot(&path, pane_id)
                            else {
                                continue;
                            };
                            provider_working = working;
                            (
                                format!("codex:{}", path.display()),
                                turns,
                                verified_session_id,
                            )
                        }
                        Provider::Opencode => {
                            let Ok(Some(opencode_session_id)) =
                                crate::transcript::find_opencode_session(
                                    &opencode_for_turns,
                                    &transcript_cwd,
                                )
                            else {
                                continue;
                            };
                            let Ok(turns) = crate::transcript::read_opencode_turns(
                                &opencode_for_turns,
                                &transcript_cwd,
                                &opencode_session_id,
                                pane_id,
                            ) else {
                                continue;
                            };
                            (format!("opencode:{opencode_session_id}"), turns, None)
                        }
                        Provider::Claude => {
                            let Some(home) = home.as_deref() else {
                                continue;
                            };
                            // What Claude itself reported through its
                            // SessionStart hook, which is the only account that
                            // survives the user typing `/resume` onto another
                            // session or Claude moving into a worktree. Falls
                            // back to derivation when nothing has been reported
                            // — an older Claude, or a hook that could not run.
                            let pinned = crate::claude_session_hook::reported_transcript(
                                session_id, pane_id,
                            )
                            .unwrap_or_else(|| {
                                crate::transcript::locate_claude_transcript(
                                    home,
                                    &transcript_cwd,
                                    &conv_id.to_string(),
                                )
                            });
                            // Follow the conversation the user is actually in:
                            // an in-TUI /resume to another session writes to a
                            // different file in the same cwd slug directory.
                            // The pinned-set filter keeps sibling panes' pinned
                            // sessions (and this pane's own pinned session,
                            // while tracking an unpinned file) out of the
                            // candidate set appropriately.
                            let watch = claude_watch
                                .entry(pane_id)
                                .or_insert_with(|| ClaudeWatchState::new(pinned.clone()));
                            // A watch whose file has never appeared may be
                            // watching the wrong directory entirely — the
                            // provider relocated before writing its first turn.
                            // Re-point it at the pinned path once that resolves
                            // somewhere real, rather than sitting idle until the
                            // switch heuristic guesses.
                            // Re-point whenever the pinned path moves. When it
                            // comes from the hook that is Claude telling us the
                            // session changed, which no amount of watching the
                            // old file would reveal.
                            if watch.path != pinned && pinned.exists() {
                                tracing::info!(
                                    pane_id,
                                    from = %watch.path.display(),
                                    to = %pinned.display(),
                                    "claude terminal pane found its transcript in another directory"
                                );
                                *watch = ClaudeWatchState::new(pinned.clone());
                            }
                            let meta = std::fs::metadata(&watch.path).ok();
                            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                            let mtime = meta.as_ref().and_then(|m| m.modified().ok());
                            let grown = size != watch.size || mtime != watch.mtime;
                            watch.size = size;
                            watch.mtime = mtime;
                            if grown {
                                watch.idle_polls = 0;
                            } else {
                                watch.idle_polls += 1;
                            }
                            if watch.idle_polls >= CLAUDE_SWITCH_IDLE_POLLS {
                                let excluded: HashSet<String> = sessions_for_turns
                                    .lock()
                                    .ok()
                                    .map(|sessions| {
                                        sessions
                                            .iter()
                                            .filter(|(other_pane_id, _)| **other_pane_id != pane_id)
                                            .map(|(_, other_conv_id)| other_conv_id.to_string())
                                            .collect()
                                    })
                                    .unwrap_or_default();
                                if let Some(next) = crate::transcript::find_claude_switch_candidate(
                                    &watch.path,
                                    watch.switch_floor(),
                                    &excluded,
                                ) {
                                    tracing::info!(
                                        pane_id,
                                        from = %watch.path.display(),
                                        to = %next.display(),
                                        "claude terminal pane followed an in-TUI session switch"
                                    );
                                    *watch = ClaudeWatchState::new(next);
                                }
                            }
                            let path = watch.path.clone();
                            let Ok(turns) = crate::transcript::read_turns(&path, pane_id, false)
                            else {
                                continue;
                            };
                            (format!("claude:{}", path.display()), turns, None)
                        }
                        _ => continue,
                    };

                    if let Some(verified_session_id) = verified_codex_session_id {
                        if remember_verified_codex_session(
                            &sessions_for_turns,
                            pane_id,
                            conv_id,
                            verified_session_id,
                        ) {
                            tracing::info!(
                                pane_id,
                                previous_session_id = %conv_id,
                                codex_session_id = %verified_session_id,
                                "captured exact Codex terminal session identity"
                            );
                            conv_id = verified_session_id;
                            announce_and_persist_panes(
                                &server_tx_for_turns,
                                session_id,
                                &working_dir_for_turns,
                                &metas_for_turns,
                                &input_channels_for_turns,
                                &sessions_for_turns,
                                &pauses_for_turns,
                                &stop_requests_for_turns,
                            );
                        }
                    }

                    let completion_count = turns.iter().filter(|turn| turn.completes_work).count();
                    let restored_at_start = startup_terminal_panes.contains(&pane_id);
                    let (tracked_source, cursor) = seen.entry(pane_id).or_insert_with(|| {
                        let baseline =
                            initial_terminal_transcript_cursor(restored_at_start, turns.len());
                        (source.clone(), baseline)
                    });
                    let completed = seen_completions.entry(pane_id).or_insert_with(|| {
                        if restored_at_start {
                            completion_count
                        } else {
                            0
                        }
                    });
                    if *tracked_source != source {
                        // Baseline to the end of the new transcript rather than
                        // replaying it. A switch does not mean the turns are
                        // new: `codex resume` and thread forks open a file that
                        // already contains the prior conversation, so starting
                        // at 0 republishes all of it as if it had just been
                        // said. That is what flooded one session with 4 million
                        // duplicate messages and took the server out of memory.
                        // The cost of this choice is at most one poll interval
                        // of turns missed at a genuine switch, which is a far
                        // better failure than unbounded duplication.
                        *tracked_source = source;
                        *cursor = turns.len();
                        *completed = completion_count;
                    }
                    // A truncated or rotated transcript would otherwise leave
                    // the cursor past the end and stall the pane permanently.
                    if *cursor > turns.len() {
                        *cursor = turns.len();
                        *completed = completion_count;
                    }
                    let previous_cursor = *cursor;
                    let awaiting_answer = terminal_questions_for_turns
                        .lock()
                        .map(|mut pending| {
                            reconcile_terminal_questions_for_pane(&mut pending, pane_id, &turns)
                        })
                        .unwrap_or(false);
                    if turns.len() > previous_cursor {
                        for turn in &turns[previous_cursor..] {
                            for msg in
                                conversation_turn_to_stream_messages(turn, session_id, conv_id)
                            {
                                let _ = server_tx_for_turns.blocking_send(msg);
                            }
                        }
                        *cursor = turns.len();
                    }
                    if awaiting_answer {
                        // Repeated while unresolved so a server reconnect can
                        // recover the actionable state even though old chat
                        // turns are deliberately not replayed.
                        let _ = server_tx_for_turns.blocking_send(CliToServer::PaneStatus {
                            session_id,
                            pane_type: PaneType::Interactive,
                            pane_id: Some(pane_id),
                            status: Some(shared::PANE_STATUS_PENDING_ANSWER.to_string()),
                        });
                    }
                    // Codex writes task_complete after its assistant message.
                    // If the prior poll already advanced the turn cursor, no
                    // new StreamMessage exists to carry the completion marker,
                    // so send the status transition explicitly.
                    let completion_was_on_new_turn = turns
                        .get(previous_cursor..)
                        .is_some_and(|new_turns| new_turns.iter().any(|turn| turn.completes_work));
                    if completion_count > *completed && !completion_was_on_new_turn {
                        let _ = server_tx_for_turns.blocking_send(CliToServer::PaneStatus {
                            session_id,
                            pane_type: PaneType::Interactive,
                            pane_id: Some(pane_id),
                            status: None,
                        });
                    }
                    *completed = completion_count;

                    if let Some(working) = observe_codex_working_state(
                        &mut codex_working,
                        pane_id,
                        provider_working,
                        connection_generation_for_turns.load(Ordering::SeqCst),
                    ) {
                        let _ = server_tx_for_turns.blocking_send(CliToServer::PaneStatus {
                            session_id,
                            pane_type: PaneType::Interactive,
                            pane_id: Some(pane_id),
                            status: working.then(|| "Working...".to_string()),
                        });
                    }
                }
            }
        });
    }
    {
        // A worker's TUI channels used to be dropped because nobody read them.
        // They are now fanned out to attached controllers instead: the project
        // runs here, and a person's `apas` in this directory watches it rather
        // than starting a second CLI over the same `.apas` and worktrees.
        drop(tui_input_tx);
        match crate::supervisor::worker_paths(session_id) {
            Ok((socket, credential_path)) => {
                match crate::pane_host::random_credential().and_then(|credential| {
                    crate::pane_host::write_private(&credential_path, credential.as_bytes())?;
                    Ok(credential)
                }) {
                    Ok(credential) => {
                        let attachments = crate::attach::Attachments::new();
                        let snapshot_metas = pane_metas.clone();
                        let snapshot = move || {
                            snapshot_metas
                                .lock()
                                .map(|panes| {
                                    let mut tabs: Vec<crate::supervisor::AttachedTab> = panes
                                        .iter()
                                        .map(|(pane_id, meta)| crate::supervisor::AttachedTab {
                                            pane_id: *pane_id,
                                            label: meta.label.clone(),
                                            mode: meta.mode.clone(),
                                        })
                                        .collect();
                                    tabs.sort_by_key(|tab| tab.pane_id);
                                    tabs
                                })
                                .unwrap_or_default()
                        };
                        match crate::attach::serve(
                            &socket,
                            credential,
                            attachments.clone(),
                            snapshot,
                        ) {
                            Ok(()) => {
                                crate::attach::forward_channels(attachments, output_rx, command_rx);
                            }
                            Err(err) => {
                                // Not fatal: a project that cannot be attached
                                // to still runs, which is the part that matters.
                                tracing::warn!(%err, "worker attach socket unavailable");
                                drop(output_rx);
                                drop(command_rx);
                            }
                        }
                    }
                    Err(err) => {
                        tracing::warn!(%err, "worker attach credential unavailable");
                        drop(output_rx);
                        drop(command_rx);
                    }
                }
            }
            Err(err) => {
                tracing::warn!(%err, "worker runtime directory unavailable");
                drop(output_rx);
                drop(command_rx);
            }
        }
    }

    tracing::info!("Running project, waiting for server connection...");
    let _ = server_task.await;
    shutdown.store(true, Ordering::SeqCst);

    // Persist before killing: `.apas` is the only record of the pane roster,
    // and anything created since the last explicit save is lost otherwise.
    // Saving first also means we capture the state the user left, not whatever
    // the teardown leaves behind.
    save_pane_configs(
        &working_dir_str,
        &pane_sessions,
        &pane_metas,
        &pane_pauses,
        &pane_stop_requests,
    );

    // A project that stopped for a reason says so. Taken before terminal
    // cleanup so the reason survives whatever teardown does, and returned to
    // the caller rather than printed: once projects share a process, the
    // caller is the thing that knows how to report one project stopping.
    let stopped_because = stop_reason.lock().ok().and_then(|mut slot| slot.take());

    // An intentional project CLI stop owns terminal cleanup. Full CLI reboot
    // execs before this path, leaving host-backed providers available for the
    // replacement controller; ordinary stop reaches here and shuts them down.
    if let Ok(mut terminals) = terminal_panes.lock() {
        for handle in terminals.values() {
            handle.shutdown();
        }
        terminals.clear();
    }

    // Kill every pane's agent and everything it spawned. Without the group
    // kill, the agents' bash commands / subagents / mcp-servers outlive APAS.
    kill_all_pane_children(&pane_metas);

    // Wait for threads.
    for thread in pane_threads {
        let _ = thread.join();
    }
    let _ = tui_event_thread.join();

    // A reboot outranks a stop reason: the project is going to be replaced, so
    // why its previous incarnation ended is not what the caller needs to act on.
    if reboot_requested.load(Ordering::SeqCst) {
        return Ok(ProjectOutcome::RebootRequested);
    }
    Ok(match stopped_because {
        Some(reason) => ProjectOutcome::Stopped(reason),
        None => ProjectOutcome::Completed,
    })
}

/// Centralized input router: forwards TUI input to the correct pane via input_channels.
/// All panes (interactive, dynamic) register in input_channels and receive TUI input automatically.
fn spawn_centralized_input_router(
    tui_input_rx: mpsc::Receiver<(u32, String)>,
    input_channels: InputChannels,
    shutdown: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        while !shutdown.load(Ordering::SeqCst) {
            match tui_input_rx.recv_timeout(Duration::from_millis(100)) {
                Ok((pane_id, text)) => {
                    let channels = input_channels.lock().unwrap();
                    if let Some(tx) = channels.get(&pane_id) {
                        let _ = tx.send((text, true)); // from_tui=true
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}

/// Persist current pane configs (including dynamic tabs) to the .apas file
fn save_pane_configs(
    working_dir: &str,
    pane_sessions: &Arc<Mutex<HashMap<u32, Uuid>>>,
    pane_metas: &PaneMetas,
    pane_pauses: &PanePauses,
    pane_stop_requests: &PaneStopRequests,
) {
    if let Ok(mut metadata) = get_or_create_project(Path::new(working_dir)) {
        let pane_sessions = pane_sessions.lock().unwrap().clone();
        let pane_metas = pane_metas.lock().unwrap().clone();
        let paused: HashMap<u32, bool> = pane_pauses
            .lock()
            .unwrap()
            .iter()
            .map(|(&pane_id, flag)| (pane_id, flag.load(Ordering::SeqCst)))
            .collect();
        let stop_requested: HashMap<u32, bool> = pane_stop_requests
            .lock()
            .unwrap()
            .iter()
            .map(|(&pane_id, flag)| (pane_id, flag.load(Ordering::SeqCst)))
            .collect();

        // Rebuild panes list from pane_sessions and pane_metas
        let mut panes: Vec<shared::PaneConfig> = pane_sessions
            .iter()
            .map(|(&pane_id, &claude_sid)| {
                let (mode, provider, label, prompt, model, effort, min_iteration_interval_minutes) =
                    if let Some(meta) = pane_metas.get(&pane_id) {
                        (
                            meta.mode.clone(),
                            meta.provider.clone(),
                            meta.label.clone(),
                            meta.prompt.clone(),
                            meta.model.clone(),
                            meta.effort.clone(),
                            meta.min_iteration_interval_minutes,
                        )
                    } else if pane_id == shared::PANE_ID_DEADLOOP {
                        (
                            shared::PaneMode::Deadloop,
                            Provider::Claude,
                            default_pane_label(pane_id, None),
                            None,
                            None,
                            None,
                            Some(DEFAULT_MIN_ITERATION_INTERVAL_MINUTES),
                        )
                    } else {
                        (
                            shared::PaneMode::Interactive,
                            Provider::Claude,
                            default_pane_label(pane_id, None),
                            None,
                            None,
                            None,
                            None,
                        )
                    };
                let (
                    worktree_path,
                    role,
                    goal,
                    backstory,
                    plan_review_mode,
                    manual_mode,
                    managed,
                    kind,
                ) = pane_metas
                    .get(&pane_id)
                    .map(|p| {
                        (
                            p.worktree_path.clone(),
                            p.role.clone(),
                            p.goal.clone(),
                            p.backstory.clone(),
                            p.plan_review_mode,
                            p.manual_mode,
                            p.managed,
                            p.kind,
                        )
                    })
                    .unwrap_or((
                        None,
                        None,
                        None,
                        None,
                        shared::PlanReviewMode::default(),
                        false,
                        false,
                        shared::PaneKind::Agent,
                    ));
                shared::PaneConfig {
                    pane_id,
                    provider,
                    mode,
                    kind,
                    session_id: claude_sid,
                    is_paused: paused.get(&pane_id).copied().unwrap_or(false),
                    stop_requested: stop_requested.get(&pane_id).copied().unwrap_or(false),
                    prompt,
                    min_iteration_interval_minutes,
                    label: Some(pane_label_or_default(
                        Some(&label),
                        pane_id,
                        model.as_deref(),
                    )),
                    model,
                    effort,
                    worktree_path,
                    role,
                    goal,
                    backstory,
                    plan_review_mode,
                    manual_mode,
                    managed,
                }
            })
            .collect();
        panes.sort_by_key(|p| p.pane_id);
        metadata.is_paused = panes
            .iter()
            .find(|pane| pane.pane_id == shared::PANE_ID_DEADLOOP)
            .map(|pane| pane.is_paused)
            .unwrap_or(false);
        metadata.panes = panes;
        let _ = save_project(Path::new(working_dir), &metadata);
    }
}

/// Handle TUI events (AddTab, CloseTab) in a background thread
fn handle_tui_events(
    event_rx: mpsc::Receiver<TuiEvent>,
    event_tx: mpsc::Sender<TuiEvent>,
    shutdown: Arc<AtomicBool>,
    output_tx: mpsc::Sender<PaneOutput>,
    server_tx: tokio_mpsc::Sender<CliToServer>,
    input_channels: InputChannels,
    session_id: Uuid,
    claude_path: &str,
    codex_path: &str,
    opencode_path: &str,
    cursor_agent_path: &str,
    working_dir: &str,
    command_tx: mpsc::Sender<TuiCommand>,
    pane_sessions: Arc<Mutex<HashMap<u32, Uuid>>>,
    pane_pauses: PanePauses,
    pane_stop_requests: PaneStopRequests,
    pane_metas: PaneMetas,
    terminal_panes: TerminalPanes,
    default_prompt: &str,
    file_watcher: Arc<crate::file_watcher::ProjectFileWatcher>,
    project_policy: ProjectPolicyState,
) {
    while !shutdown.load(Ordering::SeqCst) {
        match event_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(TuiEvent::AddTab) => {
                // The local "+" path follows the same retirement rule as the
                // web: ordinary new work opens Claude's real terminal TUI.
                // Queue the canonical configured event so policy enforcement,
                // pty registration, announcements, and persistence stay in
                // one implementation.
                let pane_id = 3 + (Uuid::new_v4().as_u128() % 1000) as u32;
                let _ = event_tx.send(TuiEvent::AddTabWithConfig {
                    pane_id,
                    label: format!("Claude {}", pane_id),
                    claude_session_id: Uuid::new_v4(),
                    mode: shared::PaneMode::Interactive,
                    provider: Provider::Claude,
                    prompt: None,
                    min_iteration_interval_minutes: None,
                    model: None,
                    effort: None,
                    worktree_path: None,
                    initial_input: None,
                    role: None,
                    goal: None,
                    backstory: None,
                    plan_review_mode: shared::PlanReviewMode::default(),
                    managed: false,
                    try_resume_first: false,
                    kind: shared::PaneKind::Terminal,
                });
            }
            Ok(TuiEvent::AddTabWithConfig {
                pane_id,
                label: requested_label,
                claude_session_id,
                mode,
                provider,
                prompt,
                min_iteration_interval_minutes,
                model,
                effort,
                worktree_path,
                initial_input,
                role,
                goal,
                backstory,
                plan_review_mode,
                managed,
                try_resume_first,
                kind,
            }) => {
                if shared::is_retired_launch(provider, model.as_deref()) {
                    tracing::warn!(pane_id, "AddPane refused for retired provider");
                    let _ = output_tx.send(PaneOutput {
                        text: "[New tab refused — unsupported provider has been retired]"
                            .to_string(),
                        pane_id,
                    });
                    continue;
                }
                // Last-line launch enforcement. This applies equally to
                // managed panes and direct TUI events, and fails closed until
                // the server's effective policy has arrived.
                if (managed
                    && !project_policy
                        .lock()
                        .ok()
                        .and_then(|policy| policy.clone())
                        .is_some_and(|policy| policy.team_available))
                    || !launch_allowed_by_server_policy(
                        &project_policy,
                        kind,
                        provider,
                        model.as_deref(),
                    )
                {
                    let denied = shared::launch_profile_key(kind, provider, model.as_deref());
                    tracing::warn!(pane_id, %denied, "AddPane refused by cluster policy");
                    let _ = output_tx.send(PaneOutput {
                        text: format!(
                            "[New tab refused — launch profile {} is unavailable under the current cluster policy]",
                            denied
                        ),
                        pane_id,
                    });
                    continue;
                }
                let label =
                    pane_label_or_default(Some(&requested_label), pane_id, model.as_deref());
                // Effort is whatever the caller asked for. Managed Claude
                // panes used to default to max because a team should think
                // hard; there is no team, and a pane an older `.apas` marked
                // managed must not quietly cost more than the one beside it.
                let normalized_effort = normalize_effort_level(effort.as_deref());
                // Track claude session and metadata for this pane
                {
                    let mut ps = pane_sessions.lock().unwrap();
                    ps.insert(pane_id, claude_session_id);
                }

                let child_proc: Arc<Mutex<Option<std::process::Child>>> =
                    Arc::new(Mutex::new(None));
                {
                    let mut metas = pane_metas.lock().unwrap();
                    metas.insert(
                        pane_id,
                        PaneMeta {
                            kind,
                            mode: mode.clone(),
                            provider: provider.clone(),
                            label: label.clone(),
                            prompt: prompt.clone(),
                            model: model.clone(),
                            effort: normalized_effort.clone(),
                            min_iteration_interval_minutes,
                            child_process: child_proc.clone(),
                            streaming_interrupt_tx: Arc::new(Mutex::new(None)),
                            control_response_tx: Arc::new(Mutex::new(None)),
                            pending_questions: Arc::new(Mutex::new(HashMap::new())),
                            effort_arc: Arc::new(Mutex::new(normalized_effort.clone())),
                            worktree_path: worktree_path.clone(),
                            role: role.clone(),
                            goal: goal.clone(),
                            backstory: backstory.clone(),
                            plan_review_mode: plan_review_mode.clone(),
                            plan_review_mode_arc: Arc::new(Mutex::new(plan_review_mode.clone())),
                            pending_plan_reviews: Arc::new(Mutex::new(HashMap::new())),
                            manual_mode: false,
                            managed,
                        },
                    );
                }
                let binary_path = resolve_pane_binary_path(
                    provider,
                    model.as_deref(),
                    claude_path,
                    codex_path,
                    opencode_path,
                    cursor_agent_path,
                );

                // Notify TUI to add the tab visually
                let _ = command_tx.send(TuiCommand::AddTab {
                    pane_id,
                    label: label.clone(),
                    mode: mode.clone(),
                });

                let _ = output_tx.send(PaneOutput {
                    text: format!("[New tab from web: {}]", label),
                    pane_id,
                });

                // Terminal tab: allocate a pty running the provider's real
                // TUI and stop here. None of the agent machinery below
                // applies — no input channel, no deadloop, no system
                // prompt — so an early return keeps those paths honest
                // rather than threading `kind` through all of them.
                if kind.is_terminal() {
                    if let Err(err) = spawn_terminal_pane(
                        &terminal_panes,
                        pane_id,
                        session_id,
                        claude_session_id,
                        &provider,
                        model.as_deref(),
                        &binary_path,
                        working_dir,
                        worktree_path.as_deref(),
                        &server_tx,
                        try_resume_first,
                        initial_input.as_deref(),
                    ) {
                        tracing::error!(pane_id, %err, "failed to start terminal pane");
                        let _ = output_tx.send(PaneOutput { text: err, pane_id });
                    }
                    // Announce before returning. This early return used to skip
                    // the shared tail below, so a new terminal tab never
                    // reached the web — it only appeared once something else
                    // provoked a PaneList, e.g. switching projects and back.
                    // The pane is already in `pane_metas` (inserted above), so
                    // the list is correct; it was simply never sent.
                    announce_and_persist_panes(
                        &server_tx,
                        session_id,
                        working_dir,
                        &pane_metas,
                        &input_channels,
                        &pane_sessions,
                        &pane_pauses,
                        &pane_stop_requests,
                    );
                    continue;
                }

                if mode == shared::PaneMode::Deadloop {
                    // Deadloop's input_tx lives inside the streaming worker
                    // and isn't registered in input_channels, so we can't
                    // replay a queued input here. Log + drop so we know it
                    // happened.
                    if initial_input.is_some() {
                        tracing::warn!(
                            pane_id,
                            "Dropping buffered input on recreated deadloop pane (no external input channel)",
                        );
                    }
                    // Deadloop tab: spawn deadloop session with its own pause flag
                    let pause_flag = Arc::new(AtomicBool::new(false));
                    let stop_flag = Arc::new(AtomicBool::new(false));
                    {
                        let mut pauses = pane_pauses.lock().unwrap();
                        pauses.insert(pane_id, pause_flag.clone());
                    }
                    {
                        let mut stop_requests = pane_stop_requests.lock().unwrap();
                        stop_requests.insert(pane_id, stop_flag.clone());
                    }
                    let dl_prompt = prompt.unwrap_or_else(|| default_prompt.to_string());
                    let resolved_min_interval_minutes = min_iteration_interval_minutes
                        .unwrap_or(DEFAULT_MIN_ITERATION_INTERVAL_MINUTES);
                    let output_tx = output_tx.clone();
                    let server_tx = server_tx.clone();
                    let shutdown = shutdown.clone();
                    let event_tx = event_tx.clone();
                    let working_dir = working_dir.to_string();
                    let input_channels_for_dl = input_channels.clone();
                    let (
                        interrupt_slot,
                        control_resp_slot,
                        pending_qs,
                        effort_arc,
                        worktree_path,
                        system_prompt,
                        pr_mode_arc,
                        pr_pending,
                    ) = pane_metas
                        .lock()
                        .unwrap()
                        .get(&pane_id)
                        .map(|m| {
                            (
                                m.streaming_interrupt_tx.clone(),
                                m.control_response_tx.clone(),
                                m.pending_questions.clone(),
                                m.effort_arc.clone(),
                                m.worktree_path.clone(),
                                crate::pane_identity::compose_system_prompt(
                                    m.role.as_deref(),
                                    m.goal.as_deref(),
                                    m.backstory.as_deref(),
                                ),
                                m.plan_review_mode_arc.clone(),
                                m.pending_plan_reviews.clone(),
                            )
                        })
                        .unwrap_or_else(|| {
                            (
                                Arc::new(Mutex::new(None)),
                                Arc::new(Mutex::new(None)),
                                Arc::new(Mutex::new(HashMap::new())),
                                Arc::new(Mutex::new(None)),
                                None,
                                None,
                                Arc::new(Mutex::new(shared::PlanReviewMode::default())),
                                Arc::new(Mutex::new(HashMap::new())),
                            )
                        });
                    let file_watcher_for_dl = file_watcher.clone();
                    thread::spawn(move || {
                        run_deadloop_session(
                            &binary_path,
                            &working_dir,
                            worktree_path,
                            system_prompt,
                            pr_mode_arc,
                            pr_pending,
                            session_id,
                            claude_session_id,
                            pane_id,
                            &dl_prompt,
                            model.clone(),
                            normalized_effort.clone(),
                            resolved_min_interval_minutes,
                            &provider,
                            output_tx,
                            server_tx,
                            shutdown,
                            pause_flag,
                            stop_flag,
                            child_proc,
                            event_tx,
                            interrupt_slot,
                            control_resp_slot,
                            pending_qs,
                            effort_arc,
                            input_channels_for_dl,
                            try_resume_first,
                            file_watcher_for_dl,
                        )
                    });
                } else {
                    // Interactive tab: spawn interactive session
                    let (input_tx, input_rx) = mpsc::channel::<PaneInput>();
                    {
                        let mut channels = input_channels.lock().unwrap();
                        channels.insert(pane_id, input_tx.clone());
                    }
                    // Bugfix: if the user just typed something at a pane
                    // whose worker hadn't registered an input channel yet
                    // (typical right after a CLI restart), replay it now
                    // so they don't have to retype.
                    if let Some(text) = initial_input.clone() {
                        if input_tx.send((text, false)).is_ok() {
                            tracing::info!(
                                pane_id,
                                "Auto-replayed buffered input on recreated interactive pane",
                            );
                        }
                    }
                    let output_tx = output_tx.clone();
                    let server_tx = server_tx.clone();
                    let shutdown = shutdown.clone();
                    let working_dir = working_dir.to_string();
                    let (
                        interrupt_slot,
                        control_resp_slot,
                        pending_qs,
                        effort_arc,
                        worktree_path,
                        system_prompt,
                        pr_mode_arc,
                        pr_pending,
                    ) = pane_metas
                        .lock()
                        .unwrap()
                        .get(&pane_id)
                        .map(|m| {
                            (
                                m.streaming_interrupt_tx.clone(),
                                m.control_response_tx.clone(),
                                m.pending_questions.clone(),
                                m.effort_arc.clone(),
                                m.worktree_path.clone(),
                                crate::pane_identity::compose_system_prompt(
                                    m.role.as_deref(),
                                    m.goal.as_deref(),
                                    m.backstory.as_deref(),
                                ),
                                m.plan_review_mode_arc.clone(),
                                m.pending_plan_reviews.clone(),
                            )
                        })
                        .unwrap_or_else(|| {
                            (
                                Arc::new(Mutex::new(None)),
                                Arc::new(Mutex::new(None)),
                                Arc::new(Mutex::new(HashMap::new())),
                                Arc::new(Mutex::new(None)),
                                None,
                                None,
                                Arc::new(Mutex::new(shared::PlanReviewMode::default())),
                                Arc::new(Mutex::new(HashMap::new())),
                            )
                        });
                    thread::spawn(move || {
                        run_pane_session(
                            &binary_path,
                            &working_dir,
                            worktree_path,
                            system_prompt,
                            pr_mode_arc,
                            pr_pending,
                            session_id,
                            claude_session_id,
                            pane_id,
                            &provider,
                            model.clone(),
                            normalized_effort.clone(),
                            input_rx,
                            output_tx,
                            server_tx,
                            shutdown,
                            child_proc,
                            interrupt_slot,
                            control_resp_slot,
                            pending_qs,
                            effort_arc,
                            try_resume_first,
                        )
                    });
                }

                announce_and_persist_panes(
                    &server_tx,
                    session_id,
                    working_dir,
                    &pane_metas,
                    &input_channels,
                    &pane_sessions,
                    &pane_pauses,
                    &pane_stop_requests,
                );
            }
            Ok(TuiEvent::CloseTab {
                pane_id,
                cleanup_action,
            }) => {
                // Remove input channel (causes interactive session thread to exit)
                {
                    let mut channels = input_channels.lock().unwrap();
                    channels.remove(&pane_id);
                }

                // For deadloop panes, signal shutdown by setting pause and removing pause flag
                // (the deadloop thread checks shutdown flag which will eventually stop it,
                // but we also kill the child process for immediate effect)
                {
                    let mut pauses = pane_pauses.lock().unwrap();
                    pauses.remove(&pane_id);
                }
                {
                    let mut stop_requests = pane_stop_requests.lock().unwrap();
                    stop_requests.remove(&pane_id);
                }
                // Kill the pty if this is a terminal pane. Closing from the web
                // already did this in the `RemovePane` handler (the remove here
                // then finds nothing); closing from the TUI arrives straight
                // here, and a terminal pane has no `child_process` entry — so
                // without this its claude/codex would keep running with nothing
                // left reading or writing it.
                let removed_terminal = terminal_panes
                    .lock()
                    .ok()
                    .and_then(|mut m| m.remove(&pane_id));
                if let Some(handle) = removed_terminal {
                    tracing::info!(pane_id, "shutting down terminal pane on tab close");
                    handle.shutdown();
                }
                let worktree_path: Option<String> = {
                    let metas = pane_metas.lock().unwrap();
                    let path = metas.get(&pane_id).and_then(|m| m.worktree_path.clone());
                    if let Some(meta) = metas.get(&pane_id) {
                        shutdown_pane_child(&meta.child_process);
                    }
                    path
                };

                // Remove from pane sessions and metas
                {
                    let mut ps = pane_sessions.lock().unwrap();
                    ps.remove(&pane_id);
                }
                {
                    let mut metas = pane_metas.lock().unwrap();
                    metas.remove(&pane_id);
                }

                // Notify TUI to remove the tab visually
                let _ = command_tx.send(TuiCommand::RemoveTab { pane_id });

                let _ = output_tx.send(PaneOutput {
                    text: format!("[Tab {} closed]", pane_id),
                    pane_id,
                });

                // If the pane owned an isolated worktree AND the caller asked
                // for cleanup, run the requested action. Errors are reported
                // back via the chat stream — we don't fail the close itself.
                if let (Some(path), Some(action)) = (worktree_path.as_deref(), cleanup_action) {
                    let cleanup_msg =
                        match crate::worktree::cleanup_on_close(working_dir, path, action) {
                            Ok(text) => text,
                            Err(err) => format!("[Worktree cleanup failed: {}]", err),
                        };
                    let _ = output_tx.send(PaneOutput {
                        text: cleanup_msg,
                        pane_id,
                    });
                }

                announce_and_persist_panes(
                    &server_tx,
                    session_id,
                    working_dir,
                    &pane_metas,
                    &input_channels,
                    &pane_sessions,
                    &pane_pauses,
                    &pane_stop_requests,
                );
            }
            Ok(TuiEvent::StartBot {
                pane_id,
                prompt,
                min_iteration_interval_minutes,
                effort,
            }) => {
                // Preserve provider/model and any existing per-pane prompt across mode switches.
                let (
                    provider,
                    existing_label,
                    existing_prompt,
                    existing_model,
                    existing_effort,
                    existing_min_interval_minutes,
                    existing_kind,
                    preserved_fields,
                ) = {
                    let metas = pane_metas.lock().unwrap();
                    match metas.get(&pane_id) {
                        Some(meta) => (
                            meta.provider,
                            meta.label.clone(),
                            meta.prompt.clone(),
                            meta.model.clone(),
                            meta.effort.clone(),
                            meta.min_iteration_interval_minutes,
                            meta.kind,
                            start_bot_preserved_fields(Some(meta)),
                        ),
                        None => (
                            Provider::Claude,
                            default_pane_label(pane_id, None),
                            None,
                            None,
                            None,
                            None,
                            shared::PaneKind::Agent,
                            start_bot_preserved_fields(None),
                        ),
                    }
                };
                let resolved_prompt = prompt.filter(|p| !p.trim().is_empty()).or(existing_prompt);
                let resolved_effort = if let Some(requested_effort) = effort.as_deref() {
                    normalize_effort_level(Some(requested_effort))
                } else {
                    normalize_effort_level(existing_effort.as_deref())
                };
                let resolved_min_interval_minutes = min_iteration_interval_minutes
                    .or(existing_min_interval_minutes)
                    .unwrap_or(DEFAULT_MIN_ITERATION_INTERVAL_MINUTES);

                if shared::is_retired_launch(provider, existing_model.as_deref()) {
                    tracing::warn!(pane_id, "StartBot refused for retired provider");
                    let _ = output_tx.send(PaneOutput {
                        text: "[Start refused — unsupported provider has been retired]".to_string(),
                        pane_id,
                    });
                    continue;
                }

                // The allowlist is not consulted for a pane that already
                // exists — it decides what may be created. Team availability
                // still is: it is the switch that stops a running team, and
                // honouring it here is what stops a disabled team coming back
                // one pane at a time.
                if !existing_pane_relaunch_allowed() {
                    let denied = shared::launch_profile_key(
                        existing_kind,
                        provider,
                        existing_model.as_deref(),
                    );
                    tracing::warn!(pane_id, %denied, "StartBot refused by cluster policy");
                    let _ = output_tx.send(PaneOutput {
                        text: format!(
                            "[Start refused — launch profile {} is unavailable under the current cluster policy]",
                            denied
                        ),
                        pane_id,
                    });
                    continue;
                }

                // Kill any old child process and wait for it to fully exit
                // so the Claude session ID is released before we --resume.
                // This covers both the Deadloop→StartBot and the
                // Stop→FinalizeStopBot→StartBot paths (where mode is
                // already Interactive but the old process may still linger).
                {
                    let metas = pane_metas.lock().unwrap();
                    if let Some(meta) = metas.get(&pane_id) {
                        if meta.mode == shared::PaneMode::Deadloop {
                            // Signal old deadloop thread to exit
                            if let Some(old_flag) = pane_stop_requests.lock().unwrap().get(&pane_id)
                            {
                                old_flag.store(true, Ordering::SeqCst);
                            }
                        }
                        // Kill and wait regardless of mode
                        if let Ok(mut guard) = meta.child_process.lock() {
                            if let Some(child) = guard.take() {
                                let mut child = child;
                                let _ = child.kill();
                                let _ = child.wait();
                            }
                        }
                    }
                }

                // Convert interactive pane to deadloop:
                // 1. Remove input channel (kills interactive session thread)
                {
                    let mut channels = input_channels.lock().unwrap();
                    channels.remove(&pane_id);
                }

                // 2. Create pause flag and child process for the deadloop
                let pause_flag = Arc::new(AtomicBool::new(false));
                let stop_flag = Arc::new(AtomicBool::new(false));
                let child_proc: Arc<Mutex<Option<std::process::Child>>> =
                    Arc::new(Mutex::new(None));
                {
                    let mut pauses = pane_pauses.lock().unwrap();
                    pauses.insert(pane_id, pause_flag.clone());
                }
                {
                    let mut stop_requests = pane_stop_requests.lock().unwrap();
                    stop_requests.insert(pane_id, stop_flag.clone());
                }
                {
                    let mut metas = pane_metas.lock().unwrap();
                    metas.insert(
                        pane_id,
                        PaneMeta {
                            kind: shared::PaneKind::Agent,
                            mode: shared::PaneMode::Deadloop,
                            provider,
                            label: existing_label,
                            prompt: resolved_prompt.clone(),
                            model: existing_model.clone(),
                            effort: resolved_effort.clone(),
                            min_iteration_interval_minutes: Some(resolved_min_interval_minutes),
                            child_process: child_proc.clone(),
                            streaming_interrupt_tx: Arc::new(Mutex::new(None)),
                            control_response_tx: Arc::new(Mutex::new(None)),
                            pending_questions: Arc::new(Mutex::new(HashMap::new())),
                            effort_arc: Arc::new(Mutex::new(resolved_effort.clone())),
                            worktree_path: preserved_fields.worktree_path,
                            role: preserved_fields.role,
                            goal: preserved_fields.goal,
                            backstory: preserved_fields.backstory,
                            plan_review_mode: preserved_fields.plan_review_mode,
                            plan_review_mode_arc: Arc::new(Mutex::new(
                                preserved_fields.plan_review_mode,
                            )),
                            pending_plan_reviews: Arc::new(Mutex::new(HashMap::new())),
                            manual_mode: preserved_fields.manual_mode,
                            managed: preserved_fields.managed,
                        },
                    );
                }

                // 3. Notify TUI to update tab mode
                let _ = command_tx.send(TuiCommand::SetMode {
                    pane_id,
                    mode: shared::PaneMode::Deadloop,
                });

                let _ = output_tx.send(PaneOutput {
                    text: "[Bot started on this pane]".to_string(),
                    pane_id,
                });

                // 4. Reuse claude session id for --resume history continuity
                let claude_session_id = {
                    let ps = pane_sessions.lock().unwrap();
                    ps.get(&pane_id).copied().unwrap_or_else(Uuid::new_v4)
                };

                // 5. Spawn deadloop session
                let dl_prompt = resolved_prompt.unwrap_or_else(|| default_prompt.to_string());
                let binary_path = resolve_pane_binary_path(
                    provider,
                    existing_model.as_deref(),
                    claude_path,
                    codex_path,
                    opencode_path,
                    cursor_agent_path,
                );
                {
                    let output_tx = output_tx.clone();
                    let server_tx = server_tx.clone();
                    let shutdown = shutdown.clone();
                    let event_tx = event_tx.clone();
                    let working_dir = working_dir.to_string();
                    let input_channels_for_dl = input_channels.clone();
                    let (
                        interrupt_slot,
                        control_resp_slot,
                        pending_qs,
                        effort_arc,
                        worktree_path,
                        system_prompt,
                        pr_mode_arc,
                        pr_pending,
                    ) = pane_metas
                        .lock()
                        .unwrap()
                        .get(&pane_id)
                        .map(|m| {
                            (
                                m.streaming_interrupt_tx.clone(),
                                m.control_response_tx.clone(),
                                m.pending_questions.clone(),
                                m.effort_arc.clone(),
                                m.worktree_path.clone(),
                                crate::pane_identity::compose_system_prompt(
                                    m.role.as_deref(),
                                    m.goal.as_deref(),
                                    m.backstory.as_deref(),
                                ),
                                m.plan_review_mode_arc.clone(),
                                m.pending_plan_reviews.clone(),
                            )
                        })
                        .unwrap_or_else(|| {
                            (
                                Arc::new(Mutex::new(None)),
                                Arc::new(Mutex::new(None)),
                                Arc::new(Mutex::new(HashMap::new())),
                                Arc::new(Mutex::new(None)),
                                None,
                                None,
                                Arc::new(Mutex::new(shared::PlanReviewMode::default())),
                                Arc::new(Mutex::new(HashMap::new())),
                            )
                        });
                    let file_watcher_for_dl = file_watcher.clone();
                    thread::spawn(move || {
                        run_deadloop_session(
                            &binary_path,
                            &working_dir,
                            worktree_path,
                            system_prompt,
                            pr_mode_arc,
                            pr_pending,
                            session_id,
                            claude_session_id,
                            pane_id,
                            &dl_prompt,
                            existing_model.clone(),
                            resolved_effort.clone(),
                            resolved_min_interval_minutes,
                            &provider,
                            output_tx,
                            server_tx,
                            shutdown,
                            pause_flag,
                            stop_flag,
                            child_proc,
                            event_tx,
                            interrupt_slot,
                            control_resp_slot,
                            pending_qs,
                            effort_arc,
                            input_channels_for_dl,
                            true,
                            file_watcher_for_dl,
                        )
                    });
                }

                // 6. Send updated pane list
                let _ = server_tx.blocking_send(CliToServer::PaneList {
                    session_id,
                    panes: build_pane_list(
                        &pane_metas,
                        &input_channels,
                        session_id,
                        &pane_sessions,
                        &pane_pauses,
                        &pane_stop_requests,
                    ),
                });

                save_pane_configs(
                    working_dir,
                    &pane_sessions,
                    &pane_metas,
                    &pane_pauses,
                    &pane_stop_requests,
                );
            }
            Ok(TuiEvent::StopBot { pane_id }) => {
                let pane_mode = {
                    let metas = pane_metas.lock().unwrap();
                    let Some(meta) = metas.get(&pane_id) else {
                        continue;
                    };
                    meta.mode.clone()
                };

                // For interactive panes: prefer the soft control_request
                // (preserves the long-lived streaming claude — same path as
                // InterruptPane), fall back to kill if the streaming
                // interrupt channel is absent (non-streaming providers) or
                // the worker is dead.
                if pane_mode == shared::PaneMode::Interactive {
                    let (soft_interrupt, _child_present): (Option<mpsc::Sender<()>>, bool) = {
                        let metas = pane_metas.lock().unwrap();
                        match metas.get(&pane_id) {
                            Some(m) => {
                                let soft = m
                                    .streaming_interrupt_tx
                                    .lock()
                                    .ok()
                                    .and_then(|g| g.as_ref().cloned());
                                let present = m
                                    .child_process
                                    .lock()
                                    .ok()
                                    .map(|g| g.is_some())
                                    .unwrap_or(false);
                                (soft, present)
                            }
                            None => (None, false),
                        }
                    };
                    let mut soft_sent = false;
                    if let Some(tx) = soft_interrupt {
                        if tx.send(()).is_ok() {
                            soft_sent = true;
                            tracing::info!(
                                pane_id,
                                "StopBot(Interactive): signaled streaming worker for soft interrupt",
                            );
                        }
                    }
                    if !soft_sent {
                        let metas = pane_metas.lock().unwrap();
                        if let Some(meta) = metas.get(&pane_id) {
                            if let Ok(mut guard) = meta.child_process.lock() {
                                if let Some(ref mut child) = *guard {
                                    let _ = child.kill();
                                }
                            }
                        }
                    }
                    let msg = if soft_sent {
                        "[Interrupted current turn — process still alive.]"
                    } else {
                        "[Process killed]"
                    };
                    let _ = output_tx.send(PaneOutput {
                        text: msg.to_string(),
                        pane_id,
                    });
                    let _ = server_tx.blocking_send(CliToServer::StreamMessage {
                        session_id,
                        message: shared::ClaudeStreamMessage::Result {
                            subtype: "text".to_string(),
                            result: msg.to_string(),
                            is_error: false,
                            total_cost_usd: 0.0,
                            duration_ms: 0,
                            session_id: session_id.to_string(),
                            extra: serde_json::Value::Null,
                        },
                        pane_type: None,
                        pane_id: Some(pane_id),
                    });
                    continue;
                }

                // Check if stop is already requested (two-stage stop)
                let already_requested = {
                    let stop_requests = pane_stop_requests.lock().unwrap();
                    stop_requests
                        .get(&pane_id)
                        .map(|f| f.load(Ordering::SeqCst))
                        .unwrap_or(false)
                };

                if !already_requested {
                    // === Stage 1: Graceful stop ===
                    // Set the stop_requested flag; deadloop will stop after current iteration
                    {
                        let stop_requests = pane_stop_requests.lock().unwrap();
                        if let Some(flag) = stop_requests.get(&pane_id) {
                            flag.store(true, Ordering::SeqCst);
                        }
                    }

                    let _ = output_tx.send(PaneOutput {
                        text: "[Stop requested — will stop after current work finishes...]"
                            .to_string(),
                        pane_id,
                    });

                    // Notify web clients
                    let _ = server_tx.blocking_send(CliToServer::StreamMessage {
                        session_id,
                        message: shared::ClaudeStreamMessage::Result {
                            subtype: "text".to_string(),
                            result: "[Stop requested — will stop after current work finishes...]"
                                .to_string(),
                            is_error: false,
                            total_cost_usd: 0.0,
                            duration_ms: 0,
                            session_id: session_id.to_string(),
                            extra: serde_json::Value::Null,
                        },
                        pane_type: None,
                        pane_id: Some(pane_id),
                    });

                    // Send updated PaneList so web sees stop_requested=true
                    let _ = server_tx.blocking_send(CliToServer::PaneList {
                        session_id,
                        panes: build_pane_list(
                            &pane_metas,
                            &input_channels,
                            session_id,
                            &pane_sessions,
                            &pane_pauses,
                            &pane_stop_requests,
                        ),
                    });

                    // Persist stop_requested immediately so a crash/restart before
                    // FinalizeStopBot does not resurrect bot mode.
                    save_pane_configs(
                        working_dir,
                        &pane_sessions,
                        &pane_metas,
                        &pane_pauses,
                        &pane_stop_requests,
                    );
                } else {
                    // === Stage 2: Force stop ===
                    // Soft path first: send control_request("interrupt") to
                    // the streaming worker. The current turn aborts, claude
                    // emits Result, the deadloop driver sees stop_requested
                    // is already true, and the loop unwinds — without losing
                    // the long-lived claude process. SIGKILL fallback if the
                    // streaming channel is absent (non-streaming provider)
                    // or send fails (worker dead). The 3-second escalation
                    // ensures a wedged turn that ignores control_request
                    // still gets killed.
                    let (soft_interrupt, child_pid): (Option<mpsc::Sender<()>>, Option<u32>) = {
                        let metas = pane_metas.lock().unwrap();
                        match metas.get(&pane_id) {
                            Some(m) => {
                                let soft = m
                                    .streaming_interrupt_tx
                                    .lock()
                                    .ok()
                                    .and_then(|g| g.as_ref().cloned());
                                let pid = m
                                    .child_process
                                    .lock()
                                    .ok()
                                    .and_then(|g| g.as_ref().map(|c| c.id()));
                                (soft, pid)
                            }
                            None => (None, None),
                        }
                    };
                    let mut soft_sent = false;
                    if let Some(tx) = soft_interrupt {
                        if tx.send(()).is_ok() {
                            soft_sent = true;
                            tracing::info!(
                                pane_id,
                                "StopBot(Force,Deadloop): signaled streaming worker for soft interrupt",
                            );
                            // Escalate to SIGKILL after 3s if the process is
                            // still alive — covers the case where the turn
                            // is genuinely wedged below the stream-json layer
                            // (e.g. claude itself isn't reading stdin).
                            if let Some(pid) = child_pid {
                                let pane_for_log = pane_id;
                                std::thread::spawn(move || {
                                    std::thread::sleep(Duration::from_secs(3));
                                    let alive =
                                        std::path::Path::new(&format!("/proc/{}", pid)).exists();
                                    if alive {
                                        tracing::warn!(
                                            pane_id = pane_for_log,
                                            pid,
                                            "StopBot(Force): soft interrupt didn't take in 3s, escalating to SIGKILL",
                                        );
                                        let _ = std::process::Command::new("kill")
                                            .arg("-KILL")
                                            .arg(pid.to_string())
                                            .status();
                                    }
                                });
                            }
                        }
                    }
                    if !soft_sent {
                        let metas = pane_metas.lock().unwrap();
                        if let Some(meta) = metas.get(&pane_id) {
                            if let Ok(mut guard) = meta.child_process.lock() {
                                if let Some(child) = guard.take() {
                                    let mut child = child;
                                    let _ = child.kill();
                                    let _ = child.wait();
                                }
                            }
                        }
                    }

                    // Reuse FinalizeStopBot logic — pass current stop_flag so
                    // the handler can verify it belongs to THIS deadloop.
                    let current_stop_flag = {
                        let stops = pane_stop_requests.lock().unwrap();
                        stops.get(&pane_id).cloned()
                    };
                    if let Some(sf) = current_stop_flag {
                        let _ = event_tx.send(TuiEvent::FinalizeStopBot {
                            pane_id,
                            stop_flag: sf,
                        });
                    }
                }
            }
            Ok(TuiEvent::FinalizeStopBot { pane_id, stop_flag }) => {
                // Finalize stop: switch from deadloop to interactive mode.
                // Called after deadloop finishes gracefully OR after force-kill.
                //
                // IMPORTANT: Verify that the stop_flag belongs to the CURRENT
                // deadloop for this pane. A stale FinalizeStopBot from an old
                // deadloop thread (that was still cleaning up when a new
                // StartBot spawned a replacement) must be ignored. Otherwise
                // it would kill the new deadloop and orphan its thread.
                {
                    let stops = pane_stop_requests.lock().unwrap();
                    if let Some(current) = stops.get(&pane_id) {
                        if !Arc::ptr_eq(&stop_flag, current) {
                            // This FinalizeStopBot is from an old deadloop — ignore it.
                            continue;
                        }
                    }
                    // If there's no entry, the flags were already cleaned up
                    // (e.g. pane was removed). Still safe to proceed.
                }

                let (
                    provider,
                    saved_label,
                    saved_prompt,
                    saved_model,
                    saved_effort,
                    saved_min_interval_minutes,
                ) = {
                    let metas = pane_metas.lock().unwrap();
                    let Some(meta) = metas.get(&pane_id) else {
                        continue;
                    };
                    if meta.mode != shared::PaneMode::Deadloop {
                        continue;
                    }
                    (
                        meta.provider,
                        meta.label.clone(),
                        meta.prompt.clone(),
                        meta.model.clone(),
                        meta.effort.clone(),
                        meta.min_iteration_interval_minutes,
                    )
                };

                // Remove deadloop control flags.
                {
                    let mut pauses = pane_pauses.lock().unwrap();
                    pauses.remove(&pane_id);
                }
                {
                    let mut stop_requests = pane_stop_requests.lock().unwrap();
                    stop_requests.remove(&pane_id);
                }

                // Create input channel for interactive mode.
                let (input_tx, input_rx) = mpsc::channel::<PaneInput>();
                {
                    let mut channels = input_channels.lock().unwrap();
                    channels.insert(pane_id, input_tx);
                }

                // Update pane meta to interactive.
                let child_proc: Arc<Mutex<Option<std::process::Child>>> =
                    Arc::new(Mutex::new(None));
                {
                    let mut metas = pane_metas.lock().unwrap();
                    metas.insert(
                        pane_id,
                        PaneMeta {
                            kind: shared::PaneKind::Agent,
                            mode: shared::PaneMode::Interactive,
                            provider,
                            label: saved_label,
                            prompt: saved_prompt,
                            model: saved_model.clone(),
                            effort: saved_effort.clone(),
                            min_iteration_interval_minutes: saved_min_interval_minutes,
                            child_process: child_proc.clone(),
                            streaming_interrupt_tx: Arc::new(Mutex::new(None)),
                            control_response_tx: Arc::new(Mutex::new(None)),
                            pending_questions: Arc::new(Mutex::new(HashMap::new())),
                            effort_arc: Arc::new(Mutex::new(saved_effort.clone())),
                            worktree_path: None,
                            role: None,
                            goal: None,
                            backstory: None,
                            plan_review_mode: shared::PlanReviewMode::default(),
                            plan_review_mode_arc: Arc::new(Mutex::new(
                                shared::PlanReviewMode::default(),
                            )),
                            pending_plan_reviews: Arc::new(Mutex::new(HashMap::new())),
                            manual_mode: false,
                            managed: false,
                        },
                    );
                }

                // Notify TUI to update tab mode.
                let _ = command_tx.send(TuiCommand::SetMode {
                    pane_id,
                    mode: shared::PaneMode::Interactive,
                });

                let _ = output_tx.send(PaneOutput {
                    text: "[Bot stopped — switched to interactive mode]".to_string(),
                    pane_id,
                });

                // Notify web clients
                let _ = server_tx.blocking_send(CliToServer::StreamMessage {
                    session_id,
                    message: shared::ClaudeStreamMessage::Result {
                        subtype: "text".to_string(),
                        result: "[Bot stopped — switched to interactive mode]".to_string(),
                        is_error: false,
                        total_cost_usd: 0.0,
                        duration_ms: 0,
                        session_id: session_id.to_string(),
                        extra: serde_json::Value::Null,
                    },
                    pane_type: None,
                    pane_id: Some(pane_id),
                });

                // Get session id for this pane.
                let claude_session_id = {
                    let ps = pane_sessions.lock().unwrap();
                    ps.get(&pane_id).copied().unwrap_or_else(Uuid::new_v4)
                };

                // Spawn interactive session.
                let binary_path = resolve_pane_binary_path(
                    provider,
                    saved_model.as_deref(),
                    claude_path,
                    codex_path,
                    opencode_path,
                    cursor_agent_path,
                );
                {
                    let output_tx = output_tx.clone();
                    let server_tx = server_tx.clone();
                    let shutdown = shutdown.clone();
                    let working_dir = working_dir.to_string();
                    let (
                        interrupt_slot,
                        control_resp_slot,
                        pending_qs,
                        effort_arc,
                        worktree_path,
                        system_prompt,
                        pr_mode_arc,
                        pr_pending,
                    ) = pane_metas
                        .lock()
                        .unwrap()
                        .get(&pane_id)
                        .map(|m| {
                            (
                                m.streaming_interrupt_tx.clone(),
                                m.control_response_tx.clone(),
                                m.pending_questions.clone(),
                                m.effort_arc.clone(),
                                m.worktree_path.clone(),
                                crate::pane_identity::compose_system_prompt(
                                    m.role.as_deref(),
                                    m.goal.as_deref(),
                                    m.backstory.as_deref(),
                                ),
                                m.plan_review_mode_arc.clone(),
                                m.pending_plan_reviews.clone(),
                            )
                        })
                        .unwrap_or_else(|| {
                            (
                                Arc::new(Mutex::new(None)),
                                Arc::new(Mutex::new(None)),
                                Arc::new(Mutex::new(HashMap::new())),
                                Arc::new(Mutex::new(None)),
                                None,
                                None,
                                Arc::new(Mutex::new(shared::PlanReviewMode::default())),
                                Arc::new(Mutex::new(HashMap::new())),
                            )
                        });
                    thread::spawn(move || {
                        run_pane_session(
                            &binary_path,
                            &working_dir,
                            worktree_path,
                            system_prompt,
                            pr_mode_arc,
                            pr_pending,
                            session_id,
                            claude_session_id,
                            pane_id,
                            &provider,
                            saved_model.clone(),
                            saved_effort.clone(),
                            input_rx,
                            output_tx,
                            server_tx,
                            shutdown,
                            child_proc,
                            interrupt_slot,
                            control_resp_slot,
                            pending_qs,
                            effort_arc,
                            true,
                        )
                    });
                }

                // Publish updated pane list and persist config.
                let _ = server_tx.blocking_send(CliToServer::PaneList {
                    session_id,
                    panes: build_pane_list(
                        &pane_metas,
                        &input_channels,
                        session_id,
                        &pane_sessions,
                        &pane_pauses,
                        &pane_stop_requests,
                    ),
                });

                save_pane_configs(
                    working_dir,
                    &pane_sessions,
                    &pane_metas,
                    &pane_pauses,
                    &pane_stop_requests,
                );
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// Build a PaneConfig list from pane metadata and input channels
fn build_pane_list(
    pane_metas: &PaneMetas,
    input_channels: &InputChannels,
    session_id: Uuid,
    pane_sessions: &Arc<Mutex<HashMap<u32, Uuid>>>,
    pane_pauses: &PanePauses,
    pane_stop_requests: &PaneStopRequests,
) -> Vec<shared::PaneConfig> {
    let metas = pane_metas.lock().unwrap();
    let channels = input_channels.lock().unwrap();
    let ps = pane_sessions.lock().unwrap();
    let pauses = pane_pauses.lock().unwrap();
    let stops = pane_stop_requests.lock().unwrap();
    let mut panes = Vec::new();

    // Build from metas (covers deadloop panes which don't have input channels)
    for (&pane_id, meta) in metas.iter() {
        let claude_sid = ps.get(&pane_id).copied().unwrap_or(session_id);
        let label = pane_label_or_default(Some(&meta.label), pane_id, meta.model.as_deref());
        let is_paused = pauses
            .get(&pane_id)
            .map(|f| f.load(Ordering::SeqCst))
            .unwrap_or(false);
        let stop_requested = stops
            .get(&pane_id)
            .map(|f| f.load(Ordering::SeqCst))
            .unwrap_or(false);
        panes.push(shared::PaneConfig {
            pane_id,
            provider: meta.provider.clone(),
            mode: meta.mode.clone(),
            kind: meta.kind,
            session_id: claude_sid,
            is_paused,
            stop_requested,
            prompt: meta.prompt.clone(),
            min_iteration_interval_minutes: meta.min_iteration_interval_minutes,
            label: Some(label),
            model: meta.model.clone(),
            effort: meta.effort.clone(),
            worktree_path: meta.worktree_path.clone(),
            role: meta.role.clone(),
            goal: meta.goal.clone(),
            backstory: meta.backstory.clone(),
            plan_review_mode: meta.plan_review_mode,
            manual_mode: meta.manual_mode,
            managed: meta.managed,
        });
    }

    // Also include any interactive panes that are in input_channels but not in metas
    for &pane_id in channels.keys() {
        if !metas.contains_key(&pane_id) {
            let claude_sid = ps.get(&pane_id).copied().unwrap_or(session_id);
            panes.push(shared::PaneConfig {
                pane_id,
                provider: shared::Provider::Claude,
                mode: shared::PaneMode::Interactive,
                // Panes recovered from `input_channels` alone are always
                // agent panes — terminal panes never register an input
                // channel (see `TerminalPanes`).
                kind: shared::PaneKind::Agent,
                session_id: claude_sid,
                is_paused: false,
                stop_requested: false,
                prompt: None,
                min_iteration_interval_minutes: None,
                label: Some(default_pane_label(pane_id, None)),
                model: None,
                effort: None,
                worktree_path: None,
                role: None,
                goal: None,
                backstory: None,
                plan_review_mode: shared::PlanReviewMode::default(),
                manual_mode: false,
                managed: false,
            });
        }
    }

    panes.sort_by_key(|p| p.pane_id);
    panes
}

#[allow(deprecated)]
fn active_usage_providers(pane_metas: &PaneMetas) -> (bool, bool, bool) {
    let metas = pane_metas.lock().unwrap();
    let mut has_claude = false;
    let mut has_codex = false;
    let mut has_deepseek = false;

    let looks_like_deepseek_label = |label: &str| {
        let normalized = label.trim().to_ascii_lowercase();
        normalized.contains("deepseek")
    };

    for meta in metas.values() {
        match meta.provider {
            Provider::Claude if shared::is_retired_model(meta.model.as_deref()) => {}
            // DeepSeek tabs likewise.
            Provider::Claude
                if is_deepseek_model(meta.model.as_deref())
                    || looks_like_deepseek_label(&meta.label) =>
            {
                has_deepseek = true
            }
            Provider::Claude => has_claude = true,
            Provider::Codex => has_codex = true,
            Provider::Minimax | Provider::Glm => {}
            Provider::Deepseek => has_deepseek = true,
            Provider::Opencode => {}
            Provider::CursorAgent => {}
        }
        if has_claude && has_codex && has_deepseek {
            break;
        }
    }

    (has_claude, has_codex, has_deepseek)
}

/// Kill any OS processes whose command line contains the given session ID.
/// This ensures the Claude session lock is released before we `--resume`.
fn kill_processes_using_session(session_id: &str) {
    let my_pid = std::process::id().to_string();
    if let Ok(output) = Command::new("pgrep").args(["-f", session_id]).output() {
        if let Ok(stdout) = std::str::from_utf8(&output.stdout) {
            for line in stdout.lines() {
                let pid = line.trim();
                if !pid.is_empty() && pid != my_pid {
                    let _ = Command::new("kill").args(["-9", pid]).output();
                }
            }
        }
    }
    // Brief pause to let the OS fully release the session
    thread::sleep(Duration::from_millis(500));
}

/// Build CLI arguments based on provider, session state, and prompt.
/// Returns (args, is_using_resume).
fn build_agent_args(
    provider: &Provider,
    session_id: &Uuid,
    prompt: &str,
    model: Option<&str>,
    effort: Option<&str>,
    first_message: bool,
    try_resume: bool,
) -> (Vec<String>, bool) {
    if shared::is_retired_launch(*provider, model) {
        return (Vec::new(), false);
    }
    match provider {
        Provider::Claude | Provider::Deepseek => {
            let mut base = vec![
                "--print".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--verbose".to_string(),
                "--dangerously-skip-permissions".to_string(),
            ];
            if let Some(model) = model {
                let trimmed = model.trim();
                // DeepSeek panes use dedicated backend env configuration.
                // Keep model selection in env
                // (ANTHROPIC_MODEL etc), not CLI flags.
                if !trimmed.is_empty() && !is_deepseek_model(Some(trimmed)) {
                    base.extend_from_slice(&["--model".to_string(), trimmed.to_string()]);
                }
            }
            if matches!(provider, Provider::Claude) && !is_deepseek_model(model) {
                if let Some(normalized_effort) = normalize_effort_level(effort) {
                    let claude_flag = effort_to_claude_flag(&normalized_effort).to_string();
                    tracing::info!(
                        target: "apas::effort",
                        level = %normalized_effort,
                        claude_flag = %claude_flag,
                        "Launching claude with --effort",
                    );
                    base.extend_from_slice(&["--effort".to_string(), claude_flag]);
                }
            }
            if first_message && try_resume {
                let mut args = base;
                args.extend_from_slice(&[
                    "--resume".to_string(),
                    session_id.to_string(),
                    prompt.to_string(),
                ]);
                (args, true)
            } else if first_message {
                let mut args = base;
                args.extend_from_slice(&[
                    "--session-id".to_string(),
                    session_id.to_string(),
                    prompt.to_string(),
                ]);
                (args, false)
            } else {
                let mut args = base;
                args.extend_from_slice(&[
                    "--resume".to_string(),
                    session_id.to_string(),
                    prompt.to_string(),
                ]);
                (args, true)
            }
        }
        Provider::Codex => {
            // Codex uses subcommands: `codex exec --json ...` or `codex exec resume --json ... <session_id> <prompt>`
            let mut base_flags = vec![
                "--json".to_string(),
                "--dangerously-bypass-approvals-and-sandbox".to_string(),
                "--skip-git-repo-check".to_string(),
            ];
            // Per-pane model + reasoning effort. Codex re-execs each turn, so
            // these ride every spawn/resume and take effect on the next turn
            // (no live-swap protocol like claude's apply_flag_settings). `-m`
            // sets the model; effort goes through codex's `-c` config override
            // `model_reasoning_effort=<low|medium|high|xhigh|max|ultra>`.
            if let Some(m) = model.map(str::trim).filter(|s| !s.is_empty()) {
                base_flags.push("--model".to_string());
                base_flags.push(m.to_string());
            }
            if let Some(level) = normalize_codex_effort(effort) {
                base_flags.push("-c".to_string());
                base_flags.push(format!("model_reasoning_effort={level}"));
            }
            if first_message && try_resume {
                let mut args = vec!["exec".to_string(), "resume".to_string()];
                args.extend(base_flags);
                args.push(session_id.to_string());
                args.push(prompt.to_string());
                (args, true)
            } else if first_message {
                // New session — just exec with prompt
                let mut args = vec!["exec".to_string()];
                args.extend(base_flags);
                args.push(prompt.to_string());
                (args, false)
            } else {
                // Subsequent — always resume
                let mut args = vec!["exec".to_string(), "resume".to_string()];
                args.extend(base_flags);
                args.push(session_id.to_string());
                args.push(prompt.to_string());
                (args, true)
            }
        }
        Provider::CursorAgent => {
            // cursor-agent uses: cursor-agent --print --output-format stream-json --force [--model m] [--continue] <prompt>
            let mut base = vec![
                "--print".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--force".to_string(),
            ];
            if let Some(model) = model {
                let trimmed = model.trim();
                if !trimmed.is_empty() {
                    base.extend_from_slice(&["--model".to_string(), trimmed.to_string()]);
                }
            }
            if first_message && try_resume {
                // We don't track cursor's internal chatId; fall back to --continue
                base.push("--continue".to_string());
                base.push(prompt.to_string());
                (base, true)
            } else if first_message {
                base.push(prompt.to_string());
                (base, false)
            } else {
                base.push("--continue".to_string());
                base.push(prompt.to_string());
                (base, true)
            }
        }
        #[allow(deprecated)]
        Provider::Minimax | Provider::Glm => (Vec::new(), false),
        Provider::Opencode => {
            // OpenCode generates its own `ses_*` identifiers, so the APAS
            // UUID cannot be passed to --session. Resume the newest session
            // in this process cwd instead. `--auto` is the documented
            // non-interactive permission mode and still honors explicit deny
            // rules from the user's OpenCode configuration.
            let mut base = vec![
                "run".to_string(),
                "--format".to_string(),
                "json".to_string(),
                "--auto".to_string(),
            ];
            if let Some(model) = model {
                let trimmed = model.trim();
                if !trimmed.is_empty() {
                    base.extend_from_slice(&["-m".to_string(), trimmed.to_string()]);
                }
            }
            let resume = !first_message || try_resume;
            if resume {
                base.push("--continue".to_string());
            }
            base.extend_from_slice(&["--".to_string(), prompt.to_string()]);
            (base, resume)
        }
    }
}

fn build_deadloop_agent_args(
    provider: &Provider,
    session_id: &Uuid,
    prompt: &str,
    model: Option<&str>,
    effort: Option<&str>,
    first_message: bool,
    try_resume_first: bool,
) -> (Vec<String>, bool) {
    build_agent_args(
        provider,
        session_id,
        prompt,
        model,
        effort,
        first_message,
        try_resume_first,
    )
}

fn is_codex_stale_session_error(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.contains("no rollout found") || lower.contains("thread/resume failed")
}

fn should_recover_deadloop_stale_session(
    stale_session_detected: bool,
    first_message: bool,
    using_resume: bool,
    exit_was_error: bool,
    had_error: bool,
) -> bool {
    stale_session_detected || (first_message && using_resume && exit_was_error && !had_error)
}

fn reset_deadloop_codex_stale_session(
    first_message: &mut bool,
    try_resume_first: &mut bool,
    claude_session_id: &mut Uuid,
    fresh_session_id: Uuid,
) -> Uuid {
    let old = *claude_session_id;
    *claude_session_id = fresh_session_id;
    *try_resume_first = false;
    *first_message = true;
    old
}

/// Intercept claude's `control_request` envelopes on stdout. Returns
/// `Some(true)` if the line was a control_request (caller should `continue`),
/// `Some(false)` if it's a control_request we explicitly leave for downstream
/// (unused today), or `None` if the line isn't a control_request at all.
///
/// Wire format mirrors `@anthropic-ai/claude-agent-sdk` v0.3.x. AskUserQuestion
/// is parked in `pending_questions` so the AnswerQuestion path can recover the
/// `request_id` and original questions when the user submits answers. Every
/// other `can_use_tool` request is auto-approved — in `bypassPermissions` mode
/// these are rare (claude pre-clears most tools itself) but harmless to
/// rubber-stamp.
fn try_handle_control_request(
    line: &str,
    pane_id: u32,
    session_id: Uuid,
    _pane_type: PaneType,
    pending_questions: &Arc<Mutex<HashMap<String, PendingAskQuestion>>>,
    control_response_tx: &mpsc::Sender<String>,
    server_tx: &tokio_mpsc::Sender<CliToServer>,
    // Phase 3.2b2: parking lot for held tool_uses. None when caller is a
    // legacy/test path that doesn't track plan reviews; otherwise the
    // PaneMeta's pending_plan_reviews Arc.
    pending_plan_reviews: Option<&Arc<Mutex<HashMap<String, PendingPlanReview>>>>,
    plan_review_mode: shared::PlanReviewMode,
) -> Option<bool> {
    // Cheap pre-filter: control_request lines always start with the
    // `{"type":"control_request"` prefix. Anything else short-circuits.
    if !line.contains("\"type\":\"control_request\"") {
        return None;
    }
    let value: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return None,
    };
    if value.get("type").and_then(|v| v.as_str()) != Some("control_request") {
        return None;
    }
    let request_id = value
        .get("request_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let request = match value.get("request") {
        Some(r) => r,
        None => return Some(true),
    };
    let subtype = request
        .get("subtype")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if subtype != "can_use_tool" {
        tracing::debug!(
            pane_id,
            subtype,
            "ignoring unhandled control_request subtype",
        );
        return Some(true);
    }
    let tool_name = request
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let tool_use_id = request
        .get("tool_use_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let input = request
        .get("input")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    if tool_name == "AskUserQuestion" {
        // Park the request so the AnswerQuestion handler can echo back the
        // original questions array and the matching request_id when the
        // user submits. The tool_use block itself is forwarded via the
        // regular assistant stream — the web UI renders the question card
        // from that block, not from a separate notification.
        let questions = input
            .get("questions")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        if let Ok(mut map) = pending_questions.lock() {
            map.insert(
                tool_use_id.clone(),
                PendingAskQuestion {
                    request_id,
                    questions,
                },
            );
        }
        tracing::info!(
            pane_id,
            tool_use_id = tool_use_id.as_str(),
            "AskUserQuestion parked; waiting for web answer",
        );
        return Some(true);
    }

    // Phase 3.2b2: ask the policy whether to hold this tool for user
    // review. If yes — park, push a PlanReviewRequest upstream, and
    // return without writing a control_response (the answer handler
    // will write it when the user clicks). If no, fall through to
    // auto-approve below.
    if crate::plan_review::should_hold_tool(plan_review_mode, &tool_name) {
        if let Some(park) = pending_plan_reviews {
            if let Ok(mut map) = park.lock() {
                map.insert(
                    tool_use_id.clone(),
                    PendingPlanReview {
                        request_id: request_id.clone(),
                        input: input.clone(),
                    },
                );
            }
            let _ = server_tx.try_send(CliToServer::PlanReviewRequest {
                session_id,
                pane_id,
                tool_use_id: tool_use_id.clone(),
                tool_name: tool_name.clone(),
                input: input.clone(),
            });
            tracing::info!(
                pane_id,
                tool = tool_name.as_str(),
                tool_use_id = tool_use_id.as_str(),
                mode = ?plan_review_mode,
                "plan review: tool held; awaiting user verdict",
            );
            return Some(true);
        }
        // No park slot (legacy/test path) — fall through to allow so
        // we never deadlock the turn just because plumbing is missing.
        tracing::warn!(
            pane_id,
            tool = tool_name.as_str(),
            "plan review: should_hold_tool=true but no parking slot; auto-approving",
        );
    }

    // Non-AskUserQuestion permission prompt slipped through bypass mode.
    // Auto-approve so the turn doesn't stall.
    let response = serde_json::json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": {
                "behavior": "allow",
                "updatedInput": input,
                "toolUseID": tool_use_id,
            }
        }
    });
    let _ = control_response_tx.send(response.to_string());
    tracing::debug!(
        pane_id,
        tool = tool_name.as_str(),
        "auto-approved non-AskUserQuestion permission prompt",
    );
    Some(true)
}

/// Intercept `control_response` envelopes that claude emits in reply to
/// our own `control_request`s — specifically the `apas-effort-*` ones we
/// queue when the user changes the effort dropdown. Returns true if the
/// line was a control_response we recognized (caller should `continue`).
/// Non-effort control_responses (success acks for other apas-issued
/// requests, if any) are also swallowed so they don't leak into
/// parse_agent_output, but only the effort ones produce chat output.
fn try_handle_control_response(
    line: &str,
    pane_id: u32,
    session_id: Uuid,
    pane_type: PaneType,
    server_tx: &tokio_mpsc::Sender<CliToServer>,
) -> bool {
    if !line.contains("\"type\":\"control_response\"") {
        return false;
    }
    let value: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if value.get("type").and_then(|v| v.as_str()) != Some("control_response") {
        return false;
    }
    let response = match value.get("response") {
        Some(r) => r,
        None => return true, // structurally a control_response but malformed; swallow
    };
    let request_id = response
        .get("request_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !request_id.starts_with("apas-effort-") {
        // Some other apas-issued control_request — let it be swallowed
        // (it's not a stream message anyway), but no chat feedback.
        return true;
    }
    let subtype = response
        .get("subtype")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let text = if subtype == "success" {
        "[✓ Effort change confirmed by claude]".to_string()
    } else {
        let err = response
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        format!("[✗ Effort change rejected by claude: {}]", err)
    };
    let msg = CliToServer::Output {
        session_id,
        data: text,
        output_type: shared::OutputType::System,
        pane_type: Some(pane_type),
        pane_id: Some(pane_id),
    };
    let _ = server_tx.blocking_send(msg);
    tracing::info!(
        pane_id,
        request_id,
        subtype,
        "received control_response for apas-effort request",
    );
    true
}

fn convert_opencode_to_claude(line: &str, session_id_str: &str) -> Option<ClaudeStreamMessage> {
    let event = serde_json::from_str::<serde_json::Value>(line).ok()?;
    match event.get("type").and_then(serde_json::Value::as_str)? {
        "text" => {
            let part = event.get("part")?;
            let text = part.get("text").and_then(serde_json::Value::as_str)?;
            if text.trim().is_empty() {
                return None;
            }
            Some(ClaudeStreamMessage::Assistant {
                message: shared::ClaudeAssistantMessage {
                    content: vec![ClaudeContentBlock::Text {
                        text: text.to_string(),
                    }],
                    model: "opencode".to_string(),
                    extra: serde_json::Value::Null,
                },
                session_id: session_id_str.to_string(),
                extra: serde_json::Value::Null,
            })
        }
        "tool_use" => {
            let part = event.get("part")?;
            let state = part.get("state").unwrap_or(&serde_json::Value::Null);
            Some(ClaudeStreamMessage::Assistant {
                message: shared::ClaudeAssistantMessage {
                    content: vec![ClaudeContentBlock::ToolUse {
                        id: part
                            .get("id")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("opencode-tool")
                            .to_string(),
                        name: part
                            .get("tool")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("tool")
                            .to_string(),
                        input: state
                            .get("input")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null),
                    }],
                    model: "opencode".to_string(),
                    extra: serde_json::Value::Null,
                },
                session_id: session_id_str.to_string(),
                extra: serde_json::Value::Null,
            })
        }
        "step_finish" => {
            let part = event.get("part").unwrap_or(&serde_json::Value::Null);
            // OpenCode emits a step_finish after a tool-call step as well as
            // after the final assistant step. Only the latter completes the
            // APAS turn; otherwise the working indicator would clear while
            // OpenCode is still executing tools.
            if matches!(
                part.get("reason").and_then(serde_json::Value::as_str),
                Some("tool-calls" | "tool_calls" | "tool_use")
            ) {
                return None;
            }
            let tokens = part
                .get("tokens")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let input = tokens
                .get("input")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default();
            let output = tokens
                .get("output")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default();
            Some(ClaudeStreamMessage::Result {
                subtype: "success".to_string(),
                result: String::new(),
                total_cost_usd: part
                    .get("cost")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or_default(),
                duration_ms: 0,
                session_id: session_id_str.to_string(),
                is_error: false,
                extra: serde_json::json!({
                    "usage": {
                        "input_tokens": input,
                        "output_tokens": output
                    }
                }),
            })
        }
        "error" => Some(ClaudeStreamMessage::Result {
            subtype: "error".to_string(),
            result: event
                .get("error")
                .map(serde_json::Value::to_string)
                .unwrap_or_else(|| "OpenCode failed".to_string()),
            total_cost_usd: 0.0,
            duration_ms: 0,
            session_id: session_id_str.to_string(),
            is_error: true,
            extra: serde_json::Value::Null,
        }),
        _ => None,
    }
}

/// Parse a line of output and convert to ClaudeStreamMessage based on provider.
/// Codex and OpenCode use their documented JSON event formats and are adapted
/// into the shared renderer shape.
fn parse_agent_output(
    provider: &Provider,
    line: &str,
    session_id_str: &str,
) -> Option<ClaudeStreamMessage> {
    match provider {
        Provider::Claude | Provider::Deepseek => {
            serde_json::from_str::<ClaudeStreamMessage>(line).ok()
        }
        Provider::Codex => match serde_json::from_str::<CodexStreamMessage>(line) {
            Ok(codex_msg) => shared::convert_codex_to_claude(&codex_msg, session_id_str),
            Err(_) => None,
        },
        Provider::Opencode => convert_opencode_to_claude(line, session_id_str),
        Provider::CursorAgent => {
            // cursor-agent --output-format stream-json emits Claude-compatible events
            serde_json::from_str::<ClaudeStreamMessage>(line).ok()
        }
        #[allow(deprecated)]
        Provider::Minimax | Provider::Glm => None,
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::{
        active_usage_providers, announce_and_persist_panes, append_codex_sqlite_home_env,
        auto_cancel_pending_questions_for_new_input, boot_restore_try_resume_first,
        build_agent_args, build_agent_switch_respawn_event, build_deadloop_agent_args,
        build_pane_env_overrides_from_keys, build_pane_list, build_pane_reboot_events,
        build_user_envelope_line, canonicalize_requested_model, codex_sqlite_home_path,
        convert_opencode_to_claude, deadloop_wait_plan, ensure_private_codex_sqlite_home,
        evaluate_deadloop_watchdog, is_codex_stale_session_error, manual_create_pr_worktree_path,
        model_switch_can_use_fast_path, normalize_codex_effort, normalize_effort_level,
        reconcile_terminal_questions_for_pane, remember_verified_codex_session,
        reset_deadloop_codex_stale_session, resolve_pane_binary_path, resolve_terminal_binary,
        restored_pane_mode_and_pause, retired_launch_rejection_output, route_web_input_to_pane,
        run_deadloop_session_inner, save_pane_configs, should_recover_deadloop_stale_session,
        start_bot_preserved_fields, stop_retired_panes, terminal_state_reports,
        truncate_str_at_char_boundary, update_project_operations, DeadloopWatchdogDecision,
        DeadloopWatchdogState, InputChannels, PaneInputRouteResult, PaneMeta, PaneMetas,
        PanePauses, PaneStopRequests, PendingAskQuestion, ASK_USER_QUESTION_AUTO_CANCEL_STATUS,
        CODEX_SQLITE_HOME_ENV, DEEPSEEK_DEFAULT_MODEL, DEEPSEEK_PRO_RUNTIME_MODEL,
    };
    use crate::conversation::{TurnAnswer, TurnQuestion, TurnRecord};
    use crate::project::{get_or_create_project, save_project};
    use crate::terminal_pane::TerminalPanes;
    use crate::tui::{PaneOutput, TuiEvent};
    use base64::Engine as _;
    use shared::{ClaudeContentBlock, ClaudeStreamMessage, CliToServer, Provider};
    use std::collections::{HashMap, HashSet};
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime};
    use tokio::sync::mpsc as tokio_mpsc;
    use uuid::Uuid;

    const FULL_PROMPT: &str =
        "Work on tasks defined in TODO.md.\n1. Analyze\n2. Implement\n3. Test";
    const WEB_PROVIDER_OPTIONS_TS: &str =
        include_str!("../../../../packages/web/src/lib/providerOptions.ts");

    /// Loading a project must not *create* a managed pane. Two boot-time
    /// migrations used to: one promoted any pane whose role mentioned
    /// "manager", "tech lead" or "reviewer", and another demoted duplicates of
    /// those roles. A person naming a pane "reviewer" in the role modal would
    /// have had it silently marked managed.
    #[test]
    fn loading_a_project_never_marks_a_pane_managed() {
        let dir = tempfile::tempdir().expect("temp project");
        let mut metadata = get_or_create_project(dir.path()).expect("create project");
        for (pane_id, role) in [(11, "reviewer"), (12, "tech lead"), (13, "team manager")] {
            let mut pane = test_pane_config(pane_id, shared::PaneMode::Interactive, false, false);
            pane.role = Some(role.to_string());
            pane.managed = false;
            metadata.panes.push(pane);
        }
        save_project(dir.path(), &metadata).expect("save project");

        let reloaded = get_or_create_project(dir.path()).expect("reload project");
        for pane in reloaded.panes.iter().filter(|p| p.pane_id >= 11) {
            assert!(
                !pane.managed,
                "pane {} named {:?} must stay unmanaged",
                pane.pane_id, pane.role
            );
        }
    }

    /// A project written while team mode existed still holds panes marked
    /// managed, with roles and built-in prompts. Nothing dispatches them now,
    /// so the only requirement is that the project loads and they come back as
    /// ordinary panes rather than being dropped or refused.
    #[test]
    fn a_stored_managed_pane_loads_as_an_ordinary_pane() {
        let dir = tempfile::tempdir().expect("temp project");
        let mut metadata = get_or_create_project(dir.path()).expect("create project");
        let mut managed = shared::PaneConfig {
            pane_id: 178,
            provider: Provider::Claude,
            mode: shared::PaneMode::Deadloop,
            kind: shared::PaneKind::Agent,
            session_id: Uuid::new_v4(),
            is_paused: false,
            stop_requested: false,
            prompt: Some("You are this project's Tech Lead...".to_string()),
            min_iteration_interval_minutes: None,
            label: Some("Tech Lead".to_string()),
            model: None,
            effort: None,
            worktree_path: None,
            role: Some("tech lead".to_string()),
            goal: Some("Orchestrate the team.".to_string()),
            backstory: None,
            plan_review_mode: shared::PlanReviewMode::default(),
            manual_mode: false,
            managed: true,
        };
        managed.label = Some("Tech Lead".to_string());
        metadata.panes.push(managed);
        save_project(dir.path(), &metadata).expect("save project");

        let reloaded = get_or_create_project(dir.path()).expect("reload project");
        let pane = reloaded
            .panes
            .iter()
            .find(|pane| pane.pane_id == 178)
            .expect("the pane survives the load");
        // Its role and prompt are ordinary pane metadata now.
        assert_eq!(pane.role.as_deref(), Some("tech lead"));
        assert!(super::existing_pane_relaunch_allowed());
    }

    /// A pane that exists relaunches, full stop. The allowlist governs what may
    /// be created, and team availability — which used to gate a managed pane —
    /// no longer governs anything, so a pane marked managed by an older `.apas`
    /// is simply a pane.
    #[test]
    fn any_existing_pane_relaunches() {
        assert!(super::existing_pane_relaunch_allowed());
    }

    #[test]
    fn deadloop_wait_cursor_is_sampled_at_wait_entry() {
        let last_started_at = Instant::now();
        let self_write_at = last_started_at + Duration::from_millis(20);
        let wait_entry_at = last_started_at + Duration::from_millis(50);
        let min_interval = Duration::from_secs(10);

        let plan = deadloop_wait_plan(last_started_at, min_interval, wait_entry_at)
            .expect("min interval still has time remaining");

        assert_eq!(plan.cursor, wait_entry_at);
        assert!(plan.cursor > self_write_at);
        assert_ne!(plan.cursor, last_started_at);
        assert_eq!(plan.remaining, min_interval - Duration::from_millis(50));
    }

    fn extract_ts_string_const<'a>(source: &'a str, name: &str) -> Option<&'a str> {
        let prefix = format!("export const {name} = ");
        source.lines().find_map(|line| {
            let value = line.trim().strip_prefix(&prefix)?;
            let value = value.strip_prefix('"')?;
            value.split_once('"').map(|(matched, _)| matched)
        })
    }

    #[test]
    fn deepseek_default_model_matches_web_provider_options() {
        let web_default =
            extract_ts_string_const(WEB_PROVIDER_OPTIONS_TS, "DEEPSEEK_DEFAULT_MODEL")
                .expect("web providerOptions.ts exports DEEPSEEK_DEFAULT_MODEL");
        let web_pro = extract_ts_string_const(WEB_PROVIDER_OPTIONS_TS, "DEEPSEEK_PRO_MODEL")
            .expect("web providerOptions.ts exports DEEPSEEK_PRO_MODEL");
        let web_flash = extract_ts_string_const(WEB_PROVIDER_OPTIONS_TS, "DEEPSEEK_FLASH_MODEL")
            .expect("web providerOptions.ts exports DEEPSEEK_FLASH_MODEL");

        assert_eq!(DEEPSEEK_DEFAULT_MODEL, web_default);
        assert_eq!(shared::DEEPSEEK_PRO_MODEL, web_pro);
        assert_eq!(shared::DEEPSEEK_FLASH_MODEL, web_flash);
    }

    #[test]
    fn deadloop_watchdog_mtime_updates_reset_activity_clock() {
        let start = Instant::now();
        let idle_threshold = Duration::from_secs(30 * 60);
        let previous_activity = start
            .checked_sub(idle_threshold + Duration::from_secs(60))
            .expect("start should support test subtraction");
        let mut state = DeadloopWatchdogState::new(start);
        state.last_activity = previous_activity;
        let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(123);

        let decision =
            evaluate_deadloop_watchdog(&mut state, start, idle_threshold, Some(mtime), true, true);

        assert_eq!(decision, DeadloopWatchdogDecision::Noop);
        assert_eq!(state.last_mtime, Some(mtime));
        assert_eq!(state.last_activity, start);
        assert_eq!(state.last_nudge, None);

        let before_threshold = start + idle_threshold - Duration::from_secs(1);
        let decision = evaluate_deadloop_watchdog(
            &mut state,
            before_threshold,
            idle_threshold,
            Some(mtime),
            true,
            true,
        );

        assert_eq!(decision, DeadloopWatchdogDecision::Noop);
        assert_eq!(state.last_nudge, None);
    }

    #[test]
    fn deadloop_watchdog_stale_activity_nudges_once_until_cooldown_expires() {
        let start = Instant::now();
        let idle_threshold = Duration::from_secs(30 * 60);
        let mut state = DeadloopWatchdogState::new(start);

        let first_nudge_at = start + idle_threshold;
        let decision = evaluate_deadloop_watchdog(
            &mut state,
            first_nudge_at,
            idle_threshold,
            None,
            true,
            true,
        );

        assert_eq!(
            decision,
            DeadloopWatchdogDecision::Nudge { idle_minutes: 30 }
        );
        assert_eq!(state.last_nudge, Some(first_nudge_at));

        let inside_cooldown = first_nudge_at + idle_threshold - Duration::from_secs(1);
        let decision = evaluate_deadloop_watchdog(
            &mut state,
            inside_cooldown,
            idle_threshold,
            None,
            true,
            true,
        );

        assert_eq!(decision, DeadloopWatchdogDecision::Noop);
        assert_eq!(state.last_nudge, Some(first_nudge_at));

        let cooldown_expired = first_nudge_at + idle_threshold;
        let decision = evaluate_deadloop_watchdog(
            &mut state,
            cooldown_expired,
            idle_threshold,
            None,
            true,
            true,
        );

        assert_eq!(
            decision,
            DeadloopWatchdogDecision::Nudge { idle_minutes: 60 }
        );
        assert_eq!(state.last_nudge, Some(cooldown_expired));
    }

    #[test]
    fn deadloop_watchdog_skips_inactive_supervisor_or_missing_jsonl_path() {
        let start = Instant::now();
        let idle_threshold = Duration::from_secs(30 * 60);
        let mut state = DeadloopWatchdogState::new(start);
        let stale_at = start + idle_threshold;

        let decision =
            evaluate_deadloop_watchdog(&mut state, stale_at, idle_threshold, None, true, false);

        assert_eq!(decision, DeadloopWatchdogDecision::Noop);
        assert_eq!(state.last_nudge, None);

        let decision =
            evaluate_deadloop_watchdog(&mut state, stale_at, idle_threshold, None, false, true);

        assert_eq!(decision, DeadloopWatchdogDecision::Noop);
        assert_eq!(state.last_nudge, None);
    }

    fn test_pane_meta(
        provider: Provider,
        managed: bool,
        effort: Option<&str>,
        effort_arc: Arc<Mutex<Option<String>>>,
    ) -> PaneMeta {
        PaneMeta {
            kind: shared::PaneKind::Agent,
            mode: shared::PaneMode::Deadloop,
            provider,
            label: "Side Developer".to_string(),
            prompt: Some("Keep helping".to_string()),
            model: None,
            effort: effort.map(str::to_string),
            min_iteration_interval_minutes: Some(5),
            child_process: Arc::new(Mutex::new(None)),
            streaming_interrupt_tx: Arc::new(Mutex::new(None)),
            control_response_tx: Arc::new(Mutex::new(None)),
            pending_questions: Arc::new(Mutex::new(HashMap::new())),
            effort_arc,
            worktree_path: Some("/tmp/apas-side-dev".to_string()),
            role: Some("developer".to_string()),
            goal: Some("Ship the side quest".to_string()),
            backstory: Some("A manually added helper pane".to_string()),
            plan_review_mode: shared::PlanReviewMode::RiskyOnly,
            plan_review_mode_arc: Arc::new(Mutex::new(shared::PlanReviewMode::RiskyOnly)),
            pending_plan_reviews: Arc::new(Mutex::new(HashMap::new())),
            manual_mode: true,
            managed,
        }
    }

    #[test]
    fn retired_rejection_is_sent_as_legacy_compatible_system_output() {
        let session_id = Uuid::new_v4();
        let message = retired_launch_rejection_output(
            session_id,
            29,
            "[New pane refused — unsupported provider has been retired]",
        );

        assert!(matches!(
            &message,
            CliToServer::Output {
                session_id: got_session,
                data,
                output_type: shared::OutputType::System,
                pane_type: None,
                pane_id: Some(29),
            } if *got_session == session_id && data.contains("unsupported provider")
        ));
        let encoded = serde_json::to_value(message).unwrap();
        assert_eq!(encoded["type"], "output");
        assert_eq!(encoded["pane_id"], 29);
    }

    #[test]
    fn stop_retired_panes_is_selective_and_preserves_metadata() {
        let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::new()));
        let mut retired = test_pane_meta(Provider::Claude, true, None, Arc::new(Mutex::new(None)));
        retired.model = Some("MiniMax-M2.7".to_string());
        let (interrupt_tx, interrupt_rx) = mpsc::channel();
        retired.streaming_interrupt_tx = Arc::new(Mutex::new(Some(interrupt_tx)));
        pane_metas.lock().unwrap().insert(7, retired);
        pane_metas.lock().unwrap().insert(
            8,
            test_pane_meta(Provider::Codex, true, None, Arc::new(Mutex::new(None))),
        );

        let retired_pause = Arc::new(AtomicBool::new(false));
        let supported_pause = Arc::new(AtomicBool::new(false));
        let pane_pauses: PanePauses = Arc::new(Mutex::new(HashMap::from([
            (7, retired_pause.clone()),
            (8, supported_pause.clone()),
        ])));
        let (retired_input_tx, _retired_input_rx) = mpsc::channel();
        let (supported_input_tx, _supported_input_rx) = mpsc::channel();
        let input_channels: InputChannels = Arc::new(Mutex::new(HashMap::from([
            (7, retired_input_tx),
            (8, supported_input_tx),
        ])));
        let terminal_panes: TerminalPanes = Arc::new(Mutex::new(HashMap::new()));

        assert_eq!(
            stop_retired_panes(&pane_metas, &pane_pauses, &input_channels, &terminal_panes,),
            vec![7],
        );
        assert!(retired_pause.load(Ordering::SeqCst));
        assert!(!supported_pause.load(Ordering::SeqCst));
        assert!(interrupt_rx.try_recv().is_ok());
        assert!(!input_channels.lock().unwrap().contains_key(&7));
        assert!(input_channels.lock().unwrap().contains_key(&8));
        let metas = pane_metas.lock().unwrap();
        assert!(metas.contains_key(&7));
        assert!(metas.contains_key(&8));
    }

    fn test_pane_config(
        pane_id: u32,
        mode: shared::PaneMode,
        is_paused: bool,
        stop_requested: bool,
    ) -> shared::PaneConfig {
        shared::PaneConfig {
            pane_id,
            provider: Provider::Codex,
            mode,
            kind: shared::PaneKind::Agent,
            session_id: Uuid::new_v4(),
            is_paused,
            stop_requested,
            prompt: None,
            min_iteration_interval_minutes: None,
            label: None,
            model: None,
            effort: None,
            worktree_path: None,
            role: None,
            goal: None,
            backstory: None,
            plan_review_mode: shared::PlanReviewMode::default(),
            manual_mode: false,
            managed: false,
        }
    }

    fn test_managed_role_pane(
        pane_id: u32,
        role: &str,
        mode: shared::PaneMode,
        prompt: Option<&str>,
        worktree_path: Option<&str>,
    ) -> shared::PaneConfig {
        let mut pane = test_pane_config(pane_id, mode, false, false);
        pane.managed = true;
        pane.role = Some(role.to_string());
        pane.prompt = prompt.map(str::to_string);
        pane.worktree_path = worktree_path.map(str::to_string);
        pane
    }

    fn test_team_pane_meta(
        label: &str,
        role: &str,
        mode: shared::PaneMode,
        managed: bool,
    ) -> PaneMeta {
        PaneMeta {
            kind: shared::PaneKind::Agent,
            mode,
            provider: Provider::Claude,
            label: label.to_string(),
            prompt: None,
            model: None,
            effort: None,
            min_iteration_interval_minutes: None,
            child_process: Arc::new(Mutex::new(None)),
            streaming_interrupt_tx: Arc::new(Mutex::new(None)),
            control_response_tx: Arc::new(Mutex::new(None)),
            pending_questions: Arc::new(Mutex::new(HashMap::new())),
            effort_arc: Arc::new(Mutex::new(None)),
            worktree_path: None,
            role: Some(role.to_string()),
            goal: None,
            backstory: None,
            plan_review_mode: shared::PlanReviewMode::default(),
            plan_review_mode_arc: Arc::new(Mutex::new(shared::PlanReviewMode::default())),
            pending_plan_reviews: Arc::new(Mutex::new(HashMap::new())),
            manual_mode: false,
            managed,
        }
    }

    fn start_team_event_label(event: &TuiEvent) -> &str {
        let TuiEvent::AddTabWithConfig { label, .. } = event else {
            panic!("Start team should emit AddTabWithConfig events");
        };
        label.as_str()
    }

    fn seed_pending_question(meta: &PaneMeta, tool_use_id: &str, request_id: &str) {
        meta.pending_questions.lock().unwrap().insert(
            tool_use_id.to_string(),
            PendingAskQuestion {
                request_id: request_id.to_string(),
                questions: serde_json::json!([{
                    "question": "Proceed?",
                    "header": "Confirm",
                }]),
            },
        );
    }

    fn pending_question_ids(pane_metas: &PaneMetas, pane_id: u32) -> HashSet<String> {
        pane_metas
            .lock()
            .unwrap()
            .get(&pane_id)
            .map(|meta| {
                meta.pending_questions
                    .lock()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A pane an older `.apas` marked managed used to be refused a PR, because
    /// team workers opened their own after review. Nothing owns a pane on the
    /// user's behalf now, so the flag decides nothing and the pane behaves like
    /// any other.
    #[test]
    fn manual_create_pr_treats_a_managed_pane_like_any_other() {
        let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::new()));
        let mut meta = test_pane_meta(Provider::Claude, true, None, Arc::new(Mutex::new(None)));
        meta.worktree_path = Some("/w/pane-42".to_string());
        pane_metas.lock().unwrap().insert(42, meta);

        assert_eq!(
            manual_create_pr_worktree_path(&pane_metas, 42).unwrap(),
            Some("/w/pane-42".to_string()),
        );
    }

    #[test]
    fn manual_create_pr_preserves_unmanaged_worktree_path() {
        let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::new()));
        pane_metas.lock().unwrap().insert(
            42,
            test_pane_meta(Provider::Claude, false, None, Arc::new(Mutex::new(None))),
        );

        let worktree_path = manual_create_pr_worktree_path(&pane_metas, 42).unwrap();

        assert_eq!(worktree_path.as_deref(), Some("/tmp/apas-side-dev"));
    }

    #[test]
    fn manual_create_pr_preserves_missing_pane_fallback() {
        let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::new()));

        let worktree_path = manual_create_pr_worktree_path(&pane_metas, 404).unwrap();

        assert_eq!(worktree_path, None);
    }

    #[test]
    fn build_agent_args_claude_resume_keeps_full_prompt() {
        let session_id = Uuid::new_v4();
        let (args, using_resume) = build_agent_args(
            &Provider::Claude,
            &session_id,
            FULL_PROMPT,
            None,
            None,
            false,
            true,
        );

        assert!(using_resume);
        assert!(args.iter().any(|arg| arg == "--resume"));
        assert_eq!(args.last().map(String::as_str), Some(FULL_PROMPT));
    }

    #[test]
    fn build_agent_args_codex_resume_keeps_full_prompt() {
        let session_id = Uuid::new_v4();
        let (args, using_resume) = build_agent_args(
            &Provider::Codex,
            &session_id,
            FULL_PROMPT,
            None,
            None,
            false,
            true,
        );

        assert!(using_resume);
        assert_eq!(args.get(0).map(String::as_str), Some("exec"));
        assert_eq!(args.get(1).map(String::as_str), Some("resume"));
        assert_eq!(args.last().map(String::as_str), Some(FULL_PROMPT));
    }

    #[test]
    fn build_agent_args_codex_includes_model_and_reasoning_effort() {
        let session_id = Uuid::new_v4();
        let (args, _) = build_agent_args(
            &Provider::Codex,
            &session_id,
            FULL_PROMPT,
            Some("gpt-5.6-sol"),
            Some("xhigh"),
            true,
            false,
        );
        assert!(args.windows(2).any(|w| w == ["--model", "gpt-5.6-sol"]));
        assert!(args
            .windows(2)
            .any(|w| w == ["-c", "model_reasoning_effort=xhigh"]));
        // Flags ride after the `exec` subcommand, before the prompt.
        assert_eq!(args.get(0).map(String::as_str), Some("exec"));
        assert_eq!(args.last().map(String::as_str), Some(FULL_PROMPT));
    }

    #[test]
    fn build_agent_args_codex_omits_model_effort_when_unset() {
        let session_id = Uuid::new_v4();
        let (args, _) = build_agent_args(
            &Provider::Codex,
            &session_id,
            FULL_PROMPT,
            None,
            None,
            true,
            false,
        );
        assert!(!args.iter().any(|a| a == "--model"));
        assert!(!args.iter().any(|a| a == "-c"));
    }

    #[test]
    fn build_agent_args_opencode_uses_auto_and_never_passes_the_apas_uuid() {
        let session_id = Uuid::parse_str("11111111-2222-4333-8444-555555555555").unwrap();
        let (fresh, fresh_resume) = build_agent_args(
            &Provider::Opencode,
            &session_id,
            FULL_PROMPT,
            Some("openai/gpt-5"),
            None,
            true,
            false,
        );
        assert!(!fresh_resume);
        assert_eq!(fresh[0], "run");
        assert!(fresh.iter().any(|arg| arg == "--auto"));
        assert!(fresh.windows(2).any(|args| args == ["-m", "openai/gpt-5"]));
        assert!(!fresh.iter().any(|arg| arg == "--continue"));
        assert!(!fresh.iter().any(|arg| arg == &session_id.to_string()));
        assert_eq!(fresh.last().map(String::as_str), Some(FULL_PROMPT));

        let (resumed, using_resume) = build_agent_args(
            &Provider::Opencode,
            &session_id,
            FULL_PROMPT,
            None,
            None,
            false,
            false,
        );
        assert!(using_resume);
        assert!(resumed.iter().any(|arg| arg == "--continue"));
        assert!(!resumed.iter().any(|arg| arg == "--session" || arg == "-s"));
        assert!(!resumed.iter().any(|arg| arg == &session_id.to_string()));
    }

    #[test]
    fn opencode_json_events_are_adapted_to_shared_messages() {
        let text = convert_opencode_to_claude(
            r#"{"type":"text","part":{"type":"text","text":"done"}}"#,
            "pane-session",
        )
        .expect("text event");
        match text {
            ClaudeStreamMessage::Assistant {
                message,
                session_id,
                ..
            } => {
                assert_eq!(session_id, "pane-session");
                assert!(matches!(
                    message.content.as_slice(),
                    [ClaudeContentBlock::Text { text }] if text == "done"
                ));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let finish = convert_opencode_to_claude(
            r#"{"type":"step_finish","part":{"reason":"stop","cost":0.25,"tokens":{"input":11,"output":7}}}"#,
            "pane-session",
        )
        .expect("finish event");
        match finish {
            ClaudeStreamMessage::Result {
                total_cost_usd,
                extra,
                is_error,
                ..
            } => {
                assert_eq!(total_cost_usd, 0.25);
                assert_eq!(extra["usage"]["input_tokens"], 11);
                assert_eq!(extra["usage"]["output_tokens"], 7);
                assert!(!is_error);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        assert!(convert_opencode_to_claude(
            r#"{"type":"step_finish","part":{"reason":"tool-calls"}}"#,
            "pane-session",
        )
        .is_none());
    }

    #[test]
    fn normalize_codex_effort_maps_levels() {
        assert_eq!(
            normalize_codex_effort(Some("high")).as_deref(),
            Some("high")
        );
        assert_eq!(
            normalize_codex_effort(Some("xhigh")).as_deref(),
            Some("xhigh")
        );
        assert_eq!(normalize_codex_effort(Some("max")).as_deref(), Some("max"));
        // apas-only ultracode + codex ultra both land on codex's `ultra`.
        assert_eq!(
            normalize_codex_effort(Some("ultra")).as_deref(),
            Some("ultra")
        );
        assert_eq!(
            normalize_codex_effort(Some("ultracode")).as_deref(),
            Some("ultra")
        );
        // minimal floors to low (gpt-5.6 has no minimal).
        assert_eq!(
            normalize_codex_effort(Some("minimal")).as_deref(),
            Some("low")
        );
        // default / empty → None (codex uses its config.toml default).
        assert_eq!(normalize_codex_effort(Some("default")), None);
        assert_eq!(normalize_codex_effort(Some("  ")), None);
        assert_eq!(normalize_codex_effort(None), None);
    }

    #[test]
    fn codex_stale_session_recovery_resets_first_resumed_message_for_fresh_retry() {
        let old_session_id = Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap();
        let fresh_session_id = Uuid::parse_str("00000000-0000-4000-8000-000000000002").unwrap();
        let mut session_id = old_session_id;
        let mut first_message = true;
        let mut try_resume_first = true;

        assert!(should_recover_deadloop_stale_session(
            false,
            first_message,
            true,
            true,
            false,
        ));

        let old = reset_deadloop_codex_stale_session(
            &mut first_message,
            &mut try_resume_first,
            &mut session_id,
            fresh_session_id,
        );

        assert_eq!(old, old_session_id);
        assert_eq!(session_id, fresh_session_id);
        assert!(first_message);
        assert!(!try_resume_first);

        let (args, using_resume) = build_deadloop_agent_args(
            &Provider::Codex,
            &session_id,
            FULL_PROMPT,
            None,
            None,
            first_message,
            try_resume_first,
        );

        assert!(!using_resume);
        assert_eq!(args.get(0).map(String::as_str), Some("exec"));
        assert!(!args.iter().any(|arg| arg == "resume"));
        assert_eq!(args.last().map(String::as_str), Some(FULL_PROMPT));
        assert!(!args.iter().any(|arg| arg == &old_session_id.to_string()));
        assert!(!args.iter().any(|arg| arg == &fresh_session_id.to_string()));
    }

    #[test]
    fn codex_stale_session_recovery_ignores_mid_loop_ordinary_failure() {
        let session_id = Uuid::parse_str("00000000-0000-4000-8000-000000000003").unwrap();
        let first_message = false;
        let try_resume_first = true;

        assert!(!should_recover_deadloop_stale_session(
            false,
            first_message,
            true,
            true,
            true,
        ));

        let (args, using_resume) = build_deadloop_agent_args(
            &Provider::Codex,
            &session_id,
            FULL_PROMPT,
            None,
            None,
            first_message,
            try_resume_first,
        );

        assert!(using_resume);
        assert_eq!(args.get(0).map(String::as_str), Some("exec"));
        assert_eq!(args.get(1).map(String::as_str), Some("resume"));
        assert!(args.iter().any(|arg| arg == &session_id.to_string()));
        assert_eq!(args.last().map(String::as_str), Some(FULL_PROMPT));
    }

    #[test]
    fn codex_stale_session_error_detects_resume_failures() {
        assert!(is_codex_stale_session_error(
            "ERROR no rollout found for thread id abc123"
        ));
        assert!(is_codex_stale_session_error(
            "thread/resume failed: remote thread disappeared"
        ));
        assert!(!is_codex_stale_session_error(
            "model returned a normal tool execution error"
        ));
    }

    #[test]
    fn boot_restore_resume_policy_is_claude_only_without_backend_model() {
        assert!(boot_restore_try_resume_first(&Provider::Claude, None));
        assert!(boot_restore_try_resume_first(
            &Provider::Claude,
            Some("sonnet")
        ));

        assert!(!boot_restore_try_resume_first(&Provider::Codex, None));
        assert!(!boot_restore_try_resume_first(&Provider::Opencode, None));
        assert!(!boot_restore_try_resume_first(&Provider::CursorAgent, None));
        assert!(!boot_restore_try_resume_first(&Provider::Minimax, None));
        assert!(!boot_restore_try_resume_first(&Provider::Glm, None));
        assert!(!boot_restore_try_resume_first(&Provider::Deepseek, None));
        assert!(!boot_restore_try_resume_first(
            &Provider::Claude,
            Some("MiniMax-M2.7")
        ));
        assert!(!boot_restore_try_resume_first(
            &Provider::Claude,
            Some("glm-4.5-air")
        ));
        assert!(!boot_restore_try_resume_first(
            &Provider::Claude,
            Some("deepseek-chat")
        ));
    }

    #[test]
    fn deepseek_model_switch_respawns_with_canonical_model_and_fresh_session() {
        let previous_session = Uuid::from_u128(1);
        let fresh_session = Uuid::from_u128(2);
        let canonical_model = canonicalize_requested_model(Some("DeepSeek-V4-Flash"));

        let event = build_agent_switch_respawn_event(
            42,
            "Developer".to_string(),
            fresh_session,
            shared::PaneMode::Deadloop,
            Provider::Claude,
            Some("keep going".to_string()),
            Some(3),
            canonical_model,
            None,
            Some("/tmp/apas-dev".to_string()),
            Some("developer".to_string()),
            Some("ship tests".to_string()),
            Some("context".to_string()),
            shared::PlanReviewMode::RiskyOnly,
            true,
        );

        let TuiEvent::AddTabWithConfig {
            claude_session_id,
            provider,
            model,
            initial_input,
            try_resume_first,
            ..
        } = event
        else {
            panic!("agent switch should respawn through AddTabWithConfig");
        };

        assert_ne!(claude_session_id, previous_session);
        assert_eq!(claude_session_id, fresh_session);
        assert_eq!(provider, Provider::Claude);
        assert_eq!(model.as_deref(), Some(shared::DEEPSEEK_FLASH_MODEL));
        assert!(initial_input.is_none());
        assert!(!try_resume_first);

        let (args, using_resume) = build_agent_args(
            &provider,
            &claude_session_id,
            FULL_PROMPT,
            model.as_deref(),
            None,
            true,
            try_resume_first,
        );
        assert!(!using_resume);
        assert!(!args.iter().any(|arg| arg == "--resume"));
        assert!(args.iter().any(|arg| arg == "--session-id"));
        assert!(args.iter().any(|arg| arg == &fresh_session.to_string()));
        assert!(!args.iter().any(|arg| arg == &previous_session.to_string()));
    }

    #[test]
    fn deepseek_pro_flash_switch_cannot_use_live_model_path() {
        assert!(!model_switch_can_use_fast_path(
            Provider::Claude,
            Some(shared::DEEPSEEK_PRO_MODEL),
            Provider::Claude,
            Some(shared::DEEPSEEK_FLASH_MODEL),
        ));
        assert!(!model_switch_can_use_fast_path(
            Provider::Claude,
            Some(shared::DEEPSEEK_FLASH_MODEL),
            Provider::Claude,
            Some(shared::DEEPSEEK_PRO_MODEL),
        ));
        assert!(model_switch_can_use_fast_path(
            Provider::Claude,
            Some("claude-sonnet-4-6"),
            Provider::Claude,
            Some("claude-opus-4-6"),
        ));
    }

    #[test]
    fn pane_reboot_terminal_pane_respawns_as_terminal() {
        let effort_arc = Arc::new(Mutex::new(None));
        let mut meta = test_pane_meta(Provider::Codex, false, None, effort_arc);
        meta.kind = shared::PaneKind::Terminal;
        meta.mode = shared::PaneMode::Interactive;

        let (_close, add_event) = build_pane_reboot_events(9, &meta, Some(Uuid::from_u128(3)));

        // Rebooting must re-open a pty. If `kind` were dropped here the
        // pane would silently come back as a headless agent worker and
        // the user's terminal tab would stop echoing entirely.
        match add_event {
            TuiEvent::AddTabWithConfig { kind, pane_id, .. } => {
                assert_eq!(kind, shared::PaneKind::Terminal);
                assert_eq!(pane_id, 9);
            }
            _ => panic!("expected AddTabWithConfig event"),
        }
    }

    #[test]
    fn reconnect_reports_configured_terminal_without_handle_as_exited() {
        let session_id = Uuid::new_v4();
        let effort_arc = Arc::new(Mutex::new(None));
        let mut terminal = test_pane_meta(Provider::Codex, false, None, effort_arc);
        terminal.kind = shared::PaneKind::Terminal;
        terminal.mode = shared::PaneMode::Interactive;
        let panes: PaneMetas = Arc::new(Mutex::new(HashMap::from([(888, terminal)])));
        let handles = crate::terminal_pane::TerminalPanes::default();

        let reports = terminal_state_reports(session_id, &panes, &handles);
        assert_eq!(reports.len(), 1);
        match &reports[0] {
            CliToServer::TerminalState {
                session_id: got_session,
                pane_id,
                instance_id,
                lifecycle,
                status,
                ..
            } => {
                assert_eq!(*got_session, session_id);
                assert_eq!(*pane_id, 888);
                assert!(instance_id.is_none());
                assert_eq!(*lifecycle, shared::TerminalLifecycle::Exited);
                assert_eq!(status.as_deref(), Some("terminal process unavailable"));
            }
            other => panic!("unexpected report: {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_dropped_transport_preserves_terminal_and_structured_turn_state() {
        let root = tempfile::tempdir().unwrap();
        let provider = root.path().join("reconnect-provider.sh");
        std::fs::write(
            &provider,
            b"#!/bin/sh\nprintf 'ready\\n'\nwhile IFS= read -r line; do printf 'echo:%s\\n' \"$line\"; done\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&provider, std::fs::Permissions::from_mode(0o700)).unwrap();
        let (server_tx, mut server_rx) = tokio_mpsc::channel(16);
        let session_id = Uuid::new_v4();
        let terminal = crate::terminal_pane::TerminalHandle::spawn(
            41,
            session_id,
            Uuid::new_v4(),
            &Provider::Claude,
            provider.to_str().unwrap(),
            root.path().to_str().unwrap(),
            &[],
            false,
            None,
            server_tx,
        )
        .unwrap();
        let instance_before = terminal.instance_id();
        let pid_before = terminal.provider_pid();
        terminal.write_bytes(b"before-reconnect\n").unwrap();

        let (structured_tx, structured_rx) = mpsc::channel();
        structured_tx
            .send(("turn remains queued".to_string(), false))
            .unwrap();
        // A failed connection attempt does not touch either in-process
        // transport consumer: automatic recovery must never cost pane state.
        assert_eq!(structured_rx.recv().unwrap().0, "turn remains queued");
        terminal.write_bytes(b"during-reconnect\n").unwrap();
        assert_eq!(terminal.instance_id(), instance_before);
        assert_eq!(terminal.provider_pid(), pid_before);

        let mut saw_during = false;
        for _ in 0..8 {
            let Some(message) =
                tokio::time::timeout(std::time::Duration::from_secs(2), server_rx.recv())
                    .await
                    .unwrap()
            else {
                break;
            };
            if let CliToServer::TerminalOutput {
                instance_id: Some(instance_id),
                data_b64,
                ..
            } = message
            {
                assert_eq!(instance_id, instance_before);
                let data = base64::engine::general_purpose::STANDARD
                    .decode(data_b64)
                    .unwrap();
                if String::from_utf8_lossy(&data).contains("during-reconnect") {
                    saw_during = true;
                    break;
                }
            }
        }
        assert!(
            saw_during,
            "terminal output stopped during transport reconnect"
        );
        terminal.shutdown();
    }

    #[test]
    fn pane_reboot_events_respawn_requested_pane_on_same_session_with_config() {
        let prior_session = Uuid::from_u128(7);
        let effort_arc = Arc::new(Mutex::new(Some("high".to_string())));
        let mut meta = test_pane_meta(Provider::Codex, true, Some("high"), effort_arc);
        meta.mode = shared::PaneMode::Interactive;
        meta.label = "Managed Developer".to_string();
        meta.model = Some("gpt-5-codex".to_string());
        meta.min_iteration_interval_minutes = Some(11);

        let (close_event, add_event) = build_pane_reboot_events(42, &meta, Some(prior_session));

        match close_event {
            TuiEvent::CloseTab {
                pane_id,
                cleanup_action,
            } => {
                assert_eq!(pane_id, 42);
                assert!(cleanup_action.is_none());
            }
            _ => panic!("expected CloseTab event"),
        }

        let TuiEvent::AddTabWithConfig {
            pane_id,
            label,
            claude_session_id,
            mode,
            provider,
            prompt,
            min_iteration_interval_minutes,
            model,
            effort,
            worktree_path,
            initial_input,
            role,
            goal,
            backstory,
            plan_review_mode,
            managed,
            try_resume_first,
            kind,
        } = add_event
        else {
            panic!("expected AddTabWithConfig event");
        };

        assert_eq!(kind, meta.kind);
        assert_eq!(pane_id, 42);
        assert_eq!(label, "Managed Developer");
        assert_eq!(claude_session_id, prior_session);
        assert_eq!(mode, shared::PaneMode::Interactive);
        assert_eq!(provider, Provider::Codex);
        assert_eq!(prompt.as_deref(), Some("Keep helping"));
        assert_eq!(min_iteration_interval_minutes, Some(11));
        assert_eq!(model.as_deref(), Some("gpt-5-codex"));
        assert_eq!(effort.as_deref(), Some("high"));
        assert_eq!(worktree_path.as_deref(), Some("/tmp/apas-side-dev"));
        assert!(initial_input.is_none());
        assert_eq!(role.as_deref(), Some("developer"));
        assert_eq!(goal.as_deref(), Some("Ship the side quest"));
        assert_eq!(backstory.as_deref(), Some("A manually added helper pane"));
        assert_eq!(plan_review_mode, shared::PlanReviewMode::RiskyOnly);
        assert!(managed);
        assert!(try_resume_first);
    }

    #[test]
    fn deadloop_initial_resume_flag_controls_codex_first_spawn_args() {
        let session_id = Uuid::from_u128(3);

        let (fresh_args, fresh_using_resume) = build_deadloop_agent_args(
            &Provider::Codex,
            &session_id,
            FULL_PROMPT,
            None,
            None,
            true,
            false,
        );
        assert!(!fresh_using_resume);
        assert_eq!(fresh_args.get(0).map(String::as_str), Some("exec"));
        assert_ne!(fresh_args.get(1).map(String::as_str), Some("resume"));
        assert!(!fresh_args.iter().any(|arg| arg == &session_id.to_string()));

        let (resume_args, resume_using_resume) = build_deadloop_agent_args(
            &Provider::Codex,
            &session_id,
            FULL_PROMPT,
            None,
            None,
            true,
            true,
        );
        assert!(resume_using_resume);
        assert_eq!(resume_args.get(0).map(String::as_str), Some("exec"));
        assert_eq!(resume_args.get(1).map(String::as_str), Some("resume"));
        assert!(resume_args.iter().any(|arg| arg == &session_id.to_string()));
    }

    #[test]
    fn start_bot_preserves_managed_role_metadata() {
        let effort_arc = Arc::new(Mutex::new(Some("max".to_string())));
        let meta = test_pane_meta(Provider::Claude, true, Some("max"), effort_arc);

        let preserved = start_bot_preserved_fields(Some(&meta));

        assert!(preserved.managed);
        assert!(preserved.manual_mode);
        assert_eq!(
            preserved.worktree_path.as_deref(),
            Some("/tmp/apas-side-dev")
        );
        assert_eq!(preserved.role.as_deref(), Some("developer"));
        assert_eq!(preserved.goal.as_deref(), Some("Ship the side quest"));
        assert_eq!(
            preserved.backstory.as_deref(),
            Some("A manually added helper pane"),
        );
        assert_eq!(
            preserved.plan_review_mode,
            shared::PlanReviewMode::RiskyOnly
        );
    }

    #[test]
    fn start_bot_defaults_untracked_panes_to_unmanaged() {
        let preserved = start_bot_preserved_fields(None);

        assert!(!preserved.managed);
        assert!(!preserved.manual_mode);
        assert!(preserved.role.is_none());
        assert_eq!(preserved.plan_review_mode, shared::PlanReviewMode::Never);
    }

    #[test]
    fn save_pane_configs_persists_pause_flags_and_legacy_deadloop_pause() {
        // `save_project` registers into `config_dir()/projects.json`; without
        // this the write lands in whatever config dir another test's env vars
        // currently point at.
        let _config = crate::config::test_config::isolated_config_dir();
        let dir = tempfile::tempdir().expect("temp project dir");
        let working_dir = dir.path().to_string_lossy().to_string();
        let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::new()));
        let pane_sessions = Arc::new(Mutex::new(HashMap::new()));
        let pane_pauses: PanePauses = Arc::new(Mutex::new(HashMap::new()));
        let pane_stop_requests: PaneStopRequests = Arc::new(Mutex::new(HashMap::new()));

        {
            let mut metas = pane_metas.lock().unwrap();
            metas.insert(
                shared::PANE_ID_DEADLOOP,
                test_pane_meta(Provider::Claude, true, None, Arc::new(Mutex::new(None))),
            );
            metas.insert(
                42,
                test_pane_meta(Provider::Codex, true, None, Arc::new(Mutex::new(None))),
            );
        }
        {
            let mut sessions = pane_sessions.lock().unwrap();
            sessions.insert(shared::PANE_ID_DEADLOOP, Uuid::new_v4());
            sessions.insert(42, Uuid::new_v4());
        }
        {
            let mut pauses = pane_pauses.lock().unwrap();
            pauses.insert(shared::PANE_ID_DEADLOOP, Arc::new(AtomicBool::new(true)));
            pauses.insert(42, Arc::new(AtomicBool::new(true)));
        }

        save_pane_configs(
            &working_dir,
            &pane_sessions,
            &pane_metas,
            &pane_pauses,
            &pane_stop_requests,
        );

        let metadata = get_or_create_project(dir.path()).expect("metadata should reload");
        assert!(metadata.is_paused);
        assert!(
            metadata
                .panes
                .iter()
                .find(|pane| pane.pane_id == shared::PANE_ID_DEADLOOP)
                .expect("deadloop pane")
                .is_paused
        );
        assert!(
            metadata
                .panes
                .iter()
                .find(|pane| pane.pane_id == 42)
                .expect("managed worker pane")
                .is_paused
        );

        {
            let pauses = pane_pauses.lock().unwrap();
            pauses
                .get(&shared::PANE_ID_DEADLOOP)
                .expect("deadloop pause flag")
                .store(false, Ordering::SeqCst);
            pauses
                .get(&42)
                .expect("worker pause flag")
                .store(false, Ordering::SeqCst);
        }
        save_pane_configs(
            &working_dir,
            &pane_sessions,
            &pane_metas,
            &pane_pauses,
            &pane_stop_requests,
        );

        let metadata = get_or_create_project(dir.path()).expect("metadata should reload");
        assert!(!metadata.is_paused);
        assert!(metadata.panes.iter().all(|pane| !pane.is_paused));
    }

    #[test]
    fn verified_codex_identity_is_compared_persisted_and_announced() {
        let _config = crate::config::test_config::isolated_config_dir();
        let dir = tempfile::tempdir().expect("temp project dir");
        let working_dir = dir.path().to_string_lossy().to_string();
        let project_id = get_or_create_project(dir.path()).unwrap().id;
        let pane_id = 42;
        let provisional = Uuid::parse_str("11111111-2222-4333-8444-555555555555").unwrap();
        let verified = Uuid::parse_str("aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee").unwrap();
        let stale = Uuid::parse_str("99999999-8888-4777-8666-555555555555").unwrap();

        let mut meta = test_pane_meta(Provider::Codex, false, None, Arc::new(Mutex::new(None)));
        meta.kind = shared::PaneKind::Terminal;
        meta.mode = shared::PaneMode::Interactive;
        let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::from([(pane_id, meta)])));
        let pane_sessions = Arc::new(Mutex::new(HashMap::from([(pane_id, provisional)])));
        let input_channels: InputChannels = Arc::new(Mutex::new(HashMap::new()));
        let pane_pauses: PanePauses = Arc::new(Mutex::new(HashMap::new()));
        let pane_stop_requests: PaneStopRequests = Arc::new(Mutex::new(HashMap::new()));

        assert!(remember_verified_codex_session(
            &pane_sessions,
            pane_id,
            provisional,
            verified,
        ));
        assert!(
            !remember_verified_codex_session(&pane_sessions, pane_id, provisional, stale),
            "a late poll must not overwrite a newer pane identity",
        );

        let (server_tx, mut server_rx) = tokio_mpsc::channel(1);
        announce_and_persist_panes(
            &server_tx,
            project_id,
            &working_dir,
            &pane_metas,
            &input_channels,
            &pane_sessions,
            &pane_pauses,
            &pane_stop_requests,
        );

        match server_rx.blocking_recv().expect("pane list announcement") {
            CliToServer::PaneList { session_id, panes } => {
                assert_eq!(session_id, project_id);
                assert_eq!(panes.len(), 1);
                assert_eq!(panes[0].session_id, verified);
            }
            other => panic!("expected pane list, got {other:?}"),
        }
        let saved = get_or_create_project(dir.path()).expect("saved pane roster");
        assert_eq!(saved.panes.len(), 1);
        assert_eq!(saved.panes[0].session_id, verified);
    }

    #[test]
    fn update_project_operations_persists_flags_and_preserves_panes() {
        let _config = crate::config::test_config::isolated_config_dir();
        let dir = tempfile::tempdir().expect("temp project dir");
        let mut metadata = get_or_create_project(dir.path()).expect("metadata should initialize");
        metadata.auto_approve_todos = false;
        metadata.auto_merge_prs = false;
        metadata.team_enabled = true;
        metadata.disallowed_tab_types = vec!["terminal:codex".to_string()];
        metadata.panes = vec![shared::PaneConfig {
            pane_id: 42,
            provider: Provider::Codex,
            mode: shared::PaneMode::Deadloop,
            kind: shared::PaneKind::Agent,
            session_id: Uuid::new_v4(),
            is_paused: true,
            stop_requested: false,
            prompt: Some("Keep implementing".to_string()),
            min_iteration_interval_minutes: Some(7),
            label: Some("Developer".to_string()),
            model: Some("gpt-5-codex".to_string()),
            effort: Some("high".to_string()),
            worktree_path: Some("/tmp/apas-worker".to_string()),
            role: Some("developer".to_string()),
            goal: Some("Ship the delegated task".to_string()),
            backstory: Some("A managed worker".to_string()),
            plan_review_mode: shared::PlanReviewMode::RiskyOnly,
            manual_mode: true,
            managed: true,
        }];
        let original_pane = metadata.panes[0].clone();
        save_project(dir.path(), &metadata).expect("seed metadata");

        let session_id = Uuid::new_v4();
        let echo = update_project_operations(dir.path(), session_id, true, false)
            .expect("flags should persist");

        match echo {
            CliToServer::ProjectFlagsChanged {
                session_id: echoed_session_id,
                auto_approve_todos,
                auto_merge_prs,
                team_enabled,
                disallowed_tab_types,
            } => {
                assert_eq!(echoed_session_id, session_id);
                assert!(auto_approve_todos);
                assert!(!auto_merge_prs);
                assert!(team_enabled);
                assert_eq!(disallowed_tab_types, vec!["terminal:codex"]);
            }
            other => panic!("unexpected echo message: {other:?}"),
        }

        let reloaded = get_or_create_project(dir.path()).expect("metadata should reload");
        assert!(reloaded.auto_approve_todos);
        assert!(!reloaded.auto_merge_prs);
        assert!(reloaded.team_enabled);
        assert_eq!(reloaded.disallowed_tab_types, vec!["terminal:codex"]);
        assert_eq!(reloaded.panes.len(), 1);
        let pane = &reloaded.panes[0];
        assert_eq!(pane.pane_id, original_pane.pane_id);
        assert_eq!(pane.provider, original_pane.provider);
        assert_eq!(pane.session_id, original_pane.session_id);
        assert_eq!(pane.is_paused, original_pane.is_paused);
        assert_eq!(pane.label, original_pane.label);
        assert_eq!(pane.model, original_pane.model);
        assert_eq!(pane.effort, original_pane.effort);
        assert_eq!(pane.worktree_path, original_pane.worktree_path);
        assert_eq!(pane.role, original_pane.role);
        assert_eq!(pane.goal, original_pane.goal);
        assert_eq!(pane.backstory, original_pane.backstory);
        assert_eq!(pane.plan_review_mode, original_pane.plan_review_mode);
        assert_eq!(pane.manual_mode, original_pane.manual_mode);
        assert_eq!(pane.managed, original_pane.managed);
    }

    #[test]
    fn restored_pane_mode_and_pause_maps_persisted_pause_state() {
        let legacy_deadloop = test_pane_config(
            shared::PANE_ID_DEADLOOP,
            shared::PaneMode::Deadloop,
            false,
            false,
        );
        assert_eq!(
            restored_pane_mode_and_pause(&legacy_deadloop, true),
            (shared::PaneMode::Deadloop, true)
        );

        let persisted_deadloop = test_pane_config(
            shared::PANE_ID_DEADLOOP,
            shared::PaneMode::Deadloop,
            true,
            false,
        );
        assert_eq!(
            restored_pane_mode_and_pause(&persisted_deadloop, false),
            (shared::PaneMode::Deadloop, true)
        );

        let paused_worker = test_pane_config(42, shared::PaneMode::Deadloop, true, false);
        assert_eq!(
            restored_pane_mode_and_pause(&paused_worker, false),
            (shared::PaneMode::Deadloop, true)
        );

        let unpaused_worker = test_pane_config(42, shared::PaneMode::Deadloop, false, false);
        assert_eq!(
            restored_pane_mode_and_pause(&unpaused_worker, true),
            (shared::PaneMode::Deadloop, false)
        );

        let stopped_deadloop = test_pane_config(
            shared::PANE_ID_DEADLOOP,
            shared::PaneMode::Deadloop,
            true,
            true,
        );
        assert_eq!(
            restored_pane_mode_and_pause(&stopped_deadloop, true),
            (shared::PaneMode::Interactive, false)
        );

        let interactive = test_pane_config(2, shared::PaneMode::Interactive, true, false);
        assert_eq!(
            restored_pane_mode_and_pause(&interactive, true),
            (shared::PaneMode::Interactive, false)
        );
    }

    #[test]
    fn paused_deadloop_session_waits_without_spawning_child() {
        let dir = tempfile::tempdir().expect("temp project dir");
        let working_dir = dir.path().to_string_lossy().to_string();
        let (output_tx, output_rx) = mpsc::channel::<PaneOutput>();
        let (event_tx, _event_rx) = mpsc::channel::<TuiEvent>();
        let (server_tx, _server_rx) = tokio_mpsc::channel::<CliToServer>(8);
        let shutdown = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(true));
        let stop_requested = Arc::new(AtomicBool::new(false));
        let child_process = Arc::new(Mutex::new(None));
        let child_for_assert = child_process.clone();
        let input_channels: InputChannels = Arc::new(Mutex::new(HashMap::new()));
        let watcher = Arc::new(crate::file_watcher::ProjectFileWatcher::new(dir.path()));
        let shutdown_for_thread = shutdown.clone();

        let handle = thread::spawn(move || {
            let provider = Provider::Codex;
            run_deadloop_session_inner(
                "apas-test-binary-that-must-not-run",
                &working_dir,
                None,
                None,
                Arc::new(Mutex::new(shared::PlanReviewMode::default())),
                Arc::new(Mutex::new(HashMap::new())),
                Uuid::new_v4(),
                Uuid::new_v4(),
                shared::PANE_ID_DEADLOOP,
                "test prompt",
                None,
                None,
                0,
                &provider,
                output_tx,
                server_tx,
                shutdown_for_thread,
                pause,
                stop_requested,
                child_process,
                event_tx,
                Arc::new(Mutex::new(None)),
                Arc::new(Mutex::new(None)),
                Arc::new(Mutex::new(HashMap::new())),
                Arc::new(Mutex::new(None)),
                input_channels,
                true,
                watcher,
            );
        });

        let mut saw_paused = false;
        for _ in 0..10 {
            if let Ok(output) = output_rx.recv_timeout(Duration::from_millis(100)) {
                assert!(!output.text.contains("Failed to spawn"));
                assert!(!output.text.contains("Error spawning"));
                saw_paused |= output.text.contains("paused - waiting for resume");
                if saw_paused {
                    break;
                }
            }
        }

        assert!(saw_paused, "paused loop should report that it is waiting");
        assert!(child_for_assert.lock().unwrap().is_none());
        shutdown.store(true, Ordering::SeqCst);
        handle.join().expect("paused worker should exit cleanly");
    }

    #[test]
    fn build_agent_args_claude_with_supported_model_includes_model_flag() {
        let session_id = Uuid::new_v4();
        let (args, _) = build_agent_args(
            &Provider::Claude,
            &session_id,
            FULL_PROMPT,
            Some("sonnet"),
            None,
            true,
            false,
        );

        assert!(args.windows(2).any(|w| w == ["--model", "sonnet"]));
    }

    #[test]
    fn build_agent_args_retired_model_returns_no_spawn_arguments() {
        let session_id = Uuid::new_v4();
        let (args, _) = build_agent_args(
            &Provider::Claude,
            &session_id,
            FULL_PROMPT,
            Some("MiniMax-M2.7"),
            None,
            true,
            false,
        );

        assert!(args.is_empty());
    }

    #[test]
    fn truncate_str_at_char_boundary_handles_multibyte_mid_codepoint() {
        // Regression: the streaming worker's "> {prompt}" preview used
        // `&prompt[..140]` which panicked when byte 140 landed inside a
        // multi-byte char. This shape ("…" = U+2026, 3 bytes E2 80 A6
        // sitting at bytes 139..142) reproduces the exact crash a mako
        // srpc-worker hit and froze the pane on.
        let s = "Investigate DSL impl Trait support, then migrate Alarm. If rusty-cpp can lower impl Job for Alarm { fn Ready(&mut self)…fn Work…fn Done… } to the C++ override pattern.";
        // Doesn't panic + returns a valid &str (chars finish cleanly).
        let preview = truncate_str_at_char_boundary(s, 140);
        assert!(preview.chars().last().is_some());
        // We rounded DOWN past the broken codepoint, so length is < 140.
        assert!(preview.len() < 140);
        // Verify a clean (ASCII-only) prefix still truncates exactly at
        // max_bytes when no codepoint straddles the cut.
        let ascii = "x".repeat(200);
        assert_eq!(truncate_str_at_char_boundary(&ascii, 50).len(), 50);
        // Short input returned verbatim.
        assert_eq!(truncate_str_at_char_boundary("hi", 100), "hi");
    }

    #[test]
    fn build_agent_args_claude_with_deepseek_model_omits_model_flag() {
        let session_id = Uuid::new_v4();
        let (args, _) = build_agent_args(
            &Provider::Claude,
            &session_id,
            FULL_PROMPT,
            Some("deepseek-v4-pro"),
            None,
            true,
            false,
        );

        assert!(!args.iter().any(|arg| arg == "--model"));
    }

    #[test]
    fn codex_sqlite_home_is_uid_scoped_and_only_injected_for_codex() {
        let path = codex_sqlite_home_path(std::path::Path::new("/var/tmp"), 3000);
        assert_eq!(
            path,
            std::path::PathBuf::from("/var/tmp/apas-codex-sqlite-3000")
        );

        let mut codex_env = Vec::new();
        append_codex_sqlite_home_env(&Provider::Codex, &path, &mut codex_env);
        assert_eq!(
            codex_env,
            vec![(
                CODEX_SQLITE_HOME_ENV.to_string(),
                path.to_string_lossy().into_owned()
            )]
        );

        let mut claude_env = Vec::new();
        append_codex_sqlite_home_env(&Provider::Claude, &path, &mut claude_env);
        assert!(claude_env.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn codex_sqlite_home_directory_is_private_and_rejects_symlinks() {
        use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

        let root = tempfile::tempdir().unwrap();
        let uid = unsafe { libc::getuid() };
        let path = codex_sqlite_home_path(root.path(), uid);
        ensure_private_codex_sqlite_home(&path, uid).unwrap();

        let metadata = std::fs::symlink_metadata(&path).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(metadata.uid(), uid);
        assert_eq!(metadata.permissions().mode() & 0o777, 0o700);

        let target = root.path().join("target");
        std::fs::create_dir(&target).unwrap();
        let link = root.path().join("unsafe-link");
        symlink(&target, &link).unwrap();
        let error = ensure_private_codex_sqlite_home(&link, uid).unwrap_err();
        assert!(error.contains("Refusing insecure"), "{error}");
    }

    #[test]
    fn deepseek_env_overrides_require_api_key() {
        let err = build_pane_env_overrides_from_keys(&Provider::Deepseek, None, None).unwrap_err();

        assert!(err.contains("missing deepseek_api_key"));
    }

    #[test]
    fn deepseek_pro_env_pins_primary_and_routes_small_work_to_flash() {
        let env = build_pane_env_overrides_from_keys(
            &Provider::Deepseek,
            None,
            Some("sk-deepseek".to_string()),
        )
        .unwrap();
        let get = |key: &str| {
            env.iter()
                .find_map(|(k, v)| (k == key).then_some(v.as_str()))
        };

        assert_eq!(
            get("ANTHROPIC_BASE_URL"),
            Some("https://api.deepseek.com/anthropic")
        );
        assert_eq!(get("ANTHROPIC_API_KEY"), Some("sk-deepseek"));
        assert_eq!(get("ANTHROPIC_AUTH_TOKEN"), Some("sk-deepseek"));
        assert_eq!(get("ANTHROPIC_MODEL"), Some(DEEPSEEK_PRO_RUNTIME_MODEL));
        assert_eq!(
            get("ANTHROPIC_DEFAULT_SONNET_MODEL"),
            Some(DEEPSEEK_PRO_RUNTIME_MODEL)
        );
        assert_eq!(
            get("ANTHROPIC_DEFAULT_OPUS_MODEL"),
            Some(DEEPSEEK_PRO_RUNTIME_MODEL)
        );
        assert_eq!(
            get("ANTHROPIC_DEFAULT_HAIKU_MODEL"),
            Some(shared::DEEPSEEK_FLASH_MODEL)
        );
        assert_eq!(
            get("CLAUDE_CODE_SUBAGENT_MODEL"),
            Some(shared::DEEPSEEK_FLASH_MODEL)
        );
    }

    #[test]
    fn deepseek_flash_env_pins_flash_primary_with_shared_credentials() {
        let env = build_pane_env_overrides_from_keys(
            &Provider::Claude,
            Some(shared::DEEPSEEK_FLASH_MODEL),
            Some("sk-deepseek".to_string()),
        )
        .unwrap();
        let get = |key: &str| {
            env.iter()
                .find_map(|(k, v)| (k == key).then_some(v.as_str()))
        };

        assert_eq!(
            get("ANTHROPIC_BASE_URL"),
            Some("https://api.deepseek.com/anthropic")
        );
        assert_eq!(get("ANTHROPIC_API_KEY"), Some("sk-deepseek"));
        assert_eq!(get("ANTHROPIC_MODEL"), Some(shared::DEEPSEEK_FLASH_MODEL));
        assert_eq!(
            get("ANTHROPIC_DEFAULT_SONNET_MODEL"),
            Some(DEEPSEEK_PRO_RUNTIME_MODEL)
        );
        assert_eq!(
            get("ANTHROPIC_DEFAULT_OPUS_MODEL"),
            Some(DEEPSEEK_PRO_RUNTIME_MODEL)
        );
        assert_eq!(
            get("ANTHROPIC_DEFAULT_HAIKU_MODEL"),
            Some(shared::DEEPSEEK_FLASH_MODEL)
        );
        assert_eq!(
            get("CLAUDE_CODE_SUBAGENT_MODEL"),
            Some(shared::DEEPSEEK_FLASH_MODEL)
        );
    }

    #[test]
    fn deepseek_env_rejects_unknown_models_before_upstream_fallback() {
        for (provider, model) in [
            (Provider::Claude, "deepseek-chat"),
            (Provider::Deepseek, "sonnet"),
        ] {
            let err = build_pane_env_overrides_from_keys(
                &provider,
                Some(model),
                Some("sk-deepseek".to_string()),
            )
            .unwrap_err();
            assert!(err.contains("Unsupported DeepSeek model"), "{err}");
        }
    }

    #[test]
    fn build_agent_args_claude_effort_passes_xhigh_verbatim() {
        let session_id = Uuid::new_v4();
        let (args, _) = build_agent_args(
            &Provider::Claude,
            &session_id,
            FULL_PROMPT,
            None,
            Some("xhigh"),
            true,
            false,
        );

        // xhigh and max are distinct levels in claude --help; don't collapse
        // them.
        assert!(args.windows(2).any(|w| w == ["--effort", "xhigh"]));
        assert!(!args.windows(2).any(|w| w == ["--effort", "max"]));
    }

    #[test]
    fn build_agent_args_claude_effort_ultracode_emits_xhigh_flag() {
        // `ultracode` is apas-only and claude's CLI rejects the literal —
        // it must be translated to `--effort xhigh` on the wire.
        let session_id = Uuid::new_v4();
        let (args, _) = build_agent_args(
            &Provider::Claude,
            &session_id,
            FULL_PROMPT,
            None,
            Some("ultracode"),
            true,
            false,
        );

        assert!(args.windows(2).any(|w| w == ["--effort", "xhigh"]));
        assert!(!args.iter().any(|arg| arg == "ultracode"));
    }

    #[test]
    fn build_user_envelope_line_prefixes_prompt_when_ultracode() {
        // Envelope-prefix path: the only behavioural difference between
        // `ultracode` and `xhigh` is the `ultracode ` keyword prepended to
        // the prompt content on every user-input envelope. Parse the
        // result rather than string-match to stay agnostic to JSON key
        // ordering (serde_json sorts insertion order, but be robust).
        let line = build_user_envelope_line("do the thing", Some("ultracode"));
        assert!(line.ends_with('\n'));
        let v: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(v["type"], "user");
        assert_eq!(v["message"]["role"], "user");
        assert_eq!(v["message"]["content"], "ultracode do the thing");

        // Non-ultracode efforts (and absent effort) must not be prefixed.
        for eff in [Some("xhigh"), Some("max"), Some("high"), None] {
            let l = build_user_envelope_line("do the thing", eff);
            let parsed: serde_json::Value = serde_json::from_str(l.trim_end()).unwrap();
            assert_eq!(parsed["message"]["content"], "do the thing");
            assert!(!l.contains("ultracode"));
        }
    }

    #[test]
    fn normalize_effort_level_passes_ultracode_through() {
        // Used by AddTabWithConfig: a caller-supplied `Some("ultracode")`
        // on a managed Claude pane must survive normalization so it lands
        // in both meta.effort and effort_arc.
        assert_eq!(
            normalize_effort_level(Some("ultracode")),
            Some("ultracode".to_string())
        );
        assert_eq!(
            normalize_effort_level(Some("UltraCode")),
            Some("ultracode".to_string())
        );
    }

    #[test]
    fn build_agent_args_claude_effort_passes_max_verbatim() {
        let session_id = Uuid::new_v4();
        let (args, _) = build_agent_args(
            &Provider::Claude,
            &session_id,
            FULL_PROMPT,
            None,
            Some("max"),
            true,
            false,
        );

        assert!(args.windows(2).any(|w| w == ["--effort", "max"]));
    }

    #[test]
    fn resolve_pane_binary_path_uses_default_claude_for_supported_model() {
        let path = resolve_pane_binary_path(
            Provider::Claude,
            Some("sonnet"),
            "claude",
            "codex",
            "opencode",
            "cursor-agent",
        );
        assert_eq!(path, "claude");
    }

    #[test]
    fn terminal_binary_preflight_returns_actionable_missing_opencode_error() {
        let missing = "/definitely/missing/apas-test-opencode";
        let error = resolve_terminal_binary(&Provider::Opencode, missing).unwrap_err();

        assert!(error.contains("OpenCode"), "{error}");
        assert!(error.contains(missing), "{error}");
        assert!(error.contains("opencode_path"), "{error}");
        assert!(error.contains("executable not found"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn terminal_binary_preflight_accepts_executable_and_rejects_non_executable() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("opencode-test");
        std::fs::write(&binary, b"#!/bin/sh\nexit 0\n").unwrap();

        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            resolve_terminal_binary(&Provider::Opencode, binary.to_str().unwrap()).unwrap(),
            binary.to_string_lossy()
        );

        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o600)).unwrap();
        let error =
            resolve_terminal_binary(&Provider::Opencode, binary.to_str().unwrap()).unwrap_err();
        assert!(error.contains("not an executable file"), "{error}");
    }

    #[test]
    fn retired_models_are_rejected_before_environment_construction() {
        for model in ["MiniMax-M2.7", "m2.7", "glm-5.1"] {
            let err = build_pane_env_overrides_from_keys(
                &Provider::Claude,
                Some(model),
                Some("supported-key-must-not-be-used".to_string()),
            )
            .unwrap_err();
            assert!(
                err.contains("retired"),
                "unexpected error for {model}: {err}"
            );
        }
    }

    #[test]
    fn active_usage_providers_detects_claude_and_codex() {
        let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::new()));
        let child_process = Arc::new(Mutex::new(None));

        {
            let mut metas = pane_metas.lock().unwrap();
            metas.insert(
                1,
                PaneMeta {
                    kind: shared::PaneKind::Agent,
                    mode: shared::PaneMode::Interactive,
                    provider: Provider::Claude,
                    label: "Interactive".to_string(),
                    prompt: None,
                    model: None,
                    effort: None,
                    min_iteration_interval_minutes: None,
                    child_process: child_process.clone(),
                    streaming_interrupt_tx: Arc::new(Mutex::new(None)),
                    control_response_tx: Arc::new(Mutex::new(None)),
                    pending_questions: Arc::new(Mutex::new(HashMap::new())),
                    effort_arc: Arc::new(Mutex::new(None)),
                    worktree_path: None,
                    role: None,
                    goal: None,
                    backstory: None,
                    plan_review_mode: shared::PlanReviewMode::default(),
                    plan_review_mode_arc: Arc::new(Mutex::new(shared::PlanReviewMode::default())),
                    pending_plan_reviews: Arc::new(Mutex::new(HashMap::new())),
                    manual_mode: false,
                    managed: false,
                },
            );
            metas.insert(
                2,
                PaneMeta {
                    kind: shared::PaneKind::Agent,
                    mode: shared::PaneMode::Interactive,
                    provider: Provider::Codex,
                    label: "Tab 2".to_string(),
                    prompt: None,
                    model: None,
                    effort: None,
                    min_iteration_interval_minutes: None,
                    child_process,
                    streaming_interrupt_tx: Arc::new(Mutex::new(None)),
                    control_response_tx: Arc::new(Mutex::new(None)),
                    pending_questions: Arc::new(Mutex::new(HashMap::new())),
                    effort_arc: Arc::new(Mutex::new(None)),
                    worktree_path: None,
                    role: None,
                    goal: None,
                    backstory: None,
                    plan_review_mode: shared::PlanReviewMode::default(),
                    plan_review_mode_arc: Arc::new(Mutex::new(shared::PlanReviewMode::default())),
                    pending_plan_reviews: Arc::new(Mutex::new(HashMap::new())),
                    manual_mode: false,
                    managed: false,
                },
            );
        }

        assert_eq!(active_usage_providers(&pane_metas), (true, true, false));
    }

    #[test]
    fn active_usage_providers_ignores_minimax_only_claude() {
        let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::new()));
        let child_process = Arc::new(Mutex::new(None));

        {
            let mut metas = pane_metas.lock().unwrap();
            metas.insert(
                1,
                PaneMeta {
                    kind: shared::PaneKind::Agent,
                    mode: shared::PaneMode::Interactive,
                    provider: Provider::Claude,
                    label: "MiniMax 1".to_string(),
                    prompt: None,
                    model: Some("MiniMax-M2.7".to_string()),
                    effort: None,
                    min_iteration_interval_minutes: None,
                    child_process,
                    streaming_interrupt_tx: Arc::new(Mutex::new(None)),
                    control_response_tx: Arc::new(Mutex::new(None)),
                    pending_questions: Arc::new(Mutex::new(HashMap::new())),
                    effort_arc: Arc::new(Mutex::new(None)),
                    worktree_path: None,
                    role: None,
                    goal: None,
                    backstory: None,
                    plan_review_mode: shared::PlanReviewMode::default(),
                    plan_review_mode_arc: Arc::new(Mutex::new(shared::PlanReviewMode::default())),
                    pending_plan_reviews: Arc::new(Mutex::new(HashMap::new())),
                    manual_mode: false,
                    managed: false,
                },
            );
        }

        assert_eq!(active_usage_providers(&pane_metas), (false, false, false));
    }

    #[test]
    fn active_usage_providers_keeps_codex_when_mixed_with_minimax() {
        let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::new()));
        let child_process = Arc::new(Mutex::new(None));

        {
            let mut metas = pane_metas.lock().unwrap();
            metas.insert(
                1,
                PaneMeta {
                    kind: shared::PaneKind::Agent,
                    mode: shared::PaneMode::Interactive,
                    provider: Provider::Claude,
                    label: "MiniMax 1".to_string(),
                    prompt: None,
                    model: Some("m2.7".to_string()),
                    effort: None,
                    min_iteration_interval_minutes: None,
                    child_process: child_process.clone(),
                    streaming_interrupt_tx: Arc::new(Mutex::new(None)),
                    control_response_tx: Arc::new(Mutex::new(None)),
                    pending_questions: Arc::new(Mutex::new(HashMap::new())),
                    effort_arc: Arc::new(Mutex::new(None)),
                    worktree_path: None,
                    role: None,
                    goal: None,
                    backstory: None,
                    plan_review_mode: shared::PlanReviewMode::default(),
                    plan_review_mode_arc: Arc::new(Mutex::new(shared::PlanReviewMode::default())),
                    pending_plan_reviews: Arc::new(Mutex::new(HashMap::new())),
                    manual_mode: false,
                    managed: false,
                },
            );
            metas.insert(
                2,
                PaneMeta {
                    kind: shared::PaneKind::Agent,
                    mode: shared::PaneMode::Interactive,
                    provider: Provider::Codex,
                    label: "Tab 2".to_string(),
                    prompt: None,
                    model: None,
                    effort: None,
                    min_iteration_interval_minutes: None,
                    child_process,
                    streaming_interrupt_tx: Arc::new(Mutex::new(None)),
                    control_response_tx: Arc::new(Mutex::new(None)),
                    pending_questions: Arc::new(Mutex::new(HashMap::new())),
                    effort_arc: Arc::new(Mutex::new(None)),
                    worktree_path: None,
                    role: None,
                    goal: None,
                    backstory: None,
                    plan_review_mode: shared::PlanReviewMode::default(),
                    plan_review_mode_arc: Arc::new(Mutex::new(shared::PlanReviewMode::default())),
                    pending_plan_reviews: Arc::new(Mutex::new(HashMap::new())),
                    manual_mode: false,
                    managed: false,
                },
            );
        }

        assert_eq!(active_usage_providers(&pane_metas), (false, true, false));
    }

    #[test]
    fn active_usage_providers_detects_explicit_minimax_provider() {
        let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::new()));
        let child_process = Arc::new(Mutex::new(None));

        {
            let mut metas = pane_metas.lock().unwrap();
            metas.insert(
                7,
                PaneMeta {
                    kind: shared::PaneKind::Agent,
                    mode: shared::PaneMode::Interactive,
                    provider: Provider::Minimax,
                    label: "MiniMax 7".to_string(),
                    prompt: None,
                    model: Some("MiniMax-M2.7".to_string()),
                    effort: None,
                    min_iteration_interval_minutes: None,
                    child_process,
                    streaming_interrupt_tx: Arc::new(Mutex::new(None)),
                    control_response_tx: Arc::new(Mutex::new(None)),
                    pending_questions: Arc::new(Mutex::new(HashMap::new())),
                    effort_arc: Arc::new(Mutex::new(None)),
                    worktree_path: None,
                    role: None,
                    goal: None,
                    backstory: None,
                    plan_review_mode: shared::PlanReviewMode::default(),
                    plan_review_mode_arc: Arc::new(Mutex::new(shared::PlanReviewMode::default())),
                    pending_plan_reviews: Arc::new(Mutex::new(HashMap::new())),
                    manual_mode: false,
                    managed: false,
                },
            );
        }

        assert_eq!(active_usage_providers(&pane_metas), (false, false, false));
    }

    #[test]
    fn auto_cancel_pending_questions_drains_only_target_pane_with_denials() {
        let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::new()));
        let (target_tx, target_rx) = mpsc::channel::<String>();
        let (other_tx, other_rx) = mpsc::channel::<String>();

        let target_meta = test_pane_meta(Provider::Claude, true, None, Arc::new(Mutex::new(None)));
        *target_meta.control_response_tx.lock().unwrap() = Some(target_tx);
        seed_pending_question(&target_meta, "toolu-a", "req-a");
        seed_pending_question(&target_meta, "toolu-b", "req-b");

        let other_meta = test_pane_meta(Provider::Claude, true, None, Arc::new(Mutex::new(None)));
        *other_meta.control_response_tx.lock().unwrap() = Some(other_tx);
        seed_pending_question(&other_meta, "toolu-other", "req-other");

        {
            let mut metas = pane_metas.lock().unwrap();
            metas.insert(7, target_meta);
            metas.insert(8, other_meta);
        }

        let cancelled = auto_cancel_pending_questions_for_new_input(&pane_metas, 7);

        assert_eq!(cancelled.len(), 2);
        assert!(pending_question_ids(&pane_metas, 7).is_empty());
        assert_eq!(
            pending_question_ids(&pane_metas, 8),
            HashSet::from(["toolu-other".to_string()])
        );
        assert!(other_rx.try_recv().is_err());

        let mut denials = HashMap::new();
        for _ in 0..2 {
            let payload = target_rx
                .try_recv()
                .expect("target pane should receive a denial");
            let value: serde_json::Value =
                serde_json::from_str(&payload).expect("denial should be valid json");
            assert_eq!(value["type"], "control_response");
            assert_eq!(value["response"]["subtype"], "success");
            assert_eq!(value["response"]["response"]["behavior"], "deny");
            assert_eq!(
                value["response"]["response"]["message"],
                "User cancelled the question by sending a new prompt."
            );
            denials.insert(
                value["response"]["response"]["toolUseID"]
                    .as_str()
                    .unwrap()
                    .to_string(),
                value["response"]["request_id"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            );
        }

        assert_eq!(denials.get("toolu-a").map(String::as_str), Some("req-a"));
        assert_eq!(denials.get("toolu-b").map(String::as_str), Some("req-b"));
        assert!(target_rx.try_recv().is_err());
        assert_eq!(
            ASK_USER_QUESTION_AUTO_CANCEL_STATUS,
            "[Pending question auto-cancelled: new message replaces it]"
        );
    }

    #[test]
    fn auto_cancel_no_sender_and_missing_meta_still_allow_input_forwarding() {
        let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::new()));
        let no_sender_meta =
            test_pane_meta(Provider::Claude, true, None, Arc::new(Mutex::new(None)));
        seed_pending_question(&no_sender_meta, "toolu-waiting", "req-waiting");
        pane_metas.lock().unwrap().insert(9, no_sender_meta);

        let input_channels: InputChannels = Arc::new(Mutex::new(HashMap::new()));
        let (pane_tx, pane_rx) = mpsc::channel::<(String, bool)>();
        input_channels.lock().unwrap().insert(9, pane_tx);

        let cancelled = auto_cancel_pending_questions_for_new_input(&pane_metas, 9);
        let routed = route_web_input_to_pane(&input_channels, 9, "new prompt");

        assert!(cancelled.is_empty());
        assert_eq!(
            pending_question_ids(&pane_metas, 9),
            HashSet::from(["toolu-waiting".to_string()])
        );
        assert_eq!(routed, PaneInputRouteResult::Sent);
        assert_eq!(
            pane_rx.try_recv().unwrap(),
            ("new prompt".to_string(), false)
        );

        let (missing_meta_tx, missing_meta_rx) = mpsc::channel::<(String, bool)>();
        input_channels.lock().unwrap().insert(99, missing_meta_tx);

        let cancelled = auto_cancel_pending_questions_for_new_input(&pane_metas, 99);
        let routed = route_web_input_to_pane(&input_channels, 99, "prompt for missing meta");

        assert!(cancelled.is_empty());
        assert_eq!(routed, PaneInputRouteResult::Sent);
        assert_eq!(
            missing_meta_rx.try_recv().unwrap(),
            ("prompt for missing meta".to_string(), false)
        );

        assert_eq!(
            route_web_input_to_pane(&input_channels, 100, "no receiver"),
            PaneInputRouteResult::MissingChannel
        );
    }

    #[test]
    fn active_usage_providers_ignores_retired_glm_model() {
        let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::new()));
        let child_process = Arc::new(Mutex::new(None));

        {
            let mut metas = pane_metas.lock().unwrap();
            metas.insert(
                9,
                PaneMeta {
                    kind: shared::PaneKind::Agent,
                    mode: shared::PaneMode::Interactive,
                    provider: Provider::Claude,
                    label: "GLM 9".to_string(),
                    prompt: None,
                    model: Some("glm-5.1".to_string()),
                    effort: None,
                    min_iteration_interval_minutes: None,
                    child_process,
                    streaming_interrupt_tx: Arc::new(Mutex::new(None)),
                    control_response_tx: Arc::new(Mutex::new(None)),
                    pending_questions: Arc::new(Mutex::new(HashMap::new())),
                    effort_arc: Arc::new(Mutex::new(None)),
                    worktree_path: None,
                    role: None,
                    goal: None,
                    backstory: None,
                    plan_review_mode: shared::PlanReviewMode::default(),
                    plan_review_mode_arc: Arc::new(Mutex::new(shared::PlanReviewMode::default())),
                    pending_plan_reviews: Arc::new(Mutex::new(HashMap::new())),
                    manual_mode: false,
                    managed: false,
                },
            );
        }

        assert_eq!(active_usage_providers(&pane_metas), (false, false, false));
    }

    #[test]
    fn active_usage_providers_does_not_infer_provider_from_label() {
        let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::new()));
        let child_process = Arc::new(Mutex::new(None));

        {
            let mut metas = pane_metas.lock().unwrap();
            metas.insert(
                10,
                PaneMeta {
                    kind: shared::PaneKind::Agent,
                    mode: shared::PaneMode::Interactive,
                    provider: Provider::Claude,
                    label: "GLM Experimental".to_string(),
                    prompt: None,
                    model: None,
                    effort: None,
                    min_iteration_interval_minutes: None,
                    child_process,
                    streaming_interrupt_tx: Arc::new(Mutex::new(None)),
                    control_response_tx: Arc::new(Mutex::new(None)),
                    pending_questions: Arc::new(Mutex::new(HashMap::new())),
                    effort_arc: Arc::new(Mutex::new(None)),
                    worktree_path: None,
                    role: None,
                    goal: None,
                    backstory: None,
                    plan_review_mode: shared::PlanReviewMode::default(),
                    plan_review_mode_arc: Arc::new(Mutex::new(shared::PlanReviewMode::default())),
                    pending_plan_reviews: Arc::new(Mutex::new(HashMap::new())),
                    manual_mode: false,
                    managed: false,
                },
            );
        }

        assert_eq!(active_usage_providers(&pane_metas), (true, false, false));
    }

    #[test]
    fn control_request_parser_parks_ask_user_question() {
        use super::{try_handle_control_request, PendingAskQuestion};
        use shared::CliToServer;
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        let pending: Arc<Mutex<HashMap<String, PendingAskQuestion>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (cr_tx, cr_rx) = std::sync::mpsc::channel::<String>();
        let (server_tx, _server_rx) = tokio::sync::mpsc::channel::<CliToServer>(8);

        let line = r#"{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Q?","header":"H","options":[{"label":"A","description":"a"}],"multiSelect":false}]},"tool_use_id":"toolu_abc"}}"#;
        let handled = try_handle_control_request(
            line,
            42,
            Uuid::new_v4(),
            shared::PaneType::Interactive,
            &pending,
            &cr_tx,
            &server_tx,
            None,
            shared::PlanReviewMode::Never,
        );
        assert_eq!(handled, Some(true));
        let guard = pending.lock().unwrap();
        assert!(guard.contains_key("toolu_abc"));
        assert_eq!(guard["toolu_abc"].request_id, "req-1");
        // AskUserQuestion does NOT auto-approve; channel stays empty.
        assert!(cr_rx.try_recv().is_err());
    }

    #[test]
    fn control_request_parser_auto_approves_other_tools() {
        use super::{try_handle_control_request, PendingAskQuestion};
        use shared::CliToServer;
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        let pending: Arc<Mutex<HashMap<String, PendingAskQuestion>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (cr_tx, cr_rx) = std::sync::mpsc::channel::<String>();
        let (server_tx, _server_rx) = tokio::sync::mpsc::channel::<CliToServer>(8);

        let line = r#"{"type":"control_request","request_id":"req-2","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"},"tool_use_id":"toolu_xyz"}}"#;
        let handled = try_handle_control_request(
            line,
            42,
            Uuid::new_v4(),
            shared::PaneType::Interactive,
            &pending,
            &cr_tx,
            &server_tx,
            None,
            shared::PlanReviewMode::Never,
        );
        assert_eq!(handled, Some(true));
        let payload = cr_rx
            .try_recv()
            .expect("auto-approve should send a control_response");
        assert!(payload.contains("\"behavior\":\"allow\""));
        assert!(payload.contains("\"request_id\":\"req-2\""));
        assert!(payload.contains("\"toolUseID\":\"toolu_xyz\""));
    }

    #[test]
    fn control_request_parser_ignores_non_control_lines() {
        use super::{try_handle_control_request, PendingAskQuestion};
        use shared::CliToServer;
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        let pending: Arc<Mutex<HashMap<String, PendingAskQuestion>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (cr_tx, _cr_rx) = std::sync::mpsc::channel::<String>();
        let (server_tx, _server_rx) = tokio::sync::mpsc::channel::<CliToServer>(8);

        // Normal assistant stream message — must be left alone for parse_agent_output.
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}],"model":"claude"},"session_id":"abc"}"#;
        let handled = try_handle_control_request(
            line,
            1,
            Uuid::new_v4(),
            shared::PaneType::Interactive,
            &pending,
            &cr_tx,
            &server_tx,
            None,
            shared::PlanReviewMode::Never,
        );
        assert_eq!(handled, None);
    }

    #[test]
    fn save_pane_configs_persists_terminal_panes() {
        // The regression: a terminal pane that never reached `.apas` was gone
        // on the next CLI start, because `.apas` is the only record of the
        // roster. This asserts the persistence layer carries `kind`, so a
        // restored pane comes back as a terminal rather than an agent.
        let _config = crate::config::test_config::isolated_config_dir();
        let dir = tempfile::tempdir().expect("temp project dir");
        let working_dir = dir.path().to_string_lossy().to_string();
        let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::new()));
        let pane_sessions = Arc::new(Mutex::new(HashMap::new()));
        let pane_pauses: PanePauses = Arc::new(Mutex::new(HashMap::new()));
        let pane_stop_requests: PaneStopRequests = Arc::new(Mutex::new(HashMap::new()));

        {
            let mut metas = pane_metas.lock().unwrap();
            let mut agent =
                test_pane_meta(Provider::Claude, false, None, Arc::new(Mutex::new(None)));
            agent.kind = shared::PaneKind::Agent;
            metas.insert(7, agent);

            let mut terminal =
                test_pane_meta(Provider::Codex, false, None, Arc::new(Mutex::new(None)));
            terminal.kind = shared::PaneKind::Terminal;
            terminal.mode = shared::PaneMode::Interactive;
            metas.insert(8, terminal);
        }
        {
            let mut sessions = pane_sessions.lock().unwrap();
            sessions.insert(7, Uuid::new_v4());
            sessions.insert(8, Uuid::new_v4());
        }

        save_pane_configs(
            &working_dir,
            &pane_sessions,
            &pane_metas,
            &pane_pauses,
            &pane_stop_requests,
        );

        let metadata = get_or_create_project(dir.path()).expect("metadata should reload");
        let terminal = metadata
            .panes
            .iter()
            .find(|p| p.pane_id == 8)
            .expect("terminal pane must survive the save");
        assert_eq!(terminal.kind, shared::PaneKind::Terminal);
        assert_eq!(terminal.provider, Provider::Codex);

        let agent = metadata
            .panes
            .iter()
            .find(|p| p.pane_id == 7)
            .expect("agent pane");
        assert_eq!(agent.kind, shared::PaneKind::Agent);
    }

    // --- self-reported terminal-pane history ------------------------------

    #[test]
    fn terminal_history_uses_the_provider_process_working_directory() {
        let project = Path::new("/srv/project");
        assert_eq!(
            super::terminal_transcript_cwd(project, Some("/srv/worktrees/claude-7")),
            Path::new("/srv/worktrees/claude-7")
        );
        assert_eq!(super::terminal_transcript_cwd(project, None), project);
    }

    #[test]
    fn only_restored_terminal_history_is_suppressed_on_first_poll() {
        assert_eq!(super::initial_terminal_transcript_cursor(true, 8), 8);
        assert_eq!(
            super::initial_terminal_transcript_cursor(false, 8),
            0,
            "a newly-created pane's fast first turn must not be skipped"
        );
    }

    #[test]
    fn restored_terminal_questions_repopulate_pending_state_without_resetting_delivery() {
        let question = TurnRecord {
            ts: "1".to_string(),
            pane_id: 7,
            role: "assistant".to_string(),
            text: String::new(),
            model: None,
            input_tokens: None,
            output_tokens: None,
            completes_work: false,
            question: Some(TurnQuestion {
                tool_use_id: "question-7".to_string(),
                tool_name: "AskUserQuestion".to_string(),
                input: serde_json::json!({
                    "questions": [{
                        "question": "Choose",
                        "options": [{"label": "A"}, {"label": "B"}]
                    }]
                }),
            }),
            answer: None,
        };
        let mut pending = HashMap::new();

        assert!(reconcile_terminal_questions_for_pane(
            &mut pending,
            7,
            std::slice::from_ref(&question),
        ));
        assert_eq!(pending["question-7"].pane_id, 7);
        assert_eq!(pending["question-7"].options[0].1, vec!["A", "B"]);

        pending.get_mut("question-7").unwrap().delivered = true;
        assert!(reconcile_terminal_questions_for_pane(
            &mut pending,
            7,
            std::slice::from_ref(&question),
        ));
        assert!(
            pending["question-7"].delivered,
            "ordinary transcript polls must not make a delivered answer writable twice"
        );

        let answer = TurnRecord {
            ts: "2".to_string(),
            pane_id: 7,
            role: "user".to_string(),
            text: String::new(),
            model: None,
            input_tokens: None,
            output_tokens: None,
            completes_work: false,
            question: None,
            answer: Some(TurnAnswer {
                tool_use_id: "question-7".to_string(),
                recorded: "A".to_string(),
            }),
        };
        assert!(!reconcile_terminal_questions_for_pane(
            &mut pending,
            7,
            &[question, answer],
        ));
        assert!(pending.is_empty());
    }

    #[test]
    fn an_active_codex_turn_is_republished_after_reconnect() {
        let mut states = HashMap::new();

        assert_eq!(
            super::observe_codex_working_state(&mut states, 293, Some(true), 1),
            Some(true),
            "an already-active restored pane must repopulate server status"
        );
        assert_eq!(
            super::observe_codex_working_state(&mut states, 293, Some(true), 1),
            None,
            "unchanged transcript polls must not resend status"
        );
        assert_eq!(
            super::observe_codex_working_state(&mut states, 293, Some(true), 2),
            Some(true),
            "an unchanged active turn must repopulate status after reconnect"
        );
        assert_eq!(
            super::observe_codex_working_state(&mut states, 293, Some(false), 2),
            Some(false),
            "the provider completion must clear the recovered status"
        );
    }

    #[test]
    fn an_initial_idle_codex_observation_cannot_clear_fresh_web_input() {
        let mut states = HashMap::new();

        assert_eq!(
            super::observe_codex_working_state(&mut states, 293, Some(false), 1),
            None,
            "idle is already the server default and should stay silent"
        );
        assert_eq!(
            super::observe_codex_working_state(&mut states, 293, None, 1),
            None,
            "legacy transcripts without lifecycle events stay compatible"
        );
        assert_eq!(
            super::observe_codex_working_state(&mut states, 293, Some(true), 1),
            Some(true)
        );
    }

    fn recorded_turn(role: &str, text: &str) -> crate::conversation::TurnRecord {
        crate::conversation::TurnRecord {
            ts: "2026-08-04T00:00:00Z".to_string(),
            pane_id: 42,
            role: role.to_string(),
            text: text.to_string(),
            model: Some("claude-opus-5".to_string()),
            input_tokens: None,
            output_tokens: None,
            completes_work: false,
            question: None,
            answer: None,
        }
    }

    /// The outcome exists so a project can end without ending the process.
    /// These pin the precedence the caller depends on.
    #[test]
    fn a_rejected_session_reports_a_stop_reason_rather_than_ending_anything() {
        let stop_reason: super::ProjectStopReason = Arc::new(Mutex::new(None));
        // What the ws loop does on ServerToCli::SessionRejected, which used to
        // be process::exit(2) — survivable only while a process hosted exactly
        // one project.
        if let Ok(mut slot) = stop_reason.lock() {
            *slot = Some("server rejected session abc: project is suspended".to_string());
        }

        let taken = stop_reason.lock().ok().and_then(|mut slot| slot.take());
        let outcome = match taken {
            Some(reason) => super::ProjectOutcome::Stopped(reason),
            None => super::ProjectOutcome::Completed,
        };
        match outcome {
            super::ProjectOutcome::Stopped(reason) => {
                assert!(reason.contains("project is suspended"))
            }
            other => panic!("expected a stop reason, got {other:?}"),
        }
    }

    /// A reboot outranks a stop reason: the project is being replaced, so why
    /// its previous incarnation ended is not what the caller acts on.
    #[test]
    fn a_reboot_outranks_a_stop_reason() {
        let reboot = Arc::new(AtomicBool::new(true));
        let stopped = Some("server rejected session abc".to_string());

        let outcome = if reboot.load(Ordering::SeqCst) {
            super::ProjectOutcome::RebootRequested
        } else {
            match stopped {
                Some(reason) => super::ProjectOutcome::Stopped(reason),
                None => super::ProjectOutcome::Completed,
            }
        };
        assert_eq!(outcome, super::ProjectOutcome::RebootRequested);
    }

    #[test]
    fn an_ordinary_shutdown_completes() {
        let stop_reason: super::ProjectStopReason = Arc::new(Mutex::new(None));
        let taken = stop_reason.lock().ok().and_then(|mut slot| slot.take());
        assert_eq!(
            match taken {
                Some(reason) => super::ProjectOutcome::Stopped(reason),
                None => super::ProjectOutcome::Completed,
            },
            super::ProjectOutcome::Completed
        );
    }

    fn pending_fruit_question(delivered: bool) -> HashMap<String, super::TerminalQuestion> {
        let mut questions = HashMap::new();
        questions.insert(
            "toolu_1".to_string(),
            super::TerminalQuestion {
                pane_id: 7,
                options: fruit_options(),
                delivered,
            },
        );
        questions
    }

    fn banana() -> HashMap<String, String> {
        let mut answers = HashMap::new();
        answers.insert("Pick a fruit".to_string(), "Banana".to_string());
        answers
    }

    /// The guards exist because the write is blind: a stale tab, a retransmit,
    /// or a question that was answered in the terminal must all send nothing.
    #[test]
    fn only_a_pending_undelivered_question_is_ever_typed_into() {
        assert_eq!(
            super::plan_terminal_answer(&pending_fruit_question(false), "toolu_1", &banana()),
            super::TerminalAnswerPlan::Deliver {
                pane_id: 7,
                keys: b"\x1b[B\r".to_vec()
            },
        );

        // The web resends unacked answers; the pane must not type twice.
        assert_eq!(
            super::plan_terminal_answer(&pending_fruit_question(true), "toolu_1", &banana()),
            super::TerminalAnswerPlan::AlreadyDelivered { pane_id: 7 },
        );

        // Answered in the terminal already: the transcript removed it, so a
        // client that has not caught up gets nothing sent on its behalf.
        assert_eq!(
            super::plan_terminal_answer(&HashMap::new(), "toolu_1", &banana()),
            super::TerminalAnswerPlan::NotPending,
        );

        // An id belonging to some other pane's question is not this one's.
        assert_eq!(
            super::plan_terminal_answer(&pending_fruit_question(false), "toolu_other", &banana()),
            super::TerminalAnswerPlan::NotPending,
        );

        let mut bogus = HashMap::new();
        bogus.insert("Pick a fruit".to_string(), "Durian".to_string());
        assert_eq!(
            super::plan_terminal_answer(&pending_fruit_question(false), "toolu_1", &bogus),
            super::TerminalAnswerPlan::UnknownOption { pane_id: 7 },
        );
    }

    fn fruit_options() -> Vec<(String, Vec<String>)> {
        vec![(
            "Pick a fruit".to_string(),
            vec![
                "Apple".to_string(),
                "Banana".to_string(),
                "Cherry".to_string(),
            ],
        )]
    }

    /// The encoding is the TUI's own stated contract: ↑/↓ to navigate, Enter
    /// to select. Selecting the first option must therefore send no movement
    /// at all — the cursor already starts there.
    #[test]
    fn an_answer_encodes_as_the_arrow_keys_the_picker_accepts() {
        let mut answers = HashMap::new();
        answers.insert("Pick a fruit".to_string(), "Apple".to_string());
        assert_eq!(
            super::encode_answer_keystrokes(&fruit_options(), &answers),
            Some(b"\r".to_vec()),
        );

        answers.insert("Pick a fruit".to_string(), "Cherry".to_string());
        assert_eq!(
            super::encode_answer_keystrokes(&fruit_options(), &answers),
            Some(b"\x1b[B\x1b[B\r".to_vec()),
            "two steps down from the default first entry",
        );
    }

    /// A label the question never offered would otherwise be encoded as some
    /// arbitrary position — answering on the human's behalf. Refuse instead.
    #[test]
    fn an_unoffered_option_is_refused_rather_than_guessed() {
        let mut answers = HashMap::new();
        answers.insert("Pick a fruit".to_string(), "Durian".to_string());
        assert_eq!(
            super::encode_answer_keystrokes(&fruit_options(), &answers),
            None,
        );

        let mut wrong_question = HashMap::new();
        wrong_question.insert("Pick a colour".to_string(), "Apple".to_string());
        assert_eq!(
            super::encode_answer_keystrokes(&fruit_options(), &wrong_question),
            None,
        );
    }

    /// A multi-question call walks the picker question by question, so the
    /// selections are concatenated in the order the questions were asked.
    #[test]
    fn a_multi_question_answer_is_one_sequence_in_question_order() {
        let options = vec![
            ("First".to_string(), vec!["a".to_string(), "b".to_string()]),
            ("Second".to_string(), vec!["x".to_string(), "y".to_string()]),
        ];
        let mut answers = HashMap::new();
        answers.insert("First".to_string(), "b".to_string());
        answers.insert("Second".to_string(), "x".to_string());
        assert_eq!(
            super::encode_answer_keystrokes(&options, &answers),
            Some(b"\x1b[B\r\r".to_vec()),
        );
    }

    /// The option list the encoder addresses comes from the tool call itself,
    /// so the positions match what the TUI renders.
    #[test]
    fn question_options_are_read_from_the_tool_call_in_order() {
        let input = serde_json::json!({
            "questions": [{
                "question": "Pick a fruit",
                "options": [{"label": "Apple"}, {"label": "Banana"}]
            }]
        });
        assert_eq!(
            super::question_options(&input),
            vec![(
                "Pick a fruit".to_string(),
                vec!["Apple".to_string(), "Banana".to_string()]
            )]
        );
        assert!(super::question_options(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn a_question_turn_becomes_the_tool_use_the_existing_card_renders() {
        let mut turn = recorded_turn("assistant", "");
        turn.question = Some(crate::conversation::TurnQuestion {
            tool_use_id: "toolu_1".into(),
            tool_name: "AskUserQuestion".into(),
            input: serde_json::json!({
                "questions": [{"question": "Pick a fruit", "options": [{"label": "Apple"}]}]
            }),
        });
        let msgs =
            super::conversation_turn_to_stream_messages(&turn, Uuid::new_v4(), Uuid::new_v4());

        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            CliToServer::StreamMessage {
                message: shared::ClaudeStreamMessage::Assistant { message, extra, .. },
                ..
            } => {
                match &message.content[0] {
                    shared::ClaudeContentBlock::ToolUse { id, name, input } => {
                        assert_eq!(id, "toolu_1");
                        assert_eq!(name, "AskUserQuestion");
                        assert_eq!(input["questions"][0]["question"], "Pick a fruit");
                    }
                    other => panic!("expected a tool_use block, got {other:?}"),
                }
                // A pending question is not an idle boundary.
                assert_eq!(extra["terminal_turn_complete"], serde_json::json!(false));
            }
            other => panic!("expected an assistant stream message, got {other:?}"),
        }
    }

    #[test]
    fn an_answer_turn_becomes_the_tool_result_the_ack_path_waits_for() {
        let mut turn = recorded_turn("user", "");
        turn.answer = Some(crate::conversation::TurnAnswer {
            tool_use_id: "toolu_1".into(),
            recorded: "The user answered: \"Pick a fruit\"=\"Banana\"".into(),
        });
        let msgs =
            super::conversation_turn_to_stream_messages(&turn, Uuid::new_v4(), Uuid::new_v4());

        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            CliToServer::StreamMessage {
                message: shared::ClaudeStreamMessage::User { message, .. },
                ..
            } => match &message.content[0] {
                shared::ClaudeContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => {
                    assert_eq!(tool_use_id, "toolu_1");
                    assert!(content.contains("Banana"));
                    assert!(!is_error);
                }
                other => panic!("expected a tool_result block, got {other:?}"),
            },
            other => panic!("expected a user stream message, got {other:?}"),
        }
    }

    #[test]
    fn a_reported_assistant_turn_becomes_the_message_an_agent_pane_would_send() {
        // The point of the translation: no new wire message, storage path, or
        // renderer — a terminal pane's history rides the agent-pane channel.
        let sid = Uuid::new_v4();
        let csid = Uuid::new_v4();
        let msgs = super::conversation_turn_to_stream_messages(
            &recorded_turn("assistant", "here is the answer"),
            sid,
            csid,
        );

        assert_eq!(msgs.len(), 1, "no usage reported, so no Result message");
        match &msgs[0] {
            CliToServer::StreamMessage {
                session_id,
                message: shared::ClaudeStreamMessage::Assistant { message, extra, .. },
                pane_id,
                ..
            } => {
                assert_eq!(*session_id, sid);
                assert_eq!(*pane_id, Some(42));
                assert_eq!(message.model, "claude-opus-5");
                assert_eq!(extra["terminal_turn_complete"], false);
                match &message.content[0] {
                    shared::ClaudeContentBlock::Text { text } => {
                        assert_eq!(text, "here is the answer")
                    }
                    other => panic!("unexpected block: {other:?}"),
                }
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn a_completed_terminal_turn_carries_the_idle_boundary() {
        let mut turn = recorded_turn("assistant", "done");
        turn.completes_work = true;
        let msgs =
            super::conversation_turn_to_stream_messages(&turn, Uuid::new_v4(), Uuid::new_v4());
        let wire = serde_json::to_string(&msgs[0]).expect("completion marker serializes");
        let round_trip: CliToServer =
            serde_json::from_str(&wire).expect("completion marker deserializes");
        match &round_trip {
            CliToServer::StreamMessage {
                message: shared::ClaudeStreamMessage::Assistant { extra, .. },
                ..
            } => assert_eq!(extra["terminal_turn_complete"], true),
            other => panic!("expected assistant message, got {other:?}"),
        }
    }

    #[test]
    fn a_user_turn_goes_over_user_input_not_a_stream_message() {
        // Regression: it used to be sent as `StreamMessage` with a
        // `ClaudeStreamMessage::User`. That variant means "tool result" to the
        // server — its converter only looks for `ToolResult` blocks — so a
        // `Text` block was silently dropped and every message the human typed
        // was missing from the conversation view, while assistant turns worked.
        let msgs = super::conversation_turn_to_stream_messages(
            &recorded_turn("user", "do the thing"),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            CliToServer::UserInput { text, pane_id, .. } => {
                assert_eq!(text, "do the thing");
                assert_eq!(*pane_id, Some(42));
            }
            other => panic!("user turns must use UserInput, got {other:?}"),
        }
    }

    #[test]
    fn reported_tokens_produce_the_result_message_usage_accounting_reads() {
        // `ws_cli` bills a turn only from a Result variant's `extra.usage`, so
        // without this second message a turn is recorded but costs nothing.
        let mut turn = recorded_turn("assistant", "done");
        turn.input_tokens = Some(1200);
        turn.output_tokens = Some(340);

        let msgs =
            super::conversation_turn_to_stream_messages(&turn, Uuid::new_v4(), Uuid::new_v4());
        assert_eq!(msgs.len(), 2);
        match &msgs[1] {
            CliToServer::StreamMessage {
                message:
                    shared::ClaudeStreamMessage::Result {
                        subtype,
                        extra,
                        total_cost_usd,
                        ..
                    },
                pane_id,
                ..
            } => {
                assert_eq!(subtype, "success");
                assert_eq!(*pane_id, Some(42));
                assert_eq!(extra["usage"]["input_tokens"], 1200);
                assert_eq!(extra["usage"]["output_tokens"], 340);
                // A self-reporting agent cannot know what it was billed;
                // inventing a number would corrupt the roll-up.
                assert_eq!(*total_cost_usd, 0.0);
            }
            other => panic!("expected a Result message, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_role_still_reaches_the_web() {
        // Degrade to "rendered plainly", never to a dropped turn.
        let msgs = super::conversation_turn_to_stream_messages(
            &recorded_turn("tool", "output"),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        assert_eq!(msgs.len(), 1);
        assert!(matches!(&msgs[0], CliToServer::UserInput { .. }));
    }

    #[test]
    fn operational_updates_cannot_change_cluster_governed_local_fields() {
        let _config = crate::config::test_config::isolated_config_dir();
        let dir = tempfile::tempdir().expect("temp project dir");
        let mut meta = get_or_create_project(dir.path()).expect("project");
        meta.team_enabled = true;
        meta.disallowed_tab_types = vec!["terminal:codex".to_string()];
        save_project(dir.path(), &meta).expect("legacy policy");

        let echo = update_project_operations(dir.path(), Uuid::new_v4(), true, true)
            .expect("update workflow operations");
        let after = get_or_create_project(dir.path()).expect("reload project");
        assert!(after.team_enabled);
        assert_eq!(after.disallowed_tab_types, vec!["terminal:codex"]);
        assert!(matches!(
            echo,
            CliToServer::ProjectFlagsChanged {
                auto_approve_todos: true,
                auto_merge_prs: true,
                team_enabled: true,
                ..
            }
        ));
    }

    // --- team mode on/off ---------------------------------------------------

    /// The floor a fresh watch reports decides whether a pane can adopt a
    /// conversation that was already on disk. It used to be the epoch whenever
    /// the pane's own transcript did not exist yet — which is every new pane,
    /// since Claude writes the file on the first turn — so everything in the
    /// directory looked newer.
    #[test]
    fn a_fresh_claude_watch_never_accepts_a_transcript_older_than_itself() {
        let dir = tempfile::tempdir().unwrap();
        // Conversations already in the directory when the pane appears.
        for name in ["old-conversation.jsonl", "recently-touched.jsonl"] {
            std::fs::write(dir.path().join(name), "{}").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let newest_existing = std::fs::metadata(dir.path().join("recently-touched.jsonl"))
            .unwrap()
            .modified()
            .unwrap();

        // The pane's own transcript: pinned, not written yet.
        let watch = super::ClaudeWatchState::new(dir.path().join("brand-new-pane.jsonl"));

        assert_ne!(watch.switch_floor(), std::time::SystemTime::UNIX_EPOCH);
        assert!(
            watch.switch_floor() >= newest_existing,
            "a watch with no file behind it must not accept anything already there"
        );
    }

    /// The floor comes from the directory, never the wall clock, because these
    /// transcripts are on NFS: comparing their mtimes against this host's clock
    /// is a cross-domain comparison that skew can break in both directions.
    #[test]
    fn the_switch_floor_is_taken_from_the_directory_not_the_clock() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("existing.jsonl"), "{}").unwrap();
        let existing = std::fs::metadata(dir.path().join("existing.jsonl"))
            .unwrap()
            .modified()
            .unwrap();

        let watch = super::ClaudeWatchState::new(dir.path().join("pinned.jsonl"));
        assert_eq!(
            watch.switch_floor(),
            existing,
            "the floor is the newest transcript already present"
        );
    }

    #[test]
    fn team_mode_is_off_for_a_project_whose_apas_predates_the_flag() {
        // The upgrade path. `.apas` files in the wild have no `team_enabled`,
        // and absent must read as off — team mode spawns autonomous panes that
        // can open PRs, so it must never arrive switched on.
        let _config = crate::config::test_config::isolated_config_dir();
        let dir = tempfile::tempdir().expect("temp project dir");
        std::fs::write(
            dir.path().join(".apas"),
            r#"{
                "id": "8c4b0c1e-0000-4000-8000-000000000001",
                "name": "legacy",
                "created_at": "2026-01-01T00:00:00Z",
                "auto_approve_todos": true,
                "auto_merge_prs": true,
                "panes": []
            }"#,
        )
        .expect("write legacy .apas");

        let meta = get_or_create_project(dir.path()).expect("load legacy metadata");

        assert!(!meta.team_enabled, "absent team_enabled must read as off");
        // The neighbouring flags still round-trip, so this is the new field
        // defaulting rather than the whole file failing to parse.
        assert!(meta.auto_approve_todos);
        assert!(meta.auto_merge_prs);
    }

    // --- closing a pane must not leave the agent's subtree running ---------
    //
    // A real agent is a process *parent*: claude/codex run bash commands,
    // subagents, and this pane's own `apas mcp-server`. `sh -c 'sleep &'`
    // stands in for all of them — a child of the agent, not of APAS.

    #[cfg(unix)]
    fn spawn_agent_with_grandchild() -> (std::process::Child, u32) {
        use std::io::BufRead;
        let mut command = std::process::Command::new("sh");
        command
            // `wait` is a builtin, so the shell can't `exec`-optimize itself
            // away and stays around as the group leader.
            .arg("-c")
            .arg("sleep 60 & echo $!; wait")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        super::spawn_in_own_process_group(&mut command);
        let mut child = command.spawn().expect("spawn stand-in agent");
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().expect("agent stdout"))
            .read_line(&mut line)
            .expect("read grandchild pid");
        let grandchild = line.trim().parse().expect("grandchild pid");
        (child, grandchild)
    }

    #[cfg(unix)]
    fn pid_is_alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    #[cfg(unix)]
    fn wait_until_gone(pid: u32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if !pid_is_alive(pid) {
                return true;
            }
            thread::sleep(Duration::from_millis(25));
        }
        !pid_is_alive(pid)
    }

    #[test]
    #[cfg(unix)]
    fn closing_a_pane_kills_the_agents_grandchildren() {
        let (mut child, grandchild) = spawn_agent_with_grandchild();
        assert!(pid_is_alive(grandchild), "grandchild should start alive");

        super::kill_pane_child_group(&mut child);

        assert!(
            wait_until_gone(grandchild, Duration::from_secs(3)),
            "closing a pane left the agent's child running — a real one would \
             be a bash command, a subagent, or the pane's apas mcp-server"
        );
    }

    /// Proves the test above measures something: the pre-fix close path — a
    /// bare `child.kill()` — reparents the grandchild to init and it lives on.
    #[test]
    #[cfg(unix)]
    fn killing_only_the_agent_orphans_its_grandchildren() {
        let (mut child, grandchild) = spawn_agent_with_grandchild();

        let _ = child.kill();
        let _ = child.wait();
        thread::sleep(Duration::from_millis(300));

        let survived = pid_is_alive(grandchild);
        // Reap before asserting so a failure can't leak a stray `sleep`.
        unsafe {
            libc::kill(grandchild as i32, libc::SIGKILL);
        }
        assert!(
            survived,
            "the regression guard is only meaningful while a bare child kill \
             orphans the subtree"
        );
    }

    /// The guard that makes a group kill safe. A pane spawned without
    /// `setpgid` sits in APAS's own process group, so `kill(-pid)` there would
    /// signal APAS and every other pane. Such a pane must fall back to killing
    /// the child alone.
    #[test]
    #[cfg(unix)]
    fn a_pane_that_leads_no_group_is_never_group_killed() {
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn");

        assert!(!super::leads_own_process_group(child.id()));
        assert!(
            !super::sigterm_pane_child_group(child.id()),
            "group signal must be refused for a non-leader"
        );

        // The fallback still kills and reaps it.
        super::kill_pane_child_group(&mut child);
        assert!(
            child.try_wait().is_ok(),
            "child must be reaped, not left a zombie"
        );
    }
}

/// Run the deadloop (autonomous) session on any pane
#[allow(clippy::too_many_arguments)]
fn run_deadloop_session(
    binary_path: &str,
    working_dir: &str,
    worktree_path: Option<String>,
    system_prompt: Option<String>,
    plan_review_mode_arc: Arc<Mutex<shared::PlanReviewMode>>,
    pending_plan_reviews: Arc<Mutex<HashMap<String, PendingPlanReview>>>,
    session_id: Uuid,
    claude_session_id: Uuid,
    pane_id: u32,
    prompt: &str,
    model: Option<String>,
    effort: Option<String>,
    min_iteration_interval_minutes: u64,
    provider: &Provider,
    output_tx: mpsc::Sender<PaneOutput>,
    server_tx: tokio_mpsc::Sender<CliToServer>,
    shutdown: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    stop_requested: Arc<AtomicBool>,
    child_process: Arc<Mutex<Option<std::process::Child>>>,
    event_tx: mpsc::Sender<TuiEvent>,
    interrupt_tx_slot: Arc<Mutex<Option<mpsc::Sender<()>>>>,
    control_response_tx_slot: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pending_questions: Arc<Mutex<HashMap<String, PendingAskQuestion>>>,
    effort_arc: Arc<Mutex<Option<String>>>,
    input_channels: InputChannels,
    // Initial value for the legacy path's `try_resume_first`. Mirror
    // of the same param on run_pane_session — false when the worker
    // is being spawned with a freshly-minted session id (e.g.
    // provider switch via UpdatePaneModel), so codex's `exec resume`
    // doesn't fail with "no rollout found".
    initial_try_resume: bool,
    // Project-shared file watcher; used to wake the deadloop early
    // when team-todo.md / .apas-team.jsonl / project_goal.md / .apas
    // change, so iterations only fire on real signal instead of
    // burning tokens on a min-interval timer.
    file_watcher: Arc<crate::file_watcher::ProjectFileWatcher>,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_deadloop_session_inner(
            binary_path,
            working_dir,
            worktree_path,
            system_prompt,
            plan_review_mode_arc,
            pending_plan_reviews,
            session_id,
            claude_session_id,
            pane_id,
            prompt,
            model,
            effort,
            min_iteration_interval_minutes,
            provider,
            output_tx.clone(),
            server_tx,
            shutdown,
            pause,
            stop_requested,
            child_process,
            event_tx,
            interrupt_tx_slot,
            control_response_tx_slot,
            pending_questions,
            effort_arc,
            input_channels,
            initial_try_resume,
            file_watcher,
        )
    }));

    if let Err(e) = result {
        let msg = if let Some(s) = e.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = e.downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown panic".to_string()
        };
        let _ = output_tx.send(PaneOutput {
            text: format!("[DEADLOOP CRASHED: {}]", msg),
            pane_id,
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeadloopWaitPlan {
    remaining: Duration,
    cursor: Instant,
}

fn deadloop_wait_plan(
    last_iteration_started_at: Instant,
    min_iteration_interval: Duration,
    wait_entry_at: Instant,
) -> Option<DeadloopWaitPlan> {
    let elapsed_since_last_start = wait_entry_at
        .checked_duration_since(last_iteration_started_at)
        .unwrap_or(Duration::ZERO);
    let remaining = min_iteration_interval
        .checked_sub(elapsed_since_last_start)
        .unwrap_or(Duration::ZERO);
    if remaining.is_zero() {
        None
    } else {
        Some(DeadloopWaitPlan {
            remaining,
            cursor: wait_entry_at,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn run_deadloop_session_inner(
    binary_path: &str,
    working_dir: &str,
    worktree_path: Option<String>,
    system_prompt: Option<String>,
    plan_review_mode_arc: Arc<Mutex<shared::PlanReviewMode>>,
    pending_plan_reviews: Arc<Mutex<HashMap<String, PendingPlanReview>>>,
    session_id: Uuid,
    claude_session_id: Uuid,
    pane_id: u32,
    prompt: &str,
    model: Option<String>,
    effort: Option<String>,
    min_iteration_interval_minutes: u64,
    provider: &Provider,
    output_tx: mpsc::Sender<PaneOutput>,
    server_tx: tokio_mpsc::Sender<CliToServer>,
    shutdown: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    stop_requested: Arc<AtomicBool>,
    child_process: Arc<Mutex<Option<std::process::Child>>>,
    event_tx: mpsc::Sender<TuiEvent>,
    interrupt_tx_slot: Arc<Mutex<Option<mpsc::Sender<()>>>>,
    control_response_tx_slot: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pending_questions: Arc<Mutex<HashMap<String, PendingAskQuestion>>>,
    effort_arc: Arc<Mutex<Option<String>>>,
    input_channels: InputChannels,
    // Same semantics as on run_pane_session — false skips the very
    // first `--resume`/`exec resume` so codex / cursor don't fail
    // against a freshly-minted session id from a provider switch.
    initial_try_resume: bool,
    file_watcher: Arc<crate::file_watcher::ProjectFileWatcher>,
) {
    // Provider::Claude → long-lived stream-json process driven from
    // run_deadloop_session_streaming. Other providers fall through to the
    // legacy per-iteration --print spawn below.
    if matches!(provider, Provider::Claude) {
        // Claude streaming derives try_resume_first from on-disk
        // session_jsonl existence; ignore the flag here.
        let _ = initial_try_resume;
        return run_deadloop_session_streaming(
            binary_path,
            working_dir,
            worktree_path,
            system_prompt,
            plan_review_mode_arc,
            pending_plan_reviews,
            session_id,
            claude_session_id,
            pane_id,
            prompt,
            model,
            effort,
            min_iteration_interval_minutes,
            provider,
            output_tx,
            server_tx,
            shutdown,
            pause,
            stop_requested,
            child_process,
            event_tx,
            interrupt_tx_slot,
            control_response_tx_slot,
            pending_questions,
            effort_arc,
            input_channels,
            file_watcher,
        );
    }
    let effective_dir: String = worktree_path.as_deref().unwrap_or(working_dir).to_string();
    let _ = system_prompt; // codex/cursor/etc. ignored for now — see role.rs note.
    let _ = plan_review_mode_arc; // non-claude legacy path doesn't gate
    let _ = pending_plan_reviews;
    let _ = input_channels; // non-streaming provider paths do not register
    let _ = interrupt_tx_slot; // unused for legacy path
    let _ = control_response_tx_slot; // unused for legacy path
    let _ = pending_questions; // unused for legacy path
    let _ = effort_arc; // unused for legacy path

    // For Codex, we need to capture the real thread_id from the first invocation
    // and use it for subsequent `codex exec resume` calls.
    let mut claude_session_id = claude_session_id;

    let _ = output_tx.send(PaneOutput {
        text: format!(
            "[Deadloop session: {}]",
            &claude_session_id.to_string()[..8]
        ),
        pane_id,
    });

    let mut iteration = 0;
    let mut first_message = true;
    let mut try_resume_first = initial_try_resume;
    let mut was_paused = false;
    // Set by the stderr reader thread when codex reports it can't
    // resume the session id apas handed it ("no rollout found for
    // thread id …"). The main loop checks this after the iteration
    // and, if set, mints a fresh thread id, saves it to .apas, and
    // retries without --resume — the recovery the existing
    // `first_message && using_resume` path only triggers on the very
    // first iteration, so a server-side rollout expiry mid-run was
    // wedging the deadloop until manual .apas surgery.
    let stale_session_detected = Arc::new(AtomicBool::new(false));
    let min_iteration_interval =
        Duration::from_secs(min_iteration_interval_minutes.saturating_mul(60));
    let mut last_iteration_started_at: Option<Instant> = None;
    // Suppress duplicate env-config errors so a misconfigured backend doesn't
    // spam the pane every iteration; cleared once env build succeeds.
    let mut last_env_err: Option<String> = None;

    while !shutdown.load(Ordering::SeqCst) {
        if stop_requested.load(Ordering::SeqCst) {
            // Graceful stop: current iteration finished, finalize the mode switch
            let _ = event_tx.send(TuiEvent::FinalizeStopBot {
                pane_id,
                stop_flag: stop_requested.clone(),
            });
            return;
        }

        if pause.load(Ordering::SeqCst) {
            if !was_paused {
                was_paused = true;
                let _ = output_tx.send(PaneOutput {
                    text: "[Deadloop paused - waiting for resume...]".to_string(),
                    pane_id,
                });
            }
            thread::sleep(Duration::from_millis(500));
            continue;
        } else if was_paused {
            was_paused = false;
            let _ = output_tx.send(PaneOutput {
                text: "[Deadloop resumed]".to_string(),
                pane_id,
            });
        }

        if let Some(last_started_at) = last_iteration_started_at {
            // Event-driven wait: block until any watched project file
            // changes after the wait-entry cursor OR the min-interval
            // timer fires — whichever first. Replaces the pure-sleep
            // loop that paid tokens on every cycle even when nothing
            // had moved.
            if let Some(wait_plan) =
                deadloop_wait_plan(last_started_at, min_iteration_interval, Instant::now())
            {
                let _ = output_tx.send(PaneOutput {
                    text: format!(
                        "[Waiting for file change or {}s timeout (min interval: {}m)]",
                        wait_plan.remaining.as_secs(),
                        min_iteration_interval_minutes
                    ),
                    pane_id,
                });
                // Cursor must be NOW (entry to the wait), not
                // last_started_at — otherwise the agent's own writes
                // during the iteration just past
                // (team-todo.md / .apas-team.jsonl mutations are this
                // pane's job) count as "change after cursor" and wake
                // the loop the instant it goes to sleep. That bug
                // collapses the 15-minute min interval to ~0s and was
                // the symptom of "the Tech Lead loops constantly".
                let reason = file_watcher.wait_until(
                    Some(wait_plan.cursor),
                    wait_plan.remaining,
                    &shutdown,
                    &pause,
                    &stop_requested,
                );
                match reason {
                    crate::file_watcher::WakeReason::FileChanged { path, .. } => {
                        let label = path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .and_then(|n| n.to_str())
                            .unwrap_or("(file)")
                            .to_string();
                        let _ = output_tx.send(PaneOutput {
                            text: format!("[Wake: {} changed]", label),
                            pane_id,
                        });
                    }
                    crate::file_watcher::WakeReason::Shutdown => return,
                    crate::file_watcher::WakeReason::Timeout => {
                        // Either real timeout (proceed) or pause/stop
                        // flipped (the loop top will handle it).
                    }
                }
                if shutdown.load(Ordering::SeqCst) {
                    return;
                }
                if stop_requested.load(Ordering::SeqCst) {
                    let _ = event_tx.send(TuiEvent::FinalizeStopBot {
                        pane_id,
                        stop_flag: stop_requested.clone(),
                    });
                    return;
                }
                if pause.load(Ordering::SeqCst) {
                    continue;
                }
            }
        }

        last_iteration_started_at = Some(Instant::now());
        iteration += 1;
        let _ = output_tx.send(PaneOutput {
            text: format!("=== Iteration {} ===", iteration),
            pane_id,
        });

        // Always use the original user-defined prompt so the agent retains
        // the full task context across resume iterations.
        let iteration_prompt = prompt;

        let _ = server_tx.try_send(CliToServer::UserInput {
            session_id,
            text: format!("[Iteration {}]\n{}", iteration, iteration_prompt),
            pane_type: Some(PaneType::Deadloop),
            pane_id: Some(pane_id),
        });

        let _ = server_tx.try_send(CliToServer::PaneStatus {
            session_id,
            pane_type: PaneType::Deadloop,
            pane_id: Some(pane_id),
            status: Some("Thinking...".to_string()),
        });

        let (args, using_resume) = build_deadloop_agent_args(
            provider,
            &claude_session_id,
            iteration_prompt,
            model.as_deref(),
            effort.as_deref(),
            first_message,
            try_resume_first,
        );
        if first_message && !try_resume_first {
            first_message = false;
        }

        // Reap whatever child this pane spawned on the prior iteration
        // BEFORE starting a new one. Codex `exec resume` does not exit on
        // its own when apas decides the turn is done (apas reads the
        // result marker on stdout, but codex may still be running a slow
        // tool call). Without this reap the previous codex keeps running
        // in the background, billing tokens against the user's quota,
        // while we start another one on top — runaway codex usage was
        // exactly this leak. SIGKILL the entire process group (deadloop
        // spawns set pgid via pre_exec, so -pgid catches the agent's
        // own children too), then wait() to fully release the zombie.
        if let Ok(mut guard) = child_process.lock() {
            if let Some(mut prior) = guard.take() {
                let prior_pid = prior.id();
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(prior_pid as i32), libc::SIGKILL);
                }
                #[cfg(not(unix))]
                {
                    let _ = prior.kill();
                }
                let _ = prior.wait();
                tracing::info!(
                    pane_id,
                    prior_pid,
                    "Reaped prior deadloop agent child before starting new iteration",
                );
            }
        }
        // Defensive sweep for processes that escaped reap entirely
        // (e.g., prior apas crash that lost its child_process handle).
        // Matches by session id in argv; only catches current-session
        // ids — codex's rotating thread_id can hide older orphans.
        kill_processes_using_session(&claude_session_id.to_string());

        let pane_env = match build_pane_env_overrides(provider, model.as_deref()) {
            Ok(env) => {
                last_env_err = None;
                env
            }
            Err(err) => {
                if last_env_err.as_deref() != Some(err.as_str()) {
                    last_env_err = Some(err.clone());
                    let err_msg = format!("[{}]", err);
                    let _ = output_tx.send(PaneOutput {
                        text: err_msg.clone(),
                        pane_id,
                    });
                    let _ = server_tx.try_send(CliToServer::StreamMessage {
                        session_id,
                        message: shared::ClaudeStreamMessage::Result {
                            subtype: "text".to_string(),
                            result: err_msg,
                            is_error: true,
                            total_cost_usd: 0.0,
                            duration_ms: 0,
                            session_id: session_id.to_string(),
                            extra: serde_json::Value::Null,
                        },
                        pane_type: Some(PaneType::Deadloop),
                        pane_id: Some(pane_id),
                    });
                }
                thread::sleep(Duration::from_secs(2));
                continue;
            }
        };

        let mut command = Command::new(binary_path);
        command
            .args(&args)
            .current_dir(&effective_dir)
            // Clear CLAUDECODE so Claude CLI doesn't refuse to start (nesting detection)
            .env_remove("CLAUDECODE")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &pane_env {
            command.env(key, value);
        }
        spawn_in_own_process_group(&mut command);

        match command.spawn() {
            Ok(mut child) => {
                let child_pid = child.id();
                let stdout = match child.stdout.take() {
                    Some(s) => s,
                    None => {
                        let _ = output_tx.send(PaneOutput {
                            text: "[Error: Failed to capture stdout]".to_string(),
                            pane_id,
                        });
                        thread::sleep(Duration::from_secs(5));
                        continue;
                    }
                };
                let stderr = child.stderr.take();

                if let Ok(mut guard) = child_process.lock() {
                    *guard = Some(child);
                }

                let (stdout_tx, stdout_rx) = mpsc::channel::<Option<String>>();
                let stdout_thread = thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines() {
                        match line {
                            Ok(l) => {
                                if stdout_tx.send(Some(l)).is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    let _ = stdout_tx.send(None);
                });

                let output_tx_stderr = output_tx.clone();
                let server_tx_stderr = server_tx.clone();
                let stale_flag_for_stderr = stale_session_detected.clone();
                let stderr_thread = stderr.map(|stderr| {
                    thread::spawn(move || {
                        let reader = BufReader::new(stderr);
                        for line in reader.lines() {
                            if let Ok(line) = line {
                                if !line.trim().is_empty() {
                                    // Detect codex's "the session id you
                                    // gave me doesn't exist" signal so the
                                    // main loop can mint a fresh id and
                                    // retry instead of crashing forever.
                                    // Covers both the older "no rollout
                                    // found" phrasing and the JSON-RPC
                                    // "thread/resume failed" prefix.
                                    if is_codex_stale_session_error(&line) {
                                        stale_flag_for_stderr.store(true, Ordering::SeqCst);
                                    }
                                    let _ = output_tx_stderr.send(PaneOutput {
                                        text: format!("[stderr] {}", line),
                                        pane_id,
                                    });
                                    let _ = server_tx_stderr.try_send(CliToServer::Output {
                                        session_id,
                                        data: format!("[stderr] {}", line),
                                        output_type: shared::OutputType::Error,
                                        pane_type: Some(PaneType::Deadloop),
                                        pane_id: Some(pane_id),
                                    });
                                }
                            }
                        }
                    })
                });

                let mut had_error = false;
                let mut process_exited = false;
                let mut exit_was_error = false;
                let mut timeouts_after_exit = 0;
                const MAX_TIMEOUTS_AFTER_EXIT: u32 = 10;
                let check_interval = Duration::from_millis(500);

                loop {
                    if shutdown.load(Ordering::SeqCst) {
                        break;
                    }

                    // Stop/pause requested mid-turn: kill this iteration's
                    // agent now so a long-running (non-Claude) turn halts
                    // immediately instead of running to completion. The loop
                    // top re-checks both flags — pause blocks, stop finalizes.
                    // Without this, "Stop team" / pause only took effect
                    // between iterations, so an in-flight Codex turn kept
                    // running and the worker appeared not to stop.
                    if pause.load(Ordering::SeqCst) || stop_requested.load(Ordering::SeqCst) {
                        let _ = output_tx.send(PaneOutput {
                            text: "[Stop/pause requested - ending current turn]".to_string(),
                            pane_id,
                        });
                        kill_process_group(child_pid);
                        break;
                    }

                    if !process_exited {
                        if let Ok(mut guard) = child_process.try_lock() {
                            if let Some(ref mut child) = *guard {
                                match child.try_wait() {
                                    Ok(Some(status)) => {
                                        process_exited = true;
                                        if !status.success() {
                                            let _ = output_tx.send(PaneOutput {
                                                text: format!(
                                                    "[Agent process exited with {}]",
                                                    status
                                                ),
                                                pane_id,
                                            });
                                            exit_was_error = true;
                                            had_error = true;
                                        } else {
                                            let _ = output_tx.send(PaneOutput {
                                                text: "[Agent process exited normally]".to_string(),
                                                pane_id,
                                            });
                                        }
                                    }
                                    Ok(None) => {}
                                    Err(e) => {
                                        let _ = output_tx.send(PaneOutput {
                                            text: format!("[Error checking process status: {}]", e),
                                            pane_id,
                                        });
                                    }
                                }
                            }
                        }
                    }

                    let session_id_str = claude_session_id.to_string();
                    match stdout_rx.recv_timeout(check_interval) {
                        Ok(Some(line)) => {
                            timeouts_after_exit = 0;
                            if line.trim().is_empty() {
                                continue;
                            }

                            // Capture Codex thread_id for session resume
                            if *provider == Provider::Codex {
                                if let Ok(codex_msg) =
                                    serde_json::from_str::<CodexStreamMessage>(&line)
                                {
                                    if let CodexStreamMessage::ThreadStarted { thread_id } =
                                        codex_msg
                                    {
                                        if let Ok(tid) = Uuid::parse_str(&thread_id) {
                                            claude_session_id = tid;
                                        }
                                        continue;
                                    }
                                }
                            }

                            match parse_agent_output(provider, &line, &session_id_str) {
                                Some(message) => {
                                    let is_result =
                                        matches!(message, ClaudeStreamMessage::Result { .. });
                                    if let ClaudeStreamMessage::Result { is_error, .. } = &message {
                                        if *is_error {
                                            had_error = true;
                                        }
                                    }
                                    let display_text = format_stream_message(&message);
                                    let _ = output_tx.send(PaneOutput {
                                        text: display_text,
                                        pane_id,
                                    });
                                    let _ = server_tx.try_send(CliToServer::StreamMessage {
                                        session_id,
                                        message,
                                        pane_type: Some(PaneType::Deadloop),
                                        pane_id: Some(pane_id),
                                    });
                                    if is_result {
                                        // Clear "Thinking..." as soon as the agent signals turn
                                        // completion.
                                        let _ = server_tx.try_send(CliToServer::PaneStatus {
                                            session_id,
                                            pane_type: PaneType::Deadloop,
                                            pane_id: Some(pane_id),
                                            status: None,
                                        });
                                        // Deadloop must iterate. Reap the agent + any
                                        // background children it left running so the next
                                        // iteration can start. (Interactive panes do NOT
                                        // do this — see run_pane_session.)
                                        kill_process_group(child_pid);
                                    }
                                }
                                None => {
                                    let _ = output_tx.send(PaneOutput {
                                        text: line.clone(),
                                        pane_id,
                                    });
                                    let _ = server_tx.try_send(CliToServer::Output {
                                        session_id,
                                        data: line,
                                        output_type: shared::OutputType::Text,
                                        pane_type: Some(PaneType::Deadloop),
                                        pane_id: Some(pane_id),
                                    });
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if process_exited {
                                timeouts_after_exit += 1;
                                if timeouts_after_exit >= MAX_TIMEOUTS_AFTER_EXIT {
                                    let _ = output_tx.send(PaneOutput {
                                        text: if exit_was_error {
                                            "[Process exited with error, restarting...]".to_string()
                                        } else {
                                            "[Process exited, restarting...]".to_string()
                                        },
                                        pane_id,
                                    });
                                    break;
                                }
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }

                let _ = stdout_thread.join();
                if let Some(handle) = stderr_thread {
                    let stderr_timeout = thread::spawn(move || {
                        let _ = handle.join();
                    });
                    thread::sleep(Duration::from_millis(500));
                    drop(stderr_timeout);
                }

                if let Ok(mut guard) = child_process.lock() {
                    if let Some(mut child) = guard.take() {
                        match child.try_wait() {
                            Ok(Some(_)) => {}
                            Ok(None) => {
                                let _ = output_tx.send(PaneOutput {
                                    text: format!("[Killing stuck process {}]", child_pid),
                                    pane_id,
                                });
                                let _ = child.kill();
                                let _ = child.wait();
                            }
                            Err(_) => {
                                let _ = child.kill();
                                let _ = child.wait();
                            }
                        }
                    }
                }

                let _ = server_tx.try_send(CliToServer::PaneStatus {
                    session_id,
                    pane_type: PaneType::Deadloop,
                    pane_id: Some(pane_id),
                    status: None,
                });

                if had_error || exit_was_error {
                    // Stale-session recovery applies on ANY iteration —
                    // codex's server can drop a thread mid-run (rollout
                    // expiry, server-side wipe), not just at startup.
                    // The stderr reader sets stale_session_detected on
                    // "no rollout found" / "thread/resume failed".
                    let stale = should_recover_deadloop_stale_session(
                        stale_session_detected.swap(false, Ordering::SeqCst),
                        first_message,
                        using_resume,
                        exit_was_error,
                        had_error,
                    );
                    if stale {
                        // Reset first_message so the next iteration takes
                        // the "exec (no resume)" branch in build_agent_args.
                        // Without this, the next turn can fall into the
                        // "subsequent -> always resume" branch and try
                        // exec resume on the just-minted id, which Codex
                        // also does not know about.
                        let old = reset_deadloop_codex_stale_session(
                            &mut first_message,
                            &mut try_resume_first,
                            &mut claude_session_id,
                            Uuid::new_v4(),
                        );
                        // Persist the fresh id straight into .apas so a
                        // CLI reboot doesn't fall back to the dead one.
                        // We don't have the full pane_metas / pane_pauses
                        // Arcs in this scope, so go through the project
                        // module directly: read .apas, mutate the matching
                        // pane's session_id, write back.
                        let project_dir = std::path::Path::new(working_dir);
                        if let Ok(mut metadata) = crate::project::get_or_create_project(project_dir)
                        {
                            if let Some(pane) = metadata.get_pane_mut(pane_id) {
                                pane.session_id = claude_session_id;
                                let _ = crate::project::save_project(project_dir, &metadata);
                            }
                        }
                        let _ = output_tx.send(PaneOutput {
                            text: format!(
                                "[Codex session {} is dead on the server; minted fresh id {} and will create a new thread next iteration]",
                                &old.to_string()[..8],
                                &claude_session_id.to_string()[..8],
                            ),
                            pane_id,
                        });
                        thread::sleep(Duration::from_secs(1));
                    } else {
                        thread::sleep(Duration::from_secs(2));
                    }
                } else {
                    first_message = false;
                    thread::sleep(Duration::from_secs(2));
                }
            }
            Err(e) => {
                let err_msg = format_spawn_error(provider, model.as_deref(), binary_path, &e);
                let _ = output_tx.send(PaneOutput {
                    text: err_msg.clone(),
                    pane_id,
                });
                let _ = server_tx.try_send(CliToServer::PaneStatus {
                    session_id,
                    pane_type: PaneType::Deadloop,
                    pane_id: Some(pane_id),
                    status: None,
                });
                // Also notify web clients about the error
                let _ = server_tx.try_send(CliToServer::StreamMessage {
                    session_id,
                    message: shared::ClaudeStreamMessage::Result {
                        subtype: "text".to_string(),
                        result: err_msg,
                        is_error: true,
                        total_cost_usd: 0.0,
                        duration_ms: 0,
                        session_id: session_id.to_string(),
                        extra: serde_json::Value::Null,
                    },
                    pane_type: Some(PaneType::Deadloop),
                    pane_id: Some(pane_id),
                });
                for _ in 0..5 {
                    if shutdown.load(Ordering::SeqCst) || stop_requested.load(Ordering::SeqCst) {
                        break;
                    }
                    thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }
}

/// Check if claude code's on-disk session log exists for the given session
/// id and working dir. Used by the streaming worker to decide whether the
/// first spawn should `--resume` an existing session or `--session-id` a new
/// one. Layout matches claude code's: `~/.claude/projects/<encoded-cwd>/<session_id>.jsonl`,
/// where the cwd is encoded by replacing `/` with `-`.
fn session_jsonl_path(working_dir: &str, session_id: &Uuid) -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let encoded: String = working_dir
        .chars()
        .map(|c| if c == '/' { '-' } else { c })
        .collect();
    Some(
        std::path::Path::new(&home)
            .join(".claude")
            .join("projects")
            .join(encoded)
            .join(format!("{}.jsonl", session_id)),
    )
}

fn session_jsonl_exists(working_dir: &str, session_id: &Uuid) -> bool {
    session_jsonl_path(working_dir, session_id)
        .map(|p| p.exists())
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
struct DeadloopWatchdogState {
    last_mtime: Option<std::time::SystemTime>,
    last_activity: Instant,
    last_nudge: Option<Instant>,
}

impl DeadloopWatchdogState {
    fn new(now: Instant) -> Self {
        Self {
            last_mtime: None,
            last_activity: now,
            last_nudge: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeadloopWatchdogDecision {
    Noop,
    Nudge { idle_minutes: u64 },
}

fn evaluate_deadloop_watchdog(
    state: &mut DeadloopWatchdogState,
    now: Instant,
    idle_threshold: Duration,
    session_mtime: Option<std::time::SystemTime>,
    session_jsonl_path_known: bool,
    supervisor_active: bool,
) -> DeadloopWatchdogDecision {
    if !supervisor_active || !session_jsonl_path_known {
        return DeadloopWatchdogDecision::Noop;
    }

    if let Some(mtime) = session_mtime {
        if state.last_mtime != Some(mtime) {
            state.last_mtime = Some(mtime);
            state.last_activity = now;
            return DeadloopWatchdogDecision::Noop;
        }
    }

    let idle = now.saturating_duration_since(state.last_activity);
    let nudge_cooldown_over = state
        .last_nudge
        .map(|last_nudge| now.saturating_duration_since(last_nudge) >= idle_threshold)
        .unwrap_or(true);
    if idle >= idle_threshold && nudge_cooldown_over {
        state.last_nudge = Some(now);
        DeadloopWatchdogDecision::Nudge {
            idle_minutes: idle.as_secs() / 60,
        }
    } else {
        DeadloopWatchdogDecision::Noop
    }
}

/// Per-pane background-task watcher for the streaming worker.
///
/// Claude code stores Bash background-task output at
/// `/tmp/claude-<uid>/<encoded-cwd>/<session_id>/tasks/<task-id>.output`,
/// growing in real time as the task writes. By default claude only re-reads
/// these files when the user asks (via `TaskOutput`), so a Monitor watcher or
/// `tail -F` that fires after the agent's last turn sits unread until the
/// user types something. With the streaming worker in place we own claude's
/// stdin, so we can poll these files and synthesize a wake-up prompt the
/// instant a task produces new output that has settled.
///
/// Algorithm:
///   * Every 5 s, scan the tasks dir for `*.output` files.
///   * Track per-file high-water mark in memory.
///   * If a file has grown since last poll, mark it "growing" and remember
///     the timestamp.
///   * If a file has NOT grown since the previous poll, has previously
///     grown (i.e. there's pending unread output), and we haven't already
///     fired for this growth episode, send a synthesized prompt to the
///     streaming worker via `wake_tx` and mark "fired".
///
/// `wake_tx` is consumed by the inner loop, which writes the prompt onto
/// claude's stdin as a normal user turn (the same mechanism we use for
/// human prompts).
fn poll_background_tasks(
    session_id: Uuid,
    working_dir: &str,
    pane_id: u32,
    wake_tx: std::sync::mpsc::Sender<String>,
    watched_tasks: Arc<Mutex<HashSet<String>>>,
    shutdown: Arc<AtomicBool>,
) {
    let uid = unsafe { libc::getuid() };
    let encoded_cwd: String = working_dir
        .chars()
        .map(|c| if c == '/' { '-' } else { c })
        .collect();
    let tasks_dir = std::path::PathBuf::from(format!(
        "/tmp/claude-{}/{}/{}/tasks",
        uid, encoded_cwd, session_id
    ));
    // Marker file written by `apas-stop-hook.sh` (configured in claude code's
    // settings.json hooks.Stop). Its mtime is the authoritative "claude went
    // idle at" timestamp. We only fire auto-wake for task output that grew
    // AFTER this mtime — output produced during a turn is part of the turn
    // and claude already saw it, so it shouldn't trigger a new prompt.
    let stop_marker_path = std::path::PathBuf::from(format!("/tmp/apas-stop-marks/{}", session_id));

    // task_id -> (last_seen_size, last_seen_mtime, fired_for_this_episode)
    let mut state: HashMap<String, (u64, std::time::SystemTime, bool)> = HashMap::new();
    // Snapshot of task ids that already existed on the first poll. These are
    // pre-restart leftovers: their writer processes (if still alive) are
    // orphans the new streaming claude has no record of, so firing wake on
    // them produces "No task found with ID …" errors. We permanently ignore
    // them for this watcher's lifetime; only ids that *appear* in the dir
    // after the first poll are eligible.
    let mut pre_existing: HashSet<String> = HashSet::new();
    let mut snapshot_done = false;
    const POLL_INTERVAL: Duration = Duration::from_secs(5);
    // Cap how much of a task's `.output` we inline into the wake prompt.
    // Big enough to convey context (a Monitor watcher's last update or a
    // bash script's tail), small enough that a chatty task can't bloat a
    // single wake into a multi-MB user turn. Claude can always Read the
    // file path in the prompt for the full content.
    const MAX_INLINE_BYTES: u64 = 4096;

    while !shutdown.load(Ordering::SeqCst) {
        let entries = match std::fs::read_dir(&tasks_dir) {
            Ok(it) => it,
            Err(_) => {
                thread::sleep(POLL_INTERVAL);
                continue;
            }
        };

        let stop_mtime = std::fs::metadata(&stop_marker_path)
            .and_then(|m| m.modified())
            .ok();

        let is_initial_snapshot = !snapshot_done;
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("output") {
                continue;
            }
            let Some(task_id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            // Two distinct .output file types live in this dir: `b*` = Bash
            // background tasks (readable by claude via the TaskOutput tool)
            // and `a*` = Agent/Task subagent IDs (NOT readable by TaskOutput;
            // their content is delivered to the parent via the Task tool's
            // result). Auto-waking on `a*` ids produces "No task with ID ..."
            // errors. We only ever want to surface `b*` ids.
            if !task_id.starts_with('b') {
                continue;
            }
            // Pre-existing snapshot: any id observed in the first poll is a
            // leftover from before this watcher started. Their writer
            // processes (if still alive) are orphans the resumed claude
            // doesn't track, so TaskOutput would fail with "No task found
            // with ID …" — exactly the symptom we keep hitting.
            if is_initial_snapshot {
                pre_existing.insert(task_id.to_string());
                continue;
            }
            if pre_existing.contains(task_id) {
                continue;
            }
            let task_id = task_id.to_string();
            let meta = match std::fs::metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let size = meta.len();
            let file_mtime = match meta.modified() {
                Ok(t) => t,
                Err(_) => continue,
            };

            let entry_state = state
                .entry(task_id.clone())
                .or_insert_with(|| (size, file_mtime, true));
            // Brand-new files we observe with their full current size are NOT
            // a wake event — they were already there when we started polling.
            // Setting fired=true suppresses the first-sighting fire.

            if size > entry_state.0 {
                // Grew since last poll → not yet settled.
                entry_state.0 = size;
                entry_state.1 = file_mtime;
                entry_state.2 = false;
                continue;
            }
            // Settled. Decide whether to fire.
            //   1. Must not have already fired for this growth episode.
            //   2. Stop hook must have observed at least one idle event
            //      (otherwise we don't know whether the file's growth is
            //      part of an in-progress turn or genuinely post-idle).
            //   3. The file's growth must have happened AFTER claude went
            //      idle (file_mtime > stop_mtime). Output produced during
            //      a turn was already consumed by claude as a tool result.
            if entry_state.2 {
                continue;
            }
            let Some(stop_mtime) = stop_mtime else {
                continue;
            };
            if entry_state.1 <= stop_mtime {
                continue;
            }
            // Watched-set gate: only fire wake for tasks the agent has
            // expressed interest in by calling Monitor or BashOutput on
            // them. Foreground bash that the agent merely ran and forgot
            // (e.g. `Bash("ls")`) writes to the same .output dir but is
            // already part of the turn's tool_result — waking on its
            // post-stop tail growth is noise. Long-running services the
            // agent fired-and-forgot (e.g. an HTTP server) are also
            // excluded by this filter.
            {
                let watched = watched_tasks.lock().unwrap();
                if !watched.contains(&task_id) {
                    continue;
                }
            }
            // Inline the tail of the task's .output so claude has the actual
            // content even if the task has since been reaped from its
            // registry (TaskOutput would say "No task found") or the file
            // has been cleaned up by the time claude processes the wake.
            let mut snippet = String::new();
            let mut snippet_truncated = false;
            if let Ok(mut f) = std::fs::File::open(&path) {
                use std::io::{Read, Seek, SeekFrom};
                let read_from = if size > MAX_INLINE_BYTES {
                    snippet_truncated = true;
                    size - MAX_INLINE_BYTES
                } else {
                    0
                };
                if f.seek(SeekFrom::Start(read_from)).is_ok() {
                    let mut buf = vec![0u8; MAX_INLINE_BYTES as usize];
                    if let Ok(n) = f.read(&mut buf) {
                        snippet = String::from_utf8_lossy(&buf[..n]).to_string();
                    }
                }
            }
            let path_display = path.display();
            let prompt = if snippet.is_empty() {
                format!(
                    "[apas auto-wake] Background task {} produced new output but the .output file is no longer readable. Path was: {}",
                    task_id, path_display
                )
            } else if snippet_truncated {
                format!(
                    "[apas auto-wake] Background task {} produced new output (showing last {} of {} bytes; Read {} for full):\n```\n{}\n```",
                    task_id, MAX_INLINE_BYTES, size, path_display, snippet
                )
            } else {
                format!(
                    "[apas auto-wake] Background task {} produced new output ({} bytes from {}):\n```\n{}\n```",
                    task_id, size, path_display, snippet
                )
            };
            if wake_tx.send(prompt).is_err() {
                return; // inner loop gone
            }
            tracing::info!(
                pane_id,
                task_id = %task_id,
                size,
                snippet_bytes = snippet.len(),
                "auto-wake fired (post-stop, settled)",
            );
            entry_state.2 = true;
        }

        if is_initial_snapshot {
            snapshot_done = true;
            tracing::info!(
                pane_id,
                pre_existing_count = pre_existing.len(),
                "auto-wake: initial task snapshot complete (pre-existing ids will be ignored)",
            );
        }

        thread::sleep(POLL_INTERVAL);
    }
}

/// Extract the `uuid` field from a raw JSON line (claude's stream-json /
/// session jsonl entries always carry one). Used by the dedup machinery
/// shared between the stdout reader and the session-jsonl tailer.
fn extract_message_uuid(line: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|v| {
            v.get("uuid")
                .and_then(|u| u.as_str())
                .map(|s| s.to_string())
        })
}

/// Tail claude code's on-disk session jsonl for messages that don't appear on
/// `claude --print`'s stdout — chiefly Task/subagent intermediate work.
///
/// When a streaming claude spawns a Task subagent, the subagent's tool calls,
/// reasoning, and tool results all land in the *parent's* session jsonl at
/// `~/.claude/projects/<encoded-cwd>/<session_id>.jsonl`, but on the parent's
/// stdout we only see `tool_use(Task)` followed by the final `tool_result`
/// containing the subagent's last message. The intermediate work is invisible
/// to apas (and therefore the web UI) without tailing the file.
///
/// Algorithm: poll the file every 1 s; on size growth, read new bytes since
/// last position, split on `\n`, parse each line, look up its `uuid` in the
/// shared `seen_uuids` set, and forward as `CliToServer::StreamMessage` if
/// not already forwarded. The stdout reader inserts uuids it forwards into
/// the same set, so whichever side observes a message first wins and the
/// other side de-duplicates.
fn tail_session_jsonl(
    session_id: Uuid,
    working_dir: &str,
    pane_id: u32,
    apas_session_id: Uuid,
    server_tx: tokio_mpsc::Sender<CliToServer>,
    seen_uuids: Arc<Mutex<HashSet<String>>>,
    shutdown: Arc<AtomicBool>,
    pane_type: PaneType,
) {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let encoded: String = working_dir
        .chars()
        .map(|c| if c == '/' { '-' } else { c })
        .collect();
    let path = std::path::Path::new(&home)
        .join(".claude")
        .join("projects")
        .join(encoded)
        .join(format!("{}.jsonl", session_id));

    let mut position: u64 = 0;
    let mut buf = String::new();
    const POLL_INTERVAL: Duration = Duration::from_secs(1);
    let session_id_str = session_id.to_string();
    let mut forwarded_count: u64 = 0;
    let mut initialized_position = false;

    while !shutdown.load(Ordering::SeqCst) {
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => {
                // File doesn't exist yet — claude hasn't written its first
                // turn for this session. Wait for it.
                thread::sleep(POLL_INTERVAL);
                continue;
            }
        };
        let len = meta.len();

        // First-time initialization: jump to current EOF so we don't replay
        // the entire prior conversation. The stdout reader has been forwarding
        // live messages since pane spawn; everything before that is already
        // in the persisted session and the web UI loads it on attach.
        if !initialized_position {
            position = len;
            initialized_position = true;
            tracing::info!(
                pane_id,
                path = %path.display(),
                start_position = position,
                "session jsonl tail initialized",
            );
            thread::sleep(POLL_INTERVAL);
            continue;
        }

        if len < position {
            // File was truncated (claude /clear, fork, etc.) — restart from
            // the new EOF.
            tracing::info!(
                pane_id,
                old_position = position,
                new_len = len,
                "session jsonl shrank; resetting position",
            );
            position = len;
            buf.clear();
            thread::sleep(POLL_INTERVAL);
            continue;
        }
        if len == position {
            thread::sleep(POLL_INTERVAL);
            continue;
        }

        let mut file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => {
                thread::sleep(POLL_INTERVAL);
                continue;
            }
        };
        use std::io::{Read, Seek, SeekFrom};
        if file.seek(SeekFrom::Start(position)).is_err() {
            thread::sleep(POLL_INTERVAL);
            continue;
        }
        let mut chunk = Vec::with_capacity((len - position) as usize);
        if file.read_to_end(&mut chunk).is_err() {
            thread::sleep(POLL_INTERVAL);
            continue;
        }
        position = len;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        // Split on newline; the trailing partial line stays in `buf` until
        // the next poll completes it.
        let mut last_newline = 0usize;
        for (i, c) in buf.char_indices() {
            if c == '\n' {
                let line = &buf[last_newline..i];
                last_newline = i + 1;
                if line.trim().is_empty() {
                    continue;
                }
                let Some(uuid) = extract_message_uuid(line) else {
                    continue;
                };
                {
                    let mut set = seen_uuids.lock().unwrap();
                    if set.contains(&uuid) {
                        continue;
                    }
                    set.insert(uuid.clone());
                }
                let Some(message) = parse_agent_output(&Provider::Claude, line, &session_id_str)
                else {
                    continue;
                };
                let _ = server_tx.blocking_send(CliToServer::StreamMessage {
                    session_id: apas_session_id,
                    message,
                    pane_type: Some(pane_type),
                    pane_id: Some(pane_id),
                });
                forwarded_count += 1;
                if forwarded_count <= 5 || forwarded_count % 50 == 0 {
                    tracing::info!(
                        pane_id,
                        uuid,
                        forwarded_count,
                        "session jsonl tail forwarded supplemental message",
                    );
                }
            }
        }
        if last_newline > 0 {
            buf.drain(..last_newline);
        }
    }
}

/// Compose the streaming pane's status string. The pane is considered "busy"
/// (still on the same turn from the user's POV) whenever the parent claude
/// is mid-inference (`thinking`) OR has any in-flight subagents. We treat
/// subagent activity as a turn extension: even after the parent emits its
/// `result`, if subagents are still running they're doing the real work,
/// so we keep the `Thinking...` indicator up.
fn compose_streaming_status(thinking: bool, n_subagents: usize) -> Option<String> {
    let busy = thinking || n_subagents > 0;
    if !busy {
        return None;
    }
    let pluralize = |n: usize| if n == 1 { "subagent" } else { "subagents" };
    if n_subagents == 0 {
        Some("Thinking...".to_string())
    } else {
        Some(format!(
            "Thinking... ({} {})",
            n_subagents,
            pluralize(n_subagents)
        ))
    }
}

/// Streaming variant for `Provider::Claude` interactive panes: keeps a single
/// long-lived `claude --print --input-format stream-json --output-format
/// stream-json --resume <id>` process alive across many turns. User prompts
/// are pushed onto its stdin as `{"type":"user","message":{"role":"user",
/// "content":"..."}}` JSON lines (the wire format the
/// `@anthropic-ai/claude-code` SDK uses, observed via slopus/happy-cli).
///
/// Why this exists, vs. the per-turn `--print --resume` path:
///   * Background tasks (Monitor watcher, `tail -F`, etc.) stay attached to
///     one stable parent. When they emit output, claude can react in the
///     same session — the precondition for any future "auto-wake" feature.
///   * Subagent / Task children stay alive across turns instead of being
///     orphaned at every Result.
///   * Cold-start cost (model load, MCP init) is paid once.
///
/// Restart conditions: child crashes, stdout EOFs, stdin write fails, or
/// shutdown / pane teardown is requested. On restart we re-spawn with
/// `--resume <claude_session_id>` so the conversation continues.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn run_pane_session_streaming(
    binary_path: &str,
    working_dir: &str,
    // Optional isolated git worktree path. When Some, the claude process
    // and the session-jsonl tailer use this as their cwd; claude's session
    // jsonl is keyed by encoded-cwd so they MUST agree. `.apas` and
    // project metadata stay rooted at `working_dir`. Phase 1.1b.
    worktree_path: Option<String>,
    // Optional pre-rendered system-prompt prefix built from role/goal/
    // backstory (Phase 2.1b). When Some, gets passed to claude as
    // `--append-system-prompt <prefix>` at every spawn.
    system_prompt: Option<String>,
    // Phase 3.2b2: live mirror of the pane's plan-review policy, plus
    // the parking map for held tool_uses. Reader thread consults both
    // on every can_use_tool control_request.
    plan_review_mode_arc: Arc<Mutex<shared::PlanReviewMode>>,
    pending_plan_reviews: Arc<Mutex<HashMap<String, PendingPlanReview>>>,
    session_id: Uuid,
    claude_session_id: Uuid,
    pane_id: u32,
    provider: &Provider,
    model: Option<String>,
    effort: Option<String>,
    input_rx: mpsc::Receiver<PaneInput>,
    output_tx: mpsc::Sender<PaneOutput>,
    server_tx: tokio_mpsc::Sender<CliToServer>,
    shutdown: Arc<AtomicBool>,
    child_process: Arc<Mutex<Option<std::process::Child>>>,
    interrupt_tx_slot: Arc<Mutex<Option<mpsc::Sender<()>>>>,
    // control_response_tx_slot: the AnswerQuestion handler pushes
    // pre-serialized control_response JSON into this channel; the inner
    // loop drains it and writes to claude's stdin. Same lifecycle as
    // interrupt_tx_slot — set on entry, cleared on shutdown.
    control_response_tx_slot: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    // pending_questions: shared map keyed by tool_use_id. Reader thread
    // inserts on AskUserQuestion control_request; AnswerQuestion handler
    // reads to recover claude's request_id + questions before pushing
    // the control_response.
    pending_questions: Arc<Mutex<HashMap<String, PendingAskQuestion>>>,
    // effort_arc: live mirror of the pane's effort. spawn_loop re-reads
    // this every iteration so a future respawn (process crash, /loop
    // iteration, etc.) picks up the latest effort. The primary live-
    // update path is the apply_flag_settings control_request — see the
    // UpdatePaneEffort handler.
    effort_arc: Arc<Mutex<Option<String>>>,
    // result_signal_tx: when Some, the stdout reader sends () on every
    // Result event. Used by the deadloop driver to detect iteration
    // boundary so it can throttle and re-inject the next prompt. None for
    // plain interactive panes.
    result_signal_tx: Option<mpsc::Sender<()>>,
    // pane_type: tagged on every StreamMessage / PaneStatus / Output sent
    // upstream so the server / web UI route correctly. Interactive for
    // user-driven panes, Deadloop for the streaming-deadloop driver.
    pane_type: PaneType,
) {
    use std::io::Write;

    // effective_dir is what we pass to claude as cwd and what we use to
    // locate its session jsonl / background-task tmp dir. With no worktree
    // set, this collapses to the project working_dir (legacy behaviour).
    let effective_dir: String = worktree_path.as_deref().unwrap_or(working_dir).to_string();

    let _ = output_tx.send(PaneOutput {
        text: format!(
            "[Session: {} (streaming)]",
            &claude_session_id.to_string()[..8]
        ),
        pane_id,
    });

    // Register a soft-interrupt channel with the InterruptPane handler. When
    // the user clicks Interrupt, the handler sends () on this channel and we
    // pump a control_request("interrupt") onto claude's stdin — which stops
    // the current turn but keeps the long-lived process alive (which is the
    // whole point of streaming mode).
    let (interrupt_tx, interrupt_rx) = mpsc::channel::<()>();
    if let Ok(mut slot) = interrupt_tx_slot.lock() {
        *slot = Some(interrupt_tx);
    }

    // Channel for control_response JSON lines. Producers: the reader thread
    // (auto-approving non-AskUserQuestion permission prompts that slip
    // through) and the AnswerQuestion handler in the WebSocket task (when
    // the user submits answers). The inner loop drains and writes to
    // claude's stdin, so all stdin writes stay serialized through a single
    // owner.
    let (control_response_tx, control_response_rx) = mpsc::channel::<String>();
    if let Ok(mut slot) = control_response_tx_slot.lock() {
        *slot = Some(control_response_tx.clone());
    }

    // Per-pane "watched task" set: task ids the agent has expressed
    // interest in by calling Monitor or BashOutput on them. The auto-wake
    // watcher only fires for ids in this set — so foreground bash output
    // that's already part of a tool_result (or fire-and-forget services
    // like an HTTP server the agent never re-checks) doesn't trigger wakes.
    // Populated by the stdout reader (see tool_use parser below) and shared
    // with poll_background_tasks.
    let watched_tasks: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    // Spawn the per-pane background-task watcher once. It survives across
    // claude respawns (state hashmap persists) so we don't refire wakes for
    // tasks that grew during a respawn gap.
    let (auto_wake_tx, auto_wake_rx) = mpsc::channel::<String>();
    let watcher_session = claude_session_id;
    let watcher_working_dir = effective_dir.clone();
    let watcher_pane_id = pane_id;
    let watcher_shutdown = shutdown.clone();
    let watcher_watched = watched_tasks.clone();
    thread::spawn(move || {
        poll_background_tasks(
            watcher_session,
            &watcher_working_dir,
            watcher_pane_id,
            auto_wake_tx,
            watcher_watched,
            watcher_shutdown,
        );
    });

    // Shared dedup set between the stdout reader (spawned per claude
    // process below) and the session-jsonl tailer (spawned once here).
    // Both check before forwarding; whoever sees a uuid first inserts it
    // and the other side skips. This lets the tailer surface subagent /
    // Task intermediate work that doesn't appear on the parent claude's
    // stdout, without doubling up on messages that DO appear there.
    let seen_uuids: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    // In-flight Task subagent tracking: every `assistant` message with a
    // tool_use block where name=="Task" is a new subagent; the matching
    // tool_result (id correlation in a later `user` message) marks it done.
    // The set's size is the live subagent count, surfaced in PaneStatus
    // as e.g. "Thinking... (2 subagents)".
    //
    // Note: this is intentionally NOT tracking Bash with run_in_background.
    // That tool returns its tool_result IMMEDIATELY with the task id, so
    // id-correlation says "done" before the underlying process has run.
    // Tracking those properly requires polling the .output file mtime
    // — separate followup.
    let in_flight_subagents: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    // Composite-status state. The inner loop sets thinking=true on prompt
    // write; the reader thread sets it false on Result. Either side calls
    // send_status() after mutating to recompute and publish the status.
    let thinking: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));

    let tailer_session = claude_session_id;
    let tailer_working_dir = effective_dir.clone();
    let tailer_pane_id = pane_id;
    let tailer_apas_session = session_id;
    let tailer_server_tx = server_tx.clone();
    let tailer_seen = seen_uuids.clone();
    let tailer_shutdown = shutdown.clone();
    let tailer_pane_type = pane_type;
    thread::spawn(move || {
        tail_session_jsonl(
            tailer_session,
            &tailer_working_dir,
            tailer_pane_id,
            tailer_apas_session,
            tailer_server_tx,
            tailer_seen,
            tailer_shutdown,
            tailer_pane_type,
        );
    });

    // First spawn for a brand-new session_id: --resume will fail ("No
    // conversation found"). Use --session-id <uuid> to create it. After the
    // first successful spawn (or after we observe the session file on disk),
    // flip to --resume for all subsequent restarts.
    let mut try_resume_first = session_jsonl_exists(&effective_dir, &claude_session_id);
    // Suppress duplicate env-config errors across spawn retries.
    let mut last_env_err: Option<String> = None;

    'spawn_loop: while !shutdown.load(Ordering::SeqCst) {
        let pane_env = match build_pane_env_overrides(provider, model.as_deref()) {
            Ok(env) => {
                last_env_err = None;
                env
            }
            Err(err) => {
                if last_env_err.as_deref() != Some(err.as_str()) {
                    last_env_err = Some(err.clone());
                    let _ = output_tx.send(PaneOutput {
                        text: format!("[{}]", err),
                        pane_id,
                    });
                }
                thread::sleep(Duration::from_secs(2));
                continue 'spawn_loop;
            }
        };

        let using_resume = try_resume_first;
        // We use --permission-prompt-tool stdio (not --dangerously-skip-permissions)
        // so claude routes AskUserQuestion calls through canUseTool via the
        // stdio control_request protocol. Bypass mode keeps regular tools
        // auto-approved; AskUserQuestion still surfaces a control_request
        // because claude's SDK gates it on requiresUserInteraction().
        let mut args: Vec<String> = vec![
            "--print".into(),
            "--input-format".into(),
            "stream-json".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
            "--permission-prompt-tool".into(),
            "stdio".into(),
            "--allow-dangerously-skip-permissions".into(),
            "--permission-mode".into(),
            "bypassPermissions".into(),
        ];
        if using_resume {
            args.push("--resume".into());
        } else {
            args.push("--session-id".into());
        }
        args.push(claude_session_id.to_string());
        if let Some(m) = model.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            if !shared::is_retired_model(Some(m)) && !is_deepseek_model(Some(m)) {
                args.push("--model".into());
                args.push(m.to_string());
            }
        }
        if !shared::is_retired_model(model.as_deref()) && !is_deepseek_model(model.as_deref()) {
            // Re-read effort from the shared cell at every spawn so a
            // UpdatePaneEffort that fires between spawns picks up the
            // latest value. The `effort` function param is the seed for
            // the first spawn only; afterwards the worker is bound to
            // whatever the UI last set.
            let current_effort = effort_arc
                .lock()
                .ok()
                .and_then(|g| g.clone())
                .or_else(|| effort.clone());
            if let Some(eff) = normalize_effort_level(current_effort.as_deref()) {
                let claude_flag = effort_to_claude_flag(&eff).to_string();
                tracing::info!(
                    target: "apas::effort",
                    pane_id,
                    level = %eff,
                    claude_flag = %claude_flag,
                    "Launching streaming claude with --effort",
                );
                args.push("--effort".into());
                args.push(claude_flag);
            }
        }

        // Phase 2.1b: append the role/goal/backstory prelude so claude
        // self-identifies. Skipped silently when the prelude is None.
        if let Some(sp) = system_prompt.as_deref() {
            args.push("--append-system-prompt".into());
            args.push(sp.to_string());
        }

        // Defensive: same guard as the per-turn path. Two `--resume` processes
        // on one session would interleave writes to the .jsonl.
        kill_processes_using_session(&claude_session_id.to_string());

        let mut command = Command::new(binary_path);
        command
            .args(&args)
            .current_dir(&effective_dir)
            .env_remove("CLAUDECODE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &pane_env {
            command.env(key, value);
        }
        spawn_in_own_process_group(&mut command);

        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = output_tx.send(PaneOutput {
                    text: format!("[Failed to spawn agent: {}]", e),
                    pane_id,
                });
                thread::sleep(Duration::from_secs(2));
                continue 'spawn_loop;
            }
        };

        let mut stdin = match child.stdin.take() {
            Some(s) => s,
            None => {
                let _ = output_tx.send(PaneOutput {
                    text: "[Failed to capture stdin]".to_string(),
                    pane_id,
                });
                let _ = child.kill();
                let _ = child.wait();
                thread::sleep(Duration::from_secs(2));
                continue 'spawn_loop;
            }
        };
        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take();
        let child_pid = child.id();

        // Hand the child to the shared slot so InterruptPane / shutdown
        // handlers elsewhere can find and signal it.
        if let Ok(mut guard) = child_process.lock() {
            *guard = Some(child);
        }

        // Reset per-spawn liveness state: the previous claude (if any) is
        // dead, so any tool_use ids it had outstanding are gone. Also reset
        // the Thinking flag.
        if let Ok(mut s) = in_flight_subagents.lock() {
            s.clear();
        }
        if let Ok(mut t) = thinking.lock() {
            *t = false;
        }

        // Reader thread: parse stream-json off stdout, forward to UI/server.
        // Unlike the per-turn path, we DO NOT exit on `result` — the process
        // stays alive for the next turn.
        let (reader_done_tx, reader_done_rx) = mpsc::channel::<()>();
        let output_tx_reader = output_tx.clone();
        let server_tx_reader = server_tx.clone();
        let provider_reader = *provider;
        let session_id_reader = session_id;
        let pane_id_reader = pane_id;
        let pane_type_reader = pane_type;
        let claude_session_id_str = claude_session_id.to_string();
        let reader_seen = seen_uuids.clone();
        let reader_result_signal = result_signal_tx.clone();
        let reader_in_flight = in_flight_subagents.clone();
        let reader_thinking = thinking.clone();
        let reader_watched = watched_tasks.clone();
        // Reader-thread access to the same pending-questions map and
        // control_response channel as the AnswerQuestion handler. The
        // reader records AskUserQuestion control_requests; for everything
        // else it auto-approves via the same channel the inner loop drains
        // onto claude's stdin.
        let reader_pending_questions = pending_questions.clone();
        let reader_control_response_tx = control_response_tx.clone();
        let reader_pending_plan_reviews = pending_plan_reviews.clone();
        let reader_plan_review_mode_arc = plan_review_mode_arc.clone();
        let reader_thread = thread::spawn(move || {
            let reader = BufReader::new(stdout);
            let mut line_count: u64 = 0;
            let mut had_io_error = false;
            // Set when the parent claude emits Result while subagents are
            // still in-flight. Cleared (and signal fired) the moment the
            // last subagent's tool_result drains the in-flight set. This
            // realizes the contract: "iteration done" means parent done
            // AND all subagents done, not just parent done.
            let mut result_pending: bool = false;
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(e) => {
                        tracing::warn!(
                            pane_id = pane_id_reader,
                            error = %e,
                            "streaming reader hit IO error",
                        );
                        had_io_error = true;
                        break;
                    }
                };
                if line.trim().is_empty() {
                    continue;
                }
                line_count += 1;
                // Intercept claude's control_request envelope before we hand
                // the line to parse_agent_output, which only understands
                // ClaudeStreamMessage. With `--permission-prompt-tool stdio`
                // claude routes permission/AskUserQuestion prompts through
                // this channel; we respond on stdin via control_response.
                if try_handle_control_response(
                    &line,
                    pane_id_reader,
                    session_id_reader,
                    pane_type_reader,
                    &server_tx_reader,
                ) {
                    continue;
                }
                let current_review_mode = reader_plan_review_mode_arc
                    .lock()
                    .ok()
                    .map(|g| *g)
                    .unwrap_or_default();
                if let Some(handled) = try_handle_control_request(
                    &line,
                    pane_id_reader,
                    session_id_reader,
                    pane_type_reader,
                    &reader_pending_questions,
                    &reader_control_response_tx,
                    &server_tx_reader,
                    Some(&reader_pending_plan_reviews),
                    current_review_mode,
                ) {
                    if handled {
                        continue;
                    }
                }
                // Dedup with the session-jsonl tailer. If this uuid was
                // already forwarded by the tailer (rare race), skip.
                if let Some(uuid) = extract_message_uuid(&line) {
                    let mut set = reader_seen.lock().unwrap();
                    if set.contains(&uuid) {
                        continue;
                    }
                    set.insert(uuid);
                }
                match parse_agent_output(&provider_reader, &line, &claude_session_id_str) {
                    Some(message) => {
                        let is_result = matches!(message, ClaudeStreamMessage::Result { .. });

                        // Sniff for Task subagent activity BEFORE moving
                        // `message` into the StreamMessage forward. New
                        // Task tool_use → insert; matching tool_result →
                        // remove. Status is recomputed and sent only when
                        // the in-flight set actually changed (debounces
                        // the noise of every assistant chunk).
                        let mut subagent_state_changed = false;
                        // Phase 4.1b: track the latest pane-status pill from
                        // any tool_use block in this assistant message; we
                        // emit it once after the loop so multi-tool messages
                        // don't spam the pill channel.
                        let mut latest_pill: Option<String> = None;
                        match &message {
                            ClaudeStreamMessage::Assistant { message: msg, .. } => {
                                for block in &msg.content {
                                    if let ClaudeContentBlock::ToolUse { id, name, input } = block {
                                        if let Some(pill) =
                                            crate::pane_status::pane_status_from_tool_use(
                                                name, input,
                                            )
                                        {
                                            latest_pill = Some(pill);
                                        }
                                        if name == "Task" {
                                            let mut s = reader_in_flight.lock().unwrap();
                                            if s.insert(id.clone()) {
                                                subagent_state_changed = true;
                                            }
                                        }
                                        // Watched-set: the agent calling
                                        // Monitor or BashOutput on a task is
                                        // the explicit "I care about this
                                        // task's future output" signal.
                                        // Auto-wake only fires for ids in
                                        // this set (poll_background_tasks
                                        // checks it). Foreground bash that
                                        // never gets re-checked stays out
                                        // of the set → no wake noise.
                                        if name == "Monitor" || name == "BashOutput" {
                                            if let Some(tid) =
                                                input.get("task_id").and_then(|v| v.as_str())
                                            {
                                                let mut w = reader_watched.lock().unwrap();
                                                if w.insert(tid.to_string()) {
                                                    tracing::info!(
                                                        pane_id = pane_id_reader,
                                                        task_id = tid,
                                                        tool = name.as_str(),
                                                        "auto-wake: agent expressed interest in task",
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            ClaudeStreamMessage::User { message: msg, .. } => {
                                for block in &msg.content {
                                    if let ClaudeContentBlock::ToolResult { tool_use_id, .. } =
                                        block
                                    {
                                        let mut s = reader_in_flight.lock().unwrap();
                                        if s.remove(tool_use_id) {
                                            subagent_state_changed = true;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }

                        let display_text = format_stream_message(&message);
                        let _ = output_tx_reader.send(PaneOutput {
                            text: display_text,
                            pane_id: pane_id_reader,
                        });
                        let _ = server_tx_reader.blocking_send(CliToServer::StreamMessage {
                            session_id: session_id_reader,
                            message,
                            pane_type: Some(pane_type_reader),
                            pane_id: Some(pane_id_reader),
                        });
                        if is_result {
                            // Parent claude finished the turn → not Thinking
                            // anymore. But subagents may still be doing real
                            // work — we keep the "busy" state via the in-flight
                            // count and only fire the result_signal once the
                            // last subagent drains.
                            if let Ok(mut t) = reader_thinking.lock() {
                                *t = false;
                            }
                        }

                        let n_after = reader_in_flight.lock().map(|s| s.len()).unwrap_or(0);

                        // Determine whether this is the "fully idle"
                        // transition: parent done AND no subagents pending.
                        // We fire result_signal exactly once per turn at this
                        // moment, even if it's late (subagents finished after
                        // parent's Result event).
                        let mut fully_idle_now = false;
                        if is_result {
                            if n_after == 0 {
                                fully_idle_now = true;
                            } else {
                                result_pending = true;
                            }
                        } else if subagent_state_changed && result_pending && n_after == 0 {
                            fully_idle_now = true;
                            result_pending = false;
                        }

                        if is_result || subagent_state_changed {
                            let t = reader_thinking.lock().map(|g| *g).unwrap_or(false);
                            let _ = server_tx_reader.blocking_send(CliToServer::PaneStatus {
                                session_id: session_id_reader,
                                pane_type: pane_type_reader,
                                pane_id: Some(pane_id_reader),
                                status: compose_streaming_status(t, n_after),
                            });
                        } else if let Some(pill) = latest_pill {
                            // Phase 4.1b: push the tool-derived pill so the
                            // pane header shows what the agent is doing
                            // mid-turn. Subagent / result transitions take
                            // priority (`compose_streaming_status` above).
                            let _ = server_tx_reader.blocking_send(CliToServer::PaneStatus {
                                session_id: session_id_reader,
                                pane_type: pane_type_reader,
                                pane_id: Some(pane_id_reader),
                                status: Some(pill),
                            });
                        }

                        if fully_idle_now {
                            tracing::info!(
                                pane_id = pane_id_reader,
                                "streaming pane fully idle (parent done, all subagents done)",
                            );
                            // Notify the deadloop driver (if attached) and
                            // unblock the inner-loop input gate so any
                            // pending user prompts can be flushed.
                            if let Some(ref tx) = reader_result_signal {
                                let _ = tx.send(());
                            }
                        }
                    }
                    None => {
                        let _ = output_tx_reader.send(PaneOutput {
                            text: line,
                            pane_id: pane_id_reader,
                        });
                    }
                }
            }
            tracing::info!(
                pane_id = pane_id_reader,
                lines = line_count,
                io_error = had_io_error,
                "streaming reader exited (stdout closed or EOF)",
            );
            let _ = reader_done_tx.send(());
        });

        let stderr_thread = stderr.map(|err| {
            let output_tx_err = output_tx.clone();
            let server_tx_err = server_tx.clone();
            let pane_id_err = pane_id;
            let session_id_err = session_id;
            let pane_type_err = pane_type;
            thread::spawn(move || {
                let reader = BufReader::new(err);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    if line.trim().is_empty() {
                        continue;
                    }
                    let _ = output_tx_err.send(PaneOutput {
                        text: format!("[stderr] {}", line),
                        pane_id: pane_id_err,
                    });
                    let _ = server_tx_err.blocking_send(CliToServer::Output {
                        session_id: session_id_err,
                        data: format!("[stderr] {}", line),
                        output_type: shared::OutputType::Error,
                        pane_type: Some(pane_type_err),
                        pane_id: Some(pane_id_err),
                    });
                }
            })
        });

        tracing::info!(
            pane_id,
            pid = child_pid,
            using_resume,
            session = %claude_session_id,
            "streaming claude spawned",
        );

        // Inner loop: pump user prompts into stdin, watch for child death.
        let mut break_reason = "shutdown";
        let mut exit_status_str: Option<String> = None;
        let mut prompts_sent: u64 = 0;
        // Pending prompts that arrived while the pane was busy (parent
        // mid-turn or any subagent in flight). Drained FIFO once the pane
        // returns to fully-idle. Auto-wakes and human-typed input share
        // this queue; they're disambiguated by the (from_tui, is_auto_wake)
        // tags.
        let mut pending: VecDeque<(String, bool, bool)> = VecDeque::new();
        loop {
            if shutdown.load(Ordering::SeqCst) {
                if let Ok(mut guard) = child_process.lock() {
                    if let Some(ref mut c) = *guard {
                        let _ = c.kill();
                    }
                }
                break_reason = "shutdown";
                break;
            }

            // Stdout EOF means the child closed it (crashed or exited).
            if reader_done_rx.try_recv().is_ok() {
                break_reason = "stdout-eof";
                break;
            }

            // Soft-interrupt request from the InterruptPane handler. Drain
            // any pending signals (we collapse multiple to one) and write a
            // control_request to claude's stdin. Wire format from
            // happy-cli/src/claude/sdk/query.ts:175-208 — claude responds by
            // aborting the current turn and emitting a Result with an
            // interrupted stop_reason; the process stays alive for the next
            // user prompt.
            let mut interrupted = false;
            while let Ok(()) = interrupt_rx.try_recv() {
                interrupted = true;
            }
            if interrupted {
                let req_id = format!("apas-interrupt-{}", uuid::Uuid::new_v4());
                let envelope = serde_json::json!({
                    "type": "control_request",
                    "request_id": req_id,
                    "request": { "subtype": "interrupt" },
                });
                let line = format!("{}\n", envelope);
                if let Err(e) = stdin.write_all(line.as_bytes()) {
                    tracing::warn!(
                        pane_id,
                        pid = child_pid,
                        error = %e,
                        "streaming interrupt write failed",
                    );
                    break_reason = "stdin-write-failed";
                    break;
                }
                let _ = stdin.flush();
                tracing::info!(
                    pane_id,
                    pid = child_pid,
                    "streaming sent control_request(interrupt)",
                );
                let _ = output_tx.send(PaneOutput {
                    text: "[Interrupted current turn — process still alive.]".to_string(),
                    pane_id,
                });
            }

            // Drain any pending control_response payloads. These come from
            // the reader thread (auto-approvals) and the AnswerQuestion
            // handler (AskUserQuestion submissions). All stdin writes funnel
            // through this single owner so we never interleave a partial
            // JSON line with another writer.
            let mut control_write_failed: Option<std::io::Error> = None;
            while let Ok(payload) = control_response_rx.try_recv() {
                let line = format!("{}\n", payload);
                if let Err(e) = stdin.write_all(line.as_bytes()) {
                    control_write_failed = Some(e);
                    break;
                }
                if let Err(e) = stdin.flush() {
                    control_write_failed = Some(e);
                    break;
                }
                tracing::debug!(
                    pane_id,
                    pid = child_pid,
                    "streaming wrote control_response to stdin",
                );
            }
            if let Some(e) = control_write_failed {
                tracing::warn!(
                    pane_id,
                    pid = child_pid,
                    error = %e,
                    "streaming control_response write failed",
                );
                break_reason = "stdin-write-failed";
                break;
            }

            // Independent liveness check (covers crash where stdout still
            // buffered): try_wait the child.
            let (exited, status_dbg) = if let Ok(mut guard) = child_process.try_lock() {
                if let Some(ref mut c) = *guard {
                    match c.try_wait() {
                        Ok(Some(s)) => (true, Some(format!("{:?}", s))),
                        Ok(None) => (false, None),
                        Err(e) => (true, Some(format!("try_wait error: {}", e))),
                    }
                } else {
                    (true, Some("guard empty".to_string()))
                }
            } else {
                (false, None)
            };
            if exited {
                exit_status_str = status_dbg;
                break_reason = "child-exited";
                break;
            }

            // Compute "busy": parent mid-turn OR any subagent still running.
            // While busy we MUST NOT push another prompt onto stdin, even
            // though stream-json claude technically supports interleaving —
            // doing so would race the parent's in-progress work and confuse
            // the conversation. We queue everything and drain FIFO once the
            // pane returns to fully-idle.
            let busy = {
                let t = thinking.lock().map(|g| *g).unwrap_or(false);
                let n = in_flight_subagents.lock().map(|s| s.len()).unwrap_or(0);
                t || n > 0
            };

            // 1) Drain queued auto-wakes / inputs into `pending` (FIFO).
            //    Auto-wakes are checked first only because their channel is
            //    cheaper to poll; ordering between the two is otherwise
            //    insertion order into `pending`.
            while let Ok(p) = auto_wake_rx.try_recv() {
                pending.push_back((p, false, true));
            }
            match input_rx.recv_timeout(Duration::from_millis(200)) {
                Ok((p, from_tui)) => pending.push_back((p, from_tui, false)),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if let Ok(mut guard) = child_process.lock() {
                        if let Some(ref mut c) = *guard {
                            let _ = c.kill();
                        }
                    }
                    break_reason = "input-channel-closed";
                    break;
                }
            }

            // 2) If busy, do nothing this tick — let subagents drain. We'll
            //    fire next_prompt the moment the reader thread observes
            //    fully-idle (Result + subagents=0) and updates the shared
            //    state.
            let next_prompt: Option<(String, bool, bool)> =
                if !busy { pending.pop_front() } else { None };

            match next_prompt {
                Some((prompt, from_tui, is_auto_wake)) => {
                    let display_prefix = if is_auto_wake { "[auto-wake]" } else { ">" };
                    let _ = output_tx.send(PaneOutput {
                        text: format!(
                            "{} {}",
                            display_prefix,
                            truncate_str_at_char_boundary(&prompt, 140)
                        ),
                        pane_id,
                    });
                    let _ = output_tx.send(PaneOutput {
                        text: "[Thinking...]".to_string(),
                        pane_id,
                    });
                    if let Ok(mut t) = thinking.lock() {
                        *t = true;
                    }
                    let n_subagents = in_flight_subagents.lock().map(|s| s.len()).unwrap_or(0);
                    let _ = server_tx.blocking_send(CliToServer::PaneStatus {
                        session_id,
                        pane_type,
                        pane_id: Some(pane_id),
                        status: compose_streaming_status(true, n_subagents),
                    });
                    if from_tui && !is_auto_wake {
                        let _ = server_tx.blocking_send(CliToServer::UserInput {
                            session_id,
                            text: prompt.clone(),
                            pane_type: Some(pane_type),
                            pane_id: Some(pane_id),
                        });
                    }
                    // Snapshot the live effort so an `ultracode`-configured
                    // pane prepends the workflow keyword to each prompt.
                    // This is the only behavioural difference between
                    // `ultracode` and `xhigh` — the wire flag is the same.
                    let live_effort = effort_arc.lock().ok().and_then(|g| g.clone());
                    let line = build_user_envelope_line(&prompt, live_effort.as_deref());
                    if let Err(e) = stdin.write_all(line.as_bytes()) {
                        let _ = output_tx.send(PaneOutput {
                            text: format!("[Failed to send input to agent: {}]", e),
                            pane_id,
                        });
                        tracing::warn!(
                            pane_id,
                            pid = child_pid,
                            error = %e,
                            "streaming stdin write failed",
                        );
                        break_reason = "stdin-write-failed";
                        break;
                    }
                    let _ = stdin.flush();
                    prompts_sent += 1;
                    tracing::info!(
                        pane_id,
                        pid = child_pid,
                        prompts_sent,
                        prompt_len = prompt.len(),
                        is_auto_wake,
                        "streaming wrote prompt to stdin",
                    );
                }
                None => {
                    // No prompt this tick (auto-wake empty + input_rx timed out);
                    // loop again to check shutdown / liveness.
                }
            }
        }

        tracing::info!(
            pane_id,
            pid = child_pid,
            reason = break_reason,
            prompts_sent,
            exit_status = ?exit_status_str,
            "streaming claude inner loop ended",
        );

        // Drop stdin to signal EOF to claude (it will exit cleanly if it
        // hasn't already), then reap.
        drop(stdin);
        if let Ok(mut guard) = child_process.lock() {
            if let Some(mut c) = guard.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        let _ = reader_thread.join();
        if let Some(t) = stderr_thread {
            let _ = t.join();
        }

        let _ = server_tx.blocking_send(CliToServer::PaneStatus {
            session_id,
            pane_type,
            pane_id: Some(pane_id),
            status: None,
        });

        if shutdown.load(Ordering::SeqCst) || break_reason == "input-channel-closed" {
            return;
        }

        // If we tried --resume on a session that doesn't exist on disk yet,
        // the child exits almost immediately with an error. Flip to
        // --session-id for the next spawn so we create the session instead.
        if using_resume && !session_jsonl_exists(&effective_dir, &claude_session_id) {
            try_resume_first = false;
            let _ = output_tx.send(PaneOutput {
                text: "[No prior session found, creating fresh session...]".to_string(),
                pane_id,
            });
        } else {
            // Once the session exists, all future restarts must --resume.
            try_resume_first = true;
        }

        // Brief backoff before respawning, so a crash-loop doesn't burn CPU.
        thread::sleep(Duration::from_secs(2));
    }
}

/// Streaming variant for `Provider::Claude` deadloop panes (a.k.a. bots).
///
/// **Agent-driven via `/loop`**: rather than apas firing iteration prompts
/// on a fixed timer, we kick off claude code's built-in `/loop` skill once
/// at startup and let claude pace itself via `ScheduleWakeup`. The runtime
/// fires the next iteration internally on the agent's chosen schedule
/// (verified to work in `--print --input-format stream-json` mode by the
/// /loop spike). When the agent has bg work in flight it can extend the
/// next-wake delay; when it's truly done it can stop calling
/// ScheduleWakeup, ending the loop naturally.
///
/// Driver responsibilities:
///   * Send `/loop <prompt>` once on the streaming worker's `input_tx`
///   * Watch `shutdown` / `stop_requested` / `pause` and react
///   * On `stop_requested`: send a soft control_request via the streaming
///     worker's interrupt channel (aborts the current turn AND cancels any
///     pending ScheduleWakeup), drop input_tx so the worker tears down,
///     fire FinalizeStopBot to flip the pane back to interactive mode
///
/// Codex bots keep the legacy per-iteration `--print` driver below.
#[allow(clippy::too_many_arguments)]
fn run_deadloop_session_streaming(
    binary_path: &str,
    working_dir: &str,
    worktree_path: Option<String>,
    system_prompt: Option<String>,
    plan_review_mode_arc: Arc<Mutex<shared::PlanReviewMode>>,
    pending_plan_reviews: Arc<Mutex<HashMap<String, PendingPlanReview>>>,
    session_id: Uuid,
    claude_session_id: Uuid,
    pane_id: u32,
    prompt: &str,
    model: Option<String>,
    effort: Option<String>,
    min_iteration_interval_minutes: u64,
    provider: &Provider,
    output_tx: mpsc::Sender<PaneOutput>,
    server_tx: tokio_mpsc::Sender<CliToServer>,
    shutdown: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    stop_requested: Arc<AtomicBool>,
    child_process: Arc<Mutex<Option<std::process::Child>>>,
    event_tx: mpsc::Sender<TuiEvent>,
    interrupt_tx_slot: Arc<Mutex<Option<mpsc::Sender<()>>>>,
    control_response_tx_slot: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pending_questions: Arc<Mutex<HashMap<String, PendingAskQuestion>>>,
    effort_arc: Arc<Mutex<Option<String>>>,
    // Bugfix: register the local input_tx here so web/TUI input can
    // reach a running deadloop pane. Pre-fix, deadloop input_tx lived
    // only inside this function and external input hit "Pane worker
    // unavailable; restart requested. Please resend." every time.
    input_channels: InputChannels,
    // Currently unused on the streaming path — the agent self-paces
    // via its own ScheduleWakeup tool, so we don't drive the wait
    // here. Kept in the signature for parity with the codex path; can
    // be wired in if/when the streaming driver gains a between-turn
    // wait we control.
    file_watcher: Arc<crate::file_watcher::ProjectFileWatcher>,
) {
    let _ = file_watcher; // see note above
    let _ = output_tx.send(PaneOutput {
        text: format!(
            "[Streaming /loop deadloop session: {}]",
            &claude_session_id.to_string()[..8]
        ),
        pane_id,
    });

    let (input_tx, input_rx) = mpsc::channel::<PaneInput>();
    // Register so external input flows here as if the deadloop were
    // an interactive pane. The streaming worker writes everything from
    // input_rx onto claude's stdin, so external input becomes another
    // turn for the running /loop — the agent picks it up between its
    // self-paced iterations.
    {
        let mut channels = input_channels.lock().unwrap();
        channels.insert(pane_id, input_tx.clone());
    }

    // Spawn the streaming worker. It keeps claude alive; we kick off /loop
    // exactly once and the runtime self-paces from there. We pass
    // `result_signal_tx = None` because we don't gate on iterations — the
    // agent picks its own cadence via ScheduleWakeup.
    {
        let binary_path = binary_path.to_string();
        let working_dir = working_dir.to_string();
        let worktree_path = worktree_path.clone();
        let system_prompt = system_prompt.clone();
        let plan_review_mode_arc_clone = plan_review_mode_arc.clone();
        let pending_plan_reviews_clone = pending_plan_reviews.clone();
        let model = model.clone();
        let effort = effort.clone();
        let provider = *provider;
        let output_tx = output_tx.clone();
        let server_tx = server_tx.clone();
        let shutdown = shutdown.clone();
        let child_process = child_process.clone();
        let interrupt_tx_slot = interrupt_tx_slot.clone();
        let control_response_tx_slot = control_response_tx_slot.clone();
        let pending_questions = pending_questions.clone();
        let effort_arc = effort_arc.clone();
        thread::spawn(move || {
            run_pane_session_streaming(
                &binary_path,
                &working_dir,
                worktree_path,
                system_prompt,
                plan_review_mode_arc_clone,
                pending_plan_reviews_clone,
                session_id,
                claude_session_id,
                pane_id,
                &provider,
                model,
                effort,
                input_rx,
                output_tx,
                server_tx,
                shutdown,
                child_process,
                interrupt_tx_slot,
                control_response_tx_slot,
                pending_questions,
                effort_arc,
                None, // no iteration gating; /loop runtime self-paces
                PaneType::Deadloop,
            );
        });
    }

    // Kick off `/loop`. The runtime keeps re-firing the prompt on the
    // schedule the agent dictates via ScheduleWakeup — no further action
    // from this thread. `min_iteration_interval_minutes` is communicated
    // to the agent in the wrapper so it can pick a sensible cadence
    // (the runtime also enforces a ~120s floor independently).
    let loop_cadence_hint = if min_iteration_interval_minutes > 0 {
        format!(
            "Iterate at roughly {}-minute cadence. Use ScheduleWakeup with delaySeconds={} between iterations, or longer if background work needs more time. ",
            min_iteration_interval_minutes,
            min_iteration_interval_minutes.saturating_mul(60),
        )
    } else {
        String::new()
    };
    let loop_input = format!("/loop {}{}", loop_cadence_hint, prompt);
    let _ = output_tx.send(PaneOutput {
        text: format!(
            "[Bot started in /loop mode (agent-paced); cadence hint: {}m]",
            min_iteration_interval_minutes
        ),
        pane_id,
    });
    let _ = server_tx.try_send(CliToServer::UserInput {
        session_id,
        text: loop_input.clone(),
        pane_type: Some(PaneType::Deadloop),
        pane_id: Some(pane_id),
    });
    if input_tx.send((loop_input, false)).is_err() {
        let _ = output_tx.send(PaneOutput {
            text: "[Streaming worker exited before /loop kickoff; deadloop ending.]".to_string(),
            pane_id,
        });
        return;
    }

    // Sit and watch shutdown / stop / pause. The /loop runtime drives
    // claude on its own; we don't fire any further prompts — EXCEPT the
    // watchdog below.
    //
    // Watchdog: the agent-paced design has a single point of failure.
    // The agent calls ScheduleWakeup and we trust claude-code's /loop
    // runtime to fire it; in the wild that timer has been observed to
    // die silently (wakeup accepted at 06:28, due 06:38, never fired —
    // child alive but dormant for 11h, zero session-jsonl writes, no
    // error anywhere). Since we deliberately don't gate iterations, no
    // one notices. So: watch the session jsonl's mtime as the activity
    // signal (claude appends every event while working OR when a
    // wakeup fires) and, if the pane has been dead-quiet well past its
    // cadence, push a resume prompt through the pane's own input
    // channel — it lands on claude's stdin as a user turn and the
    // /loop picks it back up.
    //
    // Threshold: 3× cadence, floor 30 min. ScheduleWakeup legitimately
    // sleeps up to 60 min, so a long-sleeping agent may get nudged
    // ~half an hour early — that just runs an iteration sooner (an
    // "Idle; waiting" no-op at worst), which is a far better failure
    // mode than the hours-long stalls a lost wakeup causes.
    let watchdog_jsonl = session_jsonl_path(
        worktree_path.as_deref().unwrap_or(working_dir),
        &claude_session_id,
    );
    let idle_threshold = Duration::from_secs(
        std::cmp::max(min_iteration_interval_minutes.saturating_mul(3), 30) * 60,
    );
    let mut watchdog_state = DeadloopWatchdogState::new(Instant::now());
    let mut watchdog_tick: u32 = 0;

    let mut was_paused = false;
    while !shutdown.load(Ordering::SeqCst) {
        if stop_requested.load(Ordering::SeqCst) {
            // Soft interrupt: aborts the current turn and (we expect) the
            // pending ScheduleWakeup. Then drop input_tx so the streaming
            // worker tears down — FinalizeStopBot will rebuild the pane
            // worker as plain interactive mode.
            if let Some(tx) = interrupt_tx_slot
                .lock()
                .ok()
                .and_then(|g| g.as_ref().cloned())
            {
                let _ = tx.send(());
            }
            let _ = output_tx.send(PaneOutput {
                text: "[Bot stop requested — interrupting /loop and finalizing.]".to_string(),
                pane_id,
            });
            let _ = event_tx.send(TuiEvent::FinalizeStopBot {
                pane_id,
                stop_flag: stop_requested.clone(),
            });
            return;
        }

        // Pause for streaming /loop bots: there's no way to ask claude
        // "wait" mid-run, so we treat pause as "abort the current turn,
        // reap the process, and tear down this supervisor." The pane
        // stays in Deadloop mode with child_process cleared; ResumePane
        // fires a fresh StartBot which spins up claude again with the
        // saved prompt (and --resume claude_session_id so context is
        // preserved). Previously this branch just logged a note and let
        // the loop continue, which made Pause functionally a no-op.
        if pause.load(Ordering::SeqCst) {
            if !was_paused {
                was_paused = true;
                if let Some(tx) = interrupt_tx_slot
                    .lock()
                    .ok()
                    .and_then(|g| g.as_ref().cloned())
                {
                    let _ = tx.send(());
                }
                {
                    let mut channels = input_channels.lock().unwrap();
                    channels.remove(&pane_id);
                }
                if let Ok(mut child_guard) = child_process.lock() {
                    if let Some(child) = child_guard.take() {
                        let mut child = child;
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                }
                let _ = output_tx.send(PaneOutput {
                    text: "[Bot paused — interrupted /loop and reaped claude. Click Resume to restart with the saved session.]".to_string(),
                    pane_id,
                });
            }
            return;
        }

        // Watchdog check every ~30s (loop ticks at 1s).
        watchdog_tick += 1;
        if watchdog_tick >= 30 {
            watchdog_tick = 0;
            let now = Instant::now();
            let session_mtime = watchdog_jsonl
                .as_ref()
                .and_then(|p| std::fs::metadata(p).ok())
                .and_then(|m| m.modified().ok());
            if let DeadloopWatchdogDecision::Nudge { idle_minutes } = evaluate_deadloop_watchdog(
                &mut watchdog_state,
                now,
                idle_threshold,
                session_mtime,
                watchdog_jsonl.is_some(),
                true,
            ) {
                tracing::warn!(
                    pane_id,
                    idle_minutes,
                    "watchdog: /loop pane silent past threshold — nudging agent"
                );
                let nudge = format!(
                    "[watchdog] No pane activity for {} minutes — the scheduled /loop wakeup appears to have been lost. \
                     Resume the loop now: run the next iteration per the original /loop instructions, then schedule the next wakeup as usual.",
                    idle_minutes
                );
                let _ = output_tx.send(PaneOutput {
                    text: format!(
                        "[Watchdog: no activity for {}m — nudging stalled /loop]",
                        idle_minutes
                    ),
                    pane_id,
                });
                let _ = server_tx.try_send(CliToServer::UserInput {
                    session_id,
                    text: nudge.clone(),
                    pane_type: Some(PaneType::Deadloop),
                    pane_id: Some(pane_id),
                });
                if input_tx.send((nudge, false)).is_err() {
                    // Streaming worker is gone; nothing to nudge. The
                    // inner worker normally respawns claude itself, so
                    // a dead channel means the pane is being torn down.
                    let _ = output_tx.send(PaneOutput {
                        text:
                            "[Watchdog: streaming worker gone — cannot nudge; pane needs a Reboot.]"
                                .to_string(),
                        pane_id,
                    });
                }
            }
        }

        thread::sleep(Duration::from_secs(1));
    }
}

/// Run a generic interactive pane session.
/// Input comes from a single channel — both TUI and web input are routed through input_channels.
#[allow(clippy::too_many_arguments)]
fn run_pane_session(
    binary_path: &str,
    working_dir: &str,
    worktree_path: Option<String>,
    system_prompt: Option<String>,
    plan_review_mode_arc: Arc<Mutex<shared::PlanReviewMode>>,
    pending_plan_reviews: Arc<Mutex<HashMap<String, PendingPlanReview>>>,
    session_id: Uuid,
    claude_session_id: Uuid,
    pane_id: u32,
    provider: &Provider,
    model: Option<String>,
    effort: Option<String>,
    input_rx: mpsc::Receiver<PaneInput>,
    output_tx: mpsc::Sender<PaneOutput>,
    server_tx: tokio_mpsc::Sender<CliToServer>,
    shutdown: Arc<AtomicBool>,
    child_process: Arc<Mutex<Option<std::process::Child>>>,
    interrupt_tx_slot: Arc<Mutex<Option<mpsc::Sender<()>>>>,
    control_response_tx_slot: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pending_questions: Arc<Mutex<HashMap<String, PendingAskQuestion>>>,
    effort_arc: Arc<Mutex<Option<String>>>,
    // Initial value for the legacy path's `try_resume_first` — false
    // when the worker is being spawned with a fresh-just-minted
    // session id (e.g. provider switch) so codex's `exec resume <id>`
    // doesn't error with "no rollout found". The Claude streaming
    // path derives this from disk and ignores this knob.
    initial_try_resume: bool,
) {
    // Provider::Claude → long-lived stream-json process. Other supported
    // providers use the per-turn worker path below.
    if matches!(provider, Provider::Claude) {
        return run_pane_session_streaming(
            binary_path,
            working_dir,
            worktree_path,
            system_prompt,
            plan_review_mode_arc,
            pending_plan_reviews,
            session_id,
            claude_session_id,
            pane_id,
            provider,
            model,
            effort,
            input_rx,
            output_tx,
            server_tx,
            shutdown,
            child_process,
            interrupt_tx_slot,
            control_response_tx_slot,
            pending_questions,
            effort_arc,
            None, // no deadloop driver listening for Result events
            PaneType::Interactive,
        );
    }
    let _ = plan_review_mode_arc; // non-claude legacy path doesn't gate
    let _ = pending_plan_reviews;
    let _ = system_prompt; // codex/cursor/etc. ignored for now — see role.rs note.
    let effective_dir: String = worktree_path.as_deref().unwrap_or(working_dir).to_string();
    let _ = interrupt_tx_slot; // unused for legacy path
    let _ = control_response_tx_slot; // unused for legacy path
    let _ = pending_questions; // unused for legacy path
    let _ = effort_arc; // unused for legacy path

    let mut first_message = true;
    let mut try_resume_first = initial_try_resume;
    // For Codex, we need to capture the real thread_id from the first invocation
    // and use it for subsequent `codex exec resume` calls.
    let mut claude_session_id = claude_session_id;

    let _ = output_tx.send(PaneOutput {
        text: format!("[Session: {}]", &claude_session_id.to_string()[..8]),
        pane_id,
    });
    // Suppress duplicate env-config errors across turns; cleared on success.
    let mut last_env_err: Option<String> = None;

    while !shutdown.load(Ordering::SeqCst) {
        // Wait for user input (from TUI or web, both routed through same channel)
        let (prompt, from_tui) = {
            match input_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(p) => p,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    // Input channel closed — pane was removed
                    let _ = output_tx.send(PaneOutput {
                        text: "[Pane closed]".to_string(),
                        pane_id,
                    });
                    return;
                }
            }
        };

        let _ = output_tx.send(PaneOutput {
            text: format!("> {}", truncate_str_at_char_boundary(&prompt, 100)),
            pane_id,
        });

        let _ = output_tx.send(PaneOutput {
            text: "[Thinking...]".to_string(),
            pane_id,
        });
        let _ = server_tx.blocking_send(CliToServer::PaneStatus {
            session_id,
            pane_type: shared::PaneType::Interactive,
            pane_id: Some(pane_id),
            status: Some("Thinking...".to_string()),
        });

        // Forward TUI input to server (for web UI display).
        // Web-originated input is already echoed by the server, so skip to avoid duplicates.
        if from_tui {
            let _ = server_tx.blocking_send(CliToServer::UserInput {
                session_id,
                text: prompt.clone(),
                pane_type: Some(PaneType::Interactive),
                pane_id: Some(pane_id),
            });
        }

        let (args, using_resume) = build_agent_args(
            provider,
            &claude_session_id,
            &prompt,
            model.as_deref(),
            effort.as_deref(),
            first_message,
            try_resume_first,
        );
        if first_message && !try_resume_first {
            first_message = false;
        }

        let pane_env = match build_pane_env_overrides(provider, model.as_deref()) {
            Ok(env) => {
                last_env_err = None;
                env
            }
            Err(err) => {
                if last_env_err.as_deref() != Some(err.as_str()) {
                    last_env_err = Some(err.clone());
                    let error_text = format!("[{}]", err);
                    let _ = output_tx.send(PaneOutput {
                        text: error_text.clone(),
                        pane_id,
                    });
                    let _ = server_tx.blocking_send(CliToServer::Output {
                        session_id,
                        data: error_text,
                        output_type: shared::OutputType::Text,
                        pane_type: Some(PaneType::Interactive),
                        pane_id: Some(pane_id),
                    });
                }
                continue;
            }
        };

        // Defensive: if a previous agent for this same session_id is somehow
        // still alive (orphaned grandchild kept the stdout pipe open and we
        // never reaped, or a parallel pane assigned the same id), kill it
        // before spawning a new --resume. Two concurrent --resume processes
        // on the same session would interleave writes to its .jsonl.
        // Mirrors the deadloop spawn path.
        kill_processes_using_session(&claude_session_id.to_string());

        let mut command = Command::new(binary_path);
        command
            .args(&args)
            .current_dir(&effective_dir)
            // Clear CLAUDECODE so Claude CLI doesn't refuse to start (nesting detection)
            .env_remove("CLAUDECODE")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &pane_env {
            command.env(key, value);
        }
        spawn_in_own_process_group(&mut command);

        match command.spawn() {
            Ok(mut child) => {
                let stdout = child.stdout.take().unwrap();
                let stderr = child.stderr.take().unwrap();

                // Store child in shared Arc so StopBot can kill it
                {
                    let mut guard = child_process.lock().unwrap();
                    *guard = Some(child);
                }

                let (stdout_tx, stdout_rx) = mpsc::channel::<Option<String>>();
                let stdout_thread = thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines() {
                        match line {
                            Ok(l) => {
                                if stdout_tx.send(Some(l)).is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    let _ = stdout_tx.send(None);
                });

                let output_tx_stderr = output_tx.clone();
                let server_tx_stderr = server_tx.clone();
                let pane_id_stderr = pane_id;
                let sid_stderr = session_id;
                let stderr_thread = thread::spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines() {
                        if let Ok(line) = line {
                            if !line.trim().is_empty() {
                                let _ = output_tx_stderr.send(PaneOutput {
                                    text: format!("[stderr] {}", line),
                                    pane_id: pane_id_stderr,
                                });
                                // Also forward stderr to server so it's visible in web UI
                                let _ = server_tx_stderr.blocking_send(CliToServer::Output {
                                    session_id: sid_stderr,
                                    data: format!("[stderr] {}", line),
                                    output_type: shared::OutputType::Text,
                                    pane_type: Some(PaneType::Interactive),
                                    pane_id: Some(pane_id_stderr),
                                });
                            }
                        }
                    }
                });

                let check_interval = Duration::from_millis(100);
                let mut process_exited = false;
                let mut timeouts_after_exit: u32 = 0;
                // 100ms * 30 = 3s grace for stdout drain after the child dies.
                // Without this, an orphaned grandchild (Monitor watcher,
                // tail -F, etc.) holding the stdout fd would keep the pipe
                // open forever and we'd never reach the wait() below — leaving
                // a [claude] <defunct> zombie under the apas daemon.
                const MAX_TIMEOUTS_AFTER_EXIT: u32 = 30;
                loop {
                    if shutdown.load(Ordering::SeqCst) {
                        if let Ok(mut guard) = child_process.lock() {
                            if let Some(ref mut c) = *guard {
                                let _ = c.kill();
                            }
                        }
                        break;
                    }

                    if !process_exited {
                        if let Ok(mut guard) = child_process.try_lock() {
                            if let Some(ref mut child) = *guard {
                                if let Ok(Some(_status)) = child.try_wait() {
                                    process_exited = true;
                                }
                            }
                        }
                    }

                    let session_id_str = claude_session_id.to_string();
                    match stdout_rx.recv_timeout(check_interval) {
                        Ok(Some(line)) => {
                            if line.trim().is_empty() {
                                continue;
                            }

                            // Capture Codex thread_id for session resume
                            if *provider == Provider::Codex {
                                if let Ok(codex_msg) =
                                    serde_json::from_str::<CodexStreamMessage>(&line)
                                {
                                    if let CodexStreamMessage::ThreadStarted { thread_id } =
                                        codex_msg
                                    {
                                        if let Ok(tid) = Uuid::parse_str(&thread_id) {
                                            claude_session_id = tid;
                                        }
                                        continue;
                                    }
                                }
                            }

                            match parse_agent_output(provider, &line, &session_id_str) {
                                Some(message) => {
                                    // The agent's result event signals "turn complete" at the
                                    // protocol level. Clear the pane's transient status now,
                                    // even if the underlying process keeps lingering (e.g. it
                                    // spawned background children like `tail -F`). Without
                                    // this, the UI shows "Thinking..." forever after the user
                                    // already sees the final answer + cost.
                                    let is_result =
                                        matches!(message, ClaudeStreamMessage::Result { .. });
                                    let display_text = format_stream_message(&message);
                                    let _ = output_tx.send(PaneOutput {
                                        text: display_text,
                                        pane_id,
                                    });
                                    let _ = server_tx.blocking_send(CliToServer::StreamMessage {
                                        session_id,
                                        message,
                                        pane_type: Some(PaneType::Interactive),
                                        pane_id: Some(pane_id),
                                    });
                                    if is_result {
                                        let _ = server_tx.blocking_send(CliToServer::PaneStatus {
                                            session_id,
                                            pane_type: PaneType::Interactive,
                                            pane_id: Some(pane_id),
                                            status: None,
                                        });
                                        // We deliberately do NOT kill the agent's process
                                        // group here even though `result` signals "turn
                                        // done": the user may have intentionally launched
                                        // long-lived background work (nohup builds, etc.)
                                        // that should outlive the turn. The Interrupt
                                        // button is the explicit, user-initiated way to
                                        // tear everything down.
                                    }
                                }
                                None => {
                                    let _ = output_tx.send(PaneOutput {
                                        text: line,
                                        pane_id,
                                    });
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if process_exited {
                                timeouts_after_exit += 1;
                                if timeouts_after_exit >= MAX_TIMEOUTS_AFTER_EXIT {
                                    break;
                                }
                            }
                            continue;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }

                let exit_status = {
                    let mut guard = child_process.lock().unwrap();
                    guard.as_mut().map(|c| c.wait())
                };
                // Clear child reference after wait
                {
                    let mut guard = child_process.lock().unwrap();
                    *guard = None;
                }
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();

                let _ = server_tx.blocking_send(CliToServer::PaneStatus {
                    session_id,
                    pane_type: shared::PaneType::Interactive,
                    pane_id: Some(pane_id),
                    status: None,
                });

                let exit_status = exit_status.unwrap_or(Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "child already taken",
                )));
                let had_error = exit_status.as_ref().map(|s| !s.success()).unwrap_or(true);
                if had_error {
                    let exit_msg = match &exit_status {
                        Ok(s) => format!("exit code {}", s),
                        Err(e) => format!("wait error: {}", e),
                    };
                    let error_text = if first_message && using_resume {
                        try_resume_first = false;
                        format!(
                            "[Session resume failed ({}), will create new session on next message...]",
                            exit_msg
                        )
                    } else {
                        format!(
                            "[{} process failed: {}]",
                            provider_display_name(provider, model.as_deref()),
                            exit_msg
                        )
                    };
                    let _ = output_tx.send(PaneOutput {
                        text: error_text.clone(),
                        pane_id,
                    });
                    // Forward error to server so it's visible in web UI
                    let _ = server_tx.blocking_send(CliToServer::Output {
                        session_id,
                        data: error_text,
                        output_type: shared::OutputType::Text,
                        pane_type: Some(PaneType::Interactive),
                        pane_id: Some(pane_id),
                    });
                } else {
                    first_message = false;
                }
            }
            Err(e) => {
                let error_text = format_spawn_error(provider, model.as_deref(), binary_path, &e);
                let _ = output_tx.send(PaneOutput {
                    text: error_text.clone(),
                    pane_id,
                });
                // Forward spawn error to server so it's visible in web UI
                let _ = server_tx.blocking_send(CliToServer::Output {
                    session_id,
                    data: error_text,
                    output_type: shared::OutputType::Text,
                    pane_type: Some(PaneType::Interactive),
                    pane_id: Some(pane_id),
                });
                if first_message && using_resume {
                    try_resume_first = false;
                }
            }
        }
    }
}

/// Truncate a string to max_chars characters, respecting UTF-8 boundaries
fn truncate_string(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}...", truncated)
    }
}

/// Format a stream message for display
fn format_stream_message(message: &ClaudeStreamMessage) -> String {
    match message {
        ClaudeStreamMessage::System { model, tools, .. } => {
            format!(
                "[Session started - Model: {}, Tools: {}]",
                model,
                tools.len()
            )
        }
        ClaudeStreamMessage::Assistant { message, .. } => {
            let mut output = String::new();
            for block in &message.content {
                match block {
                    shared::ClaudeContentBlock::Text { text } => output.push_str(text),
                    shared::ClaudeContentBlock::ToolUse { name, input, .. } => {
                        output.push_str(&format!("[Tool: {} - {:?}]", name, input));
                    }
                    shared::ClaudeContentBlock::ToolResult {
                        content, is_error, ..
                    } => {
                        let status = if *is_error { "Error" } else { "Result" };
                        let preview = truncate_string(content, 100);
                        output.push_str(&format!("[{}: {}]", status, preview));
                    }
                }
            }
            output
        }
        ClaudeStreamMessage::User { message, .. } => {
            let mut output = String::new();
            for block in &message.content {
                if let shared::ClaudeContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = block
                {
                    let preview = truncate_string(content, 50);
                    output.push_str(&format!("[Tool result {}: {}]", tool_use_id, preview));
                }
            }
            output
        }
        ClaudeStreamMessage::Result {
            subtype,
            result,
            total_cost_usd,
            duration_ms,
            ..
        } => {
            if result.is_empty() {
                format!(
                    "[{} - Cost: ${:.4}, Duration: {}ms]",
                    subtype, total_cost_usd, duration_ms
                )
            } else {
                format!(
                    "[{}: {} - Cost: ${:.4}, Duration: {}ms]",
                    subtype, result, total_cost_usd, duration_ms
                )
            }
        }
    }
}

/// Report a host-side retirement rejection both locally and to the server.
/// Older servers can route requests that upgraded servers reject themselves;
/// sending the system output back keeps stale web clients informed without
/// closing the mixed-version connection.
fn retired_launch_rejection_output(session_id: Uuid, pane_id: u32, text: &str) -> CliToServer {
    CliToServer::Output {
        session_id,
        data: text.to_string(),
        output_type: shared::OutputType::System,
        pane_type: None,
        pane_id: Some(pane_id),
    }
}

async fn report_retired_launch_rejection<S>(
    status_tx: &mpsc::Sender<PaneOutput>,
    ws_sender: &mut S,
    session_id: Uuid,
    pane_id: u32,
    text: &str,
) where
    S: futures::Sink<tokio_tungstenite::tungstenite::Message> + Unpin,
{
    use futures::SinkExt;

    let _ = status_tx.send(PaneOutput {
        text: text.to_string(),
        pane_id,
    });
    let message = retired_launch_rejection_output(session_id, pane_id, text);
    if let Ok(encoded) = serde_json::to_string(&message) {
        let _ = ws_sender
            .send(tokio_tungstenite::tungstenite::Message::Text(
                encoded.into(),
            ))
            .await;
    }
}

/// How a project ended.
///
/// A project used to end the process to say any of this — `process::exit` for
/// a reboot, and again for a rejected session. That reads the same only while
/// a process hosts exactly one project. Returning it instead lets the caller
/// decide what it means: today `run_headless`'s caller replaces the process,
/// and once projects share one it replaces just that project's task.
#[derive(Debug, PartialEq)]
pub enum ProjectOutcome {
    /// Shut down normally.
    Completed,
    /// Stopped for a reason worth reporting.
    Stopped(String),
    /// Asked to restart.
    RebootRequested,
}

/// Why a project stopped, when it stopped for a reason worth reporting.
///
/// The ws loop runs deep inside the project and cannot return a value to
/// anyone: `run_inner` discards its result (`let _ = server_task.await`). A
/// rejected session used to call `process::exit(2)` from there, which was
/// survivable only because a process hosted exactly one project. Recording the
/// reason here lets the project end and be reported without ending anything
/// else.
pub type ProjectStopReason = Arc<Mutex<Option<String>>>;

/// Run server connection, reconnecting automatically with bounded backoff.
async fn run_server_connection(
    server_url: &str,
    token: &str,
    session_id: Uuid,
    working_dir: &str,
    mut output_rx: tokio_mpsc::Receiver<CliToServer>,
    shutdown: Arc<AtomicBool>,
    pane_pauses: PanePauses,
    pane_stop_requests: PaneStopRequests,
    reboot_requested: Arc<AtomicBool>,
    input_channels: InputChannels,
    pane_metas: PaneMetas,
    pane_sessions: Arc<Mutex<HashMap<u32, Uuid>>>,
    terminal_panes: TerminalPanes,
    // Questions terminal panes are blocked on, so an answer can be refused
    // when it is stale, duplicated, or for a pane that is not asking.
    terminal_questions: TerminalQuestions,
    // Set when this project stops for a reason the caller should report.
    stop_reason: ProjectStopReason,
    status_tx: mpsc::Sender<PaneOutput>,
    tui_event_tx: mpsc::Sender<TuiEvent>,
    project_policy: ProjectPolicyState,
    machine_id: Option<Uuid>,
    server_connection_generation: Arc<AtomicU64>,
    mut reboot_handoff: Option<(std::path::PathBuf, crate::pane_host::RebootHandoffMarker)>,
) -> Result<()> {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    const USAGE_FETCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5 * 60);
    const USAGE_CACHE_MAX_AGE_MINUTES: i64 = 45;

    let mut reconnect_delay = std::time::Duration::from_secs(1);
    let max_reconnect_delay = std::time::Duration::from_secs(60);
    let mut connection_count = 0u32;

    // Resolve the project's git remote once: it can't change between
    // reconnects, so we avoid re-shelling out to git on every loop iteration.
    let git_remote = crate::worktree::normalized_git_remote(std::path::Path::new(working_dir));
    let git_remote_url = crate::worktree::raw_git_remote(std::path::Path::new(working_dir));
    let summary_runner = match crate::summary_runner::SummaryRunner::from_config(
        &crate::config::Config::load().unwrap_or_default(),
    )
    .await
    {
        Ok(runner) => runner,
        Err(error) => {
            tracing::warn!(%error, "Pane summary adapter disabled after validation failure");
            None
        }
    };
    let (summary_result_tx, mut summary_result_rx) =
        tokio_mpsc::channel::<shared::PaneWorkSummaryGenerationResult>(1);

    while !shutdown.load(Ordering::SeqCst) {
        let ws_url = format!("{}/ws/cli", server_url);

        if connection_count > 0 {
            let _ = status_tx.send(PaneOutput {
                text: format!("[Server: Reconnecting... (attempt {})]", connection_count),
                pane_id: shared::PANE_ID_DEADLOOP,
            });
        }

        match connect_async(&ws_url).await {
            Ok((ws_stream, _)) => {
                if let Ok(mut policy) = project_policy.lock() {
                    *policy = None;
                }
                connection_count += 1;
                reconnect_delay = std::time::Duration::from_secs(1);
                let (mut ws_sender, mut ws_receiver) = ws_stream.split();

                // Register
                let mut capabilities = vec![
                    shared::PROJECT_POLICY_CAPABILITY.to_string(),
                    shared::MOBILE_TASK_LAUNCH_CAPABILITY.to_string(),
                    shared::OPENCODE_TERMINAL_CAPABILITY.to_string(),
                    shared::CLI_LIFECYCLE_CAPABILITY.to_string(),
                ];
                if summary_runner.is_some() {
                    capabilities.push(shared::PANE_WORK_SUMMARY_CAPABILITY.to_string());
                }
                if crate::pane_host::persistent_hosting_available() {
                    capabilities.push(shared::PERSISTENT_TERMINAL_HOST_CAPABILITY.to_string());
                }
                let register_msg = CliToServer::Register {
                    token: token.to_string(),
                    version: Some(env!("APAS_VERSION").to_string()),
                    capabilities,
                };
                let msg_text = match serde_json::to_string(&register_msg) {
                    Ok(t) => t,
                    Err(e) => {
                        let _ = status_tx.send(PaneOutput {
                            text: format!("[Server: Failed to serialize register message - {}]", e),
                            pane_id: shared::PANE_ID_DEADLOOP,
                        });
                        tokio::time::sleep(reconnect_delay).await;
                        continue;
                    }
                };
                if ws_sender
                    .send(Message::Text(msg_text.into()))
                    .await
                    .is_err()
                {
                    let _ = status_tx.send(PaneOutput {
                        text: "[Server: Connection lost during registration]".to_string(),
                        pane_id: shared::PANE_ID_DEADLOOP,
                    });
                    tokio::time::sleep(reconnect_delay).await;
                    continue;
                }

                // Wait for registration response
                let registration_timeout =
                    tokio::time::timeout(std::time::Duration::from_secs(30), async {
                        while let Some(Ok(msg)) = ws_receiver.next().await {
                            match msg {
                                Message::Text(text) => {
                                    let response: ServerToCli = match serde_json::from_str(&text) {
                                        Ok(r) => r,
                                        Err(_) => continue,
                                    };
                                    match response {
                                        ServerToCli::Registered { cli_id } => {
                                            return Some(Ok(cli_id));
                                        }
                                        ServerToCli::RegistrationFailed { reason } => {
                                            return Some(Err(reason));
                                        }
                                        ServerToCli::VersionUnsupported {
                                            client_version,
                                            min_version,
                                        } => {
                                            return Some(Err(format!(
                                                "Version {} not supported, need {}",
                                                client_version, min_version
                                            )));
                                        }
                                        _ => continue,
                                    }
                                }
                                Message::Ping(_) => {
                                    return Some(Err("ping:0".to_string()));
                                }
                                _ => continue,
                            }
                        }
                        None
                    })
                    .await;

                match registration_timeout {
                    Ok(Some(Ok(cli_id))) => {
                        let _ = status_tx.send(PaneOutput {
                            text: format!("[Server: Connected ({})]", &cli_id.to_string()[..8]),
                            pane_id: shared::PANE_ID_DEADLOOP,
                        });
                    }
                    Ok(Some(Err(reason))) if reason.starts_with("ping:") => {
                        let _ = status_tx.send(PaneOutput {
                            text: "[Server: Received ping during registration, reconnecting...]"
                                .to_string(),
                            pane_id: shared::PANE_ID_DEADLOOP,
                        });
                        tokio::time::sleep(reconnect_delay).await;
                        continue;
                    }
                    Ok(Some(Err(reason))) => {
                        let _ = status_tx.send(PaneOutput {
                            text: format!("[Server: Registration failed - {}]", reason),
                            pane_id: shared::PANE_ID_DEADLOOP,
                        });
                        return Err(anyhow::anyhow!("Registration failed: {}", reason));
                    }
                    Ok(None) | Err(_) => {
                        let _ = status_tx.send(PaneOutput {
                            text: "[Server: Registration timeout or connection lost]".to_string(),
                            pane_id: shared::PANE_ID_DEADLOOP,
                        });
                        tokio::time::sleep(reconnect_delay).await;
                        continue;
                    }
                }

                // Send session start with pane list
                let hostname = hostname::get().ok().and_then(|h| h.into_string().ok());
                let pane_list = build_pane_list(
                    &pane_metas,
                    &input_channels,
                    session_id,
                    &pane_sessions,
                    &pane_pauses,
                    &pane_stop_requests,
                );

                let session_start = CliToServer::SessionStart {
                    session_id,
                    // session_id is `metadata.id` from .apas, which is the project id.
                    project_id: Some(session_id),
                    machine_id,
                    working_dir: Some(working_dir.to_string()),
                    hostname,
                    git_remote: git_remote.clone(),
                    git_remote_url: git_remote_url.clone(),
                    pane_type: None,
                    panes: Some(pane_list.clone()),
                };
                let msg_text = match serde_json::to_string(&session_start) {
                    Ok(t) => t,
                    Err(e) => {
                        let _ = status_tx.send(PaneOutput {
                            text: format!("[Server: Failed to serialize session start - {}]", e),
                            pane_id: shared::PANE_ID_DEADLOOP,
                        });
                        tokio::time::sleep(reconnect_delay).await;
                        continue;
                    }
                };
                if ws_sender
                    .send(Message::Text(msg_text.into()))
                    .await
                    .is_err()
                {
                    let _ = status_tx.send(PaneOutput {
                        text: "[Server: Connection lost during session start]".to_string(),
                        pane_id: shared::PANE_ID_DEADLOOP,
                    });
                    tokio::time::sleep(reconnect_delay).await;
                    continue;
                }

                // One-time migration input. The server accepts the first
                // snapshot only when no authoritative override exists and
                // returns the effective policy separately.
                if let Ok(meta) = get_or_create_project(Path::new(working_dir)) {
                    let snapshot = CliToServer::ProjectPolicySnapshot {
                        session_id,
                        team_enabled: meta.team_enabled,
                        disallowed_tab_types: meta.disallowed_tab_types,
                    };
                    if let Ok(text) = serde_json::to_string(&snapshot) {
                        let _ = ws_sender.send(Message::Text(text.into())).await;
                    }
                }

                // Send PaneList separately too
                let pane_list_msg = CliToServer::PaneList {
                    session_id,
                    panes: pane_list,
                };
                let msg_text = serde_json::to_string(&pane_list_msg).unwrap_or_default();
                if ws_sender
                    .send(Message::Text(msg_text.into()))
                    .await
                    .is_err()
                {
                    let _ = status_tx.send(PaneOutput {
                        text: "[Server: Connection lost during pane reconciliation]".to_string(),
                        pane_id: shared::PANE_ID_DEADLOOP,
                    });
                    tokio::time::sleep(reconnect_delay).await;
                    continue;
                }
                server_connection_generation.fetch_add(1, Ordering::SeqCst);

                // Reconcile pty generations before draining output queued
                // during the outage. Same-instance reports preserve the
                // server snapshot; a replacement clears it before seq 0.
                let mut terminal_states_sent = true;
                for state in terminal_state_reports(session_id, &pane_metas, &terminal_panes) {
                    let Ok(text) = serde_json::to_string(&state) else {
                        tracing::warn!("Failed to serialize terminal state report");
                        continue;
                    };
                    if ws_sender.send(Message::Text(text.into())).await.is_err() {
                        terminal_states_sent = false;
                        break;
                    }
                }
                if !terminal_states_sent {
                    let _ = status_tx.send(PaneOutput {
                        text: "[Server: Connection lost during terminal reconciliation]"
                            .to_string(),
                        pane_id: shared::PANE_ID_DEADLOOP,
                    });
                    tokio::time::sleep(reconnect_delay).await;
                    continue;
                }

                let lifecycle_inventory = build_lifecycle_inventory(&pane_metas, &terminal_panes);
                let inventory_message = CliToServer::CliLifecycleInventory {
                    session_id,
                    inventory: lifecycle_inventory.clone(),
                };
                if let Ok(text) = serde_json::to_string(&inventory_message) {
                    if ws_sender.send(Message::Text(text.into())).await.is_err() {
                        continue;
                    }
                }

                if let Some((marker_path, marker)) = reboot_handoff.take() {
                    let live_adopted = lifecycle_inventory
                        .panes
                        .iter()
                        .filter(|pane| pane.mode == shared::PanePreservationMode::LiveAdoptable)
                        .count();
                    let restarted = lifecycle_inventory
                        .panes
                        .iter()
                        .filter(|pane| {
                            pane.mode == shared::PanePreservationMode::RestartRequiredOnCliReboot
                        })
                        .count();
                    let completed = CliToServer::CliLifecycleStatus {
                        session_id,
                        request_id: marker.request_id,
                        operation: shared::CliLifecycleOperation::RebootCli,
                        phase: shared::CliLifecyclePhase::Succeeded,
                        message: Some(format!(
                            "CLI replacement reconciled: {live_adopted} terminal pane(s) live-adopted, {restarted} restarted or unavailable"
                        )),
                        inventory: Some(lifecycle_inventory.clone()),
                    };
                    if let Ok(text) = serde_json::to_string(&completed) {
                        if ws_sender.send(Message::Text(text.into())).await.is_err() {
                            reboot_handoff = Some((marker_path, marker));
                            continue;
                        }
                    }
                    let _ = std::fs::remove_file(&marker_path);
                    tracing::info!(request_id = %marker.request_id, live_adopted, restarted, "CLI reboot handoff completed");
                }

                let mut heartbeat_interval =
                    tokio::time::interval(std::time::Duration::from_secs(25));
                heartbeat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                heartbeat_interval.tick().await;

                let mut usage_interval = tokio::time::interval(USAGE_FETCH_INTERVAL);
                usage_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                let mut reboot_preparation: Option<(
                    Uuid,
                    std::time::Instant,
                    tokio::task::JoinHandle<anyhow::Result<std::path::PathBuf>>,
                )> = None;

                // Main loop
                loop {
                    tokio::select! {
                        prepared = async {
                            match reboot_preparation.as_mut() {
                                Some((_, _, task)) => Some(task.await),
                                None => std::future::pending().await,
                            }
                        } => {
                            let (request_id, preparation_started, _) = reboot_preparation
                                .take()
                                .expect("completed reboot preparation was present");
                            let prepared = prepared
                                .expect("reboot preparation result")
                                .map_err(anyhow::Error::from)
                                .and_then(|result| result);
                            match prepared {
                                Ok(executable) => {
                                    tracing::info!(
                                        %request_id,
                                        preparation_duration_ms = preparation_started.elapsed().as_millis(),
                                        "CLI reboot preparation completed"
                                    );
                                    save_pane_configs(
                                        working_dir,
                                        &pane_sessions,
                                        &pane_metas,
                                        &pane_pauses,
                                        &pane_stop_requests,
                                    );
                                    let inventory = build_lifecycle_inventory(&pane_metas, &terminal_panes);
                                    let runtime_ids = inventory
                                        .panes
                                        .iter()
                                        .filter_map(|pane| pane.runtime_id)
                                        .collect::<Vec<_>>();
                                    let marker_path = crate::pane_host::handoff_marker_path(session_id)?;
                                    let deadline_unix_ms = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis()
                                        .saturating_add(15 * 60 * 1000)
                                        .try_into()
                                        .unwrap_or(u64::MAX);
                                    let marker = crate::pane_host::RebootHandoffMarker {
                                        protocol_version: crate::pane_host::HOST_PROTOCOL_VERSION,
                                        request_id,
                                        project_id: session_id,
                                        expected_version: crate::update::binary_version(&executable)
                                            .unwrap_or_else(|| env!("APAS_VERSION").to_string()),
                                        expected_executable: executable,
                                        controller_generation: 1,
                                        runtime_ids,
                                        deadline_unix_ms,
                                    };
                                    if let Err(error) = crate::pane_host::write_handoff_marker(&marker_path, &marker) {
                                        let failed = CliToServer::CliLifecycleStatus {
                                            session_id,
                                            request_id,
                                            operation: shared::CliLifecycleOperation::RebootCli,
                                            phase: shared::CliLifecyclePhase::Failed,
                                            message: Some(format!("Could not write secure reboot handoff marker: {error}")),
                                            inventory: Some(inventory),
                                        };
                                        if let Ok(text) = serde_json::to_string(&failed) {
                                            let _ = ws_sender.send(Message::Text(text.into())).await;
                                        }
                                        continue;
                                    }

                                    let detach_failures = terminal_panes
                                        .lock()
                                        .map(|panes| {
                                            panes
                                                .iter()
                                                .filter_map(|(pane_id, handle)| {
                                                    handle
                                                        .detach_for_reboot()
                                                        .err()
                                                        .map(|error| (*pane_id, error.to_string()))
                                                })
                                                .collect::<Vec<_>>()
                                        })
                                        .unwrap_or_default();
                                    let handoff = CliToServer::CliLifecycleStatus {
                                        session_id,
                                        request_id,
                                        operation: shared::CliLifecycleOperation::RebootCli,
                                        phase: shared::CliLifecyclePhase::Handoff,
                                        message: Some(if detach_failures.is_empty() {
                                            "Update prepared; replacing CLI and adopting terminal runtimes".to_string()
                                        } else {
                                            format!(
                                                "Update prepared; replacing CLI with {} pane detach warning(s)",
                                                detach_failures.len()
                                            )
                                        }),
                                        inventory: Some(inventory),
                                    };
                                    if let Ok(text) = serde_json::to_string(&handoff) {
                                        let _ = ws_sender.send(Message::Text(text.into())).await;
                                        let _ = ws_sender.flush().await;
                                    }
                                    store_reboot_handoff(session_id, &marker_path);
                                    std::env::set_var("APAS_REBOOT_HANDOFF", &marker_path);
                                    tracing::info!(%request_id, marker = %marker_path.display(), detach_failures = detach_failures.len(), "reboot handoff ready");
                                    reboot_requested.store(true, Ordering::SeqCst);
                                    shutdown.store(true, Ordering::SeqCst);
                                    return Ok(());
                                }
                                Err(error) => {
                                    let failed = CliToServer::CliLifecycleStatus {
                                        session_id,
                                        request_id,
                                        operation: shared::CliLifecycleOperation::RebootCli,
                                        phase: shared::CliLifecyclePhase::Failed,
                                        message: Some(format!("CLI reboot preparation failed; current CLI and panes remain running: {error}")),
                                        inventory: Some(build_lifecycle_inventory(&pane_metas, &terminal_panes)),
                                    };
                                    if let Ok(text) = serde_json::to_string(&failed) {
                                        let _ = ws_sender.send(Message::Text(text.into())).await;
                                    }
                                    tracing::warn!(
                                        %request_id,
                                        %error,
                                        preparation_duration_ms = preparation_started.elapsed().as_millis(),
                                        "CLI reboot preparation failed without detaching panes"
                                    );
                                }
                            }
                        }
                        Some(msg) = output_rx.recv() => {
                            let msg_text = match serde_json::to_string(&msg) {
                                Ok(t) => t,
                                Err(e) => {
                                    tracing::warn!("Failed to serialize message: {}", e);
                                    continue;
                                }
                            };
                            if ws_sender.send(Message::Text(msg_text.into())).await.is_err() {
                                let _ = status_tx.send(PaneOutput {
                                    text: "[Server: Connection lost, reconnecting...]".to_string(),
                                    pane_id: shared::PANE_ID_DEADLOOP,
                                });
                                break;
                            }
                        }
                        Some(result) = summary_result_rx.recv() => {
                            let message = CliToServer::PaneWorkSummaryResult { result };
                            let Ok(text) = serde_json::to_string(&message) else {
                                tracing::warn!("Failed to serialize pane summary result");
                                continue;
                            };
                            if ws_sender.send(Message::Text(text.into())).await.is_err() {
                                break;
                            }
                        }
                        msg = ws_receiver.next() => {
                            match msg {
                                Some(Ok(Message::Text(text))) => {
                                    if let Ok(server_msg) = serde_json::from_str::<ServerToCli>(&text) {
                                        match server_msg {
                                            ServerToCli::GeneratePaneWorkSummary { job } => {
                                                if job.session_id != session_id {
                                                    let result = shared::PaneWorkSummaryGenerationResult {
                                                        protocol_version: shared::PANE_WORK_SUMMARY_PROTOCOL_VERSION,
                                                        job_id: job.job_id,
                                                        session_id: job.session_id,
                                                        pane_id: job.pane_id,
                                                        window_start: job.window_start,
                                                        source_digest: job.source_digest,
                                                        stage: job.stage,
                                                        chunk_index: job.chunk_index,
                                                        kind: shared::PaneWorkSummaryResultKind::PermanentFailure,
                                                        output: None,
                                                        error: Some("Summary job targets a different session".to_string()),
                                                        provider: None,
                                                        model: None,
                                                    };
                                                    let _ = summary_result_tx.try_send(result);
                                                    continue;
                                                }
                                                if let Some(runner) = summary_runner.clone() {
                                                    let result_tx = summary_result_tx.clone();
                                                    tokio::spawn(async move {
                                                        let result = runner.run(job).await;
                                                        let _ = result_tx.send(result).await;
                                                    });
                                                } else {
                                                    let result = shared::PaneWorkSummaryGenerationResult {
                                                        protocol_version: shared::PANE_WORK_SUMMARY_PROTOCOL_VERSION,
                                                        job_id: job.job_id,
                                                        session_id: job.session_id,
                                                        pane_id: job.pane_id,
                                                        window_start: job.window_start,
                                                        source_digest: job.source_digest,
                                                        stage: job.stage,
                                                        chunk_index: job.chunk_index,
                                                        kind: shared::PaneWorkSummaryResultKind::Unavailable,
                                                        output: None,
                                                        error: Some("No validated summary adapter is enabled".to_string()),
                                                        provider: None,
                                                        model: None,
                                                    };
                                                    let _ = summary_result_tx.try_send(result);
                                                }
                                            }
                                            ServerToCli::ProjectPolicy {
                                                session_id: policy_session_id,
                                                policy,
                                            } => {
                                                if policy_session_id != session_id {
                                                    continue;
                                                }
                                                if let Ok(mut current) = project_policy.lock() {
                                                    *current = Some(policy.clone());
                                                }
                                                let retired = stop_retired_panes(
                                                    &pane_metas,
                                                    &pane_pauses,
                                                    &input_channels,
                                                    &terminal_panes,
                                                );
                                                if !retired.is_empty() {
                                                    save_pane_configs(
                                                        working_dir,
                                                        &pane_sessions,
                                                        &pane_metas,
                                                        &pane_pauses,
                                                        &pane_stop_requests,
                                                    );
                                                    let _ = status_tx.send(PaneOutput {
                                                        text: format!(
                                                            "[Stopped unsupported retired-provider panes {:?}; supported panes were left running]",
                                                            retired,
                                                        ),
                                                        pane_id: 0,
                                                    });
                                                }
                                                // Deliberately not announced.
                                                // The message existed to
                                                // explain a restriction that no
                                                // longer applies: these panes
                                                // run and relaunch, and all
                                                // that is true of them is that
                                                // their profile could not be
                                                // chosen for something new. The
                                                // ids still travel to the web,
                                                // which shows them where the
                                                // profiles are listed.
                                            }
                                            ServerToCli::SessionRejected { session_id: rejected_id, reason } => {
                                                tracing::error!(
                                                    "Server rejected session {}: {}",
                                                    rejected_id, reason
                                                );
                                                // Stop this project, not the process. This used to
                                                // be `process::exit(2)`, which was survivable only
                                                // while a process hosted exactly one project; with
                                                // several sharing one, a single rejected session
                                                // would take down every other project on the host.
                                                if let Ok(mut slot) = stop_reason.lock() {
                                                    *slot = Some(format!(
                                                        "server rejected session {rejected_id}: {reason}"
                                                    ));
                                                }
                                                shutdown.store(true, Ordering::SeqCst);
                                                return Ok(());
                                            }
                                            ServerToCli::TerminalInput { session_id: _, pane_id, data_b64 } => {
                                                // Keystrokes straight through to the pty. No
                                                // auto-cancel / question bookkeeping like the agent
                                                // path below — a TUI owns its own input state.
                                                let handle = terminal_panes
                                                    .lock()
                                                    .ok()
                                                    .and_then(|m| m.get(&pane_id).cloned());
                                                match handle {
                                                    Some(handle) => {
                                                        match base64::engine::general_purpose::STANDARD.decode(&data_b64) {
                                                            Ok(bytes) => {
                                                                if let Err(e) = handle.write_bytes(&bytes) {
                                                                    tracing::warn!(pane_id, error = %e, "terminal write failed");
                                                                }
                                                            }
                                                            Err(e) => {
                                                                tracing::warn!(pane_id, error = %e, "terminal input was not valid base64");
                                                            }
                                                        }
                                                    }
                                                    None => {
                                                        tracing::debug!(pane_id, "terminal input for unknown pane");
                                                    }
                                                }
                                            }
                                            ServerToCli::TerminalResize { session_id: _, pane_id, cols, rows } => {
                                                let handle = terminal_panes
                                                    .lock()
                                                    .ok()
                                                    .and_then(|m| m.get(&pane_id).cloned());
                                                if let Some(handle) = handle {
                                                    if let Err(e) = handle.resize(cols, rows) {
                                                        tracing::warn!(pane_id, error = %e, "terminal resize failed");
                                                    }
                                                }
                                            }
                                            ServerToCli::Input { session_id: _, data, pane_id } => {
                                                // Route to the correct pane (from_tui=false: web-originated)
                                                let target_pane = pane_id.unwrap_or(shared::PANE_ID_INTERACTIVE);

                                                // If this pane is parked on AskUserQuestion(s),
                                                // auto-cancel them so claude can process the new
                                                // user prompt — otherwise the typed message sits
                                                // in the input queue while claude stays blocked
                                                // on the canUseTool callback waiting on an answer
                                                // the user has chosen not to give.
                                                let cancelled = auto_cancel_pending_questions_for_new_input(
                                                    &pane_metas,
                                                    target_pane,
                                                );
                                                for cancelled in cancelled {
                                                    let _ = status_tx.send(PaneOutput {
                                                        text: ASK_USER_QUESTION_AUTO_CANCEL_STATUS.to_string(),
                                                        pane_id: target_pane,
                                                    });
                                                    tracing::debug!(
                                                        pane_id = target_pane,
                                                        request_id = cancelled.request_id.as_str(),
                                                        tool_use_id = cancelled.tool_use_id.as_str(),
                                                        "Reported AskUserQuestion auto-cancel status",
                                                    );
                                                }

                                                match route_web_input_to_pane(
                                                    &input_channels,
                                                    target_pane,
                                                    &data,
                                                ) {
                                                    PaneInputRouteResult::Sent => {
                                                        continue;
                                                    }
                                                    PaneInputRouteResult::Disconnected => {
                                                        tracing::warn!(
                                                            pane_id = target_pane,
                                                            "Input channel disconnected for pane"
                                                        );
                                                        let _ = status_tx.send(PaneOutput {
                                                            text: "[Pane input channel disconnected. Restarting pane worker...]".to_string(),
                                                            pane_id: target_pane,
                                                        });
                                                        continue;
                                                    }
                                                    PaneInputRouteResult::MissingChannel => {}
                                                }

                                                // If web explicitly targeted a pane and it is missing, do not silently
                                                // fallback to another pane. Surface the issue and try to recreate it.
                                                if pane_id.is_some() {
                                                    let pane_meta = {
                                                        let metas = pane_metas.lock().unwrap();
                                                        metas.get(&target_pane).cloned()
                                                    };
                                                    let pane_session_id = {
                                                        let sessions = pane_sessions.lock().unwrap();
                                                        sessions.get(&target_pane).copied()
                                                    };

                                                    let mut buffered_input = Some(data.clone());
                                                    let mut replayed = false;
                                                    if let (Some(meta), Some(claude_session_id)) =
                                                        (pane_meta, pane_session_id)
                                                    {
                                                        tracing::warn!(
                                                            pane_id = target_pane,
                                                            "Missing input channel for pane; requesting pane worker recreation"
                                                        );
                                                        // Only replay automatically for interactive panes —
                                                        // the AddTabWithConfig handler's deadloop branch
                                                        // can't route initial_input anywhere useful.
                                                        let is_interactive = !matches!(meta.mode, shared::PaneMode::Deadloop);
                                                        let queued = if is_interactive {
                                                            replayed = true;
                                                            buffered_input.take()
                                                        } else {
                                                            None
                                                        };
                                                        let _ = tui_event_tx.send(TuiEvent::AddTabWithConfig {
                                                            pane_id: target_pane,
                                                            label: meta.label,
                                                            claude_session_id,
                                                            mode: meta.mode,
                                                            provider: meta.provider,
                                                            prompt: meta.prompt,
                                                            min_iteration_interval_minutes: meta
                                                                .min_iteration_interval_minutes,
                                                            model: meta.model,
                                                            effort: meta.effort,
                                                            worktree_path: meta.worktree_path,
                                                            initial_input: queued,
                                                            role: meta.role,
                                                            goal: meta.goal,
                                                            backstory: meta.backstory,
                                                            plan_review_mode: meta.plan_review_mode,
                                                            managed: meta.managed,
                                                            try_resume_first: true,
                                                            kind: meta.kind,
                                                        });
                                                    } else {
                                                        tracing::warn!(
                                                            pane_id = target_pane,
                                                            "Missing input channel for pane and no pane metadata found"
                                                        );
                                                    }
                                                    let _ = buffered_input;

                                                    let unavailable_status = if replayed {
                                                        format!(
                                                            "[Pane {} worker is restarting; your message will be sent automatically when it's ready.]",
                                                            target_pane,
                                                        )
                                                    } else {
                                                        format!(
                                                            "[Pane {} is unavailable. Restarting pane worker; please resend your message.]",
                                                            target_pane,
                                                        )
                                                    };
                                                    let _ = status_tx.send(PaneOutput {
                                                        text: unavailable_status.clone(),
                                                        pane_id: target_pane,
                                                    });
                                                    let _ = status_tx.send(PaneOutput {
                                                        text: unavailable_status.clone(),
                                                        pane_id: shared::PANE_ID_DEADLOOP,
                                                    });

                                                    let pane_status_msg = CliToServer::PaneStatus {
                                                        session_id,
                                                        pane_type: shared::PaneType::Interactive,
                                                        pane_id: Some(target_pane),
                                                        status: Some(
                                                            if replayed {
                                                                "Pane worker restarting; replaying your input…".to_string()
                                                            } else {
                                                                "Pane worker unavailable; restart requested. Please resend.".to_string()
                                                            },
                                                        ),
                                                    };
                                                    if let Ok(msg_text) =
                                                        serde_json::to_string(&pane_status_msg)
                                                    {
                                                        let _ = ws_sender
                                                            .send(Message::Text(msg_text.into()))
                                                            .await;
                                                    }
                                                    continue;
                                                }

                                                // Legacy input without explicit pane id: best-effort fallback to
                                                // interactive pane.
                                                let fallback_tx = {
                                                    let channels = input_channels.lock().unwrap();
                                                    channels
                                                        .get(&shared::PANE_ID_INTERACTIVE)
                                                        .cloned()
                                                };
                                                if let Some(tx) = fallback_tx {
                                                    if tx.send((data, false)).is_err() {
                                                        tracing::warn!(
                                                            "Fallback interactive pane input channel disconnected"
                                                        );
                                                    }
                                                } else {
                                                    tracing::warn!(
                                                        "No interactive pane channel available for legacy input routing"
                                                    );
                                                }
                                            }
                                            ServerToCli::Heartbeat => {}
                                            ServerToCli::PauseDeadloop { .. } => {
                                                // Legacy: pause the default deadloop pane
                                                if let Ok(pauses) = pane_pauses.lock() {
                                                    if let Some(flag) = pauses.get(&shared::PANE_ID_DEADLOOP) {
                                                        flag.store(true, Ordering::SeqCst);
                                                    }
                                                }
                                                let _ = status_tx.send(PaneOutput {
                                                    text: "[Pause command received from web]".to_string(),
                                                    pane_id: shared::PANE_ID_DEADLOOP,
                                                });
                                                if let Ok(mut metadata) = get_or_create_project(std::path::Path::new(working_dir)) {
                                                    metadata.is_paused = true;
                                                    let _ = save_project(std::path::Path::new(working_dir), &metadata);
                                                }
                                                let status_msg = CliToServer::DeadloopStatus { session_id, is_paused: true };
                                                let msg_text = serde_json::to_string(&status_msg).unwrap_or_default();
                                                let _ = ws_sender.send(Message::Text(msg_text.into())).await;
                                                let pane_msg = CliToServer::PanePaused { session_id, pane_id: shared::PANE_ID_DEADLOOP, is_paused: true };
                                                let msg_text = serde_json::to_string(&pane_msg).unwrap_or_default();
                                                let _ = ws_sender.send(Message::Text(msg_text.into())).await;
                                            }
                                            ServerToCli::ResumeDeadloop { .. } => {
                                                // Legacy: resume the default deadloop pane
                                                let retired = pane_metas
                                                    .lock()
                                                    .ok()
                                                    .and_then(|panes| panes.get(&shared::PANE_ID_DEADLOOP).cloned())
                                                    .is_some_and(|pane| shared::is_retired_launch(
                                                        pane.provider,
                                                        pane.model.as_deref(),
                                                    ));
                                                if retired {
                                                    report_retired_launch_rejection(
                                                        &status_tx,
                                                        &mut ws_sender,
                                                        session_id,
                                                        shared::PANE_ID_DEADLOOP,
                                                        "[Resume refused — unsupported provider has been retired]",
                                                    )
                                                    .await;
                                                    continue;
                                                }
                                                if let Ok(pauses) = pane_pauses.lock() {
                                                    if let Some(flag) = pauses.get(&shared::PANE_ID_DEADLOOP) {
                                                        flag.store(false, Ordering::SeqCst);
                                                    }
                                                }
                                                let _ = status_tx.send(PaneOutput {
                                                    text: "[Resume command received from web]".to_string(),
                                                    pane_id: shared::PANE_ID_DEADLOOP,
                                                });
                                                if let Ok(mut metadata) = get_or_create_project(std::path::Path::new(working_dir)) {
                                                    metadata.is_paused = false;
                                                    let _ = save_project(std::path::Path::new(working_dir), &metadata);
                                                }
                                                let status_msg = CliToServer::DeadloopStatus { session_id, is_paused: false };
                                                let msg_text = serde_json::to_string(&status_msg).unwrap_or_default();
                                                let _ = ws_sender.send(Message::Text(msg_text.into())).await;
                                                let pane_msg = CliToServer::PanePaused { session_id, pane_id: shared::PANE_ID_DEADLOOP, is_paused: false };
                                                let msg_text = serde_json::to_string(&pane_msg).unwrap_or_default();
                                                let _ = ws_sender.send(Message::Text(msg_text.into())).await;
                                            }
                                            ServerToCli::PausePane { session_id: _, pane_id: target_pane } => {
                                                if let Ok(pauses) = pane_pauses.lock() {
                                                    if let Some(flag) = pauses.get(&target_pane) {
                                                        flag.store(true, Ordering::SeqCst);
                                                    }
                                                }
                                                let _ = status_tx.send(PaneOutput {
                                                    text: format!("[Pane {} paused from web]", target_pane),
                                                    pane_id: target_pane,
                                                });
                                                // Persist so a CLI reboot restores this pane as paused
                                                // — without this, managed workers would respawn on boot.
                                                save_pane_configs(
                                                    working_dir,
                                                    &pane_sessions,
                                                    &pane_metas,
                                                    &pane_pauses,
                                                    &pane_stop_requests,
                                                );
                                                let pane_msg = CliToServer::PanePaused { session_id, pane_id: target_pane, is_paused: true };
                                                let msg_text = serde_json::to_string(&pane_msg).unwrap_or_default();
                                                let _ = ws_sender.send(Message::Text(msg_text.into())).await;
                                                // Legacy compat: also send DeadloopStatus for pane 1 and
                                                // keep the legacy `metadata.is_paused` flag in sync since
                                                // the boot path ORs it with `pane.is_paused` for pane 1.
                                                if target_pane == shared::PANE_ID_DEADLOOP {
                                                    if let Ok(mut metadata) = get_or_create_project(std::path::Path::new(working_dir)) {
                                                        if !metadata.is_paused {
                                                            metadata.is_paused = true;
                                                            let _ = save_project(std::path::Path::new(working_dir), &metadata);
                                                        }
                                                    }
                                                    let status_msg = CliToServer::DeadloopStatus { session_id, is_paused: true };
                                                    let msg_text = serde_json::to_string(&status_msg).unwrap_or_default();
                                                    let _ = ws_sender.send(Message::Text(msg_text.into())).await;
                                                }
                                            }
                                            ServerToCli::ResumePane { session_id: _, pane_id: target_pane } => {
                                                let retired = pane_metas
                                                    .lock()
                                                    .ok()
                                                    .and_then(|panes| panes.get(&target_pane).cloned())
                                                    .is_some_and(|pane| shared::is_retired_launch(
                                                        pane.provider,
                                                        pane.model.as_deref(),
                                                    ));
                                                if retired {
                                                    report_retired_launch_rejection(
                                                        &status_tx,
                                                        &mut ws_sender,
                                                        session_id,
                                                        target_pane,
                                                        "[Resume refused — unsupported provider has been retired]",
                                                    )
                                                    .await;
                                                    continue;
                                                }
                                                let allowed = pane_metas
                                                    .lock()
                                                    .ok()
                                                    .and_then(|panes| panes.get(&target_pane).cloned())
                                                    .is_some_and(|pane| {
                                                        existing_pane_relaunch_allowed()
                                                    });
                                                if !allowed {
                                                    let _ = status_tx.send(PaneOutput {
                                                        text: "[Resume refused by the current cluster policy]".to_string(),
                                                        pane_id: target_pane,
                                                    });
                                                    continue;
                                                }
                                                if let Ok(pauses) = pane_pauses.lock() {
                                                    if let Some(flag) = pauses.get(&target_pane) {
                                                        flag.store(false, Ordering::SeqCst);
                                                    }
                                                }
                                                let _ = status_tx.send(PaneOutput {
                                                    text: format!("[Pane {} resumed from web]", target_pane),
                                                    pane_id: target_pane,
                                                });
                                                save_pane_configs(
                                                    working_dir,
                                                    &pane_sessions,
                                                    &pane_metas,
                                                    &pane_pauses,
                                                    &pane_stop_requests,
                                                );
                                                let pane_msg = CliToServer::PanePaused { session_id, pane_id: target_pane, is_paused: false };
                                                let msg_text = serde_json::to_string(&pane_msg).unwrap_or_default();
                                                let _ = ws_sender.send(Message::Text(msg_text.into())).await;
                                                // Legacy compat: clear `metadata.is_paused` and emit
                                                // DeadloopStatus when pane 1 resumes.
                                                if target_pane == shared::PANE_ID_DEADLOOP {
                                                    if let Ok(mut metadata) = get_or_create_project(std::path::Path::new(working_dir)) {
                                                        if metadata.is_paused {
                                                            metadata.is_paused = false;
                                                            let _ = save_project(std::path::Path::new(working_dir), &metadata);
                                                        }
                                                    }
                                                    let status_msg = CliToServer::DeadloopStatus { session_id, is_paused: false };
                                                    let msg_text = serde_json::to_string(&status_msg).unwrap_or_default();
                                                    let _ = ws_sender.send(Message::Text(msg_text.into())).await;
                                                }
                                                // Resume a paused deadloop: if the pane is still in
                                                // Deadloop mode but its child_process has been
                                                // cleared (the supervisor's pause path reaps claude
                                                // and exits), spin a fresh worker via StartBot. With
                                                // no fields set, StartBot reuses the prompt / model /
                                                // effort / cadence saved in pane_metas, and the
                                                // existing claude_session_id keeps context.
                                                let needs_restart = {
                                                    if let Ok(metas) = pane_metas.lock() {
                                                        if let Some(meta) = metas.get(&target_pane) {
                                                            meta.mode == shared::PaneMode::Deadloop
                                                                && meta
                                                                    .child_process
                                                                    .lock()
                                                                    .ok()
                                                                    .map(|g| g.is_none())
                                                                    .unwrap_or(false)
                                                        } else {
                                                            false
                                                        }
                                                    } else {
                                                        false
                                                    }
                                                };
                                                if needs_restart {
                                                    let _ = tui_event_tx.send(TuiEvent::StartBot {
                                                        pane_id: target_pane,
                                                        prompt: None,
                                                        min_iteration_interval_minutes: None,
                                                        effort: None,
                                                    });
                                                }
                                            }
                                            ServerToCli::AddPane { session_id: _, pane_config, isolated_worktree, initial_input } => {
                                                if shared::is_retired_launch(
                                                    pane_config.provider,
                                                    pane_config.model.as_deref(),
                                                ) {
                                                    report_retired_launch_rejection(
                                                        &status_tx,
                                                        &mut ws_sender,
                                                        session_id,
                                                        pane_config.pane_id,
                                                        "[New pane refused — unsupported provider has been retired]",
                                                    )
                                                    .await;
                                                    continue;
                                                }
                                                if pane_config.kind == shared::PaneKind::Agent {
                                                    report_retired_launch_rejection(
                                                        &status_tx,
                                                        &mut ws_sender,
                                                        session_id,
                                                        pane_config.pane_id,
                                                        "[New pane refused — conversation-only panes are retired; use a Claude, Codex, or OpenCode terminal pane]",
                                                    )
                                                    .await;
                                                    continue;
                                                }
                                                if pane_metas
                                                    .lock()
                                                    .is_ok_and(|metas| metas.contains_key(&pane_config.pane_id))
                                                {
                                                    tracing::info!(
                                                        pane_id = pane_config.pane_id,
                                                        "Ignoring idempotent duplicate AddPane",
                                                    );
                                                    continue;
                                                }
                                                let label = pane_config.label.clone().unwrap_or_else(|| format!("Tab {}", pane_config.pane_id));
                                                // Phase 1.1e: if the web asked for an isolated worktree, create
                                                // it now (synchronously) and surface any error back to the user
                                                // as a chat message rather than silently dropping the request.
                                                let send_status = |text: String| {
                                                    // Local TUI surface
                                                    let _ = status_tx.send(PaneOutput {
                                                        text: text.clone(),
                                                        pane_id: pane_config.pane_id,
                                                    });
                                                    // Web surface — same plumbing as PaneStatus uses below
                                                    let msg = CliToServer::Output {
                                                        session_id,
                                                        data: text,
                                                        output_type: shared::OutputType::System,
                                                        pane_type: None,
                                                        pane_id: Some(pane_config.pane_id),
                                                    };
                                                    serde_json::to_string(&msg).ok()
                                                };
                                                let worktree_path: Option<String> = if isolated_worktree {
                                                    match crate::worktree::create_for_pane(
                                                        std::path::Path::new(&working_dir),
                                                        pane_config.pane_id,
                                                        None,
                                                        None,
                                                    ) {
                                                        Ok(path) => {
                                                            if let Some(msg_text) = send_status(format!(
                                                                "[Created isolated worktree at {} (branch apas-pane-{})]",
                                                                path, pane_config.pane_id,
                                                            )) {
                                                                let _ = ws_sender
                                                                    .send(Message::Text(msg_text.into()))
                                                                    .await;
                                                            }
                                                            Some(path)
                                                        }
                                                        Err(err) => {
                                                            if let Some(msg_text) = send_status(format!(
                                                                "[Could not create isolated worktree for new pane (falling back to shared cwd): {}]",
                                                                err,
                                                            )) {
                                                                let _ = ws_sender
                                                                    .send(Message::Text(msg_text.into()))
                                                                    .await;
                                                            }
                                                            None
                                                        }
                                                    }
                                                } else {
                                                    pane_config.worktree_path.clone()
                                                };
                                                // Delegate to TUI event handler which has server_tx and can spawn the session thread
                                                let _ = tui_event_tx.send(TuiEvent::AddTabWithConfig {
                                                    pane_id: pane_config.pane_id,
                                                    label,
                                                    claude_session_id: pane_config.session_id,
                                                    mode: pane_config.mode,
                                                    provider: pane_config.provider,
                                                    prompt: pane_config.prompt,
                                                    min_iteration_interval_minutes: pane_config.min_iteration_interval_minutes,
                                                    model: pane_config.model,
                                                    effort: pane_config.effort,
                                                    worktree_path,
                                                    initial_input,
                                                    role: pane_config.role,
                                                    goal: pane_config.goal,
                                                    backstory: pane_config.backstory,
                                                    plan_review_mode: pane_config.plan_review_mode,
                                                    managed: pane_config.managed,
                                                    // AddPane always names a brand-new pane/session.
                                                    // Restores and reboots have separate paths and
                                                    // explicitly opt into provider resume.
                                                    try_resume_first: false,
                                                    kind: pane_config.kind,
                                                });
                                            }
                                            ServerToCli::RebootPane { session_id: _, pane_id: target } => {
                                                // Snapshot meta + the pane's claude session id BEFORE
                                                // the close eats them, then close + re-add the pane
                                                // with the SAME session id and try_resume_first=true
                                                // so the agent resumes its prior conversation. We
                                                // don't reset context — that's deliberate; the user
                                                // expects "Reboot" to recover a wedged worker, not
                                                // erase what it knows.
                                                let snapshot = {
                                                    let metas = pane_metas.lock().unwrap();
                                                    metas.get(&target).cloned()
                                                };
                                                let Some(meta) = snapshot else {
                                                    tracing::warn!(
                                                        pane_id = target,
                                                        "RebootPane: pane not found",
                                                    );
                                                    let _ = status_tx.send(PaneOutput {
                                                        text: format!("[Reboot: pane {} not found]", target),
                                                        pane_id: target,
                                                    });
                                                    continue;
                                                };
                                                if shared::is_retired_launch(
                                                    meta.provider,
                                                    meta.model.as_deref(),
                                                ) {
                                                    report_retired_launch_rejection(
                                                        &status_tx,
                                                        &mut ws_sender,
                                                        session_id,
                                                        target,
                                                        "[Reboot refused — unsupported provider has been retired]",
                                                    )
                                                    .await;
                                                    continue;
                                                }
                                                if !existing_pane_relaunch_allowed() {
                                                    let _ = status_tx.send(PaneOutput {
                                                        text: "[Reboot refused by the current cluster policy]".to_string(),
                                                        pane_id: target,
                                                    });
                                                    continue;
                                                }
                                                let prior_session_id = {
                                                    let sessions = pane_sessions.lock().unwrap();
                                                    sessions.get(&target).copied()
                                                };
                                                tracing::info!(
                                                    pane_id = target,
                                                    label = %meta.label,
                                                    mode = ?meta.mode,
                                                    ?prior_session_id,
                                                    "RebootPane: begin",
                                                );
                                                let (close_event, add_event) =
                                                    build_pane_reboot_events(target, &meta, prior_session_id);

                                                if let Err(err) = tui_event_tx.send(close_event) {
                                                    tracing::warn!(pane_id = target, %err, "RebootPane: CloseTab send failed");
                                                }
                                                // Clone the add_event so the defensive-retry loop
                                                // below can re-send it if the first delivery is
                                                // silently dropped between CloseTab and processing.
                                                // Cheap — AddTabWithConfig is a plain data enum.
                                                let add_event_retry = add_event.clone();
                                                if let Err(err) = tui_event_tx.send(add_event) {
                                                    tracing::warn!(pane_id = target, %err, "RebootPane: AddTabWithConfig send failed");
                                                }
                                                let _ = status_tx.send(PaneOutput {
                                                    text: "[Pane rebooted — agent restarted on the same session]".to_string(),
                                                    pane_id: target,
                                                });
                                                tracing::info!(
                                                    pane_id = target,
                                                    "RebootPane: events dispatched, verifying respawn",
                                                );

                                                // Defensive verify-and-retry: user-reported bug on
                                                // mako Claude-6 had the pane disappearing entirely
                                                // after Reboot. Root cause was hard to reproduce —
                                                // guard against it by polling pane_metas for up to
                                                // ~2s; if the tab is still missing after the tui
                                                // event loop should have consumed both events, log
                                                // the failure and re-emit AddTabWithConfig so the
                                                // user isn't stranded staring at a vanished pane.
                                                let pane_metas_verify = pane_metas.clone();
                                                let tui_event_tx_verify = tui_event_tx.clone();
                                                let status_tx_verify = status_tx.clone();
                                                tokio::task::spawn_blocking(move || {
                                                    for attempt in 0..10 {
                                                        std::thread::sleep(Duration::from_millis(200));
                                                        let present = pane_metas_verify
                                                            .lock()
                                                            .map(|g| g.contains_key(&target))
                                                            .unwrap_or(false);
                                                        if present {
                                                            if attempt > 0 {
                                                                tracing::info!(
                                                                    pane_id = target,
                                                                    attempts = attempt + 1,
                                                                    "RebootPane: pane respawned after retry",
                                                                );
                                                            }
                                                            return;
                                                        }
                                                    }
                                                    tracing::warn!(
                                                        pane_id = target,
                                                        "RebootPane: pane still missing 2s after events dispatched; re-emitting AddTabWithConfig",
                                                    );
                                                    if let Err(err) = tui_event_tx_verify.send(add_event_retry) {
                                                        tracing::error!(
                                                            pane_id = target,
                                                            %err,
                                                            "RebootPane: defensive re-emit failed",
                                                        );
                                                        let _ = status_tx_verify.send(PaneOutput {
                                                            text: format!(
                                                                "[Reboot: pane {} vanished and auto-recovery failed; add it back manually]",
                                                                target,
                                                            ),
                                                            pane_id: target,
                                                        });
                                                    } else {
                                                        let _ = status_tx_verify.send(PaneOutput {
                                                            text: format!(
                                                                "[Reboot: pane {} was lost between close and add; re-emitted respawn]",
                                                                target,
                                                            ),
                                                            pane_id: target,
                                                        });
                                                    }
                                                });
                                            }
                                            ServerToCli::RemovePane { session_id: _, pane_id: remove_id, cleanup_action } => {
                                                // Kill the pty first if this is a terminal pane.
                                                // Removing it from the registry drops the last
                                                // handle, and `shutdown` marks the reader so the
                                                // resulting EOF isn't reported as a crash.
                                                let removed_terminal = terminal_panes
                                                    .lock()
                                                    .ok()
                                                    .and_then(|mut m| m.remove(&remove_id));
                                                if let Some(handle) = removed_terminal {
                                                    tracing::info!(pane_id = remove_id, "shutting down terminal pane");
                                                    handle.shutdown();
                                                }
                                                // Delegate to TUI event handler
                                                let _ = tui_event_tx.send(TuiEvent::CloseTab {
                                                    pane_id: remove_id,
                                                    cleanup_action,
                                                });
                                            }
                                            ServerToCli::StartBot {
                                                session_id: _,
                                                pane_id: target_pane,
                                                prompt: bot_prompt,
                                                min_iteration_interval_minutes,
                                                effort,
                                            } => {
                                                let retired = pane_metas
                                                    .lock()
                                                    .ok()
                                                    .and_then(|panes| panes.get(&target_pane).cloned())
                                                    .is_some_and(|pane| shared::is_retired_launch(
                                                        pane.provider,
                                                        pane.model.as_deref(),
                                                    ));
                                                if retired {
                                                    report_retired_launch_rejection(
                                                        &status_tx,
                                                        &mut ws_sender,
                                                        session_id,
                                                        target_pane,
                                                        "[Start refused — unsupported provider has been retired]",
                                                    )
                                                    .await;
                                                    continue;
                                                }
                                                let _ = tui_event_tx.send(TuiEvent::StartBot {
                                                    pane_id: target_pane,
                                                    prompt: bot_prompt,
                                                    min_iteration_interval_minutes,
                                                    effort,
                                                });
                                            }
                                            ServerToCli::StopBot { session_id: _, pane_id: target_pane } => {
                                                let _ = tui_event_tx.send(TuiEvent::StopBot {
                                                    pane_id: target_pane,
                                                });
                                            }
                                            ServerToCli::RebootCli { .. } => {
                                                let retired = pane_metas
                                                    .lock()
                                                    .map(|panes| panes.values().any(|pane| {
                                                        shared::is_retired_launch(
                                                            pane.provider,
                                                            pane.model.as_deref(),
                                                        )
                                                    }))
                                                    .unwrap_or(false);
                                                if retired {
                                                    report_retired_launch_rejection(
                                                        &status_tx,
                                                        &mut ws_sender,
                                                        session_id,
                                                        0,
                                                        "[CLI reboot refused while a historical retired-provider pane is present]",
                                                    )
                                                    .await;
                                                    continue;
                                                }
                                                // Before tearing down the WS for a possibly
                                                // minutes-long update + rebuild, flag every pane
                                                // as "Rebooting…" so the web shows a live status
                                                // instead of a frozen, dead-looking pane. The
                                                // server caches pane status (a client attaching
                                                // mid-reboot still sees it) and pushes nothing on
                                                // disconnect, so it stays until the rebuilt CLI
                                                // reconnects and each pane clears it with
                                                // status:None when its next turn ends.
                                                let reboot_panes = build_pane_list(
                                                    &pane_metas,
                                                    &input_channels,
                                                    session_id,
                                                    &pane_sessions,
                                                    &pane_pauses,
                                                    &pane_stop_requests,
                                                );
                                                for pane in &reboot_panes {
                                                    let status_msg = serde_json::to_string(
                                                        &CliToServer::PaneStatus {
                                                            session_id,
                                                            pane_type: shared::PaneType::default(),
                                                            pane_id: Some(pane.pane_id),
                                                            status: Some("Rebooting APAS…".to_string()),
                                                        },
                                                    )
                                                    .unwrap_or_default();
                                                    let _ = ws_sender
                                                        .send(Message::Text(status_msg.into()))
                                                        .await;
                                                }
                                                reboot_requested.store(true, Ordering::SeqCst);
                                                shutdown.store(true, Ordering::SeqCst);
                                                return Ok(());
                                            }
                                            ServerToCli::CliLifecycleRequest {
                                                session_id: requested_session,
                                                request_id,
                                                operation: shared::CliLifecycleOperation::RebootCli,
                                            } => {
                                                if requested_session != session_id {
                                                    continue;
                                                }
                                                if reboot_preparation.is_some() {
                                                    let failed = CliToServer::CliLifecycleStatus {
                                                        session_id,
                                                        request_id,
                                                        operation: shared::CliLifecycleOperation::RebootCli,
                                                        phase: shared::CliLifecyclePhase::Failed,
                                                        message: Some("A CLI reboot is already being prepared".to_string()),
                                                        inventory: Some(build_lifecycle_inventory(&pane_metas, &terminal_panes)),
                                                    };
                                                    if let Ok(text) = serde_json::to_string(&failed) {
                                                        let _ = ws_sender.send(Message::Text(text.into())).await;
                                                    }
                                                    continue;
                                                }
                                                let preparing = CliToServer::CliLifecycleStatus {
                                                    session_id,
                                                    request_id,
                                                    operation: shared::CliLifecycleOperation::RebootCli,
                                                    phase: shared::CliLifecyclePhase::Preparing,
                                                    message: Some("Preparing CLI replacement".to_string()),
                                                    inventory: Some(build_lifecycle_inventory(&pane_metas, &terminal_panes)),
                                                };
                                                if let Ok(text) = serde_json::to_string(&preparing) {
                                                    let _ = ws_sender.send(Message::Text(text.into())).await;
                                                    let _ = ws_sender.flush().await;
                                                }
                                                reboot_preparation = Some((
                                                    request_id,
                                                    std::time::Instant::now(),
                                                    tokio::task::spawn_blocking(crate::update::prepare_cli_restart),
                                                ));
                                                tracing::info!(%request_id, "preparing CLI replacement while pane controllers remain attached");
                                            }
                                            ServerToCli::RequestPaneList { .. } => {
                                                let panes = build_pane_list(&pane_metas, &input_channels, session_id,
                                                    &pane_sessions, &pane_pauses, &pane_stop_requests);
                                                let msg = serde_json::to_string(&CliToServer::PaneList {
                                                    session_id, panes }).unwrap_or_default();
                                                let _ = ws_sender.send(Message::Text(msg.into())).await;
                                            }
                                            ServerToCli::InterruptPane { session_id: _, pane_id: target_pane } => {
                                                // Snapshot the streaming-interrupt sender (if any)
                                                // and the child PID without holding the meta lock
                                                // across the kill (the worker thread may want it
                                                // on exit).
                                                let (soft_interrupt, child_pid): (Option<mpsc::Sender<()>>, Option<u32>) = {
                                                    let metas = pane_metas.lock().unwrap();
                                                    match metas.get(&target_pane) {
                                                        Some(m) => {
                                                            let soft = m.streaming_interrupt_tx.lock().ok()
                                                                .and_then(|g| g.as_ref().cloned());
                                                            let pid = m.child_process.lock().ok()
                                                                .and_then(|g| g.as_ref().map(|c| c.id()));
                                                            (soft, pid)
                                                        }
                                                        None => (None, None),
                                                    }
                                                };
                                                // Streaming-pane soft interrupt: ask the worker to
                                                // pump a control_request("interrupt") onto claude's
                                                // stdin, which aborts the current turn but keeps
                                                // the long-lived process alive. Falls through to
                                                // the SIGINT path on send failure (worker dead).
                                                if let Some(tx) = soft_interrupt {
                                                    if tx.send(()).is_ok() {
                                                        tracing::info!(
                                                            pane_id = target_pane,
                                                            "InterruptPane: signaled streaming worker for soft interrupt",
                                                        );
                                                        continue;
                                                    }
                                                    tracing::warn!(
                                                        pane_id = target_pane,
                                                        "InterruptPane: streaming worker channel dead, falling back to SIGINT",
                                                    );
                                                }
                                                match child_pid {
                                                    Some(pid) => {
                                                        tracing::info!(
                                                            pane_id = target_pane,
                                                            pid,
                                                            "InterruptPane: sending SIGINT to agent",
                                                        );
                                                        // SIGINT first; if it doesn't die in 2s, SIGKILL.
                                                        let _ = std::process::Command::new("kill")
                                                            .arg("-INT")
                                                            .arg(pid.to_string())
                                                            .status();
                                                        let pid_for_fallback = pid;
                                                        let pane_for_fallback = target_pane;
                                                        std::thread::spawn(move || {
                                                            std::thread::sleep(Duration::from_secs(2));
                                                            // If still alive, escalate.
                                                            let alive = std::path::Path::new(&format!(
                                                                "/proc/{}",
                                                                pid_for_fallback
                                                            ))
                                                            .exists();
                                                            if alive {
                                                                tracing::warn!(
                                                                    pane_id = pane_for_fallback,
                                                                    pid = pid_for_fallback,
                                                                    "InterruptPane: SIGINT didn't take, sending SIGKILL",
                                                                );
                                                                let _ = std::process::Command::new("kill")
                                                                    .arg("-KILL")
                                                                    .arg(pid_for_fallback.to_string())
                                                                    .status();
                                                            }
                                                        });
                                                    }
                                                    None => {
                                                        tracing::info!(
                                                            pane_id = target_pane,
                                                            "InterruptPane: no live agent for this pane, ignoring",
                                                        );
                                                    }
                                                }
                                            }
                                            ServerToCli::UpdatePaneEffort { session_id: _, pane_id: target_pane, effort } => {
                                                let normalized = normalize_effort_level(effort.as_deref());
                                                // Update the persisted field + the live mirror
                                                // cell that the spawn loop reads. Then snapshot
                                                // the control_response channel — we use it to
                                                // push an `apply_flag_settings` control_request
                                                // into claude's stdin, which updates effort
                                                // live without killing the process. effort_arc
                                                // is the safety net for the next fresh respawn.
                                                //
                                                // We do NOT short-circuit on "value unchanged":
                                                // the user explicitly clicked the dropdown, so
                                                // re-apply unconditionally. This also recovers
                                                // from any drift between meta.effort (what we
                                                // persisted) and the actual --effort in flight.
                                                let (control_tx, is_claude): (Option<mpsc::Sender<String>>, bool) = {
                                                    let mut metas = pane_metas.lock().unwrap();
                                                    if let Some(meta) = metas.get_mut(&target_pane) {
                                                        meta.effort = normalized.clone();
                                                        if let Ok(mut g) = meta.effort_arc.lock() {
                                                            *g = normalized.clone();
                                                        }
                                                        let tx = meta.control_response_tx
                                                            .lock()
                                                            .ok()
                                                            .and_then(|g| g.as_ref().cloned());
                                                        (tx, matches!(meta.provider, shared::Provider::Claude))
                                                    } else {
                                                        (None, false)
                                                    }
                                                };
                                                save_pane_configs(
                                                    working_dir,
                                                    &pane_sessions,
                                                    &pane_metas,
                                                    &pane_pauses,
                                                    &pane_stop_requests,
                                                );
                                                tracing::info!(
                                                    pane_id = target_pane,
                                                    effort = ?normalized,
                                                    "Pane effort updated and persisted to .apas",
                                                );
                                                // Push apply_flag_settings live — claude's SDK
                                                // accepts `effortLevel` on the persisted-settings
                                                // shape and applies it to subsequent turns. The
                                                // current in-flight turn (if any) keeps the old
                                                // effort; the next prompt fires at the new level.
                                                // No restart, no SIGINT, no waiting.
                                                if is_claude {
                                                    let chat_text = match (control_tx, normalized.clone()) {
                                                        (Some(tx), Some(level)) => {
                                                            // claude's runtime rejects the apas-only
                                                            // `ultracode` literal — send `xhigh` on
                                                            // the wire while keeping the chat-text
                                                            // and persisted meta.effort as the
                                                            // user-facing level.
                                                            let wire_level = effort_to_claude_flag(&level).to_string();
                                                            let req = serde_json::json!({
                                                                "type": "control_request",
                                                                "request_id": format!("apas-effort-{}", uuid::Uuid::new_v4()),
                                                                "request": {
                                                                    "subtype": "apply_flag_settings",
                                                                    "settings": { "effortLevel": wire_level },
                                                                },
                                                            });
                                                            if tx.send(req.to_string()).is_ok() {
                                                                tracing::info!(
                                                                    pane_id = target_pane,
                                                                    level = %level,
                                                                    wire_level = %wire_level,
                                                                    "Sent apply_flag_settings(effortLevel) live to claude",
                                                                );
                                                                Some(format!("[Effort set to {} — applies to the next prompt]", level))
                                                            } else {
                                                                tracing::warn!(
                                                                    pane_id = target_pane,
                                                                    "Effort change: control_response channel dead; new effort will apply on next claude respawn",
                                                                );
                                                                Some(format!("[Effort persisted to {}; channel dead, will apply on next claude restart]", level))
                                                            }
                                                        }
                                                        (None, Some(level)) => {
                                                            tracing::warn!(
                                                                pane_id = target_pane,
                                                                "Effort change: no control_response_tx registered (worker not initialized yet?); new effort will apply on next claude respawn",
                                                            );
                                                            Some(format!("[Effort persisted to {}; live update unavailable, will apply on next claude restart]", level))
                                                        }
                                                        (_, None) => {
                                                            // User reset to default (no --effort)
                                                            // — we don't have a way to clear via
                                                            // apply_flag_settings without
                                                            // restarting, so just persist and
                                                            // leave the live claude alone.
                                                            tracing::info!(
                                                                pane_id = target_pane,
                                                                "Effort cleared to default; live claude unchanged until next respawn",
                                                            );
                                                            Some("[Effort reset to default; takes effect on next claude restart]".to_string())
                                                        }
                                                    };
                                                    // Surface a system message in the chat so the
                                                    // change is unmistakably visible — silent
                                                    // success was confusing users into thinking
                                                    // nothing happened.
                                                    if let Some(text) = chat_text {
                                                        let msg = CliToServer::Output {
                                                            session_id,
                                                            data: text,
                                                            output_type: shared::OutputType::System,
                                                            pane_type: None,
                                                            pane_id: Some(target_pane),
                                                        };
                                                        let msg_text = serde_json::to_string(&msg).unwrap_or_default();
                                                        let _ = ws_sender.send(Message::Text(msg_text.into())).await;
                                                    }
                                                }
                                            }
                                            ServerToCli::UpdatePaneModel { session_id: _, pane_id: target_pane, provider, model } => {
                                                // Two paths:
                                                //
                                                // (fast) LIVE swap via apply_flag_settings — no
                                                //        child kill, no fresh session, no context
                                                //        reset. Fires when the transition stays on
                                                //        provider Claude AND neither the old nor
                                                //        the new model triggers a backend-swap env
                                                //        override (DeepSeek pins ANTHROPIC_BASE_URL
                                                //        at spawn and cannot be changed on a running
                                                //        process). Matches
                                                //        the effort-live pattern at ~10422.
                                                //
                                                // (slow) Kill + respawn with a fresh session id.
                                                //        Fallback for provider changes or backend
                                                //        swaps. Chat history stays visible on the
                                                //        client but is not in the new agent's
                                                //        prompt.
                                                let trimmed = canonicalize_requested_model(
                                                    model.as_deref(),
                                                );

                                                let retired = pane_metas
                                                    .lock()
                                                    .ok()
                                                    .and_then(|panes| panes.get(&target_pane).cloned())
                                                    .is_some_and(|pane| {
                                                        shared::is_retired_launch(
                                                            pane.provider,
                                                            pane.model.as_deref(),
                                                        ) || shared::is_retired_launch(
                                                            provider.unwrap_or(pane.provider),
                                                            trimmed.as_deref(),
                                                        )
                                                    });
                                                if retired {
                                                    report_retired_launch_rejection(
                                                        &status_tx,
                                                        &mut ws_sender,
                                                        session_id,
                                                        target_pane,
                                                        "[Model switch refused — unsupported provider has been retired]",
                                                    )
                                                    .await;
                                                    continue;
                                                }

                                                let allowed = pane_metas
                                                    .lock()
                                                    .ok()
                                                    .and_then(|panes| panes.get(&target_pane).cloned())
                                                    .is_some_and(|pane| {
                                                        // A model switch chooses a
                                                        // profile, so the allowlist
                                                        // governs it — and nothing
                                                        // else does.
                                                        launch_allowed_by_server_policy(
                                                            &project_policy,
                                                            pane.kind,
                                                            provider.unwrap_or(pane.provider),
                                                            trimmed.as_deref(),
                                                        )
                                                    });
                                                if !allowed {
                                                    let _ = status_tx.send(PaneOutput {
                                                        text: "[Model switch refused by the current cluster policy]".to_string(),
                                                        pane_id: target_pane,
                                                    });
                                                    continue;
                                                }

                                                // --- Fast path attempt ------------------------
                                                let fast_path_taken = 'fast: {
                                                    let metas = pane_metas.lock().unwrap();
                                                    let meta = match metas.get(&target_pane) {
                                                        Some(m) => m,
                                                        None => break 'fast false,
                                                    };
                                                    // Only attempt live swap when the pane stays on
                                                    // provider=Claude, neither side is a backend
                                                    // swap, and the user picked an explicit new
                                                    // model (clearing to default has no clean
                                                    // apply_flag_settings verb, so respawn instead).
                                                    if !model_switch_can_use_fast_path(
                                                        meta.provider,
                                                        meta.model.as_deref(),
                                                        provider.unwrap_or(meta.provider),
                                                        trimmed.as_deref(),
                                                    ) {
                                                        break 'fast false;
                                                    }
                                                    let control_tx = meta
                                                        .control_response_tx
                                                        .lock()
                                                        .ok()
                                                        .and_then(|g| g.as_ref().cloned());
                                                    let (Some(tx), Some(new_model)) =
                                                        (control_tx, trimmed.clone())
                                                    else {
                                                        break 'fast false;
                                                    };
                                                    let req = serde_json::json!({
                                                        "type": "control_request",
                                                        "request_id": format!("apas-model-{}", uuid::Uuid::new_v4()),
                                                        "request": {
                                                            "subtype": "apply_flag_settings",
                                                            "settings": { "model": new_model },
                                                        },
                                                    });
                                                    if tx.send(req.to_string()).is_ok() {
                                                        tracing::info!(
                                                            pane_id = target_pane,
                                                            new_model = %new_model,
                                                            "Sent apply_flag_settings(model) live to claude — no respawn",
                                                        );
                                                        true
                                                    } else {
                                                        tracing::warn!(
                                                            pane_id = target_pane,
                                                            "UpdatePaneModel: control_response_tx send failed; falling back to respawn",
                                                        );
                                                        false
                                                    }
                                                };

                                                if fast_path_taken {
                                                    // Update the persisted meta.model so the
                                                    // saved .apas matches what claude is now
                                                    // running, and so the next respawn (e.g.
                                                    // after a CLI reboot) reads the new value.
                                                    // Effort_arc-style mirror not needed — model
                                                    // isn't read out-of-band by the spawn loop
                                                    // between apply_flag_settings and PaneList.
                                                    {
                                                        let mut metas = pane_metas.lock().unwrap();
                                                        if let Some(meta) = metas.get_mut(&target_pane) {
                                                            meta.model = trimmed.clone();
                                                        }
                                                    }
                                                    save_pane_configs(
                                                        working_dir,
                                                        &pane_sessions,
                                                        &pane_metas,
                                                        &pane_pauses,
                                                        &pane_stop_requests,
                                                    );
                                                    // Broadcast fresh PaneList so the web tab
                                                    // header switches its model badge.
                                                    let pane_list_msg = CliToServer::PaneList {
                                                        session_id,
                                                        panes: build_pane_list(
                                                            &pane_metas,
                                                            &input_channels,
                                                            session_id,
                                                            &pane_sessions,
                                                            &pane_pauses,
                                                            &pane_stop_requests,
                                                        ),
                                                    };
                                                    if let Ok(text) = serde_json::to_string(&pane_list_msg) {
                                                        let _ = ws_sender.send(Message::Text(text.into())).await;
                                                    }
                                                    // Chat status line so the user sees the swap
                                                    // was live — no context reset, no interrupted
                                                    // turn.
                                                    let banner = format!(
                                                        "[Model switched to {} — no respawn, chat history preserved. Takes effect on the next prompt.]",
                                                        trimmed.as_deref().unwrap_or("default"),
                                                    );
                                                    let banner_msg = CliToServer::Output {
                                                        session_id,
                                                        data: banner,
                                                        output_type: shared::OutputType::System,
                                                        pane_type: None,
                                                        pane_id: Some(target_pane),
                                                    };
                                                    if let Ok(text) = serde_json::to_string(&banner_msg) {
                                                        let _ = ws_sender.send(Message::Text(text.into())).await;
                                                    }
                                                    continue;
                                                }
                                                // --- Slow path (kill + respawn) --------------
                                                tracing::info!(
                                                    pane_id = target_pane,
                                                    ?provider,
                                                    ?trimmed,
                                                    "UpdatePaneModel: fast-path not eligible or failed; falling back to kill + respawn"
                                                );
                                                let snapshot = {
                                                    let mut metas = pane_metas.lock().unwrap();
                                                    let Some(meta) = metas.get_mut(&target_pane) else {
                                                        tracing::warn!(
                                                            pane_id = target_pane,
                                                            "UpdatePaneModel: pane not found in metas; ignoring"
                                                        );
                                                        continue;
                                                    };
                                                    if let Some(p) = provider {
                                                        meta.provider = p;
                                                    }
                                                    meta.model = trimmed.clone();
                                                    Some((
                                                        meta.label.clone(),
                                                        meta.mode.clone(),
                                                        meta.provider,
                                                        meta.prompt.clone(),
                                                        meta.min_iteration_interval_minutes,
                                                        trimmed.clone(),
                                                        meta.effort.clone(),
                                                        meta.worktree_path.clone(),
                                                        meta.role.clone(),
                                                        meta.goal.clone(),
                                                        meta.backstory.clone(),
                                                        meta.plan_review_mode,
                                                        meta.managed,
                                                        meta.child_process.clone(),
                                                    ))
                                                };
                                                let Some((
                                                    label,
                                                    mode,
                                                    effective_provider,
                                                    prompt,
                                                    min_interval,
                                                    new_model,
                                                    effort,
                                                    worktree_path,
                                                    role,
                                                    goal,
                                                    backstory,
                                                    plan_review_mode,
                                                    managed,
                                                    child_process,
                                                )) = snapshot
                                                else {
                                                    continue;
                                                };

                                                // Generate a fresh claude session so the new model
                                                // doesn't try to --resume the prior conversation
                                                // (which was bound to the old model + agent).
                                                let new_session = Uuid::new_v4();
                                                {
                                                    let mut sessions = pane_sessions.lock().unwrap();
                                                    sessions.insert(target_pane, new_session);
                                                }

                                                // Kill the running claude child so the streaming
                                                // worker's read loop EOFs and the worker thread
                                                // exits. The new AddTabWithConfig below replaces
                                                // input_channels[target_pane], dropping the old
                                                // sender — the worker's input_rx then EOFs too as
                                                // a belt-and-suspenders.
                                                if let Some(terminal) = terminal_panes
                                                    .lock()
                                                    .ok()
                                                    .and_then(|mut panes| panes.remove(&target_pane))
                                                {
                                                    terminal.shutdown();
                                                }
                                                if let Ok(mut guard) = child_process.lock() {
                                                    if let Some(ref mut child) = *guard {
                                                        let _ = child.kill();
                                                    }
                                                    *guard = None;
                                                }

                                                tracing::info!(
                                                    pane_id = target_pane,
                                                    new_provider = ?provider,
                                                    new_model = ?new_model,
                                                    new_session = %new_session,
                                                    "Agent switch: killed old child, respawning with fresh session"
                                                );

                                                // Surface the change in chat so the user sees
                                                // the swap and the context reset together.
                                                let provider_label = format!("{:?}", effective_provider).to_lowercase();
                                                let model_label = new_model.as_deref().unwrap_or("default");
                                                let banner = if provider.is_some() {
                                                    format!(
                                                        "[Agent switched to {} (model: {}). The new agent starts with a fresh context — chat history above is still visible but is NOT part of the new agent's prompt.]",
                                                        provider_label, model_label
                                                    )
                                                } else {
                                                    format!(
                                                        "[Model switched to {}. The new agent starts with a fresh context — chat history above is still visible but is NOT part of the new agent's prompt.]",
                                                        model_label
                                                    )
                                                };
                                                let banner_msg = CliToServer::Output {
                                                    session_id,
                                                    data: banner,
                                                    output_type: shared::OutputType::System,
                                                    pane_type: Some(match mode {
                                                        shared::PaneMode::Deadloop => PaneType::Deadloop,
                                                        shared::PaneMode::Interactive => PaneType::Interactive,
                                                    }),
                                                    pane_id: Some(target_pane),
                                                };
                                                if let Ok(text) = serde_json::to_string(&banner_msg) {
                                                    let _ = ws_sender.send(Message::Text(text.into())).await;
                                                }

                                                // Re-emit the spawn event with the new model +
                                                // fresh session. The AddTabWithConfig handler
                                                // overwrites the PaneMeta + input_channels entries
                                                // and spawns a new worker thread.
                                                let _ = tui_event_tx.send(build_agent_switch_respawn_event(
                                                    target_pane,
                                                    label,
                                                    new_session,
                                                    mode,
                                                    effective_provider,
                                                    prompt,
                                                    min_interval,
                                                    new_model,
                                                    effort,
                                                    worktree_path,
                                                    role,
                                                    goal,
                                                    backstory,
                                                    plan_review_mode,
                                                    managed,
                                                ));

                                                // Persist to .apas + broadcast the fresh PaneList.
                                                save_pane_configs(
                                                    &working_dir,
                                                    &pane_sessions,
                                                    &pane_metas,
                                                    &pane_pauses,
                                                    &pane_stop_requests,
                                                );
                                                let pane_list_msg = CliToServer::PaneList {
                                                    session_id,
                                                    panes: build_pane_list(
                                                        &pane_metas,
                                                        &input_channels,
                                                        session_id,
                                                        &pane_sessions,
                                                        &pane_pauses,
                                                        &pane_stop_requests,
                                                    ),
                                                };
                                                if let Ok(text) = serde_json::to_string(&pane_list_msg) {
                                                    let _ = ws_sender.send(Message::Text(text.into())).await;
                                                }
                                            }
                                            ServerToCli::AnswerQuestion {
                                                session_id: _,
                                                tool_use_id,
                                                answers,
                                            } => {
                                                // Find the pane whose pending_questions map holds
                                                // this tool_use_id, then build the matching
                                                // control_response and hand it to that pane's
                                                // streaming worker for write-to-stdin. Tool use
                                                // ids are globally unique so the first match wins.
                                                let mut handled = false;
                                                let metas_snapshot: Vec<(u32, Arc<Mutex<HashMap<String, PendingAskQuestion>>>, Arc<Mutex<Option<mpsc::Sender<String>>>>)> = {
                                                    let metas = pane_metas.lock().unwrap();
                                                    metas
                                                        .iter()
                                                        .map(|(pid, m)| (
                                                            *pid,
                                                            m.pending_questions.clone(),
                                                            m.control_response_tx.clone(),
                                                        ))
                                                        .collect()
                                                };
                                                for (pid, pending_arc, tx_arc) in metas_snapshot {
                                                    let pending = {
                                                        let mut map = match pending_arc.lock() {
                                                            Ok(m) => m,
                                                            Err(_) => continue,
                                                        };
                                                        map.remove(&tool_use_id)
                                                    };
                                                    if let Some(pending) = pending {
                                                        let response = serde_json::json!({
                                                            "type": "control_response",
                                                            "response": {
                                                                "subtype": "success",
                                                                "request_id": pending.request_id,
                                                                "response": {
                                                                    "behavior": "allow",
                                                                    "updatedInput": {
                                                                        "questions": pending.questions,
                                                                        "answers": answers,
                                                                    },
                                                                    "toolUseID": tool_use_id,
                                                                }
                                                            }
                                                        });
                                                        let payload = response.to_string();
                                                        let sender = tx_arc.lock().ok().and_then(|g| g.as_ref().cloned());
                                                        match sender {
                                                            Some(tx) => {
                                                                if tx.send(payload).is_err() {
                                                                    tracing::warn!(
                                                                        pane_id = pid,
                                                                        tool_use_id = tool_use_id.as_str(),
                                                                        "AnswerQuestion: streaming worker channel dead",
                                                                    );
                                                                } else {
                                                                    tracing::info!(
                                                                        pane_id = pid,
                                                                        tool_use_id = tool_use_id.as_str(),
                                                                        "AnswerQuestion: queued control_response for stdin",
                                                                    );
                                                                }
                                                            }
                                                            None => {
                                                                tracing::warn!(
                                                                    pane_id = pid,
                                                                    tool_use_id = tool_use_id.as_str(),
                                                                    "AnswerQuestion: no control_response sender registered",
                                                                );
                                                            }
                                                        }
                                                        handled = true;
                                                        break;
                                                    }
                                                }
                                                // A terminal pane has no stdin control channel:
                                                // it hosts the provider's real TUI, so the answer
                                                // has to arrive as keystrokes. Everything above
                                                // this point in the pipeline is shared with agent
                                                // panes; only this last hop differs.
                                                if !handled {
                                                    let plan = terminal_questions
                                                        .lock()
                                                        .map(|questions| plan_terminal_answer(&questions, &tool_use_id, &answers))
                                                        .unwrap_or(TerminalAnswerPlan::NotPending);
                                                    match plan {
                                                        TerminalAnswerPlan::AlreadyDelivered { pane_id: pid } => {
                                                            // Retransmits are expected — the web
                                                            // resends unacked answers — but typing
                                                            // twice into a live TUI is not.
                                                            tracing::info!(
                                                                pane_id = pid,
                                                                tool_use_id = tool_use_id.as_str(),
                                                                "AnswerQuestion: already delivered to the terminal, sending nothing",
                                                            );
                                                            handled = true;
                                                        }
                                                        TerminalAnswerPlan::UnknownOption { pane_id: pid } => {
                                                            tracing::warn!(
                                                                pane_id = pid,
                                                                tool_use_id = tool_use_id.as_str(),
                                                                "AnswerQuestion: answer names an option this question never offered",
                                                            );
                                                        }
                                                        TerminalAnswerPlan::Deliver { pane_id: pid, keys } => {
                                                            let handle = terminal_panes
                                                                .lock()
                                                                .ok()
                                                                .and_then(|m| m.get(&pid).cloned());
                                                            match handle {
                                                                Some(handle) => match handle.write_bytes(&keys) {
                                                                    Ok(()) => {
                                                                        if let Ok(mut questions) = terminal_questions.lock() {
                                                                            if let Some(entry) = questions.get_mut(&tool_use_id) {
                                                                                entry.delivered = true;
                                                                            }
                                                                        }
                                                                        // Delivered, not answered: the
                                                                        // transcript's tool_result is what
                                                                        // confirms it, and it is what the
                                                                        // conversation view will show.
                                                                        tracing::info!(
                                                                            pane_id = pid,
                                                                            tool_use_id = tool_use_id.as_str(),
                                                                            "AnswerQuestion: wrote selection to the terminal",
                                                                        );
                                                                        handled = true;
                                                                    }
                                                                    Err(e) => {
                                                                        tracing::warn!(
                                                                            pane_id = pid,
                                                                            tool_use_id = tool_use_id.as_str(),
                                                                            error = %e,
                                                                            "AnswerQuestion: terminal write failed; question stays pending",
                                                                        );
                                                                    }
                                                                },
                                                                None => {
                                                                    tracing::warn!(
                                                                        pane_id = pid,
                                                                        tool_use_id = tool_use_id.as_str(),
                                                                        "AnswerQuestion: terminal pane has no live pty",
                                                                    );
                                                                }
                                                            }
                                                        }
                                                        TerminalAnswerPlan::NotPending => {}
                                                    }
                                                }
                                                if !handled {
                                                    tracing::warn!(
                                                        tool_use_id = tool_use_id.as_str(),
                                                        "AnswerQuestion: no matching pending AskUserQuestion (already answered or expired)",
                                                    );
                                                }
                                            }
                                            ServerToCli::UpdatePaneLabel { session_id: _, pane_id: label_pane_id, label } => {
                                                let trimmed = label.trim().to_string();
                                                if trimmed.is_empty() {
                                                    tracing::debug!(pane_id = label_pane_id, "UpdatePaneLabel: empty label ignored");
                                                    continue;
                                                }
                                                let updated = {
                                                    let mut metas = pane_metas.lock().unwrap();
                                                    if let Some(m) = metas.get_mut(&label_pane_id) {
                                                        m.label = trimmed.clone();
                                                        true
                                                    } else {
                                                        false
                                                    }
                                                };
                                                if updated {
                                                    save_pane_configs(
                                                        &working_dir,
                                                        &pane_sessions,
                                                        &pane_metas,
                                                        &pane_pauses,
                                                        &pane_stop_requests,
                                                    );
                                                    // Echo back the fresh PaneList so any web clients
                                                    // attached now see the canonical label from disk
                                                    // (the server already updated its own cache, but
                                                    // this keeps the two sources of truth in sync).
                                                    let panes = build_pane_list(
                                                        &pane_metas,
                                                        &input_channels,
                                                        session_id,
                                                        &pane_sessions,
                                                        &pane_pauses,
                                                        &pane_stop_requests,
                                                    );
                                                    let pane_list_msg = CliToServer::PaneList {
                                                        session_id,
                                                        panes,
                                                    };
                                                    if let Ok(text) = serde_json::to_string(&pane_list_msg) {
                                                        let _ = ws_sender
                                                            .send(Message::Text(text.into()))
                                                            .await;
                                                    }
                                                    tracing::info!(
                                                        pane_id = label_pane_id,
                                                        label = trimmed.as_str(),
                                                        "Pane label updated and persisted to .apas",
                                                    );
                                                }
                                            }
                                            ServerToCli::UpdatePaneRole { session_id: _, pane_id: role_pane_id, role, goal, backstory } => {
                                                // Mutate in-memory PaneMeta. Empty strings normalize to None
                                                // so the user can clear a field via the web UI.
                                                let norm = |s: Option<String>| s.and_then(|v| {
                                                    let t = v.trim().to_string();
                                                    if t.is_empty() { None } else { Some(t) }
                                                });
                                                let role = norm(role);
                                                let goal = norm(goal);
                                                let backstory = norm(backstory);
                                                let updated = {
                                                    let mut metas = pane_metas.lock().unwrap();
                                                    if let Some(m) = metas.get_mut(&role_pane_id) {
                                                        m.role = role.clone();
                                                        m.goal = goal.clone();
                                                        m.backstory = backstory.clone();
                                                        true
                                                    } else {
                                                        false
                                                    }
                                                };
                                                if updated {
                                                    save_pane_configs(
                                                        &working_dir,
                                                        &pane_sessions,
                                                        &pane_metas,
                                                        &pane_pauses,
                                                        &pane_stop_requests,
                                                    );
                                                    let hint = "[Role/goal/backstory updated. Takes effect on next pane restart (close + re-add the tab, or reboot the apas CLI).]".to_string();
                                                    let msg = CliToServer::Output {
                                                        session_id,
                                                        data: hint,
                                                        output_type: shared::OutputType::System,
                                                        pane_type: None,
                                                        pane_id: Some(role_pane_id),
                                                    };
                                                    if let Ok(text) = serde_json::to_string(&msg) {
                                                        let _ = ws_sender
                                                            .send(Message::Text(text.into()))
                                                            .await;
                                                    }
                                                }
                                            }
                                            ServerToCli::PlanReviewAnswer { session_id: _, tool_use_id, approve } => {
                                                // Find which pane has this tool_use_id parked, drain it,
                                                // and send a control_response on its stdin channel. Tool
                                                // use ids are globally unique so the first match wins.
                                                let metas_snapshot: Vec<(u32, Arc<Mutex<HashMap<String, PendingPlanReview>>>, Arc<Mutex<Option<mpsc::Sender<String>>>>)> = {
                                                    let metas = pane_metas.lock().unwrap();
                                                    metas
                                                        .iter()
                                                        .map(|(pid, m)| (
                                                            *pid,
                                                            m.pending_plan_reviews.clone(),
                                                            m.control_response_tx.clone(),
                                                        ))
                                                        .collect()
                                                };
                                                for (pid, pending_arc, tx_arc) in metas_snapshot {
                                                    let pending = {
                                                        let mut map = match pending_arc.lock() {
                                                            Ok(m) => m,
                                                            Err(_) => continue,
                                                        };
                                                        map.remove(&tool_use_id)
                                                    };
                                                    if let Some(pending) = pending {
                                                        let response = serde_json::json!({
                                                            "type": "control_response",
                                                            "response": {
                                                                "subtype": "success",
                                                                "request_id": pending.request_id,
                                                                "response": if approve {
                                                                    serde_json::json!({
                                                                        "behavior": "allow",
                                                                        "updatedInput": pending.input,
                                                                        "toolUseID": tool_use_id,
                                                                    })
                                                                } else {
                                                                    serde_json::json!({
                                                                        "behavior": "deny",
                                                                        "message": "User rejected this tool use via the plan-review card.",
                                                                        "toolUseID": tool_use_id,
                                                                    })
                                                                }
                                                            }
                                                        });
                                                        let sender = tx_arc.lock().ok().and_then(|g| g.as_ref().cloned());
                                                        if let Some(tx) = sender {
                                                            let _ = tx.send(response.to_string());
                                                            tracing::info!(
                                                                pane_id = pid,
                                                                tool_use_id = tool_use_id.as_str(),
                                                                approve,
                                                                "plan review: user verdict relayed to claude",
                                                            );
                                                        } else {
                                                            tracing::warn!(
                                                                pane_id = pid,
                                                                tool_use_id = tool_use_id.as_str(),
                                                                "plan review: no control_response sender registered",
                                                            );
                                                        }
                                                        break;
                                                    }
                                                }
                                            }
                                            ServerToCli::UpdatePaneReviewMode { session_id: _, pane_id: rmode_pane_id, mode } => {
                                                let updated = {
                                                    let mut metas = pane_metas.lock().unwrap();
                                                    if let Some(m) = metas.get_mut(&rmode_pane_id) {
                                                        m.plan_review_mode = mode;
                                                        if let Ok(mut g) = m.plan_review_mode_arc.lock() {
                                                            *g = mode;
                                                        }
                                                        true
                                                    } else {
                                                        false
                                                    }
                                                };
                                                if updated {
                                                    save_pane_configs(
                                                        &working_dir,
                                                        &pane_sessions,
                                                        &pane_metas,
                                                        &pane_pauses,
                                                        &pane_stop_requests,
                                                    );
                                                    let hint = format!(
                                                        "[Plan review mode → {:?} for this pane. Takes effect on the next tool_use.]",
                                                        mode,
                                                    );
                                                    let msg = CliToServer::Output {
                                                        session_id,
                                                        data: hint,
                                                        output_type: shared::OutputType::System,
                                                        pane_type: None,
                                                        pane_id: Some(rmode_pane_id),
                                                    };
                                                    if let Ok(text) = serde_json::to_string(&msg) {
                                                        let _ = ws_sender
                                                            .send(Message::Text(text.into()))
                                                            .await;
                                                    }
                                                }
                                            }
                                            ServerToCli::UpdatePaneManualMode { session_id: _, pane_id: mmode_pane_id, manual_mode } => {
                                                let updated = {
                                                    let mut metas = pane_metas.lock().unwrap();
                                                    if let Some(m) = metas.get_mut(&mmode_pane_id) {
                                                        m.manual_mode = manual_mode;
                                                        true
                                                    } else {
                                                        false
                                                    }
                                                };
                                                if updated {
                                                    save_pane_configs(
                                                        &working_dir,
                                                        &pane_sessions,
                                                        &pane_metas,
                                                        &pane_pauses,
                                                        &pane_stop_requests,
                                                    );
                                                    // Broadcast the fresh PaneList so the web's chip
                                                    // can flip — save_pane_configs only writes to .apas,
                                                    // it doesn't push to the server.
                                                    let pane_list_msg = CliToServer::PaneList {
                                                        session_id,
                                                        panes: build_pane_list(
                                                            &pane_metas,
                                                            &input_channels,
                                                            session_id,
                                                            &pane_sessions,
                                                            &pane_pauses,
                                                            &pane_stop_requests,
                                                        ),
                                                    };
                                                    if let Ok(text) = serde_json::to_string(&pane_list_msg) {
                                                        let _ = ws_sender
                                                            .send(Message::Text(text.into()))
                                                            .await;
                                                    }
                                                    let hint = format!(
                                                        "[Worker mode → {}. Tech Lead will {} this pane for delegations.]",
                                                        if manual_mode { "manual" } else { "autonomous" },
                                                        if manual_mode { "skip" } else { "consider" },
                                                    );
                                                    let msg = CliToServer::Output {
                                                        session_id,
                                                        data: hint,
                                                        output_type: shared::OutputType::System,
                                                        pane_type: None,
                                                        pane_id: Some(mmode_pane_id),
                                                    };
                                                    if let Ok(text) = serde_json::to_string(&msg) {
                                                        let _ = ws_sender
                                                            .send(Message::Text(text.into()))
                                                            .await;
                                                    }
                                                }
                                            }
                                            ServerToCli::RequestPaneDiff { session_id: _, pane_id: diff_pane_id } => {
                                                // Look up the pane's worktree path. If unset, return a polite error
                                                // so the web UI can render guidance instead of nothing.
                                                let wt: Option<String> = {
                                                    let metas = pane_metas.lock().unwrap();
                                                    metas.get(&diff_pane_id).and_then(|m| m.worktree_path.clone())
                                                };
                                                let result = crate::worktree::compute_pane_diff(
                                                    std::path::Path::new(&working_dir),
                                                    wt.as_deref(),
                                                );
                                                let pane_diff_msg = match result {
                                                    Ok((branch, base, diff)) => CliToServer::PaneDiff {
                                                        session_id,
                                                        pane_id: diff_pane_id,
                                                        branch: Some(branch),
                                                        base: Some(base),
                                                        diff: Some(diff),
                                                        error: None,
                                                    },
                                                    Err(err) => CliToServer::PaneDiff {
                                                        session_id,
                                                        pane_id: diff_pane_id,
                                                        branch: None,
                                                        base: None,
                                                        diff: None,
                                                        error: Some(err.to_string()),
                                                    },
                                                };
                                                if let Ok(text) = serde_json::to_string(&pane_diff_msg) {
                                                    let _ = ws_sender
                                                        .send(Message::Text(text.into()))
                                                        .await;
                                                }
                                            }
                                            ServerToCli::UpdateProjectFlags {
                                                session_id: _,
                                                auto_approve_todos,
                                                auto_merge_prs,
                                                team_enabled: _,
                                                disallowed_tab_types: _,
                                            }
                                            | ServerToCli::UpdateProjectOperations {
                                                session_id: _,
                                                auto_approve_todos,
                                                auto_merge_prs,
                                            } => {
                                                let project_dir = std::path::Path::new(&working_dir).to_path_buf();
                                                match update_project_operations(
                                                    &project_dir,
                                                    session_id,
                                                    auto_approve_todos,
                                                    auto_merge_prs,
                                                ) {
                                                    Ok(echo) => {
                                                        tracing::info!(
                                                            auto_approve_todos,
                                                            auto_merge_prs,
                                                            "project workflow settings updated"
                                                        );
                                                        if let Ok(text) = serde_json::to_string(&echo) {
                                                            let _ = ws_sender.send(Message::Text(text.into())).await;
                                                        }
                                                    }
                                                    Err(err) => {
                                                        tracing::warn!("failed to persist project workflow settings: {}", err);
                                                    }
                                                }
                                            }
                                            ServerToCli::CreatePr { session_id: _, pane_id: pr_pane_id } => {
                                                let result = match manual_create_pr_worktree_path(
                                                    &pane_metas,
                                                    pr_pane_id,
                                                ) {
                                                    Ok(wt) => {
                                                        // gh pr create + git push are blocking on a network call;
                                                        // run in spawn_blocking so the WS reader loop stays responsive.
                                                        tokio::task::spawn_blocking(move || {
                                                            crate::worktree::create_pr_for_pane(wt.as_deref())
                                                        })
                                                        .await
                                                        .unwrap_or_else(|e| {
                                                            Err(anyhow::anyhow!("task join: {}", e))
                                                        })
                                                    }
                                                    Err(err) => Err(anyhow::anyhow!("{}", err)),
                                                };
                                                let pr_msg = match result {
                                                    Ok(url) => CliToServer::PrCreated {
                                                        session_id,
                                                        pane_id: pr_pane_id,
                                                        url: Some(url),
                                                        error: None,
                                                    },
                                                    Err(err) => CliToServer::PrCreated {
                                                        session_id,
                                                        pane_id: pr_pane_id,
                                                        url: None,
                                                        error: Some(err.to_string()),
                                                    },
                                                };
                                                if let Ok(text) = serde_json::to_string(&pr_msg) {
                                                    let _ = ws_sender
                                                        .send(Message::Text(text.into()))
                                                        .await;
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                Some(Ok(Message::Ping(data))) => {
                                    if ws_sender.send(Message::Pong(data)).await.is_err() {
                                        let _ = status_tx.send(PaneOutput {
                                            text: "[Server: Failed to send pong, reconnecting...]".to_string(),
                                            pane_id: shared::PANE_ID_DEADLOOP,
                                        });
                                        break;
                                    }
                                }
                                Some(Ok(Message::Pong(_))) => {}
                                Some(Ok(Message::Close(_))) | None => {
                                    let _ = status_tx.send(PaneOutput {
                                        text: "[Server: Connection closed, reconnecting...]".to_string(),
                                        pane_id: shared::PANE_ID_DEADLOOP,
                                    });
                                    break;
                                }
                                Some(Err(e)) => {
                                    let _ = status_tx.send(PaneOutput {
                                        text: format!("[Server: Connection error ({}), reconnecting...]", e),
                                        pane_id: shared::PANE_ID_DEADLOOP,
                                    });
                                    break;
                                }
                                _ => {}
                            }
                        }
                        _ = heartbeat_interval.tick() => {
                            if ws_sender.send(Message::Ping(vec![].into())).await.is_err() {
                                let _ = status_tx.send(PaneOutput {
                                    text: "[Server: Heartbeat failed, reconnecting...]".to_string(),
                                    pane_id: shared::PANE_ID_DEADLOOP,
                                });
                                break;
                            }
                        }
                        _ = usage_interval.tick() => {
                            let (has_claude, has_codex, has_deepseek) =
                                active_usage_providers(&pane_metas);
                            let max_age = chrono::Duration::minutes(USAGE_CACHE_MAX_AGE_MINUTES);

                            if has_claude {
                                if let Some(limits) =
                                    crate::usage::read_cached_claude_usage_limits(Some(max_age))
                                {
                                    let usage_msg = CliToServer::UsageLimits { provider: Provider::Claude, limits };
                                    let msg_text = serde_json::to_string(&usage_msg).unwrap_or_default();
                                    if ws_sender.send(Message::Text(msg_text.into())).await.is_err() {
                                        tracing::warn!("Failed to send Claude usage limits to server");
                                    }
                                } else {
                                    tracing::debug!("No fresh cached Claude usage limits available");
                                }
                            }

                            if has_codex {
                                if let Some(limits) =
                                    crate::usage::read_cached_codex_usage_limits(Some(max_age))
                                {
                                    let usage_msg = CliToServer::UsageLimits { provider: Provider::Codex, limits };
                                    let msg_text = serde_json::to_string(&usage_msg).unwrap_or_default();
                                    if ws_sender.send(Message::Text(msg_text.into())).await.is_err() {
                                        tracing::warn!("Failed to send Codex usage limits to server");
                                    }
                                } else {
                                    tracing::debug!("No fresh cached Codex usage limits available");
                                }
                            }

                            if has_deepseek {
                            if let Some(limits) =
                                crate::usage::read_cached_deepseek_usage_limits(Some(max_age))
                            {
                                let usage_msg = CliToServer::UsageLimits {
                                    provider: Provider::Deepseek,
                                    limits,
                                };
                                let msg_text = serde_json::to_string(&usage_msg).unwrap_or_default();
                                if ws_sender.send(Message::Text(msg_text.into())).await.is_err() {
                                    tracing::warn!("Failed to send DeepSeek usage limits to server");
                                }
                            } else {
                                tracing::debug!("No fresh cached DeepSeek usage limits available");
                            }
                            }
                        }
                    }

                    if shutdown.load(Ordering::SeqCst) {
                        break;
                    }
                }

                if !shutdown.load(Ordering::SeqCst) {
                    let _ = status_tx.send(PaneOutput {
                        text: format!(
                            "[Server: Will reconnect in 1s (attempt {})]",
                            connection_count + 1
                        ),
                        pane_id: shared::PANE_ID_DEADLOOP,
                    });
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
            Err(e) => {
                let _ = status_tx.send(PaneOutput {
                    text: format!(
                        "[Server: Connection failed - {}. Retry in {}s]",
                        e,
                        reconnect_delay.as_secs()
                    ),
                    pane_id: shared::PANE_ID_DEADLOOP,
                });
                tokio::time::sleep(reconnect_delay).await;
                reconnect_delay = std::cmp::min(reconnect_delay * 2, max_reconnect_delay);
                connection_count += 1;
            }
        }
    }

    Ok(())
}
