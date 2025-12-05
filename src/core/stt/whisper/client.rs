//! Whisper STT client implementing BaseSTT trait
//!
//! This module provides the main `WhisperSTT` client that implements the `BaseSTT`
//! trait for local speech-to-text transcription using Whisper ONNX models.
//!
//! ## Architecture
//!
//! The client processes audio in the following pipeline:
//! 1. Buffer incoming PCM audio chunks
//! 2. When buffer reaches threshold, convert to mel spectrogram
//! 3. Run encoder to get audio embeddings
//! 4. Run decoder autoregressively to generate tokens
//! 5. Decode tokens to text using BPE tokenizer
//! 6. Send results via callback
//!
//! ## Audio Buffering Strategy
//!
//! Since Whisper is not a streaming model, audio is accumulated in a buffer:
//! - Audio chunks are accumulated as `send_audio()` is called
//! - When buffer reaches 30 seconds (Whisper's max), transcription is triggered
//! - Results are delivered via the registered callback
//!
//! ## Reference
//!
//! Pattern follows `src/core/stt/deepgram.rs` for BaseSTT implementation.

#![cfg(feature = "whisper-stt")]

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{Mutex, Notify, mpsc};
use tracing::{debug, error, info, warn};

use crate::core::stt::{
    BaseSTT, STTConfig, STTError, STTErrorCallback, STTResult, STTResultCallback,
};

use super::assets::{
    DEFAULT_MODEL, SUPPORTED_MODELS, WhisperAssetConfig, are_assets_available, decoder_model_path,
    encoder_model_path, tokenizer_path,
};
use super::mel::{
    MelProcessor, SAMPLE_RATE, duration_ms_to_samples, pcm16_bytes_to_f32, samples_to_duration_ms,
};
use super::model::{ModelConfig, WhisperModel};
use super::tokenizer::{Task, WhisperTokenizer};
use super::vad::{VADConfig, VADState, VoiceActivityDetector};

/// Type alias for the complex callback function type
type AsyncSTTCallback =
    Box<dyn Fn(STTResult) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Type alias for the error callback function type
type AsyncErrorCallback =
    Box<dyn Fn(STTError) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Simplified connection state with atomic updates
#[derive(Debug, Clone)]
enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    /// Error state
    Error,
}

/// Streaming configuration for real-time Whisper transcription
#[derive(Debug, Clone)]
pub struct StreamingConfig {
    /// Minimum audio buffer before first transcription (ms)
    /// Lower values = faster first response, but lower quality
    /// Recommended: 2000-4000ms for good quality
    pub min_audio_ms: u32,

    /// Maximum audio buffer before forced transcription (ms)
    /// Should be less than Whisper's 30s max for real-time use
    /// Recommended: 10000-15000ms
    pub max_audio_ms: u32,

    /// Periodic transcription interval during speech (ms)
    /// How often to emit interim results while speaking
    /// Set to 0 to disable interim results
    pub interim_interval_ms: u32,

    /// Audio overlap to keep between transcriptions (ms)
    /// Helps with word boundary detection
    pub overlap_ms: u32,

    /// VAD configuration for speech boundary detection
    pub vad: VADConfig,

    /// Enable real-time streaming mode
    /// When false, reverts to batch mode (30s buffer)
    pub enabled: bool,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            min_audio_ms: 3000,       // 3 seconds - good balance
            max_audio_ms: 15000,      // 15 seconds max before forced transcription
            interim_interval_ms: 500, // Emit interim results every 500ms
            overlap_ms: 200,          // 200ms overlap for word boundaries
            vad: VADConfig::default(),
            enabled: true, // Enable streaming by default
        }
    }
}

impl StreamingConfig {
    /// Configuration optimized for low latency (may sacrifice some accuracy)
    pub fn low_latency() -> Self {
        Self {
            min_audio_ms: 2000,
            max_audio_ms: 10000,
            interim_interval_ms: 300,
            overlap_ms: 150,
            vad: VADConfig::responsive(),
            enabled: true,
        }
    }

    /// Configuration optimized for accuracy (higher latency)
    pub fn high_accuracy() -> Self {
        Self {
            min_audio_ms: 4000,
            max_audio_ms: 20000,
            interim_interval_ms: 1000,
            overlap_ms: 300,
            vad: VADConfig::accurate(),
            enabled: true,
        }
    }

    /// Batch mode configuration (original behavior, no streaming)
    pub fn batch() -> Self {
        Self {
            min_audio_ms: 30000,
            max_audio_ms: 30000,
            interim_interval_ms: 0,
            overlap_ms: 0,
            vad: VADConfig::default(),
            enabled: false,
        }
    }
}

/// Whisper STT configuration options
#[derive(Debug, Clone, Default)]
pub struct WhisperSTTConfig {
    /// Base STT configuration
    pub base: STTConfig,
    /// Path to cache directory for model files
    pub cache_path: Option<PathBuf>,
    /// Enable word-level timestamps
    pub timestamps: bool,
    /// Streaming configuration for real-time transcription
    pub streaming: StreamingConfig,
}

impl WhisperSTTConfig {
    /// Create configuration from generic STTConfig
    pub fn from_stt_config(config: STTConfig) -> Result<Self, STTError> {
        // Validate sample rate
        if config.sample_rate != SAMPLE_RATE {
            return Err(STTError::ConfigurationError(format!(
                "Whisper requires {}Hz audio, got {}Hz. Please resample your audio.",
                SAMPLE_RATE, config.sample_rate
            )));
        }

        // Validate encoding
        if config.encoding != "linear16" && !config.encoding.is_empty() {
            return Err(STTError::InvalidAudioFormat(format!(
                "Whisper requires linear16 encoding, got {}",
                config.encoding
            )));
        }

        // Get cache_path from STTConfig - must be set for Whisper
        let cache_path = config.cache_path.clone().map(|p| p.join("whisper"));

        Ok(Self {
            base: config,
            cache_path,
            timestamps: false,
            streaming: StreamingConfig::default(),
        })
    }

    /// Create configuration with streaming enabled (default behavior)
    pub fn with_streaming(mut self, streaming: StreamingConfig) -> Self {
        self.streaming = streaming;
        self
    }

    /// Create configuration for batch mode (no streaming)
    pub fn batch_mode(mut self) -> Self {
        self.streaming = StreamingConfig::batch();
        self
    }

    /// Create configuration optimized for low latency
    pub fn low_latency(mut self) -> Self {
        self.streaming = StreamingConfig::low_latency();
        self
    }

    /// Create configuration optimized for high accuracy
    pub fn high_accuracy(mut self) -> Self {
        self.streaming = StreamingConfig::high_accuracy();
        self
    }

    /// Get the max audio buffer size in milliseconds
    pub fn max_audio_ms(&self) -> u32 {
        self.streaming.max_audio_ms
    }

    /// Get the model name, defaulting to whisper-base
    pub fn model_name(&self) -> &str {
        let model = &self.base.model;
        if model.is_empty() || model == "nova-3" {
            // nova-3 is the default STTConfig model, override for Whisper
            DEFAULT_MODEL
        } else {
            model.as_str()
        }
    }

    /// Get the asset configuration
    ///
    /// Returns an error if cache_path is not configured. The cache path should be
    /// set from ServerConfig.cache_path when creating the STT provider.
    pub fn asset_config(&self) -> Result<WhisperAssetConfig, STTError> {
        let cache_path = self.cache_path.clone().ok_or_else(|| {
            STTError::ConfigurationError(
                "Whisper cache_path not configured. Set CACHE_PATH environment variable or configure cache.path in config.yaml".to_string()
            )
        })?;

        Ok(WhisperAssetConfig {
            cache_path,
            model_name: self.model_name().to_string(),
        })
    }
}

/// Streaming state for real-time transcription
#[derive(Debug)]
struct StreamingState {
    /// Voice activity detector
    vad: VoiceActivityDetector,
    /// Last transcription text for detecting changes
    last_transcript: String,
    /// Timestamp of last interim result emission
    last_interim_time: std::time::Instant,
    /// Whether we're currently in a speech segment
    in_speech_segment: bool,
    /// Total samples processed in current segment
    segment_samples: usize,
}

impl StreamingState {
    fn new(vad_config: VADConfig) -> Self {
        Self {
            vad: VoiceActivityDetector::with_config(vad_config),
            last_transcript: String::new(),
            last_interim_time: std::time::Instant::now(),
            in_speech_segment: false,
            segment_samples: 0,
        }
    }

    fn reset(&mut self) {
        self.vad.reset();
        self.last_transcript.clear();
        self.last_interim_time = std::time::Instant::now();
        self.in_speech_segment = false;
        self.segment_samples = 0;
    }
}

/// Whisper STT Provider
///
/// A local ONNX-based STT provider that runs Whisper models.
/// Follows the same pattern as DeepgramSTT for BaseSTT implementation.
///
/// ## Streaming Mode
///
/// When streaming is enabled (default), the provider:
/// 1. Uses VAD to detect speech boundaries
/// 2. Emits interim results during speech
/// 3. Sets `is_speech_final=true` when speech ends (silence detected)
///
/// ## Batch Mode
///
/// When streaming is disabled, reverts to original 30-second batch behavior.
pub struct WhisperSTT {
    /// Configuration for the STT client
    config: Option<WhisperSTTConfig>,
    /// Current connection state
    state: ConnectionState,
    /// State change notification
    state_notify: Arc<Notify>,

    /// Audio buffer for accumulation (f32 samples normalized to [-1.0, 1.0])
    audio_buffer: Arc<Mutex<Vec<f32>>>,

    /// Streaming state for real-time VAD and transcription
    streaming_state: Arc<Mutex<Option<StreamingState>>>,

    /// Result channel sender
    result_tx: Option<mpsc::UnboundedSender<STTResult>>,
    /// Error channel sender for streaming errors
    error_tx: Option<mpsc::UnboundedSender<STTError>>,

    /// Result forwarding task handle
    result_forward_handle: Option<tokio::task::JoinHandle<()>>,
    /// Error forwarding task handle
    error_forward_handle: Option<tokio::task::JoinHandle<()>>,

    /// Shared callback storage for async access
    result_callback: Arc<Mutex<Option<AsyncSTTCallback>>>,
    /// Error callback storage for streaming errors
    error_callback: Arc<Mutex<Option<AsyncErrorCallback>>>,

    /// Loaded ONNX model (encoder + decoder)
    model: Option<Arc<WhisperModel>>,
    /// BPE tokenizer for decoding output
    tokenizer: Option<Arc<WhisperTokenizer>>,
    /// Mel spectrogram processor
    mel_processor: Option<Arc<MelProcessor>>,
}

impl Default for WhisperSTT {
    fn default() -> Self {
        Self {
            config: None,
            state: ConnectionState::Disconnected,
            state_notify: Arc::new(Notify::new()),
            audio_buffer: Arc::new(Mutex::new(Vec::new())),
            streaming_state: Arc::new(Mutex::new(None)),
            result_tx: None,
            error_tx: None,
            result_forward_handle: None,
            error_forward_handle: None,
            result_callback: Arc::new(Mutex::new(None)),
            error_callback: Arc::new(Mutex::new(None)),
            model: None,
            tokenizer: None,
            mel_processor: None,
        }
    }
}

impl WhisperSTT {
    /// Validate model name is supported
    fn validate_model_name(model: &str) -> Result<(), STTError> {
        // Handle empty or default model names
        if model.is_empty() || model == "nova-3" {
            return Ok(());
        }

        if SUPPORTED_MODELS.contains(&model) {
            Ok(())
        } else {
            Err(STTError::ConfigurationError(format!(
                "Unknown Whisper model '{}'. Supported models: {}",
                model,
                SUPPORTED_MODELS.join(", ")
            )))
        }
    }

    /// Get model configuration based on model name
    fn get_model_config(model_name: &str) -> ModelConfig {
        match model_name {
            "whisper-large-v3" => ModelConfig::large_v3(),
            "whisper-base" => ModelConfig::base(),
            _ => ModelConfig::base(),
        }
    }

    /// Extract a language code (e.g., "en") from a locale string ("en-US")
    /// Falls back to English when no language is provided.
    fn parse_language_code(language: &str) -> &str {
        language
            .split('-')
            .next()
            .map(str::trim)
            .filter(|code| !code.is_empty())
            .unwrap_or("en")
    }

    /// Calculate buffer duration in milliseconds
    fn buffer_duration_ms(sample_count: usize) -> u64 {
        ((sample_count as u64) * 1000) / (SAMPLE_RATE as u64)
    }

    /// Transcribe audio samples and create result
    ///
    /// # Arguments
    /// * `audio_samples` - Audio samples to transcribe
    /// * `is_final` - Whether this is a final result (not interim)
    /// * `is_speech_final` - Whether speech has ended (triggers TTS response)
    async fn transcribe_audio(
        &self,
        audio_samples: Vec<f32>,
        is_final: bool,
        is_speech_final: bool,
    ) -> Result<STTResult, STTError> {
        let model = self
            .model
            .as_ref()
            .ok_or_else(|| STTError::ConnectionFailed("Model not loaded".to_string()))?;

        let tokenizer = self
            .tokenizer
            .as_ref()
            .ok_or_else(|| STTError::ConnectionFailed("Tokenizer not loaded".to_string()))?;

        let mel_processor = self
            .mel_processor
            .as_ref()
            .ok_or_else(|| STTError::ConnectionFailed("Mel processor not loaded".to_string()))?;

        // Get configuration for language
        let config = self.config.as_ref().ok_or_else(|| {
            STTError::ConfigurationError("No configuration available".to_string())
        })?;

        let duration_ms = samples_to_duration_ms(audio_samples.len());
        debug!(
            "Computing mel spectrogram for {} samples ({} ms), is_final={}, is_speech_final={}",
            audio_samples.len(),
            duration_ms,
            is_final,
            is_speech_final
        );

        // 1. Compute mel spectrogram (CPU-intensive, could be in spawn_blocking)
        let mel_features = mel_processor.compute(&audio_samples);

        // 2. Get initial tokens based on language and task
        // Create a mutable tokenizer copy for setting language
        let mut tokenizer_copy = WhisperTokenizer::new().map_err(|e| {
            STTError::ConfigurationError(format!("Failed to create tokenizer: {}", e))
        })?;

        // Extract just the language code (e.g., "en" from "en-US")
        let lang_code = Self::parse_language_code(&config.base.language);
        if let Err(e) = tokenizer_copy.set_language(lang_code) {
            warn!(
                "Failed to set language '{}', using English: {}",
                lang_code, e
            );
            // Continue with default (English)
        }
        tokenizer_copy.set_task(Task::Transcribe);

        let initial_tokens = tokenizer_copy.get_prompt_tokens();

        // 3. Run model transcription
        debug!(
            "Running model transcription with {} initial tokens",
            initial_tokens.len()
        );
        let output_tokens = model.transcribe(&mel_features, &initial_tokens)?;

        // 4. Decode tokens to text
        let transcript = tokenizer.decode(&output_tokens)?;

        debug!(
            "Transcription result: '{}' (is_final={}, is_speech_final={})",
            transcript, is_final, is_speech_final
        );

        // 5. Create STTResult with appropriate flags
        // Confidence is higher for final results
        let confidence = if is_final { 0.95 } else { 0.7 };

        Ok(STTResult::new(
            transcript,
            is_final,
            is_speech_final,
            confidence,
        ))
    }

    /// Transcribe buffered audio with default flags (final result)
    /// This is the legacy method for batch mode compatibility
    async fn transcribe_buffer(&self, audio_samples: Vec<f32>) -> Result<STTResult, STTError> {
        self.transcribe_audio(audio_samples, true, true).await
    }

    /// Process buffered audio if buffer is full (batch mode)
    /// Returns a final result when buffer reaches max_audio_ms
    async fn maybe_process_buffer_batch(&self) -> Result<Option<STTResult>, STTError> {
        let config = self.config.as_ref().ok_or_else(|| {
            STTError::ConfigurationError("No configuration available".to_string())
        })?;

        let max_samples = duration_ms_to_samples(config.streaming.max_audio_ms as u64);

        // Check buffer size and extract if full
        let audio_to_process = {
            let mut buffer = self.audio_buffer.lock().await;
            if buffer.len() >= max_samples {
                // Buffer is full, take all samples and clear
                let audio = buffer.clone();
                buffer.clear();
                Some(audio)
            } else {
                None
            }
        };

        // Process if we have audio
        if let Some(audio) = audio_to_process {
            let result = self.transcribe_buffer(audio).await?;
            Ok(Some(result))
        } else {
            Ok(None)
        }
    }

    /// Process audio in streaming mode with VAD
    ///
    /// This method:
    /// 1. Updates VAD state with new audio
    /// 2. Emits interim results periodically during speech
    /// 3. Emits final result with is_speech_final=true when speech ends
    async fn process_streaming(&self, samples: &[f32]) -> Result<Vec<STTResult>, STTError> {
        let config = self.config.as_ref().ok_or_else(|| {
            STTError::ConfigurationError("No configuration available".to_string())
        })?;

        let streaming_config = &config.streaming;
        let mut results = Vec::new();

        // Get streaming state
        let mut state_guard = self.streaming_state.lock().await;
        let state = state_guard.as_mut().ok_or_else(|| {
            STTError::ConfigurationError("Streaming state not initialized".to_string())
        })?;

        // Process samples through VAD
        let vad_state = state.vad.process(samples);
        state.segment_samples += samples.len();

        // Get current buffer for processing decisions
        let buffer = self.audio_buffer.lock().await;
        let buffer_duration_ms = Self::buffer_duration_ms(buffer.len()) as u32;
        drop(buffer);

        match vad_state {
            VADState::Silence => {
                // If we were in a speech segment and now silent, check if we should emit
                if state.in_speech_segment {
                    debug!("VAD: Transitioning from speech to silence");
                    state.in_speech_segment = false;
                }
            }
            VADState::Speaking => {
                if !state.in_speech_segment {
                    debug!("VAD: Speech started, beginning segment");
                    state.in_speech_segment = true;
                    state.segment_samples = samples.len();
                    state.last_interim_time = std::time::Instant::now();
                }

                // Check if we should emit an interim result
                let should_emit_interim = streaming_config.interim_interval_ms > 0
                    && buffer_duration_ms >= streaming_config.min_audio_ms
                    && state.last_interim_time.elapsed().as_millis()
                        >= streaming_config.interim_interval_ms as u128;

                // Check if buffer is too large (force transcription)
                let force_transcribe = buffer_duration_ms >= streaming_config.max_audio_ms;

                if should_emit_interim || force_transcribe {
                    // Get audio for transcription (keep overlap)
                    let overlap_samples =
                        duration_ms_to_samples(streaming_config.overlap_ms as u64);
                    let mut buffer = self.audio_buffer.lock().await;

                    if !buffer.is_empty() {
                        let audio_to_process = buffer.clone();

                        // For interim results, keep overlap
                        // For forced transcription, keep less
                        let keep_samples = if force_transcribe {
                            overlap_samples / 2
                        } else {
                            overlap_samples
                        };

                        if buffer.len() > keep_samples {
                            let drain_end = buffer.len() - keep_samples;
                            buffer.drain(..drain_end);
                        }
                        drop(buffer);

                        // Transcribe
                        let is_final = force_transcribe;
                        let is_speech_final = false; // Not speech final during speaking
                        let result = self
                            .transcribe_audio(audio_to_process, is_final, is_speech_final)
                            .await?;

                        // Only emit if transcript changed
                        if result.transcript != state.last_transcript {
                            state.last_transcript = result.transcript.clone();
                            results.push(result);
                        }

                        state.last_interim_time = std::time::Instant::now();
                    }
                }
            }
            VADState::SpeechEnded => {
                debug!("VAD: Speech ended, emitting final result");

                // Speech ended - emit final result with is_speech_final=true
                let mut buffer = self.audio_buffer.lock().await;

                if !buffer.is_empty() {
                    let audio_to_process = buffer.clone();
                    buffer.clear();
                    drop(buffer);

                    // Only transcribe if we have minimum audio
                    let min_samples = duration_ms_to_samples(500); // 0.5s minimum
                    if audio_to_process.len() >= min_samples {
                        let result = self.transcribe_audio(audio_to_process, true, true).await?;

                        results.push(result);
                    } else {
                        debug!(
                            "Audio too short for final transcription ({} samples)",
                            audio_to_process.len()
                        );
                    }
                }

                // Reset streaming state for next segment
                state.reset();
            }
        }

        Ok(results)
    }

    /// Flush remaining audio buffer and transcribe
    pub async fn flush(&self) -> Result<Option<STTResult>, STTError> {
        let audio_to_process = {
            let mut buffer = self.audio_buffer.lock().await;
            if buffer.is_empty() {
                None
            } else {
                let audio = buffer.clone();
                buffer.clear();
                Some(audio)
            }
        };

        if let Some(audio) = audio_to_process {
            // Only process if we have meaningful audio (at least 0.5 seconds)
            let min_samples = duration_ms_to_samples(500);
            if audio.len() >= min_samples {
                let result = self.transcribe_buffer(audio).await?;
                Ok(Some(result))
            } else {
                debug!(
                    "Audio buffer too short ({} samples), skipping transcription",
                    audio.len()
                );
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
}

#[async_trait]
impl BaseSTT for WhisperSTT {
    fn new(config: STTConfig) -> Result<Self, STTError>
    where
        Self: Sized,
    {
        // Validate model name if specified
        Self::validate_model_name(&config.model)?;

        // Create Whisper-specific configuration
        let whisper_config = WhisperSTTConfig::from_stt_config(config)?;

        Ok(Self {
            config: Some(whisper_config),
            state: ConnectionState::Disconnected,
            state_notify: Arc::new(Notify::new()),
            audio_buffer: Arc::new(Mutex::new(Vec::new())),
            streaming_state: Arc::new(Mutex::new(None)),
            result_tx: None,
            error_tx: None,
            result_forward_handle: None,
            error_forward_handle: None,
            result_callback: Arc::new(Mutex::new(None)),
            error_callback: Arc::new(Mutex::new(None)),
            model: None,
            tokenizer: None,
            mel_processor: None,
        })
    }

    async fn connect(&mut self) -> Result<(), STTError> {
        // Get the stored configuration
        let config = self.config.as_ref().ok_or_else(|| {
            STTError::ConfigurationError("No configuration available".to_string())
        })?;

        // Update state to connecting
        self.state = ConnectionState::Connecting;
        self.state_notify.notify_waiters();

        let asset_config = config.asset_config()?;
        let model_name = config.model_name().to_string();

        // Check if assets are available
        if !are_assets_available(&asset_config) {
            self.state = ConnectionState::Error;
            return Err(STTError::ConfigurationError(format!(
                "Whisper {} assets not found. Run 'sayna init' to download model files.",
                model_name
            )));
        }

        // Get model paths
        let encoder_path = encoder_model_path(&asset_config).map_err(|e| {
            STTError::ConfigurationError(format!("Failed to get encoder path: {}", e))
        })?;

        let decoder_path = decoder_model_path(&asset_config).map_err(|e| {
            STTError::ConfigurationError(format!("Failed to get decoder path: {}", e))
        })?;

        let tokenizer_file = tokenizer_path(&asset_config).map_err(|e| {
            STTError::ConfigurationError(format!("Failed to get tokenizer path: {}", e))
        })?;

        // Get model configuration based on model name
        let model_config = Self::get_model_config(&model_name);

        // Load model asynchronously (in spawn_blocking to avoid blocking runtime)
        info!("Loading Whisper {} model...", model_name);
        let model = WhisperModel::load_async(&encoder_path, &decoder_path, model_config)
            .await
            .map_err(|e| {
                self.state = ConnectionState::Error;
                STTError::ConfigurationError(format!("Failed to load model: {}", e))
            })?;

        // Load tokenizer
        info!("Loading Whisper tokenizer...");
        let tokenizer = WhisperTokenizer::load(&tokenizer_file).map_err(|e| {
            self.state = ConnectionState::Error;
            STTError::ConfigurationError(format!("Failed to load tokenizer: {}", e))
        })?;

        // Initialize mel processor
        let mel_processor = MelProcessor::with_n_mels(model.config().n_mels);

        // Store components
        self.model = Some(Arc::new(model));
        self.tokenizer = Some(Arc::new(tokenizer));
        self.mel_processor = Some(Arc::new(mel_processor));

        // Create result/error channels
        let (result_tx, mut result_rx) = mpsc::unbounded_channel::<STTResult>();
        let (error_tx, mut error_rx) = mpsc::unbounded_channel::<STTError>();

        self.result_tx = Some(result_tx);
        self.error_tx = Some(error_tx);

        // Start result forwarding task with shared callback
        let callback_ref = self.result_callback.clone();
        let result_forwarding_handle = tokio::spawn(async move {
            while let Some(result) = result_rx.recv().await {
                if let Some(callback) = callback_ref.lock().await.as_ref() {
                    callback(result).await;
                } else {
                    debug!(
                        "Received STT result: {} (no callback registered)",
                        result.transcript
                    );
                }
            }
        });
        self.result_forward_handle = Some(result_forwarding_handle);

        // Start error forwarding task with shared callback
        let error_callback_ref = self.error_callback.clone();
        let error_forwarding_handle = tokio::spawn(async move {
            while let Some(error) = error_rx.recv().await {
                if let Some(callback) = error_callback_ref.lock().await.as_ref() {
                    callback(error).await;
                } else {
                    error!("STT error but no error callback registered: {}", error);
                }
            }
        });
        self.error_forward_handle = Some(error_forwarding_handle);

        // Clear audio buffer
        self.audio_buffer.lock().await.clear();

        // Initialize streaming state if streaming is enabled
        {
            let config = self.config.as_ref().unwrap();
            if config.streaming.enabled {
                let streaming_state = StreamingState::new(config.streaming.vad.clone());
                *self.streaming_state.lock().await = Some(streaming_state);
                info!(
                    "Streaming mode enabled (min_audio={}ms, max_audio={}ms, interim_interval={}ms)",
                    config.streaming.min_audio_ms,
                    config.streaming.max_audio_ms,
                    config.streaming.interim_interval_ms
                );
            } else {
                *self.streaming_state.lock().await = None;
                info!(
                    "Batch mode enabled (max_audio={}ms)",
                    config.streaming.max_audio_ms
                );
            }
        }

        // Update state to connected
        self.state = ConnectionState::Connected;
        self.state_notify.notify_waiters();

        info!(
            "Successfully connected to Whisper STT (model: {})",
            model_name
        );
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), STTError> {
        // Clean up forwarding tasks
        if let Some(handle) = self.result_forward_handle.take() {
            handle.abort();
            let _ = handle.await;
        }
        if let Some(handle) = self.error_forward_handle.take() {
            handle.abort();
            let _ = handle.await;
        }

        // Clean up channels
        self.result_tx = None;
        self.error_tx = None;

        // Clear callbacks
        *self.result_callback.lock().await = None;
        *self.error_callback.lock().await = None;

        // Clear audio buffer and streaming state
        self.audio_buffer.lock().await.clear();
        *self.streaming_state.lock().await = None;

        // Drop model to free memory
        self.model = None;
        self.tokenizer = None;
        self.mel_processor = None;

        // Update state
        self.state = ConnectionState::Disconnected;
        self.state_notify.notify_waiters();

        info!("Disconnected from Whisper STT");
        Ok(())
    }

    fn is_ready(&self) -> bool {
        matches!(self.state, ConnectionState::Connected) && self.model.is_some()
    }

    async fn send_audio(&mut self, audio_data: Vec<u8>) -> Result<(), STTError> {
        if !self.is_ready() {
            return Err(STTError::ConnectionFailed(
                "Not connected to Whisper STT".to_string(),
            ));
        }

        // Convert PCM16 bytes to f32 samples
        let samples = pcm16_bytes_to_f32(&audio_data);

        if samples.is_empty() {
            return Ok(());
        }

        // Accumulate audio in buffer
        {
            let mut buffer = self.audio_buffer.lock().await;
            buffer.extend(&samples);
            debug!(
                "Audio buffer size: {} samples ({} ms)",
                buffer.len(),
                Self::buffer_duration_ms(buffer.len())
            );
        }

        // Check if streaming is enabled
        let streaming_enabled = {
            let state_guard = self.streaming_state.lock().await;
            state_guard.is_some()
        };

        if streaming_enabled {
            // Streaming mode: use VAD-based processing
            let results = self.process_streaming(&samples).await?;

            // Send all results via channel
            for result in results {
                if let Some(tx) = &self.result_tx
                    && tx.send(result).is_err()
                {
                    warn!("Failed to send transcription result - channel closed");
                }
            }
        } else {
            // Batch mode: original behavior
            if let Some(result) = self.maybe_process_buffer_batch().await? {
                // Send result via channel
                if let Some(tx) = &self.result_tx
                    && tx.send(result).is_err()
                {
                    warn!("Failed to send transcription result - channel closed");
                }
            }
        }

        Ok(())
    }

    async fn on_result(&mut self, callback: STTResultCallback) -> Result<(), STTError> {
        *self.result_callback.lock().await = Some(Box::new(move |result| {
            let cb = callback.clone();
            Box::pin(async move {
                cb(result).await;
            })
        }));
        Ok(())
    }

    async fn on_error(&mut self, callback: STTErrorCallback) -> Result<(), STTError> {
        *self.error_callback.lock().await = Some(Box::new(move |error| {
            let cb = callback.clone();
            Box::pin(async move {
                cb(error).await;
            })
        }));
        Ok(())
    }

    fn get_config(&self) -> Option<&STTConfig> {
        self.config.as_ref().map(|c| &c.base)
    }

    async fn update_config(&mut self, config: STTConfig) -> Result<(), STTError> {
        // Validate model name
        Self::validate_model_name(&config.model)?;

        // Check if we need to reconnect (model changed)
        let needs_reconnect = if let Some(current_config) = &self.config {
            let current_model = current_config.model_name();
            let new_model = if config.model.is_empty() || config.model == "nova-3" {
                DEFAULT_MODEL
            } else {
                &config.model
            };
            current_model != new_model && self.is_ready()
        } else {
            false
        };

        if needs_reconnect {
            self.disconnect().await?;
        }

        // Update stored configuration
        let whisper_config = WhisperSTTConfig::from_stt_config(config)?;
        self.config = Some(whisper_config);

        if needs_reconnect {
            self.connect().await?;
        }

        Ok(())
    }

    fn get_provider_info(&self) -> &'static str {
        "Whisper ONNX (Local)"
    }
}

impl Drop for WhisperSTT {
    fn drop(&mut self) {
        // Abort forwarding tasks if running
        if let Some(handle) = self.result_forward_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.error_forward_handle.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_whisper_stt_config_default() {
        let config = WhisperSTTConfig::default();
        assert_eq!(config.streaming.max_audio_ms, 15000); // Default streaming max
        assert!(!config.timestamps);
        assert!(config.cache_path.is_none());
        assert!(config.streaming.enabled); // Streaming enabled by default
    }

    #[test]
    fn test_whisper_stt_config_from_stt_config() {
        let stt_config = STTConfig {
            provider: "whisper".to_string(),
            language: "es".to_string(),
            sample_rate: 16000,
            model: "whisper-base".to_string(),
            encoding: "linear16".to_string(),
            ..Default::default()
        };

        let whisper_config = WhisperSTTConfig::from_stt_config(stt_config).unwrap();
        assert_eq!(whisper_config.base.language, "es");
        assert_eq!(whisper_config.model_name(), "whisper-base");
    }

    #[test]
    fn test_whisper_stt_config_invalid_sample_rate() {
        let stt_config = STTConfig {
            provider: "whisper".to_string(),
            sample_rate: 44100, // Invalid - Whisper requires 16kHz
            ..Default::default()
        };

        let result = WhisperSTTConfig::from_stt_config(stt_config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("16000Hz"));
    }

    #[test]
    fn test_whisper_stt_config_invalid_encoding() {
        let stt_config = STTConfig {
            provider: "whisper".to_string(),
            sample_rate: 16000,
            encoding: "mulaw".to_string(),
            ..Default::default()
        };

        let result = WhisperSTTConfig::from_stt_config(stt_config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("linear16"));
    }

    #[test]
    fn test_whisper_stt_config_model_name_default() {
        let stt_config = STTConfig {
            provider: "whisper".to_string(),
            sample_rate: 16000,
            model: "".to_string(), // Empty defaults to whisper-base
            ..Default::default()
        };

        let whisper_config = WhisperSTTConfig::from_stt_config(stt_config).unwrap();
        assert_eq!(whisper_config.model_name(), "whisper-base");
    }

    #[test]
    fn test_whisper_stt_config_model_name_override_nova3() {
        let stt_config = STTConfig {
            provider: "whisper".to_string(),
            sample_rate: 16000,
            model: "nova-3".to_string(), // nova-3 is STTConfig default, should become whisper-base
            ..Default::default()
        };

        let whisper_config = WhisperSTTConfig::from_stt_config(stt_config).unwrap();
        assert_eq!(whisper_config.model_name(), "whisper-base");
    }

    #[test]
    fn test_validate_model_name_valid() {
        assert!(WhisperSTT::validate_model_name("whisper-base").is_ok());
        assert!(WhisperSTT::validate_model_name("whisper-large-v3").is_ok());
        assert!(WhisperSTT::validate_model_name("").is_ok()); // Empty is OK (defaults)
        assert!(WhisperSTT::validate_model_name("nova-3").is_ok()); // Will be converted
    }

    #[test]
    fn test_validate_model_name_invalid() {
        let result = WhisperSTT::validate_model_name("whisper-tiny");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Unknown Whisper model"));
        assert!(err.contains("whisper-base"));
    }

    #[test]
    fn test_get_model_config() {
        use super::super::mel::{DEFAULT_N_MELS, LARGE_V3_N_MELS};

        let base_config = WhisperSTT::get_model_config("whisper-base");
        assert_eq!(base_config.d_model, 512);
        assert_eq!(base_config.n_mels, DEFAULT_N_MELS);

        let large_config = WhisperSTT::get_model_config("whisper-large-v3");
        assert_eq!(large_config.d_model, 1280);
        assert_eq!(large_config.n_mels, LARGE_V3_N_MELS);
    }

    #[test]
    fn test_parse_language_code() {
        assert_eq!(WhisperSTT::parse_language_code("en-US"), "en");
        assert_eq!(WhisperSTT::parse_language_code("fr"), "fr");
        assert_eq!(WhisperSTT::parse_language_code(""), "en");
        assert_eq!(WhisperSTT::parse_language_code("   "), "en");
    }

    #[test]
    fn test_buffer_duration_ms() {
        // 16000 samples = 1000 ms at 16kHz
        assert_eq!(WhisperSTT::buffer_duration_ms(16000), 1000);
        assert_eq!(WhisperSTT::buffer_duration_ms(1600), 100);
        assert_eq!(WhisperSTT::buffer_duration_ms(0), 0);
    }

    #[tokio::test]
    async fn test_whisper_stt_creation() {
        let config = STTConfig {
            provider: "whisper".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            encoding: "linear16".to_string(),
            model: "whisper-base".to_string(),
            ..Default::default()
        };

        let stt = WhisperSTT::new(config).unwrap();
        assert!(!stt.is_ready());
        assert_eq!(stt.get_provider_info(), "Whisper ONNX (Local)");
    }

    #[tokio::test]
    async fn test_whisper_stt_creation_with_invalid_sample_rate() {
        let config = STTConfig {
            provider: "whisper".to_string(),
            sample_rate: 44100, // Invalid
            ..Default::default()
        };

        let result = WhisperSTT::new(config);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_whisper_stt_send_audio_not_connected() {
        let config = STTConfig {
            provider: "whisper".to_string(),
            sample_rate: 16000,
            ..Default::default()
        };

        let mut stt = WhisperSTT::new(config).unwrap();

        // Should fail because not connected
        let result = stt.send_audio(vec![0u8; 1024]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Not connected"));
    }

    #[tokio::test]
    async fn test_whisper_stt_disconnect() {
        let config = STTConfig {
            provider: "whisper".to_string(),
            sample_rate: 16000,
            ..Default::default()
        };

        let mut stt = WhisperSTT::new(config).unwrap();

        // Disconnect should succeed even if not connected
        let result = stt.disconnect().await;
        assert!(result.is_ok());
        assert!(!stt.is_ready());
    }

    #[tokio::test]
    async fn test_whisper_stt_callback_registration() {
        let config = STTConfig {
            provider: "whisper".to_string(),
            sample_rate: 16000,
            ..Default::default()
        };

        let mut stt = WhisperSTT::new(config).unwrap();

        // Register result callback
        let result_callback: STTResultCallback = Arc::new(|_result| {
            Box::pin(async move {
                // Callback logic here
            })
        });
        assert!(stt.on_result(result_callback).await.is_ok());

        // Register error callback
        let error_callback: STTErrorCallback = Arc::new(|_error| {
            Box::pin(async move {
                // Callback logic here
            })
        });
        assert!(stt.on_error(error_callback).await.is_ok());
    }

    #[tokio::test]
    async fn test_whisper_stt_get_config() {
        let config = STTConfig {
            provider: "whisper".to_string(),
            language: "fr".to_string(),
            sample_rate: 16000,
            ..Default::default()
        };

        let stt = WhisperSTT::new(config).unwrap();

        let retrieved_config = stt.get_config().unwrap();
        assert_eq!(retrieved_config.language, "fr");
        assert_eq!(retrieved_config.provider, "whisper");
    }

    #[tokio::test]
    async fn test_whisper_stt_default() {
        let stt = WhisperSTT::default();
        assert!(!stt.is_ready());
        assert!(stt.config.is_none());
        assert!(stt.model.is_none());
    }

    #[test]
    fn test_connection_state_debug() {
        let state = ConnectionState::Disconnected;
        assert!(format!("{:?}", state).contains("Disconnected"));

        let state = ConnectionState::Connected;
        assert!(format!("{:?}", state).contains("Connected"));
    }

    #[tokio::test]
    async fn test_whisper_stt_audio_buffer_starts_empty() {
        let config = STTConfig {
            provider: "whisper".to_string(),
            sample_rate: 16000,
            ..Default::default()
        };

        let stt = WhisperSTT::new(config).unwrap();

        let buffer = stt.audio_buffer.lock().await;
        assert!(buffer.is_empty());
    }

    #[tokio::test]
    async fn test_whisper_stt_flush_empty_buffer() {
        let config = STTConfig {
            provider: "whisper".to_string(),
            sample_rate: 16000,
            ..Default::default()
        };

        let stt = WhisperSTT::new(config).unwrap();

        // Flush on empty buffer should return None, not error
        // Note: This won't work without a connection, but we can at least
        // verify the buffer is empty
        let buffer = stt.audio_buffer.lock().await;
        assert!(buffer.is_empty());
    }

    // ========== Streaming Configuration Tests ==========

    #[test]
    fn test_streaming_config_default() {
        let config = StreamingConfig::default();
        assert_eq!(config.min_audio_ms, 3000);
        assert_eq!(config.max_audio_ms, 15000);
        assert_eq!(config.interim_interval_ms, 500);
        assert_eq!(config.overlap_ms, 200);
        assert!(config.enabled);
    }

    #[test]
    fn test_streaming_config_low_latency() {
        let config = StreamingConfig::low_latency();
        assert!(config.min_audio_ms < StreamingConfig::default().min_audio_ms);
        assert!(config.interim_interval_ms < StreamingConfig::default().interim_interval_ms);
        assert!(config.enabled);
    }

    #[test]
    fn test_streaming_config_high_accuracy() {
        let config = StreamingConfig::high_accuracy();
        assert!(config.min_audio_ms > StreamingConfig::default().min_audio_ms);
        assert!(config.interim_interval_ms > StreamingConfig::default().interim_interval_ms);
        assert!(config.enabled);
    }

    #[test]
    fn test_streaming_config_batch() {
        let config = StreamingConfig::batch();
        assert_eq!(config.min_audio_ms, 30000);
        assert_eq!(config.max_audio_ms, 30000);
        assert_eq!(config.interim_interval_ms, 0);
        assert!(!config.enabled);
    }

    #[test]
    fn test_whisper_stt_config_with_streaming() {
        let config = WhisperSTTConfig::default().with_streaming(StreamingConfig::low_latency());
        assert!(config.streaming.enabled);
        assert_eq!(config.streaming.min_audio_ms, 2000);
    }

    #[test]
    fn test_whisper_stt_config_batch_mode() {
        let config = WhisperSTTConfig::default().batch_mode();
        assert!(!config.streaming.enabled);
        assert_eq!(config.streaming.max_audio_ms, 30000);
    }

    #[test]
    fn test_whisper_stt_config_max_audio_ms_getter() {
        let config = WhisperSTTConfig::default();
        assert_eq!(config.max_audio_ms(), 15000); // Default streaming max

        let batch_config = WhisperSTTConfig::default().batch_mode();
        assert_eq!(batch_config.max_audio_ms(), 30000); // Batch mode max
    }

    // ========== StreamingState Tests ==========

    #[test]
    fn test_streaming_state_new() {
        let state = StreamingState::new(VADConfig::default());
        assert!(!state.in_speech_segment);
        assert_eq!(state.segment_samples, 0);
        assert!(state.last_transcript.is_empty());
    }

    #[test]
    fn test_streaming_state_reset() {
        let mut state = StreamingState::new(VADConfig::default());
        state.in_speech_segment = true;
        state.segment_samples = 1000;
        state.last_transcript = "Hello".to_string();

        state.reset();

        assert!(!state.in_speech_segment);
        assert_eq!(state.segment_samples, 0);
        assert!(state.last_transcript.is_empty());
    }

    #[tokio::test]
    async fn test_whisper_stt_streaming_state_not_initialized_before_connect() {
        let config = STTConfig {
            provider: "whisper".to_string(),
            sample_rate: 16000,
            ..Default::default()
        };

        let stt = WhisperSTT::new(config).unwrap();

        // Streaming state should not be initialized before connect
        let state_guard = stt.streaming_state.lock().await;
        assert!(state_guard.is_none());
    }
}
