//! Auto-update functionality for the APAS CLI

use anyhow::Result;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const REPO_URL: &str = "https://github.com/shuaimu/apas.git";
const CURRENT_VERSION: &str = env!("APAS_VERSION");

/// Get the path to the source directory (~/.apas/source/)
fn source_dir() -> PathBuf {
    let dir = directories::ProjectDirs::from("", "", "apas")
        .map(|d| d.data_dir().join("source"))
        .unwrap_or_else(|| PathBuf::from("/tmp/apas/source"));
    fs::create_dir_all(&dir).ok();
    dir
}

/// Parse version string (YY.MM.COMMIT) into comparable number
fn parse_version(v: &str) -> Option<u64> {
    // Format: YY.MM.COMMIT (e.g., 26.01.42)
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let yy: u64 = parts[0].parse().ok()?;
    let mm: u64 = parts[1].parse().ok()?;
    let commit: u64 = parts[2].parse().ok()?;
    // Create comparable number: YYMM * 10000 + commit
    Some(yy * 1_000_000 + mm * 10_000 + commit)
}

/// Get the path to the current executable
fn get_current_exe() -> Option<PathBuf> {
    env::current_exe().ok()
}

/// Ensure the source repo exists (clone if not, fetch if exists)
/// Returns true if there are new commits available
fn sync_source_repo() -> Option<bool> {
    let src_dir = source_dir();
    let git_dir = src_dir.join(".git");

    if git_dir.exists() {
        // Repo exists, fetch updates
        let status = Command::new("git")
            .args(["fetch", "origin"])
            .current_dir(&src_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()?;

        if !status.success() {
            return None;
        }

        // Check if there are new commits
        let output = Command::new("git")
            .args(["rev-list", "HEAD..origin/master", "--count"])
            .current_dir(&src_dir)
            .output()
            .ok()?;

        let count: u64 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .unwrap_or(0);

        Some(count > 0)
    } else {
        // Clone the repo
        eprintln!("[Auto-update] First run, cloning source repository...");
        let status = Command::new("git")
            .args(["clone", REPO_URL, src_dir.to_str()?])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()?;

        if status.success() {
            Some(false) // Just cloned, no updates needed
        } else {
            None
        }
    }
}

/// Get the version string from the source repo
fn get_source_version() -> Option<String> {
    let src_dir = source_dir();

    // Get commit count
    let output = Command::new("git")
        .args(["rev-list", "--count", "origin/master"])
        .current_dir(&src_dir)
        .output()
        .ok()?;

    let commit_count = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Get date in YY.MM format
    let output = Command::new("date")
        .args(["+%y.%m"])
        .output()
        .ok()?;

    let date = String::from_utf8_lossy(&output.stdout).trim().to_string();

    Some(format!("{}.{}", date, commit_count))
}

/// Pull updates and build the new binary
fn pull_and_build() -> Result<PathBuf> {
    let src_dir = source_dir();

    // Pull the latest changes
    eprintln!("[Auto-update] Pulling latest changes...");
    let status = Command::new("git")
        .args(["pull", "origin", "master"])
        .current_dir(&src_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    if !status.success() {
        // Try to reset and pull again in case of conflicts
        Command::new("git")
            .args(["reset", "--hard", "origin/master"])
            .current_dir(&src_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;
    }

    // Build
    eprintln!("[Auto-update] Building...");
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "apas"])
        .current_dir(&src_dir)
        .status()?;

    if !status.success() {
        anyhow::bail!("Failed to build");
    }

    Ok(src_dir.join("target/release/apas"))
}

/// Install a new binary by replacing the current one
fn install_binary(new_binary: &PathBuf) -> Result<()> {
    let current_exe = get_current_exe()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine executable path"))?;

    // Backup and replace
    let backup_path = current_exe.with_extension("old");
    fs::rename(&current_exe, &backup_path).ok();

    if let Err(e) = fs::copy(new_binary, &current_exe) {
        // Restore backup
        fs::rename(&backup_path, &current_exe).ok();
        anyhow::bail!("Failed to install: {}", e);
    }

    // Cleanup backup
    fs::remove_file(&backup_path).ok();

    Ok(())
}

/// Check for updates and install if available (manual command)
pub async fn check_and_update() -> Result<()> {
    println!("Current version: {}", CURRENT_VERSION);
    println!("Checking for updates...\n");

    // Sync source repo
    match sync_source_repo() {
        Some(has_updates) => {
            if !has_updates {
                // Check version anyway in case we're behind
                let remote_version = get_source_version().unwrap_or_default();
                let current = parse_version(CURRENT_VERSION);
                let remote = parse_version(&remote_version);

                if let (Some(c), Some(r)) = (current, remote) {
                    if r <= c {
                        println!("Already up to date ({})", CURRENT_VERSION);
                        return Ok(());
                    }
                }
            }
        }
        None => {
            anyhow::bail!("Failed to sync source repository");
        }
    }

    // Build and install
    let new_binary = pull_and_build()?;
    install_binary(&new_binary)?;

    // Get new version
    let current_exe = get_current_exe().unwrap();
    let output = Command::new(&current_exe)
        .args(["--version"])
        .output();

    let new_version = output
        .map(|o| {
            let full = String::from_utf8_lossy(&o.stdout).trim().to_string();
            full.strip_prefix("apas ").unwrap_or(&full).to_string()
        })
        .unwrap_or_else(|_| "unknown".to_string());

    println!("\nUpdated! {} -> {}", CURRENT_VERSION, new_version);
    println!("Restart apas to use the new version.");

    Ok(())
}

/// Check if an update is available, returns the new version string if available
pub fn check_for_update_available() -> Option<String> {
    // Sync source repo first
    sync_source_repo()?;

    let current = parse_version(CURRENT_VERSION)?;
    let remote_version_str = get_source_version()?;
    let remote = parse_version(&remote_version_str)?;

    if remote > current {
        Some(remote_version_str)
    } else {
        None
    }
}

/// Check for updates on boot and automatically install + restart if available
/// This function will not return if an update is installed (it exec's the new binary)
pub fn check_and_upgrade_on_boot() {
    eprintln!("[Auto-update] Checking for updates...");

    // Sync source repo (fetch or clone)
    let has_updates = match sync_source_repo() {
        Some(v) => v,
        None => {
            eprintln!("[Auto-update] Failed to sync source repository");
            return;
        }
    };

    if !has_updates {
        // Double-check by comparing versions
        let current = match parse_version(CURRENT_VERSION) {
            Some(v) => v,
            None => {
                eprintln!("[Auto-update] Failed to parse current version");
                return;
            }
        };

        let remote_version_str = match get_source_version() {
            Some(v) => v,
            None => {
                eprintln!("[Auto-update] Failed to get remote version");
                return;
            }
        };

        let remote = match parse_version(&remote_version_str) {
            Some(v) => v,
            None => {
                eprintln!("[Auto-update] Failed to parse remote version");
                return;
            }
        };

        if remote <= current {
            eprintln!("[Auto-update] Already up to date ({})", CURRENT_VERSION);
            return;
        }

        eprintln!("[Auto-update] Update available: {} -> {}", CURRENT_VERSION, remote_version_str);
    } else {
        eprintln!("[Auto-update] New commits available, updating...");
    }

    // Build and install
    eprintln!("[Auto-update] Installing update...");
    let new_binary = match pull_and_build() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[Auto-update] Build failed: {}", e);
            return;
        }
    };

    if let Err(e) = install_binary(&new_binary) {
        eprintln!("[Auto-update] Install failed: {}", e);
        return;
    }

    // Restart the process with the same arguments
    eprintln!("[Auto-update] Restarting...");
    restart_self();
}

/// Restart the current process with the same arguments
#[cfg(unix)]
fn restart_self() {
    use std::os::unix::process::CommandExt;

    let exe = match get_current_exe() {
        Some(e) => e,
        None => {
            eprintln!("[Auto-update] Failed to get executable path for restart");
            return;
        }
    };

    let args: Vec<String> = env::args().collect();

    // exec() replaces the current process - this function won't return on success
    let err = Command::new(&exe).args(&args[1..]).exec();
    eprintln!("[Auto-update] Failed to restart: {}", err);
}

#[cfg(not(unix))]
fn restart_self() {
    eprintln!("[Auto-update] Auto-restart not supported on this platform");
    eprintln!("[Auto-update] Please restart manually to use the new version");
}
