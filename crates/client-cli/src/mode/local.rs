use anyhow::Result;
use std::path::Path;

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
