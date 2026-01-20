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

fn default_true() -> bool { true }
fn default_smtp_port() -> u16 { 587 }

impl Default for SmtpConfig {
    fn default() -> Self {
        Self {
            enabled: true,  // Enable by default, using sendmail
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
                token_expiry_hours: 24,
            },
            smtp: SmtpConfig::default(),
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
