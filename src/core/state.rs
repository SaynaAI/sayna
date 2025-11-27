use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(feature = "turn-detect")]
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
#[cfg(not(feature = "turn-detect"))]
use tracing::info;
#[cfg(feature = "turn-detect")]
use tracing::{debug, info, warn};

use crate::config::ServerConfig;
use crate::core::cache::store::{CacheConfig, CacheStore};
use crate::core::tts::get_tts_provider_urls;
#[cfg(not(feature = "turn-detect"))]
use crate::core::turn_detect::TurnDetector;
#[cfg(feature = "turn-detect")]
use crate::core::turn_detect::{TurnDetector, TurnDetectorConfig};
use crate::state::SipHooksState;
use crate::utils::req_manager::ReqManager;

/// Core-specific shared state for the application.
///
/// Holds resources owned by the core layer, such as HTTP request managers
/// for TTS providers and the turn detector for speech completion detection.
#[derive(Clone)]
pub struct CoreState {
    /// HTTP request managers for TTS providers - key is provider name (e.g., "deepgram")
    pub tts_req_managers: Arc<RwLock<HashMap<String, Arc<ReqManager>>>>,
    /// Unified cache store (in-memory by default)
    pub cache: Arc<CacheStore>,
    /// Turn detector for determining end of user speech turns
    pub turn_detector: Option<Arc<RwLock<TurnDetector>>>,
    /// SIP hooks runtime state with preserved secrets
    pub sip_hooks_state: Option<Arc<RwLock<SipHooksState>>>,
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
            match ReqManager::new(4).await {
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

        // Initialize and warmup Turn Detector
        let turn_detector = Self::initialize_turn_detector(config.cache_path.as_ref()).await;

        let sip_hooks_state = if let Some(sip_config) = &config.sip {
            Some(Arc::new(RwLock::new(
                SipHooksState::new(sip_config, config.cache_path.as_deref()).await,
            )))
        } else {
            None
        };

        Arc::new(Self {
            tts_req_managers: Arc::new(RwLock::new(tts_req_managers)),
            cache,
            turn_detector,
            sip_hooks_state,
        })
    }

    /// Get a TTS request manager for a specific provider
    pub async fn get_tts_req_manager(&self, provider: &str) -> Option<Arc<ReqManager>> {
        self.tts_req_managers.read().await.get(provider).cloned()
    }

    #[cfg(feature = "turn-detect")]
    /// Initialize and warmup the Turn Detector model
    async fn initialize_turn_detector(
        cache_path: Option<&PathBuf>,
    ) -> Option<Arc<RwLock<TurnDetector>>> {
        info!("Initializing Turn Detector for speech completion detection");

        let start = Instant::now();

        // Create config with cache path
        let config = TurnDetectorConfig {
            cache_path: cache_path.cloned(),
            ..Default::default()
        };

        // Add a timeout to prevent hanging forever during initialization
        let init_timeout = Duration::from_secs(30);

        match tokio::time::timeout(init_timeout, TurnDetector::with_config(config)).await {
            Ok(Ok(detector)) => {
                let init_elapsed = start.elapsed();
                info!("Turn Detector initialized in {:?}", init_elapsed);

                // Warmup the model with sample inputs to ensure it's fully loaded
                let warmup_start = Instant::now();
                let warmup_timeout = Duration::from_secs(10);

                match tokio::time::timeout(warmup_timeout, Self::warmup_turn_detector(&detector))
                    .await
                {
                    Ok(Ok(())) => {
                        let warmup_elapsed = warmup_start.elapsed();
                        info!("Turn Detector warmup completed in {:?}", warmup_elapsed);
                    }
                    Ok(Err(e)) => {
                        warn!("Turn Detector warmup failed: {:?}", e);
                    }
                    Err(_) => {
                        warn!("Turn Detector warmup timed out after {:?}", warmup_timeout);
                    }
                }

                let total_elapsed = start.elapsed();
                info!("Turn Detector fully ready in {:?}", total_elapsed);

                Some(Arc::new(RwLock::new(detector)))
            }
            Ok(Err(e)) => {
                warn!(
                    "Failed to initialize Turn Detector: {:?}. \
                    Falling back to timer-based detection.",
                    e
                );
                None
            }
            Err(_) => {
                warn!(
                    "Turn Detector initialization timed out after {:?}. \
                    Falling back to timer-based detection.",
                    init_timeout
                );
                None
            }
        }
    }

    #[cfg(not(feature = "turn-detect"))]
    async fn initialize_turn_detector(
        cache_path: Option<&PathBuf>,
    ) -> Option<Arc<RwLock<TurnDetector>>> {
        let _ = cache_path;
        info!("Turn detection feature disabled; using timer-based speech_final fallback logic");
        None
    }

    #[cfg(feature = "turn-detect")]
    /// Warmup the Turn Detector model with sample inputs
    async fn warmup_turn_detector(detector: &TurnDetector) -> anyhow::Result<()> {
        debug!("Starting Turn Detector warmup with sample inputs");

        // Sample inputs that cover common speech patterns
        let warmup_samples = [
            "Hello",
            "How are you?",
            "What is the weather like today?",
            "Thank you very much",
            "I was wondering if",
            "Can you help me with",
            "That's great, thanks!",
            "Let me think about it",
            "I need to",
        ];

        // Run predictions on all samples to ensure model is fully loaded
        for (i, sample) in warmup_samples.iter().enumerate() {
            let start = Instant::now();
            match detector.predict_end_of_turn(sample).await {
                Ok(probability) => {
                    let elapsed = start.elapsed();
                    debug!(
                        "Warmup sample {} ('{}...'): probability={:.3}, time={:?}",
                        i + 1,
                        &sample.chars().take(20).collect::<String>(),
                        probability,
                        elapsed
                    );
                }
                Err(e) => {
                    warn!("Warmup sample {} failed: {:?}", i + 1, e);
                }
            }
        }

        debug!("Turn Detector warmup completed");
        Ok(())
    }

    /// Get the Turn Detector if available
    pub fn get_turn_detector(&self) -> Option<Arc<RwLock<TurnDetector>>> {
        self.turn_detector.clone()
    }

    /// Get SIP hooks runtime state if SIP is configured.
    pub fn get_sip_hooks_state(&self) -> Option<Arc<RwLock<SipHooksState>>> {
        self.sip_hooks_state.clone()
    }
}
