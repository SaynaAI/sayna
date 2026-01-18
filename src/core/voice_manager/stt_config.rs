//! Configuration for STT result processing

/// Configuration for STT result processing
#[derive(Clone, Copy)]
pub struct STTProcessingConfig {
    /// Use VAD-based silence detection instead of timeout.
    ///
    /// **Note**: When `stt-vad` is compiled, this field is ignored - VAD is always active.
    /// When `stt-vad` is NOT compiled, VAD is unavailable and timeout-based mode is used.
    ///
    /// Default: false
    pub use_vad_silence_detection: bool,

    /// Time to wait for STT provider to send real speech_final (ms).
    ///
    /// Only used when `stt-vad` feature is NOT compiled.
    ///
    /// Default: 2000ms
    pub stt_speech_final_wait_ms: u64,

    /// VAD silence duration threshold (ms).
    ///
    /// When `stt-vad` is compiled, this is how long continuous silence must be
    /// observed before triggering turn detection.
    ///
    /// Default: 200ms (PipeCat recommendation)
    pub vad_silence_duration_ms: u64,

    /// Maximum time to wait for turn detection inference to complete (ms).
    ///
    /// Default: 100ms
    pub turn_detection_inference_timeout_ms: u64,

    /// Hard upper bound timeout for any user utterance (ms).
    ///
    /// Default: 5000ms
    pub speech_final_hard_timeout_ms: u64,

    /// Window to prevent duplicate speech_final events (ms).
    ///
    /// Default: 500ms
    pub duplicate_window_ms: usize,
}

impl Default for STTProcessingConfig {
    fn default() -> Self {
        Self {
            use_vad_silence_detection: false,
            stt_speech_final_wait_ms: 2000,
            vad_silence_duration_ms: 200,
            turn_detection_inference_timeout_ms: 100,
            speech_final_hard_timeout_ms: 5000,
            duplicate_window_ms: 500,
        }
    }
}

impl STTProcessingConfig {
    /// Create a new STTProcessingConfig with explicit timeout values (legacy API).
    pub fn new(
        stt_speech_final_wait_ms: u64,
        turn_detection_inference_timeout_ms: u64,
        speech_final_hard_timeout_ms: u64,
        duplicate_window_ms: usize,
    ) -> Self {
        Self {
            use_vad_silence_detection: false,
            stt_speech_final_wait_ms,
            vad_silence_duration_ms: 300,
            turn_detection_inference_timeout_ms,
            speech_final_hard_timeout_ms,
            duplicate_window_ms,
        }
    }

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

    /// Set the STT speech final wait timeout (timeout mode only).
    pub fn set_stt_speech_final_wait_ms(mut self, wait_ms: u64) -> Self {
        self.stt_speech_final_wait_ms = wait_ms;
        self
    }

    /// Set the hard timeout for any utterance.
    pub fn set_hard_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.speech_final_hard_timeout_ms = timeout_ms;
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
