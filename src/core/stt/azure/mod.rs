//! Microsoft Azure Speech-to-Text WebSocket API integration.
//!
//! This module provides a streaming STT client for the Azure Cognitive Services
//! Speech-to-Text WebSocket API with support for:
//!
//! - Real-time streaming transcription
//! - Multiple regional endpoints worldwide
//! - Simple and detailed output formats
//! - Profanity filtering options
//! - Interim (partial) results
//! - Word-level timing
//! - Custom Speech model support
//! - Automatic language detection
//!
//! # Architecture
//!
//! The module is organized into focused submodules:
//!
//! - [`config`]: Configuration types (`AzureSTTConfig`, `AzureRegion`, etc.)
//! - [`messages`]: WebSocket message types for API communication
//! - [`client`]: The main `AzureSTT` client implementation
//!
//! # Example
//!
//! ```rust,no_run
//! use sayna::core::stt::{BaseSTT, STTConfig};
//! use sayna::core::stt::azure::{AzureSTT, AzureSTTConfig, AzureRegion};
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let base_config = STTConfig {
//!         api_key: "your-azure-subscription-key".to_string(),
//!         language: "en-US".to_string(),
//!         sample_rate: 16000,
//!         ..Default::default()
//!     };
//!
//!     // Create client with base config (uses default Azure region)
//!     let mut stt = AzureSTT::new(base_config)?;
//!
//!     // Register result callback
//!     stt.on_result(Arc::new(|result| {
//!         Box::pin(async move {
//!             println!("Transcript: {}", result.transcript);
//!         })
//!     })).await?;
//!
//!     // Connect to Azure
//!     stt.connect().await?;
//!
//!     // Send audio data
//!     let audio_data = vec![0u8; 1024]; // Your PCM audio data
//!     stt.send_audio(audio_data).await?;
//!
//!     // Disconnect when done
//!     stt.disconnect().await?;
//!
//!     Ok(())
//! }
//! ```

pub mod client;
pub mod config;
pub mod messages;

// Re-export public types for convenient access
pub use client::AzureSTT;
pub use config::{AzureOutputFormat, AzureProfanityOption, AzureRegion, AzureSTTConfig};
pub use messages::{
    AzureMessage, AzureMessageError, NBestResult, RecognitionStatus, SpeechEndDetected,
    SpeechHypothesis, SpeechPhrase, SpeechStartDetected, WordTiming,
};
