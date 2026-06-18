//! Auto-update functionality for the APAS CLI

use anyhow::Result;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

const REPO_URL: &str = "https://github.com/shuaimu/apas.git";
const CURRENT_VERSION: &str = env!("APAS_VERSION");

/// Snapshot of `current_exe()` taken at process launch (before any
/// binary replacement). Used by `resolve_preferred_apas_executable` so
/// reboots exec a known-good path even after `current_exe()` starts
/// returning an NFS silly-renamed `.nfsXXX` inode.
static LAUNCH_BINARY_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Capture the exec path at startup. Call this from `main` before any
/// long-running work (and definitely before the daemon could survive
/// long enough for the binary to be replaced underneath it). Only stores
/// the path if it looks like a real on-disk file — `/proc/self/exe`,
/// deleted, and `.nfsXXX` paths are skipped so the existing fallback
/// chain still applies.
pub fn capture_launch_binary_path() {
    if let Ok(path) = env::current_exe() {
        if is_safe_restart_exe(&path) {
            let _ = LAUNCH_BINARY_PATH.set(path);
        }
    }
}

/// Get the path to the source directory (~/.apas/source/)
fn source_dir() -> PathBuf {
    let dir = directories::ProjectDirs::from("", "", "apas")
        .map(|d| d.data_dir().join("source"))
        .unwrap_or_else(|| PathBuf::from("/tmp/apas/source"));
    fs::create_dir_all(&dir).ok();
    dir
}

/// Parse version string (YY.MM.COMMIT) into comparable tuple
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    // Format: YY.MM.COMMIT (e.g., 26.01.42)
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let yy: u64 = parts[0].parse().ok()?;
    let mm: u64 = parts[1].parse().ok()?;
    let commit: u64 = parts[2].parse().ok()?;
    Some((yy, mm, commit))
}

/// Get the path to the current executable as reported by the OS.
fn get_current_exe() -> Option<PathBuf> {
    env::current_exe().ok()
}

fn is_proc_self_exe(path: &Path) -> bool {
    path == Path::new("/proc/self/exe")
}

fn is_deleted_path(path: &Path) -> bool {
    path.to_string_lossy().contains(" (deleted)")
}

/// NFS "silly-rename": when an open file is unlinked on NFS, the client
/// renames it to `.nfsXXXX...` so existing open handles still work. The
/// file keeps existing (so `path.exists()` is true) but it's the *old*
/// inode, not the freshly installed binary. We must treat these as
/// stale or the auto-update path will exec() the binary it just replaced.
fn is_nfs_silly_rename(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with(".nfs"))
        .unwrap_or(false)
}

fn is_safe_restart_exe(path: &Path) -> bool {
    !is_proc_self_exe(path) && !is_deleted_path(path) && !is_nfs_silly_rename(path) && path.exists()
}

/// Return a usable on-disk executable for the current process.
/// This intentionally excludes /proc/self/exe and deleted inode paths.
fn get_current_on_disk_exe() -> Option<PathBuf> {
    let path = get_current_exe()?;
    if !is_safe_restart_exe(&path) {
        return None;
    }
    Some(path)
}

fn home_installed_exe() -> Option<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".local/bin/apas"))
        .filter(|path| path.exists())
}

fn path_installed_exe() -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join("apas");
        if !is_safe_restart_exe(&candidate) {
            continue;
        }
        return Some(candidate);
    }
    None
}

fn argv0_exe() -> Option<PathBuf> {
    let argv0 = env::args().next()?;
    let path = PathBuf::from(argv0);
    if !path.is_absolute() || !is_safe_restart_exe(&path) {
        return None;
    }
    Some(path)
}

fn default_install_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".local/bin/apas"))
}

fn resolve_install_target_exe() -> Option<PathBuf> {
    get_current_on_disk_exe()
        .or_else(home_installed_exe)
        .or_else(path_installed_exe)
        .or_else(argv0_exe)
        .or_else(default_install_path)
}

fn resolve_preferred_apas_executable_from_candidates<'a, I>(
    launch_path: Option<&'a Path>,
    candidates: I,
) -> PathBuf
where
    I: IntoIterator<Item = &'a Path>,
{
    if let Some(path) = launch_path {
        if is_safe_restart_exe(path) {
            return path.to_path_buf();
        }
    }

    candidates
        .into_iter()
        .find(|path| is_safe_restart_exe(path))
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("apas"))
}

/// Resolve the executable path we should restart/spawn.
/// Priority is always real on-disk binaries, never /proc/self/exe.
///
/// The launch-time snapshot wins if it's still on disk — after an
/// atomic binary swap, the original path now points to the *new*
/// binary, while `current_exe()` points to the `.nfsXXX` silly-rename
/// of the old inode. Capturing the path at launch and reusing it is
/// the simplest way to make reboot deterministic.
pub fn resolve_preferred_apas_executable() -> PathBuf {
    let candidates = [
        get_current_on_disk_exe(),
        home_installed_exe(),
        path_installed_exe(),
        argv0_exe(),
    ];
    resolve_preferred_apas_executable_from_candidates(
        LAUNCH_BINARY_PATH.get().map(PathBuf::as_path),
        candidates.iter().filter_map(|path| path.as_deref()),
    )
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

    // Get date in YY.MM format
    let output = Command::new("date").args(["+%y.%m"]).output().ok()?;
    let date = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Get month start timestamp for current month (YYYY-MM-01 00:00:00)
    let output = Command::new("date")
        .args(["+%Y-%m-01 00:00:00"])
        .output()
        .ok()?;
    let month_start = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Get commit count since month start
    let output = Command::new("git")
        .arg("rev-list")
        .arg("--count")
        .arg(format!("--since={month_start}"))
        .arg("origin/master")
        .current_dir(&src_dir)
        .output()
        .ok()?;

    let commit_count = String::from_utf8_lossy(&output.stdout).trim().to_string();

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
    let install_path = resolve_install_target_exe()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine install target path"))?;

    if let Some(parent) = install_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Backup and replace
    let backup_path = install_path.with_extension("old");
    let had_existing = install_path.exists();
    if had_existing {
        let _ = fs::rename(&install_path, &backup_path);
    }

    if let Err(e) = fs::copy(new_binary, &install_path) {
        // Restore backup
        if had_existing {
            let _ = fs::rename(&backup_path, &install_path);
        }
        anyhow::bail!("Failed to install: {}", e);
    }

    // Cleanup backup
    if had_existing {
        let _ = fs::remove_file(&backup_path);
    }

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
    let current_exe = resolve_preferred_apas_executable();
    let output = Command::new(&current_exe).args(["--version"]).output();

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

        eprintln!(
            "[Auto-update] Update available: {} -> {}",
            CURRENT_VERSION, remote_version_str
        );
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

/// Restart using the preferred on-disk binary (for auto-update).
#[cfg(unix)]
fn restart_self() {
    use std::os::unix::process::CommandExt;

    let args: Vec<String> = env::args().collect();

    let exe = resolve_preferred_apas_executable();

    eprintln!("[Auto-update] Restarting with new binary: {:?}", exe);

    // Clear terminal screen before restart
    print!("\x1B[2J\x1B[H");
    let _ = std::io::Write::flush(&mut std::io::stdout());

    let err = Command::new(&exe).args(&args[1..]).exec();
    eprintln!("[Auto-update] Failed to restart: {}", err);
}

#[cfg(not(unix))]
fn restart_self() {
    eprintln!("[Auto-update] Auto-restart not supported on this platform");
    eprintln!("[Auto-update] Please restart manually to use the new version");
}

/// Restart the CLI process (public, can be called from other modules)
/// This function will not return on success (it exec's the new binary)
#[cfg(unix)]
pub fn restart_cli() {
    use std::os::unix::process::CommandExt;

    let args: Vec<String> = env::args().collect();

    if let Some(newer) = check_for_update_available() {
        eprintln!(
            "[Restart] Newer git version detected ({} > {}), updating before reboot...",
            newer, CURRENT_VERSION
        );
        match pull_and_build().and_then(|binary| install_binary(&binary)) {
            Ok(()) => {
                eprintln!("[Restart] Update installed, rebooting into {}", newer);
            }
            Err(err) => {
                eprintln!(
                    "[Restart] Update before reboot failed ({}), continuing with installed binary",
                    err
                );
            }
        }
    }

    let exe = resolve_preferred_apas_executable();

    // Clear terminal screen and show countdown
    print!("\x1B[2J\x1B[H");
    let _ = std::io::Write::flush(&mut std::io::stdout());

    for i in (1..=3).rev() {
        println!("[Restart] Restarting in {}...", i);
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    println!("[Restart] Restarting now...");

    // exec() replaces the current process - this function won't return on success
    let err = Command::new(&exe).args(&args[1..]).exec();

    // If we get here, exec failed
    eprintln!("[Restart] Failed to restart: {}", err);
    eprintln!("[Restart] Executable: {:?}", exe);
    eprintln!("[Restart] Please restart manually.");
}

#[cfg(not(unix))]
pub fn restart_cli() {
    eprintln!("[Restart] Auto-restart not supported on this platform");
    eprintln!("[Restart] Please restart manually");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn touch(path: &Path) {
        fs::write(path, b"apas").unwrap();
    }

    #[test]
    fn is_nfs_silly_rename_matches_nfs_prefix() {
        assert!(is_nfs_silly_rename(Path::new(
            "/home/users/shuai/.local/bin/.nfs0000000001e8ab86000001f5"
        )));
        assert!(is_nfs_silly_rename(Path::new(".nfs1234")));
    }

    #[test]
    fn is_nfs_silly_rename_rejects_normal_paths() {
        assert!(!is_nfs_silly_rename(Path::new(
            "/home/users/shuai/.local/bin/apas"
        )));
        assert!(!is_nfs_silly_rename(Path::new("apas")));
        assert!(!is_nfs_silly_rename(Path::new("/usr/bin/apas")));
        // Hidden dotfiles that aren't NFS ghosts should not match.
        assert!(!is_nfs_silly_rename(Path::new("/home/user/.bashrc")));
        assert!(!is_nfs_silly_rename(Path::new("/home/user/.config/apas")));
    }

    #[test]
    fn resolve_preferred_apas_executable_prefers_safe_launch_path() {
        let dir = tempdir().unwrap();
        let launch = dir.path().join("launch-apas");
        let installed = dir.path().join("installed-apas");
        touch(&launch);
        touch(&installed);

        let resolved =
            resolve_preferred_apas_executable_from_candidates(Some(&launch), [installed.as_path()]);

        assert_eq!(resolved, launch);
    }

    #[test]
    fn resolve_preferred_apas_executable_rejects_unsafe_launch_paths() {
        let dir = tempdir().unwrap();
        let installed = dir.path().join("installed-apas");
        let deleted = dir.path().join("apas (deleted)");
        let nfs = dir.path().join(".nfs0000000001e8ab86000001f5");
        touch(&installed);
        touch(&deleted);
        touch(&nfs);

        for launch in [
            Path::new("/proc/self/exe"),
            deleted.as_path(),
            nfs.as_path(),
        ] {
            let resolved = resolve_preferred_apas_executable_from_candidates(
                Some(launch),
                [installed.as_path()],
            );
            assert_eq!(resolved, installed);
        }
    }

    #[test]
    fn resolve_preferred_apas_executable_skips_unsafe_candidates() {
        let dir = tempdir().unwrap();
        let installed = dir.path().join("installed-apas");
        let deleted = dir.path().join("apas (deleted)");
        let nfs = dir.path().join(".nfs0000000001e8ab86000001f5");
        touch(&installed);
        touch(&deleted);
        touch(&nfs);

        let resolved = resolve_preferred_apas_executable_from_candidates(
            None,
            [
                Path::new("/proc/self/exe"),
                deleted.as_path(),
                nfs.as_path(),
                installed.as_path(),
            ],
        );

        assert_eq!(resolved, installed);
    }

    #[test]
    fn resolve_preferred_apas_executable_falls_back_to_plain_apas_without_safe_candidates() {
        let dir = tempdir().unwrap();
        let deleted = dir.path().join("apas (deleted)");
        let nfs = dir.path().join(".nfs0000000001e8ab86000001f5");
        touch(&deleted);
        touch(&nfs);

        let resolved = resolve_preferred_apas_executable_from_candidates(
            None,
            [
                Path::new("/proc/self/exe"),
                deleted.as_path(),
                nfs.as_path(),
            ],
        );

        assert_eq!(resolved, PathBuf::from("apas"));
    }
}
