//! Host-local, immutable OMP executables. Only the updater's disposable copy is
//! writable; neither the configured executable nor a published pane executable
//! is ever passed to `omp update`.

use crate::host_storage;
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use std::cmp::Ordering;
use std::env;
use std::ffi::OsString;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime};
use uuid::Uuid;

const UPDATE_INTERVAL: Duration = Duration::from_secs(4 * 60 * 60);
const UPDATE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
const RETAIN_OLD_COPIES: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Return the most recently verified host-local OMP executable immediately.
/// The first launch copies and verifies a native ELF; later launches use the
/// published binary even if the configured source was removed or downgraded.
/// An update check is started in the background at most once per successful
/// interval; network access is never on the launch path.
/// Non-ELF launchers are returned unchanged (their package layout is needed).
pub fn omp_binary_for_launch(source: &Path) -> Result<PathBuf> {
    let dir = storage_dir()?;
    omp_binary_for_launch_in(
        source,
        &dir,
        &host_storage::legacy_provider_root().join("omp"),
    )
}

fn omp_binary_for_launch_in(source: &Path, dir: &Path, legacy: &Path) -> Result<PathBuf> {
    let lock = locked_file(&dir.join(".launch.lock"))?;
    FileExt::lock_exclusive(&lock)?;
    let result = (|| {
        let active = match read_active(dir)? {
            Some(active) => Some(active),
            None => {
                prune_stale_work(dir);
                if let Some(active) = migrate_legacy_active(dir, legacy) {
                    Some(active)
                } else {
                    if !is_native_elf(source)? {
                        return Ok(None);
                    }
                    let work = WorkDir::new(dir)?;
                    let candidate = work.path.join("omp");
                    copy_binary(source, &candidate)?;
                    if !is_native_elf(&candidate)? {
                        bail!("OMP source changed from native executable during staging");
                    }
                    let version = verify_version(&candidate, &work.path)?;
                    Some(publish(dir, &candidate, &version)?)
                }
            }
        };
        Ok(active)
    })();
    FileExt::unlock(&lock)?;
    let Some(active) = result? else {
        return Ok(source.to_path_buf());
    };

    // A failure to launch the detached thread is harmless: the next launch
    // can check again, and a usable executable has already been selected.
    if !checked_recently(&dir.join("last-successful-check")) {
        let source = source.to_path_buf();
        let background_dir = dir.to_path_buf();
        if let Err(error) = thread::Builder::new()
            .name("omp-local-update".into())
            .spawn(move || {
                if let Err(error) = check_for_update(&background_dir, &source) {
                    tracing::warn!(%error, "host-local OMP update failed; keeping last verified binary");
                }
            })
        {
            tracing::warn!(%error, "unable to start background OMP update");
        }
    }
    Ok(dir.join(active.name))
}

struct Active {
    name: String,
    version: Version,
}

fn storage_dir() -> Result<PathBuf> {
    let root = host_storage::provider_root();
    private_dir(&root)?;
    let dir = root.join("omp");
    private_dir(&dir)?;
    Ok(dir)
}

// The old cache may be absent or damaged. Never let it break a launch from a
// freshly provisioned durable store, and never modify an old published binary.
fn migrate_legacy_active(dir: &Path, legacy: &Path) -> Option<Active> {
    if dir == legacy {
        return None;
    }
    let result = (|| {
        for path in [
            legacy.parent().context("legacy OMP root has no parent")?,
            legacy,
        ] {
            let meta = match fs::symlink_metadata(path) {
                Ok(meta) => meta,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            if !meta.is_dir()
                || meta.file_type().is_symlink()
                || meta.uid() != unsafe { libc::getuid() }
                || meta.mode() & 0o7777 != 0o700
            {
                bail!("unsafe legacy OMP directory {}", path.display());
            }
        }
        let lock = locked_file(&legacy.join(".launch.lock"))?;
        FileExt::lock_exclusive(&lock)?;
        let result = (|| {
            let old = match read_active(legacy)? {
                Some(old) => old,
                None => return Ok(None),
            };
            let work = WorkDir::new(dir)?;
            let candidate = work.path.join("omp");
            copy_legacy_binary(&legacy.join(&old.name), &candidate)?;
            if !is_native_elf(&candidate)? {
                bail!("legacy OMP active executable is not a native ELF");
            }
            let actual = verify_version(&candidate, &work.path)?;
            if actual != old.version {
                bail!("legacy OMP executable version differs from its active pointer");
            }
            publish(dir, &candidate, &actual).map(Some)
        })();
        FileExt::unlock(&lock)?;
        result
    })();
    match result {
        Ok(active) => active,
        Err(error) => {
            tracing::warn!(%error, "cannot migrate legacy OMP cache; using configured source");
            None
        }
    }
}

fn private_dir(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.is_dir()
                || meta.file_type().is_symlink()
                || meta.uid() != unsafe { libc::getuid() }
            {
                bail!("unsafe OMP storage directory {}", path.display());
            }
            if meta.mode() & 0o7777 != 0o700 {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match DirBuilder::new().mode(0o700).create(path) {
                Ok(()) => return private_dir(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    return private_dir(path)
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn locked_file(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.uid() != unsafe { libc::getuid() } || meta.mode() & 0o077 != 0 {
        bail!("unsafe OMP lock {}", path.display());
    }
    Ok(file)
}

fn is_native_elf(source: &Path) -> Result<bool> {
    let mut file =
        File::open(source).with_context(|| format!("open OMP executable {}", source.display()))?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.mode() & 0o111 == 0 {
        return Ok(false);
    }
    let mut magic = [0; 4];
    Ok(file.read_exact(&mut magic).is_ok() && magic == *b"\x7fELF")
}

fn copy_binary(source: &Path, target: &Path) -> Result<()> {
    copy_binary_inner(source, target, false)
}

fn copy_legacy_binary(source: &Path, target: &Path) -> Result<()> {
    copy_binary_inner(source, target, true)
}

fn copy_binary_inner(source: &Path, target: &Path, legacy: bool) -> Result<()> {
    // Normal configured sources may be symlinks; published legacy binaries
    // must not be followed through a swapped symlink during migration.
    let resolved = if legacy {
        source.to_path_buf()
    } else {
        fs::canonicalize(source)?
    };
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&resolved)?;
    let before = input.metadata()?;
    if legacy
        && (!before.is_file()
            || before.uid() != unsafe { libc::getuid() }
            || before.mode() & 0o222 != 0
            || before.mode() & 0o100 == 0)
    {
        bail!("unsafe legacy OMP executable {}", source.display());
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(target)?;
    let copied = std::io::copy(&mut input, &mut output)
        .with_context(|| format!("copy OMP executable {}", source.display()))?;
    output.sync_all()?;
    drop(output);
    let after = if legacy {
        fs::symlink_metadata(source)?
    } else {
        fs::metadata(source)?
    };
    if copied != before.len()
        || (!legacy && fs::canonicalize(source)? != resolved)
        || !after.is_file()
        || after.file_type().is_symlink()
        || before.dev() != after.dev()
        || (legacy
            && (after.uid() != unsafe { libc::getuid() }
                || after.mode() & 0o222 != 0
                || after.mode() & 0o100 == 0))
        || before.ino() != after.ino()
        || before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        bail!("OMP source executable changed during staging");
    }
    Ok(())
}

struct WorkDir {
    path: PathBuf,
}

impl WorkDir {
    fn new(dir: &Path) -> Result<Self> {
        let path = dir.join(format!(".work-{}", Uuid::new_v4().simple()));
        DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self { path })
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            tracing::warn!(%error, path = %self.path.display(), "could not remove OMP staging directory");
        }
    }
}

fn read_active(dir: &Path) -> Result<Option<Active>> {
    let path = dir.join("active");
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("open OMP active pointer"),
    };
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.uid() != unsafe { libc::getuid() }
        || meta.len() > 200
        || meta.mode() & 0o077 != 0
    {
        bail!("unsafe OMP active pointer {}", path.display());
    }
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let (name, raw_version) = contents
        .trim_end_matches('\n')
        .split_once('\t')
        .context("invalid OMP active pointer")?;
    if !valid_name(name) {
        bail!("invalid OMP active filename");
    }
    let version = Version::parse(raw_version).context("invalid OMP active version")?;
    let binary = dir.join(name);
    let meta = fs::symlink_metadata(&binary)?;
    if !meta.is_file()
        || meta.file_type().is_symlink()
        || meta.uid() != unsafe { libc::getuid() }
        || meta.mode() & 0o222 != 0
        || meta.mode() & 0o100 == 0
    {
        bail!("unsafe OMP published executable {}", binary.display());
    }
    Ok(Some(Active {
        name: name.into(),
        version,
    }))
}

fn valid_name(name: &str) -> bool {
    name.len() == 36
        && name.starts_with("omp-")
        && name[4..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn publish(dir: &Path, candidate: &Path, version: &Version) -> Result<Active> {
    let name = format!("omp-{}", Uuid::new_v4().simple());
    let target = dir.join(&name);
    fs::set_permissions(candidate, fs::Permissions::from_mode(0o500))?;
    File::open(candidate)?.sync_all()?;
    fs::rename(candidate, &target)?;
    let pointer = dir.join(format!(".active-{}", Uuid::new_v4().simple()));
    let result = (|| {
        File::open(dir)?.sync_all()?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&pointer)?;
        writeln!(file, "{name}\t{version}")?;
        file.sync_all()?;
        fs::rename(&pointer, dir.join("active"))?;
        Ok::<_, anyhow::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(pointer);
        // No pane could observe this copy: the active pointer was not changed.
        let _ = fs::remove_file(&target);
    }
    result?;
    // An fsync error after the rename cannot undo the atomic promotion.
    if let Err(error) = File::open(dir).and_then(|file| file.sync_all()) {
        tracing::warn!(%error, "OMP active pointer promoted but directory sync failed");
    }
    Ok(Active {
        name,
        version: version.clone(),
    })
}

fn verify_version(binary: &Path, work: &Path) -> Result<Version> {
    let out_path = work.join(format!(".version-{}", Uuid::new_v4().simple()));
    let result = (|| {
        let out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&out_path)?;
        run_command(binary, &["--version"], VERSION_TIMEOUT, None, Some(out))?;
        let mut text = String::new();
        File::open(&out_path)?.take(512).read_to_string(&mut text)?;
        Version::from_output(&text).with_context(|| {
            format!(
                "unrecognized OMP version from {}: {text:?}",
                binary.display()
            )
        })
    })();
    let _ = fs::remove_file(out_path);
    result
}

fn run_command(
    binary: &Path,
    args: &[&str],
    timeout: Duration,
    path: Option<OsString>,
    stdout: Option<File>,
) -> Result<()> {
    let mut command = Command::new(binary);
    command
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    if let Some(stdout) = stdout {
        command.stdout(stdout);
    } else {
        command.stdout(Stdio::null());
    }
    if let Some(path) = path {
        command.env("PATH", path);
    }
    // Upstream's update can spawn a downloader. Terminate the entire process
    // group on timeout, not just the immediate CLI process.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("start {}", binary.display()))?;
    let start = std::time::Instant::now();
    loop {
        let status = match child.try_wait() {
            Ok(status) => status,
            Err(error) => {
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.into());
            }
        };
        if let Some(status) = status {
            if status.success() {
                return Ok(());
            }
            bail!(
                "{} {} exited with {status}",
                binary.display(),
                args.join(" ")
            );
        }
        if start.elapsed() >= timeout {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "{} {} exceeded {timeout:?}",
                binary.display(),
                args.join(" ")
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn updater_path(work: &Path, source: &Path) -> Result<OsString> {
    let configured = source.parent().unwrap_or_else(|| Path::new(""));
    let resolved = fs::canonicalize(source).ok();
    let resolved_dir = resolved.as_deref().and_then(Path::parent);
    let previous = env::var_os("PATH").unwrap_or_default();
    let mut entries = vec![work.to_path_buf()];
    for entry in env::split_paths(&previous) {
        // Preserve ordinary tools, but omit *every* other omp entry. If the
        // work copy disappeared, upstream must fail rather than target NFS.
        if entry != configured
            && resolved_dir != Some(entry.as_path())
            && fs::canonicalize(&entry).ok().as_deref() != resolved_dir
            && !entry.join("omp").exists()
        {
            entries.push(entry);
        }
    }
    Ok(env::join_paths(entries)?)
}

fn checked_recently(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| {
        meta.is_file()
            && !meta.file_type().is_symlink()
            && meta.uid() == unsafe { libc::getuid() }
            && meta
                .modified()
                .ok()
                .and_then(|when| SystemTime::now().duration_since(when).ok())
                .is_some_and(|age| age < UPDATE_INTERVAL)
    })
}

fn prune_stale(dir: &Path, active_name: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name == active_name || !valid_name(name) {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_file()
            && meta.uid() == unsafe { libc::getuid() }
            && meta
                .modified()
                .ok()
                .and_then(|time| SystemTime::now().duration_since(time).ok())
                .is_some_and(|age| age > RETAIN_OLD_COPIES)
        {
            // A live local process keeps its mapped inode after unlink.
            let _ = fs::remove_file(path);
        }
    }
}

fn prune_stale_work(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.len() != 38
            || !name.starts_with(".work-")
            || !name[6..].bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            continue;
        }
        let path = entry.path();
        if fs::symlink_metadata(&path).is_ok_and(|meta| {
            meta.is_dir()
                && meta.uid() == unsafe { libc::getuid() }
                && meta
                    .modified()
                    .ok()
                    .and_then(|time| time.elapsed().ok())
                    .is_some_and(|age| age > UPDATE_TIMEOUT + Duration::from_secs(3600))
        }) {
            // Only while holding the launch or update lock: no live attempt
            // can own a stale staging directory at this point.
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn check_for_update(dir: &Path, source: &Path) -> Result<()> {
    private_dir(dir)?;
    let lock = locked_file(&dir.join(".update.lock"))?;
    match FileExt::try_lock_exclusive(&lock) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            return Ok(()); // Another process already performs this update.
        }
        Err(error) => return Err(error.into()),
    }
    let result = (|| {
        prune_stale_work(dir);
        let last = dir.join("last-successful-check");
        if checked_recently(&last) {
            return Ok(());
        }
        let launch_lock = locked_file(&dir.join(".launch.lock"))?;
        FileExt::lock_exclusive(&launch_lock)?;
        let current = read_active(dir)?.context("no verified OMP version to update")?;
        FileExt::unlock(&launch_lock)?;
        let work = WorkDir::new(dir)?;
        let candidate = work.path.join("omp");
        copy_binary(&dir.join(&current.name), &candidate)?;
        let original = verify_version(&candidate, &work.path)?;
        if original != current.version {
            bail!("published OMP executable version differs from its active pointer");
        }
        run_command(
            &candidate,
            &["update"],
            UPDATE_TIMEOUT,
            Some(updater_path(&work.path, source)?),
            None,
        )?;
        if !is_native_elf(&candidate)? {
            bail!("OMP updater replaced its standalone executable with a non-native launcher");
        }
        let updated = verify_version(&candidate, &work.path)?;
        if updated > original {
            FileExt::lock_exclusive(&launch_lock)?;
            let result = (|| {
                let current =
                    read_active(dir)?.context("OMP active pointer removed during update")?;
                if updated > current.version {
                    publish(dir, &candidate, &updated)?;
                }
                Ok::<_, anyhow::Error>(())
            })();
            FileExt::unlock(&launch_lock)?;
            result?;
        } else if updated < original {
            bail!("OMP updater proposed a downgrade from {original} to {updated}");
        }
        // A successful unchanged check is throttled too. Any failure above
        // leaves the previous timestamp intact, so a later pane can retry.
        let marker = dir.join(format!(".checked-{}", Uuid::new_v4().simple()));
        let result = (|| {
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&marker)?;
            file.sync_all()?;
            fs::rename(&marker, last)?;
            Ok::<_, anyhow::Error>(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(marker);
        }
        result?;
        if let Some(active) = read_active(dir)? {
            prune_stale(dir, &active.name);
        }
        Ok(())
    })();
    FileExt::unlock(&lock)?;
    result
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Version {
    core: [u64; 3],
    prerelease: Vec<String>,
}

impl Version {
    fn from_output(output: &str) -> Result<Self> {
        for token in output.split_whitespace() {
            let version = token.strip_prefix("omp/").unwrap_or(token);
            if let Ok(version) = Self::parse(version) {
                return Ok(version);
            }
        }
        bail!("missing semantic version")
    }

    fn parse(raw: &str) -> Result<Self> {
        let raw = raw.strip_prefix('v').unwrap_or(raw);
        let raw = raw.split_once('+').map_or(raw, |(version, _)| version);
        let (core, pre) = raw.split_once('-').unwrap_or((raw, ""));
        let numbers: Vec<_> = core.split('.').collect();
        if numbers.len() != 3
            || numbers
                .iter()
                .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
        {
            bail!("invalid semantic version: {raw}");
        }
        let prerelease = if pre.is_empty() {
            if raw.contains('-') {
                bail!("invalid empty prerelease: {raw}");
            }
            Vec::new()
        } else {
            if pre.split('.').any(|part| {
                part.is_empty() || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            }) {
                bail!("invalid prerelease version: {raw}");
            }
            pre.split('.').map(str::to_owned).collect()
        };
        Ok(Self {
            core: [
                numbers[0].parse()?,
                numbers[1].parse()?,
                numbers[2].parse()?,
            ],
            prerelease,
        })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.core[0], self.core[1], self.core[2])?;
        if !self.prerelease.is_empty() {
            write!(f, "-{}", self.prerelease.join("."))?;
        }
        Ok(())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| {
            match (self.prerelease.is_empty(), other.prerelease.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (a, b) in self.prerelease.iter().zip(&other.prerelease) {
                        let numeric_a = a.bytes().all(|byte| byte.is_ascii_digit());
                        let numeric_b = b.bytes().all(|byte| byte.is_ascii_digit());
                        let comparison = match (numeric_a, numeric_b) {
                            (true, true) => a.len().cmp(&b.len()).then_with(|| a.cmp(b)),
                            (true, false) => Ordering::Less,
                            (false, true) => Ordering::Greater,
                            (false, false) => a.cmp(b),
                        };
                        if comparison != Ordering::Equal {
                            return comparison;
                        }
                    }
                    self.prerelease.len().cmp(&other.prerelease.len())
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executable_fixture(root: &Path, name: &str, version: &str) -> PathBuf {
        let code = root.join(format!("{name}.rs"));
        let binary = root.join(name);
        fs::write(
            &code,
            format!(
                "fn main() {{ match std::env::args().nth(1).as_deref() {{ Some(\"--version\") => println!(\"omp/{version}\"), Some(\"update\") => (), _ => std::process::exit(2) }} }}"
            ),
        )
        .unwrap();
        let output = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .arg(&code)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fixture compilation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        binary
    }

    fn publish_fixture(dir: &Path, source: &Path, version: &str) -> Active {
        let work = WorkDir::new(dir).unwrap();
        let candidate = work.path.join("omp");
        copy_binary(source, &candidate).unwrap();
        let verified = verify_version(&candidate, &work.path).unwrap();
        assert_eq!(verified, Version::parse(version).unwrap());
        publish(dir, &candidate, &verified).unwrap()
    }

    #[test]
    fn durable_launch_migrates_verified_newer_omp_and_preserves_legacy() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("old-providers");
        let legacy = root.join("omp");
        let durable = temp.path().join("durable-omp");
        for dir in [&root, &legacy, &durable] {
            private_dir(dir).unwrap();
        }
        let shared = executable_fixture(temp.path(), "shared-omp", "18.3.2");
        let cached = executable_fixture(temp.path(), "cached-omp", "18.5.1");
        let old = publish_fixture(&legacy, &cached, "18.5.1");
        fs::write(durable.join("last-successful-check"), b"").unwrap();

        let launched = omp_binary_for_launch_in(&shared, &durable, &legacy).unwrap();
        assert_eq!(launched.parent(), Some(durable.as_path()));
        assert_eq!(
            verify_version(&launched, temp.path()).unwrap(),
            Version::parse("18.5.1").unwrap()
        );
        assert_eq!(
            verify_version(&legacy.join(&old.name), temp.path()).unwrap(),
            Version::parse("18.5.1").unwrap()
        );
        assert_eq!(read_active(&legacy).unwrap().unwrap().name, old.name);

        fs::remove_file(durable.join("last-successful-check")).unwrap();
        check_for_update(&durable, &shared).unwrap();
        assert!(durable.join("last-successful-check").exists());
        assert_eq!(
            verify_version(&launched, temp.path()).unwrap(),
            Version::parse("18.5.1").unwrap()
        );
        assert!(legacy.join(&old.name).exists());
    }

    #[test]
    fn unsafe_legacy_active_falls_back_to_shared_source_without_following_link() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("old-providers");
        let legacy = root.join("omp");
        let durable = temp.path().join("durable-omp");
        for dir in [&root, &legacy, &durable] {
            private_dir(dir).unwrap();
        }
        let shared = executable_fixture(temp.path(), "shared-omp", "18.3.2");
        let cached = executable_fixture(temp.path(), "cached-omp", "18.5.1");
        let old = publish_fixture(&legacy, &cached, "18.5.1");
        fs::remove_file(legacy.join("active")).unwrap();
        std::os::unix::fs::symlink(&shared, legacy.join("active")).unwrap();
        fs::write(durable.join("last-successful-check"), b"").unwrap();

        let launched = omp_binary_for_launch_in(&shared, &durable, &legacy).unwrap();
        assert_eq!(
            verify_version(&launched, temp.path()).unwrap(),
            Version::parse("18.3.2").unwrap()
        );
        assert!(legacy.join(&old.name).exists());
        assert!(fs::symlink_metadata(legacy.join("active"))
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn mismatched_legacy_version_is_not_promoted() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("old-providers");
        let legacy = root.join("omp");
        let durable = temp.path().join("durable-omp");
        for dir in [&root, &legacy, &durable] {
            private_dir(dir).unwrap();
        }
        let shared = executable_fixture(temp.path(), "shared-omp", "18.3.2");
        let cached = executable_fixture(temp.path(), "cached-omp", "18.5.1");
        let old = publish_fixture(&legacy, &cached, "18.5.1");
        fs::write(legacy.join("active"), format!("{}\t18.6.0\n", old.name)).unwrap();
        fs::write(durable.join("last-successful-check"), b"").unwrap();

        let launched = omp_binary_for_launch_in(&shared, &durable, &legacy).unwrap();
        assert_eq!(
            verify_version(&launched, temp.path()).unwrap(),
            Version::parse("18.3.2").unwrap()
        );
        assert!(legacy.join(old.name).exists());
    }

    #[test]
    fn legacy_published_symlink_is_not_copied() {
        let temp = tempfile::tempdir().unwrap();
        let source = executable_fixture(temp.path(), "shared-omp", "18.3.2");
        let cached = executable_fixture(temp.path(), "cached-omp", "18.5.1");
        let root = temp.path().join("old-providers");
        let legacy = root.join("omp");
        let durable = temp.path().join("durable-omp");
        for dir in [&root, &legacy, &durable] {
            private_dir(dir).unwrap();
        }
        let old = publish_fixture(&legacy, &cached, "18.5.1");
        fs::remove_file(legacy.join(&old.name)).unwrap();
        std::os::unix::fs::symlink(&cached, legacy.join(&old.name)).unwrap();
        fs::write(durable.join("last-successful-check"), b"").unwrap();

        let launched = omp_binary_for_launch_in(&source, &durable, &legacy).unwrap();
        assert_eq!(
            verify_version(&launched, temp.path()).unwrap(),
            Version::parse("18.3.2").unwrap()
        );
        assert!(fs::symlink_metadata(legacy.join(old.name))
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn legacy_location_does_not_migrate_itself() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("old-providers");
        let legacy = root.join("omp");
        for dir in [&root, &legacy] {
            private_dir(dir).unwrap();
        }
        let shared = executable_fixture(temp.path(), "shared-omp", "18.3.2");
        fs::write(legacy.join("last-successful-check"), b"").unwrap();
        let launched = omp_binary_for_launch_in(&shared, &legacy, &legacy).unwrap();
        assert_eq!(
            verify_version(&launched, temp.path()).unwrap(),
            Version::parse("18.3.2").unwrap()
        );
        assert_eq!(
            read_active(&legacy).unwrap().unwrap().name,
            launched.file_name().unwrap().to_str().unwrap()
        );
    }

    #[test]
    fn version_comparison_rejects_downgrades_and_orders_canaries() {
        let version = Version::from_output("omp/18.5.1-canary.12\n").unwrap();
        assert!(version > Version::parse("18.5.1-canary.9").unwrap());
        assert!(version < Version::parse("18.5.1").unwrap());
        assert!(version > Version::parse("18.3.2").unwrap());
        assert!(Version::parse("19.0.0").unwrap() > version);
        assert_eq!(
            Version::from_output("omp/18.3.2\n").unwrap(),
            Version::parse("18.3.2").unwrap()
        );
    }

    #[test]
    fn publishing_preserves_old_executable_and_uses_new_active_pointer() {
        let temp = tempfile::tempdir().unwrap();
        private_dir(temp.path()).unwrap();
        let source = temp.path().join("fake-source");
        fs::write(&source, b"\x7fELFfake 18.3.2").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(is_native_elf(&source).unwrap());
        let first = WorkDir::new(temp.path()).unwrap();
        let candidate = first.path.join("omp");
        copy_binary(&source, &candidate).unwrap();
        let old = publish(temp.path(), &candidate, &Version::parse("18.3.2").unwrap()).unwrap();
        let second = WorkDir::new(temp.path()).unwrap();
        let candidate = second.path.join("omp");
        fs::write(&candidate, b"\x7fELFfake 18.5.1").unwrap();
        let new = publish(temp.path(), &candidate, &Version::parse("18.5.1").unwrap()).unwrap();
        assert_eq!(read_active(temp.path()).unwrap().unwrap().name, new.name);
        assert_eq!(
            fs::read(temp.path().join(old.name)).unwrap(),
            b"\x7fELFfake 18.3.2"
        );
        assert_eq!(
            fs::read(temp.path().join(&new.name)).unwrap(),
            b"\x7fELFfake 18.5.1"
        );
        fs::write(&source, b"\x7fELFfake 18.1.0").unwrap();
        assert_eq!(read_active(temp.path()).unwrap().unwrap().name, new.name);
        fs::remove_file(source).unwrap();
        assert_eq!(read_active(temp.path()).unwrap().unwrap().name, new.name);
    }

    #[test]
    fn bad_candidate_does_not_replace_existing_verified_binary() {
        let temp = tempfile::tempdir().unwrap();
        private_dir(temp.path()).unwrap();
        let work = WorkDir::new(temp.path()).unwrap();
        let candidate = work.path.join("omp");
        fs::write(&candidate, b"\x7fELFvalid").unwrap();
        let old = publish(temp.path(), &candidate, &Version::parse("18.5.1").unwrap()).unwrap();
        let bad = WorkDir::new(temp.path()).unwrap();
        fs::write(bad.path.join("omp"), b"#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(bad.path.join("omp"), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(!is_native_elf(&bad.path.join("omp")).unwrap());
        assert_eq!(read_active(temp.path()).unwrap().unwrap().name, old.name);
        assert!(temp.path().join(old.name).exists());
    }

    #[test]
    fn refuses_symlinked_storage_and_cleans_failed_staging() {
        let temp = tempfile::tempdir().unwrap();
        let link = temp.path().join("linked-omp");
        std::os::unix::fs::symlink(temp.path(), &link).unwrap();
        assert!(private_dir(&link).is_err());
        let work = WorkDir::new(temp.path()).unwrap();
        let path = work.path.clone();
        fs::write(path.join("omp"), b"partial update").unwrap();
        drop(work);
        assert!(!path.exists());
    }
}
