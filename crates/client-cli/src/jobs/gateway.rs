//! Run-scoped model and dependency gateway. Only `bridge_dir()` crosses the sandbox boundary.
//! The durable meter records provider-token-priced estimates, not an exact invoice cap.
use super::{config, ExecutionLock};
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Utc};
use config::ModelProfileConfig;
use meter::{Journal, Pricing};
use shared::{JobLimits, JobUsage};
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    net::UnixListener,
    sync::{watch, Mutex, Semaphore},
    task::JoinHandle,
    time::Instant,
};
use uuid::Uuid;

mod meter;
mod policy;
#[cfg(test)]
mod tests;
mod transport;
pub use transport::run_bridge;

/// Non-billable profile validation for readiness. Actual connections additionally resolve and pin public IPs.
pub fn validate_profile(model: &ModelProfileConfig, dependency_hosts: &[String]) -> Result<()> {
    policy::model_url(model)?;
    policy::dependency_hosts(dependency_hosts)?;
    read_credential(model)?;
    Ok(())
}

fn read_credential(profile: &ModelProfileConfig) -> Result<String> {
    let bytes = meter::read_private(&profile.credential_file, 16 * 1024)?;
    let value = String::from_utf8(bytes).context("model credential is not UTF-8")?;
    let value = value.trim_end_matches(['\r', '\n']);
    ensure!(
        !value.is_empty() && value.len() <= 8192 && value.bytes().all(|v| v.is_ascii_graphic()),
        "invalid model credential file"
    );
    Ok(value.to_owned())
}

pub struct GatewayConfig {
    pub run_id: Uuid,
    /// Host-only directory. Its `bridge` child is the only directory mounted in the worker.
    pub directory: PathBuf,
    pub model: ModelProfileConfig,
    pub dependency_hosts: Vec<String>,
    pub limits: JobLimits,
    /// Cumulative previous-attempt usage; known zero must be Some(0), never None.
    pub prior_usage: JobUsage,
    pub deadline: DateTime<Utc>,
    pub lease_expires_at: DateTime<Utc>,
}

pub struct DependencyGatewayConfig {
    pub run_id: Uuid,
    pub directory: PathBuf,
    pub dependency_hosts: Vec<String>,
    pub limits: JobLimits,
    pub deadline: DateTime<Utc>,
    pub lease_expires_at: DateTime<Utc>,
}

pub struct Gateway {
    inner: Arc<Inner>,
    bridge_directory: PathBuf,
    accept: Option<JoinHandle<()>>,
    watchdog: Option<JoinHandle<()>>,
}

struct State {
    journal: Journal,
    deadline: Instant,
    lease: Instant,
    elapsed_origin: Instant,
    elapsed_base: u64,
}

struct Inner {
    state: Mutex<State>,
    journal_path: PathBuf,
    model: Option<ModelProfileConfig>,
    credential: Option<String>,
    token: String,
    hosts: std::collections::BTreeSet<String>,
    cancelled: AtomicBool,
    stop: watch::Sender<bool>,
    dependencies: Arc<Semaphore>,
    connections: Arc<Semaphore>,
    _lock: ExecutionLock,
}

impl State {
    fn gate(&self, model: bool) -> Result<()> {
        ensure!(
            Instant::now() < self.deadline && Instant::now() < self.lease,
            "execution authority expired"
        );
        self.journal.gate(model)
    }

    fn tick_elapsed(&mut self) {
        self.journal.usage.elapsed_seconds = self
            .elapsed_base
            .saturating_add(self.elapsed_origin.elapsed().as_secs());
    }
}

impl Inner {
    async fn persist(&self, state: &mut State) -> Result<()> {
        state.tick_elapsed();
        if let Err(error) = state.journal.persist(&self.journal_path) {
            self.cancelled.store(true, Ordering::Release);
            self.stop.send_replace(true);
            return Err(error);
        }
        Ok(())
    }

    async fn unknown(&self, reason: &str) {
        let mut state = self.state.lock().await;
        state.journal.mark_unknown(reason);
        let _ = self.persist(&mut state).await;
    }

    async fn authority(&self) -> Result<()> {
        ensure!(!self.cancelled.load(Ordering::Acquire), "gateway stopped");
        let state = self.state.lock().await;
        ensure!(
            Instant::now() < state.deadline
                && Instant::now() < state.lease
                && Utc::now() < state.journal.deadline
                && Utc::now() < state.journal.lease_expires_at,
            "execution authority expired"
        );
        Ok(())
    }

    async fn output(&self, count: usize) -> Result<()> {
        let mut state = self.state.lock().await;
        state.journal.response_bytes = state
            .journal
            .response_bytes
            .checked_add(count as u64)
            .context("gateway output counter overflow")?;
        // Persist consumption even when the last in-flight chunk overruns the allowance.
        self.persist(&mut state).await?;
        ensure!(
            state.journal.response_bytes <= state.journal.limits.output_bytes,
            "gateway output allowance exhausted"
        );
        Ok(())
    }
}

impl Gateway {
    pub async fn start(config: GatewayConfig) -> Result<Self> {
        policy::model_url(&config.model)?;
        Self::start_inner(
            config.run_id,
            config.directory,
            Some(config.model),
            config.dependency_hosts,
            config.limits,
            config.prior_usage,
            config.deadline,
            config.lease_expires_at,
        )
        .await
    }

    pub async fn start_dependencies(config: DependencyGatewayConfig) -> Result<Self> {
        Self::start_inner(
            config.run_id,
            config.directory,
            None,
            config.dependency_hosts,
            config.limits,
            JobUsage {
                spend_micro_usd: Some(0),
                ..JobUsage::default()
            },
            config.deadline,
            config.lease_expires_at,
        )
        .await
    }

    async fn start_inner(
        run_id: Uuid,
        directory: PathBuf,
        model: Option<ModelProfileConfig>,
        hosts: Vec<String>,
        limits: JobLimits,
        prior_usage: JobUsage,
        deadline: DateTime<Utc>,
        lease_expires_at: DateTime<Utc>,
    ) -> Result<Self> {
        ensure!(
            limits.is_finite_positive() && limits.model_concurrency <= 16,
            "invalid gateway limits"
        );
        let now = Utc::now();
        ensure!(
            deadline > now
                && lease_expires_at > now
                && lease_expires_at <= now + chrono::Duration::seconds(120),
            "invalid gateway deadline or lease"
        );
        let remaining = limits
            .elapsed_seconds
            .saturating_sub(prior_usage.elapsed_seconds);
        ensure!(remaining > 0, "execution allowance exhausted");
        let deadline = deadline.min(deadline_after(remaining)?);
        let hosts = policy::dependency_hosts(&hosts)?;
        meter::private_directory(&directory)?;
        let lock_path = directory.join("gateway.lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&lock_path)?;
        let lock =
            ExecutionLock::acquire(lock).context("gateway already has an execution owner")?;
        let bridge_directory = directory.join("bridge");
        meter::private_directory(&bridge_directory)?;
        let socket = bridge_directory.join("gateway.sock");
        let token = bridge_directory.join("token");
        let journal_path = directory.join("usage.json");
        let mut journal = Journal::load_or_create(
            &journal_path,
            run_id,
            model.as_ref().map(Pricing::from),
            limits,
            prior_usage,
            deadline,
            lease_expires_at,
        )?;
        journal.gate(false)?;
        ensure!(
            journal
                .dependency_hosts
                .as_ref()
                .is_none_or(|previous| hosts.is_subset(previous)),
            "dependency scope cannot widen on reconnect"
        );
        journal.dependency_hosts = Some(hosts.clone());
        journal.persist(&journal_path)?;
        let credential = model.as_ref().map(read_credential).transpose()?;
        // Keep the scoped token stable across clean controller adoption, without ever storing an upstream key here.
        let worker_token = if token.try_exists()? {
            let value = String::from_utf8(meter::read_private(&token, 128)?)?;
            ensure!(
                value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit()),
                "invalid scoped token"
            );
            value
        } else {
            let value = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
            meter::atomic_private(&token, value.as_bytes())?;
            value
        };
        if let Ok(metadata) = fs::symlink_metadata(&socket) {
            ensure!(
                metadata.file_type().is_socket(),
                "gateway socket path is not a socket"
            );
            fs::remove_file(&socket)?;
        }
        let listener = UnixListener::bind(&socket)?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
        let (stop, _) = watch::channel(false);
        let elapsed_base = journal.usage.elapsed_seconds;
        let state = State {
            deadline: monotonic(journal.deadline)?,
            lease: monotonic(journal.lease_expires_at)?,
            elapsed_origin: Instant::now(),
            elapsed_base,
            journal,
        };
        let inner = Arc::new(Inner {
            state: Mutex::new(state),
            journal_path,
            model,
            credential,
            token: worker_token,
            hosts,
            cancelled: AtomicBool::new(false),
            stop,
            dependencies: Arc::new(Semaphore::new(4)),
            connections: Arc::new(Semaphore::new(32)),
            _lock: lock,
        });
        let accept_inner = inner.clone();
        let accept = tokio::spawn(async move {
            transport::serve(listener, accept_inner).await;
        });
        let watchdog_inner = inner.clone();
        let watchdog = tokio::spawn(async move {
            let mut stop = watchdog_inner.stop.subscribe();
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            loop {
                tokio::select! {
                    _ = stop.changed() => break,
                    _ = interval.tick() => {
                        if watchdog_inner.authority().await.is_err() {
                            watchdog_inner.cancelled.store(true, Ordering::Release);
                            let mut state = watchdog_inner.state.lock().await;
                            state.journal.stopped = true;
                            state.journal.stop_reason = Some("execution_authority_expired".into());
                            if !state.journal.pending.is_empty() { state.journal.mark_unknown("expired_inflight_model_request"); }
                            let _ = watchdog_inner.persist(&mut state).await;
                            watchdog_inner.stop.send_replace(true);
                            break;
                        }
                    }
                }
            }
        });
        Ok(Self {
            inner,
            bridge_directory,
            accept: Some(accept),
            watchdog: Some(watchdog),
        })
    }

    pub fn bridge_dir(&self) -> &Path {
        &self.bridge_directory
    }

    pub async fn usage(&self) -> Result<JobUsage> {
        let mut state = self.inner.state.lock().await;
        self.inner.persist(&mut state).await?;
        Ok(state.journal.public_usage())
    }

    pub async fn renew_lease(&self, expires: DateTime<Utc>) -> Result<()> {
        self.inner.authority().await?;
        let now = Utc::now();
        ensure!(
            expires > now && expires <= now + chrono::Duration::seconds(120),
            "invalid gateway lease renewal"
        );
        let mut state = self.inner.state.lock().await;
        ensure!(
            !state.journal.stopped && !state.journal.unknown,
            "gateway is fenced"
        );
        state.journal.lease_expires_at = expires.min(state.journal.deadline);
        state.lease = monotonic(state.journal.lease_expires_at)?;
        self.inner.persist(&mut state).await
    }

    pub async fn narrow_limits(&self, limits: &JobLimits) -> Result<()> {
        ensure!(limits.is_finite_positive(), "invalid gateway limits");
        let mut state = self.inner.state.lock().await;
        state.tick_elapsed();
        state.journal.limits = state.journal.limits.narrowed_by(limits);
        let remaining = state
            .journal
            .limits
            .elapsed_seconds
            .saturating_sub(state.journal.usage.elapsed_seconds);
        let deadline = deadline_after(remaining)?;
        state.journal.deadline = state.journal.deadline.min(deadline);
        state.deadline = state.deadline.min(monotonic(state.journal.deadline)?);
        self.inner.persist(&mut state).await
    }

    pub async fn deny_models(&self) -> Result<()> {
        let mut state = self.inner.state.lock().await;
        state.journal.model_enabled = false;
        self.inner.persist(&mut state).await
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        self.inner.cancelled.store(true, Ordering::Release);
        let result = {
            let mut state = self.inner.state.lock().await;
            state.journal.stopped = true;
            if !state.journal.pending.is_empty() {
                state.journal.mark_unknown("stopped_inflight_model_request");
            }
            self.inner.persist(&mut state).await
        };
        self.inner.stop.send_replace(true);
        if let Some(task) = self.accept.take() {
            let _ = task.await;
        }
        if let Some(task) = self.watchdog.take() {
            let _ = task.await;
        }
        result
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.inner.cancelled.store(true, Ordering::Release);
        self.inner.stop.send_replace(true);
        if let Some(task) = self.accept.take() {
            task.abort();
        }
        if let Some(task) = self.watchdog.take() {
            task.abort();
        }
    }
}

fn monotonic(deadline: DateTime<Utc>) -> Result<Instant> {
    Instant::now()
        .checked_add((deadline - Utc::now()).to_std().unwrap_or_default())
        .context("monotonic execution deadline is out of range")
}

fn deadline_after(seconds: u64) -> Result<DateTime<Utc>> {
    let seconds = i64::try_from(seconds).context("execution duration is too large")?;
    let duration =
        chrono::Duration::try_seconds(seconds).context("execution duration is too large")?;
    Utc::now()
        .checked_add_signed(duration)
        .context("execution deadline is out of range")
}
