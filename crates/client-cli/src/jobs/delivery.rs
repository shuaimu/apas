//! Host-owned candidate evidence and draft-only delivery. Nothing in this module
//! executes worker Git configuration or passes publication credentials to a sandbox.

mod candidate;
mod publish;
mod verify;

pub use candidate::freeze_candidate;
pub use publish::{publish_candidate, reconcile_publication};
pub use verify::verify_candidate;

use super::ExecutionLock;
use anyhow::{bail, Context, Result};
use base64::Engine;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shared::{
    JobArtifact, JobArtifactKind, JobLimits, JobReview, JobRunIdentity, JobTaskSpec,
    JobWorkerReport,
};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use uuid::Uuid;

const SAFE_GIT_CONFIG: &[(&str, &str)] = &[
    ("core.hooksPath", "/dev/null"),
    ("core.fsmonitor", "false"),
    ("credential.helper", ""),
    ("commit.gpgSign", "false"),
    ("tag.gpgSign", "false"),
    ("protocol.allow", "never"),
    ("protocol.file.allow", "always"),
    ("uploadpack.packObjectsHook", ""),
    ("core.attributesFile", "/dev/null"),
    ("core.fsync", "all"),
    ("gc.auto", "0"),
    ("maintenance.auto", "false"),
];
const MAX_FILES: usize = 16_384;
const MAX_PAGE: usize = 128 * 1024;
const RETENTION_DAYS: i64 = 30;

#[derive(Debug, Clone)]
pub struct DeliveryContext {
    pub identity: JobRunIdentity,
    pub task: JobTaskSpec,
    pub limits: JobLimits,
    pub run_dir: PathBuf,
    /// Reserved quota-backed slot, outside every worker-visible mount.
    pub workspace_root: PathBuf,
    /// Original host-owned source, never a worker checkout or its Git directory.
    pub trusted_source_repo: PathBuf,
    pub worker_output: PathBuf,
}

#[derive(Clone)]
pub struct DeliveryFence {
    cancelled: Arc<AtomicBool>,
    deadline: DateTime<Utc>,
}

impl DeliveryFence {
    pub fn new(cancelled: Arc<AtomicBool>, deadline: DateTime<Utc>) -> Self {
        Self {
            cancelled,
            deadline,
        }
    }

    pub fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            bail!("delivery_cancelled");
        }
        if Utc::now() >= self.deadline {
            bail!("delivery_deadline_expired");
        }
        Ok(())
    }

    fn remaining_seconds(&self) -> Result<u64> {
        self.check()?;
        Ok((self.deadline - Utc::now()).num_seconds().max(1) as u64)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CandidateEntry {
    path: String,
    mode: String,
    bytes: u64,
    sha256: String,
    blob: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CandidateManifest {
    base_sha: String,
    candidate_sha: String,
    tree_sha: String,
    content_digest: String,
    files: Vec<CandidateEntry>,
    exclusions: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PublicationJournal {
    /// Written and fsynced before issuing a non-idempotent network operation.
    phase: String,
    candidate_sha: String,
    repository: String,
    branch: String,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeliveryRecord {
    identity: JobRunIdentity,
    task_digest: String,
    retain_until: String,
    review: JobReview,
    candidate: Option<CandidateManifest>,
    artifacts: Vec<JobArtifact>,
    publication: Option<PublicationJournal>,
}

impl DeliveryContext {
    fn root(&self) -> PathBuf {
        self.run_dir.join("delivery")
    }
    fn data_root(&self) -> PathBuf {
        self.workspace_root.join("delivery")
    }
    fn repo(&self) -> PathBuf {
        self.data_root().join("candidate.git")
    }

    fn lock(&self) -> Result<ExecutionLock> {
        private_dir(&self.root())?;
        let file = private_open(&self.root().join("operation.lock"), true)?;
        ExecutionLock::acquire(file).context("delivery_operation_in_progress")
    }

    fn load(&self) -> Result<DeliveryRecord> {
        let record: DeliveryRecord = read_json(&self.root().join("record.json"))?;
        if record.identity != self.identity
            || record.task_digest != digest(&serde_json::to_vec(&self.task)?)
        {
            bail!("delivery_identity_conflict");
        }
        Ok(record)
    }

    fn load_or_create(&self) -> Result<DeliveryRecord> {
        if self.root().join("record.json").exists() {
            return self.load();
        }
        Ok(DeliveryRecord {
            identity: self.identity.clone(),
            task_digest: digest(&serde_json::to_vec(&self.task)?),
            retain_until: (Utc::now() + ChronoDuration::days(RETENTION_DAYS)).to_rfc3339(),
            review: JobReview {
                job_id: self.identity.job_id,
                run_id: self.identity.run_id,
                base_sha: self.task.base_sha.clone(),
                candidate_sha: None,
                files: Vec::new(),
                verification: None,
                publication: None,
                reason: None,
                next_cursor: None,
            },
            candidate: None,
            artifacts: Vec::new(),
            publication: None,
        })
    }

    fn save(&self, record: &DeliveryRecord) -> Result<()> {
        atomic_json(&self.root().join("record.json"), record)
    }

    fn artifact_path(&self, id: Uuid) -> PathBuf {
        self.root().join("artifacts").join(id.to_string())
    }

    fn retain(
        &self,
        record: &mut DeliveryRecord,
        kind: JobArtifactKind,
        bytes: &[u8],
        content_type: &str,
        truncated: bool,
    ) -> Result<JobArtifact> {
        let used: u64 = record.artifacts.iter().map(|a| a.size_bytes).sum();
        let remaining = self.limits.output_bytes.saturating_sub(used) as usize;
        let keep = remaining.min(bytes.len());
        let truncated = truncated || keep < bytes.len();
        let bytes = &bytes[..keep];
        private_dir(&self.root().join("artifacts"))?;
        let artifact = JobArtifact {
            id: Uuid::new_v4(),
            run_id: self.identity.run_id,
            kind,
            sha256: digest(bytes),
            size_bytes: bytes.len() as u64,
            content_type: content_type.into(),
            expires_at: Some(record.retain_until.clone()),
            truncated,
        };
        atomic_bytes(&self.artifact_path(artifact.id), bytes)?;
        record.artifacts.push(artifact.clone());
        Ok(artifact)
    }
}

/// The caller must additionally prove terminal ownership and positive stop of
/// every worker/verifier/publisher before releasing the quota-backed slot.
pub fn workspace_release_eligible(ctx: &DeliveryContext, now: DateTime<Utc>) -> Result<bool> {
    let record = ctx.load()?;
    Ok(!publication_unresolved(&record)
        && DateTime::parse_from_rfc3339(&record.retain_until)? <= now)
}

fn publication_unresolved(record: &DeliveryRecord) -> bool {
    record.publication.as_ref().is_some_and(|journal| {
        matches!(
            journal.phase.as_str(),
            "push_in_flight" | "create_in_flight" | "create_uncertain"
        )
    })
}

/// Explicit project deletion may bypass retention, but not outstanding external
/// requests. Keep this guard alive while deleting content, and fence new host
/// commands before acquiring it. Container absence is the caller's responsibility.
pub fn acquire_purge_guard(ctx: &DeliveryContext) -> Result<ExecutionLock> {
    let guard = ctx.lock()?;
    if publication_unresolved(&ctx.load_or_create()?) {
        bail!("publication_reconciliation_required_before_purge");
    }
    Ok(guard)
}

/// Delete expired bytes, never their manifest or publication uncertainty.
pub fn prune_expired_artifacts(ctx: &DeliveryContext, now: DateTime<Utc>) -> Result<()> {
    let _lock = ctx.lock()?;
    for artifact in ctx.load()?.artifacts {
        if artifact
            .expires_at
            .as_deref()
            .map(DateTime::parse_from_rfc3339)
            .transpose()?
            .is_some_and(|expiry| expiry <= now)
        {
            match fs::remove_file(ctx.artifact_path(artifact.id)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}

fn ensure_retained(record: &DeliveryRecord) -> Result<()> {
    if DateTime::parse_from_rfc3339(&record.retain_until)? <= Utc::now() {
        bail!("candidate_retention_expired");
    }
    Ok(())
}

pub fn artifacts(ctx: &DeliveryContext) -> Result<Vec<JobArtifact>> {
    Ok(ctx.load()?.artifacts)
}

/// The runtime can spool one bounded wire frame at a time after worker exit.
pub fn read_artifact_chunk(
    ctx: &DeliveryContext,
    id: Uuid,
    offset: u64,
    max_bytes: usize,
) -> Result<JobWorkerReport> {
    let artifact = ctx
        .load()?
        .artifacts
        .into_iter()
        .find(|a| a.id == id)
        .context("unknown_artifact")?;
    if artifact
        .expires_at
        .as_deref()
        .map(DateTime::parse_from_rfc3339)
        .transpose()?
        .is_some_and(|t| t < Utc::now())
    {
        bail!("artifact_expired");
    }
    let (bytes, next) = artifact_bytes(ctx, &artifact, offset, max_bytes.clamp(1, MAX_PAGE))?;
    Ok(JobWorkerReport::ArtifactChunk {
        identity: ctx.identity.clone(),
        artifact,
        offset,
        data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        final_chunk: next.is_none(),
    })
}

fn artifact_bytes(
    ctx: &DeliveryContext,
    artifact: &JobArtifact,
    offset: u64,
    limit: usize,
) -> Result<(Vec<u8>, Option<u64>)> {
    if offset > artifact.size_bytes {
        bail!("invalid_artifact_cursor");
    }
    let mut file =
        private_open(&ctx.artifact_path(artifact.id), false).context("artifact_unavailable")?;
    if file.metadata()?.len() != artifact.size_bytes {
        bail!("artifact_size_mismatch");
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.take(limit as u64).read_to_end(&mut bytes)?;
    let end = offset + bytes.len() as u64;
    Ok((bytes, (end < artifact.size_bytes).then_some(end)))
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } {
        bail!("unsafe_delivery_directory");
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn private_open(path: &Path, create: bool) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(create)
        .create(create)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.nlink() != 1 {
        bail!("unsafe_delivery_file");
    }
    Ok(file)
}

fn atomic_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("missing_delivery_parent")?;
    private_dir(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    atomic_bytes(path, &serde_json::to_vec(value)?)
}
fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let file = private_open(path, false)?;
    serde_json::from_reader(file.take(32 * 1024 * 1024)).context("invalid_delivery_record")
}

struct GitOutput {
    stdout: Vec<u8>,
    truncated: bool,
}

fn git_command(ctx: &DeliveryContext, repo: Option<&Path>) -> Command {
    let mut command = Command::new("/usr/bin/git");
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", ctx.root())
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_AUTHOR_NAME", "APAS Autonomous Jobs")
        .env("GIT_AUTHOR_EMAIL", "autonomous-jobs@apas.invalid")
        .env("GIT_COMMITTER_NAME", "APAS Autonomous Jobs")
        .env("GIT_COMMITTER_EMAIL", "autonomous-jobs@apas.invalid")
        .env("GIT_AUTHOR_DATE", "@1 +0000")
        .env("GIT_COMMITTER_DATE", "@1 +0000");
    // Environment configuration also reaches upload-pack children. No ambient
    // global configuration, helper, signing, filters, hooks or SSH transport.
    let overrides = SAFE_GIT_CONFIG;
    command.env("GIT_CONFIG_COUNT", overrides.len().to_string());
    for (i, (key, value)) in overrides.iter().enumerate() {
        command
            .env(format!("GIT_CONFIG_KEY_{i}"), key)
            .env(format!("GIT_CONFIG_VALUE_{i}"), value);
    }
    if let Some(repo) = repo {
        command.arg(format!("--git-dir={}", repo.display()));
    }
    command
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

async fn drain<R: AsyncRead + Unpin>(mut reader: R, cap: usize) -> Result<(Vec<u8>, bool)> {
    let mut kept = Vec::new();
    let mut truncated = false;
    let mut buffer = [0u8; 8192];
    loop {
        let n = reader.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        let take = n.min(cap.saturating_sub(kept.len()));
        kept.extend_from_slice(&buffer[..take]);
        truncated |= take < n;
    }
    Ok((kept, truncated))
}

struct GitProcessGroup(Option<i32>);

impl Drop for GitProcessGroup {
    fn drop(&mut self) {
        if let Some(group) = self.0 {
            // Trusted Git helpers share this private process group. No publisher
            // helper can outlive a cancelled host operation with credentials.
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
    }
}

async fn run_git(
    mut command: Command,
    input: Vec<u8>,
    cap: usize,
    fence: &DeliveryFence,
) -> Result<GitOutput> {
    fence.check()?;
    use std::os::unix::process::CommandExt;
    command.as_std_mut().process_group(0);
    let mut child = command.spawn().context("trusted_git_spawn_failed")?;
    let mut group = GitProcessGroup(child.id().map(|id| id as i32));
    let mut stdin = child.stdin.take().context("git_stdin_missing")?;
    let stdout = child.stdout.take().context("git_stdout_missing")?;
    let stderr = child.stderr.take().context("git_stderr_missing")?;
    let io = async move {
        let writer = async move {
            stdin.write_all(&input).await?;
            stdin.shutdown().await
        };
        let (write, out, err) = tokio::join!(writer, drain(stdout, cap), drain(stderr, 16 * 1024));
        write?;
        Ok::<_, anyhow::Error>((out?, err?))
    };
    tokio::pin!(io);
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let ((stdout, truncated), (stderr, _)) = loop {
        tokio::select! {
            result = &mut io => break result?,
            _ = tick.tick() => if let Err(error) = fence.check() {
                if let Some(group) = group.0 { unsafe { libc::kill(-group, libc::SIGKILL); } }
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(error);
            },
        }
    };
    let status = loop {
        tokio::select! {
            status = child.wait() => break status?,
            _ = tick.tick() => if let Err(error) = fence.check() {
                if let Some(group) = group.0 { unsafe { libc::kill(-group, libc::SIGKILL); } }
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(error);
            },
        }
    };
    group.0 = None;
    if !status.success() {
        bail!("trusted_git_failed: {}", String::from_utf8_lossy(&stderr));
    }
    Ok(GitOutput { stdout, truncated })
}

async fn git(
    ctx: &DeliveryContext,
    args: &[&str],
    input: Vec<u8>,
    cap: usize,
    fence: &DeliveryFence,
) -> Result<GitOutput> {
    let mut command = git_command(ctx, Some(&ctx.repo()));
    command.args(args);
    run_git(command, input, cap, fence).await
}

fn git_text(output: GitOutput) -> Result<String> {
    if output.truncated {
        bail!("git_metadata_exceeds_bound");
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn valid_sha(sha: &str) -> bool {
    sha.len() == 40
        && sha
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
