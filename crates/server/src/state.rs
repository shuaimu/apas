use crate::{config::Config, db::Database, session::SessionManager};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub config: Config,
    pub sessions: Arc<SessionManager>,
}

impl AppState {
    pub fn new(db: Database, config: Config) -> Self {
        Self {
            db,
            config,
            sessions: Arc::new(SessionManager::new()),
        }
    }
}
