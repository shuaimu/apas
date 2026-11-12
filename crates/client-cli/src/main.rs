use anyhow::Result;
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod attach;
mod auth;
mod claude;
mod claude_session_hook;
mod config;
mod conversation;
mod daemon_registry;
mod file_watcher;
mod host_storage;
mod jobs;
mod mode;
mod omp_activity;
mod omp_local;
mod pane_host;
mod pane_identity;
mod pane_status;
mod plan_review;
mod project;
mod project_instances;
mod project_join;
mod provider_local;
mod summary_runner;
mod supervisor;
mod terminal_pane;
mod transcript;
mod tui;
mod update;
mod usage;
mod worktree;

// Default server URL
const DEFAULT_SERVER: &str = "wss://apas.mpaxos.com";
// Web UI URL for users to view sessions
const WEB_UI_URL: &str = "https://apas.mpaxos.com";
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

    /// Open the terminal UI for a project already running on this host.
    ///
    /// Not the default: running `apas` in a project directory registers it and
    /// exits, because a host runs one instance per user and projects are
    /// started from the web. This is the local view for when the web is not
    /// reachable, and it renders little beyond pane names.
    #[arg(long, conflicts_with_all = ["offline", "remote", "hybrid", "headless"])]
    attach: bool,

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
    /// Register a directory (default: the current one) as a project on this
    /// machine so it appears in the web UI. Does not start it.
    ///
    /// A checkout of a repository one of your projects already uses can join
    /// that project instead, as another machine of it: registration asks
    /// before joining, and lists the projects when several match. Only a
    /// folder whose project has no tabs and has never been started can join.
    Register {
        /// Project directory (default: current directory)
        path: Option<PathBuf>,
        /// Join this existing project, by id, without asking
        #[arg(long, value_name = "ID", conflicts_with = "new")]
        project: Option<uuid::Uuid>,
        /// Register the folder as a separate project; skip matching it against
        /// your existing projects
        #[arg(long)]
        new: bool,
        /// Join the one matching project without asking
        #[arg(long, short = 'y')]
        yes: bool,
    },
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
    /// Manage per-pane isolated git worktrees (Phase 1.1 swarm plan)
    Worktree {
        #[command(subcommand)]
        action: WorktreeAction,
    },
    /// Internal persistent terminal runtime. Started under project-scoped
    /// tmux supervision; not intended for direct use.
    #[command(hide = true)]
    PaneHost {
        #[arg(long)]
        runtime_dir: PathBuf,
    },
    /// Internal durable autonomous worker host (never a terminal pane).
    #[command(hide = true)]
    JobHost {
        #[arg(long)]
        run_dir: PathBuf,
    },
    /// Internal sandbox-only loopback model/dependency bridge.
    #[command(hide = true)]
    JobGatewayBridge {
        #[arg(long)]
        socket: PathBuf,
        #[arg(long)]
        token_file: PathBuf,
        #[arg(long)]
        listen: std::net::SocketAddr,
    },
    /// Internal `SessionStart` hook for Claude panes: reads the hook payload on
    /// stdin and records which transcript that pane is writing. Not intended
    /// for direct use.
    #[command(hide = true)]
    SessionHook,
}

#[derive(Subcommand)]
enum WorktreeAction {
    /// Create a git worktree for a pane and persist its path into .apas
    Add {
        /// Pane id (0 = deadloop, 1 = interactive, or a dynamic-tab id)
        pane_id: u32,
        /// Branch name (default: `apas-pane-<id>`)
        branch: Option<String>,
        /// Worktree directory path (default: `<project>/.apas-worktrees/pane-<id>`)
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Clear a pane's worktree assignment in .apas (does NOT delete the git worktree)
    Remove {
        /// Pane id
        pane_id: u32,
    },
    /// List all worktree assignments in the current project's .apas
    List,
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
    let cli = Cli::parse();

    // A pane-host's stderr is the tmux pane it runs in, which vanishes with
    // the process. It also logs to a host-local file that outlives it (see
    // `pane_host::open_host_log`), at info level so the lifecycle — start,
    // adoption, lease expiry, provider exit, fatal signal — is on record when
    // a dead host has to be explained after the fact.
    let host_log = match &cli.command {
        Some(Commands::PaneHost { runtime_dir }) => match pane_host::open_host_log(runtime_dir) {
            Ok(log) => Some(log),
            Err(error) => {
                eprintln!("pane-host: running without a log file: {error}");
                None
            }
        },
        _ => None,
    };
    let default_filter = if host_log.is_some() { "apas=info" } else { "apas=warn" };
    if let Some(log) = &host_log {
        pane_host::install_host_diagnostics(log);
    }

    // Every project used to have its own tmux session and its own stderr file,
    // which separated their records for free. Projects now share this process,
    // so each carries a `project` span and the records must show it — without
    // that, one incident's log is an interleaving of several projects with no
    // way to tell them apart.
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_filter.into()),
        )
        // The default `Full` format prints the enclosing span and its fields,
        // which is what carries the project id onto every record.
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(host_log.map(|log| {
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(log))
        }))
        .init();

    // Snapshot the exec path while it's still a clean, real on-disk
    // path. Later, when the binary gets replaced atomically (NFS
    // silly-rename → `.nfsXXX`), `current_exe()` returns the stale
    // inode and the existing fallback chain has to guess the install
    // location. Capturing here makes `resolve_preferred_apas_executable`
    // deterministic.
    update::capture_launch_binary_path();

    // Auto-upgrade on boot if a new version is available.
    // Skip for subcommands (login, update, etc.) and for --headless:
    // headless processes are spawned by the daemon, often several at once,
    // and each one running cargo build in parallel has triggered OOM-kills.
    // The daemon itself is responsible for keeping the binary current.
    if cli.command.is_none() && !cli.headless {
        update::check_and_upgrade_on_boot();
    }

    // Auto-start daemon for interactive/remote CLI modes (best-effort).
    // Explicitly skip for --headless: a headless CLI is already a descendant
    // of the daemon that spawned it, so ensure_daemon_running would just
    // tear down the live daemon and spawn a replacement, kicking off a
    // thrash loop.
    if cli.command.is_none() && !cli.offline && !cli.headless {
        if let Err(err) = maybe_auto_start_daemon(cli.server.clone()) {
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
            Commands::Register {
                path,
                project,
                new,
                yes,
            } => {
                let config = config::Config::load().unwrap_or_default();
                let Some(token) = cli.token.clone().or_else(|| config.remote.token.clone()) else {
                    eprintln!("\x1b[33m🔐 Not logged in.\x1b[0m");
                    eprintln!("   Run '\x1b[1mapas login\x1b[0m' to authenticate.");
                    return Ok(());
                };
                let server = cli
                    .server
                    .clone()
                    .or_else(|| config.remote.server.clone())
                    .unwrap_or_else(|| DEFAULT_SERVER.to_string());
                let dir = path
                    .or_else(|| cli.working_dir.clone().map(PathBuf::from))
                    .map_or_else(std::env::current_dir, Ok)?;
                let cwd = std::env::current_dir()?;
                let options = project_join::JoinOptions { project, new, yes };
                // Only someone at a terminal can answer the join question.
                let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin());
                // The lookup is async; the rest blocks on files, `git` and
                // the terminal. So all of it runs off the runtime, and waits
                // there for the lookup.
                let runtime = tokio::runtime::Handle::current();
                let folder = dir.clone();
                let outcome = tokio::task::spawn_blocking(move || {
                    let mut lookup = |query: &project_join::LookupQuery| {
                        let client = project_join::http_client()?;
                        runtime.block_on(project_join::lookup_by_remote(
                            &client, &server, &token, query,
                        ))
                    };
                    register_folder(
                        &folder,
                        &cwd,
                        &options,
                        interactive,
                        &mut lookup,
                        &mut project_join::confirm_on_terminal,
                    )
                })
                .await?;
                match outcome {
                    Ok(message) => eprintln!("{message}"),
                    Err(err) => {
                        eprintln!("\x1b[31mCould not register {}: {err:#}\x1b[0m", dir.display());
                        return Err(err);
                    }
                }
                // The resident instance is what reports registered projects to
                // the server, so registering without one shows nothing remotely.
                if let Err(err) = maybe_auto_start_daemon(cli.server.clone()) {
                    eprintln!(
                        "\x1b[33m   This machine's APAS instance could not be started ({err}); \
                         the project will appear once `apas daemon` is running.\x1b[0m"
                    );
                }
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
            Commands::SessionHook => {
                // Never fail the hook: a non-zero exit or stray output lands in
                // the user's terminal exactly as their agent is starting, and
                // the watcher's own derivation is still there as a fallback.
                let _ = claude_session_hook::run();
                return Ok(());
            }
            Commands::PaneHost { runtime_dir } => {
                let result = pane_host::run_host(runtime_dir);
                if let Err(error) = &result {
                    tracing::error!(%error, "pane-host failed");
                }
                return result;
            }
            Commands::JobHost { run_dir } => return jobs::host::run(run_dir).await,
            Commands::JobGatewayBridge { socket, token_file, listen } => {
                return jobs::gateway::run_bridge(socket, token_file, listen).await;
            }
            Commands::Daemon { roots } => {
                let state_path = config::Config::daemon_state_path()?;
                let legacy_pid_path = config::Config::daemon_pid_path()?;
                let my_pid = std::process::id();
                if let Some(existing) = detect_running_daemon(&state_path, &legacy_pid_path) {
                    // Skip if the detected PID is ourselves (auto-start wrote state before we ran)
                    if existing.pid != my_pid {
                        // An older instance used to be stopped and replaced
                        // here. It hosts this machine's projects now, and
                        // stopping it that way (SIGTERM, four seconds, SIGKILL)
                        // ends them without saving anything or bringing them
                        // back. Replacing it is the restart on the Machines
                        // page, which updates first and `exec`s in place.
                        let older = should_restart_for_version(
                            existing.version.as_deref(),
                            CURRENT_VERSION,
                        );
                        println!(
                            "Daemon already running (pid {}, version {}).",
                            existing.pid,
                            existing
                                .version
                                .clone()
                                .unwrap_or_else(|| "unknown".to_string())
                        );
                        if older {
                            println!(
                                "! It is older than this binary ({CURRENT_VERSION}) and is left running,"
                            );
                            println!("   because it is running this host's projects.");
                            println!("   Restart it from the Machines page to update it.");
                        }
                        return Ok(());
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

                // Derived per host unless explicitly configured; see
                // `resolved_machine_id` for why it is never minted and saved.
                let machine_id = config.daemon.resolved_machine_id();

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
            Commands::Worktree { action } => {
                let project_dir = cli
                    .working_dir
                    .clone()
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                match action {
                    WorktreeAction::Add {
                        pane_id,
                        branch,
                        path,
                    } => {
                        worktree::add(&project_dir, pane_id, branch, path)?;
                    }
                    WorktreeAction::Remove { pane_id } => {
                        worktree::remove(&project_dir, pane_id)?;
                    }
                    WorktreeAction::List => {
                        worktree::list(&project_dir)?;
                    }
                }
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
        // The machine the daemon on this host would report, so a standalone
        // run owns and reports its panes as that machine. It used to report
        // none at all.
        let machine_id = config.daemon.resolved_machine_id();
        // Replacing the process is this caller's decision, not the project's.
        // The project used to `exec` from inside itself, which reads the same
        // only while a process hosts exactly one project.
        match mode::dual_pane::run_headless(&server, &token, &working_dir, machine_id).await? {
            mode::dual_pane::ProjectOutcome::RebootRequested => {
                update::restart_cli();
                std::process::exit(1);
            }
            mode::dual_pane::ProjectOutcome::Stopped(reason) => {
                eprintln!("\n[APAS] {reason}\n");
                // Exit non-zero so whatever supervises this run surfaces it.
                std::process::exit(2);
            }
            mode::dual_pane::ProjectOutcome::Completed => {}
        }
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
        // Default: register this project with the host's resident instance and
        // exit. Neither this path nor `--attach` opens a server connection —
        // the resident instance owns that — so no server URL or token is
        // resolved here. The login check stays because an unauthenticated
        // machine has nothing that can manage what we would register.
        let config = config::Config::load()?;
        if cli.token.or(config.remote.token).is_none() {
            eprintln!("\x1b[33m🔐 Not logged in.\x1b[0m");
            eprintln!("   Run '\x1b[1mapas login\x1b[0m' to authenticate.");
            return Ok(());
        }

        if cli.attach {
            // The terminal UI, on request. Only reaches a project that is
            // already running here, because it renders a worker rather than
            // owning one.
            match attach_to_running_project(&working_dir) {
                AttachOutcome::Attached => return Ok(()),
                AttachOutcome::Refused(message) => {
                    eprintln!("\x1b[33m{message}\x1b[0m");
                    return Ok(());
                }
                AttachOutcome::NotRunning => {
                    eprintln!("\x1b[33mNo project is running here to attach to.\x1b[0m");
                    eprintln!("   Start it from {WEB_UI_URL}");
                    return Ok(());
                }
            }
        }

        // A host runs one instance per user. This one has already ensured the
        // resident instance exists, so its job is to make this project
        // manageable and get out of the way — not to become a second owner of
        // it. Nothing used to stop that: `is_headless_running_for` guards only
        // the daemon's own spawns, so `apas` in a directory the daemon was
        // already running produced two owners of one project.
        match register_and_defer(&working_dir) {
            Ok(message) => {
                eprintln!("{message}");
                return Ok(());
            }
            Err(err) => {
                // Registration is the whole point of the launch; failing it
                // silently would leave a project that never appears anywhere.
                eprintln!("\x1b[31mCould not register this project: {err}\x1b[0m");
                return Err(err);
            }
        }
    }

    Ok(())
}

fn maybe_auto_start_daemon(server_override: Option<String>) -> Result<()> {
    // Auto-start only for logged-in users with persistent config token
    // to avoid leaking one-off CLI tokens in process args.
    let config = config::Config::load().unwrap_or_default();
    if config.remote.token.is_none() {
        return Ok(());
    }

    let server = server_override
        .or(config.remote.server.clone())
        .unwrap_or_else(|| DEFAULT_SERVER.to_string());

    ensure_daemon_running(&server, &config.daemon.project_roots, CURRENT_VERSION)
}

/// Make this directory's project manageable, then get out of the way.
///
/// A host runs one instance per user, and by this point it exists — either it
/// was already running or `maybe_auto_start_daemon` just started it. So the
/// useful thing a launch can do is register the project, which is all the
/// resident instance needs: it reads the shared registry on every heartbeat
/// and reports what it finds to the server. No IPC, no start request.
///
/// Returns what to tell the user, which is mostly where to go next: projects
/// are started from the web now, not by being run in.
fn register_and_defer(working_dir: &std::path::Path) -> Result<String> {
    // Creates a missing `.apas`, but never regenerates one that does not
    // parse: on a shared home that would erase every machine's panes.
    let metadata = project_instances::open_project_metadata(working_dir)?;
    let name = metadata
        .name
        .clone()
        .unwrap_or_else(|| working_dir.display().to_string());
    Ok(format!(
        "\x1b[36m✓ {name} is registered on this machine.\x1b[0m\n   \
         Start and manage it at {WEB_UI_URL}\n   \
         This host runs one APAS instance per user; it is already running."
    ))
}

/// `apas register [path]`: the explicit form of what a bare launch does.
///
/// Unlike a launch it uses the strict registration behind the web's
/// **Existing folder** flow: the folder must exist, valid metadata keeps its
/// exact bytes and identity, malformed metadata is rejected rather than
/// regenerated, an id/path conflict fails instead of retargeting another
/// project, and a registry write failure is an error rather than a warning —
/// because registering is the whole of what this command is for.
fn register_directory(dir: &Path, cwd: &Path) -> Result<String> {
    let absolute = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        cwd.join(dir)
    };
    let input = absolute
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Folder path is not valid UTF-8"))?;
    let (registered, _) = project::register_local_project(input)?;
    let name = registered.name.as_deref().unwrap_or(&registered.path);
    Ok(format!(
        "\x1b[36m✓ {name} is registered on this machine.\x1b[0m\n   \
         Folder:  {path}\n   \
         Project: {id}\n   \
         It is listed as Stopped until you start it from {WEB_UI_URL}",
        path = registered.path,
        id = registered.project_id,
    ))
}

/// `apas register [path]` with joining: register the folder, or give it the
/// identity of one of the user's projects that uses the same repository, so
/// it becomes another machine of that project instead of a new project.
///
/// `lookup` asks the server which projects use the folder's remote, and
/// `confirm` asks the person at the terminal; see `project_join::decide` for
/// when each is asked. `--new` asks neither and registers exactly as
/// [`register_directory`] does.
fn register_folder(
    dir: &Path,
    cwd: &Path,
    options: &project_join::JoinOptions,
    interactive: bool,
    lookup: &mut dyn FnMut(&project_join::LookupQuery) -> Result<shared::ProjectsByRemoteResponse>,
    confirm: &mut dyn FnMut(&str) -> bool,
) -> Result<String> {
    let absolute = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        cwd.join(dir)
    };
    if options.new {
        return register_directory(&absolute, cwd);
    }
    let folder = project::inspect_folder(&absolute)?;
    let remote = worktree::normalized_git_remote(&folder.dir);
    match project_join::decide(
        &folder,
        remote.as_deref(),
        options,
        interactive,
        lookup,
        confirm,
    )? {
        project_join::Decision::AsIs { notes } => {
            let registered = register_directory(&folder.dir, cwd)?;
            let mut message: String = notes
                .iter()
                .map(|note| format!("\x1b[33m! {note}\x1b[0m\n"))
                .collect();
            message.push_str(&registered);
            Ok(message)
        }
        project_join::Decision::Join { project } => {
            let joined = project::join_project(&folder, project.project_id, project.name.clone())?;
            let folder_name = folder
                .dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&joined.path);
            let project_name = project
                .name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .map_or_else(|| format!("project {}", joined.project_id), String::from);
            Ok(format!(
                "\x1b[36m✓ {folder_name} joined {project_name} on this machine.\x1b[0m\n   \
                 Folder:  {path}\n   \
                 Project: {id}\n   \
                 Start it here, or add a tab on this machine, from {WEB_UI_URL}",
                path = joined.path,
                id = joined.project_id,
            ))
        }
    }
}

enum AttachOutcome {
    /// Rendered the running project and returned when the user left.
    Attached,
    /// The project is running here but cannot be attached to, so this process
    /// must not start a second one. Carries what to tell the user.
    Refused(String),
    /// Nothing is running for this project here; start it normally.
    NotRunning,
}

/// Attach to a worker already running this project on this host.
///
/// A worker that predates attachment support has no socket. Refusing is the
/// deliberate choice over falling back to today's behaviour: the fallback
/// would keep the duplicate-CLI bug alive for exactly the projects that
/// predate the change, which at first is all of them.
fn attach_to_running_project(working_dir: &std::path::Path) -> AttachOutcome {
    let Some(project_id) = project::read_project_id(working_dir) else {
        return AttachOutcome::NotRunning;
    };
    let Ok((socket, credential_path)) = supervisor::worker_paths(project_id) else {
        return AttachOutcome::NotRunning;
    };
    if !socket.exists() {
        // Running here, but from before attachment existed. Refuse rather than
        // fall back: falling back would start a second CLI over the same
        // `.apas` and worktrees, which is the bug this exists to close, and it
        // would do so for exactly the projects that predate the change.
        if mode::daemon::is_headless_running_for(working_dir) {
            return AttachOutcome::Refused(format!(
                "This project is already running on this host, but that worker predates \
                 attachment and cannot be attached to.\n   \
                 Restart it to get an attachable worker: apas project stop, then apas.\n   \
                 Refusing to start a second CLI over {}",
                working_dir.display()
            ));
        }
        return AttachOutcome::NotRunning;
    }
    let Ok(credential) = std::fs::read_to_string(&credential_path) else {
        return AttachOutcome::NotRunning;
    };
    match attach::Attachment::connect(&socket, credential.trim()) {
        Ok((attachment, tabs)) => {
            eprintln!("\x1b[36m📎 Attached to the running project on this host.\x1b[0m");
            eprintln!("   Closing this window leaves it running.");
            if let Err(err) =
                attach::run_attached_tui(attachment, tabs, working_dir.display().to_string())
            {
                tracing::error!(%err, "attached session ended with an error");
            }
            AttachOutcome::Attached
        }
        Err(err) => {
            // The socket exists but did not accept us. A stale file from a
            // crashed worker is the common case, and it names nothing.
            tracing::debug!(%err, "no live worker behind the attach socket");
            AttachOutcome::NotRunning
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum LaunchDaemonPlan {
    /// Nothing is running here; this launch starts the instance.
    Start,
    /// One is already running. `older` only decides what to say — never
    /// whether to act.
    LeaveRunning { older: bool },
}

/// What a launch does about the instance already on this host.
///
/// Separated from the spawning so the decision is testable, the way
/// `plan_daemon_restart` is. The decision itself is now trivial, and that is
/// the point: the branch that used to stop a running daemon for being older is
/// gone, because it hosts this machine's projects.
fn plan_launch_daemon(running: Option<&RunningDaemon>, target_version: &str) -> LaunchDaemonPlan {
    match running {
        None => LaunchDaemonPlan::Start,
        Some(running) => LaunchDaemonPlan::LeaveRunning {
            older: should_restart_for_version(running.version.as_deref(), target_version),
        },
    }
}

fn ensure_daemon_running(server: &str, roots: &[String], target_version: &str) -> Result<()> {
    let state_path = config::Config::daemon_state_path()?;
    let legacy_pid_path = config::Config::daemon_pid_path()?;

    let running = detect_running_daemon(&state_path, &legacy_pid_path);
    // A launch used to stop an older daemon and start a new one here, back when
    // the daemon owned nothing and replacing it was invisible. It now runs the
    // projects, so that would kill whatever is running on the host — and via
    // SIGTERM/SIGKILL rather than the `exec` a real restart uses, so nothing
    // would be saved and nothing would come back. Whoever types `apas` in a
    // directory is a bystander to that work, not its owner.
    if let LaunchDaemonPlan::LeaveRunning { older } =
        plan_launch_daemon(running.as_ref(), target_version)
    {
        if older {
            let version = running
                .as_ref()
                .and_then(|running| running.version.as_deref())
                .unwrap_or("unknown version");
            println!(
                "! This machine's APAS instance is older ({version}) than this binary ({target_version})."
            );
            println!("   It is left running, because it is running this host's projects.");
            println!("   Update it from the Machines page: restart it there and it updates first.");
        }
        return Ok(());
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
    detect_running_daemon_with_process_check(state_path, legacy_pid_path, is_apas_daemon_process)
}

fn detect_running_daemon_with_process_check(
    state_path: &Path,
    legacy_pid_path: &Path,
    is_daemon_process: impl Fn(u32) -> bool,
) -> Option<RunningDaemon> {
    if let Some(state) = read_daemon_state(state_path) {
        if is_daemon_process(state.pid) {
            return Some(RunningDaemon {
                pid: state.pid,
                version: Some(state.version),
            });
        }
        let _ = fs::remove_file(state_path);
    }

    let legacy_pid = read_legacy_daemon_pid(legacy_pid_path)?;
    if is_daemon_process(legacy_pid) {
        return Some(RunningDaemon {
            pid: legacy_pid,
            version: None,
        });
    }
    let _ = fs::remove_file(legacy_pid_path);
    None
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

const CONFIG_KEYS: &str = "server, token, claude_path, codex_path, opencode_path, pi_path, omp_path, cursor_agent_path, deepseek_api_base_url, deepseek_api_key, pane_host_adoption_grace_seconds, pane_host_reboot_grace_seconds, daemon_machine_id, daemon_roots, summary_enabled, summary_adapter, summary_model, summary_timeout_seconds, summary_max_input_bytes, summary_allow_cross_provider";

fn set_config_value(config: &mut config::Config, key: &str, value: String) -> Result<()> {
    match key {
        "server" => config.remote.server = Some(value),
        "token" => config.remote.token = Some(value),
        "claude_path" => config.local.claude_path = value,
        "codex_path" => config.local.codex_path = value,
        "opencode_path" => config.local.opencode_path = value,
        "pi_path" => config.local.pi_path = value,
        "omp_path" => config.local.omp_path = value,
        "cursor_agent_path" => config.local.cursor_agent_path = value,
        "deepseek_api_base_url" => {
            config.local.deepseek_api_base_url = if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        }
        "deepseek_api_key" => {
            config.local.deepseek_api_key = if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        }
        "pane_host_adoption_grace_seconds" => {
            let seconds: u64 = value.parse()?;
            anyhow::ensure!(
                (30..=60 * 60).contains(&seconds),
                "pane_host_adoption_grace_seconds must be between 30 and 3600"
            );
            config.local.pane_host_adoption_grace_seconds = seconds;
        }
        "pane_host_reboot_grace_seconds" => {
            let seconds: u64 = value.parse()?;
            anyhow::ensure!(
                (60..=2 * 60 * 60).contains(&seconds),
                "pane_host_reboot_grace_seconds must be between 60 and 7200"
            );
            config.local.pane_host_reboot_grace_seconds = seconds;
        }
        "daemon_machine_id" => config.daemon.machine_id = Some(value),
        "daemon_roots" => {
            config.daemon.project_roots = value
                .split(',')
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .collect();
        }
        "summary_enabled" => config.summaries.enabled = parse_bool_config(&key, &value)?,
        "summary_adapter" => {
            config.summaries.adapter = match value.trim().to_ascii_lowercase().as_str() {
                "disabled" | "off" | "none" => config::SummaryAdapterKind::Disabled,
                "claude" => config::SummaryAdapterKind::Claude,
                "codex" => config::SummaryAdapterKind::Codex,
                _ => anyhow::bail!("summary_adapter must be disabled, claude, or codex"),
            }
        }
        "summary_model" => config.summaries.model = (!value.trim().is_empty()).then_some(value),
        "summary_timeout_seconds" => config.summaries.timeout_seconds = value.parse()?,
        "summary_max_input_bytes" => config.summaries.max_input_bytes = value.parse()?,
        "summary_allow_cross_provider" => {
            config.summaries.allow_cross_provider = parse_bool_config(&key, &value)?
        }
        _ => anyhow::bail!("Unknown config key: {}. Valid keys: {}", key, CONFIG_KEYS),
    }
    Ok(())
}

fn parse_bool_config(key: &str, value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => anyhow::bail!("{} must be true or false", key),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        detect_running_daemon_with_process_check, get_config_value, plan_launch_daemon,
        read_daemon_state, read_legacy_daemon_pid, set_config_value, should_restart_for_version,
        write_daemon_state, DaemonStateFile, DaemonStateGuard,
    };
    use crate::config;
    use std::fs;

    #[test]
    fn deepseek_config_get_masks_api_key() {
        let mut config = config::Config::default();
        set_config_value(
            &mut config,
            "deepseek_api_key",
            "sk-deepseek-secret".to_string(),
        )
        .unwrap();

        assert_eq!(
            config.local.deepseek_api_key.as_deref(),
            Some("sk-deepseek-secret")
        );
        assert_eq!(
            get_config_value(&config, "deepseek_api_key").unwrap(),
            "****"
        );
    }

    #[test]
    fn deepseek_config_set_blank_clears_api_key() {
        let mut config = config::Config::default();
        config.local.deepseek_api_key = Some("existing".to_string());

        set_config_value(&mut config, "deepseek_api_key", "   ".to_string()).unwrap();

        assert_eq!(config.local.deepseek_api_key, None);
        assert_eq!(get_config_value(&config, "deepseek_api_key").unwrap(), "");
    }

    #[test]
    fn deepseek_config_get_returns_api_base_url() {
        let mut config = config::Config::default();
        set_config_value(
            &mut config,
            "deepseek_api_base_url",
            "https://api.deepseek.com/anthropic".to_string(),
        )
        .unwrap();

        assert_eq!(
            get_config_value(&config, "deepseek_api_base_url").unwrap(),
            "https://api.deepseek.com/anthropic"
        );
    }

    #[test]
    fn should_restart_for_version_restarts_when_daemon_version_is_missing() {
        assert!(should_restart_for_version(None, "26.06.42"));
    }

    #[test]
    fn should_restart_for_version_restarts_when_daemon_is_older() {
        assert!(should_restart_for_version(Some("26.06.41"), "26.06.42"));
        assert!(should_restart_for_version(Some("26.05.999"), "26.06.1"));
    }

    #[test]
    fn should_restart_for_version_keeps_equal_or_newer_daemon() {
        assert!(!should_restart_for_version(Some("26.06.42"), "26.06.42"));
        assert!(!should_restart_for_version(Some("26.06.43"), "26.06.42"));
        assert!(!should_restart_for_version(Some("26.07.1"), "26.06.999"));
    }

    #[test]
    fn should_restart_for_version_is_conservative_for_unparsable_versions() {
        assert!(!should_restart_for_version(Some("dev"), "26.06.42"));
        assert!(!should_restart_for_version(Some("26.06.42"), "dev"));
        assert!(!should_restart_for_version(Some("26.06"), "26.06.42"));
        assert!(!should_restart_for_version(Some("26.06.42"), "26.06"));
    }

    #[test]
    fn main_daemon_state_guard_removes_matching_state_on_drop() {
        let dir = tempfile::tempdir().expect("temp dir");
        let state_path = dir.path().join("daemon.json");
        write_daemon_state(
            &state_path,
            &DaemonStateFile {
                pid: 42,
                version: "26.06.7".to_string(),
            },
        )
        .expect("write daemon state");

        {
            let _guard = DaemonStateGuard::new(state_path.clone(), 42);
        }

        assert!(!state_path.exists());
    }

    #[test]
    fn main_daemon_state_guard_keeps_state_for_different_pid() {
        let dir = tempfile::tempdir().expect("temp dir");
        let state_path = dir.path().join("daemon.json");
        write_daemon_state(
            &state_path,
            &DaemonStateFile {
                pid: 99,
                version: "26.06.7".to_string(),
            },
        )
        .expect("write daemon state");

        {
            let _guard = DaemonStateGuard::new(state_path.clone(), 42);
        }

        let state = read_daemon_state(&state_path).expect("daemon state remains");
        assert_eq!(state.pid, 99);
        assert_eq!(state.version, "26.06.7");
    }

    #[test]
    fn main_read_legacy_daemon_pid_parses_valid_and_rejects_invalid_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let pid_path = dir.path().join("daemon.pid");

        fs::write(&pid_path, "12345\n").expect("write valid pid");
        assert_eq!(read_legacy_daemon_pid(&pid_path), Some(12345));

        fs::write(&pid_path, "not-a-pid\n").expect("write invalid pid");
        assert_eq!(read_legacy_daemon_pid(&pid_path), None);
        assert_eq!(
            read_legacy_daemon_pid(&dir.path().join("missing.pid")),
            None
        );
    }

    /// The daemon hosts this machine's projects, so a launch that found an
    /// older one used to end every project on the host — by SIGTERM then
    /// SIGKILL, with no teardown, no saved pane roster, and no resume manifest.
    #[test]
    fn a_launch_leaves_a_running_instance_alone_however_old_it_is() {
        use super::{LaunchDaemonPlan, RunningDaemon};

        let running = |version: Option<&str>| RunningDaemon {
            pid: 4242,
            version: version.map(str::to_string),
        };

        // Older, and by a lot. Still left running.
        assert_eq!(
            plan_launch_daemon(Some(&running(Some("26.06.1"))), "26.08.77"),
            LaunchDaemonPlan::LeaveRunning { older: true }
        );
        // An unknown version used to count as "restart it".
        assert_eq!(
            plan_launch_daemon(Some(&running(None)), "26.08.77"),
            LaunchDaemonPlan::LeaveRunning { older: true }
        );
        // Same and newer are left running too, and say nothing about updating.
        assert_eq!(
            plan_launch_daemon(Some(&running(Some("26.08.77"))), "26.08.77"),
            LaunchDaemonPlan::LeaveRunning { older: false }
        );
        assert_eq!(
            plan_launch_daemon(Some(&running(Some("26.09.1"))), "26.08.77"),
            LaunchDaemonPlan::LeaveRunning { older: false }
        );
    }

    #[test]
    fn a_launch_still_starts_an_instance_when_none_is_running() {
        // The rule is about not replacing a running instance, not about
        // refusing to start one.
        assert_eq!(
            plan_launch_daemon(None, "26.08.77"),
            super::LaunchDaemonPlan::Start
        );
    }

    #[test]
    fn main_detect_running_daemon_prefers_live_state_over_legacy_pid() {
        let dir = tempfile::tempdir().expect("temp dir");
        let state_path = dir.path().join("daemon.json");
        let legacy_pid_path = dir.path().join("daemon.pid");
        write_daemon_state(
            &state_path,
            &DaemonStateFile {
                pid: 42,
                version: "26.06.7".to_string(),
            },
        )
        .expect("write daemon state");
        fs::write(&legacy_pid_path, "99\n").expect("write legacy pid");

        let running =
            detect_running_daemon_with_process_check(&state_path, &legacy_pid_path, |_| true)
                .expect("detect live daemon");

        assert_eq!(running.pid, 42);
        assert_eq!(running.version.as_deref(), Some("26.06.7"));
        assert!(state_path.exists());
        assert!(legacy_pid_path.exists());
    }

    #[test]
    fn main_detect_running_daemon_removes_stale_state_and_falls_back_to_legacy_pid() {
        let dir = tempfile::tempdir().expect("temp dir");
        let state_path = dir.path().join("daemon.json");
        let legacy_pid_path = dir.path().join("daemon.pid");
        write_daemon_state(
            &state_path,
            &DaemonStateFile {
                pid: 42,
                version: "26.06.7".to_string(),
            },
        )
        .expect("write daemon state");
        fs::write(&legacy_pid_path, "99\n").expect("write legacy pid");

        let running =
            detect_running_daemon_with_process_check(&state_path, &legacy_pid_path, |pid| {
                pid == 99
            })
            .expect("detect legacy daemon");

        assert_eq!(running.pid, 99);
        assert_eq!(running.version, None);
        assert!(!state_path.exists());
        assert!(legacy_pid_path.exists());
    }

    #[test]
    fn a_deferring_launch_registers_the_project_and_starts_nothing() {
        crate::project::test_support::with_isolated_config(|| {
            let dir = tempfile::tempdir().expect("temp dir");
            let project_dir = dir.path().join("work");
            fs::create_dir_all(&project_dir).expect("project dir");

            let message = super::register_and_defer(&project_dir).expect("register");

            // Registered where the resident instance will find it: it reads
            // this registry on every heartbeat, which is why no IPC is needed.
            let registered = crate::project::list_registered_projects().expect("registry");
            assert_eq!(registered.len(), 1);
            assert!(message.contains("registered on this machine"));
            // Projects are started from the web now, so the launch says where.
            assert!(message.contains(super::WEB_UI_URL));
        });
    }

    #[test]
    fn running_it_twice_in_one_directory_registers_once() {
        crate::project::test_support::with_isolated_config(|| {
            let dir = tempfile::tempdir().expect("temp dir");
            let project_dir = dir.path().join("work");
            fs::create_dir_all(&project_dir).expect("project dir");

            super::register_and_defer(&project_dir).expect("first");
            let first = crate::project::read_project_id(&project_dir).expect("id");
            super::register_and_defer(&project_dir).expect("second");
            let second = crate::project::read_project_id(&project_dir).expect("id");

            // The second launch is not a new project: same id, one entry.
            assert_eq!(first, second);
            assert_eq!(
                crate::project::list_registered_projects()
                    .expect("registry")
                    .len(),
                1
            );
        });
    }

    #[test]
    fn a_directory_that_is_not_yet_a_project_becomes_one() {
        // This change governs how many instances run, not when a project comes
        // into being: onboarding a local directory stays a thing a launch does,
        // because the web's create flow only clones into a new directory.
        crate::project::test_support::with_isolated_config(|| {
            let dir = tempfile::tempdir().expect("temp dir");
            let fresh = dir.path().join("fresh");
            fs::create_dir_all(&fresh).expect("dir");
            assert!(crate::project::read_project_id(&fresh).is_none());

            super::register_and_defer(&fresh).expect("register");

            assert!(crate::project::read_project_id(&fresh).is_some());
        });
    }

    #[test]
    fn a_launch_never_regenerates_metadata_it_cannot_parse() {
        // Regenerating would start an empty project under a new identity, and
        // on a shared home erase the panes every other machine saved there.
        crate::project::test_support::with_isolated_config(|| {
            let dir = tempfile::tempdir().expect("temp dir");
            let metadata = dir.path().join(".apas");
            let corrupt = b"{\"id\": \"not finished".to_vec();
            fs::write(&metadata, &corrupt).expect("write");

            assert!(super::register_and_defer(dir.path()).is_err());

            assert_eq!(fs::read(&metadata).expect("read"), corrupt);
            assert!(crate::project::list_registered_projects()
                .expect("registry")
                .is_empty());
        });
    }

    #[test]
    fn register_subcommand_takes_an_optional_folder() {
        use clap::Parser;
        let bare = super::Cli::try_parse_from(["apas", "register"]).expect("parse bare");
        assert!(matches!(
            bare.command,
            Some(super::Commands::Register {
                path: None,
                project: None,
                new: false,
                yes: false,
            })
        ));
        let with_path =
            super::Cli::try_parse_from(["apas", "register", "work"]).expect("parse path");
        match with_path.command {
            Some(super::Commands::Register {
                path: Some(path), ..
            }) => {
                assert_eq!(path, std::path::PathBuf::from("work"))
            }
            _ => panic!("expected register with a path"),
        }
    }

    #[test]
    fn register_takes_the_join_choices() {
        use clap::Parser;
        let id = uuid::Uuid::new_v4();
        let parsed = super::Cli::try_parse_from([
            "apas",
            "register",
            "work",
            "--project",
            &id.to_string(),
            "--yes",
        ])
        .expect("parse");
        match parsed.command {
            Some(super::Commands::Register {
                path: Some(_),
                project: Some(project),
                new: false,
                yes: true,
            }) => assert_eq!(project, id),
            _ => panic!("expected register with --project and --yes"),
        }
        assert!(matches!(
            super::Cli::try_parse_from(["apas", "register", "--new", "-y"])
                .expect("parse")
                .command,
            Some(super::Commands::Register {
                new: true,
                yes: true,
                ..
            })
        ));
        // Joining one project and keeping the folder separate contradict.
        assert!(super::Cli::try_parse_from([
            "apas",
            "register",
            "--new",
            "--project",
            &id.to_string(),
        ])
        .is_err());
        assert!(
            super::Cli::try_parse_from(["apas", "register", "--project", "not-a-uuid"]).is_err()
        );
    }

    #[test]
    fn register_resolves_a_relative_folder_and_keeps_its_identity() {
        crate::project::test_support::with_isolated_config(|| {
            let dir = tempfile::tempdir().expect("temp dir");
            let project_dir = dir.path().join("work");
            fs::create_dir_all(&project_dir).expect("project dir");

            let message = super::register_directory(std::path::Path::new("work"), dir.path())
                .expect("register");
            let id = crate::project::read_project_id(&project_dir).expect("id");
            assert!(message.contains(&id.to_string()));
            assert!(message.contains(super::WEB_UI_URL));

            let canonical = fs::canonicalize(&project_dir).expect("canonical");
            let registered = crate::project::list_registered_projects().expect("registry");
            assert_eq!(registered.len(), 1);
            assert_eq!(registered[0].path, canonical.to_str().expect("utf-8"));

            // Registering again, by absolute path this time, is the same project.
            super::register_directory(&project_dir, std::path::Path::new("/"))
                .expect("register again");
            assert_eq!(crate::project::read_project_id(&project_dir), Some(id));
            assert_eq!(
                crate::project::list_registered_projects()
                    .expect("registry")
                    .len(),
                1
            );
        });
    }

    #[test]
    fn register_rejects_malformed_metadata_instead_of_regenerating_it() {
        // A bare launch deletes a corrupt `.apas` and mints a new identity;
        // the explicit command must not, since that silently forks a project.
        crate::project::test_support::with_isolated_config(|| {
            let dir = tempfile::tempdir().expect("temp dir");
            let metadata = dir.path().join(".apas");
            fs::write(&metadata, b"{not json").expect("write");

            assert!(super::register_directory(dir.path(), dir.path()).is_err());
            assert_eq!(fs::read(&metadata).expect("read"), b"{not json");
            assert!(crate::project::list_registered_projects()
                .expect("registry")
                .is_empty());
        });
    }

    #[test]
    fn register_rejects_a_missing_folder() {
        crate::project::test_support::with_isolated_config(|| {
            let dir = tempfile::tempdir().expect("temp dir");
            let missing = dir.path().join("missing");
            assert!(super::register_directory(&missing, dir.path()).is_err());
            assert!(!missing.exists());
        });
    }

    // --- `apas register` joining an existing project by git remote --------

    use crate::project_join::{JoinOptions, LookupQuery};
    use shared::{ProjectRemoteInstance, ProjectRemoteMatch, ProjectsByRemoteResponse};
    use std::path::Path;
    use uuid::Uuid;

    /// What `normalized_git_remote` makes of the checkouts below.
    const REMOTE: &str = "github.com/owner/repo";

    /// A git checkout at `dir` whose `origin` is the repository [`REMOTE`]
    /// names, made without the user's git configuration.
    fn checkout(dir: &Path) {
        fs::create_dir_all(dir).expect("checkout dir");
        for args in [
            &["init", "-q"][..],
            &["remote", "add", "origin", "git@github.com:Owner/Repo.git"][..],
        ] {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .expect("run git");
            assert!(status.success(), "git {args:?}");
        }
    }

    /// One of the user's projects as the server lists it, checked out on
    /// zoo-005.
    fn remote_match(project_id: Uuid, name: &str, has_session: bool) -> ProjectRemoteMatch {
        ProjectRemoteMatch {
            project_id,
            name: Some(name.to_string()),
            has_session,
            instances: vec![ProjectRemoteInstance {
                machine_id: Some(Uuid::new_v4()),
                hostname: "zoo-005".to_string(),
                path: format!("/home/u/{name}"),
                running: true,
            }],
        }
    }

    fn by_remote(
        projects: Vec<ProjectRemoteMatch>,
        local: Option<ProjectRemoteMatch>,
    ) -> Option<ProjectsByRemoteResponse> {
        Some(ProjectsByRemoteResponse {
            git_remote: REMOTE.to_string(),
            projects,
            local,
        })
    }

    /// What one `apas register` run asked the server and the terminal.
    struct Asked {
        lookups: Vec<LookupQuery>,
        prompts: Vec<String>,
    }

    /// Run `apas register` in `dir`. The server answers `answer`, or is
    /// unreachable for `None`; the person at the terminal, when there is one,
    /// answers `confirm`.
    fn register(
        dir: &Path,
        options: JoinOptions,
        interactive: bool,
        answer: Option<ProjectsByRemoteResponse>,
        confirm: bool,
    ) -> (anyhow::Result<String>, Asked) {
        let mut lookups = Vec::new();
        let mut prompts = Vec::new();
        let result = super::register_folder(
            dir,
            Path::new("/"),
            &options,
            interactive,
            &mut |query: &LookupQuery| {
                lookups.push(query.clone());
                answer
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("connection refused"))
            },
            &mut |prompt: &str| {
                prompts.push(prompt.to_string());
                confirm
            },
        );
        (result, Asked { lookups, prompts })
    }

    /// What a plain `apas register` leaves in a folder: a pristine identity.
    fn pristine_identity(dir: &Path) -> Uuid {
        super::register_directory(dir, dir).expect("register");
        crate::project::read_project_id(dir).expect("identity")
    }

    fn registry() -> Vec<(String, String)> {
        crate::project::list_registered_projects()
            .expect("registry")
            .into_iter()
            .map(|entry| (entry.project_id, entry.path))
            .collect()
    }

    fn canonical(dir: &Path) -> String {
        fs::canonicalize(dir)
            .expect("canonical")
            .to_str()
            .expect("utf-8")
            .to_string()
    }

    fn join(project: Uuid) -> JoinOptions {
        JoinOptions {
            project: Some(project),
            ..JoinOptions::default()
        }
    }

    #[test]
    fn register_offers_the_one_project_of_the_same_repository_and_joins_it() {
        crate::project::test_support::with_isolated_config(|| {
            let home = tempfile::tempdir().expect("temp dir");
            let dir = home.path().join("chiral-laptop");
            checkout(&dir);
            let own = pristine_identity(&dir);
            let zoo = Uuid::new_v4();

            // The server lists the folder's own identity too, since its remote
            // matches; it is never offered.
            let (result, asked) = register(
                &dir,
                JoinOptions::default(),
                true,
                by_remote(
                    vec![
                        remote_match(own, "chiral-laptop", false),
                        remote_match(zoo, "chiral-network", true),
                    ],
                    Some(remote_match(own, "chiral-laptop", false)),
                ),
                true,
            );
            let message = result.expect("joined");

            assert_eq!(
                asked.lookups,
                vec![LookupQuery {
                    git_remote: REMOTE.to_string(),
                    project_id: Some(own),
                }]
            );
            assert_eq!(
                asked.prompts,
                vec![format!(
                    "Join chiral-network ({zoo}, on zoo-005:/home/u/chiral-network)? [Y/n]"
                )]
            );
            assert!(message.contains("joined chiral-network"), "{message}");
            let metadata = crate::project_instances::read_project_metadata(&dir).expect("read");
            assert_eq!(metadata.id, zoo);
            assert!(metadata.panes.is_empty());
            assert_eq!(metadata.name.as_deref(), Some("chiral-network"));
            assert_eq!(registry(), vec![(zoo.to_string(), canonical(&dir))]);
        });
    }

    #[test]
    fn declining_the_join_keeps_the_folders_own_identity() {
        crate::project::test_support::with_isolated_config(|| {
            let home = tempfile::tempdir().expect("temp dir");
            let dir = home.path().join("repo");
            checkout(&dir);
            let own = pristine_identity(&dir);

            let (result, asked) = register(
                &dir,
                JoinOptions::default(),
                true,
                by_remote(vec![remote_match(Uuid::new_v4(), "repo", true)], None),
                false,
            );
            let message = result.expect("registered");
            assert_eq!(asked.prompts.len(), 1);
            assert!(message.contains(&own.to_string()), "{message}");
            assert_eq!(crate::project::read_project_id(&dir), Some(own));
            assert_eq!(registry(), vec![(own.to_string(), canonical(&dir))]);
        });
    }

    #[test]
    fn a_folder_no_project_matches_is_registered_as_it_is() {
        crate::project::test_support::with_isolated_config(|| {
            let home = tempfile::tempdir().expect("temp dir");
            let dir = home.path().join("repo");
            checkout(&dir);

            let (result, asked) = register(
                &dir,
                JoinOptions::default(),
                true,
                by_remote(Vec::new(), None),
                true,
            );
            result.expect("registered");
            // A folder with no identity yet has none to ask about.
            assert_eq!(
                asked.lookups,
                vec![LookupQuery {
                    git_remote: REMOTE.to_string(),
                    project_id: None,
                }]
            );
            assert!(asked.prompts.is_empty());
            let id = crate::project::read_project_id(&dir).expect("a new identity");
            assert_eq!(registry(), vec![(id.to_string(), canonical(&dir))]);
        });
    }

    #[test]
    fn several_matching_projects_need_an_explicit_choice() {
        crate::project::test_support::with_isolated_config(|| {
            let home = tempfile::tempdir().expect("temp dir");
            let dir = home.path().join("repo");
            checkout(&dir);
            let (first, second) = (Uuid::new_v4(), Uuid::new_v4());
            let answer = by_remote(
                vec![
                    remote_match(first, "repo", true),
                    remote_match(second, "repo-2", false),
                ],
                None,
            );

            for options in [
                JoinOptions::default(),
                JoinOptions {
                    yes: true,
                    ..JoinOptions::default()
                },
            ] {
                let (result, asked) = register(&dir, options, true, answer.clone(), true);
                let error = format!("{:#}", result.expect_err("a choice is needed"));
                assert!(error.contains(&first.to_string()), "{error}");
                assert!(error.contains(&second.to_string()), "{error}");
                assert!(error.contains("--project"), "{error}");
                assert!(asked.prompts.is_empty());
                assert!(!dir.join(".apas").exists(), "nothing registered meanwhile");
                assert!(registry().is_empty());
            }

            let (result, asked) = register(&dir, join(second), false, answer, false);
            result.expect("joined");
            assert!(asked.prompts.is_empty(), "--project does not ask");
            assert_eq!(crate::project::read_project_id(&dir), Some(second));
            assert_eq!(registry(), vec![(second.to_string(), canonical(&dir))]);
        });
    }

    #[test]
    fn a_folder_whose_project_has_tabs_keeps_its_identity() {
        crate::project::test_support::with_isolated_config(|| {
            let home = tempfile::tempdir().expect("temp dir");
            let zoo = Uuid::new_v4();
            let own = Uuid::new_v4();
            // Tabs in `panes`, and a file from before `panes` whose legacy
            // sessions are history all the same.
            let with_tabs = serde_json::json!({
                "id": own, "name": "repo", "created_at": "2026-01-01T00:00:00Z",
                "panes": [{
                    "pane_id": 3, "provider": "codex", "mode": "interactive",
                    "kind": "terminal", "session_id": Uuid::new_v4(),
                }],
            });
            let legacy = serde_json::json!({
                "id": own, "name": "repo", "created_at": "2026-01-01T00:00:00Z",
                "deadloop_claude_session_id": Uuid::new_v4(),
            });
            for metadata in [with_tabs, legacy] {
                let dir = home.path().join(format!("repo-{}", Uuid::new_v4()));
                checkout(&dir);
                let bytes = serde_json::to_vec_pretty(&metadata).expect("json");
                fs::write(dir.join(".apas"), &bytes).expect("write");
                let answer = by_remote(vec![remote_match(zoo, "repo", true)], None);

                let (result, asked) = register(&dir, join(zoo), true, answer.clone(), true);
                let error = format!("{:#}", result.expect_err("refused"));
                assert!(error.contains("tab"), "{error}");
                assert_eq!(fs::read(dir.join(".apas")).expect("read"), bytes);

                let (result, asked_again) =
                    register(&dir, JoinOptions::default(), true, answer, true);
                let message = result.expect("registered");
                assert!(message.contains(&own.to_string()), "{message}");
                assert!(
                    asked.lookups.is_empty() && asked_again.lookups.is_empty(),
                    "tabs settle it without asking the server"
                );
                assert!(asked_again.prompts.is_empty());
                assert_eq!(fs::read(dir.join(".apas")).expect("read"), bytes);
                assert_eq!(registry(), vec![(own.to_string(), canonical(&dir))]);
                // One folder per project in the registry: clear it for the next.
                crate::project::unregister_project(&dir, own).expect("unregister");
            }
        });
    }

    #[test]
    fn a_folder_whose_project_has_history_on_the_server_keeps_its_identity() {
        crate::project::test_support::with_isolated_config(|| {
            let home = tempfile::tempdir().expect("temp dir");
            let dir = home.path().join("repo");
            checkout(&dir);
            let own = pristine_identity(&dir);
            let zoo = Uuid::new_v4();

            // No tabs left, but a session. Listed among the matches, or only
            // as `local` because the remote its sessions recorded differs.
            for answer in [
                by_remote(
                    vec![
                        remote_match(own, "repo", true),
                        remote_match(zoo, "repo", true),
                    ],
                    Some(remote_match(own, "repo", true)),
                ),
                by_remote(
                    vec![remote_match(zoo, "repo", true)],
                    Some(ProjectRemoteMatch {
                        project_id: own,
                        name: Some("repo".to_string()),
                        has_session: true,
                        instances: Vec::new(),
                    }),
                ),
            ] {
                let (result, asked) =
                    register(&dir, JoinOptions::default(), true, answer.clone(), true);
                let message = result.expect("registered");
                assert!(asked.prompts.is_empty(), "no join is offered");
                assert!(message.contains("has history"), "{message}");

                let (result, _) = register(&dir, join(zoo), true, answer, true);
                let error = format!("{:#}", result.expect_err("refused"));
                assert!(error.contains("history"), "{error}");

                assert_eq!(crate::project::read_project_id(&dir), Some(own));
                assert_eq!(registry(), vec![(own.to_string(), canonical(&dir))]);
            }
        });
    }

    #[test]
    fn a_run_nobody_can_answer_joins_only_when_told_to() {
        crate::project::test_support::with_isolated_config(|| {
            let home = tempfile::tempdir().expect("temp dir");
            let dir = home.path().join("repo");
            checkout(&dir);
            let own = pristine_identity(&dir);
            let zoo = Uuid::new_v4();
            let answer = by_remote(vec![remote_match(zoo, "repo", true)], None);

            let (result, asked) =
                register(&dir, JoinOptions::default(), false, answer.clone(), true);
            let message = result.expect("registered");
            assert!(asked.prompts.is_empty(), "nobody to ask");
            assert!(message.contains("--yes"), "{message}");
            assert!(message.contains(&zoo.to_string()), "{message}");
            assert_eq!(crate::project::read_project_id(&dir), Some(own));

            let yes = JoinOptions {
                yes: true,
                ..JoinOptions::default()
            };
            let (result, asked) = register(&dir, yes, false, answer, false);
            result.expect("joined");
            assert!(asked.prompts.is_empty());
            assert_eq!(crate::project::read_project_id(&dir), Some(zoo));
            assert_eq!(registry(), vec![(zoo.to_string(), canonical(&dir))]);
        });
    }

    /// A registry holds one folder per project. On a home shared between
    /// machines the project's own checkout is already in reach.
    #[test]
    fn a_project_already_registered_at_another_folder_is_not_joined() {
        crate::project::test_support::with_isolated_config(|| {
            let home = tempfile::tempdir().expect("temp dir");
            let zoo_dir = home.path().join("repo");
            checkout(&zoo_dir);
            let zoo = pristine_identity(&zoo_dir);
            let dir = home.path().join("repo-again");
            checkout(&dir);
            let own = pristine_identity(&dir);
            let before = registry();
            let answer = by_remote(vec![remote_match(zoo, "repo", true)], None);

            let (result, asked) =
                register(&dir, JoinOptions::default(), true, answer.clone(), true);
            let message = result.expect("registered");
            assert!(
                asked.prompts.is_empty(),
                "a join that cannot happen is not offered"
            );
            assert!(message.contains(&canonical(&zoo_dir)), "{message}");

            let (result, _) = register(&dir, join(zoo), true, answer, true);
            let error = format!("{:#}", result.expect_err("refused"));
            assert!(error.contains(&canonical(&zoo_dir)), "{error}");

            assert_eq!(crate::project::read_project_id(&dir), Some(own));
            assert_eq!(crate::project::read_project_id(&zoo_dir), Some(zoo));
            assert_eq!(registry(), before);
        });
    }

    #[test]
    fn an_unreachable_server_never_leads_to_a_join() {
        crate::project::test_support::with_isolated_config(|| {
            let home = tempfile::tempdir().expect("temp dir");
            let dir = home.path().join("repo");
            checkout(&dir);
            let zoo = Uuid::new_v4();

            // An explicit join that cannot be checked changes nothing.
            let (result, asked) = register(&dir, join(zoo), true, None, true);
            let error = format!("{:#}", result.expect_err("refused"));
            assert!(error.contains("connection refused"), "{error}");
            assert_eq!(asked.lookups.len(), 1);
            assert!(!dir.join(".apas").exists());
            assert!(registry().is_empty());

            // Otherwise the folder is registered as it is, with a warning.
            let (result, asked) = register(&dir, JoinOptions::default(), true, None, true);
            let message = result.expect("registered");
            assert!(message.contains("Could not look up"), "{message}");
            assert!(asked.prompts.is_empty());
            let id = crate::project::read_project_id(&dir).expect("identity");
            assert_ne!(id, zoo);
            assert_eq!(registry(), vec![(id.to_string(), canonical(&dir))]);
        });
    }

    #[test]
    fn new_and_a_folder_without_a_remote_never_ask_the_server() {
        crate::project::test_support::with_isolated_config(|| {
            let home = tempfile::tempdir().expect("temp dir");
            let zoo = Uuid::new_v4();
            let answer = by_remote(vec![remote_match(zoo, "repo", true)], None);

            let dir = home.path().join("repo");
            checkout(&dir);
            let new = JoinOptions {
                new: true,
                ..JoinOptions::default()
            };
            let (result, asked) = register(&dir, new, true, answer.clone(), true);
            result.expect("registered");
            assert!(asked.lookups.is_empty() && asked.prompts.is_empty());
            assert_ne!(crate::project::read_project_id(&dir), Some(zoo));

            let plain = home.path().join("notes");
            fs::create_dir_all(&plain).expect("plain folder");
            let (result, asked) = register(&plain, join(zoo), true, answer.clone(), true);
            let error = format!("{:#}", result.expect_err("refused"));
            assert!(error.contains("origin"), "{error}");
            assert!(!plain.join(".apas").exists());
            let (result, asked_again) =
                register(&plain, JoinOptions::default(), true, answer, true);
            result.expect("registered");
            assert!(asked.lookups.is_empty() && asked_again.lookups.is_empty());
            assert_ne!(crate::project::read_project_id(&plain), Some(zoo));
        });
    }

    #[test]
    fn project_must_name_one_of_the_matching_projects() {
        crate::project::test_support::with_isolated_config(|| {
            let home = tempfile::tempdir().expect("temp dir");
            let dir = home.path().join("repo");
            checkout(&dir);
            let own = pristine_identity(&dir);
            let (zoo, stranger) = (Uuid::new_v4(), Uuid::new_v4());
            let answer = by_remote(vec![remote_match(zoo, "repo", true)], None);

            let (result, _) = register(&dir, join(stranger), true, answer.clone(), true);
            let error = format!("{:#}", result.expect_err("refused"));
            assert!(error.contains("not one of your projects"), "{error}");
            assert!(
                error.contains(&zoo.to_string()),
                "the matches are listed: {error}"
            );
            assert_eq!(crate::project::read_project_id(&dir), Some(own));

            // Its own project is no join at all.
            let (result, asked) = register(&dir, join(own), true, answer, true);
            result.expect("registered");
            assert!(asked.lookups.is_empty());
            assert_eq!(crate::project::read_project_id(&dir), Some(own));
            assert_eq!(registry(), vec![(own.to_string(), canonical(&dir))]);
        });
    }

    #[test]
    fn main_detect_running_daemon_removes_stale_legacy_pid_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let state_path = dir.path().join("daemon.json");
        let legacy_pid_path = dir.path().join("daemon.pid");
        fs::write(&legacy_pid_path, "99\n").expect("write legacy pid");

        let running =
            detect_running_daemon_with_process_check(&state_path, &legacy_pid_path, |_| false);

        assert!(running.is_none());
        assert!(!legacy_pid_path.exists());
    }
}

fn get_config_value(config: &config::Config, key: &str) -> Result<String> {
    let value = match key {
        "server" => config.remote.server.clone().unwrap_or_default(),
        "token" => config
            .remote
            .token
            .as_ref()
            .map(|_| "****".to_string())
            .unwrap_or_default(),
        "claude_path" => config.local.claude_path.clone(),
        "codex_path" => config.local.codex_path.clone(),
        "opencode_path" => config.local.opencode_path.clone(),
        "pi_path" => config.local.pi_path.clone(),
        "omp_path" => config.local.omp_path.clone(),
        "cursor_agent_path" => config.local.cursor_agent_path.clone(),
        "deepseek_api_base_url" => config
            .local
            .deepseek_api_base_url
            .clone()
            .unwrap_or_default(),
        "deepseek_api_key" => config
            .local
            .deepseek_api_key
            .as_ref()
            .map(|_| "****".to_string())
            .unwrap_or_default(),
        "pane_host_adoption_grace_seconds" => {
            config.local.pane_host_adoption_grace_seconds.to_string()
        }
        "pane_host_reboot_grace_seconds" => config.local.pane_host_reboot_grace_seconds.to_string(),
        "daemon_machine_id" => config.daemon.machine_id.clone().unwrap_or_default(),
        "daemon_roots" => config.daemon.project_roots.join(","),
        "summary_enabled" => config.summaries.enabled.to_string(),
        "summary_adapter" => format!("{:?}", config.summaries.adapter).to_ascii_lowercase(),
        "summary_model" => config.summaries.model.clone().unwrap_or_default(),
        "summary_timeout_seconds" => config.summaries.timeout_seconds.to_string(),
        "summary_max_input_bytes" => config.summaries.max_input_bytes.to_string(),
        "summary_allow_cross_provider" => config.summaries.allow_cross_provider.to_string(),
        _ => anyhow::bail!("Unknown config key: {}. Valid keys: {}", key, CONFIG_KEYS),
    };
    Ok(value)
}

async fn handle_config_command(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Set { key, value } => {
            let mut config = config::Config::load().unwrap_or_default();
            set_config_value(&mut config, &key, value)?;
            config.save()?;
            println!("Configuration saved");
            if key == "summary_adapter"
                && config.summaries.adapter == config::SummaryAdapterKind::Codex
            {
                eprintln!(
                    "WARNING: the Codex summary adapter retains a read-only command tool. \
                     Prompt and sandbox controls reduce but do not eliminate host-file read risk."
                );
            }
        }
        ConfigAction::Get { key } => {
            let config = config::Config::load()?;
            let value = get_config_value(&config, &key)?;
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
            println!("opencode_path: {}", config.local.opencode_path);
            println!("pi_path: {}", config.local.pi_path);
            println!("omp_path: {}", config.local.omp_path);
            println!("cursor_agent_path: {}", config.local.cursor_agent_path);
            println!(
                "deepseek_api_base_url: {}",
                config.local.deepseek_api_base_url.unwrap_or_default()
            );
            println!(
                "deepseek_api_key: {}",
                config
                    .local
                    .deepseek_api_key
                    .as_ref()
                    .map(|_| "****")
                    .unwrap_or("")
            );
            println!(
                "pane_host_adoption_grace_seconds: {}",
                config.local.pane_host_adoption_grace_seconds
            );
            println!(
                "pane_host_reboot_grace_seconds: {}",
                config.local.pane_host_reboot_grace_seconds
            );
            println!(
                "daemon_machine_id: {}",
                config.daemon.machine_id.unwrap_or_default()
            );
            println!("daemon_roots: {}", config.daemon.project_roots.join(","));
            println!("summary_enabled: {}", config.summaries.enabled);
            println!("summary_adapter: {:?}", config.summaries.adapter);
            println!(
                "summary_model: {}",
                config.summaries.model.unwrap_or_default()
            );
            println!(
                "summary_timeout_seconds: {}",
                config.summaries.timeout_seconds
            );
            println!(
                "summary_max_input_bytes: {}",
                config.summaries.max_input_bytes
            );
            println!(
                "summary_allow_cross_provider: {}",
                config.summaries.allow_cross_provider
            );
        }
        ConfigAction::Path => {
            let path = config::Config::config_path()?;
            println!("{}", path.display());
        }
    }
    Ok(())
}
