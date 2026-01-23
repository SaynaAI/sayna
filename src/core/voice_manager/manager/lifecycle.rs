//! VoiceManager lifecycle methods: start, stop, and readiness checks.

use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::core::cache::store::CacheStore;
use crate::core::noise_filter::StreamNoiseProcessor;

use super::super::{
    callbacks::VoiceManagerTTSCallback,
    errors::{VoiceManagerError, VoiceManagerResult},
};
use super::VoiceManager;

impl VoiceManager {
    /// Set the TTS cache store and optionally the precomputed TTS config hash.
    pub async fn set_tts_cache(
        &self,
        cache: Arc<CacheStore>,
        config_hash: Option<String>,
    ) -> VoiceManagerResult<()> {
        let mut tts = self.tts.write().await;
        if let Some(provider) = tts.get_provider() {
            provider.set_cache(cache).await;
            if let Some(hash) = config_hash {
                provider.set_tts_config_hash(hash).await;
            }
            Ok(())
        } else {
            Err(VoiceManagerError::InitializationError(
                "TTS provider does not support cache".to_string(),
            ))
        }
    }

    /// Update the TTS callback with current stored callbacks.
    pub(super) async fn update_tts_callback(&self) -> VoiceManagerResult<()> {
        let audio_callback = self.tts_audio_callback.read().clone();
        let error_callback = self.tts_error_callback.read().clone();
        let complete_callback = self.tts_complete_callback.read().clone();
        let interruption_state = self.interruption_state.clone();

        let tts_callback = Arc::new(VoiceManagerTTSCallback {
            audio_callback,
            error_callback,
            interruption_state: Some(interruption_state),
            complete_callback,
        });

        let mut tts = self.tts.write().await;
        tts.on_audio(tts_callback)
            .map_err(VoiceManagerError::TTSError)?;

        Ok(())
    }

    /// Start the VoiceManager by connecting to both STT and TTS providers.
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    ///
    /// # Example
    /// ```rust,ignore
    /// # use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
    /// # use sayna::core::stt::STTConfig;
    /// # use sayna::core::tts::TTSConfig;
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = VoiceManagerConfig::new(STTConfig::default(), TTSConfig::default());
    /// # let voice_manager = VoiceManager::new(config, None)?;
    /// voice_manager.start().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start(&self) -> VoiceManagerResult<()> {
        if self.config.noise_filter_config.enabled {
            let sample_rate = self.config.noise_filter_config.sample_rate;
            info!(
                "Initializing noise processor with sample_rate={}Hz",
                sample_rate
            );
            match StreamNoiseProcessor::new(sample_rate).await {
                Ok(processor) => {
                    let mut noise_processor = self.noise_processor.write().await;
                    *noise_processor = Some(Arc::new(processor));
                    info!("Noise processor initialized successfully");
                }
                Err(e) => {
                    warn!(
                        "Failed to initialize noise processor: {:?}. Audio will pass through unfiltered.",
                        e
                    );
                }
            }
        } else {
            debug!("Noise filtering disabled in config");
        }

        {
            let mut stt = self.stt.write().await;
            stt.connect().await.map_err(VoiceManagerError::STTError)?;
        }

        {
            let mut tts = self.tts.write().await;
            tts.connect().await.map_err(VoiceManagerError::TTSError)?;
        }

        self.update_tts_callback().await?;

        Ok(())
    }

    /// Stop the VoiceManager by disconnecting from both providers.
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    ///
    /// # Example
    /// ```rust,ignore
    /// # use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
    /// # use sayna::core::stt::STTConfig;
    /// # use sayna::core::tts::TTSConfig;
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = VoiceManagerConfig::new(STTConfig::default(), TTSConfig::default());
    /// # let voice_manager = VoiceManager::new(config, None)?;
    /// voice_manager.stop().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn stop(&self) -> VoiceManagerResult<()> {
        #[cfg(feature = "stt-vad")]
        {
            if let Some(vad) = &self.vad {
                vad.write().await.reset().await;
            }
            self.silence_tracker.reset();
            self.vad_state.reset();
        }

        {
            let mut state = self.speech_final_state.write();
            state.reset_for_next_segment();
            state.user_callback = None;
        }

        {
            let mut stt = self.stt.write().await;
            stt.disconnect()
                .await
                .map_err(VoiceManagerError::STTError)?;
        }

        {
            let mut tts = self.tts.write().await;
            tts.disconnect()
                .await
                .map_err(VoiceManagerError::TTSError)?;
        }

        Ok(())
    }

    /// Check if both STT and TTS providers are ready.
    ///
    /// # Returns
    /// * `bool` - True if both providers are ready, false otherwise
    ///
    /// # Example
    /// ```rust,ignore
    /// # use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
    /// # use sayna::core::stt::STTConfig;
    /// # use sayna::core::tts::TTSConfig;
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = VoiceManagerConfig::new(STTConfig::default(), TTSConfig::default());
    /// # let voice_manager = VoiceManager::new(config, None)?;
    /// if voice_manager.is_ready().await {
    ///     println!("VoiceManager is ready!");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn is_ready(&self) -> bool {
        let stt_ready = {
            let stt = self.stt.read().await;
            stt.is_ready()
        };

        let tts_ready = {
            let tts = self.tts.read().await;
            tts.is_ready()
        };

        stt_ready && tts_ready
    }
}
