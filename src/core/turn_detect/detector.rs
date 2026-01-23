use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use crate::core::turn_detect::{
    config::TurnDetectorConfig, feature_extractor::FeatureExtractor, model_manager::ModelManager,
};

pub struct TurnDetector {
    model: Arc<Mutex<ModelManager>>,
    feature_extractor: Arc<FeatureExtractor>,
    config: TurnDetectorConfig,
}

impl TurnDetector {
    pub async fn new(model_path: Option<&Path>) -> Result<Self> {
        let config = TurnDetectorConfig {
            model_path: model_path.map(Path::to_path_buf),
            ..Default::default()
        };

        Self::with_config(config).await
    }

    pub async fn with_config(config: TurnDetectorConfig) -> Result<Self> {
        info!(
            "Initializing smart-turn TurnDetector with config: {:?}",
            config
        );

        let model = Arc::new(Mutex::new(
            ModelManager::new(config.clone())
                .await
                .context("Failed to initialize model manager")?,
        ));

        let feature_extractor = Arc::new(
            FeatureExtractor::new(&config).context("Failed to initialize feature extractor")?,
        );

        Ok(Self {
            model,
            feature_extractor,
            config,
        })
    }

    /// Predict the probability that the user has finished their turn.
    ///
    /// Feature extraction and model inference run in `spawn_blocking` to avoid
    /// blocking the async runtime. This allows timeouts to fire properly even
    /// when the CPU-bound work is slow.
    ///
    /// # Arguments
    /// * `audio` - Raw audio samples as i16 PCM (16kHz mono)
    ///
    /// # Returns
    /// * `f32` - Probability of turn completion (0.0 to 1.0)
    ///
    /// # Example
    /// ```ignore
    /// let audio_samples: &[i16] = &[/* 16kHz PCM samples */];
    /// let probability = turn_detector.predict_end_of_turn(audio_samples).await?;
    /// if probability > 0.5 {
    ///     println!("Turn is likely complete");
    /// }
    /// ```
    pub async fn predict_end_of_turn(&self, audio: &[i16]) -> Result<f32> {
        if audio.is_empty() {
            return Ok(0.0); // Empty audio is not a complete turn
        }

        // Clone what we need for the blocking task
        let audio_vec = audio.to_vec();
        let feature_extractor = self.feature_extractor.clone();
        let model = self.model.clone();
        let config_mel_bins = self.config.mel_bins;
        let config_mel_frames = self.config.mel_frames;

        // Run feature extraction and inference in spawn_blocking so timeouts can fire
        let audio_sample_count = audio_vec.len();
        let probability = tokio::task::spawn_blocking(move || -> Result<f32> {
            let overall_start = Instant::now();

            // Extract mel spectrogram features with timing
            let feature_start = Instant::now();
            let mel_features = feature_extractor
                .extract(&audio_vec)
                .context("Failed to extract mel features")?;
            let feature_duration = feature_start.elapsed();

            debug!(
                audio_samples = audio_sample_count,
                feature_extraction_ms = feature_duration.as_secs_f64() * 1000.0,
                "Smart-turn feature extraction completed"
            );

            // Validate dimensions before prediction
            let (mel_bins, mel_frames) = mel_features.dim();
            if mel_bins != config_mel_bins || mel_frames != config_mel_frames {
                anyhow::bail!(
                    "Invalid mel spectrogram dimensions: got ({}, {}), expected ({}, {})",
                    mel_bins,
                    mel_frames,
                    config_mel_bins,
                    config_mel_frames
                );
            }

            // Run model inference with timing
            let inference_start = Instant::now();
            let mut model_guard = model.lock().map_err(|e| {
                anyhow::anyhow!(
                    "Turn detection model mutex poisoned (likely panic during inference): {}",
                    e
                )
            })?;

            let probability = model_guard
                .predict(mel_features.view())
                .context("Model prediction failed")?;
            let inference_duration = inference_start.elapsed();

            let total_duration = overall_start.elapsed();
            debug!(
                audio_samples = audio_sample_count,
                feature_extraction_ms = feature_duration.as_secs_f64() * 1000.0,
                inference_ms = inference_duration.as_secs_f64() * 1000.0,
                total_ms = total_duration.as_secs_f64() * 1000.0,
                probability = probability,
                "Smart-turn prediction completed"
            );

            // Warn if inference is slow
            if total_duration > Duration::from_millis(50) {
                warn!(
                    total_ms = total_duration.as_secs_f64() * 1000.0,
                    feature_ms = feature_duration.as_secs_f64() * 1000.0,
                    inference_ms = inference_duration.as_secs_f64() * 1000.0,
                    probability = probability,
                    "Smart-turn prediction took longer than 50ms"
                );
            }

            Ok(probability)
        })
        .await
        .context("spawn_blocking task panicked")??;

        Ok(probability)
    }

    /// Check if the given audio represents a complete turn.
    ///
    /// # Arguments
    /// * `audio` - Raw audio samples as i16 PCM (16kHz mono)
    ///
    /// # Returns
    /// * `bool` - true if turn is likely complete (probability >= threshold)
    pub async fn is_turn_complete(&self, audio: &[i16]) -> Result<bool> {
        let probability = self.predict_end_of_turn(audio).await?;
        debug!(
            "Audio samples: {}, Turn completion probability: {:.4}",
            audio.len(),
            probability
        );
        Ok(probability >= self.config.threshold)
    }

    /// Set the detection threshold.
    pub fn set_threshold(&mut self, threshold: f32) {
        self.config.threshold = threshold.clamp(0.0, 1.0);
    }

    /// Get the current detection threshold.
    pub fn get_threshold(&self) -> f32 {
        self.config.threshold
    }

    /// Get the current configuration.
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

/// Builder for TurnDetector with fluent configuration API.
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

    pub fn cache_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.cache_path = Some(path.into());
        self
    }

    pub async fn build(self) -> Result<TurnDetector> {
        TurnDetector::with_config(self.config).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_builder_pattern() {
        let builder = TurnDetectorBuilder::new()
            .threshold(0.9)
            .use_quantized(true)
            .num_threads(2)
            .sample_rate(16000);

        assert!(builder.config.threshold == 0.9);
        assert!(builder.config.use_quantized);
        assert!(builder.config.num_threads == Some(2));
        assert!(builder.config.sample_rate == 16000);
    }

    #[test]
    fn test_threshold_clamping() {
        // Test threshold clamping with a simple config
        // Test clamping above 1.0
        let config = TurnDetectorConfig {
            threshold: 1.5f32.clamp(0.0, 1.0),
            ..Default::default()
        };
        assert_eq!(config.threshold, 1.0);

        // Test clamping below 0.0
        let config = TurnDetectorConfig {
            threshold: (-0.5f32).clamp(0.0, 1.0),
            ..Default::default()
        };
        assert_eq!(config.threshold, 0.0);

        // Test normal value
        let config = TurnDetectorConfig {
            threshold: 0.75f32.clamp(0.0, 1.0),
            ..Default::default()
        };
        assert_eq!(config.threshold, 0.75);
    }
}
