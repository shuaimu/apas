//! Hybrid mode - local Claude CLI + streaming structured output to remote server
//!
//! This mode runs Claude locally using `--output-format stream-json` mode
//! and streams structured JSON messages to the remote server for observation.

use anyhow::Result;
use futures::{SinkExt, StreamExt};
use shared::{CliToServer, ClaudeStreamMessage, ServerToCli};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

use crate::config::Config;
use crate::project;

const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(60);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const VERSION: &str = env!("APAS_VERSION");

const DEFAULT_PROMPT: &str = r#"Work on tasks defined in TODO.md. Repeat the following steps, don't stop until interrupted. Don't ask me for advice, just pick the best option you think that is honest, complete, and not corner-cutting:

1. Pick a task: First check if there are any repeated task that needs to be run again. If yes this is the task we need to do and go to step 2. If no repeated task needs to run, pick the top undone task with highest priority (high-medium-low), choose its first leaf task. If there are no task at all, (no fit repeated task and no undone TODO items left), sleep a minute, check if TODO.md is updated locally, and git pull to see if TODO.md is updated remotely. Restart step 1 (so this step is a dead loop until you find a todo item).
2. Analyze the task, check if this can be done with not too many LOC (i.e., smaller than 500 lines code give or take). If not, try to analyze this task and break it down into several smaller tasks, expanding it in the TODO.md. The breakdown can be nested and hierarchical. Try to make each leaf task small enough (<500 lines LOC). You can document your analysis in the doc folder for future reference.
3. Try to execute the first leaf task. Make a plan for the task before execute, put the plan in the docs folder, and add the file name in the item in TODO.md for reference. You can all write your key findings as a few sentences in the TODO item.
4. Make sure to add comprehensive test for the task executed. Run the whole ci test to make sure no regression happens. Put the test log in the logs folder as proof for manual review, log file name prefixed with datetime and commithash. If tests fail, fix them using the best, honest, complete approach, run test suites again to verify fixes work. Do not cheat such as disabling the borrow checker. Repeat this step until no tests fail.
5. Prepare for git commit, first check if you wrote any rusty unsafe code, if yes, then revert the changes and go back to Step 3 to redo task. Remove all temporary files, especially not to commit any binary files. For plan files, extract from implementation plan the design rational and user manual and put it in the docs folder. we can keep the plan files in docs/dev/ folder. Mark the task as done (or last done for repeated task) in the TODO.md with a timestamp [yy:mm:dd, hh:mm]
6. Git commit the changes. First do git pull --rebase, and fix conflicts if any. Remember to update submodule. If remote has any updates (merged through rebase), then run full ci tests again to make sure everything pass. If not pass, investigate and fix, repeat until pass all ci tests. Then do git push (if remote rejected because updates during we doing this step, restart this step).
7. Go back to step 1 for next task; don't ask me whether to continue, just continue. (The TODO.md file is possibly updated, so make sure you read the updated TODO.)"#;

/// Run in hybrid mode - local Claude CLI + streaming to remote server
pub async fn run(server_url: &str, token: &str, working_dir: &Path) -> Result<()> {
    let config = Config::load().unwrap_or_default();
    let claude_path = config.local.claude_path.clone();

    // Get or create project metadata - use project ID as session ID
    let project_meta = project::get_or_create_project(working_dir)?;
    let session_id = project_meta.id;
    let project_name = project_meta.name.clone();
    let prompt = project_meta.prompt.clone().unwrap_or_else(|| DEFAULT_PROMPT.to_string());

    // Channel for sending output to server (buffered to handle reconnections)
    let (server_tx, server_rx) = mpsc::channel::<CliToServer>(256);

    // Flag to signal shutdown
    let shutdown = Arc::new(AtomicBool::new(false));

    // Shared handle to current Claude child process (for cleanup on Ctrl+C)
    let child_process: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));

    // Set up Ctrl+C handler to kill Claude process on exit
    let shutdown_for_ctrlc = shutdown.clone();
    let child_for_ctrlc = child_process.clone();
    ctrlc::set_handler(move || {
        println!("\nShutting down...");
        shutdown_for_ctrlc.store(true, Ordering::SeqCst);
        // Kill the Claude process if running
        if let Ok(mut guard) = child_for_ctrlc.lock() {
            if let Some(ref mut child) = *guard {
                let _ = child.kill();
            }
        }
    }).expect("Failed to set Ctrl+C handler");

    // Spawn server connection task (runs in background with auto-reconnect)
    let server_url_owned = server_url.to_string();
    let token_owned = token.to_string();
    let shutdown_clone = shutdown.clone();
    let working_dir_str = working_dir.to_string_lossy().to_string();
    let _server_task = tokio::spawn(async move {
        run_server_connection(&server_url_owned, &token_owned, session_id, &working_dir_str, server_rx, shutdown_clone).await
    });

    // Run Claude with stream-json output (blocking I/O in a separate thread)
    let claude_path_owned = claude_path.clone();
    let working_dir_owned = working_dir.to_path_buf();
    let shutdown_for_claude = shutdown.clone();
    let result = tokio::task::spawn_blocking(move || {
        run_dead_loop_session(&claude_path_owned, &working_dir_owned, session_id, project_name, &prompt, server_tx, &shutdown_for_claude, child_process)
    }).await?;

    // Signal shutdown
    shutdown.store(true, Ordering::SeqCst);

    result
}

/// Run Claude CLI in a dead loop, continuously sending the same prompt
/// Uses --resume to maintain conversation continuity across invocations
fn run_dead_loop_session(
    claude_path: &str,
    working_dir: &Path,
    session_id: Uuid,
    project_name: Option<String>,
    prompt: &str,
    server_tx: mpsc::Sender<CliToServer>,
    shutdown: &Arc<AtomicBool>,
    child_handle: Arc<Mutex<Option<Child>>>,
) -> Result<()> {
    let display_name = project_name.as_deref().unwrap_or("unnamed");
    println!("Project: {} (ID: {})", display_name, session_id);
    println!("Running in autonomous mode (Ctrl+C to stop)");
    println!("\nPrompt:\n{}\n", prompt);

    let mut iteration = 0u64;
    let mut backoff_seconds = 2u64; // Start with 2 seconds, will grow exponentially on errors
    const MAX_BACKOFF: u64 = 3600; // Max 1 hour backoff

    loop {
        if shutdown.load(Ordering::SeqCst) {
            println!("\nShutdown requested, exiting...");
            break;
        }

        iteration += 1;
        println!("\n=== Iteration {} ===", iteration);

        // Send prompt to server for web UI display
        let user_input_msg = CliToServer::UserInput {
            session_id,
            text: format!("[Iteration {}]\n{}", iteration, prompt),
        };
        if server_tx.blocking_send(user_input_msg).is_err() {
            tracing::debug!("Failed to send user input to server");
        }

        // Run Claude with --print for this prompt
        let args = vec![
            "--print".to_string(),
            "--output-format".to_string(), "stream-json".to_string(),
            "--verbose".to_string(),
            "--dangerously-skip-permissions".to_string(),
            prompt.to_string(),
        ];

        // Spawn Claude for this prompt
        let mut child = match Command::new(claude_path)
            .args(&args)
            .current_dir(working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to spawn Claude: {}", e);
                std::thread::sleep(Duration::from_secs(5));
                continue;
            }
        };

        let stdout = child.stdout.take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture stdout"))?;
        let stderr = child.stderr.take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture stderr"))?;

        // Store child in shared handle so Ctrl+C handler can kill it
        if let Ok(mut guard) = child_handle.lock() {
            *guard = Some(child);
        }
        // Get a reference back for waiting
        let child_handle_clone = child_handle.clone();

        // Spawn thread to read stderr and detect rate limits
        let rate_limit_detected = Arc::new(AtomicBool::new(false));
        let rate_limit_clone = rate_limit_detected.clone();
        let stderr_thread = std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                if let Ok(line) = line {
                    eprintln!("{}", line);
                    // Check for rate limit / usage limit keywords
                    let lower = line.to_lowercase();
                    if lower.contains("rate limit")
                        || lower.contains("rate_limit")
                        || lower.contains("usage limit")
                        || lower.contains("too many requests")
                        || lower.contains("quota exceeded")
                        || lower.contains("capacity")
                        || lower.contains("overloaded") {
                        rate_limit_clone.store(true, Ordering::SeqCst);
                    }
                }
            }
        });

        // Read JSON lines from stdout
        let reader = BufReader::new(stdout);
        let mut error_in_stream = false;
        for line in reader.lines() {
            if shutdown.load(Ordering::SeqCst) {
                break;
            }

            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    tracing::debug!("Error reading stdout: {}", e);
                    break;
                }
            };

            if line.is_empty() {
                continue;
            }

            // Parse JSON line
            match serde_json::from_str::<ClaudeStreamMessage>(&line) {
                Ok(ref message) => {
                    // Check for error results (rate limit, etc.)
                    if let ClaudeStreamMessage::Result { is_error, subtype, result, .. } = message {
                        if *is_error || subtype == "error" || subtype.contains("error") {
                            error_in_stream = true;
                            // Check if it's a rate limit specifically
                            let lower_result = result.to_lowercase();
                            if lower_result.contains("rate")
                                || lower_result.contains("limit")
                                || lower_result.contains("quota")
                                || lower_result.contains("capacity") {
                                rate_limit_detected.store(true, Ordering::SeqCst);
                            }
                        }
                    }

                    // Print locally for visibility
                    print_stream_message(message);

                    // Forward to server
                    let msg = CliToServer::StreamMessage {
                        session_id,
                        message: message.clone(),
                    };
                    if server_tx.blocking_send(msg).is_err() {
                        tracing::debug!("Server channel closed");
                    }
                }
                Err(e) => {
                    tracing::debug!("Failed to parse stream message: {} - line: {}", e, line);
                }
            }
        }

        // Wait for child to exit (get it back from the shared handle)
        let status = if let Ok(mut guard) = child_handle_clone.lock() {
            if let Some(ref mut child) = *guard {
                // If shutdown requested, kill the child
                if shutdown.load(Ordering::SeqCst) {
                    let _ = child.kill();
                }
                let status = child.wait();
                *guard = None; // Clear the handle
                status
            } else {
                Err(std::io::Error::new(std::io::ErrorKind::Other, "Child process not found"))
            }
        } else {
            Err(std::io::Error::new(std::io::ErrorKind::Other, "Failed to lock child handle"))
        };

        match &status {
            Ok(s) => tracing::debug!("Claude exited with status: {}", s),
            Err(e) => tracing::debug!("Error waiting for Claude: {}", e),
        }

        // Wait for stderr thread
        let _ = stderr_thread.join();

        // Check if we hit a rate limit or error
        let hit_rate_limit = rate_limit_detected.load(Ordering::SeqCst);
        let had_error = error_in_stream || hit_rate_limit || !matches!(status, Ok(ref s) if s.success());

        if had_error {
            if hit_rate_limit {
                println!("\n[Rate limit detected! Backing off for {} seconds...]", backoff_seconds);
            } else {
                println!("\n[Error detected. Backing off for {} seconds...]", backoff_seconds);
            }

            // Wait with backoff (check shutdown flag periodically)
            let wait_until = std::time::Instant::now() + Duration::from_secs(backoff_seconds);
            while std::time::Instant::now() < wait_until {
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(Duration::from_secs(1));
            }

            // Increase backoff for next error (exponential)
            backoff_seconds = std::cmp::min(backoff_seconds * 2, MAX_BACKOFF);
        } else {
            println!("\n[Iteration {} completed]", iteration);
            // Reset backoff on success
            backoff_seconds = 2;

            // Small delay between iterations to avoid hammering
            if !shutdown.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }

    Ok(())
}

/// Print stream message locally for user visibility
fn print_stream_message(message: &ClaudeStreamMessage) {
    match message {
        ClaudeStreamMessage::System { subtype, model, tools, .. } => {
            if subtype == "init" {
                println!("\n[Session started - Model: {}, Tools: {}]\n", model, tools.len());
            }
        }
        ClaudeStreamMessage::Assistant { message: msg, .. } => {
            for block in &msg.content {
                match block {
                    shared::ClaudeContentBlock::Text { text } => {
                        println!("{}", text);
                    }
                    shared::ClaudeContentBlock::ToolUse { name, input, .. } => {
                        println!("\n[Tool: {} - {}]\n", name, serde_json::to_string(input).unwrap_or_default());
                    }
                    _ => {}
                }
            }
        }
        ClaudeStreamMessage::User { tool_use_result, .. } => {
            if let Some(result) = tool_use_result {
                if let Some(file_info) = result.get("file") {
                    if let Some(path) = file_info.get("filePath") {
                        println!("[Tool result: Read {}]", path);
                    }
                }
            }
        }
        ClaudeStreamMessage::Result { subtype, total_cost_usd, duration_ms, .. } => {
            println!("\n[{} - Cost: ${:.4}, Duration: {}ms]\n", subtype, total_cost_usd, duration_ms);
        }
    }
}

/// Manage WebSocket connection to server with auto-reconnect
async fn run_server_connection(
    server_url: &str,
    token: &str,
    session_id: Uuid,
    working_dir: &str,
    mut output_rx: mpsc::Receiver<CliToServer>,
    shutdown: Arc<AtomicBool>,
) {
    let mut reconnect_delay = INITIAL_RECONNECT_DELAY;

    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        match connect_to_server(server_url, token, session_id, working_dir, &mut output_rx, &shutdown).await {
            Ok(_) => {
                reconnect_delay = INITIAL_RECONNECT_DELAY;
            }
            Err(e) => {
                tracing::debug!("Server connection error: {}. Will retry...", e);
                reconnect_delay = std::cmp::min(reconnect_delay * 2, MAX_RECONNECT_DELAY);
            }
        }

        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        tokio::time::sleep(reconnect_delay).await;
    }
}

async fn connect_to_server(
    server_url: &str,
    token: &str,
    session_id: Uuid,
    working_dir: &str,
    output_rx: &mut mpsc::Receiver<CliToServer>,
    shutdown: &Arc<AtomicBool>,
) -> Result<()> {
    let ws_url = format!("{}/ws/cli", server_url);
    tracing::debug!("Connecting to server: {}", ws_url);

    let (ws_stream, _) = connect_async(&ws_url).await?;
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // Send registration with version
    let register_msg = CliToServer::Register {
        token: token.to_string(),
        version: Some(VERSION.to_string()),
    };
    let msg_text = serde_json::to_string(&register_msg)?;
    ws_sender.send(Message::Text(msg_text.into())).await?;

    // Wait for registration response
    loop {
        match ws_receiver.next().await {
            Some(Ok(Message::Text(text))) => {
                let response: ServerToCli = serde_json::from_str(&text)?;
                match response {
                    ServerToCli::Registered { cli_id } => {
                        tracing::debug!("Connected to server as CLI {}", cli_id);
                        break;
                    }
                    ServerToCli::RegistrationFailed { reason } => {
                        return Err(anyhow::anyhow!("Registration failed: {}", reason));
                    }
                    ServerToCli::VersionUnsupported { client_version, min_version } => {
                        eprintln!("\n========================================");
                        eprintln!("ERROR: Client version {} is no longer supported!", client_version);
                        eprintln!("Minimum required version: {}", min_version);
                        eprintln!("Please update by running: apas update");
                        eprintln!("========================================\n");
                        std::process::exit(1);
                    }
                    _ => continue,
                }
            }
            Some(Ok(Message::Ping(data))) => {
                ws_sender.send(Message::Pong(data)).await?;
            }
            Some(Err(e)) => return Err(e.into()),
            None => return Err(anyhow::anyhow!("Connection closed during registration")),
            _ => continue,
        }
    }

    // Send SessionStart to register our local session with the server
    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok());
    let session_start_msg = CliToServer::SessionStart {
        session_id,
        working_dir: Some(working_dir.to_string()),
        hostname,
    };
    let msg_text = serde_json::to_string(&session_start_msg)?;
    ws_sender.send(Message::Text(msg_text.into())).await?;
    tracing::debug!("Registered local session {} with server", session_id);

    // Channel for sending to WebSocket
    let (ws_tx, mut ws_rx) = mpsc::channel::<CliToServer>(32);

    // Heartbeat task
    let heartbeat_tx = ws_tx.clone();
    let heartbeat_shutdown = shutdown.clone();
    let heartbeat_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
        loop {
            interval.tick().await;
            if heartbeat_shutdown.load(Ordering::SeqCst) {
                break;
            }
            if heartbeat_tx.send(CliToServer::Heartbeat).await.is_err() {
                break;
            }
        }
    });

    // Task to send messages to WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(msg) = ws_rx.recv().await {
            let text = serde_json::to_string(&msg).unwrap();
            if ws_sender.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    // Main loop: forward output to server
    let shutdown_clone = shutdown.clone();
    loop {
        if shutdown_clone.load(Ordering::SeqCst) {
            break;
        }

        tokio::select! {
            Some(msg) = output_rx.recv() => {
                if ws_tx.send(msg).await.is_err() {
                    break;
                }
            }
            msg_result = ws_receiver.next() => {
                match msg_result {
                    Some(Ok(Message::Text(_))) => {
                        // Handle server messages if needed
                    }
                    Some(Ok(Message::Close(_))) => break,
                    Some(Err(_)) => break,
                    None => break,
                    _ => {}
                }
            }
        }
    }

    heartbeat_task.abort();
    send_task.abort();

    Ok(())
}
