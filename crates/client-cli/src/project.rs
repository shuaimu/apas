use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const APAS_FILE: &str = ".apas";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMetadata {
    /// Unique identifier for this project
    pub id: Uuid,
    /// Optional human-readable project name
    pub name: Option<String>,
    /// When the project was first initialized
    pub created_at: String,
    /// Custom prompt to use (if not set, uses default)
    #[serde(default)]
    pub prompt: Option<String>,
}

impl ProjectMetadata {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            prompt: None,
        }
    }

    pub fn with_name(name: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: Some(name),
            created_at: chrono::Utc::now().to_rfc3339(),
            prompt: None,
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
        };

        // Save to file
        let content = serde_json::to_string_pretty(&metadata)?;
        std::fs::write(&apas_path, content)?;

        tracing::info!("Created new project: {} ({:?})", metadata.id, metadata.name);
        Ok(metadata)
    }
}

/// Get the .apas file path for a directory
pub fn get_apas_path(dir: &Path) -> PathBuf {
    dir.join(APAS_FILE)
}

/// Check if a directory has been initialized as an apas project
pub fn is_project(dir: &Path) -> bool {
    dir.join(APAS_FILE).exists()
}
