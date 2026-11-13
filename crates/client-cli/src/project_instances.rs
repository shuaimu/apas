//! `.apas` shared by several machines' instances of one project.
//!
//! A project can run one instance per machine, every one of them attached to
//! the project's single session. On a shared home they all read and write the
//! same `.apas`, so the old save — re-read the file, replace `panes` with this
//! process's in-memory list, write it back — erased whatever another machine
//! had saved in between, and rewrote the project-level fields from whatever
//! copy the writer happened to hold.
//!
//! Every write here instead holds the project's lock ([`ProjectMetadataLock`]),
//! re-reads the file, changes only what the writer owns, and replaces the file
//! by rename:
//!
//! * [`save_instance_panes`] replaces the panes tagged with this machine and
//!   keeps every other pane as the file has it;
//! * [`update_project_settings`] changes project-level fields, and
//!   [`update_pane`] one pane, without touching anything else;
//! * [`start_instance`] adopts untagged panes unless another machine claimed
//!   the project first or runs it alone, and returns the panes this instance
//!   restores ([`open_instance`] first creates a missing file, as a runtime
//!   starting in a folder must).
//!
//! No write removes another machine's panes, not even when the owner forgets
//! that machine's tabs: forgetting changes only the server's roster, and on a
//! shared home this file is what that machine restores its tabs from if it
//! runs the project again.
//!
//! [`read_project_metadata`] is the reader for periodic and background reads.
//! Unlike `get_or_create_project` it never creates, deletes, regenerates or
//! registers anything, and a file it cannot parse is an error, so the caller
//! keeps its last good copy.
//!
//! Writes leave what they do not change exactly as the file has it: fields
//! this build does not know, at the top level and inside every pane, and the
//! order of the keys. A pane another machine saved keeps the fields its newer
//! build wrote, rather than losing them to this build's narrower idea of a
//! pane.
//!
//! A runtime without `shared::PROJECT_INSTANCES_CAPABILITY` ([`PaneOwner::Sole`])
//! uses the same lock and the same reads, but owns every pane and writes them
//! untagged, exactly as a CLI from before instances does.

use crate::config::{Config, DaemonConfig};
use crate::project::{get_apas_path, project_metadata_tmp_path, ProjectMetadata};
use anyhow::{Context, Result};
use fs2::FileExt;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use shared::PaneConfig;
use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long a write waits for the project lock before giving up. A holder
/// keeps it for one read and one rename, so a wait this long means the holder
/// is stuck; failing one save beats hanging the project behind it, and the
/// next save tries again.
const LOCK_WAIT: Duration = Duration::from_secs(10);

/// The machine a project run is on, and whether it may run beside other
/// machines' instances of the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceIdentity {
    /// The machine this run reports in `SessionStart`, and tags its panes with
    /// when it is capable. A project the daemon runs gets the daemon's, which
    /// its instance claim is filed under; a standalone run resolves it the way
    /// the daemon does.
    pub machine_id: Uuid,
    /// This host's name, for display beside the panes it runs.
    pub hostname: Option<String>,
    /// Whether this run may advertise `shared::PROJECT_INSTANCES_CAPABILITY`:
    /// it owns only the panes tagged with its machine and saves only those. A
    /// run without it owns every pane and writes them untagged.
    pub capable: bool,
}

impl InstanceIdentity {
    /// Capability is decided from `daemon.machine_id` by the function the
    /// daemon picks its claim mode with, so a project and the daemon running
    /// it agree on whether instances are on.
    pub fn new(machine_id: Uuid, daemon: &DaemonConfig) -> Self {
        Self {
            machine_id,
            hostname: current_hostname(),
            capable: daemon.project_instances_supported(),
        }
    }

    /// This identity for a run its daemon allows to be an instance, or not:
    /// `instances` false makes it the project's only runtime whatever
    /// `config.toml` allows ([`PaneOwner::Sole`]), which restores and saves
    /// every pane untagged and does not advertise the capability, as a CLI
    /// from before instances. A capable daemon starts a project so while its
    /// server keeps one CLI per session: an instance would report only its own
    /// machine's panes, and such a server replaces the whole roster with what
    /// its CLI reports. The machine id and hostname are still reported.
    pub fn with_instances(mut self, instances: bool) -> Self {
        self.capable &= instances;
        self
    }

    /// The panes of `.apas` this run owns.
    pub fn owner(&self) -> PaneOwner {
        if self.capable {
            PaneOwner::Instance {
                machine_id: self.machine_id,
                hostname: self.hostname.clone(),
            }
        } else {
            PaneOwner::Sole
        }
    }
}

/// This host's name, as `SessionStart` reports it.
fn current_hostname() -> Option<String> {
    hostname::get()
        .ok()
        .and_then(|name| name.into_string().ok())
        .filter(|name| !name.trim().is_empty())
}

/// The panes of `.apas` one runtime restores, saves and reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneOwner {
    /// A capable instance: the panes tagged with its machine. It never
    /// restores, adopts or rewrites a pane tagged for another machine, even
    /// while that machine is stopped, because tags are permanent.
    Instance {
        machine_id: Uuid,
        hostname: Option<String>,
    },
    /// A runtime without the capability, which is only ever the project's one
    /// runtime: every pane, saved as one list and untagged, exactly as a CLI
    /// from before instances saves it. Untagged panes stay adoptable by the
    /// next capable instance that runs the project alone.
    Sole,
}

impl PaneOwner {
    pub fn machine_id(&self) -> Option<Uuid> {
        match self {
            Self::Instance { machine_id, .. } => Some(*machine_id),
            Self::Sole => None,
        }
    }

    /// Whether `pane` is this owner's to restore, save and report.
    pub fn owns(&self, pane: &PaneConfig) -> bool {
        match self {
            Self::Instance { machine_id, .. } => pane.machine_id == Some(*machine_id),
            Self::Sole => true,
        }
    }

    /// Mark `pane` as this owner saves and reports it, whatever it carried
    /// before: tagged with the instance's machine and host, or untagged for a
    /// sole runtime.
    pub fn tag(&self, pane: &mut PaneConfig) {
        match self {
            Self::Instance {
                machine_id,
                hostname,
            } => {
                pane.machine_id = Some(*machine_id);
                pane.hostname = hostname.clone();
            }
            Self::Sole => {
                pane.machine_id = None;
                pane.hostname = None;
            }
        }
    }

    /// Whether `incoming`, one of this owner's panes, is the pane `entry`
    /// already records, so it takes that entry's place in the file.
    fn replaces(&self, entry: &PaneConfig, incoming: &PaneConfig) -> bool {
        if entry.pane_id != incoming.pane_id {
            return false;
        }
        match self {
            // An untagged entry with this pane's id and conversation is this
            // instance's own pane, saved untagged by a CLI from before
            // instances: taking it back keeps the pane from being listed twice.
            Self::Instance { machine_id, .. } => {
                entry.machine_id == Some(*machine_id)
                    || (entry.machine_id.is_none() && entry.session_id == incoming.session_id)
            }
            Self::Sole => true,
        }
    }
}

/// What [`start_instance`] found, and changed, in `.apas`.
#[derive(Debug, Clone)]
pub struct InstanceStart {
    /// The whole file after adoption: the project-level fields and every
    /// machine's panes.
    pub metadata: ProjectMetadata,
    /// The panes this runtime restores and reports, in pane id order: its own
    /// for an instance, all of them for a sole runtime.
    pub panes: Vec<PaneConfig>,
    /// Untagged panes this start adopted. `panes` includes them.
    pub adopted: Vec<u32>,
}

/// Read `.apas` without changing anything, for periodic and background reads
/// such as the flag rebroadcast and the reconnect policy snapshot.
///
/// Unlike `get_or_create_project` this never creates, deletes, regenerates or
/// registers the file. A missing, unreadable or unparseable file is an error,
/// and the caller keeps its last good copy: regenerating it would replace the
/// project with an empty one, and on a shared home erase every other machine's
/// panes with it. Legacy pane fields are migrated in the returned copy only.
pub fn read_project_metadata(dir: &Path) -> Result<ProjectMetadata> {
    let path = get_apas_path(dir);
    let bytes = read_metadata_bytes(&path)?
        .with_context(|| format!("Project metadata {} does not exist", path.display()))?;
    let mut metadata: ProjectMetadata = serde_json::from_slice(&bytes)
        .with_context(|| format!("Parse project metadata {}", path.display()))?;
    metadata.migrate_legacy();
    Ok(metadata)
}

/// Open `.apas` for a run in `dir` that needs the project but does not
/// restore its panes: a bare `apas` launch, and the legacy hybrid and remote
/// modes. [`open_instance`] opens it this way too before it takes its panes.
///
/// Only a missing file goes through the creation path, which recovers a
/// registered identity for the folder. An existing file is only read: if it
/// does not parse, opening fails and the file is left as it is, where
/// `get_or_create_project` would delete it and start an empty project, and on
/// a shared home erase every machine's panes with it. The project is
/// registered as every launch has registered it, outside the project lock.
/// Legacy pane fields are migrated in the returned copy only.
pub fn open_project_metadata(dir: &Path) -> Result<ProjectMetadata> {
    let path = get_apas_path(dir);
    match read_metadata_bytes(&path)? {
        None => crate::project::get_or_create_project(dir),
        Some(bytes) => {
            let mut metadata: ProjectMetadata = serde_json::from_slice(&bytes)
                .with_context(|| format!("Parse project metadata {}", path.display()))?;
            if let Err(error) = crate::project::register_project(dir, &metadata) {
                tracing::warn!(%error, "could not register the project in the user registry");
            }
            metadata.migrate_legacy();
            Ok(metadata)
        }
    }
}

/// Open `.apas` for a runtime starting in `dir` ([`open_project_metadata`]),
/// then [`start_instance`].
pub fn open_instance(dir: &Path, owner: &PaneOwner) -> Result<InstanceStart> {
    let project_id = open_project_metadata(dir)?.id;
    start_instance(dir, project_id, owner)
}

/// Take this runtime's panes as it starts: adopt the untagged ones when it
/// may, and return the ones it restores.
///
/// An instance adopts every untagged pane, tagging it with its machine,
/// unless another machine rules it out ([`another_machine_blocks_adoption`]):
/// one that runs the project alone, or one whose live instance claim is older
/// than this machine's. The daemon writes this machine's claim before it
/// starts the runtime, so two instances starting together each see the
/// other's claim: the earlier claimant adopts, and the other starts with none
/// of those panes. Adoption happens under the lock on a fresh read, so an
/// instance never takes a pane another one has already tagged. Untagged
/// entries that appear while an instance runs wait for its next start. A pane
/// tagged for another machine is never adopted or restored. A sole runtime
/// adopts nothing and restores every pane.
///
/// The file must exist; [`open_instance`] creates a missing one first. A file
/// that does not parse, or that belongs to another project, is an error and is
/// left as it is.
pub fn start_instance(dir: &Path, project_id: Uuid, owner: &PaneOwner) -> Result<InstanceStart> {
    start_instance_with(dir, project_id, owner, LOCK_WAIT, || match owner {
        PaneOwner::Instance { machine_id, .. } => {
            another_machine_blocks_adoption(project_id, *machine_id)
        }
        PaneOwner::Sole => false,
    })
}

fn start_instance_with(
    dir: &Path,
    project_id: Uuid,
    owner: &PaneOwner,
    wait: Duration,
    adoption_blocked: impl FnOnce() -> bool,
) -> Result<InstanceStart> {
    let (adopted, metadata) = transact(dir, project_id, wait, WhenMissing::Fail, |document| {
        Ok(document.adopt_untagged(owner, adoption_blocked))
    })?;
    if !adopted.is_empty() {
        tracing::info!(?adopted, "adopted untagged panes for this machine");
    }
    let mut panes: Vec<PaneConfig> = metadata
        .panes
        .iter()
        .filter(|pane| owner.owns(pane))
        .cloned()
        .collect();
    panes.sort_by_key(|pane| pane.pane_id);
    Ok(InstanceStart {
        metadata,
        panes,
        adopted,
    })
}

/// Save this runtime's panes: `panes` is the complete list it owns, in any
/// order, and replaces the list it saved last time.
///
/// An instance tags each pane with its machine and replaces only the entries
/// tagged with it, so an empty list means it has no panes and leaves every
/// other pane alone. Entries of other machines, and untagged ones, stay exactly
/// as the file has them. A sole runtime replaces the whole list, untagged.
///
/// Project-level fields are kept as the file has them, never taken from the
/// saver's copy. The one exception is the legacy `is_paused`, which mirrors
/// the legacy deadloop pane's pause: the owner of that pane updates it, and it
/// clears when no such pane remains, as it always has.
///
/// A missing file is created with this project's id, so a roster saved while
/// the file is gone survives the next start. Returns the file as written.
pub fn save_instance_panes(
    dir: &Path,
    project_id: Uuid,
    owner: &PaneOwner,
    panes: &[PaneConfig],
) -> Result<ProjectMetadata> {
    let ((), metadata) = transact(
        dir,
        project_id,
        LOCK_WAIT,
        WhenMissing::Create,
        |document| {
            document.replace_owned_panes(owner, panes);
            Ok(())
        },
    )?;
    Ok(metadata)
}

/// Change project-level fields: the operations flags, the legacy `is_paused`,
/// and the like. `update` gets the file as it stands now, panes included, and
/// only the fields it changes are written; every other field keeps whatever
/// another instance last wrote. Pane changes and a changed `id` are discarded:
/// panes change through [`save_instance_panes`] and [`update_pane`].
///
/// `update` runs under the lock, so it must not take it again, through this
/// module or through `save_project`. A missing file is created with this
/// project's id.
pub fn update_project_settings<R>(
    dir: &Path,
    project_id: Uuid,
    update: impl FnOnce(&mut ProjectMetadata) -> R,
) -> Result<R> {
    let (result, _) = transact(
        dir,
        project_id,
        LOCK_WAIT,
        WhenMissing::Create,
        |document| {
            let mut current = document.settings.clone();
            current.panes = document
                .panes
                .iter()
                .map(|pane| pane.config.clone())
                .collect();
            let result = update(&mut current);
            current.id = document.settings.id;
            current.panes = Vec::new();
            document.settings = current;
            Ok(result)
        },
    )?;
    Ok(result)
}

/// Change one pane's fields, whichever machine's it is, such as its worktree
/// assignment or a replaced provider session id. Only the fields `update`
/// changes are written. Its id and its machine tag are kept: a pane never
/// moves between machines.
///
/// `Ok(None)` when no pane has that id, and an error when several do, rather
/// than guessing which was meant. `update` runs under the lock, so it must not
/// take it again, through this module or through `save_project`.
pub fn update_pane<R>(
    dir: &Path,
    project_id: Uuid,
    pane_id: u32,
    update: impl FnOnce(&mut PaneConfig) -> R,
) -> Result<Option<R>> {
    let (result, _) = transact(
        dir,
        project_id,
        LOCK_WAIT,
        WhenMissing::Create,
        |document| {
            let mut matching = document
                .panes
                .iter_mut()
                .filter(|pane| pane.config.pane_id == pane_id);
            let Some(stored) = matching.next() else {
                return Ok(None);
            };
            anyhow::ensure!(
                matching.next().is_none(),
                "Several panes in .apas have id {pane_id}; not changing any of them"
            );
            let mut config = stored.config.clone();
            let result = update(&mut config);
            config.pane_id = stored.config.pane_id;
            config.machine_id = stored.config.machine_id;
            config.hostname = stored.config.hostname.clone();
            stored.config = config;
            Ok(Some(result))
        },
    )?;
    Ok(result)
}

/// Whether another machine's claim on the project rules out this one adopting
/// its untagged panes: a host runs it alone, or another machine claimed it
/// before this one did ([`crate::daemon_registry::LiveInstances::blocks_adoption`]).
/// A host that cannot read the claims answers yes: not adopting is the answer
/// that can never put one pane on two machines.
pub fn another_machine_blocks_adoption(project_id: Uuid, own_machine_id: Uuid) -> bool {
    match Config::config_dir() {
        Ok(config_dir) => crate::daemon_registry::live_instance_machines(
            &config_dir,
            &project_id.to_string(),
            own_machine_id,
        )
        .blocks_adoption(),
        Err(error) => {
            tracing::warn!(%error, "cannot read instance claims; not adopting untagged panes");
            true
        }
    }
}

/// Exclusive hold on one project's `.apas`, released when dropped.
///
/// Every write of the file happens under it, from every instance on every host
/// that shares the home. It lives at `<config dir>/locks/<project id>.lock`,
/// in the config directory those hosts share, and never beside `.apas`:
/// repositories ignore `.apas` by its exact name, so a lock file in the project
/// would show up in `git status`, where an agent could commit it. Lock files
/// are permanent, like the registry's, so a rename can never move the locked
/// inode out from under a waiter.
///
/// The project registry's lock may be held while taking this one, never the
/// other way round.
pub struct ProjectMetadataLock {
    path: PathBuf,
    file: Option<File>,
}

impl Drop for ProjectMetadataLock {
    fn drop(&mut self) {
        // Closed before another thread of this process may go in. On NFS the
        // lock is a POSIX lock owned by the whole process, and closing any
        // descriptor of the file drops it, including one a thread that went in
        // after us has just taken.
        if let Some(file) = self.file.take() {
            let _ = FileExt::unlock(&file);
            drop(file);
        }
        HELD_LOCKS.release(&self.path);
    }
}

/// Take the project's lock, waiting at most ten seconds. Fails rather than
/// letting the caller write unlocked.
pub fn lock_project_metadata(project_id: Uuid) -> Result<ProjectMetadataLock> {
    lock_project_metadata_within(project_id, LOCK_WAIT)
}

fn lock_project_metadata_within(project_id: Uuid, wait: Duration) -> Result<ProjectMetadataLock> {
    let path = metadata_lock_path(project_id)?;
    let deadline = Instant::now() + wait;
    anyhow::ensure!(
        HELD_LOCKS.acquire(&path, deadline),
        "Timed out after {wait:?} waiting for {}, which another thread holds",
        path.display()
    );
    // The slot is ours from here, and this guard gives it back however the
    // rest ends. The descriptor below is declared after it, so on an early
    // return it is closed first.
    let mut lock = ProjectMetadataLock { path, file: None };
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock.path)
        .with_context(|| format!("Open {}", lock.path.display()))?;
    let mut pause = Duration::from_millis(5);
    loop {
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => break,
            Err(error) if lock_contended(&error) => {
                let Some(remaining) = time_left(deadline) else {
                    anyhow::bail!(
                        "Timed out after {wait:?} waiting for {}, which another instance of \
                         this project holds",
                        lock.path.display()
                    );
                };
                std::thread::sleep(pause.min(remaining));
                pause = (pause * 2).min(Duration::from_millis(100));
            }
            Err(error) => {
                return Err(error).with_context(|| format!("Lock {}", lock.path.display()));
            }
        }
    }
    lock.file = Some(file);
    Ok(lock)
}

fn metadata_lock_path(project_id: Uuid) -> Result<PathBuf> {
    let dir = Config::config_dir()?.join("locks");
    std::fs::create_dir_all(&dir).with_context(|| format!("Create {}", dir.display()))?;
    Ok(dir.join(format!("{project_id}.lock")))
}

fn time_left(deadline: Instant) -> Option<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
}

fn lock_contended(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        // `flock` on NFS becomes a POSIX lock, which may report a held lock
        // as EACCES rather than EWOULDBLOCK.
        if error.raw_os_error() == Some(libc::EACCES) {
            return true;
        }
    }
    error.kind() == fs2::lock_contended_error().kind()
}

/// The lock files this process holds.
///
/// On NFS, Linux implements `flock` as a POSIX lock owned by the process, so
/// two threads of one daemon would both "hold" the file lock at once. Threads
/// queue here first, so only one of them ever reaches it.
struct HeldLocks {
    held: Mutex<BTreeSet<PathBuf>>,
    released: Condvar,
}

static HELD_LOCKS: HeldLocks = HeldLocks {
    held: Mutex::new(BTreeSet::new()),
    released: Condvar::new(),
};

impl HeldLocks {
    fn acquire(&self, path: &Path, deadline: Instant) -> bool {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        while held.contains(path) {
            let Some(remaining) = time_left(deadline) else {
                return false;
            };
            held = self
                .released
                .wait_timeout(held, remaining)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        held.insert(path.to_path_buf());
        true
    }

    fn release(&self, path: &Path) {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(path);
        self.released.notify_all();
    }
}

/// The file's bytes, or `None` when there is no file. A symlink is followed,
/// as every `.apas` reader always has, but anything other than a regular file
/// is refused: a FIFO there would block the reading thread forever.
fn read_metadata_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("Open {}", path.display())),
    };
    anyhow::ensure!(
        file.metadata()?.is_file(),
        "Not a regular metadata file: {}",
        path.display()
    );
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("Read {}", path.display()))?;
    Ok(Some(bytes))
}

/// Replace `.apas` with `bytes`. They are written to a temporary file beside
/// it, synced, and renamed over it, so a reader on any host sees the old file
/// or the new one and never part of either, and a crash cannot leave it
/// truncated.
pub(crate) fn write_metadata_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let staged = project_metadata_tmp_path(path);
    let written = (|| -> std::io::Result<()> {
        // Truncated rather than created exclusively: a crashed writer can leave
        // its staging file behind, and a later process that reuses its pid
        // would otherwise fail on it. The name is unique among live writers.
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&staged)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&staged, path)
    })();
    if let Err(error) = written {
        let _ = std::fs::remove_file(&staged);
        return Err(error).with_context(|| format!("Write {}", path.display()));
    }
    // The rename has taken effect; a directory that cannot be synced does not
    // turn the save into a failure.
    if let Some(parent) = path.parent() {
        if let Err(error) = File::open(parent).and_then(|dir| dir.sync_all()) {
            tracing::debug!(%error, dir = %parent.display(), "could not sync the project directory");
        }
    }
    Ok(())
}

/// What a write does when there is no `.apas`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhenMissing {
    /// Start from an empty project with this id, and write it only if the
    /// operation changes it.
    Create,
    /// Fail: the caller needs the file to exist.
    Fail,
}

/// Run one locked read-modify-write: take the lock, read the file afresh,
/// apply `change`, and write the file back only if it changed. Returns what
/// `change` returned and the file as it now stands.
fn transact<R>(
    dir: &Path,
    project_id: Uuid,
    wait: Duration,
    when_missing: WhenMissing,
    change: impl FnOnce(&mut Document) -> Result<R>,
) -> Result<(R, ProjectMetadata)> {
    let _lock = lock_project_metadata_within(project_id, wait)?;
    let mut document = Document::load(dir, project_id, when_missing)?;
    let result = change(&mut document)?;
    let metadata = document.commit()?;
    Ok((result, metadata))
}

/// `.apas` as read under the lock: the typed view the operations change, and
/// the file's own JSON, which keeps everything they leave alone exactly as it
/// was.
struct Document {
    path: PathBuf,
    /// The top-level object as the file has it, or as a new file would.
    root: Object,
    /// The project-level fields as read: what changes are measured against.
    /// Its `panes` is always empty; the panes are in `panes`.
    read_as: ProjectMetadata,
    /// The project-level fields as the operation leaves them, `panes` empty.
    settings: ProjectMetadata,
    /// Every pane, in the order they are written.
    panes: Vec<StoredPane>,
}

/// A pane, with the entry it was read from when it came from the file.
struct StoredPane {
    from_file: Option<(Object, PaneConfig)>,
    config: PaneConfig,
}

impl StoredPane {
    fn render(&self) -> Result<OrderedJson> {
        let current = to_object(&self.config)?;
        Ok(OrderedJson::Object(match &self.from_file {
            Some((entry, read_as)) => {
                let mut entry = entry.clone();
                apply_changes(&mut entry, &to_object(read_as)?, &current);
                entry
            }
            None => current,
        }))
    }
}

impl Document {
    fn load(dir: &Path, project_id: Uuid, when_missing: WhenMissing) -> Result<Self> {
        let path = get_apas_path(dir);
        let Some(bytes) = read_metadata_bytes(&path)? else {
            anyhow::ensure!(
                when_missing == WhenMissing::Create,
                "Project metadata {} does not exist",
                path.display()
            );
            let fresh = ProjectMetadata::with_id_and_name(
                project_id,
                dir.file_name()
                    .and_then(|name| name.to_str())
                    .map(String::from),
            );
            return Ok(Self {
                root: to_object(&fresh)?,
                read_as: fresh.clone(),
                settings: fresh,
                panes: Vec::new(),
                path,
            });
        };
        let mut read_as: ProjectMetadata = serde_json::from_slice(&bytes)
            .with_context(|| format!("Parse project metadata {}", path.display()))?;
        anyhow::ensure!(
            read_as.id == project_id,
            "{} belongs to project {}, not {}",
            path.display(),
            read_as.id,
            project_id
        );
        let root = match serde_json::from_slice::<OrderedJson>(&bytes)
            .with_context(|| format!("Parse project metadata {}", path.display()))?
        {
            OrderedJson::Object(root) => root,
            _ => anyhow::bail!("Project metadata {} is not an object", path.display()),
        };
        let entries: Vec<Object> = match field(&root, "panes") {
            Some(OrderedJson::Array(items)) => items
                .iter()
                .filter_map(|item| match item {
                    OrderedJson::Object(entry) => Some(entry.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let typed = std::mem::take(&mut read_as.panes);
        anyhow::ensure!(
            entries.len() == typed.len(),
            "Project metadata {} has pane entries that are not objects",
            path.display()
        );
        let mut panes: Vec<StoredPane> = entries
            .into_iter()
            .zip(typed)
            .map(|(entry, config)| StoredPane {
                from_file: Some((entry, config.clone())),
                config,
            })
            .collect();
        if panes.is_empty() {
            // A file from before `panes` existed. Every load has always made
            // its panes from the legacy session ids; the first write records
            // them like new panes.
            let mut migrated = read_as.clone();
            migrated.migrate_legacy();
            panes = migrated
                .panes
                .into_iter()
                .map(|config| StoredPane {
                    from_file: None,
                    config,
                })
                .collect();
        }
        Ok(Self {
            path,
            root,
            settings: read_as.clone(),
            read_as,
            panes,
        })
    }

    /// Tag every untagged pane with this instance's machine, unless another
    /// machine rules it out. `adoption_blocked` is asked only when there is
    /// something to adopt.
    fn adopt_untagged(
        &mut self,
        owner: &PaneOwner,
        adoption_blocked: impl FnOnce() -> bool,
    ) -> Vec<u32> {
        if owner.machine_id().is_none() {
            return Vec::new();
        }
        // Only a pane whose id no other entry shares. Two entries with one id
        // means an untagged one outlived its tab, which the server forgot or
        // closed in its roster alone (or never knew of), and its id went to a
        // pane that is in use. Adopting it would give this instance two panes
        // with one id.
        let untagged = self
            .panes
            .iter()
            .filter(|pane| pane.config.machine_id.is_none())
            .count();
        let adoptable: Vec<usize> = (0..self.panes.len())
            .filter(|&index| {
                let pane = &self.panes[index].config;
                pane.machine_id.is_none()
                    && self
                        .panes
                        .iter()
                        .filter(|other| other.config.pane_id == pane.pane_id)
                        .count()
                        == 1
            })
            .collect();
        if adoptable.len() < untagged {
            tracing::warn!(
                skipped = untagged - adoptable.len(),
                "not adopting untagged panes whose ids other panes in .apas also have"
            );
        }
        if adoptable.is_empty() {
            return Vec::new();
        }
        if adoption_blocked() {
            tracing::info!(
                untagged = adoptable.len(),
                "another machine claimed this project first or runs it alone, so its untagged \
                 panes are not adopted here"
            );
            return Vec::new();
        }
        adoptable
            .into_iter()
            .map(|index| {
                let pane = &mut self.panes[index].config;
                owner.tag(pane);
                pane.pane_id
            })
            .collect()
    }

    /// Replace the panes `owner` owns with `incoming`, keeping every other
    /// entry where it is.
    fn replace_owned_panes(&mut self, owner: &PaneOwner, incoming: &[PaneConfig]) {
        let mut pending: Vec<Option<PaneConfig>> = incoming
            .iter()
            .map(|pane| {
                let mut pane = pane.clone();
                owner.tag(&mut pane);
                Some(pane)
            })
            .collect();
        let mut merged = Vec::with_capacity(self.panes.len() + pending.len());
        let mut membership_changed = false;
        for stored in std::mem::take(&mut self.panes) {
            let replacement = pending
                .iter_mut()
                .find(|slot| {
                    slot.as_ref()
                        .is_some_and(|pane| owner.replaces(&stored.config, pane))
                })
                .and_then(Option::take);
            match replacement {
                Some(config) => {
                    membership_changed |= !owner.owns(&stored.config);
                    merged.push(StoredPane {
                        from_file: stored.from_file,
                        config,
                    });
                }
                // Its owner no longer has it.
                None if owner.owns(&stored.config) => membership_changed = true,
                None => merged.push(stored),
            }
        }
        for config in pending.into_iter().flatten() {
            if let Some(existing) = merged
                .iter()
                .find(|pane| pane.config.pane_id == config.pane_id)
            {
                tracing::warn!(
                    pane_id = config.pane_id,
                    other_machine = ?existing.config.machine_id,
                    "saving a pane whose id another pane in .apas already has; keeping both"
                );
            }
            merged.push(StoredPane {
                from_file: None,
                config,
            });
            membership_changed = true;
        }
        if membership_changed {
            merged.sort_by_key(|pane| pane.config.pane_id);
        }
        self.panes = merged;

        // Preserve the historical project-level pause mirror without allowing
        // one machine to rewrite another machine's pane history.
        let is_deadloop = |pane: &&StoredPane| pane.config.pane_id == shared::PANE_ID_DEADLOOP;
        if let Some(own) = self
            .panes
            .iter()
            .filter(is_deadloop)
            .find(|pane| owner.owns(&pane.config))
        {
            self.settings.is_paused = own.config.is_paused;
        } else if !self.panes.iter().any(|pane| is_deadloop(&pane)) {
            self.settings.is_paused = false;
        }
    }

    /// The file as the operation leaves it. Only what changed differs from
    /// `root`.
    fn render(&self) -> Result<Object> {
        let mut root = self.root.clone();
        apply_changes(
            &mut root,
            &to_object(&self.read_as)?,
            &to_object(&self.settings)?,
        );
        let panes = self
            .panes
            .iter()
            .map(StoredPane::render)
            .collect::<Result<Vec<_>>>()?;
        if field(&root, "panes").is_some() || !panes.is_empty() {
            set_field(&mut root, "panes", OrderedJson::Array(panes));
        }
        Ok(root)
    }

    /// Write the file if the operation changed it, and return it as the typed
    /// view. Unchanged saves preserve the file and its modification time.
    fn commit(self) -> Result<ProjectMetadata> {
        let rendered = self.render()?;
        if rendered != self.root {
            let bytes = serde_json::to_vec_pretty(&OrderedJson::Object(rendered))?;
            write_metadata_file(&self.path, &bytes)?;
        }
        let mut metadata = self.settings;
        metadata.panes = self.panes.into_iter().map(|pane| pane.config).collect();
        Ok(metadata)
    }
}

/// A JSON value that keeps object keys in the order the file has them.
/// `serde_json::Value` sorts them (this workspace does not enable its
/// `preserve_order` feature), so writing one back would reorder every `.apas`
/// it touched.
#[derive(Debug, Clone, PartialEq)]
enum OrderedJson {
    /// Null, a boolean, a number or a string.
    Scalar(serde_json::Value),
    Array(Vec<OrderedJson>),
    Object(Object),
}

type Object = Vec<(String, OrderedJson)>;

fn field<'a>(object: &'a Object, key: &str) -> Option<&'a OrderedJson> {
    object
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

/// Replace a field where it stands, or append it.
fn set_field(object: &mut Object, key: &str, value: OrderedJson) {
    match object.iter_mut().find(|(name, _)| name == key) {
        Some((_, existing)) => *existing = value,
        None => object.push((key.to_string(), value)),
    }
}

/// Rewrite in `target` only the fields whose serialized value differs between
/// `before` and `after`, two serializations of one typed value before and
/// after a change. Everything else in `target` stays as the file has it,
/// including fields this build does not know and the order of the keys.
fn apply_changes(target: &mut Object, before: &Object, after: &Object) {
    for (key, value) in after {
        if field(before, key) != Some(value) {
            set_field(target, key, value.clone());
        }
    }
    for (key, _) in before {
        if field(after, key).is_none() {
            target.retain(|(name, _)| name != key);
        }
    }
}

/// `value` serialized, with its keys in the order its type writes them.
fn to_object<T: Serialize>(value: &T) -> Result<Object> {
    match serde_json::from_slice::<OrderedJson>(&serde_json::to_vec(value)?)? {
        OrderedJson::Object(object) => Ok(object),
        _ => anyhow::bail!("expected a JSON object"),
    }
}

impl Serialize for OrderedJson {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::Scalar(value) => value.serialize(serializer),
            Self::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Self::Object(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for OrderedJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(OrderedJsonVisitor)
    }
}

struct OrderedJsonVisitor;

impl<'de> Visitor<'de> for OrderedJsonVisitor {
    type Value = OrderedJson;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> std::result::Result<OrderedJson, E> {
        Ok(OrderedJson::Scalar(serde_json::Value::Bool(value)))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> std::result::Result<OrderedJson, E> {
        Ok(OrderedJson::Scalar(serde_json::Value::Number(value.into())))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> std::result::Result<OrderedJson, E> {
        Ok(OrderedJson::Scalar(serde_json::Value::Number(value.into())))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> std::result::Result<OrderedJson, E> {
        serde_json::Number::from_f64(value)
            .map(|number| OrderedJson::Scalar(serde_json::Value::Number(number)))
            .ok_or_else(|| E::custom("not a finite number"))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<OrderedJson, E> {
        Ok(OrderedJson::Scalar(serde_json::Value::String(
            value.to_owned(),
        )))
    }

    fn visit_string<E: de::Error>(self, value: String) -> std::result::Result<OrderedJson, E> {
        Ok(OrderedJson::Scalar(serde_json::Value::String(value)))
    }

    fn visit_unit<E: de::Error>(self) -> std::result::Result<OrderedJson, E> {
        Ok(OrderedJson::Scalar(serde_json::Value::Null))
    }

    fn visit_none<E: de::Error>(self) -> std::result::Result<OrderedJson, E> {
        Ok(OrderedJson::Scalar(serde_json::Value::Null))
    }

    fn visit_some<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<OrderedJson, D::Error> {
        OrderedJson::deserialize(deserializer)
    }

    fn visit_seq<A: SeqAccess<'de>>(
        self,
        mut seq: A,
    ) -> std::result::Result<OrderedJson, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(OrderedJson::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(
        self,
        mut map: A,
    ) -> std::result::Result<OrderedJson, A::Error> {
        let mut entries = Vec::new();
        while let Some(entry) = map.next_entry::<String, OrderedJson>()? {
            entries.push(entry);
        }
        Ok(OrderedJson::Object(entries))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_config::{isolated_config_dir, with_config_dir};
    use crate::daemon_registry::{claim_project, ClaimMode};
    use std::sync::{Arc, Barrier};
    use tempfile::TempDir;

    fn pane(pane_id: u32, session_id: Uuid) -> PaneConfig {
        PaneConfig {
            pane_id,
            provider: shared::Provider::Codex,
            mode: shared::PaneMode::Interactive,
            kind: shared::PaneKind::Terminal,
            session_id,
            is_paused: false,
            stop_requested: false,
            prompt: None,
            min_iteration_interval_minutes: None,
            label: Some(format!("Pane {pane_id}")),
            model: None,
            effort: None,
            worktree_path: None,
            role: None,
            goal: None,
            backstory: None,
            plan_review_mode: shared::PlanReviewMode::default(),
            manual_mode: false,
            managed: false,
            machine_id: None,
            hostname: None,
        }
    }

    fn tagged(mut pane: PaneConfig, machine: Uuid, host: &str) -> PaneConfig {
        pane.machine_id = Some(machine);
        pane.hostname = Some(host.to_string());
        pane
    }

    fn instance(machine: Uuid, host: &str) -> PaneOwner {
        PaneOwner::Instance {
            machine_id: machine,
            hostname: Some(host.to_string()),
        }
    }

    /// A project directory whose `.apas` holds `panes`, written as the typed
    /// writer has always written it.
    fn project_with(panes: Vec<PaneConfig>) -> (TempDir, Uuid) {
        let dir = tempfile::tempdir().unwrap();
        let mut metadata =
            ProjectMetadata::with_id_and_name(Uuid::new_v4(), Some("demo".to_string()));
        metadata.panes = panes;
        std::fs::write(
            get_apas_path(dir.path()),
            serde_json::to_string_pretty(&metadata).unwrap(),
        )
        .unwrap();
        (dir, metadata.id)
    }

    fn on_disk(dir: &Path) -> ProjectMetadata {
        read_project_metadata(dir).unwrap()
    }

    fn bytes(dir: &Path) -> Vec<u8> {
        std::fs::read(get_apas_path(dir)).unwrap()
    }

    fn raw(dir: &Path) -> serde_json::Value {
        serde_json::from_slice(&bytes(dir)).unwrap()
    }

    fn pane_ids(panes: &[PaneConfig]) -> Vec<u32> {
        panes.iter().map(|pane| pane.pane_id).collect()
    }

    fn panes_of(metadata: &ProjectMetadata, machine: Option<Uuid>) -> Vec<u32> {
        metadata
            .panes
            .iter()
            .filter(|pane| pane.machine_id == machine)
            .map(|pane| pane.pane_id)
            .collect()
    }

    fn find(metadata: &ProjectMetadata, pane_id: u32) -> &PaneConfig {
        metadata
            .panes
            .iter()
            .find(|pane| pane.pane_id == pane_id)
            .unwrap_or_else(|| panic!("pane {pane_id} missing"))
    }

    /// Top-level keys in the order the file has them.
    fn key_order(dir: &Path) -> Vec<String> {
        match serde_json::from_slice::<OrderedJson>(&bytes(dir)).unwrap() {
            OrderedJson::Object(entries) => entries.into_iter().map(|(key, _)| key).collect(),
            other => panic!("not an object: {other:?}"),
        }
    }

    #[test]
    fn reading_never_creates_regenerates_or_registers() {
        let config = isolated_config_dir();
        let dir = tempfile::tempdir().unwrap();
        assert!(read_project_metadata(dir.path()).is_err());
        assert!(!get_apas_path(dir.path()).exists(), "no file was created");
        assert!(
            !config.path().join("projects.json").exists(),
            "nothing was registered"
        );
    }

    #[test]
    fn a_corrupt_file_is_an_error_everywhere_and_is_left_alone() {
        // A periodic read that regenerated it would replace the project with
        // an empty one and, on a shared home, erase every machine's panes.
        let _config = isolated_config_dir();
        let dir = tempfile::tempdir().unwrap();
        let path = get_apas_path(dir.path());
        let corrupt = b"{\"id\": \"not finished".to_vec();
        std::fs::write(&path, &corrupt).unwrap();
        let project = Uuid::new_v4();
        let me = instance(Uuid::new_v4(), "zoo-005");

        assert!(read_project_metadata(dir.path()).is_err());
        assert!(start_instance(dir.path(), project, &me).is_err());
        assert!(save_instance_panes(dir.path(), project, &me, &[pane(3, Uuid::new_v4())]).is_err());
        assert!(
            update_project_settings(dir.path(), project, |meta| meta.auto_merge_prs = true)
                .is_err()
        );
        assert!(update_pane(dir.path(), project, 3, |pane| pane.label = None).is_err());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            corrupt,
            "the file is untouched"
        );
    }

    #[test]
    fn opening_creates_a_missing_file_but_never_regenerates_a_corrupt_one() {
        // Opening registers, like every start, so the registry and the home
        // it migrates from are isolated too.
        crate::project::test_support::with_isolated_config(|| {
            let me = instance(Uuid::new_v4(), "zoo-005");
            let registered = |project: Uuid| {
                crate::project::list_registered_projects()
                    .unwrap()
                    .iter()
                    .any(|entry| entry.project_id == project.to_string())
            };

            let missing = tempfile::tempdir().unwrap();
            let start = open_instance(missing.path(), &me).unwrap();
            assert!(start.panes.is_empty());
            assert_eq!(on_disk(missing.path()).id, start.metadata.id);
            assert!(registered(start.metadata.id));

            let corrupt = tempfile::tempdir().unwrap();
            std::fs::write(get_apas_path(corrupt.path()), b"{\"id\": ").unwrap();
            assert!(open_instance(corrupt.path(), &me).is_err());
            assert_eq!(bytes(corrupt.path()), b"{\"id\": ", "not regenerated");

            let (existing, project) = project_with(vec![pane(2, Uuid::new_v4())]);
            let start = open_instance(existing.path(), &me).unwrap();
            assert_eq!(start.adopted, vec![2]);
            assert!(registered(project));
        });
    }

    #[test]
    fn opening_without_starting_reads_and_registers_but_writes_nothing() {
        // A bare launch and the legacy modes open the project this way: they
        // name it and register it, and must leave the file exactly as it is,
        // including a file from before `panes`.
        crate::project::test_support::with_isolated_config(|| {
            let dir = tempfile::tempdir().unwrap();
            let project = Uuid::new_v4();
            let deadloop = Uuid::new_v4();
            let legacy = serde_json::json!({
                "id": project,
                "name": "legacy",
                "created_at": "2026-01-01T00:00:00Z",
                "deadloop_claude_session_id": deadloop,
                "future_setting": "kept",
            })
            .to_string()
            .into_bytes();
            std::fs::write(get_apas_path(dir.path()), &legacy).unwrap();

            let metadata = open_project_metadata(dir.path()).unwrap();

            assert_eq!(metadata.id, project);
            assert_eq!(
                metadata
                    .get_pane(shared::PANE_ID_DEADLOOP)
                    .map(|pane| pane.session_id),
                Some(deadloop),
                "legacy panes are migrated in the copy returned"
            );
            assert_eq!(bytes(dir.path()), legacy, "and not in the file");
            assert!(crate::project::list_registered_projects()
                .unwrap()
                .iter()
                .any(|entry| entry.project_id == project.to_string()));

            let corrupt = tempfile::tempdir().unwrap();
            std::fs::write(get_apas_path(corrupt.path()), b"{\"id\": ").unwrap();
            assert!(open_project_metadata(corrupt.path()).is_err());
            assert_eq!(bytes(corrupt.path()), b"{\"id\": ", "not regenerated");
        });
    }

    #[test]
    fn a_file_that_belongs_to_another_project_is_not_written() {
        let _config = isolated_config_dir();
        let (dir, _) = project_with(vec![pane(2, Uuid::new_v4())]);
        let before = bytes(dir.path());
        let other = Uuid::new_v4();
        let me = instance(Uuid::new_v4(), "zoo-005");
        assert!(save_instance_panes(dir.path(), other, &me, &[]).is_err());
        assert!(start_instance(dir.path(), other, &me).is_err());
        assert_eq!(bytes(dir.path()), before);
    }

    #[test]
    fn saving_replaces_only_this_machines_panes() {
        let _config = isolated_config_dir();
        let (me, other) = (Uuid::new_v4(), Uuid::new_v4());
        let mine = pane(2, Uuid::new_v4());
        let (dir, project) = project_with(vec![
            tagged(pane(1, Uuid::new_v4()), me, "zoo-005"),
            tagged(mine.clone(), me, "zoo-005"),
            tagged(pane(3, Uuid::new_v4()), other, "zoo-006"),
            pane(4, Uuid::new_v4()),
        ]);

        let mut renamed = mine.clone();
        renamed.label = Some("renamed".to_string());
        let added = pane(5, Uuid::new_v4());
        let saved = save_instance_panes(
            dir.path(),
            project,
            &instance(me, "zoo-005"),
            &[added, renamed],
        )
        .unwrap();

        let disk = on_disk(dir.path());
        assert_eq!(
            pane_ids(&disk.panes),
            vec![2, 3, 4, 5],
            "pane 1 is gone, in id order"
        );
        assert_eq!(panes_of(&disk, Some(me)), vec![2, 5]);
        assert_eq!(panes_of(&disk, Some(other)), vec![3]);
        assert_eq!(
            panes_of(&disk, None),
            vec![4],
            "untagged panes are left alone"
        );
        assert_eq!(find(&disk, 2).label.as_deref(), Some("renamed"));
        assert_eq!(find(&disk, 5).hostname.as_deref(), Some("zoo-005"));
        assert_eq!(
            pane_ids(&saved.panes),
            pane_ids(&disk.panes),
            "returns the file as written"
        );
    }

    #[test]
    fn an_empty_roster_keeps_every_other_machines_panes() {
        let _config = isolated_config_dir();
        let (me, other) = (Uuid::new_v4(), Uuid::new_v4());
        let (dir, project) = project_with(vec![
            tagged(pane(1, Uuid::new_v4()), me, "zoo-005"),
            tagged(pane(2, Uuid::new_v4()), other, "zoo-006"),
            tagged(pane(3, Uuid::new_v4()), other, "zoo-006"),
            pane(4, Uuid::new_v4()),
        ]);

        save_instance_panes(dir.path(), project, &instance(me, "zoo-005"), &[]).unwrap();

        let disk = on_disk(dir.path());
        assert_eq!(panes_of(&disk, Some(me)), Vec::<u32>::new());
        assert_eq!(panes_of(&disk, Some(other)), vec![2, 3]);
        assert_eq!(panes_of(&disk, None), vec![4]);
    }

    #[test]
    fn project_level_fields_are_never_taken_from_the_saver() {
        let _config = isolated_config_dir();
        let (me, other) = (Uuid::new_v4(), Uuid::new_v4());
        let (dir, project) = project_with(vec![]);
        // Another machine changes the flags after this one last read them.
        update_project_settings(dir.path(), project, |meta| {
            meta.auto_approve_todos = true;
            meta.disallowed_tab_types = vec!["terminal:codex".to_string()];
        })
        .unwrap();
        save_instance_panes(
            dir.path(),
            project,
            &instance(other, "zoo-006"),
            &[pane(7, Uuid::new_v4())],
        )
        .unwrap();

        save_instance_panes(
            dir.path(),
            project,
            &instance(me, "zoo-005"),
            &[pane(2, Uuid::new_v4())],
        )
        .unwrap();

        let disk = on_disk(dir.path());
        assert!(disk.auto_approve_todos);
        assert_eq!(
            disk.disallowed_tab_types,
            vec!["terminal:codex".to_string()]
        );
        assert_eq!(pane_ids(&disk.panes), vec![2, 7]);
    }

    #[test]
    fn unknown_fields_and_key_order_survive_a_merge() {
        // A newer build on another machine may write fields this one does not
        // know, at the top level and in its panes. Losing them on every save
        // here would undo that build's work.
        let _config = isolated_config_dir();
        let (me, other) = (Uuid::new_v4(), Uuid::new_v4());
        let project = Uuid::new_v4();
        let (mine, theirs) = (Uuid::new_v4(), Uuid::new_v4());
        let dir = tempfile::tempdir().unwrap();
        let original = format!(
            r#"{{
  "name": "demo",
  "id": "{project}",
  "future_setting": {{"keep": [1, 2, 3]}},
  "created_at": "2026-01-01T00:00:00Z",
  "panes": [
    {{
      "pane_id": 2,
      "provider": "codex",
      "mode": "interactive",
      "kind": "terminal",
      "session_id": "{mine}",
      "label": "Mine",
      "future_pane_field": "mine",
      "machine_id": "{me}",
      "hostname": "zoo-005"
    }},
    {{
      "pane_id": 3,
      "provider": "claude",
      "mode": "interactive",
      "session_id": "{theirs}",
      "future_pane_field": {{"nested": true}},
      "label": "Theirs",
      "machine_id": "{other}",
      "hostname": "zoo-006"
    }}
  ],
  "auto_merge_prs": true
}}"#
        );
        std::fs::write(get_apas_path(dir.path()), &original).unwrap();
        let keys_before = key_order(dir.path());

        let mut renamed = pane(2, mine);
        renamed.label = Some("Renamed".to_string());
        save_instance_panes(dir.path(), project, &instance(me, "zoo-005"), &[renamed]).unwrap();

        assert_eq!(
            key_order(dir.path()),
            keys_before,
            "top-level key order is kept"
        );
        let value = raw(dir.path());
        assert_eq!(
            value["future_setting"],
            serde_json::json!({"keep": [1, 2, 3]})
        );
        assert_eq!(value["auto_merge_prs"], serde_json::json!(true));
        let panes = value["panes"].as_array().unwrap();
        assert_eq!(panes[0]["label"], serde_json::json!("Renamed"));
        assert_eq!(panes[0]["future_pane_field"], serde_json::json!("mine"));
        assert_eq!(
            panes[1]["future_pane_field"],
            serde_json::json!({"nested": true}),
            "another machine's pane keeps what its build wrote"
        );
        // The other machine's entry is exactly as it was, key order included.
        let original_value: serde_json::Value = serde_json::from_str(&original).unwrap();
        assert_eq!(panes[1], original_value["panes"][1]);
        let theirs_keys: Vec<String> =
            match serde_json::from_slice::<OrderedJson>(&bytes(dir.path())).unwrap() {
                OrderedJson::Object(root) => match field(&root, "panes") {
                    Some(OrderedJson::Array(items)) => match &items[1] {
                        OrderedJson::Object(entry) => {
                            entry.iter().map(|(key, _)| key.clone()).collect()
                        }
                        other => panic!("{other:?}"),
                    },
                    other => panic!("{other:?}"),
                },
                other => panic!("{other:?}"),
            };
        assert_eq!(
            theirs_keys,
            [
                "pane_id",
                "provider",
                "mode",
                "session_id",
                "future_pane_field",
                "label",
                "machine_id",
                "hostname"
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_write_that_changes_nothing_leaves_the_file_alone() {
        // An idle save must not rewrite shared metadata or move its mtime.
        use std::os::unix::fs::MetadataExt;
        let _config = isolated_config_dir();
        let me = Uuid::new_v4();
        let panes = vec![
            tagged(pane(1, Uuid::new_v4()), me, "zoo-005"),
            tagged(pane(2, Uuid::new_v4()), me, "zoo-005"),
        ];
        let (dir, project) = project_with(panes.clone());
        let before = std::fs::metadata(get_apas_path(dir.path())).unwrap();
        let contents = bytes(dir.path());

        save_instance_panes(dir.path(), project, &instance(me, "zoo-005"), &panes).unwrap();
        start_instance(dir.path(), project, &instance(me, "zoo-005")).unwrap();
        update_project_settings(dir.path(), project, |_| ()).unwrap();
        assert_eq!(update_pane(dir.path(), project, 9, |_| ()).unwrap(), None);

        let after = std::fs::metadata(get_apas_path(dir.path())).unwrap();
        assert_eq!(after.ino(), before.ino(), "not replaced by a rename");
        assert_eq!(bytes(dir.path()), contents);
    }

    #[test]
    fn an_instance_alone_adopts_untagged_panes() {
        let _config = isolated_config_dir();
        let (me, other) = (Uuid::new_v4(), Uuid::new_v4());
        let (dir, project) = project_with(vec![
            pane(1, Uuid::new_v4()),
            pane(2, Uuid::new_v4()),
            tagged(pane(3, Uuid::new_v4()), other, "zoo-006"),
        ]);

        let start = start_instance(dir.path(), project, &instance(me, "zoo-005")).unwrap();

        assert_eq!(start.adopted, vec![1, 2]);
        assert_eq!(pane_ids(&start.panes), vec![1, 2]);
        assert!(
            start
                .panes
                .iter()
                .all(|pane| pane.machine_id == Some(me)
                    && pane.hostname.as_deref() == Some("zoo-005"))
        );
        let disk = on_disk(dir.path());
        assert_eq!(panes_of(&disk, Some(me)), vec![1, 2], "adoption is written");
        assert_eq!(
            panes_of(&disk, Some(other)),
            vec![3],
            "never another machine's"
        );
        assert_eq!(pane_ids(&start.metadata.panes), vec![1, 2, 3]);
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    /// The live claim `host`'s daemon writes for `machine` before it starts an
    /// instance of `project`, claimed at `claimed_at`. Returns its path, which
    /// that daemon removes when the instance stops. Written by hand because
    /// `claim_project` stamps the current second, and two claims made in one
    /// second would be ordered by their random machine ids.
    fn claim_written_by(
        config: &Path,
        project: Uuid,
        machine: Uuid,
        host: &str,
        claimed_at: u64,
    ) -> PathBuf {
        let path = config
            .join("claims")
            .join(project.to_string())
            .join(format!("{machine}.json"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            serde_json::json!({
                "project_id": project.to_string(),
                "project_path": "/home/u/demo",
                "hostname": host,
                "machine_id": machine,
                "pid": 4242,
                "claimed_at": claimed_at,
                "heartbeat": now(),
                "instances": true,
            })
            .to_string(),
        )
        .unwrap();
        path
    }

    #[test]
    fn a_second_instance_leaves_untagged_panes_to_the_running_one() {
        // zoo-005 runs the project; its daemon claimed it per machine when it
        // started it. An instance starting on zoo-006 after that restores
        // nothing.
        let config = isolated_config_dir();
        let (me, running) = (Uuid::new_v4(), Uuid::new_v4());
        let (dir, project) = project_with(vec![pane(1, Uuid::new_v4()), pane(2, Uuid::new_v4())]);
        let running_claim =
            claim_written_by(config.path(), project, running, "zoo-005", now() - 100);
        // This machine's own claim, which its daemon wrote before starting it,
        // does not count against it: it is what orders the two starts.
        claim_project(
            config.path(),
            &project.to_string(),
            &dir.path().display().to_string(),
            me,
            ClaimMode::Instances,
        )
        .unwrap();
        let before = bytes(dir.path());

        let start = start_instance(dir.path(), project, &instance(me, "zoo-006")).unwrap();

        assert!(start.adopted.is_empty());
        assert!(start.panes.is_empty());
        assert_eq!(bytes(dir.path()), before, "nothing was tagged");

        // Once zoo-005 stops and its daemon releases its claim, the next start
        // adopts them.
        std::fs::remove_file(running_claim).unwrap();
        let start = start_instance(dir.path(), project, &instance(me, "zoo-006")).unwrap();
        assert_eq!(start.adopted, vec![1, 2]);
    }

    /// Both daemons claim before either instance reaches its adoption check.
    /// When the mere presence of another claim ruled adoption out, two
    /// instances starting together each saw the other, neither adopted, and
    /// the untagged tabs stayed on no machine while listed under one. The
    /// earlier claimant adopts them, whichever instance reaches the file first.
    #[test]
    fn instances_starting_together_leave_untagged_panes_to_the_first_claimant() {
        let config = isolated_config_dir();
        for zoo5_reaches_the_file_first in [true, false] {
            let (zoo5, zoo6) = (Uuid::new_v4(), Uuid::new_v4());
            let (dir, project) =
                project_with(vec![pane(1, Uuid::new_v4()), pane(2, Uuid::new_v4())]);
            // Both claims are on disk before either instance starts, zoo-005's
            // a second older than zoo-006's.
            let claimed = now();
            claim_written_by(config.path(), project, zoo5, "zoo-005", claimed - 1);
            claim_written_by(config.path(), project, zoo6, "zoo-006", claimed);
            let start_on_5 =
                || start_instance(dir.path(), project, &instance(zoo5, "zoo-005")).unwrap();
            let start_on_6 =
                || start_instance(dir.path(), project, &instance(zoo6, "zoo-006")).unwrap();

            let (on_5, on_6) = if zoo5_reaches_the_file_first {
                let on_5 = start_on_5();
                (on_5, start_on_6())
            } else {
                let on_6 = start_on_6();
                (start_on_5(), on_6)
            };

            let case = format!("zoo-005 first: {zoo5_reaches_the_file_first}");
            assert_eq!(on_5.adopted, vec![1, 2], "{case}");
            assert_eq!(pane_ids(&on_5.panes), vec![1, 2], "{case}");
            assert!(on_6.adopted.is_empty(), "{case}");
            assert!(on_6.panes.is_empty(), "{case}");
            assert_eq!(
                panes_of(&on_disk(dir.path()), Some(zoo5)),
                vec![1, 2],
                "{case}"
            );
        }
    }

    /// Claimed in the same second, the two are ordered by machine id, so
    /// exactly one of them adopts.
    #[test]
    fn claimants_of_the_same_second_are_ordered_by_machine_id() {
        let config = isolated_config_dir();
        let mut ids = [Uuid::new_v4(), Uuid::new_v4()];
        ids.sort();
        let (smaller, larger) = (ids[0], ids[1]);
        let (dir, project) = project_with(vec![pane(1, Uuid::new_v4())]);
        let claimed = now();
        claim_written_by(config.path(), project, smaller, "zoo-005", claimed);
        claim_written_by(config.path(), project, larger, "zoo-006", claimed);

        let on_larger = start_instance(dir.path(), project, &instance(larger, "zoo-006")).unwrap();
        assert!(on_larger.adopted.is_empty());
        let on_smaller =
            start_instance(dir.path(), project, &instance(smaller, "zoo-005")).unwrap();
        assert_eq!(on_smaller.adopted, vec![1]);
    }

    #[test]
    fn an_exclusive_daemon_running_the_project_blocks_adoption() {
        // A daemon from before instances holds the project-wide claim without
        // the instances marker: it runs every untagged pane itself.
        let config = isolated_config_dir();
        let (dir, project) = project_with(vec![pane(1, Uuid::new_v4())]);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let guard = config.path().join("claims").join(format!("{project}.json"));
        std::fs::create_dir_all(guard.parent().unwrap()).unwrap();
        std::fs::write(
            &guard,
            serde_json::json!({
                "project_id": project.to_string(),
                "project_path": dir.path().display().to_string(),
                "hostname": "zoo-older",
                "machine_id": Uuid::new_v4(),
                "pid": 4242,
                "claimed_at": now,
                "heartbeat": now,
            })
            .to_string(),
        )
        .unwrap();

        let start =
            start_instance(dir.path(), project, &instance(Uuid::new_v4(), "zoo-005")).unwrap();
        assert!(start.adopted.is_empty());
        assert!(start.panes.is_empty());
        assert_eq!(panes_of(&on_disk(dir.path()), None), vec![1]);
    }

    #[test]
    fn an_instance_never_takes_a_stopped_machines_panes() {
        // Tags are permanent: zoo-006 starting alone, with zoo-005 stopped,
        // does not restore or adopt zoo-005's tabs.
        let _config = isolated_config_dir();
        let (me, stopped) = (Uuid::new_v4(), Uuid::new_v4());
        let (dir, project) = project_with(vec![
            tagged(pane(1, Uuid::new_v4()), stopped, "zoo-005"),
            tagged(pane(2, Uuid::new_v4()), me, "zoo-006"),
            pane(3, Uuid::new_v4()),
        ]);

        let start = start_instance(dir.path(), project, &instance(me, "zoo-006")).unwrap();

        assert_eq!(pane_ids(&start.panes), vec![2, 3]);
        assert_eq!(start.adopted, vec![3]);
        let disk = on_disk(dir.path());
        assert_eq!(panes_of(&disk, Some(stopped)), vec![1]);
        assert_eq!(find(&disk, 1).hostname.as_deref(), Some("zoo-005"));
    }

    #[test]
    fn an_untagged_pane_sharing_an_id_is_not_adopted() {
        let _config = isolated_config_dir();
        let (me, other) = (Uuid::new_v4(), Uuid::new_v4());
        let (dir, project) = project_with(vec![
            pane(4, Uuid::new_v4()),
            tagged(pane(4, Uuid::new_v4()), other, "zoo-006"),
            pane(5, Uuid::new_v4()),
        ]);

        let start = start_instance(dir.path(), project, &instance(me, "zoo-005")).unwrap();

        assert_eq!(start.adopted, vec![5]);
        assert_eq!(pane_ids(&start.panes), vec![5]);
        let disk = on_disk(dir.path());
        assert_eq!(panes_of(&disk, None), vec![4]);
        assert_eq!(panes_of(&disk, Some(other)), vec![4]);
    }

    #[test]
    fn a_file_from_before_panes_is_adopted_and_written_out() {
        let _config = isolated_config_dir();
        let dir = tempfile::tempdir().unwrap();
        let project = Uuid::new_v4();
        let (deadloop, interactive) = (Uuid::new_v4(), Uuid::new_v4());
        std::fs::write(
            get_apas_path(dir.path()),
            serde_json::json!({
                "id": project,
                "name": "legacy",
                "created_at": "2026-01-01T00:00:00Z",
                "deadloop_claude_session_id": deadloop,
                "interactive_claude_session_id": interactive,
                "is_paused": true,
            })
            .to_string(),
        )
        .unwrap();
        let me = Uuid::new_v4();

        let start = start_instance(dir.path(), project, &instance(me, "zoo-005")).unwrap();

        assert_eq!(start.adopted, vec![1, 2]);
        let disk = on_disk(dir.path());
        assert_eq!(panes_of(&disk, Some(me)), vec![1, 2]);
        assert_eq!(find(&disk, 1).session_id, deadloop);
        assert_eq!(find(&disk, 2).session_id, interactive);
        assert!(find(&disk, 1).is_paused, "the legacy pause carries over");
        assert_eq!(
            raw(dir.path())["deadloop_claude_session_id"],
            serde_json::json!(deadloop),
            "the legacy fields stay for older readers"
        );
    }

    #[test]
    fn a_sole_runtime_restores_everything_and_saves_it_untagged() {
        // Exactly what a CLI from before instances does, which is what a host
        // with `daemon.machine_id` set must keep doing.
        let _config = isolated_config_dir();
        let other = Uuid::new_v4();
        let kept = tagged(pane(2, Uuid::new_v4()), other, "zoo-006");
        let (dir, project) = project_with(vec![
            pane(1, Uuid::new_v4()),
            kept.clone(),
            tagged(pane(3, Uuid::new_v4()), other, "zoo-006"),
        ]);
        let before = bytes(dir.path());

        let start = start_instance(dir.path(), project, &PaneOwner::Sole).unwrap();
        assert_eq!(pane_ids(&start.panes), vec![1, 2, 3]);
        assert!(start.adopted.is_empty());
        assert_eq!(bytes(dir.path()), before, "starting writes nothing");

        let mut added = pane(4, Uuid::new_v4());
        added.machine_id = Some(Uuid::new_v4());
        save_instance_panes(dir.path(), project, &PaneOwner::Sole, &[added, kept]).unwrap();

        let disk = on_disk(dir.path());
        assert_eq!(
            pane_ids(&disk.panes),
            vec![2, 4],
            "the whole list is replaced"
        );
        assert!(disk
            .panes
            .iter()
            .all(|pane| pane.machine_id.is_none() && pane.hostname.is_none()));
    }

    #[test]
    fn an_untagged_copy_of_this_instances_pane_is_taken_back() {
        // A CLI from before instances saved this machine's pane untagged.
        let _config = isolated_config_dir();
        let me = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (dir, project) = project_with(vec![pane(2, session), pane(3, Uuid::new_v4())]);

        save_instance_panes(
            dir.path(),
            project,
            &instance(me, "zoo-005"),
            &[pane(2, session), pane(3, Uuid::new_v4())],
        )
        .unwrap();

        let disk = on_disk(dir.path());
        assert_eq!(panes_of(&disk, Some(me)), vec![2, 3]);
        assert_eq!(
            panes_of(&disk, None),
            vec![3],
            "a different conversation under the same id is not this pane, so it stays"
        );
    }

    #[test]
    fn the_legacy_pause_follows_only_the_deadloop_panes_owner() {
        let _config = isolated_config_dir();
        let (me, other) = (Uuid::new_v4(), Uuid::new_v4());
        let mut paused = pane(shared::PANE_ID_DEADLOOP, Uuid::new_v4());
        paused.mode = shared::PaneMode::Deadloop;
        paused.is_paused = true;
        let (dir, project) = project_with(vec![]);

        save_instance_panes(
            dir.path(),
            project,
            &instance(me, "zoo-005"),
            &[paused.clone()],
        )
        .unwrap();
        assert!(on_disk(dir.path()).is_paused);

        // Another machine's saves do not move it.
        save_instance_panes(
            dir.path(),
            project,
            &instance(other, "zoo-006"),
            &[pane(5, Uuid::new_v4())],
        )
        .unwrap();
        assert!(on_disk(dir.path()).is_paused);

        // Its owner resuming it does.
        let mut resumed = paused.clone();
        resumed.is_paused = false;
        save_instance_panes(dir.path(), project, &instance(me, "zoo-005"), &[resumed]).unwrap();
        assert!(!on_disk(dir.path()).is_paused);

        // With no deadloop pane left anywhere, it clears.
        update_project_settings(dir.path(), project, |meta| meta.is_paused = true).unwrap();
        save_instance_panes(dir.path(), project, &instance(me, "zoo-005"), &[]).unwrap();
        assert!(!on_disk(dir.path()).is_paused);

        // An untagged pane with the same id, left behind beside this
        // machine's, does not hide this machine's pause.
        let (dir, project) = project_with(vec![pane(shared::PANE_ID_DEADLOOP, Uuid::new_v4())]);
        save_instance_panes(dir.path(), project, &instance(me, "zoo-005"), &[paused]).unwrap();
        let disk = on_disk(dir.path());
        assert_eq!(panes_of(&disk, None), vec![shared::PANE_ID_DEADLOOP]);
        assert_eq!(panes_of(&disk, Some(me)), vec![shared::PANE_ID_DEADLOOP]);
        assert!(disk.is_paused);
    }

    #[test]
    fn a_settings_write_changes_only_its_fields() {
        let _config = isolated_config_dir();
        let other = Uuid::new_v4();
        let (dir, project) = project_with(vec![]);
        // Another machine saved a pane after this one last read the file.
        save_instance_panes(
            dir.path(),
            project,
            &instance(other, "zoo-006"),
            &[pane(7, Uuid::new_v4())],
        )
        .unwrap();

        let echo = update_project_settings(dir.path(), project, |meta| {
            meta.auto_merge_prs = true;
            // Neither of these is a setting.
            meta.panes.clear();
            meta.id = Uuid::new_v4();
            (meta.team_enabled, meta.disallowed_tab_types.clone())
        })
        .unwrap();

        assert_eq!(echo, (false, Vec::<String>::new()));
        let disk = on_disk(dir.path());
        assert!(disk.auto_merge_prs);
        assert_eq!(disk.id, project);
        assert_eq!(
            panes_of(&disk, Some(other)),
            vec![7],
            "the other machine's pane survives"
        );
    }

    #[test]
    fn a_pane_update_changes_one_pane_and_keeps_its_owner() {
        let _config = isolated_config_dir();
        let (other, intruder) = (Uuid::new_v4(), Uuid::new_v4());
        let (dir, project) = project_with(vec![
            tagged(pane(3, Uuid::new_v4()), other, "zoo-006"),
            pane(4, Uuid::new_v4()),
        ]);
        let untouched = raw(dir.path())["panes"][1].clone();

        let result = update_pane(dir.path(), project, 3, |pane| {
            pane.worktree_path = Some("/work/tree".to_string());
            pane.machine_id = Some(intruder);
            pane.pane_id = 9;
            "done"
        })
        .unwrap();

        assert_eq!(result, Some("done"));
        let disk = on_disk(dir.path());
        let updated = find(&disk, 3);
        assert_eq!(updated.worktree_path.as_deref(), Some("/work/tree"));
        assert_eq!(
            updated.machine_id,
            Some(other),
            "a pane never moves machines"
        );
        assert_eq!(raw(dir.path())["panes"][1], untouched);

        // Ambiguous ids are refused rather than guessed.
        save_instance_panes(
            dir.path(),
            project,
            &instance(intruder, "zoo-007"),
            &[pane(4, Uuid::new_v4())],
        )
        .unwrap();
        let before = bytes(dir.path());
        assert!(update_pane(dir.path(), project, 4, |pane| pane.label = None).is_err());
        assert_eq!(bytes(dir.path()), before);
    }

    #[test]
    fn a_missing_file_is_recreated_by_a_save_but_not_by_a_start() {
        let _config = isolated_config_dir();
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("checkout");
        std::fs::create_dir(&dir).unwrap();
        let (project, me) = (Uuid::new_v4(), Uuid::new_v4());

        assert!(start_instance(&dir, project, &instance(me, "zoo-005")).is_err());
        assert_eq!(update_pane(&dir, project, 1, |_| ()).unwrap(), None);
        assert!(
            !get_apas_path(&dir).exists(),
            "nothing to write, nothing written"
        );

        save_instance_panes(
            &dir,
            project,
            &instance(me, "zoo-005"),
            &[pane(2, Uuid::new_v4())],
        )
        .unwrap();
        let disk = on_disk(&dir);
        assert_eq!(disk.id, project);
        assert_eq!(disk.name.as_deref(), Some("checkout"));
        assert_eq!(panes_of(&disk, Some(me)), vec![2]);
    }

    #[test]
    fn concurrent_instances_and_settings_writes_keep_every_change() {
        // Two machines on one home save their own panes while a third writer
        // flips a project flag, all at once. Each change must survive.
        let config = isolated_config_dir();
        let (dir, project) = project_with(vec![]);
        let rounds = 30u32;
        let start = Arc::new(Barrier::new(3));
        let savers: Vec<_> = [(Uuid::new_v4(), 100u32), (Uuid::new_v4(), 200u32)]
            .into_iter()
            .map(|(machine, first)| {
                let (config_dir, dir, start) = (
                    config.path().to_path_buf(),
                    dir.path().to_path_buf(),
                    start.clone(),
                );
                std::thread::spawn(move || {
                    with_config_dir(&config_dir, || {
                        let owner = instance(machine, &format!("host-{first}"));
                        let mut panes = Vec::new();
                        start.wait();
                        for round in 0..rounds {
                            panes.push(pane(first + round, Uuid::new_v4()));
                            save_instance_panes(&dir, project, &owner, &panes).unwrap();
                        }
                        machine
                    })
                })
            })
            .collect();
        let flipper = {
            let (config_dir, dir, start) = (
                config.path().to_path_buf(),
                dir.path().to_path_buf(),
                start.clone(),
            );
            std::thread::spawn(move || {
                with_config_dir(&config_dir, || {
                    start.wait();
                    for round in 0..rounds {
                        update_project_settings(&dir, project, |meta| {
                            meta.auto_merge_prs = round % 2 == 0;
                        })
                        .unwrap();
                    }
                })
            })
        };
        let machines: Vec<Uuid> = savers
            .into_iter()
            .map(|saver| saver.join().unwrap())
            .collect();
        flipper.join().unwrap();

        let disk = on_disk(dir.path());
        assert_eq!(
            panes_of(&disk, Some(machines[0])),
            (100..100 + rounds).collect::<Vec<_>>()
        );
        assert_eq!(
            panes_of(&disk, Some(machines[1])),
            (200..200 + rounds).collect::<Vec<_>>()
        );
        assert_eq!(
            disk.auto_merge_prs,
            (rounds - 1) % 2 == 0,
            "the last flag write stands"
        );
    }

    /// The parameters [`save_worker`] reads, one per line: config dir,
    /// project dir, project id, machine id, first pane id, rounds.
    const WORKER_SPEC: &str = "APAS_TEST_PROJECT_INSTANCE_WORKER";

    /// One instance's saves, run in a child process by
    /// `instances_in_separate_processes_keep_each_others_panes`, so the two
    /// instances share nothing but the files, as on two hosts.
    #[test]
    #[ignore = "run as a child process by instances_in_separate_processes_keep_each_others_panes"]
    fn save_worker() {
        let Ok(spec) = std::env::var(WORKER_SPEC) else {
            return;
        };
        let fields: Vec<&str> = spec.lines().collect();
        let [config_dir, dir, project, machine, first, rounds] = fields[..] else {
            panic!("malformed worker spec: {spec:?}");
        };
        let project: Uuid = project.parse().unwrap();
        let machine: Uuid = machine.parse().unwrap();
        let (first, rounds): (u32, u32) = (first.parse().unwrap(), rounds.parse().unwrap());
        with_config_dir(Path::new(config_dir), || {
            let owner = instance(machine, &format!("host-{first}"));
            let mut panes = Vec::new();
            for round in 0..rounds {
                panes.push(pane(first + round, Uuid::new_v4()));
                save_instance_panes(Path::new(dir), project, &owner, &panes).unwrap();
            }
        });
    }

    #[test]
    fn instances_in_separate_processes_keep_each_others_panes() {
        let config = isolated_config_dir();
        let (dir, project) = project_with(vec![]);
        let logs = tempfile::tempdir().unwrap();
        let rounds = 25u32;
        let machines = [(Uuid::new_v4(), 100u32), (Uuid::new_v4(), 300u32)];
        let mut children: Vec<(std::process::Child, PathBuf)> = machines
            .iter()
            .map(|(machine, first)| {
                let log = logs.path().join(format!("worker-{first}.log"));
                let spec = [
                    config.path().display().to_string(),
                    dir.path().display().to_string(),
                    project.to_string(),
                    machine.to_string(),
                    first.to_string(),
                    rounds.to_string(),
                ]
                .join("\n");
                let child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "project_instances::tests::save_worker",
                        "--ignored",
                        "--test-threads=1",
                    ])
                    .env(WORKER_SPEC, spec)
                    .stdout(File::create(&log).unwrap())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .unwrap();
                (child, log)
            })
            .collect();

        let deadline = Instant::now() + Duration::from_secs(120);
        for (child, log) in &mut children {
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if Instant::now() > deadline {
                    let _ = child.kill();
                    panic!(
                        "worker did not finish: {}",
                        std::fs::read_to_string(log).unwrap_or_default()
                    );
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            let output = std::fs::read_to_string(&log).unwrap_or_default();
            assert!(status.success(), "worker failed: {output}");
            assert!(output.contains("1 passed"), "the worker test ran: {output}");
        }

        let disk = on_disk(dir.path());
        for (machine, first) in machines {
            assert_eq!(
                panes_of(&disk, Some(machine)),
                (first..first + rounds).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn a_write_waits_for_the_file_lock() {
        // Held by another process on another host in production; another
        // open file description stands in for it here.
        let config = isolated_config_dir();
        let (dir, project) = project_with(vec![]);
        let holder = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(metadata_lock_path(project).unwrap())
            .unwrap();
        FileExt::lock_exclusive(&holder).unwrap();
        assert!(metadata_lock_path(project)
            .unwrap()
            .starts_with(config.path().join("locks")));

        let me = Uuid::new_v4();
        let saver = {
            let (config_dir, dir) = (config.path().to_path_buf(), dir.path().to_path_buf());
            std::thread::spawn(move || {
                with_config_dir(&config_dir, || {
                    save_instance_panes(
                        &dir,
                        project,
                        &instance(me, "zoo-005"),
                        &[pane(2, Uuid::new_v4())],
                    )
                })
            })
        };
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            !saver.is_finished(),
            "the save waits while the lock is held"
        );
        assert!(on_disk(dir.path()).panes.is_empty());

        FileExt::unlock(&holder).unwrap();
        saver.join().unwrap().unwrap();
        assert_eq!(panes_of(&on_disk(dir.path()), Some(me)), vec![2]);
    }

    #[test]
    fn a_lock_that_stays_held_fails_the_write_instead_of_writing_unlocked() {
        let _config = isolated_config_dir();
        let (dir, project) = project_with(vec![pane(1, Uuid::new_v4())]);
        let before = bytes(dir.path());
        let holder = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(metadata_lock_path(project).unwrap())
            .unwrap();
        FileExt::lock_exclusive(&holder).unwrap();

        let me = instance(Uuid::new_v4(), "zoo-005");
        let error = transact(
            dir.path(),
            project,
            Duration::from_millis(150),
            WhenMissing::Create,
            |document| {
                document.replace_owned_panes(&me, &[pane(2, Uuid::new_v4())]);
                Ok(())
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("Timed out"), "{error:#}");
        assert!(
            start_instance_with(dir.path(), project, &me, Duration::from_millis(150), || {
                false
            })
            .is_err()
        );
        assert_eq!(bytes(dir.path()), before);
    }

    #[test]
    fn a_thread_of_this_process_waits_for_another() {
        // On NFS the file lock cannot tell two threads of one process apart.
        let _config = isolated_config_dir();
        let project = Uuid::new_v4();
        let held = lock_project_metadata(project).unwrap();
        assert!(lock_project_metadata_within(project, Duration::from_millis(100)).is_err());
        drop(held);
        lock_project_metadata_within(project, Duration::from_millis(100)).unwrap();
    }

    #[test]
    fn identity_and_ownership_follow_the_daemon_machine_id_setting() {
        let machine = Uuid::new_v4();
        let capable = InstanceIdentity::new(machine, &DaemonConfig::default());
        assert!(capable.capable);
        assert_eq!(capable.owner().machine_id(), Some(machine));

        let configured = DaemonConfig {
            machine_id: Some(Uuid::new_v4().to_string()),
            ..DaemonConfig::default()
        };
        let sole = InstanceIdentity::new(machine, &configured);
        assert!(
            !sole.capable,
            "a shared configured id cannot tell instances apart"
        );
        assert_eq!(sole.owner(), PaneOwner::Sole);

        let mut reported = tagged(pane(2, Uuid::new_v4()), Uuid::new_v4(), "elsewhere");
        sole.owner().tag(&mut reported);
        assert_eq!(
            (reported.machine_id, reported.hostname.clone()),
            (None, None)
        );
        let owner = instance(machine, "zoo-005");
        owner.tag(&mut reported);
        assert!(owner.owns(&reported));
        assert_eq!(reported.hostname.as_deref(), Some("zoo-005"));
        assert!(PaneOwner::Sole.owns(&reported));
    }

    /// A daemon can run a project as its only runtime where `config.toml`
    /// allows instances, but can never make an instance of a run the setting
    /// rules out.
    #[test]
    fn a_daemon_can_take_instances_away_but_never_grant_them() {
        let machine = Uuid::new_v4();
        let alone = InstanceIdentity::new(machine, &DaemonConfig::default()).with_instances(false);
        assert!(!alone.capable);
        assert_eq!(alone.owner(), PaneOwner::Sole);
        assert_eq!(
            (alone.machine_id, alone.hostname.clone()),
            (machine, current_hostname()),
            "still the machine it is"
        );
        assert!(
            InstanceIdentity::new(machine, &DaemonConfig::default())
                .with_instances(true)
                .capable
        );

        let configured = DaemonConfig {
            machine_id: Some(Uuid::new_v4().to_string()),
            ..DaemonConfig::default()
        };
        assert!(
            !InstanceIdentity::new(machine, &configured)
                .with_instances(true)
                .capable
        );
    }
}
