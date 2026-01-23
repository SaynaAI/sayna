//! # Voice Manager
//!
//! Central coordinator for STT and TTS providers with real-time voice processing.
//!
//! This module provides [`VoiceManager`], a unified interface for managing Speech-to-Text
//! and Text-to-Speech operations. It handles provider lifecycle, callback registration,
//! and speech final detection (via VAD + Smart-Turn when `stt-vad` is enabled, or native
//! provider signals otherwise).
//!
//! See [`docs/voice_manager.md`](../../../docs/voice_manager.md) for detailed usage
//! examples including custom timing, real-time processing, and error handling patterns.

pub mod callbacks;
pub mod config;
pub mod errors;
pub mod manager;
pub mod state;
pub mod stt_config;
pub mod stt_result;
mod turn_detection_tasks;
pub(crate) mod utils;
#[cfg(feature = "stt-vad")]
pub mod vad_processor;

#[cfg(test)]
mod tests;

// Re-export commonly used items
pub use callbacks::{
    AudioClearCallback, STTCallback, STTErrorCallback, TTSAudioCallback, TTSErrorCallback,
};
pub use config::{NoiseFilterConfig, SpeechFinalConfig, VoiceManagerConfig};
pub use errors::{VoiceManagerError, VoiceManagerResult};
pub use manager::VoiceManager;
pub use state::SpeechFinalState;
pub use stt_config::STTProcessingConfig;
pub use stt_result::STTResultProcessor;
#[cfg(feature = "stt-vad")]
pub use vad_processor::VADState;
