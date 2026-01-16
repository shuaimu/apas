//! Auto-update functionality for the APAS CLI

use anyhow::Result;
use semver::Version;
use serde::Deserialize;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, SystemTime};

const REPO: &str = "shuaimu/apas";
const REPO_URL: &str = "https://github.com/shuaimu/apas.git";
const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60); // 24 hours
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
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

/// Check for updates and install if available
pub async fn check_and_update() -> Result<()> {
    // Skip if we checked recently
    if !should_check_for_updates() {
        tracing::debug!("Skipping update check (checked recently)");
        return Ok(());
    }

    mark_update_checked();

    let current_version = Version::parse(CURRENT_VERSION)?;
    println!("Current version: {}", current_version);

    // Fetch latest release from GitHub
    let client = reqwest::Client::builder()
        .user_agent("apas-updater")
        .timeout(Duration::from_secs(10))
        .build()?;

    let url = format!("https://api.github.com/repos/{}/releases/latest", REPO);
    let response = client.get(&url).send().await;

    let latest_version = match response {
        Ok(resp) if resp.status().is_success() => {
            let release: GitHubRelease = resp.json().await?;
            let latest_tag = release.tag_name.trim_start_matches('v');
            Version::parse(latest_tag)?
        }
        _ => {
            // No releases yet, check Cargo.toml from main branch
            println!("No releases found, checking main branch...");
            let cargo_url = format!(
                "https://raw.githubusercontent.com/{}/master/Cargo.toml",
                REPO
            );
            let resp = client.get(&cargo_url).send().await?;
            if !resp.status().is_success() {
                println!("Could not check for updates");
                return Ok(());
            }
            let cargo_toml = resp.text().await?;
            // Parse version from Cargo.toml
            let version_line = cargo_toml
                .lines()
                .find(|l| l.starts_with("version = "))
                .unwrap_or("version = \"0.1.0\"");
            let version_str = version_line
                .split('"')
                .nth(1)
                .unwrap_or("0.1.0");
            Version::parse(version_str)?
        }
    };

    println!("Latest version: {}", latest_version);

    if latest_version <= current_version {
        println!("Already up to date!");
        return Ok(());
    }

    println!("\nNew version available: {} -> {}", current_version, latest_version);
    println!("Building from source...\n");

    // Build from source
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

    println!("\nUpdated to version {}!", latest_version);
    println!("Restart apas to use the new version.");

    Ok(())
}

/// Check for updates in the background (non-blocking)
pub fn check_for_updates_background() {
    tokio::spawn(async {
        match check_update_available().await {
            Ok(Some(new_version)) => {
                println!("Update available: {} (run 'apas update' to install)", new_version);
            }
            Ok(None) => {}
            Err(e) => {
                tracing::debug!("Update check failed: {}", e);
            }
        }
    });
}

/// Check if update is available without installing
async fn check_update_available() -> Result<Option<String>> {
    if !should_check_for_updates() {
        return Ok(None);
    }

    mark_update_checked();

    let current_version = Version::parse(CURRENT_VERSION)?;

    let client = reqwest::Client::builder()
        .user_agent("apas-updater")
        .timeout(Duration::from_secs(5))
        .build()?;

    // Try releases first
    let url = format!("https://api.github.com/repos/{}/releases/latest", REPO);
    if let Ok(resp) = client.get(&url).send().await {
        if resp.status().is_success() {
            if let Ok(release) = resp.json::<GitHubRelease>().await {
                let latest_tag = release.tag_name.trim_start_matches('v');
                if let Ok(latest) = Version::parse(latest_tag) {
                    if latest > current_version {
                        return Ok(Some(latest.to_string()));
                    }
                }
            }
        }
    }

    // Check Cargo.toml as fallback
    let cargo_url = format!(
        "https://raw.githubusercontent.com/{}/master/Cargo.toml",
        REPO
    );
    if let Ok(resp) = client.get(&cargo_url).send().await {
        if resp.status().is_success() {
            if let Ok(text) = resp.text().await {
                if let Some(line) = text.lines().find(|l| l.starts_with("version = ")) {
                    if let Some(ver) = line.split('"').nth(1) {
                        if let Ok(latest) = Version::parse(ver) {
                            if latest > current_version {
                                return Ok(Some(latest.to_string()));
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(None)
}
