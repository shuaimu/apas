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
use tokio::sync::mpsc as tokio_mpsc;
use uuid::Uuid;

use crate::project::get_or_create_project;
use crate::tui::{App, PaneOutput};

/// Run in dual-pane mode
pub async fn run(server_url: &str, token: &str, working_dir: &Path) -> Result<()> {
    let config = crate::config::Config::load().unwrap_or_default();
    let claude_path = config.local.claude_path.clone();

    // Load or create project metadata
    let metadata = get_or_create_project(working_dir)?;
    let deadloop_session_id = metadata.id;
    // Create a derived UUID for interactive session by XORing with a fixed pattern
    let interactive_bytes: [u8; 16] = {
        let mut bytes = *deadloop_session_id.as_bytes();
        // XOR with pattern to create different but deterministic UUID
        for (i, b) in bytes.iter_mut().enumerate() {
            *b ^= 0x55 ^ (i as u8);
        }
        bytes
    };
    let interactive_session_id = Uuid::from_bytes(interactive_bytes);

    let prompt = metadata.prompt.clone().unwrap_or_else(|| {
        "You are an autonomous AI developer. Review, fix, and improve the codebase.".to_string()
    });

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

    // Spawn server connection task
    let server_tx_clone = server_tx.clone();
    let shutdown_clone = shutdown.clone();
    let server_url_clone = server_url.clone();
    let token_clone = token.clone();
    let working_dir_clone = working_dir_str.clone();
    let server_task = tokio::spawn(async move {
        run_server_connection(
            &server_url_clone,
            &token_clone,
            deadloop_session_id,
            interactive_session_id,
            &working_dir_clone,
            server_rx,
            shutdown_clone,
        )
        .await
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
            deadloop_session_id,
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
            interactive_session_id,
            input_rx,
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
    let mut iteration = 0;
    let mut backoff_seconds = 2u64;
    const MAX_BACKOFF: u64 = 3600;

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

        // Run Claude
        let args = vec![
            "--print".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
            "--dangerously-skip-permissions".to_string(),
            prompt.to_string(),
        ];

        match Command::new(claude_path)
            .args(&args)
            .current_dir(working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => {
                // Store child for cleanup
                if let Ok(mut guard) = child_process.lock() {
                    *guard = Some(child);
                }

                // Take the child back for reading
                let mut child = child_process.lock().unwrap().take().unwrap();
                let stdout = child.stdout.take().unwrap();
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
                            // Check for errors
                            if let ClaudeStreamMessage::Result { is_error, .. } = &message {
                                if *is_error {
                                    had_error = true;
                                }
                            }

                            // Display locally
                            let display_text = format_stream_message(&message);
                            let _ = output_tx.send(PaneOutput {
                                text: display_text,
                                is_deadloop: true,
                            });

                            // Send to server
                            let _ = server_tx.blocking_send(CliToServer::StreamMessage {
                                session_id,
                                message,
                                pane_type: Some(PaneType::Deadloop),
                            });
                        }
                        Err(_) => {
                            // Non-JSON output
                            let _ = output_tx.send(PaneOutput {
                                text: line,
                                is_deadloop: true,
                            });
                        }
                    }
                }

                let _ = child.wait();

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
    }
}

/// Run the interactive session
fn run_interactive_session(
    claude_path: &str,
    working_dir: &str,
    session_id: Uuid,
    input_rx: mpsc::Receiver<String>,
    output_tx: mpsc::Sender<PaneOutput>,
    server_tx: tokio_mpsc::Sender<CliToServer>,
    shutdown: Arc<AtomicBool>,
) {
    while !shutdown.load(Ordering::SeqCst) {
        // Wait for user input
        let prompt = match input_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(p) => p,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        let _ = output_tx.send(PaneOutput {
            text: format!("[Processing: {}]", &prompt[..std::cmp::min(50, prompt.len())]),
            is_deadloop: false,
        });

        // Send user input to server
        let _ = server_tx.blocking_send(CliToServer::UserInput {
            session_id,
            text: prompt.clone(),
            pane_type: Some(PaneType::Interactive),
        });

        // Run Claude for this query
        let args = vec![
            "--print".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
            "--dangerously-skip-permissions".to_string(),
            prompt,
        ];

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
                let reader = BufReader::new(stdout);

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

                let _ = output_tx.send(PaneOutput {
                    text: "[Query complete]".to_string(),
                    is_deadloop: false,
                });
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

/// Run server connection (similar to hybrid mode)
async fn run_server_connection(
    server_url: &str,
    token: &str,
    deadloop_session_id: Uuid,
    interactive_session_id: Uuid,
    working_dir: &str,
    mut output_rx: tokio_mpsc::Receiver<CliToServer>,
    shutdown: Arc<AtomicBool>,
) -> Result<()> {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    let mut reconnect_delay = std::time::Duration::from_secs(1);
    let max_reconnect_delay = std::time::Duration::from_secs(60);

    while !shutdown.load(Ordering::SeqCst) {
        let ws_url = format!("{}/ws/cli", server_url);
        tracing::info!("Connecting to server: {}", ws_url);

        match connect_async(&ws_url).await {
            Ok((ws_stream, _)) => {
                reconnect_delay = std::time::Duration::from_secs(1);
                let (mut ws_sender, mut ws_receiver) = ws_stream.split();

                // Register
                let register_msg = CliToServer::Register {
                    token: token.to_string(),
                    version: Some(env!("APAS_VERSION").to_string()),
                };
                let msg_text = serde_json::to_string(&register_msg)?;
                ws_sender.send(Message::Text(msg_text.into())).await?;

                // Wait for registration response
                while let Some(Ok(msg)) = ws_receiver.next().await {
                    if let Message::Text(text) = msg {
                        let response: ServerToCli = serde_json::from_str(&text)?;
                        match response {
                            ServerToCli::Registered { cli_id } => {
                                tracing::info!("Registered as CLI {}", cli_id);
                                break;
                            }
                            ServerToCli::RegistrationFailed { reason } => {
                                tracing::error!("Registration failed: {}", reason);
                                return Err(anyhow::anyhow!("Registration failed: {}", reason));
                            }
                            ServerToCli::VersionUnsupported {
                                client_version,
                                min_version,
                            } => {
                                tracing::error!(
                                    "Version {} not supported, minimum: {}",
                                    client_version,
                                    min_version
                                );
                                return Err(anyhow::anyhow!("Version not supported"));
                            }
                            _ => continue,
                        }
                    }
                }

                // Register both sessions
                let hostname = hostname::get()
                    .ok()
                    .and_then(|h| h.into_string().ok());

                for (session_id, pane_type) in [
                    (deadloop_session_id, PaneType::Deadloop),
                    (interactive_session_id, PaneType::Interactive),
                ] {
                    let session_start = CliToServer::SessionStart {
                        session_id,
                        working_dir: Some(working_dir.to_string()),
                        hostname: hostname.clone(),
                        pane_type: Some(pane_type),
                    };
                    let msg_text = serde_json::to_string(&session_start)?;
                    ws_sender.send(Message::Text(msg_text.into())).await?;
                }

                // Main loop
                loop {
                    tokio::select! {
                        Some(msg) = output_rx.recv() => {
                            let msg_text = serde_json::to_string(&msg)?;
                            if ws_sender.send(Message::Text(msg_text.into())).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(msg)) = ws_receiver.next() => {
                            // Handle server messages (heartbeat, etc.)
                            if let Message::Text(text) = msg {
                                if let Ok(ServerToCli::Input { session_id: _, data }) = serde_json::from_str(&text) {
                                    tracing::debug!("Received input from server: {}", data);
                                    // TODO: Route to appropriate session
                                }
                            }
                        }
                        _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                            // Heartbeat
                            let _ = ws_sender.send(Message::Text(
                                serde_json::to_string(&CliToServer::Heartbeat)?.into()
                            )).await;
                        }
                    }

                    if shutdown.load(Ordering::SeqCst) {
                        break;
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Connection failed: {}. Retrying in {:?}", e, reconnect_delay);
                tokio::time::sleep(reconnect_delay).await;
                reconnect_delay = std::cmp::min(reconnect_delay * 2, max_reconnect_delay);
            }
        }
    }

    Ok(())
}
