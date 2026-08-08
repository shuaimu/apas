use crate::{config::Config, db::Database, session::SessionManager, storage::FileStorage};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use uuid::Uuid;

/// State for device code authentication (CLI login flow)
#[derive(Debug, Clone)]
pub struct DeviceCodeState {
    pub expires_at: DateTime<Utc>,
    pub user_id: Option<Uuid>,
}

/// State for password reset tokens
#[derive(Debug, Clone)]
pub struct PasswordResetState {
    pub email: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub config: Config,
    pub sessions: Arc<SessionManager>,
    pub storage: FileStorage,
    pub device_codes: Arc<DashMap<String, DeviceCodeState>>,
    pub password_reset_tokens: Arc<DashMap<String, PasswordResetState>>,
    project_mutation_gates: Arc<DashMap<String, Arc<RwLock<()>>>>,
}

impl AppState {
    pub fn new(db: Database, config: Config) -> Self {
        // Use the same base directory as the database for file storage
        let db_path = config.database.path.clone();
        let storage_path = Path::new(&db_path)
            .parent()
            .unwrap_or(Path::new("./data"))
            .to_path_buf();

        Self {
            db,
            config,
            sessions: Arc::new(SessionManager::new()),
            storage: FileStorage::new(storage_path),
            device_codes: Arc::new(DashMap::new()),
            password_reset_tokens: Arc::new(DashMap::new()),
            project_mutation_gates: Arc::new(DashMap::new()),
        }
    }

    fn project_mutation_gate(&self, project_id: &str) -> Arc<RwLock<()>> {
        self.project_mutation_gates
            .entry(project_id.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone()
    }

    /// Shared permit for an ordinary project operation. Callers must check
    /// current database lifecycle after acquiring it; deletion marks the row
    /// first and then drains these permits before erasing storage.
    pub async fn project_operation_guard(&self, project_id: &str) -> OwnedRwLockReadGuard<()> {
        self.project_mutation_gate(project_id).read_owned().await
    }

    /// Resolve a session to its canonical project, take the shared permit,
    /// and fail closed unless the project is still active.
    pub async fn active_session_operation(
        &self,
        session_id: &str,
    ) -> anyhow::Result<(String, OwnedRwLockReadGuard<()>)> {
        let project_id = self
            .db
            .get_project_id_for_session(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("project session not found"))?;
        let guard = self.project_operation_guard(&project_id).await;
        let project = self
            .db
            .get_project(&project_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("project not found"))?;
        anyhow::ensure!(
            project.lifecycle() == crate::db::ProjectLifecycle::Active,
            "project is not active"
        );
        Ok((project_id, guard))
    }

    pub async fn project_deletion_guard(&self, project_id: &str) -> OwnedRwLockWriteGuard<()> {
        self.project_mutation_gate(project_id).write_owned().await
    }

    pub fn forget_project_mutation_gate(&self, project_id: &str) {
        self.project_mutation_gates.remove(project_id);
    }
}
