use anyhow::Result;
use futures::{SinkExt, StreamExt};
use shared::{CliToServer, ClaudeStreamMessage, PaneType, ServerToCli};
use std::io::BufRead;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

use crate::config::Config;

const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(60);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const VERSION: &str = env!("APAS_VERSION");

/// Run in remote mode - connect to backend server and stream I/O
/// Automatically reconnects on connection loss with exponential backoff
pub async fn run(server_url: &str, token: &str, working_dir: &Path) -> Result<()> {
    let config = Config::load().unwrap_or_default();
    let claude_path = config.local.claude_path.clone();

    let mut reconnect_delay = INITIAL_RECONNECT_DELAY;
    let mut attempt = 0;

    loop {
        attempt += 1;

        match run_connection(server_url, token, working_dir, &claude_path).await {
            Ok(ConnectionResult::Shutdown) => {
                // Explicit shutdown requested
                tracing::info!("Shutting down");
                break;
            }
            Ok(ConnectionResult::Disconnected) => {
                // Server closed connection or we lost connectivity - reconnect
                // Reset backoff since we had a successful connection
                reconnect_delay = INITIAL_RECONNECT_DELAY;
                attempt = 0;
                println!("Connection lost. Reconnecting in {:?}...", reconnect_delay);
                tracing::warn!("Connection lost. Reconnecting in {:?}...", reconnect_delay);
            }
            Err(e) => {
                // Connection failed - use exponential backoff
                println!(
                    "Connection error: {}. Reconnecting in {:?}... (attempt {})",
                    e, reconnect_delay, attempt
                );
                tracing::error!(
                    "Connection error: {}. Reconnecting in {:?}... (attempt {})",
                    e,
                    reconnect_delay,
                    attempt
                );
                // Exponential backoff with max cap
                reconnect_delay = std::cmp::min(reconnect_delay * 2, MAX_RECONNECT_DELAY);
            }
        }

        tokio::time::sleep(reconnect_delay).await;
    }

    Ok(())
}

/// Result of a connection attempt
enum ConnectionResult {
    /// Connection was gracefully closed by server (reconnect)
    Disconnected,
    /// Client received shutdown signal (exit)
    Shutdown,
}

async fn run_connection(
    server_url: &str,
    token: &str,
    working_dir: &Path,
    claude_path: &str,
) -> Result<ConnectionResult> {
    // Connect to WebSocket
    let ws_url = format!("{}/ws/cli", server_url);
    tracing::info!("Connecting to {}...", ws_url);

    let (ws_stream, _) = connect_async(&ws_url).await?;
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // Send registration message with version
    let register_msg = CliToServer::Register {
        token: token.to_string(),
        version: Some(VERSION.to_string()),
    };
    let msg_text = serde_json::to_string(&register_msg)?;
    ws_sender.send(Message::Text(msg_text.into())).await?;

    // Wait for registration response
    let cli_id: Uuid;
    loop {
        match ws_receiver.next().await {
            Some(Ok(Message::Text(text))) => {
                let response: ServerToCli = serde_json::from_str(&text)?;
                match response {
                    ServerToCli::Registered { cli_id: id } => {
                        cli_id = id;
                        tracing::info!("Connected and registered as CLI {}", cli_id);
                        println!("Connected to server. CLI ID: {}", cli_id);
                        break;
                    }
                    ServerToCli::RegistrationFailed { reason } => {
                        return Err(anyhow::anyhow!("Registration failed: {}", reason));
                    }
                    ServerToCli::VersionUnsupported {
                        client_version,
                        min_version,
                    } => {
                        eprintln!("\n========================================");
                        eprintln!(
                            "ERROR: Client version {} is no longer supported!",
                            client_version
                        );
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

    // Channel for sending messages to WebSocket
    let (ws_tx, mut ws_rx) = mpsc::channel::<CliToServer>(32);

    // Input channels per pane_id — each pane gets its own handler
    let pane_inputs: std::sync::Arc<
        tokio::sync::Mutex<std::collections::HashMap<u32, mpsc::Sender<String>>>,
    > = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

    // If we have a project in the working directory, send SessionStart
    // so the server associates this CLI with the project's session,
    // then start a session handler per pane ready to receive input.
    if let Ok(project) = crate::project::get_or_create_project(working_dir) {
        let hostname = hostname::get()
            .ok()
            .and_then(|v| v.into_string().ok());
        let session_start = CliToServer::SessionStart {
            session_id: project.id,
            working_dir: Some(working_dir.to_string_lossy().to_string()),
            hostname,
            pane_type: None,
            panes: Some(project.panes.clone()),
        };
        let msg_text = serde_json::to_string(&session_start)?;
        ws_sender.send(Message::Text(msg_text.into())).await?;
        tracing::info!("Sent SessionStart for project {} (session {})", project.name.as_deref().unwrap_or("unnamed"), project.id);

        // Spawn a session handler for each pane
        let session_id = project.id;
        for pane in &project.panes {
            let pane_id = pane.pane_id;
            let claude_session_id = pane.session_id;
            let dir = working_dir.to_path_buf();
            let spawn_tx = ws_tx.clone();
            let spawn_inputs = pane_inputs.clone();
            let spawn_claude_path = claude_path.to_string();
            tokio::spawn(async move {
                if let Err(e) =
                    handle_pane(session_id, claude_session_id, pane_id, &spawn_claude_path, &dir, spawn_tx, spawn_inputs)
                        .await
                {
                    tracing::error!("Session {} pane {} error: {}", session_id, pane_id, e);
                }
            });
        }
    }

    // Task to send messages to WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(msg) = ws_rx.recv().await {
            let text = serde_json::to_string(&msg).unwrap();
            if ws_sender.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    // Heartbeat task
    let heartbeat_tx = ws_tx.clone();
    let heartbeat_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
        loop {
            interval.tick().await;
            if heartbeat_tx.send(CliToServer::Heartbeat).await.is_err() {
                break;
            }
        }
    });

    // Handle incoming messages from server
    let inputs = pane_inputs.clone();
    let ws_tx_clone = ws_tx.clone();
    let claude_path_owned = claude_path.to_string();
    let working_dir_owned = working_dir.to_path_buf();

    while let Some(msg_result) = ws_receiver.next().await {
        match msg_result {
            Ok(Message::Text(text)) => {
                let parsed: Result<ServerToCli, _> = serde_json::from_str(&text);
                match parsed {
                    Ok(ServerToCli::SessionAssigned {
                        session_id,
                        working_dir: wd,
                    }) => {
                        tracing::info!("Session assigned: {}", session_id);

                        // Spawn a default interactive pane handler
                        let dir = wd
                            .map(std::path::PathBuf::from)
                            .unwrap_or_else(|| working_dir_owned.clone());

                        let ws_tx = ws_tx_clone.clone();
                        let claude_path = claude_path_owned.clone();
                        let pane_inputs = inputs.clone();
                        let claude_session_id = Uuid::new_v4();

                        tokio::spawn(async move {
                            if let Err(e) =
                                handle_pane(session_id, claude_session_id, shared::PANE_ID_INTERACTIVE, &claude_path, &dir, ws_tx, pane_inputs)
                                    .await
                            {
                                tracing::error!("Session {} error: {}", session_id, e);
                            }
                        });
                    }
                    Ok(ServerToCli::Input {
                        session_id, data, pane_id,
                    }) => {
                        // Route input to the correct pane handler
                        let target_pane = pane_id.unwrap_or(shared::PANE_ID_INTERACTIVE);
                        let pane_inputs = inputs.lock().await;
                        if let Some(sender) = pane_inputs.get(&target_pane) {
                            let _ = sender.send(data).await;
                        } else {
                            tracing::warn!("No handler for pane {} in session {}", target_pane, session_id);
                        }
                    }
                    Ok(ServerToCli::Signal { session_id, signal }) => {
                        tracing::info!("Received signal {} for session {}", signal, session_id);
                        // TODO: Forward signal to Claude process
                    }
                    Ok(ServerToCli::SessionDisconnected { session_id }) => {
                        tracing::info!("Session {} disconnected from web", session_id);
                        // Process continues running, web client may reconnect
                    }
                    Ok(ServerToCli::Heartbeat) => {
                        // Heartbeat acknowledged
                    }
                    Ok(ServerToCli::Registered { .. })
                    | Ok(ServerToCli::RegistrationFailed { .. })
                    | Ok(ServerToCli::VersionUnsupported { .. }) => {
                        // Already handled during registration
                    }
                    Ok(ServerToCli::PauseDeadloop { .. })
                    | Ok(ServerToCli::ResumeDeadloop { .. })
                    | Ok(ServerToCli::PausePane { .. })
                    | Ok(ServerToCli::ResumePane { .. })
                    | Ok(ServerToCli::AddPane { .. })
                    | Ok(ServerToCli::RemovePane { .. })
                    | Ok(ServerToCli::StartBot { .. })
                    | Ok(ServerToCli::StopBot { .. })
                    | Ok(ServerToCli::RequestPaneList { .. }) => {
                        // Pause/resume/pane/bot management not supported in remote mode
                    }
                    Ok(ServerToCli::RebootCli { .. }) => {
                        tracing::info!("Reboot command received, restarting...");
                        crate::update::restart_cli();
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse server message: {}", e);
                    }
                }
            }
            Ok(Message::Ping(_)) => {
                // tungstenite auto-responds to ping
            }
            Ok(Message::Close(_)) => {
                tracing::info!("Server closed connection");
                break;
            }
            Err(e) => {
                tracing::error!("WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    // Cleanup
    heartbeat_task.abort();
    send_task.abort();

    // Return disconnected to trigger reconnection
    Ok(ConnectionResult::Disconnected)
}

/// Handle a single pane by spawning Claude per-message (like dual_pane mode).
/// Each user input spawns `claude --print --output-format stream-json --resume <id> <prompt>`.
async fn handle_pane(
    session_id: Uuid,
    claude_session_id: Uuid,
    pane_id: u32,
    claude_path: &str,
    working_dir: &Path,
    ws_tx: mpsc::Sender<CliToServer>,
    pane_inputs: std::sync::Arc<
        tokio::sync::Mutex<std::collections::HashMap<u32, mpsc::Sender<String>>>,
    >,
) -> Result<()> {
    tracing::info!(
        "Pane handler ready: session={}, claude_session={}, pane={}",
        session_id, claude_session_id, pane_id
    );

    // Channel for input to this pane
    let (input_tx, mut input_rx) = mpsc::channel::<String>(32);

    // Register this pane's input channel
    {
        let mut inputs = pane_inputs.lock().await;
        inputs.insert(pane_id, input_tx);
    }

    let mut first_message = true;
    let mut try_resume_first = true;
    let claude_sid = claude_session_id;
    let working_dir = working_dir.to_path_buf();
    let claude_path = claude_path.to_string();

    // Wait for input messages and spawn Claude per-message
    while let Some(prompt) = input_rx.recv().await {
        tracing::info!("Session {}: received input, spawning Claude", session_id);

        // Send status
        let _ = ws_tx
            .send(CliToServer::PaneStatus {
                session_id,
                pane_type: PaneType::Interactive,
                pane_id: Some(pane_id),
                status: Some("Thinking...".to_string()),
            })
            .await;

        // Build Claude args (same as dual_pane's build_agent_args for Claude)
        let mut args = vec![
            "--print".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
            "--dangerously-skip-permissions".to_string(),
        ];
        if first_message && try_resume_first {
            args.extend_from_slice(&[
                "--resume".to_string(),
                claude_sid.to_string(),
                prompt.clone(),
            ]);
        } else if first_message {
            args.extend_from_slice(&[
                "--session-id".to_string(),
                claude_sid.to_string(),
                prompt.clone(),
            ]);
            first_message = false;
        } else {
            args.extend_from_slice(&[
                "--resume".to_string(),
                claude_sid.to_string(),
                prompt.clone(),
            ]);
        }

        let using_resume = first_message && try_resume_first || !first_message;

        // Spawn Claude as one-shot process (stdin null, stdout piped)
        match std::process::Command::new(&claude_path)
            .args(&args)
            .current_dir(&working_dir)
            .env_remove("CLAUDECODE")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(mut child) => {
                let stdout = child.stdout.take().unwrap();
                let stderr = child.stderr.take().unwrap();

                // Read stdout in a thread (blocking I/O)
                let ws_tx_stdout = ws_tx.clone();
                let stdout_handle = std::thread::spawn(move || {
                    let reader = std::io::BufReader::new(stdout);
                    for line in reader.lines() {
                        match line {
                            Ok(line) => {
                                if line.trim().is_empty() {
                                    continue;
                                }
                                // Parse stream-json into ClaudeStreamMessage
                                if let Ok(message) =
                                    serde_json::from_str::<ClaudeStreamMessage>(&line)
                                {
                                    let msg = CliToServer::StreamMessage {
                                        session_id,
                                        message,
                                        pane_type: Some(PaneType::Interactive),
                                        pane_id: Some(pane_id),
                                    };
                                    if ws_tx_stdout.blocking_send(msg).is_err() {
                                        break;
                                    }
                                } else {
                                    // Non-JSON output — send as raw text
                                    tracing::debug!("Non-JSON stdout: {}", line);
                                }
                            }
                            Err(_) => break,
                        }
                    }
                });

                // Read stderr in a thread
                let stderr_handle = std::thread::spawn(move || {
                    let reader = std::io::BufReader::new(stderr);
                    for line in reader.lines() {
                        if let Ok(line) = line {
                            if !line.trim().is_empty() {
                                tracing::warn!("Claude stderr: {}", line);
                            }
                        }
                    }
                });

                // Wait for process to finish
                let exit_status = child.wait();
                let _ = stdout_handle.join();
                let _ = stderr_handle.join();

                // Clear status
                let _ = ws_tx
                    .send(CliToServer::PaneStatus {
                        session_id,
                        pane_type: PaneType::Interactive,
                        pane_id: Some(pane_id),
                        status: None,
                    })
                    .await;

                // Handle errors (e.g., --resume failed for non-existent session)
                let had_error = exit_status.map(|s| !s.success()).unwrap_or(true);
                if had_error && first_message && using_resume {
                    tracing::warn!(
                        "Claude --resume failed, will use --session-id on next message"
                    );
                    try_resume_first = false;
                } else if first_message {
                    first_message = false;
                }
            }
            Err(e) => {
                tracing::error!("Failed to spawn Claude: {}", e);
            }
        }
    }

    // Unregister pane
    {
        let mut inputs = pane_inputs.lock().await;
        inputs.remove(&pane_id);
    }

    tracing::info!("Pane {} for session {} ended", pane_id, session_id);
    Ok(())
}
