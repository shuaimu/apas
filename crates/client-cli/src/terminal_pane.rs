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
//! The only flags passed are the provider's resume arguments (on restore),
//! its permission-bypass flag, and its provider-specific initial-prompt form:
//! a pane driven from a browser has nobody at the keyboard to answer an
//! approval prompt, and agent panes already launch with the same bypass. See
//! [`permission_bypass_flag_for`].
//!
//! The cost is that a terminal pane has none of the structured
//! integrations: no usage counters, no pane status, no diffs, no plan
//! review, no scratchpad publishing, and it is never a Tech Lead
//! delegation target. See the `PaneKind` docs in `shared`.
//!
//! Lifetime: the pty is a child of the CLI process, so a terminal pane
//! dies when `apas` restarts. [`TerminalHandle::spawn`] therefore re-execs the
//! provider and asks it to resume the pane's conversation. Claude can resume
//! the exact pinned session id; Codex and OpenCode expose their own resume
//! mechanisms instead.

use anyhow::{Context, Result};
use base64::Engine as _;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use shared::{CliToServer, Provider, TerminalLifecycle};
use std::collections::HashMap;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
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
pub type TerminalPanes = Arc<Mutex<HashMap<u32, TerminalRuntimeHandle>>>;

/// One server-facing terminal runtime. Persistent hosting is attempted for
/// supported Unix project CLIs; the established in-process PTY remains the
/// safe fallback when prerequisites are unavailable.
#[derive(Clone)]
pub enum TerminalRuntimeHandle {
    Direct(TerminalHandle),
    #[cfg(unix)]
    Hosted(HostedTerminalHandle),
}

impl TerminalRuntimeHandle {
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        pane_id: u32,
        session_id: Uuid,
        conversation_id: Uuid,
        provider: &Provider,
        binary_path: &str,
        cwd: &str,
        env: &[(String, String)],
        resume: bool,
        initial_prompt: Option<&str>,
        server_tx: tokio_mpsc::Sender<CliToServer>,
    ) -> Result<Self> {
        #[cfg(unix)]
        if crate::pane_host::persistent_hosting_available() {
            match HostedTerminalHandle::adopt_existing(pane_id, session_id, server_tx.clone()) {
                Ok(Some(handle)) => return Ok(Self::Hosted(handle)),
                Ok(None) => match HostedTerminalHandle::create(
                    pane_id,
                    session_id,
                    conversation_id,
                    provider,
                    binary_path,
                    cwd,
                    env,
                    resume,
                    initial_prompt,
                    server_tx.clone(),
                ) {
                    Ok(handle) => return Ok(Self::Hosted(handle)),
                    Err(error) => {
                        tracing::warn!(pane_id, %error, "persistent pane host launch failed; using direct PTY fallback")
                    }
                },
                Err(error) => {
                    // A descriptor for this pane exists but could not be
                    // exclusively adopted. Starting another provider would
                    // duplicate a possibly live autonomous agent, so fail
                    // closed and let the caller surface unavailability.
                    return Err(error.context("existing pane host could not be adopted"));
                }
            }
        }

        Ok(Self::Direct(TerminalHandle::spawn(
            pane_id,
            session_id,
            conversation_id,
            provider,
            binary_path,
            cwd,
            env,
            resume,
            initial_prompt,
            server_tx,
        )?))
    }

    pub fn write_bytes(&self, data: &[u8]) -> Result<()> {
        match self {
            Self::Direct(handle) => handle.write_bytes(data),
            #[cfg(unix)]
            Self::Hosted(handle) => handle.write_bytes(data),
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        match self {
            Self::Direct(handle) => handle.resize(cols, rows),
            #[cfg(unix)]
            Self::Hosted(handle) => handle.resize(cols, rows),
        }
    }

    pub fn instance_id(&self) -> Uuid {
        match self {
            Self::Direct(handle) => handle.instance_id(),
            #[cfg(unix)]
            Self::Hosted(handle) => handle.instance_id(),
        }
    }

    pub fn state_message(&self, session_id: Uuid) -> CliToServer {
        match self {
            Self::Direct(handle) => handle.state_message(session_id),
            #[cfg(unix)]
            Self::Hosted(handle) => handle.state_message(session_id),
        }
    }

    pub fn shutdown(&self) {
        match self {
            Self::Direct(handle) => handle.shutdown(),
            #[cfg(unix)]
            Self::Hosted(handle) => handle.shutdown(),
        }
    }

    pub fn preservation(&self, pane_id: u32) -> shared::PanePreservationInfo {
        match self {
            Self::Direct(_) => shared::PanePreservationInfo {
                pane_id,
                mode: shared::PanePreservationMode::RestartRequiredOnCliReboot,
                runtime_id: None,
            },
            #[cfg(unix)]
            Self::Hosted(handle) => shared::PanePreservationInfo {
                pane_id,
                mode: shared::PanePreservationMode::LiveAdoptable,
                runtime_id: Some(handle.runtime_id()),
            },
        }
    }

    #[cfg(unix)]
    pub fn detach_for_reboot(&self) -> Result<()> {
        match self {
            Self::Direct(_) => Ok(()),
            Self::Hosted(handle) => handle.detach_for_reboot(),
        }
    }
}

#[cfg(unix)]
#[derive(Clone)]
pub struct HostedTerminalHandle {
    pane_id: u32,
    instance_id: Uuid,
    descriptor: crate::pane_host::RuntimeDescriptor,
    stream: Arc<Mutex<UnixStream>>,
    lifecycle: Arc<Mutex<(TerminalLifecycle, Option<String>)>>,
    runtime: Arc<Mutex<shared::TerminalRuntimeReconciliation>>,
    shutting_down: Arc<AtomicBool>,
}

#[cfg(unix)]
impl HostedTerminalHandle {
    #[allow(clippy::too_many_arguments)]
    fn create(
        pane_id: u32,
        session_id: Uuid,
        conversation_id: Uuid,
        provider: &Provider,
        binary_path: &str,
        cwd: &str,
        env: &[(String, String)],
        resume: bool,
        initial_prompt: Option<&str>,
        server_tx: tokio_mpsc::Sender<CliToServer>,
    ) -> Result<Self> {
        let (descriptor, credential, _paths) = crate::pane_host::launch_host(session_id, pane_id)?;
        let mut stream = crate::pane_host::connect_with_retry(
            &descriptor.socket_path,
            std::time::Duration::from_secs(5),
        )?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
        crate::pane_host::write_frame(
            &mut stream,
            &crate::pane_host::ControllerToHost::Create {
                protocol_version: crate::pane_host::HOST_PROTOCOL_VERSION,
                credential,
                project_id: session_id,
                pane_id,
                runtime_id: descriptor.runtime_id,
                controller_id: Uuid::new_v4(),
                controller_generation: descriptor.controller_generation,
                provider: *provider,
                binary_path: binary_path.to_string(),
                cwd: cwd.to_string(),
                env: env.to_vec(),
                conversation_id,
                resume,
                initial_prompt: initial_prompt.map(ToString::to_string),
            },
        )?;
        Self::finish_attach(stream, descriptor, session_id, server_tx, false)
    }

    fn adopt_existing(
        pane_id: u32,
        session_id: Uuid,
        server_tx: tokio_mpsc::Sender<CliToServer>,
    ) -> Result<Option<Self>> {
        let Some((_path, mut descriptor)) =
            crate::pane_host::descriptor_for_pane(session_id, pane_id)?
        else {
            return Ok(None);
        };
        if descriptor.project_id != session_id || descriptor.pane_id != pane_id {
            anyhow::bail!("pane-host descriptor identity mismatch");
        }
        // A descriptor may outlive a crashed tmux host by a few milliseconds.
        // It is safe to discard only when supervision proves the old runtime
        // no longer exists; a live-but-unreachable host must still fail closed
        // so we never start a duplicate autonomous provider.
        if !crate::pane_host::runtime_is_live(&descriptor) {
            if let Some(runtime_dir) = descriptor.socket_path.parent() {
                let _ = std::fs::remove_dir_all(runtime_dir);
            }
            tracing::warn!(
                pane_id,
                runtime_id = %descriptor.runtime_id,
                "discarded stale pane-host descriptor; provider will resume in a new runtime"
            );
            return Ok(None);
        }
        let credential = crate::pane_host::read_credential(&descriptor.credential_path)?;
        let mut stream = crate::pane_host::connect_with_retry(
            &descriptor.socket_path,
            std::time::Duration::from_secs(5),
        )?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
        descriptor.controller_generation = descriptor.controller_generation.saturating_add(1);
        crate::pane_host::write_frame(
            &mut stream,
            &crate::pane_host::ControllerToHost::Adopt {
                protocol_version: crate::pane_host::HOST_PROTOCOL_VERSION,
                credential,
                project_id: session_id,
                pane_id,
                runtime_id: descriptor.runtime_id,
                controller_id: Uuid::new_v4(),
                controller_generation: descriptor.controller_generation,
                // Replaying the full volatile ring is safe because the
                // server deduplicates stable instance/sequence pairs.
                acknowledged_seq: None,
            },
        )?;
        match Self::finish_attach(stream, descriptor.clone(), session_id, server_tx, true) {
            Ok(handle) => {
                if handle
                    .lifecycle
                    .lock()
                    .is_ok_and(|state| state.0 == TerminalLifecycle::Exited)
                {
                    handle.shutdown();
                    Ok(None)
                } else {
                    tracing::info!(pane_id, runtime_id = %descriptor.runtime_id, instance_id = %descriptor.instance_id, "adopted persistent terminal pane");
                    Ok(Some(handle))
                }
            }
            Err(error) => Err(error),
        }
    }

    fn finish_attach(
        mut stream: UnixStream,
        descriptor: crate::pane_host::RuntimeDescriptor,
        session_id: Uuid,
        server_tx: tokio_mpsc::Sender<CliToServer>,
        live_adopted: bool,
    ) -> Result<Self> {
        let adopted =
            crate::pane_host::read_frame::<crate::pane_host::HostToController>(&mut stream)?;
        let crate::pane_host::HostToController::Adopted {
            protocol_version,
            runtime_id,
            instance_id,
            lifecycle,
            status,
            oldest_seq,
            current_seq,
            truncated,
            ..
        } = adopted
        else {
            if let crate::pane_host::HostToController::Error { message } = adopted {
                anyhow::bail!("pane host rejected controller: {message}");
            }
            anyhow::bail!("pane host did not acknowledge adoption");
        };
        if protocol_version != crate::pane_host::HOST_PROTOCOL_VERSION
            || runtime_id != descriptor.runtime_id
            || instance_id != descriptor.instance_id
        {
            anyhow::bail!("pane-host adoption response identity mismatch");
        }
        stream.set_read_timeout(None)?;
        let reader = stream.try_clone()?;
        let lifecycle_state = Arc::new(Mutex::new((lifecycle, status.clone())));
        let runtime_state = Arc::new(Mutex::new(shared::TerminalRuntimeReconciliation {
            runtime_id: Some(runtime_id),
            oldest_seq,
            current_seq,
            truncated,
            live_adopted,
        }));
        let handle = Self {
            pane_id: descriptor.pane_id,
            instance_id,
            descriptor,
            stream: Arc::new(Mutex::new(stream)),
            lifecycle: lifecycle_state,
            runtime: runtime_state,
            shutting_down: Arc::new(AtomicBool::new(false)),
        };
        handle.start_reader(session_id, reader, server_tx);
        tracing::info!(
            pane_id = handle.pane_id,
            runtime_id = %runtime_id,
            instance_id = %instance_id,
            host_protocol = protocol_version,
            oldest_seq,
            current_seq,
            truncated,
            live_adopted,
            "attached persistent terminal runtime"
        );
        Ok(handle)
    }

    fn start_reader(
        &self,
        session_id: Uuid,
        mut reader: UnixStream,
        server_tx: tokio_mpsc::Sender<CliToServer>,
    ) {
        let pane_id = self.pane_id;
        let instance_id = self.instance_id;
        let lifecycle = self.lifecycle.clone();
        let runtime = self.runtime.clone();
        let shutting_down = self.shutting_down.clone();
        let initial_runtime = runtime
            .lock()
            .map(|value| value.clone())
            .unwrap_or_default();
        let initial_state = lifecycle
            .lock()
            .map(|value| value.clone())
            .unwrap_or((TerminalLifecycle::Unknown, None));
        let _ = server_tx.try_send(CliToServer::TerminalState {
            session_id,
            pane_id,
            instance_id: Some(instance_id),
            lifecycle: initial_state.0,
            status: initial_state.1,
            runtime: Some(initial_runtime),
        });
        thread::Builder::new()
            .name(format!("apas-hosted-term-{pane_id}"))
            .spawn(move || loop {
                match crate::pane_host::read_frame::<crate::pane_host::HostToController>(
                    &mut reader,
                ) {
                    Ok(crate::pane_host::HostToController::Output { seq, data }) => {
                        if let Ok(mut metadata) = runtime.lock() {
                            metadata.current_seq = metadata.current_seq.max(seq);
                        }
                        let _ = server_tx.blocking_send(CliToServer::TerminalOutput {
                            session_id,
                            pane_id,
                            instance_id: Some(instance_id),
                            data_b64: base64::engine::general_purpose::STANDARD.encode(data),
                            seq,
                        });
                    }
                    Ok(crate::pane_host::HostToController::State {
                        lifecycle: next,
                        status,
                    }) => {
                        if let Ok(mut state) = lifecycle.lock() {
                            *state = (next, status.clone());
                        }
                        let metadata = runtime.lock().map(|value| value.clone()).ok();
                        let _ = server_tx.blocking_send(CliToServer::TerminalState {
                            session_id,
                            pane_id,
                            instance_id: Some(instance_id),
                            lifecycle: next,
                            status: status.clone(),
                            runtime: metadata,
                        });
                        if next == TerminalLifecycle::Exited {
                            let _ = server_tx.blocking_send(CliToServer::TerminalExited {
                                session_id,
                                pane_id,
                                instance_id: Some(instance_id),
                                status,
                            });
                        }
                    }
                    Ok(crate::pane_host::HostToController::Error { message }) => {
                        tracing::warn!(pane_id, %message, "pane-host controller error");
                        break;
                    }
                    Ok(crate::pane_host::HostToController::Adopted { .. }) => continue,
                    Err(error) => {
                        if !shutting_down.load(Ordering::Relaxed) {
                            tracing::info!(pane_id, %error, "pane-host controller detached");
                        }
                        break;
                    }
                }
            })
            .expect("failed to spawn pane-host reader");
    }

    fn write_command(&self, command: crate::pane_host::ControllerToHost) -> Result<()> {
        let mut stream = self
            .stream
            .lock()
            .map_err(|_| anyhow::anyhow!("pane-host stream mutex poisoned"))?;
        crate::pane_host::write_frame(&mut stream, &command)
    }

    pub fn write_bytes(&self, data: &[u8]) -> Result<()> {
        self.write_command(crate::pane_host::ControllerToHost::Input {
            data: data.to_vec(),
        })
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        if cols == 0 || rows == 0 {
            anyhow::bail!("terminal size must be non-zero");
        }
        self.write_command(crate::pane_host::ControllerToHost::Resize { cols, rows })
    }

    pub fn instance_id(&self) -> Uuid {
        self.instance_id
    }

    pub fn runtime_id(&self) -> Uuid {
        self.descriptor.runtime_id
    }

    pub fn state_message(&self, session_id: Uuid) -> CliToServer {
        let (lifecycle, status) = self.lifecycle.lock().map(|value| value.clone()).unwrap_or((
            TerminalLifecycle::Unknown,
            Some("pane-host lifecycle unavailable".to_string()),
        ));
        CliToServer::TerminalState {
            session_id,
            pane_id: self.pane_id,
            instance_id: Some(self.instance_id),
            lifecycle,
            status,
            runtime: self.runtime.lock().map(|value| value.clone()).ok(),
        }
    }

    pub fn detach_for_reboot(&self) -> Result<()> {
        self.write_command(crate::pane_host::ControllerToHost::Detach { reboot: true })
    }

    pub fn shutdown(&self) {
        if self.shutting_down.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self.write_command(crate::pane_host::ControllerToHost::Shutdown);
        if let Err(error) = crate::pane_host::terminate_tmux_host(&self.descriptor) {
            tracing::warn!(pane_id = self.pane_id, %error, "failed to clean pane-host tmux session");
        }
    }
}

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
/// Deliberately not every [`Provider`]: Claude, Codex, and OpenCode have
/// documented interactive TUIs, non-interactive permission modes, and resume
/// flows that APAS can safely drive through a bare pty. Returning `None`
/// here makes the caller reject the pane rather than spawn something that
/// paints garbage into xterm.js.
pub fn terminal_binary_for(provider: &Provider) -> Option<&'static str> {
    #[allow(deprecated)]
    match provider {
        Provider::Claude => Some("claude"),
        Provider::Codex => Some("codex"),
        Provider::Opencode => Some("opencode"),
        Provider::Minimax | Provider::Glm | Provider::Deepseek | Provider::CursorAgent => None,
    }
}

/// The provider's own "pick up where we left off" arguments.
///
/// Claude must receive the pane's exact pinned id. `--continue` is not an
/// equivalent: it selects the most recent conversation for the cwd, which can
/// belong to another pane while APAS continues watching this pane's pinned
/// transcript. Codex's `resume` remains a subcommand and therefore must be
/// appended before its permission flag. OpenCode's `--continue` selects the
/// newest session for the pane's working directory.
fn resume_args_for(provider: &Provider, conversation_id: Uuid) -> Vec<String> {
    match provider {
        Provider::Claude => vec!["--resume".to_string(), conversation_id.to_string()],
        Provider::Codex => vec!["resume".to_string()],
        Provider::Opencode => vec!["--continue".to_string()],
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
/// Verified present on the *interactive* forms of all hostable binaries, not
/// just their headless modes. That matters because an unrecognised flag does
/// not degrade; it fails the spawn outright.
fn permission_bypass_flag_for(provider: &Provider) -> Option<&'static str> {
    match provider {
        Provider::Claude => Some("--dangerously-skip-permissions"),
        Provider::Codex => Some("--dangerously-bypass-approvals-and-sandbox"),
        // Official OpenCode auto mode approves requests that are not
        // explicitly denied by the user's own permission configuration.
        Provider::Opencode => Some("--auto"),
        _ => None,
    }
}

/// Provider-specific interactive CLI arguments, separated from pty setup so
/// tests can pin the exact command line without spawning a real agent.
pub(crate) fn terminal_args_for(
    provider: &Provider,
    conversation_id: Uuid,
    resume: bool,
    initial_prompt: Option<&str>,
) -> Vec<String> {
    let mut args = Vec::new();
    if resume {
        args.extend(resume_args_for(provider, conversation_id));
    }
    if let Some(flag) = permission_bypass_flag_for(provider) {
        args.push(flag.to_string());
    }
    if !resume {
        // Pin Claude before the positional prompt. Besides preserving the
        // long-standing command shape, this keeps the prompt last so it
        // cannot accidentally absorb a following option-like token.
        if matches!(provider, Provider::Claude) {
            args.push("--session-id".to_string());
            args.push(conversation_id.to_string());
        }
        if let Some(prompt) = initial_prompt
            .map(str::trim)
            .filter(|prompt| !prompt.is_empty())
        {
            // OpenCode's positional argument is the project path. Its initial
            // instruction must use --prompt; Claude and Codex accept the
            // instruction positionally.
            if matches!(provider, Provider::Opencode) {
                args.push("--prompt".to_string());
            }
            args.push(prompt.to_string());
        }
    }
    args
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
        // Order matters: codex's resume marker is a subcommand (`codex
        // resume`) and its bypass flag follows it. The helper also prevents
        // treating OpenCode's initial instruction as its positional project
        // path.
        for arg in terminal_args_for(provider, claude_session_id, resume, initial_prompt) {
            cmd.arg(arg);
        }
        cmd.cwd(cwd);
        // A TUI keys its capabilities off TERM. Without this it inherits
        // whatever the daemon-spawned CLI had — often `dumb`, which makes
        // the provider can fall back to a line-mode renderer that looks broken
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
                    runtime: None,
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
                    runtime: None,
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

    #[cfg(test)]
    pub fn provider_pid(&self) -> Option<u32> {
        self.child.lock().ok().and_then(|child| child.process_id())
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
            runtime: None,
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
    fn only_verified_interactive_clis_can_host_a_terminal() {
        assert_eq!(terminal_binary_for(&Provider::Claude), Some("claude"));
        assert_eq!(terminal_binary_for(&Provider::Codex), Some("codex"));
        assert_eq!(terminal_binary_for(&Provider::Opencode), Some("opencode"));
        for p in [
            Provider::Minimax,
            Provider::Glm,
            Provider::Deepseek,
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
        for p in [Provider::Claude, Provider::Codex, Provider::Opencode] {
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
        assert_eq!(
            terminal_args_for(
                &Provider::Opencode,
                conversation_id,
                false,
                Some("fix the test")
            ),
            vec!["--auto", "--prompt", "fix the test"]
        );
        assert_eq!(
            terminal_args_for(
                &Provider::Opencode,
                conversation_id,
                true,
                Some("must not replay")
            ),
            vec!["--continue", "--auto"]
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
