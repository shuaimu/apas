use anyhow::Result;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;

pub struct ClaudeProcess {
    child: Child,
    stdin: ChildStdin,
}

impl ClaudeProcess {
    /// Spawn a new Claude Code process
    pub async fn spawn(
        claude_path: &str,
        working_dir: &Path,
    ) -> Result<(Self, mpsc::Receiver<String>, mpsc::Receiver<String>)> {
        let mut child = Command::new(claude_path)
            .current_dir(working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("Failed to get stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("Failed to get stdout"))?;
        let stderr = child.stderr.take().ok_or_else(|| anyhow::anyhow!("Failed to get stderr"))?;

        // Channel for stdout
        let (stdout_tx, stdout_rx) = mpsc::channel::<String>(100);
        // Channel for stderr
        let (stderr_tx, stderr_rx) = mpsc::channel::<String>(100);

        // Spawn task to read stdout
        let stdout_reader = BufReader::new(stdout);
        tokio::spawn(async move {
            let mut lines = stdout_reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if stdout_tx.send(line).await.is_err() {
                    break;
                }
            }
        });

        // Spawn task to read stderr
        let stderr_reader = BufReader::new(stderr);
        tokio::spawn(async move {
            let mut lines = stderr_reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if stderr_tx.send(line).await.is_err() {
                    break;
                }
            }
        });

        Ok((Self { child, stdin }, stdout_rx, stderr_rx))
    }

    /// Send input to the Claude process
    pub async fn send_input(&mut self, input: &str) -> Result<()> {
        self.stdin.write_all(input.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }

    /// Send a signal to the process
    pub fn send_signal(&mut self, signal: &str) -> Result<()> {
        #[cfg(unix)]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;

            if let Some(pid) = self.child.id() {
                let sig = match signal {
                    "SIGINT" => Signal::SIGINT,
                    "SIGTERM" => Signal::SIGTERM,
                    _ => return Err(anyhow::anyhow!("Unknown signal: {}", signal)),
                };
                kill(Pid::from_raw(pid as i32), sig)?;
            }
        }
        Ok(())
    }

    /// Wait for the process to exit
    pub async fn wait(&mut self) -> Result<std::process::ExitStatus> {
        Ok(self.child.wait().await?)
    }

    /// Check if the process is still running
    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        Ok(self.child.try_wait()?)
    }

    /// Kill the process
    pub async fn kill(&mut self) -> Result<()> {
        self.child.kill().await?;
        Ok(())
    }
}
