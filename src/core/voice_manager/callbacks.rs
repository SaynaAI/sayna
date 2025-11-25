//! Callback types for VoiceManager

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::core::{
    stt::{STTError, STTResult},
    tts::{AudioCallback, AudioData, TTSError},
};

use super::state::InterruptionState;

/// Callback type for STT results
pub type STTCallback =
    Arc<dyn Fn(STTResult) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Callback type for STT streaming errors
pub type STTErrorCallback =
    Arc<dyn Fn(STTError) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Callback type for TTS audio data
pub type TTSAudioCallback =
    Arc<dyn Fn(AudioData) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Callback type for TTS errors
pub type TTSErrorCallback =
    Arc<dyn Fn(TTSError) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Callback type for audio clear operations (e.g., LiveKit audio buffer clearing)
pub type AudioClearCallback =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Callback type for TTS playback completion notifications
pub type TTSCompleteCallback =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Internal TTS callback implementation for the VoiceManager
pub struct VoiceManagerTTSCallback {
    pub audio_callback: Option<TTSAudioCallback>,
    pub error_callback: Option<TTSErrorCallback>,
    pub interruption_state: Option<Arc<InterruptionState>>,
    pub complete_callback: Option<TTSCompleteCallback>,
}

impl AudioCallback for VoiceManagerTTSCallback {
    fn on_audio(&self, audio_data: AudioData) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let callback = self.audio_callback.clone();

        Box::pin(async move {
            // Call user callback if registered
            if let Some(callback) = callback {
                callback(audio_data).await;
            }
        })
    }

    fn on_error(&self, error: TTSError) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let callback = self.error_callback.clone();

        Box::pin(async move {
            if let Some(callback) = callback {
                callback(error).await;
            }
        })
    }

    fn on_complete(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let interruption_state = self.interruption_state.clone();
        let complete_callback = self.complete_callback.clone();

        Box::pin(async move {
            // Mark as completed when TTS finishes
            if let Some(state) = interruption_state {
                state
                    .is_completed
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }

            // Notify user callback
            if let Some(callback) = complete_callback {
                callback().await;
            }
        })
    }
}
