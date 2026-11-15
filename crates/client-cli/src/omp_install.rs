//! Installing OMP on a machine that has none, for the OMP tab that needs it.
//!
//! An OMP tab opened on a machine without OMP used to show an empty terminal
//! and nothing else. Now the CLI installs OMP first, the way a person would
//! with the official install, and then starts the tab:
//!
//! - It downloads the release asset for this platform from the project's
//!   GitHub release over HTTPS. It never runs a downloaded script.
//! - The version and every asset's SHA-256 and size are pinned in this file.
//!   The download must match them, and the release's own `SHA256SUMS.txt`
//!   must list the same hash, so a changed release or a bad mirror is refused
//!   rather than trusted. On macOS the binary's Developer ID signature is
//!   checked too, the one check there that names who built it.
//! - It must report the pinned version when run, and is then put at
//!   `~/.local/bin/omp`, where the official install puts it, so `omp` works
//!   in the person's own shell as on the Linux hosts. No shell rc file is
//!   edited, and an `omp` already there is never replaced: the file is
//!   published by a hard link, which fails if one exists. From there the
//!   host-local copy in `omp_local` takes over, as for any install.
//! - One install runs at a time on a machine (a lock beside the host-local
//!   store), bounded in time. A tab that needs OMP while another installs
//!   waits for that install instead of starting a second.
//!
//! Only OMP is ever installed, and only for an OMP tab. A Mac needs nothing
//! about quarantine: a file downloaded by a program that does not set
//! `com.apple.quarantine` (reqwest does not) is never stopped by Gatekeeper,
//! which is also what the official install relies on.
//!
//! Raising [`OMP_VERSION`] needs the new release's hashes and sizes from its
//! `SHA256SUMS.txt`, and the real-OMP check AGENTS.md asks for on a minor
//! version change.

use anyhow::{bail, Context, Result};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The OMP release APAS installs. 18.20.x is the minor version autopilot was
/// checked against (18.18.2 is its minimum).
pub const OMP_VERSION: &str = "18.20.2";

/// Where releases are downloaded from: `<base>/v<version>/<asset>`.
pub const OFFICIAL_RELEASES: &str = "https://github.com/can1357/oh-my-pi/releases/download";

/// What a person runs to install OMP themselves.
pub const MANUAL_INSTALL: &str = "curl -fsSL https://omp.sh/install | sh";

/// The team that signs the macOS releases ("Developer ID Application: Can
/// Boluk (VTLHGQC72S)").
const MACOS_TEAM_ID: &str = "VTLHGQC72S";

/// One pinned release asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub sha256: String,
    pub size: u64,
}

/// `SHA256SUMS.txt` of OMP 18.20.2, for the platforms APAS installs on.
const PINNED_ASSETS: [(&str, &str, u64); 4] = [
    (
        "omp-darwin-arm64",
        "e2de0974ce581bfcf2044ea795c712fa240fdf8304f7bdb4b3d81b154d2b57fc",
        224_277_648,
    ),
    (
        "omp-darwin-x64",
        "4896674a500de3e0aed700077c67739ce6aea5ecd61d100fff174143daca693b",
        236_602_832,
    ),
    (
        "omp-linux-arm64",
        "7f809e51638c2ace701244923d5ec62485bbafd74db75da742ace30c87a91ebf",
        245_419_000,
    ),
    (
        "omp-linux-x64",
        "f34cfa57311ed67a45f01bb1a21519cad4d74875d181316f00fa0cd6a4abcaaa",
        288_860_336,
    ),
];

/// A platform OMP can be installed on, as the release names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
    /// `darwin` or `linux`, as in the asset name.
    pub os: &'static str,
    /// `arm64` or `x64`.
    pub arch: &'static str,
}

impl Platform {
    pub fn asset_name(&self) -> String {
        format!("omp-{}-{}", self.os, self.arch)
    }

    /// How the person reads it: `macos/arm64`.
    pub fn label(&self) -> String {
        let os = if self.os == "darwin" {
            "macos"
        } else {
            self.os
        };
        format!("{os}/{}", self.arch)
    }

    pub fn pinned_asset(&self) -> Option<Asset> {
        let name = self.asset_name();
        PINNED_ASSETS
            .iter()
            .find(|(asset, ..)| *asset == name)
            .map(|(name, sha256, size)| Asset {
                name: name.to_string(),
                sha256: sha256.to_string(),
                size: *size,
            })
    }
}

/// The platform this host would install OMP for, or why there is none.
pub fn host_platform() -> std::result::Result<Platform, String> {
    platform_for(
        std::env::consts::OS,
        std::env::consts::ARCH,
        mac_has_arm64,
        linux_is_musl,
    )
}

fn platform_for(
    os: &str,
    arch: &str,
    mac_has_arm64: impl Fn() -> bool,
    linux_is_musl: impl Fn() -> bool,
) -> std::result::Result<Platform, String> {
    let unsupported = || format!("APAS cannot install OMP for {os}/{arch}");
    match os {
        "macos" => {
            let arch = match arch {
                "aarch64" => "arm64",
                // An x86_64 APAS can be running under Rosetta on Apple
                // silicon; the native OMP is the arm64 one there.
                "x86_64" if mac_has_arm64() => "arm64",
                "x86_64" => "x64",
                _ => return Err(unsupported()),
            };
            Ok(Platform { os: "darwin", arch })
        }
        "linux" => {
            if linux_is_musl() {
                return Err(format!(
                    "APAS installs the glibc build of OMP only, and this {arch} host uses musl"
                ));
            }
            let arch = match arch {
                "x86_64" => "x64",
                "aarch64" => "arm64",
                _ => return Err(unsupported()),
            };
            Ok(Platform { os: "linux", arch })
        }
        _ => Err(unsupported()),
    }
}

/// `sysctl hw.optional.arm64`, as the official install asks. Stays 1 under
/// Rosetta, unlike `uname -m`.
fn mac_has_arm64() -> bool {
    Command::new("/usr/sbin/sysctl")
        .args(["-in", "hw.optional.arm64"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).trim() == "1")
}

/// The APAS CLI is itself static, so its own build says nothing about the
/// host's C library. The glibc OMP cannot start on a musl host.
fn linux_is_musl() -> bool {
    linux_libc_is_musl_in(Path::new("/"), std::env::consts::ARCH)
}

/// Which C library the system under `root` uses, decided by its loaders.
///
/// A musl loader alone proves nothing: Debian's `musl-tools`, which every zoo
/// host has for the static CLI build, installs `/lib/ld-musl-x86_64.so.1`
/// beside glibc's. So the glibc loader decides first, and only Alpine is
/// taken as musl ahead of it, because its `gcompat` puts a glibc-named shim
/// there. When nothing is conclusive the answer is glibc: a wrong "glibc"
/// costs one download the version probe then refuses, a wrong "musl" refuses
/// every install.
fn linux_libc_is_musl_in(root: &Path, arch: &str) -> bool {
    if root.join("etc/alpine-release").exists() {
        return true;
    }
    let glibc_loaders: &[&str] = match arch {
        "x86_64" => &["lib64/ld-linux-x86-64.so.2", "lib/ld-linux-x86-64.so.2"],
        "aarch64" => &["lib/ld-linux-aarch64.so.1", "lib64/ld-linux-aarch64.so.1"],
        _ => &[],
    };
    if glibc_loaders
        .iter()
        .any(|loader| root.join(loader).exists())
    {
        return false;
    }
    fs::read_dir(root.join("lib")).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("ld-musl-"))
    })
}

/// `~/.local/bin`, where the official install puts `omp`.
pub fn default_install_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".local").join("bin"))
}

pub fn default_install_path() -> Option<PathBuf> {
    default_install_dir().map(|dir| dir.join("omp"))
}

pub(crate) fn is_executable_file(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// What an OMP launch should do about its executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OmpLaunchPlan {
    /// Launch with this path. It may still fail; that failure is reported.
    Launch(String),
    /// OMP is not on this machine: install it first.
    Install,
}

/// Decide from `binary_path`, the configured `omp_path` as resolved through
/// PATH (left as the bare name when `which` found nothing). A verified
/// host-local copy launches as before, even when the source is gone. An
/// explicitly configured path elsewhere is never replaced by an install: its
/// launch fails and says how to fix it.
pub fn plan_omp_launch(
    binary_path: &str,
    default_install: Option<&Path>,
    has_verified_copy: bool,
) -> OmpLaunchPlan {
    if is_executable_file(Path::new(binary_path)) || has_verified_copy {
        return OmpLaunchPlan::Launch(binary_path.to_string());
    }
    if let Some(default_install) = default_install {
        if is_executable_file(default_install) {
            // Installed where the official install puts it, but that
            // directory is not on the daemon's PATH.
            return OmpLaunchPlan::Launch(default_install.to_string_lossy().into_owned());
        }
        let bare = !binary_path.contains('/');
        if bare || Path::new(binary_path) == default_install {
            return OmpLaunchPlan::Install;
        }
    }
    OmpLaunchPlan::Launch(binary_path.to_string())
}

/// The plan for this host, from the resolved `omp_path`.
pub fn plan_for_host(binary_path: &str) -> OmpLaunchPlan {
    plan_omp_launch(
        binary_path,
        default_install_path().as_deref(),
        crate::omp_local::verified_copy().is_some(),
    )
}

/// Everything one install needs. Production uses [`InstallRequest::official`];
/// tests point it at a local server and a temporary directory.
pub struct InstallRequest {
    pub base_url: String,
    pub version: String,
    pub platform: Platform,
    pub asset: Asset,
    /// The directory `omp` is published in.
    pub install_dir: PathBuf,
    /// Held for the whole install. Opened by the caller.
    pub lock: File,
    /// How long to wait for another install holding the lock.
    pub lock_wait: Duration,
    /// The whole download, verification included.
    pub timeout: Duration,
    /// Longest wait for the next downloaded bytes.
    pub stall_timeout: Duration,
    /// Check the macOS code signature (only meaningful on macOS).
    pub verify_macos_signature: bool,
}

impl InstallRequest {
    pub fn official() -> Result<Self> {
        let platform = host_platform().map_err(anyhow::Error::msg)?;
        let asset = platform
            .pinned_asset()
            .with_context(|| format!("no pinned OMP asset for {}", platform.label()))?;
        Ok(Self {
            base_url: OFFICIAL_RELEASES.to_string(),
            version: OMP_VERSION.to_string(),
            platform,
            asset,
            install_dir: default_install_dir().context("the home directory is unknown")?,
            lock: crate::omp_local::install_lock()?,
            lock_wait: Duration::from_secs(20 * 60),
            timeout: Duration::from_secs(15 * 60),
            stall_timeout: Duration::from_secs(60),
            verify_macos_signature: cfg!(target_os = "macos"),
        })
    }

    pub fn install_path(&self) -> PathBuf {
        self.install_dir.join("omp")
    }
}

/// The first line a person sees in the tab.
pub fn starting_line(version: &str, platform: &Platform) -> String {
    format!("Installing OMP {version} for {}...", platform.label())
}

/// What a person can do when the install did not work.
pub fn failure_text(host: &str, error: &str) -> String {
    format!(
        "OMP is not installed on {host}, and APAS could not install it: {error}. \
         Install it on {host} with: {MANUAL_INSTALL}. Then reboot this tab. \
         If OMP is installed somewhere else, set its path with: apas config set omp_path <path>"
    )
}

/// Install OMP as `request` describes, reporting progress lines to
/// `progress`. Returns the installed executable. Blocking: run it on a thread
/// of its own, never on an async task.
pub fn install(request: InstallRequest, progress: &mut dyn FnMut(String)) -> Result<PathBuf> {
    let waited = lock_with_wait(&request.lock, request.lock_wait, progress)?;
    // From here, not from the call: a tab that waited behind an install that
    // then failed gets its own whole download time, not what the wait left.
    let deadline = Instant::now() + request.timeout;
    let result = (|| {
        let target = request.install_path();
        if is_executable_file(&target) {
            // Installed while this one waited, or by the person meanwhile.
            if waited {
                progress("The other install finished.".to_string());
            }
            return Ok(target);
        }
        progress(starting_line(&request.version, &request.platform));
        install_locked(&request, deadline, progress)
    })();
    let _ = fs2::FileExt::unlock(&request.lock);
    if result.is_ok() {
        crate::provider_availability::request_refresh();
    }
    result
}

fn lock_with_wait(lock: &File, wait: Duration, progress: &mut dyn FnMut(String)) -> Result<bool> {
    let start = Instant::now();
    let mut told = false;
    loop {
        match fs2::FileExt::try_lock_exclusive(lock) {
            Ok(()) => return Ok(told),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if !told {
                    progress(
                        "Another OMP install is running on this machine. Waiting for it."
                            .to_string(),
                    );
                    told = true;
                }
                if start.elapsed() >= wait {
                    bail!(
                        "another OMP install on this machine did not finish within {}",
                        describe_duration(wait)
                    );
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(error) => return Err(error).context("lock the OMP install"),
        }
    }
}

fn install_locked(
    request: &InstallRequest,
    deadline: Instant,
    progress: &mut dyn FnMut(String),
) -> Result<PathBuf> {
    fs::create_dir_all(&request.install_dir)
        .with_context(|| format!("create {}", request.install_dir.display()))?;
    remove_abandoned_downloads(&request.install_dir, request.timeout);
    let temporary = request
        .install_dir
        .join(format!("{DOWNLOAD_PREFIX}{}", Uuid::new_v4().simple()));
    let result = (|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("start the download runtime")?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        runtime
            .block_on(async {
                tokio::time::timeout(remaining, download(request, &temporary, progress)).await
            })
            .map_err(|_| {
                anyhow::anyhow!(
                    "the download did not finish within {}",
                    describe_duration(request.timeout)
                )
            })??;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o755))?;
        if request.verify_macos_signature {
            verify_macos_signature(&temporary)?;
            progress(format!("Signature verified (team {MACOS_TEAM_ID})."));
        }
        let version = crate::omp_local::probe_version(&temporary, &request.install_dir)
            .context("the downloaded OMP does not start")?;
        if !crate::omp_local::same_version(&version, &request.version) {
            bail!(
                "the downloaded OMP reports version {version}, not {}",
                request.version
            );
        }
        let target = request.install_path();
        match fs::hard_link(&temporary, &target) {
            Ok(()) => {}
            // Someone else installed one meanwhile (another host on a shared
            // home). Theirs stays; never replace an `omp` that exists.
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                progress(format!(
                    "{} appeared meanwhile and was kept.",
                    target.display()
                ));
            }
            Err(error) => {
                return Err(error).with_context(|| format!("publish {}", target.display()))
            }
        }
        progress(format!(
            "Installed OMP {} at {}. Starting it now. Sign in to a model provider in OMP's setup or with /login.",
            request.version,
            target.display()
        ));
        Ok(target)
    })();
    let _ = fs::remove_file(&temporary);
    result
}

const DOWNLOAD_PREFIX: &str = ".omp.download.";

/// Remove the partial downloads an install left behind when it never got to
/// remove its own: a daemon reboot (`exec`), a crash or a kill mid-download.
/// Each is a few hundred MB, in `~/.local/bin`, which on the Linux hosts is
/// the shared NFS home. Only files older than a whole install's time limit
/// go: a younger one can be another host's download in progress on that
/// shared home, which this host's lock does not cover.
fn remove_abandoned_downloads(install_dir: &Path, max_age: Duration) {
    let Ok(entries) = fs::read_dir(install_dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(DOWNLOAD_PREFIX)
        {
            continue;
        }
        // Not followed: only a regular file of ours is removed.
        let Ok(meta) = entry.path().symlink_metadata() else {
            continue;
        };
        let abandoned = meta.is_file()
            && meta
                .modified()
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > max_age);
        if abandoned {
            match fs::remove_file(entry.path()) {
                Ok(()) => tracing::info!(
                    path = %entry.path().display(),
                    "removed a partial OMP download an interrupted install left"
                ),
                Err(error) => tracing::warn!(
                    path = %entry.path().display(),
                    %error,
                    "could not remove a partial OMP download"
                ),
            }
        }
    }
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("apas/", env!("APAS_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .build()?)
}

async fn download(
    request: &InstallRequest,
    temporary: &Path,
    progress: &mut dyn FnMut(String),
) -> Result<()> {
    let client = client()?;
    let base = request.base_url.trim_end_matches('/');
    let version = request.version.trim_start_matches('v');
    let sums_url = format!("{base}/v{version}/SHA256SUMS.txt");
    let published = published_checksum(
        &client,
        &sums_url,
        &request.asset.name,
        request.stall_timeout,
    )
    .await?;
    if !published.eq_ignore_ascii_case(&request.asset.sha256) {
        bail!(
            "the release's checksum for {} ({published}) is not the one this APAS expects",
            request.asset.name
        );
    }

    let url = format!("{base}/v{version}/{}", request.asset.name);
    let size_mb = request.asset.size / 1_000_000;
    progress(format!(
        "Downloading {} ({size_mb} MB).",
        request.asset.name
    ));
    let response = tokio::time::timeout(request.stall_timeout, client.get(&url).send())
        .await
        .map_err(|_| anyhow::anyhow!("no answer from the download server"))?
        .with_context(|| format!("download {url}"))?
        .error_for_status()
        .with_context(|| format!("download {url}"))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(temporary)
        .with_context(|| format!("create {}", temporary.display()))?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut reported_tenth = 0;
    let mut stream = response.bytes_stream();
    loop {
        let next = tokio::time::timeout(request.stall_timeout, stream.next())
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "the download stalled for {} seconds",
                    request.stall_timeout.as_secs()
                )
            })?;
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.context("the download was interrupted")?;
        received += chunk.len() as u64;
        if received > request.asset.size {
            bail!(
                "the download is larger than the {} bytes expected",
                request.asset.size
            );
        }
        hasher.update(&chunk);
        file.write_all(&chunk)?;
        let tenth = received * 10 / request.asset.size.max(1);
        if tenth > reported_tenth && tenth < 10 {
            reported_tenth = tenth;
            progress(format!(
                "Downloaded {} of {size_mb} MB.",
                received / 1_000_000
            ));
        }
    }
    file.sync_all()?;
    drop(file);
    if received != request.asset.size {
        bail!(
            "the download ended after {received} of {} bytes",
            request.asset.size
        );
    }
    let actual = hex(&hasher.finalize());
    if !actual.eq_ignore_ascii_case(&request.asset.sha256) {
        bail!(
            "checksum mismatch for {}: expected {}, got {actual}",
            request.asset.name,
            request.asset.sha256
        );
    }
    progress("Checksum verified.".to_string());
    Ok(())
}

/// The hash `SHA256SUMS.txt` at `url` lists for `name`.
async fn published_checksum(
    client: &reqwest::Client,
    url: &str,
    name: &str,
    stall: Duration,
) -> Result<String> {
    let response = tokio::time::timeout(stall, client.get(url).send())
        .await
        .map_err(|_| anyhow::anyhow!("no answer from the download server"))?
        .with_context(|| format!("download {url}"))?
        .error_for_status()
        .with_context(|| format!("download {url}"))?;
    let body = tokio::time::timeout(stall, response.bytes())
        .await
        .map_err(|_| anyhow::anyhow!("the checksum list did not arrive"))??;
    if body.len() > 64 * 1024 {
        bail!("the release's checksum list is unexpectedly large");
    }
    parse_checksums(&String::from_utf8_lossy(&body), name)
        .with_context(|| format!("the release's checksum list does not name {name}"))
}

/// `<sha256>  <name>` lines, as `sha256sum` writes them.
fn parse_checksums(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let file = parts.next()?.trim_start_matches('*');
        (file == name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

fn describe_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds >= 120 {
        format!("{} minutes", seconds / 60)
    } else {
        format!("{} seconds", seconds.max(1))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Run `command`, giving up after `timeout`. Returns stdout and stderr.
fn output_with_timeout(mut command: Command, timeout: Duration) -> Result<std::process::Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return Ok(child.wait_with_output()?);
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("did not finish within {} seconds", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// `codesign --verify --strict`, then the signing team. The release is
/// signed with a Developer ID and notarized; an ad-hoc or foreign signature
/// is refused. Never re-signs.
fn verify_macos_signature(path: &Path) -> Result<()> {
    let codesign = "/usr/bin/codesign";
    let mut verify = Command::new(codesign);
    verify.args(["--verify", "--strict"]).arg(path);
    let out = output_with_timeout(verify, Duration::from_secs(180))
        .context("check the code signature")?;
    if !out.status.success() {
        bail!(
            "the downloaded OMP's code signature is not valid: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let mut describe = Command::new(codesign);
    describe.args(["-dv", "--verbose=2"]).arg(path);
    let out = output_with_timeout(describe, Duration::from_secs(60))
        .context("read the code signature")?;
    // codesign describes on stderr.
    let text = String::from_utf8_lossy(&out.stderr);
    if !signed_by_team(&text, MACOS_TEAM_ID) {
        bail!("the downloaded OMP is not signed by OMP's developer (team {MACOS_TEAM_ID})");
    }
    Ok(())
}

fn signed_by_team(codesign_output: &str, team: &str) -> bool {
    codesign_output
        .lines()
        .any(|line| line.trim() == format!("TeamIdentifier={team}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A release server on 127.0.0.1 serving `files` by path; anything else
    /// is a 404. Records the paths asked for.
    struct FakeReleases {
        base_url: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    fn serve(files: Vec<(String, Vec<u8>)>) -> FakeReleases {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
                        break;
                    }
                }
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("")
                    .to_string();
                seen.lock().unwrap().push(path.clone());
                let response = match files.iter().find(|(served, _)| *served == path) {
                    Some((_, body)) => {
                        let mut head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        head.extend_from_slice(body);
                        head
                    }
                    None => {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    }
                };
                let _ = stream.write_all(&response);
            }
        });
        FakeReleases { base_url, requests }
    }

    /// A stand-in OMP: a script that reports `version`.
    fn fake_omp(version: &str) -> Vec<u8> {
        format!("#!/bin/sh\necho 'omp/{version}'\n").into_bytes()
    }

    fn sha(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    const PLATFORM: Platform = Platform {
        os: "linux",
        arch: "x64",
    };

    fn make_request(server: &FakeReleases, dir: &Path, binary: &[u8]) -> InstallRequest {
        let lock_path = dir.join("install.lock");
        InstallRequest {
            base_url: server.base_url.clone(),
            version: "18.20.2".into(),
            platform: PLATFORM,
            asset: Asset {
                name: "omp-linux-x64".into(),
                sha256: sha(binary),
                size: binary.len() as u64,
            },
            install_dir: dir.join("home/.local/bin"),
            lock: OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(lock_path)
                .unwrap(),
            lock_wait: Duration::from_secs(5),
            timeout: Duration::from_secs(30),
            stall_timeout: Duration::from_secs(10),
            verify_macos_signature: false,
        }
    }

    fn release(binary: &[u8], listed_hash: &str) -> Vec<(String, Vec<u8>)> {
        vec![
            (
                "/v18.20.2/SHA256SUMS.txt".into(),
                format!("0000000000000000000000000000000000000000000000000000000000000000  LICENSE\n{listed_hash}  omp-linux-x64\n").into_bytes(),
            ),
            ("/v18.20.2/omp-linux-x64".into(), binary.to_vec()),
        ]
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .filter(|name| name.starts_with(".omp.download"))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn installs_the_verified_release_and_reports_progress() {
        let dir = tempfile::tempdir().unwrap();
        let binary = fake_omp("18.20.2");
        let server = serve(release(&binary, &sha(&binary)));
        let request = make_request(&server, dir.path(), &binary);
        let target = request.install_path();
        let mut lines = Vec::new();
        let installed = install(request, &mut |line| lines.push(line)).expect("installed");
        assert_eq!(installed, target);
        assert_eq!(fs::read(&target).unwrap(), binary);
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            lines.first().unwrap(),
            "Installing OMP 18.20.2 for linux/x64..."
        );
        assert!(lines.iter().any(|line| line == "Checksum verified."));
        assert!(lines
            .last()
            .unwrap()
            .starts_with("Installed OMP 18.20.2 at "));
        assert!(leftovers(target.parent().unwrap()).is_empty());
        assert_eq!(
            *server.requests.lock().unwrap(),
            vec!["/v18.20.2/SHA256SUMS.txt", "/v18.20.2/omp-linux-x64"]
        );
    }

    #[test]
    fn a_download_that_does_not_match_its_checksum_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let expected = fake_omp("18.20.2");
        // Same size, different bytes: what a tampered mirror would serve.
        let mut served = expected.clone();
        let last = served.len() - 2;
        served[last] = b'x';
        let server = serve(release(&served, &sha(&expected)));
        let request = make_request(&server, dir.path(), &expected);
        let target = request.install_path();
        let error = install(request, &mut |_| {}).unwrap_err();
        assert!(
            format!("{error:#}").contains("checksum mismatch"),
            "{error:#}"
        );
        assert!(!target.exists(), "nothing is installed");
        assert!(
            leftovers(target.parent().unwrap()).is_empty(),
            "the partial file is removed"
        );
    }

    #[test]
    fn a_release_whose_published_checksum_changed_is_refused_before_downloading() {
        let dir = tempfile::tempdir().unwrap();
        let binary = fake_omp("18.20.2");
        let other = "1".repeat(64);
        let server = serve(release(&binary, &other));
        let request = make_request(&server, dir.path(), &binary);
        let target = request.install_path();
        let error = install(request, &mut |_| {}).unwrap_err();
        assert!(
            format!("{error:#}").contains("is not the one this APAS expects"),
            "{error:#}"
        );
        assert!(!target.exists());
        assert_eq!(
            *server.requests.lock().unwrap(),
            vec!["/v18.20.2/SHA256SUMS.txt"]
        );
    }

    #[test]
    fn a_failed_download_installs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let binary = fake_omp("18.20.2");
        // The checksum list is there; the asset is not.
        let mut files = release(&binary, &sha(&binary));
        files.pop();
        let server = serve(files);
        let request = make_request(&server, dir.path(), &binary);
        let target = request.install_path();
        let error = install(request, &mut |_| {}).unwrap_err();
        assert!(format!("{error:#}").contains("404"), "{error:#}");
        assert!(!target.exists());

        // Nothing listening at all.
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let unreachable = FakeReleases {
            base_url: format!("http://{}", closed.local_addr().unwrap()),
            requests: Arc::default(),
        };
        drop(closed);
        let request = make_request(&unreachable, dir.path(), &binary);
        assert!(install(request, &mut |_| {}).is_err());
        assert!(!target.exists());
    }

    #[test]
    fn a_download_reporting_another_version_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let binary = fake_omp("18.19.0");
        let server = serve(release(&binary, &sha(&binary)));
        let request = make_request(&server, dir.path(), &binary);
        let target = request.install_path();
        let error = install(request, &mut |_| {}).unwrap_err();
        assert!(
            format!("{error:#}").contains("reports version 18.19.0"),
            "{error:#}"
        );
        assert!(!target.exists());
    }

    #[test]
    fn an_existing_omp_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let binary = fake_omp("18.20.2");
        let server = serve(release(&binary, &sha(&binary)));
        let request = make_request(&server, dir.path(), &binary);
        let target = request.install_path();
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, b"#!/bin/sh\necho mine\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        let installed = install(request, &mut |_| {}).unwrap();
        assert_eq!(installed, target);
        assert_eq!(fs::read(&target).unwrap(), b"#!/bin/sh\necho mine\n");
        assert!(
            server.requests.lock().unwrap().is_empty(),
            "nothing was downloaded"
        );
    }

    #[test]
    fn a_second_install_waits_for_the_first_and_uses_its_result() {
        let dir = tempfile::tempdir().unwrap();
        let binary = fake_omp("18.20.2");
        let server = serve(release(&binary, &sha(&binary)));
        let holder = make_request(&server, dir.path(), &binary);
        fs2::FileExt::lock_exclusive(&holder.lock).unwrap();
        let target = holder.install_path();

        let waiting = make_request(&server, dir.path(), &binary);
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut lines = Vec::new();
            let result = install(waiting, &mut |line| {
                let _ = tx.send(line.clone());
                lines.push(line);
            });
            (result, lines)
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            "Another OMP install is running on this machine. Waiting for it."
        );
        // The first install finishes while the second waits.
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, &binary).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        fs2::FileExt::unlock(&holder.lock).unwrap();
        let (result, lines) = thread.join().unwrap();
        assert_eq!(result.unwrap(), target);
        assert_eq!(lines.last().unwrap(), "The other install finished.");
        assert!(
            server.requests.lock().unwrap().is_empty(),
            "no second download"
        );

        // A lock that is never released is given up on, with the reason.
        let stuck = make_request(&server, dir.path(), &binary);
        fs2::FileExt::lock_exclusive(&holder.lock).unwrap();
        let mut stuck = stuck;
        stuck.lock_wait = Duration::from_millis(300);
        fs::remove_file(&target).unwrap();
        let error = install(stuck, &mut |_| {}).unwrap_err();
        assert!(format!("{error:#}").contains("did not finish"), "{error:#}");
    }

    #[test]
    fn a_tab_that_waited_behind_a_failed_install_gets_its_whole_download_time() {
        let dir = tempfile::tempdir().unwrap();
        let binary = fake_omp("18.20.2");
        let server = serve(release(&binary, &sha(&binary)));
        let holder = make_request(&server, dir.path(), &binary);
        fs2::FileExt::lock_exclusive(&holder.lock).unwrap();
        let mut waiting = make_request(&server, dir.path(), &binary);
        waiting.lock_wait = Duration::from_secs(10);
        // Shorter than the wait below: timed from the call, nothing is left.
        waiting.timeout = Duration::from_secs(2);
        let target = waiting.install_path();
        let thread = std::thread::spawn(move || install(waiting, &mut |_| {}));
        std::thread::sleep(Duration::from_secs(3));
        // The first install failed: it releases the lock with nothing there.
        fs2::FileExt::unlock(&holder.lock).unwrap();
        let installed = thread.join().unwrap().expect("its own download");
        assert_eq!(installed, target);
        assert_eq!(fs::read(&target).unwrap(), binary);
    }

    #[test]
    fn partial_downloads_an_interrupted_install_left_are_removed_but_live_ones_kept() {
        let dir = tempfile::tempdir().unwrap();
        let binary = fake_omp("18.20.2");
        let server = serve(release(&binary, &sha(&binary)));
        let request = make_request(&server, dir.path(), &binary);
        let install_dir = request.install_dir.clone();
        fs::create_dir_all(&install_dir).unwrap();
        let abandoned = install_dir.join(".omp.download.0123abandoned");
        let live = install_dir.join(".omp.download.4567live");
        let unrelated = install_dir.join("notes.omp.download.old");
        for path in [&abandoned, &live, &unrelated] {
            fs::write(path, b"partial").unwrap();
        }
        let old = std::time::SystemTime::now() - request.timeout * 2;
        for path in [&abandoned, &unrelated] {
            File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        install(request, &mut |_| {}).expect("installed");
        assert!(!abandoned.exists(), "older than an install's time limit");
        assert!(live.exists(), "may be another host's download in progress");
        assert!(unrelated.exists(), "not a download's name");
        assert_eq!(leftovers(&install_dir), vec![".omp.download.4567live"]);
    }

    #[test]
    fn platforms_are_named_as_the_release_names_them() {
        let never = || false;
        let always = || true;
        let mac = platform_for("macos", "aarch64", never, never).unwrap();
        assert_eq!(mac.asset_name(), "omp-darwin-arm64");
        assert_eq!(mac.label(), "macos/arm64");
        assert_eq!(
            starting_line("18.20.2", &mac),
            "Installing OMP 18.20.2 for macos/arm64..."
        );
        // Rosetta: an Intel APAS on Apple silicon installs the arm64 OMP.
        assert_eq!(
            platform_for("macos", "x86_64", always, never).unwrap().arch,
            "arm64"
        );
        assert_eq!(
            platform_for("macos", "x86_64", never, never).unwrap().arch,
            "x64"
        );
        assert_eq!(
            platform_for("linux", "x86_64", never, never)
                .unwrap()
                .asset_name(),
            "omp-linux-x64"
        );
        assert_eq!(
            platform_for("linux", "aarch64", never, never).unwrap().arch,
            "arm64"
        );
        for platform in [mac, platform_for("linux", "x86_64", never, never).unwrap()] {
            let asset = platform.pinned_asset().expect("pinned");
            assert_eq!(asset.sha256.len(), 64);
            assert!(asset.size > 100_000_000);
        }
    }

    #[test]
    fn unsupported_platforms_are_refused_with_the_reason() {
        let never = || false;
        assert_eq!(
            platform_for("windows", "x86_64", never, never).unwrap_err(),
            "APAS cannot install OMP for windows/x86_64"
        );
        assert_eq!(
            platform_for("linux", "riscv64", never, never).unwrap_err(),
            "APAS cannot install OMP for linux/riscv64"
        );
        assert!(platform_for("linux", "x86_64", never, || true)
            .unwrap_err()
            .contains("musl"));
        assert_eq!(
            platform_for("freebsd", "x86_64", never, never).unwrap_err(),
            "APAS cannot install OMP for freebsd/x86_64"
        );
    }

    #[test]
    fn the_c_library_is_decided_by_the_glibc_loader_before_a_musl_one() {
        fn root_with(files: &[&str]) -> tempfile::TempDir {
            let root = tempfile::tempdir().unwrap();
            for file in files {
                let path = root.path().join(file);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, b"").unwrap();
            }
            root
        }
        // A glibc host with musl-tools installed: every zoo host.
        let both = root_with(&[
            "lib64/ld-linux-x86-64.so.2",
            "lib/ld-musl-x86_64.so.1",
            "usr/lib/ld-musl-x86_64.so.1",
        ]);
        assert!(!linux_libc_is_musl_in(both.path(), "x86_64"));
        let arm_both = root_with(&["lib/ld-linux-aarch64.so.1", "lib/ld-musl-aarch64.so.1"]);
        assert!(!linux_libc_is_musl_in(arm_both.path(), "aarch64"));
        // musl only.
        let musl = root_with(&["lib/ld-musl-x86_64.so.1"]);
        assert!(linux_libc_is_musl_in(musl.path(), "x86_64"));
        // Alpine with gcompat's glibc-named shim is still musl.
        let alpine = root_with(&[
            "etc/alpine-release",
            "lib/ld-linux-x86-64.so.2",
            "lib/ld-musl-x86_64.so.1",
        ]);
        assert!(linux_libc_is_musl_in(alpine.path(), "x86_64"));
        // Nothing conclusive: glibc.
        let neither = root_with(&[]);
        assert!(!linux_libc_is_musl_in(neither.path(), "x86_64"));
    }

    /// The real answer on this machine, where the test runs on a glibc host
    /// (with or without musl-tools beside it).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_glibc_linux_host_can_install_omp() {
        let loader = match std::env::consts::ARCH {
            "x86_64" => "/lib64/ld-linux-x86-64.so.2",
            "aarch64" => "/lib/ld-linux-aarch64.so.1",
            _ => return,
        };
        if Path::new(loader).exists() && !Path::new("/etc/alpine-release").exists() {
            assert!(!linux_is_musl());
            assert!(host_platform().is_ok(), "{:?}", host_platform());
        }
    }

    #[test]
    fn the_launch_installs_only_when_omp_is_nowhere() {
        let dir = tempfile::tempdir().unwrap();
        let default_install = dir.path().join(".local/bin/omp");
        // Not on PATH, nothing installed, no verified copy.
        assert_eq!(
            plan_omp_launch("omp", Some(&default_install), false),
            OmpLaunchPlan::Install
        );
        assert_eq!(
            plan_omp_launch(
                &default_install.to_string_lossy(),
                Some(&default_install),
                false
            ),
            OmpLaunchPlan::Install
        );
        // A verified host-local copy launches even without the source.
        assert_eq!(
            plan_omp_launch("omp", Some(&default_install), true),
            OmpLaunchPlan::Launch("omp".into())
        );
        // An explicitly configured path elsewhere is never replaced.
        assert_eq!(
            plan_omp_launch("/opt/omp/bin/omp", Some(&default_install), false),
            OmpLaunchPlan::Launch("/opt/omp/bin/omp".into())
        );
        // Installed in ~/.local/bin, which is not on the daemon's PATH.
        fs::create_dir_all(default_install.parent().unwrap()).unwrap();
        fs::write(&default_install, b"#!/bin/sh\n").unwrap();
        fs::set_permissions(&default_install, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            plan_omp_launch("omp", Some(&default_install), false),
            OmpLaunchPlan::Launch(default_install.to_string_lossy().into_owned())
        );
        // Found on PATH.
        assert_eq!(
            plan_omp_launch(&default_install.to_string_lossy(), None, false),
            OmpLaunchPlan::Launch(default_install.to_string_lossy().into_owned())
        );
    }

    #[test]
    fn checksum_lists_and_signatures_are_read_exactly() {
        let list = "abc  omp\n".to_string()
            + &"a".repeat(64)
            + "  omp-darwin-arm64\n"
            + &"b".repeat(64)
            + " *omp-linux-x64\n";
        assert_eq!(
            parse_checksums(&list, "omp-darwin-arm64"),
            Some("a".repeat(64))
        );
        assert_eq!(
            parse_checksums(&list, "omp-linux-x64"),
            Some("b".repeat(64))
        );
        assert_eq!(parse_checksums(&list, "omp-darwin"), None);
        assert_eq!(
            parse_checksums(&list, "omp"),
            None,
            "a short hash is not one"
        );

        let described = "Executable=/x/omp\nIdentifier=omp\nTeamIdentifier=VTLHGQC72S\n";
        assert!(signed_by_team(described, "VTLHGQC72S"));
        assert!(!signed_by_team("TeamIdentifier=not set\n", "VTLHGQC72S"));
        assert!(!signed_by_team(
            "TeamIdentifier=VTLHGQC72SX\n",
            "VTLHGQC72S"
        ));
    }

    #[test]
    fn the_failure_text_says_what_to_do() {
        let text = failure_text("mac", "checksum mismatch");
        assert!(text.contains("could not install it: checksum mismatch"));
        assert!(text.contains(MANUAL_INSTALL));
        assert!(text.contains("apas config set omp_path"));
    }
}
