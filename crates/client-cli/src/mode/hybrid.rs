//! Hybrid mode - local interactive terminal + streaming to remote server
//!
//! This mode runs Claude locally with full interactive terminal support
//! while also streaming all output to the remote server for observation.

use anyhow::Result;
use futures::{SinkExt, StreamExt};
use shared::{CliToServer, OutputType, ServerToCli};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

use crate::config::Config;
use crate::pty::{self, PtyProcess};

const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(60);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// Run in hybrid mode - local interactive terminal + streaming to remote server
pub async fn run(server_url: &str, token: &str, working_dir: &Path) -> Result<()> {
    let config = Config::load().unwrap_or_default();
    let claude_path = config.local.claude_path.clone();

    // Generate a session ID for this local session
    let session_id = Uuid::new_v4();

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

    // Set terminal to raw mode
    let original_termios = pty::set_raw_mode()?;

    // Ensure we restore terminal on exit
    let result = run_pty_session(&claude_path, working_dir, session_id, server_tx, &shutdown).await;

    // Restore terminal
    let _ = pty::restore_terminal(&original_termios);
    println!(); // New line after session ends

    // Signal shutdown
    shutdown.store(true, Ordering::SeqCst);

    result
}

async fn run_pty_session(
    claude_path: &str,
    working_dir: &Path,
    session_id: Uuid,
    server_tx: mpsc::Sender<CliToServer>,
    shutdown: &Arc<AtomicBool>,
) -> Result<()> {
    // Spawn Claude in a PTY
    let pty_process = PtyProcess::spawn(claude_path, working_dir)?;
    let master_fd = pty_process.master_fd();

    // Clone stdin fd for reading
    let stdin_fd = std::io::stdin().as_raw_fd();

    // Use tokio's blocking task for PTY I/O since PTY fds don't work well with async
    let shutdown_clone = shutdown.clone();

    // Spawn a thread for stdin -> PTY
    let master_fd_write = master_fd;
    let stdin_thread = std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 1024];

        loop {
            // Use select/poll to check if stdin has data
            unsafe {
                let mut fds: libc::fd_set = std::mem::zeroed();
                libc::FD_SET(stdin_fd, &mut fds);

                let mut timeout = libc::timeval {
                    tv_sec: 0,
                    tv_usec: 100_000, // 100ms timeout
                };

                let result = libc::select(
                    stdin_fd + 1,
                    &mut fds,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut timeout,
                );

                if result > 0 && libc::FD_ISSET(stdin_fd, &fds) {
                    match stdin.read(&mut buf) {
                        Ok(0) => break, // EOF
                        Ok(n) => {
                            // Write to PTY master
                            let _ = libc::write(master_fd_write, buf.as_ptr() as *const libc::c_void, n);
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    });

    // Main loop: read from PTY and write to stdout + server
    let mut stdout = std::io::stdout();
    let mut buf = [0u8; 4096];
    let mut line_buffer = String::new();

    loop {
        // Check if child has exited
        if let Some(exit_code) = pty_process.try_wait() {
            tracing::debug!("Claude process exited with code: {}", exit_code);
            break;
        }

        // Read from PTY
        match pty_process.read(&mut buf) {
            Ok(0) => {
                // No data available, sleep briefly
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(n) => {
                let data = &buf[..n];

                // Write to local stdout
                let _ = stdout.write_all(data);
                let _ = stdout.flush();

                // Buffer and send to server (accumulate until newline for cleaner output)
                if let Ok(text) = std::str::from_utf8(data) {
                    line_buffer.push_str(text);

                    // Send complete lines to server
                    while let Some(pos) = line_buffer.find('\n') {
                        let line = line_buffer[..pos].to_string();
                        line_buffer = line_buffer[pos + 1..].to_string();

                        // Clean output for readable server display
                        let cleaned = clean_output(&line);
                        if !cleaned.is_empty() {
                            let _ = server_tx.try_send(CliToServer::Output {
                                session_id,
                                data: cleaned,
                                output_type: OutputType::Text,
                            });
                        }
                    }
                }
            }
            Err(e) => {
                tracing::debug!("PTY read error: {}", e);
                break;
            }
        }
    }

    // Send any remaining buffered content
    if !line_buffer.is_empty() {
        let cleaned = clean_output(&line_buffer);
        if !cleaned.is_empty() {
            let _ = server_tx.try_send(CliToServer::Output {
                session_id,
                data: cleaned,
                output_type: OutputType::Text,
            });
        }
    }

    // Signal shutdown
    shutdown.store(true, Ordering::SeqCst);

    // Wait for stdin thread (with timeout)
    let _ = stdin_thread.join();

    Ok(())
}

/// Strip ANSI escape codes and control characters from a string
fn strip_ansi_codes(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip ESC sequences
            match chars.peek() {
                Some('[') => {
                    chars.next(); // consume '['
                    // CSI sequence - skip until letter
                    while let Some(&next) = chars.peek() {
                        chars.next();
                        if next.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next(); // consume ']'
                    // OSC sequence - skip until BEL or ST
                    while let Some(&next) = chars.peek() {
                        chars.next();
                        if next == '\x07' || next == '\\' {
                            break;
                        }
                    }
                }
                Some('(') | Some(')') | Some('*') | Some('+') => {
                    chars.next();
                    chars.next(); // Skip charset designation
                }
                _ => {
                    chars.next(); // Skip single char after ESC
                }
            }
        } else if c == '\r' || c == '\x07' {
            // Skip carriage return and bell
        } else if c.is_control() && c != '\n' && c != '\t' {
            // Skip other control characters except newline and tab
        } else {
            result.push(c);
        }
    }

    result
}

/// Clean up output for display - remove spinner characters and excessive whitespace
fn clean_output(s: &str) -> String {
    let stripped = strip_ansi_codes(s);

    // Remove common spinner/progress characters
    let cleaned: String = stripped
        .chars()
        .filter(|c| {
            // Keep normal printable characters, newlines, tabs, spaces
            c.is_ascii_graphic() || *c == ' ' || *c == '\n' || *c == '\t'
                || (*c as u32 > 127 && !matches!(*c, '✳' | '✶' | '✻' | '✽' | '✢' | '●' | '◐' | '◓' | '◑' | '◒'))
        })
        .collect();

    // Collapse multiple spaces/newlines
    let mut result = String::new();
    let mut last_was_whitespace = false;
    let mut last_was_newline = false;

    for c in cleaned.chars() {
        if c == '\n' {
            if !last_was_newline {
                result.push(c);
                last_was_newline = true;
            }
            last_was_whitespace = true;
        } else if c == ' ' || c == '\t' {
            if !last_was_whitespace {
                result.push(' ');
                last_was_whitespace = true;
            }
            last_was_newline = false;
        } else {
            result.push(c);
            last_was_whitespace = false;
            last_was_newline = false;
        }
    }

    result.trim().to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_ansi_codes_basic() {
        // Plain text should pass through unchanged
        assert_eq!(strip_ansi_codes("hello world"), "hello world");
        assert_eq!(strip_ansi_codes("line1\nline2"), "line1\nline2");
    }

    #[test]
    fn test_strip_ansi_codes_csi_sequences() {
        // CSI sequences (colors, cursor movement, etc.)
        assert_eq!(strip_ansi_codes("\x1b[32mgreen\x1b[0m"), "green");
        assert_eq!(strip_ansi_codes("\x1b[1;31mbold red\x1b[0m"), "bold red");
        assert_eq!(strip_ansi_codes("\x1b[2Kcleared line"), "cleared line");
    }

    #[test]
    fn test_strip_ansi_codes_osc_sequences() {
        // OSC sequences (window title, etc.)
        assert_eq!(strip_ansi_codes("\x1b]0;Window Title\x07text"), "text");
        assert_eq!(strip_ansi_codes("prefix\x1b]0;title\x07suffix"), "prefixsuffix");
    }

    #[test]
    fn test_strip_ansi_codes_control_characters() {
        // Carriage return and bell should be stripped
        assert_eq!(strip_ansi_codes("hello\rworld"), "helloworld");
        assert_eq!(strip_ansi_codes("beep\x07beep"), "beepbeep");
    }

    #[test]
    fn test_strip_ansi_codes_preserves_newlines_and_tabs() {
        assert_eq!(strip_ansi_codes("line1\nline2\ttabbed"), "line1\nline2\ttabbed");
    }

    #[test]
    fn test_clean_output_basic() {
        assert_eq!(clean_output("hello world"), "hello world");
    }

    #[test]
    fn test_clean_output_collapses_whitespace() {
        assert_eq!(clean_output("hello   world"), "hello world");
        assert_eq!(clean_output("  leading space"), "leading space");
        assert_eq!(clean_output("trailing space  "), "trailing space");
    }

    #[test]
    fn test_clean_output_collapses_newlines() {
        assert_eq!(clean_output("line1\n\n\nline2"), "line1\nline2");
    }

    #[test]
    fn test_clean_output_removes_spinner_characters() {
        assert_eq!(clean_output("Loading✳..."), "Loading...");
        assert_eq!(clean_output("●◐◓◑◒"), "");
        assert_eq!(clean_output("text ✳ more text"), "text more text");
    }

    #[test]
    fn test_clean_output_with_ansi_codes() {
        // Combined test - ANSI codes + whitespace + spinners
        let input = "\x1b[32m  hello  \x1b[0m  ✳  \x1b[1mworld\x1b[0m  ";
        let output = clean_output(input);
        assert_eq!(output, "hello world");
    }

    #[test]
    fn test_clean_output_preserves_unicode() {
        // Regular unicode should pass through
        assert_eq!(clean_output("日本語テスト"), "日本語テスト");
        assert_eq!(clean_output("emoji 👋 test"), "emoji 👋 test");
    }

    #[test]
    fn test_clean_output_real_world_example() {
        // Simulating Claude's spinner output
        let input = "\x1b]0;✳ Initial Greeting\x07\x1b[2K\x1b[1G✳ hello · Blanching…";
        let output = clean_output(input);
        // Should not contain spinner or control sequences
        assert!(!output.contains("✳"));
        assert!(!output.contains('\x1b'));
    }
}
