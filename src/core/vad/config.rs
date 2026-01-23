//! Configuration structures for Silero VAD.
//!
//! This module provides configuration types for the Silero Voice Activity Detection
//! system, including sample rate settings, inference parameters, and silence detection
//! thresholds.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Supported sample rates for Silero-VAD.
///
/// Silero-VAD only supports 8kHz and 16kHz sample rates. The frame size and
/// context window size depend on the selected sample rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VADSampleRate {
    /// 8000 Hz - uses 256 sample frames (32ms chunks)
    #[serde(rename = "8000")]
    Rate8kHz,
    /// 16000 Hz - uses 512 sample frames (32ms chunks) - recommended
    #[serde(rename = "16000")]
    #[default]
    Rate16kHz,
}

impl VADSampleRate {
    /// Get the sample rate in Hz.
    pub fn as_hz(&self) -> u32 {
        match self {
            Self::Rate8kHz => 8000,
            Self::Rate16kHz => 16000,
        }
    }

    /// Get the frame size in samples for this sample rate.
    ///
    /// Silero-VAD processes audio in fixed-size frames:
    /// - 8kHz: 256 samples per frame (32ms)
    /// - 16kHz: 512 samples per frame (32ms)
    pub fn frame_size(&self) -> usize {
        match self {
            Self::Rate8kHz => 256,
            Self::Rate16kHz => 512,
        }
    }

    /// Get the context size in samples for this sample rate.
    ///
    /// Context is used for overlapping audio segments:
    /// - 8kHz: 32 samples context
    /// - 16kHz: 64 samples context
    pub fn context_size(&self) -> usize {
        match self {
            Self::Rate8kHz => 32,
            Self::Rate16kHz => 64,
        }
    }

    /// Get the frame duration in milliseconds.
    ///
    /// Both sample rates produce 32ms frames.
    pub fn frame_duration_ms(&self) -> u64 {
        32
    }
}

impl From<u32> for VADSampleRate {
    fn from(hz: u32) -> Self {
        match hz {
            8000 => Self::Rate8kHz,
            _ => Self::Rate16kHz, // Default to 16kHz for unsupported rates
        }
    }
}

/// Configuration for Silero-VAD detector.
///
/// This configuration controls the Silero VAD ONNX model behavior, including
/// speech detection thresholds, inference settings, and model loading options.
///
/// # Model Specifications
///
/// The Silero-VAD ONNX model has the following tensor specifications:
/// - **Input tensors:**
///   - `input`: Audio frame shape `[1, 512]` for 16kHz (or `[1, 256]` for 8kHz)
///   - `state`: RNN/LSTM state shape `[2, 1, 128]`
///   - `sr`: Sample rate as int64
/// - **Output tensors:**
///   - `output`: Speech probability (0.0-1.0)
///   - `stateN`: Updated state `[2, 1, 128]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SileroVADConfig {
    /// Speech probability threshold (0.0 to 1.0).
    ///
    /// Audio frames with probability above this threshold are considered speech.
    /// The default value of 0.5 is the recommendation from Silero-VAD.
    ///
    /// Lower values increase sensitivity (detect more speech, more false positives).
    /// Higher values decrease sensitivity (miss quiet speech, fewer false positives).
    pub threshold: f32,

    /// Sample rate for VAD processing.
    ///
    /// Silero-VAD only supports 8kHz and 16kHz. Audio at other sample rates
    /// must be resampled before processing.
    pub sample_rate: VADSampleRate,

    /// Minimum silence duration in milliseconds to trigger turn end.
    ///
    /// After detecting speech, this is how long continuous silence must be
    /// observed before considering the speech segment complete. Default: 300ms.
    pub silence_duration_ms: u64,

    /// Minimum speech duration in milliseconds before considering silence.
    ///
    /// Prevents false triggers on brief pauses within speech. The system won't
    /// start tracking silence until at least this much speech has been detected.
    /// Default: 100ms.
    pub min_speech_duration_ms: u64,

    /// Path to a local ONNX model file.
    ///
    /// If specified, the model will be loaded from this path instead of
    /// downloading from `model_url`. Takes precedence over URL-based loading.
    pub model_path: Option<PathBuf>,

    /// URL to download the model from if not available locally.
    ///
    /// The model will be cached in `cache_path` after download. Default uses
    /// the HuggingFace mirror of the Silero-VAD model.
    pub model_url: Option<String>,

    /// Directory for caching downloaded models.
    ///
    /// Downloaded models are stored in a `vad` subdirectory within this path.
    /// If not specified, model downloading will fail.
    pub cache_path: Option<PathBuf>,

    /// Number of CPU threads for ONNX inference.
    ///
    /// Since VAD inference is lightweight, a single thread is typically sufficient.
    /// Default: 1.
    pub num_threads: Option<usize>,

    /// ONNX graph optimization level.
    ///
    /// Higher optimization levels may improve inference speed but increase
    /// model loading time. Default: Level3 (maximum optimization).
    pub graph_optimization_level: GraphOptimizationLevel,
}

impl Default for SileroVADConfig {
    fn default() -> Self {
        Self {
            threshold: 0.5, // Default Silero-VAD recommendation
            sample_rate: VADSampleRate::Rate16kHz,
            // Increased from PipeCat's 200ms to 500ms for longer conversations.
            // Natural speech contains pauses of 200-400ms (breaths, thinking) that
            // shouldn't trigger premature turn detection. 500ms of true silence
            // strongly indicates the speaker has finished their turn.
            silence_duration_ms: 500,
            // Increased from 100ms to 250ms to filter out brief filler sounds like "um", "mmm"
            // that shouldn't trigger turn detection without meaningful speech content.
            min_speech_duration_ms: 250,
            model_path: None,
            model_url: Some(
                "https://huggingface.co/onnx-community/silero-vad/resolve/main/onnx/model.onnx"
                    .to_string(),
            ),
            cache_path: None,
            num_threads: Some(1), // Single thread is sufficient for VAD
            graph_optimization_level: GraphOptimizationLevel::Level3,
        }
    }
}

impl SileroVADConfig {
    /// Create a new configuration with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new configuration with a custom threshold.
    pub fn with_threshold(threshold: f32) -> Self {
        Self {
            threshold,
            ..Self::default()
        }
    }

    /// Get the cache directory for VAD models.
    ///
    /// Returns the path where downloaded models should be cached.
    /// This is the `cache_path` with a `vad` subdirectory appended.
    ///
    /// # Errors
    ///
    /// Returns an error if no cache directory is specified in the configuration.
    pub fn get_cache_dir(&self) -> Result<PathBuf> {
        let cache_dir = if let Some(cache_path) = &self.cache_path {
            cache_path.join("vad")
        } else {
            anyhow::bail!("No cache directory specified");
        };
        Ok(cache_dir)
    }

    /// Get the frame size in samples for the configured sample rate.
    pub fn frame_size(&self) -> usize {
        self.sample_rate.frame_size()
    }

    /// Get the context size in samples for the configured sample rate.
    pub fn context_size(&self) -> usize {
        self.sample_rate.context_size()
    }

    /// Set the cache path for model storage.
    pub fn set_cache_path(mut self, cache_path: PathBuf) -> Self {
        self.cache_path = Some(cache_path);
        self
    }

    /// Set the model path for local model loading.
    pub fn set_model_path(mut self, model_path: PathBuf) -> Self {
        self.model_path = Some(model_path);
        self
    }

    /// Set the silence duration threshold in milliseconds.
    pub fn set_silence_duration_ms(mut self, duration_ms: u64) -> Self {
        self.silence_duration_ms = duration_ms;
        self
    }

    /// Set the speech detection threshold.
    pub fn set_threshold(mut self, threshold: f32) -> Self {
        self.threshold = threshold;
        self
    }
}

/// ONNX graph optimization level.
///
/// Controls the level of graph optimization applied during model loading.
/// Higher levels provide better inference performance but may increase
/// model loading time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GraphOptimizationLevel {
    /// No optimization (fastest load, slowest inference)
    Disabled,
    /// Basic optimizations (constant folding, redundant node elimination)
    Basic,
    /// Extended optimizations (includes node fusions)
    Extended,
    /// Maximum optimization level (recommended for production)
    #[default]
    Level3,
}

#[cfg(feature = "stt-vad")]
impl GraphOptimizationLevel {
    /// Convert to ORT's graph optimization level.
    pub fn to_ort_level(&self) -> ort::session::builder::GraphOptimizationLevel {
        use ort::session::builder::GraphOptimizationLevel as OrtLevel;
        match self {
            Self::Disabled => OrtLevel::Disable,
            Self::Basic => OrtLevel::Level1,
            Self::Extended => OrtLevel::Level2,
            Self::Level3 => OrtLevel::Level3,
        }
    }
}

/// Configuration for VAD-based silence detection.
///
/// This configuration can be used with `VoiceManagerConfig` to configure
/// VAD-based turn detection as an alternative or supplement to
/// STT provider-based speech final detection.
///
/// # Note on runtime behavior
///
/// When the `stt-vad` feature is compiled, VAD is **always active** at runtime.
/// The `enabled` field is retained for configuration structure compatibility
/// but does not control whether VAD runs - VAD processing occurs automatically
/// whenever the feature is compiled and the VAD model is successfully initialized.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VADSilenceConfig {
    /// Whether VAD silence detection is requested.
    ///
    /// **Note:** When the `stt-vad` feature is compiled, VAD is always active
    /// regardless of this field. This field is retained for configuration
    /// structure compatibility and may be used by external systems to track
    /// intent, but it does not gate VAD processing at runtime.
    ///
    /// Default: false (for backward compatibility).
    pub enabled: bool,

    /// Silero-VAD configuration.
    pub silero_config: SileroVADConfig,
}

impl VADSilenceConfig {
    /// Create a new VAD silence configuration with VAD disabled.
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Create a new VAD silence configuration with VAD enabled and default settings.
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            silero_config: SileroVADConfig::default(),
        }
    }

    /// Create a new VAD silence configuration with custom Silero config.
    pub fn with_config(silero_config: SileroVADConfig) -> Self {
        Self {
            enabled: true,
            silero_config,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vad_sample_rate_values() {
        assert_eq!(VADSampleRate::Rate8kHz.as_hz(), 8000);
        assert_eq!(VADSampleRate::Rate16kHz.as_hz(), 16000);
    }

    #[test]
    fn test_vad_sample_rate_frame_size() {
        assert_eq!(VADSampleRate::Rate8kHz.frame_size(), 256);
        assert_eq!(VADSampleRate::Rate16kHz.frame_size(), 512);
    }

    #[test]
    fn test_vad_sample_rate_context_size() {
        assert_eq!(VADSampleRate::Rate8kHz.context_size(), 32);
        assert_eq!(VADSampleRate::Rate16kHz.context_size(), 64);
    }

    #[test]
    fn test_vad_sample_rate_default() {
        assert_eq!(VADSampleRate::default(), VADSampleRate::Rate16kHz);
    }

    #[test]
    fn test_vad_sample_rate_from_u32() {
        assert_eq!(VADSampleRate::from(8000), VADSampleRate::Rate8kHz);
        assert_eq!(VADSampleRate::from(16000), VADSampleRate::Rate16kHz);
        // Unknown rates default to 16kHz
        assert_eq!(VADSampleRate::from(44100), VADSampleRate::Rate16kHz);
    }

    #[test]
    fn test_silero_vad_config_defaults() {
        let config = SileroVADConfig::default();
        assert_eq!(config.threshold, 0.5);
        assert_eq!(config.sample_rate, VADSampleRate::Rate16kHz);
        // Increased to 500ms for longer conversations (was 200ms)
        assert_eq!(config.silence_duration_ms, 500);
        assert_eq!(config.min_speech_duration_ms, 250); // Increased to filter filler sounds
        assert_eq!(config.num_threads, Some(1));
        assert!(config.model_url.is_some());
        assert!(config.model_path.is_none());
    }

    #[test]
    fn test_silero_vad_config_builder_methods() {
        let config = SileroVADConfig::new()
            .set_threshold(0.7)
            .set_silence_duration_ms(500)
            .set_cache_path(PathBuf::from("/tmp/cache"));

        assert_eq!(config.threshold, 0.7);
        assert_eq!(config.silence_duration_ms, 500);
        assert_eq!(config.cache_path, Some(PathBuf::from("/tmp/cache")));
    }

    #[test]
    fn test_silero_vad_config_frame_size() {
        let config_16k = SileroVADConfig::default();
        assert_eq!(config_16k.frame_size(), 512);

        let config_8k = SileroVADConfig {
            sample_rate: VADSampleRate::Rate8kHz,
            ..Default::default()
        };
        assert_eq!(config_8k.frame_size(), 256);
    }

    #[test]
    fn test_get_cache_dir_with_path() {
        let config = SileroVADConfig::default().set_cache_path(PathBuf::from("/tmp/models"));
        let cache_dir = config.get_cache_dir().unwrap();
        assert_eq!(cache_dir, PathBuf::from("/tmp/models/vad"));
    }

    #[test]
    fn test_get_cache_dir_without_path() {
        let config = SileroVADConfig::default();
        assert!(config.get_cache_dir().is_err());
    }

    #[test]
    fn test_vad_silence_config_defaults() {
        let config = VADSilenceConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.silero_config.threshold, 0.5);
    }

    #[test]
    fn test_vad_silence_config_enabled() {
        let config = VADSilenceConfig::enabled();
        assert!(config.enabled);
    }

    #[test]
    fn test_graph_optimization_level_default() {
        assert_eq!(
            GraphOptimizationLevel::default(),
            GraphOptimizationLevel::Level3
        );
    }

    #[test]
    fn test_serialization() {
        let config = SileroVADConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: SileroVADConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.threshold, deserialized.threshold);
        assert_eq!(config.sample_rate, deserialized.sample_rate);
    }

    #[test]
    fn test_sample_rate_serialization() {
        // Test that sample rates serialize as numbers
        let rate = VADSampleRate::Rate16kHz;
        let json = serde_json::to_string(&rate).unwrap();
        assert_eq!(json, "\"16000\"");

        let rate = VADSampleRate::Rate8kHz;
        let json = serde_json::to_string(&rate).unwrap();
        assert_eq!(json, "\"8000\"");

        // Test deserialization
        let deserialized: VADSampleRate = serde_json::from_str("\"16000\"").unwrap();
        assert_eq!(deserialized, VADSampleRate::Rate16kHz);
    }
}
