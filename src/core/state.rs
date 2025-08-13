use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::config::ServerConfig;
use crate::core::cache::store::{CacheConfig, CacheStore};
use crate::core::tts::get_tts_provider_urls;
use crate::utils::req_manager::ReqManager;

/// Core-specific shared state for the application.
///
/// Holds resources owned by the core layer, such as HTTP request managers
/// for TTS providers.
#[derive(Clone)]
pub struct CoreState {
    /// HTTP request managers for TTS providers - key is provider name (e.g., "deepgram")
    pub tts_req_managers: Arc<RwLock<HashMap<String, Arc<ReqManager>>>>,
    /// Unified cache store (in-memory by default)
    pub cache: Arc<CacheStore>,
}

impl CoreState {
    /// Initialize core state, including TTS request managers.
    pub async fn new(config: &ServerConfig) -> Arc<Self> {
        let mut tts_req_managers = HashMap::new();

        // Build cache configuration based on ServerConfig
        let cache_cfg = if let Some(path) = &config.cache_path {
            CacheConfig::Filesystem {
                path: path.clone(),
                ttl_seconds: config.cache_ttl_seconds,
            }
        } else {
            CacheConfig::Memory {
                max_entries: 5_000_000,
                max_size_bytes: Some(500 * 1024 * 1024),
                ttl_seconds: config.cache_ttl_seconds,
            }
        };
        let cache = Arc::new(
            CacheStore::from_config(cache_cfg)
                .await
                .expect("cache init"),
        );

        let tts_provider_urls = get_tts_provider_urls();
        for (provider, url) in tts_provider_urls {
            match ReqManager::new(10).await {
                Ok(manager) => {
                    // Optionally warm up connections to providers (e.g., Deepgram)
                    let _ = manager.warmup(url.as_str(), "OPTIONS").await;
                    tts_req_managers.insert(provider.clone(), Arc::new(manager));
                    tracing::info!(
                        "Initialized {} ReqManager with 4 concurrent connections",
                        provider
                    );
                }
                Err(e) => {
                    tracing::error!("Failed to create {} ReqManager: {}", provider, e);
                }
            }
        }

        Arc::new(Self {
            tts_req_managers: Arc::new(RwLock::new(tts_req_managers)),
            cache,
        })
    }

    /// Get a TTS request manager for a specific provider
    pub async fn get_tts_req_manager(&self, provider: &str) -> Option<Arc<ReqManager>> {
        self.tts_req_managers.read().await.get(provider).cloned()
    }
}
