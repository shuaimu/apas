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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemoteConfig {
    pub server: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    pub claude_path: String,
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            claude_path: "claude".to_string(),
        }
    }
}

impl Config {
    pub fn config_path() -> Result<PathBuf> {
        let proj_dirs = ProjectDirs::from("com", "apas", "apas")
            .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;

        let config_dir = proj_dirs.config_dir();
        std::fs::create_dir_all(config_dir)?;

        Ok(config_dir.join("config.toml"))
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
