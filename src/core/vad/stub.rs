//! Stub implementation when stt-vad feature is disabled.

use std::path::Path;

use anyhow::Result;

use super::config::SileroVADConfig;

/// Stub implementation of Silero VAD when the feature is disabled.
///
/// This provides a no-op implementation that always indicates no speech,
/// allowing code that uses VAD to compile without the feature enabled.
pub struct SileroVAD {
    config: SileroVADConfig,
}

impl SileroVAD {
    /// Create a new stub VAD detector.
    ///
    /// This is a no-op that returns immediately without loading any model.
    pub async fn new(model_path: Option<&Path>) -> Result<Self> {
        let config = SileroVADConfig {
            model_path: model_path.map(Path::to_path_buf),
            ..Default::default()
        };

        Self::with_config(config).await
    }

    /// Create a new stub VAD detector with the given configuration.
    pub async fn with_config(config: SileroVADConfig) -> Result<Self> {
        Ok(Self { config })
    }

    /// Process an audio frame and return the speech probability.
    ///
    /// Always returns 0.0 (no speech) when VAD is disabled.
    pub fn process_audio(&mut self, _audio: &[i16]) -> f32 {
        0.0
    }

    /// Check if the given audio frame contains speech.
    ///
    /// Always returns false when VAD is disabled.
    pub fn is_speech(&mut self, _audio: &[i16]) -> bool {
        false
    }

    /// Reset the internal state of the VAD.
    ///
    /// No-op for stub implementation.
    pub fn reset(&mut self) {}

    /// Get the current speech probability threshold.
    pub fn get_threshold(&self) -> f32 {
        self.config.threshold
    }

    /// Set the speech probability threshold.
    pub fn set_threshold(&mut self, threshold: f32) {
        self.config.threshold = threshold.clamp(0.0, 1.0);
    }

    /// Get the current configuration.
    pub fn get_config(&self) -> &SileroVADConfig {
        &self.config
    }
}

/// Create a stub VAD detector.
pub async fn create_silero_vad(model_path: Option<&Path>) -> Result<SileroVAD> {
    SileroVAD::new(model_path).await
}
