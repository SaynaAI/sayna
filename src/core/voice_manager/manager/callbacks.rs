//! VoiceManager callback registration methods.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::core::stt::{
    STTError, STTErrorCallback as ProviderSTTErrorCallback, STTResult, STTResultCallback,
};
use crate::core::tts::{AudioData, TTSError};

use super::super::errors::{VoiceManagerError, VoiceManagerResult};
use super::VoiceManager;

impl VoiceManager {
    /// Register a callback for STT results.
    ///
    /// # Arguments
    /// * `callback` - Callback function to handle STT results
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
    /// voice_manager.on_stt_result(|result| {
    ///     Box::pin(async move {
    ///         println!("Transcription: {}", result.transcript);
    ///     })
    /// }).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn on_stt_result<F>(&self, callback: F) -> VoiceManagerResult<()>
    where
        F: Fn(STTResult) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
    {
        let callback = Arc::new(callback);

        {
            let mut state = self.speech_final_state.write();
            state.user_callback = Some(callback.clone());
        }

        let speech_final_state_clone = self.speech_final_state.clone();
        let interruption_state_clone = self.interruption_state.clone();
        let stt_processor_clone = self.stt_result_processor.clone();

        let wrapper_callback: STTResultCallback = Arc::new(move |result| {
            let callback = callback.clone();
            let speech_final_state = speech_final_state_clone.clone();
            let interruption_state = interruption_state_clone.clone();
            let stt_processor = stt_processor_clone.clone();

            Box::pin(async move {
                if !interruption_state.can_interrupt() {
                    return;
                }

                let processed_result = stt_processor
                    .process_result(result, speech_final_state)
                    .await;

                if let Some(processed_result) = processed_result {
                    callback(processed_result).await;
                }
            })
        });

        {
            let mut stt = self.stt.write().await;
            stt.on_result(wrapper_callback)
                .await
                .map_err(VoiceManagerError::STTError)?;
        }

        Ok(())
    }

    /// Register a callback for STT streaming errors.
    ///
    /// This callback is triggered when errors occur during STT streaming,
    /// such as permission errors, network failures, or API errors that happen
    /// after the initial connection is established.
    ///
    /// # Arguments
    /// * `callback` - Callback function to handle STT errors
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    ///
    /// # Example
    /// ```rust,ignore
    /// # use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
    /// # use sayna::core::stt::STTConfig;
    /// # use sayna::core::tts::TTSConfig;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let voice_manager = VoiceManager::new(
    /// #     VoiceManagerConfig::new(STTConfig::default(), TTSConfig::default()),
    /// #     None
    /// # )?;
    /// voice_manager.on_stt_error(|error| {
    ///     Box::pin(async move {
    ///         eprintln!("STT streaming error: {}", error);
    ///     })
    /// }).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn on_stt_error<F>(&self, callback: F) -> VoiceManagerResult<()>
    where
        F: Fn(STTError) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
    {
        let callback = Arc::new(callback);

        let wrapper_callback: ProviderSTTErrorCallback = Arc::new(move |error| {
            let callback = callback.clone();
            Box::pin(async move {
                callback(error).await;
            })
        });

        {
            let mut stt = self.stt.write().await;
            stt.on_error(wrapper_callback)
                .await
                .map_err(VoiceManagerError::STTError)?;
        }

        Ok(())
    }

    /// Register a callback for TTS audio data.
    ///
    /// # Arguments
    /// * `callback` - Callback function to handle TTS audio data
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
    /// voice_manager.on_tts_audio(|audio_data| {
    ///     Box::pin(async move {
    ///         println!("Received {} bytes of audio", audio_data.data.len());
    ///     })
    /// }).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn on_tts_audio<F>(&self, callback: F) -> VoiceManagerResult<()>
    where
        F: Fn(AudioData) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
    {
        {
            let mut tts_audio_callback = self.tts_audio_callback.write();
            *tts_audio_callback = Some(Arc::new(callback));
        }

        self.update_tts_callback().await
    }

    /// Register a callback for TTS errors.
    ///
    /// # Arguments
    /// * `callback` - Callback function to handle TTS errors
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    pub async fn on_tts_error<F>(&self, callback: F) -> VoiceManagerResult<()>
    where
        F: Fn(TTSError) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
    {
        {
            let mut tts_error_callback = self.tts_error_callback.write();
            *tts_error_callback = Some(Arc::new(callback));
        }

        self.update_tts_callback().await
    }

    /// Register a callback for audio clear operations.
    ///
    /// This callback is called when the TTS queue is cleared and any audio
    /// buffers (e.g., LiveKit audio source) need to be cleared as well.
    ///
    /// # Arguments
    /// * `callback` - Closure that returns a Future for clearing audio
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
    /// voice_manager.on_audio_clear(|| {
    ///     Box::pin(async move {
    ///         println!("Clearing audio buffers");
    ///     })
    /// }).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn on_audio_clear<F>(&self, callback: F) -> VoiceManagerResult<()>
    where
        F: Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
    {
        let mut audio_clear_callback = self.audio_clear_callback.write();
        *audio_clear_callback = Some(Arc::new(callback));
        Ok(())
    }

    /// Register a callback to be invoked when TTS playback completes.
    ///
    /// The completion callback is triggered after the TTS provider finishes generating
    /// all audio chunks for a given `speak()` command. This is useful for:
    /// - Updating UI state (hiding loading indicators)
    /// - Coordinating sequential actions
    /// - Analytics and monitoring
    /// - Knowing when it's safe to perform operations
    ///
    /// # Important Notes
    /// - Callback fires once per `speak()` call
    /// - Callback fires after all audio chunks are generated
    /// - Callback timing indicates server-side generation completion, not client playback
    /// - Multiple `speak()` calls will trigger multiple callbacks in FIFO order
    ///
    /// # Arguments
    /// * `callback` - Async function to call when TTS playback completes
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    ///
    /// # Example
    /// ```rust,ignore
    /// use sayna::core::voice_manager::VoiceManager;
    ///
    /// let voice_manager = VoiceManager::new(config, None)?;
    /// voice_manager.start().await?;
    ///
    /// voice_manager.on_tts_complete(|| {
    ///     Box::pin(async move {
    ///         println!("TTS playback completed!");
    ///     })
    /// }).await?;
    ///
    /// voice_manager.speak("Hello world", true, true).await?;
    /// ```
    pub async fn on_tts_complete<F>(&self, callback: F) -> VoiceManagerResult<()>
    where
        F: Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
    {
        {
            *self.tts_complete_callback.write() = Some(Arc::new(callback));
        }

        self.update_tts_callback().await
    }
}
