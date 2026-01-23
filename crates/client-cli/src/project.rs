use anyhow::Result;
use serde::{Deserialize, Serialize};
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
    /// Claude session ID for the deadloop pane (persisted for --resume)
    #[serde(default)]
    pub deadloop_claude_session_id: Option<Uuid>,
    /// Claude session ID for the interactive pane (persisted for --resume)
    #[serde(default)]
    pub interactive_claude_session_id: Option<Uuid>,
}

impl ProjectMetadata {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            prompt: None,
            deadloop_claude_session_id: None,
            interactive_claude_session_id: None,
        }
    }

    pub fn with_name(name: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: Some(name),
            created_at: chrono::Utc::now().to_rfc3339(),
            prompt: None,
            deadloop_claude_session_id: None,
            interactive_claude_session_id: None,
        }
    }

    /// Get or create the deadloop Claude session ID
    pub fn get_or_create_deadloop_session_id(&mut self) -> Uuid {
        if let Some(id) = self.deadloop_claude_session_id {
            id
        } else {
            let id = Uuid::new_v4();
            self.deadloop_claude_session_id = Some(id);
            id
        }
    }

    /// Get or create the interactive Claude session ID
    pub fn get_or_create_interactive_session_id(&mut self) -> Uuid {
        if let Some(id) = self.interactive_claude_session_id {
            id
        } else {
            let id = Uuid::new_v4();
            self.interactive_claude_session_id = Some(id);
            id
        }
    }
}

/// Get or create the .apas metadata file for a directory
pub fn get_or_create_project(dir: &Path) -> Result<ProjectMetadata> {
    let apas_path = dir.join(APAS_FILE);

    if apas_path.exists() {
        // Read existing metadata
        let content = std::fs::read_to_string(&apas_path)?;
        let metadata: ProjectMetadata = serde_json::from_str(&content)?;
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
            deadloop_claude_session_id: None,
            interactive_claude_session_id: None,
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
