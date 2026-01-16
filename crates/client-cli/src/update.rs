//! Auto-update functionality for the APAS CLI

use anyhow::Result;
use semver::Version;
use serde::Deserialize;
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const REPO: &str = "shuaimu/apas";
const BINARY_NAME: &str = "apas";
const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60); // 24 hours
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
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
    // Touch the file
    fs::write(&check_file, "").ok();
}

/// Get the platform string for downloads
fn get_platform() -> Option<String> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;

    let os_str = match os {
        "linux" => "linux",
        "macos" => "darwin",
        _ => return None,
    };

    let arch_str = match arch {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => return None,
    };

    Some(format!("{}-{}", os_str, arch_str))
}

/// Get the path to the current executable
fn get_current_exe() -> Option<PathBuf> {
    env::current_exe().ok()
}

/// Check for updates and install if available
pub async fn check_and_update() -> Result<()> {
    // Skip if we checked recently
    if !should_check_for_updates() {
        tracing::debug!("Skipping update check (checked recently)");
        return Ok(());
    }

    mark_update_checked();

    let current_version = Version::parse(CURRENT_VERSION)?;
    tracing::debug!("Current version: {}", current_version);

    // Fetch latest release from GitHub
    let client = reqwest::Client::builder()
        .user_agent("apas-updater")
        .timeout(Duration::from_secs(10))
        .build()?;

    let url = format!("https://api.github.com/repos/{}/releases/latest", REPO);
    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        tracing::debug!("Failed to fetch release info: {}", response.status());
        return Ok(());
    }

    let release: GitHubRelease = response.json().await?;
    let latest_tag = release.tag_name.trim_start_matches('v');
    let latest_version = Version::parse(latest_tag)?;

    tracing::debug!("Latest version: {}", latest_version);

    if latest_version <= current_version {
        tracing::debug!("Already up to date");
        return Ok(());
    }

    println!("New version available: {} -> {}", current_version, latest_version);

    // Find the right asset for our platform
    let platform = match get_platform() {
        Some(p) => p,
        None => {
            println!("Cannot auto-update: unsupported platform");
            return Ok(());
        }
    };

    let asset_name = format!("{}-{}", BINARY_NAME, platform);
    let asset = release.assets.iter().find(|a| a.name == asset_name);

    let asset = match asset {
        Some(a) => a,
        None => {
            println!("Cannot auto-update: no binary available for {}", platform);
            println!("Download manually from: https://github.com/{}/releases", REPO);
            return Ok(());
        }
    };

    // Download the new binary
    println!("Downloading update...");
    let binary_response = client.get(&asset.browser_download_url).send().await?;

    if !binary_response.status().is_success() {
        println!("Failed to download update: {}", binary_response.status());
        return Ok(());
    }

    let binary_data = binary_response.bytes().await?;

    // Get current executable path
    let current_exe = match get_current_exe() {
        Some(p) => p,
        None => {
            println!("Cannot auto-update: unable to determine executable path");
            return Ok(());
        }
    };

    // Write to a temporary file first
    let tmp_path = current_exe.with_extension("new");
    fs::write(&tmp_path, &binary_data)?;

    // Make it executable
    let mut perms = fs::metadata(&tmp_path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&tmp_path, perms)?;

    // Replace the old binary
    let backup_path = current_exe.with_extension("old");
    fs::rename(&current_exe, &backup_path).ok(); // Backup old version

    if let Err(e) = fs::rename(&tmp_path, &current_exe) {
        // Try to restore backup
        fs::rename(&backup_path, &current_exe).ok();
        println!("Failed to install update: {}", e);
        return Ok(());
    }

    // Remove backup
    fs::remove_file(&backup_path).ok();

    println!("Updated to version {}!", latest_version);
    println!("Restart apas to use the new version.");

    Ok(())
}

/// Check for updates in the background (non-blocking)
pub fn check_for_updates_background() {
    tokio::spawn(async {
        if let Err(e) = check_and_update().await {
            tracing::debug!("Update check failed: {}", e);
        }
    });
}
