use crate::{config::Config, db::Database, session::SessionManager, storage::FileStorage};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use std::path::Path;
use std::sync::Arc;
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
        }
    }
}
