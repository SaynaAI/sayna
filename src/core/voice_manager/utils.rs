//! Utility functions for the VoiceManager module.

/// Minimum audio samples required for turn detection (0.5 seconds at 16kHz).
#[cfg(feature = "stt-vad")]
pub(crate) const MIN_AUDIO_SAMPLES_FOR_TURN_DETECTION: usize = 8000;

/// Maximum audio samples for VAD ring buffer (30 seconds at 16kHz).
/// The VAD audio buffer uses a ring buffer strategy: when new audio would exceed
/// this limit, oldest samples are dropped to make room for the most recent audio.
/// This ensures turn detection always has access to recent context while preventing
/// unbounded memory growth during continuous streams.
#[cfg(feature = "stt-vad")]
pub(crate) const MAX_VAD_AUDIO_SAMPLES: usize = 480_000;

/// Get current time in milliseconds since Unix epoch.
pub(crate) fn get_current_time_ms() -> usize {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as usize
}
