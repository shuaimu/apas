use anyhow::Result;
use futures::{SinkExt, StreamExt};
use shared::{CliToServer, OutputType, ServerToCli};
use std::path::Path;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

use crate::claude::ClaudeProcess;
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

    // Channel for sending messages to WebSocket
    let (ws_tx, mut ws_rx) = mpsc::channel::<CliToServer>(32);

    // Active Claude processes per session
    let claude_processes: std::sync::Arc<
        tokio::sync::Mutex<std::collections::HashMap<Uuid, mpsc::Sender<String>>>,
    > = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

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
    let processes = claude_processes.clone();
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

                        // Spawn Claude process for this session
                        let dir = wd
                            .map(std::path::PathBuf::from)
                            .unwrap_or_else(|| working_dir_owned.clone());

                        let ws_tx = ws_tx_clone.clone();
                        let claude_path = claude_path_owned.clone();
                        let processes = processes.clone();

                        tokio::spawn(async move {
                            if let Err(e) =
                                handle_session(session_id, &claude_path, &dir, ws_tx, processes)
                                    .await
                            {
                                tracing::error!("Session {} error: {}", session_id, e);
                            }
                        });
                    }
                    Ok(ServerToCli::Input { session_id, data }) => {
                        // Forward input to the appropriate Claude process
                        let processes = processes.lock().await;
                        if let Some(sender) = processes.get(&session_id) {
                            let _ = sender.send(data).await;
                        }
                    }
                    Ok(ServerToCli::Signal { session_id, signal }) => {
                        tracing::info!(
                            "Received signal {} for session {}",
                            signal,
                            session_id
                        );
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
                    | Ok(ServerToCli::ResumeDeadloop { .. }) => {
                        // Pause/resume not supported in remote mode
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

async fn handle_session(
    session_id: Uuid,
    claude_path: &str,
    working_dir: &Path,
    ws_tx: mpsc::Sender<CliToServer>,
    processes: std::sync::Arc<
        tokio::sync::Mutex<std::collections::HashMap<Uuid, mpsc::Sender<String>>>,
    >,
) -> Result<()> {
    tracing::info!("Starting Claude process for session {}", session_id);

    // Spawn Claude process
    let (mut claude, mut stdout_rx, mut stderr_rx) =
        ClaudeProcess::spawn(claude_path, working_dir).await?;

    // Channel for input to this Claude process
    let (input_tx, mut input_rx) = mpsc::channel::<String>(32);

    // Register this process
    {
        let mut procs = processes.lock().await;
        procs.insert(session_id, input_tx);
    }

    // Task to forward stdout to server
    let ws_tx_stdout = ws_tx.clone();
    let stdout_task = tokio::spawn(async move {
        while let Some(line) = stdout_rx.recv().await {
            let msg = CliToServer::Output {
                session_id,
                data: line,
                output_type: OutputType::Text,
            };
            if ws_tx_stdout.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Task to forward stderr to server
    let ws_tx_stderr = ws_tx.clone();
    let stderr_task = tokio::spawn(async move {
        while let Some(line) = stderr_rx.recv().await {
            let msg = CliToServer::Output {
                session_id,
                data: line,
                output_type: OutputType::Error,
            };
            if ws_tx_stderr.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Task to forward input from server to Claude
    let input_task = tokio::spawn(async move {
        while let Some(input) = input_rx.recv().await {
            if claude.send_input(&input).await.is_err() {
                break;
            }
        }
        // Wait for process to exit
        let _ = claude.wait().await;
    });

    // Wait for process to complete
    let _ = tokio::join!(stdout_task, stderr_task, input_task);

    // Unregister process
    {
        let mut procs = processes.lock().await;
        procs.remove(&session_id);
    }

    // Notify server that session ended
    let _ = ws_tx
        .send(CliToServer::SessionEnd {
            session_id,
            reason: "Process exited".to_string(),
        })
        .await;

    tracing::info!("Session {} ended", session_id);
    Ok(())
}
