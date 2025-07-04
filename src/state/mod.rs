use std::sync::Arc;

use crate::config::ServerConfig;

/// Application state that can be shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub config: ServerConfig,
}

impl AppState {
    pub fn new(config: ServerConfig) -> Arc<Self> {
        Arc::new(Self { config })
    }
}
