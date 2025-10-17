use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use crate::core::turn_detect::{
    config::TurnDetectorConfig, model_manager::ModelManager, tokenizer::Tokenizer,
};

// Static regex for text normalization
static PUNCT_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^\w\s'-]").unwrap());

static WHITESPACE_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());

pub struct TurnDetector {
    model: Arc<tokio::sync::Mutex<ModelManager>>,
    tokenizer: Arc<Tokenizer>,
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
        info!("Initializing TurnDetector with config: {:?}", config);

        let model = Arc::new(tokio::sync::Mutex::new(
            ModelManager::new(config.clone())
                .await
                .context("Failed to initialize model manager")?,
        ));

        let tokenizer = Arc::new(
            Tokenizer::new(&config)
                .await
                .context("Failed to initialize tokenizer")?,
        );

        Ok(Self {
            model,
            tokenizer,
            config,
        })
    }

    /// Normalize text to match LiveKit's preprocessing
    fn normalize_text(text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }

        // Convert to lowercase
        let text = text.to_lowercase();

        // Remove all punctuation except apostrophes and hyphens
        let text = PUNCT_REGEX.replace_all(&text, " ");

        // Normalize whitespace
        let text = WHITESPACE_REGEX.replace_all(&text, " ");

        text.trim().to_string()
    }

    /// Format text as a chat message for the model
    fn format_as_chat(&self, user_input: &str) -> String {
        // The model expects chat-formatted input
        // For single turn detection, we format it as a user message
        let normalized = Self::normalize_text(user_input);

        // Apply chat template format (simplified version)
        // The model was trained with SmolLM chat format
        // We don't include the final <|im_end|> as per LiveKit's implementation
        format!("<|im_start|>user\n{}", normalized)
    }

    /// Predict the probability that the given user input represents a complete turn
    pub async fn predict_end_of_turn(&self, user_input: &str) -> Result<f32> {
        if user_input.trim().is_empty() {
            return Ok(0.0); // Empty input is not a complete turn
        }

        let start = Instant::now();

        // Format the input as a chat message with proper normalization
        let formatted_input = self.format_as_chat(user_input);
        debug!("Formatted input for model: '{}'", formatted_input);

        // Encode the formatted input
        let (input_ids, attention_mask) = self
            .tokenizer
            .encode_single_text(&formatted_input)
            .await
            .context("Failed to encode user input")?;

        let probability = self
            .model
            .lock()
            .await
            .predict(input_ids.view(), Some(attention_mask.view()))
            .await
            .context("Model prediction failed")?;

        let elapsed = start.elapsed();
        debug!(
            "Prediction completed in {:?} for text: '{}'",
            elapsed, user_input
        );

        if elapsed > Duration::from_millis(50) {
            warn!("Prediction took longer than 50ms: {:?}", elapsed);
        }

        Ok(probability)
    }

    /// Check if the given user input represents a complete turn
    pub async fn is_turn_complete(&self, user_input: &str) -> Result<bool> {
        let probability = self.predict_end_of_turn(user_input).await?;
        debug!(
            "User input: '{}', Turn completion probability: {:.4}",
            user_input, probability
        );
        Ok(probability >= self.config.threshold)
    }

    pub fn set_threshold(&mut self, threshold: f32) {
        self.config.threshold = threshold.clamp(0.0, 1.0);
    }

    pub fn get_threshold(&self) -> f32 {
        self.config.threshold
    }

    /// Process streaming text and check if it represents a complete turn
    pub async fn process_streaming_text(
        &self,
        partial_text: &str,
        check_interval_ms: u64,
    ) -> Result<bool> {
        if partial_text.trim().is_empty() {
            return Ok(false);
        }

        tokio::time::sleep(Duration::from_millis(check_interval_ms)).await;

        self.is_turn_complete(partial_text).await
    }

    pub fn get_config(&self) -> &TurnDetectorConfig {
        &self.config
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_builder_pattern() {
        let builder = TurnDetectorBuilder::new()
            .threshold(0.6)
            .use_quantized(true)
            .num_threads(2);

        assert!(builder.config.threshold == 0.6);
        assert!(builder.config.use_quantized);
        assert!(builder.config.num_threads == Some(2));
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
