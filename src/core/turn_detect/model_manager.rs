use anyhow::{Context, Result};
use ndarray::ArrayView2;
use ort::session::Session;
use ort::session::builder::SessionBuilder;
use ort::value::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::{debug, info, warn};

use crate::core::turn_detect::{assets, config::TurnDetectorConfig};

/// Expected input tensor name for smart-turn model
const INPUT_TENSOR_NAME: &str = "input_features";

pub struct ModelManager {
    session: Arc<Mutex<Session>>,
    config: TurnDetectorConfig,
}

impl ModelManager {
    pub async fn new(config: TurnDetectorConfig) -> Result<Self> {
        let model_path = assets::model_path(&config)?;

        info!("Loading smart-turn ONNX model from: {:?}", model_path);

        // Load model in blocking thread to avoid blocking async runtime
        let session = tokio::task::spawn_blocking({
            let model_path = model_path.clone();
            let config = config.clone();
            move || Self::create_session(&model_path, &config)
        })
        .await
        .context("Failed to spawn blocking task for ONNX model loading")??;

        // Validate model inputs
        Self::validate_model_io(&session)?;

        Ok(Self {
            session: Arc::new(Mutex::new(session)),
            config,
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

        Ok(session)
    }

    fn validate_model_io(session: &Session) -> Result<()> {
        let inputs = session.inputs();
        let outputs = session.outputs();

        debug!("Smart-turn model input count: {}", inputs.len());
        for (i, input) in inputs.iter().enumerate() {
            debug!("  Input {}: name={}", i, input.name());
        }

        debug!("Smart-turn model output count: {}", outputs.len());
        for (i, output) in outputs.iter().enumerate() {
            debug!("  Output {}: name={}", i, output.name());
        }

        // Validate we have at least one input
        if inputs.is_empty() {
            anyhow::bail!("Smart-turn model has no inputs");
        }

        // Check for expected input name
        let has_input_features = inputs.iter().any(|i| i.name() == INPUT_TENSOR_NAME);
        if !has_input_features {
            warn!(
                "Smart-turn model doesn't have expected input '{}'. Available inputs: {:?}",
                INPUT_TENSOR_NAME,
                inputs.iter().map(|i| i.name()).collect::<Vec<_>>()
            );
        }

        if outputs.is_empty() {
            anyhow::bail!("Smart-turn model has no outputs");
        }

        Ok(())
    }

    /// Run inference on mel spectrogram features.
    ///
    /// # Arguments
    /// * `mel_features` - Mel spectrogram with shape (mel_bins, mel_frames) as configured
    ///
    /// # Returns
    /// * `f32` - Turn completion probability (0.0-1.0)
    pub async fn predict(&mut self, mel_features: ArrayView2<'_, f32>) -> Result<f32> {
        let (mel_bins, mel_frames) = mel_features.dim();

        // Validate input dimensions against config (single source of truth)
        if mel_bins != self.config.mel_bins || mel_frames != self.config.mel_frames {
            anyhow::bail!(
                "Invalid mel spectrogram dimensions: got ({}, {}), expected ({}, {})",
                mel_bins,
                mel_frames,
                self.config.mel_bins,
                self.config.mel_frames
            );
        }

        debug!(
            "Running smart-turn inference with mel features shape: ({}, {})",
            mel_bins, mel_frames
        );

        // Convert to 3D tensor: (1, mel_bins, mel_frames) for batch dimension
        let input_data: Vec<f32> = mel_features.iter().copied().collect();
        let input_value = Value::from_array((
            [1, self.config.mel_bins, self.config.mel_frames],
            input_data,
        ))
        .context("Failed to create input tensor")?
        .into();

        // Build inputs
        let inputs: Vec<(&str, Value)> = vec![(INPUT_TENSOR_NAME, input_value)];

        // Lock session and run inference
        let mut session = self.session.lock().unwrap();

        // Get output name before running
        let output_name = session.outputs()[0].name().to_string();

        let outputs = session.run(inputs).context("Smart-turn inference failed")?;

        // Extract output probability
        let output_tensor = outputs
            .get(output_name.as_str())
            .context("No output from smart-turn model")?
            .try_extract_tensor::<f32>()
            .context("Failed to extract output tensor")?;

        let (shape, data) = output_tensor;

        debug!("Smart-turn output shape: {:?}", shape);

        // Extract probability value
        // Smart-turn outputs a single sigmoid probability
        let probability = if !data.is_empty() {
            let raw_value = data[0];

            // Check if value is already in probability range
            if (0.0..=1.0).contains(&raw_value) {
                raw_value
            } else {
                // Apply sigmoid if it's a logit
                1.0 / (1.0 + (-raw_value).exp())
            }
        } else {
            warn!("Smart-turn model produced empty output");
            0.0
        };

        debug!("Smart-turn probability: {:.4}", probability);

        Ok(probability)
    }

    pub fn config(&self) -> &TurnDetectorConfig {
        &self.config
    }
}
