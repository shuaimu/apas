//! Autonomous job hosts are independent of project tasks and terminal rosters.
pub mod config;
pub mod delivery;
pub mod gateway;
pub mod host;
mod result;
pub mod rpc;
pub mod sandbox;

use anyhow::{ensure, Context, Result};
use shared::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

pub struct ExecutionLock(std::fs::File);

impl ExecutionLock {
    fn acquire(file: std::fs::File) -> std::io::Result<Self> {
        fs2::FileExt::try_lock_exclusive(&file)?;
        Ok(Self(file))
    }
}

impl Drop for ExecutionLock {
    fn drop(&mut self) {
        // A concurrent provider fork can inherit the open file description before
        // exec closes it. Closing only our descriptor need not release its flock.
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

pub struct DaemonJobs {
    pub reports: mpsc::Receiver<JobWorkerReport>,
    commands: mpsc::Sender<JobWorkerCommand>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for DaemonJobs {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl DaemonJobs {
    pub fn start(machine: Uuid, owner: Uuid) -> Result<Self> {
        let config = crate::config::Config::load()?.autonomous_jobs;
        let root = config.storage_root();
        let generation = host::controller_generation(&config)?;
        let controller = Uuid::new_v4();
        let (send, reports) = mpsc::channel(256);
        let (commands, mut receive) = mpsc::channel::<JobWorkerCommand>(64);
        let adopted = Arc::new(Mutex::new(HashMap::<Uuid, u64>::new()));
        let command_config = config.clone();
        let command_send = send.clone();
        let command_task = tokio::spawn(async move {
            let mut pending = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    Some(command)=receive.recv()=>{
                        let config=command_config.clone();let send=command_send.clone();
                        // Dropping this connection also drops JoinSet, fencing
                        // outstanding old-connection deliveries before re-exec.
                        pending.spawn(async move{
                            let id=host::command_id(&command);let identity=host::command_identity(&command).cloned();
                            if let Err(error)=dispatch(&config,machine,owner,generation,controller,command,&send).await{
                                let _=send.send(JobWorkerReport::Rejected{command_id:id,identity,code:"host_command_rejected".into(),message:error.to_string()}).await;
                            }
                        });
                    }
                    Some(_)=pending.join_next(),if !pending.is_empty()=>{},
                    else=>break,
                }
            }
        });
        let poll_send = send.clone();
        let poll_adopted = adopted.clone();
        let poll_config = config.clone();
        let poll_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            let mut first = true;
            let mut purged = std::collections::HashSet::new();
            loop {
                interval.tick().await;
                let mut inventories = Vec::new();
                let dirs = match run_dirs(&root) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                for dir in dirs {
                    if let Ok(Some(tombstone)) = host::purge_tombstone(&dir) {
                        if tombstone.identity.machine_id != machine
                            || tombstone.identity.cluster_owner_user_id != owner
                        {
                            continue;
                        }
                        if !purged.contains(&tombstone.identity.run_id) {
                            let command = JobWorkerCommand::PurgeRun {
                                command_id: tombstone.command_id,
                                identity: tombstone.identity.clone(),
                            };
                            if let Ok(reports) =
                                host::offline_command(&dir, generation, controller, &command).await
                            {
                                for report in reports {
                                    if poll_send.send(report).await.is_err() {
                                        return;
                                    }
                                }
                                purged.insert(tombstone.identity.run_id);
                            }
                        }
                        continue;
                    }
                    if let Ok(Some(mut tombstone)) = host::tombstone(&dir) {
                        if tombstone.identity.machine_id != machine
                            || tombstone.identity.cluster_owner_user_id != owner
                        {
                            continue;
                        }
                        if tombstone.evidence.is_none() {
                            let _ = host::reconcile_cancel(&poll_config, &tombstone).await;
                            if let Ok(Some(updated)) = host::tombstone(&dir) {
                                tombstone = updated;
                            }
                        }
                        inventories.push(tombstone.inventory());
                        let seen = poll_adopted
                            .lock()
                            .await
                            .contains_key(&tombstone.identity.run_id);
                        if !seen {
                            if let Some(report) = tombstone.report() {
                                if poll_send.send(report).await.is_err() {
                                    return;
                                }
                                poll_adopted
                                    .lock()
                                    .await
                                    .insert(tombstone.identity.run_id, 1);
                            }
                        }
                        continue;
                    }
                    let record = match host::load(&dir) {
                        Ok(r) => r,
                        Err(error) => {
                            tracing::warn!(%error,"unreadable job ownership journal retained");
                            continue;
                        }
                    };
                    if record.identity.machine_id != machine
                        || record.identity.cluster_owner_user_id != owner
                    {
                        continue;
                    }
                    let id = record.identity.run_id;
                    let cursor = {
                        let cursors = poll_adopted.lock().await;
                        cursors.get(&id).copied()
                    };
                    if record.state.is_terminal() {
                        if let Ok(response) = host::offline_reports(&dir, cursor.unwrap_or(0)) {
                            inventories.push(response.inventory);
                            for report in response.reports {
                                if poll_send.send(report).await.is_err() {
                                    return;
                                }
                            }
                            poll_adopted.lock().await.insert(id, response.next);
                        }
                        if !record.events_expired
                            && record.terminal_at.is_some_and(|at| {
                                chrono::Utc::now() >= at + chrono::Duration::days(30)
                            })
                        {
                            if let Err(error) = host::prune_retained(&dir, chrono::Utc::now()).await
                            {
                                tracing::warn!(%error,"retaining unresolved job evidence");
                            }
                        }
                        continue;
                    }
                    let _ =
                        host::request(&dir, generation, controller, host::Operation::Adopt).await;
                    let response = match host::request(
                        &dir,
                        generation,
                        controller,
                        host::Operation::Poll {
                            after: cursor.unwrap_or(0),
                        },
                    )
                    .await
                    {
                        Ok(response) => response,
                        Err(_) => {
                            // A held host lock means alive-but-unresponsive, never absent.
                            let inventory = match host::reconcile_dead(&dir).await {
                                Ok(i) => i,
                                Err(_) => {
                                    let mut i = record.inventory();
                                    i.state = JobState::RecoveryRequired;
                                    i
                                }
                            };
                            match host::offline_reports(&dir, cursor.unwrap_or(0)) {
                                Ok(mut response) => {
                                    response.inventory = inventory;
                                    response
                                }
                                Err(_) => {
                                    inventories.push(inventory);
                                    continue;
                                }
                            }
                        }
                    };
                    inventories.push(response.inventory.clone());
                    for report in response.reports {
                        if poll_send.send(report).await.is_err() {
                            return;
                        }
                    }
                    poll_adopted.lock().await.insert(id, response.next);
                }
                // Inventory is sent without starting any ordinary project task.
                if first || !inventories.is_empty() {
                    if poll_send
                        .send(JobWorkerReport::Inventory {
                            command_id: Uuid::new_v4(),
                            machine_id: machine,
                            cluster_owner_user_id: owner,
                            runs: inventories,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                    first = false;
                }
            }
        });
        let readiness_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                let report = sandbox::readiness(&config, machine, owner).await;
                if send
                    .send(JobWorkerReport::Readiness { readiness: report })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        Ok(Self {
            reports,
            commands,
            tasks: vec![command_task, poll_task, readiness_task],
        })
    }
    pub async fn send(&self, command: JobWorkerCommand) -> Result<()> {
        self.commands
            .send(command)
            .await
            .context("job daemon command dispatcher closed")
    }
}

fn run_dirs(root: &Path) -> Result<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(vec![]);
    }
    host::private_dir(root)?;
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|n| Uuid::parse_str(n).is_ok())
        {
            dirs.push(entry.path());
        }
    }
    Ok(dirs)
}
fn source_for(project_id: &str) -> Result<PathBuf> {
    crate::project::list_registered_projects()?
        .into_iter()
        .find(|p| p.project_id == project_id)
        .map(|p| PathBuf::from(p.path))
        .context("project has no host-local registered source repository")
}

async fn dispatch(
    config: &config::AutonomousJobConfig,
    machine: Uuid,
    owner: Uuid,
    generation: u64,
    controller: Uuid,
    command: JobWorkerCommand,
    send: &mpsc::Sender<JobWorkerReport>,
) -> Result<()> {
    match &command {
        JobWorkerCommand::Prepare {
            command_id,
            project_id,
            machine_id,
            cluster_owner_user_id,
            base_ref,
            worker_profile,
        } => {
            ensure!(
                *machine_id == machine && *cluster_owner_user_id == owner,
                "prepare target differs from authenticated daemon owner/machine"
            );
            let ready = sandbox::readiness(config, machine, owner).await;
            let profile = ready
                .profiles
                .into_iter()
                .find(|p| &p.key == worker_profile)
                .context("unknown worker profile")?;
            ensure!(
                profile.ready,
                "worker unavailable: {}",
                profile.unavailable_reasons.join("; ")
            );
            let source = source_for(project_id)?;
            let (repository, base_sha) = resolve_source(&source, base_ref).await?;
            send.send(JobWorkerReport::Prepared {
                command_id: *command_id,
                project_id: project_id.clone(),
                machine_id: machine,
                cluster_owner_user_id: owner,
                repository,
                base_ref: base_ref.clone(),
                base_sha,
                profile,
            })
            .await?;
        }
        JobWorkerCommand::Inventory { command_id } => {
            let mut runs = Vec::new();
            for dir in run_dirs(&config.storage_root())? {
                if host::purge_tombstone(&dir)?.is_some() {
                    continue;
                }
                let inventory = if let Some(tombstone) = host::tombstone(&dir)? {
                    tombstone.inventory()
                } else {
                    host::load(&dir)?.inventory()
                };
                if inventory.identity.machine_id == machine
                    && inventory.identity.cluster_owner_user_id == owner
                {
                    runs.push(inventory);
                }
            }
            send.send(JobWorkerReport::Inventory {
                command_id: *command_id,
                machine_id: machine,
                cluster_owner_user_id: owner,
                runs,
            })
            .await?;
        }
        _ => {
            let identity =
                host::command_identity(&command).context("command missing run identity")?;
            ensure!(
                identity.machine_id == machine && identity.cluster_owner_user_id == owner,
                "command target differs from authenticated daemon owner/machine"
            );
            let dir = config.storage_root().join(identity.run_id.to_string());
            if host::purge_tombstone(&dir)?.is_some() {
                for report in host::offline_command(&dir, generation, controller, &command).await? {
                    send.send(report).await?;
                }
                return Ok(());
            }
            if matches!(command, JobWorkerCommand::PurgeRun { .. })
                && host::tombstone(&dir)?.is_some()
            {
                send.send(host::purge_cancelled(config, &command).await?)
                    .await?;
                return Ok(());
            }
            if matches!(command, JobWorkerCommand::Cancel { .. }) && !dir.join("host.json").exists()
            {
                send.send(host::cancel_before_start(config, &command).await?)
                    .await?;
                return Ok(());
            }
            if let JobWorkerCommand::Start { task, .. } = &command {
                if !dir.exists() {
                    ensure!(config.enabled, "autonomous execution disabled on this host");
                    let source = source_for(&identity.project_id)?;
                    let (repository, base_sha) = resolve_source(&source, &task.base_sha).await?;
                    ensure!(
                        repository == task.repository && base_sha == task.base_sha,
                        "admitted repository/base changed"
                    );
                    // Serialize launch reservation across duplicate start delivery.
                    let _lock = host::admission_lock(config)?;
                    if !dir.exists() {
                        host::reserve(config, source, command.clone())?;
                    }
                }
            }
            let record = host::load(&dir)?;
            ensure!(
                record.identity == *identity,
                "recorded ownership differs from requested run epoch"
            );
            if record.state.is_terminal() {
                for report in host::offline_command(&dir, generation, controller, &command).await? {
                    send.send(report).await?;
                }
                return Ok(());
            }
            // Only a new authenticated daemon generation can adopt, not a stale command.
            if host::request(&dir, generation, controller, host::Operation::Adopt)
                .await
                .is_err()
            {
                for report in host::offline_command(&dir, generation, controller, &command).await? {
                    send.send(report).await?;
                }
                return Ok(());
            }
            match host::request(
                &dir,
                generation,
                controller,
                host::Operation::Command {
                    command: command.clone(),
                },
            )
            .await
            {
                Ok(response) => {
                    for report in response.reports {
                        send.send(report).await?;
                    }
                }
                Err(error) => {
                    match host::offline_command(&dir, generation, controller, &command).await {
                        Ok(reports) => {
                            for report in reports {
                                send.send(report).await?;
                            }
                        }
                        Err(_) => return Err(error),
                    }
                }
            }
        }
    }
    Ok(())
}
async fn resolve_source(source: &Path, base: &str) -> Result<(String, String)> {
    ensure!(
        !base.is_empty()
            && base.len() <= 256
            && !base.starts_with('-')
            && !base.contains(['\n', '\r']),
        "invalid base reference"
    );
    async fn git(source: &Path, args: &[&str]) -> Result<String> {
        let output = tokio::time::timeout(
            Duration::from_secs(15),
            tokio::process::Command::new("/usr/bin/git")
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", "/nonexistent")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_TERMINAL_PROMPT", "0")
                .arg("--no-optional-locks")
                .arg("-c")
                .arg("core.hooksPath=/dev/null")
                .arg("-C")
                .arg(source)
                .args(args)
                .output(),
        )
        .await??;
        ensure!(
            output.status.success() && output.stdout.len() <= 4096,
            "cannot resolve approved Git source"
        );
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }
    let sha = git(
        source,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{base}^{{commit}}"),
        ],
    )
    .await?;
    ensure!(
        sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
        "source commit is not a SHA-1 commit"
    );
    let remote = git(source, &["config", "--get", "remote.origin.url"]).await?;
    let repo = remote
        .strip_prefix("https://github.com/")
        .or_else(|| remote.strip_prefix("git@github.com:"))
        .context("only explicit github.com repositories are publishable")?
        .trim_end_matches(".git");
    ensure!(
        repo.split('/').count() == 2
            && repo
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._/".contains(&b))
            && !repo.contains(".."),
        "invalid GitHub repository identity"
    );
    Ok((repo.into(), sha))
}
