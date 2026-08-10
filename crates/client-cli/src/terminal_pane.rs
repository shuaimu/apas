//! Pty-hosted terminal panes (`shared::PaneKind::Terminal`).
//!
//! The legacy/managed-team pane kind runs a provider headlessly
//! (`claude --print --output-format stream-json`) and parses structured
//! events. The normal user-created terminal pane instead allocates a pty, execs
//! the provider's **real interactive TUI**, and ships the raw bytes to the
//! browser where xterm.js renders them. Nothing is parsed, so nothing
//! needs to be kept in sync with a provider's output
//! format — the point of the feature is to reuse the CLI as it ships.
//!
//! The only flags passed are the provider's exact resume arguments (on
//! restore) and its permission-bypass flag: a pane driven from a browser has
//! nobody at the keyboard to answer an approval prompt, and agent panes
//! already launch with the same bypass. See [`permission_bypass_flag_for`].
//!
//! The cost is that a terminal pane has none of the structured
//! integrations: no usage counters, no pane status, no diffs, no plan
//! review, no scratchpad publishing, and it is never a Tech Lead
//! delegation target. See the `PaneKind` docs in `shared`.
//!
//! Lifetime: the pty is a child of the CLI process, so a terminal pane
//! dies when `apas` restarts. [`TerminalHandle::spawn`] therefore re-execs the
//! provider and asks it to resume the pane's conversation. Claude can resume
//! the exact pinned session id; codex currently exposes only its own resume
//! picker/subcommand here.

use anyhow::{Context, Result};
use base64::Engine as _;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use shared::{CliToServer, Provider, TerminalLifecycle};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::mpsc as tokio_mpsc;
use uuid::Uuid;

/// Registry of live terminal panes, keyed by `pane_id`.
///
/// Separate from the agent panes' `InputChannels` on purpose: agent input
/// is a prompt string routed into a turn loop, terminal input is raw
/// keystrokes written straight to a pty. Keeping them apart means the
/// `Terminal*` messages never touch the agent path.
pub type TerminalPanes = Arc<Mutex<HashMap<u32, TerminalHandle>>>;

/// Bytes pulled from the pty per read. Large enough that a TUI redraw
/// usually lands in one frame, small enough to stay interactive.
const READ_CHUNK_BYTES: usize = 8 * 1024;

/// Fallback size used between spawn and the browser's first
/// `TerminalResize`. A TUI that renders at 80x24 and then reflows is
/// normal; starting at 0x0 is not (some TUIs divide by the row count).
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// Which binaries a terminal pane may host.
///
/// Deliberately not every [`Provider`]: only Claude and Codex have verified
/// terminal behavior. Other providers have interaction models that we have
/// not verified render correctly through a bare pty. Returning `None`
/// here makes the caller reject the pane rather than spawn something that
/// paints garbage into xterm.js.
pub fn terminal_binary_for(provider: &Provider) -> Option<&'static str> {
    #[allow(deprecated)]
    match provider {
        Provider::Claude => Some("claude"),
        Provider::Codex => Some("codex"),
        Provider::Minimax
        | Provider::Glm
        | Provider::Deepseek
        | Provider::Opencode
        | Provider::CursorAgent => None,
    }
}

/// The provider's own "pick up where we left off" arguments.
///
/// Claude must receive the pane's exact pinned id. `--continue` is not an
/// equivalent: it selects the most recent conversation for the cwd, which can
/// belong to another pane while APAS continues watching this pane's pinned
/// transcript. Codex's `resume` remains a subcommand and therefore must be
/// appended before its permission flag.
fn resume_args_for(provider: &Provider, conversation_id: Uuid) -> Vec<String> {
    match provider {
        Provider::Claude => vec!["--resume".to_string(), conversation_id.to_string()],
        Provider::Codex => vec!["resume".to_string()],
        _ => Vec::new(),
    }
}

/// The provider's "don't stop to ask me" flag.
///
/// A terminal pane is driven from a browser, so an interactive approval prompt
/// is a dead end in practice: the TUI blocks until someone notices the tab.
/// Agent panes already launch with exactly these flags (see
/// `build_agent_args`), so this makes the two pane kinds behave consistently
/// rather than introducing a new policy.
///
/// Verified present on the *interactive* forms of both binaries, not just the
/// headless ones — `claude --help` and `codex --help`. That matters because an
/// unrecognised flag doesn't degrade, it fails the spawn outright.
fn permission_bypass_flag_for(provider: &Provider) -> Option<&'static str> {
    match provider {
        Provider::Claude => Some("--dangerously-skip-permissions"),
        Provider::Codex => Some("--dangerously-bypass-approvals-and-sandbox"),
        _ => None,
    }
}

/// A running pty and the handles needed to drive it.
///
/// Cloneable so the message-routing loop can hold one while the reader
/// thread owns another; all shared state is behind `Arc`.
#[derive(Clone)]
pub struct TerminalHandle {
    pane_id: u32,
    /// Stable for exactly one spawned provider process, including across
    /// any number of CLI WebSocket reconnects.
    instance_id: Uuid,
    /// Writer half of the pty master. Keystrokes go here.
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// Master pty, retained for `TIOCSWINSZ` on resize.
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
    /// Per-pane monotonic chunk counter shared with the reader thread.
    seq: Arc<AtomicU64>,
    /// Set by [`TerminalHandle::shutdown`] so the reader thread exits
    /// quietly instead of reporting the kill as an unexpected exit.
    shutting_down: Arc<AtomicBool>,
    /// Last size we applied, so a redundant resize doesn't churn the TUI.
    last_size: Arc<Mutex<(u16, u16)>>,
    /// Retained after the reader exits so a later reconnect can report a
    /// dead provider even though no browser saw the live exit event.
    lifecycle: Arc<Mutex<(TerminalLifecycle, Option<String>)>>,
}

impl TerminalHandle {
    /// Allocate a pty, spawn `provider`'s interactive TUI in it, and start
    /// streaming output to the server.
    ///
    /// `cwd` should already account for an isolated worktree when the pane
    /// has one — this function does not resolve worktrees.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        pane_id: u32,
        session_id: Uuid,
        // The pane's own conversation id, pinned so its transcript can be
        // located deterministically.
        claude_session_id: Uuid,
        provider: &Provider,
        binary_path: &str,
        cwd: &str,
        env: &[(String, String)],
        resume: bool,
        initial_prompt: Option<&str>,
        server_tx: tokio_mpsc::Sender<CliToServer>,
    ) -> Result<Self> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("failed to allocate pty for terminal pane")?;

        let mut cmd = CommandBuilder::new(binary_path);
        // Order matters: codex's resume flag is really a *subcommand*
        // (`codex resume`), and its bypass flag has to follow it. Claude's are
        // both plain top-level flags, so it is order-insensitive. Appending
        // resume first satisfies both.
        if resume {
            for arg in resume_args_for(provider, claude_session_id) {
                cmd.arg(arg);
            }
        }
        if let Some(flag) = permission_bypass_flag_for(provider) {
            cmd.arg(flag);
        }
        // Pin claude's session id to the one APAS already minted for this pane.
        // That makes the transcript path exact -- `~/.claude/projects/<cwd with
        // / as ->/<id>.jsonl` -- which is what lets the CLI read this pane's
        // history without guessing which file is whose. Codex has no
        // equivalent flag; see `transcript::find_codex_rollout`.
        //
        // Deliberately skipped when resuming: claude receives this same id via
        // `--resume <id>` above, while codex has no equivalent pinning flag.
        if !resume && matches!(provider, Provider::Claude) {
            cmd.arg("--session-id");
            cmd.arg(claude_session_id.to_string());
        }
        // Both supported interactive CLIs accept a positional first prompt.
        // Supplying it at process creation is atomic and avoids racing raw
        // keystrokes against a full-screen TUI that has not initialized yet.
        // A restored pane uses the provider's resume flow instead and must not
        // replay an old instruction.
        if !resume {
            if let Some(prompt) = initial_prompt.map(str::trim).filter(|p| !p.is_empty()) {
                cmd.arg(prompt);
            }
        }
        cmd.cwd(cwd);
        // A TUI keys its capabilities off TERM. Without this it inherits
        // whatever the daemon-spawned CLI had — often `dumb`, which makes
        // claude/codex fall back to a line-mode renderer that looks broken
        // in xterm.js. COLORTERM is what unlocks 24-bit colour.
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        for (k, v) in env {
            cmd.env(k, v);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("failed to spawn {binary_path} in pty"))?;

        // Drop the slave in the parent: while we hold it open the pty
        // never signals EOF, so the reader thread would hang forever
        // after the child exits instead of reporting the exit.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .context("failed to clone pty reader")?;
        let writer = pair
            .master
            .take_writer()
            .context("failed to take pty writer")?;

        let handle = Self {
            pane_id,
            instance_id: Uuid::new_v4(),
            writer: Arc::new(Mutex::new(writer)),
            master: Arc::new(Mutex::new(pair.master)),
            child: Arc::new(Mutex::new(child)),
            seq: Arc::new(AtomicU64::new(0)),
            shutting_down: Arc::new(AtomicBool::new(false)),
            last_size: Arc::new(Mutex::new((DEFAULT_COLS, DEFAULT_ROWS))),
            lifecycle: Arc::new(Mutex::new((TerminalLifecycle::Running, None))),
        };

        handle.start_reader(session_id, reader, server_tx);

        tracing::info!(
            pane_id,
            binary = %binary_path,
            cwd = %cwd,
            resume,
            "spawned terminal pane"
        );
        Ok(handle)
    }

    /// Pump pty output to the server on a dedicated OS thread.
    ///
    /// A blocking read is the only portable way to drain a pty, so this
    /// cannot live on the tokio runtime; `blocking_send` applies real
    /// backpressure instead of dropping frames, which matters because a
    /// dropped chunk mid-escape-sequence corrupts the emulator.
    fn start_reader(
        &self,
        session_id: Uuid,
        mut reader: Box<dyn Read + Send>,
        server_tx: tokio_mpsc::Sender<CliToServer>,
    ) {
        let pane_id = self.pane_id;
        let seq = Arc::clone(&self.seq);
        let shutting_down = Arc::clone(&self.shutting_down);
        let child = Arc::clone(&self.child);
        let lifecycle = Arc::clone(&self.lifecycle);
        let instance_id = self.instance_id;

        thread::Builder::new()
            .name(format!("apas-term-{pane_id}"))
            .spawn(move || {
                // Announce the generation from this blocking thread before it
                // can emit output. `spawn` may itself run inside tokio, where
                // `blocking_send` would panic.
                let _ = server_tx.blocking_send(CliToServer::TerminalState {
                    session_id,
                    pane_id,
                    instance_id: Some(instance_id),
                    lifecycle: TerminalLifecycle::Running,
                    status: None,
                });
                let mut buf = vec![0u8; READ_CHUNK_BYTES];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let data_b64 =
                                base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                            let msg = CliToServer::TerminalOutput {
                                session_id,
                                pane_id,
                                instance_id: Some(instance_id),
                                data_b64,
                                seq: seq.fetch_add(1, Ordering::Relaxed),
                            };
                            if server_tx.blocking_send(msg).is_err() {
                                // Server channel closed — the CLI is
                                // shutting down; nothing left to send to.
                                return;
                            }
                        }
                        Err(e) => {
                            // EIO on a pty master is the normal way a
                            // Linux read reports "child hung up", not a
                            // fault worth surfacing.
                            if e.kind() != std::io::ErrorKind::Other {
                                tracing::debug!(pane_id, error = %e, "terminal pty read ended");
                            }
                            break;
                        }
                    }
                }

                if shutting_down.load(Ordering::Relaxed) {
                    return;
                }

                let status = child
                    .lock()
                    .ok()
                    .and_then(|mut c| c.wait().ok())
                    .map(|s| format!("exited with status {s:?}"));

                if let Ok(mut state) = lifecycle.lock() {
                    *state = (TerminalLifecycle::Exited, status.clone());
                }

                tracing::info!(pane_id, ?status, "terminal pane process ended");
                let _ = server_tx.blocking_send(CliToServer::TerminalState {
                    session_id,
                    pane_id,
                    instance_id: Some(instance_id),
                    lifecycle: TerminalLifecycle::Exited,
                    status: status.clone(),
                });
                let _ = server_tx.blocking_send(CliToServer::TerminalExited {
                    session_id,
                    pane_id,
                    instance_id: Some(instance_id),
                    status,
                });
            })
            .expect("failed to spawn terminal reader thread");
    }

    /// Write raw keystrokes to the pty master.
    pub fn write_bytes(&self, data: &[u8]) -> Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| anyhow::anyhow!("terminal writer mutex poisoned"))?;
        writer.write_all(data).context("pty write failed")?;
        writer.flush().context("pty flush failed")?;
        Ok(())
    }

    /// Resize the pty so the hosted TUI re-lays-out to match the browser
    /// viewport. No-ops when the size is unchanged — xterm.js fires fit()
    /// on every container mutation and a redundant `TIOCSWINSZ` makes
    /// full-screen TUIs repaint for nothing.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let cols = cols.max(1);
        let rows = rows.max(1);
        {
            let mut last = self
                .last_size
                .lock()
                .map_err(|_| anyhow::anyhow!("terminal size mutex poisoned"))?;
            if *last == (cols, rows) {
                return Ok(());
            }
            *last = (cols, rows);
        }
        let master = self
            .master
            .lock()
            .map_err(|_| anyhow::anyhow!("terminal master mutex poisoned"))?;
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("pty resize failed")?;
        Ok(())
    }

    pub fn instance_id(&self) -> Uuid {
        self.instance_id
    }

    /// Reconcile the child before reporting cached lifecycle. Usually the
    /// reader thread records an exit first; `try_wait` closes the smaller
    /// race where reconnect lands after process death but before pty EOF.
    pub fn state_message(&self, session_id: Uuid) -> CliToServer {
        let already_exited = self
            .lifecycle
            .lock()
            .map(|state| state.0 == TerminalLifecycle::Exited)
            .unwrap_or(false);
        if !already_exited && !self.shutting_down.load(Ordering::Relaxed) {
            let observed = self
                .child
                .lock()
                .ok()
                .and_then(|mut child| child.try_wait().ok().flatten())
                .map(|status| format!("exited with status {status:?}"));
            if let Some(status) = observed {
                if let Ok(mut state) = self.lifecycle.lock() {
                    *state = (TerminalLifecycle::Exited, Some(status));
                }
            }
        }
        self.state_message_from_cache(session_id)
    }

    fn state_message_from_cache(&self, session_id: Uuid) -> CliToServer {
        let (lifecycle, status) = self.lifecycle.lock().map(|state| state.clone()).unwrap_or((
            TerminalLifecycle::Unknown,
            Some("terminal lifecycle lock poisoned".to_string()),
        ));
        CliToServer::TerminalState {
            session_id,
            pane_id: self.pane_id,
            instance_id: Some(self.instance_id),
            lifecycle,
            status,
        }
    }

    /// Kill the hosted process and stop the reader thread.
    ///
    /// Marks `shutting_down` first so the reader treats the resulting EOF
    /// as intentional and skips the `TerminalExited` report — the pane is
    /// being torn down, so the web has nothing to act on.
    pub fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::Relaxed);
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(deprecated)]
    fn only_claude_and_codex_can_host_a_terminal() {
        assert_eq!(terminal_binary_for(&Provider::Claude), Some("claude"));
        assert_eq!(terminal_binary_for(&Provider::Codex), Some("codex"));
        for p in [
            Provider::Minimax,
            Provider::Glm,
            Provider::Deepseek,
            Provider::Opencode,
            Provider::CursorAgent,
        ] {
            assert_eq!(
                terminal_binary_for(&p),
                None,
                "{p:?} must not be offered as a terminal host"
            );
        }
    }

    /// End-to-end over a real pty: spawn, capture output, observe exit.
    #[test]
    fn every_terminal_host_launches_without_permission_prompts() {
        // A terminal pane is driven from a browser; an approval prompt there
        // just blocks until someone notices the tab. Any provider we are
        // willing to host must therefore have a bypass flag.
        for p in [Provider::Claude, Provider::Codex] {
            assert!(
                terminal_binary_for(&p).is_some(),
                "{p:?} should be hostable"
            );
            assert!(
                permission_bypass_flag_for(&p).is_some(),
                "{p:?} is hostable but would stop to ask for permission"
            );
        }
    }

    #[test]
    #[allow(deprecated)]
    fn non_hostable_providers_get_no_flags_at_all() {
        // An unrecognised flag fails the spawn outright rather than degrading,
        // so providers we have not verified get nothing.
        for p in [
            Provider::Minimax,
            Provider::Glm,
            Provider::Deepseek,
            Provider::Opencode,
            Provider::CursorAgent,
        ] {
            assert_eq!(permission_bypass_flag_for(&p), None, "{p:?}");
            assert!(resume_args_for(&p, Uuid::nil()).is_empty(), "{p:?}");
        }
    }

    #[test]
    fn restore_uses_provider_resume_arguments_without_losing_claude_identity() {
        // `resume` is a subcommand for codex, not a flag, so the bypass flag
        // has to come after it -- `codex resume --dangerously-...`, never
        // `codex --dangerously-... resume`. Claude's are both top-level flags
        // and order-insensitive. Pinning the relative order here because the
        // spawn builds them in sequence and a swap only fails at runtime.
        let conversation_id = Uuid::parse_str("11111111-2222-4333-8444-555555555555").unwrap();
        assert_eq!(
            resume_args_for(&Provider::Codex, conversation_id),
            vec!["resume".to_string()]
        );
        assert_eq!(
            permission_bypass_flag_for(&Provider::Codex),
            Some("--dangerously-bypass-approvals-and-sandbox")
        );
        assert_eq!(
            resume_args_for(&Provider::Claude, conversation_id),
            vec!["--resume".to_string(), conversation_id.to_string()]
        );
        assert_eq!(
            permission_bypass_flag_for(&Provider::Claude),
            Some("--dangerously-skip-permissions")
        );
    }

    #[test]
    fn pty_streams_output_and_reports_exit() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (tx, mut rx) = tokio_mpsc::channel(64);
            let session_id = Uuid::new_v4();
            let conv_id = Uuid::new_v4();
            let handle = TerminalHandle::spawn(
                7,
                session_id,
                conv_id,
                &Provider::Claude,
                "/bin/echo",
                "/tmp",
                &[],
                false,
                Some("diagnose the failing test"),
                tx,
            )
            .expect("spawn pty");
            let instance_id = handle.instance_id();

            let mut collected = Vec::new();
            let mut exited = false;
            let mut running_state_seen = false;
            let mut exited_state_seen = false;
            while let Some(msg) = rx.recv().await {
                match msg {
                    CliToServer::TerminalOutput {
                        pane_id,
                        instance_id: got_instance,
                        data_b64,
                        session_id: got,
                        ..
                    } => {
                        assert_eq!(pane_id, 7);
                        assert_eq!(got, session_id);
                        assert_eq!(got_instance, Some(instance_id));
                        collected.extend(
                            base64::engine::general_purpose::STANDARD
                                .decode(data_b64)
                                .unwrap(),
                        );
                    }
                    CliToServer::TerminalState {
                        pane_id,
                        instance_id: got_instance,
                        lifecycle,
                        ..
                    } => {
                        assert_eq!(pane_id, 7);
                        assert_eq!(got_instance, Some(instance_id));
                        match lifecycle {
                            TerminalLifecycle::Running => running_state_seen = true,
                            TerminalLifecycle::Exited => exited_state_seen = true,
                            other => panic!("unexpected lifecycle: {other:?}"),
                        }
                    }
                    CliToServer::TerminalExited {
                        pane_id,
                        instance_id: got_instance,
                        ..
                    } => {
                        assert_eq!(pane_id, 7);
                        assert_eq!(got_instance, Some(instance_id));
                        exited = true;
                        break;
                    }
                    other => panic!("unexpected message: {other:?}"),
                }
            }

            assert!(exited, "reader never reported the child exit");
            assert!(running_state_seen, "spawn never reported running state");
            assert!(exited_state_seen, "reader never retained exited state");
            match handle.state_message(session_id) {
                CliToServer::TerminalState {
                    instance_id: got_instance,
                    lifecycle,
                    ..
                } => {
                    assert_eq!(got_instance, Some(instance_id));
                    assert_eq!(lifecycle, TerminalLifecycle::Exited);
                }
                other => panic!("unexpected state report: {other:?}"),
            }
            // `/bin/echo` prints its argv, so this doubles as end-to-end proof
            // that the flags actually reach the spawned process — not just that
            // the mapping functions return them. The pty turns the trailing
            // newline into CRLF, which is what xterm.js expects, so compare
            // against the trimmed line.
            //
            // `--session-id` is the load-bearing one: it pins claude's
            // conversation id to the pane's, which is the only reason the
            // pane's transcript can be located exactly rather than guessed.
            assert_eq!(
                String::from_utf8_lossy(&collected).trim_end(),
                format!(
                    "--dangerously-skip-permissions --session-id {conv_id} diagnose the failing test"
                ),
            );
            handle.shutdown();
        });
    }

    #[test]
    fn terminal_instance_identity_is_stable_and_unique_per_spawn() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (tx, _rx) = tokio_mpsc::channel(64);
            let session_id = Uuid::new_v4();
            let first = TerminalHandle::spawn(
                1,
                session_id,
                Uuid::new_v4(),
                &Provider::Claude,
                "/bin/cat",
                "/tmp",
                &[],
                false,
                None,
                tx.clone(),
            )
            .expect("spawn first pty");
            let second = TerminalHandle::spawn(
                1,
                session_id,
                Uuid::new_v4(),
                &Provider::Claude,
                "/bin/cat",
                "/tmp",
                &[],
                false,
                None,
                tx,
            )
            .expect("spawn replacement pty");

            let first_id = first.instance_id();
            assert_eq!(first.instance_id(), first_id);
            assert_ne!(first_id, second.instance_id());
            first.shutdown();
            second.shutdown();
        });
    }

    #[test]
    fn resize_is_idempotent_and_clamps_zero() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (tx, _rx) = tokio_mpsc::channel(64);
            let handle = TerminalHandle::spawn(
                1,
                Uuid::new_v4(),
                Uuid::new_v4(),
                &Provider::Claude,
                "/bin/cat",
                "/tmp",
                &[],
                false,
                None,
                tx,
            )
            .expect("spawn pty");

            handle.resize(120, 40).expect("first resize");
            // Same size again: must be accepted without re-issuing TIOCSWINSZ.
            handle.resize(120, 40).expect("redundant resize");
            // A 0-dimension viewport (hidden tab, pre-layout) must not
            // reach the pty — some TUIs divide by the row count.
            handle.resize(0, 0).expect("zero resize clamps");

            handle.shutdown();
        });
    }
}
