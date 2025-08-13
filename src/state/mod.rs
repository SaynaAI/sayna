use std::sync::Arc;

use crate::config::ServerConfig;
use crate::core::CoreState;
use crate::core::cache::store::CacheStore;
use crate::utils::req_manager::ReqManager;

/// Application state that can be shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub config: ServerConfig,
    /// Core layer state that holds shared resources, such as TTS request managers
    pub core_state: Arc<CoreState>,
}

impl AppState {
    pub async fn new(config: ServerConfig) -> Arc<Self> {
        let core_state = CoreState::new(&config).await;

        Arc::new(Self { config, core_state })
    }

    /// Get a TTS request manager for a specific provider
    pub async fn get_tts_req_manager(&self, provider: &str) -> Option<Arc<ReqManager>> {
        self.core_state.get_tts_req_manager(provider).await
    }

    /// Get a handle to the application's cache store
    pub fn cache(&self) -> Arc<CacheStore> {
        self.core_state.cache.clone()
    }
}
