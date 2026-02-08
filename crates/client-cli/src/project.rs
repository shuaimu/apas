use anyhow::Result;
use serde::{Deserialize, Serialize};
use shared::PaneConfig;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const APAS_FILE: &str = ".apas";

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
            let interactive_session = self.interactive_claude_session_id.unwrap_or_else(Uuid::new_v4);

            self.panes = vec![
                PaneConfig {
                    pane_id: shared::PANE_ID_DEADLOOP,
                    provider: shared::Provider::Claude,
                    mode: shared::PaneMode::Deadloop,
                    session_id: deadloop_session,
                    is_paused: self.is_paused,
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
        if let Some(pane) = self.panes.iter().find(|p| p.pane_id == shared::PANE_ID_DEADLOOP) {
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
        if let Some(pane) = self.panes.iter().find(|p| p.pane_id == shared::PANE_ID_INTERACTIVE) {
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

/// Get or create the .apas metadata file for a directory
pub fn get_or_create_project(dir: &Path) -> Result<ProjectMetadata> {
    let apas_path = dir.join(APAS_FILE);

    if apas_path.exists() {
        // Read existing metadata
        let content = std::fs::read_to_string(&apas_path)?;
        let mut metadata: ProjectMetadata = serde_json::from_str(&content)?;
        // Migrate legacy pane config if needed
        metadata.migrate_legacy();
        Ok(metadata)
    } else {
        // Create new metadata with directory name as project name
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .map(String::from);

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

        tracing::info!("Created new project: {} ({:?})", metadata.id, metadata.name);
        Ok(metadata)
    }
}

/// Save project metadata back to the .apas file
pub fn save_project(dir: &Path, metadata: &ProjectMetadata) -> Result<()> {
    let apas_path = dir.join(APAS_FILE);
    let content = serde_json::to_string_pretty(metadata)?;
    std::fs::write(&apas_path, content)?;
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
