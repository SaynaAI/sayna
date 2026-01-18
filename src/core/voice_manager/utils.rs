//! Utility functions for the VoiceManager module.

/// Minimum audio samples required for turn detection (0.5 seconds at 16kHz).
#[cfg(feature = "stt-vad")]
pub(crate) const MIN_AUDIO_SAMPLES_FOR_TURN_DETECTION: usize = 8000;

/// Maximum audio samples for VAD buffer (30 seconds at 16kHz).
/// Prevents unbounded memory growth during continuous noise or long speech segments.
#[cfg(feature = "stt-vad")]
pub(crate) const MAX_VAD_AUDIO_SAMPLES: usize = 480_000;

/// Get current time in milliseconds since Unix epoch.
pub(crate) fn get_current_time_ms() -> usize {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as usize
}
