use std::path::{Path, PathBuf};

use anyhow::Result;

use super::config::TurnDetectorConfig;

/// No-op placeholder used when the `turn-detect` feature is disabled.
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
    pub async fn predict_end_of_turn(&self, _user_input: &str) -> Result<f32> {
        Ok(0.0)
    }

    /// Always returns `false`, indicating the turn is not complete.
    pub async fn is_turn_complete(&self, _user_input: &str) -> Result<bool> {
        Ok(false)
    }

    /// Ignores streaming text and returns `false` without waiting.
    pub async fn process_streaming_text(
        &self,
        _partial_text: &str,
        _check_interval_ms: u64,
    ) -> Result<bool> {
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

    pub fn max_sequence_length(mut self, length: usize) -> Self {
        self.config.max_sequence_length = length;
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

    pub fn tokenizer_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.tokenizer_path = Some(path.into());
        self
    }

    pub fn tokenizer_url(mut self, url: impl Into<String>) -> Self {
        self.config.tokenizer_url = Some(url.into());
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

    pub async fn build(self) -> Result<TurnDetector> {
        TurnDetector::with_config(self.config).await
    }
}

pub async fn create_turn_detector(model_path: Option<&Path>) -> Result<TurnDetector> {
    TurnDetector::new(model_path).await
}
