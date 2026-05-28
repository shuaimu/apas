use anyhow::Result;
use serde::{Deserialize, Serialize};
use shared::PaneConfig;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const APAS_FILE: &str = ".apas";
const USER_PROJECTS_FILE: &str = "projects.json";
const LEGACY_USER_REGISTRY_DIR: &str = ".apas";

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
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, content)?;
    std::fs::rename(tmp_path, path)?;
    Ok(())
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
    let tmp_path = apas_path.with_extension("apas.tmp");
    std::fs::write(&tmp_path, &content)?;
    std::fs::rename(&tmp_path, &apas_path)?;
    if let Err(err) = register_project(dir, metadata) {
        tracing::warn!("Failed to register project in user registry: {}", err);
    }
    tracing::debug!("Saved project metadata to {:?}", apas_path);
    Ok(())
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
