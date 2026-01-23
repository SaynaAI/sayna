//! Silero VAD detector implementation.
//!
//! This module provides the main VAD detector that uses the Silero VAD model
//! for voice activity detection.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use tracing::{debug, info};

use super::config::SileroVADConfig;
use super::model_manager::VADModelManager;

/// Silero Voice Activity Detector.
///
/// Processes audio frames and returns speech probability scores.
/// Maintains internal LSTM state for temporal awareness.
///
/// This detector uses the Silero VAD ONNX model to detect speech in audio frames.
/// It is thread-safe and can be shared across async tasks.
///
/// # Example
///
/// ```ignore
/// use sayna::core::vad::{SileroVAD, SileroVADBuilder};
///
/// // Using the builder pattern
/// let vad = SileroVADBuilder::new()
///     .threshold(0.6)
///     .silence_duration_ms(400)
///     .build()
///     .await?;
///
/// // Process audio frames
/// let audio_frame: &[i16] = &[0i16; 512]; // 512 samples for 16kHz
/// let probability = vad.process_audio(audio_frame).await?;
/// let is_speech = vad.is_speech(audio_frame).await?;
/// ```
pub struct SileroVAD {
    model: Arc<Mutex<VADModelManager>>,
    config: SileroVADConfig,
}

impl SileroVAD {
    /// Create a new SileroVAD with optional model path.
    ///
    /// If `model_path` is provided, loads the model from that path.
    /// Otherwise, uses the default model URL or cached model.
    ///
    /// # Arguments
    ///
    /// * `model_path` - Optional path to a local ONNX model file
    ///
    /// # Errors
    ///
    /// Returns an error if the model cannot be loaded.
    pub async fn new(model_path: Option<&Path>) -> Result<Self> {
        let config = SileroVADConfig {
            model_path: model_path.map(Path::to_path_buf),
            ..Default::default()
        };

        Self::with_config(config).await
    }

    /// Create a new SileroVAD with full configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration for the VAD detector
    ///
    /// # Errors
    ///
    /// Returns an error if the model cannot be loaded.
    pub async fn with_config(config: SileroVADConfig) -> Result<Self> {
        info!("Initializing Silero-VAD with config: {:?}", config);

        let model = Arc::new(Mutex::new(
            VADModelManager::new(config.clone())
                .await
                .context("Failed to initialize VAD model manager")?,
        ));

        Ok(Self { model, config })
    }

    /// Test-only: Create a SileroVAD stub without loading a real ONNX model.
    ///
    /// This constructor creates a `SileroVAD` instance that can be used in tests
    /// to enter the VAD processing path without requiring the actual ONNX model.
    /// The returned instance should only be used with `VADState::set_force_error(true)`
    /// which causes `process_audio_chunk` to return an error before any model access.
    ///
    /// # Usage
    ///
    /// ```ignore
    /// let vad = SileroVAD::stub_for_testing(SileroVADConfig::default());
    /// let vad_state = Arc::new(VADState::new(16000 * 8, 512));
    /// vad_state.set_force_error(true); // Required!
    /// ```
    ///
    /// # Note
    ///
    /// Calling `process_audio` on a stub instance will return an error, not panic,
    /// because the underlying model manager has no ONNX session.
    #[cfg(test)]
    pub fn stub_for_testing(config: SileroVADConfig) -> Self {
        Self {
            model: Arc::new(Mutex::new(VADModelManager::new_stub_for_testing(
                config.clone(),
            ))),
            config,
        }
    }

    /// Process an audio frame and return speech probability.
    ///
    /// This method runs the CPU-bound ONNX inference in a blocking thread
    /// via `spawn_blocking` to avoid blocking the Tokio runtime.
    ///
    /// # Arguments
    ///
    /// * `audio` - Audio samples as i16, must be exactly frame_size samples
    ///   (512 for 16kHz, 256 for 8kHz)
    ///
    /// # Returns
    ///
    /// * `f32` - Speech probability between 0.0 and 1.0
    ///   - Values > threshold indicate speech
    ///   - Values <= threshold indicate silence/non-speech
    ///
    /// # Errors
    ///
    /// Returns an error if the audio frame size is incorrect or inference fails.
    pub async fn process_audio(&self, audio: &[i16]) -> Result<f32> {
        let model = Arc::clone(&self.model);
        let audio = audio.to_vec();

        tokio::task::spawn_blocking(move || {
            let mut model = model.lock();
            model.process_audio(&audio)
        })
        .await
        .context("VAD inference task was cancelled")?
    }

    /// Check if the given audio frame contains speech.
    ///
    /// # Arguments
    ///
    /// * `audio` - Audio samples as i16
    ///
    /// # Returns
    ///
    /// * `bool` - true if speech detected (probability > threshold), false otherwise
    ///
    /// # Errors
    ///
    /// Returns an error if audio processing fails.
    pub async fn is_speech(&self, audio: &[i16]) -> Result<bool> {
        let prob = self.process_audio(audio).await?;
        Ok(prob > self.config.threshold)
    }

    /// Reset the internal state.
    ///
    /// Call this when starting a new audio stream or after a long pause.
    /// This clears the LSTM state and context buffer.
    pub async fn reset(&self) {
        let model = Arc::clone(&self.model);
        // Reset is fast, but we still use spawn_blocking for consistency
        // and to avoid holding the lock across await points
        let _ = tokio::task::spawn_blocking(move || {
            let mut model = model.lock();
            model.reset();
            debug!("Silero-VAD state reset");
        })
        .await;
    }

    /// Get the configured speech threshold.
    pub fn get_threshold(&self) -> f32 {
        self.config.threshold
    }

    /// Update the speech threshold.
    ///
    /// The threshold is clamped to the range [0.0, 1.0].
    pub fn set_threshold(&mut self, threshold: f32) {
        self.config.threshold = threshold.clamp(0.0, 1.0);
    }

    /// Get the expected frame size in samples.
    ///
    /// This is the number of audio samples that must be passed to `process_audio`:
    /// - 512 samples for 16kHz sample rate
    /// - 256 samples for 8kHz sample rate
    pub fn frame_size(&self) -> usize {
        self.config.sample_rate.frame_size()
    }

    /// Get the configured sample rate in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate.as_hz()
    }

    /// Get the configured silence duration in milliseconds.
    ///
    /// This is the minimum duration of silence required to consider
    /// a speech segment complete.
    pub fn silence_duration_ms(&self) -> u64 {
        self.config.silence_duration_ms
    }

    /// Get the full configuration.
    pub fn get_config(&self) -> &SileroVADConfig {
        &self.config
    }

    /// Get a clone of the internal model manager Arc for lock-free inference.
    ///
    /// This allows callers to perform VAD inference without holding an external
    /// lock across the await point. The returned Arc can be used with
    /// [`Self::process_audio_with_model`].
    pub fn get_model(&self) -> Arc<Mutex<VADModelManager>> {
        Arc::clone(&self.model)
    }

    /// Process audio using a pre-cloned model reference.
    ///
    /// This is useful when you need to avoid holding an async lock across the
    /// await point. First call [`Self::get_model`] while holding the lock,
    /// release the lock, then call this method.
    ///
    /// # Arguments
    ///
    /// * `model` - Arc clone of the VADModelManager from [`Self::get_model`]
    /// * `audio` - Audio samples as i16
    ///
    /// # Returns
    ///
    /// * `f32` - Speech probability between 0.0 and 1.0
    pub async fn process_audio_with_model(
        model: Arc<Mutex<VADModelManager>>,
        audio: &[i16],
    ) -> anyhow::Result<f32> {
        let audio = audio.to_vec();

        tokio::task::spawn_blocking(move || {
            let mut model = model.lock();
            model.process_audio(&audio)
        })
        .await
        .context("VAD inference task was cancelled")?
    }
}

/// Builder for SileroVAD with fluent configuration API.
///
/// Provides a convenient way to configure and create a `SileroVAD` instance.
///
/// # Example
///
/// ```ignore
/// let vad = SileroVADBuilder::new()
///     .threshold(0.6)
///     .silence_duration_ms(400)
///     .min_speech_duration_ms(150)
///     .num_threads(2)
///     .build()
///     .await?;
/// ```
pub struct SileroVADBuilder {
    config: SileroVADConfig,
}

impl Default for SileroVADBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SileroVADBuilder {
    /// Create a new builder with default configuration.
    pub fn new() -> Self {
        Self {
            config: SileroVADConfig::default(),
        }
    }

    /// Set the speech threshold (0.0 to 1.0).
    ///
    /// Audio frames with probability above this threshold are considered speech.
    /// Lower values increase sensitivity (more speech detection, more false positives).
    /// Higher values decrease sensitivity (miss quiet speech, fewer false positives).
    ///
    /// Default: 0.5
    pub fn threshold(mut self, threshold: f32) -> Self {
        self.config.threshold = threshold.clamp(0.0, 1.0);
        self
    }

    /// Set the silence duration in milliseconds.
    ///
    /// This is the minimum duration of continuous silence required to consider
    /// a speech segment complete.
    ///
    /// Default: 300ms
    pub fn silence_duration_ms(mut self, ms: u64) -> Self {
        self.config.silence_duration_ms = ms;
        self
    }

    /// Set the minimum speech duration in milliseconds.
    ///
    /// Prevents false triggers on brief pauses within speech. The system won't
    /// start tracking silence until at least this much speech has been detected.
    ///
    /// Default: 100ms
    pub fn min_speech_duration_ms(mut self, ms: u64) -> Self {
        self.config.min_speech_duration_ms = ms;
        self
    }

    /// Set the model path.
    ///
    /// If specified, the model will be loaded from this path instead of
    /// downloading from the model URL.
    pub fn model_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.model_path = Some(path.into());
        self
    }

    /// Set the model URL.
    ///
    /// The model will be downloaded from this URL if not available locally.
    pub fn model_url(mut self, url: impl Into<String>) -> Self {
        self.config.model_url = Some(url.into());
        self
    }

    /// Set the cache path.
    ///
    /// Downloaded models are stored in a `vad` subdirectory within this path.
    pub fn cache_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.cache_path = Some(path.into());
        self
    }

    /// Set the number of threads for ONNX inference.
    ///
    /// Since VAD inference is lightweight, a single thread is typically sufficient.
    ///
    /// Default: 1
    pub fn num_threads(mut self, threads: usize) -> Self {
        self.config.num_threads = Some(threads);
        self
    }

    /// Build the SileroVAD instance.
    ///
    /// # Errors
    ///
    /// Returns an error if the model cannot be loaded.
    pub async fn build(self) -> Result<SileroVAD> {
        SileroVAD::with_config(self.config).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_pattern() {
        let builder = SileroVADBuilder::new()
            .threshold(0.6)
            .silence_duration_ms(400)
            .min_speech_duration_ms(150)
            .num_threads(2);

        assert_eq!(builder.config.threshold, 0.6);
        assert_eq!(builder.config.silence_duration_ms, 400);
        assert_eq!(builder.config.min_speech_duration_ms, 150);
        assert_eq!(builder.config.num_threads, Some(2));
    }

    #[test]
    fn test_builder_threshold_clamping() {
        // Test clamping above 1.0
        let builder = SileroVADBuilder::new().threshold(1.5);
        assert_eq!(builder.config.threshold, 1.0);

        // Test clamping below 0.0
        let builder = SileroVADBuilder::new().threshold(-0.5);
        assert_eq!(builder.config.threshold, 0.0);

        // Test normal value
        let builder = SileroVADBuilder::new().threshold(0.75);
        assert_eq!(builder.config.threshold, 0.75);
    }

    #[test]
    fn test_builder_default() {
        let builder = SileroVADBuilder::default();
        assert_eq!(builder.config.threshold, 0.5); // Default Silero-VAD recommendation
        assert_eq!(builder.config.silence_duration_ms, 500); // 500ms for longer conversations
        assert_eq!(builder.config.min_speech_duration_ms, 250); // Filter brief filler sounds
    }

    #[test]
    fn test_builder_model_path() {
        let builder = SileroVADBuilder::new().model_path("/path/to/model.onnx");
        assert_eq!(
            builder.config.model_path,
            Some(PathBuf::from("/path/to/model.onnx"))
        );
    }

    #[test]
    fn test_builder_cache_path() {
        let builder = SileroVADBuilder::new().cache_path("/tmp/vad_cache");
        assert_eq!(
            builder.config.cache_path,
            Some(PathBuf::from("/tmp/vad_cache"))
        );
    }

    #[test]
    fn test_builder_model_url() {
        let builder = SileroVADBuilder::new().model_url("https://example.com/model.onnx");
        assert_eq!(
            builder.config.model_url,
            Some("https://example.com/model.onnx".to_string())
        );
    }
}
