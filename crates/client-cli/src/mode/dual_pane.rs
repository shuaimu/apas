//! Tab-based mode: Multiple independent Claude sessions as tabs
//!
//! New projects start with one default tab:
//! - Interactive session
//!
//! Users can create and close tabs dynamically from both TUI and web UI.

use anyhow::Result;
use shared::{
    ClaudeStreamMessage, CliToServer, CodexStreamMessage, PaneType, Provider, ServerToCli,
};
use std::collections::HashMap;
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
    for (pane_id, _, label, mode, _, _, _, _, _, is_paused) in &tabs_to_restore {
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
        pane_threads.push(thread::spawn(move || {
            run_deadloop_session(
                &binary_path,
                &working_dir,
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
        pane_threads.push(thread::spawn(move || {
            run_pane_session(
                &binary_path,
                &working_dir,
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
                    thread::spawn(move || {
                        run_pane_session(
                            &binary_path,
                            &working_dir,
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
                    thread::spawn(move || {
                        run_deadloop_session(
                            &binary_path,
                            &working_dir,
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
                    thread::spawn(move || {
                        run_pane_session(
                            &binary_path,
                            &working_dir,
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
            Ok(TuiEvent::CloseTab(pane_id)) => {
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
                {
                    let metas = pane_metas.lock().unwrap();
                    if let Some(meta) = metas.get(&pane_id) {
                        if let Ok(mut guard) = meta.child_process.lock() {
                            if let Some(ref mut child) = *guard {
                                let _ = child.kill();
                            }
                        }
                    }
                }

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
                    thread::spawn(move || {
                        run_deadloop_session(
                            &binary_path,
                            &working_dir,
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

                // For interactive panes: force-kill the child process directly
                if pane_mode == shared::PaneMode::Interactive {
                    {
                        let metas = pane_metas.lock().unwrap();
                        if let Some(meta) = metas.get(&pane_id) {
                            if let Ok(mut guard) = meta.child_process.lock() {
                                if let Some(ref mut child) = *guard {
                                    let _ = child.kill();
                                }
                            }
                        }
                    }
                    let _ = output_tx.send(PaneOutput {
                        text: "[Process killed]".to_string(),
                        pane_id,
                    });
                    let _ = server_tx.blocking_send(CliToServer::StreamMessage {
                        session_id,
                        message: shared::ClaudeStreamMessage::Result {
                            subtype: "text".to_string(),
                            result: "[Process killed]".to_string(),
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
                    // Kill the deadloop child process and wait for full exit
                    {
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
                    thread::spawn(move || {
                        run_pane_session(
                            &binary_path,
                            &working_dir,
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
                },
            );
        }

        assert_eq!(
            active_usage_providers(&pane_metas),
            (false, false, false, true)
        );
    }
}

/// Run the deadloop (autonomous) session on any pane
fn run_deadloop_session(
    binary_path: &str,
    working_dir: &str,
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
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_deadloop_session_inner(
            binary_path,
            working_dir,
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

fn run_deadloop_session_inner(
    binary_path: &str,
    working_dir: &str,
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
) {
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
            Ok(env) => env,
            Err(err) => {
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
                thread::sleep(Duration::from_secs(2));
                continue;
            }
        };

        let mut command = Command::new(binary_path);
        command
            .args(&args)
            .current_dir(working_dir)
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

/// Run a generic interactive pane session.
/// Input comes from a single channel — both TUI and web input are routed through input_channels.
fn run_pane_session(
    binary_path: &str,
    working_dir: &str,
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
) {
    let mut first_message = true;
    let mut try_resume_first = true;
    // For Codex, we need to capture the real thread_id from the first invocation
    // and use it for subsequent `codex exec resume` calls.
    let mut claude_session_id = claude_session_id;

    let _ = output_tx.send(PaneOutput {
        text: format!("[Session: {}]", &claude_session_id.to_string()[..8]),
        pane_id,
    });

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
            Ok(env) => env,
            Err(err) => {
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
                continue;
            }
        };

        let mut command = Command::new(binary_path);
        command
            .args(&args)
            .current_dir(working_dir)
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
                loop {
                    if shutdown.load(Ordering::SeqCst) {
                        if let Ok(mut guard) = child_process.lock() {
                            if let Some(ref mut c) = *guard {
                                let _ = c.kill();
                            }
                        }
                        break;
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
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
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
                                            ServerToCli::AddPane { session_id: _, pane_config } => {
                                                let label = pane_config.label.clone().unwrap_or_else(|| format!("Tab {}", pane_config.pane_id));
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
                                                });
                                            }
                                            ServerToCli::RemovePane { session_id: _, pane_id: remove_id } => {
                                                // Delegate to TUI event handler
                                                let _ = tui_event_tx.send(TuiEvent::CloseTab(remove_id));
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
                                            ServerToCli::UpdatePaneEffort { session_id: _, pane_id: target_pane, effort } => {
                                                let normalized = normalize_effort_level(effort.as_deref());
                                                {
                                                    let mut metas = pane_metas.lock().unwrap();
                                                    if let Some(meta) = metas.get_mut(&target_pane) {
                                                        meta.effort = normalized.clone();
                                                    }
                                                }
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
