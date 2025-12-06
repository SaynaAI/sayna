//! Whisper STT client implementing BaseSTT trait
//!
//! This module provides the main `WhisperSTT` client that implements the `BaseSTT`
//! trait for local speech-to-text transcription using Whisper ONNX models.
//!
//! ## Architecture
//!
//! The client processes audio in the following pipeline:
//! 1. Buffer incoming PCM audio chunks via `send_audio()`
//! 2. Every 500ms, run transcription and emit interim results
//! 3. Use VAD to detect 1.5 seconds of silence for end-of-speech
//! 4. On speech end, emit final result and clear buffer
//!
//! ## Reference
//!
//! Pattern follows `src/core/stt/deepgram.rs` for BaseSTT implementation.

#![cfg(feature = "whisper-stt")]

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

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

/// Interval for emitting interim transcription results (milliseconds)
const INTERIM_INTERVAL_MS: u64 = 200;

/// Silence duration to detect end of speech (milliseconds)
const SILENCE_TIMEOUT_MS: u32 = 1500;

/// Minimum audio duration before first transcription (milliseconds)
const MIN_AUDIO_MS: u64 = 100;

/// Type alias for the async result callback
type AsyncSTTCallback =
    Box<dyn Fn(STTResult) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Type alias for the async error callback
type AsyncErrorCallback =
    Box<dyn Fn(STTError) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Connection state
#[derive(Debug, Clone)]
enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

/// Whisper STT configuration options
#[derive(Debug, Clone, Default)]
pub struct WhisperSTTConfig {
    /// Base STT configuration
    pub base: STTConfig,
    /// Path to cache directory for model files
    pub cache_path: Option<PathBuf>,
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

        let cache_path = config.cache_path.clone().map(|p| p.join("whisper"));

        Ok(Self {
            base: config,
            cache_path,
        })
    }

    /// Get the model name, defaulting to whisper-base
    pub fn model_name(&self) -> &str {
        let model = &self.base.model;
        if model.is_empty() || model == "nova-3" {
            DEFAULT_MODEL
        } else {
            model.as_str()
        }
    }

    /// Get the asset configuration
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

/// Whisper STT Provider
///
/// A local ONNX-based STT provider that runs Whisper models.
///
/// ## Transcription Flow
///
/// 1. Audio chunks are accumulated in a buffer via `send_audio()`
/// 2. Every 500ms, the model runs and emits interim results
/// 3. VAD detects 1.5 seconds of silence to mark end of speech
/// 4. On speech end, emits final result with `is_speech_final=true` and clears buffer
pub struct WhisperSTT {
    /// Configuration
    config: Option<WhisperSTTConfig>,
    /// Connection state
    state: ConnectionState,
    /// State change notification
    state_notify: Arc<Notify>,

    /// Audio buffer (f32 samples normalized to [-1.0, 1.0])
    audio_buffer: Arc<Mutex<Vec<f32>>>,

    /// Voice activity detector for silence detection
    vad: Arc<Mutex<VoiceActivityDetector>>,
    /// Last transcription time for 500ms interval
    last_transcription_time: Arc<Mutex<Instant>>,
    /// Last transcript text for deduplication
    last_transcript: Arc<Mutex<String>>,

    /// Result channel sender
    result_tx: Option<mpsc::UnboundedSender<STTResult>>,
    /// Error channel sender
    error_tx: Option<mpsc::UnboundedSender<STTError>>,

    /// Result forwarding task handle
    result_forward_handle: Option<tokio::task::JoinHandle<()>>,
    /// Error forwarding task handle
    error_forward_handle: Option<tokio::task::JoinHandle<()>>,

    /// Shared callback storage
    result_callback: Arc<Mutex<Option<AsyncSTTCallback>>>,
    /// Error callback storage
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
            vad: Arc::new(Mutex::new(VoiceActivityDetector::with_config(VADConfig {
                silence_duration_ms: SILENCE_TIMEOUT_MS,
                ..Default::default()
            }))),
            last_transcription_time: Arc::new(Mutex::new(Instant::now())),
            last_transcript: Arc::new(Mutex::new(String::new())),
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

    /// Extract language code from locale string (e.g., "en" from "en-US")
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

    /// Filter out special Whisper annotations like [silence], [ Silence ], [music], etc.
    ///
    /// Whisper outputs these bracketed/parenthesized annotations for non-speech audio.
    /// We filter them out since they're not actual speech content.
    /// Handles variations with spaces inside brackets: [silence], [ silence ], [ Silence ], etc.
    fn filter_special_annotations(transcript: &str) -> String {
        use regex::Regex;

        // Keywords to filter (case-insensitive, with optional spaces inside brackets/parens)
        let keywords = [
            "silence",
            "music",
            "applause",
            "laughter",
            "noise",
            "blank_audio",
            "blank audio",
            "inaudible",
        ];

        let mut result = transcript.to_string();

        for keyword in &keywords {
            // Match [keyword] or [ keyword ] with optional spaces, case-insensitive
            let bracket_pattern = format!(r"(?i)\[\s*{}\s*\]", regex::escape(keyword));
            if let Ok(re) = Regex::new(&bracket_pattern) {
                result = re.replace_all(&result, "").to_string();
            }

            // Match (keyword) or ( keyword ) with optional spaces, case-insensitive
            let paren_pattern = format!(r"(?i)\(\s*{}\s*\)", regex::escape(keyword));
            if let Ok(re) = Regex::new(&paren_pattern) {
                result = re.replace_all(&result, "").to_string();
            }
        }

        // Clean up whitespace (multiple spaces, leading/trailing)
        result.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Transcribe audio samples and create result
    async fn transcribe_audio(
        &self,
        audio_samples: &[f32],
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

        let config = self.config.as_ref().ok_or_else(|| {
            STTError::ConfigurationError("No configuration available".to_string())
        })?;

        let duration_ms = samples_to_duration_ms(audio_samples.len());
        debug!(
            "Transcribing {} samples ({} ms), is_final={}, is_speech_final={}",
            audio_samples.len(),
            duration_ms,
            is_final,
            is_speech_final
        );

        // 1. Compute mel spectrogram
        let mel_features = mel_processor.compute(audio_samples);

        // 2. Set up tokenizer with language
        let mut tokenizer_copy = WhisperTokenizer::new().map_err(|e| {
            STTError::ConfigurationError(format!("Failed to create tokenizer: {}", e))
        })?;

        let lang_code = Self::parse_language_code(&config.base.language);
        if let Err(e) = tokenizer_copy.set_language(lang_code) {
            warn!(
                "Failed to set language '{}', using English: {}",
                lang_code, e
            );
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
        let raw_transcript = tokenizer.decode(&output_tokens)?;

        // 5. Filter out special annotations like [silence], [music], etc.
        let transcript = Self::filter_special_annotations(&raw_transcript);

        debug!(
            "Transcription: '{}' (raw: '{}', is_final={}, is_speech_final={})",
            transcript, raw_transcript, is_final, is_speech_final
        );

        let confidence = if is_final { 0.95 } else { 0.7 };

        Ok(STTResult::new(
            transcript,
            is_final,
            is_speech_final,
            confidence,
        ))
    }

    /// Process audio and emit results based on timing and VAD
    async fn process_audio(&self, new_samples: &[f32]) -> Result<(), STTError> {
        // Update VAD with new samples
        let vad_state = {
            let mut vad = self.vad.lock().await;
            vad.process(new_samples)
        };

        // Get current buffer state
        let buffer = self.audio_buffer.lock().await;
        let buffer_duration_ms = Self::buffer_duration_ms(buffer.len());
        drop(buffer);

        // Check if we should transcribe based on 500ms interval
        let should_transcribe_interim = {
            let last_time = self.last_transcription_time.lock().await;
            last_time.elapsed().as_millis() >= INTERIM_INTERVAL_MS as u128
                && buffer_duration_ms >= MIN_AUDIO_MS
        };

        // Handle based on VAD state
        match vad_state {
            VADState::SpeechEnded => {
                // Speech ended - emit final result and clear buffer
                debug!("VAD: Speech ended, emitting final result");

                let audio_to_process = {
                    let mut buffer = self.audio_buffer.lock().await;
                    let audio = buffer.clone();
                    buffer.clear();
                    audio
                };

                // Reset VAD for next speech segment
                {
                    let mut vad = self.vad.lock().await;
                    vad.reset();
                }

                // Only transcribe if we have minimum audio
                let min_samples = duration_ms_to_samples(MIN_AUDIO_MS);
                if audio_to_process.len() >= min_samples {
                    let result = self.transcribe_audio(&audio_to_process, true, true).await?;
                    self.emit_result(result).await;
                }

                // Reset last transcript
                *self.last_transcript.lock().await = String::new();
                *self.last_transcription_time.lock().await = Instant::now();
            }
            VADState::Speaking | VADState::Silence => {
                // During speech or silence, emit interim results every 500ms
                if should_transcribe_interim {
                    let buffer = self.audio_buffer.lock().await;
                    let audio_to_process = buffer.clone();
                    drop(buffer);

                    if !audio_to_process.is_empty() {
                        let result = self
                            .transcribe_audio(&audio_to_process, false, false)
                            .await?;

                        // Only emit if transcript changed
                        let mut last = self.last_transcript.lock().await;
                        if result.transcript != *last {
                            *last = result.transcript.clone();
                            drop(last);
                            self.emit_result(result).await;
                        }
                    }

                    *self.last_transcription_time.lock().await = Instant::now();
                }
            }
        }

        Ok(())
    }

    /// Emit a result through the channel
    async fn emit_result(&self, result: STTResult) {
        if let Some(tx) = &self.result_tx {
            if tx.send(result).is_err() {
                warn!("Failed to send transcription result - channel closed");
            }
        }
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
            let min_samples = duration_ms_to_samples(MIN_AUDIO_MS);
            if audio.len() >= min_samples {
                let result = self.transcribe_audio(&audio, true, true).await?;
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
        Self::validate_model_name(&config.model)?;
        let sample_rate = config.sample_rate;
        let whisper_config = WhisperSTTConfig::from_stt_config(config)?;

        // Create VAD config with the sample rate from STT config
        let vad_config = VADConfig {
            silence_duration_ms: SILENCE_TIMEOUT_MS,
            sample_rate,
            ..Default::default()
        };

        Ok(Self {
            config: Some(whisper_config),
            state: ConnectionState::Disconnected,
            state_notify: Arc::new(Notify::new()),
            audio_buffer: Arc::new(Mutex::new(Vec::new())),
            vad: Arc::new(Mutex::new(VoiceActivityDetector::with_config(vad_config))),
            last_transcription_time: Arc::new(Mutex::new(Instant::now())),
            last_transcript: Arc::new(Mutex::new(String::new())),
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
        let config = self.config.as_ref().ok_or_else(|| {
            STTError::ConfigurationError("No configuration available".to_string())
        })?;

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

        let model_config = Self::get_model_config(&model_name);

        // Load model
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

        // Create channels
        let (result_tx, mut result_rx) = mpsc::unbounded_channel::<STTResult>();
        let (error_tx, mut error_rx) = mpsc::unbounded_channel::<STTError>();

        self.result_tx = Some(result_tx);
        self.error_tx = Some(error_tx);

        // Start result forwarding task
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

        // Start error forwarding task
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

        // Clear audio buffer and reset state
        self.audio_buffer.lock().await.clear();
        self.vad.lock().await.reset();
        *self.last_transcription_time.lock().await = Instant::now();
        *self.last_transcript.lock().await = String::new();

        self.state = ConnectionState::Connected;
        self.state_notify.notify_waiters();

        info!(
            "Connected to Whisper STT (model: {}, interim_interval: {}ms, silence_timeout: {}ms)",
            model_name, INTERIM_INTERVAL_MS, SILENCE_TIMEOUT_MS
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

        // Clear audio buffer and reset VAD
        self.audio_buffer.lock().await.clear();
        self.vad.lock().await.reset();

        // Drop model to free memory
        self.model = None;
        self.tokenizer = None;
        self.mel_processor = None;

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
                "Audio buffer: {} samples ({} ms)",
                buffer.len(),
                Self::buffer_duration_ms(buffer.len())
            );
        }

        // Process audio and emit results
        self.process_audio(&samples).await?;

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
        assert!(config.cache_path.is_none());
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
            model: "".to_string(),
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
            model: "nova-3".to_string(),
            ..Default::default()
        };

        let whisper_config = WhisperSTTConfig::from_stt_config(stt_config).unwrap();
        assert_eq!(whisper_config.model_name(), "whisper-base");
    }

    #[test]
    fn test_validate_model_name_valid() {
        assert!(WhisperSTT::validate_model_name("whisper-base").is_ok());
        assert!(WhisperSTT::validate_model_name("whisper-large-v3").is_ok());
        assert!(WhisperSTT::validate_model_name("").is_ok());
        assert!(WhisperSTT::validate_model_name("nova-3").is_ok());
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
            sample_rate: 44100,
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

        let result_callback: STTResultCallback = Arc::new(|_result| Box::pin(async move {}));
        assert!(stt.on_result(result_callback).await.is_ok());

        let error_callback: STTErrorCallback = Arc::new(|_error| Box::pin(async move {}));
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

    #[test]
    fn test_constants() {
        assert_eq!(INTERIM_INTERVAL_MS, 500);
        assert_eq!(SILENCE_TIMEOUT_MS, 1500);
        assert_eq!(MIN_AUDIO_MS, 500);
    }

    #[test]
    fn test_filter_special_annotations_silence() {
        assert_eq!(WhisperSTT::filter_special_annotations("[silence]"), "");
        assert_eq!(WhisperSTT::filter_special_annotations("[SILENCE]"), "");
        assert_eq!(WhisperSTT::filter_special_annotations("(silence)"), "");
        // With spaces inside brackets (common Whisper output)
        assert_eq!(WhisperSTT::filter_special_annotations("[ Silence ]"), "");
        assert_eq!(WhisperSTT::filter_special_annotations("[ silence ]"), "");
        assert_eq!(WhisperSTT::filter_special_annotations("[  SILENCE  ]"), "");
        assert_eq!(WhisperSTT::filter_special_annotations("( silence )"), "");
    }

    #[test]
    fn test_filter_special_annotations_music() {
        assert_eq!(WhisperSTT::filter_special_annotations("[music]"), "");
        assert_eq!(WhisperSTT::filter_special_annotations("[Music]"), "");
        assert_eq!(WhisperSTT::filter_special_annotations("(music)"), "");
        // With spaces inside brackets
        assert_eq!(WhisperSTT::filter_special_annotations("[ Music ]"), "");
        assert_eq!(WhisperSTT::filter_special_annotations("[ MUSIC ]"), "");
    }

    #[test]
    fn test_filter_special_annotations_mixed() {
        assert_eq!(
            WhisperSTT::filter_special_annotations("Hello [silence] world"),
            "Hello world"
        );
        assert_eq!(
            WhisperSTT::filter_special_annotations("[music] Welcome"),
            "Welcome"
        );
        assert_eq!(
            WhisperSTT::filter_special_annotations("Goodbye [applause]"),
            "Goodbye"
        );
    }

    #[test]
    fn test_filter_special_annotations_preserves_normal_text() {
        assert_eq!(
            WhisperSTT::filter_special_annotations("Hello world"),
            "Hello world"
        );
        assert_eq!(
            WhisperSTT::filter_special_annotations("This is a normal sentence."),
            "This is a normal sentence."
        );
    }

    #[test]
    fn test_filter_special_annotations_empty() {
        assert_eq!(WhisperSTT::filter_special_annotations(""), "");
        assert_eq!(WhisperSTT::filter_special_annotations("   "), "");
    }

    #[test]
    fn test_filter_special_annotations_multiple() {
        // Note: Current implementation only removes first occurrence of each pattern
        // This is acceptable as Whisper rarely outputs multiple of the same annotation
        assert_eq!(
            WhisperSTT::filter_special_annotations("[laughter] Ha ha [noise]"),
            "Ha ha"
        );
    }
}
