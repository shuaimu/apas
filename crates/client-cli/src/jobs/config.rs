//! Operator-owned autonomous execution profiles. Never supplied by a job caller.

use serde::{Deserialize, Serialize};
use shared::{JobLimits, JobModelApi};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AutonomousJobConfig {
    pub enabled: bool,
    /// Private host-local control storage, never mounted into untrusted containers.
    pub storage_root: Option<PathBuf>,
    /// Dedicated finite ext4/xfs mounts, exclusively reserved one per run.
    pub workspace_roots: Vec<PathBuf>,
    #[serde(default = "default_podman_binary")]
    pub podman_binary: PathBuf,
    /// Dedicated Podman vfs store/runroot; both must be provisioned to enable jobs.
    pub podman_storage_root: Option<PathBuf>,
    pub podman_runroot: Option<PathBuf>,
    pub profiles: Vec<WorkerProfileConfig>,
    pub models: Vec<ModelProfileConfig>,
    pub verifiers: Vec<VerificationProfileConfig>,
    pub publishers: Vec<PublisherProfileConfig>,
}

impl Default for AutonomousJobConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            storage_root: None,
            workspace_roots: Vec::new(),
            podman_binary: default_podman_binary(),
            podman_storage_root: None,
            podman_runroot: None,
            profiles: Vec::new(),
            models: Vec::new(),
            verifiers: Vec::new(),
            publishers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerProfileConfig {
    pub key: String,
    pub label: String,
    pub image: String,
    pub omp_version: String,
    pub omp_binary: String,
    pub models: Vec<String>,
    pub verifiers: Vec<String>,
    pub publisher: String,
    pub limits: JobLimits,
    #[serde(default)]
    pub dependency_hosts: Vec<String>,
    /// Private operator-recorded results of real RPC/child/gateway qualification.
    #[serde(default)]
    pub qualification_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelProfileConfig {
    pub key: String,
    pub label: String,
    pub model: String,
    pub api: JobModelApi,
    /// Operator-qualified native model capabilities; never inferred from a name.
    pub context_window: u32,
    pub reasoning: bool,
    pub upstream_base_url: String,
    pub credential_file: PathBuf,
    pub input_micro_usd_per_million: u64,
    pub output_micro_usd_per_million: u64,
    pub cached_input_micro_usd_per_million: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationProfileConfig {
    pub key: String,
    pub label: String,
    pub image: String,
    pub checks: Vec<VerificationCheckConfig>,
    #[serde(default)]
    pub dependency_hosts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationCheckConfig {
    pub name: String,
    pub argv: Vec<String>,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublisherProfileConfig {
    pub key: String,
    pub credential_file: PathBuf,
    pub repositories: Vec<String>,
    pub protected_base_refs: Vec<String>,
    /// Explicit operator qualification; a draft PR can still execute CI.
    pub ci_qualified: bool,
}

fn default_podman_binary() -> PathBuf {
    PathBuf::from("/usr/bin/podman")
}

impl AutonomousJobConfig {
    pub fn storage_root(&self) -> PathBuf {
        self.storage_root.clone().unwrap_or_else(|| {
            PathBuf::from(format!("/var/lib/apas/users/{}/jobs", unsafe {
                libc::geteuid()
            }))
        })
    }
}
