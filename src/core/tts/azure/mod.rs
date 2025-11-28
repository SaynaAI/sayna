//! Microsoft Azure Text-to-Speech provider.
//!
//! This module provides Azure-specific TTS configuration and provider implementation
//! for the Azure Cognitive Services Text-to-Speech API.
//!
//! # Architecture
//!
//! The module is organized into two main components:
//!
//! - **config**: Azure-specific configuration types (`AzureTTSConfig`, `AzureAudioEncoding`)
//!   and SSML generation utilities.
//! - **provider**: The `AzureTTS` provider implementation and `AzureRequestBuilder`
//!   for constructing HTTP requests.
//!
//! # Example
//!
//! ```rust,ignore
//! use sayna::core::tts::azure::{AzureTTS, AzureTTSConfig, AzureAudioEncoding};
//! use sayna::core::tts::{TTSConfig, BaseTTS};
//! use sayna::core::providers::azure::AzureRegion;
//!
//! // Create configuration
//! let config = TTSConfig {
//!     api_key: "your-azure-subscription-key".to_string(),
//!     voice_id: Some("en-US-JennyNeural".to_string()),
//!     audio_format: Some("linear16".to_string()),
//!     speaking_rate: Some(1.0),
//!     ..Default::default()
//! };
//!
//! // Create provider with specific region
//! let mut tts = AzureTTS::with_region(config, AzureRegion::WestEurope)?;
//!
//! // Connect and synthesize
//! tts.connect().await?;
//! tts.speak("Hello, world!", true).await?;
//! tts.disconnect().await?;
//! ```
//!
//! # Azure TTS API Reference
//!
//! - TTS endpoint: `https://{region}.tts.speech.microsoft.com/cognitiveservices/v1`
//! - Required headers: `Ocp-Apim-Subscription-Key`, `Content-Type: application/ssml+xml`,
//!   `X-Microsoft-OutputFormat`
//! - Documentation: <https://learn.microsoft.com/en-us/azure/ai-services/speech-service/rest-text-to-speech>

mod config;
mod provider;

// Re-export configuration types
pub use config::{
    AZURE_OUTPUT_FORMAT_HEADER, AZURE_TTS_URL, AzureAudioEncoding, AzureTTSConfig, build_ssml,
    escape_xml,
};

// Re-export provider types
pub use provider::{AzureRequestBuilder, AzureTTS};
