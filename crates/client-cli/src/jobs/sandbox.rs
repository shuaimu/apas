//! Rootless Podman boundary. Nothing in this module falls back to host execution.
//!
//! Writable storage uses an operator-provisioned pool of finite local filesystems.
//! Each filesystem is exclusively reserved for one run and fits its allowance.
//! Control journals live on a different filesystem, outside worker-writeable quota.
//! APAS never mounts, resizes, reformats or silently reclaims retained run storage.

use super::config::{
    AutonomousJobConfig, VerificationCheckConfig, VerificationProfileConfig, WorkerProfileConfig,
};
use anyhow::{bail, ensure, Context, Result};
use chrono::{Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use shared::{
    JobLimits, JobModelProfile, JobVerificationProfile, JobWorkerProfile, JobWorkerReadiness,
};
use std::{
    ffi::CString,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
};
use uuid::Uuid;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_LOG_BYTES: u64 = 100 * 1024 * 1024;
const LABEL: &str = "io.apas.autonomous-job";

fn launch_allowance(deadline: tokio::time::Instant) -> Result<(Duration, u64)> {
    let remaining = deadline
        .saturating_duration_since(tokio::time::Instant::now())
        .as_secs();
    ensure!(remaining >= 2, "execution allowance expired during sandbox preflight; insufficient time to create and run a bounded container");
    let creation_seconds = (remaining / 2).min(COMMAND_TIMEOUT.as_secs());
    // Conmon starts its relative clock after create. Reserve the complete bounded
    // creation window instead of accidentally adding it to the admitted deadline.
    Ok((
        Duration::from_secs(creation_seconds),
        remaining - creation_seconds,
    ))
}

#[derive(Debug, Clone)]
pub struct PreparedWorkspace {
    pub checkout: PathBuf,
    pub session: PathBuf,
}

pub struct ContainerProcess {
    pub container_id: String,
    pub child: Child,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerIdentity {
    pub id: String,
    pub run_id: Uuid,
    pub running: bool,
}

#[derive(Debug)]
pub struct VerifierOutcome {
    pub container_id: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct Sandbox {
    root: PathBuf,
    workspace_roots: Vec<PathBuf>,
    podman: PathBuf,
    home: PathBuf,
    runtime_dir: PathBuf,
    isolated_podman: Option<(PathBuf, PathBuf)>,
    qualified_profiles: std::collections::BTreeMap<String, (String, String)>,
    recovery_only: bool,
}

pub fn worker_container_name(run_id: Uuid) -> String {
    format!("apas-job-{run_id}")
}

pub fn profile_digest<T: Serialize>(profile: &T) -> Result<String> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(profile)?)
    ))
}

pub(super) fn bootstrap_digest() -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(include_bytes!("omp-worker.mjs"))
    )
}

/// Bind admission and operational qualification to resolved capabilities and
/// policy, not merely the mutable keys referring to operator-owned profiles.
pub fn worker_profile_digest(
    config: &AutonomousJobConfig,
    worker: &WorkerProfileConfig,
) -> Result<String> {
    let mut models = Vec::with_capacity(worker.models.len());
    for key in &worker.models {
        let mut matches = config.models.iter().filter(|profile| &profile.key == key);
        let model = matches
            .next()
            .with_context(|| format!("model profile {key} is not configured"))?;
        ensure!(
            matches.next().is_none(),
            "model profile key {key} is ambiguous"
        );
        models.push(model);
    }
    let mut verifiers = Vec::with_capacity(worker.verifiers.len());
    for key in &worker.verifiers {
        let mut matches = config
            .verifiers
            .iter()
            .filter(|profile| &profile.key == key);
        let verifier = matches
            .next()
            .with_context(|| format!("verification profile {key} is not configured"))?;
        ensure!(
            matches.next().is_none(),
            "verification profile key {key} is ambiguous"
        );
        verifiers.push(verifier);
    }
    let mut publishers = config
        .publishers
        .iter()
        .filter(|profile| profile.key == worker.publisher);
    let publisher = publishers
        .next()
        .context("worker publisher profile is not configured")?;
    ensure!(
        publishers.next().is_none(),
        "worker publisher profile key is ambiguous"
    );
    profile_digest(&(
        worker,
        models,
        verifiers,
        publisher,
        shared::WORKER_EXECUTION_PROTOCOL_VERSION,
        bootstrap_digest(),
    ))
}

pub fn validate_image_pin(image: &str) -> Result<()> {
    let (repository, digest) = image
        .rsplit_once("@sha256:")
        .context("image must be operator-provided repository@sha256:<64 lowercase hex digits>")?;
    ensure!(
        !repository.is_empty()
            && !repository.starts_with('-')
            && !repository.chars().any(char::is_whitespace)
            && digest.len() == 64
            && digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "image must use an immutable lowercase sha256 digest, not a tag"
    );
    Ok(())
}

pub fn validate_credential(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute(),
        "credential file must be an absolute operator-provisioned path"
    );
    let metadata =
        std::fs::symlink_metadata(path).context("dedicated credential file is not provisioned")?;
    ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() > 0
            && metadata.len() <= 64 * 1024,
        "credential must be a nonempty private regular file owned by the daemon user (mode 0600)"
    );
    Ok(())
}

fn private_directory(path: &Path) -> Result<()> {
    let metadata =
        std::fs::symlink_metadata(path).context("private job directory is not provisioned")?;
    ensure!(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "job storage must be a private nonsymlink directory owned by the daemon user (mode 0700)"
    );
    Ok(())
}

fn create_private(path: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}

impl Sandbox {
    pub fn new(config: &AutonomousJobConfig) -> Result<Self> {
        Self::open(config, false)
    }

    /// Stop/reconcile historical units even when admission or profiles are disabled.
    /// This handle cannot prepare or launch any execution.
    pub fn for_recovery(config: &AutonomousJobConfig) -> Result<Self> {
        Self::open(config, true)
    }

    fn open(config: &AutonomousJobConfig, recovery_only: bool) -> Result<Self> {
        ensure!(
            recovery_only || config.enabled,
            "autonomous jobs are disabled in host configuration"
        );
        ensure!(
            cfg!(target_os = "linux"),
            "autonomous jobs require Linux rootless Podman"
        );
        ensure!(
            unsafe { libc::geteuid() } != 0,
            "autonomous jobs require a non-root host account"
        );
        let requested = config.storage_root();
        ensure!(requested.is_absolute(), "job storage root must be absolute");
        private_directory(&requested)?;
        let root = requested.canonicalize()?;
        ensure!(
            root == requested,
            "job storage root must not contain symlinks or parent components"
        );
        local_control_storage(&root)?;
        ensure!(
            recovery_only || !config.workspace_roots.is_empty(),
            "provision at least one dedicated bounded workspace filesystem slot"
        );
        let mut slot_devices = std::collections::HashSet::new();
        for slot in &config.workspace_roots {
            if recovery_only {
                continue;
            }
            private_directory(slot)?;
            ensure!(
                slot.is_absolute()
                    && slot.canonicalize()? == *slot
                    && !slot.starts_with(&root)
                    && !root.starts_with(slot),
                "workspace slots must be canonical paths separate from control storage"
            );
            ensure!(
                std::fs::metadata(slot)?.dev() != std::fs::metadata(&root)?.dev(),
                "workspace exhaustion must not consume the control journal filesystem"
            );
            ensure!(slot_devices.insert(std::fs::metadata(slot)?.dev()),
                "each workspace slot must have its own filesystem, not another mount of a shared device");
        }
        ensure!(
            config.podman_binary.is_absolute(),
            "podman_binary must be an absolute operator-owned executable"
        );
        let binary = std::fs::metadata(&config.podman_binary)
            .context("Podman executable is not installed")?;
        ensure!(
            binary.is_file() && binary.mode() & 0o022 == 0 && binary.mode() & 0o111 != 0,
            "Podman executable must not be group/world writable"
        );
        let isolated_podman = match (&config.podman_storage_root, &config.podman_runroot) {
            (Some(storage), Some(runroot)) => {
                for path in [storage, runroot] {
                    private_directory(path)?;
                    ensure!(path.is_absolute() && path.canonicalize()? == *path
                        && config.workspace_roots.iter().all(|slot| !path.starts_with(slot) && !slot.starts_with(path)),
                        "isolated Podman directories must be canonical and outside workspace slots");
                }
                local_control_storage(storage)?;
                Some((storage.clone(), runroot.clone()))
            }
            _ => bail!("provision both podman_storage_root and podman_runroot for the isolated vfs runtime"),
        };
        // Only the trusted runtime receives HOME/XDG_RUNTIME_DIR. Neither is copied
        // into the container environment. Do not silently repair a broken runroot.
        let home = dirs::home_dir().context("host account has no home for rootless Podman")?;
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::geteuid() })));
        private_directory(&runtime_dir).context(
            "provision a private XDG_RUNTIME_DIR for this UID; do not reuse another user's runroot",
        )?;
        let mut qualified_profiles = std::collections::BTreeMap::new();
        for profile in &config.profiles {
            if recovery_only {
                continue;
            }
            ensure!(
                qualified_profiles
                    .insert(
                        profile.key.clone(),
                        (
                            profile_digest(profile)?,
                            worker_profile_digest(config, profile)?
                        )
                    )
                    .is_none(),
                "worker profile keys must be unique"
            );
        }
        Ok(Self {
            root,
            workspace_roots: config.workspace_roots.clone(),
            podman: config.podman_binary.clone(),
            home,
            runtime_dir,
            isolated_podman,
            qualified_profiles,
            recovery_only,
        })
    }

    pub fn storage_root(&self) -> &Path {
        &self.root
    }

    fn reservation_path(&self, slot: &Path) -> PathBuf {
        self.root.join(format!(
            "slot-{:x}.owner",
            Sha256::digest(slot.as_os_str().as_bytes())
        ))
    }

    /// Resolve retained quota-backed data without starting or replacing an attempt.
    pub fn workspace_root(&self, run_id: Uuid) -> Result<PathBuf> {
        for slot in &self.workspace_roots {
            if std::fs::read_to_string(self.reservation_path(slot))
                .ok()
                .as_deref()
                == Some(&run_id.to_string())
            {
                return Ok(slot.join(run_id.to_string()));
            }
        }
        bail!("run has no durable workspace reservation; reconciliation is required")
    }

    /// Reclaim expired retained data while keeping all control/stop journals.
    ///
    /// The caller must hold the run ownership lock and durably establish terminal
    /// execution, expired retention (or authorized purge) and no unresolved operation.
    /// This does not cancel work: any live or uncertain container prevents release.
    pub async fn release_workspace(&self, run_id: Uuid) -> Result<()> {
        let containers: Vec<_> = self
            .inventory()
            .await?
            .into_iter()
            .filter(|entry| entry.run_id == run_id)
            .collect();
        ensure!(
            containers.iter().all(|entry| !entry.running),
            "workspace retention cleanup cannot release an active run"
        );
        for container in containers {
            let inspect: Value = serde_json::from_slice(
                &self
                    .output(&["inspect", "--type=container", &container.id])
                    .await?,
            )?;
            let state = inspect
                .pointer("/0/State")
                .context("retention cleanup lacks container stop evidence")?;
            ensure!(
                state.get("Running").and_then(Value::as_bool) == Some(false)
                    && state.get("Pid").and_then(Value::as_u64) == Some(0)
                    && state.get("Paused").and_then(Value::as_bool) != Some(true),
                "retention cleanup requires positive complete-container stop evidence"
            );
            // Never force removal: a competing start must cause cleanup to fail.
            self.output(&["rm", &container.id]).await?;
        }
        ensure!(
            self.inventory()
                .await?
                .iter()
                .all(|entry| entry.run_id != run_id),
            "run container appeared during retention cleanup; retain its reservation"
        );
        for slot in &self.workspace_roots {
            let reservation = self.reservation_path(slot);
            let owner = match std::fs::read_to_string(&reservation) {
                Ok(owner) => owner,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if owner != run_id.to_string() {
                continue;
            }
            let cleanup_limits = JobLimits {
                workspace_bytes: u64::MAX,
                ..JobLimits::default()
            };
            self.check_slot_state(slot, &cleanup_limits, false)?;
            let data = slot.join(run_id.to_string());
            match std::fs::symlink_metadata(&data) {
                Ok(metadata) => {
                    ensure!(
                        metadata.is_dir()
                            && !metadata.file_type().is_symlink()
                            && data.canonicalize()? == data,
                        "retained workspace identity changed; operator reconciliation required"
                    );
                    let mounts = std::fs::read_to_string("/proc/self/mountinfo")?;
                    let prefix = format!("{}/", data.display());
                    ensure!(
                        !mounts
                            .lines()
                            .filter_map(|line| line.split_whitespace().nth(4))
                            .any(|mount| mount == data.to_string_lossy()
                                || mount.starts_with(&prefix)),
                        "retained workspace contains a nested mount; refusing recursive cleanup"
                    );
                    Self::make_retained_directories_removable(&data, metadata.dev())?;
                    std::fs::remove_dir_all(&data)?;
                    std::fs::File::open(slot)?.sync_all()?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            // A crash before this unlink leaves a retained reservation, never an
            // available slot containing another run's source or credential data.
            std::fs::remove_file(reservation)?;
            std::fs::File::open(&self.root)?.sync_all()?;
        }
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("slot-") && name.ends_with(".owner") {
                ensure!(std::fs::read_to_string(entry.path())? != run_id.to_string(),
                    "run owns a workspace slot absent from current configuration; restore its mapping before purge");
            }
        }
        Ok(())
    }

    fn make_retained_directories_removable(path: &Path, device: u64) -> Result<()> {
        // All containers are absent and the caller holds the run lock. Do not follow
        // worker-created symlinks or cross a filesystem while restoring traversal.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
        let mut directories = vec![std::fs::read_dir(path)?];
        while let Some(directory) = directories.last_mut() {
            let Some(entry) = directory.next() else {
                directories.pop();
                continue;
            };
            let entry = entry?;
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                ensure!(
                    metadata.dev() == device && metadata.uid() == unsafe { libc::geteuid() },
                    "retained directory ownership/filesystem changed; operator cleanup required"
                );
                ensure!(directories.len() < 256, "retained directory nesting exceeds safe cleanup depth; operator cleanup required");
                std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o700))?;
                directories.push(std::fs::read_dir(entry.path())?);
            }
        }
        Ok(())
    }

    fn reserve_workspace(&self, run_id: Uuid, limits: &JobLimits) -> Result<PathBuf> {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        if let Ok(existing) = self.workspace_root(run_id) {
            self.check_slot(
                existing.parent().context("workspace slot has no parent")?,
                limits,
            )?;
            return Ok(existing);
        }
        for slot in &self.workspace_roots {
            if self.check_slot(slot, limits).is_err() {
                continue;
            }
            let reservation = self.reservation_path(slot);
            let mut file = match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&reservation)
            {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            };
            file.write_all(run_id.to_string().as_bytes())?;
            file.sync_all()?;
            std::fs::File::open(&self.root)?.sync_all()?;
            let run_root = slot.join(run_id.to_string());
            create_private(&run_root)?;
            return Ok(run_root);
        }
        bail!("no unreserved workspace filesystem fits the admitted allowance; provision another bounded slot or explicitly retire retained data")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.podman);
        if let Some((storage, runroot)) = &self.isolated_podman {
            command
                .arg("--root")
                .arg(storage)
                .arg("--runroot")
                .arg(runroot)
                .arg("--storage-driver=vfs");
        }
        command
            .env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("HOME", &self.home)
            .env("XDG_RUNTIME_DIR", &self.runtime_dir)
            .env("LANG", "C.UTF-8")
            .stdin(Stdio::null())
            .kill_on_drop(true);
        // Sandbox::open rejects non-Linux hosts, but its runtime check cannot
        // keep the compiler from resolving Linux-only libc symbols on macOS.
        #[cfg(target_os = "linux")]
        {
            // An orphaned create/start client must not launch work after its trusted
            // host dies. The detached conmon still owns the container's hard timeout.
            let parent = unsafe { libc::getpid() };
            unsafe {
                command.pre_exec(move || {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::getppid() != parent {
                        return Err(std::io::Error::from_raw_os_error(libc::ECHILD));
                    }
                    Ok(())
                });
            }
        }
        command
    }

    async fn output(&self, args: &[&str]) -> Result<Vec<u8>> {
        let mut command = self.command();
        command.args(args);
        let output = tokio::time::timeout(COMMAND_TIMEOUT, command.output())
            .await
            .context("rootless Podman command timed out; inspect host runtime/runroot")??;
        // Do not expose stderr, which may contain configured host paths or secrets.
        ensure!(output.status.success(), "rootless Podman {} failed ({}); operator must inspect Podman storage, runroot and cgroup configuration", args.first().unwrap_or(&"command"), output.status);
        ensure!(
            output.stdout.len() <= 4 * 1024 * 1024,
            "Podman returned oversized metadata"
        );
        Ok(output.stdout)
    }

    fn check_storage(&self, limits: &JobLimits) -> Result<()> {
        private_directory(&self.root)?;
        local_control_storage(&self.root)?;
        ensure!(self.workspace_roots.iter().any(|slot| self.check_slot(slot, limits).is_ok()),
            "no provisioned workspace slot has a verified local filesystem capacity within workspace_bytes");
        Ok(())
    }

    fn check_slot(&self, slot: &Path, limits: &JobLimits) -> Result<()> {
        self.check_slot_state(slot, limits, true)
    }

    fn check_slot_state(
        &self,
        slot: &Path,
        limits: &JobLimits,
        require_free_space: bool,
    ) -> Result<()> {
        ensure!(
            limits.is_finite_positive(),
            "all execution limits must be finite and positive"
        );
        private_directory(slot)?;
        ensure!(
            std::fs::metadata(slot)?.dev() != std::fs::metadata(&self.root)?.dev(),
            "workspace slot and control journals must be on different filesystems"
        );
        let mountinfo = std::fs::read_to_string("/proc/self/mountinfo")?;
        let root = slot.to_str().context("job storage root must be UTF-8")?;
        ensure!(
            !root.contains([' ', '\n', '\\', ',']),
            "job storage root contains unsupported mount-path characters"
        );
        let mut found = false;
        for line in mountinfo.lines() {
            let Some((left, right)) = line.split_once(" - ") else {
                continue;
            };
            let fields: Vec<_> = left.split_whitespace().collect();
            let fs: Vec<_> = right.split_whitespace().collect();
            if fields.len() < 6 || fs.len() < 3 {
                continue;
            }
            if fields[4] == root {
                ensure!(fields[3] == "/" && matches!(fs[0], "ext4" | "xfs") && fs[1].starts_with("/dev/"),
                    "job storage must be a dedicated local ext4/xfs block-device filesystem, not NFS, bind mount, overlay or tmpfs");
                ensure!(
                    fields[5].split(',').any(|v| v == "rw"),
                    "job storage filesystem is read-only"
                );
                found = true;
            } else if fields[4].starts_with(&format!("{root}/")) {
                bail!("nested mounts in job storage are forbidden because they escape the storage ceiling");
            }
        }
        ensure!(found, "provision job storage as a dedicated local filesystem mount; ordinary directories have no enforceable storage ceiling");
        let path = CString::new(slot.as_os_str().as_bytes())?;
        let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        ensure!(
            unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } == 0,
            "cannot inspect job filesystem capacity"
        );
        let stat = unsafe { stat.assume_init() };
        let capacity = (stat.f_blocks as u128) * (stat.f_frsize as u128);
        ensure!(capacity > 0 && capacity <= u128::from(limits.workspace_bytes),
            "dedicated job filesystem capacity exceeds approved workspace_bytes; provision a smaller bounded filesystem or approve its full capacity");
        ensure!(
            !require_free_space || (stat.f_bavail > 0 && stat.f_favail > 0),
            "job storage is exhausted; retain evidence and reclaim completed jobs before admission"
        );
        Ok(())
    }

    async fn check_runtime(&self) -> Result<()> {
        let info: Value =
            serde_json::from_slice(&self.output(&["info", "--format", "json"]).await?)?;
        ensure!(
            info.pointer("/host/security/rootless")
                .and_then(Value::as_bool)
                == Some(true),
            "Podman is not operating rootless"
        );
        ensure!(
            info.pointer("/host/cgroupVersion").and_then(Value::as_str) == Some("v2"),
            "delegated cgroup v2 is required"
        );
        let controllers = info
            .pointer("/host/cgroupControllers")
            .and_then(Value::as_array)
            .context("Podman did not report delegated cgroup controllers")?;
        for required in ["cpu", "memory", "pids"] {
            ensure!(
                controllers.iter().any(|v| v.as_str() == Some(required)),
                "required cgroup controller {required} is not delegated to the job account"
            );
        }
        Ok(())
    }

    async fn check_image(&self, image: &str) -> Result<()> {
        validate_image_pin(image)?;
        let value: Value =
            serde_json::from_slice(&self.output(&["image", "inspect", image]).await?)?;
        let image_info = value.get(0).context(
            "pinned image is not present in local Podman storage; operator must import it",
        )?;
        let digests = image_info
            .get("RepoDigests")
            .and_then(Value::as_array)
            .context("image lacks verified repository digests")?;
        ensure!(
            digests.iter().any(|v| v.as_str() == Some(image)),
            "local image does not match the operator-pinned repository digest"
        );
        Ok(())
    }

    pub async fn prepare(
        &self,
        source: &Path,
        base_sha: &str,
        run_id: Uuid,
        limits: &JobLimits,
    ) -> Result<PreparedWorkspace> {
        ensure!(
            !self.recovery_only,
            "recovery-only sandbox cannot prepare new execution"
        );
        self.check_storage(limits)?;
        ensure!(
            matches!(base_sha.len(), 40 | 64) && base_sha.bytes().all(|b| b.is_ascii_hexdigit()),
            "base must be a full committed object ID, not a branch or expression"
        );
        let source = source
            .canonicalize()
            .context("admitted source checkout is unavailable")?;
        ensure!(
            source.is_dir()
                && !source.starts_with(&self.root)
                && self
                    .workspace_roots
                    .iter()
                    .all(|slot| !source.starts_with(slot)),
            "source must be an existing checkout outside private job storage"
        );
        let run_root = self.reserve_workspace(run_id, limits)?;
        private_directory(&run_root)?;
        let root = run_root.join("workspace");
        create_private(&root).context(
            "workspace already exists: reconcile the original run rather than replacing it",
        )?;
        let checkout = root.join("checkout");
        let session = root.join("session");
        create_private(&session)?;
        let mut clone = clean_git();
        clone
            .args([
                "-c",
                "protocol.file.allow=always",
                "clone",
                "--no-local",
                "--no-hardlinks",
                "--no-checkout",
                "--template=",
                "--",
            ])
            .arg(&source)
            .arg(&checkout);
        run_git(clone)
            .await
            .context("standalone committed source clone failed")?;
        std::fs::set_permissions(&checkout, std::fs::Permissions::from_mode(0o700))?;
        ensure!(
            !checkout.join(".git/objects/info/alternates").exists()
                && checkout.join(".git").is_dir(),
            "source clone has shared Git metadata"
        );
        let mut resolve = clean_git();
        resolve.arg("-C").arg(&checkout).args([
            "rev-parse",
            "--verify",
            &format!("{base_sha}^{{commit}}"),
        ]);
        let resolved = run_git(resolve).await?;
        ensure!(
            String::from_utf8(resolved)?
                .trim()
                .eq_ignore_ascii_case(base_sha),
            "base object is not the admitted commit"
        );
        let mut checkout_command = clean_git();
        checkout_command
            .arg("-C")
            .arg(&checkout)
            .args(["checkout", "--detach", "--force", base_sha, "--"]);
        run_git(checkout_command).await?;
        // No source remote, hooks or copied credentials survive into worker Git.
        let mut remove_remote = clean_git();
        remove_remote
            .arg("-C")
            .arg(&checkout)
            .args(["remote", "remove", "origin"]);
        run_git(remove_remote).await?;
        std::fs::File::open(&root)?.sync_all()?;
        Ok(PreparedWorkspace { checkout, session })
    }

    fn contained_directory(
        &self,
        run_id: Uuid,
        path: &Path,
        limits: &JobLimits,
    ) -> Result<PathBuf> {
        let canonical = path.canonicalize()?;
        let reserved = self.workspace_root(run_id)?;
        ensure!(canonical.starts_with(&reserved) && canonical != reserved && canonical == path,
            "sandbox writable mount must be a nonsymlink child of this run's reserved workspace slot");
        self.check_slot(
            reserved.parent().context("workspace slot has no parent")?,
            limits,
        )?;
        private_directory(path)?;
        Ok(canonical)
    }

    fn create_command(
        &self,
        name: &str,
        run_id: Uuid,
        image: &str,
        limits: &JobLimits,
        checkout: &Path,
        session: &Path,
        bridge: Option<&Path>,
        env: &[(String, String)],
        argv: &[String],
        lifetime_seconds: u64,
        checkout_read_only: bool,
    ) -> Result<Command> {
        ensure!(
            !argv.is_empty() && !argv[0].is_empty(),
            "sandbox command cannot be empty"
        );
        ensure!(
            lifetime_seconds > 0 && lifetime_seconds <= limits.elapsed_seconds,
            "container lifetime must fit the remaining elapsed allowance"
        );
        let checkout = self.contained_directory(run_id, checkout, limits)?;
        let session = self.contained_directory(run_id, session, limits)?;
        let mut command = self.command();
        command.args([
            "create",
            "--name",
            name,
            "--label",
            &format!("{LABEL}={run_id}"),
            "--pull=never",
            "--interactive",
            "--network=none",
            "--ipc=private",
            "--pid=private",
            "--uts=private",
            "--cgroupns=private",
            "--cgroups=enabled",
            "--userns=keep-id",
            "--user",
            &format!("{}:{}", unsafe { libc::geteuid() }, unsafe {
                libc::getegid()
            }),
            "--cap-drop=ALL",
            "--security-opt=no-new-privileges",
            "--read-only",
            "--read-only-tmpfs=false",
            "--unsetenv-all",
            "--image-volume=ignore",
            "--no-hosts",
            "--log-driver=none",
            "--ulimit=core=0:0",
            "--ulimit=nofile=1024:1024",
            "--workdir=/workspace",
            "--memory",
            &limits.memory_bytes.to_string(),
            "--memory-swap",
            &limits.memory_bytes.to_string(),
            "--cpus",
            &format!(
                "{}.{:03}",
                limits.cpu_millis / 1000,
                limits.cpu_millis % 1000
            ),
            "--pids-limit",
            &limits.pids.to_string(),
            "--timeout",
            &lifetime_seconds.to_string(),
            "--shm-size",
            "1m",
            "--tmpfs",
            &format!(
                "/tmp:rw,nosuid,nodev,size={},mode=1777",
                (limits.memory_bytes / 8).min(256 * 1024 * 1024)
            ),
            "--tmpfs",
            "/run:rw,nosuid,nodev,noexec,size=1048576,mode=755",
            "--volume",
            &format!(
                "{}:/workspace:{},rprivate,nosuid,nodev",
                checkout.display(),
                if checkout_read_only { "ro" } else { "rw" }
            ),
            "--volume",
            &format!("{}:/session:rw,rprivate,nosuid,nodev", session.display()),
            "--env=PATH=/usr/local/bin:/usr/bin:/bin",
            "--env=HOME=/session",
            "--env=XDG_CONFIG_HOME=/session/config",
            "--env=XDG_CACHE_HOME=/session/cache",
            "--env=TMPDIR=/tmp",
            "--env=LANG=C.UTF-8",
            "--env=GIT_CONFIG_NOSYSTEM=1",
            "--env=GIT_CONFIG_GLOBAL=/dev/null",
            "--env=GIT_TERMINAL_PROMPT=0",
            "--entrypoint=/usr/local/bin/apas-job-entrypoint",
        ]);
        if let Some(bridge) = bridge {
            private_directory(bridge)?;
            ensure!(
                bridge.canonicalize()? == bridge,
                "gateway bridge directory must not contain symlinks"
            );
            let entries = std::fs::read_dir(bridge)?.collect::<std::io::Result<Vec<_>>>()?;
            ensure!((2..=3).contains(&entries.len()) && entries.iter().all(|entry| matches!(entry.file_name().to_str(), Some("gateway.sock" | "token" | "worker-profile.json"))),
                "gateway mount must contain only run-scoped socket/token and immutable worker profile");
            use std::os::unix::fs::FileTypeExt;
            ensure!(
                std::fs::symlink_metadata(bridge.join("gateway.sock"))?
                    .file_type()
                    .is_socket(),
                "gateway endpoint must be a real run-scoped Unix socket, not a symlink"
            );
            validate_credential(&bridge.join("token"))?;
            if bridge.join("worker-profile.json").exists() {
                validate_credential(&bridge.join("worker-profile.json"))?;
            }
            command
                .arg("--volume")
                .arg(format!(
                    "{}:/run/apas-bridge:ro,rprivate,nosuid,nodev,noexec",
                    bridge.display()
                ))
                .arg("--env=APAS_JOB_BRIDGE=1")
                .arg("--env=APAS_DEPENDENCY_BASE_URL=http://127.0.0.1:17840/dependencies/");
        }
        for (key, value) in env {
            ensure!(
                allowed_worker_env(key),
                "worker environment key is not explicitly permitted: {key}"
            );
            ensure!(!value.contains('\0'), "worker environment contains NUL");
            command.arg("--env").arg(format!("{key}={value}"));
        }
        command.arg(image).args(argv);
        Ok(command)
    }

    async fn launch(
        &self,
        mut command: Command,
        name: &str,
        creation_timeout: Duration,
        deadline: tokio::time::Instant,
    ) -> Result<ContainerProcess> {
        let output = tokio::time::timeout(creation_timeout, command.output()).await
            .context("sandbox creation timed out; retain deterministic container identity for reconciliation")??;
        ensure!(
            output.status.success(),
            "restricted Podman container creation failed; check runtime and resource delegation"
        );
        let id = String::from_utf8(output.stdout)?.trim().to_owned();
        ensure!(
            id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "Podman returned invalid container identity; reconcile named unit {name}"
        );
        ensure!(tokio::time::Instant::now() < deadline, "execution deadline expired during container creation; reconcile the retained created unit");
        let child = self
            .command()
            .args(["start", "--attach", "--interactive", &id])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        Ok(ContainerProcess {
            container_id: id,
            child,
        })
    }

    pub async fn spawn_worker(
        &self,
        run_id: Uuid,
        profile: &WorkerProfileConfig,
        limits: &JobLimits,
        workspace: &PreparedWorkspace,
        bridge_dir: &Path,
        env: &[(String, String)],
        argv: &[String],
        checkout_read_only: bool,
    ) -> Result<ContainerProcess> {
        ensure!(
            !self.recovery_only,
            "recovery-only sandbox cannot launch a worker"
        );
        let deadline = tokio::time::Instant::now()
            .checked_add(Duration::from_secs(limits.elapsed_seconds))
            .context("elapsed allowance exceeds supported monotonic deadline range")?;
        ensure!(
            limits.fits_within(&profile.limits),
            "requested limits exceed the approved worker profile"
        );
        self.check_storage(limits)?;
        self.check_runtime().await?;
        self.check_image(&profile.image).await?;
        let (configured, qualified) = self
            .qualified_profiles
            .get(&profile.key)
            .context("worker profile is not configured")?;
        ensure!(
            *configured == profile_digest(profile)?,
            "worker profile changed after sandbox configuration"
        );
        check_qualification(profile, qualified)?;
        ensure!(
            argv.first() == Some(&profile.omp_binary),
            "worker executable differs from the pinned profile"
        );
        let name = worker_container_name(run_id);
        let (creation_timeout, lifetime_seconds) = launch_allowance(deadline)?;
        let command = self.create_command(
            &name,
            run_id,
            &profile.image,
            limits,
            &workspace.checkout,
            &workspace.session,
            Some(bridge_dir),
            env,
            argv,
            lifetime_seconds,
            checkout_read_only,
        )?;
        self.launch(command, &name, creation_timeout, deadline)
            .await
    }

    pub async fn inventory(&self) -> Result<Vec<ContainerIdentity>> {
        let bytes = self
            .output(&[
                "ps",
                "--all",
                "--no-trunc",
                "--filter",
                &format!("label={LABEL}"),
                "--format",
                "json",
            ])
            .await?;
        let entries: Vec<Value> = serde_json::from_slice(&bytes)?;
        entries
            .into_iter()
            .map(|entry| {
                let id = entry
                    .get("Id")
                    .or_else(|| entry.get("ID"))
                    .and_then(Value::as_str)
                    .context("container inventory lacks ID")?
                    .to_owned();
                let run_id = entry
                    .get("Labels")
                    .and_then(|v| v.get(LABEL))
                    .and_then(Value::as_str)
                    .context("container inventory lacks run label")?
                    .parse()?;
                let state = entry
                    .get("State")
                    .and_then(Value::as_str)
                    .context("container inventory lacks state")?;
                Ok(ContainerIdentity {
                    id,
                    run_id,
                    running: matches!(state, "running" | "paused" | "stopping" | "restarting"),
                })
            })
            .collect()
    }

    pub async fn stop(&self, id: &str) -> Result<()> {
        ensure!(
            !id.is_empty()
                && !id.starts_with('-')
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "invalid container identity"
        );
        let inspect = self.output(&["inspect", "--type=container", id]).await?;
        let before: Value = serde_json::from_slice(&inspect)?;
        let state = before
            .get(0)
            .and_then(|v| v.get("State"))
            .context("container stop inventory is missing")?;
        if state.get("Running").and_then(Value::as_bool) != Some(false)
            || state.get("Pid").and_then(Value::as_u64) != Some(0)
        {
            // Podman stop escalates SIGTERM to SIGKILL for the complete cgroup.
            self.output(&["stop", "--time", "3", id]).await?;
        }
        let after: Value =
            serde_json::from_slice(&self.output(&["inspect", "--type=container", id]).await?)?;
        let state = after
            .get(0)
            .and_then(|v| v.get("State"))
            .context("container stop evidence is missing")?;
        ensure!(state.get("Running").and_then(Value::as_bool) == Some(false)
            && state.get("Pid").and_then(Value::as_u64) == Some(0)
            && state.get("Paused").and_then(Value::as_bool) != Some(true),
            "complete container termination could not be confirmed; retain recovery-required ownership");
        Ok(())
    }

    pub async fn run_verifier(
        &self,
        run_id: Uuid,
        profile: &VerificationProfileConfig,
        limits: &JobLimits,
        checkout: &Path,
        check: &VerificationCheckConfig,
        bridge_dir: Option<&Path>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<VerifierOutcome> {
        ensure!(
            !self.recovery_only,
            "recovery-only sandbox cannot launch verification"
        );
        let deadline = tokio::time::Instant::now()
            .checked_add(Duration::from_secs(
                check.timeout_seconds.min(limits.elapsed_seconds),
            ))
            .context("verification allowance exceeds supported monotonic deadline range")?;
        self.check_storage(limits)?;
        self.check_runtime().await?;
        self.check_image(&profile.image).await?;
        ensure!(
            profile
                .checks
                .iter()
                .any(|approved| approved.name == check.name
                    && approved.argv == check.argv
                    && approved.timeout_seconds == check.timeout_seconds),
            "verification command is not in the approved profile"
        );
        ensure!(
            check.timeout_seconds > 0 && !check.argv.is_empty(),
            "verification command and timeout are required"
        );
        ensure!(
            !cancelled.load(Ordering::SeqCst),
            "verification cancelled before launch"
        );
        let session = self
            .workspace_root(run_id)?
            .join(format!("verifier-session-{}", Uuid::new_v4()));
        create_private(&session)?;
        let name = format!("apas-verify-{run_id}-{}", Uuid::new_v4());
        let (creation_timeout, lifetime_seconds) = launch_allowance(deadline)?;
        let command = self.create_command(
            &name,
            run_id,
            &profile.image,
            limits,
            checkout,
            &session,
            bridge_dir,
            &[],
            &check.argv,
            lifetime_seconds,
            false,
        )?;
        let mut process = self
            .launch(command, &name, creation_timeout, deadline)
            .await?;
        drop(process.child.stdin.take());
        let max = limits.output_bytes.min(MAX_LOG_BYTES) as usize;
        let stdout = process
            .child
            .stdout
            .take()
            .context("verifier stdout pipe missing")?;
        let stderr = process
            .child
            .stderr
            .take()
            .context("verifier stderr pipe missing")?;
        let out_task = tokio::spawn(read_bounded(stdout, max / 2));
        let err_task = tokio::spawn(read_bounded(stderr, max - max / 2));
        let timeout = tokio::time::sleep_until(deadline);
        tokio::pin!(timeout);
        let mut timed_out = false;
        let mut was_cancelled = false;
        let status = loop {
            tokio::select! {
                result = process.child.wait() => break Some(result?),
                _ = &mut timeout => { timed_out = true; break None; }
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if cancelled.load(Ordering::SeqCst) { was_cancelled = true; break None; }
                }
            }
        };
        self.stop(&process.container_id).await?;
        if status.is_none() {
            let _ = tokio::time::timeout(COMMAND_TIMEOUT, process.child.wait()).await?;
        }
        let (stdout, out_truncated) = out_task.await??;
        let (stderr, err_truncated) = err_task.await??;
        Ok(VerifierOutcome {
            container_id: process.container_id,
            exit_code: status.and_then(|s| s.code()),
            timed_out,
            cancelled: was_cancelled,
            stdout,
            stderr,
            truncated: out_truncated || err_truncated,
        })
    }

    pub async fn probe(&self, profile: &WorkerProfileConfig) -> Result<()> {
        ensure!(
            !self.recovery_only,
            "recovery-only sandbox cannot launch readiness probes"
        );
        self.check_storage(&profile.limits)?;
        self.check_runtime().await?;
        self.check_image(&profile.image).await?;
        ensure!(
            profile.omp_binary.starts_with('/') && !profile.omp_version.is_empty(),
            "profile must pin the in-image OMP executable and version"
        );
        let run_id = Uuid::new_v4();
        let root = self.reserve_workspace(run_id, &profile.limits)?;
        let result: Result<()> = async {
        let checkout = root.join("probe");
        let session = root.join("session");
        create_private(&checkout)?;
        create_private(&session)?;
        let name = format!("apas-probe-{run_id}");
        let argv = vec!["/usr/bin/python3".into(), "-c".into(), PROBE_SCRIPT.into(), profile.limits.memory_bytes.to_string(), profile.limits.pids.to_string(), profile.limits.cpu_millis.to_string(), profile.omp_binary.clone(), profile.omp_version.clone(), profile.limits.workspace_bytes.to_string(), bootstrap_digest()];
        let command = self.create_command(&name, run_id, &profile.image, &profile.limits, &checkout, &session, None, &[], &argv, COMMAND_TIMEOUT.as_secs().min(profile.limits.elapsed_seconds), false)?;
        let mut process = self.launch(command, &name, COMMAND_TIMEOUT, tokio::time::Instant::now() + COMMAND_TIMEOUT * 2).await?;
        drop(process.child.stdin.take());
        let stdout = process.child.stdout.take().context("probe stdout missing")?;
        let stderr = process.child.stderr.take().context("probe stderr missing")?;
        let out_task = tokio::spawn(read_bounded(stdout, 4096));
        let err_task = tokio::spawn(read_bounded(stderr, 4096));
        let status = tokio::time::timeout(COMMAND_TIMEOUT, process.child.wait()).await;
        self.stop(&process.container_id).await?;
        let status = status.context("sandbox behavioral probe timed out")??;
        let (output, truncated) = out_task.await??;
        let _ = err_task.await??;
        ensure!(status.success() && !truncated && output == b"APAS_SANDBOX_OK\n", "sandbox behavioral probe failed: qualify non-root, readonly image, cgroup bounds, private namespaces, network denial and pinned OMP version");
        // Prove the inspection mount differs from implementation while bounded
        // session output remains writable. No model or provider is invoked.
        let inspection_name = format!("apas-inspect-probe-{run_id}");
        let argv = vec!["/usr/bin/python3".into(), "-c".into(), READONLY_PROBE_SCRIPT.into()];
        let command = self.create_command(&inspection_name, run_id, &profile.image, &profile.limits, &checkout, &session, None, &[], &argv, COMMAND_TIMEOUT.as_secs().min(profile.limits.elapsed_seconds), true)?;
        let mut inspection = self.launch(command, &inspection_name, COMMAND_TIMEOUT, tokio::time::Instant::now() + COMMAND_TIMEOUT * 2).await?;
        drop(inspection.child.stdin.take());
        let stdout = tokio::spawn(read_bounded(inspection.child.stdout.take().context("inspection probe stdout missing")?, 4096));
        let stderr = tokio::spawn(read_bounded(inspection.child.stderr.take().context("inspection probe stderr missing")?, 4096));
        let status = tokio::time::timeout(COMMAND_TIMEOUT, inspection.child.wait()).await;
        self.stop(&inspection.container_id).await?;
        let (output, truncated) = stdout.await??;
        let _ = stderr.await??;
        ensure!(status??.success() && !truncated && output == b"APAS_INSPECTION_OK\n", "read-only inspection and session output probe failed");
        // Separate live descendant test: an exited probe alone is not stop evidence.
        let stop_name = format!("apas-stop-probe-{run_id}");
        let argv = vec!["/usr/bin/python3".into(), "-c".into(), "import os,time; os.fork(); time.sleep(120)".into()];
        let command = self.create_command(&stop_name, run_id, &profile.image, &profile.limits, &checkout, &session, None, &[], &argv, COMMAND_TIMEOUT.as_secs().min(profile.limits.elapsed_seconds), false)?;
        let mut descendants = self.launch(command, &stop_name, COMMAND_TIMEOUT, tokio::time::Instant::now() + COMMAND_TIMEOUT * 2).await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let state: Value = serde_json::from_slice(&self.output(&["inspect", "--type=container", &descendants.container_id]).await?)?;
        ensure!(state.pointer("/0/State/Running").and_then(Value::as_bool) == Some(true), "sandbox stop probe never became running");
        self.stop(&descendants.container_id).await?;
        tokio::time::timeout(COMMAND_TIMEOUT, descendants.child.wait()).await??;
        Ok(())
        }.await;
        // A failed readiness probe may release its slot only after every container
        // in this unique probe run is positively stopped. Uncertain units retain it.
        for container in self
            .inventory()
            .await?
            .into_iter()
            .filter(|entry| entry.run_id == run_id)
        {
            self.stop(&container.id).await?;
            self.output(&["rm", &container.id]).await?;
        }
        // Only these freshly created, never-worker-writable probe directories.
        let slot = root.parent().context("probe workspace has no slot")?;
        let reservation = self.reservation_path(slot);
        std::fs::remove_dir_all(&root)?;
        std::fs::remove_file(reservation)?;
        std::fs::File::open(&self.root)?.sync_all()?;
        result
    }
}

fn local_control_storage(path: &Path) -> Result<()> {
    let path = CString::new(path.as_os_str().as_bytes())?;
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    ensure!(
        unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) } == 0,
        "cannot inspect control storage filesystem"
    );
    let kind = unsafe { stat.assume_init() }.f_type as u64;
    ensure!(
        matches!(kind, 0xef53 | 0x58465342 | 0x9123683e | 0x2fc12fc1),
        "durable control storage requires local ext4, XFS, Btrfs or ZFS; no NFS/tmpfs fallback"
    );
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QualificationEvidence {
    profile_digest: String,
    qualified_at: chrono::DateTime<Utc>,
    omp_rpc: bool,
    child_questions: bool,
    gateway_metering: bool,
    execution_protocol_version: u32,
    bootstrap_digest: String,
    readonly_inspection: bool,
    inspection_results: bool,
    implementation_results: bool,
}

fn check_qualification(profile: &WorkerProfileConfig, expected_digest: &str) -> Result<()> {
    let path = profile.qualification_file.as_ref().context("record real pinned-bootstrap RPC, child-question, metering, read-only inspection and inspection/implementation result qualification before enabling this profile")?;
    validate_credential(path)
        .context("qualification evidence must be a private operator-owned regular file")?;
    let evidence: QualificationEvidence = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(evidence.profile_digest == expected_digest && evidence.qualified_at <= Utc::now()
        && evidence.omp_rpc && evidence.child_questions && evidence.gateway_metering
        && evidence.execution_protocol_version == shared::WORKER_EXECUTION_PROTOCOL_VERSION
        && evidence.bootstrap_digest == bootstrap_digest() && evidence.readonly_inspection
        && evidence.inspection_results && evidence.implementation_results,
        "qualification evidence does not prove the selected immutable bootstrap's RPC, metering, read-only inspection and result behavior");
    Ok(())
}

fn allowed_worker_env(key: &str) -> bool {
    matches!(
        key,
        "OPENAI_API_KEY"
            | "OPENAI_BASE_URL"
            | "OPENAI_API_BASE"
            | "ANTHROPIC_API_KEY"
            | "ANTHROPIC_BASE_URL"
            | "ANTHROPIC_AUTH_TOKEN"
            | "OMP_CONFIG_DIR"
            | "PI_CODING_AGENT_DIR"
            | "OMP_SESSION_DIR"
            | "APAS_JOB_ID"
            | "APAS_JOB_RUN_ID"
    )
}

fn clean_git() -> Command {
    let mut command = Command::new("/usr/bin/git");
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/nonexistent")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "protocol.allow=never",
        ])
        .stdin(Stdio::null())
        .kill_on_drop(true);
    command
}

async fn run_git(mut command: Command) -> Result<Vec<u8>> {
    let output = tokio::time::timeout(Duration::from_secs(120), command.output())
        .await
        .context("private source preparation timed out")??;
    ensure!(
        output.status.success(),
        "trusted Git source preparation failed ({})",
        output.status
    );
    Ok(output.stdout)
}

async fn read_bounded<R: AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> Result<(Vec<u8>, bool)> {
    let mut retained = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let keep = read.min(limit.saturating_sub(retained.len()));
        retained.extend_from_slice(&buffer[..keep]);
        truncated |= keep < read;
    }
    Ok((retained, truncated))
}

pub async fn readiness(
    config: &AutonomousJobConfig,
    machine_id: Uuid,
    cluster_owner_user_id: Uuid,
) -> JobWorkerReadiness {
    let checked = Utc::now();
    let sandbox = Sandbox::new(config);
    let mut profiles = Vec::with_capacity(config.profiles.len());
    for profile in &config.profiles {
        let mut reasons = Vec::new();
        let digest = match worker_profile_digest(config, profile) {
            Ok(digest) => {
                if let Err(error) = check_qualification(profile, &digest) {
                    reasons.push(error.to_string());
                }
                digest
            }
            Err(error) => {
                reasons.push(error.to_string());
                String::new()
            }
        };
        let mut models = Vec::new();
        let mut verifiers = Vec::new();
        for key in &profile.models {
            match config.models.iter().find(|model| &model.key == key) {
                Some(model) => {
                    if let Err(error) =
                        super::gateway::validate_profile(model, &profile.dependency_hosts)
                    {
                        reasons.push(format!("Model profile {key}: {error}"));
                    }
                    if validate_credential(&model.credential_file).is_err() {
                        reasons.push(format!(
                            "Model profile {key}: provision a dedicated private credential file"
                        ));
                    }
                    if model.input_micro_usd_per_million == 0
                        || model.output_micro_usd_per_million == 0
                    {
                        reasons.push(format!(
                            "Model profile {key}: qualified nonzero metering rates are required"
                        ));
                    }
                    models.push(JobModelProfile {
                        key: model.key.clone(),
                        label: model.label.clone(),
                        model: model.model.clone(),
                        api: model.api,
                    });
                }
                None => reasons.push(format!("Model profile {key} is not configured")),
            }
        }
        if models.is_empty() {
            reasons.push("At least one dedicated model profile is required".into());
        }
        for key in &profile.verifiers {
            match config
                .verifiers
                .iter()
                .find(|verifier| &verifier.key == key)
            {
                Some(verifier) => {
                    if verifier.checks.is_empty()
                        || verifier
                            .checks
                            .iter()
                            .any(|c| c.argv.is_empty() || c.timeout_seconds == 0)
                    {
                        reasons.push(format!(
                            "Verification profile {key} has no valid mandatory checks"
                        ));
                    }
                    if let Ok(sandbox) = &sandbox {
                        let mut probe_profile = profile.clone();
                        probe_profile.image = verifier.image.clone();
                        probe_profile.omp_binary = "/usr/bin/python3".into();
                        probe_profile.omp_version = "Python 3".into();
                        if let Err(error) = sandbox.probe(&probe_profile).await {
                            reasons.push(format!("Verification profile {key}: {error}"));
                        }
                    }
                    verifiers.push(JobVerificationProfile {
                        key: verifier.key.clone(),
                        label: verifier.label.clone(),
                        digest: profile_digest(verifier).unwrap_or_default(),
                        image: verifier.image.clone(),
                    });
                }
                None => reasons.push(format!("Verification profile {key} is not configured")),
            }
        }
        if verifiers.is_empty() {
            reasons.push("At least one independent verification profile is required".into());
        }
        match config.publishers.iter().find(|publisher| publisher.key == profile.publisher) {
            Some(publisher) if publisher.ci_qualified && !publisher.repositories.is_empty() && !publisher.protected_base_refs.is_empty() && validate_credential(&publisher.credential_file).is_ok() => {}
            _ => reasons.push("Provision a repository-scoped publisher credential with protected bases and qualified untrusted-PR CI policy".into()),
        }
        match &sandbox {
            Ok(sandbox) => {
                if let Err(error) = sandbox.probe(profile).await {
                    reasons.push(error.to_string());
                }
            }
            Err(error) => reasons.push(error.to_string()),
        }
        profiles.push(JobWorkerProfile {
            key: profile.key.clone(),
            label: profile.label.clone(),
            digest,
            image: profile.image.clone(),
            omp_version: profile.omp_version.clone(),
            model_profiles: models,
            verification_profiles: verifiers,
            limits: profile.limits.clone(),
            ready: reasons.is_empty(),
            unavailable_reasons: reasons,
        });
    }
    JobWorkerReadiness {
        machine_id,
        cluster_owner_user_id,
        protocol_version: shared::WORKER_EXECUTION_PROTOCOL_VERSION,
        checked_at: checked.to_rfc3339(),
        expires_at: (checked + ChronoDuration::seconds(shared::JOB_READINESS_SECONDS as i64))
            .to_rfc3339(),
        profiles,
    }
}

const PROBE_SCRIPT: &str = r#"import errno, json, os, pathlib, socket, subprocess, sys
memory,pids,cpu,binary,version,workspace,bootstrap=sys.argv[1:]
assert os.getuid()!=0
status=pathlib.Path('/proc/self/status').read_text()
assert 'NoNewPrivs:\t1' in status and 'CapEff:\t0000000000000000' in status
assert 'Seccomp:\t2' in status
assert int(pathlib.Path('/sys/fs/cgroup/memory.max').read_text())<=int(memory)
assert int(pathlib.Path('/sys/fs/cgroup/pids.max').read_text())<=int(pids)
quota,period=pathlib.Path('/sys/fs/cgroup/cpu.max').read_text().split()
assert quota!='max' and int(quota)*1000<=int(cpu)*int(period)
assert pathlib.Path('/sys/fs/cgroup/memory.swap.max').read_text().strip()=='0'
assert set(os.listdir('/sys/class/net'))=={'lo'}
s=socket.socket();s.settimeout(1)
try:s.connect(('1.1.1.1',443));raise AssertionError('external route')
except OSError:pass
finally:s.close()
for path in ['/etc/apas-probe-write','/usr/apas-probe-write']:
 try:open(path,'w');raise AssertionError('writable image')
 except OSError:pass
for path in ['/root/.ssh','/run/podman/podman.sock','/var/run/docker.sock','/var/lib/apas']:
 assert not os.path.exists(path)
for path in ['/workspace/probe','/session/probe','/tmp/probe']:
 with open(path,'w') as f:f.write('bounded writable location')
 os.unlink(path)
try:
 with open('/workspace/quota-probe','w') as f:
  try:os.posix_fallocate(f.fileno(),0,int(workspace)+1);raise AssertionError('storage ceiling unenforced')
  except OSError as e:assert e.errno in (errno.ENOSPC,errno.EDQUOT)
finally:
 os.unlink('/workspace/quota-probe')
assert version in subprocess.check_output([binary,'--version'],stderr=subprocess.STDOUT,timeout=15).decode()
if binary=='/usr/local/bin/apas-omp-worker':
 contract=json.loads(subprocess.check_output([binary,'--worker-contract'],stderr=subprocess.STDOUT,timeout=15))
 assert contract=={'version':2,'resultVersion':1,'resultPath':'/session/apas-worker-result.json','bootstrapDigest':bootstrap}
print('APAS_SANDBOX_OK')
"#;

const READONLY_PROBE_SCRIPT: &str = r#"import errno, os, stat
try:
 open('/workspace/apas-inspection-probe','w')
 raise AssertionError('inspection checkout is writable')
except OSError as error:
 assert error.errno==errno.EROFS
path='/session/apas-worker-result.json'
with open(path,'x') as output:output.write('{}')
assert stat.S_ISREG(os.lstat(path).st_mode)
os.unlink(path)
print('APAS_INSPECTION_OK')
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn non_linux_hosts_refuse_execution_and_recovery() {
        let mut config = AutonomousJobConfig::default();
        config.enabled = true;
        assert!(Sandbox::new(&config)
            .unwrap_err()
            .to_string()
            .contains("autonomous jobs require Linux rootless Podman"));
        config.enabled = false;
        assert!(Sandbox::for_recovery(&config)
            .unwrap_err()
            .to_string()
            .contains("autonomous jobs require Linux rootless Podman"));
    }

    #[test]
    fn qualification_requires_real_result_contract_for_exact_bootstrap() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("qualification.json");
        let profile: WorkerProfileConfig = serde_json::from_value(serde_json::json!({
            "key":"worker","label":"Worker","image":"pinned-test-image","omp_version":"18.18.0",
            "omp_binary":"/usr/local/bin/apas-omp-worker","models":[],"verifiers":[],
            "publisher":"publisher","limits":JobLimits::default(),"qualification_file":path
        }))
        .unwrap();
        let original = serde_json::json!({
            "profile_digest":"qualified-profile","qualified_at":Utc::now().to_rfc3339(),
            "omp_rpc":true,"child_questions":true,"gateway_metering":true,
            "execution_protocol_version":shared::WORKER_EXECUTION_PROTOCOL_VERSION,
            "bootstrap_digest":bootstrap_digest(),"readonly_inspection":true,
            "inspection_results":true,"implementation_results":true
        });
        let save = |value: &Value| {
            std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        };
        save(&original);
        assert!(check_qualification(&profile, "qualified-profile").is_ok());
        assert!(check_qualification(&profile, "different-profile").is_err());
        for field in [
            "readonly_inspection",
            "inspection_results",
            "implementation_results",
        ] {
            let mut value = original.clone();
            value[field] = Value::Bool(false);
            save(&value);
            assert!(
                check_qualification(&profile, "qualified-profile").is_err(),
                "{field}"
            );
            value.as_object_mut().unwrap().remove(field);
            save(&value);
            assert!(
                check_qualification(&profile, "qualified-profile").is_err(),
                "{field}"
            );
        }
        let mut value = original.clone();
        value["execution_protocol_version"] = serde_json::json!(1);
        save(&value);
        assert!(check_qualification(&profile, "qualified-profile").is_err());
        value = original;
        value["bootstrap_digest"] = serde_json::json!("different-bootstrap");
        save(&value);
        assert!(check_qualification(&profile, "qualified-profile").is_err());
    }

    #[test]
    fn mutable_or_malformed_image_cannot_enter_sandbox() {
        for image in [
            "worker:latest",
            "worker@sha256:short",
            "--privileged@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "worker@sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ] {
            assert!(validate_image_pin(image).is_err());
        }
        assert!(validate_image_pin("registry.example/worker@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_ok());
    }

    #[test]
    fn ambient_loader_proxy_and_ssh_authority_are_rejected() {
        for key in [
            "LD_PRELOAD",
            "PATH",
            "HOME",
            "SSH_AUTH_SOCK",
            "HTTPS_PROXY",
            "GIT_CONFIG_COUNT",
            "NODE_OPTIONS",
            "BUN_OPTIONS",
            "APAS_JOB_BRIDGE",
        ] {
            assert!(!allowed_worker_env(key), "{key}");
        }
    }

    #[tokio::test]
    async fn verifier_output_is_drained_but_retention_is_bounded() {
        let bytes = vec![b'x'; 40000];
        let (retained, truncated) = read_bounded(bytes.as_slice(), 31).await.unwrap();
        assert_eq!(retained.len(), 31);
        assert!(truncated);
    }

    #[test]
    fn credential_symlink_and_shared_permissions_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let credential = root.path().join("credential");
        std::fs::write(&credential, "dedicated-test-token").unwrap();
        std::fs::set_permissions(&credential, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(validate_credential(&credential).is_err());
        std::fs::set_permissions(&credential, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(validate_credential(&credential).is_ok());
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&credential, &link).unwrap();
        assert!(validate_credential(&link).is_err());
    }

    #[test]
    fn ordinary_directory_cannot_claim_a_workspace_quota() {
        let control = tempfile::tempdir().unwrap();
        let slot = tempfile::tempdir().unwrap();
        let sandbox = Sandbox {
            root: control.path().to_owned(),
            workspace_roots: vec![slot.path().to_owned()],
            podman: "/usr/bin/podman".into(),
            home: control.path().to_owned(),
            runtime_dir: control.path().to_owned(),
            isolated_podman: None,
            qualified_profiles: std::collections::BTreeMap::new(),
            recovery_only: false,
        };
        assert!(sandbox
            .check_slot(slot.path(), &JobLimits::default())
            .is_err());
    }

    #[test]
    fn retained_cleanup_restores_directories_without_following_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let nested = root.path().join("locked");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("source"), "retained source").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
        std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o000)).unwrap();
        let outside_mode = std::fs::metadata(outside.path()).unwrap().mode();
        Sandbox::make_retained_directories_removable(
            root.path(),
            std::fs::metadata(root.path()).unwrap().dev(),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(nested.join("source")).unwrap(),
            "retained source"
        );
        assert_eq!(
            std::fs::metadata(outside.path()).unwrap().mode(),
            outside_mode
        );
    }

    #[test]
    fn admission_digest_binds_resolved_model_and_publication_authority() {
        let mut config: AutonomousJobConfig = serde_json::from_value(serde_json::json!({
            "profiles": [{
                "key":"worker", "label":"Worker", "image":"test-image", "omp_version":"18.18.0",
                "omp_binary":"/usr/local/bin/apas-omp-worker", "models":["model"], "verifiers":["verify"],
                "publisher":"publisher", "limits":JobLimits::default()
            }],
            "models": [{
                "key":"model", "label":"Model", "model":"qualified-model", "api":"openai_responses",
                "context_window":32768, "reasoning":false,
                "upstream_base_url":"https://provider.example/v1", "credential_file":"/private/model",
                "input_micro_usd_per_million":100, "output_micro_usd_per_million":200
            }],
            "verifiers": [{"key":"verify","label":"Verify","image":"test-verifier","checks":[]}],
            "publishers": [{
                "key":"publisher", "credential_file":"/private/publisher", "repositories":["owner/repo"],
                "protected_base_refs":["main"], "ci_qualified":true
            }]
        })).unwrap();
        let original = worker_profile_digest(&config, &config.profiles[0]).unwrap();
        config.models[0].context_window += 1;
        let model_changed = worker_profile_digest(&config, &config.profiles[0]).unwrap();
        assert_ne!(original, model_changed);
        config.publishers[0]
            .repositories
            .push("owner/another-repo".into());
        let authority_changed = worker_profile_digest(&config, &config.profiles[0]).unwrap();
        assert_ne!(model_changed, authority_changed);
        config.models.push(config.models[0].clone());
        assert!(worker_profile_digest(&config, &config.profiles[0]).is_err());
    }

    #[tokio::test]
    async fn recovery_handle_cannot_prepare_new_execution() {
        let root = tempfile::tempdir().unwrap();
        let sandbox = Sandbox {
            root: root.path().to_owned(),
            workspace_roots: Vec::new(),
            podman: "/nonexistent/podman".into(),
            home: root.path().to_owned(),
            runtime_dir: root.path().to_owned(),
            isolated_podman: None,
            qualified_profiles: std::collections::BTreeMap::new(),
            recovery_only: true,
        };
        let error = sandbox
            .prepare(
                root.path(),
                &"a".repeat(40),
                Uuid::new_v4(),
                &JobLimits::default(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("recovery-only sandbox"));
    }

    #[test]
    fn preflight_and_container_creation_do_not_refresh_elapsed_allowance() {
        let now = tokio::time::Instant::now();
        assert!(launch_allowance(now).is_err());
        assert!(launch_allowance(now + Duration::from_secs(1)).is_err());
        let (creation, lifetime) = launch_allowance(now + Duration::from_secs(100)).unwrap();
        assert!(creation <= COMMAND_TIMEOUT);
        assert!(lifetime > 0 && creation.as_secs() + lifetime <= 100);
        let (_, shorter) = launch_allowance(now + Duration::from_secs(10)).unwrap();
        assert!(shorter < lifetime);
    }
}
