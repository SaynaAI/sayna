use anyhow::{Context, Result};
use ndarray::{Array2, ArrayView2};
use ort::session::Session;
use ort::session::builder::SessionBuilder;
use ort::value::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::{debug, info, warn};

use crate::core::turn_detect::{assets, config::TurnDetectorConfig};

pub struct ModelManager {
    session: Arc<Mutex<Session>>,
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
            session: Arc::new(Mutex::new(session)),
            config,
            input_names,
            cached_attention_mask: None,
        })
    }

    fn create_session(model_path: &Path, config: &TurnDetectorConfig) -> Result<Session> {
        let mut builder = SessionBuilder::new()?
            .with_optimization_level(config.graph_optimization_level.to_ort_level())?;

        if let Some(num_threads) = config.num_threads {
            builder = builder
                .with_intra_threads(num_threads)?
                .with_inter_threads(1)?;
        }

        let session = builder.commit_from_file(model_path)?;

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

        // Prepare arrays as owned Array2 for ort 2.0
        let input_array = input_ids.to_owned();
        let mask_array = if let Some(mask) = attention_mask {
            mask.to_owned()
        } else {
            // Reuse cached attention mask if dimensions match, otherwise create new
            let needs_new_mask = self
                .cached_attention_mask
                .as_ref()
                .is_none_or(|cached| cached.dim() != (batch_size, sequence_length));

            if needs_new_mask {
                self.cached_attention_mask =
                    Some(Array2::<i64>::ones((batch_size, sequence_length)));
            }

            self.cached_attention_mask.as_ref().unwrap().clone()
        };

        // Build input values with names - ort 2.0 uses Vec<(&str, Value)>
        let input_name_0 = &self.input_names[0];
        let input_name_1 = if self.input_names.len() > 1 {
            &self.input_names[1]
        } else {
            input_name_0
        };

        debug!(
            "Creating input tensors for: {} and {}",
            input_name_0, input_name_1
        );

        // Convert ndarray 0.15.6 arrays to format compatible with ort 2.0
        // ort expects ([shape...], Vec<data>) tuples for OwnedTensorArrayData
        let input_dim = input_array.dim();
        let input_data: Vec<i64> = input_array.iter().copied().collect();
        let input_value = Value::from_array(([input_dim.0, input_dim.1], input_data))?.into();

        let mask_dim = mask_array.dim();
        let mask_data: Vec<i64> = mask_array.iter().copied().collect();
        let mask_value = Value::from_array(([mask_dim.0, mask_dim.1], mask_data))?.into();

        let inputs: Vec<(&str, Value)> = vec![
            (input_name_0.as_str(), input_value),
            (input_name_1.as_str(), mask_value),
        ];

        // Lock the session mutex for inference
        let mut session = self.session.lock().unwrap();

        // Get output name before running to avoid borrow conflicts
        let output_name = session.outputs[0].name.clone();

        let outputs = session.run(inputs)?;

        let logits_tensor = outputs
            .get(output_name.as_str())
            .context("No output from model")?
            .try_extract_tensor::<f32>()
            .context("Failed to extract tensor")?;

        // Extract shape and data from tensor tuple
        let (shape, data) = logits_tensor;

        debug!("Model output shape: {:?}", shape);

        // Log some sample values to understand the output range
        if !data.is_empty() {
            let first_values: Vec<f32> = data.iter().take(10).copied().collect();
            debug!("First 10 output values: {:?}", first_values);
        }

        // Handle different output formats based on shape dimensions
        let shape_dims: Vec<usize> = shape.iter().map(|&d| d as usize).collect();
        let end_prob = if shape_dims.len() == 2 && shape_dims[1] == 2 {
            // Binary classification output [batch_size, 2]
            // Index 0: probability of continuing
            // Index 1: probability of turn completion
            let end_logit = data[1];
            let continue_logit = data[0];
            Self::softmax(end_logit, continue_logit)
        } else if shape_dims.len() == 2 && shape_dims[1] == 1 {
            // Single probability output [batch_size, 1]
            // Direct probability of turn completion
            let prob = data[0];
            // If it's already a probability (0-1), use it directly
            // If it's a logit, apply sigmoid
            if (0.0..=1.0).contains(&prob) {
                prob
            } else {
                // Apply sigmoid to convert logit to probability
                1.0 / (1.0 + (-prob).exp())
            }
        } else if shape_dims.len() == 1 {
            // Single value output
            let prob = data[0];
            // If it's already a probability (0-1), use it directly
            // If it's a logit, apply sigmoid
            if (0.0..=1.0).contains(&prob) {
                prob
            } else {
                // Apply sigmoid to convert logit to probability
                1.0 / (1.0 + (-prob).exp())
            }
        } else if shape_dims.len() == 3 && shape_dims[0] == 1 {
            // LiveKit turn detector output format: [batch_size=1, sequence_length, num_classes]
            // The model outputs LOGITS that need to be converted to probabilities

            let seq_len = shape_dims[1];
            let num_classes = shape_dims[2];

            debug!(
                "Turn detector output: seq_len={}, num_classes={}",
                seq_len, num_classes
            );

            // Get all logits at the last sequence position
            let last_position_start = (seq_len - 1) * num_classes;

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

            let eos_logit = data[last_position_start + eos_token_id];

            // Find the maximum logit for numerical stability in softmax
            let max_logit = data[last_position_start..(last_position_start + num_classes)]
                .iter()
                .cloned()
                .fold(f32::NEG_INFINITY, f32::max);

            // Apply softmax to get probability
            let exp_sum: f32 = (0..num_classes)
                .map(|i| (data[last_position_start + i] - max_logit).exp())
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
                shape_dims
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
