use anyhow::Result;
use serde::{Deserialize, Serialize};
use shared::PaneConfig;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const APAS_FILE: &str = ".apas";
const USER_REGISTRY_DIR: &str = ".apas";
const USER_PROJECTS_FILE: &str = "projects.json";

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
                    label: Some("Claude Deadloop".to_string()),
                    model: None,
                },
                PaneConfig {
                    pane_id: shared::PANE_ID_INTERACTIVE,
                    provider: shared::Provider::Claude,
                    mode: shared::PaneMode::Interactive,
                    session_id: interactive_session,
                    is_paused: false,
                    stop_requested: false,
                    prompt: None,
                    label: Some("Claude Interactive".to_string()),
                    model: None,
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
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    let registry_dir = home.join(USER_REGISTRY_DIR);
    std::fs::create_dir_all(&registry_dir)?;
    Ok(registry_dir.join(USER_PROJECTS_FILE))
}

pub fn list_registered_projects() -> Result<Vec<RegisteredProject>> {
    let path = project_registry_path()?;
    let registry = read_project_registry(&path)?;
    Ok(registry.projects)
}

pub fn register_project(dir: &Path, metadata: &ProjectMetadata) -> Result<()> {
    let path = project_registry_path()?;
    let mut registry = match read_project_registry(&path) {
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

fn write_project_registry(path: &Path, registry: &ProjectRegistry) -> Result<()> {
    let content = serde_json::to_string_pretty(registry)?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, content)?;
    std::fs::rename(tmp_path, path)?;
    Ok(())
}

/// Get or create the .apas metadata file for a directory
pub fn get_or_create_project(dir: &Path) -> Result<ProjectMetadata> {
    let apas_path = dir.join(APAS_FILE);

    if apas_path.exists() {
        // Read existing metadata
        let content = std::fs::read_to_string(&apas_path)?;
        let mut metadata: ProjectMetadata = serde_json::from_str(&content)?;
        // Migrate legacy pane config if needed
        metadata.migrate_legacy();
        if let Err(err) = register_project(dir, &metadata) {
            tracing::warn!("Failed to register project in user registry: {}", err);
        }
        Ok(metadata)
    } else {
        // Create new metadata with directory name as project name
        let name = dir.file_name().and_then(|n| n.to_str()).map(String::from);

        let metadata = ProjectMetadata {
            id: Uuid::new_v4(),
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
    std::fs::write(&apas_path, content)?;
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
