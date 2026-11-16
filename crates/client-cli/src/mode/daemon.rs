use crate::daemon_registry::{ClaimMode, ClaimOutcome, ClaimRefusal, InstanceClaimGuard};
use anyhow::{Context, Result};
use futures::{future::BoxFuture, SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use shared::{
    DaemonToServer, DeepseekBackendInfo, EffectiveProjectPolicy, MachineInfo, MachineProjectInfo,
    ProjectProvisioningMode, ServerToDaemon,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
/// How long a stop waits for a project's task to finish tearing down before
/// giving up on it.
const PROJECT_STOP_WAIT: Duration = Duration::from_secs(30);

/// How often the daemon checks whether a newer `apas` has been installed.
///
/// Deliberately slow. An upgrade is never urgent — the running daemon works
/// fine — and the check costs a `stat` plus, on change, spawning
/// `apas --version`. Tying it to the 10s heartbeat meant the version path was
/// exercised 360x more often than anything could benefit from.
const USAGE_REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
const VERSION: &str = env!("APAS_VERSION");
/// How long a checkout's remote is trusted before it is read again. Remotes
/// rarely change, and every read runs `git` against a home that may be NFS.
const GIT_REMOTE_MAX_AGE: Duration = Duration::from_secs(5 * 60);
const TMUX_SESSION_PREFIX: &str = "apas";
const DEEPSEEK_API_BASE_URL: &str = "https://api.deepseek.com/anthropic";
const PROVISIONING_RECEIPT: &str = ".apas-provisioning.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ProvisioningReceipt {
    request_id: String,
    project_id: String,
}

/// The PATH the user's interactive shell builds, so providers installed by
/// nvm, cargo or Homebrew are found.
///
/// On macOS the shell is asked as a login shell too (`-lic`): zsh reads
/// `/etc/zprofile` (path_helper) and `~/.zprofile`, where Homebrew's
/// `brew shellenv` adds `/opt/homebrew/bin`, only then. A daemon started
/// from a minimal environment otherwise lost Homebrew's tmux, and its
/// terminals silently fell back to direct PTYs. Linux keeps `-ic`: a login
/// shell there can print from `.bash_profile`, which would pollute the answer.
fn resolve_user_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "/bin/bash".to_string());

    let output = Command::new(shell)
        .arg(if cfg!(target_os = "macos") {
            "-lic"
        } else {
            "-ic"
        })
        .arg("printf %s \"$PATH\"")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

/// The shell's PATH, then any inherited entry it lacks. Replacing the PATH
/// outright could drop a directory the daemon was started with.
fn merge_paths(shell: &str, inherited: &str) -> String {
    let mut entries: Vec<&str> = Vec::new();
    for entry in shell.split(':').chain(inherited.split(':')) {
        if !entry.is_empty() && !entries.contains(&entry) {
            entries.push(entry);
        }
    }
    entries.join(":")
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| crate::omp_install::is_executable_file(candidate))
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn deepseek_backend_info_from_config(
    config: &crate::config::Config,
) -> Option<DeepseekBackendInfo> {
    let api_base_url = Some(DEEPSEEK_API_BASE_URL.to_string());
    let api_key = normalize_optional_string(config.local.deepseek_api_key.clone());
    let api_key_configured = api_key.is_some();
    Some(DeepseekBackendInfo {
        api_base_url,
        api_key,
        api_key_configured,
    })
}

fn update_local_deepseek_backend_config(
    _api_base_url: Option<String>,
    api_key: Option<String>,
    clear_api_key: bool,
) -> Result<Option<DeepseekBackendInfo>> {
    let mut config = crate::config::Config::load().unwrap_or_default();
    config.local.deepseek_api_base_url = Some(DEEPSEEK_API_BASE_URL.to_string());

    if clear_api_key {
        config.local.deepseek_api_key = None;
    } else if let Some(key) = api_key {
        config.local.deepseek_api_key = normalize_optional_string(Some(key));
    }

    config.save()?;
    Ok(deepseek_backend_info_from_config(&config))
}

fn headless_pids_for(project_path: &Path) -> Vec<u32> {
    let path_str = project_path.to_string_lossy();
    let my_pid = std::process::id();
    // `/proc` on Linux, `ps` elsewhere. Reading `/proc` alone found nothing
    // on macOS, so a standalone `apas --headless -d <path>` there was never
    // seen and the daemon could start a second owner of the same `.apas`.
    crate::process_info::list()
        .into_iter()
        .filter(|(pid, _)| *pid != my_pid)
        .filter(|(_, args)| {
            args.first().is_some_and(|arg| arg.contains("apas"))
                && args.iter().any(|arg| arg == "--headless")
                && crate::process_info::args_have_flag_value(args, "-d", path_str.as_ref())
        })
        .map(|(pid, _)| pid)
        .collect()
}

fn headless_pid_for(project_path: &Path) -> Option<u32> {
    headless_pids_for(project_path).into_iter().next()
}

/// NFS-shared config dir, where the peer registry and project claims live.
/// Distinct from `Config::runtime_dir()`, which is host-local.
fn config_dir_for_registry() -> std::path::PathBuf {
    crate::config::Config::config_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
}

/// Check if there's already a running `apas --headless` process for the given project path.
/// Prevents the daemon from spawning duplicates when a CLI was started externally
/// or survived a daemon restart.
pub(crate) fn is_headless_running_for(project_path: &Path) -> bool {
    headless_pid_for(project_path).is_some()
}

/// Resident-set size of a running process in KiB (`/proc` on Linux, `ps`
/// elsewhere). `None` if the process is gone.
fn read_process_rss_kb(pid: u32) -> Option<u64> {
    crate::process_info::rss_kb(pid)
}

fn sanitize_for_unit(project_id: &str) -> String {
    project_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn tmux_session_name(project_id: &str) -> String {
    format!("{}_{}", TMUX_SESSION_PREFIX, sanitize_for_unit(project_id))
}

/// Per-project tmux socket name. We give each project its own tmux server so
/// one project's processes can live in their own systemd scope independent of
/// the daemon and of other projects.
fn tmux_socket_name(project_id: &str) -> String {
    format!("apas-{}", sanitize_for_unit(project_id))
}

fn tmux_has_session(project_id: &str, session_name: &str) -> bool {
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

fn tmux_kill_session(project_id: &str, session_name: &str) -> Result<()> {
    if !tmux_has_session(project_id, session_name) {
        return Ok(());
    }

    let output = Command::new("tmux")
        .args([
            "-L",
            &tmux_socket_name(project_id),
            "kill-session",
            "-t",
            session_name,
        ])
        .output()?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    anyhow::bail!("Failed to kill tmux session {}: {}", session_name, stderr)
}

#[derive(Debug)]
struct ProjectEntry {
    name: Option<String>,
    path: PathBuf,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ResumeProject {
    project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<PathBuf>,
    /// The server this daemon was connected to when it replaced itself keeps
    /// one CLI per session ([`StartScope::Alone`]), so the replacement starts
    /// the project again as its only runtime where no other host runs it
    /// ([`StartScope::again`]). The replacement cannot ask a server first:
    /// it resumes before it connects, and resuming as an instance would drop
    /// the panes tagged for other machines from that server's roster. Absent
    /// from manifests written before this, which resume as configured, as
    /// they always did.
    #[serde(default, skip_serializing_if = "is_false")]
    single_machine_server: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ResumeManifestEntry {
    Detailed(ResumeProject),
    /// Compatibility with manifests written by versions before paths were
    /// included. Those can still resume when the registry is healthy.
    LegacyProjectId(String),
}

/// Projects that were running when this instance replaced itself.
///
/// Under the process-per-project model an upgrade left projects alone: they
/// lived in their own tmux sessions and `exec` never touched them. Running
/// them inside this process means `exec` takes them with it, so what was
/// running has to be written down before the replacement and started again
/// after it.
///
/// Kept in the runtime directory deliberately: it is volatile, so a machine
/// reboot clears it and nothing auto-starts on boot that nobody asked for.
fn resume_manifest_path() -> Option<std::path::PathBuf> {
    let dir = crate::config::Config::runtime_dir().ok()?.join("sup");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("resume.json"))
}

fn write_resume_manifest(projects: &[ResumeProject]) {
    let Some(path) = resume_manifest_path() else {
        return;
    };
    write_resume_manifest_at(&path, projects);
}

/// The manifest logic, given its path. Split out so it is testable without
/// mutating process-wide environment variables — several test modules already
/// move `XDG_RUNTIME_DIR` under their own locks, and adding another writer
/// makes them race rather than serialise.
fn write_resume_manifest_at(path: &std::path::Path, projects: &[ResumeProject]) {
    if projects.is_empty() {
        let _ = std::fs::remove_file(path);
        return;
    }
    match serde_json::to_vec(projects) {
        Ok(bytes) => {
            if let Err(err) = std::fs::write(path, bytes) {
                // Losing this costs a restart of the projects, not the
                // projects themselves, so it is not worth failing the upgrade.
                tracing::warn!(%err, "could not record which projects to resume");
            }
        }
        Err(err) => tracing::warn!(%err, "could not encode the resume manifest"),
    }
}

/// Read and clear the manifest. Cleared on read so a crash during resume does
/// not leave the instance trying the same start on every boot.
///
/// On a Mac it also takes the manifest an APAS from before the macOS runtime
/// dir wrote under the temp dir. That APAS is what a Mac's first Reboot to
/// update replaces, and `exec` keeps its `$TMPDIR`, so its manifest is found
/// exactly; without this the projects it was running did not come back. A
/// manifest left there by a replacement that then did not stay (another old
/// daemon was found running) is taken by the next daemon to start instead,
/// which is the daemon that should resume them.
fn take_resume_manifest() -> Vec<ResumeProject> {
    let legacy = crate::config::Config::legacy_daemon_state_dirs()
        .into_iter()
        .map(|dir| dir.join("sup").join("resume.json"));
    let paths: Vec<std::path::PathBuf> = resume_manifest_path().into_iter().chain(legacy).collect();
    take_resume_manifests_at(&paths)
}

/// Take every manifest in `paths`, the first naming a project winning.
fn take_resume_manifests_at(paths: &[std::path::PathBuf]) -> Vec<ResumeProject> {
    let mut projects: Vec<ResumeProject> = Vec::new();
    for path in paths {
        for project in take_resume_manifest_at(path) {
            if !projects
                .iter()
                .any(|known| known.project_id == project.project_id)
            {
                projects.push(project);
            }
        }
    }
    projects
}

fn take_resume_manifest_at(path: &std::path::Path) -> Vec<ResumeProject> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let _ = std::fs::remove_file(path);
    serde_json::from_slice::<Vec<ResumeManifestEntry>>(&bytes)
        .unwrap_or_default()
        .into_iter()
        .map(|entry| match entry {
            ResumeManifestEntry::Detailed(project) => project,
            ResumeManifestEntry::LegacyProjectId(project_id) => ResumeProject {
                project_id,
                path: None,
                single_machine_server: false,
            },
        })
        .collect()
}

/// Re-exec into the installed binary when it is newer than the running one.
///
/// The daemon is the only thing that upgrades a machine unattended. Before
/// this, `ensure_daemon_running` was the sole upgrade path and it runs on
/// *interactive CLI startup* — so a node nobody logs into keeps its daemon
/// forever. zoo-002 sat nine versions behind for exactly that reason.
///
/// Uses `exec` rather than spawn-and-exit, which matters for three reasons:
///
///  * the pid is preserved, so the `daemon.json` state file stays correct and
///    `detect_running_daemon` is not briefly fooled into starting a second one,
///  * the session is preserved, so the daemon stays `setsid`-detached without
///    having to re-detach,
///  * destructors do **not** run, so `RegistrationGuard` never withdraws this
///    host's record or releases its project claims. A spawn-and-exit would open
///    a window where a peer daemon sees the projects unclaimed and could spawn
///    a duplicate CLI against the same `.apas` and worktrees — the exact race
///    the claim system exists to prevent.
///
/// Projects run inside this process, so the `exec` takes them with it: the
/// resume manifest written here is how the replacement starts them again. Pane
/// hosts are separate processes and are unaffected; the resumed projects adopt
/// them.
///
/// Returns only on failure — on success the process image is gone.
#[cfg(unix)]
fn exec_into_newer_binary(version: &str, running: &[ResumeProject]) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    // `exec` runs no destructors, so this is the last moment the replacement
    // can be told what was running.
    write_resume_manifest(running);
    let exe = crate::update::resolve_preferred_apas_executable();
    let args: Vec<String> = std::env::args().skip(1).collect();
    tracing::info!(
        %version,
        exe = %exe.display(),
        "daemon upgrading itself: re-exec into newer binary"
    );
    std::process::Command::new(exe).args(args).exec()
}

/// Apply a requested restart. Returns only on failure — success replaces this
/// process image.
///
/// Every fallible step runs while this daemon is still serving, so a failed
/// update leaves a working daemon rather than a machine with none. That is
/// the same discipline `prepare_cli_restart` uses for project CLIs.
#[cfg(unix)]
fn perform_requested_restart(running: &[ResumeProject]) -> anyhow::Error {
    tracing::info!("daemon restart requested: preparing replacement");
    let executable = match crate::update::prepare_cli_restart() {
        Ok(executable) => executable,
        Err(err) => {
            // Nothing has been replaced, so the machine keeps the daemon it
            // had. The updater lock also ensures a peer cannot be midway
            // through replacing the shared executable when validation runs.
            tracing::error!(%err, "daemon restart: update failed; staying on the current daemon");
            return err;
        }
    };
    let version = crate::update::binary_version(&executable)
        .unwrap_or_else(|| env!("APAS_VERSION").to_string());
    anyhow::Error::from(exec_into_newer_binary(&version, running))
}

#[derive(Debug)]
/// A project running inside this process.
///
/// The handle and the flag are the whole of supervision: the flag ends the
/// project through its ordinary teardown, and the handle says whether it is
/// still going. This replaces asking `/proc` whether some other process
/// exists, which is how running state could disagree with reality.
struct RunningProject {
    shutdown: Arc<AtomicBool>,
    handle: tokio::task::JoinHandle<()>,
    /// Why the task ended, when it ended on its own for a reason worth
    /// reporting: the server rejected its session, or the runtime failed.
    /// Filled by the task as it ends; see [`RunningProject::ended_because`].
    ended_because: Arc<Mutex<Option<String>>>,
    /// The shape this start claimed the project in, which the heartbeat keeps
    /// it in: [`ClaimMode::Instances`] for an instance, and
    /// [`ClaimMode::Exclusive`] for the project's only runtime, started by a
    /// daemon that runs projects alone or alone because the server keeps one
    /// CLI per session ([`StartScope::Alone`]).
    claims: ClaimMode,
}

impl RunningProject {
    /// Why the task ended, once it has, for the inventory to report as the
    /// project's `last_error`.
    ///
    /// The start that ran it was reported a success, since the runtime did
    /// start, and the reason was only logged. So a start the server then
    /// refused, beside an older CLI on another machine say, showed the project
    /// running and then stopped with nothing on the Machines page saying why.
    fn ended_because(&self) -> Option<String> {
        if !self.handle.is_finished() {
            return None;
        }
        self.ended_because
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// Whether a start may run beside other machines' instances of its project,
/// which also decides how its runtime owns the project's panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartScope {
    /// As this daemon's [`ClaimMode`] allows: a capable daemon starts an
    /// instance, which runs only its own machine's panes.
    AsConfigured,
    /// Only where no other host runs the project, whatever the claim mode
    /// allows: the server this daemon is connected to keeps one CLI per
    /// session ([`DaemonState::requested_start_scope`]). And then as daemons
    /// did before instances: as the project's only runtime, which restores
    /// every pane in `.apas`, other machines' included, saves them untagged
    /// and does not advertise `project_instances_v1`, and with claims of the
    /// shape such daemons write
    /// ([`crate::daemon_registry::claim_project_alone`]).
    ///
    /// An instance would report only its own machine's panes, and a server
    /// that keeps one CLI per session replaces the whole roster with what its
    /// CLI reports: every tab another machine's instance had tagged would
    /// vanish from it, with no stopped badge, and a tab added meanwhile could
    /// take one of their ids.
    Alone,
    /// What was running here, started again after the daemon replaced
    /// itself, or after a replacement failed, while the server keeps one CLI
    /// per session: as [`StartScope::Alone`] where no other host runs the
    /// project, and otherwise as configured, as it ran before. Never refused
    /// for what the server runs, which never stops an instance already
    /// running ([`StartScope::again`]).
    AloneWherePossible,
}

impl StartScope {
    /// How a project that was running here starts again after the daemon
    /// replaced itself, or after a replacement failed, given whether the
    /// server the daemon was connected to keeps one CLI per session.
    ///
    /// A resume runs before the new process has heard from any server, so the
    /// process it replaced writes that answer into the resume manifest
    /// ([`ResumeProject::single_machine_server`]); a restart after a failed
    /// replacement still has its connection's. A server that runs instances
    /// gets an instance back either way: the "Reboot to update" its refusals
    /// ask for is what turns a project's only runtime, started while the
    /// server kept one CLI per session, into an instance.
    fn again(single_machine_server: bool) -> Self {
        if single_machine_server {
            Self::AloneWherePossible
        } else {
            Self::AsConfigured
        }
    }
}

/// The newest start of each project on this host, by project id; see
/// [`StartClaim`].
type ClaimStarts = Arc<Mutex<HashMap<String, u64>>>;

/// This host's claims on a project, held by the task of one start of it.
///
/// [`InstanceClaimGuard`] releases them however that task ends. But a stop
/// that gives up waiting for a task leaves it running detached, and the
/// project can be started here again meanwhile: `claim_project` finds the
/// claims still this host's, and the new task runs under the very same files.
/// Released unconditionally, the old task's eventual end deleted them from
/// under the new one, and until a heartbeat retook them a daemon on another
/// host that runs projects alone, one predating instances say, could start
/// the project beside it. So every start takes the next number for its
/// project, and a task releases the claims only while its start is still the
/// newest. Otherwise they are the newer start's, and its task releases them
/// when it ends.
struct StartClaim {
    guard: Option<InstanceClaimGuard>,
    starts: ClaimStarts,
    project_id: String,
    start: u64,
}

impl StartClaim {
    /// Leave the claims in place when this goes, as
    /// [`InstanceClaimGuard::keep`] does.
    fn keep(mut self) {
        if let Some(guard) = self.guard.take() {
            guard.keep();
        }
    }
}

impl Drop for StartClaim {
    fn drop(&mut self) {
        let Some(guard) = self.guard.take() else {
            return;
        };
        // Under the lock a start holds while it claims, so no start can take
        // the claims between this check and the release.
        let starts = self
            .starts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if starts.get(&self.project_id) == Some(&self.start) {
            drop(guard);
        } else {
            tracing::info!(
                project_id = %self.project_id,
                "the project was started here again while this task was ending; its claims \
                 are that start's"
            );
            guard.keep();
        }
    }
}

/// A project asking to be replaced.
///
/// The task cannot restart itself — it is the thing being replaced, and only
/// the instance holds the table. It says so on this channel and ends; the
/// daemon starts a fresh task for it.
type RestartRequests = tokio::sync::mpsc::UnboundedSender<String>;

/// What a started project's task runs: the project runtime, except in tests.
/// Everything around it, from claims to the handling of how it ended, is the
/// same either way.
type ProjectLauncher = Arc<
    dyn Fn(ProjectLaunch) -> BoxFuture<'static, Result<crate::mode::dual_pane::ProjectOutcome>>
        + Send
        + Sync,
>;

struct ProjectLaunch {
    server_url: String,
    token: String,
    project_path: PathBuf,
    shutdown: Arc<AtomicBool>,
    machine_id: Uuid,
    /// Whether the runtime may run as an instance, as its claims do
    /// ([`RunningProject::claims`]). False runs it as the project's only
    /// runtime whatever `config.toml` allows, as a CLI from before instances
    /// runs ([`StartScope::Alone`]).
    instances: bool,
}

fn project_runtime_launcher() -> ProjectLauncher {
    Arc::new(|launch: ProjectLaunch| {
        Box::pin(async move {
            crate::mode::dual_pane::run_project(
                &launch.server_url,
                &launch.token,
                &launch.project_path,
                launch.shutdown,
                launch.machine_id,
                launch.instances,
            )
            .await
        })
    })
}

/// How a request to start a project went.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StartOutcome {
    /// A task was started for it.
    Started,
    /// Its task was already running here.
    AlreadyRunning,
    /// Nothing was started. The reason is also the project's `last_error`.
    Refused(String),
}

/// Each registered checkout's canonical `origin`, read off the heartbeat
/// path.
///
/// `normalized_git_remote` runs `git`, which on an NFS home can stall far
/// longer than a heartbeat may. So the inventory reports whatever was last
/// read, and reads happen in the background: a slow one delays only the
/// remote, never the heartbeat.
#[derive(Clone)]
struct GitRemotes {
    entries: Arc<Mutex<HashMap<PathBuf, CachedGitRemote>>>,
    /// One read runs at a time, so a stalled filesystem ties up one blocking
    /// thread rather than one more on every heartbeat.
    reading: Arc<AtomicBool>,
    read: Arc<dyn Fn(&Path) -> Option<String> + Send + Sync>,
    max_age: Duration,
}

struct CachedGitRemote {
    remote: Option<String>,
    read_at: Instant,
}

/// Clears [`GitRemotes::reading`] however the read ends.
struct ReadingFlag(Arc<AtomicBool>);

impl Drop for ReadingFlag {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl GitRemotes {
    fn new() -> Self {
        Self::with_reader(Arc::new(|path: &Path| {
            crate::worktree::normalized_git_remote(path)
        }))
    }

    fn with_reader(read: Arc<dyn Fn(&Path) -> Option<String> + Send + Sync>) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            reading: Arc::new(AtomicBool::new(false)),
            read,
            max_age: GIT_REMOTE_MAX_AGE,
        }
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, CachedGitRemote>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The last remote read for `path`, without reading anything.
    fn get(&self, path: &Path) -> Option<String> {
        self.entries()
            .get(path)
            .and_then(|cached| cached.remote.clone())
    }

    /// Record a remote read elsewhere, by a caller already off the runtime.
    fn record(&self, path: PathBuf, remote: Option<String>) {
        self.entries().insert(
            path,
            CachedGitRemote {
                remote,
                read_at: Instant::now(),
            },
        );
    }

    /// Forget checkouts that are no longer registered, and start reading, in
    /// the background, every one whose remote is unknown or old. Returns the
    /// read it started, if any; there is none when everything is current, a
    /// read is still running, or there is no runtime to run it on.
    fn refresh(&self, paths: Vec<PathBuf>) -> Option<tokio::task::JoinHandle<()>> {
        let due: Vec<PathBuf> = {
            let mut entries = self.entries();
            let registered: HashSet<&PathBuf> = paths.iter().collect();
            entries.retain(|path, _| registered.contains(path));
            paths
                .iter()
                .filter(|path| {
                    entries
                        .get(*path)
                        .is_none_or(|cached| cached.read_at.elapsed() >= self.max_age)
                })
                .cloned()
                .collect()
        };
        if due.is_empty() {
            return None;
        }
        let runtime = tokio::runtime::Handle::try_current().ok()?;
        if self
            .reading
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return None;
        }
        let flag = ReadingFlag(self.reading.clone());
        let remotes = self.clone();
        Some(runtime.spawn_blocking(move || {
            let _flag = flag;
            for path in due {
                let remote = (remotes.read)(&path);
                remotes.record(path, remote);
            }
        }))
    }
}

struct DaemonState {
    machine_info: MachineInfo,
    projects: HashMap<String, ProjectEntry>,
    sessions: HashMap<String, String>,
    /// Projects this instance is running, keyed by project id.
    running: HashMap<String, RunningProject>,
    /// How this daemon claims what it runs, which also decides whether it
    /// advertises `project_instances_v1`.
    claim_mode: ClaimMode,
    /// Whether the server this daemon last registered with runs project
    /// instances: it echoed `project_instances_v1` in `Registered`. Decided
    /// again at every registration, since the server can be upgraded or
    /// rolled back while the daemon runs ([`DaemonState::note_registration`]).
    /// Read only for what that connection asks for.
    server_runs_instances: bool,
    /// The NFS-shared config dir holding the claims, resolved once so a
    /// project's task can release its claims from any thread.
    registry_dir: PathBuf,
    /// The start of each project that owns this host's claims on it.
    claim_starts: ClaimStarts,
    /// How long a stop waits for a project's task: [`PROJECT_STOP_WAIT`],
    /// which tests shorten.
    stop_wait: Duration,
    git_remotes: GitRemotes,
    launcher: ProjectLauncher,
}

fn should_forget_project(
    seen_in_registry: bool,
    has_running_task: bool,
    has_legacy_session: bool,
) -> bool {
    !seen_in_registry && !has_running_task && !has_legacy_session
}

impl DaemonState {
    fn new(machine_info: MachineInfo, claim_mode: ClaimMode) -> Self {
        Self {
            machine_info,
            projects: HashMap::new(),
            running: HashMap::new(),
            sessions: HashMap::new(),
            claim_mode,
            server_runs_instances: false,
            registry_dir: config_dir_for_registry(),
            claim_starts: ClaimStarts::default(),
            stop_wait: PROJECT_STOP_WAIT,
            git_remotes: GitRemotes::new(),
            launcher: project_runtime_launcher(),
        }
    }

    fn refresh_projects(&mut self) {
        let discovered = match crate::project::list_registered_projects() {
            Ok(projects) => projects,
            Err(err) => {
                tracing::warn!("Failed to read project registry: {}", err);
                // A failed read says nothing about which projects disappeared.
                // Retain the last known inventory; treating an NFS/JSON error
                // as an empty registry erased the only paths a restart had.
                return;
            }
        };
        self.apply_project_inventory(discovered);
    }

    fn apply_project_inventory(&mut self, discovered: Vec<crate::project::RegisteredProject>) {
        let mut seen = HashSet::new();

        for project in discovered {
            if project.project_id.trim().is_empty() || project.path.trim().is_empty() {
                continue;
            }
            let project_id = project.project_id.clone();
            seen.insert(project_id.clone());
            match self.projects.get_mut(&project_id) {
                Some(existing) => {
                    existing.name = project.name.clone();
                    existing.path = PathBuf::from(project.path);
                }
                None => {
                    self.projects.insert(
                        project_id.clone(),
                        ProjectEntry {
                            name: project.name,
                            path: PathBuf::from(project.path),
                            last_error: None,
                        },
                    );
                }
            }
        }

        // Remove disappeared projects only when they are not actively running.
        let stale_ids: Vec<String> = self
            .projects
            .keys()
            .filter(|project_id| {
                should_forget_project(
                    seen.contains(*project_id),
                    self.running.contains_key(*project_id),
                    self.sessions.contains_key(*project_id),
                )
            })
            .cloned()
            .collect();
        for project_id in stale_ids {
            self.projects.remove(&project_id);
        }
    }

    async fn register_local_project(
        &mut self,
        path: String,
    ) -> Result<(MachineProjectInfo, Vec<MachineProjectInfo>)> {
        #[cfg(test)]
        let config_dir = crate::config::Config::config_dir()?;
        let (registered, inventory, remote) = tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            let registration = crate::config::test_config::with_config_dir(&config_dir, || {
                crate::project::register_local_project(&path)
            });
            #[cfg(not(test))]
            let registration = crate::project::register_local_project(&path);
            // Already off the runtime, so read the checkout's remote here: the
            // reply carries it rather than waiting for a background read.
            registration.map(|(registered, inventory)| {
                let remote = crate::worktree::normalized_git_remote(Path::new(&registered.path));
                (registered, inventory, remote)
            })
        })
        .await
        .context("Local folder registration worker failed")??;
        self.apply_project_inventory(inventory);
        self.git_remotes
            .record(PathBuf::from(&registered.path), remote);
        let projects = self.snapshot_projects();
        let project = projects
            .iter()
            .find(|project| {
                project.project_id == registered.project_id && project.path == registered.path
            })
            .cloned()
            .context("Registered project is missing from machine inventory; retry")?;
        Ok((project, projects))
    }

    fn reap_exited_processes(&mut self) {
        let exited: Vec<String> = self
            .sessions
            .iter()
            .filter_map(|(project_id, session_name)| {
                if tmux_has_session(project_id, session_name) {
                    None
                } else {
                    Some(project_id.clone())
                }
            })
            .collect();

        for project_id in exited {
            self.sessions.remove(&project_id);
        }
    }

    /// Stop anything left over from the process-per-project model.
    ///
    /// This is the one part of the merge that is not additive at rollout. An
    /// older instance ran each project as `apas --headless` in its own tmux
    /// session, and `exec` never touched those, so a newer instance that
    /// simply started its own tasks would leave two owners of one `.apas` and
    /// one set of worktrees — exactly the duplication everything else here
    /// exists to prevent. The newer instance therefore stops them before it
    /// starts anything.
    ///
    /// Pane hosts are deliberately untouched: they own the PTYs, and leaving
    /// them alive is what lets the projects come back inside their adoption
    /// grace with their terminal agents intact.
    ///
    /// This finds projects an older *daemon* started, which carry `-d <path>`.
    /// It cannot find one a person started by running `apas` in a directory
    /// before that became register-and-exit: those have no arguments at all,
    /// and nothing distinguishes them from an `apas --attach` someone is
    /// using. Killing by working directory would take attachments with it, so
    /// those stay a deliberate manual step in the cutover.
    fn retire_process_per_project_leftovers(&mut self) {
        for (project_id, project) in &self.projects {
            let session_name = tmux_session_name(project_id);
            let had_session = tmux_has_session(project_id, &session_name);
            let pids = headless_pids_for(&project.path);
            if !had_session && pids.is_empty() {
                continue;
            }
            tracing::info!(
                project_id,
                had_session,
                processes = pids.len(),
                "stopping a project left by the process-per-project model"
            );
            if had_session {
                if let Err(err) = tmux_kill_session(project_id, &session_name) {
                    tracing::warn!(project_id, %err, "could not kill the old tmux session");
                }
            }
            for pid in pids {
                let _ = Command::new("kill").arg(pid.to_string()).status();
            }
        }
        self.sessions.clear();
    }

    /// Projects this instance is currently running.
    fn running_project_ids(&self) -> Vec<String> {
        self.running
            .iter()
            .filter(|(_, project)| !project.handle.is_finished())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Record both identity and path so a replacement does not need the
    /// shared registry in order to find what this exact process was running,
    /// and whether this daemon's server keeps one CLI per session, which the
    /// replacement cannot ask before it resumes them.
    fn running_projects_for_resume(&self) -> Vec<ResumeProject> {
        self.running
            .iter()
            .filter(|(_, running)| !running.handle.is_finished())
            .filter_map(|(project_id, _)| {
                self.projects
                    .get(project_id)
                    .map(|project| self.resume_entry(project_id, project))
            })
            .collect()
    }

    fn resume_entry(&self, project_id: &str, project: &ProjectEntry) -> ResumeProject {
        ResumeProject {
            project_id: project_id.to_string(),
            path: Some(project.path.clone()),
            // Every replacement is asked for over a connection that has
            // registered, so this is that server's answer.
            single_machine_server: self.requested_start_scope() == StartScope::Alone,
        }
    }

    /// The resume list for a restart that one project asked for, which must
    /// name that project even though its task has already ended.
    ///
    /// `running_projects_for_resume` deliberately skips finished tasks so a
    /// project that genuinely stopped is not resurrected. A project requests a
    /// CLI replacement by ending its own task, so taken literally that rule
    /// drops exactly the project the user acted on: they press "Reboot CLI"
    /// and it is the one thing that does not come back.
    fn resume_projects_after_requested_restart(&self, project_id: &str) -> Vec<ResumeProject> {
        let mut resume = self.running_projects_for_resume();
        if !resume.iter().any(|entry| entry.project_id == project_id) {
            if let Some(project) = self.projects.get(project_id) {
                resume.push(self.resume_entry(project_id, project));
            }
        }
        resume
    }

    /// Rehydrate a project path from the handoff manifest when the shared
    /// registry is unavailable or has lost the entry. The `.apas` file is the
    /// project-local authority, so require its id to match before adopting it.
    fn restore_project_from_resume(&mut self, project: &ResumeProject) -> Result<()> {
        if self.projects.contains_key(&project.project_id) {
            return Ok(());
        }
        let path = project.path.as_ref().with_context(|| {
            format!(
                "project {} is absent from the registry and its legacy resume record has no path",
                project.project_id
            )
        })?;
        let metadata_path = crate::project::get_apas_path(path);
        let raw = std::fs::read_to_string(&metadata_path)
            .with_context(|| format!("read resume metadata {}", metadata_path.display()))?;
        let mut metadata: crate::project::ProjectMetadata = serde_json::from_str(&raw)
            .with_context(|| format!("parse resume metadata {}", metadata_path.display()))?;
        metadata.migrate_legacy();
        if metadata.id.to_string() != project.project_id {
            anyhow::bail!(
                "resume metadata {} belongs to {}, expected {}",
                metadata_path.display(),
                metadata.id,
                project.project_id
            );
        }

        // Repair the discovery index when possible, but do not make a healthy
        // project depend on that shared write: the in-memory entry is enough
        // to resume it and later heartbeats can retry registration.
        if let Err(err) = crate::project::register_project(path, &metadata) {
            tracing::warn!(
                project_id = %project.project_id,
                path = %path.display(),
                %err,
                "could not repair project registry from resume metadata"
            );
        }
        self.projects.insert(
            project.project_id.clone(),
            ProjectEntry {
                name: metadata.name,
                path: path.clone(),
                last_error: None,
            },
        );
        Ok(())
    }

    /// Bring this host's claims in line with what it runs: keep them alive
    /// for every instance running here, retake any that went missing, and let
    /// every other claim of ours lapse. Run at startup once resumed projects
    /// have started, and on every heartbeat.
    ///
    /// Claims follow this daemon's own tasks. A daemon replaced by `exec`
    /// leaves the claims of what it was running in place, since no destructor
    /// runs and the pid stays the same; the replacement takes each back as it
    /// resumes the project, and whatever it does not resume lapses within
    /// `STALE_AFTER_SECS`. An external `apas --headless -d <path>` run is not
    /// claimed: a standalone run takes no claim, and the server decides
    /// whether such a runtime may attach beside another machine's.
    ///
    /// A peer holding the guard exclusively for something running here is
    /// logged rather than seized. That combination means two runtimes are
    /// live where only one may be, which is the thing claims exist to prevent
    /// — stealing the claim would hide it, and the operator needs to see it.
    ///
    /// Each project's claims keep the shape its start took
    /// ([`RunningProject::claims`]): refreshed in the daemon's own shape, a
    /// project's only runtime, started while the server kept one CLI per
    /// session, would mark the guard again and let a capable daemon on another
    /// host start an instance beside it.
    fn reconcile_running_claims(&self) {
        for claims in [ClaimMode::Instances, ClaimMode::Exclusive] {
            let running: Vec<(String, String)> = self
                .running
                .iter()
                .filter(|(_, project)| !project.handle.is_finished() && project.claims == claims)
                .filter_map(|(project_id, _)| {
                    self.projects
                        .get(project_id)
                        .map(|project| (project_id.clone(), project.path.display().to_string()))
                })
                .collect();
            crate::daemon_registry::refresh_own_claims(
                &self.registry_dir,
                self.machine_info.machine_id,
                claims,
                &running,
            );
        }
    }

    /// The heartbeat's part in the shared registry: refresh this host's
    /// record, keep the claims of what runs here alive and let the rest lapse.
    /// If these age past `STALE_AFTER_SECS`, peers conclude this host died or
    /// that its instances stopped.
    fn keep_claims_fresh(&self) {
        let _ = crate::daemon_registry::publish_self(
            &self.registry_dir,
            self.machine_info.machine_id,
            VERSION,
        );
        self.reconcile_running_claims();
    }

    /// Run blocking `work` on a blocking thread and wait for it, keeping the
    /// claims fresh on every `heartbeat` tick meanwhile.
    ///
    /// The message loop is what refreshes them. A clone run inline on it left
    /// them unrefreshed for as long as the clone took, and past
    /// `STALE_AFTER_SECS` peers took this host's instances for stopped: a
    /// daemon that runs projects alone, one predating instances say, could
    /// then start a project running here, a second runtime on the same `.apas`
    /// and worktrees. The loop still waits for the work, so nothing else it
    /// handles runs meanwhile, as before; only the claims keep going.
    async fn wait_off_loop<T: Send + 'static>(
        &self,
        heartbeat: &mut tokio::time::Interval,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T> {
        // Tests redirect the config dir per thread, and the work registers
        // projects there; carry it onto the worker, as registration does.
        #[cfg(test)]
        let config_dir = crate::config::Config::config_dir()?;
        let mut work = tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            let done = crate::config::test_config::with_config_dir(&config_dir, work);
            #[cfg(not(test))]
            let done = work();
            done
        });
        loop {
            tokio::select! {
                done = &mut work => return done.context("a blocking worker failed"),
                _ = heartbeat.tick() => self.keep_claims_fresh(),
            }
        }
    }

    /// Record what the server answered this daemon's registration with.
    ///
    /// A server that does not echo `project_instances_v1` keeps one CLI per
    /// session: a second machine's `SessionStart` replaces the first machine's
    /// instance there, and that instance's tabs can no longer be controlled.
    /// So until a registration says otherwise, a start asked for over this
    /// connection runs only where no other host runs the project, and then as
    /// the project's only runtime, as daemons ran projects before instances
    /// ([`DaemonState::requested_start_scope`]). Nothing already running here
    /// is stopped or changed because of it: each project keeps running as it
    /// started, and its claims keep their shape.
    fn note_registration(&mut self, negotiated_capabilities: &[String]) {
        self.server_runs_instances = negotiated_capabilities
            .iter()
            .any(|capability| capability == shared::PROJECT_INSTANCES_CAPABILITY);
        if self.claim_mode == ClaimMode::Instances && !self.server_runs_instances {
            tracing::info!(
                "the server does not run project instances, so this host starts a project only \
                 where no other machine runs it, as its only runtime, until the server does; \
                 projects running here keep running"
            );
        }
    }

    /// How a start that the current server connection asks for may run:
    /// beside other machines' instances, as an instance, only when this
    /// daemon can run them and the server echoed that it runs them too.
    /// Otherwise a capable daemon starts it alone, as the project's only
    /// runtime ([`StartScope::Alone`]).
    fn requested_start_scope(&self) -> StartScope {
        if self.claim_mode == ClaimMode::Instances && !self.server_runs_instances {
            StartScope::Alone
        } else {
            StartScope::AsConfigured
        }
    }

    /// Start a background read of every checkout whose remote is unknown or
    /// old; the inventory reports what is known meanwhile.
    fn refresh_git_remotes(&self) -> Option<tokio::task::JoinHandle<()>> {
        self.git_remotes.refresh(
            self.projects
                .values()
                .map(|project| project.path.clone())
                .collect(),
        )
    }

    fn snapshot_projects(&self) -> Vec<MachineProjectInfo> {
        let mut projects = Vec::with_capacity(self.projects.len());

        for (project_id, project) in &self.projects {
            // Held, not inferred. A project runs inside this process, so the
            // task table is the answer; `/proc` is consulted only to notice an
            // externally started `--headless` run, which is the debugging
            // escape hatch rather than the normal path.
            let task = self.running.get(project_id);
            let running_here = task.is_some_and(|project| !project.handle.is_finished());
            let external = headless_pid_for(&project.path);
            let pid = if running_here {
                Some(std::process::id())
            } else {
                external
            };
            // Memory is the whole instance once projects share it, so
            // attributing it to one project would be a lie. Report it only for
            // an external single-project run, where it still means something.
            let memory_kb = if running_here {
                None
            } else {
                external.and_then(read_process_rss_kb)
            };
            projects.push(MachineProjectInfo {
                project_id: project_id.clone(),
                name: project.name.clone(),
                path: project.path.to_string_lossy().to_string(),
                preferred_machine_id: None,
                is_running: running_here || external.is_some(),
                pid,
                memory_kb,
                // `last_error` holds a refusal recorded since this task
                // started, which clears it, so it is reported first. Why the
                // task ended goes with the task, when the project is started
                // again or stopped.
                last_error: project
                    .last_error
                    .clone()
                    .or_else(|| task.and_then(RunningProject::ended_because)),
                git_remote: self.git_remotes.get(&project.path),
                open_state: None,
                part: None,
                part_reason: None,
                other_hosts: Vec::new(),
            });
        }

        projects.sort_by(|a, b| a.path.cmp(&b.path));
        projects
    }

    /// Start a project in this instance, or say why not.
    ///
    /// Every refusal becomes the project's `last_error`, so the Machines page
    /// shows why a start did nothing; refusals used to return silently. `Err`
    /// is left for a project this daemon does not know.
    ///
    /// `scope` says whether the project may run beside other machines'
    /// instances, and so whether its runtime starts as an instance or as the
    /// project's only runtime, with claims of the matching shape
    /// ([`DaemonState::claim_for_start`]). A project already running here is
    /// never refused, and keeps running as it started.
    fn start_project(
        &mut self,
        project_id: &str,
        server_url: &str,
        token: &str,
        restart_tx: &RestartRequests,
        scope: StartScope,
    ) -> Result<StartOutcome> {
        // Reap any exited tracked processes before deciding whether to spawn.
        self.reap_exited_processes();

        // Already running here: the table answers this, not a tmux session we
        // would have to go and look for.
        if self
            .running
            .get(project_id)
            .is_some_and(|project| !project.handle.is_finished())
        {
            return Ok(StartOutcome::AlreadyRunning);
        }
        // A task that ended leaves an entry behind; drop it so the project can
        // be started again without manual cleanup.
        self.running.remove(project_id);

        let project_path = self
            .projects
            .get(project_id)
            .map(|project| project.path.clone())
            .ok_or_else(|| anyhow::anyhow!("Unknown project id: {}", project_id))?;

        // An externally started `--headless -d <path>` run — the standalone
        // debugging path, or a project left over from the process-per-project
        // model. It is not ours to supervise, but starting a second owner for
        // the same `.apas` and worktrees is exactly what must not happen.
        // Checked before claiming, so this refusal leaves no claim behind.
        if is_headless_running_for(&project_path) {
            let reason = "an `apas --headless` run started outside the daemon already runs \
                          this project on this host";
            return Ok(StartOutcome::Refused(
                self.record_start_refusal(project_id, reason.to_string()),
            ));
        }

        // Cross-host guard. `is_headless_running_for` above only reads the
        // local /proc, so on a shared-NFS cluster it cannot see a runtime
        // another host started for this same project. The claim files are how
        // peers tell each other who runs what: another machine's instance does
        // not stop a capable daemon from starting one of its own, but a holder
        // that runs the project alone does.
        //
        // Written before the task starts, so the instance deciding whether to
        // adopt untagged panes already sees it, and so does a peer starting
        // the same project at the same moment: the earlier of the two claims
        // is the one that adopts.
        //
        // Claimed under the project's start count, which this start then
        // takes, so a task of an earlier start that is still ending cannot
        // release the claims between the two; see `StartClaim`.
        //
        // A start that must run alone refuses while any other host runs the
        // project, however it claimed it: the server keeps one CLI per
        // session, and a second machine's would replace the first's there.
        let machine_id = self.machine_info.machine_id;
        let claim_starts = self.claim_starts.clone();
        let mut starts = claim_starts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (claimed, claims) = self.claim_for_start(project_id, &project_path, scope);
        match claimed {
            Ok(ClaimOutcome::HeldBy(refusal)) => {
                drop(starts);
                return Ok(StartOutcome::Refused(
                    self.record_start_refusal(project_id, refusal.reason()),
                ));
            }
            Ok(_) => {}
            Err(err) => {
                // A claim we cannot write is not worth failing the spawn over;
                // the heartbeat retries it for as long as the project runs.
                tracing::warn!(project_id, %err, "could not record project claim");
            }
        }
        let start = {
            let newest = starts.entry(project_id.to_string()).or_insert(0);
            *newest += 1;
            *newest
        };
        drop(starts);
        // The runtime owns the project's panes as its claims say: an instance
        // only its own machine's, the project's only runtime every one.
        let instances = claims == ClaimMode::Instances;
        if instances {
            // Running beside other machines is the point of instances, but it
            // is also what an operator chasing a duplicate needs to see.
            let others = crate::daemon_registry::live_instance_machines(
                &self.registry_dir,
                project_id,
                machine_id,
            );
            if others.any() {
                tracing::info!(
                    project_id,
                    machines = ?others
                        .machines
                        .iter()
                        .chain(others.exclusive_legacy.iter())
                        .map(|holder| holder.hostname.as_str())
                        .collect::<Vec<_>>(),
                    "starting another instance of a project that other machines run"
                );
            }
        } else if self.claim_mode == ClaimMode::Instances {
            // Why a host that runs instances is suddenly running tabs that
            // other machines ran, and reporting them as its own.
            tracing::info!(
                project_id,
                "starting this project as its only runtime, because the server keeps one CLI per \
                 session: it runs every pane in .apas, other machines' included, and saves them \
                 untagged, as before instances"
            );
        }
        // Released when the task ends, however it ends, except when it asks
        // for the daemon to be replaced or the project was started here again
        // meanwhile.
        let claim = StartClaim {
            guard: Some(InstanceClaimGuard::new(
                self.registry_dir.clone(),
                project_id.to_string(),
                machine_id,
            )),
            starts: self.claim_starts.clone(),
            project_id: project_id.to_string(),
            start,
        };

        // The project runs here, as a task of this instance. It used to be a
        // headless CLI in its own tmux session, which this daemon then could
        // not see — running state was inferred from /proc, and a restarted
        // daemon came back owning nothing.
        //
        // Blocking work inside a project stays on its own threads. Nothing
        // here may block the runtime, or one project would stall the others.
        let project_shutdown = Arc::new(AtomicBool::new(false));
        let launch = (self.launcher)(ProjectLaunch {
            server_url: server_url.to_string(),
            token: token.to_string(),
            project_path,
            shutdown: project_shutdown.clone(),
            machine_id,
            instances,
        });
        let id_for_task = project_id.to_string();
        let restart_tx = restart_tx.clone();
        // Why the task ended, for the inventory to report once it has; see
        // `RunningProject::ended_because`.
        let ended_because: Arc<Mutex<Option<String>>> = Arc::default();
        let record_end = {
            let ended_because = ended_because.clone();
            move |reason: String| {
                *ended_because
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(reason);
            }
        };
        let handle = tokio::spawn(async move {
            // Every record this project emits carries its identity: sharing a
            // process took away the per-project tmux session and log file that
            // used to separate them for free.
            let span = tracing::info_span!("project", id = %id_for_task);
            let _entered = span.enter();
            match launch.await {
                Ok(crate::mode::dual_pane::ProjectOutcome::Completed) => {
                    tracing::info!("project stopped");
                }
                Ok(crate::mode::dual_pane::ProjectOutcome::Stopped(reason)) => {
                    // One project's fatal condition, contained: it used to end
                    // the process, which would now end every other project.
                    tracing::error!(%reason, "project stopped");
                    record_end(reason);
                }
                Ok(crate::mode::dual_pane::ProjectOutcome::RebootRequested) => {
                    tracing::info!("project asked to restart");
                    // The daemon replaces itself and resumes this project,
                    // taking its claims back under the same pid. Releasing
                    // them first would show peers an unclaimed project for
                    // the length of the update.
                    claim.keep();
                    // Replacing one project no longer replaces the process,
                    // which is what a reboot used to mean when a process was
                    // a project.
                    let _ = restart_tx.send(id_for_task.clone());
                }
                Err(err) => {
                    tracing::error!(%err, "project ended with an error");
                    record_end(format!("the project stopped with an error: {err:#}"));
                }
            }
        });

        if let Some(project) = self.projects.get_mut(project_id) {
            project.last_error = None;
        }
        self.running.insert(
            project_id.to_string(),
            RunningProject {
                shutdown: project_shutdown,
                handle,
                ended_because,
                claims,
            },
        );
        tracing::info!(project_id, "started project in this instance");
        Ok(StartOutcome::Started)
    }

    /// Claim `project_id` for a start in `scope`. Returns the outcome and the
    /// shape the claims take, which is also how the runtime owns the
    /// project's panes ([`RunningProject::claims`]): this daemon's
    /// [`ClaimMode`] as configured, and [`ClaimMode::Exclusive`] for a start
    /// that runs alone because the server keeps one CLI per session.
    fn claim_for_start(
        &self,
        project_id: &str,
        project_path: &Path,
        scope: StartScope,
    ) -> (Result<ClaimOutcome>, ClaimMode) {
        let project_path = project_path.display().to_string();
        let machine_id = self.machine_info.machine_id;
        let as_configured = || {
            crate::daemon_registry::claim_project(
                &self.registry_dir,
                project_id,
                &project_path,
                machine_id,
                self.claim_mode,
            )
        };
        let alone = || {
            crate::daemon_registry::claim_project_alone(
                &self.registry_dir,
                project_id,
                &project_path,
                machine_id,
                self.claim_mode,
            )
        };
        match scope {
            StartScope::AsConfigured => (as_configured(), self.claim_mode),
            StartScope::Alone => (alone(), ClaimMode::Exclusive),
            StartScope::AloneWherePossible => match alone() {
                Ok(ClaimOutcome::HeldBy(ClaimRefusal::SingleMachineServer(holder))) => {
                    // Then it ran here as an instance beside that host's,
                    // started while the server ran instances. Run alone, it
                    // would run that host's panes as well.
                    tracing::info!(
                        project_id,
                        other_host = %holder.hostname,
                        "another host runs this project, so it starts here again as an instance, \
                         as it ran before"
                    );
                    (as_configured(), self.claim_mode)
                }
                claimed => (claimed, ClaimMode::Exclusive),
            },
        }
    }

    /// Record why a project was not started, as its `last_error`, and return
    /// that reason for the caller to report.
    fn record_start_refusal(&mut self, project_id: &str, reason: String) -> String {
        tracing::warn!(project_id, %reason, "not starting project");
        if let Some(project) = self.projects.get_mut(project_id) {
            project.last_error = Some(reason.clone());
        }
        reason
    }

    /// Answer a `StartProjectCli`. Returns what to send back, in order: the
    /// inventory, then, when the request carried an id, exactly one
    /// `ProjectRuntimeStarted`, whatever happened. The inventory goes first so
    /// the server already holds this start's running state and `last_error`
    /// when the outcome arrives.
    ///
    /// `success` means the project now runs in this daemon, including when it
    /// already did. It is the start, not the attachment: the server still
    /// waits for the instance's `SessionStart`.
    ///
    /// The start runs beside other machines' instances only when the server
    /// said it runs them too, and otherwise as the project's only runtime
    /// ([`DaemonState::requested_start_scope`]).
    fn handle_start_request(
        &mut self,
        project_id: String,
        policy: Option<EffectiveProjectPolicy>,
        request_id: Option<Uuid>,
        server_url: &str,
        token: &str,
        restart_tx: &RestartRequests,
    ) -> Vec<DaemonToServer> {
        let result = match policy {
            // Fails closed, so a mixed-version rollout cannot revive a
            // suspended project.
            None => Err(self.record_start_refusal(
                &project_id,
                "the start request carried no cluster policy from the server".to_string(),
            )),
            Some(policy) if policy.project_suspended => Err(self.record_start_refusal(
                &project_id,
                format!(
                    "the project is suspended (policy version {})",
                    policy.version
                ),
            )),
            Some(_) => match self.start_project(
                &project_id,
                server_url,
                token,
                restart_tx,
                self.requested_start_scope(),
            ) {
                Ok(StartOutcome::Started | StartOutcome::AlreadyRunning) => Ok(()),
                Ok(StartOutcome::Refused(reason)) => Err(reason),
                Err(err) => {
                    tracing::warn!("Failed to start project {}: {}", project_id, err);
                    Err(err.to_string())
                }
            },
        };
        let mut replies = vec![DaemonToServer::Heartbeat {
            projects: self.snapshot_projects(),
        }];
        if let Some(request_id) = request_id {
            let (success, error) = match result {
                Ok(()) => (true, None),
                Err(reason) => (false, Some(reason)),
            };
            replies.push(DaemonToServer::ProjectRuntimeStarted {
                request_id,
                project_id,
                success,
                error,
            });
        }
        replies
    }

    /// Stop a project and wait for it to finish tearing down. Returns false
    /// only when its task was still running when the wait ran out.
    ///
    /// Bounded, because teardown kills pane children and joins roughly thirty
    /// threads: a project that will not stop must not hold the instance that
    /// every other project is running in. A task given up on keeps running
    /// detached until it ends.
    async fn stop_running_project(&mut self, project_id: &str) -> bool {
        let Some(project) = self.running.remove(project_id) else {
            return true;
        };
        project.shutdown.store(true, Ordering::SeqCst);
        match tokio::time::timeout(self.stop_wait, project.handle).await {
            Ok(Ok(())) => {
                tracing::info!(project_id, "project stopped");
                true
            }
            Ok(Err(err)) => {
                // A panicking project unwinds its own task. Reported, and the
                // instance and every other project carry on.
                tracing::error!(project_id, %err, "project ended abnormally");
                true
            }
            Err(_) => {
                tracing::warn!(
                    project_id,
                    wait = ?self.stop_wait,
                    "project did not stop in time; abandoning the wait"
                );
                false
            }
        }
    }

    async fn stop_project(&mut self, project_id: &str) -> Result<()> {
        // Stop the task first: it owns the panes, and letting it tear down
        // before the pane hosts are shut means it can do so cleanly.
        if self.stop_running_project(project_id).await {
            // Its task released its claims as it ended. Releasing here too
            // covers claims of ours with no task behind them, such as those of
            // a project a replaced daemon ran and this one did not resume. A
            // task still tearing down keeps its claims until it ends; nothing
            // refreshes them meanwhile, so they lapse if it never does. If the
            // project is started here again first, they are that start's, and
            // the old task's end leaves them alone (`StartClaim`).
            crate::daemon_registry::release_project(
                &self.registry_dir,
                project_id,
                self.machine_info.machine_id,
            );
        }
        if let Ok(project_uuid) = Uuid::parse_str(project_id) {
            let stopped = crate::pane_host::shutdown_project_hosts(project_uuid)?;
            if stopped > 0 {
                tracing::info!(project_id, stopped, "stopped persistent pane hosts");
            }
        }
        let session_name = self
            .sessions
            .remove(project_id)
            .unwrap_or_else(|| tmux_session_name(project_id));
        tmux_kill_session(project_id, &session_name)?;

        if let Some(project) = self.projects.get(project_id) {
            for pid in headless_pids_for(&project.path) {
                let _ = Command::new("kill").arg(pid.to_string()).status();
            }
        }
        Ok(())
    }

    /// Clone a repo into a fresh instance directory under the projects root
    /// (default `~/apas_projects`, auto-suffixed on collision), check out a new
    /// branch, register a `.apas`, and start the headless CLI. Returns the new
    /// (project_id, absolute path).
    fn create_instance(
        &mut self,
        restart_tx: &RestartRequests,
        git_remote: &str,
        instance_name: &str,
        branch: &str,
        clone_url: Option<&str>,
        base_path: Option<&str>,
        server_url: &str,
        token: &str,
        request_id: Option<&str>,
        reserved_project_id: Option<&str>,
        provisioning_mode: ProjectProvisioningMode,
    ) -> Result<(String, String)> {
        // Projects root: an explicit base_path (with leading ~ expanded against
        // THIS machine's $HOME), else ~/apas_projects.
        let managed_root = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("could not resolve home directory"))?
            .join("apas_projects");
        let root = if provisioning_mode == ProjectProvisioningMode::PublicGithub {
            managed_root
        } else {
            match base_path {
                Some(p) if !p.trim().is_empty() => expand_tilde(p.trim()),
                _ => managed_root,
            }
        };

        if provisioning_mode == ProjectProvisioningMode::PublicGithub {
            let request_id = request_id.context("shared provisioning request id is required")?;
            let project_id = reserved_project_id.context("reserved project id is required")?;
            Uuid::parse_str(project_id).context("reserved project id is invalid")?;
            if let Some(existing) = find_provisioning_receipt(&root, request_id, project_id)? {
                return Ok((
                    project_id.to_string(),
                    existing.to_string_lossy().to_string(),
                ));
            }
        }
        let name = crate::worktree::sanitize_instance_name(instance_name);
        let dest = crate::worktree::unique_dir(&root.join(&name));

        // Make sure the projects root exists before cloning into it.
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let url = if provisioning_mode == ProjectProvisioningMode::PublicGithub {
            let submitted = clone_url.context("public GitHub clone URL is required")?;
            shared::parse_public_github_repository(submitted)?.clone_url()
        } else {
            self.resolve_clone_url(git_remote, clone_url)
        };

        // Clone + branch + register. If any of these PRE-registration steps
        // fail, remove the partial clone so retries don't accumulate orphan
        // directories (unique_dir would otherwise auto-suffix forever).
        let safe_branch = crate::worktree::sanitize_branch_name(branch);
        let metadata = match (|| {
            if provisioning_mode == ProjectProvisioningMode::PublicGithub {
                crate::worktree::clone_public_repo(&url, &dest)?;
            } else {
                crate::worktree::clone_repo(&url, &dest)?;
            }
            let created_branch = crate::worktree::checkout_unique_branch(&dest, &safe_branch)?;
            tracing::info!(
                "Cloned {} into {} on branch {}",
                url,
                dest.display(),
                created_branch
            );
            if provisioning_mode == ProjectProvisioningMode::PublicGithub {
                let request_id = request_id.expect("validated shared request id");
                let project_id =
                    Uuid::parse_str(reserved_project_id.expect("validated shared project id"))?;
                let receipt = ProvisioningReceipt {
                    request_id: request_id.to_string(),
                    project_id: project_id.to_string(),
                };
                std::fs::write(
                    dest.join(PROVISIONING_RECEIPT),
                    serde_json::to_vec_pretty(&receipt)?,
                )?;
                let metadata = crate::project::ProjectMetadata::with_id_and_name(
                    project_id,
                    dest.file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_string),
                );
                crate::project::save_project(&dest, &metadata)?;
                Ok(metadata)
            } else {
                crate::project::get_or_create_project(&dest)
            }
        })() {
            Ok(metadata) => metadata,
            Err(err) => {
                let _ = std::fs::remove_dir_all(&dest);
                return Err(err);
            }
        };

        // The instance now exists and is registered. Shared provisioning is
        // deliberately split: the server first commits project ownership and
        // placement, then sends StartProjectCli as a separate command.
        // create failure — surface it as a warning (the project shows on the
        // machines page with last_error and can be started there) so the user
        // doesn't get a "failed" toast for an instance that was created.
        let project_id = metadata.id.to_string();
        self.refresh_projects();
        // This thread just cloned the checkout, so one more local `git` read
        // costs little, and the next inventory then files the new checkout
        // under its repository instead of waiting for a background read.
        if let Some(path) = self
            .projects
            .get(&project_id)
            .map(|entry| entry.path.clone())
        {
            let remote = crate::worktree::normalized_git_remote(&path);
            self.git_remotes.record(path, remote);
        }
        if provisioning_mode == ProjectProvisioningMode::TrustedOwner {
            // A start this connection asked for, like a `StartProjectCli`: a
            // checkout committing a `.apas` takes that project's id, which may
            // run elsewhere.
            match self.start_project(
                &project_id,
                server_url,
                token,
                restart_tx,
                self.requested_start_scope(),
            ) {
                Ok(StartOutcome::Started | StartOutcome::AlreadyRunning) => {}
                Ok(StartOutcome::Refused(reason)) => tracing::warn!(
                    "Created instance {} but did not auto-start it: {}",
                    project_id,
                    reason
                ),
                Err(err) => tracing::warn!(
                    "Created instance {} but failed to auto-start it: {}",
                    project_id,
                    err
                ),
            }
        }

        Ok((project_id, dest.to_string_lossy().to_string()))
    }

    /// Answer a `CreateProjectInstance` that joins an existing project
    /// (`join_existing`). Returns what to send back, in order: the inventory,
    /// then the acknowledgement, whatever the outcome.
    ///
    /// On success the web goes on to add the checkout's first tab, with an
    /// `AddPane` naming this machine, which the server resolves through its
    /// inventory of this machine. It handles one daemon's messages in order,
    /// so with the inventory first it already lists the checkout when the
    /// acknowledgement reaches the web. The other way round, that request
    /// would race the inventory refresh.
    ///
    /// The clone runs off the message loop, which keeps the claims fresh on
    /// `heartbeat` meanwhile ([`DaemonState::wait_off_loop`]).
    #[allow(clippy::too_many_arguments)]
    async fn handle_join_instance(
        &mut self,
        git_remote: &str,
        instance_name: &str,
        clone_url: Option<&str>,
        base_path: Option<&str>,
        request_id: Option<String>,
        project_id: Option<&str>,
        provisioning_mode: ProjectProvisioningMode,
        heartbeat: &mut tokio::time::Interval,
    ) -> Vec<DaemonToServer> {
        let ack = match self
            .join_instance(
                git_remote,
                instance_name,
                clone_url,
                base_path,
                project_id,
                provisioning_mode,
                heartbeat,
            )
            .await
        {
            Ok((project_id, path)) => {
                tracing::info!(
                    "Joined project {} with a new checkout at {}",
                    project_id,
                    path
                );
                DaemonToServer::ProjectInstanceCreated {
                    request_id,
                    project_id: Some(project_id),
                    path: Some(path),
                    error: None,
                }
            }
            Err(err) => {
                tracing::warn!("Joining a project with a new checkout failed: {:#}", err);
                DaemonToServer::ProjectInstanceCreated {
                    request_id,
                    project_id: None,
                    path: None,
                    error: Some(format!("{err:#}")),
                }
            }
        };
        vec![
            DaemonToServer::Heartbeat {
                projects: self.snapshot_projects(),
            },
            ack,
        ]
    }

    /// Clone a repository onto this machine as another checkout of an
    /// existing project: the checkout takes that project's id with no panes,
    /// and is registered but not started. Its first tab, added from the web,
    /// starts it. Returns (project_id, absolute path).
    ///
    /// Unlike a new instance it creates no branch, since it joins the
    /// project's work rather than starting a line of its own, and it is only
    /// for the owner's own machines. A clone that fails before it is
    /// registered is removed, as a new instance's is.
    #[allow(clippy::too_many_arguments)]
    async fn join_instance(
        &mut self,
        git_remote: &str,
        instance_name: &str,
        clone_url: Option<&str>,
        base_path: Option<&str>,
        project_id: Option<&str>,
        provisioning_mode: ProjectProvisioningMode,
        heartbeat: &mut tokio::time::Interval,
    ) -> Result<(String, String)> {
        // The server sends a join only on the owner path, and only to a daemon
        // advertising instances; shared-cluster provisioning never joins.
        anyhow::ensure!(
            provisioning_mode == ProjectProvisioningMode::TrustedOwner,
            "joining an existing project is only for the owner's own machines"
        );
        anyhow::ensure!(
            self.claim_mode == ClaimMode::Instances,
            "this machine runs each project alone, because config.toml sets daemon.machine_id, \
             so it cannot run a project beside another machine"
        );
        // A join exists to run the project here beside its other machines,
        // which a server keeping one CLI per session cannot do. Such a server
        // predates joins and never asks for one; this keeps it that way.
        anyhow::ensure!(
            self.server_runs_instances,
            "the APAS server this host is connected to cannot run a project on more than one \
             machine at a time, so a checkout here cannot join a project that runs elsewhere"
        );
        let project_uuid = project_id
            .context("the join names no project")
            .and_then(|id| Uuid::parse_str(id).context("the project to join is not a valid id"))?;
        let project_id = project_uuid.to_string();
        // A registry holds one folder per project, and on a home shared
        // between machines that checkout is already in reach from here.
        self.refresh_projects();
        if let Some(existing) = self.projects.get(&project_id) {
            anyhow::bail!(
                "project {} is already registered on this machine at {}; start it there instead",
                project_id,
                existing.path.display()
            );
        }

        let root = match base_path {
            Some(p) if !p.trim().is_empty() => expand_tilde(p.trim()),
            _ => dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("could not resolve home directory"))?
                .join("apas_projects"),
        };
        let name = crate::worktree::sanitize_instance_name(instance_name);
        let dest = crate::worktree::unique_dir(&root.join(&name));

        // Everything that blocks, from finding a URL to clone (`git` in every
        // registered checkout) to registering the clone, runs off the loop.
        // Nothing else runs on the loop meanwhile, so no other join can pick
        // the same folder.
        let checkouts: Vec<PathBuf> = self
            .projects
            .values()
            .map(|project| project.path.clone())
            .collect();
        let git_remote = git_remote.to_string();
        let clone_url = clone_url.map(str::to_string);
        let joined = self
            .wait_off_loop(heartbeat, move || {
                clone_to_join(
                    project_uuid,
                    &dest,
                    &git_remote,
                    clone_url.as_deref(),
                    &checkouts,
                )
            })
            .await??;
        self.apply_project_inventory(joined.inventory);
        self.git_remotes
            .record(PathBuf::from(&joined.registered.path), joined.remote);
        Ok((joined.registered.project_id, joined.registered.path))
    }

    fn discard_project_provisioning(&mut self, request_id: &str, project_id: &str) -> Result<bool> {
        let root = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("could not resolve home directory"))?
            .join("apas_projects");
        let Some(path) = find_provisioning_receipt(&root, request_id, project_id)? else {
            return Ok(false);
        };
        let parsed_project_id = Uuid::parse_str(project_id)?;
        anyhow::ensure!(
            crate::project::read_project_id(&path) == Some(parsed_project_id),
            "provisioning metadata does not match"
        );
        let _ = crate::project::unregister_project(&path, parsed_project_id)?;
        std::fs::remove_dir_all(&path)?;
        self.refresh_projects();
        Ok(true)
    }

    /// Resolve a cloneable URL for the canonical `git_remote` (host/owner/repo),
    /// among this machine's registered checkouts; see [`clone_url_for`].
    fn resolve_clone_url(&self, git_remote: &str, clone_url: Option<&str>) -> String {
        clone_url_for(
            git_remote,
            clone_url,
            self.projects.values().map(|project| project.path.as_path()),
        )
    }
}

/// Resolve a cloneable URL for the canonical `git_remote` (host/owner/repo):
/// honor an explicit `clone_url`, else reuse the exact `origin` of an
/// existing checkout of the same repo among `checkouts` (preserves SSH/auth),
/// else reconstruct an https URL from the key. Runs `git` in each checkout
/// until one matches.
fn clone_url_for<'a>(
    git_remote: &str,
    clone_url: Option<&str>,
    checkouts: impl IntoIterator<Item = &'a Path>,
) -> String {
    if let Some(u) = clone_url {
        let u = u.trim();
        if !u.is_empty() {
            return u.to_string();
        }
    }
    for checkout in checkouts {
        if crate::worktree::normalized_git_remote(checkout).as_deref() == Some(git_remote) {
            if let Some(raw) = crate::worktree::raw_git_remote(checkout) {
                return raw;
            }
        }
    }
    format!("https://{}.git", git_remote.trim_matches('/'))
}

/// A checkout cloned and registered to join a project, with what the
/// registration left in the registry and the checkout's canonical remote.
struct JoinedCheckout {
    registered: crate::project::RegisteredProject,
    inventory: Vec<crate::project::RegisteredProject>,
    remote: Option<String>,
}

/// The part of a join that blocks, run off the message loop: find a URL to
/// clone among this machine's `checkouts`, clone into `dest`, write its
/// `.apas` with the project's id and no panes, register it, and read its
/// remote. The clone is removed if anything fails before it is registered.
fn clone_to_join(
    project_uuid: Uuid,
    dest: &Path,
    git_remote: &str,
    clone_url: Option<&str>,
    checkouts: &[PathBuf],
) -> Result<JoinedCheckout> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let url = clone_url_for(
        git_remote,
        clone_url,
        checkouts.iter().map(PathBuf::as_path),
    );

    let registered = (|| -> Result<(
        crate::project::RegisteredProject,
        Vec<crate::project::RegisteredProject>,
    )> {
        crate::worktree::clone_repo(&url, dest)?;
        // Written outright: a repository that commits a `.apas` would
        // otherwise hand the checkout another project's id and panes.
        let metadata = crate::project::ProjectMetadata::with_id_and_name(
            project_uuid,
            dest.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string),
        );
        let bytes = serde_json::to_vec_pretty(&metadata)?;
        {
            let _lock = crate::project_instances::lock_project_metadata(project_uuid)?;
            crate::project_instances::write_metadata_file(
                &crate::project::get_apas_path(dest),
                &bytes,
            )?;
        }
        // Strict registration refuses the project at another folder under
        // the registry lock, where the daemon's check could have raced.
        let path = std::fs::canonicalize(dest)?;
        let path = path
            .to_str()
            .context("the checkout's path is not valid UTF-8")?;
        crate::project::register_local_project(path)
    })();
    let (registered, inventory) = match registered {
        Ok(registered) => registered,
        Err(err) => {
            let _ = std::fs::remove_dir_all(dest);
            return Err(err);
        }
    };
    tracing::info!(
        "Cloned {} into {} to join project {}",
        url,
        registered.path,
        project_uuid
    );
    // It was just cloned, so one more local `git` read costs little, and the
    // inventory sent next files the checkout under its repository at once.
    let remote = crate::worktree::normalized_git_remote(Path::new(&registered.path));
    Ok(JoinedCheckout {
        registered,
        inventory,
        remote,
    })
}

fn find_provisioning_receipt(
    root: &Path,
    request_id: &str,
    project_id: &str,
) -> Result<Option<PathBuf>> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Ok(None);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(raw) = std::fs::read(path.join(PROVISIONING_RECEIPT)) else {
            continue;
        };
        let Ok(receipt) = serde_json::from_slice::<ProvisioningReceipt>(&raw) else {
            continue;
        };
        if receipt.request_id == request_id && receipt.project_id == project_id {
            let canonical_root = std::fs::canonicalize(root)?;
            let canonical_path = std::fs::canonicalize(&path)?;
            anyhow::ensure!(
                canonical_path.parent() == Some(canonical_root.as_path()),
                "provisioning checkout escaped managed root"
            );
            anyhow::ensure!(
                crate::project::read_project_id(&canonical_path)
                    .is_some_and(|id| id.to_string() == project_id),
                "provisioning receipt metadata does not match"
            );
            return Ok(Some(canonical_path));
        }
    }
    Ok(None)
}

/// Expand a leading `~` / `~/` in `p` against this machine's $HOME.
fn expand_tilde(p: &str) -> PathBuf {
    if p == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    } else if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(p)
}

/// What this daemon tells the server it can do.
///
/// `project_instances_v1` only in [`ClaimMode::Instances`]: a daemon whose
/// machine id is configured shares that id with every host on its home, so
/// the server could not tell its instances apart.
fn daemon_capabilities(claim_mode: ClaimMode) -> Vec<String> {
    let mut capabilities = vec![
        shared::PROJECT_POLICY_CAPABILITY.to_string(),
        shared::PANE_HOST_CLEANUP_ACK_CAPABILITY.to_string(),
        shared::SHARED_PROJECT_PROVISIONING_CAPABILITY.to_string(),
        shared::LOCAL_PROJECT_REGISTRATION_CAPABILITY.to_string(),
        shared::OMP_JOB_WORKER_CAPABILITY.to_string(),
        shared::OMP_PERSISTENT_WORKER_CAPABILITY.to_string(),
    ];
    if claim_mode == ClaimMode::Instances {
        capabilities.push(shared::PROJECT_INSTANCES_CAPABILITY.to_string());
    }
    capabilities
}

pub async fn run(
    server_url: &str,
    token: &str,
    machine_id: Uuid,
    _project_roots: Vec<PathBuf>,
) -> Result<()> {
    // Projects run inside this process now, so they inherit this PATH rather
    // than one set per spawned child. The old model passed
    // `env PATH=<login shell PATH>` on every project's command line precisely
    // because a daemon started from a minimal environment cannot find
    // nvm/cargo-installed providers; applying it here keeps that property for
    // every project at once. Providers are resolved to absolute paths at
    // project startup, so this has to be right before any project starts.
    if let Some(path) = resolve_user_shell_path() {
        let path = merge_paths(&path, &std::env::var("PATH").unwrap_or_default());
        if std::env::var("PATH").ok().as_deref() != Some(path.as_str()) {
            tracing::info!("using the login shell PATH so projects can find their providers");
            std::env::set_var("PATH", &path);
        }
    }
    // Without tmux, terminal tabs run on direct PTYs that end with the CLI.
    // Say which one is used, or that there is none, rather than leave it to
    // a missing "survives a reboot" to tell.
    match find_on_path("tmux") {
        Some(tmux) => {
            tracing::info!(tmux = %tmux.display(), "terminal tabs can use persistent pane hosts")
        }
        None => tracing::warn!(
            "tmux is not on PATH, so terminal tabs run without persistent pane hosts and \
             restart with this CLI; install tmux to keep them across a restart"
        ),
    }
    tracing::info!(
        hostname = %hostname::get().ok().and_then(|name| name.into_string().ok()).unwrap_or_default(),
        "this machine's id is derived from its user and hostname; a changed hostname is a new machine"
    );

    let hostname = hostname::get()
        .ok()
        .and_then(|value| value.into_string().ok())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    let config = crate::config::Config::load().unwrap_or_default();
    // Which providers this machine can launch: a quick look now, so the
    // registration carries it, versions from the monitor shortly after.
    let providers = crate::provider_availability::quick(&config);
    crate::provider_availability::start_monitor();
    let machine_info = MachineInfo {
        machine_id,
        hostname,
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        daemon_version: Some(VERSION.to_string()),
        deepseek_backend: deepseek_backend_info_from_config(&config),
        last_seen: None,
        providers: Some(providers),
    };
    // From the same setting the machine id came from: a configured id is
    // shared with every host on this home, so instances could not be told
    // apart by it.
    let claim_mode = ClaimMode::for_configured_machine_id(config.daemon.machine_id.as_deref());

    let registry = crate::project::project_registry_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "~/.config/apas/projects.json".to_string());
    tracing::info!(
        "Starting daemon for machine {} using project registry {}",
        machine_id,
        registry
    );
    if claim_mode == ClaimMode::Exclusive {
        tracing::info!(
            "daemon.machine_id is set in config.toml, so this host runs each project alone \
             and does not run instances beside other machines"
        );
    }

    let shutdown = Arc::new(AtomicBool::new(false));
    {
        let shutdown = shutdown.clone();
        ctrlc::set_handler(move || {
            shutdown.store(true, Ordering::SeqCst);
        })?;
    }

    // Announce ourselves on the shared NFS registry so peer daemons can see
    // this host is alive. Liveness travels as a heartbeat, never a pid -- a
    // pid is only interpretable on the host that owns it.
    let registry_dir = config_dir_for_registry();
    if let Err(err) = crate::daemon_registry::publish_self(&registry_dir, machine_id, VERSION) {
        tracing::warn!(%err, "could not publish daemon record to the shared registry");
    }
    // Withdraw on ANY exit, not just the connected loop: a daemon killed while
    // still retrying its connection would otherwise leave a record behind.
    let _registration = crate::daemon_registry::RegistrationGuard::new(registry_dir.clone());
    match crate::daemon_registry::live_peers(&registry_dir) {
        peers if peers.is_empty() => {
            tracing::info!("no other apas daemons visible on this shared config dir")
        }
        peers => tracing::info!(
            peers = ?peers.iter().map(|p| p.hostname.as_str()).collect::<Vec<_>>(),
            "other apas daemons are live on this shared config dir"
        ),
    }

    let mut state = DaemonState::new(machine_info, claim_mode);
    state.refresh_projects();
    // Read the checkouts' remotes in the background now, so the first
    // heartbeats are likely to carry them.
    state.refresh_git_remotes();
    // Before anything starts here: an older instance's projects are separate
    // processes this one cannot supervise, and two owners of one project is
    // the failure this whole change exists to remove.
    state.retire_process_per_project_leftovers();

    // Projects ask to be replaced here; only this loop holds the table that
    // can do it.
    let (restart_tx, mut restart_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // Start again what this instance was running before it replaced itself.
    // Pane hosts are separate processes and survived the `exec`, so a prompt
    // resume lands inside their adoption grace and the terminal agents carry
    // straight on. The claims survived the `exec` too, and each start below
    // takes its project's back under this same pid.
    //
    // Before any server has answered: these were running, and what a server
    // says it can do never stops an instance already running. Each runs as
    // the server the replaced daemon was connected to allowed, which it wrote
    // down: alone where that server kept one CLI per session and no other
    // host runs the project, and otherwise as configured.
    for project in take_resume_manifest() {
        let project_id = project.project_id.clone();
        if let Err(err) = state.restore_project_from_resume(&project) {
            tracing::warn!(project_id, %err, "could not restore project path after replacement");
            continue;
        }
        match state.start_project(
            &project_id,
            server_url,
            token,
            &restart_tx,
            StartScope::again(project.single_machine_server),
        ) {
            Ok(StartOutcome::Started | StartOutcome::AlreadyRunning) => {
                tracing::info!(project_id, "resumed project after replacement");
            }
            Ok(StartOutcome::Refused(reason)) => {
                tracing::warn!(project_id, %reason, "could not resume project after replacement");
            }
            Err(err) => {
                tracing::warn!(project_id, %err, "could not resume project after replacement");
            }
        }
    }
    // Claims now match what runs here, before any request is served, so a
    // restart or self-upgrade never leaves a window where our instances look
    // stopped to peers.
    state.reconcile_running_claims();

    let mut reconnect_delay = INITIAL_RECONNECT_DELAY;

    while !shutdown.load(Ordering::SeqCst) {
        // Projects keep running while the server is unreachable, so their
        // claims have to stay fresh between connection attempts too, not only
        // on a connected heartbeat.
        let _ = crate::daemon_registry::publish_self(&registry_dir, machine_id, VERSION);
        state.reconcile_running_claims();
        state.reap_exited_processes();
        state.refresh_projects();

        match run_connection(
            server_url,
            token,
            &mut state,
            shutdown.clone(),
            &restart_tx,
            &mut restart_rx,
        )
        .await
        {
            Ok(()) => {
                reconnect_delay = INITIAL_RECONNECT_DELAY;
            }
            Err(err) => {
                tracing::warn!(
                    "Daemon connection failed: {} (retry in {:?})",
                    err,
                    reconnect_delay
                );
                tokio::time::sleep(reconnect_delay).await;
                reconnect_delay = std::cmp::min(reconnect_delay * 2, MAX_RECONNECT_DELAY);
            }
        }
    }

    // Projects run inside this process now, so a shutdown has to stop them
    // rather than leave them: they used to be self-sufficient processes with
    // their own reconnection loops, and a daemon exiting simply did not
    // concern them. Stopping each one properly is what saves its pane roster
    // and kills its agents' subtrees; dropping the process instead would lose
    // both.
    //
    // This also ends each project's terminal agents. A stop that is not a
    // reboot owns terminal cleanup: the project's teardown shuts every pane
    // host down (`TerminalHandle::shutdown`, which kills the host's tmux
    // session and removes its runtime files). Pane hosts are left for a
    // replacement to adopt only by a requested reboot, which re-execs and
    // never reaches this point.
    let running = state.running_project_ids();
    if !running.is_empty() {
        tracing::info!(count = running.len(), "stopping projects before shutdown");
        for project_id in running {
            state.stop_running_project(&project_id).await;
        }
    }
    tracing::info!("Daemon stopped");
    Ok(())
}

async fn run_connection(
    server_url: &str,
    token: &str,
    state: &mut DaemonState,
    shutdown: Arc<AtomicBool>,
    restart_tx: &RestartRequests,
    restart_rx: &mut tokio::sync::mpsc::UnboundedReceiver<String>,
) -> Result<()> {
    let ws_url = format!("{}/ws/daemon", server_url);
    let (ws_stream, _) = connect_async(&ws_url).await?;
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    let register = DaemonToServer::Register {
        token: token.to_string(),
        machine: state.machine_info.clone(),
        projects: state.snapshot_projects(),
        capabilities: daemon_capabilities(state.claim_mode),
    };
    ws_sender
        .send(Message::Text(serde_json::to_string(&register)?.into()))
        .await?;

    // Registration response
    let job_owner = loop {
        match ws_receiver.next().await {
            Some(Ok(Message::Text(text))) => {
                let msg: ServerToDaemon = serde_json::from_str(&text)?;
                match msg {
                    ServerToDaemon::Registered {
                        cluster_owner_user_id,
                        negotiated_capabilities,
                        ..
                    } => {
                        state.note_registration(&negotiated_capabilities);
                        break cluster_owner_user_id;
                    }
                    ServerToDaemon::RegistrationFailed { reason } => {
                        return Err(anyhow::anyhow!("Registration failed: {}", reason));
                    }
                    _ => {}
                }
            }
            Some(Ok(Message::Ping(data))) => {
                ws_sender.send(Message::Pong(data)).await?;
            }
            Some(Err(err)) => return Err(err.into()),
            None => {
                return Err(anyhow::anyhow!(
                    "Daemon websocket closed during registration"
                ))
            }
            _ => {}
        }
    };
    let mut jobs = job_owner.and_then(|owner| {
        match crate::jobs::DaemonJobs::start(state.machine_info.machine_id, owner) {
            Ok(jobs) => Some(jobs),
            Err(error) => {
                tracing::warn!(%error, "job host configuration unavailable; terminal service unchanged");
                None
            }
        }
    });

    // Report current state to server. CLI processes are only started explicitly
    // via StartProjectCli and are not auto-started on daemon reconnect.
    let update = DaemonToServer::Heartbeat {
        projects: state.snapshot_projects(),
    };
    let text = serde_json::to_string(&update)?;
    ws_sender.send(Message::Text(text.into())).await?;

    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut usage_refresh = tokio::time::interval(USAGE_REFRESH_INTERVAL);
    usage_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    usage_refresh.tick().await;

    refresh_usage_limits_cache().await;

    loop {
        if shutdown.load(Ordering::SeqCst) {
            // RegistrationGuard (installed in `run`) withdraws on drop.
            return Ok(());
        }

        tokio::select! {
            Some(report) = async {
                match jobs.as_mut() {
                    Some(jobs) => jobs.reports.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                let message = DaemonToServer::AutonomousJob { report };
                ws_sender.send(Message::Text(serde_json::to_string(&message)?.into())).await?;
            }
            Some(project_id) = restart_rx.recv() => {
                // A pane's "Reboot CLI" asked for the CLI to be replaced.
                // Projects run inside this process now, so the CLI *is* the
                // daemon and there is no smaller unit whose replacement
                // changes the running version. Restarting only this project's
                // task left the freshly installed binary sitting on disk with
                // the old code still serving: the update was prepared and then
                // never applied, which is why this button could never finish
                // an update while the machine-addressed one could.
                //
                // So do what `ServerToDaemon::RebootDaemon` does. The resume
                // manifest brings every project back, and pane hosts are
                // separate processes that `exec` does not touch, so terminal
                // agents are adopted rather than restarted.
                //
                // The honest cost: one project's button now restarts every
                // project on the host. That is what replacing the CLI means
                // once the projects live inside it.
                #[cfg(unix)]
                {
                    // Returns only on failure, and every fallible step runs
                    // while this daemon is still serving, so a failed update
                    // leaves the host a working daemon rather than none.
                    let err = perform_requested_restart(
                        &state.resume_projects_after_requested_restart(&project_id),
                    );
                    tracing::error!(
                        project_id,
                        %err,
                        "requested CLI restart failed; restarting just this project instead"
                    );
                }
                // Reached only when the replacement failed, or on a platform
                // that cannot `exec`. Restarting the one project still clears
                // whatever made the user ask, so it beats doing nothing. Its
                // task kept the claims for the replacement, so this start
                // finds them still its own. Never refused for what the server
                // runs: the project was running here a moment ago, and that
                // never stops an instance already running. It starts as the
                // resume would have ([`StartScope::again`]).
                state.stop_running_project(&project_id).await;
                let scope = StartScope::again(state.requested_start_scope() == StartScope::Alone);
                match state.start_project(&project_id, server_url, token, restart_tx, scope) {
                    Ok(StartOutcome::Started | StartOutcome::AlreadyRunning) => {
                        tracing::info!(project_id, "restarted project");
                    }
                    Ok(StartOutcome::Refused(reason)) => {
                        tracing::warn!(project_id, %reason, "could not restart project");
                    }
                    Err(err) => {
                        tracing::warn!(project_id, %err, "could not restart project");
                    }
                }
            }
            _ = usage_refresh.tick() => {
                refresh_usage_limits_cache().await;
            }
            _ = heartbeat.tick() => {
                // Keep our record and our instances' claims fresh. Once per
                // tick: this used to run on every wake of the loop, rewriting
                // the shared files for each message received.
                state.keep_claims_fresh();

                state.reap_exited_processes();
                state.refresh_projects();
                state.refresh_git_remotes();

                // A provider installed or removed since the last report.
                if let Some(latest) = crate::provider_availability::latest() {
                    if state.machine_info.providers.as_ref() != Some(&latest) {
                        state.machine_info.providers = Some(latest);
                        let update = DaemonToServer::MachineInfoUpdate {
                            machine: state.machine_info.clone(),
                        };
                        ws_sender
                            .send(Message::Text(serde_json::to_string(&update)?.into()))
                            .await?;
                    }
                }

                let heartbeat_msg = DaemonToServer::Heartbeat {
                    projects: state.snapshot_projects(),
                };
                let text = serde_json::to_string(&heartbeat_msg)?;
                ws_sender.send(Message::Text(text.into())).await?;
            }
            incoming = ws_receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let msg: ServerToDaemon = serde_json::from_str(&text)?;
                        match msg {
                            ServerToDaemon::Registered {
                                negotiated_capabilities,
                                ..
                            } => {
                                // Answered again on this connection: the
                                // server's latest word on what it runs.
                                state.note_registration(&negotiated_capabilities);
                            }
                            ServerToDaemon::RegistrationFailed { reason } => {
                                return Err(anyhow::anyhow!("Daemon auth dropped: {}", reason));
                            }
                            ServerToDaemon::AutonomousJob { command } => {
                                if let Some(jobs) = &jobs {
                                    jobs.send(command).await?;
                                } else {
                                    let report = shared::JobWorkerReport::Rejected {
                                        command_id: crate::jobs::host::command_id(&command),
                                        identity: crate::jobs::host::command_identity(&command).cloned(),
                                        code: "job_host_unavailable".into(),
                                        message: "Authenticated job owner or valid host configuration is unavailable".into(),
                                    };
                                    ws_sender.send(Message::Text(serde_json::to_string(&DaemonToServer::AutonomousJob { report })?.into())).await?;
                                }
                            }
                            ServerToDaemon::StartProjectCli {
                                project_id,
                                policy,
                                request_id,
                            } => {
                                // Every outcome, refusals included, comes back
                                // through here: a server waiting on a
                                // correlated start hears exactly once.
                                for reply in state.handle_start_request(
                                    project_id,
                                    policy,
                                    request_id,
                                    server_url,
                                    token,
                                    restart_tx,
                                ) {
                                    ws_sender
                                        .send(Message::Text(serde_json::to_string(&reply)?.into()))
                                        .await?;
                                }
                            }
                            ServerToDaemon::StopProjectCli { project_id, request_id } => {
                                let stop_result = state.stop_project(&project_id).await;
                                if let Err(err) = &stop_result {
                                    tracing::warn!("Failed to stop project {}: {}", project_id, err);
                                }
                                let remaining_pane_hosts = Uuid::parse_str(&project_id)
                                    .ok()
                                    .and_then(|project_uuid| crate::pane_host::list_project_descriptors(project_uuid).ok())
                                    .map(|descriptors| descriptors.len())
                                    .unwrap_or_default();
                                if let Some(request_id) = request_id {
                                    let success = stop_result.is_ok() && remaining_pane_hosts == 0;
                                    let ack = DaemonToServer::ProjectRuntimeStopped {
                                        request_id,
                                        project_id: project_id.clone(),
                                        success,
                                        remaining_pane_hosts,
                                        error: stop_result.err().map(|error| error.to_string()),
                                    };
                                    ws_sender
                                        .send(Message::Text(serde_json::to_string(&ack)?.into()))
                                        .await?;
                                }
                                let update = DaemonToServer::Heartbeat {
                                    projects: state.snapshot_projects(),
                                };
                                let text = serde_json::to_string(&update)?;
                                ws_sender.send(Message::Text(text.into())).await?;
                            }
                            ServerToDaemon::RegisterLocalProject { path, request_id } => {
                                let result = match state.register_local_project(path).await {
                                    Ok((project, projects)) => {
                                        // The server finalizes identity/placement only against
                                        // inventory confirmed by this authenticated daemon.
                                        let heartbeat = DaemonToServer::Heartbeat { projects };
                                        ws_sender
                                            .send(Message::Text(serde_json::to_string(&heartbeat)?.into()))
                                            .await?;
                                        shared::LocalProjectRegistrationResult::Registered { project }
                                    }
                                    Err(error) => shared::LocalProjectRegistrationResult::Failed {
                                        error: format!("{error:#}"),
                                    },
                                };
                                let ack = DaemonToServer::LocalProjectRegistered { request_id, result };
                                ws_sender
                                    .send(Message::Text(serde_json::to_string(&ack)?.into()))
                                    .await?;
                            }
                            ServerToDaemon::CreateProjectInstance {
                                git_remote,
                                instance_name,
                                clone_url,
                                base_path,
                                request_id,
                                project_id,
                                provisioning_mode,
                                join_existing: true,
                                ..
                            } => {
                                // Another checkout of an existing project. The
                                // loop waits for the clone, which runs on a
                                // blocking thread while the claims keep being
                                // refreshed on the heartbeat. The inventory goes
                                // out before the acknowledgement; see
                                // `handle_join_instance`.
                                for reply in state
                                    .handle_join_instance(
                                        &git_remote,
                                        &instance_name,
                                        clone_url.as_deref(),
                                        base_path.as_deref(),
                                        request_id,
                                        project_id.as_deref(),
                                        provisioning_mode,
                                        &mut heartbeat,
                                    )
                                    .await
                                {
                                    ws_sender
                                        .send(Message::Text(serde_json::to_string(&reply)?.into()))
                                        .await?;
                                }
                            }
                            ServerToDaemon::CreateProjectInstance {
                                git_remote,
                                instance_name,
                                branch,
                                clone_url,
                                base_path,
                                request_id,
                                project_id,
                                provisioning_mode,
                                join_existing: false,
                            } => {
                                // NOTE: clone runs inline on the message loop; a
                                // very large clone briefly delays heartbeats.
                                // GIT_TERMINAL_PROMPT=0 makes auth fail fast.
                                let ack = match state.create_instance(
                                    restart_tx,
                                    &git_remote,
                                    &instance_name,
                                    &branch,
                                    clone_url.as_deref(),
                                    base_path.as_deref(),
                                    server_url,
                                    token,
                                    request_id.as_deref(),
                                    project_id.as_deref(),
                                    provisioning_mode,
                                ) {
                                    Ok((project_id, path)) => {
                                        tracing::info!(
                                            "Created project instance {} at {}",
                                            project_id,
                                            path
                                        );
                                        DaemonToServer::ProjectInstanceCreated {
                                            request_id,
                                            project_id: Some(project_id),
                                            path: Some(path),
                                            error: None,
                                        }
                                    }
                                    Err(err) => {
                                        tracing::warn!("create_instance failed: {}", err);
                                        DaemonToServer::ProjectInstanceCreated {
                                            request_id,
                                            project_id: None,
                                            path: None,
                                            error: Some(if provisioning_mode
                                                == ProjectProvisioningMode::PublicGithub
                                            {
                                                shared::scrub_shared_clone_error(&err).to_string()
                                            } else {
                                                err.to_string()
                                            }),
                                        }
                                    }
                                };
                                ws_sender
                                    .send(Message::Text(serde_json::to_string(&ack)?.into()))
                                    .await?;
                                let update = DaemonToServer::Heartbeat {
                                    projects: state.snapshot_projects(),
                                };
                                ws_sender
                                    .send(Message::Text(serde_json::to_string(&update)?.into()))
                                    .await?;
                            }
                            ServerToDaemon::DiscardProjectProvisioning {
                                request_id,
                                project_id,
                            } => {
                                let result =
                                    state.discard_project_provisioning(&request_id, &project_id);
                                let ack = match result {
                                    Ok(discarded) => DaemonToServer::ProjectProvisioningDiscarded {
                                        request_id,
                                        project_id,
                                        discarded,
                                        error: None,
                                    },
                                    Err(error) => {
                                        tracing::warn!(%error, "provisioning cleanup failed");
                                        DaemonToServer::ProjectProvisioningDiscarded {
                                            request_id,
                                            project_id,
                                            discarded: false,
                                            error: Some(
                                                "Provisioning cleanup could not be completed"
                                                    .to_string(),
                                            ),
                                        }
                                    }
                                };
                                ws_sender
                                    .send(Message::Text(serde_json::to_string(&ack)?.into()))
                                    .await?;
                            }
                            ServerToDaemon::RebootDaemon => {
                                // Projects run inside this process, so the
                                // `exec` takes them with it and the resume
                                // manifest starts them again. Pane hosts are
                                // separate processes it does not touch, so
                                // terminal agents are adopted, not restarted.
                                #[cfg(unix)]
                                {
                                    let err = perform_requested_restart(
                                        &state.running_projects_for_resume(),
                                    );
                                    tracing::error!(
                                        %err,
                                        "requested daemon restart failed; continuing on the current daemon"
                                    );
                                }
                                #[cfg(not(unix))]
                                {
                                    tracing::warn!(
                                        "requested daemon restart is not supported on this platform"
                                    );
                                }
                            }
                            ServerToDaemon::RefreshProjects => {
                                state.refresh_projects();
                                let refresh_msg = DaemonToServer::Heartbeat {
                                    projects: state.snapshot_projects(),
                                };
                                let text = serde_json::to_string(&refresh_msg)?;
                                ws_sender.send(Message::Text(text.into())).await?;
                            }
                            #[allow(deprecated)]
                            ServerToDaemon::SetMiniMaxConfig { .. }
                            | ServerToDaemon::SetGlmConfig { .. } => {
                                // Keep the connection alive for an older
                                // server, but deliberately discard the
                                // retired credential payload.
                                tracing::warn!(
                                    "ignored machine configuration for a retired provider"
                                );
                            }
                            ServerToDaemon::SetDeepseekConfig {
                                api_base_url,
                                api_key,
                                clear_api_key,
                            } => {
                                match update_local_deepseek_backend_config(
                                    api_base_url,
                                    api_key,
                                    clear_api_key,
                                ) {
                                    Ok(deepseek_backend) => {
                                        state.machine_info.deepseek_backend = deepseek_backend;
                                        let update = DaemonToServer::MachineInfoUpdate {
                                            machine: state.machine_info.clone(),
                                        };
                                        let text = serde_json::to_string(&update)?;
                                        ws_sender.send(Message::Text(text.into())).await?;
                                    }
                                    Err(err) => {
                                        tracing::warn!(
                                            "Failed to update DeepSeek backend config: {}",
                                            err
                                        );
                                    }
                                }
                            }
                            ServerToDaemon::Heartbeat => {
                                let pong = DaemonToServer::Heartbeat {
                                    projects: state.snapshot_projects(),
                                };
                                let text = serde_json::to_string(&pong)?;
                                ws_sender.send(Message::Text(text.into())).await?;
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        ws_sender.send(Message::Pong(data)).await?;
                    }
                    Some(Ok(Message::Close(_))) => {
                        return Err(anyhow::anyhow!("Daemon websocket closed"));
                    }
                    Some(Ok(_)) => {}
                    Some(Err(err)) => return Err(err.into()),
                    None => return Err(anyhow::anyhow!("Daemon websocket disconnected")),
                }
            }
        }
    }
}

async fn refresh_usage_limits_cache() {
    match crate::usage::refresh_claude_usage_limits().await {
        Ok(_) => tracing::debug!("Refreshed Claude usage limits cache"),
        Err(err) => tracing::debug!("Failed to refresh Claude usage limits cache: {}", err),
    }

    match crate::usage::refresh_codex_usage_limits().await {
        Ok(_) => tracing::debug!("Refreshed Codex usage limits cache"),
        Err(err) => tracing::debug!("Failed to refresh Codex usage limits cache: {}", err),
    }
}

#[cfg(test)]
mod shared_provisioning_tests {
    use super::*;

    #[test]
    fn provisioning_receipts_are_idempotent_and_bound_to_both_ids() {
        let root = tempfile::tempdir().unwrap();
        let checkout = root.path().join("repo");
        std::fs::create_dir_all(&checkout).unwrap();
        let project_id = Uuid::new_v4();
        let metadata =
            crate::project::ProjectMetadata::with_id_and_name(project_id, Some("repo".to_string()));
        std::fs::write(
            checkout.join(".apas"),
            serde_json::to_vec_pretty(&metadata).unwrap(),
        )
        .unwrap();
        std::fs::write(
            checkout.join(PROVISIONING_RECEIPT),
            serde_json::to_vec_pretty(&ProvisioningReceipt {
                request_id: "request-a".to_string(),
                project_id: project_id.to_string(),
            })
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            find_provisioning_receipt(root.path(), "request-a", &project_id.to_string()).unwrap(),
            Some(std::fs::canonicalize(&checkout).unwrap())
        );
        assert!(
            find_provisioning_receipt(root.path(), "request-b", &project_id.to_string())
                .unwrap()
                .is_none()
        );
        assert!(
            find_provisioning_receipt(root.path(), "request-a", &Uuid::new_v4().to_string())
                .unwrap()
                .is_none()
        );
    }
}

#[cfg(test)]
mod attribution_tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::SubscriberExt;

    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Each project used to have its own tmux session and stderr file, which
    /// told their records apart for free. Sharing a process took that away, so
    /// the span has to carry the identity into the output — otherwise one
    /// incident's log is several projects interleaved with no way to separate
    /// them.
    #[test]
    fn records_from_two_projects_are_distinguishable() {
        let captured = Captured::default();
        let sink = captured.clone();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(move || sink.clone()),
        );

        tracing::subscriber::with_default(subscriber, || {
            let alpha = tracing::info_span!("project", id = "alpha");
            alpha.in_scope(|| tracing::error!("pane failed"));
            let beta = tracing::info_span!("project", id = "beta");
            beta.in_scope(|| tracing::error!("pane failed"));
        });

        let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = output
            .lines()
            .filter(|l| l.contains("pane failed"))
            .collect();
        assert_eq!(lines.len(), 2, "both records were emitted: {output}");
        assert!(
            lines[0].contains("alpha"),
            "the first record names its project: {}",
            lines[0]
        );
        assert!(
            lines[1].contains("beta"),
            "the second record names its project: {}",
            lines[1]
        );
    }
}

#[cfg(test)]
mod containment_tests {
    /// Containment rests on unwind. With `panic = "abort"`, one project's
    /// panic would abort the process and take every other project on the host
    /// with it — silently, since nothing else would change. This fails if
    /// anyone ever sets it.
    /// The instance stops its projects on shutdown, but only when something
    /// sets the shutdown flag. `ctrlc` handles SIGINT alone unless the
    /// `termination` feature is on, and a `setsid`-detached daemon essentially
    /// never receives a SIGINT — so without this the whole teardown is
    /// unreachable and nothing looks any different. This fails if the feature
    /// is ever dropped.
    #[test]
    fn stopping_the_instance_must_reach_its_teardown() {
        let manifest = include_str!("../../Cargo.toml");
        let line = manifest
            .lines()
            .map(str::trim)
            .find(|line| line.starts_with("ctrlc"))
            .expect("the signal handler dependency is declared");
        assert!(
            line.contains("termination"),
            "without the termination feature a SIGTERM ends the instance \
             outright, dropping every project it hosts: {line}"
        );
    }

    #[test]
    fn a_panicking_project_must_not_be_able_to_abort_the_instance() {
        let manifests = [
            include_str!("../../Cargo.toml"),
            include_str!("../../../../Cargo.toml"),
        ];
        for manifest in manifests {
            let offending: Vec<&str> = manifest
                .lines()
                .map(str::trim)
                .filter(|line| line.starts_with("panic") && line.contains("abort"))
                .collect();
            assert!(
                offending.is_empty(),
                "panic = \"abort\" turns any project panic into a host-wide outage: {offending:?}"
            );
        }
    }

    /// The behaviour that relies on it: a panicking task is an error to its
    /// joiner, not a dead process.
    #[tokio::test]
    async fn a_panicking_task_is_reported_and_the_rest_carry_on() {
        let doomed = tokio::spawn(async { panic!("a project fell over") });
        let survivor = tokio::spawn(async { "still here" });

        assert!(doomed.await.is_err(), "the panic surfaces to the joiner");
        assert_eq!(survivor.await.unwrap(), "still here");
    }
}

#[cfg(test)]
mod local_registration_tests {
    use super::*;

    fn state() -> DaemonState {
        DaemonState::new(
            MachineInfo {
                machine_id: Uuid::new_v4(),
                hostname: "local-registration-test".into(),
                os: "linux".into(),
                arch: "x86_64".into(),
                daemon_version: None,
                deepseek_backend: None,
                last_seen: None,
                providers: None,
            },
            ClaimMode::Instances,
        )
    }

    #[test]
    fn registration_reports_stopped_inventory_without_launching_saved_panes() {
        crate::project::test_support::with_isolated_config(|| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let dir = tempfile::tempdir().unwrap();
                let mut metadata = crate::project::ProjectMetadata::new();
                metadata.deadloop_claude_session_id = Some(Uuid::new_v4());
                metadata.migrate_legacy();
                let bytes = serde_json::to_vec(&metadata).unwrap();
                std::fs::write(dir.path().join(".apas"), &bytes).unwrap();
                let mut state = state();
                let (project, inventory) = state
                    .register_local_project(dir.path().to_str().unwrap().to_string())
                    .await
                    .unwrap();
                assert_eq!(project.project_id, metadata.id.to_string());
                assert!(!project.is_running);
                assert!(project.pid.is_none());
                assert!(project.memory_kb.is_none());
                assert_eq!(inventory.len(), 1);
                assert_eq!(inventory[0].project_id, project.project_id);
                assert!(state.running.is_empty());
                assert!(state.sessions.is_empty());
                assert_eq!(std::fs::read(dir.path().join(".apas")).unwrap(), bytes);
            });
        });
    }

    #[test]
    fn repeated_registration_preserves_running_task_and_error_state() {
        crate::project::test_support::with_isolated_config(|| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().to_str().unwrap().to_string();
                let mut state = state();
                let (project, _) = state.register_local_project(path.clone()).await.unwrap();
                let shutdown = Arc::new(AtomicBool::new(false));
                let handle = tokio::spawn(std::future::pending::<()>());
                let task_id = handle.id();
                state.running.insert(
                    project.project_id.clone(),
                    RunningProject {
                        shutdown: shutdown.clone(),
                        handle,
                        ended_because: Default::default(),
                        claims: ClaimMode::Instances,
                    },
                );
                state
                    .projects
                    .get_mut(&project.project_id)
                    .unwrap()
                    .last_error = Some("retained diagnostic".into());
                let (repeated, inventory) = state.register_local_project(path).await.unwrap();
                assert_eq!(repeated.project_id, project.project_id);
                assert!(repeated.is_running);
                assert_eq!(repeated.pid, Some(std::process::id()));
                assert_eq!(repeated.last_error.as_deref(), Some("retained diagnostic"));
                assert_eq!(inventory.len(), 1);
                assert_eq!(state.running.len(), 1);
                assert_eq!(state.running[&project.project_id].handle.id(), task_id);
                assert!(!shutdown.load(Ordering::Relaxed));
                state.running[&project.project_id].handle.abort();
            });
        });
    }

    #[test]
    fn registration_error_keeps_existing_inventory_and_files() {
        crate::project::test_support::with_isolated_config(|| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let good = tempfile::tempdir().unwrap();
                let bad = tempfile::tempdir().unwrap();
                std::fs::write(bad.path().join(".apas"), b"corrupt").unwrap();
                let mut state = state();
                let (project, _) = state
                    .register_local_project(good.path().to_str().unwrap().into())
                    .await
                    .unwrap();
                assert!(state
                    .register_local_project(bad.path().to_str().unwrap().into())
                    .await
                    .is_err());
                assert_eq!(state.projects.len(), 1);
                assert!(state.projects.contains_key(&project.project_id));
                assert!(state.running.is_empty());
                assert_eq!(std::fs::read(bad.path().join(".apas")).unwrap(), b"corrupt");
            });
        });
    }
}

#[cfg(test)]
mod requested_restart_tests {
    use super::{DaemonState, ProjectEntry, RunningProject};
    use crate::daemon_registry::ClaimMode;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn machine() -> shared::MachineInfo {
        shared::MachineInfo {
            machine_id: uuid::Uuid::nil(),
            hostname: "test-host".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            daemon_version: None,
            deepseek_backend: None,
            last_seen: None,
            providers: None,
        }
    }

    fn entry(state: &mut DaemonState, project_id: &str, handle: tokio::task::JoinHandle<()>) {
        state.projects.insert(
            project_id.to_string(),
            ProjectEntry {
                name: None,
                path: PathBuf::from(format!("/projects/{project_id}")),
                last_error: None,
            },
        );
        state.running.insert(
            project_id.to_string(),
            RunningProject {
                shutdown: Arc::new(AtomicBool::new(false)),
                handle,
                ended_because: Default::default(),
                claims: ClaimMode::Instances,
            },
        );
    }

    async fn finished_task() -> tokio::task::JoinHandle<()> {
        let handle = tokio::spawn(async {});
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !handle.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await;
        handle
    }

    fn long_running_task() -> tokio::task::JoinHandle<()> {
        tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        })
    }

    /// The project that asked for the reboot is the one the resume manifest
    /// would otherwise drop.
    ///
    /// A project requests a CLI replacement by ending its own task, so by the
    /// time the instance acts the task is finished — and finished tasks are
    /// deliberately excluded from the manifest. Without special handling the
    /// user presses "Reboot CLI" and it is the one project that never returns.
    #[tokio::test]
    async fn the_project_that_asked_to_restart_is_still_resumed() {
        let mut state = DaemonState::new(machine(), ClaimMode::Instances);
        entry(&mut state, "requester", finished_task().await);
        entry(&mut state, "bystander", long_running_task());

        let resume = state.resume_projects_after_requested_restart("requester");
        let ids: Vec<&str> = resume.iter().map(|p| p.project_id.as_str()).collect();

        assert!(
            ids.contains(&"requester"),
            "the project that asked for the restart must be resumed: {ids:?}"
        );
        assert!(
            ids.contains(&"bystander"),
            "replacing the instance restarts every project on the host: {ids:?}"
        );
        assert_eq!(resume.len(), 2, "no project is listed twice: {ids:?}");
        assert_eq!(
            resume
                .iter()
                .find(|p| p.project_id == "requester")
                .and_then(|p| p.path.clone()),
            Some(PathBuf::from("/projects/requester")),
            "a resumed project carries the path needed to start it again"
        );
    }

    /// A project still running when it asks must not be listed twice, or the
    /// replacement would start it once per entry.
    #[tokio::test]
    async fn a_still_running_requester_is_not_duplicated() {
        let mut state = DaemonState::new(machine(), ClaimMode::Instances);
        entry(&mut state, "requester", long_running_task());

        let resume = state.resume_projects_after_requested_restart("requester");
        assert_eq!(resume.len(), 1, "one entry per project");
        assert_eq!(resume[0].project_id, "requester");
    }

    /// A request naming a project this instance does not know must not invent
    /// one: the replacement would have no path to start it from.
    #[tokio::test]
    async fn an_unknown_project_is_not_invented() {
        let state = DaemonState::new(machine(), ClaimMode::Instances);
        assert!(state
            .resume_projects_after_requested_restart("never-heard-of-it")
            .is_empty());
    }

    /// A project that stopped on its own is still excluded. The special case
    /// is scoped to the requester, not a blanket "resume everything".
    #[tokio::test]
    async fn an_unrelated_finished_project_is_still_not_resumed() {
        let mut state = DaemonState::new(machine(), ClaimMode::Instances);
        entry(&mut state, "requester", long_running_task());
        entry(&mut state, "stopped-on-its-own", finished_task().await);

        let resume = state.resume_projects_after_requested_restart("requester");
        let ids: Vec<&str> = resume.iter().map(|p| p.project_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["requester"],
            "only the requester is special-cased"
        );
    }
}

#[cfg(test)]
mod resume_tests {
    use super::{
        should_forget_project, take_resume_manifest_at, take_resume_manifests_at,
        write_resume_manifest_at, ResumeProject,
    };
    use std::path::PathBuf;

    /// A Mac's first Reboot to update: the replaced APAS wrote its manifest
    /// under the temp dir, the replacement reads its new runtime dir first.
    /// Both are taken and cleared, and a project named in both resumes once,
    /// as the current manifest says.
    #[test]
    fn a_manifest_the_replaced_apas_wrote_in_its_old_place_is_taken_too() {
        let dir = tempfile::tempdir().expect("temp dir");
        let current = dir.path().join("run/sup/resume.json");
        let legacy = dir.path().join("tmp/apas-501/apas/sup/resume.json");
        let missing = dir.path().join("elsewhere/sup/resume.json");
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        let project = |id: &str, path: &str, alone: bool| ResumeProject {
            project_id: id.to_string(),
            path: Some(PathBuf::from(path)),
            single_machine_server: alone,
        };
        write_resume_manifest_at(&current, &[project("alpha", "/new/alpha", false)]);
        write_resume_manifest_at(
            &legacy,
            &[
                project("alpha", "/old/alpha", true),
                project("chiral", "/Users/me/chiral-network", false),
            ],
        );
        let paths = vec![current.clone(), missing, legacy.clone()];
        assert_eq!(
            take_resume_manifests_at(&paths),
            vec![
                project("alpha", "/new/alpha", false),
                project("chiral", "/Users/me/chiral-network", false),
            ]
        );
        assert!(!current.exists() && !legacy.exists(), "both are cleared");
        assert!(take_resume_manifests_at(&paths).is_empty());
    }

    /// Under the process-per-project model an upgrade left projects alone —
    /// they were in their own tmux sessions and `exec` never touched them.
    /// Running them inside this process means `exec` takes them with it, so
    /// what was running has to survive the replacement in writing.
    #[test]
    fn what_was_running_survives_the_replacement() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("resume.json");

        let projects = vec![
            ResumeProject {
                project_id: "alpha".to_string(),
                path: Some(PathBuf::from("/projects/alpha")),
                single_machine_server: false,
            },
            ResumeProject {
                project_id: "beta".to_string(),
                path: Some(PathBuf::from("/projects/beta")),
                single_machine_server: false,
            },
        ];
        write_resume_manifest_at(&path, &projects);
        assert_eq!(
            take_resume_manifest_at(&path),
            projects,
            "paths survive so resume does not depend on the shared registry"
        );

        // Cleared on read: a crash during resume must not leave the instance
        // retrying the same start on every boot.
        assert!(take_resume_manifest_at(&path).is_empty());
    }

    #[test]
    fn nothing_running_leaves_nothing_to_resume() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("resume.json");

        write_resume_manifest_at(
            &path,
            &[ResumeProject {
                project_id: "alpha".to_string(),
                path: Some(PathBuf::from("/projects/alpha")),
                single_machine_server: false,
            }],
        );
        write_resume_manifest_at(&path, &[]);
        assert!(take_resume_manifest_at(&path).is_empty());
    }

    #[test]
    fn legacy_id_only_manifest_remains_readable() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("resume.json");
        std::fs::write(&path, br#"["alpha"]"#).expect("write legacy manifest");

        assert_eq!(
            take_resume_manifest_at(&path),
            vec![ResumeProject {
                project_id: "alpha".to_string(),
                path: None,
                single_machine_server: false,
            }]
        );
    }

    /// Whether the replaced daemon's server kept one CLI per session survives
    /// the replacement, which resumes before it can ask a server. A manifest
    /// written before the flag existed resumes as configured, as it always
    /// did, and a daemon from before the flag still reads one that carries
    /// it, so rolling back does not cost the projects their resume.
    #[test]
    fn whether_the_server_kept_one_cli_per_session_survives_the_replacement() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("resume.json");
        let projects = vec![
            ResumeProject {
                project_id: "alpha".to_string(),
                path: Some(PathBuf::from("/projects/alpha")),
                single_machine_server: true,
            },
            ResumeProject {
                project_id: "beta".to_string(),
                path: Some(PathBuf::from("/projects/beta")),
                single_machine_server: false,
            },
        ];
        write_resume_manifest_at(&path, &projects);

        /// A resume entry as a daemon from before the flag declares it.
        #[derive(serde::Deserialize)]
        struct Earlier {
            project_id: String,
            #[serde(default)]
            path: Option<PathBuf>,
        }
        let earlier: Vec<Earlier> =
            serde_json::from_slice(&std::fs::read(&path).expect("read manifest"))
                .expect("an earlier daemon reads it");
        assert_eq!(
            earlier
                .iter()
                .map(|project| (project.project_id.as_str(), project.path.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("alpha", Some(PathBuf::from("/projects/alpha"))),
                ("beta", Some(PathBuf::from("/projects/beta")))
            ]
        );
        assert_eq!(take_resume_manifest_at(&path), projects);

        std::fs::write(
            &path,
            br#"[{"project_id":"alpha","path":"/projects/alpha"}]"#,
        )
        .expect("write an earlier manifest");
        assert_eq!(
            take_resume_manifest_at(&path),
            vec![ResumeProject {
                project_id: "alpha".to_string(),
                path: Some(PathBuf::from("/projects/alpha")),
                single_machine_server: false,
            }]
        );
    }

    #[test]
    fn a_running_project_survives_a_missing_registry_entry() {
        assert!(!should_forget_project(false, true, false));
        assert!(!should_forget_project(false, false, true));
        assert!(should_forget_project(false, false, false));
    }
}

#[cfg(test)]
mod instance_tests {
    use super::*;
    use crate::config::test_config::ConfigDirGuard;
    use crate::mode::dual_pane::ProjectOutcome;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// How the stand-in for the project runtime ends.
    #[derive(Clone, Copy, Debug)]
    enum Ending {
        WhenStopped,
        Completed,
        Stopped,
        Failed,
        RebootRequested,
        Panics,
    }

    fn launcher(ending: Ending) -> ProjectLauncher {
        Arc::new(move |launch: ProjectLaunch| {
            Box::pin(async move {
                match ending {
                    Ending::WhenStopped => {
                        while !launch.shutdown.load(Ordering::SeqCst) {
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                        Ok(ProjectOutcome::Completed)
                    }
                    Ending::Completed => Ok(ProjectOutcome::Completed),
                    Ending::Stopped => Ok(ProjectOutcome::Stopped("session rejected".into())),
                    Ending::Failed => Err(anyhow::anyhow!("could not reach the server")),
                    Ending::RebootRequested => Ok(ProjectOutcome::RebootRequested),
                    Ending::Panics => panic!("a project fell over"),
                }
            })
        })
    }

    /// A runtime whose teardown outlasts a stop's wait: it ends once `release`
    /// is set, whatever `shutdown` says, and sets `ended` as it does.
    fn slow_to_stop(release: Arc<AtomicBool>, ended: Arc<AtomicBool>) -> ProjectLauncher {
        Arc::new(move |_launch: ProjectLaunch| {
            let (release, ended) = (release.clone(), ended.clone());
            Box::pin(async move {
                while !release.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                ended.store(true, Ordering::SeqCst);
                Ok(ProjectOutcome::Completed)
            })
        })
    }

    /// Each launch's project path, and whether it was let run as an instance.
    type Launches = Arc<Mutex<Vec<(PathBuf, bool)>>>;

    /// `inner`, recording into `launches` how each launch was asked to run.
    fn recording(inner: ProjectLauncher, launches: Launches) -> ProjectLauncher {
        Arc::new(move |launch: ProjectLaunch| {
            launches
                .lock()
                .unwrap()
                .push((launch.project_path.clone(), launch.instances));
            inner(launch)
        })
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    struct Harness {
        state: DaemonState,
        restart_tx: RestartRequests,
        restart_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
        projects: tempfile::TempDir,
        // This test's config directory, which holds the claims, and the
        // thread's redirect to it, for as long as the harness lives.
        config: ConfigDirGuard,
        launches: Launches,
    }

    fn harness(mode: ClaimMode, ending: Ending) -> Harness {
        let config = crate::config::test_config::isolated_config_dir();
        let mut state = DaemonState::new(
            MachineInfo {
                machine_id: Uuid::new_v4(),
                hostname: "instance-test".into(),
                os: "linux".into(),
                arch: "x86_64".into(),
                daemon_version: None,
                deepseek_backend: None,
                last_seen: None,
                providers: None,
            },
            mode,
        );
        assert_eq!(
            state.registry_dir,
            config.path(),
            "claims stay in the test's directory"
        );
        // Registered with a server that runs instances, unless a test says
        // otherwise.
        state.note_registration(&[shared::PROJECT_INSTANCES_CAPABILITY.to_string()]);
        let launches = Launches::default();
        state.launcher = recording(launcher(ending), launches.clone());
        let (restart_tx, restart_rx) = tokio::sync::mpsc::unbounded_channel();
        Harness {
            state,
            restart_tx,
            restart_rx,
            projects: tempfile::tempdir().unwrap(),
            config,
            launches,
        }
    }

    impl Harness {
        fn machine(&self) -> Uuid {
            self.state.machine_info.machine_id
        }

        /// Whether the newest launch of `project_id` was let run as an
        /// instance, or `None` if it was never launched.
        fn launched_as_instance(&self, project_id: &str) -> Option<bool> {
            let path = self.projects.path().join(project_id);
            self.launches
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|(launched, _)| *launched == path)
                .map(|(_, instances)| *instances)
        }

        fn add_project(&mut self, project_id: &str) -> PathBuf {
            let path = self.projects.path().join(project_id);
            self.state.projects.insert(
                project_id.to_string(),
                ProjectEntry {
                    name: None,
                    path: path.clone(),
                    last_error: None,
                },
            );
            path
        }

        fn start(&mut self, project_id: &str) -> StartOutcome {
            self.start_as(project_id, StartScope::AsConfigured)
        }

        fn start_as(&mut self, project_id: &str, scope: StartScope) -> StartOutcome {
            self.state
                .start_project(
                    project_id,
                    "ws://127.0.0.1:9",
                    "token",
                    &self.restart_tx,
                    scope,
                )
                .unwrap()
        }

        fn request_start(
            &mut self,
            project_id: &str,
            policy: Option<EffectiveProjectPolicy>,
            request_id: Option<Uuid>,
        ) -> Vec<DaemonToServer> {
            self.state.handle_start_request(
                project_id.to_string(),
                policy,
                request_id,
                "ws://127.0.0.1:9",
                "token",
                &self.restart_tx,
            )
        }

        fn guard_path(&self, project_id: &str) -> PathBuf {
            self.config
                .path()
                .join("claims")
                .join(format!("{project_id}.json"))
        }

        fn instance_path(&self, project_id: &str, machine: Uuid) -> PathBuf {
            self.config
                .path()
                .join("claims")
                .join(project_id)
                .join(format!("{machine}.json"))
        }

        fn read(&self, path: &Path) -> serde_json::Value {
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
        }

        /// Wait for a project's task to end on its own.
        async fn wait_until_ended(&self, project_id: &str) {
            let handle = &self.state.running[project_id].handle;
            for _ in 0..400 {
                if handle.is_finished() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("{project_id}'s task did not end");
        }
    }

    /// A claim another host wrote, in the on-disk shape every daemon reads.
    fn write_foreign_claim(path: &Path, host: &str, machine: Uuid, heartbeat: u64, marked: bool) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut claim = serde_json::json!({
            "project_id": "p",
            "project_path": "/elsewhere/p",
            "hostname": host,
            "machine_id": machine,
            "pid": 4242,
            "claimed_at": heartbeat,
            "heartbeat": heartbeat,
        });
        if marked {
            claim["instances"] = serde_json::json!(true);
        }
        std::fs::write(path, serde_json::to_vec(&claim).unwrap()).unwrap();
    }

    /// The project runtime's own process, as `/proc` shows an
    /// `apas --headless -d <path>` run started outside the daemon.
    struct ExternalHeadlessRun(std::process::Child);

    impl ExternalHeadlessRun {
        fn start(path: &Path) -> Self {
            use std::os::unix::process::CommandExt;
            let child = Command::new("sh")
                .arg0("apas")
                .args(["-c", "sleep 30; true", "--headless", "-d"])
                .arg(path)
                .process_group(0)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("start a stand-in headless run");
            Self(child)
        }
    }

    impl Drop for ExternalHeadlessRun {
        fn drop(&mut self) {
            // The shell and its `sleep` share the group it leads.
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            let _ = self.0.wait();
        }
    }

    fn outcome_of(replies: &[DaemonToServer]) -> Option<(Uuid, String, bool, Option<String>)> {
        let mut outcomes = replies.iter().filter_map(|reply| match reply {
            DaemonToServer::ProjectRuntimeStarted {
                request_id,
                project_id,
                success,
                error,
            } => Some((*request_id, project_id.clone(), *success, error.clone())),
            _ => None,
        });
        let outcome = outcomes.next();
        assert!(
            outcomes.next().is_none(),
            "one outcome per request: {replies:?}"
        );
        outcome
    }

    fn inventory_entry(replies: &[DaemonToServer], project_id: &str) -> MachineProjectInfo {
        match replies.first() {
            Some(DaemonToServer::Heartbeat { projects }) => projects
                .iter()
                .find(|project| project.project_id == project_id)
                .cloned()
                .expect("the project is in the inventory"),
            other => panic!("the inventory goes first, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_started_instance_holds_its_claims_and_stopping_it_releases_them() {
        let mut h = harness(ClaimMode::Instances, Ending::WhenStopped);
        h.add_project("proj-stop");
        assert_eq!(h.start("proj-stop"), StartOutcome::Started);

        // Claimed before the runtime ran, so its own adoption check sees it.
        let own = h.read(&h.instance_path("proj-stop", h.machine()));
        assert_eq!(
            own["hostname"],
            serde_json::json!(crate::daemon_registry::hostname())
        );
        let guard = h.read(&h.guard_path("proj-stop"));
        assert_eq!(guard["instances"], serde_json::json!(true));
        assert_eq!(h.start("proj-stop"), StartOutcome::AlreadyRunning);

        h.state.stop_project("proj-stop").await.unwrap();
        assert!(!h.instance_path("proj-stop", h.machine()).exists());
        assert!(
            !h.guard_path("proj-stop").exists(),
            "stopped here, startable elsewhere at once"
        );
    }

    /// Every way a task can end without being asked to releases its claims.
    /// They used to be kept, and refreshed, for as long as the daemon ran.
    #[tokio::test]
    async fn an_instance_that_ends_on_its_own_releases_its_claims() {
        for ending in [
            Ending::Completed,
            Ending::Stopped,
            Ending::Failed,
            Ending::Panics,
        ] {
            let mut h = harness(ClaimMode::Instances, ending);
            h.add_project("proj-end");
            assert_eq!(h.start("proj-end"), StartOutcome::Started, "{ending:?}");
            h.wait_until_ended("proj-end").await;
            assert!(
                !h.instance_path("proj-end", h.machine()).exists(),
                "{ending:?}"
            );
            assert!(!h.guard_path("proj-end").exists(), "{ending:?}");
            // Nothing stops it starting again.
            assert_eq!(h.start("proj-end"), StartOutcome::Started, "{ending:?}");
        }
    }

    /// A project asking for the daemon to be replaced keeps its claims: the
    /// replacement resumes it under the same pid, and releasing first would
    /// show peers an unclaimed project for the length of the update.
    #[tokio::test]
    async fn an_instance_asking_for_a_replacement_keeps_its_claims() {
        let mut h = harness(ClaimMode::Instances, Ending::RebootRequested);
        h.add_project("proj-reboot");
        assert_eq!(h.start("proj-reboot"), StartOutcome::Started);
        assert_eq!(h.restart_rx.recv().await.as_deref(), Some("proj-reboot"));
        h.wait_until_ended("proj-reboot").await;
        assert!(h.instance_path("proj-reboot", h.machine()).exists());
        assert!(h.guard_path("proj-reboot").exists());

        // When the replacement fails the daemon restarts just this project,
        // which finds the claims still its own.
        h.state.launcher = launcher(Ending::WhenStopped);
        assert!(h.state.stop_running_project("proj-reboot").await);
        assert_eq!(h.start("proj-reboot"), StartOutcome::Started);
        assert!(h.instance_path("proj-reboot", h.machine()).exists());
    }

    /// A stop that gives up on a task leaves it running detached, and the
    /// project can be started here again before that task ends. The claims are
    /// the new start's then. The old task's end used to delete them from under
    /// the new one, and until a heartbeat retook them an older daemon on
    /// another host could start the project beside it.
    #[tokio::test]
    async fn a_task_a_stop_gave_up_on_leaves_the_next_starts_claims_alone() {
        let mut h = harness(ClaimMode::Instances, Ending::WhenStopped);
        h.state.stop_wait = Duration::from_millis(50);
        let release = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(AtomicBool::new(false));
        h.state.launcher = slow_to_stop(release.clone(), ended.clone());
        h.add_project("proj-slow");
        assert_eq!(h.start("proj-slow"), StartOutcome::Started);

        // The stop gives up; the old task runs on and keeps the claims.
        h.state.stop_project("proj-slow").await.unwrap();
        assert!(!ended.load(Ordering::SeqCst));
        assert!(h.instance_path("proj-slow", h.machine()).exists());
        assert!(h.guard_path("proj-slow").exists());

        h.state.launcher = launcher(Ending::WhenStopped);
        assert_eq!(h.start("proj-slow"), StartOutcome::Started);

        release.store(true, Ordering::SeqCst);
        for _ in 0..400 {
            if ended.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // The old task's claims went, or stayed, in the same poll that ended
        // it, which on this single-threaded test runtime is over by now.
        assert!(ended.load(Ordering::SeqCst), "the old task ended");
        assert!(
            h.instance_path("proj-slow", h.machine()).exists(),
            "the new start's instance claim survives the old task's end"
        );
        assert!(h.guard_path("proj-slow").exists(), "and so does the guard");
        assert_eq!(
            h.read(&h.guard_path("proj-slow"))["instances"],
            serde_json::json!(true)
        );

        // The new start's own end still releases them.
        h.state.stop_project("proj-slow").await.unwrap();
        assert!(!h.instance_path("proj-slow", h.machine()).exists());
        assert!(!h.guard_path("proj-slow").exists());
    }

    /// The daemon's own shutdown stops each project the same way, so the
    /// claims go with them.
    #[tokio::test]
    async fn shutting_the_daemon_down_releases_every_instance() {
        let mut h = harness(ClaimMode::Instances, Ending::WhenStopped);
        for project_id in ["proj-a", "proj-b"] {
            h.add_project(project_id);
            assert_eq!(h.start(project_id), StartOutcome::Started);
        }
        for project_id in h.state.running_project_ids() {
            assert!(h.state.stop_running_project(&project_id).await);
        }
        for project_id in ["proj-a", "proj-b"] {
            assert!(!h.instance_path(project_id, h.machine()).exists());
            assert!(!h.guard_path(project_id).exists());
        }
    }

    /// Only what runs here is kept alive; a stopped project's leftover claim
    /// is left to lapse.
    #[tokio::test]
    async fn the_heartbeat_refreshes_only_running_instances() {
        let mut h = harness(ClaimMode::Instances, Ending::WhenStopped);
        h.add_project("proj-live");
        h.add_project("proj-idle");
        assert_eq!(h.start("proj-live"), StartOutcome::Started);
        let me = crate::daemon_registry::hostname();
        let old = now_secs() - 60;
        write_foreign_claim(
            &h.instance_path("proj-live", h.machine()),
            &me,
            h.machine(),
            old,
            true,
        );
        write_foreign_claim(
            &h.instance_path("proj-idle", h.machine()),
            &me,
            h.machine(),
            old,
            true,
        );

        h.state.reconcile_running_claims();
        let live = h.read(&h.instance_path("proj-live", h.machine()));
        assert!(live["heartbeat"].as_u64().unwrap() >= old + 60);
        assert_eq!(live["pid"], serde_json::json!(std::process::id()));
        let idle = h.read(&h.instance_path("proj-idle", h.machine()));
        assert_eq!(idle["heartbeat"], serde_json::json!(old));
    }

    /// Blocking work the message loop waits for, a clone to join a project
    /// above all, runs off it while the loop keeps the claims of what runs
    /// here fresh. Run inline, it left them to lapse for as long as it took:
    /// past `STALE_AFTER_SECS`, a daemon that runs projects alone could start
    /// a project running here.
    #[tokio::test]
    async fn the_claims_stay_fresh_while_the_loop_waits_on_blocking_work() {
        let mut h = harness(ClaimMode::Instances, Ending::WhenStopped);
        h.add_project("proj-live");
        assert_eq!(h.start("proj-live"), StartOutcome::Started);
        let me = crate::daemon_registry::hostname();
        let old = now_secs() - 60;
        let claim = h.instance_path("proj-live", h.machine());
        write_foreign_claim(&claim, &me, h.machine(), old, true);

        // The work ends only once it sees the claim refreshed, so the refresh
        // happened while it blocked.
        let mut heartbeat = tokio::time::interval(Duration::from_millis(10));
        let refreshed_meanwhile = h
            .state
            .wait_off_loop(&mut heartbeat, move || {
                let deadline = Instant::now() + Duration::from_secs(10);
                while Instant::now() < deadline {
                    let heartbeat = std::fs::read(&claim)
                        .ok()
                        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                        .and_then(|claim| claim["heartbeat"].as_u64());
                    if heartbeat.is_some_and(|heartbeat| heartbeat > old) {
                        return true;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                false
            })
            .await
            .unwrap();
        assert!(refreshed_meanwhile);
        assert!(h
            .state
            .running_project_ids()
            .contains(&"proj-live".to_string()));
    }

    /// Joining is the point: another machine running the project is no
    /// reason to refuse, and its guard stays with it.
    #[tokio::test]
    async fn a_capable_daemon_starts_beside_another_machines_instance() {
        let mut h = harness(ClaimMode::Instances, Ending::WhenStopped);
        h.add_project("proj-shared");
        let peer = Uuid::new_v4();
        let now = now_secs();
        write_foreign_claim(
            &h.instance_path("proj-shared", peer),
            "peer.invalid",
            peer,
            now,
            true,
        );
        write_foreign_claim(
            &h.guard_path("proj-shared"),
            "peer.invalid",
            peer,
            now,
            true,
        );

        let replies = h.request_start(
            "proj-shared",
            Some(EffectiveProjectPolicy::default()),
            Some(Uuid::new_v4()),
        );
        let (_, _, success, error) = outcome_of(&replies).unwrap();
        assert!(success, "{error:?}");
        assert!(inventory_entry(&replies, "proj-shared").is_running);
        assert!(h.instance_path("proj-shared", h.machine()).exists());
        assert_eq!(
            h.read(&h.guard_path("proj-shared"))["hostname"],
            serde_json::json!("peer.invalid")
        );
    }

    /// A server that does not echo `project_instances_v1` keeps one CLI per
    /// session, so a second machine's runtime would replace the first's there
    /// and leave its tabs beyond reach. For as long as that connection lasts,
    /// a start runs only where no other host runs the project, as before
    /// instances; a registration with the echo brings joining back. Nothing
    /// running here is refused or stopped because of it.
    #[tokio::test]
    async fn without_the_servers_echo_a_project_starts_only_where_no_other_host_runs_it() {
        let policy = || Some(EffectiveProjectPolicy::default());
        let mut h = harness(ClaimMode::Instances, Ending::WhenStopped);
        for project_id in ["proj-shared", "proj-alone", "proj-resumed"] {
            h.add_project(project_id);
        }
        let peer = Uuid::new_v4();
        for project_id in ["proj-shared", "proj-resumed"] {
            write_foreign_claim(
                &h.instance_path(project_id, peer),
                "peer.invalid",
                peer,
                now_secs(),
                true,
            );
            write_foreign_claim(
                &h.guard_path(project_id),
                "peer.invalid",
                peer,
                now_secs(),
                true,
            );
        }
        h.state.note_registration(&[]);

        // Refused beside the peer, with the reason as its last error, and
        // nothing claimed.
        let request_id = Uuid::new_v4();
        let replies = h.request_start("proj-shared", policy(), Some(request_id));
        let (id, _, success, error) = outcome_of(&replies).unwrap();
        assert_eq!((id, success), (request_id, false));
        let error = error.unwrap();
        assert!(
            error.contains("peer.invalid is running this project"),
            "{error}"
        );
        assert!(
            error.contains("cannot run a project on more than one machine"),
            "{error}"
        );
        let entry = inventory_entry(&replies, "proj-shared");
        assert_eq!(entry.last_error.as_deref(), Some(error.as_str()));
        assert!(!entry.is_running);
        assert!(!h.state.running.contains_key("proj-shared"));
        assert!(!h.instance_path("proj-shared", h.machine()).exists());
        assert_eq!(
            h.read(&h.guard_path("proj-shared"))["hostname"],
            serde_json::json!("peer.invalid")
        );

        // Where no other host runs it, it starts as daemons ran it before
        // instances: as the project's only runtime, which runs every pane, and
        // claimed as those daemons claim, with the guard alone and unmarked,
        // which keeps a capable daemon on another host out of it. Started as
        // an instance it would report only this machine's panes, and this
        // server would replace the project's roster with them. (Before this,
        // the start was an instance with an instance's claims.)
        let replies = h.request_start("proj-alone", policy(), Some(Uuid::new_v4()));
        let (_, _, success, error) = outcome_of(&replies).unwrap();
        assert!(success, "{error:?}");
        assert_eq!(h.launched_as_instance("proj-alone"), Some(false));
        assert!(!h.instance_path("proj-alone", h.machine()).exists());
        let guard = h.read(&h.guard_path("proj-alone"));
        assert_eq!(
            guard["hostname"],
            serde_json::json!(crate::daemon_registry::hostname())
        );
        assert!(guard.get("instances").is_none(), "{guard}");

        // Another host starting it meanwhile does not turn a further start of
        // what runs here into a refusal or a second launch, and the heartbeat
        // keeps its claims in their shape.
        let other = Uuid::new_v4();
        write_foreign_claim(
            &h.instance_path("proj-alone", other),
            "other.invalid",
            other,
            now_secs(),
            true,
        );
        let replies = h.request_start("proj-alone", policy(), Some(Uuid::new_v4()));
        let (_, _, success, error) = outcome_of(&replies).unwrap();
        assert!(success, "{error:?}");
        let entry = inventory_entry(&replies, "proj-alone");
        assert!(entry.is_running);
        assert_eq!(entry.last_error, None);
        h.state.reconcile_running_claims();
        assert!(!h.instance_path("proj-alone", h.machine()).exists());
        assert!(h
            .read(&h.guard_path("proj-alone"))
            .get("instances")
            .is_none());

        // What was running here when the daemon replaced itself, or when a
        // replacement failed, is never refused for what the server runs.
        // Another host runs this one, so it ran here as an instance beside
        // that host and starts as one again: run alone, it would run that
        // host's panes as well.
        assert_eq!(
            h.start_as("proj-resumed", StartScope::again(true)),
            StartOutcome::Started
        );
        assert_eq!(h.launched_as_instance("proj-resumed"), Some(true));
        assert!(h.instance_path("proj-resumed", h.machine()).exists());

        // Registered again with a server that runs instances: it joins, as an
        // instance.
        h.state
            .note_registration(&[shared::PROJECT_INSTANCES_CAPABILITY.to_string()]);
        let replies = h.request_start("proj-shared", policy(), Some(Uuid::new_v4()));
        let (_, _, success, error) = outcome_of(&replies).unwrap();
        assert!(success, "{error:?}");
        let entry = inventory_entry(&replies, "proj-shared");
        assert!(entry.is_running);
        assert_eq!(entry.last_error, None);
        assert!(h.instance_path("proj-shared", h.machine()).exists());
        assert_eq!(h.launched_as_instance("proj-shared"), Some(true));

        // What already runs here keeps running as it started, its claims in
        // their shape.
        let replies = h.request_start("proj-alone", policy(), Some(Uuid::new_v4()));
        assert!(outcome_of(&replies).unwrap().2);
        h.state.reconcile_running_claims();
        assert!(!h.instance_path("proj-alone", h.machine()).exists());
        assert!(h
            .read(&h.guard_path("proj-alone"))
            .get("instances")
            .is_none());
        assert_eq!(
            h.launches
                .lock()
                .unwrap()
                .iter()
                .filter(|(path, _)| path.ends_with("proj-alone"))
                .count(),
            1,
            "launched once"
        );

        // A daemon that runs projects alone anyway keeps its own reason.
        let mut h = harness(ClaimMode::Exclusive, Ending::WhenStopped);
        h.add_project("proj");
        write_foreign_claim(
            &h.guard_path("proj"),
            "peer.invalid",
            peer,
            now_secs(),
            true,
        );
        h.state.note_registration(&[]);
        let replies = h.request_start("proj", policy(), Some(Uuid::new_v4()));
        let (_, _, success, error) = outcome_of(&replies).unwrap();
        assert!(!success);
        assert!(error
            .unwrap()
            .contains("config.toml sets daemon.machine_id"));
    }

    /// A project the daemon starts again after replacing itself, or after a
    /// replacement failed, runs as the server the daemon was connected to
    /// allowed: alone where that server kept one CLI per session and no other
    /// host runs the project, so the server's roster keeps every tab, and as
    /// an instance where the server runs instances, which is what the
    /// "Reboot to update" a server asks for has to bring back. The heartbeat
    /// keeps each start's claims in its own shape, side by side, and a stop
    /// releases either.
    #[tokio::test]
    async fn a_project_started_again_runs_as_its_server_allowed() {
        let mut h = harness(ClaimMode::Instances, Ending::WhenStopped);
        for project_id in ["proj-alone", "proj-instance"] {
            h.add_project(project_id);
        }

        assert_eq!(
            h.start_as("proj-alone", StartScope::again(true)),
            StartOutcome::Started
        );
        assert_eq!(h.launched_as_instance("proj-alone"), Some(false));
        assert!(!h.instance_path("proj-alone", h.machine()).exists());
        assert!(h
            .read(&h.guard_path("proj-alone"))
            .get("instances")
            .is_none());

        assert_eq!(
            h.start_as("proj-instance", StartScope::again(false)),
            StartOutcome::Started
        );
        assert_eq!(h.launched_as_instance("proj-instance"), Some(true));
        assert!(h.instance_path("proj-instance", h.machine()).exists());
        assert_eq!(
            h.read(&h.guard_path("proj-instance"))["instances"],
            serde_json::json!(true)
        );

        let me = crate::daemon_registry::hostname();
        let old = now_secs() - 60;
        for project_id in ["proj-alone", "proj-instance"] {
            write_foreign_claim(&h.guard_path(project_id), &me, h.machine(), old, true);
        }
        h.state.reconcile_running_claims();
        let alone = h.read(&h.guard_path("proj-alone"));
        assert!(alone["heartbeat"].as_u64().unwrap() >= old + 60);
        assert!(alone.get("instances").is_none(), "{alone}");
        assert!(!h.instance_path("proj-alone", h.machine()).exists());
        let instance = h.read(&h.guard_path("proj-instance"));
        assert!(instance["heartbeat"].as_u64().unwrap() >= old + 60);
        assert_eq!(instance["instances"], serde_json::json!(true));
        assert!(h.instance_path("proj-instance", h.machine()).exists());

        for project_id in ["proj-alone", "proj-instance"] {
            h.state.stop_project(project_id).await.unwrap();
            assert!(!h.guard_path(project_id).exists(), "{project_id}");
            assert!(
                !h.instance_path(project_id, h.machine()).exists(),
                "{project_id}"
            );
        }
    }

    /// The replacement resumes before it has heard from any server, so the
    /// daemon it replaces writes down whether its server keeps one CLI per
    /// session, for every project it resumes, the one that asked for the
    /// restart included. A daemon that runs projects alone anyway has nothing
    /// to write.
    #[tokio::test]
    async fn a_replacement_is_told_whether_its_server_kept_one_cli_per_session() {
        let single_machine_server = |state: &DaemonState| -> Vec<bool> {
            state
                .resume_projects_after_requested_restart("proj")
                .iter()
                .map(|project| project.single_machine_server)
                .collect()
        };
        let mut h = harness(ClaimMode::Instances, Ending::WhenStopped);
        h.add_project("proj");
        assert_eq!(h.start("proj"), StartOutcome::Started);
        assert_eq!(single_machine_server(&h.state), vec![false]);
        h.state.note_registration(&[]);
        assert_eq!(single_machine_server(&h.state), vec![true]);
        assert_eq!(
            h.state
                .running_projects_for_resume()
                .iter()
                .map(|project| project.single_machine_server)
                .collect::<Vec<_>>(),
            vec![true]
        );
        h.state.stop_project("proj").await.unwrap();
        assert_eq!(
            single_machine_server(&h.state),
            vec![true],
            "the project that asked is listed although its task ended"
        );

        let mut h = harness(ClaimMode::Exclusive, Ending::WhenStopped);
        h.add_project("proj");
        assert_eq!(h.start("proj"), StartOutcome::Started);
        h.state.note_registration(&[]);
        assert_eq!(single_machine_server(&h.state), vec![false]);
    }

    /// `ProjectRuntimeStarted` answers every correlated request exactly once,
    /// after the inventory, on every path a start can take.
    #[tokio::test]
    async fn every_start_outcome_is_reported_once_after_the_inventory() {
        let policy = || Some(EffectiveProjectPolicy::default());

        // Refusals: each also becomes the project's last_error, which the
        // inventory sent first already carries.
        let refusals: Vec<(
            &str,
            Box<dyn Fn(&mut Harness) -> Option<EffectiveProjectPolicy>>,
        )> = vec![
            ("cluster policy", Box::new(|_| None)),
            (
                "suspended",
                Box::new(|_| {
                    Some(EffectiveProjectPolicy {
                        project_suspended: true,
                        ..EffectiveProjectPolicy::default()
                    })
                }),
            ),
            (
                "reboot older-peer.invalid to update it",
                Box::new(|h| {
                    let guard = h.guard_path("proj");
                    write_foreign_claim(
                        &guard,
                        "older-peer.invalid",
                        Uuid::new_v4(),
                        now_secs(),
                        false,
                    );
                    Some(EffectiveProjectPolicy::default())
                }),
            ),
        ];
        for (expected, setup) in refusals {
            let mut h = harness(ClaimMode::Instances, Ending::WhenStopped);
            h.add_project("proj");
            let policy = setup(&mut h);
            let request_id = Uuid::new_v4();
            let replies = h.request_start("proj", policy, Some(request_id));
            let (id, project_id, success, error) = outcome_of(&replies).unwrap();
            assert_eq!(
                (id, project_id.as_str(), success),
                (request_id, "proj", false)
            );
            let error = error.unwrap();
            assert!(error.contains(expected), "{expected}: {error}");
            let entry = inventory_entry(&replies, "proj");
            assert_eq!(entry.last_error.as_deref(), Some(error.as_str()));
            assert!(!entry.is_running);
            assert!(h.state.running.is_empty());
            assert!(!h.instance_path("proj", h.machine()).exists(), "{expected}");
        }

        // An external `--headless` run already owns the project here.
        let mut h = harness(ClaimMode::Instances, Ending::WhenStopped);
        let path = h.add_project("proj");
        let _external = ExternalHeadlessRun::start(&path);
        let replies = h.request_start("proj", policy(), Some(Uuid::new_v4()));
        let (_, _, success, error) = outcome_of(&replies).unwrap();
        assert!(!success);
        assert!(error.unwrap().contains("--headless"));
        assert!(h.state.running.is_empty());
        assert!(
            !h.instance_path("proj", h.machine()).exists(),
            "a standalone run takes no claim, and none is made for it"
        );

        // A project this daemon does not know.
        let mut h = harness(ClaimMode::Instances, Ending::WhenStopped);
        let replies = h.request_start("proj-unknown", policy(), Some(Uuid::new_v4()));
        let (_, _, success, error) = outcome_of(&replies).unwrap();
        assert!(!success);
        assert!(error.unwrap().contains("Unknown project id"));

        // Started, then asked again while running: both succeed.
        let mut h = harness(ClaimMode::Instances, Ending::WhenStopped);
        h.add_project("proj");
        for _ in 0..2 {
            let replies = h.request_start("proj", policy(), Some(Uuid::new_v4()));
            let (_, _, success, error) = outcome_of(&replies).unwrap();
            assert!(success, "{error:?}");
            assert_eq!(error, None);
            let entry = inventory_entry(&replies, "proj");
            assert!(entry.is_running);
            assert_eq!(entry.last_error, None);
        }
        assert_eq!(h.state.running.len(), 1);

        // A start nobody correlated asks for no report.
        let mut h = harness(ClaimMode::Instances, Ending::WhenStopped);
        h.add_project("proj");
        let replies = h.request_start("proj", policy(), None);
        assert_eq!(replies.len(), 1);
        assert!(outcome_of(&replies).is_none());
        assert!(inventory_entry(&replies, "proj").is_running);
    }

    /// A successful start clears an earlier refusal from `last_error`.
    #[tokio::test]
    async fn a_successful_start_clears_an_earlier_refusal() {
        let mut h = harness(ClaimMode::Instances, Ending::WhenStopped);
        h.add_project("proj");
        let replies = h.request_start("proj", None, None);
        assert!(inventory_entry(&replies, "proj").last_error.is_some());
        let replies = h.request_start("proj", Some(EffectiveProjectPolicy::default()), None);
        assert_eq!(inventory_entry(&replies, "proj").last_error, None);
    }

    /// A project whose runtime ends on its own for a reason reports it as its
    /// last error once its task has ended: a session the server rejected,
    /// beside an older CLI on another machine say, or a runtime that failed.
    /// Its start was a success, since the runtime did start, so nothing else
    /// says why it is not running. A clean end reports nothing, a refusal
    /// recorded afterwards is reported instead, and the next start clears it.
    #[tokio::test]
    async fn a_project_that_ends_for_a_reason_reports_it_as_its_last_error() {
        let policy = || Some(EffectiveProjectPolicy::default());
        for (ending, expected) in [
            (Ending::Stopped, Some("session rejected")),
            (Ending::Failed, Some("could not reach the server")),
            (Ending::Completed, None),
        ] {
            let mut h = harness(ClaimMode::Instances, ending);
            h.add_project("proj");
            let replies = h.request_start("proj", policy(), Some(Uuid::new_v4()));
            let (_, _, success, error) = outcome_of(&replies).unwrap();
            assert!(success, "{ending:?}: the runtime started: {error:?}");
            h.wait_until_ended("proj").await;

            let entry = h
                .state
                .snapshot_projects()
                .into_iter()
                .find(|project| project.project_id == "proj")
                .unwrap();
            assert!(!entry.is_running, "{ending:?}");
            match expected {
                Some(expected) => {
                    let error = entry
                        .last_error
                        .unwrap_or_else(|| panic!("{ending:?}: no last error"));
                    assert!(error.contains(expected), "{ending:?}: {error}");

                    let replies = h.request_start("proj", None, None);
                    let error = inventory_entry(&replies, "proj").last_error.unwrap();
                    assert!(error.contains("cluster policy"), "{ending:?}: {error}");
                }
                None => assert_eq!(entry.last_error, None, "{ending:?}"),
            }

            h.state.launcher = launcher(Ending::WhenStopped);
            let replies = h.request_start("proj", policy(), None);
            let entry = inventory_entry(&replies, "proj");
            assert!(entry.is_running, "{ending:?}");
            assert_eq!(entry.last_error, None, "{ending:?}");
        }
    }

    /// A host whose machine id is configured shares it with every host on its
    /// home: it runs projects alone, says so to the server, and refuses to
    /// start beside a capable instance elsewhere.
    #[tokio::test]
    async fn a_configured_machine_id_runs_projects_alone() {
        let configured = Some("6ba7b811-9dad-11d1-80b4-00c04fd430c8");
        assert_eq!(
            ClaimMode::for_configured_machine_id(configured),
            ClaimMode::Exclusive
        );
        assert!(!daemon_capabilities(ClaimMode::Exclusive)
            .contains(&shared::PROJECT_INSTANCES_CAPABILITY.to_string()));
        let capable = daemon_capabilities(ClaimMode::for_configured_machine_id(None));
        assert!(capable.contains(&shared::PROJECT_INSTANCES_CAPABILITY.to_string()));
        assert_eq!(
            capable.len(),
            daemon_capabilities(ClaimMode::Exclusive).len() + 1,
            "nothing else differs"
        );

        let mut h = harness(ClaimMode::Exclusive, Ending::WhenStopped);
        h.add_project("proj");
        let peer = Uuid::new_v4();
        write_foreign_claim(
            &h.guard_path("proj"),
            "peer.invalid",
            peer,
            now_secs(),
            true,
        );
        let replies = h.request_start(
            "proj",
            Some(EffectiveProjectPolicy::default()),
            Some(Uuid::new_v4()),
        );
        let (_, _, success, error) = outcome_of(&replies).unwrap();
        assert!(!success);
        assert!(error
            .unwrap()
            .contains("config.toml sets daemon.machine_id"));

        // Alone, it holds the guard unmarked, exactly as before instances.
        std::fs::remove_file(h.guard_path("proj")).unwrap();
        assert_eq!(h.start("proj"), StartOutcome::Started);
        let guard = h.read(&h.guard_path("proj"));
        assert!(guard.get("instances").is_none(), "{guard}");
        assert!(!h.config.path().join("claims").join("proj").exists());
    }

    fn git(args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn checkout_with_origin(dir: &Path, origin: &str) {
        let path = dir.to_str().unwrap();
        git(&["init", "-q", path]);
        git(&["-C", path, "remote", "add", "origin", origin]);
    }

    /// A read that never returns must not hold up the inventory: it reports
    /// nothing for that checkout until the read completes, and no second read
    /// piles up behind the first.
    #[tokio::test]
    async fn a_slow_git_read_never_holds_up_the_inventory() {
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let wait = Mutex::new(wait);
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = reads.clone();
        let remotes = GitRemotes::with_reader(Arc::new(move |_path: &Path| {
            counter.fetch_add(1, Ordering::SeqCst);
            let _ = wait.lock().unwrap().recv();
            Some("github.com/owner/repo".to_string())
        }));
        let path = PathBuf::from("/checkouts/repo");
        let read = remotes.refresh(vec![path.clone()]).expect("a read starts");
        assert_eq!(
            remotes.get(&path),
            None,
            "nothing known yet, and nothing waited for"
        );
        assert!(
            remotes.refresh(vec![path.clone()]).is_none(),
            "one read at a time"
        );

        release.send(()).unwrap();
        read.await.unwrap();
        assert_eq!(remotes.get(&path).as_deref(), Some("github.com/owner/repo"));
        assert!(
            remotes.refresh(vec![path.clone()]).is_none(),
            "still current"
        );
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn old_remotes_are_read_again_and_unregistered_checkouts_forgotten() {
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = reads.clone();
        let mut remotes = GitRemotes::with_reader(Arc::new(move |path: &Path| {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            Some(format!(
                "github.com/owner/{}-{n}",
                path.file_name()?.to_str()?
            ))
        }));
        remotes.max_age = Duration::ZERO;
        let (a, b) = (PathBuf::from("/c/a"), PathBuf::from("/c/b"));
        remotes
            .refresh(vec![a.clone(), b.clone()])
            .unwrap()
            .await
            .unwrap();
        assert!(remotes.get(&a).is_some() && remotes.get(&b).is_some());

        remotes.refresh(vec![a.clone()]).unwrap().await.unwrap();
        assert_eq!(remotes.get(&b), None, "no longer registered");
        assert_eq!(reads.load(Ordering::SeqCst), 3, "a read again once old");
    }

    /// End to end with a real checkout: the inventory carries the canonical
    /// remote that `SessionStart` would, so a project that has never run still
    /// files under its repository.
    #[test]
    fn the_inventory_reports_each_checkouts_canonical_remote() {
        crate::project::test_support::with_isolated_config(|| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let mut h = harness(ClaimMode::Instances, Ending::WhenStopped);
                let path = h.add_project("proj");
                std::fs::create_dir_all(&path).unwrap();
                checkout_with_origin(&path, "git@github.com:Owner/Repo.git");
                h.add_project("not-a-checkout");

                let before = h.state.snapshot_projects();
                assert!(before.iter().all(|project| project.git_remote.is_none()));
                h.state.refresh_git_remotes().unwrap().await.unwrap();
                let after = h.state.snapshot_projects();
                let remote = |id: &str| {
                    after
                        .iter()
                        .find(|project| project.project_id == id)
                        .unwrap()
                        .git_remote
                        .clone()
                };
                assert_eq!(remote("proj").as_deref(), Some("github.com/owner/repo"));
                assert_eq!(remote("not-a-checkout"), None);
            });
        });
    }

    /// Registering a folder answers with its remote straight away, rather
    /// than after the next background read.
    #[test]
    fn a_registered_checkout_reports_its_remote_at_once() {
        crate::project::test_support::with_isolated_config(|| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                let dir = tempfile::tempdir().unwrap();
                checkout_with_origin(dir.path(), "https://github.com/Owner/Repo.git");
                let mut h = harness(ClaimMode::Instances, Ending::WhenStopped);
                let (project, inventory) = h
                    .state
                    .register_local_project(dir.path().to_str().unwrap().to_string())
                    .await
                    .unwrap();
                assert_eq!(project.git_remote.as_deref(), Some("github.com/owner/repo"));
                assert_eq!(
                    inventory[0].git_remote.as_deref(),
                    Some("github.com/owner/repo")
                );
            });
        });
    }
}

#[cfg(test)]
mod join_tests {
    use super::*;

    /// Run `git` in `dir` without the user's configuration, and return what it
    /// printed.
    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=apas-test",
                "-c",
                "user.email=test@apas.invalid",
            ])
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("utf-8")
    }

    /// A repository to clone from, with one commit on `main`, plus whatever
    /// `extra` commits beside it.
    fn origin(root: &Path, extra: &[(&str, &[u8])]) -> PathBuf {
        let origin = root.join("origin");
        std::fs::create_dir_all(&origin).expect("origin dir");
        git(&origin, &["init", "-q", "-b", "main"]);
        std::fs::write(origin.join("README"), b"hello").expect("write");
        for (path, bytes) in extra {
            let path = origin.join(path);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
            std::fs::write(path, bytes).expect("write");
        }
        git(&origin, &["add", "-A"]);
        git(&origin, &["commit", "-q", "-m", "first"]);
        origin
    }

    /// A daemon registered with a server that runs instances, the only kind
    /// that asks for a join.
    fn state(mode: ClaimMode) -> DaemonState {
        let mut state = DaemonState::new(
            MachineInfo {
                machine_id: Uuid::new_v4(),
                hostname: "join-test".into(),
                os: "linux".into(),
                arch: "x86_64".into(),
                daemon_version: None,
                deepseek_backend: None,
                last_seen: None,
                providers: None,
            },
            mode,
        );
        state.note_registration(&[shared::PROJECT_INSTANCES_CAPABILITY.to_string()]);
        state
    }

    /// Ask `state` to join `project_id` with a clone of `origin` named `repo`
    /// under `base`, as the message loop does: on the runtime, with the clone
    /// on a blocking thread.
    fn join(
        state: &mut DaemonState,
        origin: &Path,
        base: &Path,
        project_id: Option<&str>,
        mode: ProjectProvisioningMode,
    ) -> Vec<DaemonToServer> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
            state
                .handle_join_instance(
                    "github.com/owner/repo",
                    "repo",
                    Some(origin.to_str().expect("utf-8")),
                    Some(base.to_str().expect("utf-8")),
                    Some("join-request".to_string()),
                    project_id,
                    mode,
                    &mut heartbeat,
                )
                .await
        })
    }

    /// The inventory and, after it, the acknowledgement, as (inventory,
    /// project_id, path, error).
    fn acknowledgement(
        replies: &[DaemonToServer],
    ) -> (
        &[MachineProjectInfo],
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        match replies {
            [DaemonToServer::Heartbeat { projects }, DaemonToServer::ProjectInstanceCreated {
                request_id,
                project_id,
                path,
                error,
            }] => {
                assert_eq!(request_id.as_deref(), Some("join-request"));
                (projects, project_id.clone(), path.clone(), error.clone())
            }
            other => panic!("the inventory, then the acknowledgement: {other:?}"),
        }
    }

    /// The checkout takes the project's identity with no panes, is registered
    /// and listed with its remote before the acknowledgement, keeps the
    /// branch it was cloned on, and is not started.
    #[test]
    fn a_join_clones_registers_and_lists_the_checkout_before_acknowledging() {
        crate::project::test_support::with_isolated_config(|| {
            let root = tempfile::tempdir().expect("temp dir");
            let origin = origin(root.path(), &[]);
            let base = root.path().join("projects");
            let mut state = state(ClaimMode::Instances);
            let project = Uuid::new_v4();

            let replies = join(
                &mut state,
                &origin,
                &base,
                Some(&project.to_string()),
                ProjectProvisioningMode::TrustedOwner,
            );
            let (inventory, project_id, path, error) = acknowledgement(&replies);
            assert_eq!(error, None);
            assert_eq!(project_id, Some(project.to_string()));
            let path = PathBuf::from(path.expect("path"));
            assert_eq!(
                path,
                std::fs::canonicalize(base.join("repo")).expect("checkout")
            );

            let listed = inventory
                .iter()
                .find(|entry| entry.project_id == project.to_string())
                .expect("the inventory sent first lists the checkout");
            assert_eq!(listed.path, path.to_str().expect("utf-8"));
            assert!(!listed.is_running);
            assert!(listed.git_remote.is_some());
            assert_eq!(
                listed.git_remote,
                crate::worktree::normalized_git_remote(&path)
            );

            let metadata =
                crate::project_instances::read_project_metadata(&path).expect("metadata");
            assert_eq!(metadata.id, project);
            assert!(metadata.panes.is_empty());
            let registered = crate::project::list_registered_projects().expect("registry");
            assert_eq!(registered.len(), 1);
            assert_eq!(registered[0].project_id, project.to_string());
            assert_eq!(registered[0].path, path.to_str().expect("utf-8"));

            assert_eq!(
                git(&path, &["branch", "--format=%(refname:short)"]).trim(),
                "main",
                "no branch of its own"
            );
            assert!(state.running.is_empty(), "registered, not started");
        });
    }

    /// A repository that commits a `.apas` must not hand the checkout that
    /// file's identity or panes.
    #[test]
    fn a_join_replaces_a_committed_metadata_file() {
        crate::project::test_support::with_isolated_config(|| {
            let root = tempfile::tempdir().expect("temp dir");
            let committed = serde_json::json!({
                "id": Uuid::new_v4(), "name": "committed", "created_at": "2026-01-01T00:00:00Z",
                "panes": [{
                    "pane_id": 3, "provider": "codex", "mode": "interactive",
                    "session_id": Uuid::new_v4(),
                }],
            });
            let committed = serde_json::to_vec_pretty(&committed).expect("json");
            let origin = origin(root.path(), &[(".apas", &committed)]);
            let mut state = state(ClaimMode::Instances);
            let project = Uuid::new_v4();

            let replies = join(
                &mut state,
                &origin,
                &root.path().join("projects"),
                Some(&project.to_string()),
                ProjectProvisioningMode::TrustedOwner,
            );
            let (_, _, path, error) = acknowledgement(&replies);
            assert_eq!(error, None);
            let metadata =
                crate::project_instances::read_project_metadata(Path::new(&path.expect("path")))
                    .expect("metadata");
            assert_eq!(metadata.id, project);
            assert!(metadata.panes.is_empty());
        });
    }

    /// A join that fails before registration removes its clone, as a new
    /// instance's does, and registers nothing.
    #[test]
    fn a_join_that_fails_before_registration_leaves_nothing_behind() {
        crate::project::test_support::with_isolated_config(|| {
            let root = tempfile::tempdir().expect("temp dir");
            let base = root.path().join("projects");
            let mut state = state(ClaimMode::Instances);
            let project = Uuid::new_v4().to_string();

            // The clone fails.
            let replies = join(
                &mut state,
                &root.path().join("no-such-repository"),
                &base,
                Some(&project),
                ProjectProvisioningMode::TrustedOwner,
            );
            let (_, project_id, path, error) = acknowledgement(&replies);
            assert!(error.is_some());
            assert_eq!((project_id, path), (None, None));
            assert!(!base.join("repo").exists());

            // The clone succeeds, but the checkout's metadata cannot be written:
            // the repository commits a directory where `.apas` goes.
            let origin = origin(root.path(), &[(".apas/keep", b"")]);
            let replies = join(
                &mut state,
                &origin,
                &base,
                Some(&project),
                ProjectProvisioningMode::TrustedOwner,
            );
            let (inventory, _, _, error) = acknowledgement(&replies);
            assert!(error.is_some());
            assert!(!base.join("repo").exists(), "the partial clone is removed");
            assert!(inventory.is_empty());
            assert!(crate::project::list_registered_projects()
                .expect("registry")
                .is_empty());
        });
    }

    /// A registry holds one folder per project, and on a home shared between
    /// machines the project's checkout is already in reach: start it there.
    #[test]
    fn a_join_is_refused_for_a_project_this_machine_already_has() {
        crate::project::test_support::with_isolated_config(|| {
            let root = tempfile::tempdir().expect("temp dir");
            let existing = root.path().join("existing");
            std::fs::create_dir_all(&existing).expect("existing checkout");
            let (registered, _) =
                crate::project::register_local_project(existing.to_str().expect("utf-8"))
                    .expect("register");
            let origin = origin(root.path(), &[]);
            let base = root.path().join("projects");
            let mut state = state(ClaimMode::Instances);

            let replies = join(
                &mut state,
                &origin,
                &base,
                Some(&registered.project_id),
                ProjectProvisioningMode::TrustedOwner,
            );
            let (_, _, _, error) = acknowledgement(&replies);
            let error = error.expect("refused");
            assert!(error.contains(&registered.path), "{error}");
            assert!(!base.exists(), "refused before cloning");
            assert_eq!(
                crate::project::list_registered_projects()
                    .expect("registry")
                    .len(),
                1
            );
        });
    }

    /// Joining is for the owner's own machines, on a daemon that runs
    /// instances through a server that runs them too, and names a valid
    /// project.
    #[test]
    fn only_a_capable_owner_join_naming_a_project_clones_anything() {
        crate::project::test_support::with_isolated_config(|| {
            let root = tempfile::tempdir().expect("temp dir");
            let origin = origin(root.path(), &[]);
            let base = root.path().join("projects");
            let project = Uuid::new_v4().to_string();
            let refusals = [
                (
                    ClaimMode::Instances,
                    Some(project.as_str()),
                    ProjectProvisioningMode::PublicGithub,
                    "owner",
                ),
                (
                    ClaimMode::Exclusive,
                    Some(project.as_str()),
                    ProjectProvisioningMode::TrustedOwner,
                    "daemon.machine_id",
                ),
                (
                    ClaimMode::Instances,
                    None,
                    ProjectProvisioningMode::TrustedOwner,
                    "names no project",
                ),
                (
                    ClaimMode::Instances,
                    Some("not-a-project-id"),
                    ProjectProvisioningMode::TrustedOwner,
                    "not a valid id",
                ),
            ];
            for (mode, project_id, provisioning, expected) in refusals {
                let mut state = state(mode);
                let replies = join(&mut state, &origin, &base, project_id, provisioning);
                let (_, _, _, error) = acknowledgement(&replies);
                let error = error.expect("refused");
                assert!(error.contains(expected), "{expected}: {error}");
                assert!(!base.exists(), "{expected}: nothing cloned");
            }

            // A server keeping one CLI per session predates joins and never
            // asks for one; were it to, the checkout could not run beside the
            // project's other machines.
            let mut state = state(ClaimMode::Instances);
            state.note_registration(&[]);
            let replies = join(
                &mut state,
                &origin,
                &base,
                Some(&project),
                ProjectProvisioningMode::TrustedOwner,
            );
            let (_, _, _, error) = acknowledgement(&replies);
            let error = error.expect("refused");
            assert!(
                error.contains("cannot run a project on more than one machine"),
                "{error}"
            );
            assert!(!base.exists(), "nothing cloned");

            assert!(crate::project::list_registered_projects()
                .expect("registry")
                .is_empty());
        });
    }

    #[test]
    fn the_shell_path_comes_first_and_nothing_inherited_is_lost() {
        assert_eq!(
            super::merge_paths("/opt/homebrew/bin:/usr/bin", "/usr/bin:/bin:/custom"),
            "/opt/homebrew/bin:/usr/bin:/bin:/custom"
        );
        assert_eq!(super::merge_paths("", "/usr/bin"), "/usr/bin");
        assert_eq!(super::merge_paths("/a::/a", ""), "/a");
    }
}
