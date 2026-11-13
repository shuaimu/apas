//! Authenticated, detached pipe hosts. Jobs never enter pane-host descriptor walks.
use super::{
    config::AutonomousJobConfig,
    delivery, gateway, rpc,
    sandbox::{self, Sandbox},
    ExecutionLock,
};
use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use shared::*;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::mpsc,
};
use uuid::Uuid;

const HOST_VERSION: u32 = 1;
const MAX_RECORD: u64 = 16 * 1024 * 1024;
const CONTROL_RESERVE: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    command: JobWorkerCommand,
    result: Option<JobWorkerReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub identity: JobRunIdentity,
    pub task: JobTaskSpec,
    pub limits: JobLimits,
    pub config: AutonomousJobConfig,
    pub source: PathBuf,
    pub boot_id: String,
    pub container_id: String,
    pub socket: PathBuf,
    pub started_at: DateTime<Utc>,
    pub deadline: DateTime<Utc>,
    pub lease_expires_at: DateTime<Utc>,
    pub state: JobState,
    pub usage: JobUsage,
    pub rpc: rpc::Session,
    pub stop_evidence: Option<JobStopEvidence>,
    pub checkout: Option<PathBuf>,
    pub workspace_root: Option<PathBuf>,
    pub session_dir: Option<PathBuf>,
    pub generation: u64,
    pub controller: Uuid,
    pub source_seq: u64,
    pub spool_seq: u64,
    pub spool_bytes: u64,
    pub cancelled: bool,
    pub host_started: bool,
    #[serde(default)]
    pub launch_complete: bool,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub delivery_in_flight: bool,
    #[serde(default)]
    spooled_artifacts: std::collections::BTreeSet<Uuid>,
    #[serde(default)]
    pub terminal_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub events_expired: bool,
    receipts: BTreeMap<Uuid, Receipt>,
}

#[derive(Serialize, Deserialize)]
pub struct Request {
    version: u32,
    credential: String,
    generation: u64,
    controller: Uuid,
    pub operation: Operation,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Operation {
    Adopt,
    Poll { after: u64 },
    Command { command: JobWorkerCommand },
}
#[derive(Serialize, Deserialize)]
pub struct Response {
    pub inventory: JobRunInventory,
    pub reports: Vec<JobWorkerReport>,
    pub next: u64,
    pub error: Option<String>,
}

pub fn private_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::DirBuilder::new().recursive(true).create(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        File::open(path)?.sync_all()?;
        if let Some(parent) = path.parent() {
            File::open(parent)?.sync_all()?;
        }
    }
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir()
            && !meta.file_type().is_symlink()
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o077 == 0,
        "job directory must be private and owned by host uid"
    );
    Ok(())
}
fn private_file(path: &Path, create: bool) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0,
        "job file is not private"
    );
    Ok(file)
}
pub fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    atomic_bytes(path, &serde_json::to_vec(value)?)
}
pub fn atomic_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("missing private parent")?;
    let temp = parent.join(format!(".{}", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let file = private_file(path, false)?;
    ensure!(
        file.metadata()?.len() <= MAX_RECORD,
        "oversized durable job record"
    );
    Ok(serde_json::from_reader(file)?)
}
pub fn boot_id() -> Result<String> {
    Ok(fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim()
        .to_owned())
}
pub fn load(dir: &Path) -> Result<Record> {
    read_json(&dir.join("host.json"))
}
impl Record {
    fn save(&self, dir: &Path) -> Result<()> {
        atomic_json(&dir.join("host.json"), self)
    }
    fn adopt(&mut self, generation: u64, controller: Uuid) -> Result<()> {
        ensure!(
            generation > self.generation
                || (generation == self.generation && controller == self.controller),
            "stale daemon generation"
        );
        self.generation = generation;
        self.controller = controller;
        Ok(())
    }
    pub fn inventory(&self) -> JobRunInventory {
        JobRunInventory {
            identity: self.identity.clone(),
            state: self.state,
            boot_id: self.boot_id.clone(),
            container_id: Some(self.container_id.clone()),
            last_source_seq: self.source_seq,
            prompt_confirmed: self.rpc.prompt == rpc::PromptAdmission::Confirmed,
            usage: self.usage.clone(),
            stop_evidence: self.stop_evidence.clone(),
        }
    }
    fn next_source(&mut self) -> u64 {
        self.source_seq += 1;
        self.source_seq
    }
    fn emit(&mut self, dir: &Path, report: JobWorkerReport) -> Result<()> {
        let bytes = serde_json::to_vec(&report)?;
        ensure!(
            bytes.len() < JOB_MAX_FRAME_BYTES / 2,
            "job report too large for bounded replay"
        );
        let essential = !matches!(
            report,
            JobWorkerReport::Event {
                event_kind: JobEventKind::Progress | JobEventKind::Subagent,
                ..
            }
        );
        let cap =
            self.limits
                .output_bytes
                .saturating_add(if essential { CONTROL_RESERVE } else { 0 });
        ensure!(
            self.spool_bytes.saturating_add(bytes.len() as u64) <= cap,
            "job event output limit exhausted"
        );
        self.spool_seq += 1;
        self.spool_bytes += bytes.len() as u64;
        atomic_bytes(
            &dir.join("events")
                .join(format!("{:020}.json", self.spool_seq)),
            &bytes,
        )?;
        self.save(dir)
    }
    fn event(
        &mut self,
        dir: &Path,
        kind: JobEventKind,
        summary: &str,
        detail: Option<Value>,
    ) -> Result<()> {
        let seq = self.next_source();
        self.emit(
            dir,
            JobWorkerReport::Event {
                identity: self.identity.clone(),
                source_seq: seq,
                event_kind: kind,
                summary: summary.into(),
                detail,
                usage: Some(self.usage.clone()),
            },
        )
    }
    fn response(&self, dir: &Path, after: u64) -> Result<Response> {
        if self.events_expired {
            return Ok(Response {
                inventory: self.inventory(),
                reports: vec![],
                next: self.spool_seq,
                error: None,
            });
        }
        let mut reports = Vec::new();
        let mut next = after;
        let mut bytes = 0;
        for seq in after.saturating_add(1)..=self.spool_seq {
            let path = dir.join("events").join(format!("{seq:020}.json"));
            let size = fs::metadata(&path)?.len() as usize;
            if !reports.is_empty() && (bytes + size > 512 * 1024 || reports.len() >= 32) {
                break;
            }
            reports.push(read_json(&path)?);
            bytes += size;
            next = seq;
        }
        Ok(Response {
            inventory: self.inventory(),
            reports,
            next,
            error: None,
        })
    }
}

pub fn command_id(command: &JobWorkerCommand) -> Uuid {
    match command {
        JobWorkerCommand::Prepare { command_id, .. }
        | JobWorkerCommand::Start { command_id, .. }
        | JobWorkerCommand::RenewLease { command_id, .. }
        | JobWorkerCommand::Answer { command_id, .. }
        | JobWorkerCommand::Cancel { command_id, .. }
        | JobWorkerCommand::Inventory { command_id }
        | JobWorkerCommand::Verify { command_id, .. }
        | JobWorkerCommand::Publish { command_id, .. }
        | JobWorkerCommand::RefreshPublication { command_id, .. }
        | JobWorkerCommand::PurgeRun { command_id, .. } => *command_id,
    }
}
pub fn command_identity(command: &JobWorkerCommand) -> Option<&JobRunIdentity> {
    match command {
        JobWorkerCommand::Start { identity, .. }
        | JobWorkerCommand::RenewLease { identity, .. }
        | JobWorkerCommand::Answer { identity, .. }
        | JobWorkerCommand::Cancel { identity, .. }
        | JobWorkerCommand::Verify { identity, .. }
        | JobWorkerCommand::Publish { identity, .. }
        | JobWorkerCommand::RefreshPublication { identity, .. }
        | JobWorkerCommand::PurgeRun { identity, .. } => Some(identity),
        _ => None,
    }
}

/// The irreversible local attempt reservation precedes process creation. Any
/// interrupted reservation is recovered, never replaced with another prompt.
pub fn reserve(
    config: &AutonomousJobConfig,
    source: PathBuf,
    command: JobWorkerCommand,
) -> Result<PathBuf> {
    let JobWorkerCommand::Start {
        command_id,
        identity,
        task,
        limits,
        lease_expires_at,
        ..
    } = command.clone()
    else {
        bail!("start command required")
    };
    let sandbox = Sandbox::new(config)?;
    super::result::validate_context(&task, &identity)?;
    if let Some(context) = &task.worker {
        ensure!(
            limits.attempts == 1 && limits.fits_within(&context.mandate.limits.per_run),
            "worker execution must be one attempt within the approved per-run allowance"
        );
    }
    let dir = sandbox.storage_root().join(identity.run_id.to_string());
    ensure!(
        !dir.exists(),
        "run already reserved; adopt existing ownership"
    );
    let lease = parse_lease(&lease_expires_at)?;
    let now = Utc::now();
    let deadline = deadline_after(now, limits.elapsed_seconds)?;
    private_dir(&dir)?;
    private_dir(&dir.join("events"))?;
    let runtime = crate::config::Config::runtime_dir()?
        .join("jobs")
        .join(identity.run_id.simple().to_string());
    private_dir(&runtime)?;
    let mut secret = [0u8; 32];
    File::open("/dev/urandom")?.read_exact(&mut secret)?;
    use base64::Engine;
    atomic_bytes(
        &dir.join("credential"),
        base64::engine::general_purpose::STANDARD
            .encode(secret)
            .as_bytes(),
    )?;
    let mut receipts = BTreeMap::new();
    receipts.insert(
        command_id,
        Receipt {
            command,
            result: Some(JobWorkerReport::Accepted {
                command_id,
                identity: identity.clone(),
            }),
        },
    );
    let record = Record {
        identity: identity.clone(),
        task,
        limits: limits.clone(),
        config: config.clone(),
        source,
        boot_id: boot_id()?,
        container_id: sandbox::worker_container_name(identity.run_id),
        socket: runtime.join("host.sock"),
        started_at: now,
        deadline,
        lease_expires_at: lease,
        state: JobState::Preparing,
        usage: JobUsage {
            spend_micro_usd: Some(0),
            ..Default::default()
        },
        rpc: rpc::Session::new(identity.run_id),
        stop_evidence: None,
        checkout: None,
        workspace_root: None,
        session_dir: None,
        generation: 0,
        controller: Uuid::nil(),
        source_seq: 0,
        spool_seq: 0,
        spool_bytes: 0,
        cancelled: false,
        host_started: false,
        launch_complete: false,
        stop_reason: None,
        delivery_in_flight: false,
        spooled_artifacts: Default::default(),
        terminal_at: None,
        events_expired: false,
        receipts,
    };
    record.save(&dir)?;
    let log = OpenOptions::new()
        .append(true)
        .create_new(true)
        .mode(0o600)
        .open(dir.join("host.stderr"))?;
    let mut child =
        tokio::process::Command::new(crate::update::resolve_preferred_apas_executable());
    child
        .arg("job-host")
        .arg("--run-dir")
        .arg(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .kill_on_drop(false);
    unsafe {
        child.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    child
        .spawn()
        .context("detached job host launch failed; reserved attempt requires reconciliation")?;
    Ok(dir)
}

pub async fn request(
    dir: &Path,
    generation: u64,
    controller: Uuid,
    operation: Operation,
) -> Result<Response> {
    let record = load(dir)?;
    let mut credential = String::new();
    private_file(&dir.join("credential"), false)?.read_to_string(&mut credential)?;
    let stream = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match UnixStream::connect(&record.socket).await {
                Ok(stream) => return Ok::<_, std::io::Error>(stream),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                    ) =>
                {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await??;
    let (reader, mut writer) = stream.into_split();
    rpc::write_frame(
        &mut writer,
        &serde_json::to_value(Request {
            version: HOST_VERSION,
            credential,
            generation,
            controller,
            operation,
        })?,
    )
    .await?;
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        rpc::read_frame(&mut BufReader::new(reader), JOB_MAX_FRAME_BYTES),
    )
    .await??;
    let response: Response = serde_json::from_value(response)?;
    if let Some(error) = &response.error {
        bail!("{error}");
    }
    Ok(response)
}
fn parse_lease(value: &str) -> Result<DateTime<Utc>> {
    let expires = DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc);
    let now = Utc::now();
    ensure!(
        expires > now && expires <= now + chrono::Duration::seconds(JOB_LEASE_SECONDS as i64),
        "invalid or overlong execution lease"
    );
    Ok(expires)
}
fn deadline_after(start: DateTime<Utc>, seconds: u64) -> Result<DateTime<Utc>> {
    let seconds = i64::try_from(seconds)
        .context("elapsed allowance is outside the supported duration range")?;
    let duration = chrono::Duration::try_seconds(seconds)
        .context("elapsed allowance is outside the supported duration range")?;
    start
        .checked_add_signed(duration)
        .context("elapsed allowance is outside the supported timestamp range")
}
fn monotonic_deadline(expires: DateTime<Utc>) -> Result<Instant> {
    let remaining = expires
        .signed_duration_since(Utc::now())
        .to_std()
        .unwrap_or_default();
    Instant::now()
        .checked_add(remaining)
        .context("deadline is outside the supported monotonic clock range")
}

struct Running {
    gateway: gateway::Gateway,
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
}
enum Work {
    Started(Result<(Running, sandbox::PreparedWorkspace)>),
    Frame(Result<Value>),
    Frozen(Result<(JobStopEvidence, Option<WorkerResult>)>),
    Expired(String),
    Delivery(Uuid, Result<JobWorkerReport>),
}

async fn launch(
    record: Record,
    dir: PathBuf,
    cancelled: Arc<AtomicBool>,
) -> Result<(Running, sandbox::PreparedWorkspace)> {
    let sandbox = Sandbox::new(&record.config)?;
    let profile = record
        .config
        .profiles
        .iter()
        .find(|p| p.key == record.task.worker_profile)
        .context("worker profile removed")?;
    ensure!(
        sandbox::worker_profile_digest(&record.config, profile)?
            == record.task.worker_profile_digest,
        "worker profile changed since admission"
    );
    ensure!(
        profile.omp_version == "18.18.0" && profile.omp_binary == "/usr/local/bin/apas-omp-worker",
        "worker requires pinned isolated OMP bootstrap"
    );
    ensure!(
        record.limits.fits_within(&profile.limits),
        "limits exceed worker profile"
    );
    let model = record
        .config
        .models
        .iter()
        .find(|p| p.key == record.task.model_profile && profile.models.contains(&p.key))
        .context("model profile not approved")?
        .clone();
    let verifier = record
        .config
        .verifiers
        .iter()
        .find(|p| p.key == record.task.verification_profile && profile.verifiers.contains(&p.key))
        .context("verification profile not approved")?;
    ensure!(
        sandbox::profile_digest(verifier)? == record.task.verification_profile_digest,
        "verification profile changed"
    );
    sandbox.probe(profile).await?;
    let workspace = sandbox
        .prepare(
            &record.source,
            &record.task.base_sha,
            record.identity.run_id,
            &record.limits,
        )
        .await?;
    ensure!(
        !cancelled.load(Ordering::Acquire),
        "preparation was cancelled before worker launch"
    );
    let gateway = gateway::Gateway::start(gateway::GatewayConfig {
        run_id: record.identity.run_id,
        directory: dir.join("gateway"),
        model: model.clone(),
        dependency_hosts: profile.dependency_hosts.clone(),
        limits: record.limits.clone(),
        prior_usage: record.usage.clone(),
        deadline: record.deadline,
        lease_expires_at: record.lease_expires_at,
    })
    .await?;
    atomic_json(
        &gateway.bridge_dir().join("worker-profile.json"),
        &json!({"model":model.model,"api":match model.api{JobModelApi::OpenaiResponses=>"openai-responses",JobModelApi::AnthropicMessages=>"anthropic-messages"},"contextWindow":model.context_window,"reasoning":model.reasoning,"maxOutputTokens":record.limits.model_output_tokens,"concurrency":record.limits.model_concurrency,"elapsedSeconds":record.limits.elapsed_seconds}),
    )?;
    let env = vec![("PI_CODING_AGENT_DIR".into(), "/session/agent".into())];
    let argv = vec![profile.omp_binary.clone()];
    ensure!(
        !cancelled.load(Ordering::Acquire),
        "worker launch is fenced"
    );
    let mut execution_limits = record.limits.clone();
    execution_limits.elapsed_seconds = u64::try_from(
        record
            .deadline
            .signed_duration_since(Utc::now())
            .num_seconds(),
    )
    .context("approved execution deadline expired before launch")?
    .min(record.limits.elapsed_seconds);
    ensure!(
        execution_limits.elapsed_seconds > 0,
        "approved execution deadline expired before launch"
    );
    let mut process = sandbox
        .spawn_worker(
            record.identity.run_id,
            profile,
            &execution_limits,
            &workspace,
            gateway.bridge_dir(),
            &env,
            &argv,
            record
                .task
                .worker
                .as_ref()
                .is_some_and(|context| context.purpose == WorkerRunPurpose::Inspection),
        )
        .await?;
    if cancelled.load(Ordering::Acquire) {
        sandbox.stop(&process.container_id).await?;
        bail!("worker launch cancelled; complete execution unit stopped");
    }
    let stdout = process
        .child
        .stdout
        .take()
        .context("missing worker stdout pipe")?;
    let stderr = process
        .child
        .stderr
        .take()
        .context("missing worker stderr pipe")?;
    let stdin = process
        .child
        .stdin
        .take()
        .context("missing worker stdin pipe")?;
    Ok((
        Running {
            gateway,
            child: process.child,
            stdin,
            stdout: Some(stdout),
            stderr: Some(stderr),
        },
        workspace,
    ))
}

fn read_worker_pipes(
    worker: &mut Running,
    dir: &Path,
    limit: u64,
    tx: mpsc::Sender<Work>,
) -> Result<()> {
    let stdout = worker.stdout.take().context("stdout already adopted")?;
    let stderr = worker.stderr.take().context("stderr already adopted")?;
    let frames = tx.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut decoder = rpc::Decoder::default();
        loop {
            let result = match rpc::read_frame(&mut reader, JOB_MAX_FRAME_BYTES).await {
                Ok(value) => decoder.decode(value),
                Err(error) => Err(error),
            };
            match result {
                Ok(Some(frame)) => {
                    if frames.send(Work::Frame(Ok(frame))).await.is_err() {
                        break;
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = frames.send(Work::Frame(Err(error))).await;
                    break;
                }
            }
        }
    });
    let path = dir.join("worker.stderr");
    tokio::spawn(async move {
        if let Err(error) = drain_stderr(stderr, &path, limit.min(1024 * 1024)).await {
            let _ = tx.send(Work::Frame(Err(error))).await;
        }
    });
    Ok(())
}
async fn drain_stderr(mut input: tokio::process::ChildStderr, path: &Path, cap: u64) -> Result<()> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    let mut size = 0u64;
    let mut buf = [0u8; 8192];
    loop {
        let n = input.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        ensure!(
            size + n as u64 <= cap,
            "worker stderr output limit exhausted"
        );
        file.write_all(&buf[..n])?;
        size += n as u64;
    }
    file.sync_all()?;
    Ok(())
}

fn delivery_context(record: &Record, dir: &Path) -> Result<delivery::DeliveryContext> {
    Ok(delivery::DeliveryContext {
        identity: record.identity.clone(),
        task: record.task.clone(),
        limits: record.limits.clone(),
        run_dir: dir.to_owned(),
        workspace_root: record
            .workspace_root
            .clone()
            .context("missing bounded workspace reservation")?,
        trusted_source_repo: record.source.clone(),
        worker_output: record
            .checkout
            .clone()
            .context("no prepared candidate workspace")?,
    })
}
async fn stop_units(record: &Record, reason: &str) -> Result<JobStopEvidence> {
    let sandbox = Sandbox::for_recovery(&record.config)?;
    // Full labelled inventory is also positive absence evidence when a reserved
    // attempt failed before container creation. Never infer absence from a PID.
    for unit in sandbox.inventory().await? {
        if unit.run_id == record.identity.run_id {
            sandbox.stop(&unit.id).await?;
        }
    }
    ensure!(
        !sandbox
            .inventory()
            .await?
            .iter()
            .any(|u| u.run_id == record.identity.run_id && u.running),
        "job descendants still running"
    );
    ensure!(
        record.launch_complete || record.boot_id != boot_id()?,
        "same-boot launch outcome is uncertain; absence cannot establish termination"
    );
    Ok(JobStopEvidence {
        boot_id: boot_id()?,
        container_id: record.container_id.clone(),
        stopped_at: Utc::now().to_rfc3339(),
        reason: reason.into(),
        descendants_stopped: true,
    })
}

async fn snapshot_usage(record: &mut Record, worker: &Running) {
    match worker.gateway.usage().await {
        Ok(mut usage) => {
            usage.elapsed_seconds = record.usage.elapsed_seconds.max(
                Utc::now()
                    .signed_duration_since(record.started_at)
                    .num_seconds()
                    .max(0) as u64,
            );
            record.usage = usage;
        }
        Err(_) => {
            record.usage.spend_micro_usd = None;
        }
    }
}

async fn stop(
    record: &mut Record,
    dir: &Path,
    running: &mut Option<Running>,
    reason: &str,
    failed: bool,
) -> Result<()> {
    record.cancelled = true;
    record.state = JobState::Cancelling;
    record.stop_reason = Some(reason.into());
    record.save(dir)?;
    if let Some(worker) = running.as_mut() {
        let _ = worker.gateway.shutdown().await;
        snapshot_usage(record, worker).await;
    }
    if !record.launch_complete {
        // The create/start transaction may still be in flight. Its result is
        // awaited; an empty inventory now would not prove absence.
        return Ok(());
    }
    match stop_units(record, reason).await {
        Ok(evidence) => {
            record.stop_evidence = Some(evidence.clone());
            if record.delivery_in_flight {
                // An already-issued publication can still return a real PR.
                // Record stop evidence, but finish cancellation only after that
                // operation has persisted/reconciled its outcome.
                record.save(dir)?;
                return Ok(());
            }
            record.state = if failed {
                JobState::Failed
            } else {
                JobState::Cancelled
            };
            let seq = record.next_source();
            record.emit(
                dir,
                JobWorkerReport::Stopped {
                    identity: record.identity.clone(),
                    source_seq: seq,
                    evidence,
                    usage: record.usage.clone(),
                },
            )?;
            if failed {
                let uncertain = record.rpc.prompt == rpc::PromptAdmission::Uncertain
                    || record
                        .rpc
                        .questions
                        .iter()
                        .any(|q| q.state == JobQuestionState::Uncertain);
                if uncertain {
                    record.state = JobState::RecoveryRequired;
                    record.event(dir, JobEventKind::RecoveryRequired, reason, None)?;
                } else {
                    record.event(dir, JobEventKind::Failed, reason, None)?;
                }
            }
        }
        Err(error) => {
            record.state = JobState::RecoveryRequired;
            record.event(
                dir,
                JobEventKind::RecoveryRequired,
                "Execution stop is unconfirmed",
                Some(json!({"reason":reason,"error":error.to_string()})),
            )?;
        }
    }
    if let Some(mut worker) = running.take() {
        let _ = tokio::time::timeout(Duration::from_secs(3), worker.child.wait()).await;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Authority {
    lease: DateTime<Utc>,
    deadline: DateTime<Utc>,
    lease_clock: Instant,
    deadline_clock: Instant,
}
impl Authority {
    fn reason(&self, now: DateTime<Utc>, clock: Instant) -> Option<&'static str> {
        if now >= self.lease || clock >= self.lease_clock {
            Some("Execution authorization lease expired")
        } else if now >= self.deadline || clock >= self.deadline_clock {
            Some("Approved cumulative execution deadline expired")
        } else {
            None
        }
    }
}

fn independent_watchdog(
    mut record: Record,
    dir: PathBuf,
    cancelled: Arc<AtomicBool>,
    launch_finished: Arc<AtomicBool>,
    mut authority: tokio::sync::watch::Receiver<Option<Authority>>,
    events: mpsc::Sender<Work>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                changed=authority.changed()=>{if changed.is_err(){return}},
                _=interval.tick()=>{},
            }
            let Some(current) = *authority.borrow() else {
                return;
            };
            let reason = current.reason(Utc::now(), Instant::now());
            if reason.is_none() && !cancelled.load(Ordering::Acquire) {
                continue;
            }
            cancelled.store(true, Ordering::Release);
            // Cancellation fences create before waiting. If create already won,
            // await its bounded completion before publishing positive absence.
            while !launch_finished.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            record.launch_complete = true;
            let reason = reason.unwrap_or("Run execution fenced");
            if let Ok(evidence) = stop_units(&record, reason).await {
                let _ = atomic_json(&dir.join("emergency-stop.json"), &evidence);
            }
            let _ = events.send(Work::Expired(reason.into())).await;
            return;
        }
    });
}

/// This process survives daemon exec. Its timer and gateway are independent of
/// the controller connection; adoption never changes either deadline.
pub async fn run(dir: PathBuf) -> Result<()> {
    let original = load(&dir)?;
    let result = run_inner(dir.clone()).await;
    if result.is_err() {
        // Even journal/output failures fence the real execution unit. Retain
        // separate evidence if the primary event spool could not be committed.
        if let Ok(evidence) = stop_units(&original, "Trusted host exited with an error").await {
            let _ = atomic_json(&dir.join("emergency-stop.json"), &evidence);
        }
    }
    result
}

async fn run_inner(dir: PathBuf) -> Result<()> {
    private_dir(&dir)?;
    let lock = private_file(&dir.join("host.lock"), true)?;
    let _lock = ExecutionLock::acquire(lock).context("another host owns this attempt")?;
    let mut record = load(&dir)?;
    ensure!(
        record.boot_id == boot_id()?,
        "host reboot requires absence reconciliation, not worker recreation"
    );
    ensure!(
        !record.host_started
            && record.rpc.prompt == rpc::PromptAdmission::NotSubmitted
            && record.spool_seq == 0,
        "host restart cannot replay an existing attempt"
    );
    record.host_started = true;
    record.save(&dir)?;
    if record.socket.exists() {
        fs::remove_file(&record.socket)?;
    }
    let listener = UnixListener::bind(&record.socket)?;
    fs::set_permissions(&record.socket, fs::Permissions::from_mode(0o600))?;
    let mut credential = String::new();
    private_file(&dir.join("credential"), false)?.read_to_string(&mut credential)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let (tx, mut rx) = mpsc::channel(64);
    let startup_record = record.clone();
    let startup_dir = dir.clone();
    let startup_tx = tx.clone();
    let launch_finished = Arc::new(AtomicBool::new(false));
    let launched = launch_finished.clone();
    let launch_cancelled = cancelled.clone();
    let _startup = tokio::spawn(async move {
        let result = launch(startup_record, startup_dir, launch_cancelled).await;
        launched.store(true, Ordering::Release);
        let _ = startup_tx.send(Work::Started(result)).await;
    });
    let mut delivery_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut running: Option<Running> = None;
    let start = Instant::now();
    let mut lease_deadline = monotonic_deadline(record.lease_expires_at)?;
    let mut elapsed_deadline = monotonic_deadline(record.deadline)?;
    let (authority_tx, authority_rx) = tokio::sync::watch::channel(Some(Authority {
        lease: record.lease_expires_at,
        deadline: record.deadline,
        lease_clock: lease_deadline,
        deadline_clock: elapsed_deadline,
    }));
    independent_watchdog(
        record.clone(),
        dir.clone(),
        cancelled.clone(),
        launch_finished,
        authority_rx,
        tx.clone(),
    );
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let start_command = *record
        .receipts
        .keys()
        .next()
        .context("missing launch receipt")?;
    loop {
        if record.state.is_terminal() && !record.delivery_in_flight {
            record.terminal_at.get_or_insert_with(Utc::now);
            authority_tx.send_replace(None);
            if let Some(worker) = running.as_mut() {
                worker.gateway.shutdown().await?;
            }
            record.save(&dir)?;
            return Ok(());
        }
        tokio::select! {
            _=tick.tick()=>{
                if !record.cancelled && !record.state.is_terminal() {
                    record.usage.elapsed_seconds=start.elapsed().as_secs().max(Utc::now().signed_duration_since(record.started_at).num_seconds().max(0) as u64);
                    if let Some(worker)=running.as_ref(){match worker.gateway.usage().await{Ok(mut usage)=>{usage.elapsed_seconds=record.usage.elapsed_seconds;record.usage=usage},Err(_)=>{record.usage.spend_micro_usd=None;}}}
                    let reason=if Instant::now()>=lease_deadline || Utc::now()>=record.lease_expires_at {Some("Execution authorization lease expired")}else if Instant::now()>=elapsed_deadline || Utc::now()>=record.deadline {Some("Approved cumulative execution deadline expired")}else if record.usage.spend_micro_usd.is_none(){Some("Required model metering unavailable")}else if record.usage.spend_micro_usd.is_some_and(|s|s>=record.limits.spend_micro_usd){Some("Reported spend threshold reached")}else{None};
                    if let Some(reason)=reason {cancelled.store(true,Ordering::SeqCst);stop(&mut record,&dir,&mut running,reason,false).await?;}
                    else if start.elapsed().as_secs()%10==0 {record.event(&dir,JobEventKind::Usage,"Aggregate run usage",None)?;}
                }
            }
            incoming=listener.accept()=>{
                let (stream,_)=incoming?;
                let uid=stream.peer_cred()?.uid();
                if uid!=unsafe{libc::geteuid()}{continue}
                let (reader,mut writer)=stream.into_split();
                let request=tokio::time::timeout(Duration::from_secs(2),rpc::read_frame(&mut BufReader::new(reader),JOB_MAX_FRAME_BYTES)).await;
                let request=match request {Ok(Ok(value))=>serde_json::from_value::<Request>(value),_=>continue};
                let Ok(request)=request else {continue};
                if request.version!=HOST_VERSION || !constant_time_equal(request.credential.as_bytes(),credential.as_bytes()){continue}
                let mut response=Response{inventory:record.inventory(),reports:vec![],next:0,error:None};
                let mut purged=false;
                let result:Result<()>=async {
                    if matches!(request.operation,Operation::Adopt){
                        let changed=request.generation!=record.generation || request.controller!=record.controller;
                        record.adopt(request.generation,request.controller)?;
                        if changed{record.save(&dir)?;}
                    }else{
                        ensure!(request.generation==record.generation && request.controller==record.controller,"stale daemon connection");
                    }
                    match request.operation {
                        Operation::Adopt=>{},
                        Operation::Poll{after}=>{response=record.response(&dir,after)?;},
                        Operation::Command{command}=>{
                            ensure!(command_identity(&command)==Some(&record.identity),"stale run, target or execution epoch");
                            let id=command_id(&command);
                            if let Some(receipt)=record.receipts.get(&id){
                                ensure!(serde_json::to_value(&receipt.command)?==serde_json::to_value(&command)?,"command id reused with changed payload");
                                if let Some(report)=&receipt.result{response.reports.push(report.clone());return Ok(());}
                                if !matches!(command,JobWorkerCommand::Cancel{..}|JobWorkerCommand::RefreshPublication{..}|JobWorkerCommand::PurgeRun{..}){return Ok(());}
                            }
                            if !matches!(command,JobWorkerCommand::Cancel{..}|JobWorkerCommand::RefreshPublication{..}|JobWorkerCommand::PurgeRun{..}){
                                ensure!(!record.cancelled && !cancelled.load(Ordering::Acquire) && !record.state.is_terminal(),"run fenced or terminal");
                                ensure!(Utc::now()<record.lease_expires_at && Utc::now()<record.deadline && Instant::now()<lease_deadline && Instant::now()<elapsed_deadline,"expired execution authority cannot be renewed or used");
                            }
                            record.receipts.insert(id,Receipt{command:command.clone(),result:None});record.save(&dir)?;
                            match command {
                                JobWorkerCommand::PurgeRun{..}=>{
                                    ensure!(!record.delivery_in_flight,"live delivery operation must settle before content purge");
                                    cancelled.store(true,Ordering::Release);
                                    if let Some(worker)=running.as_mut(){worker.gateway.shutdown().await?;}
                                    let report=purge_locked(&dir,&record,id).await?;
                                    authority_tx.send_replace(None);
                                    response.reports.push(report);purged=true;
                                    return Ok(());
                                }
                                JobWorkerCommand::RefreshPublication{..}=>{
                                    let ctx=delivery_context(&record,&dir)?;
                                    let config=crate::config::Config::load()?.autonomous_jobs;
                                    let publication=delivery::reconcile_publication(&ctx,&config).await?;
                                    let report=JobWorkerReport::PublicationObserved{command_id:id,identity:record.identity.clone(),publication};
                                    record.receipts.get_mut(&id).unwrap().result=Some(report.clone());
                                    record.emit(&dir,report.clone())?;
                                    response.reports.push(report);
                                    return Ok(());
                                }
                                JobWorkerCommand::RenewLease{lease_expires_at,limits,..}=>{
                                    let expires=parse_lease(&lease_expires_at)?;
                                    let resource_narrowed=limits.cpu_millis<record.limits.cpu_millis || limits.memory_bytes<record.limits.memory_bytes || limits.pids<record.limits.pids || limits.workspace_bytes<record.limits.workspace_bytes;
                                    if resource_narrowed {
                                        cancelled.store(true,Ordering::Release);
                                        stop(&mut record,&dir,&mut running,"Resource policy narrowed below the active sandbox's enforced ceiling",false).await?;
                                        let report=JobWorkerReport::Accepted{command_id:id,identity:record.identity.clone()};
                                        record.receipts.get_mut(&id).unwrap().result=Some(report.clone());record.save(&dir)?;
                                        response.reports.push(report);
                                        return Ok(());
                                    }
                                    record.limits=record.limits.narrowed_by(&limits);
                                    record.deadline=record.deadline.min(deadline_after(record.started_at,record.limits.elapsed_seconds)?);
                                    elapsed_deadline=elapsed_deadline.min(start.checked_add(Duration::from_secs(record.limits.elapsed_seconds)).context("elapsed allowance exceeds the monotonic clock range")?).min(monotonic_deadline(record.deadline)?);
                                    record.lease_expires_at=expires;
                                    lease_deadline=monotonic_deadline(expires)?;
                                    authority_tx.send_replace(Some(Authority{lease:record.lease_expires_at,deadline:record.deadline,lease_clock:lease_deadline,deadline_clock:elapsed_deadline}));
                                    if let Some(worker)=running.as_ref(){worker.gateway.narrow_limits(&record.limits).await?;worker.gateway.renew_lease(expires).await?;}
                                }
                                JobWorkerCommand::Answer{question_id,dialog_id,tool_call_id,answers,..}=>{
                                    let frame=record.rpc.reserve_answer(question_id,&dialog_id,&tool_call_id,&answers)?;
                                    record.save(&dir)?;
                                    let worker=running.as_mut().context("native worker unavailable; answer remains uncertain")?;
                                    rpc::write_frame(&mut worker.stdin,&frame).await?;
                                    record.state=JobState::Running;
                                }
                                JobWorkerCommand::Cancel{reason,..}=>{
                                    cancelled.store(true,Ordering::SeqCst);
                                    stop(&mut record,&dir,&mut running,&reason,false).await?;
                                }
                                JobWorkerCommand::Verify{..}|JobWorkerCommand::Publish{..}=>{
                                    ensure!(record.task.worker.as_ref().is_none_or(|context| context.purpose == WorkerRunPurpose::Implementation),"inspection has no verification or publication authority");
                                    ensure!(record.rpc.settled && record.stop_evidence.as_ref().is_some_and(|e|e.descendants_stopped),"worker mutation has not positively stopped");
                                    ensure!(delivery_task.as_ref().is_none_or(|h|h.is_finished()),"delivery operation already in flight");
                                    let ctx=delivery_context(&record,&dir)?;let config=record.config.clone();let fence=delivery::DeliveryFence::new(cancelled.clone(),record.deadline);let send=tx.clone();let identity=record.identity.clone();
                                    record.state=if matches!(command,JobWorkerCommand::Verify{..}){JobState::Verifying}else{JobState::Publishing};
                                    record.delivery_in_flight=true;record.save(&dir)?;
                                    delivery_task=Some(tokio::spawn(async move {
                                        let result=match command {
                                            JobWorkerCommand::Verify{..}=>delivery::verify_candidate(&ctx,&config,&fence).await.map(|review|JobWorkerReport::Verified{command_id:id,identity,review}),
                                            JobWorkerCommand::Publish{candidate_sha,..}=>delivery::publish_candidate(&ctx,&config,&candidate_sha,&fence).await.map(|publication|JobWorkerReport::Published{command_id:id,identity,publication}),
                                            _=>unreachable!(),
                                        };
                                        let _=send.send(Work::Delivery(id,result)).await;
                                    }));
                                }
                                _=>bail!("unsupported command on existing run"),
                            }
                            let accepted=JobWorkerReport::Accepted{command_id:id,identity:record.identity.clone()};
                            record.receipts.get_mut(&id).unwrap().result=Some(accepted.clone());record.save(&dir)?;response.reports.push(accepted);
                        }
                    }
                    Ok(())
                }.await;
                if let Err(error)=result {response.error=Some(error.to_string());}
                response.inventory=record.inventory();
                let _=tokio::time::timeout(Duration::from_secs(3),rpc::write_frame(&mut writer,&serde_json::to_value(response)?)).await;
                if purged{return Ok(())}
            }
            Some(work)=rx.recv()=>{
                match work {
                    Work::Started(result)=>{
                        record.launch_complete=true;record.save(&dir)?;
                        match result {
                        Ok((mut worker,workspace))=>{
                            read_worker_pipes(&mut worker,&dir,record.limits.output_bytes,tx.clone())?;
                            record.workspace_root=Some(Sandbox::new(&record.config)?.workspace_root(record.identity.run_id)?);
                            running=Some(worker);record.checkout=Some(workspace.checkout);record.session_dir=Some(workspace.session);record.save(&dir)?;
                            if record.cancelled {
                                let reason=record.stop_reason.clone().unwrap_or_else(||"Cancelled during preparation".into());
                                stop(&mut record,&dir,&mut running,&reason,false).await?;
                            } else if let Some(worker)=running.as_ref() {
                                worker.gateway.narrow_limits(&record.limits).await?;
                                worker.gateway.renew_lease(record.lease_expires_at).await?;
                            }
                        }
                        Err(error)=>{
                            let was_cancelled=record.cancelled;
                            cancelled.store(true,Ordering::SeqCst);
                            let reason=record.stop_reason.clone().unwrap_or_else(||format!("Worker preparation failed: {error}"));
                            stop(&mut record,&dir,&mut running,&reason,!was_cancelled).await?;
                        }
                        }
                    },
                    Work::Frame(frame) if !record.cancelled && !record.rpc.settled && running.is_some()=>{
                        let effects=frame.and_then(|frame|record.rpc.accept(&record.identity,frame));
                        match effects {
                            Ok(effects)=>{
                                record.save(&dir)?;
                                for effect in effects {
                                    let result:Result<()>=async {match effect {
                                        rpc::Effect::Send(value)=>rpc::write_frame(&mut running.as_mut().context("worker stopped")?.stdin,&value).await?,
                                        rpc::Effect::Qualified=>{
                                            ensure!(!cancelled.load(Ordering::Acquire) && Utc::now()<record.lease_expires_at && Utc::now()<record.deadline,"prompt admission authority expired");
                                            let message=super::result::prompt(&record.task,&record.identity)?;
                                            let frame=record.rpc.reserve_prompt(message)?;record.save(&dir)?;
                                            rpc::write_frame(&mut running.as_mut().context("worker stopped")?.stdin,&frame).await?;
                                        }
                                        rpc::Effect::PromptConfirmed=>{
                                            record.state=JobState::Running;
                                            let report=JobWorkerReport::Accepted{command_id:start_command,identity:record.identity.clone()};record.receipts.get_mut(&start_command).unwrap().result=Some(report.clone());record.emit(&dir,report)?;
                                            record.event(&dir,JobEventKind::PhaseChanged,"Native prompt admission confirmed",Some(json!({"state":"running"})))?;
                                        }
                                        rpc::Effect::Progress(summary,detail)=>record.event(&dir,JobEventKind::Progress,&summary,detail)?,
                                        rpc::Effect::Child(detail)=>record.event(&dir,JobEventKind::Subagent,"Native delegated work",Some(detail))?,
                                        rpc::Effect::Question(question)=>{record.state=JobState::NeedsInput;let seq=record.next_source();record.emit(&dir,JobWorkerReport::Question{identity:record.identity.clone(),source_seq:seq,question})?;}
                                        rpc::Effect::AnswerConfirmed(question)=>{let seq=record.next_source();record.emit(&dir,JobWorkerReport::AnswerConfirmed{identity:record.identity.clone(),source_seq:seq,question})?;}
                                        rpc::Effect::Settled=>{
                                            let worker=running.as_mut().context("worker missing at settlement")?;
                                            worker.gateway.deny_models().await?;
                                            let owned=record.clone();let ctx=delivery_context(&record,&dir)?;let send=tx.clone();
                                            let fence=delivery::DeliveryFence::new(cancelled.clone(),record.deadline);
                                            record.delivery_in_flight=true;record.save(&dir)?;
                                            delivery_task=Some(tokio::spawn(async move {
                                                let result=async {
                                                    let evidence=stop_units(&owned,"Native session settled; validating bounded result").await?;
                                                    let result=owned.task.worker.as_ref().map(|context| super::result::read_result(
                                                        owned.workspace_root.as_deref().context("missing workspace reservation")?,
                                                        owned.session_dir.as_deref().context("missing worker session")?,
                                                        context,&evidence,owned.limits.output_bytes)).transpose()?;
                                                    if result.as_ref().is_none_or(|result| result.disposition == WorkerResultDisposition::CandidateReady) {
                                                        delivery::freeze_candidate(&ctx,&evidence,&fence).await?;
                                                    }
                                                    Ok((evidence,result))
                                                }.await;
                                                let _=send.send(Work::Frozen(result)).await;
                                            }));
                                        }
                                    };Ok(())}.await;
                                    if let Err(error)=result {cancelled.store(true,Ordering::SeqCst);stop(&mut record,&dir,&mut running,&format!("Runtime protocol failure: {error}"),true).await?;break}
                                }
                            }
                            Err(error)=>{cancelled.store(true,Ordering::SeqCst);stop(&mut record,&dir,&mut running,&format!("Native protocol quarantined: {error}"),true).await?;}
                        }
                    }
                    Work::Frozen(result)=>{
                        record.delivery_in_flight=false;record.save(&dir)?;
                        match result {
                        Ok((evidence,result)) if !record.cancelled=>{
                            record.stop_evidence=Some(evidence.clone());
                            let candidate_ready=result.as_ref().is_none_or(|result| result.disposition == WorkerResultDisposition::CandidateReady);
                            if let Some(mut worker)=running.take(){
                                worker.gateway.shutdown().await?;
                                snapshot_usage(&mut record,&worker).await;
                                let _=tokio::time::timeout(Duration::from_secs(3),worker.child.wait()).await;
                            }
                            record.state=if candidate_ready {JobState::Verifying}
                                else if result.as_ref().is_some_and(|result| matches!(result.disposition,WorkerResultDisposition::NoWork|WorkerResultDisposition::SelectWork)) {JobState::Completed}
                                else {JobState::Failed};
                            if let Some(result)=result {
                                let seq=record.next_source();record.emit(&dir,JobWorkerReport::WorkerResult{identity:record.identity.clone(),source_seq:seq,result,evidence:evidence.clone(),usage:record.usage.clone()})?;
                            }
                            record.event(&dir,JobEventKind::PhaseChanged,if candidate_ready {"Worker stopped and immutable candidate frozen"} else {"Worker stopped and bounded result retained"},Some(json!({"state":record.state,"stop_evidence":evidence})))?;
                            if candidate_ready {
                                let seq=record.next_source();record.emit(&dir,JobWorkerReport::Settled{identity:record.identity.clone(),source_seq:seq,usage:record.usage.clone()})?;
                            }
                        }
                        Err(error) if !record.cancelled=>{cancelled.store(true,Ordering::SeqCst);stop(&mut record,&dir,&mut running,&format!("Worker result validation or candidate freeze failed: {error}"),true).await?;}
                        _=>{
                            let reason=record.stop_reason.clone().unwrap_or_else(||"Cancelled during candidate freeze".into());
                            stop(&mut record,&dir,&mut running,&reason,false).await?;
                        }
                        }
                    },
                    Work::Expired(reason)=>{
                        if !record.cancelled && !record.state.is_terminal() {
                            stop(&mut record,&dir,&mut running,&reason,false).await?;
                        }
                    }
                    Work::Delivery(id,result)=>{
                        record.delivery_in_flight=false;
                        match result {
                            Ok(report)=>{
                                if matches!(report,JobWorkerReport::Published{..}) && !record.cancelled {record.state=JobState::ReviewReady;}
                                if let JobWorkerReport::Verified{review,..}=&report {
                                    let passed=review.verification.as_ref().is_some_and(|v|!v.checks.is_empty() && v.checks.iter().all(|c|c.status==JobCheckStatus::Passed && c.exit_code==Some(0)));
                                    if !passed && !record.cancelled {record.state=JobState::Failed;}
                                }
                                if record.state.is_terminal(){
                                    authority_tx.send_replace(None);
                                    if let Some(worker)=running.as_mut(){worker.gateway.shutdown().await?;}
                                }
                                if let Some(worker)=running.as_ref(){snapshot_usage(&mut record,worker).await;}
                                record.event(&dir,JobEventKind::Usage,"Final aggregate usage for delivery phase",None)?;
                                // Verification references only complete uploaded evidence.
                                let ctx=delivery_context(&record,&dir)?;
                                for artifact in delivery::artifacts(&ctx)? {
                                    if record.spooled_artifacts.contains(&artifact.id){continue}
                                    let mut offset=0;
                                    loop {
                                        record.emit(&dir,delivery::read_artifact_chunk(&ctx,artifact.id,offset,64*1024)?)?;
                                        offset+=64*1024;
                                        if offset>=artifact.size_bytes{break}
                                    }
                                    record.spooled_artifacts.insert(artifact.id);record.save(&dir)?;
                                }
                                record.receipts.get_mut(&id).context("missing delivery receipt")?.result=Some(report.clone());record.emit(&dir,report)?;
                            }
                            Err(error)=>{
                                let report=JobWorkerReport::Rejected{command_id:id,identity:Some(record.identity.clone()),code:if record.cancelled{"cancelled"}else{"delivery_failed"}.into(),message:error.to_string()};
                                record.receipts.get_mut(&id).context("missing delivery receipt")?.result=Some(report.clone());record.emit(&dir,report)?;
                                if !record.cancelled {record.state=JobState::Blocked;record.event(&dir,JobEventKind::Failed,"Delivery failed; retained candidate preserved",None)?;}
                            }
                        }
                        if record.cancelled {
                            let reason=record.stop_reason.clone().unwrap_or_else(||"Cancelled during delivery".into());
                            stop(&mut record,&dir,&mut running,&reason,false).await?;
                        }
                    }
                    _=>{}
                }
            }
        }
    }
}
fn constant_time_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |v, (a, b)| v | (a ^ b)) == 0
}

/// A dead host never resumes an opaque pipe. Reconcile all labelled execution
/// units under the local lock; uncertainty remains visible until positive stop.
pub async fn reconcile_dead(dir: &Path) -> Result<JobRunInventory> {
    let lock = private_file(&dir.join("host.lock"), true)?;
    let _lock = ExecutionLock::acquire(lock).context("host is still alive; do not replace it")?;
    let mut record = load(dir)?;
    if !record.state.is_terminal()
        && !(record.cancelled
            && record.state == JobState::RecoveryRequired
            && record.stop_evidence.is_some())
    {
        let was_recovery = record.state == JobState::RecoveryRequired && record.cancelled;
        record.cancelled = true;
        if !record.launch_complete {
            if let Ok(evidence) = read_json::<JobStopEvidence>(&dir.join("emergency-stop.json")) {
                if evidence.descendants_stopped
                    && evidence.container_id == record.container_id
                    && evidence.boot_id == boot_id()?
                {
                    record.launch_complete = true;
                }
            }
        }
        match stop_units(
            &record,
            "Host ownership lost; prompt/answer admission must not be replayed",
        )
        .await
        {
            Ok(evidence) => {
                record.stop_evidence = Some(evidence.clone());
                let seq = record.next_source();
                record.emit(
                    dir,
                    JobWorkerReport::Stopped {
                        identity: record.identity.clone(),
                        source_seq: seq,
                        evidence,
                        usage: record.usage.clone(),
                    },
                )?;
            }
            Err(_) => {}
        }
        record.state = JobState::RecoveryRequired;
        if !was_recovery {
            record.event(
                dir,
                JobEventKind::RecoveryRequired,
                "Persistent host lost; retained attempt requires explicit recovery",
                None,
            )?;
        } else {
            record.save(dir)?;
        }
    }
    Ok(record.inventory())
}
pub fn offline_reports(dir: &Path, after: u64) -> Result<Response> {
    load(dir)?.response(dir, after)
}

/// Serialize Start and pre-launch Cancel on the provisioned local filesystem.
pub fn admission_lock(config: &AutonomousJobConfig) -> Result<ExecutionLock> {
    let root = config.storage_root();
    ensure!(root.exists(), "provisioned job control root is unavailable");
    private_dir(&root)?;
    let lock = private_file(&root.join("admission.lock"), true)?;
    ExecutionLock::acquire(lock).context("another local admission is reserving ownership")
}

pub fn controller_generation(config: &AutonomousJobConfig) -> Result<u64> {
    let now: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos()
        .try_into()?;
    if !config.storage_root().exists() {
        return Ok(now);
    }
    let _lock = admission_lock(config)?;
    let path = config.storage_root().join("controller-generation.json");
    let previous = if path.exists() {
        read_json::<u64>(&path)?
    } else {
        0
    };
    let generation = now.max(
        previous
            .checked_add(1)
            .context("controller generation exhausted")?,
    );
    atomic_json(&path, &generation)?;
    Ok(generation)
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CancelTombstone {
    pub identity: JobRunIdentity,
    command: JobWorkerCommand,
    pub evidence: Option<JobStopEvidence>,
}
impl CancelTombstone {
    pub fn report(&self) -> Option<JobWorkerReport> {
        self.evidence
            .clone()
            .map(|evidence| JobWorkerReport::Stopped {
                identity: self.identity.clone(),
                source_seq: 1,
                evidence,
                usage: JobUsage {
                    spend_micro_usd: Some(0),
                    ..Default::default()
                },
            })
    }
    pub fn inventory(&self) -> JobRunInventory {
        JobRunInventory {
            identity: self.identity.clone(),
            state: if self.evidence.is_some() {
                JobState::Cancelled
            } else {
                JobState::RecoveryRequired
            },
            boot_id: self
                .evidence
                .as_ref()
                .map(|e| e.boot_id.clone())
                .unwrap_or_default(),
            container_id: Some(sandbox::worker_container_name(self.identity.run_id)),
            last_source_seq: u64::from(self.evidence.is_some()),
            prompt_confirmed: false,
            usage: JobUsage {
                spend_micro_usd: Some(0),
                ..Default::default()
            },
            stop_evidence: self.evidence.clone(),
        }
    }
}
pub fn tombstone(dir: &Path) -> Result<Option<CancelTombstone>> {
    let path = dir.join("cancelled-before-start.json");
    if path.exists() {
        Ok(Some(read_json(&path)?))
    } else {
        Ok(None)
    }
}
pub async fn cancel_before_start(
    config: &AutonomousJobConfig,
    command: &JobWorkerCommand,
) -> Result<JobWorkerReport> {
    let JobWorkerCommand::Cancel {
        identity, reason, ..
    } = command
    else {
        bail!("cancel command required")
    };
    let _lock = admission_lock(config)?;
    let dir = config.storage_root().join(identity.run_id.to_string());
    ensure!(
        purge_tombstone(&dir)?.is_none(),
        "run content was already purged"
    );
    ensure!(
        !dir.join("host.json").exists(),
        "launch already reserved; cancellation must target that host"
    );
    private_dir(&dir)?;
    let path = dir.join("cancelled-before-start.json");
    let mut tombstone = if path.exists() {
        read_json::<CancelTombstone>(&path)?
    } else {
        CancelTombstone {
            identity: identity.clone(),
            command: command.clone(),
            evidence: None,
        }
    };
    ensure!(
        tombstone.identity == *identity,
        "cancelled run has another execution identity"
    );
    atomic_json(&path, &tombstone)?;
    if let Some(report) = tombstone.report() {
        return Ok(report);
    }
    let sandbox = Sandbox::for_recovery(config)?;
    for unit in sandbox.inventory().await? {
        if unit.run_id == identity.run_id {
            sandbox.stop(&unit.id).await?;
        }
    }
    ensure!(
        !sandbox
            .inventory()
            .await?
            .iter()
            .any(|u| u.run_id == identity.run_id && u.running),
        "run-owned descendants remain"
    );
    tombstone.evidence = Some(JobStopEvidence {
        boot_id: boot_id()?,
        container_id: sandbox::worker_container_name(identity.run_id),
        stopped_at: Utc::now().to_rfc3339(),
        reason: reason.clone(),
        descendants_stopped: true,
    });
    atomic_json(&path, &tombstone)?;
    tombstone
        .report()
        .context("missing positive cancellation evidence")
}

pub async fn reconcile_cancel(
    config: &AutonomousJobConfig,
    tombstone: &CancelTombstone,
) -> Result<JobWorkerReport> {
    cancel_before_start(config, &tombstone.command).await
}
pub async fn purge_cancelled(
    config: &AutonomousJobConfig,
    command: &JobWorkerCommand,
) -> Result<JobWorkerReport> {
    let _admission = admission_lock(config)?;
    let identity = command_identity(command).context("missing purge identity")?;
    let dir = config.storage_root().join(identity.run_id.to_string());
    let lock = private_file(&dir.join("host.lock"), true)?;
    let _lock = ExecutionLock::acquire(lock).context("run ownership is still active")?;
    purge_cancelled_locked(config, &dir, command).await
}
async fn purge_cancelled_locked(
    config: &AutonomousJobConfig,
    dir: &Path,
    command: &JobWorkerCommand,
) -> Result<JobWorkerReport> {
    let JobWorkerCommand::PurgeRun {
        command_id,
        identity,
    } = command
    else {
        bail!("purge command required")
    };
    let cancelled = tombstone(dir)?.context("missing cancellation ownership tombstone")?;
    ensure!(cancelled.identity == *identity, "purge identity mismatch");
    ensure!(
        cancelled
            .evidence
            .as_ref()
            .is_some_and(|e| e.descendants_stopped),
        "cancellation has no positive stop evidence"
    );
    let sandbox = Sandbox::for_recovery(config)?;
    for unit in sandbox.inventory().await? {
        if unit.run_id == identity.run_id {
            sandbox.stop(&unit.id).await?;
        }
    }
    ensure!(
        !sandbox
            .inventory()
            .await?
            .iter()
            .any(|u| u.run_id == identity.run_id && u.running),
        "run descendants remain active"
    );
    let evidence = JobStopEvidence {
        boot_id: boot_id()?,
        container_id: sandbox::worker_container_name(identity.run_id),
        stopped_at: Utc::now().to_rfc3339(),
        reason: "Project content deletion after positive execution stop".into(),
        descendants_stopped: true,
    };
    let mut purged = PurgeTombstone {
        identity: identity.clone(),
        command_id: *command_id,
        evidence,
        resources_released: false,
        complete: false,
    };
    atomic_json(&dir.join("purge.json"), &purged)?;
    sandbox.release_workspace(identity.run_id).await?;
    purged.resources_released = true;
    atomic_json(&dir.join("purge.json"), &purged)?;
    finish_purge(dir, &mut purged)?;
    Ok(JobWorkerReport::Purged {
        command_id: *command_id,
        identity: identity.clone(),
    })
}

/// Read-only external refresh and stop remain possible after a completed host
/// exits. This path never starts an OMP process or creates another attempt.
pub async fn offline_command(
    dir: &Path,
    generation: u64,
    controller: Uuid,
    command: &JobWorkerCommand,
) -> Result<Vec<JobWorkerReport>> {
    let lock = private_file(&dir.join("host.lock"), true)?;
    let _lock = ExecutionLock::acquire(lock)
        .context("existing host is alive; reconcile its authenticated socket")?;
    if let Some(mut tombstone) = purge_tombstone(dir)? {
        ensure!(
            command_identity(command) == Some(&tombstone.identity),
            "purged run has another identity"
        );
        ensure!(
            matches!(command, JobWorkerCommand::PurgeRun { .. }),
            "run content was deleted; execution cannot restart"
        );
        if tombstone.resources_released {
            finish_purge(dir, &mut tombstone)?;
            return Ok(vec![JobWorkerReport::Purged {
                command_id: command_id(command),
                identity: tombstone.identity,
            }]);
        }
    }
    if matches!(command, JobWorkerCommand::PurgeRun { .. }) && tombstone(dir)?.is_some() {
        return Ok(vec![
            purge_cancelled_locked(
                &crate::config::Config::load()?.autonomous_jobs,
                dir,
                command,
            )
            .await?,
        ]);
    }
    let mut record = load(dir)?;
    ensure!(
        command_identity(command) == Some(&record.identity),
        "stale execution identity"
    );
    record.adopt(generation, controller)?;
    let id = command_id(command);
    if let Some(receipt) = record.receipts.get(&id) {
        ensure!(
            serde_json::to_value(&receipt.command)? == serde_json::to_value(command)?,
            "command id reused with changed payload"
        );
        if let Some(result) = &receipt.result {
            return Ok(vec![result.clone()]);
        }
        ensure!(
            matches!(
                command,
                JobWorkerCommand::RefreshPublication { .. }
                    | JobWorkerCommand::Cancel { .. }
                    | JobWorkerCommand::PurgeRun { .. }
            ),
            "uncertain operation cannot be blindly replayed"
        );
    }
    if matches!(command, JobWorkerCommand::PurgeRun { .. }) {
        return Ok(vec![purge_locked(dir, &record, id).await?]);
    }
    record.receipts.insert(
        id,
        Receipt {
            command: command.clone(),
            result: None,
        },
    );
    record.save(dir)?;
    let report = match command {
        JobWorkerCommand::RefreshPublication { .. } => {
            let ctx = delivery_context(&record, dir)?;
            let config = crate::config::Config::load()?.autonomous_jobs;
            let publication = delivery::reconcile_publication(&ctx, &config).await?;
            if publication.is_some() {
                record.delivery_in_flight = false;
            }
            JobWorkerReport::PublicationObserved {
                command_id: id,
                identity: record.identity.clone(),
                publication,
            }
        }
        JobWorkerCommand::Cancel { reason, .. } => {
            ensure!(!record.delivery_in_flight,"publisher ownership is unresolved; reconcile retained transaction before stop completion");
            let evidence = stop_units(&record, reason).await?;
            record.stop_evidence = Some(evidence.clone());
            record.cancelled = true;
            record.state = JobState::Cancelled;
            record.terminal_at.get_or_insert_with(Utc::now);
            let seq = record.next_source();
            JobWorkerReport::Stopped {
                identity: record.identity.clone(),
                source_seq: seq,
                evidence,
                usage: record.usage.clone(),
            }
        }
        _ => bail!("missing persistent execution host; original attempt cannot be replayed"),
    };
    record.receipts.get_mut(&id).unwrap().result = Some(report.clone());
    record.emit(dir, report.clone())?;
    Ok(vec![report])
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PurgeTombstone {
    pub identity: JobRunIdentity,
    pub command_id: Uuid,
    pub evidence: JobStopEvidence,
    pub resources_released: bool,
    pub complete: bool,
}
pub fn purge_tombstone(dir: &Path) -> Result<Option<PurgeTombstone>> {
    let path = dir.join("purge.json");
    if path.exists() {
        Ok(Some(read_json(&path)?))
    } else {
        Ok(None)
    }
}
fn finish_purge(dir: &Path, tombstone: &mut PurgeTombstone) -> Result<()> {
    ensure!(
        tombstone.resources_released,
        "external execution/storage release is unconfirmed"
    );
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if matches!(entry.file_name().to_str(), Some("host.lock" | "purge.json")) {
            continue;
        }
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
    }
    File::open(dir)?.sync_all()?;
    tombstone.complete = true;
    atomic_json(&dir.join("purge.json"), tombstone)?;
    Ok(())
}
async fn purge_locked(dir: &Path, record: &Record, command_id: Uuid) -> Result<JobWorkerReport> {
    ensure!(
        record
            .stop_evidence
            .as_ref()
            .is_some_and(|e| e.descendants_stopped),
        "content purge requires retained positive stop evidence"
    );
    let _delivery_guard = if dir.join("delivery").exists() {
        Some(delivery::acquire_purge_guard(&delivery_context(
            record, dir,
        )?)?)
    } else {
        None
    };
    let mut evidence = stop_units(
        record,
        "Project content deletion after positive execution stop",
    )
    .await?;
    evidence.reason = "Project content deletion after positive execution stop".into();
    let mut tombstone = PurgeTombstone {
        identity: record.identity.clone(),
        command_id,
        evidence,
        resources_released: false,
        complete: false,
    };
    atomic_json(&dir.join("purge.json"), &tombstone)?;
    Sandbox::for_recovery(&record.config)?
        .release_workspace(record.identity.run_id)
        .await?;
    tombstone.resources_released = true;
    atomic_json(&dir.join("purge.json"), &tombstone)?;
    // The exact owned socket is outside the pane-host namespace.
    if record.socket.exists() {
        fs::remove_file(&record.socket)?;
    }
    finish_purge(dir, &mut tombstone)?;
    Ok(JobWorkerReport::Purged {
        command_id,
        identity: record.identity.clone(),
    })
}

pub async fn prune_retained(dir: &Path, now: DateTime<Utc>) -> Result<()> {
    let lock = private_file(&dir.join("host.lock"), true)?;
    let _lock = ExecutionLock::acquire(lock).context("live host retains ownership")?;
    let mut record = load(dir)?;
    let expired = record
        .terminal_at
        .is_some_and(|at| now >= at + chrono::Duration::days(30));
    if !expired
        || !record.state.is_terminal()
        || !record.launch_complete
        || record.delivery_in_flight
        || record.receipts.values().any(|r| r.result.is_none())
        || !record
            .stop_evidence
            .as_ref()
            .is_some_and(|e| e.descendants_stopped)
    {
        return Ok(());
    }
    if dir.join("delivery").exists() {
        let ctx = delivery_context(&record, dir)?;
        if !delivery::workspace_release_eligible(&ctx, now)? {
            return Ok(());
        }
        delivery::prune_expired_artifacts(&ctx, now)?;
    }
    Sandbox::for_recovery(&record.config)?
        .release_workspace(record.identity.run_id)
        .await?;
    // Persist a replay tombstone before deleting bounded event/log content.
    record.events_expired = true;
    record.save(dir)?;
    for entry in fs::read_dir(dir.join("events"))? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            fs::remove_file(entry.path())?;
        }
    }
    File::open(dir.join("events"))?.sync_all()?;
    for name in ["worker.stderr", "host.stderr"] {
        let path = dir.join(name);
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    File::open(dir)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lease_is_bounded_and_expired_values_fail() {
        assert!(parse_lease(&(Utc::now() - chrono::Duration::seconds(1)).to_rfc3339()).is_err());
        assert!(parse_lease(&(Utc::now() + chrono::Duration::seconds(121)).to_rfc3339()).is_err());
        assert!(parse_lease(&(Utc::now() + chrono::Duration::seconds(119)).to_rfc3339()).is_ok());
    }
    #[test]
    fn unsupported_external_deadlines_are_controlled_errors() {
        assert!(deadline_after(Utc::now(), u64::MAX).is_err());
        assert!(deadline_after(Utc::now(), i64::MAX as u64).is_err());
        assert!(deadline_after(DateTime::<Utc>::MAX_UTC, 1).is_err());
        let now = Utc::now();
        assert_eq!(
            deadline_after(now, 120)
                .unwrap()
                .signed_duration_since(now)
                .num_seconds(),
            120
        );
    }
    #[test]
    fn private_journal_rejects_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        atomic_json(&target, &json!({"safe":true})).unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("link")).unwrap();
        assert!(read_json::<Value>(&dir.path().join("link")).is_err());
    }
    fn retained_record() -> Record {
        let identity = JobRunIdentity {
            project_id: "project".into(),
            job_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            cluster_owner_user_id: Uuid::new_v4(),
            machine_id: Uuid::new_v4(),
            epoch: 4,
        };
        let task = JobTaskSpec {
            worker: None,
            instruction: "Retained private task".into(),
            acceptance_criteria: vec![],
            source_url: None,
            repository: "owner/repo".into(),
            base_ref: "main".into(),
            base_sha: "a".repeat(40),
            worker_profile: "worker".into(),
            worker_profile_digest: "digest".into(),
            verification_profile: "verify".into(),
            verification_profile_digest: "digest".into(),
            model_profile: "model".into(),
        };
        let now = Utc::now();
        Record {
            rpc: rpc::Session::new(identity.run_id),
            identity,
            task,
            limits: JobLimits::default(),
            config: AutonomousJobConfig::default(),
            source: "/source".into(),
            boot_id: "boot".into(),
            container_id: "unit".into(),
            socket: "/socket".into(),
            started_at: now,
            deadline: now + chrono::Duration::seconds(600),
            lease_expires_at: now + chrono::Duration::seconds(100),
            state: JobState::Running,
            usage: JobUsage {
                elapsed_seconds: 42,
                spend_micro_usd: Some(123),
                ..Default::default()
            },
            stop_evidence: None,
            checkout: None,
            workspace_root: None,
            session_dir: None,
            generation: 3,
            controller: Uuid::new_v4(),
            source_seq: 9,
            spool_seq: 0,
            spool_bytes: 0,
            cancelled: false,
            host_started: true,
            launch_complete: true,
            stop_reason: None,
            delivery_in_flight: false,
            spooled_artifacts: Default::default(),
            terminal_at: None,
            events_expired: false,
            receipts: BTreeMap::new(),
        }
    }
    #[test]
    fn same_attempt_adoption_preserves_opaque_input_and_allowance() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = retained_record();
        record.task.worker = Some(serde_json::from_value(json!({
            "worker_id":Uuid::new_v4(),"mandate_revision":1,"backlog_revision":2,
            "schedule_generation":7,"reservation_id":Uuid::new_v4(),"purpose":"inspection",
            "mandate":{
                "title":"Inspect","description":"Inspect assigned responsibility","checklist":["Check correctness"],
                "machine_id":record.identity.machine_id,"worker_profile":"worker",
                "verification_profile":"verify","model_profile":"model","base_ref":"main",
                "interval_seconds":900,"limits":{"per_run":record.limits,
                    "elapsed_seconds_24h":7200,"spend_micro_usd_24h":20_000_000,
                    "runs_24h":2,"attempts_per_item":1}
            },
            "items":[],"selected_item_id":null,"checkpoint":null
        })).unwrap());
        record.rpc.prompt = rpc::PromptAdmission::Uncertain;
        record.save(dir.path()).unwrap();
        let mut adopted = load(dir.path()).unwrap();
        let controller = Uuid::new_v4();
        adopted.adopt(4, controller).unwrap();
        adopted.save(dir.path()).unwrap();
        let mut reexec = load(dir.path()).unwrap();
        assert_eq!(reexec.identity, record.identity);
        assert_eq!(reexec.task.worker, record.task.worker);
        assert_eq!(reexec.source_seq, 9);
        assert_eq!(reexec.usage, record.usage);
        assert_eq!(reexec.deadline, record.deadline);
        assert_eq!(reexec.lease_expires_at, record.lease_expires_at);
        assert!(reexec.rpc.reserve_prompt("unsafe replay".into()).is_err());
        assert!(reexec.adopt(3, record.controller).is_err());
        assert!(reexec.adopt(4, Uuid::new_v4()).is_err());
    }
    #[test]
    fn only_one_local_owner_can_reconcile_an_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.lock");
        let descriptor = private_file(&path, true).unwrap();
        // Model a concurrent fork retaining the same open file description.
        let inherited = descriptor.try_clone().unwrap();
        let first = ExecutionLock::acquire(descriptor).unwrap();
        assert!(ExecutionLock::acquire(private_file(&path, false).unwrap()).is_err());
        drop(first);
        let second = ExecutionLock::acquire(private_file(&path, false).unwrap()).unwrap();
        drop(inherited);
        assert!(ExecutionLock::acquire(private_file(&path, false).unwrap()).is_err());
        drop(second);
        assert!(ExecutionLock::acquire(private_file(&path, false).unwrap()).is_ok());
    }
    #[test]
    fn authorization_expiry_survives_wall_clock_rollback() {
        let now = Utc::now();
        let clock = Instant::now();
        let authority = Authority {
            lease: now + chrono::Duration::seconds(120),
            deadline: now + chrono::Duration::seconds(600),
            lease_clock: clock + Duration::from_secs(120),
            deadline_clock: clock + Duration::from_secs(600),
        };
        assert!(authority.reason(now, clock).is_none());
        assert_eq!(
            authority.reason(
                now - chrono::Duration::hours(1),
                clock + Duration::from_secs(121)
            ),
            Some("Execution authorization lease expired")
        );
        let deadline = Authority {
            lease: now + chrono::Duration::seconds(120),
            deadline: now + chrono::Duration::seconds(2),
            lease_clock: clock + Duration::from_secs(120),
            deadline_clock: clock + Duration::from_secs(2),
        };
        assert_eq!(
            deadline.reason(now, clock + Duration::from_secs(3)),
            Some("Approved cumulative execution deadline expired")
        );
    }
    #[test]
    fn fsynced_event_replay_keeps_source_identity() {
        let dir = tempfile::tempdir().unwrap();
        private_dir(&dir.path().join("events")).unwrap();
        let mut record = retained_record();
        record.save(dir.path()).unwrap();
        record
            .event(dir.path(), JobEventKind::Progress, "one", None)
            .unwrap();
        record
            .event(dir.path(), JobEventKind::Progress, "two", None)
            .unwrap();
        let restored = load(dir.path()).unwrap();
        let response = restored.response(dir.path(), 1).unwrap();
        assert_eq!(response.next, 2);
        assert_eq!(response.reports.len(), 1);
        assert!(
            matches!(&response.reports[0],JobWorkerReport::Event{source_seq:11,summary,..} if summary=="two")
        );
    }
    fn purge_marker(record: &Record, released: bool) -> PurgeTombstone {
        PurgeTombstone {
            identity: record.identity.clone(),
            command_id: Uuid::new_v4(),
            evidence: JobStopEvidence {
                boot_id: record.boot_id.clone(),
                container_id: record.container_id.clone(),
                stopped_at: Utc::now().to_rfc3339(),
                reason: "Project content deleted".into(),
                descendants_stopped: true,
            },
            resources_released: released,
            complete: false,
        }
    }
    #[test]
    fn purge_never_deletes_content_without_resource_release() {
        let dir = tempfile::tempdir().unwrap();
        let record = retained_record();
        record.save(dir.path()).unwrap();
        assert!(finish_purge(dir.path(), &mut purge_marker(&record, false)).is_err());
        assert!(dir.path().join("host.json").exists());
    }
    #[tokio::test]
    async fn interrupted_purge_resumes_without_replaying_execution() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("run");
        private_dir(&dir).unwrap();
        let record = retained_record();
        record.save(&dir).unwrap();
        private_dir(&dir.join("delivery")).unwrap();
        atomic_json(
            &dir.join("delivery/artifact.json"),
            &json!({"private":"task content"}),
        )
        .unwrap();
        let external = root.path().join("unrelated.json");
        atomic_json(&external, &json!({"keep":true})).unwrap();
        std::os::unix::fs::symlink(&external, dir.join("external")).unwrap();
        let marker = purge_marker(&record, true);
        atomic_json(&dir.join("purge.json"), &marker).unwrap();
        let command = JobWorkerCommand::PurgeRun {
            command_id: marker.command_id,
            identity: record.identity.clone(),
        };
        let first = offline_command(&dir, 4, Uuid::new_v4(), &command)
            .await
            .unwrap();
        let repeated = offline_command(&dir, 5, Uuid::new_v4(), &command)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(first).unwrap(),
            serde_json::to_value(repeated).unwrap()
        );
        assert!(!dir.join("host.json").exists());
        assert!(!dir.join("delivery").exists());
        assert!(!dir.join("external").exists());
        assert!(external.exists());
        assert!(purge_tombstone(&dir).unwrap().unwrap().complete);
        let start = JobWorkerCommand::Start {
            command_id: Uuid::new_v4(),
            identity: record.identity,
            task: record.task,
            limits: record.limits,
            policy_digest: "policy".into(),
            lease_expires_at: record.lease_expires_at.to_rfc3339(),
        };
        assert!(offline_command(&dir, 6, Uuid::new_v4(), &start)
            .await
            .is_err());
    }
}
