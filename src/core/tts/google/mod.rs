//! Google Cloud Text-to-Speech provider.
//!
//! This module provides integration with Google Cloud Text-to-Speech API,
//! including authentication, configuration, request building, and API client functionality.
//!
//! # Architecture
//!
//! The module is organized into:
//! - **config**: Google TTS-specific configuration (`GoogleTTSConfig`, `GoogleAudioEncoding`)
//! - **provider**: Authentication wrapper (`TTSGoogleAuthClient`) and request builder (`GoogleRequestBuilder`)
//!
//! # Usage
//!
//! ```rust,ignore
//! use sayna::core::tts::google::{GoogleRequestBuilder, GoogleTTSConfig, TTSGoogleAuthClient};
//! use sayna::core::tts::{TTSConfig, provider::TTSRequestBuilder};
//! use sayna::core::providers::google::TokenProvider;
//!
//! // 1. Create auth client and get token
//! let auth_client = TTSGoogleAuthClient::from_api_key("/path/to/credentials.json")?;
//! let token = auth_client.get_token().await?;
//!
//! // 2. Create configurations
//! let base_config = TTSConfig {
//!     voice_id: Some("en-US-Wavenet-D".to_string()),
//!     ..Default::default()
//! };
//! let google_config = GoogleTTSConfig::from_base_config(base_config.clone(), "project-id".to_string());
//!
//! // 3. Build requests
//! let builder = GoogleRequestBuilder::new(base_config, google_config, token);
//! let client = reqwest::Client::new();
//! let request = builder.build_http_request(&client, "Hello, world!");
//! ```

mod config;
mod provider;

// Re-export configuration types
pub use config::{GoogleAudioEncoding, GoogleTTSConfig};

// Re-export the auth wrapper for use by other modules
pub use provider::TTSGoogleAuthClient;

// Re-export request builder and API endpoint constant
pub use provider::{GOOGLE_TTS_URL, GoogleRequestBuilder};

// Re-export the main GoogleTTS provider
pub use provider::GoogleTTS;

// Re-export error conversion for internal use
#[allow(unused_imports)]
pub(crate) use provider::google_error_to_tts;
