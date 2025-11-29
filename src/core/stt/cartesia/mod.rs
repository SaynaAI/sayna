//! Cartesia Speech-to-Text WebSocket API integration.
//!
//! This module provides a streaming STT client for the Cartesia WebSocket API
//! with support for:
//!
//! - Real-time streaming transcription
//! - Voice Activity Detection (VAD) with configurable thresholds
//! - Multiple audio format support (PCM)
//! - Configurable sample rates
//!
//! # Architecture
//!
//! The module is organized into focused submodules:
//!
//! - [`config`]: Configuration types (`CartesiaSTTConfig`, `CartesiaAudioFormat`, etc.)
//! - [`messages`]: WebSocket message types for API communication
//! - [`client`]: The main `CartesiaSTT` client implementation
//!
//! # Example
//!
//! ```rust,no_run
//! use sayna::core::stt::{BaseSTT, STTConfig, CartesiaSTT};
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = STTConfig {
//!         api_key: "your-cartesia-api-key".to_string(),
//!         language: "en".to_string(),
//!         sample_rate: 16000,
//!         ..Default::default()
//!     };
//!
//!     let mut stt = CartesiaSTT::new(config)?;
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
pub use client::CartesiaSTT;
pub use config::{
    CartesiaAudioEncoding, CartesiaSTTConfig, SUPPORTED_SAMPLE_RATES, is_sample_rate_supported,
};
pub use messages::{
    CartesiaCommand, CartesiaMessage, CartesiaSTTError, CartesiaTranscript, CartesiaWord,
    DoneMessage, FinalTranscript, PartialTranscript, ReadyMessage,
};
