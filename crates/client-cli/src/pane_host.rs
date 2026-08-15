//! Persistent one-pane terminal host and controller transport.
//!
//! The host owns the provider PTY outside the replaceable project CLI.  Only
//! non-secret identity is persisted; a random credential lives in a separate
//! mode-0600 host-local file and raw terminal bytes remain in memory.

use anyhow::{bail, Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use shared::{Provider, TerminalLifecycle};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub const HOST_PROTOCOL_VERSION: u32 = 1;
const MAX_FRAME_BYTES: usize = 1024 * 1024;
const OUTPUT_RING_MAX_BYTES: usize = 512 * 1024;
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const READ_CHUNK_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeDescriptor {
    pub protocol_version: u32,
    pub project_id: Uuid,
    pub pane_id: u32,
    pub runtime_id: Uuid,
    pub instance_id: Uuid,
    pub owner_uid: u32,
    pub controller_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller_id: Option<Uuid>,
    pub session_name: String,
    pub socket_path: PathBuf,
    pub credential_path: PathBuf,
    pub created_at_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputChunk {
    pub seq: u64,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControllerToHost {
    Create {
        protocol_version: u32,
        credential: String,
        project_id: Uuid,
        pane_id: u32,
        runtime_id: Uuid,
        controller_id: Uuid,
        controller_generation: u64,
        provider: Provider,
        binary_path: String,
        cwd: String,
        env: Vec<(String, String)>,
        conversation_id: Uuid,
        resume: bool,
        initial_prompt: Option<String>,
    },
    Adopt {
        protocol_version: u32,
        credential: String,
        project_id: Uuid,
        pane_id: u32,
        runtime_id: Uuid,
        controller_id: Uuid,
        controller_generation: u64,
        acknowledged_seq: Option<u64>,
    },
    Input {
        data: Vec<u8>,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    Ack {
        seq: u64,
    },
    Detach {
        reboot: bool,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostToController {
    Adopted {
        protocol_version: u32,
        runtime_id: Uuid,
        instance_id: Uuid,
        provider_pid: Option<u32>,
        process_group_id: Option<i32>,
        lifecycle: TerminalLifecycle,
        status: Option<String>,
        oldest_seq: u64,
        current_seq: u64,
        truncated: bool,
        controller_generation: u64,
    },
    Output {
        seq: u64,
        data: Vec<u8>,
    },
    State {
        lifecycle: TerminalLifecycle,
        status: Option<String>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebootHandoffMarker {
    pub protocol_version: u32,
    pub request_id: Uuid,
    pub project_id: Uuid,
    pub expected_executable: PathBuf,
    pub expected_version: String,
    pub controller_generation: u64,
    pub runtime_ids: Vec<Uuid>,
    pub deadline_unix_ms: u64,
}

#[derive(Debug, Clone)]
pub struct RuntimePaths {
    pub dir: PathBuf,
    pub descriptor: PathBuf,
    pub credential: PathBuf,
    pub socket: PathBuf,
}

fn short_uuid(id: Uuid) -> String {
    id.simple().to_string()[..12].to_string()
}

pub fn runtime_root() -> Result<PathBuf> {
    let root = crate::config::Config::runtime_dir()?.join("ph");
    ensure_private_dir(&root)?;
    Ok(root)
}

pub fn project_runtime_root(project_id: Uuid) -> Result<PathBuf> {
    let root = runtime_root()?.join(short_uuid(project_id));
    ensure_private_dir(&root)?;
    Ok(root)
}

fn project_tombstone(project_id: Uuid) -> Result<PathBuf> {
    Ok(project_runtime_root(project_id)?.join("tombstone"))
}

pub fn handoff_marker_path(project_id: Uuid) -> Result<PathBuf> {
    Ok(project_runtime_root(project_id)?.join("handoff.json"))
}

pub fn tombstone_project_hosts(project_id: Uuid) -> Result<()> {
    write_private(&project_tombstone(project_id)?, b"cleanup-in-progress")
}

pub fn clear_project_host_tombstone(project_id: Uuid) {
    if let Ok(path) = project_tombstone(project_id) {
        let _ = fs::remove_file(path);
    }
    prune_project_runtime_root(project_id);
}

/// Remove a project's runtime directory once nothing is left in it.
///
/// Pane cleanup only ever removed the per-pane directory, so every project that
/// had hosted a pane left an empty skeleton under the runtime root forever. They
/// cost almost nothing individually — this is a tmpfs and the directories are
/// empty — but they accumulate without bound and they actively obscure real
/// state: an audit that lists project directories reads dozens of dead projects
/// alongside the live ones.
///
/// `remove_dir` rather than a check-then-delete: it refuses on a non-empty
/// directory, so a surviving pane, an in-progress `tombstone`, or a pending
/// `handoff.json` all keep the directory automatically, with no window between
/// deciding it is empty and removing it.
fn prune_project_runtime_root(project_id: Uuid) {
    if let Ok(root) = project_runtime_root(project_id) {
        let _ = fs::remove_dir(root);
    }
}

pub fn runtime_paths(project_id: Uuid, pane_id: u32, runtime_id: Uuid) -> Result<RuntimePaths> {
    let dir =
        project_runtime_root(project_id)?.join(format!("{pane_id}-{}", short_uuid(runtime_id)));
    ensure_private_dir(&dir)?;
    Ok(RuntimePaths {
        descriptor: dir.join("runtime.json"),
        credential: dir.join("credential"),
        socket: dir.join("c.sock"),
        dir,
    })
}

fn ensure_private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    let metadata = fs::metadata(path)?;
    if metadata.mode() & 0o077 != 0 || metadata.uid() != unsafe { libc::getuid() } {
        bail!("insecure pane-host runtime directory {}", path.display());
    }
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true).mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn atomic_write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", Uuid::new_v4().simple()));
    write_private(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn write_descriptor(path: &Path, descriptor: &RuntimeDescriptor) -> Result<()> {
    atomic_write_private(path, &serde_json::to_vec_pretty(descriptor)?)
}

pub fn read_descriptor(path: &Path) -> Result<RuntimeDescriptor> {
    verify_private_file(path)?;
    let descriptor: RuntimeDescriptor = serde_json::from_slice(&fs::read(path)?)?;
    if descriptor.protocol_version != HOST_PROTOCOL_VERSION {
        bail!(
            "incompatible pane-host protocol {}",
            descriptor.protocol_version
        );
    }
    Ok(descriptor)
}

fn verify_private_file(path: &Path) -> Result<()> {
    let metadata = fs::metadata(path)?;
    if metadata.mode() & 0o077 != 0 || metadata.uid() != unsafe { libc::getuid() } {
        bail!("insecure pane-host file {}", path.display());
    }
    Ok(())
}

fn random_credential() -> Result<String> {
    let mut bytes = [0u8; 32];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

pub fn write_handoff_marker(path: &Path, marker: &RebootHandoffMarker) -> Result<()> {
    atomic_write_private(path, &serde_json::to_vec_pretty(marker)?)
}

pub fn read_handoff_marker(path: &Path) -> Result<RebootHandoffMarker> {
    verify_private_file(path)?;
    let marker: RebootHandoffMarker = serde_json::from_slice(&fs::read(path)?)?;
    if marker.protocol_version != HOST_PROTOCOL_VERSION || marker.deadline_unix_ms < now_unix_ms() {
        bail!("pane-host reboot handoff marker is incompatible or expired");
    }
    Ok(marker)
}

pub fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<()> {
    let body = serde_json::to_vec(value)?;
    if body.len() > MAX_FRAME_BYTES {
        bail!("pane-host frame exceeds {MAX_FRAME_BYTES} bytes");
    }
    stream.write_all(&(body.len() as u32).to_be_bytes())?;
    stream.write_all(&body)?;
    stream.flush()?;
    Ok(())
}

pub fn read_frame<T: DeserializeOwned>(stream: &mut UnixStream) -> Result<T> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME_BYTES {
        bail!("invalid pane-host frame length {len}");
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    serde_json::from_slice(&body).context("decode pane-host frame")
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub fn tmux_socket_name(project_id: Uuid) -> String {
    format!("apas-{}", sanitize(&project_id.to_string()))
}

fn tmux_has_session(project_id: Uuid, session_name: &str) -> bool {
    Command::new("tmux")
        .args([
            "-L",
            &tmux_socket_name(project_id),
            "has-session",
            "-t",
            session_name,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn runtime_is_live(descriptor: &RuntimeDescriptor) -> bool {
    tmux_has_session(descriptor.project_id, &descriptor.session_name)
}

pub fn persistent_hosting_available() -> bool {
    let executable = crate::update::resolve_preferred_apas_executable();
    cfg!(unix)
        && Command::new("tmux")
            .arg("-V")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        && Command::new(executable)
            .args(["pane-host", "--help"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        && runtime_root().is_ok()
}

pub fn launch_host(
    project_id: Uuid,
    pane_id: u32,
) -> Result<(RuntimeDescriptor, String, RuntimePaths)> {
    if !persistent_hosting_available() {
        bail!("persistent pane hosting prerequisites are unavailable");
    }
    if project_tombstone(project_id)?.exists() {
        bail!("project pane-host cleanup is in progress");
    }
    let runtime_id = Uuid::new_v4();
    let instance_id = Uuid::new_v4();
    let paths = runtime_paths(project_id, pane_id, runtime_id)?;
    let credential = random_credential()?;
    write_private(&paths.credential, credential.as_bytes())?;
    let session_name = format!("ph_{pane_id}_{}", &runtime_id.simple().to_string()[..8]);
    let descriptor = RuntimeDescriptor {
        protocol_version: HOST_PROTOCOL_VERSION,
        project_id,
        pane_id,
        runtime_id,
        instance_id,
        owner_uid: unsafe { libc::getuid() },
        controller_generation: 1,
        controller_id: None,
        session_name: session_name.clone(),
        socket_path: paths.socket.clone(),
        credential_path: paths.credential.clone(),
        created_at_unix_ms: now_unix_ms(),
    };
    write_descriptor(&paths.descriptor, &descriptor)?;

    let executable = crate::update::resolve_preferred_apas_executable();
    let output = Command::new("tmux")
        .arg("-L")
        .arg(tmux_socket_name(project_id))
        .arg("new-session")
        .arg("-d")
        .arg("-s")
        .arg(&session_name)
        .arg(executable)
        .arg("pane-host")
        .arg("--runtime-dir")
        .arg(&paths.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    if !output.status.success() {
        let _ = fs::remove_dir_all(&paths.dir);
        bail!(
            "tmux pane-host launch failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok((descriptor, credential, paths))
}

pub fn list_project_descriptors(project_id: Uuid) -> Result<Vec<(PathBuf, RuntimeDescriptor)>> {
    let root = project_runtime_root(project_id)?;
    let mut descriptors = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_name() == "tombstone" {
            continue;
        }
        let path = entry.path().join("runtime.json");
        if !path.is_file() {
            continue;
        }
        match read_descriptor(&path) {
            Ok(descriptor) if descriptor.project_id == project_id => {
                descriptors.push((path, descriptor));
            }
            Ok(_) | Err(_) => {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
    Ok(descriptors)
}

pub fn descriptor_for_pane(
    project_id: Uuid,
    pane_id: u32,
) -> Result<Option<(PathBuf, RuntimeDescriptor)>> {
    Ok(list_project_descriptors(project_id)?
        .into_iter()
        .find(|(_, descriptor)| descriptor.pane_id == pane_id))
}

pub fn terminate_tmux_host(descriptor: &RuntimeDescriptor) -> Result<()> {
    if tmux_has_session(descriptor.project_id, &descriptor.session_name) {
        let status = Command::new("tmux")
            .args([
                "-L",
                &tmux_socket_name(descriptor.project_id),
                "kill-session",
                "-t",
                &descriptor.session_name,
            ])
            .status()?;
        if !status.success() {
            bail!(
                "failed to kill pane-host tmux session {}",
                descriptor.session_name
            );
        }
    }
    if let Some(dir) = descriptor.socket_path.parent() {
        let _ = fs::remove_dir_all(dir);
    }
    prune_project_runtime_root(descriptor.project_id);
    Ok(())
}

pub fn shutdown_project_hosts(project_id: Uuid) -> Result<usize> {
    tombstone_project_hosts(project_id)?;
    let mut count = 0;
    for (_, descriptor) in list_project_descriptors(project_id)? {
        terminate_tmux_host(&descriptor)?;
        count += 1;
    }
    clear_project_host_tombstone(project_id);
    Ok(count)
}

pub fn reconcile_project_hosts(
    project_id: Uuid,
    configured_terminal_panes: &std::collections::HashSet<u32>,
) -> Result<usize> {
    let mut removed = 0;
    for (_, descriptor) in list_project_descriptors(project_id)? {
        if !configured_terminal_panes.contains(&descriptor.pane_id) {
            terminate_tmux_host(&descriptor)?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[derive(Debug)]
struct Ring {
    chunks: VecDeque<OutputChunk>,
    bytes: usize,
    truncated: bool,
}

impl Ring {
    fn new() -> Self {
        Self {
            chunks: VecDeque::new(),
            bytes: 0,
            truncated: false,
        }
    }

    fn push(&mut self, chunk: OutputChunk) {
        self.bytes += chunk.data.len();
        self.chunks.push_back(chunk);
        while self.bytes > OUTPUT_RING_MAX_BYTES {
            if let Some(oldest) = self.chunks.pop_front() {
                self.bytes = self.bytes.saturating_sub(oldest.data.len());
                self.truncated = true;
            } else {
                break;
            }
        }
    }

    fn bounds(&self) -> (u64, u64, bool) {
        (
            self.chunks.front().map(|chunk| chunk.seq).unwrap_or(0),
            self.chunks.back().map(|chunk| chunk.seq).unwrap_or(0),
            self.truncated,
        )
    }
}

struct HostedProcess {
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    provider_pid: Option<u32>,
    process_group_id: Option<i32>,
    lifecycle: Mutex<(TerminalLifecycle, Option<String>)>,
    ring: Mutex<Ring>,
    next_seq: AtomicU64,
    shutting_down: AtomicBool,
}

impl HostedProcess {
    #[allow(clippy::too_many_arguments)]
    fn spawn(
        provider: Provider,
        binary_path: &str,
        cwd: &str,
        env: &[(String, String)],
        conversation_id: Uuid,
        resume: bool,
        initial_prompt: Option<&str>,
    ) -> Result<Arc<Self>> {
        let pair = native_pty_system().openpty(PtySize {
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        // Spawn the provider directly so portable_pty's pre_exec `setsid()`
        // + `TIOCSCTTY` leave it the session leader with the pty as its
        // controlling terminal and as the pty's foreground process group.
        // Wrapping the binary in an extra `setsid` would fork it into a new
        // session without a controlling terminal, so resizes (TIOCSWINSZ)
        // never deliver SIGWINCH to it and TUIs that cache the terminal
        // size at startup (opencode's opentui) stay stuck at the initial
        // 80-column width. Cleanup still terminates the whole tree because
        // the direct child is a session/process-group leader.
        let mut command = CommandBuilder::new(binary_path);
        for arg in crate::terminal_pane::terminal_args_for(
            &provider,
            conversation_id,
            resume,
            initial_prompt,
        ) {
            command.arg(arg);
        }
        command.cwd(cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        for (key, value) in env {
            command.env(key, value);
        }
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let provider_pid = child.process_id();
        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let process = Arc::new(Self {
            writer: Mutex::new(writer),
            master: Mutex::new(pair.master),
            child: Mutex::new(child),
            provider_pid,
            process_group_id: provider_pid.map(|pid| pid as i32),
            lifecycle: Mutex::new((TerminalLifecycle::Running, None)),
            ring: Mutex::new(Ring::new()),
            next_seq: AtomicU64::new(0),
            shutting_down: AtomicBool::new(false),
        });
        Self::start_reader(&process, reader);
        Ok(process)
    }

    fn start_reader(process: &Arc<Self>, mut reader: Box<dyn Read + Send>) {
        let process = Arc::clone(process);
        thread::spawn(move || {
            let mut buf = vec![0u8; READ_CHUNK_BYTES];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(len) => {
                        let seq = process.next_seq.fetch_add(1, Ordering::Relaxed);
                        if let Ok(mut ring) = process.ring.lock() {
                            ring.push(OutputChunk {
                                seq,
                                data: buf[..len].to_vec(),
                            });
                        }
                    }
                    Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            if !process.shutting_down.load(Ordering::Relaxed) {
                let status = process
                    .child
                    .lock()
                    .ok()
                    .and_then(|mut child| child.wait().ok())
                    .map(|status| format!("exited with status {status:?}"));
                if let Ok(mut lifecycle) = process.lifecycle.lock() {
                    *lifecycle = (TerminalLifecycle::Exited, status);
                }
            }
        });
    }

    fn shutdown(&self) {
        if self.shutting_down.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(group) = self.process_group_id {
            unsafe { libc::kill(-group, libc::SIGTERM) };
            thread::sleep(Duration::from_millis(500));
            unsafe { libc::kill(-group, libc::SIGKILL) };
        }
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn peer_uid(stream: &UnixStream) -> Option<u32> {
    #[cfg(target_os = "linux")]
    unsafe {
        let mut cred: libc::ucred = std::mem::zeroed();
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        if libc::getsockopt(
            std::os::fd::AsRawFd::as_raw_fd(stream),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        ) == 0
        {
            return Some(cred.uid);
        }
    }
    None
}

fn validate_identity(
    descriptor: &RuntimeDescriptor,
    credential: &str,
    supplied_credential: &str,
    project_id: Uuid,
    pane_id: u32,
    runtime_id: Uuid,
) -> Result<()> {
    // Avoid leaking how much of the bearer credential matched through a
    // timing side channel. The socket and credential file are already
    // owner-only, but authentication should remain robust on its own.
    let expected = credential.as_bytes();
    let supplied = supplied_credential.as_bytes();
    let mut mismatch = expected.len() ^ supplied.len();
    for index in 0..expected.len().max(supplied.len()) {
        mismatch |= usize::from(
            expected.get(index).copied().unwrap_or_default()
                ^ supplied.get(index).copied().unwrap_or_default(),
        );
    }
    if mismatch != 0 {
        bail!("invalid pane-host credential");
    }
    if descriptor.project_id != project_id
        || descriptor.pane_id != pane_id
        || descriptor.runtime_id != runtime_id
    {
        bail!("pane-host runtime identity mismatch");
    }
    Ok(())
}

fn validate_controller_generation(
    descriptor: &RuntimeDescriptor,
    controller_id: Uuid,
    controller_generation: u64,
) -> Result<()> {
    if controller_generation < descriptor.controller_generation
        || (controller_generation == descriptor.controller_generation
            && descriptor
                .controller_id
                .is_some_and(|current| current != controller_id))
    {
        bail!("stale pane-host controller generation");
    }
    Ok(())
}

fn serve_controller(
    mut stream: UnixStream,
    descriptor_path: &Path,
    descriptor: &mut RuntimeDescriptor,
    credential: &str,
    process: &mut Option<Arc<HostedProcess>>,
    shutdown: &Arc<AtomicBool>,
) -> Result<bool> {
    if project_tombstone(descriptor.project_id)?.exists() {
        bail!("project pane-host cleanup is in progress");
    }
    if peer_uid(&stream).is_some_and(|uid| uid != descriptor.owner_uid) {
        bail!("pane-host peer uid mismatch");
    }
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let first = read_frame::<ControllerToHost>(&mut stream)?;
    // Close the check/read race: cleanup may begin while a delayed controller
    // is authenticating. Never grant or refresh a lease after tombstoning.
    if project_tombstone(descriptor.project_id)?.exists() {
        bail!("project pane-host cleanup began during controller authentication");
    }
    let (controller_id, controller_generation, acknowledged_seq, reboot_detach) = match first {
        ControllerToHost::Create {
            protocol_version,
            credential: supplied,
            project_id,
            pane_id,
            runtime_id,
            controller_id,
            controller_generation,
            provider,
            binary_path,
            cwd,
            env,
            conversation_id,
            resume,
            initial_prompt,
            ..
        } => {
            if protocol_version != HOST_PROTOCOL_VERSION || process.is_some() {
                bail!("incompatible or duplicate pane-host create");
            }
            validate_identity(
                descriptor, credential, &supplied, project_id, pane_id, runtime_id,
            )?;
            *process = Some(HostedProcess::spawn(
                provider,
                &binary_path,
                &cwd,
                &env,
                conversation_id,
                resume,
                initial_prompt.as_deref(),
            )?);
            (controller_id, controller_generation, None, false)
        }
        ControllerToHost::Adopt {
            protocol_version,
            credential: supplied,
            project_id,
            pane_id,
            runtime_id,
            controller_id,
            controller_generation,
            acknowledged_seq,
            ..
        } => {
            if protocol_version != HOST_PROTOCOL_VERSION || process.is_none() {
                bail!("incompatible or unavailable pane-host runtime");
            }
            validate_identity(
                descriptor, credential, &supplied, project_id, pane_id, runtime_id,
            )?;
            validate_controller_generation(descriptor, controller_id, controller_generation)?;
            (
                controller_id,
                controller_generation,
                acknowledged_seq,
                false,
            )
        }
        _ => bail!("first pane-host frame must create or adopt"),
    };

    descriptor.controller_generation = controller_generation;
    descriptor.controller_id = Some(controller_id);
    write_descriptor(descriptor_path, descriptor)?;
    let process_ref = process
        .as_ref()
        .context("pane-host process missing")?
        .clone();
    let (lifecycle, status) = process_ref
        .lifecycle
        .lock()
        .map(|value| value.clone())
        .unwrap_or((TerminalLifecycle::Unknown, None));
    let (oldest_seq, current_seq, truncated) = process_ref
        .ring
        .lock()
        .map(|ring| ring.bounds())
        .unwrap_or((0, 0, false));
    write_frame(
        &mut stream,
        &HostToController::Adopted {
            protocol_version: HOST_PROTOCOL_VERSION,
            runtime_id: descriptor.runtime_id,
            instance_id: descriptor.instance_id,
            provider_pid: process_ref.provider_pid,
            process_group_id: process_ref.process_group_id,
            lifecycle,
            status: status.clone(),
            oldest_seq,
            current_seq,
            truncated,
            controller_generation,
        },
    )?;

    let initial = process_ref
        .ring
        .lock()
        .map(|ring| ring.chunks.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let mut last_sent = acknowledged_seq;
    for chunk in initial {
        if last_sent.is_none_or(|seq| chunk.seq > seq) {
            write_frame(
                &mut stream,
                &HostToController::Output {
                    seq: chunk.seq,
                    data: chunk.data,
                },
            )?;
            last_sent = Some(chunk.seq);
        }
    }

    stream.set_read_timeout(None)?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let connected = Arc::new(AtomicBool::new(true));
    let reboot = Arc::new(AtomicBool::new(reboot_detach));
    let mut reader = stream.try_clone()?;
    let process_for_reader = process_ref.clone();
    let connected_for_reader = connected.clone();
    let reboot_for_reader = reboot.clone();
    let shutdown_for_reader = shutdown.clone();
    thread::spawn(move || {
        while connected_for_reader.load(Ordering::Relaxed) {
            match read_frame::<ControllerToHost>(&mut reader) {
                Ok(ControllerToHost::Input { data }) => {
                    if let Ok(mut writer) = process_for_reader.writer.lock() {
                        let _ = writer.write_all(&data);
                        let _ = writer.flush();
                    }
                }
                Ok(ControllerToHost::Resize { cols, rows }) if cols > 0 && rows > 0 => {
                    if let Ok(master) = process_for_reader.master.lock() {
                        let _ = master.resize(PtySize {
                            rows,
                            cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        });
                    }
                }
                Ok(ControllerToHost::Ack { .. }) => {}
                Ok(ControllerToHost::Detach { reboot: requested }) => {
                    reboot_for_reader.store(requested, Ordering::Relaxed);
                    break;
                }
                Ok(ControllerToHost::Shutdown) => {
                    process_for_reader.shutdown();
                    shutdown_for_reader.store(true, Ordering::SeqCst);
                    break;
                }
                Ok(_) | Err(_) => break,
            }
        }
        connected_for_reader.store(false, Ordering::SeqCst);
    });

    let mut sent_state = (lifecycle, status);
    while connected.load(Ordering::Relaxed) && !shutdown.load(Ordering::Relaxed) {
        let chunks = process_ref
            .ring
            .lock()
            .map(|ring| {
                ring.chunks
                    .iter()
                    .filter(|chunk| last_sent.is_none_or(|seq| chunk.seq > seq))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for chunk in chunks {
            if write_frame(
                &mut stream,
                &HostToController::Output {
                    seq: chunk.seq,
                    data: chunk.data,
                },
            )
            .is_err()
            {
                connected.store(false, Ordering::SeqCst);
                break;
            }
            last_sent = Some(chunk.seq);
        }
        let state = process_ref
            .lifecycle
            .lock()
            .map(|value| value.clone())
            .unwrap_or((TerminalLifecycle::Unknown, None));
        if state != sent_state {
            if write_frame(
                &mut stream,
                &HostToController::State {
                    lifecycle: state.0,
                    status: state.1.clone(),
                },
            )
            .is_err()
            {
                connected.store(false, Ordering::SeqCst);
                break;
            }
            sent_state = state;
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(reboot.load(Ordering::Relaxed))
}

fn run_host_with_grace(
    runtime_dir: PathBuf,
    adoption_grace: Duration,
    reboot_grace: Duration,
) -> Result<()> {
    ensure_private_dir(&runtime_dir)?;
    let descriptor_path = runtime_dir.join("runtime.json");
    let mut descriptor = read_descriptor(&descriptor_path)?;
    if descriptor.socket_path.parent() != Some(runtime_dir.as_path()) {
        bail!("pane-host descriptor socket escaped runtime directory");
    }
    verify_private_file(&descriptor.credential_path)?;
    let credential = fs::read_to_string(&descriptor.credential_path)?;
    let _ = fs::remove_file(&descriptor.socket_path);
    let listener = UnixListener::bind(&descriptor.socket_path)?;
    fs::set_permissions(&descriptor.socket_path, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let mut process = None;
    let mut detached_at = Instant::now();
    let mut detached_grace = adoption_grace;

    while !shutdown.load(Ordering::Relaxed) {
        if project_tombstone(descriptor.project_id)?.exists() {
            tracing::info!(
                pane_id = descriptor.pane_id,
                "pane-host stopping for project tombstone"
            );
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                match serve_controller(
                    stream,
                    &descriptor_path,
                    &mut descriptor,
                    credential.trim(),
                    &mut process,
                    &shutdown,
                ) {
                    Ok(reboot) => {
                        detached_at = Instant::now();
                        detached_grace = if reboot { reboot_grace } else { adoption_grace };
                        tracing::info!(
                            pane_id = descriptor.pane_id,
                            runtime_id = %descriptor.runtime_id,
                            host_protocol = HOST_PROTOCOL_VERSION,
                            reboot,
                            detached_runtime_count = 1,
                            lease_seconds = detached_grace.as_secs(),
                            "pane-host controller detached"
                        );
                    }
                    Err(error) => {
                        tracing::warn!(%error, pane_id = descriptor.pane_id, "pane-host controller rejected");
                    }
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if process.is_some() && detached_at.elapsed() >= detached_grace {
                    tracing::warn!(
                        pane_id = descriptor.pane_id,
                        runtime_id = %descriptor.runtime_id,
                        detached_runtime_count = 1,
                        detached_age_ms = detached_at.elapsed().as_millis(),
                        "pane-host adoption lease expired"
                    );
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error.into()),
        }
    }
    if let Some(process) = process {
        process.shutdown();
    }
    drop(listener);
    let _ = fs::remove_dir_all(&runtime_dir);
    Ok(())
}

pub fn run_host(runtime_dir: PathBuf) -> Result<()> {
    let config = crate::config::Config::load().unwrap_or_default();
    let adoption_grace = Duration::from_secs(
        config
            .local
            .pane_host_adoption_grace_seconds
            .clamp(30, 60 * 60),
    );
    let reboot_grace = Duration::from_secs(
        config
            .local
            .pane_host_reboot_grace_seconds
            .clamp(60, 2 * 60 * 60),
    );
    run_host_with_grace(runtime_dir, adoption_grace, reboot_grace)
}

pub fn connect_with_retry(path: &Path, timeout: Duration) -> Result<UnixStream> {
    let deadline = Instant::now() + timeout;
    loop {
        match UnixStream::connect(path) {
            Ok(stream) => return Ok(stream),
            Err(error) if Instant::now() < deadline => {
                if !matches!(
                    error.kind(),
                    ErrorKind::NotFound | ErrorKind::ConnectionRefused | ErrorKind::WouldBlock
                ) {
                    tracing::debug!(%error, socket = %path.display(), "pane-host connect retry");
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

pub fn read_credential(path: &Path) -> Result<String> {
    verify_private_file(path)?;
    Ok(fs::read_to_string(path)?.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_runtime(
        root: &tempfile::TempDir,
        pane_id: u32,
    ) -> (PathBuf, RuntimeDescriptor, String) {
        let runtime_dir = root.path().join(format!("runtime-{pane_id}"));
        ensure_private_dir(&runtime_dir).unwrap();
        let credential = format!("test-credential-{pane_id}");
        let credential_path = runtime_dir.join("credential");
        write_private(&credential_path, credential.as_bytes()).unwrap();
        let descriptor = RuntimeDescriptor {
            protocol_version: HOST_PROTOCOL_VERSION,
            project_id: Uuid::new_v4(),
            pane_id,
            runtime_id: Uuid::new_v4(),
            instance_id: Uuid::new_v4(),
            owner_uid: unsafe { libc::getuid() },
            controller_generation: 1,
            controller_id: None,
            session_name: format!("test-{pane_id}"),
            socket_path: runtime_dir.join("c.sock"),
            credential_path,
            created_at_unix_ms: now_unix_ms(),
        };
        write_descriptor(&runtime_dir.join("runtime.json"), &descriptor).unwrap();
        (runtime_dir, descriptor, credential)
    }

    fn create_test_process(
        stream: &mut UnixStream,
        descriptor: &RuntimeDescriptor,
        credential: &str,
        provider_path: &Path,
    ) -> HostToController {
        write_frame(
            stream,
            &ControllerToHost::Create {
                protocol_version: HOST_PROTOCOL_VERSION,
                credential: credential.to_string(),
                project_id: descriptor.project_id,
                pane_id: descriptor.pane_id,
                runtime_id: descriptor.runtime_id,
                controller_id: Uuid::new_v4(),
                controller_generation: 1,
                provider: Provider::Claude,
                binary_path: provider_path.to_string_lossy().to_string(),
                cwd: provider_path
                    .parent()
                    .unwrap()
                    .to_string_lossy()
                    .to_string(),
                env: Vec::new(),
                conversation_id: Uuid::new_v4(),
                resume: false,
                initial_prompt: None,
            },
        )
        .unwrap();
        read_frame(stream).unwrap()
    }

    #[test]
    fn frame_round_trip_and_malformed_limit() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("test.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let writer = thread::spawn(move || {
            let mut stream = UnixStream::connect(socket).unwrap();
            write_frame(&mut stream, &ControllerToHost::Ack { seq: 9 }).unwrap();
        });
        let (mut stream, _) = listener.accept().unwrap();
        assert!(matches!(
            read_frame(&mut stream).unwrap(),
            ControllerToHost::Ack { seq: 9 }
        ));
        writer.join().unwrap();

        for length in [0_u32, (MAX_FRAME_BYTES as u32) + 1] {
            let (mut writer, mut reader) = UnixStream::pair().unwrap();
            writer.write_all(&length.to_be_bytes()).unwrap();
            assert!(read_frame::<ControllerToHost>(&mut reader).is_err());
        }
    }

    #[test]
    fn ring_evicts_whole_chunks_and_marks_truncation() {
        let mut ring = Ring::new();
        ring.push(OutputChunk {
            seq: 1,
            data: vec![1; OUTPUT_RING_MAX_BYTES],
        });
        ring.push(OutputChunk {
            seq: 2,
            data: vec![2; 1],
        });
        assert_eq!(ring.chunks.len(), 1);
        assert_eq!(ring.chunks.front().unwrap().seq, 2);
        assert!(ring.truncated);
    }

    #[test]
    fn runtime_files_are_private_and_descriptor_has_no_credential() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("runtime");
        ensure_private_dir(&dir).unwrap();
        let secret = dir.join("credential");
        write_private(&secret, b"super-secret").unwrap();
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&secret).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let descriptor = RuntimeDescriptor {
            protocol_version: HOST_PROTOCOL_VERSION,
            project_id: Uuid::new_v4(),
            pane_id: 4,
            runtime_id: Uuid::new_v4(),
            instance_id: Uuid::new_v4(),
            owner_uid: unsafe { libc::getuid() },
            controller_generation: 1,
            controller_id: None,
            session_name: "test".to_string(),
            socket_path: dir.join("c.sock"),
            credential_path: secret,
            created_at_unix_ms: 1,
        };
        let encoded = serde_json::to_string(&descriptor).unwrap();
        assert!(!encoded.contains("super-secret"));
    }

    #[test]
    fn identity_mismatch_is_rejected() {
        let descriptor = RuntimeDescriptor {
            protocol_version: HOST_PROTOCOL_VERSION,
            project_id: Uuid::new_v4(),
            pane_id: 7,
            runtime_id: Uuid::new_v4(),
            instance_id: Uuid::new_v4(),
            owner_uid: unsafe { libc::getuid() },
            controller_generation: 1,
            controller_id: None,
            session_name: "test".to_string(),
            socket_path: PathBuf::from("c.sock"),
            credential_path: PathBuf::from("credential"),
            created_at_unix_ms: 1,
        };
        assert!(validate_identity(
            &descriptor,
            "right",
            "right",
            Uuid::new_v4(),
            descriptor.pane_id,
            descriptor.runtime_id,
        )
        .is_err());
        assert!(validate_identity(
            &descriptor,
            "right",
            "wrong",
            descriptor.project_id,
            descriptor.pane_id,
            descriptor.runtime_id,
        )
        .is_err());

        let current_controller = Uuid::new_v4();
        let mut leased = descriptor.clone();
        leased.controller_generation = 4;
        leased.controller_id = Some(current_controller);
        assert!(validate_controller_generation(&leased, current_controller, 4).is_ok());
        assert!(validate_controller_generation(&leased, Uuid::new_v4(), 4).is_err());
        assert!(validate_controller_generation(&leased, current_controller, 3).is_err());
        assert!(validate_controller_generation(&leased, Uuid::new_v4(), 5).is_ok());
    }

    #[test]
    fn incompatible_descriptor_is_rejected_without_reading_runtime_state() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runtime.json");
        write_private(
            &path,
            serde_json::to_string(&serde_json::json!({
                "protocol_version": HOST_PROTOCOL_VERSION + 1,
                "project_id": Uuid::new_v4(),
                "pane_id": 1,
                "runtime_id": Uuid::new_v4(),
                "instance_id": Uuid::new_v4(),
                "owner_uid": unsafe { libc::getuid() },
                "controller_generation": 1,
                "session_name": "incompatible",
                "socket_path": root.path().join("c.sock"),
                "credential_path": root.path().join("credential"),
                "created_at_unix_ms": 1,
            }))
            .unwrap()
            .as_bytes(),
        )
        .unwrap();
        assert!(read_descriptor(&path).is_err());
    }

    #[test]
    fn reboot_handoff_markers_are_atomic_private_and_reject_expiry_or_corruption() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("handoff.json");
        let mut marker = RebootHandoffMarker {
            protocol_version: HOST_PROTOCOL_VERSION,
            request_id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            expected_executable: PathBuf::from("/usr/bin/apas"),
            expected_version: "26.08.1".to_string(),
            controller_generation: 2,
            runtime_ids: vec![Uuid::new_v4()],
            deadline_unix_ms: now_unix_ms() + 60_000,
        };
        write_handoff_marker(&path, &marker).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            read_handoff_marker(&path).unwrap().request_id,
            marker.request_id
        );

        marker.deadline_unix_ms = now_unix_ms().saturating_sub(1);
        write_handoff_marker(&path, &marker).unwrap();
        assert!(read_handoff_marker(&path).is_err());
        write_private(&path, b"not-json").unwrap();
        assert!(read_handoff_marker(&path).is_err());
    }

    #[test]
    fn provider_exit_is_reported_and_host_shutdown_erases_runtime() {
        let root = tempfile::tempdir().unwrap();
        let (runtime_dir, descriptor, credential) = test_runtime(&root, 21);
        let provider = root.path().join("exiting-provider.sh");
        fs::write(&provider, b"#!/bin/sh\nexit 7\n").unwrap();
        fs::set_permissions(&provider, fs::Permissions::from_mode(0o700)).unwrap();
        let host_dir = runtime_dir.clone();
        let host = thread::spawn(move || {
            run_host_with_grace(host_dir, Duration::from_secs(2), Duration::from_secs(2)).unwrap()
        });
        let mut stream =
            connect_with_retry(&descriptor.socket_path, Duration::from_secs(3)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let HostToController::Adopted { lifecycle, .. } =
            create_test_process(&mut stream, &descriptor, &credential, &provider)
        else {
            panic!("host did not acknowledge provider creation");
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut exited = lifecycle == TerminalLifecycle::Exited;
        while Instant::now() < deadline && !exited {
            if let HostToController::State { lifecycle, .. } = read_frame(&mut stream).unwrap() {
                exited = lifecycle == TerminalLifecycle::Exited;
            }
        }
        assert!(exited, "provider exit was not reported");
        write_frame(&mut stream, &ControllerToHost::Shutdown).unwrap();
        drop(stream);
        host.join().unwrap();
        assert!(!runtime_dir.exists());
    }

    #[test]
    fn detached_lease_expiry_kills_the_complete_provider_process_group() {
        let root = tempfile::tempdir().unwrap();
        let (runtime_dir, descriptor, credential) = test_runtime(&root, 22);
        let provider = root.path().join("tree-provider.sh");
        fs::write(&provider, b"#!/bin/sh\nsleep 30 &\nwait\n").unwrap();
        fs::set_permissions(&provider, fs::Permissions::from_mode(0o700)).unwrap();
        let host_dir = runtime_dir.clone();
        let host = thread::spawn(move || {
            run_host_with_grace(
                host_dir,
                Duration::from_millis(100),
                Duration::from_millis(150),
            )
            .unwrap()
        });
        let mut stream =
            connect_with_retry(&descriptor.socket_path, Duration::from_secs(3)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let HostToController::Adopted {
            process_group_id: Some(group),
            ..
        } = create_test_process(&mut stream, &descriptor, &credential, &provider)
        else {
            panic!("provider process group was not reported");
        };
        drop(stream);
        host.join().unwrap();
        assert!(!runtime_dir.exists());
        let still_exists = unsafe { libc::kill(-group, 0) } == 0;
        assert!(
            !still_exists,
            "provider process group survived lease cleanup"
        );
    }

    #[test]
    fn project_host_cleanup_is_tombstoned_and_idempotent_for_stale_descriptors() {
        let project_id = Uuid::new_v4();
        let runtime_id = Uuid::new_v4();
        let paths = runtime_paths(project_id, 55, runtime_id).unwrap();
        write_private(&paths.credential, b"test-only-credential").unwrap();
        let descriptor = RuntimeDescriptor {
            protocol_version: HOST_PROTOCOL_VERSION,
            project_id,
            pane_id: 55,
            runtime_id,
            instance_id: Uuid::new_v4(),
            owner_uid: unsafe { libc::getuid() },
            controller_generation: 1,
            controller_id: None,
            session_name: format!("missing-{runtime_id}"),
            socket_path: paths.socket.clone(),
            credential_path: paths.credential.clone(),
            created_at_unix_ms: now_unix_ms(),
        };
        write_descriptor(&paths.descriptor, &descriptor).unwrap();

        assert_eq!(shutdown_project_hosts(project_id).unwrap(), 1);
        assert!(!paths.dir.exists());
        assert_eq!(shutdown_project_hosts(project_id).unwrap(), 0);
        assert!(!project_tombstone(project_id).unwrap().exists());
        let _ = fs::remove_dir_all(project_runtime_root(project_id).unwrap());
    }

    #[test]
    fn pane_cleanup_removes_the_project_directory_only_once_it_is_empty() {
        // The per-pane directory was always removed; the project directory above
        // it never was, so every project that hosted a pane left an empty
        // skeleton behind forever.
        let root = tempfile::tempdir().unwrap();
        let prev = std::env::var_os("XDG_RUNTIME_DIR");
        std::env::set_var("XDG_RUNTIME_DIR", root.path());

        let project = Uuid::new_v4();
        let project_dir = project_runtime_root(project).unwrap();

        let first = runtime_paths(project, 1, Uuid::new_v4()).unwrap();
        let second = runtime_paths(project, 2, Uuid::new_v4()).unwrap();
        assert!(first.dir.exists() && second.dir.exists());

        let desc = |paths: &RuntimePaths, pane_id: u32| RuntimeDescriptor {
            protocol_version: HOST_PROTOCOL_VERSION,
            project_id: project,
            pane_id,
            runtime_id: Uuid::new_v4(),
            instance_id: Uuid::new_v4(),
            controller_id: Some(Uuid::new_v4()),
            controller_generation: 1,
            owner_uid: unsafe { libc::getuid() },
            created_at_unix_ms: 0,
            session_name: format!("ph_{pane_id}_test"),
            socket_path: paths.socket.clone(),
            credential_path: paths.credential.clone(),
        };

        // One pane closing must not take the project directory with it — its
        // sibling is still hosted there.
        terminate_tmux_host(&desc(&first, 1)).unwrap();
        assert!(!first.dir.exists(), "closed pane's directory should be gone");
        assert!(
            project_dir.exists(),
            "project directory must survive while another pane is hosted"
        );

        // A tombstone means cleanup is in flight; the directory has to stay.
        tombstone_project_hosts(project).unwrap();
        terminate_tmux_host(&desc(&second, 2)).unwrap();
        assert!(
            project_dir.exists(),
            "project directory must survive an in-progress tombstone"
        );

        // Clearing the tombstone empties it, and only then is it removed.
        clear_project_host_tombstone(project);
        assert!(
            !project_dir.exists(),
            "empty project directory should be pruned"
        );

        match prev {
            Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
            None => std::env::remove_var("XDG_RUNTIME_DIR"),
        }
    }

    #[test]
    fn project_stop_wins_against_reboot_detach_and_delayed_adoption() {
        let root = tempfile::tempdir().unwrap();
        let (runtime_dir, descriptor, credential) = test_runtime(&root, 56);
        let provider = root.path().join("stop-race-provider.sh");
        fs::write(&provider, b"#!/bin/sh\nsleep 30 &\nwait\n").unwrap();
        fs::set_permissions(&provider, fs::Permissions::from_mode(0o700)).unwrap();
        let host_dir = runtime_dir.clone();
        let host = thread::spawn(move || {
            run_host_with_grace(host_dir, Duration::from_secs(2), Duration::from_secs(2)).unwrap()
        });
        let mut controller =
            connect_with_retry(&descriptor.socket_path, Duration::from_secs(3)).unwrap();
        controller
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let HostToController::Adopted {
            process_group_id: Some(group),
            ..
        } = create_test_process(&mut controller, &descriptor, &credential, &provider)
        else {
            panic!("provider process group was not reported");
        };

        tombstone_project_hosts(descriptor.project_id).unwrap();
        write_frame(&mut controller, &ControllerToHost::Detach { reboot: true }).unwrap();
        drop(controller);
        host.join().unwrap();
        assert!(!runtime_dir.exists());
        assert!(UnixStream::connect(&descriptor.socket_path).is_err());
        assert!(unsafe { libc::kill(-group, 0) } != 0);
        clear_project_host_tombstone(descriptor.project_id);
        let _ = fs::remove_dir_all(project_runtime_root(descriptor.project_id).unwrap());
    }

    fn exercise_controller_replacement(provider_kind: Provider, project_id: Uuid, pane_id: u32) {
        let root = tempfile::tempdir().unwrap();
        let runtime_dir = root.path().join("runtime");
        ensure_private_dir(&runtime_dir).unwrap();
        let provider = root.path().join("fake-provider.sh");
        fs::write(
            &provider,
            b"#!/bin/sh\nprintf 'ready\\n'\nwhile IFS= read -r line; do printf 'echo:%s\\n' \"$line\"; done\n",
        )
        .unwrap();
        fs::set_permissions(&provider, fs::Permissions::from_mode(0o700)).unwrap();

        let credential = "test-credential";
        let credential_path = runtime_dir.join("credential");
        write_private(&credential_path, credential.as_bytes()).unwrap();
        let descriptor_path = runtime_dir.join("runtime.json");
        let descriptor = RuntimeDescriptor {
            protocol_version: HOST_PROTOCOL_VERSION,
            project_id,
            pane_id,
            runtime_id: Uuid::new_v4(),
            instance_id: Uuid::new_v4(),
            owner_uid: unsafe { libc::getuid() },
            controller_generation: 1,
            controller_id: None,
            session_name: "test".to_string(),
            socket_path: runtime_dir.join("c.sock"),
            credential_path,
            created_at_unix_ms: now_unix_ms(),
        };
        write_descriptor(&descriptor_path, &descriptor).unwrap();
        let host_dir = runtime_dir.clone();
        let host = thread::spawn(move || run_host(host_dir).unwrap());

        let mut first =
            connect_with_retry(&descriptor.socket_path, Duration::from_secs(3)).unwrap();
        first
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write_frame(
            &mut first,
            &ControllerToHost::Create {
                protocol_version: HOST_PROTOCOL_VERSION,
                credential: credential.to_string(),
                project_id: descriptor.project_id,
                pane_id: descriptor.pane_id,
                runtime_id: descriptor.runtime_id,
                controller_id: Uuid::new_v4(),
                controller_generation: 1,
                provider: provider_kind,
                binary_path: provider.to_string_lossy().to_string(),
                cwd: root.path().to_string_lossy().to_string(),
                env: Vec::new(),
                conversation_id: Uuid::new_v4(),
                resume: false,
                initial_prompt: None,
            },
        )
        .unwrap();
        let HostToController::Adopted {
            provider_pid: first_pid,
            process_group_id: first_group,
            instance_id: first_instance,
            ..
        } = read_frame(&mut first).unwrap()
        else {
            panic!("host did not create provider");
        };
        write_frame(
            &mut first,
            &ControllerToHost::Input {
                data: b"hello\n".to_vec(),
            },
        )
        .unwrap();
        let mut highest_seq = 0;
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let HostToController::Output { seq, data } = read_frame(&mut first).unwrap() {
                highest_seq = highest_seq.max(seq);
                if String::from_utf8_lossy(&data).contains("hello") {
                    break;
                }
            }
        }
        // Queue output immediately before detaching. Whether the provider
        // writes it just before or just after socket close, the stable ring
        // must replay it to the replacement controller exactly once.
        write_frame(
            &mut first,
            &ControllerToHost::Input {
                data: b"detached-output\n".to_vec(),
            },
        )
        .unwrap();
        write_frame(&mut first, &ControllerToHost::Detach { reboot: true }).unwrap();
        drop(first);

        let mut second =
            connect_with_retry(&descriptor.socket_path, Duration::from_secs(3)).unwrap();
        second
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write_frame(
            &mut second,
            &ControllerToHost::Adopt {
                protocol_version: HOST_PROTOCOL_VERSION,
                credential: credential.to_string(),
                project_id: descriptor.project_id,
                pane_id: descriptor.pane_id,
                runtime_id: descriptor.runtime_id,
                controller_id: Uuid::new_v4(),
                controller_generation: 2,
                acknowledged_seq: Some(highest_seq),
            },
        )
        .unwrap();
        let HostToController::Adopted {
            provider_pid: second_pid,
            process_group_id: second_group,
            instance_id: second_instance,
            current_seq,
            ..
        } = read_frame(&mut second).unwrap()
        else {
            panic!("host did not permit replacement controller");
        };
        assert_eq!(second_pid, first_pid);
        assert_eq!(second_group, first_group);
        assert_eq!(second_instance, first_instance);
        assert!(current_seq >= highest_seq);
        let mut replayed_sequences = std::collections::HashSet::new();
        let mut saw_detached = false;
        let replay_deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < replay_deadline && !saw_detached {
            if let HostToController::Output { seq, data } = read_frame(&mut second).unwrap() {
                assert!(seq > highest_seq, "acknowledged output must not replay");
                assert!(
                    replayed_sequences.insert(seq),
                    "replayed output sequence duplicated"
                );
                saw_detached = String::from_utf8_lossy(&data).contains("detached-output");
            }
        }
        assert!(
            saw_detached,
            "output produced around detach was not replayed"
        );

        write_frame(
            &mut second,
            &ControllerToHost::Resize {
                cols: 132,
                rows: 43,
            },
        )
        .unwrap();
        write_frame(
            &mut second,
            &ControllerToHost::Input {
                data: b"after-adoption\n".to_vec(),
            },
        )
        .unwrap();
        let mut saw_after_adoption = false;
        let input_deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < input_deadline && !saw_after_adoption {
            if let HostToController::Output { seq, data } = read_frame(&mut second).unwrap() {
                assert!(
                    replayed_sequences.insert(seq),
                    "live output sequence duplicated"
                );
                saw_after_adoption = String::from_utf8_lossy(&data).contains("after-adoption");
            }
        }
        assert!(saw_after_adoption, "input stopped working after adoption");
        write_frame(&mut second, &ControllerToHost::Shutdown).unwrap();
        drop(second);
        host.join().unwrap();
    }

    #[test]
    fn claude_process_survives_controller_replacement() {
        exercise_controller_replacement(Provider::Claude, Uuid::new_v4(), 12);
    }

    #[test]
    fn codex_process_survives_controller_replacement() {
        exercise_controller_replacement(Provider::Codex, Uuid::new_v4(), 12);
    }

    #[test]
    fn opencode_process_survives_controller_replacement() {
        exercise_controller_replacement(Provider::Opencode, Uuid::new_v4(), 12);
    }

    #[test]
    fn multiple_panes_adopt_independently_for_one_project() {
        let project_id = Uuid::new_v4();
        thread::scope(|scope| {
            scope.spawn(|| exercise_controller_replacement(Provider::Claude, project_id, 30));
            scope.spawn(|| exercise_controller_replacement(Provider::Codex, project_id, 31));
        });
    }
}
