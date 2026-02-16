use anyhow::Result;
use clap::{Parser, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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

struct DaemonPidGuard {
    path: PathBuf,
    pid: u32,
}

impl DaemonPidGuard {
    fn new(path: PathBuf, pid: u32) -> Self {
        Self { path, pid }
    }
}

impl Drop for DaemonPidGuard {
    fn drop(&mut self) {
        let expected = self.pid.to_string();
        if let Ok(contents) = fs::read_to_string(&self.path) {
            if contents.trim() == expected {
                let _ = fs::remove_file(&self.path);
            }
        }
    }
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
        /// Project root directory to scan (repeatable)
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
                let pid_path = config::Config::daemon_pid_path()?;
                if let Some(existing_pid) = daemon_pid_running(&pid_path) {
                    println!("Daemon already running (pid {}).", existing_pid);
                    return Ok(());
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
                write_daemon_pid(&pid_path, std::process::id())?;
                let _pid_guard = DaemonPidGuard::new(pid_path, std::process::id());
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

    ensure_daemon_running(&server, &config.daemon.project_roots)
}

fn ensure_daemon_running(server: &str, roots: &[String]) -> Result<()> {
    let pid_path = config::Config::daemon_pid_path()?;
    if daemon_pid_running(&pid_path).is_some() {
        return Ok(());
    }

    let current_exe = std::env::current_exe()?;
    let mut cmd = Command::new(current_exe);
    cmd.arg("daemon").arg("--server").arg(server);
    for root in roots {
        cmd.arg("--root").arg(root);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let child = cmd.spawn()?;
    write_daemon_pid(&pid_path, child.id())?;
    Ok(())
}

fn daemon_pid_running(path: &Path) -> Option<u32> {
    let pid = read_daemon_pid(path)?;
    if is_apas_daemon_process(pid) {
        Some(pid)
    } else {
        let _ = fs::remove_file(path);
        None
    }
}

fn read_daemon_pid(path: &Path) -> Option<u32> {
    let text = fs::read_to_string(path).ok()?;
    text.trim().parse::<u32>().ok()
}

fn write_daemon_pid(path: &Path, pid: u32) -> Result<()> {
    fs::write(path, pid.to_string())?;
    Ok(())
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
                "codex_path" => config.local.codex_path = value,
                "daemon_machine_id" => config.daemon.machine_id = Some(value),
                "daemon_roots" => {
                    config.daemon.project_roots = value
                        .split(',')
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                        .collect();
                }
                _ => anyhow::bail!(
                    "Unknown config key: {}. Valid keys: server, token, claude_path, codex_path, daemon_machine_id, daemon_roots",
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
                "codex_path" => config.local.codex_path,
                "daemon_machine_id" => config.daemon.machine_id.unwrap_or_default(),
                "daemon_roots" => config.daemon.project_roots.join(","),
                _ => anyhow::bail!(
                    "Unknown config key: {}. Valid keys: server, token, claude_path, codex_path, daemon_machine_id, daemon_roots",
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
            println!("codex_path: {}", config.local.codex_path);
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
