//! Dual-pane mode: Split terminal with deadloop (left) and interactive (right) sessions
//!
//! Runs two independent Claude sessions:
//! - Left pane: Autonomous deadloop worker (same as hybrid mode)
//! - Right pane: Interactive session for user queries

use anyhow::Result;
use shared::{CliToServer, ClaudeStreamMessage, PaneType, ServerToCli};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::mpsc as tokio_mpsc;
use uuid::Uuid;

use crate::project::get_or_create_project;
use crate::tui::{App, PaneOutput};

const DEFAULT_PROMPT: &str = r#"Work on tasks defined in TODO.md. Repeat the following steps, don't stop until interrupted. Don't ask me for advice, just pick the best option you think that is honest, complete, and not corner-cutting:

1. Pick a task: First check if there are any repeated task that needs to be run again. If yes this is the task we need to do and go to step 2. If no repeated task needs to run, pick the top undone task with highest priority (high-medium-low), choose its first leaf task. If there are no task at all, (no fit repeated task and no undone TODO items left), sleep a minute, check if TODO.md is updated locally, and git pull to see if TODO.md is updated remotely. Restart step 1 (so this step is a dead loop until you find a todo item).
2. Analyze the task, check if this can be done with not too many LOC (i.e., smaller than 500 lines code give or take). If not, try to analyze this task and break it down into several smaller tasks, expanding it in the TODO.md. The breakdown can be nested and hierarchical. Try to make each leaf task small enough (<500 lines LOC). You can document your analysis in the doc folder for future reference.
3. Try to execute the first leaf task. Make a plan for the task before execute, put the plan in the docs folder, and add the file name in the item in TODO.md for reference. You can all write your key findings as a few sentences in the TODO item.
4. Make sure to add comprehensive test for the task executed. Run the whole ci test to make sure no regression happens. Put the test log in the logs folder as proof for manual review, log file name prefixed with datetime and commithash. If tests fail, fix them using the best, honest, complete approach, run test suites again to verify fixes work. Do not cheat such as disabling the borrow checker. Repeat this step until no tests fail.
5. Prepare for git commit, first check if you wrote any rusty unsafe code, if yes, then revert the changes and go back to Step 3 to redo task. Remove all temporary files, especially not to commit any binary files. For plan files, extract from implementation plan the design rational and user manual and put it in the docs folder. we can keep the plan files in docs/dev/ folder. Mark the task as done (or last done for repeated task) in the TODO.md with a timestamp [yy:mm:dd, hh:mm]
6. Git commit the changes. First do git pull --rebase, and fix conflicts if any. Remember to update submodule. If remote has any updates (merged through rebase), then run full ci tests again to make sure everything pass. If not pass, investigate and fix, repeat until pass all ci tests. Then do git push (if remote rejected because updates during we doing this step, restart this step).
7. Go back to step 1 for next task; don't ask me whether to continue, just continue. (The TODO.md file is possibly updated, so make sure you read the updated TODO.)"#;

/// Run in dual-pane mode
pub async fn run(server_url: &str, token: &str, working_dir: &Path) -> Result<()> {
    let config = crate::config::Config::load().unwrap_or_default();
    let claude_path = config.local.claude_path.clone();

    // Load or create project metadata
    let metadata = get_or_create_project(working_dir)?;
    // Use same session_id for both panes - pane_type differentiates them
    let session_id = metadata.id;

    let prompt = metadata.prompt.clone()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PROMPT.to_string());

    let working_dir_str = working_dir.to_string_lossy().to_string();
    let server_url = server_url.to_string();
    let token = token.to_string();

    // Channels for TUI <-> sessions
    let (input_tx, input_rx) = mpsc::channel::<String>();
    let (output_tx, output_rx) = mpsc::channel::<PaneOutput>();

    // Channel for sending to server
    let (server_tx, server_rx) = tokio_mpsc::channel::<CliToServer>(256);

    // Shutdown flag
    let shutdown = Arc::new(AtomicBool::new(false));

    // Shared reference to child process for cleanup
    let child_process: Arc<Mutex<Option<std::process::Child>>> = Arc::new(Mutex::new(None));
    let child_for_handler = child_process.clone();

    // Setup Ctrl+C handler
    let shutdown_for_handler = shutdown.clone();
    ctrlc::set_handler(move || {
        shutdown_for_handler.store(true, Ordering::SeqCst);
        // Kill child process if running
        if let Ok(mut guard) = child_for_handler.lock() {
            if let Some(ref mut child) = *guard {
                let _ = child.kill();
            }
        }
    })?;

    // Channel for web input -> interactive session
    let (web_input_tx, web_input_rx) = mpsc::channel::<String>();

    // Spawn server connection task
    let shutdown_clone = shutdown.clone();
    let server_url_clone = server_url.clone();
    let token_clone = token.clone();
    let working_dir_clone = working_dir_str.clone();
    let status_output_tx = output_tx.clone();
    let server_task = tokio::spawn(async move {
        run_server_connection(
            &server_url_clone,
            &token_clone,
            session_id,
            &working_dir_clone,
            server_rx,
            shutdown_clone,
            web_input_tx,
            status_output_tx,
        )
        .await
    });

    // Send initial messages to show TUI is working
    let _ = output_tx.send(PaneOutput {
        text: "[Deadloop pane initializing...]".to_string(),
        is_deadloop: true,
    });
    let _ = output_tx.send(PaneOutput {
        text: "[Interactive pane initializing...]".to_string(),
        is_deadloop: false,
    });

    // Spawn deadloop session in a thread
    let deadloop_output_tx = output_tx.clone();
    let deadloop_server_tx = server_tx.clone();
    let deadloop_shutdown = shutdown.clone();
    let deadloop_working_dir = working_dir_str.clone();
    let deadloop_claude_path = claude_path.clone();
    let deadloop_child = child_process.clone();
    let deadloop_prompt = prompt.clone();
    let deadloop_thread = thread::spawn(move || {
        run_deadloop_session(
            &deadloop_claude_path,
            &deadloop_working_dir,
            session_id,
            &deadloop_prompt,
            deadloop_output_tx,
            deadloop_server_tx,
            deadloop_shutdown,
            deadloop_child,
        )
    });

    // Spawn interactive session in a thread
    let interactive_output_tx = output_tx.clone();
    let interactive_server_tx = server_tx.clone();
    let interactive_shutdown = shutdown.clone();
    let interactive_working_dir = working_dir_str.clone();
    let interactive_claude_path = claude_path.clone();
    let interactive_thread = thread::spawn(move || {
        run_interactive_session(
            &interactive_claude_path,
            &interactive_working_dir,
            session_id,
            input_rx,
            web_input_rx,
            interactive_output_tx,
            interactive_server_tx,
            interactive_shutdown,
        )
    });

    // Run TUI in main thread
    let mut app = App::new(input_tx, output_rx);
    if let Err(e) = app.run() {
        tracing::error!("TUI error: {}", e);
    }

    // Signal shutdown
    shutdown.store(true, Ordering::SeqCst);

    // Wait for threads to finish
    let _ = deadloop_thread.join();
    let _ = interactive_thread.join();
    server_task.abort();

    Ok(())
}

/// Run the deadloop (autonomous) session
fn run_deadloop_session(
    claude_path: &str,
    working_dir: &str,
    session_id: Uuid,
    prompt: &str,
    output_tx: mpsc::Sender<PaneOutput>,
    server_tx: tokio_mpsc::Sender<CliToServer>,
    shutdown: Arc<AtomicBool>,
    child_process: Arc<Mutex<Option<std::process::Child>>>,
) {
    // Wrap in panic catcher to prevent silent thread crashes
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_deadloop_session_inner(
            claude_path,
            working_dir,
            session_id,
            prompt,
            output_tx.clone(),
            server_tx,
            shutdown,
            child_process,
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
            is_deadloop: true,
        });
    }
}

fn run_deadloop_session_inner(
    claude_path: &str,
    working_dir: &str,
    session_id: Uuid,
    prompt: &str,
    output_tx: mpsc::Sender<PaneOutput>,
    server_tx: tokio_mpsc::Sender<CliToServer>,
    shutdown: Arc<AtomicBool>,
    child_process: Arc<Mutex<Option<std::process::Child>>>,
) {
    let _ = output_tx.send(PaneOutput {
        text: "[Deadloop thread started]".to_string(),
        is_deadloop: true,
    });

    let mut iteration = 0;
    let mut backoff_seconds = 2u64;
    const MAX_BACKOFF: u64 = 3600;
    const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60); // 1 hour
    let mut last_update_check = Instant::now();

    while !shutdown.load(Ordering::SeqCst) {
        iteration += 1;
        let _ = output_tx.send(PaneOutput {
            text: format!("=== Iteration {} ===", iteration),
            is_deadloop: true,
        });

        // Send user input to server
        let _ = server_tx.blocking_send(CliToServer::UserInput {
            session_id,
            text: format!("[Iteration {}]\n{}", iteration, prompt),
            pane_type: Some(PaneType::Deadloop),
        });

        // Run Claude - pass prompt via stdin for reliability with long prompts
        let args = vec![
            "--print".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
            "--dangerously-skip-permissions".to_string(),
        ];

        match Command::new(claude_path)
            .args(&args)
            .current_dir(working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(mut child) => {
                // Write prompt to stdin
                if let Some(mut stdin) = child.stdin.take() {
                    use std::io::Write;
                    let _ = stdin.write_all(prompt.as_bytes());
                    drop(stdin);
                }

                // Take stdout for reading
                let stdout = match child.stdout.take() {
                    Some(s) => s,
                    None => {
                        let _ = output_tx.send(PaneOutput {
                            text: "[Error: Failed to capture stdout]".to_string(),
                            is_deadloop: true,
                        });
                        thread::sleep(std::time::Duration::from_secs(5));
                        continue;
                    }
                };

                // Take stderr for reading
                let stderr = child.stderr.take();

                // Store child for cleanup
                if let Ok(mut guard) = child_process.lock() {
                    *guard = Some(child);
                }

                // Spawn thread to read stderr
                let output_tx_stderr = output_tx.clone();
                let server_tx_stderr = server_tx.clone();
                let session_id_stderr = session_id;
                let stderr_thread = stderr.map(|stderr| {
                    thread::spawn(move || {
                        let reader = BufReader::new(stderr);
                        for line in reader.lines() {
                            if let Ok(line) = line {
                                if !line.trim().is_empty() {
                                    let _ = output_tx_stderr.send(PaneOutput {
                                        text: format!("[stderr] {}", line),
                                        is_deadloop: true,
                                    });
                                    let _ = server_tx_stderr.blocking_send(CliToServer::Output {
                                        session_id: session_id_stderr,
                                        data: format!("[stderr] {}", line),
                                        output_type: shared::OutputType::Error,
                                    });
                                }
                            }
                        }
                    })
                });

                let reader = BufReader::new(stdout);
                let mut had_error = false;

                for line in reader.lines() {
                    if shutdown.load(Ordering::SeqCst) {
                        break;
                    }

                    let line = match line {
                        Ok(l) => l,
                        Err(_) => break,
                    };

                    if line.trim().is_empty() {
                        continue;
                    }

                    // Parse and process
                    match serde_json::from_str::<ClaudeStreamMessage>(&line) {
                        Ok(message) => {
                            if let ClaudeStreamMessage::Result { is_error, .. } = &message {
                                if *is_error {
                                    had_error = true;
                                }
                            }

                            let display_text = format_stream_message(&message);
                            let _ = output_tx.send(PaneOutput {
                                text: display_text,
                                is_deadloop: true,
                            });

                            let _ = server_tx.blocking_send(CliToServer::StreamMessage {
                                session_id,
                                message,
                                pane_type: Some(PaneType::Deadloop),
                            });
                        }
                        Err(_) => {
                            // Non-JSON output - display and forward to server
                            let _ = output_tx.send(PaneOutput {
                                text: line.clone(),
                                is_deadloop: true,
                            });
                            let _ = server_tx.blocking_send(CliToServer::Output {
                                session_id,
                                data: line,
                                output_type: shared::OutputType::Text,
                            });
                        }
                    }
                }

                // Wait for stderr thread
                if let Some(handle) = stderr_thread {
                    let _ = handle.join();
                }

                // Wait for child to finish
                if let Ok(mut guard) = child_process.lock() {
                    if let Some(mut child) = guard.take() {
                        let _ = child.wait();
                    }
                }

                // Backoff on error
                if had_error {
                    backoff_seconds = std::cmp::min(backoff_seconds * 2, MAX_BACKOFF);
                    let _ = output_tx.send(PaneOutput {
                        text: format!("[Backing off for {}s due to error]", backoff_seconds),
                        is_deadloop: true,
                    });

                    for _ in 0..backoff_seconds {
                        if shutdown.load(Ordering::SeqCst) {
                            break;
                        }
                        thread::sleep(std::time::Duration::from_secs(1));
                    }
                } else {
                    backoff_seconds = 2;
                    thread::sleep(std::time::Duration::from_secs(2));
                }
            }
            Err(e) => {
                let _ = output_tx.send(PaneOutput {
                    text: format!("[Error starting Claude: {}]", e),
                    is_deadloop: true,
                });
                thread::sleep(std::time::Duration::from_secs(5));
            }
        }

        // Check for updates every hour (notify only, don't auto-restart in TUI mode)
        if last_update_check.elapsed() >= UPDATE_CHECK_INTERVAL {
            last_update_check = Instant::now();
            let output_tx_update = output_tx.clone();
            thread::spawn(move || {
                if let Some(new_version) = crate::update::check_for_update_available() {
                    let _ = output_tx_update.send(PaneOutput {
                        text: format!("[Update available: {} - restart to apply]", new_version),
                        is_deadloop: true,
                    });
                }
            });
        }
    }
}

/// Run the interactive session using --session-id and --resume to maintain conversation context
fn run_interactive_session(
    claude_path: &str,
    working_dir: &str,
    session_id: Uuid,
    tui_input_rx: mpsc::Receiver<String>,
    web_input_rx: mpsc::Receiver<String>,
    output_tx: mpsc::Sender<PaneOutput>,
    server_tx: tokio_mpsc::Sender<CliToServer>,
    shutdown: Arc<AtomicBool>,
) {
    // Generate a unique Claude session ID for this interactive pane
    // This allows multiple APAS instances to have separate conversations
    let claude_session_id = Uuid::new_v4();
    let mut first_message = true;

    let _ = output_tx.send(PaneOutput {
        text: format!("[Interactive session: {}]", &claude_session_id.to_string()[..8]),
        is_deadloop: false,
    });

    while !shutdown.load(Ordering::SeqCst) {
        // Wait for user input from either TUI or web
        // Track the source to avoid duplicate UserInput messages
        let (prompt, from_tui) = {
            // Try TUI input first
            match tui_input_rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(p) => (p, true),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Try web input
                    match web_input_rx.recv_timeout(std::time::Duration::from_millis(50)) {
                        Ok(p) => (p, false), // Web input - server already saved/broadcast it
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => continue,
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        };

        let _ = output_tx.send(PaneOutput {
            text: format!("> {}", &prompt[..std::cmp::min(100, prompt.len())]),
            is_deadloop: false,
        });

        // Only send UserInput to server for TUI inputs
        // Web inputs are already saved/broadcast by the server when it receives them
        if from_tui {
            let _ = server_tx.blocking_send(CliToServer::UserInput {
                session_id,
                text: prompt.clone(),
                pane_type: Some(PaneType::Interactive),
            });
        }

        // Build args:
        // - First message: use --session-id to create session with specific ID
        // - Subsequent: use --resume with the session ID to continue
        // Note: --verbose is required when using --print with --output-format stream-json
        let args = if first_message {
            first_message = false;
            vec![
                "--print".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--verbose".to_string(),
                "--dangerously-skip-permissions".to_string(),
                "--session-id".to_string(),
                claude_session_id.to_string(),
                prompt,
            ]
        } else {
            vec![
                "--print".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--verbose".to_string(),
                "--dangerously-skip-permissions".to_string(),
                "--resume".to_string(),
                claude_session_id.to_string(),
                prompt,
            ]
        };

        match Command::new(claude_path)
            .args(&args)
            .current_dir(working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(mut child) => {
                let stdout = child.stdout.take().unwrap();
                let stderr = child.stderr.take().unwrap();
                let reader = BufReader::new(stdout);

                // Spawn thread to capture stderr
                let output_tx_stderr = output_tx.clone();
                let stderr_thread = thread::spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines() {
                        if let Ok(line) = line {
                            if !line.trim().is_empty() {
                                let _ = output_tx_stderr.send(PaneOutput {
                                    text: format!("[stderr] {}", line),
                                    is_deadloop: false,
                                });
                            }
                        }
                    }
                });

                for line in reader.lines() {
                    if shutdown.load(Ordering::SeqCst) {
                        break;
                    }

                    let line = match line {
                        Ok(l) => l,
                        Err(_) => break,
                    };

                    if line.trim().is_empty() {
                        continue;
                    }

                    // Parse and process
                    match serde_json::from_str::<ClaudeStreamMessage>(&line) {
                        Ok(message) => {
                            // Display locally
                            let display_text = format_stream_message(&message);
                            let _ = output_tx.send(PaneOutput {
                                text: display_text,
                                is_deadloop: false,
                            });

                            // Send to server
                            let _ = server_tx.blocking_send(CliToServer::StreamMessage {
                                session_id,
                                message,
                                pane_type: Some(PaneType::Interactive),
                            });
                        }
                        Err(_) => {
                            let _ = output_tx.send(PaneOutput {
                                text: line,
                                is_deadloop: false,
                            });
                        }
                    }
                }

                let _ = child.wait();
                let _ = stderr_thread.join();
            }
            Err(e) => {
                let _ = output_tx.send(PaneOutput {
                    text: format!("[Error: {}]", e),
                    is_deadloop: false,
                });
            }
        }
    }
}

/// Format a stream message for display
fn format_stream_message(message: &ClaudeStreamMessage) -> String {
    match message {
        ClaudeStreamMessage::System { model, tools, .. } => {
            format!("[Session started - Model: {}, Tools: {}]", model, tools.len())
        }
        ClaudeStreamMessage::Assistant { message, .. } => {
            let mut output = String::new();
            for block in &message.content {
                match block {
                    shared::ClaudeContentBlock::Text { text } => {
                        output.push_str(text);
                    }
                    shared::ClaudeContentBlock::ToolUse { name, input, .. } => {
                        output.push_str(&format!("[Tool: {} - {:?}]", name, input));
                    }
                    shared::ClaudeContentBlock::ToolResult { content, is_error, .. } => {
                        let status = if *is_error { "Error" } else { "Result" };
                        let preview = if content.len() > 100 {
                            format!("{}...", &content[..100])
                        } else {
                            content.clone()
                        };
                        output.push_str(&format!("[{}: {}]", status, preview));
                    }
                }
            }
            output
        }
        ClaudeStreamMessage::User { message, .. } => {
            let mut output = String::new();
            for block in &message.content {
                if let shared::ClaudeContentBlock::ToolResult { tool_use_id, content, .. } = block {
                    let preview = if content.len() > 50 {
                        format!("{}...", &content[..50])
                    } else {
                        content.clone()
                    };
                    output.push_str(&format!("[Tool result {}: {}]", tool_use_id, preview));
                }
            }
            output
        }
        ClaudeStreamMessage::Result {
            subtype,
            total_cost_usd,
            duration_ms,
            ..
        } => {
            format!(
                "[{} - Cost: ${:.4}, Duration: {}ms]",
                subtype, total_cost_usd, duration_ms
            )
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
    web_input_tx: mpsc::Sender<String>,
    status_tx: mpsc::Sender<PaneOutput>,
) -> Result<()> {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    let mut reconnect_delay = std::time::Duration::from_secs(1);
    let max_reconnect_delay = std::time::Duration::from_secs(60);
    let mut connection_count = 0u32;

    while !shutdown.load(Ordering::SeqCst) {
        let ws_url = format!("{}/ws/cli", server_url);

        if connection_count > 0 {
            let _ = status_tx.send(PaneOutput {
                text: format!("[Server: Reconnecting... (attempt {})]", connection_count),
                is_deadloop: true,
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
                let msg_text = serde_json::to_string(&register_msg)?;
                if ws_sender.send(Message::Text(msg_text.into())).await.is_err() {
                    let _ = status_tx.send(PaneOutput {
                        text: "[Server: Connection lost during registration]".to_string(),
                        is_deadloop: true,
                    });
                    tokio::time::sleep(reconnect_delay).await;
                    continue;
                }

                // Wait for registration response with timeout
                let registration_timeout = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    async {
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
                                            return Some(Err(format!("Version {} not supported, need {}", client_version, min_version)));
                                        }
                                        _ => continue,
                                    }
                                }
                                Message::Ping(data) => {
                                    // Respond to ping during registration
                                    return Some(Err(format!("ping:{}", data.len())));
                                }
                                _ => continue,
                            }
                        }
                        None
                    }
                ).await;

                match registration_timeout {
                    Ok(Some(Ok(cli_id))) => {
                        let _ = status_tx.send(PaneOutput {
                            text: format!("[Server: Connected ({})]", &cli_id.to_string()[..8]),
                            is_deadloop: true,
                        });
                        // Successfully registered, continue to session start
                    }
                    Ok(Some(Err(reason))) if reason.starts_with("ping:") => {
                        // Got a ping, need to handle it - restart the connection
                        let _ = status_tx.send(PaneOutput {
                            text: "[Server: Received ping during registration, reconnecting...]".to_string(),
                            is_deadloop: true,
                        });
                        tokio::time::sleep(reconnect_delay).await;
                        continue;
                    }
                    Ok(Some(Err(reason))) => {
                        let _ = status_tx.send(PaneOutput {
                            text: format!("[Server: Registration failed - {}]", reason),
                            is_deadloop: true,
                        });
                        return Err(anyhow::anyhow!("Registration failed: {}", reason));
                    }
                    Ok(None) | Err(_) => {
                        let _ = status_tx.send(PaneOutput {
                            text: "[Server: Registration timeout or connection lost]".to_string(),
                            is_deadloop: true,
                        });
                        tokio::time::sleep(reconnect_delay).await;
                        continue;
                    }
                }

                // Register session (pane_type in messages will differentiate deadloop vs interactive)
                let hostname = hostname::get()
                    .ok()
                    .and_then(|h| h.into_string().ok());

                let session_start = CliToServer::SessionStart {
                    session_id,
                    working_dir: Some(working_dir.to_string()),
                    hostname,
                    pane_type: None, // Single session, pane_type on individual messages
                };
                let msg_text = serde_json::to_string(&session_start)?;
                if ws_sender.send(Message::Text(msg_text.into())).await.is_err() {
                    let _ = status_tx.send(PaneOutput {
                        text: "[Server: Connection lost during session start]".to_string(),
                        is_deadloop: true,
                    });
                    tokio::time::sleep(reconnect_delay).await;
                    continue;
                }

                // Use a persistent heartbeat interval instead of creating new sleep each time
                let mut heartbeat_interval = tokio::time::interval(std::time::Duration::from_secs(25));
                heartbeat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                // Skip the first immediate tick
                heartbeat_interval.tick().await;

                // Main loop
                loop {
                    tokio::select! {
                        Some(msg) = output_rx.recv() => {
                            let msg_text = serde_json::to_string(&msg)?;
                            if ws_sender.send(Message::Text(msg_text.into())).await.is_err() {
                                let _ = status_tx.send(PaneOutput {
                                    text: "[Server: Connection lost, reconnecting...]".to_string(),
                                    is_deadloop: true,
                                });
                                break;
                            }
                        }
                        msg = ws_receiver.next() => {
                            match msg {
                                Some(Ok(Message::Text(text))) => {
                                    if let Ok(server_msg) = serde_json::from_str::<ServerToCli>(&text) {
                                        match server_msg {
                                            ServerToCli::Input { session_id: _, data } => {
                                                // Forward to interactive session
                                                let _ = web_input_tx.send(data);
                                            }
                                            ServerToCli::Heartbeat => {
                                                // Heartbeat response, nothing to do
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                Some(Ok(Message::Ping(data))) => {
                                    // Respond to server ping with pong
                                    if ws_sender.send(Message::Pong(data)).await.is_err() {
                                        let _ = status_tx.send(PaneOutput {
                                            text: "[Server: Failed to send pong, reconnecting...]".to_string(),
                                            is_deadloop: true,
                                        });
                                        break;
                                    }
                                }
                                Some(Ok(Message::Pong(_))) => {
                                    // Server responded to our ping, connection is alive
                                }
                                Some(Ok(Message::Close(_))) | None => {
                                    let _ = status_tx.send(PaneOutput {
                                        text: "[Server: Connection closed, reconnecting...]".to_string(),
                                        is_deadloop: true,
                                    });
                                    break;
                                }
                                Some(Err(e)) => {
                                    let _ = status_tx.send(PaneOutput {
                                        text: format!("[Server: Connection error ({}), reconnecting...]", e),
                                        is_deadloop: true,
                                    });
                                    break;
                                }
                                _ => {}
                            }
                        }
                        _ = heartbeat_interval.tick() => {
                            // Send ping to server to keep connection alive
                            if ws_sender.send(Message::Ping(vec![].into())).await.is_err() {
                                let _ = status_tx.send(PaneOutput {
                                    text: "[Server: Heartbeat failed, reconnecting...]".to_string(),
                                    is_deadloop: true,
                                });
                                break;
                            }
                        }
                    }

                    if shutdown.load(Ordering::SeqCst) {
                        break;
                    }
                }

                // Small delay before reconnecting
                if !shutdown.load(Ordering::SeqCst) {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
            Err(e) => {
                let _ = status_tx.send(PaneOutput {
                    text: format!("[Server: Connection failed - {}. Retry in {}s]", e, reconnect_delay.as_secs()),
                    is_deadloop: true,
                });
                tokio::time::sleep(reconnect_delay).await;
                reconnect_delay = std::cmp::min(reconnect_delay * 2, max_reconnect_delay);
                connection_count += 1;
            }
        }
    }

    Ok(())
}
