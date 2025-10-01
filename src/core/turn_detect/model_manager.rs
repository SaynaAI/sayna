use anyhow::{Context, Result};
use ndarray::{Array2, ArrayView2, CowArray};
use once_cell::sync::Lazy;
use ort::{Environment, Session, SessionBuilder, Value};
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::core::turn_detect::{assets, config::TurnDetectorConfig};

// Global shared ONNX Environment to avoid expensive bootstrap per detector
static GLOBAL_ONNX_ENV: Lazy<Arc<Environment>> = Lazy::new(|| {
    Arc::new(
        Environment::builder()
            .with_name("turn_detect")
            .build()
            .expect("Failed to create ONNX environment"),
    )
});

pub struct ModelManager {
    session: Arc<Session>,
    config: TurnDetectorConfig,
    // Cached input names to avoid repeated allocations
    input_names: Vec<String>,
    // Pre-allocated attention mask for common sequence lengths
    cached_attention_mask: Option<Array2<i64>>,
}

impl ModelManager {
    pub async fn new(config: TurnDetectorConfig) -> Result<Self> {
        let model_path = assets::model_path(&config)?;

        info!("Loading ONNX model from: {:?}", model_path);

        // Move the blocking ONNX model loading to a dedicated blocking thread
        // to prevent blocking the async runtime
        let session = tokio::task::spawn_blocking({
            let model_path = model_path.clone();
            let config = config.clone();
            move || Self::create_session(&model_path, &config)
        })
        .await
        .context("Failed to spawn blocking task for ONNX model loading")??;

        // Cache input names to avoid repeated allocations
        let input_names: Vec<String> = session
            .inputs
            .iter()
            .map(|input| input.name.clone())
            .collect();

        debug!("Cached model input names: {:?}", input_names);

        Ok(Self {
            session: Arc::new(session),
            config,
            input_names,
            cached_attention_mask: None,
        })
    }

    fn create_session(model_path: &Path, config: &TurnDetectorConfig) -> Result<Session> {
        let mut builder = SessionBuilder::new(&GLOBAL_ONNX_ENV)?
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
        &mut self,
        input_ids: ArrayView2<'_, i64>,
        attention_mask: Option<ArrayView2<'_, i64>>,
    ) -> Result<f32> {
        let batch_size = input_ids.shape()[0];
        let sequence_length = input_ids.shape()[1];

        debug!(
            "Running inference with batch_size={}, sequence_length={}",
            batch_size, sequence_length
        );

        // Use cached input names
        if self.input_names.is_empty() {
            anyhow::bail!("Model has no input names");
        }

        debug!("Using cached input names: {:?}", self.input_names);

        // Convert ndarray to ONNX values
        let allocator = self.session.allocator();

        // Prepare arrays - need to keep them alive for the whole inference
        let input_array = CowArray::from(input_ids.to_owned()).into_dyn();
        let mask_array = if let Some(mask) = attention_mask {
            CowArray::from(mask.to_owned()).into_dyn()
        } else {
            // Reuse cached attention mask if dimensions match, otherwise create new
            let needs_new_mask = self
                .cached_attention_mask
                .as_ref()
                .is_none_or(|cached| cached.shape() != [batch_size, sequence_length]);

            if needs_new_mask {
                self.cached_attention_mask =
                    Some(Array2::<i64>::ones((batch_size, sequence_length)));
            }

            CowArray::from(self.cached_attention_mask.as_ref().unwrap().clone()).into_dyn()
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
            .first()
            .context("No output from model")?
            .try_extract::<f32>()
            .context("Failed to extract tensor")?;

        let logits_view = logits.view();
        let shape = logits_view.shape();

        debug!("Model output shape: {:?}", shape);

        // Log some sample values to understand the output range
        if !shape.is_empty() && shape[0] > 0 {
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
            if (0.0..=1.0).contains(&prob) {
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
            if (0.0..=1.0).contains(&prob) {
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
            let slice = logits_view
                .as_slice()
                .ok_or_else(|| anyhow::anyhow!("Failed to get slice from logits view"))?;

            // Get the logits for key tokens at the last position
            // Token 2 is </s> (standard end-of-sequence)
            // Token 49153 is <|im_end|> (chat format end marker)
            let eos_token_id = 2;

            // Guard against out-of-bounds access if model has fewer classes
            if num_classes <= eos_token_id {
                warn!(
                    "Model has {} classes, but EOS token ID is {}. Using fallback.",
                    num_classes, eos_token_id
                );
                return Ok(0.3); // Conservative fallback
            }

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

            debug!(
                "EOS token (</s>) logit: {:.4}, probability: {:.6}",
                eos_logit, eos_prob
            );
            debug!("Turn completion probability: {:.4}", eos_prob);

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

    pub fn config(&self) -> &TurnDetectorConfig {
        &self.config
    }
}
