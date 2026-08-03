use anyhow::Result;
use serde::{Deserialize, Serialize};
use shared::PaneConfig;
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
            panes: PaneConfig::defaults(),
            auto_approve_todos: false,
            auto_merge_prs: false,
            team_enabled: false,
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
            panes: PaneConfig::defaults(),
            auto_approve_todos: false,
            auto_merge_prs: false,
            team_enabled: false,
            deadloop_claude_session_id: None,
            interactive_claude_session_id: None,
            is_paused: false,
        }
    }

    /// Migrate legacy fields to panes list if needed
    pub fn migrate_legacy(&mut self) {
        if self.panes.is_empty() {
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
    maybe_migrate_legacy_project_registry(&path);
    Ok(path)
}

pub fn list_registered_projects() -> Result<Vec<RegisteredProject>> {
    let path = project_registry_path()?;
    let registry = read_existing_project_registry(&path)?;
    Ok(registry.projects)
}

pub fn register_project(dir: &Path, metadata: &ProjectMetadata) -> Result<()> {
    let path = project_registry_path()?;
    let mut registry = match read_existing_project_registry(&path) {
        Ok(registry) => registry,
        Err(err) => {
            tracing::warn!(
                "Project registry at {:?} is unreadable ({}), recreating it",
                path,
                err
            );
            ProjectRegistry::default()
        }
    };
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
    // PID in the tmp filename so concurrent apas processes (the daemon
    // + per-project CLIs + spawned --headless workers, which often boot
    // near-simultaneously) don't share a staging path. Previously they
    // all wrote `projects.json.tmp` and raced — the late renamer hit
    // ENOENT because an earlier process's rename had consumed the tmp.
    let tmp_path = project_registry_tmp_path(path);
    std::fs::write(&tmp_path, content)?;
    std::fs::rename(tmp_path, path)?;
    Ok(())
}

fn project_registry_tmp_path(path: &Path) -> PathBuf {
    path.with_extension(format!("json.{}.tmp", std::process::id()))
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
    let registry = read_existing_project_registry(&path).ok()?;
    find_registered_project_by_path_in_registry(&registry, dir)
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
                tracing::warn!(
                    "Corrupt .apas file {:?}: {}. Regenerating.",
                    apas_path,
                    err
                );
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
            panes: PaneConfig::defaults(),
            auto_approve_todos: false,
            auto_merge_prs: false,
            team_enabled: false,
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
pub fn get_apas_path(dir: &Path) -> PathBuf {
    dir.join(APAS_FILE)
}

/// Check if a directory has been initialized as an apas project
pub fn is_project(dir: &Path) -> bool {
    dir.join(APAS_FILE).exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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

    fn restore_env_var(key: &str, value: Option<std::ffi::OsString>) {
        if let Some(value) = value {
            std::env::set_var(key, value);
        } else {
            std::env::remove_var(key);
        }
    }

    fn with_isolated_config<T>(test: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        let xdg_config_home = tempfile::tempdir().expect("temp xdg config home");
        let home = tempfile::tempdir().expect("temp home");
        let old_xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
        let old_home = std::env::var_os("HOME");

        std::env::set_var("XDG_CONFIG_HOME", xdg_config_home.path());
        std::env::set_var("HOME", home.path());

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(test));

        restore_env_var("XDG_CONFIG_HOME", old_xdg_config_home);
        restore_env_var("HOME", old_home);

        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
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
    fn project_registry_tmp_path_includes_process_id() {
        let path = unique_temp_dir("tmp-path").join(USER_PROJECTS_FILE);
        let tmp_path = project_registry_tmp_path(&path);
        let expected_name = format!("projects.json.{}.tmp", std::process::id());

        assert_eq!(
            tmp_path.file_name().and_then(|name| name.to_str()),
            Some(expected_name.as_str())
        );
        assert_ne!(tmp_path, path.with_extension("json.tmp"));
    }

    #[test]
    fn write_project_registry_writes_wrapped_json_without_shared_tmp() {
        let dir = unique_temp_dir("write-registry");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join(USER_PROJECTS_FILE);
        let shared_tmp_path = path.with_extension("json.tmp");
        let pid_tmp_path = project_registry_tmp_path(&path);
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
        assert!(
            !pid_tmp_path.exists(),
            "pid-scoped temp file should be renamed away"
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
