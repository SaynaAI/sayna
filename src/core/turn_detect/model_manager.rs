use anyhow::{Context, Result};
use ndarray::{Array2, ArrayView2, CowArray};
use ort::{Environment, Session, SessionBuilder, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tracing::{debug, info, warn};

use crate::core::turn_detect::config::TurnDetectorConfig;

pub struct ModelManager {
    session: Arc<Session>,
    config: TurnDetectorConfig,
}

impl ModelManager {
    pub async fn new(config: TurnDetectorConfig) -> Result<Self> {
        let model_path = if let Some(path) = &config.model_path {
            path.clone()
        } else {
            Self::ensure_model_downloaded(&config).await?
        };

        info!("Loading ONNX model from: {:?}", model_path);

        let session = Self::create_session(&model_path, &config)?;

        Ok(Self {
            session: Arc::new(session),
            config,
        })
    }

    fn create_session(model_path: &Path, config: &TurnDetectorConfig) -> Result<Session> {
        let env = Arc::new(Environment::builder().with_name("turn_detect").build()?);
        let mut builder = SessionBuilder::new(&env)?
            .with_optimization_level(config.graph_optimization_level.to_ort_level())?;

        if let Some(num_threads) = config.num_threads {
            builder = builder
                .with_intra_threads(num_threads as i16)?
                .with_inter_threads(1)?;
        }

        let session = builder.with_model_from_file(model_path)?;

        Self::validate_model_inputs(&session)?;

        Ok(session)
    }

    fn validate_model_inputs(session: &Session) -> Result<()> {
        let inputs = &session.inputs;

        debug!("Model input count: {}", inputs.len());
        for (i, input) in inputs.iter().enumerate() {
            debug!("Input {}: {:?}", i, input.name);
        }

        let outputs = &session.outputs;
        debug!("Model output count: {}", outputs.len());
        for (i, output) in outputs.iter().enumerate() {
            debug!("Output {}: {:?}", i, output.name);
        }

        if inputs.is_empty() {
            anyhow::bail!("Model has no inputs");
        }

        Ok(())
    }

    pub async fn predict(
        &self,
        input_ids: ArrayView2<'_, i64>,
        attention_mask: Option<ArrayView2<'_, i64>>,
    ) -> Result<f32> {
        let batch_size = input_ids.shape()[0];
        let sequence_length = input_ids.shape()[1];

        debug!(
            "Running inference with batch_size={}, sequence_length={}",
            batch_size, sequence_length
        );

        // Get input names from the session
        let input_names: Vec<String> = self
            .session
            .inputs
            .iter()
            .map(|input| input.name.clone())
            .collect();

        if input_names.is_empty() {
            anyhow::bail!("Model has no input names");
        }

        debug!("Model input names: {:?}", input_names);

        // Convert ndarray to ONNX values
        let allocator = self.session.allocator();

        // Prepare arrays - need to keep them alive for the whole inference
        let input_array = CowArray::from(input_ids.to_owned()).into_dyn();
        let mask_array = if let Some(mask) = attention_mask {
            CowArray::from(mask.to_owned()).into_dyn()
        } else {
            // Create a default attention mask of ones
            CowArray::from(Array2::<i64>::ones((batch_size, sequence_length))).into_dyn()
        };

        // Build input values in the correct order
        let mut input_values = Vec::new();

        for (i, input_info) in self.session.inputs.iter().enumerate() {
            let name = &input_info.name;
            debug!("Processing input {}: {}", i, name);

            if i == 0 || name.contains("input_ids") {
                input_values.push(Value::from_array(allocator, &input_array)?);
            } else if name.contains("attention_mask") {
                input_values.push(Value::from_array(allocator, &mask_array)?);
            } else {
                // Try to use the first array as a fallback
                input_values.push(Value::from_array(allocator, &input_array)?);
            }
        }

        let outputs = self.session.run(input_values)?;

        let logits = outputs
            .get(0)
            .context("No output from model")?
            .try_extract::<f32>()
            .context("Failed to extract tensor")?;

        let logits_view = logits.view();
        let shape = logits_view.shape();

        debug!("Model output shape: {:?}", shape);
        
        // Log some sample values to understand the output range
        if shape.len() > 0 && shape[0] > 0 {
            let first_values: Vec<f32> = (0..10.min(logits_view.len()))
                .map(|i| logits_view.as_slice().unwrap()[i])
                .collect();
            debug!("First 10 output values: {:?}", first_values);
        }

        // Handle different output formats
        let end_prob = if shape.len() == 2 && shape[1] == 2 {
            // Binary classification output [batch_size, 2]
            // Index 0: probability of continuing
            // Index 1: probability of turn completion
            let end_logit = logits_view[[0, 1]];
            let continue_logit = logits_view[[0, 0]];
            Self::softmax(end_logit, continue_logit)
        } else if shape.len() == 2 && shape[1] == 1 {
            // Single probability output [batch_size, 1]
            // Direct probability of turn completion
            let prob = logits_view[[0, 0]];
            // If it's already a probability (0-1), use it directly
            // If it's a logit, apply sigmoid
            if prob >= 0.0 && prob <= 1.0 {
                prob
            } else {
                // Apply sigmoid to convert logit to probability
                1.0 / (1.0 + (-prob).exp())
            }
        } else if shape.len() == 1 {
            // Single value output
            let prob = logits_view[[0]];
            // If it's already a probability (0-1), use it directly
            // If it's a logit, apply sigmoid
            if prob >= 0.0 && prob <= 1.0 {
                prob
            } else {
                // Apply sigmoid to convert logit to probability
                1.0 / (1.0 + (-prob).exp())
            }
        } else if shape.len() == 3 && shape[0] == 1 {
            // LiveKit turn detector output format: [batch_size=1, sequence_length, num_classes]
            // The model outputs LOGITS that need to be converted to probabilities
            
            let seq_len = shape[1];
            let num_classes = shape[2];
            
            debug!(
                "Turn detector output: seq_len={}, num_classes={}",
                seq_len, num_classes
            );
            
            // Get all logits at the last sequence position
            let last_position_start = (seq_len - 1) * num_classes;
            
            // Let's examine what tokens have high probability at the last position
            let slice = logits_view.as_slice()
                .ok_or_else(|| anyhow::anyhow!("Failed to get slice from logits view"))?;
            
            // Get the logits for key tokens at the last position
            // Token 2 is </s> (standard end-of-sequence)
            // Token 49153 is <|im_end|> (chat format end marker)
            let eos_token_id = 2;
            let eos_logit = slice[last_position_start + eos_token_id];
            
            // Find the maximum logit for numerical stability in softmax
            let max_logit = slice[last_position_start..(last_position_start + num_classes)]
                .iter()
                .cloned()
                .fold(f32::NEG_INFINITY, f32::max);
            
            // Apply softmax to get probability
            let exp_sum: f32 = (0..num_classes)
                .map(|i| (slice[last_position_start + i] - max_logit).exp())
                .sum();
            
            let eos_prob = (eos_logit - max_logit).exp() / exp_sum;
            
            debug!("EOS token (</s>) logit: {:.4}, probability: {:.6}", eos_logit, eos_prob);
            info!("Turn completion probability: {:.4}", eos_prob);
            
            // The model predicts </s> (end-of-sequence) token for complete turns
            eos_prob
        } else {
            warn!(
                "Unexpected output shape: {:?}. Using conservative estimate.",
                shape
            );
            0.3 // Conservative estimate
        };

        debug!("End probability: {:.4}", end_prob);

        Ok(end_prob)
    }

    fn softmax(end_logit: f32, continue_logit: f32) -> f32 {
        let max_logit = end_logit.max(continue_logit);
        let exp_end = (end_logit - max_logit).exp();
        let exp_continue = (continue_logit - max_logit).exp();
        exp_end / (exp_end + exp_continue)
    }

    async fn ensure_model_downloaded(config: &TurnDetectorConfig) -> Result<PathBuf> {
        let cache_dir = Self::get_cache_dir()?;
        fs::create_dir_all(&cache_dir).await?;

        let model_filename = "model_quantized.onnx";

        let model_path = cache_dir.join(model_filename);

        if model_path.exists() {
            info!("Using cached model at: {:?}", model_path);
            return Ok(model_path);
        }

        let model_url = config
            .model_url
            .as_ref()
            .context("No model URL specified and model not found locally")?;

        info!("Downloading model from: {}", model_url);
        Self::download_file(model_url, &model_path).await?;

        Ok(model_path)
    }

    fn get_cache_dir() -> Result<PathBuf> {
        let cache_dir = if let Ok(dir) = std::env::var("TURN_DETECT_CACHE_DIR") {
            PathBuf::from(dir)
        } else {
            dirs::cache_dir()
                .context("Failed to get cache directory")?
                .join("sayna")
                .join("turn_detect")
        };
        Ok(cache_dir)
    }

    async fn download_file(url: &str, path: &Path) -> Result<()> {
        let response = reqwest::get(url)
            .await
            .context("Failed to download model")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to download model: HTTP {}", response.status());
        }

        let bytes = response.bytes().await?;

        if let Some(expected_hash) = Self::get_expected_hash(url) {
            Self::verify_hash(&bytes, &expected_hash)?;
        }

        fs::write(path, bytes).await?;
        info!("Model downloaded successfully to: {:?}", path);

        Ok(())
    }

    fn get_expected_hash(url: &str) -> Option<String> {
        if url.contains("model_quantized.onnx") {
            Some("expected_hash_here".to_string())
        } else {
            None
        }
    }

    fn verify_hash(data: &[u8], expected: &str) -> Result<()> {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        let actual = format!("{:x}", result);

        if actual != expected {
            warn!("Hash mismatch - expected: {}, actual: {}", expected, actual);
        }

        Ok(())
    }

    pub fn config(&self) -> &TurnDetectorConfig {
        &self.config
    }
}

fn dirs_cache_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        home::home_dir().map(|h| h.join("Library").join("Caches"))
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var("XDG_CACHE_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| home::home_dir().map(|h| h.join(".cache")))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("LOCALAPPDATA").ok().map(PathBuf::from)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

mod dirs {
    use super::*;

    pub fn cache_dir() -> Option<PathBuf> {
        super::dirs_cache_dir()
    }
}

mod home {
    use std::path::PathBuf;

    pub fn home_dir() -> Option<PathBuf> {
        std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .ok()
            .map(PathBuf::from)
    }
}
