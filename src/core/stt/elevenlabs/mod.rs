//! ElevenLabs Speech-to-Text Real-Time WebSocket API integration.
//!
//! This module provides a streaming STT client for the ElevenLabs Real-Time
//! WebSocket API with support for:
//!
//! - Real-time streaming transcription
//! - Multiple regional endpoints (US, EU, India)
//! - VAD-based automatic or manual commit strategies
//! - Word-level timestamps
//! - Multiple audio format support (PCM, μ-law)
//!
//! # Architecture
//!
//! The module is organized into focused submodules:
//!
//! - [`config`]: Configuration types (`ElevenLabsSTTConfig`, `ElevenLabsAudioFormat`, etc.)
//! - [`messages`]: WebSocket message types for API communication
//! - [`client`]: The main `ElevenLabsSTT` client implementation
//!
//! # Example
//!
//! ```rust,no_run
//! use sayna::core::stt::{BaseSTT, STTConfig, ElevenLabsSTT};
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = STTConfig {
//!         api_key: "your-elevenlabs-api-key".to_string(),
//!         language: "en".to_string(),
//!         sample_rate: 16000,
//!         ..Default::default()
//!     };
//!
//!     let mut stt = ElevenLabsSTT::new(config)?;
//!     stt.connect().await?;
//!
//!     // Register callback for results
//!     stt.on_result(Arc::new(|result| {
//!         Box::pin(async move {
//!             println!("Transcription: {}", result.transcript);
//!         })
//!     })).await?;
//!
//!     // Send audio data
//!     let audio_data = vec![0u8; 1024];
//!     stt.send_audio(audio_data).await?;
//!
//!     Ok(())
//! }
//! ```

mod client;
mod config;
mod messages;

#[cfg(test)]
mod tests;

// Re-export public types
pub use client::ElevenLabsSTT;
pub use config::{CommitStrategy, ElevenLabsAudioFormat, ElevenLabsRegion, ElevenLabsSTTConfig};
pub use messages::{
    CommittedTranscript, CommittedTranscriptWithTimestamps, ElevenLabsMessage, ElevenLabsSTTError,
    EndOfStream, InputAudioChunk, PartialTranscript, SessionStarted, WordTiming,
};
