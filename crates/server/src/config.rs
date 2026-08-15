use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub auth: AuthConfig,
    #[serde(default)]
    pub smtp: SmtpConfig,
    #[serde(default)]
    pub mobile: MobileConfig,
    #[serde(default)]
    pub summaries: SummaryConfig,
    #[serde(default)]
    pub system_admin: SystemAdminConfig,
}

/// The deployment's single system administrator. This is a credential, not an
/// account: it lives outside the `users` table, cannot be granted through any
/// UI, and its token authorizes nothing but `/admin/*`. The bootstrap secret
/// is used only to seed the credential when none is stored yet; rotate it from
/// the administration surface after the first sign-in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemAdminConfig {
    #[serde(default = "default_system_admin_username")]
    pub username: String,
    #[serde(default)]
    pub bootstrap_password: String,
    #[serde(default = "default_system_admin_token_expiry_minutes")]
    pub token_expiry_minutes: u64,
    #[serde(default = "default_system_admin_max_failures")]
    pub max_failed_attempts: u32,
    #[serde(default = "default_system_admin_lockout_seconds")]
    pub lockout_seconds: u64,
}

fn default_system_admin_username() -> String {
    "admin".to_string()
}

fn default_system_admin_token_expiry_minutes() -> u64 {
    120
}

fn default_system_admin_max_failures() -> u32 {
    5
}

fn default_system_admin_lockout_seconds() -> u64 {
    300
}

impl Default for SystemAdminConfig {
    fn default() -> Self {
        Self {
            username: default_system_admin_username(),
            bootstrap_password: String::new(),
            token_expiry_minutes: default_system_admin_token_expiry_minutes(),
            max_failed_attempts: default_system_admin_max_failures(),
            lockout_seconds: default_system_admin_lockout_seconds(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_summary_reconcile_minutes")]
    pub reconcile_interval_minutes: u64,
    #[serde(default = "default_summary_global_concurrency")]
    pub global_concurrency: usize,
    #[serde(default = "default_summary_sessions_per_scan")]
    pub max_sessions_per_scan: usize,
    #[serde(default = "default_summary_source_bytes")]
    pub max_source_bytes: usize,
    #[serde(default = "default_summary_chunk_bytes")]
    pub max_chunk_bytes: usize,
    #[serde(default = "default_summary_chunks")]
    pub max_chunks: usize,
    #[serde(default = "default_summary_timeout_seconds")]
    pub job_timeout_seconds: u64,
    #[serde(default = "default_summary_refresh_throttle_seconds")]
    pub refresh_throttle_seconds: u64,
    #[serde(default = "default_summary_attempts")]
    pub max_attempts: u32,
}

fn default_summary_reconcile_minutes() -> u64 {
    15
}
fn default_summary_global_concurrency() -> usize {
    2
}
fn default_summary_sessions_per_scan() -> usize {
    100
}
fn default_summary_source_bytes() -> usize {
    64 * 1024
}
fn default_summary_chunk_bytes() -> usize {
    12 * 1024
}
fn default_summary_chunks() -> usize {
    16
}
fn default_summary_timeout_seconds() -> u64 {
    120
}
fn default_summary_refresh_throttle_seconds() -> u64 {
    60
}
fn default_summary_attempts() -> u32 {
    3
}

impl Default for SummaryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            reconcile_interval_minutes: default_summary_reconcile_minutes(),
            global_concurrency: default_summary_global_concurrency(),
            max_sessions_per_scan: default_summary_sessions_per_scan(),
            max_source_bytes: default_summary_source_bytes(),
            max_chunk_bytes: default_summary_chunk_bytes(),
            max_chunks: default_summary_chunks(),
            job_timeout_seconds: default_summary_timeout_seconds(),
            refresh_throttle_seconds: default_summary_refresh_throttle_seconds(),
            max_attempts: default_summary_attempts(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MobileConfig {
    #[serde(default)]
    pub features: shared::MobileFeatureFlags,
    #[serde(default)]
    pub allow_insecure_localhost: bool,
    #[serde(default)]
    pub push: MobilePushConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MobilePushConfig {
    #[serde(default = "default_expo_push_url")]
    pub expo_push_url: String,
    #[serde(default = "default_expo_receipts_url")]
    pub expo_receipts_url: String,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default = "default_push_batch_size")]
    pub batch_size: usize,
}

fn default_expo_push_url() -> String {
    "https://exp.host/--/api/v2/push/send".to_string()
}

fn default_expo_receipts_url() -> String {
    "https://exp.host/--/api/v2/push/getReceipts".to_string()
}

fn default_push_batch_size() -> usize {
    100
}

impl Default for MobilePushConfig {
    fn default() -> Self {
        Self {
            expo_push_url: default_expo_push_url(),
            expo_receipts_url: default_expo_receipts_url(),
            access_token: None,
            batch_size: default_push_batch_size(),
        }
    }
}

impl Default for MobileConfig {
    fn default() -> Self {
        Self {
            features: shared::MobileFeatureFlags {
                bootstrap: false,
                coding_mutations: false,
                terminal: false,
                notifications: false,
                deep_links: false,
            },
            allow_insecure_localhost: false,
            push: MobilePushConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub jwt_secret: String,
    pub token_expiry_hours: u64,
    #[serde(default = "default_mobile_access_expiry_minutes")]
    pub mobile_access_expiry_minutes: u64,
    #[serde(default = "default_mobile_refresh_expiry_days")]
    pub mobile_refresh_expiry_days: u64,
}

fn default_mobile_access_expiry_minutes() -> u64 {
    15
}

fn default_mobile_refresh_expiry_days() -> u64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmtpConfig {
    pub enabled: bool,
    /// Use local sendmail binary instead of SMTP server
    #[serde(default = "default_true")]
    pub use_sendmail: bool,
    /// SMTP server host (only used if use_sendmail is false)
    #[serde(default)]
    pub host: String,
    /// SMTP server port (only used if use_sendmail is false)
    #[serde(default = "default_smtp_port")]
    pub port: u16,
    /// SMTP username (only used if use_sendmail is false)
    #[serde(default)]
    pub username: String,
    /// SMTP password (only used if use_sendmail is false)
    #[serde(default)]
    pub password: String,
    pub from_email: String,
    pub from_name: String,
}

fn default_true() -> bool {
    true
}
fn default_smtp_port() -> u16 {
    587
}

impl Default for SmtpConfig {
    fn default() -> Self {
        Self {
            enabled: true, // Enable by default, using sendmail
            use_sendmail: true,
            host: "".to_string(),
            port: 587,
            username: "".to_string(),
            password: "".to_string(),
            from_email: "noreply@apas.mpaxos.com".to_string(),
            from_name: "APAS".to_string(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 8080,
            },
            database: DatabaseConfig {
                path: "./data/apas.db".to_string(),
            },
            auth: AuthConfig {
                jwt_secret: "change-me-in-production".to_string(),
                token_expiry_hours: 876000, // ~100 years (never expire)
                mobile_access_expiry_minutes: default_mobile_access_expiry_minutes(),
                mobile_refresh_expiry_days: default_mobile_refresh_expiry_days(),
            },
            smtp: SmtpConfig::default(),
            mobile: MobileConfig::default(),
            summaries: SummaryConfig::default(),
            system_admin: SystemAdminConfig::default(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        // Try to load from environment variable
        if let Ok(path) = std::env::var("APAS_CONFIG") {
            return Self::load_from_path(&PathBuf::from(path));
        }

        // Try to load from default locations
        let default_paths = vec![
            PathBuf::from("apas-server.toml"),
            PathBuf::from("config/apas-server.toml"),
            PathBuf::from("/etc/apas/server.toml"),
        ];

        for path in default_paths {
            if path.exists() {
                return Self::load_from_path(&path);
            }
        }

        // Return default config if no file found
        tracing::warn!("No config file found, using defaults");
        Ok(Self::default())
    }

    fn load_from_path(path: &PathBuf) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}
