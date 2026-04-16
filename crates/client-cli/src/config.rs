use anyhow::Result;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub remote: RemoteConfig,
    #[serde(default)]
    pub local: LocalConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemoteConfig {
    pub server: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    pub claude_path: String,
    #[serde(default = "default_minimax_path")]
    pub minimax_path: String,
    #[serde(default = "default_codex_path")]
    pub codex_path: String,
    #[serde(default = "default_opencode_path")]
    pub opencode_path: String,
    #[serde(default = "default_cursor_agent_path")]
    pub cursor_agent_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimax_api_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimax_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glm_api_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glm_api_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default)]
    pub machine_id: Option<String>,
    #[serde(default)]
    pub project_roots: Vec<String>,
}

fn default_minimax_path() -> String {
    "claude2".to_string()
}

fn default_codex_path() -> String {
    "codex".to_string()
}

fn default_opencode_path() -> String {
    "opencode".to_string()
}

fn default_cursor_agent_path() -> String {
    "cursor-agent".to_string()
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            claude_path: "claude".to_string(),
            minimax_path: default_minimax_path(),
            codex_path: default_codex_path(),
            opencode_path: default_opencode_path(),
            cursor_agent_path: default_cursor_agent_path(),
            minimax_api_base_url: None,
            minimax_api_key: None,
            glm_api_base_url: None,
            glm_api_key: None,
        }
    }
}

impl Config {
    pub fn config_dir() -> Result<PathBuf> {
        let proj_dirs = ProjectDirs::from("com", "apas", "apas")
            .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;

        let config_dir = proj_dirs.config_dir();
        std::fs::create_dir_all(config_dir)?;
        Ok(config_dir.to_path_buf())
    }

    pub fn config_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.toml"))
    }

    pub fn daemon_pid_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("daemon.pid"))
    }

    pub fn daemon_state_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("daemon.json"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;

        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}
