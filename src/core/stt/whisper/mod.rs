//! Whisper Speech-to-Text - Local ONNX-based STT Provider
//!
//! This module provides a local STT solution using OpenAI's Whisper model
//! running on ONNX Runtime. No cloud API is required.
//!
//! ## Features
//!
//! - Local inference using ONNX Runtime (no API keys needed)
//! - Multiple Whisper model sizes (tiny, base, small, medium, large)
//! - Multilingual transcription with language detection
//! - 16kHz mono PCM audio input
//! - Word-level timestamps support
//!
//! ## System Requirements
//!
//! - `whisper-stt` feature flag must be enabled
//! - ONNX Runtime (bundled with the crate)
//! - Model files downloaded via `sayna init`
//!
//! ## Usage
//!
//! ```rust,ignore
//! use sayna::core::stt::{BaseSTT, STTConfig, WhisperSTT};
//!
//! let config = STTConfig {
//!     provider: "whisper".to_string(),
//!     language: "en".to_string(),
//!     sample_rate: 16000,
//!     ..Default::default()
//! };
//!
//! let mut stt = WhisperSTT::new(config)?;
//! stt.connect().await?;
//!
//! // Send audio data
//! let audio_data = vec![0u8; 1024];
//! stt.send_audio(audio_data).await?;
//! ```
//!
//! ## Architecture
//!
//! The module is organized into focused submodules:
//!
//! - [`assets`]: HuggingFace model download and caching
//! - [`client`]: The main `WhisperSTT` client implementation
//! - [`mel`]: Mel spectrogram computation for audio preprocessing
//! - [`model`]: ONNX model manager for encoder and decoder
//! - [`tokenizer`]: BPE tokenizer for output decoding

mod assets;
mod client;
mod mel;
mod model;
mod tokenizer;
mod vad;

// Re-export public types
pub use assets::{
    DEFAULT_MODEL, SUPPORTED_MODELS, WhisperAssetConfig, are_assets_available, config_path,
    decoder_model_path, download_assets, encoder_model_path, get_model_url, list_available_models,
    tokenizer_path,
};
pub use client::{StreamingConfig, WhisperSTT, WhisperSTTConfig};
pub use mel::{
    CHUNK_LENGTH, CHUNK_LENGTH_SECONDS, DEFAULT_N_MELS, HOP_LENGTH, LARGE_V3_N_MELS, MelProcessor,
    N_FFT, N_FRAMES, SAMPLE_RATE, duration_ms_to_samples, pcm16_bytes_to_f32, pcm16_to_f32,
    samples_to_duration_ms, validate_audio_format,
};
pub use tokenizer::{
    LANGUAGE_NAMES, LANGUAGES, Task, Timestamp, WhisperTokenizer, token_strings, tokens,
};

// Re-export model types
pub use model::{InferenceConfig, ModelConfig, WhisperModel};

// Re-export VAD types
pub use vad::{VADConfig, VADState, VoiceActivityDetector, compute_energy, is_speech};
