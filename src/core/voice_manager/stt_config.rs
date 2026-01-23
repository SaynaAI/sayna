//! Configuration for STT result processing

/// Configuration for STT result processing
///
/// This simplified config removes all timeout-based fallbacks. Speech final events
/// are ONLY emitted when:
/// 1. STT provider sends a real `is_speech_final=true` event
/// 2. VAD detects silence -> Smart-Turn confirms turn complete (when `stt-vad` enabled)
#[derive(Clone, Copy)]
pub struct STTProcessingConfig {
    /// Use VAD-based silence detection instead of timeout.
    ///
    /// **Note**: When `stt-vad` is compiled, this field is ignored - VAD is always active.
    /// When `stt-vad` is NOT compiled, VAD is unavailable.
    ///
    /// Default: false
    pub use_vad_silence_detection: bool,

    /// VAD silence duration threshold (ms).
    ///
    /// When `stt-vad` is compiled, this is how long continuous silence must be
    /// observed before triggering turn detection.
    ///
    /// Default: 500ms (increased from 200ms for longer conversations)
    pub vad_silence_duration_ms: u64,

    /// Maximum time to wait for turn detection inference to complete (ms).
    ///
    /// This timeout covers both feature extraction and model inference, which run
    /// in `spawn_blocking` to avoid blocking the async runtime. The 800ms default
    /// provides ample headroom for CPU-bound inference while still allowing the
    /// timeout to fire if the model is stuck or overloaded.
    ///
    /// Default: 800ms
    pub turn_detection_inference_timeout_ms: u64,

    /// Window to prevent duplicate speech_final events (ms).
    ///
    /// Default: 500ms
    pub duplicate_window_ms: usize,
}

impl Default for STTProcessingConfig {
    fn default() -> Self {
        Self {
            use_vad_silence_detection: false,
            vad_silence_duration_ms: 500,
            turn_detection_inference_timeout_ms: 800,
            duplicate_window_ms: 500,
        }
    }
}

impl STTProcessingConfig {
    /// Create a new config with VAD-based silence detection settings.
    pub fn with_vad(vad_silence_duration_ms: u64) -> Self {
        Self {
            use_vad_silence_detection: true,
            vad_silence_duration_ms,
            ..Self::default()
        }
    }

    /// Set the VAD enable flag (no-op under `stt-vad` feature).
    #[cfg_attr(feature = "stt-vad", allow(unused_variables, unused_mut))]
    pub fn set_use_vad(mut self, use_vad: bool) -> Self {
        #[cfg(not(feature = "stt-vad"))]
        {
            self.use_vad_silence_detection = use_vad;
        }
        self
    }

    /// Set the VAD silence duration threshold.
    pub fn set_vad_silence_duration_ms(mut self, duration_ms: u64) -> Self {
        self.vad_silence_duration_ms = duration_ms;
        self
    }

    /// Set the turn detection inference timeout.
    pub fn set_turn_detection_inference_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.turn_detection_inference_timeout_ms = timeout_ms;
        self
    }

    /// Set the duplicate window to prevent duplicate speech_final events.
    pub fn set_duplicate_window_ms(mut self, window_ms: usize) -> Self {
        self.duplicate_window_ms = window_ms;
        self
    }
}
