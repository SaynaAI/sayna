//! VoiceManager audio processing methods: receive_audio, speak, clear_tts, etc.

use bytes::Bytes;
use std::sync::Arc;
use std::sync::atomic::Ordering;
#[cfg(feature = "stt-vad")]
use tokio::sync::mpsc;
use tracing::{debug, error};

use super::super::{
    errors::{VoiceManagerError, VoiceManagerResult},
    utils,
};
use super::VoiceManager;

impl VoiceManager {
    /// Send audio data to the STT provider for transcription.
    ///
    /// # Arguments
    /// * `audio` - Audio bytes to process
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
    /// let audio_data = vec![0u8; 1024];
    /// voice_manager.receive_audio(audio_data).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn receive_audio(&self, audio: Vec<u8>) -> VoiceManagerResult<()> {
        // Clone the Arc out of the lock to avoid holding RwLock guard across .await.
        // This follows async best practices per CLAUDE.md "Core Components" section:
        // VoiceManager uses "Thread-safe with Arc<RwLock<>> for concurrent access".
        let processor_arc: Option<Arc<_>> = {
            let noise_processor = self.noise_processor.read().await;
            noise_processor.clone()
        };

        // Only convert to Bytes when noise processing is needed; otherwise pass-through
        // the original Vec<u8> without copying. This avoids an unnecessary allocation
        // on the hot path per CLAUDE.md "Core Components" → "VoiceManager".
        let processed_audio = if let Some(processor) = processor_arc {
            let audio_bytes = Bytes::from(audio);
            let sample_rate = self.config.noise_filter_config.sample_rate;
            let input_len = audio_bytes.len();
            match processor.process(audio_bytes.clone(), sample_rate).await {
                Ok(filtered) => {
                    debug!(
                        "Noise filter processed {} bytes -> {} bytes",
                        input_len,
                        filtered.len()
                    );
                    filtered.to_vec()
                }
                Err(e) => {
                    error!(
                        "Noise filter error: {:?}. Passing through original audio.",
                        e
                    );
                    audio_bytes.to_vec()
                }
            }
        } else {
            // No noise processor: pass through unchanged without any copy
            audio
        };

        #[cfg(feature = "stt-vad")]
        if let Some(ref vad_tx) = self.vad_task_tx {
            match vad_tx.try_send(processed_audio.clone()) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    debug!(
                        "VAD channel full, dropping audio frame. \
                         Consider increasing VAD_CHANNEL_CAPACITY if this happens frequently."
                    );
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    tracing::warn!("VAD worker has stopped, skipping VAD processing");
                }
            }
        }

        let mut stt = self.stt.write().await;
        stt.send_audio(processed_audio)
            .await
            .map_err(VoiceManagerError::STTError)?;
        Ok(())
    }

    /// Send text to the TTS provider for synthesis.
    ///
    /// # Arguments
    /// * `text` - Text to synthesize
    /// * `flush` - Whether to immediately flush and start processing the text
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
    /// voice_manager.speak("Hello, world!", false).await?;
    /// voice_manager.speak("How are you?", false).await?;
    /// voice_manager.speak("Final message", true).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn speak(&self, text: &str, flush: bool) -> VoiceManagerResult<()> {
        {
            let mut tts = self.tts.write().await;
            tts.speak(text, flush)
                .await
                .map_err(VoiceManagerError::TTSError)?;
        }

        Ok(())
    }

    /// Send text to the TTS provider with interruption control.
    ///
    /// # Arguments
    /// * `text` - Text to synthesize
    /// * `flush` - Whether to immediately flush and start processing the text
    /// * `allow_interruption` - Whether this audio can be interrupted by STT or clear commands
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    pub async fn speak_with_interruption(
        &self,
        text: &str,
        flush: bool,
        allow_interruption: bool,
    ) -> VoiceManagerResult<()> {
        self.interruption_state
            .allow_interruption
            .store(allow_interruption, Ordering::Release);

        if !allow_interruption {
            if let Some(sample_rate) = self.config.tts_config.sample_rate {
                self.interruption_state
                    .current_sample_rate
                    .store(sample_rate, Ordering::Release);
            }

            let now = utils::get_current_time_ms();

            self.interruption_state
                .non_interruptible_until_ms
                .store(now, Ordering::Release);

            self.interruption_state
                .is_completed
                .store(false, Ordering::SeqCst);
        } else {
            self.interruption_state.reset();
        }

        {
            let mut tts = self.tts.write().await;
            tts.speak(text, flush)
                .await
                .map_err(VoiceManagerError::TTSError)?;
        }

        Ok(())
    }

    /// Check if interruption is currently blocked.
    ///
    /// # Returns
    /// * `bool` - True if interruption is currently blocked
    pub async fn is_interruption_blocked(&self) -> bool {
        !self.interruption_state.can_interrupt()
    }

    /// Clear any queued text from the TTS provider and audio buffers.
    ///
    /// This method clears both the TTS text queue and any audio buffers
    /// (e.g., LiveKit audio source) if an audio clear callback is registered.
    /// The clear operation is fire-and-forget; completion is signaled by the
    /// audio_clear_callback returning.
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    pub async fn clear_tts(&self) -> VoiceManagerResult<()> {
        if !self.interruption_state.can_interrupt() {
            return Ok(());
        }

        debug!("Starting audio clearing process");

        let mut tts = self.tts.write().await;
        tts.clear().await.map_err(VoiceManagerError::TTSError)?;
        drop(tts);

        {
            let callback_opt = self.audio_clear_callback.read().clone();
            if let Some(callback) = callback_opt {
                callback().await;
            }
        }

        self.interruption_state.reset();

        debug!("Completed audio clearing process");

        Ok(())
    }

    /// Flush the TTS provider to process queued text immediately.
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    pub async fn flush_tts(&self) -> VoiceManagerResult<()> {
        let tts = self.tts.read().await;
        tts.flush().await.map_err(VoiceManagerError::TTSError)?;
        Ok(())
    }
}
