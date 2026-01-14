//! Hybrid mode - local Claude CLI + streaming structured output to remote server
//!
//! This mode runs Claude locally using `--output-format stream-json` mode
//! and streams structured JSON messages to the remote server for observation.

use anyhow::Result;
use futures::{SinkExt, StreamExt};
use shared::{CliToServer, ClaudeStreamMessage, ServerToCli};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

use crate::config::Config;
use crate::project;

const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(60);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// Run in hybrid mode - local Claude CLI + streaming to remote server
pub async fn run(server_url: &str, token: &str, working_dir: &Path) -> Result<()> {
    let config = Config::load().unwrap_or_default();
    let claude_path = config.local.claude_path.clone();

    // Get or create project metadata - use project ID as session ID
    let project_meta = project::get_or_create_project(working_dir)?;
    let session_id = project_meta.id;
    let project_name = project_meta.name.clone();

    // Channel for sending output to server (buffered to handle reconnections)
    let (server_tx, server_rx) = mpsc::channel::<CliToServer>(256);

    // Flag to signal shutdown
    let shutdown = Arc::new(AtomicBool::new(false));

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
        run_stream_json_session(&claude_path_owned, &working_dir_owned, session_id, project_name, server_tx, &shutdown_for_claude)
    }).await?;

    // Signal shutdown
    shutdown.store(true, Ordering::SeqCst);

    result
}

/// Run Claude CLI in interactive mode using --print for each prompt
/// Uses --session-id to maintain conversation continuity across invocations
fn run_stream_json_session(
    claude_path: &str,
    working_dir: &Path,
    session_id: Uuid,
    project_name: Option<String>,
    server_tx: mpsc::Sender<CliToServer>,
    shutdown: &Arc<AtomicBool>,
) -> Result<()> {
    let display_name = project_name.as_deref().unwrap_or("unnamed");
    println!("Project: {} (ID: {})", display_name, session_id);
    println!("Type your prompts (Ctrl+D to exit):\n");

    let local_stdin = std::io::stdin();

    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        print!("> ");
        std::io::stdout().flush()?;

        let mut input = String::new();
        match local_stdin.read_line(&mut input) {
            Ok(0) => {
                // EOF (Ctrl+D)
                println!("\nExiting...");
                break;
            }
            Ok(_) => {
                let input = input.trim();
                if input.is_empty() {
                    continue;
                }

                // Send user input to server for web UI display
                let user_input_msg = CliToServer::UserInput {
                    session_id,
                    text: input.to_string(),
                };
                if server_tx.blocking_send(user_input_msg).is_err() {
                    tracing::debug!("Failed to send user input to server");
                }

                // Run Claude with --print for this single prompt
                let mut args = vec![
                    "--print".to_string(),
                    "--output-format".to_string(), "stream-json".to_string(),
                    "--verbose".to_string(),
                    "--dangerously-skip-permissions".to_string(),
                ];

                // Always use --resume with our project-based session ID
                // Claude CLI will create the session if it doesn't exist, or continue if it does
                args.push("--resume".to_string());
                args.push(session_id.to_string());

                // Add the prompt as the last argument
                args.push(input.to_string());

                // Spawn Claude for this prompt
                let mut child = Command::new(claude_path)
                    .args(&args)
                    .current_dir(working_dir)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()?;

                let stdout = child.stdout.take()
                    .ok_or_else(|| anyhow::anyhow!("Failed to capture stdout"))?;
                let stderr = child.stderr.take()
                    .ok_or_else(|| anyhow::anyhow!("Failed to capture stderr"))?;

                // Spawn thread to read stderr
                let stderr_thread = std::thread::spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines() {
                        if let Ok(line) = line {
                            eprintln!("{}", line);
                        }
                    }
                });

                // Read JSON lines from stdout
                let reader = BufReader::new(stdout);
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
                        Ok(message) => {
                            // Print locally for visibility
                            print_stream_message(&message);

                            // Forward to server
                            let msg = CliToServer::StreamMessage {
                                session_id,
                                message,
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

                // Wait for child to exit
                let status = child.wait()?;
                tracing::debug!("Claude exited with status: {}", status);

                // Wait for stderr thread
                let _ = stderr_thread.join();

                println!(); // Add blank line between responses
            }
            Err(e) => {
                tracing::debug!("Error reading input: {}", e);
                break;
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

    // Send registration
    let register_msg = CliToServer::Register {
        token: token.to_string(),
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
    let session_start_msg = CliToServer::SessionStart {
        session_id,
        working_dir: Some(working_dir.to_string()),
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
