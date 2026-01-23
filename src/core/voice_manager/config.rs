//! Configuration types for the VoiceManager

use crate::core::vad::{VADSampleRate, VADSilenceConfig};
use crate::core::{stt::STTConfig, tts::TTSConfig};

/// Configuration for noise filtering in VoiceManager
#[derive(Debug, Clone)]
pub struct NoiseFilterConfig {
    /// Whether noise filtering is enabled for this session.
    /// Defaults to true when `noise-filter` feature is compiled, false otherwise.
    pub enabled: bool,
    /// Sample rate for noise filtering (must match input audio sample rate).
    /// Defaults to 16000 Hz (standard for STT).
    pub sample_rate: u32,
}

impl Default for NoiseFilterConfig {
    fn default() -> Self {
        Self {
            // Enable by default when the feature is compiled
            enabled: cfg!(feature = "noise-filter"),
            sample_rate: 16000,
        }
    }
}

/// Configuration for speech final timing control.
///
/// **Note on removed timeout fields**: The `stt_speech_final_wait_ms` and
/// `speech_final_hard_timeout_ms` fields were removed because all timeout-based
/// fallbacks have been replaced by more reliable mechanisms:
///
/// - When `stt-vad` is enabled: VAD detects silence, then Smart-Turn confirms turn
///   completion with semantic understanding. This is more accurate than arbitrary timeouts.
/// - When `stt-vad` is NOT enabled: The STT provider's native `is_speech_final` signal
///   is used directly, as providers already implement their own timeout logic.
///
/// See `STTProcessingConfig` in `stt_config.rs` for the current speech-final behavior.
#[derive(Debug, Clone, Copy)]
pub struct SpeechFinalConfig {
    /// Maximum time to wait for turn detection inference to complete (ms).
    ///
    /// This timeout covers both feature extraction and model inference in the
    /// Smart-Turn detection path. The work runs in `spawn_blocking` so this
    /// timeout can fire even when the CPU is busy with inference.
    ///
    /// Default: 800ms - provides ample headroom for the ~50ms typical inference
    /// while ensuring stuck models don't block speech_final indefinitely.
    pub turn_detection_inference_timeout_ms: u64,
    /// Window to prevent duplicate speech_final events (ms).
    ///
    /// When both the STT provider and VAD/Smart-Turn emit speech_final events
    /// for the same utterance, this window prevents duplicate callbacks.
    ///
    /// Default: 500ms
    pub duplicate_window_ms: usize,
    /// Silence duration (ms) to wait before retrying Smart-Turn after an Incomplete result.
    ///
    /// When Smart-Turn determines the turn is not yet complete, the system waits
    /// for this additional silence duration before triggering another turn detection
    /// attempt. This prevents rapid-fire retries while still allowing natural pauses.
    ///
    /// Default: 300ms
    pub retry_silence_duration_ms: u64,
    /// Maximum total silence duration (ms) before forcing a speech_final event.
    ///
    /// This acts as a backup timeout: if the user remains silent for this duration
    /// (regardless of Smart-Turn results), a speech_final event is emitted. This
    /// prevents indefinite waiting in edge cases where Smart-Turn keeps returning
    /// Incomplete.
    ///
    /// Default: 5000ms (5 seconds)
    pub backup_silence_timeout_ms: u64,
}

impl Default for SpeechFinalConfig {
    fn default() -> Self {
        Self {
            turn_detection_inference_timeout_ms: 800, // 800ms max for model inference
            duplicate_window_ms: 500,                 // 500ms duplicate prevention window
            retry_silence_duration_ms: 300,           // 300ms silence before retry
            backup_silence_timeout_ms: 5000,          // 5s max silence before forced speech_final
        }
    }
}

/// Configuration for the VoiceManager
#[derive(Debug, Clone)]
pub struct VoiceManagerConfig {
    /// Configuration for the STT provider
    pub stt_config: STTConfig,
    /// Configuration for the TTS provider
    pub tts_config: TTSConfig,
    /// Configuration for speech final timing control
    pub speech_final_config: SpeechFinalConfig,
    /// Configuration for VAD-based silence detection (optional)
    ///
    /// When the `stt-vad` feature is compiled, VAD is always active and uses
    /// these settings for silence detection and turn detection. The `enabled`
    /// field in `VADSilenceConfig` is retained for compatibility but does not
    /// control runtime behavior - VAD runs automatically under `stt-vad`.
    pub vad_config: VADSilenceConfig,
    /// Configuration for noise filtering
    ///
    /// When the `noise-filter` feature is compiled and enabled is true,
    /// audio will be processed through DeepFilterNet before VAD and STT.
    pub noise_filter_config: NoiseFilterConfig,
}

impl VoiceManagerConfig {
    /// Create a new VoiceManagerConfig with default speech final configuration
    ///
    /// STT configuration serves as the source of truth for audio format parameters.
    /// Both noise filter and VAD sample rates are derived from the STT sample rate
    /// to ensure consistent audio processing throughout the receive pipeline.
    pub fn new(stt_config: STTConfig, tts_config: TTSConfig) -> Self {
        // Use STT sample rate for noise filter
        let noise_filter_config = NoiseFilterConfig {
            sample_rate: stt_config.sample_rate,
            ..Default::default()
        };
        // Use STT sample rate for VAD (converted via VADSampleRate::from)
        let mut vad_config = VADSilenceConfig::default();
        vad_config.silero_config.sample_rate = VADSampleRate::from(stt_config.sample_rate);
        Self {
            stt_config,
            tts_config,
            speech_final_config: SpeechFinalConfig::default(),
            vad_config,
            noise_filter_config,
        }
    }

    /// Create a new VoiceManagerConfig with custom speech final configuration
    pub fn with_speech_final_config(
        stt_config: STTConfig,
        tts_config: TTSConfig,
        speech_final_config: SpeechFinalConfig,
    ) -> Self {
        let noise_filter_config = NoiseFilterConfig {
            sample_rate: stt_config.sample_rate,
            ..Default::default()
        };
        // Use STT sample rate for VAD
        let mut vad_config = VADSilenceConfig::default();
        vad_config.silero_config.sample_rate = VADSampleRate::from(stt_config.sample_rate);
        Self {
            stt_config,
            tts_config,
            speech_final_config,
            vad_config,
            noise_filter_config,
        }
    }

    /// Create a new VoiceManagerConfig with VAD-based silence detection enabled
    ///
    /// Note: The provided `vad_config.silero_config.sample_rate` will be overridden
    /// to match the STT sample rate, ensuring consistent audio format across the
    /// receive pipeline.
    ///
    /// # Arguments
    /// * `stt_config` - STT provider configuration
    /// * `tts_config` - TTS provider configuration
    /// * `vad_config` - VAD silence detection configuration
    pub fn with_vad_config(
        stt_config: STTConfig,
        tts_config: TTSConfig,
        mut vad_config: VADSilenceConfig,
    ) -> Self {
        let noise_filter_config = NoiseFilterConfig {
            sample_rate: stt_config.sample_rate,
            ..Default::default()
        };
        // Override VAD sample rate to match STT config
        vad_config.silero_config.sample_rate = VADSampleRate::from(stt_config.sample_rate);
        Self {
            stt_config,
            tts_config,
            speech_final_config: SpeechFinalConfig::default(),
            vad_config,
            noise_filter_config,
        }
    }

    /// Create a new VoiceManagerConfig with all custom configurations
    ///
    /// Note: The provided `vad_config.silero_config.sample_rate` will be overridden
    /// to match the STT sample rate, ensuring consistent audio format across the
    /// receive pipeline.
    ///
    /// # Arguments
    /// * `stt_config` - STT provider configuration
    /// * `tts_config` - TTS provider configuration
    /// * `speech_final_config` - Speech final timing configuration
    /// * `vad_config` - VAD silence detection configuration
    pub fn with_full_config(
        stt_config: STTConfig,
        tts_config: TTSConfig,
        speech_final_config: SpeechFinalConfig,
        mut vad_config: VADSilenceConfig,
    ) -> Self {
        let noise_filter_config = NoiseFilterConfig {
            sample_rate: stt_config.sample_rate,
            ..Default::default()
        };
        // Override VAD sample rate to match STT config
        vad_config.silero_config.sample_rate = VADSampleRate::from(stt_config.sample_rate);
        Self {
            stt_config,
            tts_config,
            speech_final_config,
            vad_config,
            noise_filter_config,
        }
    }

    /// Set VAD configuration on an existing config
    ///
    /// Note: The provided `vad_config.silero_config.sample_rate` will be overridden
    /// to match the STT sample rate from `self.stt_config`, ensuring consistent
    /// audio format across the receive pipeline.
    pub fn set_vad_config(mut self, mut vad_config: VADSilenceConfig) -> Self {
        // Override VAD sample rate to match STT config
        vad_config.silero_config.sample_rate = VADSampleRate::from(self.stt_config.sample_rate);
        self.vad_config = vad_config;
        self
    }

    /// Set noise filter configuration on an existing config
    pub fn set_noise_filter_config(mut self, noise_filter_config: NoiseFilterConfig) -> Self {
        self.noise_filter_config = noise_filter_config;
        self
    }
}
