use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use shared::PaneConfig;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

const APAS_FILE: &str = ".apas";
const USER_PROJECTS_FILE: &str = "projects.json";
const LEGACY_USER_REGISTRY_DIR: &str = ".apas";
static APAS_METADATA_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredProject {
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ProjectRegistry {
    #[serde(default)]
    projects: Vec<RegisteredProject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMetadata {
    /// Unique identifier for this project (APAS session ID)
    pub id: Uuid,
    /// Optional human-readable project name
    pub name: Option<String>,
    /// When the project was first initialized
    pub created_at: String,
    /// Custom prompt to use (if not set, uses default)
    #[serde(default)]
    pub prompt: Option<String>,
    /// Dynamic pane configurations
    #[serde(default)]
    pub panes: Vec<PaneConfig>,

    /// Tech-Lead autonomy: when true, the Tech Lead may flip Global
    /// TODOs from `proposed` → `approved` without a human click in
    /// the Overview.
    #[serde(default)]
    pub auto_approve_todos: bool,

    /// Tech-Lead autonomy: when true, the Tech Lead may `gh pr merge`
    /// (or close with a rejection comment, or post a "needs more work"
    /// review) on the PRs of `pr_open` Global TODOs during its loop.
    #[serde(default)]
    pub auto_merge_prs: bool,

    /// Whether managed team mode (Manager / Tech Lead / Developer /
    /// Reviewer) is available for this project.
    ///
    /// Off unless explicitly enabled, and `serde(default)` makes that true
    /// for `.apas` files written before this field existed too -- an
    /// upgrade turns team mode off everywhere until a project's owner or
    /// admin opts back in. That is deliberate: team mode spawns autonomous
    /// panes that can open PRs, so it should never arrive switched on.
    ///
    /// Only the project's owner or admin can change it, enforced server-side
    /// in `ws_web`; the CLI treats whatever reaches it as authoritative and
    /// refuses `StartTeam` while this is false.
    #[serde(default)]
    pub team_enabled: bool,

    /// Tab types this project refuses to create, as `<kind>:<provider>` keys
    /// (`shared::tab_type_key`). Empty means everything is allowed.
    ///
    /// A deny list rather than an allow list so `serde(default)` on an older
    /// `.apas` means "no restrictions" instead of "no tabs at all". Only the
    /// project's owner or admin can change it, enforced server-side; the CLI
    /// refuses `AddPane` for a disallowed type regardless of what the web
    /// offered.
    #[serde(default)]
    pub disallowed_tab_types: Vec<String>,

    // Legacy fields for backward compatibility (read-only migration)
    /// Claude session ID for the deadloop pane (legacy - use panes instead)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadloop_claude_session_id: Option<Uuid>,
    /// Claude session ID for the interactive pane (legacy - use panes instead)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interactive_claude_session_id: Option<Uuid>,
    /// Whether the deadloop is paused (legacy - use panes[].is_paused instead)
    #[serde(default)]
    pub is_paused: bool,
}

impl ProjectMetadata {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            prompt: None,
            // A new project starts empty. The user picks what to open;
            // materialising a Claude pane nobody asked for meant every
            // fresh project immediately spawned an agent process.
            panes: Vec::new(),
            auto_approve_todos: false,
            auto_merge_prs: false,
            team_enabled: false,
            disallowed_tab_types: Vec::new(),
            deadloop_claude_session_id: None,
            interactive_claude_session_id: None,
            is_paused: false,
        }
    }

    pub fn with_name(name: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: Some(name),
            created_at: chrono::Utc::now().to_rfc3339(),
            prompt: None,
            // A new project starts empty. The user picks what to open;
            // materialising a Claude pane nobody asked for meant every
            // fresh project immediately spawned an agent process.
            panes: Vec::new(),
            auto_approve_todos: false,
            auto_merge_prs: false,
            team_enabled: false,
            disallowed_tab_types: Vec::new(),
            deadloop_claude_session_id: None,
            interactive_claude_session_id: None,
            is_paused: false,
        }
    }

    /// Construct zero-pane metadata with an existing or reserved identity.
    pub fn with_id_and_name(id: Uuid, name: Option<String>) -> Self {
        let mut metadata = Self::new();
        metadata.id = id;
        metadata.name = name;
        metadata
    }

    /// Migrate legacy fields to panes list if needed
    pub fn migrate_legacy(&mut self) {
        // Only migrate when there is legacy state to migrate. This used to fire
        // on any empty pane list, and it runs on *every* load via
        // `get_or_create_project` — so a project deliberately holding no panes
        // had two resurrected under it on the next read. The legacy session-id
        // fields are what distinguish a pre-`panes` file from a new project.
        let has_legacy_state = self.deadloop_claude_session_id.is_some()
            || self.interactive_claude_session_id.is_some();
        if self.panes.is_empty() && has_legacy_state {
            // Migrate from legacy fields
            let deadloop_session = self.deadloop_claude_session_id.unwrap_or_else(Uuid::new_v4);
            let interactive_session = self
                .interactive_claude_session_id
                .unwrap_or_else(Uuid::new_v4);

            self.panes = vec![
                PaneConfig {
                    pane_id: shared::PANE_ID_DEADLOOP,
                    provider: shared::Provider::Claude,
                    mode: shared::PaneMode::Deadloop,
                    kind: shared::PaneKind::Agent,
                    session_id: deadloop_session,
                    is_paused: self.is_paused,
                    stop_requested: false,
                    prompt: self.prompt.clone(),
                    min_iteration_interval_minutes: Some(15),
                    label: Some("Claude Deadloop".to_string()),
                    model: None,
                    effort: None,
                    worktree_path: None,
                    role: None,
                    goal: None,
                    backstory: None,
                    plan_review_mode: shared::PlanReviewMode::default(),
                    manual_mode: false,
                    managed: false,
                },
                PaneConfig {
                    pane_id: shared::PANE_ID_INTERACTIVE,
                    provider: shared::Provider::Claude,
                    mode: shared::PaneMode::Interactive,
                    kind: shared::PaneKind::Agent,
                    session_id: interactive_session,
                    is_paused: false,
                    stop_requested: false,
                    prompt: None,
                    min_iteration_interval_minutes: None,
                    label: Some("Claude Interactive".to_string()),
                    model: None,
                    effort: None,
                    worktree_path: None,
                    role: None,
                    goal: None,
                    backstory: None,
                    plan_review_mode: shared::PlanReviewMode::default(),
                    manual_mode: false,
                    managed: false,
                },
            ];
        }
    }

    /// Get or create the deadloop Claude session ID (legacy compat)
    pub fn get_or_create_deadloop_session_id(&mut self) -> Uuid {
        self.migrate_legacy();
        // Find the first deadloop pane
        if let Some(pane) = self
            .panes
            .iter()
            .find(|p| p.pane_id == shared::PANE_ID_DEADLOOP)
        {
            return pane.session_id;
        }
        // Fallback to legacy field
        if let Some(id) = self.deadloop_claude_session_id {
            id
        } else {
            let id = Uuid::new_v4();
            self.deadloop_claude_session_id = Some(id);
            id
        }
    }

    /// Get or create the interactive Claude session ID (legacy compat)
    pub fn get_or_create_interactive_session_id(&mut self) -> Uuid {
        self.migrate_legacy();
        // Find the first interactive pane
        if let Some(pane) = self
            .panes
            .iter()
            .find(|p| p.pane_id == shared::PANE_ID_INTERACTIVE)
        {
            return pane.session_id;
        }
        // Fallback to legacy field
        if let Some(id) = self.interactive_claude_session_id {
            id
        } else {
            let id = Uuid::new_v4();
            self.interactive_claude_session_id = Some(id);
            id
        }
    }

    /// Get a pane by ID
    pub fn get_pane(&self, pane_id: u32) -> Option<&PaneConfig> {
        self.panes.iter().find(|p| p.pane_id == pane_id)
    }

    /// Get a mutable pane by ID
    pub fn get_pane_mut(&mut self, pane_id: u32) -> Option<&mut PaneConfig> {
        self.panes.iter_mut().find(|p| p.pane_id == pane_id)
    }
}

pub fn project_registry_path() -> Result<PathBuf> {
    let path = crate::config::Config::config_dir()?.join(USER_PROJECTS_FILE);
    with_project_registry_lock(&path, true, || {
        maybe_migrate_legacy_project_registry(&path);
        Ok(())
    })?;
    Ok(path)
}

pub fn list_registered_projects() -> Result<Vec<RegisteredProject>> {
    let path = project_registry_path()?;
    with_project_registry_lock(&path, false, || {
        let registry = read_existing_project_registry(&path)?;
        Ok(registry.projects)
    })
}

pub fn register_project(dir: &Path, metadata: &ProjectMetadata) -> Result<()> {
    let path = project_registry_path()?;
    register_project_at_registry_path(&path, dir, metadata)
}

fn register_project_at_registry_path(
    path: &Path,
    dir: &Path,
    metadata: &ProjectMetadata,
) -> Result<()> {
    with_project_registry_lock(&path, true, || {
        // Never turn a transient/partial NFS read into a one-project registry.
        // The caller can keep running and retry; destroying the other entries
        // makes the next daemon restart unable to find or resume them.
        let mut registry = read_existing_project_registry(&path)?;
        let normalized_dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        let dir_str = normalized_dir.to_string_lossy().to_string();
        let project_id = metadata.id.to_string();

        // De-duplicate by project id and by path.
        registry
            .projects
            .retain(|entry| entry.project_id != project_id && entry.path != dir_str);

        registry.projects.push(RegisteredProject {
            project_id,
            name: metadata.name.clone(),
            path: dir_str,
        });
        registry.projects.sort_by(|a, b| a.path.cmp(&b.path));

        write_project_registry(&path, &registry)
    })
}

/// Register an existing directory without migrating metadata or starting a runtime.
/// Return the committed inventory from the same lock transaction as the registration.
pub fn register_local_project(input: &str) -> Result<(RegisteredProject, Vec<RegisteredProject>)> {
    let dir = resolve_local_project_path(input)?;
    // Unlike ordinary CLI discovery, adoption must not perform best-effort
    // legacy migration or interpret an unreadable registry as an empty one.
    let registry_path = crate::config::Config::config_dir()?.join(USER_PROJECTS_FILE);
    register_local_project_at_registry_path(&registry_path, &dir)
}

fn resolve_local_project_path(input: &str) -> Result<PathBuf> {
    let path = if let Some(relative) = input.strip_prefix("~/") {
        dirs::home_dir()
            .context("Could not determine daemon user's home directory")?
            .join(relative)
    } else {
        let path = PathBuf::from(input);
        anyhow::ensure!(
            path.is_absolute(),
            "Folder path must be absolute or begin with ~/"
        );
        path
    };
    let canonical = std::fs::canonicalize(&path)
        .with_context(|| format!("Resolve existing folder {}", path.display()))?;
    anyhow::ensure!(
        canonical.is_dir(),
        "Not a directory: {}",
        canonical.display()
    );
    anyhow::ensure!(
        canonical.to_str().is_some(),
        "Folder path is not valid UTF-8"
    );
    Ok(canonical)
}

/// Missing is distinct from inaccessible, malformed, or unsafe. O_NOFOLLOW
/// also closes the check/open symlink race; O_NONBLOCK prevents a raced FIFO
/// from hanging the daemon before we can check the opened file's type.
fn read_registration_file(path: &Path) -> Result<Option<Vec<u8>>> {
    let stat = match std::fs::symlink_metadata(path) {
        Ok(stat) => stat,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("Inspect {}", path.display())),
    };
    anyhow::ensure!(
        stat.is_file(),
        "Not a regular metadata file: {}",
        path.display()
    );
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("Open {}", path.display()))?;
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

fn read_local_registration_registry(path: &Path) -> Result<ProjectRegistry> {
    let (source, bytes) = match read_registration_file(path)? {
        Some(bytes) => (path.to_path_buf(), bytes),
        None => {
            let legacy = legacy_project_registry_path()?;
            match read_registration_file(&legacy)? {
                Some(bytes) => (legacy, bytes),
                None => return Ok(ProjectRegistry::default()),
            }
        }
    };
    // Keep both existing registry formats, but reject truncated/empty files
    // and objects missing `projects` instead of overwriting them.
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RegistryFormat {
        Wrapped { projects: Vec<RegisteredProject> },
        Array(Vec<RegisteredProject>),
    }
    let parsed: RegistryFormat = serde_json::from_slice(&bytes)
        .with_context(|| format!("Parse project registry {}", source.display()))?;
    let projects = match parsed {
        RegistryFormat::Wrapped { projects } | RegistryFormat::Array(projects) => projects,
    };
    Ok(ProjectRegistry { projects })
}

fn check_local_registration_conflicts(
    registry: &ProjectRegistry,
    dir: &Path,
    id: Uuid,
) -> Result<()> {
    for entry in &registry.projects {
        let same_path = normalize_project_path(Path::new(&entry.path)) == dir;
        let same_id = Uuid::parse_str(&entry.project_id).ok() == Some(id);
        anyhow::ensure!(
            !same_path || same_id,
            "Folder {} is already registered with a different project identity ({})",
            dir.display(),
            entry.project_id
        );
        anyhow::ensure!(
            !same_id || same_path,
            "Project {} is already registered at a different folder ({})",
            id,
            entry.path
        );
    }
    Ok(())
}

fn register_local_project_at_registry_path(
    registry_path: &Path,
    dir: &Path,
) -> Result<(RegisteredProject, Vec<RegisteredProject>)> {
    with_project_registry_lock(registry_path, true, || {
        let mut registry = read_local_registration_registry(registry_path)?;
        let metadata_path = dir.join(APAS_FILE);
        let parse_metadata = |bytes: &[u8]| -> Result<ProjectMetadata> {
            serde_json::from_slice(bytes)
                .with_context(|| format!("Parse project metadata {}", metadata_path.display()))
        };
        let metadata = match read_registration_file(&metadata_path)? {
            Some(bytes) => parse_metadata(&bytes)?,
            None => {
                let metadata = match find_registered_project_by_path_in_registry(&registry, dir) {
                    Some(existing) => ProjectMetadata::with_id_and_name(
                        Uuid::parse_str(&existing.project_id)
                            .context("Registered folder has an invalid project identity")?,
                        existing.name,
                    ),
                    None => {
                        let mut metadata = ProjectMetadata::new();
                        metadata.name = dir
                            .file_name()
                            .and_then(|name| name.to_str())
                            .map(String::from);
                        metadata
                    }
                };
                check_local_registration_conflicts(&registry, dir, metadata.id)?;
                // Publish a fully written file without replacing anything,
                // including a dangling symlink or another creator's identity.
                let mut staged = tempfile::NamedTempFile::new_in(dir)
                    .context("Create project metadata staging file")?;
                serde_json::to_writer_pretty(&mut staged, &metadata)?;
                staged.as_file().sync_all()?;
                match staged.persist_noclobber(&metadata_path) {
                    Ok(_) => {
                        std::fs::File::open(dir)?.sync_all()?;
                        metadata
                    }
                    Err(err) if err.error.kind() == std::io::ErrorKind::AlreadyExists => {
                        let bytes = read_registration_file(&metadata_path)?
                            .context("Project metadata disappeared during registration; retry")?;
                        parse_metadata(&bytes)?
                    }
                    Err(err) => {
                        return Err(err.error).context("Publish project metadata");
                    }
                }
            }
        };
        check_local_registration_conflicts(&registry, dir, metadata.id)?;
        let project = RegisteredProject {
            project_id: metadata.id.to_string(),
            name: metadata.name,
            path: dir
                .to_str()
                .context("Folder path is not valid UTF-8")?
                .to_string(),
        };
        registry
            .projects
            .retain(|entry| normalize_project_path(Path::new(&entry.path)) != dir);
        registry.projects.push(project.clone());
        registry.projects.sort_by(|a, b| a.path.cmp(&b.path));
        write_project_registry(registry_path, &registry)
            .with_context(|| format!("Persist project registry {}", registry_path.display()))?;
        Ok((project, registry.projects))
    })
}

/// Remove exactly one project/path pair from the local registry. This is used
/// by marker-bound provisioning cleanup before the checkout is deleted.
pub fn unregister_project(dir: &Path, project_id: Uuid) -> Result<bool> {
    let path = project_registry_path()?;
    with_project_registry_lock(&path, true, || {
        let mut registry = read_existing_project_registry(&path)?;
        let normalized_dir = normalize_project_path(dir);
        let before = registry.projects.len();
        registry.projects.retain(|entry| {
            !(entry.project_id == project_id.to_string()
                && normalize_project_path(Path::new(&entry.path)) == normalized_dir)
        });
        if registry.projects.len() == before {
            return Ok(false);
        }
        write_project_registry(&path, &registry)?;
        Ok(true)
    })
}

fn project_registry_lock_path(path: &Path) -> PathBuf {
    path.with_extension("json.lock")
}

/// Run one registry operation while holding an advisory lock on shared NFS.
///
/// Linux implements `flock` on NFS using whole-file POSIX locks, so every APAS
/// process on every host participates in the same critical section. The lock
/// file is permanent; replacing `projects.json` by rename therefore cannot
/// accidentally move the inode that carries the lock out from under a waiter.
fn with_project_registry_lock<T>(
    path: &Path,
    exclusive: bool,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock_path = project_registry_lock_path(path);
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    if exclusive {
        FileExt::lock_exclusive(&lock_file)?;
    } else {
        FileExt::lock_shared(&lock_file)?;
    }

    let result = operation();
    let unlock_result = FileExt::unlock(&lock_file);
    match (result, unlock_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(err), _) => Err(err),
        (Ok(_), Err(err)) => Err(err.into()),
    }
}

fn read_project_registry(path: &Path) -> Result<ProjectRegistry> {
    if !path.exists() {
        return Ok(ProjectRegistry::default());
    }

    let content = std::fs::read_to_string(path)?;
    if content.trim().is_empty() {
        return Ok(ProjectRegistry::default());
    }

    // Backward/format compatibility: support both wrapped object and plain array.
    if let Ok(registry) = serde_json::from_str::<ProjectRegistry>(&content) {
        return Ok(registry);
    }
    if let Ok(projects) = serde_json::from_str::<Vec<RegisteredProject>>(&content) {
        return Ok(ProjectRegistry { projects });
    }

    anyhow::bail!("Failed to parse project registry at {:?}", path)
}

fn read_existing_project_registry(preferred_path: &Path) -> Result<ProjectRegistry> {
    if preferred_path.exists() {
        return read_project_registry(preferred_path);
    }

    if let Ok(legacy_path) = legacy_project_registry_path() {
        if legacy_path.exists() {
            return read_project_registry(&legacy_path);
        }
    }

    Ok(ProjectRegistry::default())
}

fn legacy_project_registry_path() -> Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    Ok(home.join(LEGACY_USER_REGISTRY_DIR).join(USER_PROJECTS_FILE))
}

fn maybe_migrate_legacy_project_registry(preferred_path: &Path) {
    if preferred_path.exists() {
        return;
    }

    let legacy_path = match legacy_project_registry_path() {
        Ok(path) => path,
        Err(err) => {
            tracing::debug!("Skipping legacy project registry migration: {}", err);
            return;
        }
    };

    if !legacy_path.exists() {
        return;
    }

    if let Some(parent) = preferred_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            tracing::warn!(
                "Failed to create project registry directory {:?}: {}",
                parent,
                err
            );
            return;
        }
    }

    match std::fs::rename(&legacy_path, preferred_path) {
        Ok(()) => {
            tracing::info!(
                "Migrated project registry from {:?} to {:?}",
                legacy_path,
                preferred_path
            );
        }
        Err(rename_err) => {
            tracing::warn!(
                "Failed to move legacy project registry from {:?} to {:?}: {}. Falling back to copy.",
                legacy_path,
                preferred_path,
                rename_err
            );
            match std::fs::copy(&legacy_path, preferred_path) {
                Ok(_) => {
                    if let Err(err) = std::fs::remove_file(&legacy_path) {
                        tracing::warn!(
                            "Copied legacy project registry to {:?}, but failed to remove {:?}: {}",
                            preferred_path,
                            legacy_path,
                            err
                        );
                    } else {
                        tracing::info!(
                            "Migrated project registry from {:?} to {:?}",
                            legacy_path,
                            preferred_path
                        );
                    }
                }
                Err(copy_err) => {
                    tracing::warn!(
                        "Failed to copy legacy project registry from {:?} to {:?}: {}",
                        legacy_path,
                        preferred_path,
                        copy_err
                    );
                }
            }
        }
    }
}

fn write_project_registry(path: &Path, registry: &ProjectRegistry) -> Result<()> {
    let content = serde_json::to_string_pretty(registry)?;
    // A UUID remains unique across hosts. A PID alone is host-local, so two
    // NFS clients can otherwise stage to the same path and one rename consumes
    // the other's file (the ENOENT loop seen during the 2026-08-30 incident).
    let tmp_path = project_registry_tmp_path(path);
    let result = (|| -> Result<()> {
        let mut tmp = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp_path)?;
        tmp.write_all(content.as_bytes())?;
        tmp.sync_all()?;
        std::fs::rename(&tmp_path, path)?;
        if let Some(parent) = path.parent() {
            std::fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

fn project_registry_tmp_path(path: &Path) -> PathBuf {
    path.with_extension(format!("json.{}.tmp", Uuid::new_v4()))
}

fn normalize_project_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn find_registered_project_by_path_in_registry(
    registry: &ProjectRegistry,
    dir: &Path,
) -> Option<RegisteredProject> {
    let normalized_dir = normalize_project_path(dir);
    registry.projects.iter().find_map(|entry| {
        let entry_path = PathBuf::from(&entry.path);
        if normalize_project_path(&entry_path) == normalized_dir {
            Some(entry.clone())
        } else {
            None
        }
    })
}

fn find_registered_project_by_path(dir: &Path) -> Option<RegisteredProject> {
    let path = project_registry_path().ok()?;
    with_project_registry_lock(&path, false, || {
        let registry = read_existing_project_registry(&path)?;
        Ok(find_registered_project_by_path_in_registry(&registry, dir))
    })
    .ok()?
}

/// Get or create the .apas metadata file for a directory
pub fn get_or_create_project(dir: &Path) -> Result<ProjectMetadata> {
    let apas_path = dir.join(APAS_FILE);

    if apas_path.exists() {
        // Read existing metadata
        let content = std::fs::read_to_string(&apas_path)?;
        let mut metadata: ProjectMetadata = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(err) => {
                tracing::warn!("Corrupt .apas file {:?}: {}. Regenerating.", apas_path, err);
                // Fall through to recreate — remove the corrupt file and regenerate
                let _ = std::fs::remove_file(&apas_path);
                return get_or_create_project(dir);
            }
        };
        // Migrate legacy pane config if needed
        metadata.migrate_legacy();
        if let Err(err) = register_project(dir, &metadata) {
            tracing::warn!("Failed to register project in user registry: {}", err);
        }
        Ok(metadata)
    } else {
        // Create new metadata with directory name as project name.
        // If this project was previously registered, preserve its session ID.
        let default_name = dir.file_name().and_then(|n| n.to_str()).map(String::from);
        let mut recovered_name: Option<String> = None;
        let id = if let Some(project) = find_registered_project_by_path(dir) {
            recovered_name = project.name;
            match Uuid::parse_str(&project.project_id) {
                Ok(existing_id) => {
                    tracing::warn!(
                        "Project metadata {:?} is missing; recovering existing project id {} from registry",
                        apas_path,
                        existing_id
                    );
                    existing_id
                }
                Err(err) => {
                    let new_id = Uuid::new_v4();
                    tracing::warn!(
                        "Project metadata {:?} is missing; registry project id {:?} is invalid ({}), generating new id {}",
                        apas_path,
                        project.project_id,
                        err,
                        new_id
                    );
                    new_id
                }
            }
        } else {
            Uuid::new_v4()
        };
        let name = recovered_name.or(default_name);

        let metadata = ProjectMetadata {
            id,
            name,
            created_at: chrono::Utc::now().to_rfc3339(),
            prompt: None,
            // A new project starts empty. The user picks what to open;
            // materialising a Claude pane nobody asked for meant every
            // fresh project immediately spawned an agent process.
            panes: Vec::new(),
            auto_approve_todos: false,
            auto_merge_prs: false,
            team_enabled: false,
            disallowed_tab_types: Vec::new(),
            deadloop_claude_session_id: None,
            interactive_claude_session_id: None,
            is_paused: false,
        };

        // Save to file
        let content = serde_json::to_string_pretty(&metadata)?;
        std::fs::write(&apas_path, content)?;
        if let Err(err) = register_project(dir, &metadata) {
            tracing::warn!("Failed to register project in user registry: {}", err);
        }

        tracing::info!("Created new project: {} ({:?})", metadata.id, metadata.name);
        Ok(metadata)
    }
}

/// Save project metadata back to the .apas file
pub fn save_project(dir: &Path, metadata: &ProjectMetadata) -> Result<()> {
    let apas_path = dir.join(APAS_FILE);
    let content = serde_json::to_string_pretty(metadata)?;
    let tmp_path = project_metadata_tmp_path(&apas_path);
    std::fs::write(&tmp_path, &content)?;
    std::fs::rename(&tmp_path, &apas_path)?;
    if let Err(err) = register_project(dir, metadata) {
        tracing::warn!("Failed to register project in user registry: {}", err);
    }
    tracing::debug!("Saved project metadata to {:?}", apas_path);
    Ok(())
}

fn project_metadata_tmp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(APAS_FILE);
    let counter = APAS_METADATA_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(
        "{}.{}.{}.tmp",
        file_name,
        std::process::id(),
        counter
    ))
}

/// Get the .apas file path for a directory
/// The project id recorded in `.apas`, without creating one.
///
/// `get_or_create_project` writes a `.apas` when there is none, which is right
/// when launching a project and wrong when merely asking whether one is
/// already running here.
pub fn read_project_id(dir: &Path) -> Option<uuid::Uuid> {
    let raw = std::fs::read_to_string(get_apas_path(dir)).ok()?;
    let metadata: ProjectMetadata = serde_json::from_str(&raw).ok()?;
    Some(metadata.id)
}

pub fn get_apas_path(dir: &Path) -> PathBuf {
    dir.join(APAS_FILE)
}

/// Check if a directory has been initialized as an apas project
pub fn is_project(dir: &Path) -> bool {
    dir.join(APAS_FILE).exists()
}

/// Environment isolation shared by every test that touches the project
/// registry. It lives outside `mod tests` because `main.rs` needs it too, and
/// a second copy of the lock would not serialise against this one — which is
/// the whole point, since these tests mutate `HOME` process-wide to exercise
/// legacy-registry migration.
#[cfg(test)]
pub(crate) mod test_support {

    pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Take the environment lock, tolerating a previous holder's panic.
    ///
    /// A failing test unwinds through the guard and poisons the mutex, so one
    /// genuine assertion failure turned into eight unrelated ones and buried
    /// the real defect. Poisoning protects invariants inside the data; there is
    /// no data here, only the serialisation.
    pub(crate) fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn restore_env_var(key: &str, value: Option<std::ffi::OsString>) {
        if let Some(value) = value {
            std::env::set_var(key, value);
        } else {
            std::env::remove_var(key);
        }
    }

    pub(crate) fn with_isolated_config<T>(test: impl FnOnce() -> T) -> T {
        let guard = env_lock();
        let config = crate::config::test_config::isolated_config_dir();
        let home = tempfile::tempdir().expect("temp home");
        let old_home = std::env::var_os("HOME");

        std::env::set_var("HOME", home.path());

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(test));

        restore_env_var("HOME", old_home);
        drop(config);

        // Release before resuming: unwinding through the guard is what poisoned
        // the lock for every test that came after.
        drop(guard);

        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::with_isolated_config;

    #[test]
    fn a_new_project_starts_with_no_panes() {
        // Materialising a Claude pane meant every fresh project immediately
        // spawned an agent process nobody asked for.
        assert!(ProjectMetadata::new().panes.is_empty());
        assert!(ProjectMetadata::with_name("x".into()).panes.is_empty());
    }

    #[test]
    fn an_empty_pane_list_survives_a_reload() {
        // The trap: `get_or_create_project` calls `migrate_legacy` on every
        // load, and that used to refill any empty pane list — with *two*
        // legacy panes — so "no panes" was not representable at all.
        let _config = crate::config::test_config::isolated_config_dir();
        let dir = tempfile::tempdir().expect("temp project dir");

        let first = get_or_create_project(dir.path()).expect("create");
        assert!(first.panes.is_empty(), "created with no panes");

        let second = get_or_create_project(dir.path()).expect("reload");
        assert!(second.panes.is_empty(), "still no panes after a reload");
        assert_eq!(second.id, first.id, "same project, not recreated");
    }

    #[test]
    fn a_pre_panes_apas_is_still_migrated() {
        // The migration must keep working for files that predate `panes` and
        // carry the legacy session-id fields — that is the case it exists for.
        let _config = crate::config::test_config::isolated_config_dir();
        let dir = tempfile::tempdir().expect("temp project dir");
        std::fs::write(
            dir.path().join(".apas"),
            r#"{
                "id": "8c4b0c1e-0000-4000-8000-0000000000aa",
                "name": "legacy",
                "created_at": "2026-01-01T00:00:00Z",
                "deadloop_claude_session_id": "8c4b0c1e-0000-4000-8000-0000000000bb",
                "interactive_claude_session_id": "8c4b0c1e-0000-4000-8000-0000000000cc"
            }"#,
        )
        .expect("write legacy .apas");

        let meta = get_or_create_project(dir.path()).expect("load legacy");
        assert_eq!(
            meta.panes.len(),
            2,
            "legacy deadloop + interactive restored"
        );
        assert!(meta
            .panes
            .iter()
            .any(|p| p.pane_id == shared::PANE_ID_DEADLOOP));
        assert!(meta
            .panes
            .iter()
            .any(|p| p.pane_id == shared::PANE_ID_INTERACTIVE));
    }

    #[test]
    fn migrate_legacy_leaves_a_modern_empty_project_alone() {
        // Same check at the unit level: no legacy fields means nothing to
        // migrate, regardless of the pane list being empty.
        let mut meta = ProjectMetadata::new();
        assert!(meta.panes.is_empty());
        meta.migrate_legacy();
        assert!(meta.panes.is_empty(), "nothing to migrate, nothing created");
    }
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "apas-project-{}-{}-{}",
            label,
            std::process::id(),
            stamp
        ))
    }

    fn metadata_with_id(id: Uuid, name: &str) -> ProjectMetadata {
        let mut metadata = ProjectMetadata::with_name(name.to_string());
        metadata.id = id;
        metadata
    }

    fn sample_registered_project(name: &str) -> RegisteredProject {
        RegisteredProject {
            project_id: Uuid::new_v4().to_string(),
            name: Some(name.to_string()),
            path: format!("/tmp/apas-{name}"),
        }
    }

    fn preferred_registry_path() -> PathBuf {
        crate::config::Config::config_dir()
            .expect("config dir")
            .join(USER_PROJECTS_FILE)
    }

    #[test]
    fn local_registration_preserves_plain_files_and_recovers_missing_metadata() {
        with_isolated_config(|| {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("notes.txt"), b"user contents\n").unwrap();
            let input = dir.path().to_str().unwrap();
            let (first, inventory) = register_local_project(input).unwrap();
            assert_eq!(inventory.len(), 1);
            assert_eq!(
                first.path,
                std::fs::canonicalize(dir.path()).unwrap().to_str().unwrap()
            );
            let metadata: ProjectMetadata =
                serde_json::from_slice(&std::fs::read(dir.path().join(APAS_FILE)).unwrap())
                    .unwrap();
            assert!(metadata.panes.is_empty());
            assert!(!dir.path().join(".git").exists());
            assert!(!dir.path().join(".gitignore").exists());
            assert_eq!(
                std::fs::read(dir.path().join("notes.txt")).unwrap(),
                b"user contents\n"
            );
            std::fs::remove_file(dir.path().join(APAS_FILE)).unwrap();
            let (recovered, inventory) = register_local_project(input).unwrap();
            assert_eq!(recovered.project_id, first.project_id);
            assert_eq!(recovered.name, first.name);
            assert_eq!(inventory.len(), 1);
        });
    }

    #[test]
    fn local_registration_resolves_home_paths_and_rejects_invalid_targets() {
        with_isolated_config(|| {
            let home = dirs::home_dir().unwrap();
            let dir = home.join("work/My Project $literal");
            std::fs::create_dir_all(&dir).unwrap();
            let (project, _) = register_local_project("~/work/My Project $literal").unwrap();
            assert_eq!(
                project.path,
                std::fs::canonicalize(&dir).unwrap().to_str().unwrap()
            );
            let file = home.join("file");
            std::fs::write(&file, b"retained").unwrap();
            for input in [
                "",
                " ",
                "relative/path",
                "~other/path",
                "~",
                file.to_str().unwrap(),
            ] {
                assert!(register_local_project(input).is_err(), "{input:?}");
            }
            let missing = home.join("missing");
            assert!(register_local_project(missing.to_str().unwrap()).is_err());
            assert!(!missing.exists());
            assert_eq!(std::fs::read(file).unwrap(), b"retained");
        });
    }

    #[test]
    fn local_registration_preserves_existing_metadata_bytes_and_legacy_configuration() {
        with_isolated_config(|| {
            let dir = tempfile::tempdir().unwrap();
            let mut metadata = ProjectMetadata::with_name("Retained name".into());
            metadata.prompt = Some("saved prompt".into());
            metadata.team_enabled = true;
            metadata.auto_approve_todos = true;
            metadata.disallowed_tab_types = vec!["agent:claude".into()];
            metadata.deadloop_claude_session_id = Some(Uuid::new_v4());
            metadata.migrate_legacy();
            let mut value = serde_json::to_value(&metadata).unwrap();
            value["future_configuration"] = serde_json::json!({"keep": [1, 2, 3]});
            let bytes =
                format!(" \n{}\n", serde_json::to_string_pretty(&value).unwrap()).into_bytes();
            std::fs::write(dir.path().join(APAS_FILE), &bytes).unwrap();
            for _ in 0..2 {
                let (registered, _) = register_local_project(dir.path().to_str().unwrap()).unwrap();
                assert_eq!(registered.project_id, metadata.id.to_string());
                assert_eq!(registered.name, metadata.name);
                assert_eq!(std::fs::read(dir.path().join(APAS_FILE)).unwrap(), bytes);
            }
            // A legacy-only file must not be migrated/reformatted on adoption either.
            value["panes"] = serde_json::json!([]);
            let legacy = serde_json::to_vec(&value).unwrap();
            std::fs::write(dir.path().join(APAS_FILE), &legacy).unwrap();
            register_local_project(dir.path().to_str().unwrap()).unwrap();
            assert_eq!(std::fs::read(dir.path().join(APAS_FILE)).unwrap(), legacy);
        });
    }

    #[test]
    fn local_registration_rejects_corrupt_metadata_and_registry_without_replacing_them() {
        with_isolated_config(|| {
            let dir = tempfile::tempdir().unwrap();
            let metadata_path = dir.path().join(APAS_FILE);
            std::fs::write(&metadata_path, b"{broken metadata").unwrap();
            assert!(register_local_project(dir.path().to_str().unwrap()).is_err());
            assert_eq!(std::fs::read(&metadata_path).unwrap(), b"{broken metadata");
            std::fs::remove_file(&metadata_path).unwrap();
            let registry_path = preferred_registry_path();
            for bytes in [b"{broken registry".as_slice(), b"", b"{}", b"null"] {
                std::fs::write(&registry_path, bytes).unwrap();
                assert!(register_local_project(dir.path().to_str().unwrap()).is_err());
                assert_eq!(std::fs::read(&registry_path).unwrap(), bytes);
                assert!(!metadata_path.exists());
            }
        });
    }

    #[cfg(unix)]
    #[test]
    fn local_registration_allows_directory_aliases_but_rejects_unsafe_metadata() {
        use std::os::unix::fs::symlink;
        with_isolated_config(|| {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path().join("project");
            let alias = root.path().join("alias");
            std::fs::create_dir(&dir).unwrap();
            symlink(&dir, &alias).unwrap();
            let (first, _) = register_local_project(alias.to_str().unwrap()).unwrap();
            let (second, inventory) = register_local_project(dir.to_str().unwrap()).unwrap();
            assert_eq!(first.project_id, second.project_id);
            assert_eq!(inventory.len(), 1);
            let metadata = dir.join(APAS_FILE);
            let target = root.path().join("retained");
            std::fs::rename(&metadata, &target).unwrap();
            let original = std::fs::read(&target).unwrap();
            symlink(&target, &metadata).unwrap();
            assert!(register_local_project(dir.to_str().unwrap()).is_err());
            assert_eq!(std::fs::read(&target).unwrap(), original);
            std::fs::remove_file(&metadata).unwrap();
            let missing = root.path().join("absent");
            symlink(&missing, &metadata).unwrap();
            assert!(register_local_project(dir.to_str().unwrap()).is_err());
            assert!(!missing.exists());
            std::fs::remove_file(&metadata).unwrap();
            std::fs::create_dir(&metadata).unwrap();
            assert!(register_local_project(dir.to_str().unwrap()).is_err());
            assert!(metadata.is_dir());
            std::fs::remove_dir(&metadata).unwrap();
            let fifo = std::ffi::CString::new(metadata.to_str().unwrap()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
            assert!(register_local_project(dir.to_str().unwrap()).is_err());
        });
    }

    #[test]
    fn concurrent_local_registration_creates_one_identity() {
        with_isolated_config(|| {
            let dir = tempfile::tempdir().unwrap();
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(12));
            let workers: Vec<_> = (0..12)
                .map(|_| {
                    let path = dir.path().to_str().unwrap().to_string();
                    let barrier = barrier.clone();
                    let config_dir = crate::config::Config::config_dir().unwrap();
                    std::thread::spawn(move || {
                        barrier.wait();
                        crate::config::test_config::with_config_dir(&config_dir, || {
                            register_local_project(&path).unwrap().0.project_id
                        })
                    })
                })
                .collect();
            let ids: Vec<_> = workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect();
            assert!(ids.iter().all(|id| id == &ids[0]));
            let registry = list_registered_projects().unwrap();
            assert_eq!(registry.len(), 1);
            assert_eq!(registry[0].project_id, ids[0]);
            assert_eq!(read_project_id(dir.path()).unwrap().to_string(), ids[0]);
        });
    }

    #[test]
    fn local_registration_rejects_unreadable_registry_and_invalid_recovered_identity() {
        with_isolated_config(|| {
            let dir = tempfile::tempdir().unwrap();
            let registry_path = preferred_registry_path();
            std::fs::create_dir(&registry_path).unwrap();
            assert!(register_local_project(dir.path().to_str().unwrap()).is_err());
            assert!(registry_path.is_dir());
            assert!(!dir.path().join(APAS_FILE).exists());
            std::fs::remove_dir(&registry_path).unwrap();
            let bytes = serde_json::to_vec(&ProjectRegistry {
                projects: vec![RegisteredProject {
                    project_id: "invalid-identity".into(),
                    name: Some("Retain this entry".into()),
                    path: dir.path().to_str().unwrap().into(),
                }],
            })
            .unwrap();
            std::fs::write(&registry_path, &bytes).unwrap();
            assert!(register_local_project(dir.path().to_str().unwrap()).is_err());
            assert_eq!(std::fs::read(&registry_path).unwrap(), bytes);
            assert!(!dir.path().join(APAS_FILE).exists());
        });
    }

    #[test]
    fn local_registration_rejects_both_identity_conflicts_atomically() {
        with_isolated_config(|| {
            let first = tempfile::tempdir().unwrap();
            let second = tempfile::tempdir().unwrap();
            register_local_project(first.path().to_str().unwrap()).unwrap();
            let original_metadata = std::fs::read(first.path().join(APAS_FILE)).unwrap();
            let registry_path = preferred_registry_path();
            let original_registry = std::fs::read(&registry_path).unwrap();
            // Same identity at another directory must not relocate it.
            std::fs::write(second.path().join(APAS_FILE), &original_metadata).unwrap();
            assert!(register_local_project(second.path().to_str().unwrap()).is_err());
            assert_eq!(std::fs::read(&registry_path).unwrap(), original_registry);
            assert_eq!(
                std::fs::read(second.path().join(APAS_FILE)).unwrap(),
                original_metadata
            );
            // Same directory with another identity must not replace it.
            let different = serde_json::to_vec(&ProjectMetadata::new()).unwrap();
            std::fs::write(first.path().join(APAS_FILE), &different).unwrap();
            assert!(register_local_project(first.path().to_str().unwrap()).is_err());
            assert_eq!(std::fs::read(&registry_path).unwrap(), original_registry);
            assert_eq!(
                std::fs::read(first.path().join(APAS_FILE)).unwrap(),
                different
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn failed_registry_persistence_keeps_metadata_for_manual_retry() {
        with_isolated_config(|| {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path().join("project");
            std::fs::create_dir(&dir).unwrap();
            // The registry and its lock fit NAME_MAX, but the existing atomic
            // writer's UUID-suffixed staging filename does not. This induces
            // a real write failure after metadata publication, even as root.
            let registry_path = root.path().join(format!("{}.json", "r".repeat(230)));
            assert!(register_local_project_at_registry_path(&registry_path, &dir).is_err());
            let retained = std::fs::read(dir.join(APAS_FILE)).unwrap();
            let metadata: ProjectMetadata = serde_json::from_slice(&retained).unwrap();
            let (registered, _) = register_local_project(dir.to_str().unwrap()).unwrap();
            assert_eq!(registered.project_id, metadata.id.to_string());
            assert_eq!(std::fs::read(dir.join(APAS_FILE)).unwrap(), retained);
        });
    }

    fn git_output(dir: &Path, args: &[&str]) -> Vec<u8> {
        let output = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {:?}", output.stderr);
        output.stdout
    }

    #[test]
    fn local_registration_leaves_dirty_git_checkouts_and_all_origins_unchanged() {
        with_isolated_config(|| {
            for origin in [
                Some("https://github.com/example/repo.git"),
                Some("ssh://git.example.test/repo"),
                None,
            ] {
                let dir = tempfile::tempdir().unwrap();
                git_output(dir.path(), &["init", "-b", "retained-branch"]);
                std::fs::write(dir.path().join("tracked.txt"), b"committed\n").unwrap();
                std::fs::write(dir.path().join(".gitignore"), b"ignored-local\n").unwrap();
                git_output(dir.path(), &["add", "."]);
                git_output(
                    dir.path(),
                    &[
                        "-c",
                        "user.name=Test",
                        "-c",
                        "user.email=test@example.test",
                        "-c",
                        "commit.gpgsign=false",
                        "commit",
                        "-m",
                        "initial",
                    ],
                );
                if let Some(origin) = origin {
                    git_output(dir.path(), &["remote", "add", "origin", origin]);
                }
                std::fs::write(dir.path().join("tracked.txt"), b"dirty user change\n").unwrap();
                std::fs::write(dir.path().join("untracked.txt"), b"untracked\n").unwrap();
                let head = git_output(dir.path(), &["rev-parse", "HEAD"]);
                let branch = git_output(dir.path(), &["symbolic-ref", "HEAD"]);
                let config = std::fs::read(dir.path().join(".git/config")).unwrap();
                let index = std::fs::read(dir.path().join(".git/index")).unwrap();
                register_local_project(dir.path().to_str().unwrap()).unwrap();
                assert_eq!(git_output(dir.path(), &["rev-parse", "HEAD"]), head);
                assert_eq!(git_output(dir.path(), &["symbolic-ref", "HEAD"]), branch);
                assert_eq!(
                    std::fs::read(dir.path().join(".git/config")).unwrap(),
                    config
                );
                assert_eq!(std::fs::read(dir.path().join(".git/index")).unwrap(), index);
                assert_eq!(
                    std::fs::read(dir.path().join("tracked.txt")).unwrap(),
                    b"dirty user change\n"
                );
                assert_eq!(
                    std::fs::read(dir.path().join("untracked.txt")).unwrap(),
                    b"untracked\n"
                );
                assert_eq!(
                    std::fs::read(dir.path().join(".gitignore")).unwrap(),
                    b"ignored-local\n"
                );
            }
        });
    }

    fn write_legacy_registry(content: &str) -> PathBuf {
        let legacy_path = legacy_project_registry_path().expect("legacy registry path");
        std::fs::create_dir_all(
            legacy_path
                .parent()
                .expect("legacy registry should have parent"),
        )
        .expect("create legacy registry dir");
        std::fs::write(&legacy_path, content).expect("write legacy registry");
        legacy_path
    }

    #[test]
    fn project_registry_tmp_path_is_unique_across_hosts() {
        let path = unique_temp_dir("tmp-path").join(USER_PROJECTS_FILE);
        let first = project_registry_tmp_path(&path);
        let second = project_registry_tmp_path(&path);
        let first_name = first
            .file_name()
            .and_then(|name| name.to_str())
            .expect("utf-8 temp filename");

        assert!(first_name.starts_with("projects.json."));
        assert!(first_name.ends_with(".tmp"));
        assert_ne!(first, second, "a UUID, unlike a PID, is cross-host unique");
        assert_ne!(first, path.with_extension("json.tmp"));
    }

    #[test]
    fn write_project_registry_writes_wrapped_json_without_shared_tmp() {
        let dir = unique_temp_dir("write-registry");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join(USER_PROJECTS_FILE);
        let shared_tmp_path = path.with_extension("json.tmp");
        let expected_project = RegisteredProject {
            project_id: Uuid::new_v4().to_string(),
            name: Some("demo".to_string()),
            path: dir.join("demo").to_string_lossy().to_string(),
        };
        let registry = ProjectRegistry {
            projects: vec![expected_project.clone()],
        };

        write_project_registry(&path, &registry).expect("write registry");

        assert!(
            !shared_tmp_path.exists(),
            "shared projects.json.tmp should not remain"
        );
        let temporary_entries: Vec<_> = std::fs::read_dir(&dir)
            .expect("read registry dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with("projects.json.") && name.ends_with(".tmp"))
            .collect();
        assert!(
            temporary_entries.is_empty(),
            "unique staging files should be renamed away: {temporary_entries:?}"
        );

        let content = std::fs::read_to_string(&path).expect("read registry");
        let value: serde_json::Value = serde_json::from_str(&content).expect("valid json");
        assert!(value
            .get("projects")
            .and_then(|projects| projects.as_array())
            .is_some());

        let written: ProjectRegistry = serde_json::from_str(&content).expect("wrapped registry");
        assert_eq!(written.projects.len(), 1);
        assert_eq!(written.projects[0].project_id, expected_project.project_id);
        assert_eq!(written.projects[0].path, expected_project.path);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_existing_project_registry_falls_back_to_legacy_wrapped_json() {
        with_isolated_config(|| {
            let preferred_path = preferred_registry_path();
            let expected_project = sample_registered_project("legacy-wrapped");
            let content = serde_json::to_string(&ProjectRegistry {
                projects: vec![expected_project.clone()],
            })
            .expect("wrapped registry json");
            write_legacy_registry(&content);

            let registry =
                read_existing_project_registry(&preferred_path).expect("read legacy registry");

            assert!(!preferred_path.exists());
            assert_eq!(registry.projects.len(), 1);
            assert_eq!(registry.projects[0].project_id, expected_project.project_id);
            assert_eq!(registry.projects[0].name, expected_project.name);
            assert_eq!(registry.projects[0].path, expected_project.path);
        });
    }

    #[test]
    fn read_existing_project_registry_falls_back_to_legacy_plain_array_json() {
        with_isolated_config(|| {
            let preferred_path = preferred_registry_path();
            let expected_project = sample_registered_project("legacy-array");
            let content = serde_json::to_string(&vec![expected_project.clone()])
                .expect("array registry json");
            write_legacy_registry(&content);

            let registry =
                read_existing_project_registry(&preferred_path).expect("read legacy registry");

            assert!(!preferred_path.exists());
            assert_eq!(registry.projects.len(), 1);
            assert_eq!(registry.projects[0].project_id, expected_project.project_id);
            assert_eq!(registry.projects[0].name, expected_project.name);
            assert_eq!(registry.projects[0].path, expected_project.path);
        });
    }

    #[test]
    fn maybe_migrate_legacy_project_registry_moves_content_to_preferred_path() {
        with_isolated_config(|| {
            let preferred_path = preferred_registry_path();
            let expected_project = sample_registered_project("migrated");
            let content = serde_json::to_string(&ProjectRegistry {
                projects: vec![expected_project.clone()],
            })
            .expect("wrapped registry json");
            let legacy_path = write_legacy_registry(&content);

            maybe_migrate_legacy_project_registry(&preferred_path);

            assert!(preferred_path.exists());
            assert!(!legacy_path.exists());
            let registry = read_project_registry(&preferred_path).expect("read migrated registry");
            assert_eq!(registry.projects.len(), 1);
            assert_eq!(registry.projects[0].project_id, expected_project.project_id);
            assert_eq!(registry.projects[0].name, expected_project.name);
            assert_eq!(registry.projects[0].path, expected_project.path);
        });
    }

    #[test]
    fn maybe_migrate_legacy_project_registry_does_not_overwrite_preferred_registry() {
        with_isolated_config(|| {
            let preferred_path = preferred_registry_path();
            let preferred_project = sample_registered_project("preferred");
            let legacy_project = sample_registered_project("legacy");
            write_project_registry(
                &preferred_path,
                &ProjectRegistry {
                    projects: vec![preferred_project.clone()],
                },
            )
            .expect("write preferred registry");
            let legacy_content = serde_json::to_string(&ProjectRegistry {
                projects: vec![legacy_project],
            })
            .expect("legacy registry json");
            let legacy_path = write_legacy_registry(&legacy_content);

            maybe_migrate_legacy_project_registry(&preferred_path);

            assert!(legacy_path.exists());
            let registry = read_project_registry(&preferred_path).expect("read preferred registry");
            assert_eq!(registry.projects.len(), 1);
            assert_eq!(
                registry.projects[0].project_id,
                preferred_project.project_id
            );
            assert_eq!(registry.projects[0].name, preferred_project.name);
            assert_eq!(registry.projects[0].path, preferred_project.path);
        });
    }

    #[test]
    fn register_project_deduplicates_by_id_and_path_and_sorts_output() {
        with_isolated_config(|| {
            let projects_root = tempfile::tempdir().expect("temp projects root");
            let a_dir = projects_root.path().join("a-project");
            let m_dir = projects_root.path().join("m-project");
            let z_dir = projects_root.path().join("z-project");
            for dir in [&a_dir, &m_dir, &z_dir] {
                std::fs::create_dir_all(dir).expect("create project dir");
            }

            let duplicate_id = Uuid::new_v4();
            let old_path_id = Uuid::new_v4();
            let replacement_path_id = Uuid::new_v4();

            register_project(&z_dir, &metadata_with_id(duplicate_id, "old duplicate id"))
                .expect("register old duplicate id");
            register_project(&a_dir, &metadata_with_id(duplicate_id, "new duplicate id"))
                .expect("replace duplicate id");
            register_project(&m_dir, &metadata_with_id(old_path_id, "old path"))
                .expect("register old path");
            register_project(&m_dir, &metadata_with_id(replacement_path_id, "new path"))
                .expect("replace duplicate path");

            let registry_path = project_registry_path().expect("registry path");
            let registry = read_project_registry(&registry_path).expect("read registry");
            assert_eq!(registry.projects.len(), 2);

            let a_path = std::fs::canonicalize(&a_dir)
                .expect("canonical a")
                .to_string_lossy()
                .to_string();
            let m_path = std::fs::canonicalize(&m_dir)
                .expect("canonical m")
                .to_string_lossy()
                .to_string();
            let z_path = std::fs::canonicalize(&z_dir)
                .expect("canonical z")
                .to_string_lossy()
                .to_string();

            assert_eq!(
                registry
                    .projects
                    .iter()
                    .map(|project| project.path.as_str())
                    .collect::<Vec<_>>(),
                vec![a_path.as_str(), m_path.as_str()]
            );
            assert!(registry
                .projects
                .iter()
                .any(|project| project.project_id == duplicate_id.to_string()
                    && project.path == a_path
                    && project.name.as_deref() == Some("new duplicate id")));
            assert!(registry.projects.iter().any(|project| project.project_id
                == replacement_path_id.to_string()
                && project.path == m_path
                && project.name.as_deref() == Some("new path")));
            assert!(!registry
                .projects
                .iter()
                .any(|project| project.project_id == old_path_id.to_string()));
            assert!(!registry
                .projects
                .iter()
                .any(|project| project.path == z_path));
        });
    }

    #[test]
    fn register_project_never_replaces_an_unreadable_registry() {
        with_isolated_config(|| {
            let registry_path = preferred_registry_path();
            std::fs::create_dir_all(registry_path.parent().expect("registry parent"))
                .expect("create registry parent");
            let corrupt = b"{not valid json";
            std::fs::write(&registry_path, corrupt).expect("write corrupt registry");

            let project_dir = tempfile::tempdir().expect("temp project");
            let metadata = ProjectMetadata::with_name("must-not-replace".to_string());
            assert!(
                register_project(project_dir.path(), &metadata).is_err(),
                "a corrupt shared index must be preserved for recovery"
            );
            assert_eq!(
                std::fs::read(&registry_path).expect("read preserved registry"),
                corrupt
            );
        });
    }

    #[test]
    fn concurrent_registry_updates_keep_every_project() {
        with_isolated_config(|| {
            let projects_root = tempfile::tempdir().expect("temp projects root");
            let registry_path = preferred_registry_path();
            let mut workers = Vec::new();
            for index in 0..16 {
                let dir = projects_root.path().join(format!("project-{index:02}"));
                std::fs::create_dir_all(&dir).expect("create project dir");
                let metadata = ProjectMetadata::with_name(format!("project-{index:02}"));
                let registry_path = registry_path.clone();
                workers.push(std::thread::spawn(move || {
                    register_project_at_registry_path(&registry_path, &dir, &metadata)
                        .expect("register concurrently");
                }));
            }
            for worker in workers {
                worker.join().expect("registry worker");
            }

            let registered = list_registered_projects().expect("read registry");
            assert_eq!(registered.len(), 16, "no read/modify/write update was lost");
        });
    }

    #[test]
    fn project_metadata_tmp_path_is_unique_and_pid_scoped() {
        let dir = tempfile::tempdir().expect("temp project dir");
        let apas_path = dir.path().join(APAS_FILE);

        let first = project_metadata_tmp_path(&apas_path);
        let second = project_metadata_tmp_path(&apas_path);

        assert_eq!(first.parent(), Some(dir.path()));
        assert_ne!(first, apas_path.with_extension("apas.tmp"));
        assert_ne!(first, second);

        let tmp_name = first
            .file_name()
            .and_then(|name| name.to_str())
            .expect("utf-8 temp filename");
        assert!(tmp_name.starts_with(".apas."));
        assert!(tmp_name.contains(&format!(".{}.", std::process::id())));
        assert!(tmp_name.ends_with(".tmp"));
    }

    #[test]
    fn save_project_writes_valid_metadata_without_stale_shared_tmp() {
        with_isolated_config(|| {
            let dir = tempfile::tempdir().expect("temp project dir");
            let mut metadata = ProjectMetadata::with_name("demo".to_string());
            metadata.prompt = Some("first save".to_string());

            save_project(dir.path(), &metadata).expect("first save");
            metadata.prompt = Some("second save".to_string());
            save_project(dir.path(), &metadata).expect("second save");

            let apas_path = dir.path().join(APAS_FILE);
            let content = std::fs::read_to_string(&apas_path).expect("read .apas");
            let saved: ProjectMetadata =
                serde_json::from_str(&content).expect("valid .apas metadata JSON");

            assert_eq!(saved.id, metadata.id);
            assert_eq!(saved.name, metadata.name);
            assert_eq!(saved.prompt, Some("second save".to_string()));
            assert!(!dir.path().join(".apas.tmp").exists());

            let stale_tmp_entries: Vec<_> = std::fs::read_dir(dir.path())
                .expect("read project dir")
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .filter(|name| name.starts_with(".apas.") && name.ends_with(".tmp"))
                .collect();
            assert!(
                stale_tmp_entries.is_empty(),
                "stale metadata temp files remained: {stale_tmp_entries:?}"
            );
        });
    }

    #[test]
    fn finds_registered_project_by_path() {
        let dir = unique_temp_dir("match");
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let expected = RegisteredProject {
            project_id: Uuid::new_v4().to_string(),
            name: Some("demo".to_string()),
            path: dir.to_string_lossy().to_string(),
        };
        let registry = ProjectRegistry {
            projects: vec![expected.clone()],
        };

        let found = find_registered_project_by_path_in_registry(&registry, &dir);
        assert_eq!(
            found.expect("project should be found").project_id,
            expected.project_id
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn does_not_match_other_paths() {
        let existing_dir = unique_temp_dir("existing");
        let target_dir = unique_temp_dir("target");
        std::fs::create_dir_all(&existing_dir).expect("create temp dir");
        std::fs::create_dir_all(&target_dir).expect("create temp dir");

        let registry = ProjectRegistry {
            projects: vec![RegisteredProject {
                project_id: Uuid::new_v4().to_string(),
                name: Some("other".to_string()),
                path: existing_dir.to_string_lossy().to_string(),
            }],
        };

        assert!(find_registered_project_by_path_in_registry(&registry, &target_dir).is_none());

        let _ = std::fs::remove_dir_all(&existing_dir);
        let _ = std::fs::remove_dir_all(&target_dir);
    }
}
