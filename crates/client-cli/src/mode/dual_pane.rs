//! Tab-based mode: Multiple independent Claude sessions as tabs
//!
//! New projects start with one default tab:
//! - Interactive session
//!
//! Users can create and close tabs dynamically from both TUI and web UI.

use anyhow::Result;
use shared::{
    ClaudeContentBlock, ClaudeStreamMessage, CliToServer, CodexStreamMessage, PaneType, Provider,
    ServerToCli,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::mpsc as tokio_mpsc;
use uuid::Uuid;

use crate::project::{get_or_create_project, save_project};
use crate::tui::{App, PaneOutput, TuiCommand, TuiEvent};

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

fn is_minimax_model(model: Option<&str>) -> bool {
    model
        .map(|m| {
            let normalized = m.trim().to_ascii_lowercase();
            !normalized.is_empty()
                && (normalized.contains("minimax") || normalized.starts_with("m2"))
        })
        .unwrap_or(false)
}

fn is_glm_model(model: Option<&str>) -> bool {
    model
        .map(|m| {
            let normalized = m.trim().to_ascii_lowercase();
            !normalized.is_empty() && (normalized.starts_with("glm") || normalized.contains("glm-"))
        })
        .unwrap_or(false)
}

fn default_pane_label(pane_id: u32, model: Option<&str>) -> String {
    match pane_id {
        shared::PANE_ID_DEADLOOP => "Deadloop".to_string(),
        shared::PANE_ID_INTERACTIVE => "Interactive".to_string(),
        _ if is_minimax_model(model) => format!("MiniMax {}", pane_id),
        _ if is_glm_model(model) => format!("GLM {}", pane_id),
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
    if is_minimax_model(model) && is_generic_tab_label(trimmed, pane_id) {
        return default;
    }
    if is_glm_model(model) && is_generic_tab_label(trimmed, pane_id) {
        return default;
    }
    trimmed.to_string()
}

fn resolve_pane_binary_path(
    provider: Provider,
    _model: Option<&str>,
    claude_path: &str,
    _minimax_path: &str,
    codex_path: &str,
    opencode_path: &str,
    cursor_agent_path: &str,
) -> String {
    match provider {
        Provider::Claude | Provider::Minimax | Provider::Glm => claude_path.to_string(),
        Provider::Codex => codex_path.to_string(),
        Provider::Opencode => opencode_path.to_string(),
        Provider::CursorAgent => cursor_agent_path.to_string(),
    }
}

fn provider_display_name(provider: &Provider, model: Option<&str>) -> &'static str {
    match provider {
        Provider::Claude if is_minimax_model(model) => "MiniMax",
        Provider::Claude if is_glm_model(model) => "GLM",
        Provider::Claude => "Claude",
        Provider::Codex => "Codex",
        Provider::Minimax => "MiniMax",
        Provider::Glm => "GLM",
        Provider::Opencode => "OpenCode",
        Provider::CursorAgent => "Cursor",
    }
}

fn provider_config_key(provider: &Provider, model: Option<&str>) -> &'static str {
    match provider {
        Provider::Claude if is_minimax_model(model) => "claude_path",
        Provider::Claude => "claude_path",
        Provider::Codex => "codex_path",
        Provider::Minimax => "claude_path",
        Provider::Glm => "claude_path",
        Provider::Opencode => "opencode_path",
        Provider::CursorAgent => "cursor_agent_path",
    }
}

const MINIMAX_API_BASE_URL: &str = "https://api.minimax.io/anthropic";
const GLM_API_BASE_URL: &str = "https://api.z.ai/api/anthropic";
const GLM_DEFAULT_HAIKU_MODEL: &str = "glm-4.5-air";

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

fn normalize_effort_level(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = trimmed.to_ascii_lowercase();
    // Valid levels (from `claude --help`): low, medium, high, xhigh, max.
    // Pass the user's selection through verbatim so xhigh and max stay
    // distinct — previously we coerced xhigh → max.
    match normalized.as_str() {
        "default" | "auto" | "none" | "off" => None,
        "low" => Some("low".to_string()),
        "medium" | "med" => Some("medium".to_string()),
        "high" => Some("high".to_string()),
        "xhigh" | "x-high" => Some("xhigh".to_string()),
        "max" => Some("max".to_string()),
        _ => None,
    }
}

#[derive(Debug, Clone, Default)]
struct MiniMaxBackendRuntimeConfig {
    api_key: Option<String>,
}

fn load_minimax_backend_runtime_config() -> MiniMaxBackendRuntimeConfig {
    let config = crate::config::Config::load().unwrap_or_default();
    MiniMaxBackendRuntimeConfig {
        api_key: trim_to_option(config.local.minimax_api_key),
    }
}

#[derive(Debug, Clone, Default)]
struct GlmBackendRuntimeConfig {
    api_key: Option<String>,
}

fn load_glm_backend_runtime_config() -> GlmBackendRuntimeConfig {
    let config = crate::config::Config::load().unwrap_or_default();
    GlmBackendRuntimeConfig {
        api_key: trim_to_option(config.local.glm_api_key),
    }
}

fn build_pane_env_overrides(
    provider: &Provider,
    model: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    if !matches!(
        provider,
        Provider::Claude | Provider::Minimax | Provider::Glm
    ) {
        return Ok(Vec::new());
    }
    let is_minimax = matches!(provider, Provider::Minimax) || is_minimax_model(model);
    let is_glm = !is_minimax && (matches!(provider, Provider::Glm) || is_glm_model(model));
    if !is_minimax && !is_glm {
        return Ok(Vec::new());
    }

    let (api_base_url, api_key, missing_key_message) = if is_minimax {
        let runtime = load_minimax_backend_runtime_config();
        (
            MINIMAX_API_BASE_URL.to_string(),
            runtime.api_key,
            "MiniMax backend is not configured (missing minimax_api_key). Update it on the Machines page or run: apas config set minimax_api_key <key>.".to_string(),
        )
    } else {
        let runtime = load_glm_backend_runtime_config();
        (
            GLM_API_BASE_URL.to_string(),
            runtime.api_key,
            "GLM backend is not configured (missing glm_api_key). Update it on the Machines page or run: apas config set glm_api_key <key>.".to_string(),
        )
    };
    let api_key = api_key.ok_or(missing_key_message)?;

    let mut env = vec![
        ("ANTHROPIC_BASE_URL".to_string(), api_base_url),
        // Keep both names for compatibility across Claude CLI versions/wrappers.
        ("ANTHROPIC_AUTH_TOKEN".to_string(), api_key.clone()),
        ("ANTHROPIC_API_KEY".to_string(), api_key),
    ];
    if let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) {
        if is_minimax {
            env.push(("ANTHROPIC_MODEL".to_string(), model.to_string()));
        } else if is_glm {
            // Z.AI's Claude bridge expects model switching via default model
            // mapping variables instead of ANTHROPIC_MODEL for GLM-5.x.
            env.push((
                "ANTHROPIC_DEFAULT_SONNET_MODEL".to_string(),
                model.to_string(),
            ));
            env.push((
                "ANTHROPIC_DEFAULT_OPUS_MODEL".to_string(),
                model.to_string(),
            ));
            env.push((
                "ANTHROPIC_DEFAULT_HAIKU_MODEL".to_string(),
                GLM_DEFAULT_HAIKU_MODEL.to_string(),
            ));
        }
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

type PaneInput = (String, bool);

/// Per-pane input channel registry.
/// Maps pane_id -> Sender<PaneInput> for routing input to the correct session thread.
type InputChannels = Arc<Mutex<HashMap<u32, mpsc::Sender<PaneInput>>>>;

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

/// Run in tab-based mode
pub async fn run(server_url: &str, token: &str, working_dir: &Path) -> Result<()> {
    run_inner(server_url, token, working_dir, false).await
}

/// Run in headless mode — same as normal but without TUI (for daemon-spawned sessions)
pub async fn run_headless(server_url: &str, token: &str, working_dir: &Path) -> Result<()> {
    run_inner(server_url, token, working_dir, true).await
}

async fn run_inner(
    server_url: &str,
    token: &str,
    working_dir: &Path,
    headless: bool,
) -> Result<()> {
    if !headless {
        // Clear terminal screen for a clean start
        print!("\x1B[2J\x1B[H");
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }

    let config = crate::config::Config::load().unwrap_or_default();
    // Resolve binary paths to absolute paths at startup (while PATH is correct).
    // Systemd-run environments may have a minimal PATH that misses nvm/cargo bins.
    let claude_path = resolve_binary_path(&config.local.claude_path);
    // Legacy compatibility only: MiniMax now uses claude_path + backend env config.
    let minimax_path = resolve_binary_path(&config.local.minimax_path);
    let codex_path = resolve_binary_path(&config.local.codex_path);
    let opencode_path = resolve_binary_path(&config.local.opencode_path);
    let cursor_agent_path = resolve_binary_path(&config.local.cursor_agent_path);

    // Load or create project metadata
    let mut metadata = get_or_create_project(working_dir)?;
    let session_id = metadata.id;

    // Persist migrated metadata and ensure there is always at least one pane.
    if metadata.panes.is_empty() {
        metadata.panes = shared::PaneConfig::defaults();
    }
    save_project(working_dir, &metadata)?;

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
        Option<String>, // worktree_path from .apas (Phase 1.1)
    )> = metadata
        .panes
        .iter()
        .map(|pane| {
            // If a stop was requested but never finalized before a crash/restart,
            // restore as interactive to avoid accidentally re-starting bot mode.
            let mode = if pane.mode == shared::PaneMode::Deadloop && pane.stop_requested {
                shared::PaneMode::Interactive
            } else {
                pane.mode.clone()
            };
            let label =
                pane_label_or_default(pane.label.as_deref(), pane.pane_id, pane.model.as_deref());
            let is_paused = if mode == shared::PaneMode::Deadloop {
                if pane.pane_id == shared::PANE_ID_DEADLOOP {
                    pane.is_paused || metadata.is_paused
                } else {
                    pane.is_paused
                }
            } else {
                false
            };

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
            )
        })
        .collect();
    tabs_to_restore.sort_by_key(|(pane_id, ..)| *pane_id);

    // Channel for sending to server
    let (server_tx, server_rx) = tokio_mpsc::channel::<CliToServer>(256);

    // Shutdown flag
    let shutdown = Arc::new(AtomicBool::new(false));

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
        ) in &tabs_to_restore
        {
            let child_proc: Arc<Mutex<Option<std::process::Child>>> = Arc::new(Mutex::new(None));
            metas.insert(
                *pane_id,
                PaneMeta {
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
                },
            );
            sessions.insert(*pane_id, *pane_session_id);

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

    let initial_tabs: Vec<(u32, String, shared::PaneMode)> = tabs_to_restore
        .iter()
        .map(|(pane_id, _, label, mode, ..)| (*pane_id, label.clone(), mode.clone()))
        .collect();

    if initial_tabs.is_empty() {
        tracing::warn!("No panes available to restore; UI will start with no tabs.");
    }

    // Setup Ctrl+C handler
    let shutdown_for_handler = shutdown.clone();
    let metas_for_handler = pane_metas.clone();
    ctrlc::set_handler(move || {
        shutdown_for_handler.store(true, Ordering::SeqCst);
        // Kill all child processes
        if let Ok(metas) = metas_for_handler.lock() {
            for meta in metas.values() {
                if let Ok(mut guard) = meta.child_process.lock() {
                    if let Some(ref mut child) = *guard {
                        let _ = child.kill();
                    }
                }
            }
        }
    })?;

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
        let event_tx_for_server = event_tx.clone();
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
                status_tx,
                event_tx_for_server,
            )
            .await
        })
    };

    // Send initial messages for restored panes.
    for (pane_id, _, label, mode, _, _, _, _, _, is_paused, _) in &tabs_to_restore {
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
        let sid = session_id;
        let binary_path = resolve_pane_binary_path(
            provider,
            model.as_deref(),
            &claude_path,
            &minimax_path,
            &codex_path,
            &opencode_path,
            &cursor_agent_path,
        );
        let (interrupt_slot, control_resp_slot, pending_qs, effort_arc, worktree_path) = pane_metas
            .lock()
            .unwrap()
            .get(&pane_id)
            .map(|m| (
                m.streaming_interrupt_tx.clone(),
                m.control_response_tx.clone(),
                m.pending_questions.clone(),
                m.effort_arc.clone(),
                m.worktree_path.clone(),
            ))
            .unwrap_or_else(|| (
                Arc::new(Mutex::new(None)),
                Arc::new(Mutex::new(None)),
                Arc::new(Mutex::new(HashMap::new())),
                Arc::new(Mutex::new(None)),
                None,
            ));
        pane_threads.push(thread::spawn(move || {
            run_deadloop_session(
                &binary_path,
                &working_dir,
                worktree_path,
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
            )
        }));
    }

    for (pane_id, pane_session_id, provider, model, effort, input_rx, child_proc) in interactive_startups {
        let output_tx = output_tx.clone();
        let server_tx = server_tx.clone();
        let shutdown = shutdown.clone();
        let working_dir = working_dir_str.clone();
        let sid = session_id;
        let binary_path = resolve_pane_binary_path(
            provider,
            model.as_deref(),
            &claude_path,
            &minimax_path,
            &codex_path,
            &opencode_path,
            &cursor_agent_path,
        );
        let (interrupt_slot, control_resp_slot, pending_qs, effort_arc, worktree_path) = pane_metas
            .lock()
            .unwrap()
            .get(&pane_id)
            .map(|m| (
                m.streaming_interrupt_tx.clone(),
                m.control_response_tx.clone(),
                m.pending_questions.clone(),
                m.effort_arc.clone(),
                m.worktree_path.clone(),
            ))
            .unwrap_or_else(|| (
                Arc::new(Mutex::new(None)),
                Arc::new(Mutex::new(None)),
                Arc::new(Mutex::new(HashMap::new())),
                Arc::new(Mutex::new(None)),
                None,
            ));
        pane_threads.push(thread::spawn(move || {
            run_pane_session(
                &binary_path,
                &working_dir,
                worktree_path,
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
        let minimax_path_event = minimax_path.clone();
        let codex_path_event = codex_path.clone();
        let opencode_path_event = opencode_path.clone();
        let cursor_agent_path_event = cursor_agent_path.clone();
        let pane_sessions_event = pane_sessions.clone();
        let pane_pauses_event = pane_pauses.clone();
        let pane_stop_requests_event = pane_stop_requests.clone();
        let pane_metas_event = pane_metas.clone();
        let event_tx_event = event_tx.clone();
        let default_prompt_for_events = default_prompt.clone();
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
                &minimax_path_event,
                &codex_path_event,
                &opencode_path_event,
                &cursor_agent_path_event,
                &working_dir_event,
                command_tx,
                pane_sessions_event,
                pane_pauses_event,
                pane_stop_requests_event,
                pane_metas_event,
                &default_prompt_for_events,
            )
        })
    };

    if headless {
        // Headless mode: drain output (nobody reads it) and wait for server task
        drop(output_rx);
        drop(command_rx);
        drop(tui_input_tx);
        tracing::info!("Running in headless mode, waiting for server connection...");
        let _ = server_task.await;
        // Reboot request from web should also restart daemon-spawned headless CLIs.
        if reboot_requested.load(Ordering::SeqCst) {
            crate::update::restart_cli();
            std::process::exit(1);
        }
        shutdown.store(true, Ordering::SeqCst);
    } else {
        // Run TUI in main thread.
        let mut app = App::new(tui_input_tx, output_rx, event_tx, command_rx, initial_tabs)
            .with_shutdown(shutdown.clone());
        if let Err(e) = app.run() {
            tracing::error!("TUI error: {}", e);
        }
        // Signal shutdown
        shutdown.store(true, Ordering::SeqCst);

        // If reboot was requested, restart immediately
        if reboot_requested.load(Ordering::SeqCst) {
            server_task.abort();
            crate::update::restart_cli();
            std::process::exit(1);
        }

        server_task.abort();
    }

    // Kill all running child processes
    if let Ok(metas) = pane_metas.lock() {
        for meta in metas.values() {
            if let Ok(mut guard) = meta.child_process.lock() {
                if let Some(ref mut child) = *guard {
                    let _ = child.kill();
                }
            }
        }
    }

    // Wait for threads.
    for thread in pane_threads {
        let _ = thread.join();
    }
    let _ = tui_event_thread.join();

    Ok(())
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
                let worktree_path = pane_metas
                    .get(&pane_id)
                    .and_then(|p| p.worktree_path.clone());
                shared::PaneConfig {
                    pane_id,
                    provider,
                    mode,
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
                }
            })
            .collect();
        panes.sort_by_key(|p| p.pane_id);
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
    minimax_path: &str,
    codex_path: &str,
    opencode_path: &str,
    cursor_agent_path: &str,
    working_dir: &str,
    command_tx: mpsc::Sender<TuiCommand>,
    pane_sessions: Arc<Mutex<HashMap<u32, Uuid>>>,
    pane_pauses: PanePauses,
    pane_stop_requests: PaneStopRequests,
    pane_metas: PaneMetas,
    default_prompt: &str,
) {
    while !shutdown.load(Ordering::SeqCst) {
        match event_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(TuiEvent::AddTab) => {
                // Generate a new pane_id (TUI always creates Claude interactive tabs)
                let pane_id = 3 + (Uuid::new_v4().as_u128() % 1000) as u32;
                let claude_session_id = Uuid::new_v4();
                let label = format!("Tab {}", pane_id);
                let mode = shared::PaneMode::Interactive;
                let provider = Provider::Claude;

                // Create input channel — TUI and web input both flow through this
                let (input_tx, input_rx) = mpsc::channel::<PaneInput>();
                {
                    let mut channels = input_channels.lock().unwrap();
                    channels.insert(pane_id, input_tx);
                }

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
                            mode: mode.clone(),
                            provider: provider.clone(),
                            label: label.clone(),
                            prompt: None,
                            model: None,
                            effort: None,
                            min_iteration_interval_minutes: None,
                            child_process: child_proc.clone(),
                            streaming_interrupt_tx: Arc::new(Mutex::new(None)),
                            control_response_tx: Arc::new(Mutex::new(None)),
                            pending_questions: Arc::new(Mutex::new(HashMap::new())),
                            effort_arc: Arc::new(Mutex::new(None)),
                            worktree_path: None,
                        },
                    );
                }

                // Notify TUI to add the tab visually
                let _ = command_tx.send(TuiCommand::AddTab {
                    pane_id,
                    label: label.clone(),
                    mode: mode.clone(),
                });

                let _ = output_tx.send(PaneOutput {
                    text: format!("[New tab created: {}]", label),
                    pane_id,
                });

                // Spawn interactive session thread
                {
                    let output_tx = output_tx.clone();
                    let server_tx = server_tx.clone();
                    let shutdown = shutdown.clone();
                    let binary_path = resolve_pane_binary_path(
                        Provider::Claude,
                        None,
                        claude_path,
                        minimax_path,
                        codex_path,
                        opencode_path,
                        cursor_agent_path,
                    );
                    let working_dir = working_dir.to_string();
                    let (interrupt_slot, control_resp_slot, pending_qs, effort_arc, worktree_path) = pane_metas
                        .lock()
                        .unwrap()
                        .get(&pane_id)
                        .map(|m| (
                            m.streaming_interrupt_tx.clone(),
                            m.control_response_tx.clone(),
                            m.pending_questions.clone(),
                            m.effort_arc.clone(),
                            m.worktree_path.clone(),
                        ))
                        .unwrap_or_else(|| (
                            Arc::new(Mutex::new(None)),
                            Arc::new(Mutex::new(None)),
                            Arc::new(Mutex::new(HashMap::new())),
                            Arc::new(Mutex::new(None)),
                            None,
                        ));
                    thread::spawn(move || {
                        run_pane_session(
                            &binary_path,
                            &working_dir,
                            worktree_path,
                            session_id,
                            claude_session_id,
                            pane_id,
                            &Provider::Claude,
                            None,
                            None,
                            input_rx,
                            output_tx,
                            server_tx,
                            shutdown,
                            child_proc,
                            interrupt_slot,
                            control_resp_slot,
                            pending_qs,
                            effort_arc,
                        )
                    });
                }

                // Send pane list update
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

                // Persist to .apas
                save_pane_configs(
                    working_dir,
                    &pane_sessions,
                    &pane_metas,
                    &pane_pauses,
                    &pane_stop_requests,
                );
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
            }) => {
                let label =
                    pane_label_or_default(Some(&requested_label), pane_id, model.as_deref());
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
                        },
                    );
                }
                let binary_path = resolve_pane_binary_path(
                    provider,
                    model.as_deref(),
                    claude_path,
                    minimax_path,
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

                if mode == shared::PaneMode::Deadloop {
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
                    let (interrupt_slot, control_resp_slot, pending_qs, effort_arc, worktree_path) = pane_metas
                        .lock()
                        .unwrap()
                        .get(&pane_id)
                        .map(|m| (
                            m.streaming_interrupt_tx.clone(),
                            m.control_response_tx.clone(),
                            m.pending_questions.clone(),
                            m.effort_arc.clone(),
                            m.worktree_path.clone(),
                        ))
                        .unwrap_or_else(|| (
                            Arc::new(Mutex::new(None)),
                            Arc::new(Mutex::new(None)),
                            Arc::new(Mutex::new(HashMap::new())),
                            Arc::new(Mutex::new(None)),
                            None,
                        ));
                    thread::spawn(move || {
                        run_deadloop_session(
                            &binary_path,
                            &working_dir,
                            worktree_path,
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
                        )
                    });
                } else {
                    // Interactive tab: spawn interactive session
                    let (input_tx, input_rx) = mpsc::channel::<PaneInput>();
                    {
                        let mut channels = input_channels.lock().unwrap();
                        channels.insert(pane_id, input_tx);
                    }
                    let output_tx = output_tx.clone();
                    let server_tx = server_tx.clone();
                    let shutdown = shutdown.clone();
                    let working_dir = working_dir.to_string();
                    let (interrupt_slot, control_resp_slot, pending_qs, effort_arc, worktree_path) = pane_metas
                        .lock()
                        .unwrap()
                        .get(&pane_id)
                        .map(|m| (
                            m.streaming_interrupt_tx.clone(),
                            m.control_response_tx.clone(),
                            m.pending_questions.clone(),
                            m.effort_arc.clone(),
                            m.worktree_path.clone(),
                        ))
                        .unwrap_or_else(|| (
                            Arc::new(Mutex::new(None)),
                            Arc::new(Mutex::new(None)),
                            Arc::new(Mutex::new(HashMap::new())),
                            Arc::new(Mutex::new(None)),
                            None,
                        ));
                    thread::spawn(move || {
                        run_pane_session(
                            &binary_path,
                            &working_dir,
                            worktree_path,
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
                        )
                    });
                }

                // Send pane list update
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

                // Persist to .apas
                save_pane_configs(
                    working_dir,
                    &pane_sessions,
                    &pane_metas,
                    &pane_pauses,
                    &pane_stop_requests,
                );
            }
            Ok(TuiEvent::CloseTab { pane_id, cleanup_action }) => {
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
                let worktree_path: Option<String> = {
                    let metas = pane_metas.lock().unwrap();
                    let path = metas.get(&pane_id).and_then(|m| m.worktree_path.clone());
                    if let Some(meta) = metas.get(&pane_id) {
                        if let Ok(mut guard) = meta.child_process.lock() {
                            if let Some(ref mut child) = *guard {
                                let _ = child.kill();
                            }
                        }
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
                    let cleanup_msg = match crate::worktree::cleanup_on_close(
                        working_dir,
                        path,
                        action,
                    ) {
                        Ok(text) => text,
                        Err(err) => format!("[Worktree cleanup failed: {}]", err),
                    };
                    let _ = output_tx.send(PaneOutput {
                        text: cleanup_msg,
                        pane_id,
                    });
                }

                // Send pane list update
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

                // Persist to .apas
                save_pane_configs(
                    working_dir,
                    &pane_sessions,
                    &pane_metas,
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
                        ),
                        None => (
                            Provider::Claude,
                            default_pane_label(pane_id, None),
                            None,
                            None,
                            None,
                            None,
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
                            worktree_path: None,
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
                    minimax_path,
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
                    let (interrupt_slot, control_resp_slot, pending_qs, effort_arc, worktree_path) = pane_metas
                        .lock()
                        .unwrap()
                        .get(&pane_id)
                        .map(|m| (
                            m.streaming_interrupt_tx.clone(),
                            m.control_response_tx.clone(),
                            m.pending_questions.clone(),
                            m.effort_arc.clone(),
                            m.worktree_path.clone(),
                        ))
                        .unwrap_or_else(|| (
                            Arc::new(Mutex::new(None)),
                            Arc::new(Mutex::new(None)),
                            Arc::new(Mutex::new(HashMap::new())),
                            Arc::new(Mutex::new(None)),
                            None,
                        ));
                    thread::spawn(move || {
                        run_deadloop_session(
                            &binary_path,
                            &working_dir,
                            worktree_path,
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
                                let soft = m.streaming_interrupt_tx.lock().ok()
                                    .and_then(|g| g.as_ref().cloned());
                                let present = m.child_process.lock().ok()
                                    .map(|g| g.is_some()).unwrap_or(false);
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
                                let soft = m.streaming_interrupt_tx.lock().ok()
                                    .and_then(|g| g.as_ref().cloned());
                                let pid = m.child_process.lock().ok()
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
                                    let alive = std::path::Path::new(&format!(
                                        "/proc/{}", pid
                                    )).exists();
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
                    minimax_path,
                    codex_path,
                    opencode_path,
                    cursor_agent_path,
                );
                {
                    let output_tx = output_tx.clone();
                    let server_tx = server_tx.clone();
                    let shutdown = shutdown.clone();
                    let working_dir = working_dir.to_string();
                    let (interrupt_slot, control_resp_slot, pending_qs, effort_arc, worktree_path) = pane_metas
                        .lock()
                        .unwrap()
                        .get(&pane_id)
                        .map(|m| (
                            m.streaming_interrupt_tx.clone(),
                            m.control_response_tx.clone(),
                            m.pending_questions.clone(),
                            m.effort_arc.clone(),
                            m.worktree_path.clone(),
                        ))
                        .unwrap_or_else(|| (
                            Arc::new(Mutex::new(None)),
                            Arc::new(Mutex::new(None)),
                            Arc::new(Mutex::new(HashMap::new())),
                            Arc::new(Mutex::new(None)),
                            None,
                        ));
                    thread::spawn(move || {
                        run_pane_session(
                            &binary_path,
                            &working_dir,
                            worktree_path,
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
            session_id: claude_sid,
            is_paused,
            stop_requested,
            prompt: meta.prompt.clone(),
            min_iteration_interval_minutes: meta.min_iteration_interval_minutes,
            label: Some(label),
            model: meta.model.clone(),
            effort: meta.effort.clone(),
            worktree_path: meta.worktree_path.clone(),
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
                session_id: claude_sid,
                is_paused: false,
                stop_requested: false,
                prompt: None,
                min_iteration_interval_minutes: None,
                label: Some(default_pane_label(pane_id, None)),
                model: None,
                effort: None,
                worktree_path: None,
            });
        }
    }

    panes.sort_by_key(|p| p.pane_id);
    panes
}

fn active_usage_providers(pane_metas: &PaneMetas) -> (bool, bool, bool, bool) {
    let metas = pane_metas.lock().unwrap();
    let mut has_claude = false;
    let mut has_codex = false;
    let mut has_minimax = false;
    let mut has_glm = false;

    let looks_like_minimax_label = |label: &str| {
        let normalized = label.trim().to_ascii_lowercase();
        normalized.contains("minimax") || normalized.contains("mini max")
    };
    let looks_like_glm_label = |label: &str| {
        let normalized = label.trim().to_ascii_lowercase();
        normalized.contains("glm")
            || normalized.contains("z.ai")
            || normalized.contains("zai")
            || normalized.contains("zhipu")
    };

    for meta in metas.values() {
        match meta.provider {
            // MiniMax tabs run through Claude CLI transport, but Anthropic usage
            // limits are not meaningful for them.
            Provider::Claude
                if is_minimax_model(meta.model.as_deref()) || looks_like_minimax_label(&meta.label) =>
            {
                has_minimax = true
            }
            // GLM tabs also run through Claude transport and should not map to
            // Anthropic usage limits.
            Provider::Claude
                if is_glm_model(meta.model.as_deref()) || looks_like_glm_label(&meta.label) =>
            {
                has_glm = true
            }
            Provider::Claude => has_claude = true,
            Provider::Codex => has_codex = true,
            Provider::Minimax => has_minimax = true,
            Provider::Glm => has_glm = true,
            Provider::Opencode => {}
            Provider::CursorAgent => {}
        }
        if has_claude && has_codex && has_minimax && has_glm {
            break;
        }
    }

    (has_claude, has_codex, has_minimax, has_glm)
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
    match provider {
        Provider::Claude | Provider::Minimax | Provider::Glm => {
            let mut base = vec![
                "--print".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--verbose".to_string(),
                "--dangerously-skip-permissions".to_string(),
            ];
            if let Some(model) = model {
                let trimmed = model.trim();
                // MiniMax/GLM panes use dedicated backend env configuration.
                // Keep model selection in env (ANTHROPIC_MODEL), not CLI flags.
                if !trimmed.is_empty()
                    && !is_minimax_model(Some(trimmed))
                    && !is_glm_model(Some(trimmed))
                {
                    base.extend_from_slice(&["--model".to_string(), trimmed.to_string()]);
                }
            }
            if matches!(provider, Provider::Claude)
                && !is_minimax_model(model)
                && !is_glm_model(model)
            {
                if let Some(normalized_effort) = normalize_effort_level(effort) {
                    tracing::info!(
                        target: "apas::effort",
                        effort = %normalized_effort,
                        "Launching claude with --effort",
                    );
                    base.extend_from_slice(&["--effort".to_string(), normalized_effort]);
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
            let base_flags = vec![
                "--json".to_string(),
                "--dangerously-bypass-approvals-and-sandbox".to_string(),
                "--skip-git-repo-check".to_string(),
            ];
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
        Provider::Opencode => {
            // OpenCode uses: opencode run --format json [-m model] [-c -s session_id] -- <prompt>
            let mut base = vec!["run".to_string(), "--format".to_string(), "json".to_string()];
            if let Some(model) = model {
                let trimmed = model.trim();
                if !trimmed.is_empty() {
                    base.extend_from_slice(&["-m".to_string(), trimmed.to_string()]);
                }
            }
            if first_message && try_resume {
                base.extend_from_slice(&[
                    "-c".to_string(),
                    "-s".to_string(),
                    session_id.to_string(),
                    "--".to_string(),
                    prompt.to_string(),
                ]);
                (base, true)
            } else if first_message {
                base.extend_from_slice(&["--".to_string(), prompt.to_string()]);
                (base, false)
            } else {
                // Subsequent messages — always resume
                base.extend_from_slice(&[
                    "-c".to_string(),
                    "-s".to_string(),
                    session_id.to_string(),
                    "--".to_string(),
                    prompt.to_string(),
                ]);
                (base, true)
            }
        }
    }
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
    _session_id: Uuid,
    _pane_type: PaneType,
    pending_questions: &Arc<Mutex<HashMap<String, PendingAskQuestion>>>,
    control_response_tx: &mpsc::Sender<String>,
    _server_tx: &tokio_mpsc::Sender<CliToServer>,
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
    let subtype = request.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
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
    let input = request.get("input").cloned().unwrap_or(serde_json::Value::Null);

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
    let subtype = response.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
    let text = if subtype == "success" {
        "[✓ Effort change confirmed by claude]".to_string()
    } else {
        let err = response.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error");
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

/// Parse a line of output and convert to ClaudeStreamMessage based on provider.
/// For Codex, parses as CodexStreamMessage and converts.
fn parse_agent_output(
    provider: &Provider,
    line: &str,
    session_id_str: &str,
) -> Option<ClaudeStreamMessage> {
    match provider {
        Provider::Claude | Provider::Minimax | Provider::Glm => {
            serde_json::from_str::<ClaudeStreamMessage>(line).ok()
        }
        Provider::Codex => match serde_json::from_str::<CodexStreamMessage>(line) {
            Ok(codex_msg) => shared::convert_codex_to_claude(&codex_msg, session_id_str),
            Err(_) => None,
        },
        Provider::Opencode => {
            // OpenCode --format json outputs JSON lines; try parsing as ClaudeStreamMessage
            serde_json::from_str::<ClaudeStreamMessage>(line).ok()
        }
        Provider::CursorAgent => {
            // cursor-agent --output-format stream-json emits Claude-compatible events
            serde_json::from_str::<ClaudeStreamMessage>(line).ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        active_usage_providers, build_agent_args, pane_label_or_default, resolve_pane_binary_path,
        PaneMeta, PaneMetas,
    };
    use shared::Provider;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    const FULL_PROMPT: &str =
        "Work on tasks defined in TODO.md.\n1. Analyze\n2. Implement\n3. Test";

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
    fn build_agent_args_claude_with_non_minimax_model_includes_model_flag() {
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
    fn build_agent_args_claude_with_minimax_model_omits_model_flag() {
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

        assert!(!args.iter().any(|arg| arg == "--model"));
    }

    #[test]
    fn build_agent_args_claude_with_glm_model_omits_model_flag() {
        let session_id = Uuid::new_v4();
        let (args, _) = build_agent_args(
            &Provider::Claude,
            &session_id,
            FULL_PROMPT,
            Some("glm-5.1"),
            None,
            true,
            false,
        );

        assert!(!args.iter().any(|arg| arg == "--model"));
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
    fn build_agent_args_claude_minimax_backend_ignores_effort_flag() {
        let session_id = Uuid::new_v4();
        let (args, _) = build_agent_args(
            &Provider::Claude,
            &session_id,
            FULL_PROMPT,
            Some("MiniMax-M2.7"),
            Some("max"),
            true,
            false,
        );

        assert!(!args.iter().any(|arg| arg == "--effort"));
    }

    #[test]
    fn resolve_pane_binary_path_uses_claude_for_minimax_model() {
        let path = resolve_pane_binary_path(
            Provider::Claude,
            Some("MiniMax-M2.7"),
            "claude",
            "claude2",
            "codex",
            "opencode",
            "cursor-agent",
        );
        assert_eq!(path, "claude");
    }

    #[test]
    fn resolve_pane_binary_path_uses_claude_for_m2_alias_model() {
        let path =
            resolve_pane_binary_path(Provider::Claude, Some("m2.7"), "claude", "claude2", "codex", "opencode", "cursor-agent");
        assert_eq!(path, "claude");
    }

    #[test]
    fn resolve_pane_binary_path_uses_default_claude_for_non_minimax_model() {
        let path = resolve_pane_binary_path(
            Provider::Claude,
            Some("sonnet"),
            "claude",
            "claude2",
            "codex",
            "opencode",
            "cursor-agent",
        );
        assert_eq!(path, "claude");
    }

    #[test]
    fn resolve_pane_binary_path_keeps_absolute_claude_path_for_minimax_model() {
        let path = resolve_pane_binary_path(
            Provider::Claude,
            Some("MiniMax-M2.7"),
            "/opt/bin/claude",
            "claude2",
            "codex",
            "opencode",
            "cursor-agent",
        );
        assert_eq!(path, "/opt/bin/claude");
    }

    #[test]
    fn pane_label_or_default_rebrands_generic_minimax_tab_label() {
        let label = pane_label_or_default(Some("Tab 42"), 42, Some("MiniMax-M2.7"));
        assert_eq!(label, "MiniMax 42");
    }

    #[test]
    fn pane_label_or_default_preserves_custom_label() {
        let label = pane_label_or_default(Some("Research"), 42, Some("MiniMax-M2.7"));
        assert_eq!(label, "Research");
    }

    #[test]
    fn pane_label_or_default_rebrands_generic_glm_tab_label() {
        let label = pane_label_or_default(Some("Tab 11"), 11, Some("glm-5.1"));
        assert_eq!(label, "GLM 11");
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
                },
            );
            metas.insert(
                2,
                PaneMeta {
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
                },
            );
        }

        assert_eq!(
            active_usage_providers(&pane_metas),
            (true, true, false, false)
        );
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
                },
            );
        }

        assert_eq!(
            active_usage_providers(&pane_metas),
            (false, false, true, false)
        );
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
                },
            );
            metas.insert(
                2,
                PaneMeta {
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
                },
            );
        }

        assert_eq!(
            active_usage_providers(&pane_metas),
            (false, true, true, false)
        );
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
                },
            );
        }

        assert_eq!(
            active_usage_providers(&pane_metas),
            (false, false, true, false)
        );
    }

    #[test]
    fn active_usage_providers_detects_glm_only_claude() {
        let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::new()));
        let child_process = Arc::new(Mutex::new(None));

        {
            let mut metas = pane_metas.lock().unwrap();
            metas.insert(
                9,
                PaneMeta {
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
                },
            );
        }

        assert_eq!(
            active_usage_providers(&pane_metas),
            (false, false, false, true)
        );
    }

    #[test]
    fn active_usage_providers_detects_glm_label_without_model() {
        let pane_metas: PaneMetas = Arc::new(Mutex::new(HashMap::new()));
        let child_process = Arc::new(Mutex::new(None));

        {
            let mut metas = pane_metas.lock().unwrap();
            metas.insert(
                10,
                PaneMeta {
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
                },
            );
        }

        assert_eq!(
            active_usage_providers(&pane_metas),
            (false, false, false, true)
        );
    }

    #[test]
    fn control_request_parser_parks_ask_user_question() {
        use super::{try_handle_control_request, PendingAskQuestion};
        use shared::CliToServer;
        use std::sync::{Arc, Mutex};
        use std::collections::HashMap;

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
        use std::sync::{Arc, Mutex};
        use std::collections::HashMap;

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
        );
        assert_eq!(handled, Some(true));
        let payload = cr_rx.try_recv().expect("auto-approve should send a control_response");
        assert!(payload.contains("\"behavior\":\"allow\""));
        assert!(payload.contains("\"request_id\":\"req-2\""));
        assert!(payload.contains("\"toolUseID\":\"toolu_xyz\""));
    }

    #[test]
    fn control_request_parser_ignores_non_control_lines() {
        use super::{try_handle_control_request, PendingAskQuestion};
        use shared::CliToServer;
        use std::sync::{Arc, Mutex};
        use std::collections::HashMap;

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
        );
        assert_eq!(handled, None);
    }
}

/// Run the deadloop (autonomous) session on any pane
#[allow(clippy::too_many_arguments)]
fn run_deadloop_session(
    binary_path: &str,
    working_dir: &str,
    worktree_path: Option<String>,
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
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_deadloop_session_inner(
            binary_path,
            working_dir,
            worktree_path,
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

#[allow(clippy::too_many_arguments)]
fn run_deadloop_session_inner(
    binary_path: &str,
    working_dir: &str,
    worktree_path: Option<String>,
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
) {
    // Provider::Claude → long-lived stream-json process driven from
    // run_deadloop_session_streaming. Other providers fall through to the
    // legacy per-iteration --print spawn below.
    if matches!(provider, Provider::Claude) {
        return run_deadloop_session_streaming(
            binary_path,
            working_dir,
            worktree_path,
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
        );
    }
    let effective_dir: String = worktree_path
        .as_deref()
        .unwrap_or(working_dir)
        .to_string();
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
    let mut try_resume_first = true;
    let mut was_paused = false;
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
            if let Some(mut remaining) =
                min_iteration_interval.checked_sub(last_started_at.elapsed())
            {
                if !remaining.is_zero() {
                    let _ = output_tx.send(PaneOutput {
                        text: format!(
                            "[Waiting {}s before next iteration (min interval: {}m)]",
                            remaining.as_secs(),
                            min_iteration_interval_minutes
                        ),
                        pane_id,
                    });
                }
                while !remaining.is_zero() {
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
                        break;
                    }

                    let sleep_for = std::cmp::min(remaining, Duration::from_millis(500));
                    thread::sleep(sleep_for);
                    remaining = min_iteration_interval
                        .checked_sub(last_started_at.elapsed())
                        .unwrap_or(Duration::ZERO);
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

        let (args, using_resume) = build_agent_args(
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

        // Kill any lingering Claude processes that still hold this session ID.
        // This can happen after Stop→Start when the old process wasn't fully reaped.
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
        // Deadloop only: put the agent in its own process group so we can
        // group-kill it (and any background children it spawned) the moment
        // it emits its result event. The deadloop must iterate; an agent
        // that lingers past result wedges every subsequent iteration.
        // Interactive panes deliberately don't do this — users may want to
        // launch persistent background work that outlives the turn.
        #[cfg(unix)]
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

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
                let stderr_thread = stderr.map(|stderr| {
                    thread::spawn(move || {
                        let reader = BufReader::new(stderr);
                        for line in reader.lines() {
                            if let Ok(line) = line {
                                if !line.trim().is_empty() {
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
                                    let is_result = matches!(
                                        message,
                                        ClaudeStreamMessage::Result { .. }
                                    );
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
                    if first_message && using_resume && exit_was_error && !had_error {
                        // Process failed to start (e.g. session not found).
                        // Generate a new session ID and create a fresh session.
                        claude_session_id = Uuid::new_v4();
                        try_resume_first = false;
                        let _ = output_tx.send(PaneOutput {
                            text: "[Session not found, will create new session...]".to_string(),
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
fn session_jsonl_exists(working_dir: &str, session_id: &Uuid) -> bool {
    let Some(home) = std::env::var_os("HOME") else { return false };
    let encoded: String = working_dir
        .chars()
        .map(|c| if c == '/' { '-' } else { c })
        .collect();
    let path = std::path::Path::new(&home)
        .join(".claude")
        .join("projects")
        .join(encoded)
        .join(format!("{}.jsonl", session_id));
    path.exists()
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
    let stop_marker_path = std::path::PathBuf::from(format!(
        "/tmp/apas-stop-marks/{}",
        session_id
    ));

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
            let Some(stop_mtime) = stop_mtime else { continue };
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
    let Some(home) = std::env::var_os("HOME") else { return };
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
                let Some(message) = parse_agent_output(
                    &Provider::Claude,
                    line,
                    &session_id_str,
                ) else {
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
    let effective_dir: String = worktree_path
        .as_deref()
        .unwrap_or(working_dir)
        .to_string();

    let _ = output_tx.send(PaneOutput {
        text: format!("[Session: {} (streaming)]", &claude_session_id.to_string()[..8]),
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
            if !is_minimax_model(Some(m)) && !is_glm_model(Some(m)) {
                args.push("--model".into());
                args.push(m.to_string());
            }
        }
        if !is_minimax_model(model.as_deref()) && !is_glm_model(model.as_deref()) {
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
                tracing::info!(
                    target: "apas::effort",
                    pane_id,
                    effort = %eff,
                    "Launching streaming claude with --effort",
                );
                args.push("--effort".into());
                args.push(eff);
            }
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
                if let Some(handled) = try_handle_control_request(
                    &line,
                    pane_id_reader,
                    session_id_reader,
                    pane_type_reader,
                    &reader_pending_questions,
                    &reader_control_response_tx,
                    &server_tx_reader,
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
                        match &message {
                            ClaudeStreamMessage::Assistant { message: msg, .. } => {
                                for block in &msg.content {
                                    if let ClaudeContentBlock::ToolUse { id, name, input } = block {
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
                                            if let Some(tid) = input
                                                .get("task_id")
                                                .and_then(|v| v.as_str())
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
                                    if let ClaudeContentBlock::ToolResult {
                                        tool_use_id,
                                        ..
                                    } = block
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
            let next_prompt: Option<(String, bool, bool)> = if !busy {
                pending.pop_front()
            } else {
                None
            };

            match next_prompt {
                Some((prompt, from_tui, is_auto_wake)) => {
                    let display_prefix = if is_auto_wake { "[auto-wake]" } else { ">" };
                    let _ = output_tx.send(PaneOutput {
                        text: format!(
                            "{} {}",
                            display_prefix,
                            &prompt[..std::cmp::min(140, prompt.len())]
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
                    let n_subagents = in_flight_subagents
                        .lock()
                        .map(|s| s.len())
                        .unwrap_or(0);
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
                    // Wire format from happy-cli/src/claude/sdk/utils.ts:190.
                    // Plain string content, not a content-block array.
                    let envelope = serde_json::json!({
                        "type": "user",
                        "message": { "role": "user", "content": prompt },
                    });
                    let line = format!("{}\n", envelope);
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
) {
    let _ = output_tx.send(PaneOutput {
        text: format!(
            "[Streaming /loop deadloop session: {}]",
            &claude_session_id.to_string()[..8]
        ),
        pane_id,
    });

    let (input_tx, input_rx) = mpsc::channel::<PaneInput>();

    // Spawn the streaming worker. It keeps claude alive; we kick off /loop
    // exactly once and the runtime self-paces from there. We pass
    // `result_signal_tx = None` because we don't gate on iterations — the
    // agent picks its own cadence via ScheduleWakeup.
    {
        let binary_path = binary_path.to_string();
        let working_dir = working_dir.to_string();
        let worktree_path = worktree_path.clone();
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
    // claude on its own; we don't fire any further prompts.
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

        // Pause is poorly defined for /loop (the agent paces itself; we
        // can't easily withhold the next iteration without killing the
        // process). We surface a one-line note the first time pause is
        // requested and otherwise let the loop run.
        if pause.load(Ordering::SeqCst) {
            if !was_paused {
                was_paused = true;
                let _ = output_tx.send(PaneOutput {
                    text: "[Note: pause not directly supported for /loop bots — the agent paces itself. Use Stop to interrupt.]".to_string(),
                    pane_id,
                });
            }
        } else if was_paused {
            was_paused = false;
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
) {
    // Provider::Claude → long-lived stream-json process. Other providers
    // (Codex, Cursor, OpenCode, MiniMax, GLM) → legacy per-turn --print
    // spawn below.
    if matches!(provider, Provider::Claude) {
        return run_pane_session_streaming(
            binary_path,
            working_dir,
            worktree_path,
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
    let effective_dir: String = worktree_path
        .as_deref()
        .unwrap_or(working_dir)
        .to_string();
    let _ = interrupt_tx_slot; // unused for legacy path
    let _ = control_response_tx_slot; // unused for legacy path
    let _ = pending_questions; // unused for legacy path
    let _ = effort_arc; // unused for legacy path

    let mut first_message = true;
    let mut try_resume_first = true;
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
            text: format!("> {}", &prompt[..std::cmp::min(100, prompt.len())]),
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
                                    let is_result = matches!(
                                        message,
                                        ClaudeStreamMessage::Result { .. }
                                    );
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
                        format!("[Session resume failed ({}), will create new session on next message...]", exit_msg)
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

/// Run server connection with automatic reconnection
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
    status_tx: mpsc::Sender<PaneOutput>,
    tui_event_tx: mpsc::Sender<TuiEvent>,
) -> Result<()> {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    const USAGE_FETCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5 * 60);
    const USAGE_CACHE_MAX_AGE_MINUTES: i64 = 45;

    let mut reconnect_delay = std::time::Duration::from_secs(1);
    let max_reconnect_delay = std::time::Duration::from_secs(60);
    let mut connection_count = 0u32;

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
                connection_count += 1;
                reconnect_delay = std::time::Duration::from_secs(1);
                let (mut ws_sender, mut ws_receiver) = ws_stream.split();

                // Register
                let register_msg = CliToServer::Register {
                    token: token.to_string(),
                    version: Some(env!("APAS_VERSION").to_string()),
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
                                            return Some(Ok(cli_id))
                                        }
                                        ServerToCli::RegistrationFailed { reason } => {
                                            return Some(Err(reason))
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
                    working_dir: Some(working_dir.to_string()),
                    hostname,
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

                // Send PaneList separately too
                let pane_list_msg = CliToServer::PaneList {
                    session_id,
                    panes: pane_list,
                };
                let msg_text = serde_json::to_string(&pane_list_msg).unwrap_or_default();
                let _ = ws_sender.send(Message::Text(msg_text.into())).await;

                let mut heartbeat_interval =
                    tokio::time::interval(std::time::Duration::from_secs(25));
                heartbeat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                heartbeat_interval.tick().await;

                let mut usage_interval = tokio::time::interval(USAGE_FETCH_INTERVAL);
                usage_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

                // Main loop
                loop {
                    tokio::select! {
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
                        msg = ws_receiver.next() => {
                            match msg {
                                Some(Ok(Message::Text(text))) => {
                                    if let Ok(server_msg) = serde_json::from_str::<ServerToCli>(&text) {
                                        match server_msg {
                                            ServerToCli::SessionRejected { session_id: rejected_id, reason } => {
                                                eprintln!(
                                                    "\n[APAS] Server rejected session {}: {}\n",
                                                    rejected_id, reason
                                                );
                                                tracing::error!(
                                                    "Server rejected session {}: {}",
                                                    rejected_id, reason
                                                );
                                                // Exit cleanly so systemd-run / the TUI surfaces the error.
                                                std::process::exit(2);
                                            }
                                            ServerToCli::Input { session_id: _, data, pane_id } => {
                                                // Route to the correct pane (from_tui=false: web-originated)
                                                let target_pane = pane_id.unwrap_or(shared::PANE_ID_INTERACTIVE);
                                                let target_tx = {
                                                    let channels = input_channels.lock().unwrap();
                                                    channels.get(&target_pane).cloned()
                                                };

                                                if let Some(tx) = target_tx {
                                                    if tx.send((data, false)).is_err() {
                                                        tracing::warn!(
                                                            pane_id = target_pane,
                                                            "Input channel disconnected for pane"
                                                        );
                                                        let _ = status_tx.send(PaneOutput {
                                                            text: "[Pane input channel disconnected. Restarting pane worker...]".to_string(),
                                                            pane_id: target_pane,
                                                        });
                                                    }
                                                    continue;
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

                                                    if let (Some(meta), Some(claude_session_id)) =
                                                        (pane_meta, pane_session_id)
                                                    {
                                                        tracing::warn!(
                                                            pane_id = target_pane,
                                                            "Missing input channel for pane; requesting pane worker recreation"
                                                        );
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
                                                        });
                                                    } else {
                                                        tracing::warn!(
                                                            pane_id = target_pane,
                                                            "Missing input channel for pane and no pane metadata found"
                                                        );
                                                    }

                                                    let unavailable_status = format!(
                                                        "[Pane {} is unavailable. Restarting pane worker; please resend your message.]",
                                                        target_pane
                                                    );
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
                                                            "Pane worker unavailable; restart requested. Please resend."
                                                                .to_string(),
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
                                                let pane_msg = CliToServer::PanePaused { session_id, pane_id: target_pane, is_paused: true };
                                                let msg_text = serde_json::to_string(&pane_msg).unwrap_or_default();
                                                let _ = ws_sender.send(Message::Text(msg_text.into())).await;
                                                // Legacy compat: also send DeadloopStatus for pane 1
                                                if target_pane == shared::PANE_ID_DEADLOOP {
                                                    if let Ok(mut metadata) = get_or_create_project(std::path::Path::new(working_dir)) {
                                                        metadata.is_paused = true;
                                                        let _ = save_project(std::path::Path::new(working_dir), &metadata);
                                                    }
                                                    let status_msg = CliToServer::DeadloopStatus { session_id, is_paused: true };
                                                    let msg_text = serde_json::to_string(&status_msg).unwrap_or_default();
                                                    let _ = ws_sender.send(Message::Text(msg_text.into())).await;
                                                }
                                            }
                                            ServerToCli::ResumePane { session_id: _, pane_id: target_pane } => {
                                                if let Ok(pauses) = pane_pauses.lock() {
                                                    if let Some(flag) = pauses.get(&target_pane) {
                                                        flag.store(false, Ordering::SeqCst);
                                                    }
                                                }
                                                let _ = status_tx.send(PaneOutput {
                                                    text: format!("[Pane {} resumed from web]", target_pane),
                                                    pane_id: target_pane,
                                                });
                                                let pane_msg = CliToServer::PanePaused { session_id, pane_id: target_pane, is_paused: false };
                                                let msg_text = serde_json::to_string(&pane_msg).unwrap_or_default();
                                                let _ = ws_sender.send(Message::Text(msg_text.into())).await;
                                                // Legacy compat: also send DeadloopStatus for pane 1
                                                if target_pane == shared::PANE_ID_DEADLOOP {
                                                    if let Ok(mut metadata) = get_or_create_project(std::path::Path::new(working_dir)) {
                                                        metadata.is_paused = false;
                                                        let _ = save_project(std::path::Path::new(working_dir), &metadata);
                                                    }
                                                    let status_msg = CliToServer::DeadloopStatus { session_id, is_paused: false };
                                                    let msg_text = serde_json::to_string(&status_msg).unwrap_or_default();
                                                    let _ = ws_sender.send(Message::Text(msg_text.into())).await;
                                                }
                                            }
                                            ServerToCli::AddPane { session_id: _, pane_config, isolated_worktree } => {
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
                                                });
                                            }
                                            ServerToCli::RemovePane { session_id: _, pane_id: remove_id, cleanup_action } => {
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
                                                reboot_requested.store(true, Ordering::SeqCst);
                                                shutdown.store(true, Ordering::SeqCst);
                                                return Ok(());
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
                                                            let req = serde_json::json!({
                                                                "type": "control_request",
                                                                "request_id": format!("apas-effort-{}", uuid::Uuid::new_v4()),
                                                                "request": {
                                                                    "subtype": "apply_flag_settings",
                                                                    "settings": { "effortLevel": level },
                                                                },
                                                            });
                                                            if tx.send(req.to_string()).is_ok() {
                                                                tracing::info!(
                                                                    pane_id = target_pane,
                                                                    effort = %level,
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
                                                if !handled {
                                                    tracing::warn!(
                                                        tool_use_id = tool_use_id.as_str(),
                                                        "AnswerQuestion: no matching pending AskUserQuestion (already answered or expired)",
                                                    );
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
                            let (has_claude, has_codex, has_minimax, has_glm) =
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

                            if has_minimax {
                                if let Some(limits) =
                                    crate::usage::read_cached_minimax_usage_limits(Some(max_age))
                                {
                                    let usage_msg = CliToServer::UsageLimits { provider: Provider::Minimax, limits };
                                    let msg_text = serde_json::to_string(&usage_msg).unwrap_or_default();
                                    if ws_sender.send(Message::Text(msg_text.into())).await.is_err() {
                                        tracing::warn!("Failed to send MiniMax usage limits to server");
                                    }
                                } else {
                                    tracing::debug!("No fresh cached MiniMax usage limits available");
                                }
                            }

                            // Always publish fresh cached GLM usage when available.
                            // GLM usage comes from daemon-level polling and should be visible
                            // on Machines page even if current pane metadata is legacy/missing.
                            if let Some(limits) =
                                crate::usage::read_cached_glm_usage_limits(Some(max_age))
                            {
                                let usage_msg = CliToServer::UsageLimits {
                                    provider: Provider::Glm,
                                    limits,
                                };
                                let msg_text = serde_json::to_string(&usage_msg).unwrap_or_default();
                                if ws_sender.send(Message::Text(msg_text.into())).await.is_err() {
                                    tracing::warn!("Failed to send GLM usage limits to server");
                                }
                            } else if has_glm {
                                tracing::debug!("No fresh cached GLM usage limits available");
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

/// Helper to get claude path from config
fn config_claude_path(_working_dir: &str) -> String {
    crate::config::Config::load()
        .unwrap_or_default()
        .local
        .claude_path
}
