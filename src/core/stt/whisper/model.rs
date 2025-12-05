//! ONNX model manager for Whisper encoder and decoder
//!
//! This module handles loading and running inference on Whisper ONNX models.
//! Whisper uses a encoder-decoder transformer architecture.
//!
//! ## Model Architecture
//!
//! - **Encoder**: Processes mel spectrogram → audio embeddings
//!   - Input: [batch, n_mels, n_frames] mel spectrogram
//!   - Output: [batch, seq_len, d_model] embeddings
//!
//! - **Decoder**: Autoregressively generates tokens from embeddings
//!   - Inputs: token IDs, encoder output, KV cache
//!   - Output: next token logits
//!
//! ## ONNX Files
//!
//! - `encoder_model.onnx` - Audio encoder
//! - `decoder_model_merged.onnx` - Decoder with merged KV cache
//!
//! ## Reference
//!
//! Pattern follows `src/core/tts/kokoro/model.rs` for ONNX model management.

#![cfg(feature = "whisper-stt")]

use crate::core::stt::STTError;
use ort::{
    execution_providers::CPUExecutionProvider,
    session::{
        Session, SessionInputValue, SessionInputs,
        builder::{GraphOptimizationLevel, SessionBuilder},
    },
    value::{Tensor, Value},
};
use std::borrow::Cow;
use std::path::Path;
use std::sync::Mutex;
use tracing::{debug, info};

use super::mel::{DEFAULT_N_MELS, LARGE_V3_N_MELS, N_FRAMES};
use super::tokenizer::tokens;

/// Encoder model input/output tensor names
mod encoder_schema {
    /// Input: mel spectrogram features [batch, n_mels, n_frames]
    pub const INPUT_FEATURES: &str = "input_features";
    /// Output: encoder hidden states [batch, seq_len, d_model]
    pub const LAST_HIDDEN_STATE: &str = "last_hidden_state";
}

/// Decoder model input/output tensor names
mod decoder_schema {
    /// Input: token IDs [batch, seq_len]
    pub const INPUT_IDS: &str = "input_ids";
    /// Input: encoder hidden states [batch, encoder_seq_len, d_model]
    pub const ENCODER_HIDDEN_STATES: &str = "encoder_hidden_states";
    /// Output: logits [batch, seq_len, vocab_size]
    pub const LOGITS: &str = "logits";
}

/// Whisper model configuration
#[derive(Debug, Clone)]
pub struct ModelConfig {
    /// Model dimension (d_model)
    pub d_model: usize,
    /// Number of encoder layers
    pub encoder_layers: usize,
    /// Number of decoder layers
    pub decoder_layers: usize,
    /// Number of attention heads
    pub n_heads: usize,
    /// Vocabulary size
    pub vocab_size: usize,
    /// Number of encoder output frames (1500 for Whisper)
    pub encoder_output_frames: usize,
    /// Number of mel bins expected by the encoder input
    pub n_mels: usize,
}

impl ModelConfig {
    /// Configuration for Whisper base model
    pub fn base() -> Self {
        Self {
            d_model: 512,
            encoder_layers: 6,
            decoder_layers: 6,
            n_heads: 8,
            vocab_size: 51865,
            encoder_output_frames: 1500,
            n_mels: DEFAULT_N_MELS,
        }
    }

    /// Configuration for Whisper tiny model
    pub fn tiny() -> Self {
        Self {
            d_model: 384,
            encoder_layers: 4,
            decoder_layers: 4,
            n_heads: 6,
            vocab_size: 51865,
            encoder_output_frames: 1500,
            n_mels: DEFAULT_N_MELS,
        }
    }

    /// Configuration for Whisper small model
    pub fn small() -> Self {
        Self {
            d_model: 768,
            encoder_layers: 12,
            decoder_layers: 12,
            n_heads: 12,
            vocab_size: 51865,
            encoder_output_frames: 1500,
            n_mels: DEFAULT_N_MELS,
        }
    }

    /// Configuration for Whisper medium model
    pub fn medium() -> Self {
        Self {
            d_model: 1024,
            encoder_layers: 24,
            decoder_layers: 24,
            n_heads: 16,
            vocab_size: 51865,
            encoder_output_frames: 1500,
            n_mels: DEFAULT_N_MELS,
        }
    }

    /// Configuration for Whisper large models (v1, v2, v3)
    pub fn large() -> Self {
        Self {
            d_model: 1280,
            encoder_layers: 32,
            decoder_layers: 32,
            n_heads: 20,
            vocab_size: 51865,
            encoder_output_frames: 1500,
            n_mels: DEFAULT_N_MELS,
        }
    }

    /// Configuration for Whisper large-v3 (uses 128 mel bins)
    pub fn large_v3() -> Self {
        Self {
            n_mels: LARGE_V3_N_MELS,
            ..Self::large()
        }
    }
}

/// Inference configuration options
#[derive(Debug, Clone)]
pub struct InferenceConfig {
    /// Number of threads for ONNX inference (default: 4)
    pub num_threads: usize,
    /// Maximum sequence length for decoder output (default: 448)
    pub max_length: usize,
    /// Temperature for sampling (0.0 = greedy, higher = more random)
    pub temperature: f32,
    /// Whether to suppress blank tokens at the start
    pub suppress_blank: bool,
    /// Token IDs to suppress (never predict)
    pub suppress_tokens: Vec<i64>,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            num_threads: 4,
            max_length: 448,  // Whisper's default max tokens
            temperature: 0.0, // Greedy decoding by default
            suppress_blank: true,
            suppress_tokens: vec![], // Empty by default
        }
    }
}

/// Whisper ONNX model manager
///
/// Handles loading and running inference on Whisper encoder and decoder models.
/// The encoder runs once per audio segment, while the decoder runs autoregressively.
pub struct WhisperModel {
    /// Encoder ONNX session (protected by mutex for thread safety)
    encoder: Mutex<Session>,
    /// Decoder ONNX session (protected by mutex for thread safety)
    decoder: Mutex<Session>,
    /// Model architecture configuration
    config: ModelConfig,
    /// Inference configuration
    inference_config: InferenceConfig,
}

impl WhisperModel {
    /// Load Whisper model from ONNX files
    ///
    /// # Arguments
    /// * `encoder_path` - Path to encoder ONNX file
    /// * `decoder_path` - Path to decoder ONNX file
    /// * `config` - Model architecture configuration
    ///
    /// # Returns
    /// * `Result<Self, STTError>` - Loaded model or error
    pub fn load(
        encoder_path: &Path,
        decoder_path: &Path,
        config: ModelConfig,
    ) -> Result<Self, STTError> {
        Self::load_with_config(
            encoder_path,
            decoder_path,
            config,
            InferenceConfig::default(),
        )
    }

    /// Load Whisper model with custom inference configuration
    ///
    /// # Arguments
    /// * `encoder_path` - Path to encoder ONNX file
    /// * `decoder_path` - Path to decoder ONNX file
    /// * `config` - Model architecture configuration
    /// * `inference_config` - Inference settings (threads, max length, etc.)
    ///
    /// # Returns
    /// * `Result<Self, STTError>` - Loaded model or error
    pub fn load_with_config(
        encoder_path: &Path,
        decoder_path: &Path,
        config: ModelConfig,
        inference_config: InferenceConfig,
    ) -> Result<Self, STTError> {
        // Validate paths exist
        if !encoder_path.exists() {
            return Err(STTError::ConfigurationError(format!(
                "Encoder model not found: {}. Run 'sayna init' to download.",
                encoder_path.display()
            )));
        }
        if !decoder_path.exists() {
            return Err(STTError::ConfigurationError(format!(
                "Decoder model not found: {}. Run 'sayna init' to download.",
                decoder_path.display()
            )));
        }

        info!("Loading Whisper encoder from: {}", encoder_path.display());
        let encoder = Self::create_session(encoder_path, &inference_config)?;
        Self::validate_encoder_session(&encoder)?;

        info!("Loading Whisper decoder from: {}", decoder_path.display());
        let decoder = Self::create_session(decoder_path, &inference_config)?;
        Self::validate_decoder_session(&decoder)?;

        info!(
            "Whisper model loaded successfully (d_model={}, vocab_size={})",
            config.d_model, config.vocab_size
        );

        Ok(Self {
            encoder: Mutex::new(encoder),
            decoder: Mutex::new(decoder),
            config,
            inference_config,
        })
    }

    /// Load Whisper model asynchronously (non-blocking)
    ///
    /// Wraps model loading in a blocking task to avoid blocking the async runtime.
    pub async fn load_async(
        encoder_path: &Path,
        decoder_path: &Path,
        config: ModelConfig,
    ) -> Result<Self, STTError> {
        Self::load_async_with_config(
            encoder_path,
            decoder_path,
            config,
            InferenceConfig::default(),
        )
        .await
    }

    /// Load Whisper model asynchronously with custom inference configuration
    pub async fn load_async_with_config(
        encoder_path: &Path,
        decoder_path: &Path,
        config: ModelConfig,
        inference_config: InferenceConfig,
    ) -> Result<Self, STTError> {
        let encoder_path = encoder_path.to_path_buf();
        let decoder_path = decoder_path.to_path_buf();

        tokio::task::spawn_blocking(move || {
            Self::load_with_config(&encoder_path, &decoder_path, config, inference_config)
        })
        .await
        .map_err(|e| {
            STTError::ConfigurationError(format!("Failed to spawn model load task: {}", e))
        })?
    }

    /// Create an ONNX session with the given configuration
    fn create_session(path: &Path, config: &InferenceConfig) -> Result<Session, STTError> {
        // Get execution providers (CPU only for now)
        let providers = vec![CPUExecutionProvider::default().build()];

        // Build session with configuration
        let mut builder = SessionBuilder::new().map_err(|e| {
            STTError::ConfigurationError(format!("Failed to create session builder: {}", e))
        })?;

        // Apply optimization level (Level3 for best performance)
        builder = builder
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| {
                STTError::ConfigurationError(format!("Failed to set optimization level: {}", e))
            })?;

        // Apply thread configuration
        if config.num_threads > 0 {
            builder = builder
                .with_intra_threads(config.num_threads)
                .map_err(|e| {
                    STTError::ConfigurationError(format!("Failed to set intra threads: {}", e))
                })?
                .with_inter_threads(1)
                .map_err(|e| {
                    STTError::ConfigurationError(format!("Failed to set inter threads: {}", e))
                })?;
        }

        // Set execution providers
        builder = builder.with_execution_providers(providers).map_err(|e| {
            STTError::ConfigurationError(format!("Failed to set execution providers: {}", e))
        })?;

        // Load model from file
        let session = builder.commit_from_file(path).map_err(|e| {
            STTError::ConfigurationError(format!("Failed to load ONNX model: {}", e))
        })?;

        Ok(session)
    }

    /// Validate encoder session has expected inputs/outputs
    fn validate_encoder_session(session: &Session) -> Result<(), STTError> {
        let input_names: Vec<&str> = session.inputs.iter().map(|i| i.name.as_str()).collect();
        let output_names: Vec<&str> = session.outputs.iter().map(|o| o.name.as_str()).collect();

        debug!("Encoder inputs: {:?}", input_names);
        debug!("Encoder outputs: {:?}", output_names);

        if !input_names.contains(&encoder_schema::INPUT_FEATURES) {
            return Err(STTError::ConfigurationError(format!(
                "Encoder missing input '{}'. Found: {:?}",
                encoder_schema::INPUT_FEATURES,
                input_names
            )));
        }

        if !output_names.contains(&encoder_schema::LAST_HIDDEN_STATE) {
            return Err(STTError::ConfigurationError(format!(
                "Encoder missing output '{}'. Found: {:?}",
                encoder_schema::LAST_HIDDEN_STATE,
                output_names
            )));
        }

        Ok(())
    }

    /// Validate decoder session has expected inputs/outputs
    fn validate_decoder_session(session: &Session) -> Result<(), STTError> {
        let input_names: Vec<&str> = session.inputs.iter().map(|i| i.name.as_str()).collect();
        let output_names: Vec<&str> = session.outputs.iter().map(|o| o.name.as_str()).collect();

        debug!("Decoder inputs: {:?}", input_names);
        debug!("Decoder outputs: {:?}", output_names);

        if !input_names.contains(&decoder_schema::INPUT_IDS) {
            return Err(STTError::ConfigurationError(format!(
                "Decoder missing input '{}'. Found: {:?}",
                decoder_schema::INPUT_IDS,
                input_names
            )));
        }

        if !input_names.contains(&decoder_schema::ENCODER_HIDDEN_STATES) {
            return Err(STTError::ConfigurationError(format!(
                "Decoder missing input '{}'. Found: {:?}",
                decoder_schema::ENCODER_HIDDEN_STATES,
                input_names
            )));
        }

        if !output_names.contains(&decoder_schema::LOGITS) {
            return Err(STTError::ConfigurationError(format!(
                "Decoder missing output '{}'. Found: {:?}",
                decoder_schema::LOGITS,
                output_names
            )));
        }

        Ok(())
    }

    /// Run encoder on mel spectrogram to produce audio features
    ///
    /// # Arguments
    /// * `mel_features` - Mel spectrogram [N_MELS * N_FRAMES] (flattened, row-major)
    ///
    /// # Returns
    /// * Encoder hidden states [encoder_output_frames * d_model] (flattened)
    pub fn encode(&self, mel_features: &[f32]) -> Result<Vec<f32>, STTError> {
        // Validate input size
        let expected_size = self.config.n_mels * N_FRAMES;
        if mel_features.len() != expected_size {
            return Err(STTError::AudioProcessingError(format!(
                "Invalid mel features size: expected {}, got {}",
                expected_size,
                mel_features.len()
            )));
        }

        // Prepare input tensor [batch=1, n_mels=80, n_frames=3000]
        let input_tensor =
            Tensor::from_array(([1, self.config.n_mels, N_FRAMES], mel_features.to_vec()))
                .map_err(|e| {
                    STTError::AudioProcessingError(format!("Failed to create input tensor: {}", e))
                })?;

        // Lock encoder and run inference
        let mut encoder = self.encoder.lock().map_err(|e| {
            STTError::AudioProcessingError(format!("Failed to lock encoder: {}", e))
        })?;

        let inputs: Vec<(Cow<'static, str>, SessionInputValue<'static>)> = vec![(
            Cow::Borrowed(encoder_schema::INPUT_FEATURES),
            SessionInputValue::Owned(Value::from(input_tensor)),
        )];

        let outputs = encoder.run(SessionInputs::from(inputs)).map_err(|e| {
            STTError::AudioProcessingError(format!("Encoder inference failed: {}", e))
        })?;

        // Extract hidden states output
        let (shape, data) = outputs[encoder_schema::LAST_HIDDEN_STATE]
            .try_extract_tensor::<f32>()
            .map_err(|e| {
                STTError::AudioProcessingError(format!("Failed to extract encoder output: {}", e))
            })?;

        debug!(
            "Encoder output shape: {:?}, total elements: {}",
            shape,
            data.len()
        );

        Ok(data.to_vec())
    }

    /// Run a single decoder step to get next token logits
    ///
    /// # Arguments
    /// * `token_ids` - Previous token IDs [seq_len]
    /// * `encoder_output` - Encoder hidden states [encoder_output_frames * d_model]
    ///
    /// # Returns
    /// * Logits for next token [vocab_size]
    pub fn decode_step(
        &self,
        token_ids: &[i64],
        encoder_output: &[f32],
    ) -> Result<Vec<f32>, STTError> {
        let seq_len = token_ids.len();
        if seq_len == 0 {
            return Err(STTError::AudioProcessingError(
                "Token sequence cannot be empty".to_string(),
            ));
        }

        // Validate encoder output size
        let expected_encoder_size = self.config.encoder_output_frames * self.config.d_model;
        if encoder_output.len() != expected_encoder_size {
            return Err(STTError::AudioProcessingError(format!(
                "Invalid encoder output size: expected {}, got {}",
                expected_encoder_size,
                encoder_output.len()
            )));
        }

        // Prepare input_ids tensor [batch=1, seq_len]
        let input_ids_tensor =
            Tensor::from_array(([1, seq_len], token_ids.to_vec())).map_err(|e| {
                STTError::AudioProcessingError(format!("Failed to create input_ids tensor: {}", e))
            })?;

        // Prepare encoder_hidden_states tensor [batch=1, encoder_seq_len, d_model]
        let encoder_tensor = Tensor::from_array((
            [1, self.config.encoder_output_frames, self.config.d_model],
            encoder_output.to_vec(),
        ))
        .map_err(|e| {
            STTError::AudioProcessingError(format!("Failed to create encoder tensor: {}", e))
        })?;

        // Lock decoder and run inference
        let mut decoder = self.decoder.lock().map_err(|e| {
            STTError::AudioProcessingError(format!("Failed to lock decoder: {}", e))
        })?;

        let inputs: Vec<(Cow<'static, str>, SessionInputValue<'static>)> = vec![
            (
                Cow::Borrowed(decoder_schema::INPUT_IDS),
                SessionInputValue::Owned(Value::from(input_ids_tensor)),
            ),
            (
                Cow::Borrowed(decoder_schema::ENCODER_HIDDEN_STATES),
                SessionInputValue::Owned(Value::from(encoder_tensor)),
            ),
        ];

        let outputs = decoder.run(SessionInputs::from(inputs)).map_err(|e| {
            STTError::AudioProcessingError(format!("Decoder inference failed: {}", e))
        })?;

        // Extract logits output [batch=1, seq_len, vocab_size]
        let (shape, data) = outputs[decoder_schema::LOGITS]
            .try_extract_tensor::<f32>()
            .map_err(|e| {
                STTError::AudioProcessingError(format!("Failed to extract decoder output: {}", e))
            })?;

        debug!("Decoder output shape: {:?}", shape);

        // Return only the last position's logits (for next token prediction)
        let vocab_size = self.config.vocab_size;
        let start_idx = (seq_len - 1) * vocab_size;
        let end_idx = start_idx + vocab_size;

        if end_idx > data.len() {
            return Err(STTError::AudioProcessingError(format!(
                "Logits extraction failed: expected at least {} elements, got {}",
                end_idx,
                data.len()
            )));
        }

        Ok(data[start_idx..end_idx].to_vec())
    }

    /// Transcribe mel spectrogram to token sequence
    ///
    /// # Arguments
    /// * `mel_features` - Mel spectrogram [N_MELS * N_FRAMES]
    /// * `initial_tokens` - Initial prompt tokens (from tokenizer)
    ///
    /// # Returns
    /// * Generated token sequence
    pub fn transcribe(
        &self,
        mel_features: &[f32],
        initial_tokens: &[i64],
    ) -> Result<Vec<i64>, STTError> {
        // Run encoder once
        let encoder_output = self.encode(mel_features)?;

        // Initialize token sequence with prompt
        let mut token_ids = initial_tokens.to_vec();

        // Autoregressive decoding loop
        while token_ids.len() < self.inference_config.max_length {
            // Get logits for next token
            let logits = self.decode_step(&token_ids, &encoder_output)?;

            // Apply suppression if configured
            let logits = self.apply_suppressions(&logits, &token_ids);

            // Sample next token
            let next_token = self.sample_token(&logits);

            // Check for end-of-text
            if next_token == tokens::EOT {
                break;
            }

            token_ids.push(next_token);
        }

        debug!(
            "Transcription complete: {} tokens generated",
            token_ids.len() - initial_tokens.len()
        );

        Ok(token_ids)
    }

    /// Transcribe mel spectrogram asynchronously
    pub async fn transcribe_async(
        &self,
        mel_features: Vec<f32>,
        initial_tokens: Vec<i64>,
    ) -> Result<Vec<i64>, STTError> {
        // Note: The actual inference is CPU-bound, but we could potentially
        // yield between decoder steps for better async behavior
        self.transcribe(&mel_features, &initial_tokens)
    }

    /// Apply token suppressions to logits
    fn apply_suppressions(&self, logits: &[f32], current_tokens: &[i64]) -> Vec<f32> {
        let mut logits = logits.to_vec();

        // Suppress configured tokens
        for &token in &self.inference_config.suppress_tokens {
            if (token as usize) < logits.len() {
                logits[token as usize] = f32::NEG_INFINITY;
            }
        }

        // Suppress blank at start if configured
        if self.inference_config.suppress_blank && current_tokens.len() <= 4 {
            // Token 220 is space in GPT-2 vocab, 50257 is EOT
            const SPACE_TOKEN: usize = 220;
            if SPACE_TOKEN < logits.len() {
                logits[SPACE_TOKEN] = f32::NEG_INFINITY;
            }
        }

        logits
    }

    /// Sample next token from logits
    fn sample_token(&self, logits: &[f32]) -> i64 {
        if self.inference_config.temperature <= 0.0 {
            // Greedy decoding
            self.greedy_decode(logits)
        } else {
            // Temperature sampling
            self.sample_with_temperature(logits, self.inference_config.temperature)
        }
    }

    /// Greedy decoding: select token with highest logit
    fn greedy_decode(&self, logits: &[f32]) -> i64 {
        logits
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(idx, _)| idx as i64)
            .unwrap_or(tokens::EOT)
    }

    /// Temperature sampling with softmax
    fn sample_with_temperature(&self, logits: &[f32], temperature: f32) -> i64 {
        // Apply temperature
        let scaled: Vec<f32> = logits.iter().map(|&x| x / temperature).collect();

        // Compute softmax
        let max_logit = scaled.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let exp_sum: f32 = scaled.iter().map(|&x| (x - max_logit).exp()).sum();
        let probs: Vec<f32> = scaled
            .iter()
            .map(|&x| (x - max_logit).exp() / exp_sum)
            .collect();

        // Sample from distribution
        let random: f32 = rand::random();
        let mut cumsum = 0.0;
        for (idx, &prob) in probs.iter().enumerate() {
            cumsum += prob;
            if cumsum >= random {
                return idx as i64;
            }
        }

        // Fallback to last token
        (probs.len() - 1) as i64
    }

    /// Get model architecture configuration
    pub fn config(&self) -> &ModelConfig {
        &self.config
    }

    /// Get inference configuration
    pub fn inference_config(&self) -> &InferenceConfig {
        &self.inference_config
    }

    /// Update inference configuration
    pub fn set_inference_config(&mut self, config: InferenceConfig) {
        self.inference_config = config;
    }

    /// Get the expected encoder input shape
    pub fn encoder_input_shape(&self) -> (usize, usize, usize) {
        (1, self.config.n_mels, N_FRAMES)
    }

    /// Get the expected encoder output shape
    pub fn encoder_output_shape(&self) -> (usize, usize, usize) {
        (1, self.config.encoder_output_frames, self.config.d_model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_config_base() {
        let config = ModelConfig::base();
        assert_eq!(config.d_model, 512);
        assert_eq!(config.encoder_layers, 6);
        assert_eq!(config.decoder_layers, 6);
        assert_eq!(config.n_heads, 8);
        assert_eq!(config.vocab_size, 51865);
        assert_eq!(config.encoder_output_frames, 1500);
        assert_eq!(config.n_mels, DEFAULT_N_MELS);
    }

    #[test]
    fn test_model_config_tiny() {
        let config = ModelConfig::tiny();
        assert_eq!(config.d_model, 384);
        assert_eq!(config.encoder_layers, 4);
        assert_eq!(config.decoder_layers, 4);
        assert_eq!(config.n_heads, 6);
        assert_eq!(config.n_mels, DEFAULT_N_MELS);
    }

    #[test]
    fn test_model_config_small() {
        let config = ModelConfig::small();
        assert_eq!(config.d_model, 768);
        assert_eq!(config.encoder_layers, 12);
        assert_eq!(config.decoder_layers, 12);
        assert_eq!(config.n_heads, 12);
        assert_eq!(config.n_mels, DEFAULT_N_MELS);
    }

    #[test]
    fn test_model_config_medium() {
        let config = ModelConfig::medium();
        assert_eq!(config.d_model, 1024);
        assert_eq!(config.encoder_layers, 24);
        assert_eq!(config.decoder_layers, 24);
        assert_eq!(config.n_heads, 16);
        assert_eq!(config.n_mels, DEFAULT_N_MELS);
    }

    #[test]
    fn test_model_config_large() {
        let config = ModelConfig::large();
        assert_eq!(config.d_model, 1280);
        assert_eq!(config.encoder_layers, 32);
        assert_eq!(config.decoder_layers, 32);
        assert_eq!(config.n_heads, 20);
        assert_eq!(config.n_mels, DEFAULT_N_MELS);
    }

    #[test]
    fn test_model_config_large_v3() {
        let config = ModelConfig::large_v3();
        assert_eq!(config.d_model, 1280);
        assert_eq!(config.encoder_layers, 32);
        assert_eq!(config.decoder_layers, 32);
        assert_eq!(config.n_heads, 20);
        assert_eq!(config.n_mels, LARGE_V3_N_MELS);
    }

    #[test]
    fn test_inference_config_default() {
        let config = InferenceConfig::default();
        assert_eq!(config.num_threads, 4);
        assert_eq!(config.max_length, 448);
        assert_eq!(config.temperature, 0.0);
        assert!(config.suppress_blank);
        assert!(config.suppress_tokens.is_empty());
    }

    #[test]
    fn test_schema_constants() {
        // Verify encoder schema
        assert_eq!(encoder_schema::INPUT_FEATURES, "input_features");
        assert_eq!(encoder_schema::LAST_HIDDEN_STATE, "last_hidden_state");

        // Verify decoder schema
        assert_eq!(decoder_schema::INPUT_IDS, "input_ids");
        assert_eq!(
            decoder_schema::ENCODER_HIDDEN_STATES,
            "encoder_hidden_states"
        );
        assert_eq!(decoder_schema::LOGITS, "logits");
    }

    #[test]
    fn test_encoder_input_shape() {
        let config = ModelConfig::base();
        assert_eq!(config.n_mels, DEFAULT_N_MELS);
        assert_eq!(N_FRAMES, 3000);
    }

    #[test]
    fn test_greedy_decode_simple() {
        // Test greedy decoding logic without model
        let logits = [0.1, 0.5, 0.3, 0.8, 0.2]; // Token 3 has highest
        let max_idx = logits
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(idx, _)| idx)
            .unwrap();
        assert_eq!(max_idx, 3);
    }

    #[test]
    fn test_greedy_decode_with_neg_infinity() {
        // Test that NEG_INFINITY is handled correctly
        let logits = [f32::NEG_INFINITY, 0.5, f32::NEG_INFINITY, 0.3];
        let max_idx = logits
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(idx, _)| idx)
            .unwrap();
        assert_eq!(max_idx, 1); // Token 1 has highest valid value
    }

    #[test]
    fn test_temperature_scaling() {
        // Test that temperature scaling works correctly
        let logits = [1.0, 2.0, 3.0];
        let temperature = 2.0;
        let scaled: Vec<f32> = logits.iter().map(|&x| x / temperature).collect();
        assert_eq!(scaled, vec![0.5, 1.0, 1.5]);
    }

    #[test]
    fn test_softmax_normalization() {
        // Test softmax produces valid probability distribution
        let logits = [1.0, 2.0, 3.0];
        let max_logit = 3.0f32;
        let exp_sum: f32 = logits.iter().map(|&x| (x - max_logit).exp()).sum();
        let probs: Vec<f32> = logits
            .iter()
            .map(|&x| (x - max_logit).exp() / exp_sum)
            .collect();

        // Sum should be ~1.0
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6);

        // All probs should be positive
        assert!(probs.iter().all(|&p| p > 0.0));

        // Highest logit should have highest prob
        assert!(probs[2] > probs[1]);
        assert!(probs[1] > probs[0]);
    }

    #[test]
    fn test_load_nonexistent_encoder() {
        let result = WhisperModel::load(
            Path::new("/nonexistent/encoder.onnx"),
            Path::new("/nonexistent/decoder.onnx"),
            ModelConfig::base(),
        );
        assert!(result.is_err());
        if let Err(STTError::ConfigurationError(msg)) = result {
            assert!(msg.contains("Encoder model not found"));
            assert!(msg.contains("sayna init"));
        } else {
            panic!("Expected ConfigurationError");
        }
    }

    #[tokio::test]
    async fn test_load_async_nonexistent() {
        let result = WhisperModel::load_async(
            Path::new("/nonexistent/encoder.onnx"),
            Path::new("/nonexistent/decoder.onnx"),
            ModelConfig::base(),
        )
        .await;
        assert!(result.is_err());
    }

    #[test]
    fn test_apply_suppressions_empty() {
        let config = InferenceConfig {
            suppress_tokens: vec![],
            suppress_blank: false,
            ..Default::default()
        };

        let logits = vec![0.1, 0.2, 0.3];
        let _current_tokens = [tokens::SOT];

        // Without a model, test the logic directly
        let mut result = logits.clone();
        for &token in &config.suppress_tokens {
            if (token as usize) < result.len() {
                result[token as usize] = f32::NEG_INFINITY;
            }
        }

        // No suppressions applied
        assert_eq!(result, logits);
    }

    #[test]
    fn test_apply_suppressions_with_tokens() {
        let suppress_tokens = vec![1i64, 2i64];
        let logits = vec![0.1, 0.2, 0.3, 0.4];

        let mut result = logits.clone();
        for &token in &suppress_tokens {
            if (token as usize) < result.len() {
                result[token as usize] = f32::NEG_INFINITY;
            }
        }

        assert_eq!(result[0], 0.1);
        assert_eq!(result[1], f32::NEG_INFINITY);
        assert_eq!(result[2], f32::NEG_INFINITY);
        assert_eq!(result[3], 0.4);
    }
}
