use std::path::{Path, PathBuf};

use anyhow::Result;

use super::config::TurnDetectorConfig;

/// No-op placeholder used when the `stt-vad` feature is disabled.
pub struct TurnDetector {
    config: TurnDetectorConfig,
}

impl TurnDetector {
    /// Construct a disabled turn detector. Returns immediately without loading models.
    pub async fn new(model_path: Option<&Path>) -> Result<Self> {
        let config = TurnDetectorConfig {
            model_path: model_path.map(Path::to_path_buf),
            ..Default::default()
        };

        Self::with_config(config).await
    }

    /// Accept a config but keeps the detector inert.
    pub async fn with_config(config: TurnDetectorConfig) -> Result<Self> {
        Ok(Self { config })
    }

    /// Always returns `0.0`, indicating no end-of-turn confidence.
    ///
    /// # Arguments
    /// * `audio` - Raw audio samples as i16 PCM (ignored in stub)
    pub async fn predict_end_of_turn(&self, _audio: &[i16]) -> Result<f32> {
        Ok(0.0)
    }

    /// Always returns `false`, indicating the turn is not complete.
    ///
    /// # Arguments
    /// * `audio` - Raw audio samples as i16 PCM (ignored in stub)
    pub async fn is_turn_complete(&self, _audio: &[i16]) -> Result<bool> {
        Ok(false)
    }

    pub fn set_threshold(&mut self, threshold: f32) {
        self.config.threshold = threshold.clamp(0.0, 1.0);
    }

    pub fn get_threshold(&self) -> f32 {
        self.config.threshold
    }

    pub fn get_config(&self) -> &TurnDetectorConfig {
        &self.config
    }

    /// Get the expected sample rate for audio input.
    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }

    /// Get the maximum audio duration in seconds.
    pub fn max_audio_duration_seconds(&self) -> u8 {
        self.config.max_audio_duration_seconds
    }
}

/// Builder that mirrors the real implementation but produces an inert detector.
pub struct TurnDetectorBuilder {
    config: TurnDetectorConfig,
}

impl Default for TurnDetectorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnDetectorBuilder {
    pub fn new() -> Self {
        Self {
            config: TurnDetectorConfig::default(),
        }
    }

    pub fn threshold(mut self, threshold: f32) -> Self {
        self.config.threshold = threshold;
        self
    }

    pub fn model_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.model_path = Some(path.into());
        self
    }

    pub fn model_url(mut self, url: impl Into<String>) -> Self {
        self.config.model_url = Some(url.into());
        self
    }

    pub fn use_quantized(mut self, quantized: bool) -> Self {
        self.config.use_quantized = quantized;
        self
    }

    pub fn num_threads(mut self, threads: usize) -> Self {
        self.config.num_threads = Some(threads);
        self
    }

    pub fn sample_rate(mut self, rate: u32) -> Self {
        self.config.sample_rate = rate;
        self
    }

    pub async fn build(self) -> Result<TurnDetector> {
        TurnDetector::with_config(self.config).await
    }
}

pub async fn create_turn_detector(model_path: Option<&Path>) -> Result<TurnDetector> {
    TurnDetector::new(model_path).await
}
