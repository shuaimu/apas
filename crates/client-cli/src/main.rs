use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
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
