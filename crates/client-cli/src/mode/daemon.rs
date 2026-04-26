use anyhow::Result;
use futures::{SinkExt, StreamExt};
use shared::{
    DaemonToServer, GlmBackendInfo, MachineInfo, MachineProjectInfo, MiniMaxBackendInfo,
    ServerToDaemon,
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
const USAGE_REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
const VERSION: &str = env!("APAS_VERSION");
const TMUX_SESSION_PREFIX: &str = "apas";
const MINIMAX_API_BASE_URL: &str = "https://api.minimax.io/anthropic";
const GLM_API_BASE_URL: &str = "https://api.z.ai/api/anthropic";

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

fn launch_path() -> String {
    resolve_user_shell_path()
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".to_string())
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

fn minimax_backend_info_from_config(config: &crate::config::Config) -> Option<MiniMaxBackendInfo> {
    let api_base_url = Some(MINIMAX_API_BASE_URL.to_string());
    let api_key = normalize_optional_string(config.local.minimax_api_key.clone());
    let api_key_configured = api_key.is_some();
    Some(MiniMaxBackendInfo {
        api_base_url,
        api_key,
        api_key_configured,
    })
}

fn update_local_minimax_backend_config(
    _api_base_url: Option<String>,
    api_key: Option<String>,
    clear_api_key: bool,
) -> Result<Option<MiniMaxBackendInfo>> {
    let mut config = crate::config::Config::load().unwrap_or_default();
    config.local.minimax_api_base_url = Some(MINIMAX_API_BASE_URL.to_string());

    if clear_api_key {
        config.local.minimax_api_key = None;
    } else if let Some(key) = api_key {
        config.local.minimax_api_key = normalize_optional_string(Some(key));
    }

    config.save()?;
    Ok(minimax_backend_info_from_config(&config))
}

fn glm_backend_info_from_config(config: &crate::config::Config) -> Option<GlmBackendInfo> {
    let api_base_url = Some(GLM_API_BASE_URL.to_string());
    let api_key = normalize_optional_string(config.local.glm_api_key.clone());
    let api_key_configured = api_key.is_some();
    Some(GlmBackendInfo {
        api_base_url,
        api_key,
        api_key_configured,
    })
}

fn update_local_glm_backend_config(
    _api_base_url: Option<String>,
    api_key: Option<String>,
    clear_api_key: bool,
) -> Result<Option<GlmBackendInfo>> {
    let mut config = crate::config::Config::load().unwrap_or_default();
    config.local.glm_api_base_url = Some(GLM_API_BASE_URL.to_string());

    if clear_api_key {
        config.local.glm_api_key = None;
    } else if let Some(key) = api_key {
        config.local.glm_api_key = normalize_optional_string(Some(key));
    }

    config.save()?;
    Ok(glm_backend_info_from_config(&config))
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

/// Check if there's already a running `apas --headless` process for the given project path.
/// Prevents the daemon from spawning duplicates when a CLI was started externally
/// or survived a daemon restart.
fn is_headless_running_for(project_path: &Path) -> bool {
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

/// Quote a string for safe use inside a single-quoted sh -c argument.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
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

#[derive(Debug)]
struct DaemonState {
    machine_info: MachineInfo,
    projects: HashMap<String, ProjectEntry>,
    sessions: HashMap<String, String>,
}

impl DaemonState {
    fn new(machine_info: MachineInfo) -> Self {
        Self {
            machine_info,
            projects: HashMap::new(),
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

    fn snapshot_projects(&self) -> Vec<MachineProjectInfo> {
        let mut projects = Vec::with_capacity(self.projects.len());

        for (project_id, project) in &self.projects {
            let pid = headless_pid_for(&project.path);
            let memory_kb = pid.and_then(read_process_rss_kb);
            projects.push(MachineProjectInfo {
                project_id: project_id.clone(),
                name: project.name.clone(),
                path: project.path.to_string_lossy().to_string(),
                is_running: pid.is_some(),
                pid,
                memory_kb,
                last_error: project.last_error.clone(),
            });
        }

        projects.sort_by(|a, b| a.path.cmp(&b.path));
        projects
    }

    fn start_project(&mut self, project_id: &str, server_url: &str, token: &str) -> Result<()> {
        // Reap any exited tracked processes before deciding whether to spawn.
        self.reap_exited_processes();

        if self
            .sessions
            .get(project_id)
            .map(|session_name| tmux_has_session(project_id, session_name))
            .unwrap_or(false)
        {
            return Ok(());
        }
        self.sessions.remove(project_id);

        let project = self
            .projects
            .get_mut(project_id)
            .ok_or_else(|| anyhow::anyhow!("Unknown project id: {}", project_id))?;

        let session_name = tmux_session_name(project_id);

        // Check if an external process (e.g. manually started via systemd-run,
        // or surviving from a previous daemon) is already running for this project.
        if is_headless_running_for(&project.path) {
            if tmux_has_session(project_id, &session_name) {
                self.sessions
                    .insert(project_id.to_string(), session_name.clone());
            }
            tracing::info!(
                "Project {} already has a running headless CLI, skipping spawn",
                project_id
            );
            return Ok(());
        }

        // Prefer a real on-disk installed binary, never /proc/self/exe.
        let executable = crate::update::resolve_preferred_apas_executable();
        let child_path = launch_path();
        if tmux_has_session(project_id, &session_name) {
            tmux_kill_session(project_id, &session_name)?;
        }

        // Per-project socket so each project gets its own tmux server, and
        // one project's tmux dying doesn't affect others. tmux itself
        // double-forks and the server reparents to PID 1 on detach, so it
        // survives the daemon exiting and our login session ending.
        let socket_name = tmux_socket_name(project_id);
        // Log headless stderr to a per-project file so we can postmortem
        // crashes (tmux normally swallows stderr into its pane buffer which
        // is lost when tmux dies).
        let stderr_log = format!("/tmp/apas-headless-{}.log", sanitize_for_unit(project_id));
        // Build the command as "sh -c '... exec apas ... 2>>logfile'" so the
        // redirection happens inside the shell tmux runs, after env/PATH have
        // been applied.
        // RUST_LOG=apas=info so the headless daemon emits the streaming worker's
        // tracing::info! breadcrumbs (spawn, prompt-sent, reader-exit, inner
        // loop break reason). Default tracing level is "warn", which hides
        // them. The log goes to /tmp/apas-headless-<id>.log.
        let inner_cmd = format!(
            "exec env -u CLAUDECODE PATH={} RUST_LOG=apas=info {} --headless --server {} --token {} -d {} 2>>{}",
            shell_escape(&child_path),
            shell_escape(&executable.to_string_lossy()),
            shell_escape(server_url),
            shell_escape(token),
            shell_escape(&project.path.to_string_lossy()),
            shell_escape(&stderr_log),
        );
        let mut cmd = Command::new("tmux");
        cmd.arg("-L")
            .arg(&socket_name)
            .arg("new-session")
            .arg("-d")
            .arg("-s")
            .arg(&session_name)
            .arg("-c")
            .arg(&project.path)
            .arg("sh")
            .arg("-c")
            .arg(&inner_cmd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let output = cmd.output();

        match output {
            Ok(output) if output.status.success() => {
                project.last_error = None;
                self.sessions
                    .insert(project_id.to_string(), session_name.clone());
                tracing::info!(
                    "Started project {} headless CLI in tmux session {}",
                    project_id,
                    session_name
                );
                Ok(())
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let err_msg = if stderr.is_empty() {
                    format!("tmux exited with status {}", output.status)
                } else {
                    stderr
                };
                project.last_error = Some(format!("Failed to start CLI: {}", err_msg));
                Err(anyhow::anyhow!(err_msg))
            }
            Err(err) => {
                project.last_error = Some(format!("Failed to start CLI: {}", err));
                Err(err.into())
            }
        }
    }

    fn stop_project(&mut self, project_id: &str) -> Result<()> {
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
}

pub async fn run(
    server_url: &str,
    token: &str,
    machine_id: Uuid,
    _project_roots: Vec<PathBuf>,
) -> Result<()> {
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
        minimax_backend: minimax_backend_info_from_config(&config),
        glm_backend: glm_backend_info_from_config(&config),
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

    let mut state = DaemonState::new(machine_info);
    state.refresh_projects();

    let mut reconnect_delay = INITIAL_RECONNECT_DELAY;

    while !shutdown.load(Ordering::SeqCst) {
        state.reap_exited_processes();
        state.refresh_projects();

        match run_connection(server_url, token, &mut state, shutdown.clone()).await {
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

    // Don't kill headless CLIs on shutdown — they are self-sufficient and will
    // keep running with their own server reconnection loops. This allows the
    // daemon to be restarted/upgraded without disrupting active sessions.
    tracing::info!("Daemon stopped (headless CLIs left running)");
    Ok(())
}

async fn run_connection(
    server_url: &str,
    token: &str,
    state: &mut DaemonState,
    shutdown: Arc<AtomicBool>,
) -> Result<()> {
    let ws_url = format!("{}/ws/daemon", server_url);
    let (ws_stream, _) = connect_async(&ws_url).await?;
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    let register = DaemonToServer::Register {
        token: token.to_string(),
        machine: state.machine_info.clone(),
        projects: state.snapshot_projects(),
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
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut usage_refresh = tokio::time::interval(USAGE_REFRESH_INTERVAL);
    usage_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    usage_refresh.tick().await;

    refresh_usage_limits_cache().await;

    loop {
        if shutdown.load(Ordering::SeqCst) {
            return Ok(());
        }

        tokio::select! {
            _ = usage_refresh.tick() => {
                refresh_usage_limits_cache().await;
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
                            ServerToDaemon::StartProjectCli { project_id } => {
                                if let Err(err) = state.start_project(&project_id, server_url, token) {
                                    tracing::warn!("Failed to start project {}: {}", project_id, err);
                                }
                                let update = DaemonToServer::Heartbeat {
                                    projects: state.snapshot_projects(),
                                };
                                let text = serde_json::to_string(&update)?;
                                ws_sender.send(Message::Text(text.into())).await?;
                            }
                            ServerToDaemon::StopProjectCli { project_id } => {
                                if let Err(err) = state.stop_project(&project_id) {
                                    tracing::warn!("Failed to stop project {}: {}", project_id, err);
                                }
                                let update = DaemonToServer::Heartbeat {
                                    projects: state.snapshot_projects(),
                                };
                                let text = serde_json::to_string(&update)?;
                                ws_sender.send(Message::Text(text.into())).await?;
                            }
                            ServerToDaemon::RefreshProjects => {
                                state.refresh_projects();
                                let refresh_msg = DaemonToServer::Heartbeat {
                                    projects: state.snapshot_projects(),
                                };
                                let text = serde_json::to_string(&refresh_msg)?;
                                ws_sender.send(Message::Text(text.into())).await?;
                            }
                            ServerToDaemon::SetMiniMaxConfig {
                                api_base_url,
                                api_key,
                                clear_api_key,
                            } => {
                                match update_local_minimax_backend_config(
                                    api_base_url,
                                    api_key,
                                    clear_api_key,
                                ) {
                                    Ok(minimax_backend) => {
                                        state.machine_info.minimax_backend = minimax_backend;
                                        let update = DaemonToServer::MachineInfoUpdate {
                                            machine: state.machine_info.clone(),
                                        };
                                        let text = serde_json::to_string(&update)?;
                                        ws_sender.send(Message::Text(text.into())).await?;
                                    }
                                    Err(err) => {
                                        tracing::warn!(
                                            "Failed to update MiniMax backend config: {}",
                                            err
                                        );
                                    }
                                }
                            }
                            ServerToDaemon::SetGlmConfig {
                                api_base_url,
                                api_key,
                                clear_api_key,
                            } => {
                                match update_local_glm_backend_config(
                                    api_base_url,
                                    api_key,
                                    clear_api_key,
                                ) {
                                    Ok(glm_backend) => {
                                        state.machine_info.glm_backend = glm_backend;
                                        let update = DaemonToServer::MachineInfoUpdate {
                                            machine: state.machine_info.clone(),
                                        };
                                        let text = serde_json::to_string(&update)?;
                                        ws_sender.send(Message::Text(text.into())).await?;
                                    }
                                    Err(err) => {
                                        tracing::warn!(
                                            "Failed to update GLM backend config: {}",
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

    match crate::usage::refresh_minimax_usage_limits().await {
        Ok(_) => tracing::debug!("Refreshed MiniMax usage limits cache"),
        Err(err) => tracing::debug!("Failed to refresh MiniMax usage limits cache: {}", err),
    }

    match crate::usage::refresh_glm_usage_limits().await {
        Ok(_) => tracing::debug!("Refreshed GLM usage limits cache"),
        Err(err) => tracing::debug!("Failed to refresh GLM usage limits cache: {}", err),
    }
}
