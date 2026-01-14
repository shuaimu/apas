use crate::{config::Config, db::Database, session::SessionManager, storage::FileStorage};
use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub config: Config,
    pub sessions: Arc<SessionManager>,
    pub storage: FileStorage,
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
        }
    }
}
