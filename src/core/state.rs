use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(feature = "stt-vad")]
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
#[cfg(not(feature = "stt-vad"))]
use tracing::info;
#[cfg(feature = "stt-vad")]
use tracing::{debug, info, warn};

use crate::config::ServerConfig;
use crate::core::cache::store::{CacheConfig, CacheStore};
use crate::core::tts::get_tts_provider_urls;
#[cfg(not(feature = "stt-vad"))]
use crate::core::turn_detect::TurnDetector;
#[cfg(feature = "stt-vad")]
use crate::core::turn_detect::{TurnDetector, TurnDetectorConfig};
#[cfg(feature = "stt-vad")]
use crate::core::vad::{SileroVAD, SileroVADConfig};
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
    /// Silero-VAD model for voice activity detection
    #[cfg(feature = "stt-vad")]
    pub silero_vad: Option<Arc<RwLock<SileroVAD>>>,
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

        // Initialize Silero-VAD for voice activity detection
        #[cfg(feature = "stt-vad")]
        let silero_vad = Self::initialize_silero_vad(config.cache_path.as_ref()).await;

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
            #[cfg(feature = "stt-vad")]
            silero_vad,
            sip_hooks_state,
        })
    }

    /// Get a TTS request manager for a specific provider
    pub async fn get_tts_req_manager(&self, provider: &str) -> Option<Arc<ReqManager>> {
        self.tts_req_managers.read().await.get(provider).cloned()
    }

    #[cfg(feature = "stt-vad")]
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

    #[cfg(not(feature = "stt-vad"))]
    async fn initialize_turn_detector(
        cache_path: Option<&PathBuf>,
    ) -> Option<Arc<RwLock<TurnDetector>>> {
        let _ = cache_path;
        info!("Turn detection feature disabled; using timer-based speech_final fallback logic");
        None
    }

    #[cfg(feature = "stt-vad")]
    /// Warmup the Turn Detector model with sample audio inputs
    async fn warmup_turn_detector(detector: &TurnDetector) -> anyhow::Result<()> {
        debug!("Starting Turn Detector warmup with sample audio inputs");

        let sample_rate = detector.sample_rate();

        // Generate sample audio inputs with varying durations to warm up the model
        // These simulate different audio lengths that the model might receive
        let warmup_durations_seconds = [0.5, 1.0, 2.0, 4.0];

        // Run predictions on all samples to ensure model is fully loaded
        for (i, duration) in warmup_durations_seconds.iter().enumerate() {
            // Generate a simple sine wave at 440 Hz (A4 note) for warmup
            let num_samples = (sample_rate as f32 * duration) as usize;
            let audio_samples: Vec<i16> = (0..num_samples)
                .map(|j| {
                    let t = j as f32 / sample_rate as f32;
                    let sample = (2.0 * std::f32::consts::PI * 440.0 * t).sin();
                    (sample * 16000.0) as i16
                })
                .collect();

            let start = Instant::now();
            match detector.predict_end_of_turn(&audio_samples).await {
                Ok(probability) => {
                    let elapsed = start.elapsed();
                    debug!(
                        "Warmup sample {} ({}s, {} samples): probability={:.3}, time={:?}",
                        i + 1,
                        duration,
                        audio_samples.len(),
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

    #[cfg(feature = "stt-vad")]
    /// Initialize and warmup the Silero-VAD model for voice activity detection
    async fn initialize_silero_vad(cache_path: Option<&PathBuf>) -> Option<Arc<RwLock<SileroVAD>>> {
        info!("Initializing Silero-VAD for voice activity detection");

        let start = Instant::now();

        // Create config with cache path
        let config = SileroVADConfig {
            cache_path: cache_path.cloned(),
            ..Default::default()
        };

        // Add a timeout to prevent hanging forever during initialization
        let init_timeout = Duration::from_secs(30);

        match tokio::time::timeout(init_timeout, SileroVAD::with_config(config)).await {
            Ok(Ok(vad)) => {
                let elapsed = start.elapsed();
                info!("Silero-VAD initialized in {:?}", elapsed);

                // Warmup the model with a sample frame to ensure it's fully loaded
                let warmup_start = Instant::now();
                let warmup_timeout = Duration::from_secs(5);

                match tokio::time::timeout(warmup_timeout, Self::warmup_silero_vad(&vad)).await {
                    Ok(Ok(())) => {
                        let warmup_elapsed = warmup_start.elapsed();
                        debug!("Silero-VAD warmup completed in {:?}", warmup_elapsed);
                    }
                    Ok(Err(e)) => {
                        warn!("Silero-VAD warmup failed: {:?}", e);
                    }
                    Err(_) => {
                        warn!("Silero-VAD warmup timed out after {:?}", warmup_timeout);
                    }
                }

                let total_elapsed = start.elapsed();
                info!("Silero-VAD fully ready in {:?}", total_elapsed);

                Some(Arc::new(RwLock::new(vad)))
            }
            Ok(Err(e)) => {
                warn!(
                    "Failed to initialize Silero-VAD: {:?}. \
                    VAD-based silence detection disabled.",
                    e
                );
                None
            }
            Err(_) => {
                warn!(
                    "Silero-VAD initialization timed out after {:?}. \
                    VAD-based silence detection disabled.",
                    init_timeout
                );
                None
            }
        }
    }

    #[cfg(feature = "stt-vad")]
    /// Warmup the Silero-VAD model with a sample frame
    async fn warmup_silero_vad(vad: &SileroVAD) -> anyhow::Result<()> {
        debug!("Starting Silero-VAD warmup with sample frame");

        // Create a sample frame of silence (512 samples for 16kHz)
        let sample_frame: Vec<i16> = vec![0i16; 512];

        let start = Instant::now();
        let probability = vad.process_audio(&sample_frame).await?;
        let elapsed = start.elapsed();

        debug!(
            "Silero-VAD warmup: probability={:.3}, time={:?}",
            probability, elapsed
        );

        Ok(())
    }

    /// Get the Silero-VAD if available
    #[cfg(feature = "stt-vad")]
    pub fn get_silero_vad(&self) -> Option<Arc<RwLock<SileroVAD>>> {
        self.silero_vad.clone()
    }

    /// Get the Silero-VAD if available (stub when feature disabled)
    #[cfg(not(feature = "stt-vad"))]
    pub fn get_silero_vad(&self) -> Option<()> {
        None
    }

    /// Get SIP hooks runtime state if SIP is configured.
    pub fn get_sip_hooks_state(&self) -> Option<Arc<RwLock<SipHooksState>>> {
        self.sip_hooks_state.clone()
    }
}
