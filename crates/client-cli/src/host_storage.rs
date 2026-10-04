//! Host-local executable storage. The admin-provisioned user directory under
//! `/var/lib` is durable; existing `/var/tmp` stores remain the safe fallback
//! until a host is provisioned. Never select an NFS-mounted home directory.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

const USERS_DIR: &str = "/var/lib/apas/users";
static WARNED_LEGACY_STORAGE: AtomicBool = AtomicBool::new(false);

fn warn_legacy_storage() {
    if !WARNED_LEGACY_STORAGE.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            "durable host-local binary storage is unavailable; using /var/tmp until /var/lib/apas/users/<uid> is provisioned"
        );
    }
}

fn durable_user_root_in(users: &Path, uid: u32, admin_uid: u32) -> Option<PathBuf> {
    // The parent must not allow another user to swap the user directory while
    // APAS is choosing where to execute provider binaries.
    let parent = fs::symlink_metadata(users).ok()?;
    if !parent.is_dir()
        || parent.file_type().is_symlink()
        || parent.uid() != admin_uid
        || parent.mode() & 0o022 != 0
    {
        return None;
    }
    let user = users.join(uid.to_string());
    let meta = fs::symlink_metadata(&user).ok()?;
    if !meta.is_dir()
        || meta.file_type().is_symlink()
        || meta.uid() != uid
        || meta.mode() & 0o7777 != 0o700
    {
        return None;
    }
    Some(user)
}

fn durable_user_root() -> Option<PathBuf> {
    let uid = unsafe { libc::getuid() };
    let base = Path::new("/var/lib/apas");
    let meta = fs::symlink_metadata(base).ok()?;
    if !meta.is_dir()
        || meta.file_type().is_symlink()
        || meta.uid() != 0
        || meta.mode() & 0o022 != 0
    {
        return None;
    }
    durable_user_root_in(Path::new(USERS_DIR), uid, 0)
}

pub fn legacy_provider_root() -> PathBuf {
    PathBuf::from(format!("/var/tmp/apas-providers-{}", unsafe {
        libc::getuid()
    }))
}

pub fn provider_root() -> PathBuf {
    if let Some(dir) = durable_user_root() {
        return dir.join("providers");
    }
    warn_legacy_storage();
    legacy_provider_root()
}

pub fn pane_host_bin_dir() -> PathBuf {
    if let Some(dir) = durable_user_root() {
        return dir.join("pane-host");
    }
    warn_legacy_storage();
    PathBuf::from(format!("/var/tmp/apas-bin-{}", unsafe { libc::getuid() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    #[test]
    fn selects_only_an_owner_private_directory_under_admin_control() {
        let root = tempfile::tempdir().unwrap();
        let users = root.path().join("users");
        let uid = unsafe { libc::getuid() };
        fs::create_dir(&users).unwrap();
        assert!(durable_user_root_in(&users, uid, uid).is_none());
        let user = users.join(uid.to_string());
        fs::create_dir(&user).unwrap();
        fs::set_permissions(&user, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(durable_user_root_in(&users, uid, uid), Some(user.clone()));
        fs::set_permissions(&user, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(durable_user_root_in(&users, uid, uid).is_none());
        fs::remove_dir(&user).unwrap();
        symlink(root.path(), &user).unwrap();
        assert!(durable_user_root_in(&users, uid, uid).is_none());
        fs::set_permissions(&users, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(durable_user_root_in(&users, uid, uid).is_none());
    }
}
