use anyhow::Result;
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

mod auth;
mod claude;
mod config;
mod conversation;
mod daemon_registry;
mod file_watcher;
mod manager;
mod mcp;
mod mode;
mod attach;
mod pane_host;
mod supervisor;
mod pane_status;
mod plan_review;
mod project;
mod role;
mod scratchpad;
mod suggested_workers;
mod summary_runner;
mod team_todo;
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
    /// Serve the team-mode MCP tool surface on stdio (Phase 3.1 follow-up).
    /// Spawned per pane by the CLI, not run by hand — `--pane-id` is what
    /// stamps published records, so it must match the calling pane.
    McpServer {
        /// Project root containing .apas / team-todo.md / .apas-team.jsonl
        #[arg(long)]
        project_dir: PathBuf,
        /// Pane this server publishes as
        #[arg(long)]
        pane_id: u32,
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
    // Every project used to have its own tmux session and its own stderr file,
    // which separated their records for free. Projects now share this process,
    // so each carries a `project` span and the records must show it — without
    // that, one incident's log is an interleaving of several projects with no
    // way to tell them apart.
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "apas=warn".into()),
        )
        // The default `Full` format prints the enclosing span and its fields,
        // which is what carries the project id onto every record.
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    let cli = Cli::parse();

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
            Commands::McpServer {
                project_dir,
                pane_id,
            } => {
                // stdout is the JSON-RPC channel — anything else written there
                // corrupts the stream. Tracing already goes to stderr (see the
                // subscriber above), so nothing extra is needed here beyond not
                // printing.
                return mcp::run(project_dir, pane_id).await;
            }
            Commands::PaneHost { runtime_dir } => {
                return pane_host::run_host(runtime_dir);
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

                // Derive per host rather than mint-and-persist. The old code
                // generated a random UUID and saved it to config.toml -- which
                // lives on the shared NFS home, so whichever host ran first
                // donated its identity to every other host in the cluster, and
                // the server's machine map (keyed by this id) collapsed them
                // into one flickering entry. A derived id needs no storage,
                // survives reboots, and is necessarily distinct per host.
                // An explicitly configured value still wins, for overrides.
                let machine_id = config
                    .daemon
                    .machine_id
                    .as_ref()
                    .and_then(|raw| Uuid::parse_str(raw).ok())
                    .unwrap_or_else(daemon_registry::derive_machine_id);

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
        // Replacing the process is this caller's decision, not the project's.
        // The project used to `exec` from inside itself, which reads the same
        // only while a process hosts exactly one project.
        match mode::dual_pane::run_headless(&server, &token, &working_dir).await? {
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
                    eprintln!(
                        "\x1b[33mNo project is running here to attach to.\x1b[0m"
                    );
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
    let metadata = project::get_or_create_project(working_dir)?;
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
            if let Err(err) = attach::run_attached_tui(
                attachment,
                tabs,
                working_dir.display().to_string(),
            ) {
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

const CONFIG_KEYS: &str = "server, token, claude_path, codex_path, opencode_path, cursor_agent_path, deepseek_api_base_url, deepseek_api_key, pane_host_adoption_grace_seconds, pane_host_reboot_grace_seconds, daemon_machine_id, daemon_roots, summary_enabled, summary_adapter, summary_model, summary_timeout_seconds, summary_max_input_bytes, summary_allow_cross_provider";

fn set_config_value(config: &mut config::Config, key: &str, value: String) -> Result<()> {
    match key {
        "server" => config.remote.server = Some(value),
        "token" => config.remote.token = Some(value),
        "claude_path" => config.local.claude_path = value,
        "codex_path" => config.local.codex_path = value,
        "opencode_path" => config.local.opencode_path = value,
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
        read_daemon_state,
        read_legacy_daemon_pid, set_config_value, should_restart_for_version, write_daemon_state,
        DaemonStateFile, DaemonStateGuard,
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
                crate::project::list_registered_projects().expect("registry").len(),
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
