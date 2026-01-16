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

/// Parse version string (YY-MM-DD-COMMIT) into comparable number
fn parse_version(v: &str) -> Option<u64> {
    // Format: YY-MM-DD-COMMIT (e.g., 26-01-15-42)
    let parts: Vec<&str> = v.split('-').collect();
    if parts.len() != 4 {
        return None;
    }
    let yy: u64 = parts[0].parse().ok()?;
    let mm: u64 = parts[1].parse().ok()?;
    let dd: u64 = parts[2].parse().ok()?;
    let commit: u64 = parts[3].parse().ok()?;
    // Create comparable number: YYMMDD * 10000 + commit
    Some(yy * 100_000_000 + mm * 1_000_000 + dd * 10_000 + commit)
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

    // Shallow clone to check commit count
    let status = Command::new("git")
        .args(["clone", "--depth", "1", REPO_URL, build_dir.to_str()?])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;

    if !status.success() {
        return None;
    }

    // Get commit count (need to unshallow for accurate count)
    let output = Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(&build_dir)
        .output()
        .ok()?;

    let commit_count = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Get date
    let output = Command::new("date")
        .args(["+%y-%m-%d"])
        .output()
        .ok()?;

    let date = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Cleanup
    fs::remove_dir_all(&build_dir).ok();

    Some(format!("{}-{}", date, commit_count))
}

/// Check for updates and install if available
pub async fn check_and_update() -> Result<()> {
    println!("Current version: {}", CURRENT_VERSION);
    println!("Checking for updates...\n");

    // Clone and build from source
    let build_dir = env::temp_dir().join(format!("apas-update-{}", std::process::id()));

    // Clone repo
    println!("Cloning repository...");
    let status = Command::new("git")
        .args(["clone", "--depth", "1", REPO_URL, build_dir.to_str().unwrap()])
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

    // Get new version
    let output = Command::new(&current_exe)
        .args(["--version"])
        .output();

    let new_version = output
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
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
