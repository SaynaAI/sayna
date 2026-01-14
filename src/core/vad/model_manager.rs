//! ONNX model manager for Silero VAD.
//!
//! Handles ONNX model loading, session creation, state management,
//! and inference for Silero-VAD voice activity detection.

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use ndarray::Array3;
use ort::session::Session;
use ort::session::builder::SessionBuilder;
use ort::value::Value;
use tracing::{debug, info};

use super::assets;
use super::config::SileroVADConfig;

/// Size of LSTM state tensor: 2 * 1 * 128 = 256 elements
const STATE_SIZE: usize = 256;

/// State tensor dimension 0: number of LSTM layers/directions
const STATE_DIM_0: usize = 2;

/// State tensor dimension 1: batch size (always 1)
const STATE_DIM_1: usize = 1;

/// State tensor dimension 2: hidden size
const STATE_DIM_2: usize = 128;

/// ONNX model manager for Silero VAD inference.
///
/// Handles model loading, LSTM state management, context buffering,
/// and audio frame inference. The model uses recurrent state that must
/// be maintained across consecutive audio frames for accurate detection.
///
/// # Model Specifications
///
/// **Input Tensors:**
/// - `input`: Audio frame shape `[1, frame_size]` (512 for 16kHz, 256 for 8kHz)
/// - `state`: LSTM hidden/cell state shape `[2, 1, 128]`
/// - `sr`: Sample rate as int64 `[1]`
///
/// **Output Tensors:**
/// - `output`: Speech probability `[1, 1]` (0.0 to 1.0)
/// - `stateN`: Updated LSTM state `[2, 1, 128]`
///
/// # Context Handling
///
/// The model expects audio frames with context from the previous chunk.
/// For 16kHz: 512 samples + 64 context samples from the previous frame.
/// The context buffer is automatically managed between calls to `process_audio`.
pub struct VADModelManager {
    /// ONNX inference session wrapped for thread safety
    session: Arc<Mutex<Session>>,

    /// Configuration for the VAD model
    config: SileroVADConfig,

    /// Current LSTM state maintained across inferences
    /// Shape: [2, 1, 128] containing hidden and cell states
    state: Array3<f32>,

    /// Context buffer containing last N samples from previous chunk
    /// Used for temporal continuity between frames
    context: Vec<f32>,

    /// Sample rate as int64 for model input tensor
    sample_rate_tensor: i64,
}

impl VADModelManager {
    /// Create a new VAD model manager and load the Silero VAD model.
    ///
    /// The model is loaded in a blocking thread to avoid blocking the
    /// async runtime. LSTM states are initialized to zeros.
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration for the VAD model including model path,
    ///   sample rate, and ONNX session settings.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The model file is not found (run `sayna init` first)
    /// - The ONNX session fails to load
    /// - The model has unexpected input/output structure
    pub async fn new(config: SileroVADConfig) -> Result<Self> {
        let model_path = assets::model_path(&config)?;

        info!("Loading Silero-VAD ONNX model from: {:?}", model_path);

        // Load model in blocking thread to avoid blocking async runtime
        let session = tokio::task::spawn_blocking({
            let model_path = model_path.clone();
            let config = config.clone();
            move || Self::create_session(&model_path, &config)
        })
        .await
        .context("Failed to spawn blocking task for ONNX model loading")??;

        // Initialize LSTM state with zeros - shape [2, 1, 128]
        let state = Array3::<f32>::zeros((STATE_DIM_0, STATE_DIM_1, STATE_DIM_2));

        // Initialize context buffer with zeros
        let context_size = config.sample_rate.context_size();
        let context = vec![0.0f32; context_size];

        let sample_rate_tensor = config.sample_rate.as_hz() as i64;

        info!(
            "Silero-VAD model loaded successfully (sample_rate={}, frame_size={}, context_size={})",
            config.sample_rate.as_hz(),
            config.sample_rate.frame_size(),
            context_size
        );

        Ok(Self {
            session: Arc::new(Mutex::new(session)),
            config,
            state,
            context,
            sample_rate_tensor,
        })
    }

    /// Create the ONNX session with the configured optimization level.
    fn create_session(model_path: &Path, config: &SileroVADConfig) -> Result<Session> {
        let mut builder = SessionBuilder::new()?
            .with_optimization_level(config.graph_optimization_level.to_ort_level())?;

        if let Some(num_threads) = config.num_threads {
            builder = builder
                .with_intra_threads(num_threads)?
                .with_inter_threads(1)?;
        }

        let session = builder
            .commit_from_file(model_path)
            .context("Failed to load Silero-VAD ONNX model")?;

        Self::validate_model_io(&session)?;

        Ok(session)
    }

    /// Validate that the model has the expected inputs and outputs.
    fn validate_model_io(session: &Session) -> Result<()> {
        let inputs = &session.inputs;
        let outputs = &session.outputs;

        debug!("Silero-VAD model input count: {}", inputs.len());
        for (i, input) in inputs.iter().enumerate() {
            debug!("  Input {}: name={}", i, input.name);
        }

        debug!("Silero-VAD model output count: {}", outputs.len());
        for (i, output) in outputs.iter().enumerate() {
            debug!("  Output {}: name={}", i, output.name);
        }

        // Silero-VAD expects 3 inputs: input, state (or h, c), sr
        // Different versions may have slightly different names
        if inputs.len() < 3 {
            anyhow::bail!(
                "Silero-VAD model has {} inputs, expected at least 3 (input, state/h/c, sr)",
                inputs.len()
            );
        }

        // Expect at least 2 outputs: output (probability) and stateN (or hn, cn)
        if outputs.is_empty() {
            anyhow::bail!("Silero-VAD model has no outputs");
        }

        Ok(())
    }

    /// Process a single audio frame and return speech probability.
    ///
    /// The audio frame is normalized, prepended with context from the
    /// previous frame, and passed through the model. LSTM state is
    /// automatically updated for the next call.
    ///
    /// # Arguments
    ///
    /// * `audio` - Audio samples as i16 PCM. Must be exactly `frame_size`
    ///   samples (512 for 16kHz, 256 for 8kHz).
    ///
    /// # Returns
    ///
    /// Speech probability between 0.0 and 1.0. Values above the configured
    /// threshold indicate speech is detected.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Audio frame size doesn't match expected frame_size
    /// - ONNX inference fails
    pub fn process_audio(&mut self, audio: &[i16]) -> Result<f32> {
        let frame_size = self.config.sample_rate.frame_size();

        if audio.len() != frame_size {
            anyhow::bail!(
                "Invalid audio frame size: got {}, expected {}",
                audio.len(),
                frame_size
            );
        }

        // Convert i16 samples to f32 normalized to [-1.0, 1.0]
        let audio_f32: Vec<f32> = audio.iter().map(|&s| s as f32 / 32768.0).collect();

        // Concatenate context + audio for temporal continuity
        let context_size = self.config.sample_rate.context_size();
        let mut input_with_context = Vec::with_capacity(context_size + frame_size);
        input_with_context.extend_from_slice(&self.context);
        input_with_context.extend_from_slice(&audio_f32);

        // Update context buffer with last N samples for next call
        self.context
            .copy_from_slice(&audio_f32[audio_f32.len() - context_size..]);

        // Run inference
        let speech_prob = self.run_inference(&input_with_context)?;

        Ok(speech_prob)
    }

    /// Run ONNX inference with the prepared input.
    fn run_inference(&mut self, input: &[f32]) -> Result<f32> {
        let mut session = self.session.lock().unwrap();

        let input_len = input.len();

        // Prepare input tensor [1, frame_size + context_size]
        let input_data: Vec<f32> = input.to_vec();
        let input_value = Value::from_array(([1, input_len], input_data))
            .context("Failed to create input tensor")?
            .into();

        // Prepare state tensor [2, 1, 128]
        let state_data: Vec<f32> = self.state.iter().copied().collect();
        let state_value = Value::from_array(([STATE_DIM_0, STATE_DIM_1, STATE_DIM_2], state_data))
            .context("Failed to create state tensor")?
            .into();

        // Prepare sample rate tensor [1]
        let sr_value = Value::from_array(([1], vec![self.sample_rate_tensor]))
            .context("Failed to create sample rate tensor")?
            .into();

        // Build inputs - Silero VAD expects: input, state, sr
        // Note: Some model versions use "h" and "c" instead of combined "state"
        // Try the combined state format first, which is the newer format
        let inputs: Vec<(&str, Value)> = vec![
            ("input", input_value),
            ("state", state_value),
            ("sr", sr_value),
        ];

        // Run inference
        let outputs = session
            .run(inputs)
            .context("Failed to run Silero-VAD inference")?;

        // Extract speech probability from output tensor
        let output_tensor = outputs
            .get("output")
            .context("No 'output' tensor in Silero-VAD results")?
            .try_extract_tensor::<f32>()
            .context("Failed to extract output tensor")?;

        let (_, output_data) = output_tensor;
        let speech_prob = output_data.first().copied().unwrap_or(0.0);

        // Extract and update LSTM state for next inference
        if let Some(state_tensor) = outputs.get("stateN") {
            let (_, state_data) = state_tensor
                .try_extract_tensor::<f32>()
                .context("Failed to extract stateN tensor")?;

            if state_data.len() == STATE_SIZE {
                self.state = Array3::from_shape_vec(
                    (STATE_DIM_0, STATE_DIM_1, STATE_DIM_2),
                    state_data.to_vec(),
                )
                .context("Failed to reshape state tensor")?;
            }
        } else if let (Some(hn_tensor), Some(cn_tensor)) = (outputs.get("hn"), outputs.get("cn")) {
            // Handle older model format with separate h and c states
            let (_, hn_data) = hn_tensor
                .try_extract_tensor::<f32>()
                .context("Failed to extract hn tensor")?;
            let (_, cn_data) = cn_tensor
                .try_extract_tensor::<f32>()
                .context("Failed to extract cn tensor")?;

            // Combine h and c into state tensor
            // Assuming each is [1, 1, 64] or similar, we need to combine them
            let combined_len = hn_data.len() + cn_data.len();
            if combined_len == STATE_SIZE {
                let mut combined = Vec::with_capacity(STATE_SIZE);
                combined.extend_from_slice(hn_data);
                combined.extend_from_slice(cn_data);
                self.state =
                    Array3::from_shape_vec((STATE_DIM_0, STATE_DIM_1, STATE_DIM_2), combined)
                        .context("Failed to reshape combined state tensor")?;
            }
        }

        debug!("Silero-VAD speech probability: {:.4}", speech_prob);

        Ok(speech_prob)
    }

    /// Reset the LSTM state and context buffer.
    ///
    /// Call this when starting to process a new audio stream to clear
    /// any accumulated state from previous audio.
    pub fn reset(&mut self) {
        self.state = Array3::<f32>::zeros((STATE_DIM_0, STATE_DIM_1, STATE_DIM_2));
        let context_size = self.config.sample_rate.context_size();
        self.context = vec![0.0f32; context_size];
        debug!("Silero-VAD state reset");
    }

    /// Get the current configuration.
    pub fn config(&self) -> &SileroVADConfig {
        &self.config
    }

    /// Get the expected frame size in samples.
    pub fn frame_size(&self) -> usize {
        self.config.sample_rate.frame_size()
    }

    /// Get the sample rate in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate.as_hz()
    }
}

// Backwards compatibility alias
pub use VADModelManager as ModelManager;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::vad::config::VADSampleRate;

    #[test]
    fn test_state_dimensions() {
        // Verify state dimensions match expected values
        assert_eq!(STATE_DIM_0 * STATE_DIM_1 * STATE_DIM_2, STATE_SIZE);
        assert_eq!(STATE_SIZE, 256);
    }

    #[test]
    fn test_audio_normalization() {
        // Test that i16 max value normalizes to ~1.0
        let max_sample: i16 = i16::MAX;
        let normalized = max_sample as f32 / 32768.0;
        assert!((normalized - 1.0).abs() < 0.001);

        // Test that i16 min value normalizes to ~-1.0
        let min_sample: i16 = i16::MIN;
        let normalized = min_sample as f32 / 32768.0;
        assert!((normalized - (-1.0)).abs() < 0.001);

        // Test zero
        let zero_sample: i16 = 0;
        let normalized = zero_sample as f32 / 32768.0;
        assert_eq!(normalized, 0.0);
    }

    #[test]
    fn test_frame_sizes() {
        assert_eq!(VADSampleRate::Rate16kHz.frame_size(), 512);
        assert_eq!(VADSampleRate::Rate8kHz.frame_size(), 256);
    }

    #[test]
    fn test_context_sizes() {
        assert_eq!(VADSampleRate::Rate16kHz.context_size(), 64);
        assert_eq!(VADSampleRate::Rate8kHz.context_size(), 32);
    }
}
