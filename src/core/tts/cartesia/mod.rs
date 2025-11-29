//! Cartesia Text-to-Speech provider.
//!
//! This module provides integration with Cartesia's TTS REST API for
//! high-quality speech synthesis using the Sonic voice models.
//!
//! # Architecture
//!
//! The Cartesia TTS provider follows Sayna's HTTP-based TTS pattern:
//!
//! ```text
//! CartesiaTTS
//!     │
//!     └── TTSProvider (generic HTTP infrastructure)
//!             │
//!             ├── ReqManager (connection pooling)
//!             ├── Dispatcher (ordered audio delivery)
//!             └── QueueWorker (sequential request processing)
//! ```
//!
//! The module is organized into:
//! - **config**: Cartesia-specific configuration types
//!   - `CartesiaTTSConfig` - Provider configuration
//!   - `CartesiaOutputFormat` - Audio format specification
//!   - `CartesiaAudioContainer` - Container type enum
//!   - `CartesiaAudioEncoding` - PCM encoding enum
//!
//! - **provider**: Request builder and main provider
//!   - `CartesiaRequestBuilder` - HTTP request construction
//!   - `CartesiaTTS` - Main provider implementing `BaseTTS`
//!
//! # Quick Start
//!
//! ## Via Factory Function (Recommended)
//!
//! ```rust,ignore
//! use sayna::core::tts::{create_tts_provider, TTSConfig, BaseTTS};
//!
//! let config = TTSConfig {
//!     provider: "cartesia".to_string(),
//!     api_key: std::env::var("CARTESIA_API_KEY").unwrap(),
//!     voice_id: Some("a0e99841-438c-4a64-b679-ae501e7d6091".to_string()),
//!     audio_format: Some("linear16".to_string()),
//!     sample_rate: Some(24000),
//!     ..Default::default()
//! };
//!
//! let mut tts = create_tts_provider("cartesia", config)?;
//! tts.connect().await?;
//! tts.speak("Hello, world!", true).await?;
//! ```
//!
//! ## Direct Instantiation
//!
//! ```rust,ignore
//! use sayna::core::tts::cartesia::CartesiaTTS;
//! use sayna::core::tts::{TTSConfig, BaseTTS};
//!
//! let config = TTSConfig {
//!     api_key: std::env::var("CARTESIA_API_KEY").unwrap(),
//!     voice_id: Some("a0e99841-438c-4a64-b679-ae501e7d6091".to_string()),
//!     ..Default::default()
//! };
//!
//! let mut tts = CartesiaTTS::new(config)?;
//! tts.connect().await?;
//! tts.speak("Hello from Cartesia!", true).await?;
//! tts.disconnect().await?;
//! ```
//!
//! # Authentication
//!
//! Cartesia uses API key authentication with Bearer token:
//!
//! ```http
//! Authorization: Bearer <CARTESIA_API_KEY>
//! Cartesia-Version: 2025-04-16
//! ```
//!
//! Set the API key via:
//! - Environment variable: `CARTESIA_API_KEY`
//! - YAML config: `providers.cartesia_api_key`
//! - Direct: `TTSConfig.api_key`
//!
//! # Supported Audio Formats
//!
//! | Format | Container | Encoding | Use Case |
//! |--------|-----------|----------|----------|
//! | `linear16` | raw | pcm_s16le | Real-time streaming (default) |
//! | `wav` | wav | pcm_s16le | File storage |
//! | `mp3` | mp3 | - | Bandwidth optimization |
//! | `mulaw` | raw | pcm_mulaw | US telephony |
//! | `alaw` | raw | pcm_alaw | European telephony |
//!
//! # API Reference
//!
//! - Endpoint: `https://api.cartesia.ai/tts/bytes`
//! - Authentication: Bearer token in Authorization header
//! - Required header: `Cartesia-Version` (API version date)
//! - Documentation: <https://docs.cartesia.ai/api-reference/tts/bytes>
//!
//! # See Also
//!
//! - [`crate::core::tts::BaseTTS`] - Base trait for TTS providers
//! - [`crate::core::tts::create_tts_provider`] - Factory function
//! - [`crate::core::stt::cartesia`] - Cartesia STT provider

mod config;
mod provider;

// =============================================================================
// Public Re-exports
// =============================================================================

// Configuration types
pub use config::{
    CARTESIA_TTS_URL, CartesiaAudioContainer, CartesiaAudioEncoding, CartesiaOutputFormat,
    CartesiaTTSConfig, DEFAULT_API_VERSION, DEFAULT_MODEL, SUPPORTED_SAMPLE_RATES,
};

// Provider types
pub use provider::{CartesiaRequestBuilder, CartesiaTTS};
