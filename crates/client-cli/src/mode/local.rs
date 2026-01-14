use anyhow::Result;
use std::path::Path;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::config::Config;

/// Run in local mode - transparent pass-through to Claude Code
pub async fn run(working_dir: &Path) -> Result<()> {
    let config = Config::load().unwrap_or_default();
    let claude_path = &config.local.claude_path;

    tracing::debug!("Starting Claude Code from: {}", claude_path);

    // Spawn Claude Code process with inherited stdio for full transparency
    let status = tokio::process::Command::new(claude_path)
        .current_dir(working_dir)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .await?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

/// Run in local mode with captured I/O (for testing or special cases)
pub async fn run_captured(working_dir: &Path) -> Result<()> {
    let config = Config::load().unwrap_or_default();
    let claude_path = &config.local.claude_path;

    let (mut claude, mut stdout_rx, mut stderr_rx) =
        crate::claude::ClaudeProcess::spawn(claude_path, working_dir).await?;

    // Task to read stdin and forward to Claude
    let stdin_task = tokio::spawn(async move {
        let stdin = io::stdin();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            if claude.send_input(&line).await.is_err() {
                break;
            }
        }
    });

    // Task to print stdout
    let stdout_task = tokio::spawn(async move {
        let mut stdout = io::stdout();
        while let Some(line) = stdout_rx.recv().await {
            let _ = stdout.write_all(line.as_bytes()).await;
            let _ = stdout.write_all(b"\n").await;
            let _ = stdout.flush().await;
        }
    });

    // Task to print stderr
    let stderr_task = tokio::spawn(async move {
        let mut stderr = io::stderr();
        while let Some(line) = stderr_rx.recv().await {
            let _ = stderr.write_all(line.as_bytes()).await;
            let _ = stderr.write_all(b"\n").await;
            let _ = stderr.flush().await;
        }
    });

    // Wait for all tasks
    let _ = tokio::join!(stdin_task, stdout_task, stderr_task);

    Ok(())
}
