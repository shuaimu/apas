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
            // Check shutdown flag
            if shutdown_clone.load(Ordering::SeqCst) {
                break;
            }

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
/// This handles CSI, OSC, and other escape sequences comprehensively
fn strip_ansi_codes(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // ESC character - start of escape sequence
            match chars.peek() {
                Some('[') => {
                    chars.next(); // consume '['
                    // CSI sequence: ESC [ ... <letter>
                    // Skip parameters and intermediate bytes until final byte
                    while let Some(&next) = chars.peek() {
                        chars.next();
                        // Final byte is in range 0x40-0x7E (@ to ~)
                        if next >= '@' && next <= '~' {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next(); // consume ']'
                    // OSC sequence: ESC ] ... (BEL | ESC \)
                    // These are Operating System Commands like window title
                    while let Some(&next) = chars.peek() {
                        if next == '\x07' {
                            // BEL terminates OSC
                            chars.next();
                            break;
                        } else if next == '\x1b' {
                            // Check for ST (String Terminator): ESC \
                            chars.next();
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        } else {
                            chars.next();
                        }
                    }
                }
                Some('P') => {
                    chars.next(); // consume 'P'
                    // DCS sequence: ESC P ... ST
                    // Device Control String - skip until String Terminator
                    while let Some(&next) = chars.peek() {
                        if next == '\x1b' {
                            chars.next();
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        } else {
                            chars.next();
                        }
                    }
                }
                Some('(') | Some(')') | Some('*') | Some('+') => {
                    // Charset designation: ESC ( <char>
                    chars.next();
                    chars.next();
                }
                Some('#') | Some('%') => {
                    // Line size / charset: ESC # <digit> or ESC % <char>
                    chars.next();
                    chars.next();
                }
                Some(' ') => {
                    // 7/8-bit controls: ESC SP <char>
                    chars.next();
                    chars.next();
                }
                Some(c) if *c >= '0' && *c <= '~' => {
                    // Single character function: ESC <char>
                    chars.next();
                }
                _ => {
                    // Unknown, just skip ESC
                }
            }
        } else if c == '\u{009b}' {
            // CSI introduced by single byte (8-bit): C1 control code
            while let Some(&next) = chars.peek() {
                chars.next();
                if next >= '@' && next <= '~' {
                    break;
                }
            }
        } else if c == '\u{009d}' {
            // OSC introduced by single byte (8-bit): C1 control code
            while let Some(&next) = chars.peek() {
                if next == '\x07' || next == '\u{009c}' {
                    chars.next();
                    break;
                }
                chars.next();
            }
        } else if c == '\r' || c == '\x07' || c == '\x08' {
            // Skip carriage return, bell, and backspace
        } else if c.is_control() && c != '\n' && c != '\t' {
            // Skip other control characters except newline and tab
        } else {
            result.push(c);
        }
    }

    result
}

/// Spinner and progress indicator characters to filter out
const SPINNER_CHARS: &[char] = &[
    '✳', '✶', '✻', '✽', '✢', '✣', '✤', '✥',
    '●', '○', '◐', '◓', '◑', '◒', '◔', '◕',
    '◴', '◵', '◶', '◷', '◰', '◱', '◲', '◳',
    '⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏', // Braille spinner
    '⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷',
    '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█', // Progress bars
    '▏', '▎', '▍', '▌', '▋', '▊', '▉',
    '⏳', '⌛', '🔄',
];

/// Box drawing and decorative characters that are often just visual noise
const DECORATIVE_CHARS: &[char] = &[
    '─', '━', '│', '┃', '┌', '┐', '└', '┘', '├', '┤', '┬', '┴', '┼',
    '╭', '╮', '╯', '╰', '╱', '╲', '╳',
    '═', '║', '╔', '╗', '╚', '╝', '╠', '╣', '╦', '╩', '╬',
    '▶', '▷', '◀', '◁', '▲', '△', '▼', '▽',
    '❯', '❮', '›', '‹', '»', '«',
];

/// Clean up output for display - remove terminal artifacts and format for web
fn clean_output(s: &str) -> String {
    // First strip all ANSI escape sequences
    let stripped = strip_ansi_codes(s);

    // Handle the "]0;..." pattern that appears when OSC isn't fully stripped
    // This can happen if the ESC was already removed but the rest remains
    let mut cleaned = String::new();
    let mut chars = stripped.chars().peekable();

    while let Some(c) = chars.next() {
        // Detect orphaned OSC content: ]0;... or ]1;... etc
        // This handles cases where ESC was stripped but ]0;title remains
        if c == ']' {
            if let Some(&next) = chars.peek() {
                if next.is_ascii_digit() {
                    chars.next(); // consume digit
                    // Check for optional second digit
                    if let Some(&d) = chars.peek() {
                        if d.is_ascii_digit() {
                            chars.next();
                        }
                    }
                    if chars.peek() == Some(&';') {
                        chars.next(); // consume ';'
                        // Skip the title/content
                        // Since BEL may already be stripped, we look for:
                        // - BEL character
                        // - Another ] (start of new OSC)
                        // - Newline
                        // - Or transition from "title-like" chars to regular content
                        // Title chars are typically: letters, digits, spaces, and some punctuation
                        let mut saw_space = false;
                        while let Some(&ch) = chars.peek() {
                            if ch == '\x07' || ch == '\n' || ch == ']' {
                                if ch == '\x07' {
                                    chars.next();
                                }
                                break;
                            }
                            // Heuristic: titles don't usually have multiple consecutive spaces
                            // or contain certain characters that indicate start of real content
                            if ch == ' ' {
                                if saw_space {
                                    // Two spaces - probably end of title
                                    break;
                                }
                                saw_space = true;
                            } else {
                                saw_space = false;
                            }
                            // Check for spinner chars which indicate we've gone past the title
                            if SPINNER_CHARS.contains(&ch) {
                                break;
                            }
                            chars.next();
                        }
                        continue;
                    }
                }
            }
        }

        // Filter out spinner characters
        if SPINNER_CHARS.contains(&c) {
            continue;
        }

        // Filter out decorative box-drawing characters
        if DECORATIVE_CHARS.contains(&c) {
            // Replace with space to maintain word separation
            if !cleaned.ends_with(' ') && !cleaned.ends_with('\n') {
                cleaned.push(' ');
            }
            continue;
        }

        // Skip other control-like Unicode characters
        if c != ' ' && c != '\n' && c != '\t' {
            let cat = unicode_general_category(c);
            if cat == UnicodeCategory::Control ||
               cat == UnicodeCategory::Format ||
               cat == UnicodeCategory::PrivateUse {
                continue;
            }
        }

        cleaned.push(c);
    }

    // Normalize whitespace
    normalize_whitespace(&cleaned)
}

/// Simple Unicode general category detection for common cases
#[derive(PartialEq, Debug)]
enum UnicodeCategory {
    Control,
    Format,
    PrivateUse,
    Other,
}

fn unicode_general_category(c: char) -> UnicodeCategory {
    let cp = c as u32;

    // Control characters
    if cp <= 0x1F || (cp >= 0x7F && cp <= 0x9F) {
        return UnicodeCategory::Control;
    }

    // Format characters (common ranges)
    if cp == 0x00AD || // Soft hyphen
       (cp >= 0x0600 && cp <= 0x0605) ||
       cp == 0x061C ||
       cp == 0x06DD ||
       cp == 0x070F ||
       (cp >= 0x200B && cp <= 0x200F) || // Zero-width spaces
       (cp >= 0x2028 && cp <= 0x202E) || // Line/paragraph separators, bidi
       (cp >= 0x2060 && cp <= 0x206F) || // Word joiner, invisible operators
       cp == 0xFEFF || // BOM
       (cp >= 0xFFF9 && cp <= 0xFFFB) {
        return UnicodeCategory::Format;
    }

    // Private use areas
    if (cp >= 0xE000 && cp <= 0xF8FF) ||
       (cp >= 0xF0000 && cp <= 0xFFFFD) ||
       (cp >= 0x100000 && cp <= 0x10FFFD) {
        return UnicodeCategory::PrivateUse;
    }

    UnicodeCategory::Other
}

/// Normalize whitespace: collapse multiple spaces/newlines, trim
fn normalize_whitespace(s: &str) -> String {
    let mut result = String::new();
    let mut last_was_space = false;
    let mut last_was_newline = false;
    let mut pending_newline = false;

    for c in s.chars() {
        match c {
            '\n' => {
                if !last_was_newline && !result.is_empty() {
                    pending_newline = true;
                    last_was_newline = true;
                }
                last_was_space = true;
            }
            ' ' | '\t' => {
                if !last_was_space && !result.is_empty() {
                    // Don't add space yet, wait to see if there's content
                    last_was_space = true;
                }
            }
            _ => {
                // We have actual content
                if pending_newline {
                    result.push('\n');
                    pending_newline = false;
                    last_was_space = false;
                } else if last_was_space && !result.is_empty() {
                    result.push(' ');
                }
                result.push(c);
                last_was_space = false;
                last_was_newline = false;
            }
        }
    }

    result
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

    // ==================== strip_ansi_codes tests ====================

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
        // Cursor movement
        assert_eq!(strip_ansi_codes("\x1b[10;20Htext"), "text");
        assert_eq!(strip_ansi_codes("\x1b[?25l\x1b[?25h"), ""); // Hide/show cursor
    }

    #[test]
    fn test_strip_ansi_codes_osc_sequences() {
        // OSC sequences (window title, etc.)
        assert_eq!(strip_ansi_codes("\x1b]0;Window Title\x07text"), "text");
        assert_eq!(strip_ansi_codes("prefix\x1b]0;title\x07suffix"), "prefixsuffix");
        // OSC with ST terminator
        assert_eq!(strip_ansi_codes("\x1b]0;Title\x1b\\text"), "text");
    }

    #[test]
    fn test_strip_ansi_codes_dcs_sequences() {
        // Device Control String sequences
        assert_eq!(strip_ansi_codes("\x1bPsome data\x1b\\text"), "text");
    }

    #[test]
    fn test_strip_ansi_codes_control_characters() {
        // Carriage return, bell, and backspace should be stripped
        assert_eq!(strip_ansi_codes("hello\rworld"), "helloworld");
        assert_eq!(strip_ansi_codes("beep\x07beep"), "beepbeep");
        assert_eq!(strip_ansi_codes("back\x08space"), "backspace");
    }

    #[test]
    fn test_strip_ansi_codes_preserves_newlines_and_tabs() {
        assert_eq!(strip_ansi_codes("line1\nline2\ttabbed"), "line1\nline2\ttabbed");
    }

    #[test]
    fn test_strip_ansi_codes_8bit_controls() {
        // 8-bit CSI (0x9B) and OSC (0x9D) - C1 control codes
        assert_eq!(strip_ansi_codes("\u{009b}32mtext\u{009b}0m"), "text");
        assert_eq!(strip_ansi_codes("\u{009d}title\x07text"), "text");
    }

    // ==================== clean_output tests ====================

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
        // Braille spinners
        assert_eq!(clean_output("⠋⠙⠹loading"), "loading");
    }

    #[test]
    fn test_clean_output_removes_box_drawing() {
        // Box drawing characters should be replaced with spaces
        let input = "───text───";
        let output = clean_output(input);
        assert_eq!(output, "text");
    }

    #[test]
    fn test_clean_output_removes_decorative_arrows() {
        assert_eq!(clean_output("❯ prompt"), "prompt");
        assert_eq!(clean_output("▶ playing"), "playing");
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
        assert_eq!(clean_output("Ñoño café"), "Ñoño café");
    }

    #[test]
    fn test_clean_output_orphaned_osc() {
        // When ESC is stripped but ]0;title remains
        // Note: BEL (\x07) gets stripped by strip_ansi_codes before clean_output sees it
        // So we rely on heuristics like double spaces or spinner chars to find title end

        // Title followed by double space and content
        assert_eq!(clean_output("]0;Window Title  actual content"), "actual content");

        // Title followed by spinner char
        assert_eq!(clean_output("]0;Window Title✳ content"), "content");

        // Real-world pattern: OSC stripped but title visible, then content
        assert_eq!(clean_output("]0;My Title  hello world"), "hello world");

        // Multiple OSC patterns
        assert_eq!(clean_output("]0;first  ]1;second  text"), "text");
    }

    #[test]
    fn test_clean_output_real_world_claude_spinner() {
        // Simulating Claude's spinner output pattern
        let input = "\x1b]0;✳ Initial Greeting\x07\x1b[2K\x1b[1G✳ hello · Blanching…";
        let output = clean_output(input);
        // Should not contain spinner or control sequences
        assert!(!output.contains("✳"));
        assert!(!output.contains('\x1b'));
        assert!(!output.contains(']'));
        // Should contain actual content
        assert!(output.contains("hello") || output.contains("Blanching"));
    }

    #[test]
    fn test_clean_output_complex_terminal_output() {
        // More complex real-world terminal output
        let input = "\x1b[2K\x1b[1G❯ \x1b[32mhello\x1b[0m\n\x1b[2K✳ Processing...\r✳ Done!";
        let output = clean_output(input);
        assert!(output.contains("hello"));
        assert!(!output.contains("✳"));
        assert!(!output.contains("❯"));
    }

    #[test]
    fn test_clean_output_removes_zero_width_chars() {
        // Zero-width spaces and joiners
        let input = "hello\u{200B}world\u{200C}test\u{200D}end";
        let output = clean_output(input);
        assert_eq!(output, "helloworldtestend");
    }

    #[test]
    fn test_normalize_whitespace() {
        assert_eq!(normalize_whitespace("a  b"), "a b");
        assert_eq!(normalize_whitespace("a\n\nb"), "a\nb");
        assert_eq!(normalize_whitespace("  a  "), "a");
        assert_eq!(normalize_whitespace("a \n b"), "a\nb");
    }

    #[test]
    fn test_unicode_category_detection() {
        assert_eq!(unicode_general_category('\x00'), UnicodeCategory::Control);
        assert_eq!(unicode_general_category('\x1b'), UnicodeCategory::Control);
        assert_eq!(unicode_general_category('\u{200B}'), UnicodeCategory::Format);
        assert_eq!(unicode_general_category('\u{FEFF}'), UnicodeCategory::Format);
        assert_eq!(unicode_general_category('\u{E000}'), UnicodeCategory::PrivateUse);
        assert_eq!(unicode_general_category('a'), UnicodeCategory::Other);
        assert_eq!(unicode_general_category('日'), UnicodeCategory::Other);
    }
}
