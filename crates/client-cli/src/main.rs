use anyhow::Result;
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

mod auth;
mod claude;
mod config;
mod mode;
mod project;
mod tui;
mod update;
mod usage;

// Default server URL
const DEFAULT_SERVER: &str = "ws://apas.mpaxos.com:8080";
// Web UI URL for users to view sessions
const WEB_UI_URL: &str = "http://apas.mpaxos.com";
const CURRENT_VERSION: &str = env!("APAS_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonStateFile {
    pid: u32,
    version: String,
}

struct DaemonStateGuard {
    path: PathBuf,
    pid: u32,
}

impl DaemonStateGuard {
    fn new(path: PathBuf, pid: u32) -> Self {
        Self { path, pid }
    }
}

impl Drop for DaemonStateGuard {
    fn drop(&mut self) {
        if let Some(state) = read_daemon_state(&self.path) {
            if state.pid == self.pid {
                let _ = fs::remove_file(&self.path);
            }
        }
    }
}

#[derive(Debug, Clone)]
struct RunningDaemon {
    pid: u32,
    version: Option<String>,
}

#[derive(Parser)]
#[command(name = "apas")]
#[command(about = "Claude Code wrapper - runs locally and streams output to remote server")]
#[command(version = env!("APAS_VERSION"))]
struct Cli {
    /// Run in offline/local mode only - no server connection
    #[arg(long, visible_alias = "local", conflicts_with = "remote")]
    offline: bool,

    /// Run in remote-only mode - no local I/O, server controls everything
    #[arg(long, conflicts_with = "offline")]
    remote: bool,

    /// Run headless - same as default tabbed mode but without TUI (for daemon-spawned sessions)
    #[arg(long, conflicts_with_all = ["offline", "remote", "hybrid"])]
    headless: bool,

    /// Run in hybrid mode - single pane with local terminal + streaming (legacy)
    #[arg(long, conflicts_with_all = ["offline", "remote"])]
    hybrid: bool,

    /// Server URL (overrides config)
    #[arg(long)]
    server: Option<String>,

    /// Auth token (overrides config)
    #[arg(long)]
    token: Option<String>,

    /// Working directory
    #[arg(short = 'd', long)]
    working_dir: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Check for updates and install if available
    Update,
    /// Login to the APAS server
    Login,
    /// Logout from the APAS server
    Logout,
    /// Show current login status
    Whoami,
    /// Run per-machine daemon for machine/project control in web UI
    Daemon {
        /// Legacy option (ignored): daemon now reads ~/.config/apas/projects.json
        #[arg(long = "root", short = 'r')]
        roots: Vec<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Set a configuration value
    Set {
        /// Configuration key (server, token)
        key: String,
        /// Configuration value
        value: String,
    },
    /// Get a configuration value
    Get {
        /// Configuration key
        key: String,
    },
    /// Show all configuration
    Show,
    /// Get the config file path
    Path,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing (default to warn to avoid interfering with TUI)
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "apas=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    let cli = Cli::parse();

    // Auto-upgrade on boot if a new version is available
    // Skip for subcommands like update, login, etc.
    if cli.command.is_none() {
        update::check_and_upgrade_on_boot();
    }

    // Auto-start daemon for interactive/remote CLI modes (best-effort).
    if cli.command.is_none() && !cli.offline {
        if let Err(err) = maybe_auto_start_daemon(&cli) {
            tracing::warn!("Failed to auto-start daemon: {}", err);
        }
    }

    // Handle subcommands
    if let Some(command) = cli.command {
        match command {
            Commands::Config { action } => return handle_config_command(action).await,
            Commands::Update => {
                println!("Checking for updates...");
                update::check_and_update().await?;
                return Ok(());
            }
            Commands::Login => {
                let config = config::Config::load().unwrap_or_default();
                let server = cli
                    .server
                    .or(config.remote.server)
                    .unwrap_or_else(|| DEFAULT_SERVER.to_string());
                let token = auth::login(&server).await?;

                // Save the token
                let mut config = config::Config::load().unwrap_or_default();
                config.remote.token = Some(token);
                config.save()?;

                return Ok(());
            }
            Commands::Logout => {
                let mut config = config::Config::load().unwrap_or_default();
                auth::logout(&mut config)?;
                return Ok(());
            }
            Commands::Whoami => {
                let config = config::Config::load().unwrap_or_default();
                let server = cli
                    .server
                    .or(config.remote.server.clone())
                    .unwrap_or_else(|| DEFAULT_SERVER.to_string());
                auth::whoami(&config, &server).await?;
                return Ok(());
            }
            Commands::Daemon { roots } => {
                let state_path = config::Config::daemon_state_path()?;
                let legacy_pid_path = config::Config::daemon_pid_path()?;
                let my_pid = std::process::id();
                if let Some(existing) = detect_running_daemon(&state_path, &legacy_pid_path) {
                    // Skip if the detected PID is ourselves (auto-start wrote state before we ran)
                    if existing.pid != my_pid {
                        let should_restart = should_restart_for_version(
                            existing.version.as_deref(),
                            CURRENT_VERSION,
                        );
                        if should_restart {
                            tracing::info!(
                                "Restarting daemon pid {} due to older/unknown version {:?} -> {}",
                                existing.pid,
                                existing.version,
                                CURRENT_VERSION
                            );
                            stop_daemon_process(existing.pid)?;
                        } else {
                            println!(
                                "Daemon already running (pid {}, version {}).",
                                existing.pid,
                                existing.version.unwrap_or_else(|| "unknown".to_string())
                            );
                            return Ok(());
                        }
                    }
                }

                let mut config = config::Config::load().unwrap_or_default();
                let server = cli
                    .server
                    .or(config.remote.server.clone())
                    .unwrap_or_else(|| DEFAULT_SERVER.to_string());
                let token = match cli.token.or(config.remote.token.clone()) {
                    Some(t) => t,
                    None => {
                        eprintln!("\x1b[33m🔐 Not logged in.\x1b[0m");
                        eprintln!("   Run '\x1b[1mapas login\x1b[0m' to authenticate.");
                        return Ok(());
                    }
                };

                let machine_id = match config.daemon.machine_id.as_ref() {
                    Some(raw) => Uuid::parse_str(raw).unwrap_or_else(|_| Uuid::new_v4()),
                    None => Uuid::new_v4(),
                };
                config.daemon.machine_id = Some(machine_id.to_string());

                let project_roots = if !roots.is_empty() {
                    tracing::warn!(
                        "--root is deprecated and ignored; daemon now uses ~/.config/apas/projects.json"
                    );
                    config.daemon.project_roots = roots
                        .iter()
                        .map(|root| root.to_string_lossy().to_string())
                        .collect();
                    roots
                } else {
                    config
                        .daemon
                        .project_roots
                        .iter()
                        .map(PathBuf::from)
                        .collect()
                };

                config.save()?;
                let state = DaemonStateFile {
                    pid: std::process::id(),
                    version: CURRENT_VERSION.to_string(),
                };
                write_daemon_state(&state_path, &state)?;
                let _state_guard = DaemonStateGuard::new(state_path, std::process::id());
                mode::daemon::run(&server, &token, machine_id, project_roots).await?;
                return Ok(());
            }
        }
    }

    // Get working directory
    let working_dir = cli
        .working_dir
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    if cli.offline {
        // Offline/local mode - no server connection
        tracing::info!("Starting in offline mode (no server connection)");
        mode::local::run(&working_dir).await?;
    } else if cli.headless {
        // Headless mode - tabbed mode without TUI (for daemon-spawned sessions)
        let config = config::Config::load()?;
        let server = cli
            .server
            .or(config.remote.server)
            .unwrap_or_else(|| DEFAULT_SERVER.to_string());
        let token = match cli.token.or(config.remote.token) {
            Some(t) => t,
            None => {
                eprintln!("Not logged in. Run 'apas login' to authenticate.");
                return Ok(());
            }
        };

        tracing::info!("Starting in headless mode (streaming to {})", server);
        mode::dual_pane::run_headless(&server, &token, &working_dir).await?;
    } else if cli.remote {
        // Remote-only mode - no local I/O
        let config = config::Config::load()?;
        let server = cli
            .server
            .or(config.remote.server)
            .unwrap_or_else(|| DEFAULT_SERVER.to_string());
        let token = match cli.token.or(config.remote.token) {
            Some(t) => t,
            None => {
                eprintln!("\x1b[33m🔐 Not logged in.\x1b[0m");
                eprintln!("   Run '\x1b[1mapas login\x1b[0m' to authenticate.");
                return Ok(());
            }
        };

        // Show web UI hint
        eprintln!(
            "\x1b[36m📺 View this session in browser: {}\x1b[0m",
            WEB_UI_URL
        );

        tracing::info!("Starting in remote-only mode, connecting to {}", server);
        mode::remote::run(&server, &token, &working_dir).await?;
    } else if cli.hybrid {
        // Hybrid mode - single pane local terminal + streaming to server
        let config = config::Config::load()?;
        let server = cli
            .server
            .or(config.remote.server)
            .unwrap_or_else(|| DEFAULT_SERVER.to_string());
        let token = match cli.token.or(config.remote.token) {
            Some(t) => t,
            None => {
                eprintln!("\x1b[33m🔐 Not logged in.\x1b[0m");
                eprintln!("   Run '\x1b[1mapas login\x1b[0m' to authenticate.");
                return Ok(());
            }
        };

        // Show web UI hint
        eprintln!(
            "\x1b[36m📺 View this session in browser: {}\x1b[0m",
            WEB_UI_URL
        );

        tracing::info!("Starting in hybrid mode (local + streaming to {})", server);
        mode::hybrid::run(&server, &token, &working_dir).await?;
    } else {
        // Default: tabbed mode (interactive tab by default)
        let config = config::Config::load()?;
        let server = cli
            .server
            .or(config.remote.server)
            .unwrap_or_else(|| DEFAULT_SERVER.to_string());
        let token = match cli.token.or(config.remote.token) {
            Some(t) => t,
            None => {
                eprintln!("\x1b[33m🔐 Not logged in.\x1b[0m");
                eprintln!("   Run '\x1b[1mapas login\x1b[0m' to authenticate.");
                return Ok(());
            }
        };

        // Show web UI hint
        eprintln!(
            "\x1b[36m📺 View this session in browser: {}\x1b[0m",
            WEB_UI_URL
        );

        tracing::info!("Starting in dual-pane mode (streaming to {})", server);
        mode::dual_pane::run(&server, &token, &working_dir).await?;
    }

    Ok(())
}

fn maybe_auto_start_daemon(cli: &Cli) -> Result<()> {
    // Auto-start only for logged-in users with persistent config token
    // to avoid leaking one-off CLI tokens in process args.
    let config = config::Config::load().unwrap_or_default();
    if config.remote.token.is_none() {
        return Ok(());
    }

    let server = cli
        .server
        .clone()
        .or(config.remote.server.clone())
        .unwrap_or_else(|| DEFAULT_SERVER.to_string());

    ensure_daemon_running(&server, &config.daemon.project_roots, CURRENT_VERSION)
}

fn ensure_daemon_running(server: &str, roots: &[String], target_version: &str) -> Result<()> {
    let state_path = config::Config::daemon_state_path()?;
    let legacy_pid_path = config::Config::daemon_pid_path()?;

    if let Some(running) = detect_running_daemon(&state_path, &legacy_pid_path) {
        if should_restart_for_version(running.version.as_deref(), target_version) {
            tracing::info!(
                "Auto-restarting daemon pid {} for version upgrade {:?} -> {}",
                running.pid,
                running.version,
                target_version
            );
            stop_daemon_process(running.pid)?;
        } else {
            return Ok(());
        }
    }

    let mut cmd = Command::new(resolve_apas_executable());
    cmd.arg("--server").arg(server).arg("daemon");
    for root in roots {
        cmd.arg("--root").arg(root);
    }
    // Preserve the launching shell's PATH so the daemon and its headless
    // children can find claude/codex installed via nvm etc.
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    let log_path = state_path.with_file_name("daemon.log");
    let log_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .unwrap_or_else(|_| fs::File::create("/dev/null").unwrap());
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log_file));

    // Detach the daemon fully from this process so it survives our exit and
    // logout: new session (setsid) + new process group. Without this, the
    // daemon's lifetime gets tied to our session's cgroup/pgrp.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                // New session: detach from controlling terminal and
                // escape the parent's process group.
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    let child = cmd.spawn()?;
    let state = DaemonStateFile {
        pid: child.id(),
        version: target_version.to_string(),
    };
    write_daemon_state(&state_path, &state)?;
    // Keep legacy pid file for compatibility with older tooling.
    let _ = fs::write(legacy_pid_path, child.id().to_string());
    Ok(())
}

fn should_restart_for_version(running_version: Option<&str>, target_version: &str) -> bool {
    match running_version {
        None => true, // Older daemon versions didn't persist version metadata.
        Some(running) => match compare_versions(running, target_version) {
            Some(std::cmp::Ordering::Less) => true,
            Some(_) => false,
            None => false, // Unknown format: keep current daemon to avoid accidental downgrade.
        },
    }
}

fn compare_versions(left: &str, right: &str) -> Option<std::cmp::Ordering> {
    let l = parse_version(left)?;
    let r = parse_version(right)?;
    Some(l.cmp(&r))
}

fn parse_version(v: &str) -> Option<u64> {
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let yy: u64 = parts[0].parse().ok()?;
    let mm: u64 = parts[1].parse().ok()?;
    let commit: u64 = parts[2].parse().ok()?;
    Some(yy * 1_000_000 + mm * 10_000 + commit)
}

fn detect_running_daemon(state_path: &Path, legacy_pid_path: &Path) -> Option<RunningDaemon> {
    if let Some(state) = read_daemon_state(state_path) {
        if is_apas_daemon_process(state.pid) {
            return Some(RunningDaemon {
                pid: state.pid,
                version: Some(state.version),
            });
        }
        let _ = fs::remove_file(state_path);
    }

    let legacy_pid = read_legacy_daemon_pid(legacy_pid_path)?;
    if is_apas_daemon_process(legacy_pid) {
        return Some(RunningDaemon {
            pid: legacy_pid,
            version: None,
        });
    }
    let _ = fs::remove_file(legacy_pid_path);
    None
}

fn stop_daemon_process(pid: u32) -> Result<()> {
    if !is_apas_daemon_process(pid) {
        return Ok(());
    }

    let _ = Command::new("kill").arg(pid.to_string()).status();

    for _ in 0..40 {
        if !is_apas_daemon_process(pid) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }

    let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();

    for _ in 0..20 {
        if !is_apas_daemon_process(pid) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }

    anyhow::bail!("Failed to stop existing daemon process {}", pid)
}

fn resolve_apas_executable() -> PathBuf {
    update::resolve_preferred_apas_executable()
}

fn read_daemon_state(path: &Path) -> Option<DaemonStateFile> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str::<DaemonStateFile>(&text).ok()
}

fn write_daemon_state(path: &Path, state: &DaemonStateFile) -> Result<()> {
    let content = serde_json::to_string(state)?;
    fs::write(path, content)?;
    Ok(())
}

fn read_legacy_daemon_pid(path: &Path) -> Option<u32> {
    let text = fs::read_to_string(path).ok()?;
    text.trim().parse::<u32>().ok()
}

fn is_apas_daemon_process(pid: u32) -> bool {
    let cmdline_path = format!("/proc/{}/cmdline", pid);
    let raw = match fs::read(cmdline_path) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };
    if raw.is_empty() {
        return false;
    }

    let args: Vec<String> = raw
        .split(|b| *b == 0)
        .filter(|segment| !segment.is_empty())
        .map(|segment| String::from_utf8_lossy(segment).to_string())
        .collect();

    if args.is_empty() {
        return false;
    }

    let has_apas_binary = args.iter().any(|arg| arg.contains("apas"));
    let has_daemon_arg = args.iter().any(|arg| arg == "daemon");
    has_apas_binary && has_daemon_arg
}

async fn handle_config_command(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Set { key, value } => {
            let mut config = config::Config::load().unwrap_or_default();
            match key.as_str() {
                "server" => config.remote.server = Some(value),
                "token" => config.remote.token = Some(value),
                "claude_path" => config.local.claude_path = value,
                "minimax_path" => config.local.minimax_path = value,
                "codex_path" => config.local.codex_path = value,
                "minimax_api_base_url" => {
                    config.local.minimax_api_base_url = if value.trim().is_empty() {
                        None
                    } else {
                        Some(value)
                    };
                }
                "minimax_api_key" => {
                    config.local.minimax_api_key = if value.trim().is_empty() {
                        None
                    } else {
                        Some(value)
                    };
                }
                "glm_api_base_url" => {
                    config.local.glm_api_base_url = if value.trim().is_empty() {
                        None
                    } else {
                        Some(value)
                    };
                }
                "glm_api_key" => {
                    config.local.glm_api_key = if value.trim().is_empty() {
                        None
                    } else {
                        Some(value)
                    };
                }
                "daemon_machine_id" => config.daemon.machine_id = Some(value),
                "daemon_roots" => {
                    config.daemon.project_roots = value
                        .split(',')
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                        .collect();
                }
                _ => anyhow::bail!(
                    "Unknown config key: {}. Valid keys: server, token, claude_path, minimax_path, codex_path, minimax_api_base_url, minimax_api_key, glm_api_base_url, glm_api_key, daemon_machine_id, daemon_roots",
                    key
                ),
            }
            config.save()?;
            println!("Configuration saved");
        }
        ConfigAction::Get { key } => {
            let config = config::Config::load()?;
            let value = match key.as_str() {
                "server" => config.remote.server.unwrap_or_default(),
                "token" => config
                    .remote
                    .token
                    .map(|_| "****")
                    .unwrap_or_default()
                    .to_string(),
                "claude_path" => config.local.claude_path,
                "minimax_path" => config.local.minimax_path,
                "codex_path" => config.local.codex_path,
                "minimax_api_base_url" => config.local.minimax_api_base_url.unwrap_or_default(),
                "minimax_api_key" => config
                    .local
                    .minimax_api_key
                    .map(|_| "****".to_string())
                    .unwrap_or_default(),
                "glm_api_base_url" => config.local.glm_api_base_url.unwrap_or_default(),
                "glm_api_key" => config
                    .local
                    .glm_api_key
                    .map(|_| "****".to_string())
                    .unwrap_or_default(),
                "daemon_machine_id" => config.daemon.machine_id.unwrap_or_default(),
                "daemon_roots" => config.daemon.project_roots.join(","),
                _ => anyhow::bail!(
                    "Unknown config key: {}. Valid keys: server, token, claude_path, minimax_path, codex_path, minimax_api_base_url, minimax_api_key, glm_api_base_url, glm_api_key, daemon_machine_id, daemon_roots",
                    key
                ),
            };
            println!("{}", value);
        }
        ConfigAction::Show => {
            let config = config::Config::load()?;
            println!("server: {}", config.remote.server.unwrap_or_default());
            println!(
                "token: {}",
                config.remote.token.map(|_| "****").unwrap_or_default()
            );
            println!("claude_path: {}", config.local.claude_path);
            println!("minimax_path: {}", config.local.minimax_path);
            println!("codex_path: {}", config.local.codex_path);
            println!(
                "minimax_api_base_url: {}",
                config.local.minimax_api_base_url.unwrap_or_default()
            );
            println!(
                "minimax_api_key: {}",
                config
                    .local
                    .minimax_api_key
                    .as_ref()
                    .map(|_| "****")
                    .unwrap_or("")
            );
            println!(
                "glm_api_base_url: {}",
                config.local.glm_api_base_url.unwrap_or_default()
            );
            println!(
                "glm_api_key: {}",
                config
                    .local
                    .glm_api_key
                    .as_ref()
                    .map(|_| "****")
                    .unwrap_or("")
            );
            println!(
                "daemon_machine_id: {}",
                config.daemon.machine_id.unwrap_or_default()
            );
            println!("daemon_roots: {}", config.daemon.project_roots.join(","));
        }
        ConfigAction::Path => {
            let path = config::Config::config_path()?;
            println!("{}", path.display());
        }
    }
    Ok(())
}
