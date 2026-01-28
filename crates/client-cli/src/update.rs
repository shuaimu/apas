//! Auto-update functionality for the APAS CLI

use anyhow::Result;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, SystemTime};

const REPO_URL: &str = "https://github.com/shuaimu/apas.git";
const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60); // 24 hours
const CURRENT_VERSION: &str = env!("APAS_VERSION");

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

/// Get the path to the update check timestamp file
fn update_check_file() -> PathBuf {
    let config_dir = directories::ProjectDirs::from("", "", "apas")
        .map(|d| d.cache_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("/tmp/apas"));

    fs::create_dir_all(&config_dir).ok();
    config_dir.join("last_update_check")
}

/// Check if we should check for updates (based on time since last check)
fn should_check_for_updates() -> bool {
    let check_file = update_check_file();

    if let Ok(metadata) = fs::metadata(&check_file) {
        if let Ok(modified) = metadata.modified() {
            if let Ok(elapsed) = SystemTime::now().duration_since(modified) {
                return elapsed > UPDATE_CHECK_INTERVAL;
            }
        }
    }

    true // Check if file doesn't exist or can't read it
}

/// Mark that we checked for updates
fn mark_update_checked() {
    let check_file = update_check_file();
    fs::write(&check_file, "").ok();
}

/// Get the path to the current executable
fn get_current_exe() -> Option<PathBuf> {
    env::current_exe().ok()
}

/// Get latest version from remote repo by checking commit count
fn get_remote_version() -> Option<String> {
    let build_dir = env::temp_dir().join(format!("apas-version-check-{}", std::process::id()));

    // Full clone needed for accurate commit count
    let status = Command::new("git")
        .args(["clone", REPO_URL, build_dir.to_str()?])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;

    if !status.success() {
        return None;
    }

    // Get commit count
    let output = Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(&build_dir)
        .output()
        .ok()?;

    let commit_count = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Get date in YY.MM format
    let output = Command::new("date")
        .args(["+%y.%m"])
        .output()
        .ok()?;

    let date = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Cleanup
    fs::remove_dir_all(&build_dir).ok();

    Some(format!("{}.{}", date, commit_count))
}

/// Check for updates and install if available
pub async fn check_and_update() -> Result<()> {
    println!("Current version: {}", CURRENT_VERSION);
    println!("Checking for updates...\n");

    // Clone and build from source
    let build_dir = env::temp_dir().join(format!("apas-update-{}", std::process::id()));

    // Clone repo (full clone needed for accurate commit count in version)
    println!("Cloning repository...");
    let status = Command::new("git")
        .args(["clone", REPO_URL, build_dir.to_str().unwrap()])
        .status()?;

    if !status.success() {
        anyhow::bail!("Failed to clone repository");
    }

    // Build
    println!("Building...");
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "apas"])
        .current_dir(&build_dir)
        .status()?;

    if !status.success() {
        fs::remove_dir_all(&build_dir).ok();
        anyhow::bail!("Failed to build");
    }

    // Get current executable path
    let current_exe = get_current_exe()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine executable path"))?;

    // Copy new binary
    let new_binary = build_dir.join("target/release/apas");

    // Backup and replace
    let backup_path = current_exe.with_extension("old");
    fs::rename(&current_exe, &backup_path).ok();

    if let Err(e) = fs::copy(&new_binary, &current_exe) {
        // Restore backup
        fs::rename(&backup_path, &current_exe).ok();
        fs::remove_dir_all(&build_dir).ok();
        anyhow::bail!("Failed to install: {}", e);
    }

    // Cleanup
    fs::remove_file(&backup_path).ok();
    fs::remove_dir_all(&build_dir).ok();

    // Get new version (--version outputs "apas X.Y.Z", extract just version)
    let output = Command::new(&current_exe)
        .args(["--version"])
        .output();

    let new_version = output
        .map(|o| {
            let full = String::from_utf8_lossy(&o.stdout).trim().to_string();
            // Extract version number after "apas "
            full.strip_prefix("apas ").unwrap_or(&full).to_string()
        })
        .unwrap_or_else(|_| "unknown".to_string());

    println!("\nUpdated! {} -> {}", CURRENT_VERSION, new_version);
    println!("Restart apas to use the new version.");

    Ok(())
}

/// Check for updates in the background (non-blocking)
pub fn check_for_updates_background() {
    // Skip if we checked recently
    if !should_check_for_updates() {
        return;
    }

    mark_update_checked();

    std::thread::spawn(|| {
        let current = parse_version(CURRENT_VERSION);
        let remote = get_remote_version().and_then(|v| parse_version(&v));

        if let (Some(curr), Some(rem)) = (current, remote) {
            if rem > curr {
                println!("Update available! Run 'apas update' to install.");
            }
        }
    });
}

/// Check if an update is available, returns the new version string if available
pub fn check_for_update_available() -> Option<String> {
    let current = parse_version(CURRENT_VERSION)?;
    let remote_version_str = get_remote_version()?;
    let remote = parse_version(&remote_version_str)?;

    if remote > current {
        Some(remote_version_str)
    } else {
        None
    }
}

/// Check for updates on boot and automatically install + restart if available
/// Respects the update check interval to avoid checking too frequently
/// This function will not return if an update is installed (it exec's the new binary)
pub fn check_and_upgrade_on_boot() {
    // Skip if we checked recently
    if !should_check_for_updates() {
        return;
    }

    mark_update_checked();
    auto_update_and_restart();
}

/// Check for updates and automatically install + restart if available
/// This function will not return if an update is installed (it exec's the new binary)
pub fn auto_update_and_restart() {
    eprintln!("[Auto-update] Checking for updates...");

    // Check if update is available
    let current = match parse_version(CURRENT_VERSION) {
        Some(v) => v,
        None => {
            eprintln!("[Auto-update] Failed to parse current version");
            return;
        }
    };

    let remote_version_str = match get_remote_version() {
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
    eprintln!("[Auto-update] Installing update...");

    // Run the update synchronously
    let rt = tokio::runtime::Runtime::new().unwrap();
    if let Err(e) = rt.block_on(check_and_update()) {
        eprintln!("[Auto-update] Update failed: {}", e);
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
