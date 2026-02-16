use anyhow::Result;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use shared::{DaemonToServer, MachineInfo, MachineProjectInfo, ServerToDaemon};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
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
const MAX_DISCOVERY_DEPTH: usize = 8;
const VERSION: &str = env!("APAS_VERSION");

#[derive(Debug)]
struct ProjectEntry {
    project_id: String,
    name: Option<String>,
    path: PathBuf,
    last_error: Option<String>,
}

#[derive(Debug)]
struct DaemonState {
    machine_info: MachineInfo,
    project_roots: Vec<PathBuf>,
    projects: HashMap<String, ProjectEntry>,
    processes: HashMap<String, Child>,
}

impl DaemonState {
    fn new(machine_info: MachineInfo, project_roots: Vec<PathBuf>) -> Self {
        Self {
            machine_info,
            project_roots,
            projects: HashMap::new(),
            processes: HashMap::new(),
        }
    }

    fn refresh_projects(&mut self) {
        let discovered = discover_projects(&self.project_roots);
        let mut seen = HashSet::new();

        for project in discovered {
            seen.insert(project.project_id.clone());
            match self.projects.get_mut(&project.project_id) {
                Some(existing) => {
                    existing.name = project.name.clone();
                    existing.path = project.path.clone();
                }
                None => {
                    self.projects.insert(project.project_id.clone(), project);
                }
            }
        }

        // Remove disappeared projects only when they are not actively running.
        let stale_ids: Vec<String> = self
            .projects
            .keys()
            .filter(|project_id| !seen.contains(*project_id) && !self.processes.contains_key(*project_id))
            .cloned()
            .collect();
        for project_id in stale_ids {
            self.projects.remove(&project_id);
        }
    }

    fn reap_exited_processes(&mut self) {
        let mut exited = Vec::new();
        for (project_id, child) in &mut self.processes {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        if let Some(project) = self.projects.get_mut(project_id) {
                            project.last_error =
                                Some(format!("Process exited with status {}", status));
                        }
                    }
                    exited.push(project_id.clone());
                }
                Ok(None) => {}
                Err(err) => {
                    if let Some(project) = self.projects.get_mut(project_id) {
                        project.last_error = Some(format!("Failed to poll process: {}", err));
                    }
                    exited.push(project_id.clone());
                }
            }
        }

        for project_id in exited {
            self.processes.remove(&project_id);
        }
    }

    fn stop_all(&mut self) {
        let project_ids: Vec<String> = self.processes.keys().cloned().collect();
        for project_id in project_ids {
            let _ = self.stop_project(&project_id);
        }
    }

    fn snapshot_projects(&self) -> Vec<MachineProjectInfo> {
        let mut projects = Vec::with_capacity(self.projects.len());

        for (project_id, project) in &self.projects {
            let running = self.processes.get(project_id);
            projects.push(MachineProjectInfo {
                project_id: project_id.clone(),
                name: project.name.clone(),
                path: project.path.to_string_lossy().to_string(),
                is_running: running.is_some(),
                pid: running.map(|child| child.id()),
                last_error: project.last_error.clone(),
            });
        }

        projects.sort_by(|a, b| a.path.cmp(&b.path));
        projects
    }

    fn start_project(&mut self, project_id: &str, server_url: &str, token: &str) -> Result<()> {
        self.reap_exited_processes();

        if self.processes.contains_key(project_id) {
            return Ok(());
        }

        let project = self
            .projects
            .get_mut(project_id)
            .ok_or_else(|| anyhow::anyhow!("Unknown project id: {}", project_id))?;

        let executable = std::env::current_exe()?;
        let child = Command::new(executable)
            .arg("--remote")
            .arg("--server")
            .arg(server_url)
            .arg("--token")
            .arg(token)
            .arg("-d")
            .arg(&project.path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();

        match child {
            Ok(child) => {
                project.last_error = None;
                self.processes.insert(project_id.to_string(), child);
                Ok(())
            }
            Err(err) => {
                project.last_error = Some(format!("Failed to start CLI: {}", err));
                Err(err.into())
            }
        }
    }

    fn stop_project(&mut self, project_id: &str) -> Result<()> {
        let mut child = match self.processes.remove(project_id) {
            Some(child) => child,
            None => return Ok(()),
        };

        let _ = child.kill();
        let _ = child.wait();
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct StoredProject {
    id: Uuid,
    name: Option<String>,
}

fn discover_projects(roots: &[PathBuf]) -> Vec<ProjectEntry> {
    let mut results = Vec::new();
    let mut seen = HashSet::new();

    for root in roots {
        for project_dir in find_project_dirs(root) {
            let apas_file = project_dir.join(".apas");
            let content = match std::fs::read_to_string(&apas_file) {
                Ok(content) => content,
                Err(_) => continue,
            };
            let parsed: StoredProject = match serde_json::from_str(&content) {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            let project_id = parsed.id.to_string();
            if seen.contains(&project_id) {
                continue;
            }
            seen.insert(project_id.clone());
            results.push(ProjectEntry {
                project_id,
                name: parsed.name,
                path: project_dir,
                last_error: None,
            });
        }
    }

    results
}

fn find_project_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];

    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DISCOVERY_DEPTH {
            continue;
        }

        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        let mut has_apas = false;
        let mut subdirs = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name().and_then(|n| n.to_str()) == Some(".apas") {
                has_apas = true;
                continue;
            }

            if !path.is_dir() {
                continue;
            }

            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if matches!(
                name,
                ".git"
                    | "node_modules"
                    | "target"
                    | ".next"
                    | ".turbo"
                    | ".cache"
                    | ".idea"
                    | ".vscode"
            ) {
                continue;
            }

            subdirs.push(path);
        }

        if has_apas {
            dirs.push(dir.clone());
            continue;
        }

        for subdir in subdirs {
            stack.push((subdir, depth + 1));
        }
    }

    dirs
}

pub async fn run(
    server_url: &str,
    token: &str,
    machine_id: Uuid,
    project_roots: Vec<PathBuf>,
) -> Result<()> {
    let hostname = hostname::get()
        .ok()
        .and_then(|value| value.into_string().ok())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    let roots = if project_roots.is_empty() {
        vec![std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))]
    } else {
        project_roots
    };

    let machine_info = MachineInfo {
        machine_id,
        hostname,
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        daemon_version: Some(VERSION.to_string()),
        last_seen: None,
    };

    tracing::info!(
        "Starting daemon for machine {} with {} root(s)",
        machine_id,
        roots.len()
    );

    let shutdown = Arc::new(AtomicBool::new(false));
    {
        let shutdown = shutdown.clone();
        ctrlc::set_handler(move || {
            shutdown.store(true, Ordering::SeqCst);
        })?;
    }

    let mut state = DaemonState::new(machine_info, roots);
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

    state.stop_all();
    tracing::info!("Daemon stopped");
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
            None => return Err(anyhow::anyhow!("Daemon websocket closed during registration")),
            _ => {}
        }
    }

    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        if shutdown.load(Ordering::SeqCst) {
            return Ok(());
        }

        tokio::select! {
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
