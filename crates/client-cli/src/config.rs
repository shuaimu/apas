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
    #[serde(default)]
    pub summaries: SummaryConfig,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SummaryAdapterKind {
    #[default]
    Disabled,
    Claude,
    /// Explicit opt-in: uses ephemeral, read-only headless execution but
    /// retains Codex's command tool and therefore has residual host-read risk.
    Codex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub adapter: SummaryAdapterKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default = "default_summary_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_summary_input_bytes")]
    pub max_input_bytes: usize,
    #[serde(default)]
    pub allow_cross_provider: bool,
}

fn default_summary_timeout_seconds() -> u64 {
    120
}

fn default_summary_input_bytes() -> usize {
    64 * 1024
}

impl Default for SummaryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            adapter: SummaryAdapterKind::Disabled,
            model: None,
            timeout_seconds: default_summary_timeout_seconds(),
            max_input_bytes: default_summary_input_bytes(),
            allow_cross_provider: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RemoteConfig {
    pub server: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    pub claude_path: String,
    #[serde(default = "default_codex_path")]
    pub codex_path: String,
    #[serde(default = "default_opencode_path")]
    pub opencode_path: String,
    #[serde(default = "default_cursor_agent_path")]
    pub cursor_agent_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepseek_api_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepseek_api_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default)]
    pub machine_id: Option<String>,
    #[serde(default)]
    pub project_roots: Vec<String>,
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
            codex_path: default_codex_path(),
            opencode_path: default_opencode_path(),
            cursor_agent_path: default_cursor_agent_path(),
            deepseek_api_base_url: None,
            deepseek_api_key: None,
        }
    }
}

/// Per-test isolation for [`Config::config_dir`].
///
/// Every input `config_dir()` resolves from -- `APAS_CONFIG_DIR`,
/// `XDG_CONFIG_HOME`, `$HOME` -- is process-global, and `cargo test` runs this
/// crate's tests in parallel threads of a *single* process. So tests cannot
/// isolate themselves from one another by setting those variables: whichever
/// test wrote one last wins for every other thread, however carefully each
/// test locks its own writes.
///
/// That raced. `project::tests` seed a temp `XDG_CONFIG_HOME` and assert on
/// files under it; `mode::dual_pane` tests concurrently call
/// `get_or_create_project`, which registers into `config_dir()/projects.json`
/// -- resolving, for those few milliseconds, to the project tests' directory.
/// The `!preferred_path.exists()` assertions then failed at random.
///
/// A thread-local override removes the shared state instead of locking around
/// it: each test thread resolves its own directory, with no lock and no
/// required ordering between tests. Tests that exercise env-based resolution
/// (`project::tests`) simply install no override and keep the real path.
#[cfg(test)]
pub(crate) mod test_config {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    thread_local! {
        static OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    /// Redirects `Config::config_dir()` on this thread for as long as it is
    /// held, then restores whatever was there before -- so nesting is safe and
    /// a panicking test cannot leak the override into the next test on the
    /// same thread.
    #[must_use = "the override lasts only as long as the guard is held"]
    pub(crate) struct ConfigDirGuard {
        previous: Option<PathBuf>,
        dir: tempfile::TempDir,
    }

    impl ConfigDirGuard {
        #[allow(dead_code)]
        pub(crate) fn path(&self) -> &Path {
            self.dir.path()
        }
    }

    impl Drop for ConfigDirGuard {
        fn drop(&mut self) {
            OVERRIDE.with(|slot| *slot.borrow_mut() = self.previous.take());
        }
    }

    /// Give this test its own config dir. Hold the guard for the whole test:
    /// anything that reaches `Config::config_dir()` -- project registration in
    /// particular -- then writes somewhere no other test can observe.
    pub(crate) fn isolated_config_dir() -> ConfigDirGuard {
        let dir = tempfile::tempdir().expect("temp config dir");
        let previous = OVERRIDE.with(|slot| slot.borrow_mut().replace(dir.path().to_path_buf()));
        ConfigDirGuard { previous, dir }
    }

    pub(super) fn current() -> Option<PathBuf> {
        OVERRIDE.with(|slot| slot.borrow().clone())
    }
}

impl Config {
    pub fn config_dir() -> Result<PathBuf> {
        // Tests isolate themselves per-thread; env vars can't, see `test_config`.
        #[cfg(test)]
        if let Some(dir) = test_config::current() {
            std::fs::create_dir_all(&dir)?;
            return Ok(dir);
        }
        // Escape hatch. Without it every code path that touches
        // `get_or_create_project` writes into the real `~/.config/apas/` --
        // including tests, which registered one entry per temp dir and left
        // dozens of dead paths the daemon then tried to spawn projects for.
        // Also useful operationally: two daemons on one host, or a throwaway
        // config for testing, without touching the user's real state.
        if let Some(dir) = std::env::var_os("APAS_CONFIG_DIR") {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir)?;
            return Ok(dir);
        }
        let proj_dirs = ProjectDirs::from("com", "apas", "apas")
            .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;

        let config_dir = proj_dirs.config_dir();
        std::fs::create_dir_all(config_dir)?;
        Ok(config_dir.to_path_buf())
    }

    pub fn config_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.toml"))
    }

    /// Host-local scratch dir for state that is only meaningful on this
    /// machine.
    ///
    /// `config_dir()` is under `$HOME`, which on a shared-NFS cluster every
    /// host sees. Pid files stored there were read by peers that then resolved
    /// the pid through their *own* `/proc` -- so a daemon on one host would
    /// either find an unrelated local process and refuse to start, or find
    /// nothing and take over another host's state.
    ///
    /// `$XDG_RUNTIME_DIR` is tmpfs and per-host, which is exactly right for a
    /// pid: it should not survive a reboot. Falls back to a uid-scoped dir
    /// under the system temp dir when the variable is unset (cron, ssh
    /// without a login session).
    pub fn runtime_dir() -> Result<PathBuf> {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| {
                let uid = unsafe { libc::getuid() };
                std::env::temp_dir().join(format!("apas-{uid}"))
            });
        let dir = base.join("apas");
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    pub fn daemon_pid_path() -> Result<PathBuf> {
        Ok(Self::runtime_dir()?.join("daemon.pid"))
    }

    pub fn daemon_state_path() -> Result<PathBuf> {
        Ok(Self::runtime_dir()?.join("daemon.json"))
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

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn retired_legacy_local_keys_are_ignored_and_not_serialized() {
        let config: Config = toml::from_str(
            r#"
            [local]
            claude_path = "claude"
            codex_path = "codex"
            minimax_path = "legacy-wrapper"
            minimax_api_base_url = "https://legacy.invalid"
            minimax_api_key = "must-not-survive"
            glm_api_base_url = "https://legacy.invalid"
            glm_api_key = "must-not-survive"
            deepseek_api_key = "supported-key"
            "#,
        )
        .expect("legacy fields should be ignored");

        assert_eq!(config.local.claude_path, "claude");
        assert_eq!(config.local.codex_path, "codex");
        assert_eq!(
            config.local.deepseek_api_key.as_deref(),
            Some("supported-key")
        );
        let saved = toml::to_string(&config).unwrap();
        assert!(!saved.contains("minimax"));
        assert!(!saved.contains("glm_api"));
        assert!(!saved.contains("must-not-survive"));
    }
}
