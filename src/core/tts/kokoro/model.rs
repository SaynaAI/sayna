//! ONNX Model Manager for Kokoro TTS
//!
//! Handles loading and running inference on the Kokoro ONNX model.
//! Supports both standard and timestamped model variants.
//!
//! ## Model Variants
//!
//! - **Standard**: Outputs only audio waveform (`audio` tensor)
//! - **Timestamped**: Outputs audio waveform + per-phoneme durations for word alignment
//!
//! ## Execution Providers
//!
//! By default, uses CPU execution provider. CUDA support can be enabled
//! with the `cuda` feature flag (requires CUDA toolkit and appropriate
//! ONNX Runtime build).

use crate::core::tts::{TTSError, TTSResult};
use std::path::Path;
use std::sync::Mutex;

#[cfg(feature = "kokoro-tts")]
use ort::{
    execution_providers::CPUExecutionProvider,
    session::{
        Session, SessionInputValue, SessionInputs,
        builder::{GraphOptimizationLevel, SessionBuilder},
    },
    value::{Tensor, Value},
};
#[cfg(feature = "kokoro-tts")]
use std::borrow::Cow;

/// Model input/output schema constants
mod schema {
    pub const STYLE: &str = "style";
    pub const SPEED: &str = "speed";

    /// Standard model (audio-only output)
    pub mod standard {
        pub const TOKENS: &str = "tokens";
        pub const AUDIO: &str = "audio";
    }

    /// Timestamped model (audio + durations)
    pub mod timestamped {
        pub const TOKENS: &str = "input_ids";
        pub const AUDIO: &str = "waveform";
        #[allow(dead_code)]
        pub const DURATIONS: &str = "durations";
    }
}

/// Model variant based on output capabilities
#[cfg(feature = "kokoro-tts")]
pub enum ModelVariant {
    /// Standard model with audio-only output
    Standard(Session),
    /// Timestamped model with audio and duration outputs
    Timestamped(Session),
}

#[cfg(feature = "kokoro-tts")]
impl ModelVariant {
    fn audio_key(&self) -> &'static str {
        match self {
            ModelVariant::Standard(_) => schema::standard::AUDIO,
            ModelVariant::Timestamped(_) => schema::timestamped::AUDIO,
        }
    }

    fn tokens_key(&self) -> &'static str {
        match self {
            ModelVariant::Standard(_) => schema::standard::TOKENS,
            ModelVariant::Timestamped(_) => schema::timestamped::TOKENS,
        }
    }

    #[allow(dead_code)]
    fn session(&self) -> &Session {
        match self {
            ModelVariant::Standard(s) | ModelVariant::Timestamped(s) => s,
        }
    }

    fn session_mut(&mut self) -> &mut Session {
        match self {
            ModelVariant::Standard(s) | ModelVariant::Timestamped(s) => s,
        }
    }
}

/// Configuration for model loading
#[cfg(feature = "kokoro-tts")]
#[derive(Debug, Clone, Default)]
pub struct ModelConfig {
    /// Number of threads for intra-op parallelism (default: 4)
    pub num_threads: Option<usize>,
}

/// Kokoro ONNX Model wrapper
pub struct KokoroModel {
    #[cfg(feature = "kokoro-tts")]
    inner: Mutex<ModelVariant>,
    #[cfg(not(feature = "kokoro-tts"))]
    _phantom: std::marker::PhantomData<()>,
}

impl KokoroModel {
    /// Load the ONNX model from a file with default configuration
    #[cfg(feature = "kokoro-tts")]
    pub async fn load<P: AsRef<Path>>(path: P) -> TTSResult<Self> {
        Self::load_with_config(path, ModelConfig::default()).await
    }

    /// Load the ONNX model from a file with custom configuration
    ///
    /// # Arguments
    /// * `path` - Path to the ONNX model file
    /// * `config` - Model configuration (threads, optimization level)
    #[cfg(feature = "kokoro-tts")]
    pub async fn load_with_config<P: AsRef<Path>>(path: P, config: ModelConfig) -> TTSResult<Self> {
        let path = path.as_ref().to_path_buf();

        if !path.exists() {
            return Err(TTSError::InvalidConfiguration(format!(
                "Model file not found: {}. Run 'sayna init' to download.",
                path.display()
            )));
        }

        // Load model in blocking task to avoid blocking the async runtime
        let session = tokio::task::spawn_blocking(move || Self::create_session(&path, &config))
            .await
            .map_err(|e| {
                TTSError::InternalError(format!("Failed to spawn model load task: {}", e))
            })??;

        // Determine model variant based on output count
        let output_count = session.outputs.len();
        let variant = if output_count > 1 {
            tracing::info!(
                "Kokoro model loaded: timestamped variant ({} outputs)",
                output_count
            );
            ModelVariant::Timestamped(session)
        } else {
            tracing::info!(
                "Kokoro model loaded: standard variant ({} output)",
                output_count
            );
            ModelVariant::Standard(session)
        };

        Ok(Self {
            inner: Mutex::new(variant),
        })
    }

    /// Load the ONNX model (stub when feature is disabled)
    #[cfg(not(feature = "kokoro-tts"))]
    pub async fn load<P: AsRef<Path>>(_path: P) -> TTSResult<Self> {
        Err(TTSError::InvalidConfiguration(
            "Kokoro TTS feature is not enabled".to_string(),
        ))
    }

    /// Create ONNX session with configuration
    #[cfg(feature = "kokoro-tts")]
    fn create_session(path: &Path, config: &ModelConfig) -> TTSResult<Session> {
        // Get execution providers
        let providers = Self::get_execution_providers();

        // Build session with configuration
        let mut builder = SessionBuilder::new().map_err(|e| {
            TTSError::InternalError(format!("Failed to create session builder: {}", e))
        })?;

        // Apply optimization level (always use Level3 for best performance)
        builder = builder
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| {
                TTSError::InternalError(format!("Failed to set optimization level: {}", e))
            })?;

        // Apply thread configuration (default to 4 threads)
        let num_threads = config.num_threads.unwrap_or(4);
        if num_threads > 0 {
            builder = builder
                .with_intra_threads(num_threads)
                .map_err(|e| {
                    TTSError::InternalError(format!("Failed to set intra threads: {}", e))
                })?
                .with_inter_threads(1)
                .map_err(|e| {
                    TTSError::InternalError(format!("Failed to set inter threads: {}", e))
                })?;
        }

        // Set execution providers
        builder = builder.with_execution_providers(providers).map_err(|e| {
            TTSError::InternalError(format!("Failed to set execution providers: {}", e))
        })?;

        // Load model from file
        let session = builder
            .commit_from_file(path)
            .map_err(|e| TTSError::InternalError(format!("Failed to load model: {}", e)))?;

        // Validate model inputs
        Self::validate_model_inputs(&session)?;

        Ok(session)
    }

    /// Get execution providers based on available features
    #[cfg(feature = "kokoro-tts")]
    fn get_execution_providers() -> Vec<ort::execution_providers::ExecutionProviderDispatch> {
        // TODO: When CUDA feature is added, include CUDAExecutionProvider
        // #[cfg(feature = "cuda")]
        // {
        //     vec![
        //         CUDAExecutionProvider::default().build(),
        //         CPUExecutionProvider::default().build(),
        //     ]
        // }

        // CPU-only for now
        vec![CPUExecutionProvider::default().build()]
    }

    /// Validate model inputs match expected schema
    #[cfg(feature = "kokoro-tts")]
    fn validate_model_inputs(session: &Session) -> TTSResult<()> {
        let inputs = &session.inputs;
        let outputs = &session.outputs;

        tracing::debug!(
            "Model inputs: {:?}",
            inputs.iter().map(|i| &i.name).collect::<Vec<_>>()
        );
        tracing::debug!(
            "Model outputs: {:?}",
            outputs.iter().map(|o| &o.name).collect::<Vec<_>>()
        );

        if inputs.is_empty() {
            return Err(TTSError::InternalError("Model has no inputs".to_string()));
        }

        if outputs.is_empty() {
            return Err(TTSError::InternalError("Model has no outputs".to_string()));
        }

        // Check for required inputs (tokens/input_ids, style, speed)
        let input_names: Vec<&str> = inputs.iter().map(|i| i.name.as_str()).collect();

        let has_tokens = input_names.contains(&schema::standard::TOKENS)
            || input_names.contains(&schema::timestamped::TOKENS);

        if !has_tokens {
            return Err(TTSError::InternalError(format!(
                "Model missing tokens input. Expected '{}' or '{}', found: {:?}",
                schema::standard::TOKENS,
                schema::timestamped::TOKENS,
                input_names
            )));
        }

        if !input_names.contains(&schema::STYLE) {
            return Err(TTSError::InternalError(format!(
                "Model missing '{}' input. Found: {:?}",
                schema::STYLE,
                input_names
            )));
        }

        if !input_names.contains(&schema::SPEED) {
            return Err(TTSError::InternalError(format!(
                "Model missing '{}' input. Found: {:?}",
                schema::SPEED,
                input_names
            )));
        }

        Ok(())
    }

    /// Run inference on the model
    ///
    /// # Arguments
    /// * `tokens` - Token indices (will be batched and padded)
    /// * `style` - Style embedding vector
    /// * `speed` - Speaking rate (1.0 = normal)
    ///
    /// # Returns
    /// Audio samples as f32 vector
    #[cfg(feature = "kokoro-tts")]
    pub async fn infer(
        &mut self,
        tokens: Vec<i64>,
        style: Vec<Vec<f32>>,
        speed: f32,
    ) -> TTSResult<Vec<f32>> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| TTSError::InternalError(format!("Failed to lock model: {}", e)))?;

        let variant = &mut *guard;
        let tokens_key = variant.tokens_key();
        let audio_key = variant.audio_key();

        // Prepare inputs
        let inputs = Self::prepare_inputs(tokens_key, vec![tokens], style, speed)?;

        // Run inference
        let session = variant.session_mut();
        let outputs = session
            .run(SessionInputs::from(inputs))
            .map_err(|e| TTSError::InternalError(format!("Inference failed: {}", e)))?;

        // Extract audio output
        let (shape, data) = outputs[audio_key]
            .try_extract_tensor::<f32>()
            .or_else(|_| {
                outputs
                    .get("waveforms")
                    .map(|v| v.try_extract_tensor::<f32>())
                    .transpose()
                    .ok()
                    .flatten()
                    .ok_or(())
            })
            .map_err(|_| TTSError::InternalError("Could not extract audio output".to_string()))?;

        tracing::debug!(
            "Inference complete: audio shape {:?}, {} samples",
            shape,
            data.len()
        );

        Ok(data.to_vec())
    }

    /// Run inference (stub when feature is disabled)
    #[cfg(not(feature = "kokoro-tts"))]
    pub async fn infer(
        &mut self,
        _tokens: Vec<i64>,
        _style: Vec<Vec<f32>>,
        _speed: f32,
    ) -> TTSResult<Vec<f32>> {
        Err(TTSError::InvalidConfiguration(
            "Kokoro TTS feature is not enabled".to_string(),
        ))
    }

    /// Run inference with timestamps
    #[cfg(feature = "kokoro-tts")]
    #[allow(dead_code)]
    pub async fn infer_with_timestamps(
        &mut self,
        tokens: Vec<i64>,
        style: Vec<Vec<f32>>,
        speed: f32,
    ) -> TTSResult<(Vec<f32>, Option<Vec<f32>>)> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| TTSError::InternalError(format!("Failed to lock model: {}", e)))?;

        let variant = &mut *guard;
        let tokens_key = variant.tokens_key();
        let audio_key = variant.audio_key();

        // Prepare inputs
        let inputs = Self::prepare_inputs(tokens_key, vec![tokens], style, speed)?;

        // Run inference
        let session = variant.session_mut();
        let outputs = session
            .run(SessionInputs::from(inputs))
            .map_err(|e| TTSError::InternalError(format!("Inference failed: {}", e)))?;

        // Extract audio
        let (_, audio_data) = outputs[audio_key]
            .try_extract_tensor::<f32>()
            .map_err(|_| TTSError::InternalError("Could not extract audio output".to_string()))?;

        // Try to extract durations (only available in timestamped model)
        let durations = outputs
            .get(schema::timestamped::DURATIONS)
            .and_then(|v| v.try_extract_tensor::<f32>().ok())
            .map(|(_, d)| d.to_vec());

        Ok((audio_data.to_vec(), durations))
    }

    /// Run inference with timestamps (stub when feature is disabled)
    #[cfg(not(feature = "kokoro-tts"))]
    pub async fn infer_with_timestamps(
        &mut self,
        _tokens: Vec<i64>,
        _style: Vec<Vec<f32>>,
        _speed: f32,
    ) -> TTSResult<(Vec<f32>, Option<Vec<f32>>)> {
        Err(TTSError::InvalidConfiguration(
            "Kokoro TTS feature is not enabled".to_string(),
        ))
    }

    #[cfg(feature = "kokoro-tts")]
    fn prepare_inputs(
        tokens_key: &'static str,
        tokens: Vec<Vec<i64>>,
        styles: Vec<Vec<f32>>,
        speed: f32,
    ) -> TTSResult<Vec<(Cow<'static, str>, SessionInputValue<'static>)>> {
        // Tokens tensor [batch, seq_len]
        let batch_size = tokens.len();
        let seq_len = tokens.first().map(|t| t.len()).unwrap_or(0);
        let tokens_flat: Vec<i64> = tokens.into_iter().flatten().collect();

        let tokens_tensor =
            Tensor::from_array(([batch_size, seq_len], tokens_flat)).map_err(|e| {
                TTSError::InternalError(format!("Failed to create tokens tensor: {}", e))
            })?;

        // Style tensor [batch, style_dim]
        let style_batch_size = styles.len();
        let style_dim = styles.first().map(|s| s.len()).unwrap_or(256);
        let styles_flat: Vec<f32> = styles.into_iter().flatten().collect();

        let style_tensor = Tensor::from_array(([style_batch_size, style_dim], styles_flat))
            .map_err(|e| {
                TTSError::InternalError(format!("Failed to create style tensor: {}", e))
            })?;

        // Speed tensor [1]
        let speed_tensor = Tensor::from_array(([1], vec![speed])).map_err(|e| {
            TTSError::InternalError(format!("Failed to create speed tensor: {}", e))
        })?;

        Ok(vec![
            (
                Cow::Borrowed(tokens_key),
                SessionInputValue::Owned(Value::from(tokens_tensor)),
            ),
            (
                Cow::Borrowed(schema::STYLE),
                SessionInputValue::Owned(Value::from(style_tensor)),
            ),
            (
                Cow::Borrowed(schema::SPEED),
                SessionInputValue::Owned(Value::from(speed_tensor)),
            ),
        ])
    }

    /// Check if this is a timestamped model
    #[cfg(feature = "kokoro-tts")]
    #[allow(dead_code)]
    pub fn supports_timestamps(&self) -> bool {
        if let Ok(guard) = self.inner.lock() {
            matches!(&*guard, ModelVariant::Timestamped(_))
        } else {
            false
        }
    }

    #[cfg(not(feature = "kokoro-tts"))]
    pub fn supports_timestamps(&self) -> bool {
        false
    }

    /// Get model information
    #[cfg(feature = "kokoro-tts")]
    #[allow(dead_code)]
    pub fn get_info(&self) -> serde_json::Value {
        if let Ok(guard) = self.inner.lock() {
            let session = guard.session();
            let input_names: Vec<_> = session.inputs.iter().map(|i| i.name.as_str()).collect();
            let output_names: Vec<_> = session.outputs.iter().map(|o| o.name.as_str()).collect();

            serde_json::json!({
                "inputs": input_names,
                "outputs": output_names,
                "timestamped": matches!(&*guard, ModelVariant::Timestamped(_)),
            })
        } else {
            serde_json::json!({
                "error": "Failed to lock model"
            })
        }
    }

    #[cfg(not(feature = "kokoro-tts"))]
    pub fn get_info(&self) -> serde_json::Value {
        serde_json::json!({
            "error": "Kokoro TTS feature is not enabled"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_constants() {
        // Verify schema constants are correct
        assert_eq!(schema::STYLE, "style");
        assert_eq!(schema::SPEED, "speed");
        assert_eq!(schema::standard::TOKENS, "tokens");
        assert_eq!(schema::standard::AUDIO, "audio");
        assert_eq!(schema::timestamped::TOKENS, "input_ids");
        assert_eq!(schema::timestamped::AUDIO, "waveform");
        assert_eq!(schema::timestamped::DURATIONS, "durations");
    }

    #[cfg(feature = "kokoro-tts")]
    #[test]
    fn test_model_config_default() {
        let config = ModelConfig::default();
        assert_eq!(config.num_threads, None);
    }

    #[cfg(feature = "kokoro-tts")]
    #[test]
    fn test_model_config_custom() {
        let config = ModelConfig {
            num_threads: Some(8),
        };
        assert_eq!(config.num_threads, Some(8));
    }

    #[cfg(feature = "kokoro-tts")]
    #[test]
    fn test_model_variant_keys() {
        // We can't easily create a mock Session, so we test the key methods indirectly
        // by verifying the schema constants they use
        assert_eq!(schema::standard::TOKENS, "tokens");
        assert_eq!(schema::standard::AUDIO, "audio");
        assert_eq!(schema::timestamped::TOKENS, "input_ids");
        assert_eq!(schema::timestamped::AUDIO, "waveform");
    }

    #[tokio::test]
    async fn test_load_nonexistent_file() {
        let result = KokoroModel::load("/nonexistent/path/model.onnx").await;
        assert!(result.is_err());

        if let Err(TTSError::InvalidConfiguration(msg)) = result {
            assert!(msg.contains("Model file not found"));
            assert!(msg.contains("sayna init"));
        } else {
            panic!("Expected InvalidConfiguration error");
        }
    }

    #[cfg(not(feature = "kokoro-tts"))]
    #[tokio::test]
    async fn test_load_disabled_feature() {
        let result = KokoroModel::load("/any/path.onnx").await;
        assert!(result.is_err());

        if let Err(TTSError::InvalidConfiguration(msg)) = result {
            assert!(msg.contains("not enabled"));
        } else {
            panic!("Expected InvalidConfiguration error");
        }
    }

    #[cfg(not(feature = "kokoro-tts"))]
    #[test]
    fn test_supports_timestamps_disabled() {
        // Can't create KokoroModel without feature, so this is a compile-time check
        // that the stub method exists
    }
}
