//! Host-local snapshots of provider executables. The installed providers remain
//! authoritative; a successful snapshot is never modified or overwritten.

use anyhow::{bail, Context, Result};
use fs2::FileExt;
use std::fs::{self, File, Metadata, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const RETAIN_OLD_COPIES: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Return a verified, immutable host-local copy of a native provider executable.
/// Scripts (including npm/Node launchers) must continue to run at their original
/// paths, where their relative imports and interpreter setup still work. The
/// caller decides how to fall back to `source` if local storage or validation fails.
/// The provider's normal installer is never invoked or altered here.
pub fn cache_provider_binary(provider: &str, source: &Path) -> Result<PathBuf> {
    cache_provider_binary_in(&crate::host_storage::provider_root(), provider, source)
}

fn cache_provider_binary_in(root: &Path, provider: &str, source: &Path) -> Result<PathBuf> {
    if provider.is_empty()
        || !provider
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("invalid provider cache component {provider:?}");
    }

    // Resolve release symlinks before opening: the file descriptor pins the
    // bytes being copied even if an installer swaps the symlink afterward.
    let resolved = fs::canonicalize(source)
        .with_context(|| format!("resolve provider executable {}", source.display()))?;
    let mut input = File::open(&resolved)
        .with_context(|| format!("open provider executable {}", resolved.display()))?;
    let original = input.metadata()?;
    if !original.is_file() || original.mode() & 0o111 == 0 {
        return Ok(source.to_path_buf());
    }
    let mut header = [0u8; 18];
    match input.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Ok(source.to_path_buf());
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("read provider executable {}", resolved.display()));
        }
    }
    if !native_elf(&header) {
        return Ok(source.to_path_buf());
    }
    input.seek(SeekFrom::Start(0))?;

    let uid = unsafe { libc::getuid() };
    ensure_private_dir(root, uid)?;
    let dir = root.join(provider);
    ensure_private_dir(&dir, uid)?;
    let lock_path = dir.join(".cache.lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&lock_path)
        .with_context(|| format!("open provider cache lock {}", lock_path.display()))?;
    let lock_meta = lock.metadata()?;
    if !lock_meta.is_file() || lock_meta.uid() != uid || lock_meta.mode() & 0o077 != 0 {
        bail!("unsafe provider cache lock {}", lock_path.display());
    }
    lock.lock_exclusive()?;
    // Drop releases the flock on every return, including all failed probes.
    let result = cache_locked(&dir, source, &resolved, &mut input, &original, uid);
    let unlock = FileExt::unlock(&lock);
    match (result, unlock) {
        (Ok(path), Ok(())) => Ok(path),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error.into()),
    }
}

fn native_elf(header: &[u8; 18]) -> bool {
    if &header[..4] != b"\x7fELF" || !matches!(header[4], 1 | 2) {
        return false;
    }
    let elf_type = match header[5] {
        1 => u16::from_le_bytes([header[16], header[17]]),
        2 => u16::from_be_bytes([header[16], header[17]]),
        _ => return false,
    };
    matches!(elf_type, 2 | 3) // ET_EXEC or ET_DYN (PIE)
}

fn ensure_private_dir(dir: &Path, uid: u32) -> Result<()> {
    match fs::create_dir(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error).with_context(|| format!("create {}", dir.display())),
    }
    let meta = fs::symlink_metadata(dir)?;
    if !meta.is_dir() || meta.uid() != uid {
        bail!("unsafe provider cache directory {}", dir.display());
    }
    if meta.mode() & 0o777 != 0o700 {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

// mtime alone cannot identify in-place installer writes; ctime and inode also
// change on the usual atomic replacement and on metadata-preserving rewrites.
fn identity(path: &Path, meta: &Metadata) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hash);
    meta.dev().hash(&mut hash);
    meta.ino().hash(&mut hash);
    meta.len().hash(&mut hash);
    meta.mtime().hash(&mut hash);
    meta.mtime_nsec().hash(&mut hash);
    meta.ctime().hash(&mut hash);
    meta.ctime_nsec().hash(&mut hash);
    format!("{:016x}", hash.finish())
}

fn source_unchanged(source: &Path, resolved: &Path, initial: &Metadata) -> Result<()> {
    let current_path = fs::canonicalize(source)?;
    if current_path != resolved {
        bail!(
            "provider release changed while copying {}",
            source.display()
        );
    }
    let current = fs::metadata(source)?;
    if identity(resolved, &current) != identity(resolved, initial) {
        bail!(
            "provider executable changed while copying {}",
            source.display()
        );
    }
    Ok(())
}

fn cache_locked(
    dir: &Path,
    source: &Path,
    resolved: &Path,
    input: &mut File,
    original: &Metadata,
    uid: u32,
) -> Result<PathBuf> {
    source_unchanged(source, resolved, original)?;
    let target = dir.join(format!("bin-{}", identity(resolved, original)));
    match fs::symlink_metadata(&target) {
        Ok(meta) => {
            if !meta.is_file()
                || meta.uid() != uid
                || meta.len() != original.len()
                || meta.mode() & 0o777 != 0o500
            {
                bail!("unsafe cached provider binary {}", target.display());
            }
            // No 300MB read on a cache hit. Previous promotions were probed
            // before publication; recheck that the configured release is still
            // the one this name represents before handing out the path.
            source_unchanged(source, resolved, original)?;
            return Ok(target);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    // The lock means no other launch is currently producing a staging file.
    // Only incomplete copies are discarded; published versions are never
    // unlinked while a pane might still be about to exec their pathname.
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(".staging-"))
            && (entry.file_type()?.is_file() || entry.file_type()?.is_symlink())
        {
            fs::remove_file(entry.path())?;
        }
    }

    let stage = dir.join(format!(".staging-{}", Uuid::new_v4().simple()));
    let mut cleanup = Stage(&stage);
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(&stage)?;
    let written = std::io::copy(input, &mut output)
        .with_context(|| format!("copy provider executable {}", source.display()))?;
    if written != original.len() {
        bail!(
            "provider executable size changed while copying {}",
            source.display()
        );
    }
    output.sync_all()?;
    fs::set_permissions(&stage, fs::Permissions::from_mode(0o500))?;
    drop(output);
    source_unchanged(source, resolved, original)?;
    probe_version(&stage)?;
    source_unchanged(source, resolved, original)?;
    // Hard-linking publishes atomically without replacing an executable a
    // pane already uses. Old versions stay available through the retention
    // window for launches that have selected a path but not exec'd it yet.
    fs::hard_link(&stage, &target)
        .with_context(|| format!("publish provider executable {}", target.display()))?;
    fs::remove_file(&stage)?;
    cleanup.0 = Path::new("");
    // Existing processes keep local unlinked images alive. Delay pruning so a
    // launch that has selected an old path still has time to exec it.
    for entry in fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path == target || !entry.file_name().to_string_lossy().starts_with("bin-") {
            continue;
        }
        if entry.metadata().is_ok_and(|meta| {
            meta.is_file()
                && meta.uid() == uid
                && meta
                    .modified()
                    .ok()
                    .and_then(|time| time.elapsed().ok())
                    .is_some_and(|age| age > RETAIN_OLD_COPIES)
        }) {
            let _ = fs::remove_file(path);
        }
    }
    Ok(target)
}

struct Stage<'a>(&'a Path);

impl Drop for Stage<'_> {
    fn drop(&mut self) {
        if !self.0.as_os_str().is_empty() {
            let _ = fs::remove_file(self.0);
        }
    }
}

fn probe_version(binary: &Path) -> Result<()> {
    // Concurrent forks can temporarily inherit a staging write descriptor
    // from another thread and make exec return ETXTBSY even after our writer
    // closed. Retry only that transient error, as for the pane-host copy.
    let mut attempt = 0;
    let mut child = loop {
        match Command::new(binary)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => break child,
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 4 => {
                attempt += 1;
                thread::sleep(Duration::from_millis(20 * attempt));
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("execute provider copy {}", binary.display()));
            }
        }
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            bail!(
                "provider copy {} failed --version: {status}",
                binary.display()
            );
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("provider copy {} timed out on --version", binary.display());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::process::Command;

    #[test]
    fn release_swap_preserves_old_snapshot_and_reuses_unchanged_copy() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("release");
        let link = temp.path().join("provider");
        fs::copy("/bin/true", &source).unwrap();
        symlink(&source, &link).unwrap();
        let root = temp.path().join("providers");
        let first = cache_provider_binary_in(&root, "claude", &link).unwrap();
        let first_ino = fs::metadata(&first).unwrap().ino();
        assert_eq!(
            first,
            cache_provider_binary_in(&root, "claude", &link).unwrap()
        );
        assert_eq!(first_ino, fs::metadata(&first).unwrap().ino());
        let replacement = temp.path().join("new-release");
        fs::copy("/bin/echo", &replacement).unwrap();
        fs::remove_file(&link).unwrap();
        symlink(&replacement, &link).unwrap();
        let second = cache_provider_binary_in(&root, "claude", &link).unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(&first).unwrap(), fs::read("/bin/true").unwrap());
        assert_eq!(fs::read(&second).unwrap(), fs::read("/bin/echo").unwrap());
        assert!(Command::new(&first)
            .arg("--version")
            .status()
            .unwrap()
            .success());
        assert!(Command::new(&second)
            .arg("--version")
            .status()
            .unwrap()
            .success());
        fs::copy("/bin/true", &replacement).unwrap();
        let third = cache_provider_binary_in(&root, "claude", &link).unwrap();
        assert_ne!(second, third);
        assert_eq!(fs::read(&second).unwrap(), fs::read("/bin/echo").unwrap());
        assert_eq!(fs::read(&third).unwrap(), fs::read("/bin/true").unwrap());
        assert_eq!(
            fs::metadata(root.join("claude")).unwrap().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn switching_roots_recopies_unchanged_native_sources_without_touching_old_snapshots() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("legacy");
        let durable = temp.path().join("durable");
        for (provider, executable) in [("claude", "/bin/true"), ("codex", "/bin/echo")] {
            let source = temp.path().join(format!("{provider}-source"));
            fs::copy(executable, &source).unwrap();
            let old = cache_provider_binary_in(&legacy, provider, &source).unwrap();
            let old_inode = fs::metadata(&old).unwrap().ino();

            let new = cache_provider_binary_in(&durable, provider, &source).unwrap();
            assert!(new.starts_with(durable.join(provider)));
            assert_ne!(new, old);
            assert_ne!(fs::metadata(&new).unwrap().ino(), old_inode);
            assert_eq!(fs::read(&new).unwrap(), fs::read(&source).unwrap());
            assert_eq!(fs::read(&old).unwrap(), fs::read(&source).unwrap());
            assert_eq!(fs::metadata(&old).unwrap().ino(), old_inode);
            assert_eq!(
                cache_provider_binary_in(&durable, provider, &source).unwrap(),
                new
            );
            assert!(Command::new(&old)
                .arg("--version")
                .status()
                .unwrap()
                .success());
            assert!(Command::new(&new)
                .arg("--version")
                .status()
                .unwrap()
                .success());
        }
    }

    #[test]
    fn failed_probe_does_not_publish_and_original_is_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("provider");
        fs::copy("/bin/true", &source).unwrap();
        // Keep an ELF executable header but make its architecture impossible
        // to execute; unlike `false --version`, this reliably fails the probe.
        let mut invalid = fs::read(&source).unwrap();
        invalid[18..20].copy_from_slice(&[0xff, 0xff]);
        fs::write(&source, &invalid).unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
        let original = invalid;
        let root = temp.path().join("providers");
        assert!(cache_provider_binary_in(&root, "codex", &source).is_err());
        assert_eq!(fs::read(&source).unwrap(), original);
        let entries: Vec<String> = fs::read_dir(root.join("codex"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(entries, [".cache.lock"]);
    }

    #[test]
    fn concurrent_launches_share_one_complete_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("provider");
        fs::copy("/bin/true", &source).unwrap();
        let root = temp.path().join("providers");
        let launches: Vec<_> = (0..8)
            .map(|_| {
                let source = source.clone();
                let root = root.clone();
                thread::spawn(move || cache_provider_binary_in(&root, "codex", &source))
            })
            .collect();
        let paths: Vec<_> = launches
            .into_iter()
            .map(|launch| launch.join().unwrap().unwrap())
            .collect();
        assert!(paths.iter().all(|path| path == &paths[0]));
        assert_eq!(fs::read(&paths[0]).unwrap(), fs::read(&source).unwrap());
        assert!(Command::new(&paths[0])
            .arg("--version")
            .status()
            .unwrap()
            .success());
    }

    #[test]
    fn release_changed_after_source_open_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let new = temp.path().join("new");
        let link = temp.path().join("provider");
        fs::copy("/bin/true", &old).unwrap();
        fs::copy("/bin/echo", &new).unwrap();
        symlink(&old, &link).unwrap();
        let mut input = File::open(&old).unwrap();
        let original = input.metadata().unwrap();
        fs::remove_file(&link).unwrap();
        symlink(&new, &link).unwrap();
        let dir = temp.path().join("cache");
        fs::create_dir(&dir).unwrap();
        assert!(
            cache_locked(&dir, &link, &old, &mut input, &original, unsafe {
                libc::getuid()
            })
            .is_err()
        );
        assert_eq!(fs::read_dir(dir).unwrap().count(), 0);
    }

    #[test]
    fn script_and_symlinked_cache_root_are_not_relocated() {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("npm-launcher");
        fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let root = temp.path().join("providers");
        assert_eq!(
            cache_provider_binary_in(&root, "claude", &script).unwrap(),
            script
        );
        assert!(!root.exists());
        symlink(temp.path(), &root).unwrap();
        assert!(cache_provider_binary_in(&root, "claude", Path::new("/bin/true")).is_err());
    }
}
