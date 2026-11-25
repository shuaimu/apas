//! Which terminal providers this machine can launch, for `MachineInfo`.
//!
//! The daemon reports it so a viewer can tell, before opening a tab, that a
//! machine has no OMP (or can install it). Found the way a launch finds the
//! executable: the configured path through the daemon's PATH (the login-shell
//! PATH it applied to itself), and for OMP also its verified host-local copy
//! and `~/.local/bin/omp`. Versions come from `--version` with a short
//! timeout, cached per executable, and never on the heartbeat: a monitor
//! thread refreshes the list every few minutes, after an install, and at
//! every daemon start (which a requested restart is).

use shared::{MachineProviderStatus, Provider};
use std::collections::HashMap;
use std::io::Read as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

const REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// The providers reported, in this order.
const PROVIDERS: [Provider; 5] = [
    Provider::Claude,
    Provider::Codex,
    Provider::Opencode,
    Provider::Antigravity,
    Provider::Omp,
];

static REFRESH_REQUESTED: AtomicBool = AtomicBool::new(false);

fn latest_slot() -> &'static Mutex<Option<Vec<MachineProviderStatus>>> {
    static LATEST: OnceLock<Mutex<Option<Vec<MachineProviderStatus>>>> = OnceLock::new();
    LATEST.get_or_init(Default::default)
}

/// Ask the monitor to look again soon (after an install, for instance).
pub fn request_refresh() {
    REFRESH_REQUESTED.store(true, Ordering::SeqCst);
}

/// The newest list the monitor found, if it has run.
pub fn latest() -> Option<Vec<MachineProviderStatus>> {
    latest_slot().lock().ok()?.clone()
}

/// The list without versions: only file checks, fast enough for startup.
pub fn quick(config: &crate::config::Config) -> Vec<MachineProviderStatus> {
    detect(config, &mut |_| None, &crate::omp_local::verified_copy)
}

/// Start the thread that keeps [`latest`] current. Call once, after the
/// daemon has applied its PATH.
pub fn start_monitor() {
    let spawned = std::thread::Builder::new()
        .name("provider-availability".into())
        .spawn(|| {
            let mut cache = VersionCache::default();
            loop {
                let config = crate::config::Config::load().unwrap_or_default();
                let found = detect(
                    &config,
                    &mut |path| cache.version(path),
                    &crate::omp_local::verified_copy,
                );
                if let Ok(mut slot) = latest_slot().lock() {
                    *slot = Some(found);
                }
                let started = Instant::now();
                while started.elapsed() < REFRESH_INTERVAL
                    && !REFRESH_REQUESTED.swap(false, Ordering::SeqCst)
                {
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "could not start the provider availability monitor");
    }
}

fn configured_path(config: &crate::config::Config, provider: Provider) -> &str {
    match provider {
        Provider::Claude => &config.local.claude_path,
        Provider::Codex => &config.local.codex_path,
        Provider::Opencode => &config.local.opencode_path,
        Provider::Antigravity => &config.local.antigravity_path,
        Provider::Omp => &config.local.omp_path,
        _ => "",
    }
}

/// `which`, as a launch resolves a bare name.
fn resolve(name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    if name.contains('/') {
        let path = PathBuf::from(name);
        return crate::omp_install::is_executable_file(&path).then_some(path);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| crate::omp_install::is_executable_file(candidate))
}

fn detect(
    config: &crate::config::Config,
    version_of: &mut dyn FnMut(&Path) -> Option<String>,
    verified_omp: &dyn Fn() -> Option<(PathBuf, String)>,
) -> Vec<MachineProviderStatus> {
    detect_with(
        config,
        version_of,
        verified_omp,
        crate::omp_install::default_install_path().as_deref(),
        crate::omp_install::host_platform().is_ok(),
    )
}

/// [`detect`] with the official install place and platform support given,
/// so tests never look at the real `~/.local/bin/omp`.
fn detect_with(
    config: &crate::config::Config,
    version_of: &mut dyn FnMut(&Path) -> Option<String>,
    verified_omp: &dyn Fn() -> Option<(PathBuf, String)>,
    default_install: Option<&Path>,
    platform_supported: bool,
) -> Vec<MachineProviderStatus> {
    PROVIDERS
        .into_iter()
        .map(|provider| {
            let configured = configured_path(config, provider);
            let mut found = resolve(configured);
            let mut version = None;
            let mut can_install = false;
            if provider == Provider::Antigravity && configured == "agy" && found.is_none() {
                found = crate::terminal_pane::installed_antigravity();
                can_install = found.is_none()
                    && cfg!(all(
                        any(target_os = "linux", target_os = "macos"),
                        any(target_arch = "aarch64", target_arch = "x86_64")
                    ));
            }
            if provider == Provider::Omp && found.is_none() {
                // What a launch would use instead: the verified host-local
                // copy (its version is already known), or the official
                // install's place when it is not on PATH.
                let verified = verified_omp();
                let has_verified_copy = verified.is_some();
                if let Some((copy, copy_version)) = verified {
                    found = Some(copy);
                    version = Some(copy_version);
                } else if !configured.contains('/')
                    || default_install.is_some_and(|default| Path::new(configured) == default)
                {
                    found = default_install
                        .filter(|path| crate::omp_install::is_executable_file(path))
                        .map(Path::to_path_buf);
                }
                // Offer an install only where a launch would really make
                // one: the same decision, from the same function.
                can_install = found.is_none()
                    && platform_supported
                    && crate::omp_install::plan_omp_launch(
                        configured,
                        default_install,
                        has_verified_copy,
                    ) == crate::omp_install::OmpLaunchPlan::Install;
            }
            if version.is_none() {
                version = found.as_deref().and_then(&mut *version_of);
            }
            MachineProviderStatus {
                provider,
                installed: found.is_some(),
                version,
                can_install,
            }
        })
        .collect()
}

/// `--version` answers, kept while the executable is unchanged.
#[derive(Default)]
struct VersionCache {
    entries: HashMap<PathBuf, (u64, Option<SystemTime>, Option<String>)>,
}

impl VersionCache {
    fn version(&mut self, path: &Path) -> Option<String> {
        let meta = std::fs::metadata(path).ok()?;
        let key = (meta.len(), meta.modified().ok());
        if let Some((len, modified, version)) = self.entries.get(path) {
            if (*len, *modified) == key {
                return version.clone();
            }
        }
        let version = ask_version(path);
        // Only an answer is kept: a probe that timed out (a cold NFS home)
        // is asked again next time, not hidden until the binary changes.
        if version.is_some() {
            self.entries
                .insert(path.to_path_buf(), (key.0, key.1, version.clone()));
        }
        version
    }
}

fn ask_version(path: &Path) -> Option<String> {
    let spawn = || {
        Command::new(path)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
    };
    // An executable just written (an install, an update) can be briefly
    // "busy" while another thread's fork still holds it open.
    let mut attempt = 0;
    let mut child = loop {
        match spawn() {
            Ok(child) => break child,
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 5 => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return None,
        }
    };
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = std::io::Read::read_to_end(&mut (&mut stdout).take(4096), &mut out);
        out
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return None,
            Ok(None) if start.elapsed() < VERSION_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                // The whole group: a launcher may have started the real
                // program as a child.
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    version_from_output(&String::from_utf8_lossy(&reader.join().ok()?))
}

/// The first word that looks like a version: `2.1.233 (Claude Code)`,
/// `codex-cli 0.157.0`, `omp/18.20.2`, `v1.2.3`.
fn version_from_output(output: &str) -> Option<String> {
    output.split_whitespace().find_map(|word| {
        let word = word.rsplit('/').next().unwrap_or(word);
        let word = word.strip_prefix('v').unwrap_or(word);
        let word = word.trim_end_matches([',', ';', ')']);
        let mut parts = word.split('.');
        let numeric = |part: Option<&str>| {
            part.is_some_and(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        };
        (numeric(parts.next()) && numeric(parts.next()) && word.len() <= 40)
            .then(|| word.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn versions_are_read_from_each_providers_answer() {
        assert_eq!(
            version_from_output("2.1.233 (Claude Code)\n").as_deref(),
            Some("2.1.233")
        );
        assert_eq!(
            version_from_output("codex-cli 0.157.0").as_deref(),
            Some("0.157.0")
        );
        assert_eq!(
            version_from_output("omp/18.20.2").as_deref(),
            Some("18.20.2")
        );
        assert_eq!(version_from_output("v1.4.0").as_deref(), Some("1.4.0"));
        assert_eq!(version_from_output("no version here"), None);
    }

    #[test]
    fn a_version_is_asked_once_per_unchanged_executable() {
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("asked");
        let binary = dir.path().join("tool");
        std::fs::write(
            &binary,
            format!(
                "#!/bin/sh\necho x >> '{}'\necho 'tool 3.2.1'\n",
                counter.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut cache = VersionCache::default();
        assert_eq!(cache.version(&binary).as_deref(), Some("3.2.1"));
        assert_eq!(cache.version(&binary).as_deref(), Some("3.2.1"));
        assert_eq!(
            std::fs::read_to_string(&counter).unwrap().lines().count(),
            1
        );
    }

    #[test]
    fn a_version_that_could_not_be_read_is_asked_again() {
        let dir = tempfile::tempdir().unwrap();
        let counter = dir.path().join("asked");
        let binary = dir.path().join("tool");
        std::fs::write(
            &binary,
            format!("#!/bin/sh\necho x >> '{}'\nexit 1\n", counter.display()),
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut cache = VersionCache::default();
        assert_eq!(cache.version(&binary), None);
        assert_eq!(cache.version(&binary), None);
        assert_eq!(
            std::fs::read_to_string(&counter).unwrap().lines().count(),
            2
        );
    }

    #[test]
    fn a_version_that_hangs_is_given_up_on() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("slow");
        std::fs::write(&binary, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let start = Instant::now();
        assert_eq!(ask_version(&binary), None);
        assert!(start.elapsed() < Duration::from_secs(15));
    }

    #[test]
    fn every_provider_is_reported_and_custom_paths_are_respected() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude");
        std::fs::write(&claude, "#!/bin/sh\necho '2.1.233 (Claude Code)'\n").unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut config = crate::config::Config::default();
        config.local.claude_path = claude.to_string_lossy().into_owned();
        config.local.codex_path = dir
            .path()
            .join("missing-codex")
            .to_string_lossy()
            .into_owned();
        config.local.antigravity_path = dir.path().join("missing-agy").display().to_string();
        config.local.opencode_path = dir
            .path()
            .join("missing-opencode")
            .to_string_lossy()
            .into_owned();
        // An explicit path that is not there: never looked for elsewhere.
        config.local.omp_path = dir
            .path()
            .join("missing-omp")
            .to_string_lossy()
            .into_owned();
        let mut asked = Vec::new();
        let found = detect(
            &config,
            &mut |path| {
                asked.push(path.to_path_buf());
                ask_version(path)
            },
            &|| None,
        );
        let providers: Vec<_> = found.iter().map(|status| status.provider).collect();
        assert_eq!(providers, PROVIDERS.to_vec());
        assert!(found[0].installed);
        assert_eq!(found[0].version.as_deref(), Some("2.1.233"));
        assert!(!found[1].installed && found[1].version.is_none());
        assert!(found.iter().take(4).all(|status| !status.can_install));
        // A launch never installs over an explicit path elsewhere, so
        // nothing offers to.
        assert!(!found[4].installed && !found[4].can_install);
        assert_eq!(asked, vec![claude], "versions only for what was found");

        let quick = detect(&config, &mut |_| None, &|| None);
        assert!(quick.iter().all(|status| status.version.is_none()));
        assert!(quick[0].installed);

        // A verified host-local OMP counts, with the version it was
        // published with, and is not asked again.
        let copy = dir.path().join("omp-copy");
        let mut asked_omp = false;
        let found = detect(
            &config,
            &mut |path| {
                asked_omp |= path == copy;
                None
            },
            &|| Some((copy.clone(), "18.20.2".to_string())),
        );
        assert!(!asked_omp);
        assert!(found[4].installed);
        assert_eq!(found[4].version.as_deref(), Some("18.20.2"));
    }

    #[test]
    fn an_install_is_offered_only_where_a_launch_would_make_one() {
        let dir = tempfile::tempdir().unwrap();
        let default_install = dir.path().join("bin").join("omp");
        let mut config = crate::config::Config::default();
        let omp = |config: &crate::config::Config, supported: bool| {
            detect_with(
                config,
                &mut |_| None,
                &|| None,
                Some(&default_install),
                supported,
            )
            .into_iter()
            .find(|status| status.provider == Provider::Omp)
            .unwrap()
        };

        // A bare name found nowhere, and the official place itself.
        config.local.omp_path = "apas-test-omp-that-is-nowhere".into();
        let status = omp(&config, true);
        assert!(!status.installed && status.can_install);
        config.local.omp_path = default_install.to_string_lossy().into_owned();
        assert!(omp(&config, true).can_install);

        // An unsupported platform offers nothing.
        assert!(!omp(&config, false).can_install);

        // An explicit path elsewhere is never installed over.
        config.local.omp_path = dir.path().join("custom-omp").to_string_lossy().into_owned();
        let status = omp(&config, true);
        assert!(!status.installed && !status.can_install);

        // Once installed in the official place, nothing more to offer.
        std::fs::create_dir_all(default_install.parent().unwrap()).unwrap();
        std::fs::write(&default_install, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&default_install, std::fs::Permissions::from_mode(0o755)).unwrap();
        config.local.omp_path = "apas-test-omp-that-is-nowhere".into();
        let status = omp(&config, true);
        assert!(status.installed && !status.can_install);
    }
}
