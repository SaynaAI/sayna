//! Utility functions for the VoiceManager module.

/// Minimum audio samples required for turn detection (1.0 second at 16kHz).
///
/// Increased from 0.5s to 1.0s to prevent turn detection from running on very
/// short audio clips that are mostly zero-padding, which can cause false positives
/// on brief pauses like inhaling or filler sounds ("um", "uh").
#[cfg(feature = "stt-vad")]
pub(crate) const MIN_AUDIO_SAMPLES_FOR_TURN_DETECTION: usize = 16_000;

/// Maximum audio samples for VAD ring buffer (8 seconds at 16kHz).
/// The VAD audio buffer uses a ring buffer strategy: when new audio would exceed
/// this limit, oldest samples are dropped to make room for the most recent audio.
/// This ensures turn detection always has access to recent context while preventing
/// unbounded memory growth during continuous streams.
///
/// 8 seconds (128,000 samples) matches the Smart-Turn model's expected input duration,
/// providing full context for accurate turn detection. The model was trained on up to
/// 8 seconds of audio, so using the full window improves accuracy for distinguishing
/// brief pauses from actual turn endings.
#[cfg(feature = "stt-vad")]
pub(crate) const MAX_VAD_AUDIO_SAMPLES: usize = 128_000;

/// Get current time in milliseconds since Unix epoch.
pub(crate) fn get_current_time_ms() -> usize {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as usize
}
