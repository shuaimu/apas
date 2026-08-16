use anyhow::Result;
use futures::{SinkExt, StreamExt};
use shared::{
    DaemonToServer, DeepseekBackendInfo, MachineInfo, MachineProjectInfo, ServerToDaemon,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

/// How often the daemon checks whether a newer `apas` has been installed.
///
/// Deliberately slow. An upgrade is never urgent — the running daemon works
/// fine — and the check costs a `stat` plus, on change, spawning
/// `apas --version`. Tying it to the 10s heartbeat meant the version path was
/// exercised 360x more often than anything could benefit from.
const UPGRADE_CHECK_INTERVAL: Duration = Duration::from_secs(15 * 60);
const USAGE_REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
const VERSION: &str = env!("APAS_VERSION");
const TMUX_SESSION_PREFIX: &str = "apas";
const DEEPSEEK_API_BASE_URL: &str = "https://api.deepseek.com/anthropic";

fn resolve_user_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "/bin/bash".to_string());

    let output = Command::new(shell)
        .arg("-ic")
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
    let mut matches = Vec::new();

    let Ok(entries) = std::fs::read_dir("/proc") else {
        return matches;
    };
    for entry in entries.flatten() {
        let pid: u32 = match entry.file_name().to_str().and_then(|s| s.parse().ok()) {
            Some(pid) => pid,
            None => continue,
        };
        if pid == my_pid {
            continue;
        }
        let cmdline_path = entry.path().join("cmdline");
        let Ok(data) = std::fs::read(&cmdline_path) else {
            continue;
        };
        let args: Vec<&str> = data
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .filter_map(|s| std::str::from_utf8(s).ok())
            .collect();

        let is_apas = args.first().map_or(false, |a| a.contains("apas"));
        let has_headless = args.iter().any(|a| *a == "--headless");
        let has_dir = args
            .windows(2)
            .any(|w| w[0] == "-d" && w[1] == path_str.as_ref());
        if is_apas && has_headless && has_dir {
            matches.push(pid);
        }
    }
    matches
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

/// Read resident-set size (VmRSS) of a running process from /proc/<pid>/status,
/// returned in KiB. Returns None if the process is gone or /proc is unreadable.
fn read_process_rss_kb(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{}/status", pid)).ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            // "VmRSS:     12345 kB"
            let trimmed = rest.trim();
            let numeric = trimmed.split_whitespace().next()?;
            return numeric.parse::<u64>().ok();
        }
    }
    None
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

fn write_resume_manifest(project_ids: &[String]) {
    let Some(path) = resume_manifest_path() else {
        return;
    };
    write_resume_manifest_at(&path, project_ids);
}

/// The manifest logic, given its path. Split out so it is testable without
/// mutating process-wide environment variables — several test modules already
/// move `XDG_RUNTIME_DIR` under their own locks, and adding another writer
/// makes them race rather than serialise.
fn write_resume_manifest_at(path: &std::path::Path, project_ids: &[String]) {
    if project_ids.is_empty() {
        let _ = std::fs::remove_file(path);
        return;
    }
    match serde_json::to_vec(project_ids) {
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
fn take_resume_manifest() -> Vec<String> {
    match resume_manifest_path() {
        Some(path) => take_resume_manifest_at(&path),
        None => Vec::new(),
    }
}

fn take_resume_manifest_at(path: &std::path::Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let _ = std::fs::remove_file(path);
    serde_json::from_slice(&bytes).unwrap_or_default()
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
/// Headless project CLIs live in their own tmux sessions and are unaffected;
/// the replacement adopts them via `is_headless_running_for` + `tmux_has_session`.
///
/// Returns only on failure — on success the process image is gone.
#[cfg(unix)]
fn exec_into_newer_binary(version: &str, running: &[String]) -> std::io::Error {
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

/// What a requested daemon restart should do before replacing itself.
///
/// Separated from the replacement so the decision is testable: the `exec` that
/// follows never returns, which makes it the one part that cannot be asserted
/// on directly.
#[derive(Debug, PartialEq)]
enum RestartPlan {
    /// A newer version is available; install it, then replace.
    UpdateThenReplace(String),
    /// Already current; replace with the same binary.
    ReplaceInPlace,
}

fn plan_daemon_restart(available: Option<String>) -> RestartPlan {
    match available {
        Some(newer) => RestartPlan::UpdateThenReplace(newer),
        None => RestartPlan::ReplaceInPlace,
    }
}

/// Apply a requested restart. Returns only on failure — success replaces this
/// process image.
///
/// Every fallible step runs while this daemon is still serving, so a failed
/// update leaves a working daemon rather than a machine with none. That is
/// the same discipline `prepare_cli_restart` uses for project CLIs.
#[cfg(unix)]
fn perform_requested_restart(running: &[String]) -> anyhow::Error {
    match plan_daemon_restart(crate::update::check_for_update_available()) {
        RestartPlan::UpdateThenReplace(newer) => {
            tracing::info!(%newer, "daemon restart requested: preparing update before replacing");
            match crate::update::prepare_cli_restart() {
                Ok(_) => {}
                Err(err) => {
                    // Nothing has been replaced, so the machine keeps the
                    // daemon it had.
                    tracing::error!(%err, "daemon restart: update failed; staying on the current daemon");
                    return err;
                }
            }
            anyhow::Error::from(exec_into_newer_binary(&newer, running))
        }
        RestartPlan::ReplaceInPlace => {
            tracing::info!("daemon restart requested: already current, replacing in place");
            anyhow::Error::from(exec_into_newer_binary(env!("APAS_VERSION"), running))
        }
    }
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
}

/// A project asking to be replaced.
///
/// The task cannot restart itself — it is the thing being replaced, and only
/// the instance holds the table. It says so on this channel and ends; the
/// daemon starts a fresh task for it.
type RestartRequests = tokio::sync::mpsc::UnboundedSender<String>;

struct DaemonState {
    machine_info: MachineInfo,
    projects: HashMap<String, ProjectEntry>,
    sessions: HashMap<String, String>,
    /// Projects this instance is running, keyed by project id.
    running: HashMap<String, RunningProject>,
}

impl DaemonState {
    fn new(machine_info: MachineInfo) -> Self {
        Self {
            machine_info,
            projects: HashMap::new(),
            running: HashMap::new(),
            sessions: HashMap::new(),
        }
    }

    fn refresh_projects(&mut self) {
        let discovered = match crate::project::list_registered_projects() {
            Ok(projects) => projects,
            Err(err) => {
                tracing::warn!("Failed to read project registry: {}", err);
                Vec::new()
            }
        };
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
                !seen.contains(*project_id) && !self.sessions.contains_key(*project_id)
            })
            .cloned()
            .collect();
        for project_id in stale_ids {
            self.projects.remove(&project_id);
        }
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

    /// Claim every project this host is already running.
    ///
    /// Claims are otherwise only taken in `start_project`, so a daemon that
    /// restarts — or re-execs to self-upgrade — comes back owning nothing while
    /// its project CLIs keep running in their own tmux sessions. During that
    /// gap a peer sees the projects unclaimed, and its `is_headless_running_for`
    /// only reads its *own* `/proc`, so it cannot tell they are alive here: a
    /// `StartProjectCli` there would spawn a second CLI against the same `.apas`
    /// and worktrees. Reconciling at startup closes the window entirely.
    ///
    /// A peer holding the claim for something we are running is logged rather
    /// than seized. That combination means two CLIs are already live for one
    /// project, which is the thing claims exist to prevent — stealing the claim
    /// would hide it, and the operator needs to see it.
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

    fn reconcile_running_claims(&mut self) {
        let registry_dir = config_dir_for_registry();
        for (project_id, project) in &self.projects {
            if !is_headless_running_for(&project.path) {
                continue;
            }
            match crate::daemon_registry::claim_project(
                &registry_dir,
                project_id,
                &project.path.display().to_string(),
                self.machine_info.machine_id,
            ) {
                Ok(crate::daemon_registry::ClaimOutcome::Acquired) => {
                    tracing::info!(
                        project_id,
                        path = %project.path.display(),
                        "reclaimed a project that was already running here"
                    );
                }
                Ok(crate::daemon_registry::ClaimOutcome::AlreadyOurs) => {}
                Ok(crate::daemon_registry::ClaimOutcome::HeldBy(peer)) => {
                    tracing::warn!(
                        project_id,
                        peer = %peer.hostname,
                        heartbeat_age_secs = peer.age_secs,
                        "project is running here but claimed by a peer — two CLIs may be live for it"
                    );
                }
                Err(err) => {
                    tracing::warn!(project_id, %err, "could not reconcile project claim");
                }
            }
        }
    }

    fn snapshot_projects(&self) -> Vec<MachineProjectInfo> {
        let mut projects = Vec::with_capacity(self.projects.len());

        for (project_id, project) in &self.projects {
            // Held, not inferred. A project runs inside this process, so the
            // task table is the answer; `/proc` is consulted only to notice an
            // externally started `--headless` run, which is the debugging
            // escape hatch rather than the normal path.
            let running_here = self
                .running
                .get(project_id)
                .is_some_and(|project| !project.handle.is_finished());
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
                is_running: running_here || external.is_some(),
                pid,
                memory_kb,
                last_error: project.last_error.clone(),
            });
        }

        projects.sort_by(|a, b| a.path.cmp(&b.path));
        projects
    }

    fn start_project(
        &mut self,
        project_id: &str,
        server_url: &str,
        token: &str,
        restart_tx: &RestartRequests,
    ) -> Result<()> {
        // Reap any exited tracked processes before deciding whether to spawn.
        self.reap_exited_processes();

        // Already running here: the table answers this, not a tmux session we
        // would have to go and look for.
        if self
            .running
            .get(project_id)
            .is_some_and(|project| !project.handle.is_finished())
        {
            return Ok(());
        }
        // A task that ended leaves an entry behind; drop it so the project can
        // be started again without manual cleanup.
        self.running.remove(project_id);

        let project = self
            .projects
            .get_mut(project_id)
            .ok_or_else(|| anyhow::anyhow!("Unknown project id: {}", project_id))?;

        // Cross-host guard first. `is_headless_running_for` below only reads
        // the local /proc, so on a shared-NFS cluster it cannot see a headless
        // CLI another host already started for this same project -- and both
        // daemons would spawn one, writing the same .apas and worktrees.
        // The claim file is how peers tell each other who owns what.
        match crate::daemon_registry::claim_project(
            &config_dir_for_registry(),
            project_id,
            &project.path.display().to_string(),
            self.machine_info.machine_id,
        ) {
            Ok(crate::daemon_registry::ClaimOutcome::HeldBy(peer)) => {
                tracing::info!(
                    project_id,
                    peer = %peer.hostname,
                    heartbeat_age_secs = peer.age_secs,
                    "project is running on another host; skipping spawn"
                );
                return Ok(());
            }
            Ok(_) => {}
            Err(err) => {
                // A claim we cannot write is not worth failing the spawn over;
                // the local /proc guard below still applies.
                tracing::warn!(project_id, %err, "could not record project claim");
            }
        }

        // An externally started `--headless -d <path>` run — the standalone
        // debugging path, or a project left over from the process-per-project
        // model. It is not ours to supervise, but starting a second owner for
        // the same `.apas` and worktrees is exactly what must not happen.
        if is_headless_running_for(&project.path) {
            tracing::info!(
                project_id,
                "project already has an external headless run; not starting a second"
            );
            return Ok(());
        }

        // The project runs here, as a task of this instance. It used to be a
        // headless CLI in its own tmux session, which this daemon then could
        // not see — running state was inferred from /proc, and a restarted
        // daemon came back owning nothing.
        //
        // Blocking work inside a project stays on its own threads. Nothing
        // here may block the runtime, or one project would stall the others.
        let project_path = project.path.clone();
        let project_shutdown = Arc::new(AtomicBool::new(false));
        let task_shutdown = project_shutdown.clone();
        let server = server_url.to_string();
        let token = token.to_string();
        let id_for_task = project_id.to_string();
        let restart_tx = restart_tx.clone();
        let handle = tokio::spawn(async move {
            // Every record this project emits carries its identity: sharing a
            // process took away the per-project tmux session and log file that
            // used to separate them for free.
            let span = tracing::info_span!("project", id = %id_for_task);
            let _entered = span.enter();
            match crate::mode::dual_pane::run_project(
                &server,
                &token,
                &project_path,
                task_shutdown,
            )
            .await
            {
                Ok(crate::mode::dual_pane::ProjectOutcome::Completed) => {
                    tracing::info!("project stopped");
                }
                Ok(crate::mode::dual_pane::ProjectOutcome::Stopped(reason)) => {
                    // One project's fatal condition, contained: it used to end
                    // the process, which would now end every other project.
                    tracing::error!(%reason, "project stopped");
                }
                Ok(crate::mode::dual_pane::ProjectOutcome::RebootRequested) => {
                    tracing::info!("project asked to restart");
                    // Replacing one project no longer replaces the process,
                    // which is what a reboot used to mean when a process was
                    // a project.
                    let _ = restart_tx.send(id_for_task.clone());
                }
                Err(err) => {
                    tracing::error!(%err, "project ended with an error");
                }
            }
        });

        project.last_error = None;
        self.running.insert(
            project_id.to_string(),
            RunningProject {
                shutdown: project_shutdown,
                handle,
            },
        );
        tracing::info!(project_id, "started project in this instance");
        Ok(())
    }

    /// Stop a project and wait for it to finish tearing down.
    ///
    /// Bounded, because teardown kills pane children and joins roughly thirty
    /// threads: a project that will not stop must not hold the instance that
    /// every other project is running in.
    async fn stop_running_project(&mut self, project_id: &str) {
        let Some(project) = self.running.remove(project_id) else {
            return;
        };
        project.shutdown.store(true, Ordering::SeqCst);
        match tokio::time::timeout(std::time::Duration::from_secs(30), project.handle).await {
            Ok(Ok(())) => tracing::info!(project_id, "project stopped"),
            Ok(Err(err)) => {
                // A panicking project unwinds its own task. Reported, and the
                // instance and every other project carry on.
                tracing::error!(project_id, %err, "project ended abnormally");
            }
            Err(_) => tracing::warn!(
                project_id,
                "project did not stop within 30s; abandoning the wait"
            ),
        }
    }

    async fn stop_project(&mut self, project_id: &str) -> Result<()> {
        // Stop the task first: it owns the panes, and letting it tear down
        // before the pane hosts are shut means it can do so cleanly.
        self.stop_running_project(project_id).await;
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
    ) -> Result<(String, String)> {
        // Projects root: an explicit base_path (with leading ~ expanded against
        // THIS machine's $HOME), else ~/apas_projects.
        let root = match base_path {
            Some(p) if !p.trim().is_empty() => expand_tilde(p.trim()),
            _ => dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("could not resolve home directory"))?
                .join("apas_projects"),
        };
        let name = crate::worktree::sanitize_instance_name(instance_name);
        let dest = crate::worktree::unique_dir(&root.join(&name));

        // Make sure the projects root exists before cloning into it.
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let url = self.resolve_clone_url(git_remote, clone_url);

        // Clone + branch + register. If any of these PRE-registration steps
        // fail, remove the partial clone so retries don't accumulate orphan
        // directories (unique_dir would otherwise auto-suffix forever).
        let safe_branch = crate::worktree::sanitize_branch_name(branch);
        let metadata = match (|| {
            crate::worktree::clone_repo(&url, &dest)?;
            let created_branch = crate::worktree::checkout_unique_branch(&dest, &safe_branch)?;
            tracing::info!(
                "Cloned {} into {} on branch {}",
                url,
                dest.display(),
                created_branch
            );
            crate::project::get_or_create_project(&dest)
        })() {
            Ok(metadata) => metadata,
            Err(err) => {
                let _ = std::fs::remove_dir_all(&dest);
                return Err(err);
            }
        };

        // The instance now exists and is registered. A start failure is NOT a
        // create failure — surface it as a warning (the project shows on the
        // machines page with last_error and can be started there) so the user
        // doesn't get a "failed" toast for an instance that was created.
        let project_id = metadata.id.to_string();
        self.refresh_projects();
        if let Err(err) = self.start_project(&project_id, server_url, token, restart_tx) {
            tracing::warn!(
                "Created instance {} but failed to auto-start it: {}",
                project_id,
                err
            );
        }

        Ok((project_id, dest.to_string_lossy().to_string()))
    }

    /// Resolve a cloneable URL for the canonical `git_remote` (host/owner/repo):
    /// honor an explicit `clone_url`, else reuse the exact `origin` of an
    /// existing checkout of the same repo on this machine (preserves SSH/auth),
    /// else reconstruct an https URL from the key.
    fn resolve_clone_url(&self, git_remote: &str, clone_url: Option<&str>) -> String {
        if let Some(u) = clone_url {
            let u = u.trim();
            if !u.is_empty() {
                return u.to_string();
            }
        }
        for project in self.projects.values() {
            if crate::worktree::normalized_git_remote(&project.path).as_deref() == Some(git_remote)
            {
                if let Some(raw) = crate::worktree::raw_git_remote(&project.path) {
                    return raw;
                }
            }
        }
        format!("https://{}.git", git_remote.trim_matches('/'))
    }
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
        if std::env::var("PATH").ok().as_deref() != Some(path.as_str()) {
            tracing::info!("using the login shell PATH so projects can find their providers");
            std::env::set_var("PATH", &path);
        }
    }

    let hostname = hostname::get()
        .ok()
        .and_then(|value| value.into_string().ok())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    let config = crate::config::Config::load().unwrap_or_default();
    let machine_info = MachineInfo {
        machine_id,
        hostname,
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        daemon_version: Some(VERSION.to_string()),
        deepseek_backend: deepseek_backend_info_from_config(&config),
        last_seen: None,
    };

    let registry = crate::project::project_registry_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "~/.config/apas/projects.json".to_string());
    tracing::info!(
        "Starting daemon for machine {} using project registry {}",
        machine_id,
        registry
    );

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

    let mut state = DaemonState::new(machine_info);
    state.refresh_projects();
    // Adopt anything already running here before serving any request, so a
    // restart or self-upgrade never leaves a window where our projects look
    // unclaimed to peers.
    // Before anything starts here: an older instance's projects are separate
    // processes this one cannot supervise, and two owners of one project is
    // the failure this whole change exists to remove.
    state.retire_process_per_project_leftovers();
    state.reconcile_running_claims();

    // Projects ask to be replaced here; only this loop holds the table that
    // can do it.
    let (restart_tx, mut restart_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // Start again what this instance was running before it replaced itself.
    // Pane hosts are separate processes and survived the `exec`, so a prompt
    // resume lands inside their adoption grace and the terminal agents carry
    // straight on.
    for project_id in take_resume_manifest() {
        if let Err(err) = state.start_project(&project_id, server_url, token, &restart_tx) {
            tracing::warn!(project_id, %err, "could not resume project after replacement");
        } else {
            tracing::info!(project_id, "resumed project after replacement");
        }
    }

    let mut reconnect_delay = INITIAL_RECONNECT_DELAY;

    while !shutdown.load(Ordering::SeqCst) {
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
    // Pane hosts are untouched and keep their PTYs, so a replacement instance
    // that starts within their adoption grace picks the terminals back up.
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
        capabilities: vec![
            shared::PROJECT_POLICY_CAPABILITY.to_string(),
            shared::PANE_HOST_CLEANUP_ACK_CAPABILITY.to_string(),
        ],
    };
    ws_sender
        .send(Message::Text(serde_json::to_string(&register)?.into()))
        .await?;

    // Registration response
    loop {
        match ws_receiver.next().await {
            Some(Ok(Message::Text(text))) => {
                let msg: ServerToDaemon = serde_json::from_str(&text)?;
                match msg {
                    ServerToDaemon::Registered { .. } => break,
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
    }

    // Report current state to server. CLI processes are only started explicitly
    // via StartProjectCli and are not auto-started on daemon reconnect.
    let update = DaemonToServer::Heartbeat {
        projects: state.snapshot_projects(),
    };
    let text = serde_json::to_string(&update)?;
    ws_sender.send(Message::Text(text.into())).await?;

    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    // Snapshot of the installed binary at boot. A change here is what triggers
    // the (more expensive) version comparison. Seeded now rather than left
    // empty so an unchanged binary never provokes a check.
    #[allow(unused_mut)]
    let mut binary_fingerprint = crate::update::apas_binary_fingerprint();
    let mut upgrade_check = tokio::time::interval(UPGRADE_CHECK_INTERVAL);
    // `interval` fires immediately on first tick; that first check is harmless
    // (the fingerprint was just seeded, so it is a no-op) and means a daemon
    // started against an already-newer binary corrects itself at once.
    upgrade_check.tick().await;
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut usage_refresh = tokio::time::interval(USAGE_REFRESH_INTERVAL);
    usage_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    usage_refresh.tick().await;

    refresh_usage_limits_cache().await;

    // Shared NFS registry paths + this host's identity, for the heartbeat below.
    let registry_dir = config_dir_for_registry();
    let self_machine_id = state.machine_info.machine_id;

    loop {
        if shutdown.load(Ordering::SeqCst) {
            // RegistrationGuard (installed in `run`) withdraws on drop.
            return Ok(());
        }

        // Keep our record and our project claims fresh. If these age past
        // STALE_AFTER_SECS a peer will conclude this host died and take over
        // projects we are actively running.
        let _ = crate::daemon_registry::publish_self(&registry_dir, self_machine_id, VERSION);
        crate::daemon_registry::refresh_own_claims(&registry_dir);

        tokio::select! {
            Some(project_id) = restart_rx.recv() => {
                // Stop it properly before starting a fresh one: the task has
                // already finished, but its entry and pane hosts have not been
                // cleaned up.
                state.stop_running_project(&project_id).await;
                if let Err(err) = state.start_project(&project_id, server_url, token, restart_tx) {
                    tracing::warn!(project_id, %err, "could not restart project");
                } else {
                    tracing::info!(project_id, "restarted project");
                }
            }
            _ = usage_refresh.tick() => {
                refresh_usage_limits_cache().await;
            }
            _ = upgrade_check.tick() => {
                // The stat is the gate: `apas --version` is only spawned when
                // the binary on disk has actually changed.
                #[cfg(unix)]
                {
                    let fp = crate::update::apas_binary_fingerprint();
                    if fp != binary_fingerprint {
                        binary_fingerprint = fp;
                        if let Some(newer) = crate::update::newer_installed_version() {
                            // Never returns on success.
                            let running = state.running_project_ids();
                            let err = exec_into_newer_binary(&newer, &running);
                            tracing::error!(
                                %err,
                                "daemon self-upgrade re-exec failed; continuing on the old binary"
                            );
                        }
                    }
                }
            }
            _ = heartbeat.tick() => {
                state.reap_exited_processes();
                state.refresh_projects();

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
                            ServerToDaemon::Registered { .. } => {}
                            ServerToDaemon::RegistrationFailed { reason } => {
                                return Err(anyhow::anyhow!("Daemon auth dropped: {}", reason));
                            }
                            ServerToDaemon::StartProjectCli { project_id, policy } => {
                                let Some(policy) = policy else {
                                    tracing::warn!(
                                        "Refusing to start project {} without authoritative cluster policy",
                                        project_id
                                    );
                                    continue;
                                };
                                if policy.project_suspended {
                                    tracing::warn!(
                                        "Refusing to start suspended project {} (policy version {})",
                                        project_id,
                                        policy.version
                                    );
                                    continue;
                                }
                                if let Err(err) = state.start_project(&project_id, server_url, token, restart_tx) {
                                    tracing::warn!("Failed to start project {}: {}", project_id, err);
                                }
                                let update = DaemonToServer::Heartbeat {
                                    projects: state.snapshot_projects(),
                                };
                                let text = serde_json::to_string(&update)?;
                                ws_sender.send(Message::Text(text.into())).await?;
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
                            ServerToDaemon::CreateProjectInstance {
                                git_remote,
                                instance_name,
                                branch,
                                clone_url,
                                base_path,
                                request_id,
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
                                            error: Some(err.to_string()),
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
                            ServerToDaemon::RebootDaemon => {
                                // The projects on this host are owned by their
                                // own tmux sessions, not by this process, so
                                // replacing it disturbs nothing that is running.
                                #[cfg(unix)]
                                {
                                    let err = perform_requested_restart(&state.running_project_ids());
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
        let lines: Vec<&str> = output.lines().filter(|l| l.contains("pane failed")).collect();
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
mod resume_tests {
    use super::{take_resume_manifest_at, write_resume_manifest_at};

    /// Under the process-per-project model an upgrade left projects alone —
    /// they were in their own tmux sessions and `exec` never touched them.
    /// Running them inside this process means `exec` takes them with it, so
    /// what was running has to survive the replacement in writing.
    #[test]
    fn what_was_running_survives_the_replacement() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("resume.json");

        write_resume_manifest_at(&path, &["alpha".to_string(), "beta".to_string()]);
        assert_eq!(
            take_resume_manifest_at(&path),
            vec!["alpha".to_string(), "beta".to_string()]
        );

        // Cleared on read: a crash during resume must not leave the instance
        // retrying the same start on every boot.
        assert!(take_resume_manifest_at(&path).is_empty());
    }

    #[test]
    fn nothing_running_leaves_nothing_to_resume() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("resume.json");

        write_resume_manifest_at(&path, &["alpha".to_string()]);
        write_resume_manifest_at(&path, &[]);
        assert!(take_resume_manifest_at(&path).is_empty());
    }
}

#[cfg(test)]
mod restart_tests {
    use super::{plan_daemon_restart, RestartPlan};

    /// A requested restart is worth requesting because it picks up an update.
    /// Restarting the same binary is the fallback, not the purpose.
    #[test]
    fn a_requested_restart_updates_when_an_update_is_available() {
        assert_eq!(
            plan_daemon_restart(Some("26.08.99".to_string())),
            RestartPlan::UpdateThenReplace("26.08.99".to_string()),
        );
    }

    #[test]
    fn an_up_to_date_daemon_replaces_itself_in_place() {
        assert_eq!(plan_daemon_restart(None), RestartPlan::ReplaceInPlace);
    }
}
